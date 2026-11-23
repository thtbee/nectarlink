// SPDX-License-Identifier: MPL-2.0
//! The pairing ceremony (protocol §9) over the `nectarlink-pair/0` ALPN.
//!
//! Two flows:
//! - **QR**: the host (PC) shows a pairing link with a one-time secret; the
//!   joiner (phone) proves knowledge of it.
//! - **Nearby**: numeric comparison. Both screens show a 6-digit code derived
//!   from both identities and committed nonces; both users confirm.
//!
//! Both flows end the same way, so a pairing is never stored on only one
//! side in normal operation: the host sends the last protocol message, the
//! initiator acknowledges it with `pair.done`, the host stores the pairing
//! and closes with [`CLOSE_PAIRED`], and the initiator stores it when it sees
//! that close. Failures close with [`CLOSE_FAILED`] and nobody stores.

use std::{
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use iroh::{
    EndpointAddr,
    endpoint::{Connection, ConnectionError, RecvStream, SendStream, VarInt},
};
use nectarlink_protocol::{
    ALPN_PAIR, DeviceId, Envelope, ErrorCode,
    messages::*,
    pairing::{self as proto, NONCE_LEN, PairingUri, SECRET_LEN},
    read_frame, write_frame,
};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::{
    Error, Result,
    events::{NodeEvent, PairingEvent, PairingFailure},
    node::Shared,
};

/// How long the host stays in pairing mode.
pub(crate) const PAIRING_TTL: Duration = Duration::from_secs(5 * 60);
/// How long to wait for the users to compare the 6-digit codes.
const SAS_DECISION_TIMEOUT: Duration = Duration::from_secs(120);
/// How long each pairing message may take to arrive.
const STEP_TIMEOUT: Duration = Duration::from_secs(20);
/// Wrong proofs tolerated before pairing mode ends.
const MAX_FAILED_ATTEMPTS: u32 = 5;
const DIAL_TIMEOUT: Duration = Duration::from_secs(15);
/// How long the initiator waits for the host to store the pairing and close.
const FINISH_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a host that failed waits for the initiator to read its error.
const LINGER_TIMEOUT: Duration = Duration::from_secs(2);

/// Close code: the host stored the pairing.
const CLOSE_PAIRED: u32 = 0;
/// Close code: the ceremony failed; nothing was stored.
const CLOSE_FAILED: u32 = 1;

/// Host-side pairing mode, active while a QR code is shown.
struct HostMode {
    secret: [u8; SECRET_LEN],
    expires_at: Instant,
    failures_by_peer: std::collections::HashMap<DeviceId, u32>,
    /// Which showing of a QR code this is: a ceremony that proved itself
    /// against one may only finish while that one is still shown.
    generation: u64,
    /// The device that proved it has the code and is finishing, if any: one
    /// code pairs one device.
    claimed_by: Option<DeviceId>,
}

/// Pairing state shared by both flows. Only one ceremony runs at a time.
#[derive(Default)]
pub(crate) struct PairingState {
    host: Mutex<Option<HostMode>>,
    /// Waiting for the local user to confirm or reject the 6-digit code.
    sas_decision: Mutex<Option<(DeviceId, oneshot::Sender<bool>)>>,
    /// True while a nearby ceremony is in flight, so a second one cannot
    /// overwrite `sas_decision`.
    nearby_active: std::sync::atomic::AtomicBool,
    /// Cancelled (and replaced) when the user cancels pairing: every ceremony
    /// running then stops, whatever step it waits on.
    ceremonies: Mutex<CancellationToken>,
    generations: std::sync::atomic::AtomicU64,
}

/// A QR code's claim by the device finishing with it; let go when dropped,
/// so a ceremony that fails before it's stored leaves the code usable.
struct QrClaim<'a> {
    state: &'a PairingState,
    generation: u64,
    peer: DeviceId,
}

impl Drop for QrClaim<'_> {
    fn drop(&mut self) {
        if let Some(mode) = lock(&self.state.host).as_mut()
            && mode.generation == self.generation
            && mode.claimed_by == Some(self.peer)
        {
            mode.claimed_by = None;
        }
    }
}

pub(crate) struct NearbyGuard<'a>(&'a PairingState);

impl Drop for NearbyGuard<'_> {
    fn drop(&mut self) {
        self.0.clear_decision();
        self.0.nearby_active.store(false, std::sync::atomic::Ordering::Release);
    }
}

impl std::fmt::Debug for PairingState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairingState").finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl PairingState {
    /// The QR code shown now, if one is: its secret and generation.
    fn host_secret(&self) -> Option<([u8; SECRET_LEN], u64)> {
        let mut host = lock(&self.host);
        match host.as_ref() {
            Some(mode) if mode.expires_at > Instant::now() => Some((mode.secret, mode.generation)),
            Some(_) => {
                *host = None;
                None
            }
            None => None,
        }
    }

    pub(crate) fn is_hosting(&self) -> bool {
        self.host_secret().is_some()
    }

    fn host_generation(&self) -> Option<u64> {
        self.host_secret().map(|(_, generation)| generation)
    }

    fn end_host(&self) {
        *lock(&self.host) = None;
    }

    /// Ends pairing mode if it's still the showing `generation` started; a
    /// newer one is left alone.
    fn end_host_if(&self, generation: Option<u64>) {
        let mut host = lock(&self.host);
        if generation.is_some() && host.as_ref().map(|m| m.generation) == generation {
            *host = None;
        }
    }

    fn next_generation(&self) -> u64 {
        self.generations.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1
    }

    /// The token every ceremony starting now stops on.
    fn ceremony(&self) -> CancellationToken {
        lock(&self.ceremonies).clone()
    }

    /// Stops every ceremony running now; later ones start afresh.
    fn cancel_ceremonies(&self) {
        std::mem::take(&mut *lock(&self.ceremonies)).cancel();
    }

    /// Claims the QR code of `generation` for `peer`, unless it's gone or
    /// another device is already finishing with it.
    fn claim_qr(&self, generation: u64, peer: DeviceId) -> Option<QrClaim<'_>> {
        let mut host = lock(&self.host);
        let mode = host.as_mut().filter(|m| m.generation == generation && m.expires_at > Instant::now())?;
        if mode.claimed_by.is_some_and(|by| by != peer) {
            return None;
        }
        mode.claimed_by = Some(peer);
        Some(QrClaim { state: self, generation, peer })
    }

    /// Whether the QR code of `generation` is still shown (not cancelled,
    /// replaced or expired).
    fn host_is(&self, generation: u64) -> bool {
        self.host_generation() == Some(generation)
    }

    /// Records a failed attempt for `peer`; returns true if pairing mode just ended.
    fn record_failure(&self, peer: DeviceId) -> bool {
        let mut host = lock(&self.host);
        if let Some(mode) = host.as_mut() {
            let count = mode.failures_by_peer.entry(peer).or_insert(0);
            *count += 1;
            if *count >= MAX_FAILED_ATTEMPTS {
                *host = None;
                return true;
            }
        }
        false
    }

    fn cancel_nearby(&self) {
        if let Some((_, tx)) = lock(&self.sas_decision).take() {
            let _ = tx.send(false);
        }
    }

    fn begin_nearby(&self) -> Option<NearbyGuard<'_>> {
        self.nearby_active
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .ok()
            .map(|_| NearbyGuard(self))
    }

    /// Delivers the local user's decision about the 6-digit code.
    pub(crate) fn decide(&self, matches: bool) -> Result<()> {
        match lock(&self.sas_decision).take() {
            Some((_, tx)) => {
                let _ = tx.send(matches);
                Ok(())
            }
            None => Err(Error::NotPairing),
        }
    }

    fn await_decision(&self, peer: DeviceId) -> Result<oneshot::Receiver<bool>> {
        let mut slot = lock(&self.sas_decision);
        if slot.is_some() {
            return Err(Error::Denied);
        }
        let (tx, rx) = oneshot::channel();
        *slot = Some((peer, tx));
        Ok(rx)
    }

    fn clear_decision(&self) {
        *lock(&self.sas_decision) = None;
    }
}

/// Starts host pairing mode and returns the link to show as a QR code.
pub(crate) async fn start_host(shared: &Arc<Shared>) -> Result<PairingUri> {
    let secret: [u8; SECRET_LEN] = rand::random();
    *lock(&shared.pairing.host) = Some(HostMode {
        secret,
        expires_at: Instant::now() + PAIRING_TTL,
        failures_by_peer: std::collections::HashMap::new(),
        generation: shared.pairing.next_generation(),
        claimed_by: None,
    });

    // Expire quietly; tell the UI if nobody paired in time.
    let weak = Arc::downgrade(shared);
    tokio::spawn(async move {
        tokio::time::sleep(PAIRING_TTL).await;
        if let Some(shared) = weak.upgrade() {
            let still_ours = lock(&shared.pairing.host).as_ref().is_some_and(|m| m.secret == secret);
            if still_ours {
                shared.pairing.end_host();
                shared.emit(NodeEvent::Pairing(PairingEvent::Failed(PairingFailure::Expired)));
            }
        }
    });

    let name = local_device(shared).name;
    Ok(PairingUri {
        id: shared.id,
        secret,
        name,
        addrs: {
            let mut addrs = shared.direct_addrs().await;
            let tailscale: Vec<SocketAddr> =
                shared.tailscale_sockets().into_iter().filter(|a| !addrs.contains(a)).collect();
            addrs.extend(tailscale);
            link_addrs(addrs, default_route_ip(), &shared.local_tailscale_ips())
        },
    })
}

/// Most addresses a pairing link carries; more only make the QR code denser.
const MAX_LINK_ADDRS: usize = 3;

/// Picks the addresses worth putting in a pairing link. The best LAN address
/// always gets a slot: the one the default route leaves from (`preferred`),
/// else private IPv4 (what a phone on the same network almost always uses),
/// else other IPv4, else one IPv6. Among private ranges, home routers'
/// `192.168/16` and `10/8` come before `172.16/12`, where Hyper-V, WSL and
/// Docker put their adapters (#148). Then this device's Tailscale addresses,
/// but only ones its own Tailscale confirms (`local_tailscale`): on a network
/// that keeps devices apart they're the only ones that work, and VMware's
/// adapters use `192.168/16`, so they'd crowd them out otherwise. A carrier's
/// CGNAT address (also `100.64/10`) is just another IPv4 address. Remaining
/// slots go to the next best LAN addresses. Link-local IPv6 is useless without
/// a scope ID. Discovery on the network finds the device even if none of
/// these work.
fn link_addrs(
    mut addrs: Vec<SocketAddr>,
    preferred: Option<IpAddr>,
    local_tailscale: &[IpAddr],
) -> Vec<SocketAddr> {
    let proven = |ip: IpAddr| crate::session::is_corroborated_tailnet_ip(ip, local_tailscale);
    let rank = |a: &SocketAddr| match a.ip() {
        ip if Some(ip) == preferred => 0,
        IpAddr::V4(v4) if v4.is_private() && v4.octets()[0] != 172 => 2,
        IpAddr::V4(v4) if v4.is_private() => 3,
        IpAddr::V4(v4) if v4.is_loopback() || v4.is_link_local() => 6,
        IpAddr::V4(_) => 4,
        IpAddr::V6(_) => 5,
    };
    let has_non_loopback = addrs.iter().any(|a| match a.ip() {
        IpAddr::V6(v6) => !v6.is_loopback() && !v6.is_unicast_link_local(),
        IpAddr::V4(v4) => !v4.is_loopback(),
    });
    addrs.retain(|a| match a.ip() {
        IpAddr::V6(v6) => !v6.is_loopback() && !v6.is_unicast_link_local(),
        IpAddr::V4(v4) => !has_non_loopback || !v4.is_loopback(),
    });
    addrs.dedup();
    let (mut tailnet, mut lan): (Vec<SocketAddr>, Vec<SocketAddr>) =
        addrs.into_iter().partition(|a| Some(a.ip()) != preferred && proven(a.ip()));
    lan.sort_by_key(rank);
    // IPv4 first: every tailnet has it, IPv6 is optional.
    tailnet.sort_by_key(SocketAddr::is_ipv6);

    let mut picked = Vec::new();
    let mut lan = lan.into_iter();
    let mut has_v6 = false;
    let mut next_lan = |picked: &mut Vec<SocketAddr>| {
        for addr in lan.by_ref() {
            if addr.is_ipv6() {
                if has_v6 {
                    continue;
                }
                has_v6 = true;
            }
            picked.push(addr);
            return true;
        }
        false
    };
    // The reserved LAN slot, then Tailscale, then more LAN.
    next_lan(&mut picked);
    for addr in tailnet {
        if picked.len() == MAX_LINK_ADDRS {
            break;
        }
        picked.push(addr);
    }
    while picked.len() < MAX_LINK_ADDRS && next_lan(&mut picked) {}
    picked
}

pub(crate) fn cancel(shared: &Shared) {
    shared.pairing.end_host();
    shared.pairing.clear_decision();
    shared.pairing.cancel_ceremonies();
}

/// Runs `step` unless the user cancels pairing first.
async fn until_cancelled<T>(
    ceremony: &CancellationToken,
    step: impl Future<Output = Result<T>>,
) -> Result<T> {
    tokio::select! {
        _ = ceremony.cancelled() => Err(Error::Declined),
        result = step => result,
    }
}

// ---- Initiator side ----

/// Joins a host by scanning its pairing link (the phone side of the QR flow).
pub(crate) async fn join_qr(shared: &Arc<Shared>, uri: &PairingUri) -> Result<()> {
    let ceremony = shared.pairing.ceremony();
    shared.remember_addrs(&uri.id, &uri.addrs);
    let conn = until_cancelled(&ceremony, dial(shared, &uri.id)).await?;
    let exchange = async {
        let (mut send, mut recv) = conn.open_bi().await.map_err(crate::error::net)?;
        let device = local_device(shared);
        let proof = proto::pair_proof(&uri.secret, &shared.id, &uri.id).to_vec();
        send_env(&mut send, Envelope::new(types::PAIR_REQUEST, &PairRequest { device, proof })?).await?;
        let accept: PairAccept = recv_env(&mut recv, STEP_TIMEOUT).await?.expect_body(types::PAIR_ACCEPT)?;
        Ok((send, accept.device.sanitized()))
    };
    // A cancel counts until `pair.done` goes out; after that the host stores it.
    let result = match until_cancelled(&ceremony, exchange).await {
        Ok((send, device)) => finish_initiator(&conn, send).await.map(|()| device),
        Err(e) => Err(e),
    };
    settle_initiator(shared, &conn, uri.id, result).await?;
    // Only now is it a paired device: keep the link's addresses to reconnect by.
    if let Err(e) = shared.store.replace_candidates(
        &uri.id,
        crate::store::CandidateSource::Discovery,
        &uri.addrs,
        crate::now_unix(),
    ) {
        tracing::debug!(error = %e, "couldn't keep the pairing link's addresses");
    }
    shared.refresh_lookup(&uri.id);
    Ok(())
}

/// Starts the nearby (numeric comparison) flow with a device in pairing
/// mode. The 6-digit code arrives as [`PairingEvent::SasCode`]; answer with
/// [`crate::Node::pairing_confirm`].
pub(crate) async fn start_nearby(shared: &Arc<Shared>, peer: DeviceId) -> Result<()> {
    let ceremony = shared.pairing.ceremony();
    let nearby = shared.pairing.begin_nearby().ok_or(Error::Denied)?;
    let conn = match until_cancelled(&ceremony, dial(shared, &peer)).await {
        Ok(conn) => conn,
        Err(e) => {
            drop(nearby);
            return Err(e);
        }
    };
    let exchange = async {
        let (mut send, mut recv) = conn.open_bi().await.map_err(crate::error::net)?;
        let n_a: [u8; NONCE_LEN] = rand::random();
        let commit = PairCommit { device: local_device(shared), c: proto::commitment(&n_a).to_vec() };
        send_env(&mut send, Envelope::new(types::PAIR_COMMIT, &commit)?).await?;
        let nonce: PairNonce = recv_env(&mut recv, STEP_TIMEOUT).await?.expect_body(types::PAIR_NONCE)?;
        send_env(&mut send, Envelope::new(types::PAIR_REVEAL, &PairReveal { n_a: n_a.to_vec() })?).await?;

        // We are A (initiator), the host is B.
        let code = proto::sas_code(&n_a, &nonce.n_b, &shared.id, &peer);
        exchange_decisions(shared, peer, code, &mut send, &mut recv, Role::Initiator).await?;
        Ok((send, nonce.device.sanitized()))
    };
    let result = match until_cancelled(&ceremony, exchange).await {
        Ok((send, device)) => finish_initiator(&conn, send).await.map(|()| device),
        Err(e) => Err(e),
    };
    drop(nearby);
    settle_initiator(shared, &conn, peer, result).await
}

/// Sends `pair.done` and waits for the host to store the pairing and close
/// with [`CLOSE_PAIRED`]. Any other outcome means the host didn't store it.
async fn finish_initiator(conn: &Connection, mut send: SendStream) -> Result<()> {
    send_env(&mut send, Envelope::empty(types::PAIR_DONE)).await?;
    let _ = send.finish();
    match tokio::time::timeout(FINISH_TIMEOUT, conn.closed()).await {
        Ok(ConnectionError::ApplicationClosed(close))
            if close.error_code == VarInt::from_u32(CLOSE_PAIRED) =>
        {
            Ok(())
        }
        Ok(other) => Err(Error::Network(format!("pairing ended unexpectedly: {other}"))),
        Err(_) => Err(Error::Timeout),
    }
}

async fn settle_initiator(
    shared: &Arc<Shared>,
    conn: &Connection,
    peer: DeviceId,
    result: Result<DeviceInfo>,
) -> Result<()> {
    match result {
        Ok(device) => shared.complete_pairing(peer, device).await,
        Err(err) => {
            // Reached, but refused because it isn't on its pairing screen:
            // say so, rather than blaming the network (#514).
            if refused_not_pairing(conn) {
                shared.emit(NodeEvent::Pairing(PairingEvent::Failed(PairingFailure::NotPairing)));
                return Err(err);
            }
            conn.close(VarInt::from_u32(CLOSE_FAILED), b"failed");
            Err(fail(shared, err))
        }
    }
}

/// Whether the host closed this connection because it isn't in pairing mode.
fn refused_not_pairing(conn: &Connection) -> bool {
    matches!(
        conn.close_reason(),
        Some(ConnectionError::ApplicationClosed(close))
            if close.error_code == VarInt::from_u32(ErrorCode::Denied.close_code()) && close.reason.as_ref() == NOT_PAIRING
    )
}

/// The reason a host gives when it refuses pairing outside pairing mode.
const NOT_PAIRING: &[u8] = b"not pairing";

// ---- Host side ----

/// Handles an incoming pairing connection (host side of both flows).
pub(crate) async fn accept(shared: Arc<Shared>, conn: Connection) {
    let peer = crate::device_id(&conn.remote_id());
    // Over the USB tunnel pairing needs no pairing mode, but only while adb
    // could be what reached the tunnel: any app on the phone can reach it too.
    let is_usb = shared.is_usb_connection(&conn) && shared.platform.adb_may_be_connected();
    let ceremony = shared.pairing.ceremony();
    // The QR code shown when this started, if one was: it may be ended once
    // this pairs, but not a newer one.
    let host_generation = shared.pairing.host_generation();
    if host_generation.is_none() && !is_usb {
        tracing::debug!(peer = %peer.short(), "refusing pairing connection: not in pairing mode");
        conn.close(VarInt::from_u32(ErrorCode::Denied.close_code()), NOT_PAIRING);
        return;
    }
    // The first stream must come soon, and not after a cancel.
    let streams = until_cancelled(&ceremony, async {
        tokio::time::timeout(STEP_TIMEOUT, conn.accept_bi())
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(crate::error::net)
    })
    .await;
    let (mut send, mut recv) = match streams {
        Ok(streams) => streams,
        Err(e) => {
            tracing::debug!(peer = %peer.short(), error = %e, "pairing connection ended before its first stream");
            conn.close(VarInt::from_u32(CLOSE_FAILED), b"failed");
            return;
        }
    };
    let result = until_cancelled(&ceremony, async {
        let first = recv_env(&mut recv, STEP_TIMEOUT).await?;
        let mut claim = None;
        let device = match first.t.as_str() {
            types::PAIR_REQUEST => {
                let (device, qr) = accept_qr(&shared, peer, first, &mut send).await?;
                claim = Some(qr);
                device
            }
            types::PAIR_COMMIT => {
                match accept_nearby(&shared, peer, first, &mut send, &mut recv, is_usb).await {
                    Ok(Some(device)) => device,
                    Ok(None) => return Err(Error::Denied),
                    Err((e, sas_shown)) => {
                        let ended = shared.pairing.record_failure(peer);
                        if sas_shown {
                            return Err(fail(&shared, e));
                        }
                        if ended {
                            shared.emit(NodeEvent::Pairing(PairingEvent::Failed(PairingFailure::Rejected)));
                        }
                        return Err(e);
                    }
                }
            }
            other => {
                let reply = Envelope::error(ErrorCode::Unsupported, format!("unexpected {other}"));
                send_env(&mut send, reply).await?;
                return Err(Error::Protocol(format!("unexpected pairing message {other}")));
            }
        };
        // Store only once the initiator acknowledged our last message, and
        // for a QR code only while that code is still shown.
        recv_env(&mut recv, STEP_TIMEOUT).await?.expect(types::PAIR_DONE)?;
        if let Some(qr) = &claim
            && !shared.pairing.host_is(qr.generation)
        {
            return Err(Error::NotPairing);
        }
        shared.pairing.end_host_if(claim.as_ref().map(|qr| qr.generation).or(host_generation));
        Ok(device)
    })
    .await;

    match result {
        Ok(device) => match shared.complete_pairing(peer, device).await {
            Ok(()) => conn.close(VarInt::from_u32(CLOSE_PAIRED), b"paired"),
            Err(e) => {
                tracing::warn!(error = %e, "failed to store pairing");
                conn.close(VarInt::from_u32(CLOSE_FAILED), b"storage");
            }
        },
        Err(err) => {
            tracing::info!(peer = %peer.short(), error = %err, "pairing attempt failed");
            // Let the initiator read our error before the connection goes away.
            let _ = send.finish();
            let _ = tokio::time::timeout(LINGER_TIMEOUT, conn.closed()).await;
            conn.close(VarInt::from_u32(CLOSE_FAILED), b"failed");
        }
    }
}

async fn accept_qr<'a>(
    shared: &'a Arc<Shared>,
    peer: DeviceId,
    env: Envelope,
    send: &mut SendStream,
) -> Result<(DeviceInfo, QrClaim<'a>)> {
    let request: PairRequest = env.body()?;
    let Some((secret, generation)) = shared.pairing.host_secret() else {
        send_env(send, Envelope::error(ErrorCode::Denied, "pairing mode ended")).await?;
        return Err(Error::NotPairing);
    };
    if !proto::verify_pair_proof(&secret, &peer, &shared.id, &request.proof) {
        send_env(send, Envelope::error(ErrorCode::Denied, "pairing code mismatch")).await?;
        if shared.pairing.record_failure(peer) {
            shared.emit(NodeEvent::Pairing(PairingEvent::Failed(PairingFailure::Rejected)));
        }
        return Err(Error::Denied);
    }
    let Some(claim) = shared.pairing.claim_qr(generation, peer) else {
        send_env(send, Envelope::error(ErrorCode::Busy, "another device is pairing with this code")).await?;
        return Err(Error::Denied);
    };
    shared.pairing.cancel_nearby();
    send_env(send, Envelope::new(types::PAIR_ACCEPT, &PairAccept { device: local_device(shared) })?).await?;
    Ok((request.device.sanitized(), claim))
}

async fn accept_nearby(
    shared: &Arc<Shared>,
    peer: DeviceId,
    env: Envelope,
    send: &mut SendStream,
    recv: &mut RecvStream,
    is_usb: bool,
) -> std::result::Result<Option<DeviceInfo>, (Error, bool)> {
    if !shared.pairing.is_hosting() && !is_usb {
        send_env(send, Envelope::error(ErrorCode::Denied, "pairing mode ended"))
            .await
            .map_err(|e| (e, false))?;
        return Ok(None);
    }
    let Some(_nearby) = shared.pairing.begin_nearby() else {
        send_env(send, Envelope::error(ErrorCode::Busy, "another pairing is in progress"))
            .await
            .map_err(|e| (e, false))?;
        return Ok(None);
    };
    let commit: PairCommit = env.body().map_err(|e| (Error::from(e), false))?;
    let peer_device = commit.device.sanitized();
    shared.note_discovered(peer, Some(peer_device.name.clone()));
    let n_b: [u8; NONCE_LEN] = rand::random();
    let nonce = PairNonce { device: local_device(shared), n_b: n_b.to_vec() };
    let nonce_env = Envelope::new(types::PAIR_NONCE, &nonce).map_err(|e| (Error::from(e), false))?;
    send_env(send, nonce_env).await.map_err(|e| (e, false))?;

    let reveal: PairReveal = recv_env(recv, STEP_TIMEOUT)
        .await
        .map_err(|e| (e, false))?
        .expect_body(types::PAIR_REVEAL)
        .map_err(|e| (Error::from(e), false))?;
    if !proto::verify_commitment(&commit.c, &reveal.n_a) {
        send_env(send, Envelope::error(ErrorCode::Denied, "commitment mismatch"))
            .await
            .map_err(|e| (e, false))?;
        return Err((Error::Denied, false));
    }
    // The initiator is A, we are B.
    let code = proto::sas_code(&reveal.n_a, &n_b, &peer, &shared.id);
    exchange_decisions(shared, peer, code, send, recv, Role::Host).await.map_err(|e| (e, true))?;
    Ok(Some(peer_device))
}

// ---- Code comparison ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    /// Sends its confirmation as soon as the local user confirms.
    Initiator,
    /// Replies last: only after both the local user and the initiator confirmed.
    Host,
}

/// Shows the 6-digit code and exchanges both users' decisions. Watches the
/// peer while waiting for the local user, so a decline on the other device
/// ends the comparison right away.
async fn exchange_decisions(
    shared: &Arc<Shared>,
    peer: DeviceId,
    code: String,
    send: &mut SendStream,
    recv: &mut RecvStream,
    role: Role,
) -> Result<()> {
    let decision = shared.pairing.await_decision(peer)?;
    shared.emit(NodeEvent::Pairing(PairingEvent::SasCode { peer, code }));

    let mut decision = Box::pin(tokio::time::timeout(SAS_DECISION_TIMEOUT, decision));
    // Created once: reading a frame isn't cancel-safe, so it must not be
    // restarted when the other branch wins.
    let mut remote = Box::pin(recv_env(recv, SAS_DECISION_TIMEOUT));
    let (mut local_ok, mut remote_ok) = (false, false);
    while !(local_ok && remote_ok) {
        tokio::select! {
            outcome = &mut decision, if !local_ok => {
                if !matches!(outcome, Ok(Ok(true))) {
                    shared.pairing.clear_decision();
                    send_env(send, Envelope::error(ErrorCode::Denied, "codes did not match")).await?;
                    return Err(Error::Declined);
                }
                local_ok = true;
                if role == Role::Initiator {
                    send_env(send, Envelope::empty(types::PAIR_CONFIRM)).await?;
                }
            }
            reply = &mut remote, if !remote_ok => {
                let outcome = reply.and_then(|env| Ok(env.expect(types::PAIR_CONFIRM)?));
                if let Err(err) = outcome {
                    shared.pairing.clear_decision();
                    return Err(err);
                }
                remote_ok = true;
            }
        }
    }
    if role == Role::Host {
        send_env(send, Envelope::empty(types::PAIR_CONFIRM)).await?;
    }
    Ok(())
}

// ---- Helpers ----

fn local_device(shared: &Shared) -> DeviceInfo {
    shared.local.read().unwrap_or_else(|e| e.into_inner()).device.clone()
}

async fn dial(shared: &Arc<Shared>, peer: &DeviceId) -> Result<Connection> {
    let mut addr = EndpointAddr::new(crate::public_key(peer)?);
    let usb_target = shared.usb_peer_or_pending_addr(peer);
    if let Some((usb_addr, _)) = usb_target {
        addr = addr.with_ip_addr(usb_addr);
    }
    let err = match tokio::time::timeout(DIAL_TIMEOUT, shared.endpoint.connect(addr, ALPN_PAIR)).await {
        Ok(Ok(conn)) => {
            if !shared.verify_usb_connection(&conn).await {
                conn.close(VarInt::from_u32(CLOSE_FAILED), b"usb mismatch");
                return Err(Error::Denied);
            }
            if let Some((usb_addr, true)) = usb_target
                && shared.usb_peer_addr(peer) != Some(usb_addr)
            {
                shared.cancel_pending_usb_port(usb_addr.port());
                conn.close(VarInt::from_u32(CLOSE_FAILED), b"usb unverified");
                return Err(Error::Denied);
            }
            return Ok(conn);
        }
        Ok(Err(e)) => crate::error::net(e),
        Err(_) => Error::Timeout,
    };
    if let Some((usb_addr, true)) = usb_target {
        shared.cancel_pending_usb_port(usb_addr.port());
        return Err(err);
    }
    shared.emit(NodeEvent::Pairing(PairingEvent::Failed(PairingFailure::Unreachable)));
    Err(err)
}

/// Sends one message; a peer that stops reading can't hold it past [`STEP_TIMEOUT`].
async fn send_env(send: &mut SendStream, env: Envelope) -> Result<()> {
    tokio::time::timeout(STEP_TIMEOUT, write_frame(send, &env.to_cbor()))
        .await
        .map_err(|_| Error::Timeout)??;
    Ok(())
}

async fn recv_env(recv: &mut RecvStream, timeout: Duration) -> Result<Envelope> {
    let frame = tokio::time::timeout(timeout, read_frame(recv)).await.map_err(|_| Error::Timeout)??;
    Ok(Envelope::from_cbor(&frame.ok_or(Error::Offline)?)?)
}

/// Reports a failed ceremony to the UI and passes the error through.
fn fail(shared: &Shared, err: Error) -> Error {
    let failure = match &err {
        Error::Denied => PairingFailure::Rejected,
        Error::Declined => PairingFailure::Declined,
        Error::Timeout | Error::Offline | Error::Network(_) => PairingFailure::Unreachable,
        other => PairingFailure::Other(other.to_string()),
    };
    shared.emit(NodeEvent::Pairing(PairingEvent::Failed(failure)));
    err
}

/// This device's address on the default route, the one that most likely
/// reaches the LAN, if there's a default route. Connecting a UDP socket only
/// picks a route; nothing is sent.
fn default_route_ip() -> Option<IpAddr> {
    let socket = std::net::UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    // TEST-NET-1 (RFC 5737): never local, so it takes the default route.
    socket.connect(("192.0.2.1", 9)).ok()?;
    Some(socket.local_addr().ok()?.ip()).filter(|ip| !ip.is_unspecified() && !ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(addrs: Vec<SocketAddr>) -> Vec<String> {
        addrs.iter().map(ToString::to_string).collect()
    }

    fn parse(addrs: &[&str]) -> Vec<SocketAddr> {
        addrs.iter().map(|a| a.parse().unwrap()).collect()
    }

    /// This device's own Tailscale, which proves `100.64/10` is a tailnet here.
    fn tailscale() -> Vec<IpAddr> {
        vec!["100.67.18.105".parse().unwrap()]
    }

    #[test]
    fn pairing_links_carry_the_useful_addresses() {
        let addrs = parse(&[
            "[2001:db8:1::1]:5000",
            "[fe80::1]:5000",
            "100.64.0.1:5000",
            "[2001:db8:1::2]:5000",
            "192.168.1.20:5000",
            "127.0.0.1:5000",
        ]);
        let picked = strings(link_addrs(addrs.clone(), None, &tailscale()));
        assert_eq!(picked, ["192.168.1.20:5000", "100.64.0.1:5000", "[2001:db8:1::1]:5000"]);
        let fallback = strings(link_addrs(parse(&["127.0.0.1:5000"]), None, &[]));
        assert_eq!(fallback, ["127.0.0.1:5000"]);
        assert!(link_addrs(Vec::new(), None, &[]).is_empty());
    }

    /// Without this device's own Tailscale, `100.64/10` is a carrier's CGNAT:
    /// no reserved slot, just another IPv4 address after the LAN ones.
    #[test]
    fn a_cgnat_address_gets_no_tailscale_slot() {
        let addrs =
            parse(&["100.67.18.105:61845", "192.168.1.123:61845", "10.0.0.5:61845", "172.17.0.1:61845"]);
        let picked = strings(link_addrs(addrs, None, &[]));
        assert_eq!(picked, ["192.168.1.123:61845", "10.0.0.5:61845", "172.17.0.1:61845"]);
    }

    /// Virtual adapters used to fill the slots after Wi-Fi, leaving out Tailscale.
    #[test]
    fn pairing_links_keep_the_tailnet_address_beside_virtual_adapters() {
        let addrs = parse(&[
            "172.17.144.1:61845",
            "172.21.192.1:61845",
            "100.67.18.105:61845",
            "192.168.1.123:61845",
            "[fd7a:115c:a1e0::1]:61845",
        ]);
        let picked = strings(link_addrs(addrs, None, &tailscale()));
        assert_eq!(picked, ["192.168.1.123:61845", "100.67.18.105:61845", "[fd7a:115c:a1e0::1]:61845"]);
    }

    /// Tailscale never takes the only LAN slot, however many addresses it has.
    #[test]
    fn pairing_links_always_keep_one_lan_address() {
        let addrs = parse(&[
            "100.67.18.105:61845",
            "[fd7a:115c:a1e0::1]:61845",
            "[fd7a:115c:a1e0::2]:61845",
            "10.20.30.40:61845",
        ]);
        let picked = strings(link_addrs(addrs, None, &tailscale()));
        assert_eq!(picked, ["10.20.30.40:61845", "100.67.18.105:61845", "[fd7a:115c:a1e0::1]:61845"]);
    }

    /// VMware's VMnet1 and VMnet8 are `192.168` too, beside the Wi-Fi the route leaves from.
    #[test]
    fn pairing_links_keep_the_tailnet_address_beside_vmware() {
        let addrs =
            parse(&["192.168.56.1:61845", "192.168.80.1:61845", "100.67.18.105:61845", "10.20.30.40:61845"]);
        let wifi = "10.20.30.40".parse().ok();
        let picked = strings(link_addrs(addrs, wifi, &tailscale()));
        assert_eq!(picked, ["10.20.30.40:61845", "100.67.18.105:61845", "192.168.56.1:61845"]);
    }

    /// #148: three Hyper-V adapters used to fill every slot, leaving out Wi-Fi.
    #[test]
    fn pairing_links_keep_the_lan_address_beside_virtual_adapters() {
        let addrs =
            parse(&["172.17.144.1:61845", "172.21.192.1:61845", "172.27.208.1:61845", "192.168.1.123:61845"]);
        let picked = |preferred: Option<&str>| {
            strings(link_addrs(addrs.clone(), preferred.map(|p| p.parse().unwrap()), &[]))
        };
        // Without a default route, the home range still comes first.
        assert_eq!(picked(None), ["192.168.1.123:61845", "172.17.144.1:61845", "172.21.192.1:61845"]);
        // The default route's address leads, even in 172.16/12.
        assert_eq!(
            picked(Some("172.21.192.1")),
            ["172.21.192.1:61845", "192.168.1.123:61845", "172.17.144.1:61845"]
        );
        // And a 10/8 LAN beats the adapters too.
        let mut with_ten = addrs.clone();
        with_ten.pop();
        with_ten.push("10.0.0.5:61845".parse().unwrap());
        assert_eq!(strings(link_addrs(with_ten, None, &[]))[0], "10.0.0.5:61845");
    }

    #[test]
    fn the_default_route_address_is_a_real_one() {
        // No default route (an offline test machine) is fine too.
        if let Some(ip) = default_route_ip() {
            assert!(!ip.is_loopback() && !ip.is_unspecified(), "{ip}");
        }
    }

    #[test]
    fn second_nearby_ceremony_is_refused_without_clobbering_decision() {
        let state = PairingState::default();
        let guard = state.begin_nearby().expect("first nearby starts");
        assert!(state.begin_nearby().is_none(), "concurrent nearby is refused");

        let peer_a = DeviceId([1; 32]);
        let peer_b = DeviceId([2; 32]);
        let mut rx = state.await_decision(peer_a).expect("first decision registered");
        assert!(matches!(state.await_decision(peer_b), Err(Error::Denied)));

        state.decide(true).expect("decision delivered to first ceremony");
        assert_eq!(rx.try_recv(), Ok(true));

        drop(guard);
        assert!(state.begin_nearby().is_some(), "slot released after guard drops");
    }

    fn hosting(state: &PairingState) -> u64 {
        let generation = state.next_generation();
        *lock(&state.host) = Some(HostMode {
            secret: [7; SECRET_LEN],
            expires_at: Instant::now() + PAIRING_TTL,
            failures_by_peer: std::collections::HashMap::new(),
            generation,
            claimed_by: None,
        });
        generation
    }

    #[test]
    fn one_qr_code_pairs_one_device_at_a_time() {
        let state = PairingState::default();
        let generation = hosting(&state);
        let (a, b) = (DeviceId([1; 32]), DeviceId([2; 32]));
        let claim = state.claim_qr(generation, a).expect("the first device claims it");
        assert!(state.claim_qr(generation, b).is_none(), "a second device must wait");
        drop(claim);
        assert!(state.claim_qr(generation, b).is_some(), "a failed ceremony lets the code go");
    }

    #[test]
    fn an_old_ceremony_cant_finish_on_or_end_a_newer_code() {
        let state = PairingState::default();
        let old = hosting(&state);
        let claim = state.claim_qr(old, DeviceId([1; 32])).unwrap();
        let new = hosting(&state);
        assert!(!state.host_is(claim.generation), "the old code's ceremony can't store");
        state.end_host_if(Some(old));
        assert!(state.host_is(new), "and doesn't end the new code");
        state.end_host_if(Some(new));
        assert!(!state.is_hosting());
    }

    #[test]
    fn cancel_stops_ceremonies_running_now_but_not_later_ones() {
        let state = PairingState::default();
        let running = state.ceremony();
        state.cancel_ceremonies();
        assert!(running.is_cancelled());
        assert!(!state.ceremony().is_cancelled());
    }

    #[test]
    fn failures_are_tracked_per_peer_and_cancel_nearby_dismisses_pending_sas() {
        let state = PairingState::default();
        *lock(&state.host) = Some(HostMode {
            secret: [7; SECRET_LEN],
            expires_at: Instant::now() + PAIRING_TTL,
            failures_by_peer: std::collections::HashMap::new(),
            generation: 1,
            claimed_by: None,
        });
        let peer_a = DeviceId([1; 32]);
        let peer_b = DeviceId([2; 32]);
        for _ in 0..4 {
            assert!(!state.record_failure(peer_a));
            assert!(!state.record_failure(peer_b));
        }
        assert!(state.is_hosting(), "4 failures from two different peers do not end hosting");
        assert!(state.record_failure(peer_a), "5th failure from one peer ends hosting");
        assert!(!state.is_hosting());

        let mut rx = state.await_decision(peer_a).unwrap();
        state.cancel_nearby();
        assert_eq!(rx.try_recv(), Ok(false));
    }
}
