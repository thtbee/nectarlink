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

/// A PC's reader of a phone's mirroring: the phone, the session, video or sound.
pub(crate) type MirrorStop = (DeviceId, u32, &'static str);

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
    /// This phone's call in progress (docs/protocol/calls.md).
    pub(crate) calls: crate::calls::Current,
    /// This phone's quick settings state (docs/protocol/toggles.md).
    pub(crate) toggles: crate::toggles::Current,
    /// This PC's Wake-on-LAN adapter addresses (docs/protocol/actions.md).
    pub(crate) wake_info: crate::actions::CurrentWakeInfo,
    /// This PC's Deck layout and live state, and connected PCs' Decks (docs/protocol/deck.md).
    pub(crate) deck: crate::deck::Current,
    /// Stop signals for phone screens shown here, by phone.
    pub(crate) mirror_stops: Mutex<HashMap<MirrorStop, Arc<tokio::sync::Notify>>>,
    /// Stop signals for phone webcam streams arriving here, by phone.
    pub(crate) webcam_stops: Mutex<HashMap<DeviceId, Arc<tokio::sync::Notify>>>,
    /// Per-peer rate limiters and prompt state for remote input.
    pub(crate) remote: Mutex<crate::remote::RemoteState>,
    /// Per-peer prompt state and open folders for the storage service.
    pub(crate) storage: Mutex<crate::storage::StorageState>,
    /// Encrypted-at-rest history of the last 50 clips exchanged with peers.
    pub(crate) clipboard_history: crate::clipboard_history::ClipboardHistoryStore,
    /// Smart suggestion for the most recently received text clip, if any.
    pub(crate) last_clip_suggestion: Mutex<Option<(DeviceId, crate::ClipSuggestion)>>,
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

    pub(crate) fn peer_name(&self, peer: &DeviceId) -> String {
        self.store.get_peer(peer).ok().flatten().map(|p| p.info.name).unwrap_or_else(|| peer.short())
    }

    pub(crate) fn nudge_reconnect(&self) {
        for supervisor in lock(&self.supervisors).values() {
            supervisor.wake.notify_one();
        }
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

    /// Publishes the session's current path and round-trip time, if it's
    /// still the peer's session (a replaced one has nothing to say).
    pub fn publish_online(&self, session: &Session) {
        let sessions = lock(&self.sessions);
        if sessions.get(&session.peer).is_some_and(|current| std::ptr::eq(current.as_ref(), session)) {
            self.set_link(session.peer, LinkState::Online { path: session.path(), rtt_ms: session.rtt_ms() });
        }
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
        let info = info.sanitized();
        let paired_at = crate::now_unix();
        self.store.upsert_peer(&peer, &info, paired_at)?;
        tracing::info!(peer = %peer.short(), "paired");
        let device = PairedDevice {
            id: peer,
            info,
            paired_at,
            link: LinkState::Offline { last_seen: None },
            can_wake: false,
        };
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
        lock(&self.remote).remove_peer(peer);
        lock(&self.storage).remove_peer(peer);
        self.toggles.remove_peer(peer);
        self.deck.remove_peer(peer);
        self.stop_webcam(peer);
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
        let remote_device = remote.device.sanitized();
        if let Err(e) = self.store.update_info(&peer, &remote_device) {
            tracing::warn!(error = %e, "failed to update peer info");
        }
        let caps = features::sanitize_capabilities(remote.caps);
        if let Err(e) = self.store.update_capabilities(&peer, Some(&caps), Some(remote.power.effective())) {
            tracing::warn!(error = %e, "failed to update peer capabilities");
        }
        self.emit(NodeEvent::PeerInfoChanged { device: peer, info: remote_device });
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
            shared.send_call_state(&s).await;
            shared.send_toggles_state(&s).await;
            crate::actions::send_wake_info(&shared, &s).await;
            shared.send_deck(&s).await;
        });
        Some(session)
    }

    fn on_session_end(&self, ended: &Arc<Session>) {
        let paired = self.store.is_paired(&ended.peer).unwrap_or(false);
        let mut sessions = lock(&self.sessions);
        let current = sessions.get(&ended.peer).is_some_and(|current| Arc::ptr_eq(current, ended));
        if current {
            sessions.remove(&ended.peer);
            lock(&self.storage).remove_peer(&ended.peer);
            self.toggles.remove_peer(&ended.peer);
            self.deck.remove_peer(&ended.peer);
            if paired {
                tracing::info!(peer = %ended.peer.short(), "disconnected");
                // Still under the lock: a new session for the peer can't
                // register (and say it's online) until this is said, so a
                // stale "offline" never lands after a fresh "online".
                self.set_link(ended.peer, LinkState::Offline { last_seen: Some(crate::now_unix()) });
            }
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
        let clipboard_history =
            crate::clipboard_history::ClipboardHistoryStore::open(&config.data_dir, protector);

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
            calls: Default::default(),
            toggles: Default::default(),
            wake_info: Default::default(),
            deck: Default::default(),
            mirror_stops: Mutex::new(HashMap::new()),
            webcam_stops: Mutex::new(HashMap::new()),
            remote: Mutex::new(Default::default()),
            storage: Mutex::new(Default::default()),
            clipboard_history,
            last_clip_suggestion: Mutex::new(None),
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
        self.shared.nudge_reconnect();
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
            self.shared.send_toggles_state(&session).await;
            crate::actions::send_wake_info(&self.shared, &session).await;
            self.shared.send_deck(&session).await;
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
                can_wake: p.wake_info.as_ref().is_some_and(|w| !w.macs.is_empty()),
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
        if toggle == crate::toggles::TOGGLE {
            self.shared.toggles_toggled(peer, enabled);
        }
        if toggle == crate::actions::POWER_TOGGLE
            && let Some(session) = self.shared.session(&peer)
        {
            let shared = self.shared.clone();
            self.shared.runtime.spawn(async move {
                crate::actions::send_wake_info(&shared, &session).await;
            });
        }
        if toggle == crate::storage::TOGGLE {
            let mut st = self.shared.storage.lock().unwrap_or_else(|e| e.into_inner());
            if enabled {
                st.clear_prompted(&peer);
            } else {
                st.remove_peer(&peer);
            }
        }
        if toggle == crate::webcam::TOGGLE && !enabled {
            self.shared.stop_webcam(&peer);
            let platform = self.shared.platform.clone();
            drop(self.shared.runtime.spawn_blocking(move || platform.webcam_stop_requested(&peer)));
            if let Some(session) = self.shared.session(&peer) {
                self.shared.runtime.spawn(async move {
                    let _ = crate::webcam::stop(&session).await;
                });
            }
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

    /// Sends a voice recording and its markers to a paired PC; returns the
    /// transfer's ID. Progress and the outcome arrive as
    /// [`NodeEvent::Transfer`]; the transfer waits for the PC to connect and
    /// resumes after interruptions.
    pub async fn send_recording(
        &self,
        peer: DeviceId,
        file: transfer::OutgoingFile,
        markers: Vec<transfer::RecordingMarker>,
    ) -> Result<String> {
        transfer::send_recording(&self.shared, peer, file, markers).await
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
        let env = Envelope::new(types::CLIP_SET, &ClipSet { text: text.clone() })?;
        self.request(peer, env).await?.expect(types::OK)?;
        let peer_name = self.shared.peer_name(&peer);
        if self.shared.clipboard_history.record_text(&text, &peer_name, false) {
            self.shared.emit(NodeEvent::ClipboardHistoryChanged);
        }
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

    /// Whether the local clipboard history is enabled.
    pub fn clipboard_history_enabled(&self) -> bool {
        self.shared.clipboard_history.enabled()
    }

    /// Turns the local clipboard history on or off.
    pub fn set_clipboard_history_enabled(&self, enabled: bool) -> Result<()> {
        self.shared.clipboard_history.set_enabled(enabled)
    }

    /// Lists the local clipboard history (pinned first, then newest first),
    /// optionally filtered by `query`.
    pub fn clipboard_history(&self, query: Option<&str>) -> Vec<crate::ClipboardHistoryEntry> {
        self.shared.clipboard_history.list(query)
    }

    /// Decrypts and returns `(mime, bytes)` for an image entry in the
    /// clipboard history.
    pub fn clipboard_history_image(&self, id: &str) -> Option<(String, Vec<u8>)> {
        self.shared.clipboard_history.image_bytes(id)
    }

    /// Pins or unpins an entry in the clipboard history.
    pub fn pin_clipboard_history(&self, id: &str, pinned: bool) -> bool {
        let changed = self.shared.clipboard_history.set_pinned(id, pinned);
        if changed {
            self.shared.emit(NodeEvent::ClipboardHistoryChanged);
        }
        changed
    }

    /// Deletes one entry from the clipboard history.
    pub fn delete_clipboard_history(&self, id: &str) -> bool {
        let deleted = self.shared.clipboard_history.delete(id);
        if deleted {
            self.shared.emit(NodeEvent::ClipboardHistoryChanged);
        }
        deleted
    }

    /// Clears the entire clipboard history and deletes its encrypted files from disk.
    pub fn clear_clipboard_history(&self) -> Result<()> {
        self.shared.clipboard_history.clear()?;
        self.shared.emit(NodeEvent::ClipboardHistoryChanged);
        Ok(())
    }

    /// Copies a clipboard history entry back onto this device's OS clipboard.
    pub fn copy_clipboard_history(&self, id: &str) -> Result<()> {
        let entry = self.shared.clipboard_history.entry(id).ok_or(Error::NotFound)?;
        match entry.kind {
            crate::ClipboardItemKind::Text => {
                self.shared.platform.set_clipboard(&entry.text).map_err(|e| Error::Internal(e.to_string()))
            }
            crate::ClipboardItemKind::Image => {
                let (mime, bytes) = self.shared.clipboard_history.image_bytes(id).ok_or(Error::NotFound)?;
                self.shared
                    .platform
                    .set_clipboard_image(&mime, &bytes)
                    .map_err(|e| Error::Internal(e.to_string()))
            }
        }
    }

    /// Returns the smart suggestion classified for the most recently received
    /// text clip (`None` when the latest clip had no suggestion or was an OTP).
    pub fn last_clip_suggestion(&self) -> Option<(DeviceId, crate::ClipSuggestion)> {
        self.shared.last_clip_suggestion.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    // ---- Actions (docs/protocol/actions.md) ----

    /// Locks or sleeps a paired PC.
    pub async fn pc_power(&self, peer: DeviceId, action: crate::PowerAction) -> Result<()> {
        let session = self.connected(&peer)?;
        crate::actions::pc_power(&self.shared, &session, action).await
    }

    /// Sets this PC's Wake-on-LAN adapter addresses (`pc.wake_info`) and sends
    /// them to every connected phone when they changed.
    pub async fn set_wake_info(&self, info: nectarlink_protocol::messages::PcWakeInfo) {
        let prev = self.shared.wake_info.get();
        let clean = self.shared.wake_info.set(info);
        if prev.as_ref() != Some(&clean) {
            for session in self.shared.live_sessions() {
                crate::actions::send_wake_info(&self.shared, &session).await;
            }
        }
    }

    /// Returns the stored Wake-on-LAN addresses for a paired PC, if any.
    pub fn wake_info(&self, peer: DeviceId) -> Result<Option<nectarlink_protocol::messages::PcWakeInfo>> {
        Ok(self.shared.store.get_peer(&peer)?.ok_or(Error::NotPaired)?.wake_info)
    }

    /// Sends Wake-on-LAN magic packets for a paired PC over UDP to its stored
    /// subnet broadcasts and `255.255.255.255` on ports 9 and 7, repeated a
    /// few times over ~2 s.
    pub async fn wake(&self, peer: DeviceId) -> Result<()> {
        crate::actions::wake(
            &self.shared,
            &peer,
            nectarlink_protocol::messages::wake::DEFAULT_PORTS,
            &[],
            4,
            Duration::from_millis(650),
        )
        .await
    }

    /// Sends Wake-on-LAN magic packets for a paired PC on custom UDP ports and
    /// additional target IPv4 addresses (used by tests).
    pub async fn wake_on_ports(
        &self,
        peer: DeviceId,
        ports: &[u16],
        extra_targets: &[std::net::Ipv4Addr],
    ) -> Result<()> {
        crate::actions::wake(&self.shared, &peer, ports, extra_targets, 2, Duration::from_millis(40)).await
    }

    /// Opens a web link on a paired device.
    pub async fn open_link(&self, peer: DeviceId, url: String) -> Result<()> {
        let session = self.connected(&peer)?;
        crate::actions::open_link(&self.shared, &session, url).await
    }

    // ---- Screen mirroring (docs/protocol/mirror.md) ----

    /// Asks a paired phone for its screen. The phone asks its user; video
    /// then arrives through [`Platform::mirror_sink`](crate::Platform::mirror_sink).
    pub async fn mirror_start(&self, peer: DeviceId, options: crate::MirrorStart) -> Result<()> {
        let session = self.connected(&peer)?;
        crate::mirror::start(&self.shared, &session, options).await
    }

    /// Stops showing a phone's screen or app window (`mirroring`), and
    /// tells the phone.
    pub async fn mirror_stop(&self, peer: DeviceId, mirroring: u32) {
        self.shared.stop_showing(&peer, mirroring);
        if let Ok(session) = self.connected(&peer) {
            let _ = crate::mirror::stop(&session, mirroring).await;
        }
    }

    /// Asks a phone for a keyframe (the decoder lost its place).
    pub async fn mirror_keyframe(&self, peer: DeviceId, mirroring: u32) {
        if let Ok(session) = self.connected(&peer) {
            crate::mirror::request_keyframe(&session, mirroring).await;
        }
    }

    /// Resizes an app window's display (`mirroring` != 0) on a phone.
    pub async fn mirror_resize(&self, peer: DeviceId, mirroring: u32, width: u32, height: u32) -> Result<()> {
        let session = self.connected(&peer)?;
        crate::mirror::resize(&self.shared, &session, mirroring, width, height).await
    }

    /// The PC's mouse and keyboard on a phone's mirrored screen or app window.
    pub async fn mirror_input(
        &self,
        peer: DeviceId,
        mirroring: u32,
        input: crate::MirrorInput,
    ) -> Result<()> {
        let session = self.connected(&peer)?;
        crate::mirror::input(&self.shared, &session, mirroring, input).await
    }

    /// The apps a phone can open in windows of their own (Elevated), by name.
    pub async fn mirror_apps(&self, peer: DeviceId) -> Result<Vec<crate::PhoneApp>> {
        let session = self.connected(&peer)?;
        crate::mirror::apps(&self.shared, &session).await
    }

    /// Opens this phone's video stream to a PC that asked for the screen.
    pub async fn mirror_open(&self, peer: DeviceId) -> Result<crate::MirrorStream> {
        let session = self.connected(&peer)?;
        crate::mirror::open(&self.shared, &session, false).await
    }

    /// Opens this phone's sound stream to a PC that asked for the screen
    /// with its sound.
    pub async fn mirror_open_audio(&self, peer: DeviceId) -> Result<crate::MirrorStream> {
        let session = self.connected(&peer)?;
        crate::mirror::open(&self.shared, &session, true).await
    }

    // ---- Webcam (docs/protocol/webcam.md) ----

    /// Asks a paired phone to start streaming its camera as a webcam.
    /// Video then arrives through [`Platform::webcam_sink`](crate::Platform::webcam_sink).
    pub async fn webcam_start(&self, peer: DeviceId, options: crate::WebcamStart) -> Result<()> {
        let session = self.connected(&peer)?;
        crate::webcam::start(&self.shared, &session, options).await
    }

    /// Stops a webcam stream with `peer` (either direction) and tells `peer`.
    pub async fn webcam_stop(&self, peer: DeviceId) {
        self.shared.stop_webcam(&peer);
        if let Ok(session) = self.connected(&peer) {
            let _ = crate::webcam::stop(&session).await;
        }
    }

    /// Asks a phone for a fresh H.264 keyframe on the webcam stream.
    pub async fn webcam_keyframe(&self, peer: DeviceId) {
        if let Ok(session) = self.connected(&peer) {
            crate::webcam::request_keyframe(&session).await;
        }
    }

    /// Opens this phone's camera stream to a paired PC.
    pub async fn webcam_open(&self, peer: DeviceId) -> Result<crate::MirrorStream> {
        let session = self.connected(&peer)?;
        crate::webcam::open(&self.shared, &session).await
    }

    // ---- Messages (docs/protocol/sms.md) ----

    /// This phone's messages changed (in `thread`, or anywhere): connected
    /// PCs that show them catch up.
    pub async fn sms_changed(&self, thread: Option<String>) {
        self.shared.sms_changed(thread).await;
    }

    /// A paired phone's latest conversations, newest first.
    pub async fn sms_threads(&self, peer: DeviceId, limit: u32) -> Result<Vec<crate::SmsThread>> {
        let session = self.connected(&peer)?;
        crate::sms::threads(&self.shared, &session, limit).await
    }

    /// A conversation's messages before `before` (Unix ms; the latest when
    /// `None`), newest first.
    pub async fn sms_messages(
        &self,
        peer: DeviceId,
        thread: String,
        before: Option<i64>,
        limit: u32,
    ) -> Result<Vec<crate::SmsMessage>> {
        let session = self.connected(&peer)?;
        crate::sms::messages(&self.shared, &session, thread, before, limit).await
    }

    /// Sends a text through a paired phone.
    pub async fn sms_send(&self, peer: DeviceId, to: Vec<String>, body: String) -> Result<()> {
        let session = self.connected(&peer)?;
        crate::sms::send(&self.shared, &session, to, body).await
    }

    /// A picture in a message: its type and bytes.
    pub async fn sms_part(&self, peer: DeviceId, id: String) -> Result<(String, Vec<u8>)> {
        let session = self.connected(&peer)?;
        crate::sms::part(&self.shared, &session, id).await
    }

    // ---- Calls (docs/protocol/calls.md) ----

    /// A call on this phone rang, was answered or ended; sent to every
    /// connected PC the user allows (and to PCs that connect during it).
    pub async fn call_changed(&self, call: crate::CallState) -> Result<()> {
        self.shared.call_changed(call).await
    }

    /// Answers, declines or silences a call on a paired phone.
    pub async fn call_command(&self, peer: DeviceId, id: String, command: crate::CallCommand) -> Result<()> {
        let session = self.connected(&peer)?;
        crate::calls::command(&self.shared, &session, id, command).await
    }

    /// This phone's call history changed: connected PCs that show it catch up.
    pub async fn call_log_changed(&self) {
        self.shared.call_log_changed().await;
    }

    /// A paired phone's recent calls before `before` (Unix ms; the latest
    /// when `None`), newest first.
    pub async fn call_log(
        &self,
        peer: DeviceId,
        before: Option<i64>,
        limit: u32,
    ) -> Result<Vec<crate::CallLogEntry>> {
        let session = self.connected(&peer)?;
        crate::calls::log(&self.shared, &session, before, limit).await
    }

    /// Asks a paired phone to place a call to `number` (or open the dialer
    /// with it filled in).
    pub async fn call_dial(&self, peer: DeviceId, number: String) -> Result<()> {
        let session = self.connected(&peer)?;
        crate::calls::dial(&self.shared, &session, number).await
    }

    // ---- Contacts (docs/protocol/contacts.md) ----

    /// This phone's contacts changed: connected PCs that show them catch up.
    pub async fn contacts_changed(&self) {
        self.shared.contacts_changed().await;
    }

    /// Lists or searches a paired phone's contacts (favorites first, then
    /// alphabetical), skipping `offset`.
    pub async fn contacts(
        &self,
        peer: DeviceId,
        query: Option<String>,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<crate::Contact>> {
        let session = self.connected(&peer)?;
        crate::contacts::list(&self.shared, &session, query, offset, limit).await
    }

    // ---- Photos (docs/protocol/photos.md) ----

    /// A photo or screenshot just appeared on this phone; announced to every
    /// connected PC the user allows. PCs then ask for it through
    /// [`Platform::open_photo`](crate::Platform::open_photo).
    pub async fn photo_taken(&self, photo: crate::Photo) -> Result<()> {
        self.shared.photo_taken(photo).await
    }

    /// This phone's photo or video library changed: connected PCs that show
    /// photos catch up.
    pub async fn photos_changed(&self) {
        self.shared.photos_changed().await;
    }

    /// Lists a paired phone's photo and video albums.
    pub async fn photo_albums(&self, peer: DeviceId) -> Result<Vec<crate::PhotoAlbum>> {
        let session = self.connected(&peer)?;
        crate::photos::albums(&self.shared, &session).await
    }

    /// Lists a paired phone's photos and videos (in `album`, or all when
    /// `None`), newest first: from the latest, or after `before`, the
    /// previous page's last item (its date and ID).
    pub async fn photo_list(
        &self,
        peer: DeviceId,
        album: Option<String>,
        before: Option<(i64, String)>,
        limit: u32,
    ) -> Result<Vec<crate::PhotoItem>> {
        let session = self.connected(&peer)?;
        crate::photos::list(&self.shared, &session, album, before, limit).await
    }

    /// Fetches small JPEG thumbnails for a batch of item IDs on a paired phone.
    pub async fn photo_thumbs(&self, peer: DeviceId, ids: Vec<String>) -> Result<Vec<crate::PhotoThumb>> {
        let session = self.connected(&peer)?;
        crate::photos::thumbs(&self.shared, &session, ids).await
    }

    /// Asks a phone for a photo or video; returns the ID of the transfer
    /// that brings it (reported like any other).
    pub async fn fetch_photo(&self, peer: DeviceId, id: String) -> Result<String> {
        let session = self.connected(&peer)?;
        crate::photos::fetch(&self.shared, &session, id).await
    }

    /// Asks a phone for one or more photos or videos in a single transfer;
    /// returns the transfer ID.
    pub async fn fetch_photos(&self, peer: DeviceId, ids: Vec<String>) -> Result<String> {
        let session = self.connected(&peer)?;
        crate::photos::fetch_many(&self.shared, &session, ids).await
    }

    // ---- Remote input (docs/protocol/remote.md) ----

    /// Checks whether a paired PC currently accepts remote input from this
    /// phone. If `remote_input` is off on the PC, returns [`Error::Denied`]
    /// and triggers the PC's one-time prompt the first time.
    pub async fn remote_check(&self, peer: DeviceId) -> Result<()> {
        let session = self.connected(&peer)?;
        crate::remote::check(&self.shared, &session).await
    }

    /// Sends a remote input event to a paired PC.
    pub async fn remote_input(&self, peer: DeviceId, input: crate::RemoteInput) -> Result<()> {
        let session = self.connected(&peer)?;
        crate::remote::input(&self.shared, &session, input).await
    }

    /// Sends relative pointer motion `(dx, dy)` in a QUIC datagram (never on
    /// the control stream).
    pub async fn remote_move(&self, peer: DeviceId, dx: f32, dy: f32) -> Result<()> {
        let session = self.connected(&peer)?;
        crate::remote::move_pointer(&self.shared, &session, dx, dy).await
    }

    /// Sends smooth scroll `(dx, dy)` in a QUIC datagram.
    pub async fn remote_scroll_fast(&self, peer: DeviceId, dx: f32, dy: f32) -> Result<()> {
        let session = self.connected(&peer)?;
        crate::remote::scroll_fast(&self.shared, &session, dx, dy).await
    }

    /// Sends a laser pointer update to a paired PC (`x`, `y` in `0.0..=1.0`).
    pub async fn remote_laser(&self, peer: DeviceId, on: bool, x: f32, y: f32) -> Result<()> {
        let session = self.connected(&peer)?;
        crate::remote::laser(&self.shared, &session, on, x, y).await
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

    // ---- Phone toggles (docs/protocol/toggles.md) ----

    /// The latest quick settings reported by a connected phone, if any.
    pub fn phone_toggles(&self, peer: DeviceId) -> Option<crate::PhoneToggles> {
        self.shared.toggles.peer(&peer)
    }

    /// This phone's quick settings state (on startup and whenever any toggle
    /// changes); sent to every connected PC the user allows (and to PCs that
    /// connect later).
    pub async fn toggles_changed(&self, toggles: crate::PhoneToggles) -> Result<()> {
        self.shared.toggles_changed(toggles).await
    }

    /// Asks a paired phone to change one quick setting (`id`: `dnd`, `ringer`,
    /// `flashlight`, `volume`, `brightness`, `wifi`, `bluetooth`).
    pub async fn set_phone_toggle(
        &self,
        peer: DeviceId,
        id: String,
        value: crate::PhoneToggleValue,
    ) -> Result<()> {
        let session = self.connected(&peer)?;
        crate::toggles::set(&self.shared, &session, id, value).await
    }

    // ---- Deck (docs/protocol/deck.md) ----

    /// The latest Deck layout reported by a connected PC, if any.
    pub fn deck_layout(&self, peer: DeviceId) -> Option<crate::DeckLayout> {
        self.shared.deck.peer_layout(&peer)
    }

    /// The latest live Deck state reported by a connected PC, if any.
    pub fn deck_state(&self, peer: DeviceId) -> Option<crate::DeckState> {
        self.shared.deck.peer_state(&peer)
    }

    /// Updates this PC's Deck layout and sends it to every connected phone
    /// when this device offers `deck.actions`.
    pub async fn set_deck_layout(&self, layout: crate::DeckLayout) -> Result<()> {
        self.shared.set_deck_layout(layout).await
    }

    /// Updates this PC's live Deck state and sends it to every connected phone
    /// when it changed and this device offers `deck.actions`.
    pub async fn set_deck_state(&self, state: crate::DeckState) -> Result<()> {
        self.shared.set_deck_state(state).await
    }

    /// Asks a paired PC to run the action bound to `tile` (`deck.press`).
    pub async fn deck_press(&self, peer: DeviceId, tile: String) -> Result<()> {
        let session = self.connected(&peer)?;
        crate::deck::press(&self.shared, &session, tile).await
    }

    /// Asks a paired PC to change its master speaker volume (`0..=100`) and/or
    /// mute state (`pc.audio.set`).
    pub async fn set_pc_audio(&self, peer: DeviceId, volume: Option<u8>, muted: Option<bool>) -> Result<()> {
        let session = self.connected(&peer)?;
        crate::deck::set_pc_audio(&self.shared, &session, volume, muted).await
    }

    // ---- Phone storage (PC File Explorer) ----

    /// Lists a directory on a connected phone (`storage.list`).
    pub async fn storage_list(
        &self,
        peer: DeviceId,
        path: impl Into<String>,
    ) -> Result<Vec<crate::StorageEntry>> {
        let session = self.connected(&peer)?;
        crate::storage::list(&self.shared, &session, path.into()).await
    }

    /// Reads a byte range `[offset, offset + length)` (or to end of file if
    /// `length` is `None`) from `path` on a connected phone (`storage.read`).
    pub async fn storage_read(
        &self,
        peer: DeviceId,
        path: impl Into<String>,
        offset: u64,
        length: Option<u64>,
    ) -> Result<(crate::StorageReadMeta, Vec<u8>)> {
        let (meta, mut recv) = self.storage_read_stream(peer, path, offset, length).await?;
        let mut buf = Vec::with_capacity((meta.length.min(16 * 1024 * 1024)) as usize);
        while let Some(chunk) = recv.read_chunk(64 * 1024).await.map_err(|_| crate::Error::Offline)? {
            if (buf.len() + chunk.len()) as u64 > meta.length {
                return Err(crate::Error::Protocol("phone sent more bytes than length".into()));
            }
            buf.extend_from_slice(&chunk);
        }
        if buf.len() as u64 != meta.length {
            return Err(crate::Error::Offline);
        }
        Ok((meta, buf))
    }

    /// Opens a ranged read stream from `path` on a connected phone
    /// (`storage.read`), returning the metadata and raw QUIC receive stream.
    pub async fn storage_read_stream(
        &self,
        peer: DeviceId,
        path: impl Into<String>,
        offset: u64,
        length: Option<u64>,
    ) -> Result<(crate::StorageReadMeta, iroh::endpoint::RecvStream)> {
        let session = self.connected(&peer)?;
        crate::storage::open_read_stream(&self.shared, &session, path.into(), offset, length).await
    }

    /// Uploads a local file to `path` on a connected phone (`storage.write`).
    pub async fn storage_write(
        &self,
        peer: DeviceId,
        path: impl Into<String>,
        source: &std::path::Path,
    ) -> Result<crate::StorageWriteDone> {
        self.storage_write_with_id(peer, path, source, None, None).await
    }

    /// Uploads a local file to `path` on a connected phone (`storage.write`),
    /// reusing `id` when resuming an interrupted upload.
    pub async fn storage_write_with_id(
        &self,
        peer: DeviceId,
        path: impl Into<String>,
        source: &std::path::Path,
        id: Option<String>,
        stop_after: Option<u64>,
    ) -> Result<crate::StorageWriteDone> {
        let session = self.connected(&peer)?;
        let id = id.unwrap_or_else(crate::storage::new_upload_id);
        crate::storage::write_with_id(
            &self.shared,
            &session,
            id,
            path.into(),
            crate::FileSource::Path(source.to_path_buf()),
            None,
            stop_after,
        )
        .await
    }

    /// Creates a directory at `path` on a connected phone (`storage.mkdir`).
    pub async fn storage_mkdir(&self, peer: DeviceId, path: impl Into<String>) -> Result<()> {
        let session = self.connected(&peer)?;
        crate::storage::mkdir(&self.shared, &session, path.into()).await
    }

    /// Renames or moves `from` to `to` on a connected phone (`storage.rename`).
    pub async fn storage_rename(
        &self,
        peer: DeviceId,
        from: impl Into<String>,
        to: impl Into<String>,
    ) -> Result<()> {
        let session = self.connected(&peer)?;
        crate::storage::rename(&self.shared, &session, from.into(), to.into()).await
    }

    /// Deletes `path` on a connected phone (`storage.delete`).
    pub async fn storage_delete(
        &self,
        peer: DeviceId,
        path: impl Into<String>,
        confirmed: bool,
    ) -> Result<()> {
        let session = self.connected(&peer)?;
        crate::storage::delete(&self.shared, &session, path.into(), confirmed).await
    }

    /// Notifies connected PCs that have `path` open in File Explorer that its
    /// contents changed (`storage.changed`, debounced to at most once per second).
    pub async fn storage_changed(&self, path: impl Into<String>) {
        crate::storage::notify_changed(&self.shared, path.into()).await;
    }

    /// Directory paths currently watched by any connected PC (most recently
    /// listed folders).
    pub fn storage_open_folders(&self) -> Vec<String> {
        self.shared.storage.lock().unwrap_or_else(|e| e.into_inner()).open_folders_all()
    }

    // ---- Local state reported by the app ----

    /// Reports this device's battery; forwarded to connected devices.
    pub async fn update_battery(&self, battery: Battery) {
        let battery = battery.sanitized();
        self.shared.local.write().unwrap_or_else(|e| e.into_inner()).battery = Some(battery.clone());
        if let Ok(env) = Envelope::new(types::EVENT_BATTERY, &battery) {
            self.shared.broadcast(env).await;
        }
    }

    /// Updates this device's description (e.g. after a rename).
    pub async fn update_device_info(&self, device: DeviceInfo) {
        let device = device.sanitized();
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
        for session in self.shared.live_sessions() {
            self.shared.send_deck(&session).await;
        }
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
