// SPDX-License-Identifier: MPL-2.0
//! One authenticated session with a paired device: the hello handshake, the
//! control stream, request/response correlation and the heartbeat.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU32, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use iroh::endpoint::{Connection, RecvStream, SendStream, VarInt};
use nectarlink_protocol::{
    DeviceId, Envelope, ErrorCode, MIN_PROTOCOL_VERSION, PROTOCOL_VERSION, ProtocolError, messages::*,
    negotiate_version, read_frame, write_frame,
};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::{
    Error, Result,
    error::Side,
    events::{ConnectionPath, NodeEvent},
    node::Shared,
};

pub(crate) const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(20);
const OUTBOX_CAPACITY: usize = 256;

/// QUIC close code used when a duplicate connection is dropped.
pub(crate) const CLOSE_DUPLICATE: u32 = 100;
/// QUIC close code for a normal shutdown.
pub(crate) const CLOSE_NORMAL: u32 = 0;

/// Builds this device's hello from the shared local state.
pub(crate) fn local_hello(shared: &Shared) -> Hello {
    let local = shared.local.read().unwrap_or_else(|e| e.into_inner());
    Hello {
        proto: PROTOCOL_VERSION,
        min: MIN_PROTOCOL_VERSION,
        app: local.app_version.clone(),
        device: local.device.clone(),
        caps: local.capabilities(),
        power: local.power,
    }
}

/// Runs the dialer side of the handshake on a fresh connection.
pub(crate) async fn handshake_dialer(
    conn: &Connection,
    hello: &Hello,
) -> Result<(SendStream, RecvStream, Hello)> {
    let run = async {
        let (mut send, mut recv) = conn.open_bi().await.map_err(crate::error::net)?;
        write_frame(&mut send, &Envelope::new(types::HELLO, hello)?.to_cbor()).await?;
        let remote = read_hello(&mut recv).await?;
        check_versions(conn, &mut send, hello, &remote).await?;
        Ok((send, recv, remote))
    };
    tokio::time::timeout(HANDSHAKE_TIMEOUT, run).await.map_err(|_| Error::Timeout)?
}

/// Runs the listener side of the handshake on an accepted connection.
pub(crate) async fn handshake_listener(
    conn: &Connection,
    hello: &Hello,
) -> Result<(SendStream, RecvStream, Hello)> {
    let run = async {
        let (mut send, mut recv) = conn.accept_bi().await.map_err(crate::error::net)?;
        let remote = read_hello(&mut recv).await?;
        check_versions(conn, &mut send, hello, &remote).await?;
        write_frame(&mut send, &Envelope::new(types::HELLO, hello)?.to_cbor()).await?;
        Ok((send, recv, remote))
    };
    tokio::time::timeout(HANDSHAKE_TIMEOUT, run).await.map_err(|_| Error::Timeout)?
}

async fn read_hello(recv: &mut RecvStream) -> Result<Hello> {
    let frame = read_frame(recv).await?.ok_or(Error::Offline)?;
    Ok(Envelope::from_cbor(&frame)?.expect_body(types::HELLO)?)
}

async fn check_versions(
    conn: &Connection,
    send: &mut SendStream,
    local: &Hello,
    remote: &Hello,
) -> Result<()> {
    if negotiate_version((local.proto, local.min), (remote.proto, remote.min)).is_some() {
        return Ok(());
    }
    // Tell the peer why before closing; best effort.
    let msg = format!("protocol {} is not compatible with {}", remote.proto, local.proto);
    let _ = write_frame(send, &Envelope::error(ErrorCode::VersionTooOld, msg).to_cbor()).await;
    let side = if remote.proto < local.min { Side::Remote } else { Side::Local };
    conn.close(VarInt::from_u32(ErrorCode::VersionTooOld.close_code()), b"version");
    Err(Error::VersionTooOld { side })
}

/// A live session. Cheap to share; dropping the last handle does not close
/// the connection (call [`Session::close`]).
pub(crate) struct Session {
    pub peer: DeviceId,
    /// Who opened the connection; used to resolve duplicate connections.
    pub dialer: DeviceId,
    pub conn: Connection,
    outbox: mpsc::Sender<Envelope>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Envelope>>>,
    next_id: AtomicU64,
    rtt_ms: AtomicU32,
    /// Apps whose notification icon this peer already got.
    sent_icons: crate::notifications::SentIcons,
    /// Artwork keys sent to this device (docs/protocol/media.md).
    sent_art: crate::notifications::SentIcons,
    pub(crate) cancel: CancellationToken,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("peer", &self.peer)
            .field("dialer", &self.dialer)
            .finish_non_exhaustive()
    }
}

impl Session {
    /// Starts the session's tasks. `on_end` runs once when the session ends,
    /// for any reason.
    pub(crate) fn spawn(
        shared: &Arc<Shared>,
        conn: Connection,
        dialer: DeviceId,
        (send, recv): (SendStream, RecvStream),
        on_end: impl FnOnce(Arc<Session>) + Send + 'static,
    ) -> Arc<Session> {
        let peer = crate::device_id(&conn.remote_id());
        // Start from QUIC's estimate (from the handshake) until the first
        // heartbeat measures the round trip end to end.
        let initial_rtt = conn
            .paths()
            .iter()
            .find(|p| p.is_selected())
            .map_or(0, |p| p.rtt().as_millis().min(u128::from(u32::MAX)) as u32);
        let (outbox, outbox_rx) = mpsc::channel(OUTBOX_CAPACITY);
        let session = Arc::new(Session {
            peer,
            dialer,
            conn,
            outbox,
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            rtt_ms: AtomicU32::new(initial_rtt),
            sent_icons: Default::default(),
            sent_art: Default::default(),
            cancel: shared.cancel.child_token(),
        });

        let weak = Arc::downgrade(shared);
        let s = session.clone();
        tokio::spawn(async move {
            let cancel = s.cancel.clone();
            tokio::select! {
                _ = cancel.cancelled() => {}
                _ = writer(send, outbox_rx) => {}
                _ = reader(&s, recv, &weak) => {}
                _ = heartbeat(&s, &weak) => {}
                _ = streams(&s, &weak) => {}
                _ = s.conn.closed() => {}
            }
            cancel.cancel();
            // Fail any requests still waiting for a reply.
            s.pending.lock().unwrap_or_else(|e| e.into_inner()).clear();
            tracing::debug!(peer = %s.peer.short(), "session ended");
            on_end(s);
        });
        session
    }

    pub fn rtt_ms(&self) -> u32 {
        self.rtt_ms.load(Ordering::Relaxed)
    }

    /// The path the connection currently uses.
    pub fn path(&self) -> ConnectionPath {
        let relayed = self.conn.paths().iter().find(|p| p.is_selected()).is_some_and(|p| p.is_relay());
        if relayed { ConnectionPath::Relay } else { ConnectionPath::Lan }
    }

    /// The peer's direct IP addresses currently in use, for reconnecting later.
    pub fn remote_ip_addrs(&self) -> Vec<std::net::SocketAddr> {
        self.conn
            .paths()
            .iter()
            .filter_map(|p| match p.remote_addr() {
                iroh::TransportAddr::Ip(addr) => Some(*addr),
                _ => None,
            })
            .collect()
    }

    pub(crate) fn sent_icons(&self) -> std::sync::MutexGuard<'_, std::collections::HashSet<String>> {
        self.sent_icons.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn sent_art(&self) -> std::sync::MutexGuard<'_, std::collections::HashSet<String>> {
        self.sent_art.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn is_alive(&self) -> bool {
        !self.cancel.is_cancelled() && self.conn.close_reason().is_none()
    }

    /// Queues a one-way message.
    pub async fn send(&self, env: Envelope) -> Result<()> {
        self.outbox.send(env).await.map_err(|_| Error::Offline)
    }

    /// Sends a request and waits for the reply with the matching `re`.
    pub async fn request(&self, env: Envelope, timeout: Duration) -> Result<Envelope> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).insert(id, tx);
        let result = async {
            self.send(env.with_id(id)).await?;
            match tokio::time::timeout(timeout, rx).await {
                Ok(Ok(reply)) => Ok(reply),
                Ok(Err(_)) => Err(Error::Offline),
                Err(_) => Err(Error::Timeout),
            }
        }
        .await;
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
        result
    }

    /// Closes the connection with an application code.
    pub fn close(&self, code: u32, reason: &[u8]) {
        self.conn.close(VarInt::from_u32(code), reason);
        self.cancel.cancel();
    }

    fn complete(&self, env: Envelope) -> bool {
        let Some(re) = env.re else { return false };
        match self.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&re) {
            Some(tx) => {
                let _ = tx.send(env);
                true
            }
            None => false,
        }
    }
}

async fn writer(mut send: SendStream, mut outbox: mpsc::Receiver<Envelope>) {
    while let Some(env) = outbox.recv().await {
        if let Err(e) = write_frame(&mut send, &env.to_cbor()).await {
            tracing::debug!(error = %e, "control stream write failed");
            return;
        }
    }
    let _ = send.finish();
}

async fn reader(session: &Arc<Session>, mut recv: RecvStream, shared: &Weak<Shared>) {
    loop {
        let frame = match read_frame(&mut recv).await {
            Ok(Some(frame)) => frame,
            Ok(None) => return,
            Err(ProtocolError::FrameTooLarge(len)) => {
                tracing::warn!(peer = %session.peer.short(), len, "peer sent an oversized frame");
                session.close(ErrorCode::FrameTooLarge.close_code(), b"frame too large");
                return;
            }
            Err(e) => {
                tracing::debug!(peer = %session.peer.short(), error = %e, "control stream read failed");
                return;
            }
        };
        let env = match Envelope::from_cbor(&frame) {
            Ok(env) => env,
            Err(e) => {
                tracing::warn!(peer = %session.peer.short(), error = %e, "dropping malformed message");
                continue;
            }
        };
        if session.complete(env.clone()) {
            continue;
        }
        let Some(shared) = shared.upgrade() else { return };
        if let Err(e) = handle(&shared, session, env).await {
            tracing::warn!(peer = %session.peer.short(), error = %e, "failed to handle message");
        }
    }
}

/// Handles one incoming message that isn't a reply.
async fn handle(shared: &Arc<Shared>, session: &Arc<Session>, env: Envelope) -> Result<()> {
    let peer = session.peer;
    match env.t.as_str() {
        types::PING => {
            let ping: Ping = env.body()?;
            session.send(Envelope::new(types::PONG, &ping)?.reply_to(env.id)).await?;
        }
        types::EVENT_BATTERY => {
            let battery: Battery = env.body()?;
            shared.emit(NodeEvent::Battery { device: peer, battery });
        }
        types::EVENT_DEVICE => {
            let info: DeviceInfo = env.body()?;
            shared.store.update_info(&peer, &info)?;
            shared.emit(NodeEvent::PeerInfoChanged { device: peer, info });
        }
        types::HELLO_UPDATE => {
            let update: HelloUpdate = env.body()?;
            if let Some(info) = update.device {
                shared.store.update_info(&peer, &info)?;
                shared.emit(NodeEvent::PeerInfoChanged { device: peer, info });
            }
            let caps = update.caps.map(crate::features::sanitize_capabilities);
            let power = update.power.map(PowerLevel::effective);
            if caps.is_some() || power.is_some() {
                shared.store.update_capabilities(&peer, caps.as_ref(), power)?;
            }
            if let Some(power) = power {
                shared.emit(NodeEvent::PeerPowerChanged { device: peer, power });
            }
            shared.refresh_capabilities(&peer);
        }
        types::DEVICE_RING => {
            let ring: Ring = env.body()?;
            if ring.on {
                shared.platform.start_ringing();
            } else {
                shared.platform.stop_ringing();
            }
            shared.emit(NodeEvent::Ring { device: peer, on: ring.on });
            session.send(Envelope::empty(types::OK).reply_to(env.id)).await?;
        }
        types::PAIR_REVOKE => {
            tracing::info!(peer = %peer.short(), "peer unpaired this device");
            shared.forget_peer(&peer).await?;
        }
        t if t.starts_with("notify.") && crate::notifications::handle(shared, session, &env).await? => {}
        t if t.starts_with("media.") && crate::media::handle(shared, session, &env).await? => {}
        t if t.starts_with("clip.") && crate::clipboard::handle(shared, session, &env).await? => {}
        other => {
            if env.id.is_some() {
                let reply = Envelope::error(ErrorCode::Unsupported, format!("unknown message type {other}"));
                session.send(reply.reply_to(env.id)).await?;
            }
            // Unknown one-way messages are ignored (forward compatibility).
        }
    }
    Ok(())
}

/// Streams the other device opens (file transfers), each handled on its own.
async fn streams(session: &Arc<Session>, shared: &Weak<Shared>) {
    while let Ok((send, recv)) = session.conn.accept_bi().await {
        let Some(shared) = shared.upgrade() else { return };
        tokio::spawn(crate::transfer::accept_stream(shared, session.clone(), send, recv));
    }
}

async fn heartbeat(session: &Arc<Session>, shared: &Weak<Shared>) {
    let started = Instant::now();
    let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        let sent_at = Instant::now();
        let ts = sent_at.duration_since(started).as_millis() as u64;
        let Ok(ping) = Envelope::new(types::PING, &Ping { ts }) else { continue };
        match session.request(ping, REQUEST_TIMEOUT).await {
            Ok(reply) if reply.t == types::PONG => {
                let rtt = sent_at.elapsed().as_millis().min(u32::MAX as u128) as u32;
                let previous = session.rtt_ms.swap(rtt, Ordering::Relaxed);
                if let Some(shared) = shared.upgrade()
                    && previous.abs_diff(rtt) >= 5
                {
                    shared.publish_online(session);
                }
            }
            Ok(_) => {}
            // QUIC keep-alives and idle timeouts detect dead links; a missed
            // ping is not fatal on its own.
            Err(e) => tracing::debug!(peer = %session.peer.short(), error = %e, "heartbeat failed"),
        }
    }
}
