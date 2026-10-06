// SPDX-License-Identifier: MPL-2.0
//! [`Node`]: the core's entry point. Owns the network endpoint, the trust
//! store, live sessions and one reconnect supervisor per paired device.

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex, RwLock, Weak},
    time::Duration,
};

use iroh::{
    Endpoint, EndpointAddr, RelayMode,
    address_lookup::{DnsAddressLookup, MemoryLookup, PkarrPublisher, PkarrResolver},
    endpoint::{Connection, QuicTransportConfig, VarInt, presets},
    endpoint_info::{EndpointInfo, UserData},
    protocol::{AcceptError, ProtocolHandler, Router},
};
use iroh_mdns_address_lookup::{DiscoveryEvent, MdnsAddressLookup};
use nectarlink_protocol::{
    ALPN_PAIR, ALPN_SESSION, DeviceId, Envelope, ErrorCode,
    messages::{
        Battery, ClipSet, DeviceInfo, HelloUpdate, MediaCommand, MediaPlayer, Notification, NotifyAction,
        NotifyKey, PowerLevel, Ring, types,
    },
    pairing::PairingUri,
};
use tokio::sync::{Notify, broadcast};
use tokio_util::sync::CancellationToken;

use crate::{
    Error, NodeConfig, Platform, Result,
    events::{ConnectionPath, DiscoveredDevice, LinkState, NodeEvent, PairedDevice, PairingEvent},
    features::{self, CapabilityMatrix, DeviceFacts, MatrixInputs},
    identity,
    notifications::{self, Feed},
    pairing::{self, PairingState},
    session::{self, CLOSE_DUPLICATE, CLOSE_NORMAL, REQUEST_TIMEOUT, Session},
    store::Store,
    transfer,
};

const DIAL_TIMEOUT: Duration = Duration::from_secs(15);
const BACKOFF_MIN: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(30);
/// How long the device with the higher ID waits before dialing (see `maintain`).
const DEFER_DIAL: Duration = Duration::from_millis(750);
const EVENT_CAPACITY: usize = 512;
/// A connection with no traffic for this long is dead. iroh sends a
/// keep-alive every 5 s, so a healthy link is never idle that long, and a
/// device that drops off the network shows as offline within seconds
/// instead of after QUIC's default of 30 s.
const IDLE_TIMEOUT: Duration = Duration::from_secs(8);
/// Flow-control windows: how much may be in flight per stream and in all.
const STREAM_WINDOW: u32 = 16 * 1024 * 1024;
const CONNECTION_WINDOW: u32 = 32 * 1024 * 1024;

/// Capabilities every build offers.
const BASE_CAPABILITIES: &[&str] = &[
    "core.ping",
    "device.battery",
    "device.ring",
    "files.transfer",
    "clip.image",
    "media.remote",
    "link.open",
];

/// This device's mutable description, sent to peers.
#[derive(Debug, Clone)]
pub(crate) struct LocalState {
    pub app_version: String,
    pub device: DeviceInfo,
    pub power: PowerLevel,
    pub extra_capabilities: Vec<String>,
    pub battery: Option<Battery>,
}

impl LocalState {
    pub fn capabilities(&self) -> Vec<String> {
        let mut caps: Vec<String> = BASE_CAPABILITIES.iter().map(|c| (*c).to_owned()).collect();
        for cap in &self.extra_capabilities {
            if !caps.contains(cap) {
                caps.push(cap.clone());
            }
        }
        caps
    }
}

struct Supervisor {
    wake: Arc<Notify>,
    cancel: CancellationToken,
}

/// State shared by the node, its sessions and background tasks.
pub(crate) struct Shared {
    pub id: DeviceId,
    pub endpoint: Endpoint,
    pub store: Store,
    pub platform: Arc<dyn Platform>,
    pub local: RwLock<LocalState>,
    pub pairing: PairingState,
    pub cancel: CancellationToken,
    events: broadcast::Sender<NodeEvent>,
    sessions: Mutex<HashMap<DeviceId, Arc<Session>>>,
    links: Mutex<HashMap<DeviceId, LinkState>>,
    supervisors: Mutex<HashMap<DeviceId, Supervisor>>,
    discovered: Mutex<HashMap<DeviceId, DiscoveredDevice>>,
    /// The last capability matrix emitted per device, to emit only changes.
    matrices: Mutex<HashMap<DeviceId, CapabilityMatrix>>,
    away_mode: bool,
    memory: MemoryLookup,
    /// This device's notifications, when it mirrors them (phones).
    pub notifications: Feed,
    /// This device's media players (docs/protocol/media.md).
    pub players: crate::media::Players,
    pub data_dir: std::path::PathBuf,
    /// Where received files go.
    pub downloads_dir: std::path::PathBuf,
    /// Running transfers, to cancel them.
    transfers: Mutex<HashMap<String, CancellationToken>>,
    /// The node's runtime, for work started from non-async callers.
    pub(crate) runtime: tokio::runtime::Handle,
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared").field("id", &self.id).finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Shared {
    pub fn emit(&self, event: NodeEvent) {
        // No subscribers is fine; events are a UI convenience.
        let _ = self.events.send(event);
    }

    /// Where partly received files wait until complete.
    pub fn incoming_dir(&self) -> std::path::PathBuf {
        self.data_dir.join("incoming")
    }

    pub fn register_transfer(&self, id: &str, cancel: CancellationToken) {
        lock(&self.transfers).insert(id.to_owned(), cancel);
    }

    pub fn unregister_transfer(&self, id: &str) {
        lock(&self.transfers).remove(id);
    }

    /// Every connected device's session.
    pub fn live_sessions(&self) -> Vec<Arc<Session>> {
        lock(&self.sessions).values().filter(|s| s.is_alive()).cloned().collect()
    }

    /// Whether a device toggle (see [`features::DEVICE_TOGGLES`]) is on for
    /// `peer`; unknown toggles are off.
    pub fn toggle_on(&self, peer: &DeviceId, toggle: &str) -> bool {
        self.store
            .toggles(peer)
            .ok()
            .and_then(|t| t.get(toggle).copied())
            .or_else(|| features::toggle_default(toggle))
            .unwrap_or(false)
    }

    /// The capabilities this device offers right now.
    pub fn local_capabilities(&self) -> Vec<String> {
        self.local.read().unwrap_or_else(|e| e.into_inner()).capabilities()
    }

    pub fn session(&self, peer: &DeviceId) -> Option<Arc<Session>> {
        lock(&self.sessions).get(peer).filter(|s| s.is_alive()).cloned()
    }

    fn set_link(&self, peer: DeviceId, link: LinkState) {
        let changed = lock(&self.links).insert(peer, link.clone()).as_ref() != Some(&link);
        if changed {
            self.emit(NodeEvent::LinkChanged { device: peer, link });
        }
    }

    pub fn link(&self, peer: &DeviceId, last_seen: Option<i64>) -> LinkState {
        lock(&self.links).get(peer).cloned().unwrap_or(LinkState::Offline { last_seen })
    }

    /// Publishes the session's current path and round-trip time.
    pub fn publish_online(&self, session: &Session) {
        self.set_link(session.peer, LinkState::Online { path: session.path(), rtt_ms: session.rtt_ms() });
    }

    /// Our direct addresses, for pairing links. Waits briefly for the
    /// endpoint to learn them right after startup.
    pub async fn direct_addrs(&self) -> Vec<SocketAddr> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            let addrs: Vec<SocketAddr> = self.endpoint.addr().ip_addrs().copied().collect();
            if !addrs.is_empty() || tokio::time::Instant::now() >= deadline {
                return addrs;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Feeds known addresses of a peer into address lookup.
    pub fn remember_addrs(&self, peer: &DeviceId, addrs: &[SocketAddr]) {
        if addrs.is_empty() {
            return;
        }
        if let Ok(key) = crate::public_key(peer) {
            self.memory.add_endpoint_info(EndpointInfo::new(key).with_ip_addrs(addrs.to_vec()));
        }
    }

    /// Computes the capability matrix for a paired device from its last known
    /// capabilities, this device's state and the user's toggles.
    pub fn capability_matrix(&self, peer: &DeviceId) -> Result<CapabilityMatrix> {
        let record = self.store.get_peer(peer)?.ok_or(Error::NotPaired)?;
        let toggles = self.store.toggles(peer)?;
        let local = {
            let local = self.local.read().unwrap_or_else(|e| e.into_inner());
            DeviceFacts::new(&local.device, local.capabilities().into_iter().collect(), local.power)
        };
        let remote = DeviceFacts::new(&record.info, record.caps, record.power);
        let paths: &[ConnectionPath] = if self.away_mode {
            &[ConnectionPath::Lan, ConnectionPath::Relay]
        } else {
            &[ConnectionPath::Lan]
        };
        Ok(features::compute(*peer, MatrixInputs { local: &local, peer: &remote, paths, toggles: &toggles }))
    }

    /// Recomputes a device's capability matrix and emits it if it changed.
    pub fn refresh_capabilities(&self, peer: &DeviceId) {
        let matrix = match self.capability_matrix(peer) {
            Ok(matrix) => matrix,
            Err(Error::NotPaired) => return,
            Err(e) => {
                tracing::warn!(peer = %peer.short(), error = %e, "failed to compute capabilities");
                return;
            }
        };
        let changed = lock(&self.matrices).insert(*peer, matrix.clone()).as_ref() != Some(&matrix);
        if changed {
            self.emit(NodeEvent::Capabilities(matrix));
        }
    }

    /// Recomputes every paired device's matrix (after a local change).
    fn refresh_all_capabilities(&self) {
        match self.store.list_peers() {
            Ok(peers) => peers.iter().for_each(|p| self.refresh_capabilities(&p.id)),
            Err(e) => tracing::warn!(error = %e, "failed to list peers"),
        }
    }

    /// Stores a newly paired device and starts keeping it connected.
    pub async fn complete_pairing(self: &Arc<Self>, peer: DeviceId, info: DeviceInfo) -> Result<()> {
        let paired_at = crate::now_unix();
        self.store.upsert_peer(&peer, &info, paired_at)?;
        tracing::info!(peer = %peer.short(), "paired");
        let device = PairedDevice { id: peer, info, paired_at, link: LinkState::Offline { last_seen: None } };
        self.emit(NodeEvent::DeviceAdded(device.clone()));
        self.emit(NodeEvent::Pairing(PairingEvent::Paired(device)));
        lock(&self.discovered).remove(&peer);
        self.start_supervisor(peer);
        Ok(())
    }

    /// Removes a paired device locally (after we or the peer unpaired).
    pub async fn forget_peer(&self, peer: &DeviceId) -> Result<()> {
        let existed = self.store.remove_peer(peer)?;
        if let Some(sup) = lock(&self.supervisors).remove(peer) {
            sup.cancel.cancel();
        }
        if let Some(session) = lock(&self.sessions).remove(peer) {
            session.close(CLOSE_NORMAL, b"unpaired");
        }
        lock(&self.links).remove(peer);
        lock(&self.matrices).remove(peer);
        if existed {
            self.emit(NodeEvent::DeviceRemoved(*peer));
        }
        Ok(())
    }

    /// Registers an established session, resolving duplicates: if both
    /// devices dialed at once, the connection opened by the device with the
    /// lower ID wins on both sides.
    fn register_session(
        self: &Arc<Self>,
        conn: Connection,
        dialer: DeviceId,
        streams: (iroh::endpoint::SendStream, iroh::endpoint::RecvStream),
        remote: nectarlink_protocol::messages::Hello,
    ) -> Option<Arc<Session>> {
        let peer = crate::device_id(&conn.remote_id());
        let mut sessions = lock(&self.sessions);
        if let Some(existing) = sessions.get(&peer).filter(|s| s.is_alive()) {
            let keep_existing = existing.dialer != dialer && existing.dialer < dialer;
            if keep_existing {
                drop(sessions);
                conn.close(VarInt::from_u32(CLOSE_DUPLICATE), b"duplicate");
                return None;
            }
            existing.close(CLOSE_DUPLICATE, b"duplicate");
        }

        let weak = Arc::downgrade(self);
        let session = Session::spawn(self, conn, dialer, streams, move |ended| {
            if let Some(shared) = weak.upgrade() {
                shared.on_session_end(&ended);
            }
        });
        sessions.insert(peer, session.clone());
        drop(sessions);

        tracing::info!(peer = %peer.short(), path = ?session.path(), "connected");
        let addrs = session.remote_ip_addrs();
        if let Err(e) = self.store.record_seen(&peer, crate::now_unix(), &addrs) {
            tracing::warn!(error = %e, "failed to record peer addresses");
        }
        self.remember_addrs(&peer, &addrs);
        if let Err(e) = self.store.update_info(&peer, &remote.device) {
            tracing::warn!(error = %e, "failed to update peer info");
        }
        let caps = features::sanitize_capabilities(remote.caps);
        if let Err(e) = self.store.update_capabilities(&peer, Some(&caps), Some(remote.power.effective())) {
            tracing::warn!(error = %e, "failed to update peer capabilities");
        }
        self.emit(NodeEvent::PeerInfoChanged { device: peer, info: remote.device });
        self.emit(NodeEvent::PeerPowerChanged { device: peer, power: remote.power.effective() });
        self.refresh_capabilities(&peer);
        self.publish_online(&session);

        // Bring the peer up to date with state it may have missed.
        let battery = self.local.read().unwrap_or_else(|e| e.into_inner()).battery.clone();
        let shared = self.clone();
        let s = session.clone();
        tokio::spawn(async move {
            if let Some(battery) = battery
                && let Ok(env) = Envelope::new(types::EVENT_BATTERY, &battery)
            {
                let _ = s.send(env).await;
            }
            shared.send_notification_snapshot(&s).await;
            shared.send_media_state(&s).await;
        });
        Some(session)
    }

    fn on_session_end(&self, ended: &Arc<Session>) {
        let removed = {
            let mut sessions = lock(&self.sessions);
            match sessions.get(&ended.peer) {
                Some(current) if Arc::ptr_eq(current, ended) => sessions.remove(&ended.peer).is_some(),
                _ => false,
            }
        };
        if removed && self.store.is_paired(&ended.peer).unwrap_or(false) {
            tracing::info!(peer = %ended.peer.short(), "disconnected");
            self.set_link(ended.peer, LinkState::Offline { last_seen: Some(crate::now_unix()) });
        }
    }

    fn start_supervisor(self: &Arc<Self>, peer: DeviceId) {
        let mut supervisors = lock(&self.supervisors);
        if supervisors.contains_key(&peer) {
            return;
        }
        let wake = Arc::new(Notify::new());
        let cancel = self.cancel.child_token();
        supervisors.insert(peer, Supervisor { wake: wake.clone(), cancel: cancel.clone() });
        tokio::spawn(maintain(Arc::downgrade(self), peer, wake, cancel));
    }

    fn wake_supervisor(&self, peer: &DeviceId) {
        if let Some(sup) = lock(&self.supervisors).get(peer) {
            sup.wake.notify_one();
        }
    }

    /// Dials a paired device and runs the handshake.
    async fn dial(self: &Arc<Self>, peer: DeviceId) -> Result<Arc<Session>> {
        let addr = EndpointAddr::new(crate::public_key(&peer)?);
        let conn = tokio::time::timeout(DIAL_TIMEOUT, self.endpoint.connect(addr, ALPN_SESSION))
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(crate::error::net)?;
        let hello = session::local_hello(self);
        let (send, recv, remote) = session::handshake_dialer(&conn, &hello).await?;
        self.register_session(conn, self.id, (send, recv), remote)
            .or_else(|| self.session(&peer))
            .ok_or(Error::Offline)
    }

    /// Sends a message to every connected device.
    async fn broadcast(&self, env: Envelope) {
        for session in self.live_sessions() {
            if let Err(e) = session.send(env.clone()).await {
                tracing::debug!(peer = %session.peer.short(), error = %e, "broadcast failed");
            }
        }
    }
}

/// Keeps one paired device connected: waits while a session is alive,
/// otherwise dials with exponential backoff. Woken early by LAN discovery.
async fn maintain(shared: Weak<Shared>, peer: DeviceId, wake: Arc<Notify>, cancel: CancellationToken) {
    let mut backoff = BACKOFF_MIN;
    loop {
        let Some(node) = shared.upgrade() else { return };
        if cancel.is_cancelled() {
            return;
        }
        if let Some(session) = node.session(&peer) {
            drop(node);
            tokio::select! {
                _ = session.cancel.cancelled() => {}
                _ = cancel.cancelled() => return,
            }
            backoff = BACKOFF_MIN;
            continue;
        }
        // When both devices come up together they'd dial each other at once and
        // one connection would be dropped. The higher ID briefly defers so the
        // lower one usually wins; it still dials in case the other can't reach it.
        if node.id > peer && backoff == BACKOFF_MIN {
            drop(node);
            tokio::select! {
                _ = tokio::time::sleep(DEFER_DIAL) => {}
                _ = cancel.cancelled() => return,
            }
            let Some(again) = shared.upgrade() else { return };
            if again.session(&peer).is_some() {
                continue;
            }
            let result = again.dial(peer).await;
            drop(again);
            if handle_dial_result(result, peer, &mut backoff) {
                continue;
            }
        } else {
            let result = node.dial(peer).await;
            drop(node);
            if handle_dial_result(result, peer, &mut backoff) {
                continue;
            }
        }
        // Up to 20% jitter so many devices don't retry in lockstep.
        let jitter = backoff.mul_f64(rand::random::<f64>() * 0.2);
        tokio::select! {
            _ = tokio::time::sleep(backoff + jitter) => {}
            _ = wake.notified() => {}
            _ = cancel.cancelled() => return,
        }
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
}

/// Returns true if the dial succeeded (and resets the backoff).
fn handle_dial_result(result: Result<Arc<Session>>, peer: DeviceId, backoff: &mut Duration) -> bool {
    match result {
        Ok(_) => {
            *backoff = BACKOFF_MIN;
            true
        }
        Err(e) => {
            tracing::debug!(peer = %peer.short(), error = %e, retry_in = ?backoff, "dial failed");
            false
        }
    }
}

/// Accepts `nectarlink/0` sessions from paired devices.
#[derive(Debug, Clone)]
struct SessionProtocol(Weak<Shared>);

impl ProtocolHandler for SessionProtocol {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let Some(shared) = self.0.upgrade() else { return Err(AcceptError::from_err(Error::Offline)) };
        let peer = crate::device_id(&conn.remote_id());
        if !shared.store.is_paired(&peer).unwrap_or(false) {
            tracing::debug!(peer = %peer.short(), "refusing session from unpaired device");
            conn.close(VarInt::from_u32(ErrorCode::Unpaired.close_code()), b"unpaired");
            return Err(AcceptError::from_err(Error::NotPaired));
        }
        let hello = session::local_hello(&shared);
        match session::handshake_listener(&conn, &hello).await {
            Ok((send, recv, remote)) => {
                shared.register_session(conn, peer, (send, recv), remote);
                Ok(())
            }
            Err(e) => {
                tracing::debug!(peer = %peer.short(), error = %e, "incoming handshake failed");
                Err(AcceptError::from_err(e))
            }
        }
    }
}

/// Accepts `nectarlink-pair/0` connections while pairing.
#[derive(Debug, Clone)]
struct PairProtocol(Weak<Shared>);

impl ProtocolHandler for PairProtocol {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let Some(shared) = self.0.upgrade() else { return Err(AcceptError::from_err(Error::Offline)) };
        pairing::accept(shared, conn).await;
        Ok(())
    }
}

/// The Nectarlink engine. One per app.
///
/// Commands are async methods; everything that changes is reported through
/// [`Node::events`]. Cheap to clone.
#[derive(Clone)]
pub struct Node {
    shared: Arc<Shared>,
    router: Arc<Router>,
}

impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Node").field("id", &self.shared.id).finish_non_exhaustive()
    }
}

impl Node {
    /// Loads or creates the device identity and trust store, starts
    /// networking, and begins reconnecting to paired devices.
    pub async fn start(config: NodeConfig, platform: Arc<dyn Platform>) -> Result<Node> {
        std::fs::create_dir_all(&config.data_dir)?;
        let protector = config.key_protector.clone().unwrap_or_else(identity::default_protector);
        let secret = identity::load_or_create(&config.data_dir, protector.as_ref())?;
        let store = Store::open(&config.data_dir)?;

        // A stable port lets paired devices reconnect to the addresses they
        // remember even where local discovery is blocked (guest Wi-Fi, some
        // routers and VPNs). Reuse the last one; take any free port if it's
        // gone, and remember that instead.
        let saved = if config.port == 0 { Ports::load(&config.data_dir) } else { None };
        let wanted =
            if config.port != 0 { Some(Ports { v4: config.port, v6: Some(config.port) }) } else { saved };
        // Right after a restart the previous process may still be letting go
        // of the ports, so try them for a moment before taking others.
        let mut bound = bind_endpoint(&config, secret.clone(), wanted).await;
        for _ in 0..PORT_RETRIES {
            if bound.is_ok() || wanted.is_none() {
                break;
            }
            tokio::time::sleep(PORT_RETRY_DELAY).await;
            bound = bind_endpoint(&config, secret.clone(), wanted).await;
        }
        let endpoint = match bound {
            Ok(endpoint) => endpoint,
            Err(e) if config.port == 0 && wanted.is_some() => {
                tracing::info!(error = %e, "saved ports unavailable; using others");
                bind_endpoint(&config, secret, None).await?
            }
            Err(e) => return Err(e),
        };
        if config.port == 0
            && let Some(bound) = Ports::of(&endpoint.bound_sockets())
            && Some(bound) != saved
        {
            bound.save(&config.data_dir);
        }
        let id = crate::device_id(&endpoint.id());

        let lookups = endpoint.address_lookup().map_err(crate::error::net)?;
        let memory = MemoryLookup::new();
        lookups.add(memory.clone());

        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        let shared = Arc::new(Shared {
            id,
            endpoint: endpoint.clone(),
            store,
            platform,
            local: RwLock::new(LocalState {
                app_version: config.app_version.clone(),
                device: config.device.clone(),
                power: config.power,
                extra_capabilities: config.capabilities.clone(),
                battery: None,
            }),
            pairing: PairingState::default(),
            cancel: CancellationToken::new(),
            events,
            sessions: Mutex::new(HashMap::new()),
            links: Mutex::new(HashMap::new()),
            supervisors: Mutex::new(HashMap::new()),
            discovered: Mutex::new(HashMap::new()),
            matrices: Mutex::new(HashMap::new()),
            away_mode: config.away_mode,
            memory,
            notifications: Feed::default(),
            players: Default::default(),
            data_dir: config.data_dir.clone(),
            downloads_dir: config.downloads_dir.clone().unwrap_or_else(|| config.data_dir.join("received")),
            transfers: Mutex::new(HashMap::new()),
            runtime: tokio::runtime::Handle::current(),
        });

        if config.lan_discovery {
            start_lan_discovery(&shared, &config.device.name)?;
        }

        let router = Router::builder(endpoint)
            .accept(ALPN_SESSION, SessionProtocol(Arc::downgrade(&shared)))
            .accept(ALPN_PAIR, PairProtocol(Arc::downgrade(&shared)))
            .spawn();

        let incoming = shared.incoming_dir();
        tokio::task::spawn_blocking(move || transfer::clean_incoming(&incoming));

        for peer in shared.store.list_peers()? {
            shared.remember_addrs(&peer.id, &peer.last_addrs);
            shared.start_supervisor(peer.id);
        }
        tracing::info!(device = %id.short(), lan = config.lan_discovery, away = config.away_mode, "node started");
        Ok(Node { shared, router: Arc::new(router) })
    }

    /// Stops all sessions and networking.
    pub async fn shutdown(&self) {
        self.shared.cancel.cancel();
        let sessions: Vec<_> = lock(&self.shared.sessions).drain().map(|(_, s)| s).collect();
        for session in sessions {
            session.close(CLOSE_NORMAL, b"shutdown");
        }
        if let Err(e) = self.router.shutdown().await {
            tracing::warn!(error = %e, "router shutdown failed");
        }
        self.shared.endpoint.close().await;
    }

    pub fn device_id(&self) -> DeviceId {
        self.shared.id
    }

    /// Subscribes to node events. Each subscriber gets its own stream.
    pub fn events(&self) -> broadcast::Receiver<NodeEvent> {
        self.shared.events.subscribe()
    }

    /// This device's direct addresses (for diagnostics and tests).
    pub async fn direct_addrs(&self) -> Vec<SocketAddr> {
        self.shared.direct_addrs().await
    }

    // ---- Pairing ----

    /// Enters pairing mode and returns the link to show as a QR code.
    pub async fn pairing_start_qr(&self) -> Result<PairingUri> {
        pairing::start_host(&self.shared).await
    }

    /// Pairs with the device whose pairing link was scanned.
    pub async fn pairing_join(&self, uri: &str) -> Result<()> {
        let uri = PairingUri::parse(uri)?;
        if uri.id == self.shared.id {
            return Err(Error::Protocol("cannot pair a device with itself".into()));
        }
        pairing::join_qr(&self.shared, &uri).await
    }

    /// Starts nearby pairing with a discovered device. The other device must
    /// be in pairing mode (showing its pairing screen). The 6-digit code
    /// arrives as [`PairingEvent::SasCode`] on both devices.
    pub async fn pairing_start_nearby(&self, peer: DeviceId) -> Result<()> {
        pairing::start_nearby(&self.shared, peer).await
    }

    /// Answers the 6-digit code comparison shown via
    /// [`PairingEvent::SasCode`].
    pub fn pairing_confirm(&self, codes_match: bool) -> Result<()> {
        self.shared.pairing.decide(codes_match)
    }

    /// Leaves pairing mode and abandons any code comparison.
    pub fn pairing_cancel(&self) {
        pairing::cancel(&self.shared);
    }

    /// Whether this device is currently showing a pairing code.
    pub fn is_pairing(&self) -> bool {
        self.shared.pairing.is_hosting()
    }

    /// Adds addresses where a device can be reached. Used for "connect by IP"
    /// when local discovery is blocked (some routers and VPNs filter mDNS).
    pub fn add_known_addrs(&self, peer: DeviceId, addrs: &[SocketAddr]) {
        self.shared.remember_addrs(&peer, addrs);
        self.shared.wake_supervisor(&peer);
    }

    /// Tells the core the network may have changed (Wi-Fi switched, resumed
    /// from sleep). Paths are re-checked and offline devices are dialed right
    /// away instead of after their backoff. Harmless when nothing changed.
    pub async fn network_changed(&self) {
        self.shared.endpoint.network_change().await;
        for supervisor in lock(&self.shared.supervisors).values() {
            supervisor.wake.notify_one();
        }
    }

    /// What a "refresh" button does: reconnects to devices that aren't
    /// connected, and brings connected ones back in sync both ways (their
    /// notifications and media here, this device's there).
    pub async fn refresh(&self) {
        self.network_changed().await;
        self.shared.refresh_all_capabilities();
        for session in self.shared.live_sessions() {
            let _ = session.send(Envelope::empty(types::NOTIFY_SYNC)).await;
            let _ = session.send(Envelope::empty(types::MEDIA_SYNC)).await;
            self.shared.send_notification_snapshot(&session).await;
            self.shared.send_media_state(&session).await;
        }
    }

    /// Devices discovered on the local network that aren't paired yet.
    pub fn discovered_devices(&self) -> Vec<DiscoveredDevice> {
        lock(&self.shared.discovered).values().cloned().collect()
    }

    // ---- Paired devices ----

    pub fn paired_devices(&self) -> Result<Vec<PairedDevice>> {
        Ok(self
            .shared
            .store
            .list_peers()?
            .into_iter()
            .map(|p| PairedDevice {
                link: self.shared.link(&p.id, p.last_seen),
                id: p.id,
                info: p.info,
                paired_at: p.paired_at,
            })
            .collect())
    }

    /// Unpairs a device, telling it if it's connected.
    pub async fn unpair(&self, peer: DeviceId) -> Result<()> {
        if !self.shared.store.is_paired(&peer)? {
            return Err(Error::NotPaired);
        }
        if let Some(session) = self.shared.session(&peer) {
            let _ = session.send(Envelope::empty(types::PAIR_REVOKE)).await;
            // Let the revoke reach the peer before the connection closes.
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        self.shared.forget_peer(&peer).await
    }

    /// Asks a device to start or stop ringing.
    pub async fn ring(&self, peer: DeviceId, on: bool) -> Result<()> {
        let reply = self.request(peer, Envelope::new(types::DEVICE_RING, &Ring { on })?).await?;
        reply.expect(types::OK)?;
        Ok(())
    }

    /// Sends a request to a connected device. If the connection is replaced
    /// mid-request (e.g. a duplicate connection was resolved), retries once
    /// on the new one, so requests must be idempotent.
    async fn request(&self, peer: DeviceId, env: Envelope) -> Result<Envelope> {
        let first = self.connected(&peer)?;
        match first.request(env.clone(), REQUEST_TIMEOUT).await {
            Err(Error::Offline) => match self.shared.session(&peer) {
                Some(next) if !Arc::ptr_eq(&next, &first) => next.request(env, REQUEST_TIMEOUT).await,
                _ => Err(Error::Offline),
            },
            other => other,
        }
    }

    fn connected(&self, peer: &DeviceId) -> Result<Arc<Session>> {
        if !self.shared.store.is_paired(peer)? {
            return Err(Error::NotPaired);
        }
        self.shared.session(peer).ok_or(Error::Offline)
    }

    // ---- Capabilities ----

    /// What works with a paired device, and how to unlock what doesn't.
    /// Changes arrive as [`NodeEvent::Capabilities`]. Before the device has
    /// connected once, its capabilities are unknown and most features show
    /// as locked.
    pub fn capabilities(&self, peer: DeviceId) -> Result<CapabilityMatrix> {
        self.shared.capability_matrix(&peer)
    }

    /// The per-device toggles (see [`features::DEVICE_TOGGLES`]) with their
    /// current values.
    pub fn device_toggles(&self, peer: DeviceId) -> Result<Vec<(&'static str, bool)>> {
        if !self.shared.store.is_paired(&peer)? {
            return Err(Error::NotPaired);
        }
        let set = self.shared.store.toggles(&peer)?;
        Ok(features::DEVICE_TOGGLES
            .iter()
            .map(|(name, default)| (*name, set.get(*name).copied().unwrap_or(*default)))
            .collect())
    }

    /// Allows or disallows something for one device, e.g. `"clipboard"`.
    pub fn set_device_toggle(&self, peer: DeviceId, toggle: &str, enabled: bool) -> Result<()> {
        if features::toggle_default(toggle).is_none() {
            return Err(Error::Unsupported);
        }
        if !self.shared.store.is_paired(&peer)? {
            return Err(Error::NotPaired);
        }
        self.shared.store.set_toggle(&peer, toggle, enabled)?;
        self.shared.refresh_capabilities(&peer);
        if toggle == notifications::TOGGLE {
            self.notifications_toggled(peer, enabled);
        }
        if toggle == crate::media::TOGGLE {
            self.shared.media_toggled(peer, enabled);
        }
        Ok(())
    }

    /// Brings both sides in line after the user allowed or stopped
    /// notifications for a device.
    fn notifications_toggled(&self, peer: DeviceId, enabled: bool) {
        let shared = &self.shared;
        if shared.notifications.is_active() {
            // This phone: send the PC everything, or clear it.
            if let Some(session) = shared.session(&peer) {
                let task = shared.clone();
                shared.runtime.spawn(async move { task.send_notification_snapshot(&session).await });
            }
        } else if enabled {
            // This PC: ask the phone for what it shows now.
            if let Some(session) = shared.session(&peer) {
                shared.runtime.spawn(async move {
                    let _ = session.send(Envelope::empty(types::NOTIFY_SYNC)).await;
                });
            }
        } else {
            shared.emit(NodeEvent::NotificationsReset { device: peer, items: Vec::new() });
        }
    }

    // ---- Files (docs/protocol/files.md) ----

    /// Sends files to a paired device; returns the transfer's ID. Progress
    /// and the outcome arrive as [`NodeEvent::Transfer`]; the transfer
    /// waits for the device to connect and resumes after interruptions.
    pub async fn send_files(&self, peer: DeviceId, files: Vec<transfer::OutgoingFile>) -> Result<String> {
        transfer::send(&self.shared, peer, files).await
    }

    /// Cancels a transfer in either direction. Unknown IDs are ignored.
    pub fn cancel_transfer(&self, id: &str) {
        if let Some(cancel) = lock(&self.shared.transfers).get(id) {
            cancel.cancel();
        }
    }

    // ---- Clipboard (docs/protocol/clipboard.md) ----

    /// Puts text on a paired device's clipboard. Fails with
    /// [`Error::Denied`] when the user turned the clipboard off for that
    /// device (here or there), and [`Error::TooLarge`] beyond
    /// [`crate::CLIP_MAX_BYTES`].
    pub async fn send_clipboard(&self, peer: DeviceId, text: String) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        if text.len() > nectarlink_protocol::messages::CLIP_MAX_BYTES {
            return Err(Error::TooLarge);
        }
        if !self.shared.toggle_on(&peer, crate::clipboard::TOGGLE) {
            return Err(Error::Denied);
        }
        let env = Envelope::new(types::CLIP_SET, &ClipSet { text })?;
        self.request(peer, env).await?.expect(types::OK)?;
        Ok(())
    }

    /// Puts an image on a paired device's clipboard (`image/png` or
    /// `image/jpeg`), when the user copied one. [`Error::Unsupported`] when
    /// the device's app can't take images, [`Error::Denied`] when the user
    /// turned the clipboard off for it, and [`Error::TooLarge`] beyond
    /// [`crate::CLIP_MAX_IMAGE_BYTES`].
    pub async fn send_clipboard_image(&self, peer: DeviceId, mime: String, bytes: Vec<u8>) -> Result<()> {
        let session = self.connected(&peer)?;
        crate::clipboard::send_image(&self.shared, &session, mime, bytes).await
    }

    // ---- Actions (docs/protocol/actions.md) ----

    /// Locks or sleeps a paired PC.
    pub async fn pc_power(&self, peer: DeviceId, action: crate::PowerAction) -> Result<()> {
        let session = self.connected(&peer)?;
        crate::actions::pc_power(&self.shared, &session, action).await
    }

    /// Opens a web link on a paired device.
    pub async fn open_link(&self, peer: DeviceId, url: String) -> Result<()> {
        let session = self.connected(&peer)?;
        crate::actions::open_link(&self.shared, &session, url).await
    }

    // ---- Media (docs/protocol/media.md) ----

    /// This device's media players changed (most relevant first); sent to
    /// every connected device the user allows. Include each player's
    /// artwork every time: the core sends it once per device and session.
    pub async fn media_changed(&self, players: Vec<MediaPlayer>) {
        self.shared.media_changed(players).await;
    }

    /// Runs a command on a paired device's media player.
    pub async fn media_command(
        &self,
        peer: DeviceId,
        player: String,
        action: crate::MediaAction,
        position: Option<u64>,
    ) -> Result<()> {
        if !self.shared.toggle_on(&peer, crate::media::TOGGLE) {
            return Err(Error::Denied);
        }
        let command = MediaCommand { player, action: action.as_str().into(), position };
        let env = Envelope::new(types::MEDIA_COMMAND, &command)?;
        self.request(peer, env).await?.expect(types::OK)?;
        Ok(())
    }

    // ---- Notifications (docs/protocol/notifications.md) ----

    /// A notification appeared or changed on this phone; sent to every
    /// connected PC the user allows. Include the app icon (PNG) each time:
    /// the core sends it once per PC and session.
    pub async fn notification_posted(&self, notification: Notification) {
        self.shared.notification_posted(notification).await;
    }

    /// A notification went away on this phone.
    pub async fn notification_removed(&self, key: String) {
        self.shared.notification_removed(key).await;
    }

    /// Everything this phone shows now (when the notification listener
    /// connects, or an empty list when access was revoked).
    pub async fn notifications_reset(&self, items: Vec<Notification>) {
        self.shared.notifications_reset(items).await;
    }

    /// Dismisses a phone's notification there.
    pub async fn dismiss_notification(&self, peer: DeviceId, key: String) -> Result<()> {
        let env = Envelope::new(types::NOTIFY_DISMISS, &NotifyKey { key })?;
        self.request(peer, env).await?.expect(types::OK)?;
        Ok(())
    }

    /// Runs an action of a phone's notification; `reply` is the text for a
    /// reply action.
    pub async fn run_notification_action(
        &self,
        peer: DeviceId,
        key: String,
        action: String,
        reply: Option<String>,
    ) -> Result<()> {
        let env = Envelope::new(types::NOTIFY_ACTION, &NotifyAction { key, action, reply })?;
        self.request(peer, env).await?.expect(types::OK)?;
        Ok(())
    }

    // ---- Local state reported by the app ----

    /// Reports this device's battery; forwarded to connected devices.
    pub async fn update_battery(&self, battery: Battery) {
        self.shared.local.write().unwrap_or_else(|e| e.into_inner()).battery = Some(battery.clone());
        if let Ok(env) = Envelope::new(types::EVENT_BATTERY, &battery) {
            self.shared.broadcast(env).await;
        }
    }

    /// Updates this device's description (e.g. after a rename).
    pub async fn update_device_info(&self, device: DeviceInfo) {
        self.shared.local.write().unwrap_or_else(|e| e.into_inner()).device = device.clone();
        let update = HelloUpdate { device: Some(device), ..Default::default() };
        if let Ok(env) = Envelope::new(types::HELLO_UPDATE, &update) {
            self.shared.broadcast(env).await;
        }
        self.shared.refresh_all_capabilities();
    }

    /// Updates this device's power level and the capabilities it offers
    /// beyond the built-in ones (they depend on permissions and add-ons).
    pub async fn update_power(&self, power: PowerLevel, extra_capabilities: Vec<String>) {
        let caps = {
            let mut local = self.shared.local.write().unwrap_or_else(|e| e.into_inner());
            local.power = power;
            local.extra_capabilities = extra_capabilities;
            local.capabilities()
        };
        let update = HelloUpdate { power: Some(power), caps: Some(caps), device: None };
        if let Ok(env) = Envelope::new(types::HELLO_UPDATE, &update) {
            self.shared.broadcast(env).await;
        }
        self.shared.refresh_all_capabilities();
    }
}

const PORT_FILE: &str = "port";
/// Retrying the saved ports: 5 × 200 ms.
const PORT_RETRIES: usize = 5;
const PORT_RETRY_DELAY: Duration = Duration::from_millis(200);

/// The UDP ports the endpoint listens on, remembered across restarts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Ports {
    v4: u16,
    /// None where the machine has no IPv6.
    v6: Option<u16>,
}

impl Ports {
    /// Reads "<v4> [<v6>]".
    fn load(dir: &std::path::Path) -> Option<Ports> {
        let text = std::fs::read_to_string(dir.join(PORT_FILE)).ok()?;
        let mut parts = text.split_whitespace().map(str::parse::<u16>);
        let v4 = parts.next()?.ok().filter(|p| *p != 0)?;
        let v6 = parts.next().and_then(Result::ok).filter(|p| *p != 0);
        Some(Ports { v4, v6 })
    }

    fn of(bound: &[SocketAddr]) -> Option<Ports> {
        let v4 = bound.iter().find(|a| a.is_ipv4())?.port();
        let v6 = bound.iter().find(|a| a.is_ipv6()).map(SocketAddr::port);
        Some(Ports { v4, v6 })
    }

    fn save(self, dir: &std::path::Path) {
        let text = match self.v6 {
            Some(v6) => format!("{} {v6}", self.v4),
            None => self.v4.to_string(),
        };
        if let Err(e) = std::fs::write(dir.join(PORT_FILE), text) {
            tracing::warn!(error = %e, "can't remember the ports");
        }
    }
}

/// Binds the endpoint on `ports`, or any free ports.
async fn bind_endpoint(
    config: &NodeConfig,
    secret: iroh::SecretKey,
    ports: Option<Ports>,
) -> Result<Endpoint> {
    let transport = QuicTransportConfig::builder()
        .max_idle_timeout(Some(IDLE_TIMEOUT.try_into().expect("the idle timeout fits a QUIC varint")))
        // Room for a file transfer to keep the link busy through Wi-Fi's
        // latency spikes (the defaults suit ~100 Mbit/s at 100 ms; a phone
        // on Wi-Fi 6 does several times that, with bursts of delay).
        .stream_receive_window(VarInt::from_u32(STREAM_WINDOW))
        .receive_window(VarInt::from_u32(CONNECTION_WINDOW))
        .send_window(u64::from(CONNECTION_WINDOW))
        .build();
    let mut builder = Endpoint::builder(presets::Minimal)
        .secret_key(secret)
        .alpns(vec![ALPN_SESSION.to_vec(), ALPN_PAIR.to_vec()])
        .transport_config(transport);
    builder = if config.away_mode {
        builder
            .relay_mode(RelayMode::Default)
            .address_lookup(PkarrPublisher::n0_dns())
            .address_lookup(PkarrResolver::n0_dns())
            .address_lookup(DnsAddressLookup::n0_dns())
    } else {
        // LAN only: no relays, nothing published outside the network.
        builder.relay_mode(RelayMode::Disabled)
    };
    if let Some(ports) = ports {
        builder = builder
            .clear_ip_transports()
            .bind_addr(SocketAddr::from(([0, 0, 0, 0], ports.v4)))
            .map_err(crate::error::net)?;
        if let Some(v6) = ports.v6 {
            builder = builder.bind_addr(SocketAddr::from(([0u16; 8], v6))).map_err(crate::error::net)?;
        }
    }
    builder.bind().await.map_err(crate::error::net)
}

/// Announces this device on the LAN and watches for others.
fn start_lan_discovery(shared: &Arc<Shared>, name: &str) -> Result<()> {
    let mdns = MdnsAddressLookup::builder()
        .service_name("nectarlink")
        .build(shared.endpoint.id())
        .map_err(crate::error::net)?;
    shared.endpoint.address_lookup().map_err(crate::error::net)?.add(mdns.clone());
    // Announce our name so nearby pairing can show it; truncate safely.
    let mut announced = name.to_owned();
    while announced.len() > UserData::MAX_LENGTH {
        announced.pop();
    }
    if let Ok(data) = UserData::try_from(announced) {
        shared.endpoint.set_user_data_for_address_lookup(Some(data));
    }

    let weak = Arc::downgrade(shared);
    let cancel = shared.cancel.child_token();
    tokio::spawn(async move {
        use n0_future::StreamExt;
        let mut events = mdns.subscribe().await;
        loop {
            let event = tokio::select! {
                _ = cancel.cancelled() => return,
                event = events.next() => match event { Some(e) => e, None => return },
            };
            let Some(shared) = weak.upgrade() else { return };
            match event {
                DiscoveryEvent::Discovered { endpoint_info, .. } => {
                    let id = crate::device_id(&endpoint_info.endpoint_id);
                    if shared.store.is_paired(&id).unwrap_or(false) {
                        shared.wake_supervisor(&id);
                        continue;
                    }
                    let device =
                        DiscoveredDevice { id, name: endpoint_info.user_data().map(|d| d.to_string()) };
                    let is_new =
                        lock(&shared.discovered).insert(id, device.clone()).as_ref() != Some(&device);
                    if is_new {
                        shared.emit(NodeEvent::Discovered(device));
                    }
                }
                DiscoveryEvent::Expired { endpoint_id } => {
                    let id = crate::device_id(&endpoint_id);
                    if lock(&shared.discovered).remove(&id).is_some() {
                        shared.emit(NodeEvent::DiscoveryExpired(id));
                    }
                }
                _ => {}
            }
        }
    });
    Ok(())
}
