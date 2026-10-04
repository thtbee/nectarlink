// SPDX-License-Identifier: MPL-2.0
//! The pairing ceremony (protocol §9) over the `nectarlink-pair/0` ALPN.
//!
//! Two flows:
//! - **QR**: the host (PC) shows a pairing link with a one-time secret; the
//!   joiner (phone) proves knowledge of it.
//! - **Nearby**: numeric comparison. Both screens show a 6-digit code derived
//!   from both identities and committed nonces; both users confirm.

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use iroh::{
    EndpointAddr,
    endpoint::{Connection, RecvStream, SendStream, VarInt},
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

impl PairingState {
    fn host_secret(&self) -> Option<[u8; SECRET_LEN]> {
        let mut host = self.host.lock().unwrap_or_else(|e| e.into_inner());
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
        *self.host.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Records a failed attempt; returns true if pairing mode just ended.
    fn record_failure(&self) -> bool {
        let mut host = self.host.lock().unwrap_or_else(|e| e.into_inner());
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
        let slot = self.sas_decision.lock().unwrap_or_else(|e| e.into_inner()).take();
        match slot {
            Some((_, tx)) => {
                let _ = tx.send(matches);
                Ok(())
            }
            None => Err(Error::NotPairing),
        }
    }

    fn await_decision(&self, peer: DeviceId) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        *self.sas_decision.lock().unwrap_or_else(|e| e.into_inner()) = Some((peer, tx));
        rx
    }

    fn clear_decision(&self) {
        *self.sas_decision.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// Starts host pairing mode and returns the link to show as a QR code.
pub(crate) async fn start_host(shared: &Arc<Shared>) -> Result<PairingUri> {
    let secret: [u8; SECRET_LEN] = rand::random();
    *shared.pairing.host.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(HostMode { secret, expires_at: Instant::now() + PAIRING_TTL, failures: 0 });

    // Expire quietly; tell the UI if nobody paired in time.
    let weak = Arc::downgrade(shared);
    tokio::spawn(async move {
        tokio::time::sleep(PAIRING_TTL).await;
        if let Some(shared) = weak.upgrade() {
            let still_ours = shared
                .pairing
                .host
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .is_some_and(|m| m.secret == secret);
            if still_ours {
                shared.pairing.end_host();
                shared.emit(NodeEvent::Pairing(PairingEvent::Failed(PairingFailure::Expired)));
            }
        }
    });

    let name = shared.local.read().unwrap_or_else(|e| e.into_inner()).device.name.clone();
    Ok(PairingUri { id: shared.id, secret, name, addrs: shared.direct_addrs().await })
}

pub(crate) fn cancel(shared: &Shared) {
    shared.pairing.end_host();
    shared.pairing.clear_decision();
}

/// Joins a host by scanning its pairing link (the phone side of the QR flow).
pub(crate) async fn join_qr(shared: &Arc<Shared>, uri: &PairingUri) -> Result<()> {
    shared.remember_addrs(&uri.id, &uri.addrs);
    let conn = dial(shared, &uri.id).await?;
    let result = async {
        let (mut send, mut recv) = conn.open_bi().await.map_err(crate::error::net)?;
        let device = shared.local.read().unwrap_or_else(|e| e.into_inner()).device.clone();
        let proof = proto::pair_proof(&uri.secret, &shared.id, &uri.id).to_vec();
        send_env(&mut send, Envelope::new(types::PAIR_REQUEST, &PairRequest { device, proof })?).await?;
        let accept: PairAccept = recv_env(&mut recv).await?.expect_body(types::PAIR_ACCEPT)?;
        finish_initiator(&conn, send, recv).await;
        Ok(accept.device)
    }
    .await;
    conn.close(VarInt::from_u32(0), b"done");
    match result {
        Ok(device) => shared.complete_pairing(uri.id, device).await,
        Err(err) => Err(fail(shared, err)),
    }
}

/// Starts the nearby (numeric comparison) flow with a discovered device.
/// The 6-digit code arrives as [`PairingEvent::SasCode`]; answer with
/// [`crate::Node::pairing_confirm`].
pub(crate) async fn start_nearby(shared: &Arc<Shared>, peer: DeviceId) -> Result<()> {
    let conn = dial(shared, &peer).await?;
    let result = async {
        let (mut send, mut recv) = conn.open_bi().await.map_err(crate::error::net)?;
        let device = shared.local.read().unwrap_or_else(|e| e.into_inner()).device.clone();
        let n_a: [u8; NONCE_LEN] = rand::random();

        let commit = PairCommit { device, c: proto::commitment(&n_a).to_vec() };
        send_env(&mut send, Envelope::new(types::PAIR_COMMIT, &commit)?).await?;
        let nonce: PairNonce = recv_env(&mut recv).await?.expect_body(types::PAIR_NONCE)?;
        send_env(&mut send, Envelope::new(types::PAIR_REVEAL, &PairReveal { n_a: n_a.to_vec() })?).await?;

        let code = proto::sas_code(&n_a, &nonce.n_b, &shared.id, &peer);
        compare_codes(shared, peer, code, &mut send, &mut recv).await?;
        finish_initiator(&conn, send, recv).await;
        Ok(nonce.device)
    }
    .await;
    conn.close(VarInt::from_u32(0), b"done");
    match result {
        Ok(device) => shared.complete_pairing(peer, device).await,
        Err(err) => Err(fail(shared, err)),
    }
}

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
        let first = recv_env(&mut recv).await?;
        match first.t.as_str() {
            types::PAIR_REQUEST => accept_qr(&shared, peer, first, &mut send).await,
            types::PAIR_COMMIT => {
                accept_nearby(&shared, peer, first, &mut send, &mut recv).await.map_err(|e| fail(&shared, e))
            }
            other => {
                let reply = Envelope::error(ErrorCode::Unsupported, format!("unexpected {other}"));
                send_env(&mut send, reply).await?;
                Err(Error::Protocol(format!("unexpected pairing message {other}")))
            }
        }
    }
    .await;
    // Store before closing, so the joiner's first session is accepted.
    match result {
        Ok(device) => {
            if let Err(e) = shared.complete_pairing(peer, device).await {
                tracing::warn!(error = %e, "failed to store pairing");
            }
        }
        Err(err) => tracing::info!(peer = %peer.short(), error = %err, "pairing attempt failed"),
    }
    finish_host(&conn, send).await;
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
    let device = shared.local.read().unwrap_or_else(|e| e.into_inner()).device.clone();
    send_env(send, Envelope::new(types::PAIR_ACCEPT, &PairAccept { device })?).await?;
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
    let device = shared.local.read().unwrap_or_else(|e| e.into_inner()).device.clone();
    let n_b: [u8; NONCE_LEN] = rand::random();
    send_env(send, Envelope::new(types::PAIR_NONCE, &PairNonce { device, n_b: n_b.to_vec() })?).await?;

    let reveal: PairReveal = recv_env(recv).await?.expect_body(types::PAIR_REVEAL)?;
    if !proto::verify_commitment(&commit.c, &reveal.n_a) {
        send_env(send, Envelope::error(ErrorCode::Denied, "commitment mismatch")).await?;
        return Err(Error::Denied);
    }
    // The initiator is A, we are B.
    let code = proto::sas_code(&reveal.n_a, &n_b, &peer, &shared.id);
    compare_codes(shared, peer, code, send, recv).await?;
    shared.pairing.end_host();
    Ok(commit.device)
}

/// Shows the code, waits for the local decision, exchanges confirmations.
/// Succeeds only if both users confirmed.
async fn compare_codes(
    shared: &Arc<Shared>,
    peer: DeviceId,
    code: String,
    send: &mut SendStream,
    recv: &mut RecvStream,
) -> Result<()> {
    let decision = shared.pairing.await_decision(peer);
    shared.emit(NodeEvent::Pairing(PairingEvent::SasCode { peer, code }));

    let local_ok = match tokio::time::timeout(SAS_DECISION_TIMEOUT, decision).await {
        Ok(Ok(ok)) => ok,
        Ok(Err(_)) => false, // cancelled
        Err(_) => {
            shared.pairing.clear_decision();
            false
        }
    };
    if !local_ok {
        send_env(send, Envelope::error(ErrorCode::Denied, "codes did not match")).await?;
        return Err(Error::Declined);
    }
    if let Err(err) = send_env(send, Envelope::empty(types::PAIR_CONFIRM)).await {
        // The peer may already have declined and stopped reading; prefer its reason.
        if let Ok(Ok(Some(frame))) = tokio::time::timeout(Duration::from_secs(2), read_frame(recv)).await
            && let Ok(env) = Envelope::from_cbor(&frame)
        {
            env.expect(types::PAIR_CONFIRM)?;
        }
        return Err(err);
    }

    let remote =
        tokio::time::timeout(SAS_DECISION_TIMEOUT, read_frame(recv)).await.map_err(|_| Error::Timeout)??;
    let remote = Envelope::from_cbor(&remote.ok_or(Error::Offline)?)?;
    remote.expect(types::PAIR_CONFIRM)?;
    Ok(())
}

/// How long to wait for the other side to finish the ceremony cleanly.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

/// Initiator side: lets the host read our last message and close the
/// connection. Closing first would discard data the host hasn't read yet.
async fn finish_initiator(conn: &Connection, mut send: SendStream, mut recv: RecvStream) {
    let _ = send.finish();
    let _ = recv.stop(VarInt::from_u32(0));
    let _ = tokio::time::timeout(CLOSE_TIMEOUT, conn.closed()).await;
}

/// Host side: waits until the initiator has received everything we sent,
/// then closes the connection.
async fn finish_host(conn: &Connection, mut send: SendStream) {
    let _ = send.finish();
    let _ = tokio::time::timeout(CLOSE_TIMEOUT, send.stopped()).await;
    conn.close(VarInt::from_u32(0), b"done");
}

async fn dial(shared: &Arc<Shared>, peer: &DeviceId) -> Result<Connection> {
    let addr = EndpointAddr::new(crate::public_key(peer)?);
    match tokio::time::timeout(DIAL_TIMEOUT, shared.endpoint.connect(addr, ALPN_PAIR)).await {
        Ok(Ok(conn)) => Ok(conn),
        Ok(Err(e)) => {
            shared.emit(NodeEvent::Pairing(PairingEvent::Failed(PairingFailure::Unreachable)));
            Err(crate::error::net(e))
        }
        Err(_) => {
            shared.emit(NodeEvent::Pairing(PairingEvent::Failed(PairingFailure::Unreachable)));
            Err(Error::Timeout)
        }
    }
}

async fn send_env(send: &mut SendStream, env: Envelope) -> Result<()> {
    write_frame(send, &env.to_cbor()).await?;
    Ok(())
}

async fn recv_env(recv: &mut RecvStream) -> Result<Envelope> {
    let frame = tokio::time::timeout(STEP_TIMEOUT, read_frame(recv)).await.map_err(|_| Error::Timeout)??;
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
