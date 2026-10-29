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

#[path = "wake.rs"]
pub(crate) mod wake;

pub(crate) const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const HEARTBEAT_INTERVAL: Duration = Duration::from_millis(2500);
const HEARTBEAT_TIMEOUT: Duration = Duration::from_millis(3000);
const HEARTBEAT_RETRY_TIMEOUT: Duration = Duration::from_millis(1500);
const OUTBOX_CAPACITY: usize = 256;

/// QUIC close code used when a duplicate connection is dropped.
pub(crate) const CLOSE_DUPLICATE: u32 = 100;
/// QUIC close code for a normal shutdown.
pub(crate) const CLOSE_NORMAL: u32 = 0;

/// Returns true if `ip` is a strictly local/private LAN or loopback address
/// (excluding carrier CGNAT `100.64.0.0/10` and global internet IPv6 `2000::/3`).
pub(crate) fn is_lan_ip(ip: std::net::IpAddr) -> bool {
    match ip.to_canonical() {
        std::net::IpAddr::V4(v4) => v4.is_private() || v4.is_link_local() || v4.is_loopback(),
        std::net::IpAddr::V6(v6) => v6.is_loopback() || v6.is_unicast_link_local() || v6.is_unique_local(),
    }
}

/// Returns true if two IPv6 addresses share the same `/64` subnet prefix.
#[cfg(test)]
pub(crate) fn is_same_ipv6_subnet(a: std::net::Ipv6Addr, b: std::net::Ipv6Addr) -> bool {
    a.segments()[..4] == b.segments()[..4]
}

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
    pub(crate) started: Instant,
    last_rx_ms: AtomicU64,
    usb_bridge_ports: Arc<Mutex<std::collections::HashSet<u16>>>,
    usb_peers: Arc<Mutex<HashMap<DeviceId, (std::net::SocketAddr, CancellationToken)>>>,
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
            started: Instant::now(),
            last_rx_ms: AtomicU64::new(0),
            usb_bridge_ports: shared.usb_bridge_ports.clone(),
            usb_peers: shared.usb_peers.clone(),
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
                _ = datagrams(&s, &weak) => {}
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

    pub(crate) fn touch_rx(&self) {
        let elapsed = self.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        self.last_rx_ms.store(elapsed, Ordering::Relaxed);
    }

    pub(crate) fn rx_since(&self, since: Instant) -> bool {
        let since_ms = since
            .checked_duration_since(self.started)
            .map_or(0, |d| d.as_millis().min(u128::from(u64::MAX)) as u64);
        self.last_rx_ms.load(Ordering::Relaxed) > since_ms
    }

    fn is_usb_addr(&self, addr: &std::net::SocketAddr) -> bool {
        addr.ip().to_canonical().is_loopback()
            && self.usb_bridge_ports.lock().unwrap_or_else(|e| e.into_inner()).contains(&addr.port())
    }

    fn has_usb_tunnel(&self) -> bool {
        self.usb_peers.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&self.peer)
    }

    /// The path the connection currently uses.
    pub fn path(&self) -> ConnectionPath {
        if self.has_usb_tunnel() {
            return ConnectionPath::Usb;
        }
        let paths = self.conn.paths();
        let selected = paths.iter().find(|p| p.is_selected()).or_else(|| paths.iter().next());
        match selected {
            Some(p) if p.is_relay() => ConnectionPath::Relay,
            Some(p) if matches!(p.remote_addr(), iroh::TransportAddr::Ip(addr) if self.is_usb_addr(addr)) => {
                ConnectionPath::Usb
            }
            _ => {
                if paths.iter().any(
                    |p| matches!(p.remote_addr(), iroh::TransportAddr::Ip(addr) if self.is_usb_addr(addr)),
                ) {
                    ConnectionPath::Usb
                } else {
                    ConnectionPath::Lan
                }
            }
        }
    }

    /// The peer's direct IP addresses currently in use, for reconnecting later.
    pub fn remote_ip_addrs(&self) -> Vec<std::net::SocketAddr> {
        self.conn
            .paths()
            .iter()
            .filter_map(|p| match p.remote_addr() {
                iroh::TransportAddr::Ip(addr) if !self.is_usb_addr(addr) && !addr.ip().is_loopback() => {
                    Some(*addr)
                }
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

    /// Sends a QUIC datagram (protocol v0 §4).
    pub fn send_datagram(&self, data: bytes::Bytes) -> Result<()> {
        self.conn.send_datagram(data).map_err(crate::error::net)
    }

    /// Opens a bidirectional stream on the session connection.
    pub async fn open_bi(&self) -> Result<(SendStream, RecvStream)> {
        self.conn.open_bi().await.map_err(crate::error::net)
    }

    /// Queues a one-way message.
    pub async fn send(&self, env: Envelope) -> Result<()> {
        self.outbox.send(env).await.map_err(|_| Error::Offline)
    }

    /// Best-effort non-blocking enqueue for loss-tolerant real-time media frames
    /// (such as live PCM audio fallback).
    ///
    /// Returns `Ok(())` even when a frame is dropped because the outbox is past half
    /// capacity or full, so real-time media never fills the control stream outbox and
    /// starves heartbeats or RPCs. Do not use this for messages that must arrive.
    pub fn try_send_realtime(&self, env: Envelope) -> Result<()> {
        if self.outbox.capacity() <= OUTBOX_CAPACITY / 2 {
            return Ok(());
        }
        match self.outbox.try_send(env) {
            Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => Ok(()),
            Err(mpsc::error::TrySendError::Closed(_)) => Err(Error::Offline),
        }
    }

    /// Sends a request and waits for the reply with the matching `re`.
    pub async fn request(&self, env: Envelope, timeout: Duration) -> Result<Envelope> {
        self.request_tracked(env, timeout).await.0
    }

    /// Like [`request`](Self::request), and also says whether the request was
    /// sent. A request that went out may have run on the peer even if no
    /// reply came back, so only an unsent one is safe to send again.
    pub async fn request_tracked(&self, env: Envelope, timeout: Duration) -> (Result<Envelope>, bool) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).insert(id, tx);
        let mut sent = false;
        let result = match tokio::time::timeout(timeout, async {
            self.send(env.with_id(id)).await?;
            sent = true;
            rx.await.map_err(|_| Error::Offline)
        })
        .await
        {
            Ok(inner) => inner,
            Err(_) => Err(Error::Timeout),
        };
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
        (result, sent)
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

/// Whether `mac` is the Wake-on-LAN address of a PC paired with this device.
fn is_known_wake_mac(shared: &Shared, mac: &[u8; 6]) -> bool {
    let mac = nectarlink_protocol::messages::format_mac(mac);
    shared
        .store
        .list_peers()
        .unwrap_or_default()
        .iter()
        .any(|p| p.wake_info.as_ref().is_some_and(|w| w.macs.iter().any(|m| m.eq_ignore_ascii_case(&mac))))
}

/// The control stream goes out before media streams ([`crate::mirror`]):
/// with a few app windows filling the link, heartbeats queued behind video
/// timed out and closed a live session.
const CONTROL_PRIORITY: i32 = 8;
const _: () = assert!(
    CONTROL_PRIORITY > crate::mirror::AUDIO_PRIORITY && CONTROL_PRIORITY > crate::mirror::VIDEO_PRIORITY
);

async fn writer(mut send: SendStream, mut outbox: mpsc::Receiver<Envelope>) {
    let _ = send.set_priority(CONTROL_PRIORITY);
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
            Ok(Some(frame)) => {
                session.touch_rx();
                frame
            }
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
        if env.re.is_some() && session.complete(env.clone()) {
            continue;
        }
        let Some(shared) = shared.upgrade() else { return };
        if is_background_rpc(&env.t) {
            let session = session.clone();
            tokio::spawn(async move {
                if let Err(e) = handle(&shared, &session, env).await {
                    tracing::warn!(peer = %session.peer.short(), error = %e, "failed to handle message");
                }
            });
        } else if let Err(e) = handle(&shared, session, env).await {
            tracing::warn!(peer = %session.peer.short(), error = %e, "failed to handle message");
        }
    }
}

/// Requests that can take a while (the phone's user or apps, a whole photo
/// to send) run on a task of their own so they never stall the control
/// stream (heartbeats, and other messages). Ones whose order matters
/// (clipboard, media and call commands: pause after play, unmute after
/// mute) stay in order on the stream.
fn is_background_rpc(t: &str) -> bool {
    matches!(
        t,
        types::DEVICE_RING
            | types::NOTIFY_DISMISS
            | types::NOTIFY_ACTION
            | types::PC_POWER
            | types::LINK_OPEN
            | types::PHOTOS_ALBUMS
            | types::PHOTOS_LIST
            | types::PHOTOS_THUMBS
            | types::PHOTOS_GET
            | types::CALL_LOG
            | types::CONTACTS_LIST
            | types::SMS_THREADS
            | types::SMS_MESSAGES
            | types::SMS_SEND
            | types::SMS_PART
            | types::MIRROR_START
            | types::MIRROR_APPS
            | types::WEBCAM_START
            | types::CAMERA_CAPTURE_REQUEST
            | types::STORAGE_LIST
            | types::STORAGE_MKDIR
            | types::STORAGE_RENAME
            | types::STORAGE_DELETE
    )
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
            let battery = env.body::<Battery>()?.sanitized();
            shared.emit(NodeEvent::Battery { device: peer, battery });
        }
        types::EVENT_DEVICE => {
            let info = env.body::<DeviceInfo>()?.sanitized();
            shared.store.update_info(&peer, &info)?;
            let effective = shared.store.get_peer(&peer).ok().flatten().map(|p| p.info).unwrap_or(info);
            shared.emit(NodeEvent::PeerInfoChanged { device: peer, info: effective });
        }
        types::HELLO_UPDATE => {
            let update: HelloUpdate = env.body()?;
            if let Some(info) = update.device.map(DeviceInfo::sanitized) {
                shared.store.update_info(&peer, &info)?;
                let effective = shared.store.get_peer(&peer).ok().flatten().map(|p| p.info).unwrap_or(info);
                shared.emit(NodeEvent::PeerInfoChanged { device: peer, info: effective });
            }
            let caps = update.caps.map(crate::features::sanitize_capabilities);
            let power = update.power.map(PowerLevel::effective);
            let had_caps = caps.is_some();
            if caps.is_some() || power.is_some() {
                shared.store.update_capabilities(&peer, caps.as_ref(), power)?;
            }
            if let Some(power) = power {
                shared.emit(NodeEvent::PeerPowerChanged { device: peer, power });
            }
            shared.refresh_capabilities(&peer);
            if had_caps {
                shared.send_toggles_state(session).await;
            }
        }
        types::DEVICE_RING => {
            let ring: Ring = env.body()?;
            let platform = shared.platform.clone();
            let on = ring.on;
            let _ = tokio::task::spawn_blocking(move || {
                if on {
                    platform.start_ringing();
                } else {
                    platform.stop_ringing();
                }
            })
            .await;
            shared.emit(NodeEvent::Ring { device: peer, on: ring.on });
            session.send(Envelope::empty(types::OK).reply_to(env.id)).await?;
        }
        types::PAIR_REVOKE => {
            tracing::info!(peer = %peer.short(), "peer unpaired this device");
            shared.forget_peer(&peer).await?;
        }
        t if t.starts_with("notify.") && crate::notifications::handle(shared, session, &env).await? => {}
        t if t.starts_with("media.") && crate::media::handle(shared, session, &env).await? => {}
        t if (t.starts_with("pc.") || t.starts_with("link.") || t.starts_with("task."))
            && crate::actions::handle(shared, session, &env).await? => {}
        t if t.starts_with("clip.") && crate::clipboard::handle(shared, session, &env).await? => {}
        t if t.starts_with("photos.") && crate::photos::handle(shared, session, &env).await? => {}
        t if t.starts_with("call.") && crate::calls::handle(shared, session, &env).await? => {}
        t if t.starts_with("contacts.") && crate::contacts::handle(shared, session, &env).await? => {}
        t if t.starts_with("sms.") && crate::sms::handle(shared, session, &env).await? => {}
        t if t.starts_with("mirror.") && crate::mirror::handle(shared, session, &env).await? => {}
        t if t.starts_with("webcam.") && crate::webcam::handle(shared, session, &env).await? => {}
        t if t.starts_with("mic.") && crate::mic::handle(shared, session, &env).await? => {}
        t if t.starts_with("elevated.usb") && crate::elevated_usb::handle(shared, session, &env).await? => {}
        t if t.starts_with("camera.capture.")
            && crate::continuity_camera::handle(shared, session, &env).await? => {}
        t if t.starts_with("remote.") && crate::remote::handle(shared, session, &env).await? => {}
        t if t.starts_with("phone.toggle") && crate::toggles::handle(shared, session, &env).await? => {}
        t if (t.starts_with("deck.") || t == types::PC_AUDIO_SET)
            && crate::deck::handle(shared, session, &env).await? => {}
        t if t.starts_with("storage.") && crate::storage::handle(shared, session, &env).await? => {}
        t if (t == types::FILES_RESUME || t == types::FILES_CANCEL)
            && crate::transfer::handle_control(shared, peer, &env).await? => {}
        types::WAKE_RELAY => {
            let req = env.body::<WakeRelay>().ok();
            // Only for a PC paired with this device, by an address it gave as its own.
            let known = req.as_ref().is_some_and(|r| is_known_wake_mac(shared, &r.mac));
            if let Some(req) = req.filter(|_| known) {
                let bcast = req.broadcast.as_deref().and_then(|s| s.parse().ok());
                let _ = wake::send_magic_packet(req.mac, bcast);
            } else {
                tracing::debug!(peer = %peer.short(), "refusing to wake an address no paired PC gave");
            }
            if env.id.is_some() {
                let reply = if known {
                    Envelope::empty(types::OK)
                } else {
                    Envelope::error(ErrorCode::Denied, "not a paired PC's wake address")
                };
                session.send(reply.reply_to(env.id)).await?;
            }
        }
        types::AUDIO_PCM => {
            if let Ok(frame) = env.body::<AudioPcmFrame>() {
                shared.emit(NodeEvent::PcAudioFrame {
                    device: peer,
                    sample_rate: frame.sample_rate,
                    channels: frame.channels,
                    pcm: frame.pcm,
                });
            }
        }
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

/// Concurrent incoming data streams allowed from one peer at a time.
const MAX_INCOMING_STREAMS: usize = 32;

/// Streams the other device opens (file transfers), each handled on its own.
async fn streams(session: &Arc<Session>, shared: &Weak<Shared>) {
    let limit = Arc::new(tokio::sync::Semaphore::new(MAX_INCOMING_STREAMS));
    while let Ok((send, recv)) = session.conn.accept_bi().await {
        session.touch_rx();
        let Some(shared) = shared.upgrade() else { return };
        let Ok(permit) = limit.clone().acquire_owned().await else { return };
        let session = session.clone();
        tokio::spawn(async move {
            crate::transfer::accept_stream(shared, session, send, recv).await;
            drop(permit);
        });
    }
}

async fn datagrams(session: &Arc<Session>, shared: &Weak<Shared>) {
    loop {
        match session.conn.read_datagram().await {
            Ok(bytes) => {
                session.touch_rx();
                let Some(shared) = shared.upgrade() else { return };
                if bytes.len() > 10 && &bytes[0..4] == b"NLPA" {
                    let sample_rate = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
                    let channels = bytes[8];
                    let pcm = bytes[10..].to_vec();
                    if sample_rate > 0 && channels > 0 && !pcm.is_empty() {
                        shared.emit(NodeEvent::PcAudioFrame {
                            device: session.peer,
                            sample_rate,
                            channels,
                            pcm,
                        });
                    }
                } else {
                    crate::remote::handle_datagram(&shared, session.peer, &bytes);
                }
            }
            Err(_) => {
                // If datagrams aren't available on this connection, stay parked
                // until the session ends rather than tearing it down.
                std::future::pending::<()>().await;
            }
        }
    }
}

pub(crate) async fn probe_after_network_change(shared: &Arc<Shared>, session: &Arc<Session>) {
    if !session.is_alive() {
        return;
    }
    if !shared.is_connection_allowed(&session.conn) {
        tracing::info!(peer = %session.peer.short(), "closing non-LAN session after network change");
        session.close(CLOSE_NORMAL, b"away mode disabled");
        return;
    }
    let probe_start = Instant::now();
    let Ok(ping) = Envelope::new(types::PING, &Ping { ts: 0 }) else { return };
    if session.request(ping, Duration::from_millis(2000)).await.is_err() && !session.rx_since(probe_start) {
        tracing::info!(peer = %session.peer.short(), "closing unreachable session after network change");
        session.close(CLOSE_NORMAL, b"network lost");
    }
}

async fn heartbeat(session: &Arc<Session>, shared: &Weak<Shared>) {
    let started = Instant::now();
    let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        if let Some(shared_arc) = shared.upgrade()
            && !shared_arc.is_connection_allowed(&session.conn)
        {
            tracing::info!(peer = %session.peer.short(), "closing session on disallowed non-LAN path");
            session.close(CLOSE_NORMAL, b"away mode disabled");
            return;
        }
        let sent_at = Instant::now();
        let ts = sent_at.duration_since(started).as_millis() as u64;
        let Ok(ping) = Envelope::new(types::PING, &Ping { ts }) else { continue };
        match session.request(ping, HEARTBEAT_TIMEOUT).await {
            Ok(reply) if reply.t == types::PONG => {
                if let Some(shared_arc) = shared.upgrade() {
                    let rtt = sent_at.elapsed().as_millis().min(u32::MAX as u128) as u32;
                    let previous = session.rtt_ms.swap(rtt, Ordering::Relaxed);
                    if previous.abs_diff(rtt) >= 5 {
                        shared_arc.publish_online(session);
                    }
                }
            }
            Ok(_) => {}
            Err(_) if session.rx_since(sent_at) => {}
            Err(e) => {
                // Retry once; if still unreachable and no frames arrived, close.
                let retry_ts = started.elapsed().as_millis() as u64;
                let alive = if let Ok(retry_ping) = Envelope::new(types::PING, &Ping { ts: retry_ts }) {
                    matches!(session.request(retry_ping, HEARTBEAT_RETRY_TIMEOUT).await, Ok(r) if r.t == types::PONG)
                        || session.rx_since(sent_at)
                } else {
                    session.rx_since(sent_at)
                };
                if !alive {
                    tracing::info!(peer = %session.peer.short(), error = %e, "heartbeat timed out; closing dead session");
                    session.close(CLOSE_NORMAL, b"heartbeat timeout");
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{is_lan_ip, is_same_ipv6_subnet};
    use std::net::{IpAddr, Ipv6Addr};

    #[test]
    fn lan_ip_accepts_private_local_and_rejects_wan_and_cgnat() {
        for ok in [
            "127.0.0.1",
            "192.168.1.42",
            "10.0.0.15",
            "172.16.5.2",
            "169.254.10.20",
            "::1",
            "fe80::1234",
            "fd12:3456:789a::1",
        ] {
            let ip: IpAddr = ok.parse().unwrap();
            assert!(is_lan_ip(ip), "expected {ok} to be treated as LAN IP");
        }

        for bad in ["8.8.8.8", "100.64.0.1", "100.115.92.2", "2001:4860:4860::8888", "2401:4900:1c2a:8b00::1"]
        {
            let ip: IpAddr = bad.parse().unwrap();
            assert!(!is_lan_ip(ip), "expected {bad} to be rejected as non-LAN IP");
        }
    }

    #[test]
    fn ipv6_same_subnet_matches_dual_stack_router_prefix_and_rejects_cellular() {
        let pc_wifi: Ipv6Addr = "2001:db8:2042:8b43:894c:6df4:ca1c:7561".parse().unwrap();
        let phone_wifi: Ipv6Addr = "2001:db8:2042:8b43:a9eb:bf3f:78e8:35bf".parse().unwrap();
        let phone_lte: Ipv6Addr = "2001:db8:1c2a:9911:1122:3344:5566:7788".parse().unwrap();
        assert!(is_same_ipv6_subnet(pc_wifi, phone_wifi));
        assert!(!is_same_ipv6_subnet(pc_wifi, phone_lte));
    }
}
