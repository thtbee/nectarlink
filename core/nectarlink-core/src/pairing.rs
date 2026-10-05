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
    failures: u32,
}

/// Pairing state shared by both flows. Only one ceremony runs at a time.
#[derive(Default)]
pub(crate) struct PairingState {
    host: Mutex<Option<HostMode>>,
    /// Waiting for the local user to confirm or reject the 6-digit code.
    sas_decision: Mutex<Option<(DeviceId, oneshot::Sender<bool>)>>,
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
    fn host_secret(&self) -> Option<[u8; SECRET_LEN]> {
        let mut host = lock(&self.host);
        match host.as_ref() {
            Some(mode) if mode.expires_at > Instant::now() => Some(mode.secret),
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

    fn end_host(&self) {
        *lock(&self.host) = None;
    }

    /// Records a failed attempt; returns true if pairing mode just ended.
    fn record_failure(&self) -> bool {
        let mut host = lock(&self.host);
        if let Some(mode) = host.as_mut() {
            mode.failures += 1;
            if mode.failures >= MAX_FAILED_ATTEMPTS {
                *host = None;
                return true;
            }
        }
        false
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

    fn await_decision(&self, peer: DeviceId) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        *lock(&self.sas_decision) = Some((peer, tx));
        rx
    }

    fn clear_decision(&self) {
        *lock(&self.sas_decision) = None;
    }
}

/// Starts host pairing mode and returns the link to show as a QR code.
pub(crate) async fn start_host(shared: &Arc<Shared>) -> Result<PairingUri> {
    let secret: [u8; SECRET_LEN] = rand::random();
    *lock(&shared.pairing.host) =
        Some(HostMode { secret, expires_at: Instant::now() + PAIRING_TTL, failures: 0 });

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
    Ok(PairingUri { id: shared.id, secret, name, addrs: link_addrs(shared.direct_addrs().await) })
}

/// Most addresses a pairing link carries; more only make the QR code denser.
const MAX_LINK_ADDRS: usize = 3;

/// Picks the addresses worth putting in a pairing link: private IPv4 first
/// (what a phone on the same network almost always uses), then other IPv4,
/// then at most one IPv6. Link-local IPv6 is useless without a scope ID.
/// Discovery on the network finds the device even if none of these work.
fn link_addrs(mut addrs: Vec<SocketAddr>) -> Vec<SocketAddr> {
    let rank = |a: &SocketAddr| match a.ip() {
        IpAddr::V4(v4) if v4.is_private() => 0,
        IpAddr::V4(v4) if v4.is_loopback() || v4.is_link_local() => 3,
        IpAddr::V4(_) => 1,
        IpAddr::V6(_) => 2,
    };
    addrs.retain(|a| match a.ip() {
        IpAddr::V6(v6) => !v6.is_loopback() && !v6.is_unicast_link_local(),
        IpAddr::V4(v4) => !v4.is_loopback(),
    });
    addrs.sort_by_key(rank);
    let mut picked = Vec::new();
    let mut has_v6 = false;
    for addr in addrs {
        if addr.is_ipv6() {
            if has_v6 {
                continue;
            }
            has_v6 = true;
        }
        picked.push(addr);
        if picked.len() == MAX_LINK_ADDRS {
            break;
        }
    }
    picked
}

pub(crate) fn cancel(shared: &Shared) {
    shared.pairing.end_host();
    shared.pairing.clear_decision();
}

// ---- Initiator side ----

/// Joins a host by scanning its pairing link (the phone side of the QR flow).
pub(crate) async fn join_qr(shared: &Arc<Shared>, uri: &PairingUri) -> Result<()> {
    shared.remember_addrs(&uri.id, &uri.addrs);
    let conn = dial(shared, &uri.id).await?;
    let result = async {
        let (mut send, mut recv) = conn.open_bi().await.map_err(crate::error::net)?;
        let device = local_device(shared);
        let proof = proto::pair_proof(&uri.secret, &shared.id, &uri.id).to_vec();
        send_env(&mut send, Envelope::new(types::PAIR_REQUEST, &PairRequest { device, proof })?).await?;
        let accept: PairAccept = recv_env(&mut recv, STEP_TIMEOUT).await?.expect_body(types::PAIR_ACCEPT)?;
        finish_initiator(&conn, send).await?;
        Ok(accept.device)
    }
    .await;
    settle_initiator(shared, &conn, uri.id, result).await
}

/// Starts the nearby (numeric comparison) flow with a device in pairing
/// mode. The 6-digit code arrives as [`PairingEvent::SasCode`]; answer with
/// [`crate::Node::pairing_confirm`].
pub(crate) async fn start_nearby(shared: &Arc<Shared>, peer: DeviceId) -> Result<()> {
    let conn = dial(shared, &peer).await?;
    let result = async {
        let (mut send, mut recv) = conn.open_bi().await.map_err(crate::error::net)?;
        let n_a: [u8; NONCE_LEN] = rand::random();
        let commit = PairCommit { device: local_device(shared), c: proto::commitment(&n_a).to_vec() };
        send_env(&mut send, Envelope::new(types::PAIR_COMMIT, &commit)?).await?;
        let nonce: PairNonce = recv_env(&mut recv, STEP_TIMEOUT).await?.expect_body(types::PAIR_NONCE)?;
        send_env(&mut send, Envelope::new(types::PAIR_REVEAL, &PairReveal { n_a: n_a.to_vec() })?).await?;

        // We are A (initiator), the host is B.
        let code = proto::sas_code(&n_a, &nonce.n_b, &shared.id, &peer);
        exchange_decisions(shared, peer, code, &mut send, &mut recv, Role::Initiator).await?;
        finish_initiator(&conn, send).await?;
        Ok(nonce.device)
    }
    .await;
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
            conn.close(VarInt::from_u32(CLOSE_FAILED), b"failed");
            Err(fail(shared, err))
        }
    }
}

// ---- Host side ----

/// Handles an incoming pairing connection (host side of both flows).
pub(crate) async fn accept(shared: Arc<Shared>, conn: Connection) {
    let peer = crate::device_id(&conn.remote_id());
    if !shared.pairing.is_hosting() {
        tracing::debug!(peer = %peer.short(), "refusing pairing connection: not in pairing mode");
        conn.close(VarInt::from_u32(ErrorCode::Denied.close_code()), b"not pairing");
        return;
    }
    let (mut send, mut recv) = match conn.accept_bi().await {
        Ok(streams) => streams,
        Err(e) => {
            tracing::debug!(peer = %peer.short(), error = %e, "pairing connection dropped");
            return;
        }
    };
    let result = async {
        let first = recv_env(&mut recv, STEP_TIMEOUT).await?;
        let device = match first.t.as_str() {
            types::PAIR_REQUEST => accept_qr(&shared, peer, first, &mut send).await?,
            types::PAIR_COMMIT => accept_nearby(&shared, peer, first, &mut send, &mut recv)
                .await
                .map_err(|e| fail(&shared, e))?,
            other => {
                let reply = Envelope::error(ErrorCode::Unsupported, format!("unexpected {other}"));
                send_env(&mut send, reply).await?;
                return Err(Error::Protocol(format!("unexpected pairing message {other}")));
            }
        };
        // Store only once the initiator acknowledged our last message.
        recv_env(&mut recv, STEP_TIMEOUT).await?.expect(types::PAIR_DONE)?;
        Ok(device)
    }
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

async fn accept_qr(
    shared: &Arc<Shared>,
    peer: DeviceId,
    env: Envelope,
    send: &mut SendStream,
) -> Result<DeviceInfo> {
    let request: PairRequest = env.body()?;
    let Some(secret) = shared.pairing.host_secret() else {
        send_env(send, Envelope::error(ErrorCode::Denied, "pairing mode ended")).await?;
        return Err(Error::NotPairing);
    };
    if !proto::verify_pair_proof(&secret, &peer, &shared.id, &request.proof) {
        send_env(send, Envelope::error(ErrorCode::Denied, "pairing code mismatch")).await?;
        if shared.pairing.record_failure() {
            shared.emit(NodeEvent::Pairing(PairingEvent::Failed(PairingFailure::Rejected)));
        }
        return Err(Error::Denied);
    }
    shared.pairing.end_host();
    send_env(send, Envelope::new(types::PAIR_ACCEPT, &PairAccept { device: local_device(shared) })?).await?;
    Ok(request.device)
}

async fn accept_nearby(
    shared: &Arc<Shared>,
    peer: DeviceId,
    env: Envelope,
    send: &mut SendStream,
    recv: &mut RecvStream,
) -> Result<DeviceInfo> {
    let commit: PairCommit = env.body()?;
    let n_b: [u8; NONCE_LEN] = rand::random();
    let nonce = PairNonce { device: local_device(shared), n_b: n_b.to_vec() };
    send_env(send, Envelope::new(types::PAIR_NONCE, &nonce)?).await?;

    let reveal: PairReveal = recv_env(recv, STEP_TIMEOUT).await?.expect_body(types::PAIR_REVEAL)?;
    if !proto::verify_commitment(&commit.c, &reveal.n_a) {
        send_env(send, Envelope::error(ErrorCode::Denied, "commitment mismatch")).await?;
        return Err(Error::Denied);
    }
    // The initiator is A, we are B.
    let code = proto::sas_code(&reveal.n_a, &n_b, &peer, &shared.id);
    exchange_decisions(shared, peer, code, send, recv, Role::Host).await?;
    shared.pairing.end_host();
    Ok(commit.device)
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
    let decision = shared.pairing.await_decision(peer);
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
    let addr = EndpointAddr::new(crate::public_key(peer)?);
    let err = match tokio::time::timeout(DIAL_TIMEOUT, shared.endpoint.connect(addr, ALPN_PAIR)).await {
        Ok(Ok(conn)) => return Ok(conn),
        Ok(Err(e)) => crate::error::net(e),
        Err(_) => Error::Timeout,
    };
    shared.emit(NodeEvent::Pairing(PairingEvent::Failed(PairingFailure::Unreachable)));
    Err(err)
}

async fn send_env(send: &mut SendStream, env: Envelope) -> Result<()> {
    write_frame(send, &env.to_cbor()).await?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairing_links_carry_the_useful_addresses() {
        let addrs: Vec<SocketAddr> = [
            "[2409:40d2::1]:5000",
            "[fe80::1]:5000",
            "100.64.0.1:5000",
            "[2409:40d2::2]:5000",
            "192.168.31.121:5000",
            "127.0.0.1:5000",
        ]
        .iter()
        .map(|a| a.parse().unwrap())
        .collect();
        let picked: Vec<String> = link_addrs(addrs).iter().map(ToString::to_string).collect();
        assert_eq!(picked, ["192.168.31.121:5000", "100.64.0.1:5000", "[2409:40d2::1]:5000"]);
        assert!(link_addrs(Vec::new()).is_empty());
    }
}
