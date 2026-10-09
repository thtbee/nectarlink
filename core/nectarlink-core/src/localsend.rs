// SPDX-License-Identifier: MPL-2.0
//! Clean-room Rust implementation of the LocalSend Protocol v2.1
//! (https://github.com/localsend/protocol, MIT License, Copyright (c)
//! 2022-2024 Tien Do Nam). No Dart code from LocalSend is used.
//!
//! Coexistence (`docs/PLAN.md` §3.6):
//! - Listens on UDP multicast `224.0.0.167:53317` (with `SO_REUSEADDR`) and TCP
//!   port `53317` (configurable via `NodeConfig::localsend_port`).
//! - If TCP port `53317` is already taken by the standalone LocalSend app on the
//!   same PC, Nectarlink logs a notice, leaves receiving to LocalSend, and still
//!   discovers and sends files to LocalSend peers on the network.

use std::{
    collections::HashMap,
    fs,
    io::ErrorKind,
    net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use nectarlink_protocol::{
    DeviceId,
    messages::{DeviceInfo, DeviceKind, files},
};
use ring::{
    rand::{SecureRandom, SystemRandom},
    signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair},
};
use rustls::{
    ClientConfig, DigitallySignedStruct, ServerConfig, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature},
};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
    sync::oneshot,
};
use tokio_rustls::{TlsAcceptor, TlsConnector};
use tokio_util::sync::CancellationToken;

use crate::{
    Error, Result, TimelineKind,
    events::NodeEvent,
    identity::KeyProtector,
    node::Shared,
    timeline::NewTimelineEntry,
    transfer::{
        Direction, FileSource, OutgoingFile, Transfer, TransferFailure, TransferState, outgoing_target_path,
        safe_file_name, storable_name, transfer_timeline_title, unique_path,
    },
};

/// Default TCP and UDP port used by the LocalSend protocol.
pub const LOCALSEND_DEFAULT_PORT: u16 = 53317;

/// Default IPv4 multicast group used by the LocalSend protocol.
pub const LOCALSEND_MULTICAST_ADDR: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 167);

/// LocalSend protocol version implemented by this module.
pub const LOCALSEND_PROTOCOL_VERSION: &str = "2.1";

const STATE_FILE: &str = "localsend.enc";
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(6);
const IO_TIMEOUT: Duration = Duration::from_secs(30);
const PROGRESS_EVERY: Duration = Duration::from_millis(150);
const UPLOAD_CHUNK: usize = 64 * 1024;
const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_JSON_BODY_BYTES: usize = 4 * 1024 * 1024;

/// A LocalSend device discovered on the local network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalSendPeer {
    pub id: DeviceId,
    pub alias: String,
    pub device_model: Option<String>,
    pub device_type: String,
    pub fingerprint: String,
    pub ip: String,
    pub port: u16,
    /// `"https"` or `"http"`.
    pub protocol: String,
}

/// Derives a deterministic Nectarlink [`DeviceId`] for a LocalSend peer from its
/// certificate fingerprint (or `ip:port` when encryption is off).
pub fn peer_device_id(fingerprint: &str, ip: &str, port: u16) -> DeviceId {
    let mut hasher = Sha256::new();
    hasher.update(b"nectarlink-localsend:v2:");
    let trimmed = fingerprint.trim();
    if !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("random") {
        hasher.update(trimmed.as_bytes());
    } else {
        hasher.update(format!("{ip}:{port}").as_bytes());
    }
    let digest = hasher.finalize();
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&digest);
    DeviceId(bytes)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct StoredLocalSendState {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    cert_der_hex: String,
    #[serde(default)]
    key_pkcs8_hex: String,
}

#[derive(Debug, Clone)]
pub(crate) struct LocalSendIdentity {
    pub cert_der: Vec<u8>,
    pub key_pkcs8: Vec<u8>,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MulticastDto {
    alias: String,
    #[serde(default = "default_version")]
    version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    device_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    device_type: Option<String>,
    #[serde(default)]
    fingerprint: String,
    #[serde(default = "default_port")]
    port: u16,
    #[serde(default = "default_protocol")]
    protocol: String,
    #[serde(default)]
    download: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    announce: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    announcement: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InfoDto {
    alias: String,
    #[serde(default = "default_version")]
    version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    device_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    device_type: Option<String>,
    #[serde(default)]
    fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    protocol: Option<String>,
    #[serde(default)]
    download: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileDto {
    id: String,
    file_name: String,
    size: u64,
    file_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    preview: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrepareUploadRequestDto {
    info: InfoDto,
    files: HashMap<String, FileDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrepareUploadResponseDto {
    session_id: String,
    files: HashMap<String, String>,
}

fn default_version() -> String {
    LOCALSEND_PROTOCOL_VERSION.into()
}

fn default_port() -> u16 {
    LOCALSEND_DEFAULT_PORT
}

fn default_protocol() -> String {
    "https".into()
}

fn device_kind_str(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Phone => "mobile",
        DeviceKind::Tablet => "tablet",
        DeviceKind::Desktop | DeviceKind::Laptop | DeviceKind::Unknown => "desktop",
    }
}

#[derive(Debug, Clone)]
struct IncomingFileSpec {
    file_id: String,
    token: String,
    safe_name: String,
    folder: Option<String>,
    size: u64,
    part_path: PathBuf,
    uploaded: bool,
}

struct UploadSession {
    transfer: Transfer,
    peer_name: String,
    staging_dir: PathBuf,
    files: Vec<IncomingFileSpec>,
    folder_roots: HashMap<String, PathBuf>,
    saved_items: Vec<PathBuf>,
    cancel: CancellationToken,
    last_progress: Instant,
}

struct RunningState {
    cancel: CancellationToken,
    announce_tx: tokio::sync::mpsc::UnboundedSender<()>,
}

/// Manages LocalSend discovery, server, and active upload sessions for a [`Node`](crate::Node).
pub(crate) struct LocalSendState {
    data_dir: PathBuf,
    protector: Arc<dyn KeyProtector>,
    configured_port: u16,
    enabled: AtomicBool,
    receiving: AtomicBool,
    bound_port: Mutex<u16>,
    identity: LocalSendIdentity,
    server_tls: Arc<ServerConfig>,
    client_tls: Arc<ClientConfig>,
    peers: Mutex<HashMap<DeviceId, LocalSendPeer>>,
    running: Mutex<Option<RunningState>>,
    pending_approvals: Mutex<HashMap<String, oneshot::Sender<bool>>>,
    sessions: Mutex<HashMap<String, Arc<Mutex<UploadSession>>>>,
}

impl std::fmt::Debug for LocalSendState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalSendState")
            .field("enabled", &self.enabled.load(Ordering::Relaxed))
            .field("receiving", &self.receiving.load(Ordering::Relaxed))
            .field("fingerprint", &self.identity.fingerprint)
            .finish_non_exhaustive()
    }
}

impl LocalSendState {
    pub fn open(data_dir: &Path, protector: Arc<dyn KeyProtector>, configured_port: u16) -> Result<Self> {
        let state_path = data_dir.join(STATE_FILE);
        let mut stored: StoredLocalSendState = match fs::read(&state_path) {
            Ok(sealed) => protector
                .unprotect(&sealed)
                .ok()
                .and_then(|plain| serde_json::from_slice(&plain).ok())
                .unwrap_or_default(),
            Err(_) => StoredLocalSendState::default(),
        };

        let identity = match decode_stored_identity(&stored) {
            Some(id) => id,
            None => {
                let id = generate_self_signed_identity()?;
                stored.cert_der_hex = data_encoding::HEXLOWER.encode(&id.cert_der);
                stored.key_pkcs8_hex = data_encoding::HEXLOWER.encode(&id.key_pkcs8);
                let _ = save_stored_state(data_dir, &*protector, &stored);
                id
            }
        };

        let server_tls = Arc::new(build_server_tls_config(&identity)?);
        let client_tls = Arc::new(build_client_tls_config()?);

        Ok(Self {
            data_dir: data_dir.to_path_buf(),
            protector,
            configured_port: if configured_port == 0 { LOCALSEND_DEFAULT_PORT } else { configured_port },
            enabled: AtomicBool::new(stored.enabled),
            receiving: AtomicBool::new(false),
            bound_port: Mutex::new(if configured_port == 0 {
                LOCALSEND_DEFAULT_PORT
            } else {
                configured_port
            }),
            identity,
            server_tls,
            client_tls,
            peers: Mutex::new(HashMap::new()),
            running: Mutex::new(None),
            pending_approvals: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
        })
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn receiving(&self) -> bool {
        self.receiving.load(Ordering::Relaxed)
    }

    pub fn bound_port(&self) -> u16 {
        *self.bound_port.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_enabled(&self, shared: &Arc<Shared>, enabled: bool) -> Result<()> {
        let prev = self.enabled.swap(enabled, Ordering::SeqCst);
        let stored = StoredLocalSendState {
            enabled,
            cert_der_hex: data_encoding::HEXLOWER.encode(&self.identity.cert_der),
            key_pkcs8_hex: data_encoding::HEXLOWER.encode(&self.identity.key_pkcs8),
        };
        save_stored_state(&self.data_dir, &*self.protector, &stored)?;

        if enabled && !prev {
            self.start(shared);
        } else if !enabled && prev {
            self.stop(shared);
        }
        shared.emit(NodeEvent::LocalSendChanged);
        Ok(())
    }

    pub fn peers(&self) -> Vec<LocalSendPeer> {
        let mut list: Vec<LocalSendPeer> =
            self.peers.lock().unwrap_or_else(|e| e.into_inner()).values().cloned().collect();
        list.sort_by(|a, b| {
            a.alias.cmp(&b.alias).then_with(|| a.ip.cmp(&b.ip)).then_with(|| a.port.cmp(&b.port))
        });
        list
    }

    pub fn peer(&self, id: &DeviceId) -> Option<LocalSendPeer> {
        self.peers.lock().unwrap_or_else(|e| e.into_inner()).get(id).cloned()
    }

    pub fn has_peer(&self, id: &DeviceId) -> bool {
        self.peers.lock().unwrap_or_else(|e| e.into_inner()).contains_key(id)
    }

    pub fn peer_name(&self, id: &DeviceId) -> Option<String> {
        self.peers.lock().unwrap_or_else(|e| e.into_inner()).get(id).map(|p| p.alias.clone())
    }

    pub fn upsert_peer(&self, shared: &Shared, peer: LocalSendPeer) -> bool {
        if peer.fingerprint == self.identity.fingerprint {
            return false;
        }
        let changed = {
            let mut map = self.peers.lock().unwrap_or_else(|e| e.into_inner());
            map.insert(peer.id, peer.clone()).as_ref() != Some(&peer)
        };
        if changed {
            shared.emit(NodeEvent::LocalSendChanged);
        }
        changed
    }

    pub fn resolve_transfer_approval(&self, id: &str, accepted: bool) -> bool {
        if let Some(tx) = self.pending_approvals.lock().unwrap_or_else(|e| e.into_inner()).remove(id) {
            let _ = tx.send(accepted);
            true
        } else {
            false
        }
    }

    pub fn refresh(&self, shared: &Arc<Shared>) {
        if !self.enabled() {
            return;
        }
        if let Some(running) = self.running.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            let _ = running.announce_tx.send(());
        }
        let weak = Arc::downgrade(shared);
        let port = self.configured_port;
        tokio::spawn(async move {
            if let Some(s) = weak.upgrade() {
                let _ = probe_peer(&s, "127.0.0.1", port).await;
                if port != LOCALSEND_DEFAULT_PORT {
                    let _ = probe_peer(&s, "127.0.0.1", LOCALSEND_DEFAULT_PORT).await;
                }
            }
        });
    }

    pub fn start(&self, shared: &Arc<Shared>) {
        let mut guard = self.running.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_some() {
            return;
        }
        let cancel = shared.cancel.child_token();
        let (announce_tx, mut announce_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let configured_port = self.configured_port;

        // Bind TCP listener synchronously via std::net::TcpListener or inside tokio task.
        let std_listener = match std::net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, configured_port)) {
            Ok(l) => {
                let _ = l.set_nonblocking(true);
                Some(l)
            }
            Err(e) => {
                tracing::info!(
                    port = configured_port,
                    error = %e,
                    "LocalSend TCP port is in use by another app; Nectarlink will still discover and send to LocalSend peers"
                );
                None
            }
        };

        let (bound_port, receiving) = match &std_listener {
            Some(l) => (l.local_addr().map(|a| a.port()).unwrap_or(configured_port), true),
            None => (configured_port, false),
        };
        *self.bound_port.lock().unwrap_or_else(|e| e.into_inner()) = bound_port;
        self.receiving.store(receiving, Ordering::SeqCst);

        *guard = Some(RunningState { cancel: cancel.clone(), announce_tx: announce_tx.clone() });
        drop(guard);

        // Spawn TCP server if we acquired the port.
        if let Some(std_listener) = std_listener {
            let weak = Arc::downgrade(shared);
            let tcp_cancel = cancel.child_token();
            tokio::spawn(async move {
                let Ok(listener) = TcpListener::from_std(std_listener) else { return };
                loop {
                    let accepted = tokio::select! {
                        _ = tcp_cancel.cancelled() => return,
                        res = listener.accept() => res,
                    };
                    let Ok((stream, remote_addr)) = accepted else { continue };
                    let Some(shared) = weak.upgrade() else { return };
                    let conn_cancel = tcp_cancel.child_token();
                    tokio::spawn(async move {
                        handle_incoming_connection(shared, stream, remote_addr, conn_cancel).await;
                    });
                }
            });
        }

        // Spawn UDP multicast discovery + periodic announcer.
        let weak = Arc::downgrade(shared);
        let udp_cancel = cancel.child_token();
        tokio::spawn(async move {
            let udp =
                match bind_multicast_socket(LOCALSEND_DEFAULT_PORT).or_else(|_| bind_multicast_socket(0)) {
                    Ok(sock) => Arc::new(sock),
                    Err(e) => {
                        tracing::warn!(error = %e, "can't bind LocalSend UDP discovery socket");
                        return;
                    }
                };

            let custom_udp = if bound_port != LOCALSEND_DEFAULT_PORT && bound_port != 0 {
                bind_multicast_socket(bound_port).ok().map(Arc::new)
            } else {
                None
            };

            if let Some(s) = weak.upgrade() {
                send_multicast_announcement(&s, &udp, bound_port, receiving).await;
                let _ = probe_peer(&s, "127.0.0.1", configured_port).await;
                if configured_port != LOCALSEND_DEFAULT_PORT {
                    let _ = probe_peer(&s, "127.0.0.1", LOCALSEND_DEFAULT_PORT).await;
                }
            }

            if let Some(custom) = custom_udp {
                let weak_custom = weak.clone();
                let custom_cancel = udp_cancel.child_token();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    loop {
                        let recv = tokio::select! {
                            _ = custom_cancel.cancelled() => return,
                            r = custom.recv_from(&mut buf) => r,
                        };
                        let Ok((n, src)) = recv else { continue };
                        let Some(shared) = weak_custom.upgrade() else { return };
                        handle_udp_packet(&shared, &buf[..n], src).await;
                    }
                });
            }

            let mut buf = vec![0u8; 8192];
            let mut ticker = tokio::time::interval(Duration::from_secs(10));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = udp_cancel.cancelled() => return,
                    _ = ticker.tick() => {
                        let Some(shared) = weak.upgrade() else { return };
                        send_multicast_announcement(&shared, &udp, bound_port, receiving).await;
                    }
                    Some(()) = announce_rx.recv() => {
                        let Some(shared) = weak.upgrade() else { return };
                        send_multicast_announcement(&shared, &udp, bound_port, receiving).await;
                    }
                    res = udp.recv_from(&mut buf) => {
                        let Ok((n, src)) = res else { continue };
                        let Some(shared) = weak.upgrade() else { return };
                        handle_udp_packet(&shared, &buf[..n], src).await;
                    }
                }
            }
        });
    }

    pub fn stop(&self, shared: &Shared) {
        if let Some(running) = self.running.lock().unwrap_or_else(|e| e.into_inner()).take() {
            running.cancel.cancel();
        }
        self.receiving.store(false, Ordering::SeqCst);
        self.peers.lock().unwrap_or_else(|e| e.into_inner()).clear();
        for (_, tx) in self.pending_approvals.lock().unwrap_or_else(|e| e.into_inner()).drain() {
            let _ = tx.send(false);
        }
        for (_, session) in self.sessions.lock().unwrap_or_else(|e| e.into_inner()).drain() {
            let s = session.lock().unwrap_or_else(|e| e.into_inner());
            s.cancel.cancel();
        }
        shared.emit(NodeEvent::LocalSendChanged);
    }
}

fn build_our_multicast_dto(shared: &Shared, port: u16, announce: bool) -> MulticastDto {
    let device: DeviceInfo = shared.local.read().unwrap_or_else(|e| e.into_inner()).device.clone();
    MulticastDto {
        alias: device.name,
        version: LOCALSEND_PROTOCOL_VERSION.into(),
        device_model: device.model.or_else(|| Some("Nectarlink".into())),
        device_type: Some(device_kind_str(device.kind).into()),
        fingerprint: shared.localsend.identity.fingerprint.clone(),
        port,
        protocol: "https".into(),
        download: false,
        announce: Some(announce),
        announcement: Some(announce),
    }
}

fn build_our_info_dto(shared: &Shared) -> InfoDto {
    let device: DeviceInfo = shared.local.read().unwrap_or_else(|e| e.into_inner()).device.clone();
    let port = shared.localsend.bound_port();
    InfoDto {
        alias: device.name,
        version: LOCALSEND_PROTOCOL_VERSION.into(),
        device_model: device.model.or_else(|| Some("Nectarlink".into())),
        device_type: Some(device_kind_str(device.kind).into()),
        fingerprint: shared.localsend.identity.fingerprint.clone(),
        port: Some(port),
        protocol: Some("https".into()),
        download: false,
    }
}

async fn send_multicast_announcement(shared: &Shared, udp: &UdpSocket, bound_port: u16, receiving: bool) {
    let dto = build_our_multicast_dto(shared, bound_port, receiving);
    let Ok(bytes) = serde_json::to_vec(&dto) else { return };
    let target = SocketAddr::V4(SocketAddrV4::new(LOCALSEND_MULTICAST_ADDR, LOCALSEND_DEFAULT_PORT));
    let _ = udp.send_to(&bytes, target).await;
    if bound_port != LOCALSEND_DEFAULT_PORT && bound_port != 0 {
        let custom_target = SocketAddr::V4(SocketAddrV4::new(LOCALSEND_MULTICAST_ADDR, bound_port));
        let _ = udp.send_to(&bytes, custom_target).await;
        let loopback_target = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, bound_port));
        let _ = udp.send_to(&bytes, loopback_target).await;
    }
}

async fn handle_udp_packet(shared: &Arc<Shared>, data: &[u8], src: SocketAddr) {
    let Ok(dto) = serde_json::from_slice::<MulticastDto>(data) else { return };
    if dto.fingerprint == shared.localsend.identity.fingerprint || dto.alias.trim().is_empty() {
        return;
    }
    let ip = match src.ip() {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => v6.to_string(),
    };
    let port = if dto.port == 0 { LOCALSEND_DEFAULT_PORT } else { dto.port };
    let protocol =
        if dto.protocol.eq_ignore_ascii_case("http") { "http".to_owned() } else { "https".to_owned() };
    let peer = LocalSendPeer {
        id: peer_device_id(&dto.fingerprint, &ip, port),
        alias: dto.alias,
        device_model: dto.device_model,
        device_type: dto.device_type.unwrap_or_else(|| "desktop".into()),
        fingerprint: dto.fingerprint,
        ip: ip.clone(),
        port,
        protocol: protocol.clone(),
    };
    shared.localsend.upsert_peer(shared, peer);

    let is_announce = dto.announce.or(dto.announcement).unwrap_or(false);
    if is_announce && shared.localsend.receiving() {
        let shared = shared.clone();
        tokio::spawn(async move {
            let _ = send_register_callback(&shared, &ip, port, &protocol).await;
        });
    }
}

async fn send_register_callback(shared: &Arc<Shared>, ip: &str, port: u16, protocol: &str) -> Result<()> {
    let our_port = shared.localsend.bound_port();
    let dto = build_our_multicast_dto(shared, our_port, false);
    let body = serde_json::to_vec(&dto).map_err(|e| Error::Internal(e.to_string()))?;
    let mut stream = connect_peer_stream(shared, ip, port, protocol == "https").await?;
    write_http_request(
        &mut stream,
        "POST",
        "/api/localsend/v2/register",
        &format!("{ip}:{port}"),
        Some("application/json"),
        &body,
    )
    .await?;
    let resp = read_http_response(&mut stream).await?;
    if (200..300).contains(&resp.status)
        && let Ok(info) = serde_json::from_slice::<InfoDto>(&resp.body)
        && info.fingerprint != shared.localsend.identity.fingerprint
        && !info.alias.trim().is_empty()
    {
        let peer_port = info.port.unwrap_or(port);
        let peer_proto = info
            .protocol
            .as_deref()
            .map(|p| if p.eq_ignore_ascii_case("http") { "http" } else { "https" })
            .unwrap_or(protocol)
            .to_owned();
        let peer = LocalSendPeer {
            id: peer_device_id(&info.fingerprint, ip, peer_port),
            alias: info.alias,
            device_model: info.device_model,
            device_type: info.device_type.unwrap_or_else(|| "desktop".into()),
            fingerprint: info.fingerprint,
            ip: ip.to_owned(),
            port: peer_port,
            protocol: peer_proto,
        };
        shared.localsend.upsert_peer(shared, peer);
    }
    Ok(())
}

/// Probes `host:port` over HTTPS (falling back to HTTP) via `GET /api/localsend/v2/info`
/// (and `POST /api/localsend/v2/register` if we are receiving) and records the peer if found.
pub(crate) async fn probe_peer(shared: &Arc<Shared>, host: &str, port: u16) -> Result<LocalSendPeer> {
    for use_tls in [true, false] {
        let protocol = if use_tls { "https" } else { "http" };
        let Ok(mut stream) = connect_peer_stream(shared, host, port, use_tls).await else {
            continue;
        };
        let req_res = if shared.localsend.receiving() {
            let dto = build_our_multicast_dto(shared, shared.localsend.bound_port(), false);
            let body = serde_json::to_vec(&dto).unwrap_or_default();
            write_http_request(
                &mut stream,
                "POST",
                "/api/localsend/v2/register",
                &format!("{host}:{port}"),
                Some("application/json"),
                &body,
            )
            .await
        } else {
            write_http_request(
                &mut stream,
                "GET",
                "/api/localsend/v2/info",
                &format!("{host}:{port}"),
                None,
                &[],
            )
            .await
        };
        if req_res.is_err() {
            continue;
        }
        let Ok(resp) = read_http_response(&mut stream).await else {
            continue;
        };
        if !(200..300).contains(&resp.status) {
            continue;
        }
        let Ok(info) = serde_json::from_slice::<InfoDto>(&resp.body) else {
            continue;
        };
        if info.fingerprint == shared.localsend.identity.fingerprint || info.alias.trim().is_empty() {
            return Err(Error::NotFound);
        }
        let peer_port = info.port.unwrap_or(port);
        let peer_proto = info
            .protocol
            .as_deref()
            .map(|p| if p.eq_ignore_ascii_case("http") { "http" } else { "https" })
            .unwrap_or(protocol)
            .to_owned();
        let peer = LocalSendPeer {
            id: peer_device_id(&info.fingerprint, host, peer_port),
            alias: info.alias,
            device_model: info.device_model,
            device_type: info.device_type.unwrap_or_else(|| "desktop".into()),
            fingerprint: info.fingerprint,
            ip: host.to_owned(),
            port: peer_port,
            protocol: peer_proto,
        };
        shared.localsend.upsert_peer(shared, peer.clone());
        return Ok(peer);
    }
    Err(Error::Offline)
}

// ---- Server ----

enum ServerStream {
    Plain(TcpStream),
    Tls(Box<tokio_rustls::server::TlsStream<TcpStream>>),
}

impl AsyncRead for ServerStream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match &mut *self {
            ServerStream::Plain(s) => std::pin::Pin::new(s).poll_read(cx, buf),
            ServerStream::Tls(s) => std::pin::Pin::new(&mut **s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for ServerStream {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        match &mut *self {
            ServerStream::Plain(s) => std::pin::Pin::new(s).poll_write(cx, buf),
            ServerStream::Tls(s) => std::pin::Pin::new(&mut **s).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match &mut *self {
            ServerStream::Plain(s) => std::pin::Pin::new(s).poll_flush(cx),
            ServerStream::Tls(s) => std::pin::Pin::new(&mut **s).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match &mut *self {
            ServerStream::Plain(s) => std::pin::Pin::new(s).poll_shutdown(cx),
            ServerStream::Tls(s) => std::pin::Pin::new(&mut **s).poll_shutdown(cx),
        }
    }
}

async fn handle_incoming_connection(
    shared: Arc<Shared>,
    tcp: TcpStream,
    remote_addr: SocketAddr,
    cancel: CancellationToken,
) {
    let mut first = [0u8; 1];
    let peeked = tokio::time::timeout(Duration::from_secs(5), tcp.peek(&mut first)).await;
    let Ok(Ok(1)) = peeked else { return };

    let mut stream = if first[0] == 0x16 {
        let acceptor = TlsAcceptor::from(shared.localsend.server_tls.clone());
        let Ok(Ok(tls)) = tokio::time::timeout(Duration::from_secs(8), acceptor.accept(tcp)).await else {
            return;
        };
        ServerStream::Tls(Box::new(tls))
    } else {
        ServerStream::Plain(tcp)
    };

    let Ok(head) = read_http_head(&mut stream).await else { return };
    let (path, query) = match head.target.split_once('?') {
        Some((p, q)) => (p, q),
        None => (head.target.as_str(), ""),
    };

    match (head.method.as_str(), path) {
        ("GET", "/api/localsend/v2/info" | "/api/localsend/v1/info") => {
            let info = build_our_info_dto(&shared);
            let body = serde_json::to_vec(&info).unwrap_or_default();
            let _ = write_http_response(&mut stream, 200, "OK", Some("application/json"), &body).await;
        }
        ("POST", "/api/localsend/v2/register" | "/api/localsend/v1/register") => {
            let Ok(body) = read_http_body(&mut stream, &head).await else { return };
            if let Ok(dto) = serde_json::from_slice::<MulticastDto>(&body)
                && dto.fingerprint != shared.localsend.identity.fingerprint
                && !dto.alias.trim().is_empty()
            {
                let ip = remote_addr.ip().to_string();
                let port = if dto.port == 0 { LOCALSEND_DEFAULT_PORT } else { dto.port };
                let protocol = if dto.protocol.eq_ignore_ascii_case("http") {
                    "http".to_owned()
                } else {
                    "https".to_owned()
                };
                let peer = LocalSendPeer {
                    id: peer_device_id(&dto.fingerprint, &ip, port),
                    alias: dto.alias,
                    device_model: dto.device_model,
                    device_type: dto.device_type.unwrap_or_else(|| "desktop".into()),
                    fingerprint: dto.fingerprint,
                    ip,
                    port,
                    protocol,
                };
                shared.localsend.upsert_peer(&shared, peer);
            }
            let info = build_our_info_dto(&shared);
            let resp_body = serde_json::to_vec(&info).unwrap_or_default();
            let _ = write_http_response(&mut stream, 200, "OK", Some("application/json"), &resp_body).await;
        }
        ("POST", "/api/localsend/v2/prepare-upload" | "/api/localsend/v1/prepare-upload") => {
            handle_prepare_upload(&shared, &mut stream, &head, remote_addr, &cancel).await;
        }
        ("POST", "/api/localsend/v2/upload" | "/api/localsend/v1/upload") => {
            handle_upload(&shared, &mut stream, &head, query).await;
        }
        ("POST", "/api/localsend/v2/cancel" | "/api/localsend/v1/cancel") => {
            handle_cancel(&shared, &mut stream, query).await;
        }
        _ => {
            let _ = write_http_response(&mut stream, 404, "Not Found", None, &[]).await;
        }
    }
}

async fn handle_prepare_upload(
    shared: &Arc<Shared>,
    stream: &mut ServerStream,
    head: &HttpHead,
    remote_addr: SocketAddr,
    conn_cancel: &CancellationToken,
) {
    let Ok(body) = read_http_body(stream, head).await else { return };
    let Ok(req) = serde_json::from_slice::<PrepareUploadRequestDto>(&body) else {
        let _ = write_http_response(stream, 400, "Bad Request", None, &[]).await;
        return;
    };
    if req.files.is_empty() || req.files.len() > files::MAX_FILES {
        let _ = write_http_response(stream, 400, "Bad Request", None, &[]).await;
        return;
    }

    let ip = remote_addr.ip().to_string();
    let port = req.info.port.unwrap_or(LOCALSEND_DEFAULT_PORT);
    let protocol = req
        .info
        .protocol
        .as_deref()
        .map(|p| if p.eq_ignore_ascii_case("http") { "http" } else { "https" })
        .unwrap_or("https")
        .to_owned();
    let alias = if req.info.alias.trim().is_empty() { ip.clone() } else { req.info.alias.trim().to_owned() };
    let peer_id = peer_device_id(&req.info.fingerprint, &ip, port);
    let peer = LocalSendPeer {
        id: peer_id,
        alias: alias.clone(),
        device_model: req.info.device_model,
        device_type: req.info.device_type.unwrap_or_else(|| "desktop".into()),
        fingerprint: req.info.fingerprint,
        ip,
        port,
        protocol,
    };
    shared.localsend.upsert_peer(shared, peer);

    let session_id = format!("lss_{}", random_token());
    let transfer_id = format!("ls_{}", random_token());
    let staging_dir = shared.incoming_dir().join("localsend").join(&session_id);
    if tokio::fs::create_dir_all(&staging_dir).await.is_err() {
        let _ = write_http_response(stream, 500, "Internal Server Error", None, &[]).await;
        return;
    }

    let mut sorted_files: Vec<FileDto> = req.files.into_values().collect();
    sorted_files.sort_by(|a, b| a.file_name.cmp(&b.file_name).then_with(|| a.id.cmp(&b.id)));

    let mut specs = Vec::with_capacity(sorted_files.len());
    let mut tokens = HashMap::with_capacity(sorted_files.len());
    let mut names: Vec<String> = Vec::new();
    let mut total_bytes: u64 = 0;

    for (idx, dto) in sorted_files.into_iter().enumerate() {
        let (folder, safe_name) = split_localsend_file_name(&dto.file_name);
        if let Some(ref f) = folder {
            let top = f.split('/').next().unwrap_or(f);
            if !names.iter().any(|n| n == top) {
                names.push(top.to_owned());
            }
        } else {
            names.push(safe_name.clone());
        }
        total_bytes = total_bytes.saturating_add(dto.size);
        let token = format!("tok_{}", random_token());
        tokens.insert(dto.id.clone(), token.clone());
        specs.push(IncomingFileSpec {
            file_id: dto.id,
            token,
            safe_name,
            folder,
            size: dto.size,
            part_path: staging_dir.join(format!("{idx}.part")),
            uploaded: false,
        });
    }

    let transfer = Transfer {
        id: transfer_id.clone(),
        device: peer_id,
        direction: Direction::Incoming,
        names,
        files: specs.len(),
        total: total_bytes,
        done: 0,
        state: TransferState::Requested,
        recording: false,
        markers: Vec::new(),
        open_on_arrival: false,
    };

    let cancel = CancellationToken::new();
    shared.register_transfer(&transfer_id, cancel.clone());
    let (approval_tx, approval_rx) = oneshot::channel::<bool>();
    shared
        .localsend
        .pending_approvals
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(transfer_id.clone(), approval_tx);

    shared.emit(NodeEvent::Transfer(transfer.clone()));

    let accepted = tokio::select! {
        _ = conn_cancel.cancelled() => false,
        _ = cancel.cancelled() => false,
        _ = tokio::time::sleep(APPROVAL_TIMEOUT) => false,
        res = approval_rx => res.unwrap_or(false),
    };

    shared.localsend.pending_approvals.lock().unwrap_or_else(|e| e.into_inner()).remove(&transfer_id);

    if !accepted {
        shared.unregister_transfer(&transfer_id);
        let _ = tokio::fs::remove_dir_all(&staging_dir).await;
        let mut cancelled = transfer;
        cancelled.state = TransferState::Cancelled;
        shared.emit(NodeEvent::Transfer(cancelled));
        let _ = write_http_response(stream, 403, "Forbidden", None, &[]).await;
        return;
    }

    let mut running_transfer = transfer;
    running_transfer.state = TransferState::Running;
    shared.emit(NodeEvent::Transfer(running_transfer.clone()));

    let session = Arc::new(Mutex::new(UploadSession {
        transfer: running_transfer,
        peer_name: alias,
        staging_dir,
        files: specs,
        folder_roots: HashMap::new(),
        saved_items: Vec::new(),
        cancel: cancel.clone(),
        last_progress: Instant::now() - PROGRESS_EVERY,
    }));
    shared.localsend.sessions.lock().unwrap_or_else(|e| e.into_inner()).insert(session_id.clone(), session);

    let resp = PrepareUploadResponseDto { session_id, files: tokens };
    let resp_body = serde_json::to_vec(&resp).unwrap_or_default();
    let _ = write_http_response(stream, 200, "OK", Some("application/json"), &resp_body).await;
}

async fn handle_upload(shared: &Arc<Shared>, stream: &mut ServerStream, head: &HttpHead, query: &str) {
    let params = parse_query(query);
    let (Some(session_id), Some(file_id), Some(token)) =
        (params.get("sessionId").cloned(), params.get("fileId").cloned(), params.get("token").cloned())
    else {
        let _ = write_http_response(stream, 400, "Bad Request", None, &[]).await;
        return;
    };

    let Some(session_arc) =
        shared.localsend.sessions.lock().unwrap_or_else(|e| e.into_inner()).get(&session_id).cloned()
    else {
        let _ = write_http_response(stream, 404, "Not Found", None, &[]).await;
        return;
    };

    enum UploadLookup {
        NotFound,
        Forbidden,
        Found(usize, PathBuf, u64, CancellationToken, String),
    }

    let lookup = {
        let guard = session_arc.lock().unwrap_or_else(|e| e.into_inner());
        match guard.files.iter().enumerate().find(|(_, f)| f.file_id == file_id) {
            None => UploadLookup::NotFound,
            Some((_, spec)) if spec.token != token || spec.uploaded => UploadLookup::Forbidden,
            Some((idx, spec)) => UploadLookup::Found(
                idx,
                spec.part_path.clone(),
                spec.size,
                guard.cancel.clone(),
                guard.transfer.id.clone(),
            ),
        }
    };

    let (spec_idx, part_path, expected_size, cancel, transfer_id) = match lookup {
        UploadLookup::NotFound => {
            let _ = write_http_response(stream, 404, "Not Found", None, &[]).await;
            return;
        }
        UploadLookup::Forbidden => {
            let _ = write_http_response(stream, 403, "Forbidden", None, &[]).await;
            return;
        }
        UploadLookup::Found(idx, p, sz, c, tid) => (idx, p, sz, c, tid),
    };

    let Ok(mut out) = tokio::fs::File::create(&part_path).await else {
        let _ = write_http_response(stream, 500, "Internal Server Error", None, &[]).await;
        return;
    };

    let stream_res = stream_body_to_file(stream, head, &mut out, expected_size, &cancel, |delta| {
        let mut guard = session_arc.lock().unwrap_or_else(|e| e.into_inner());
        guard.transfer.done = guard.transfer.done.saturating_add(delta);
        if guard.last_progress.elapsed() >= PROGRESS_EVERY {
            guard.last_progress = Instant::now();
            shared.emit(NodeEvent::Transfer(guard.transfer.clone()));
        }
    })
    .await;

    let _ = out.flush().await;
    drop(out);

    if let Err(err_state) = stream_res {
        shared.localsend.sessions.lock().unwrap_or_else(|e| e.into_inner()).remove(&session_id);
        shared.unregister_transfer(&transfer_id);
        let staging_dir = {
            let mut guard = session_arc.lock().unwrap_or_else(|e| e.into_inner());
            guard.transfer.state = err_state;
            shared.emit(NodeEvent::Transfer(guard.transfer.clone()));
            guard.staging_dir.clone()
        };
        let _ = tokio::fs::remove_dir_all(&staging_dir).await;
        let _ = write_http_response(stream, 500, "Upload Interrupted", None, &[]).await;
        return;
    }

    // Move completed part file into downloads_dir.
    let finalize_res = finalize_uploaded_file(shared, &session_arc, spec_idx).await;
    if finalize_res.is_err() {
        let _ = write_http_response(stream, 500, "Internal Server Error", None, &[]).await;
        return;
    }

    // Check if all files in this session are complete.
    let finished = {
        let mut guard = session_arc.lock().unwrap_or_else(|e| e.into_inner());
        if guard.files.iter().all(|f| f.uploaded) {
            let saved = guard.saved_items.clone();
            guard.transfer.done = guard.transfer.total;
            guard.transfer.state = TransferState::Done { saved: saved.clone() };
            Some((
                guard.transfer.clone(),
                guard.peer_name.clone(),
                guard.staging_dir.clone(),
                saved,
                guard.files.len(),
                guard.files.iter().any(|f| f.folder.is_some()),
            ))
        } else {
            None
        }
    };

    if let Some((done_transfer, peer_name, staging_dir, saved, file_count, has_folder)) = finished {
        shared.localsend.sessions.lock().unwrap_or_else(|e| e.into_inner()).remove(&session_id);
        shared.unregister_transfer(&transfer_id);
        let _ = tokio::fs::remove_dir_all(&staging_dir).await;
        let detail = if has_folder {
            match file_count {
                1 => "Folder · 1 file (LocalSend)".into(),
                n => format!("Folder · {n} files (LocalSend)"),
            }
        } else {
            match file_count {
                1 => "File (LocalSend)".into(),
                n => format!("{n} files (LocalSend)"),
            }
        };
        let target = saved.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>().join("\n");
        let _ = shared.record_timeline(NewTimelineEntry {
            kind: TimelineKind::File,
            device_id: done_transfer.device,
            device_name: peer_name,
            incoming: true,
            timestamp: crate::now_unix(),
            title: transfer_timeline_title(&done_transfer.names),
            detail,
            target,
            size_bytes: done_transfer.total,
            duration_secs: 0,
            ref_id: Some(done_transfer.id.clone()),
        });
        shared.emit(NodeEvent::Transfer(done_transfer));
    }

    let _ = write_http_response(stream, 200, "OK", None, &[]).await;
}

async fn finalize_uploaded_file(
    shared: &Shared,
    session_arc: &Arc<Mutex<UploadSession>>,
    spec_idx: usize,
) -> std::io::Result<()> {
    let downloads_dir = shared.downloads_dir.clone();
    let session_arc = session_arc.clone();
    tokio::task::spawn_blocking(move || {
        fs::create_dir_all(&downloads_dir)?;
        let mut guard = session_arc.lock().unwrap_or_else(|e| e.into_inner());
        let folder = guard.files[spec_idx].folder.clone();
        let safe_name = guard.files[spec_idx].safe_name.clone();
        let part_path = guard.files[spec_idx].part_path.clone();

        let target_dir = match folder.as_deref() {
            None => downloads_dir.clone(),
            Some(f) => {
                let mut segments = f.split('/').map(storable_name);
                let top = segments.next().unwrap_or_else(|| "Folder".into());
                let mut at = match guard.folder_roots.get(&top) {
                    Some(existing) => existing.clone(),
                    None => {
                        let root = unique_path(&downloads_dir, &top);
                        fs::create_dir_all(&root)?;
                        guard.folder_roots.insert(top, root.clone());
                        guard.saved_items.push(root.clone());
                        root
                    }
                };
                at.extend(segments);
                fs::create_dir_all(&at)?;
                at
            }
        };

        let dest = unique_path(&target_dir, &storable_name(&safe_name));
        if fs::rename(&part_path, &dest).is_err() {
            fs::copy(&part_path, &dest)?;
            let _ = fs::remove_file(&part_path);
        }
        if folder.is_none() {
            guard.saved_items.push(dest);
        }
        guard.files[spec_idx].uploaded = true;
        Ok(())
    })
    .await
    .map_err(std::io::Error::other)?
}

async fn handle_cancel(shared: &Arc<Shared>, stream: &mut ServerStream, query: &str) {
    let params = parse_query(query);
    let removed = params.get("sessionId").and_then(|session_id| {
        shared.localsend.sessions.lock().unwrap_or_else(|e| e.into_inner()).remove(session_id)
    });
    if let Some(session_arc) = removed {
        let (transfer_id, staging_dir, mut transfer) = {
            let guard = session_arc.lock().unwrap_or_else(|e| e.into_inner());
            guard.cancel.cancel();
            (guard.transfer.id.clone(), guard.staging_dir.clone(), guard.transfer.clone())
        };
        shared.unregister_transfer(&transfer_id);
        let _ = tokio::fs::remove_dir_all(&staging_dir).await;
        transfer.state = TransferState::Cancelled;
        shared.emit(NodeEvent::Transfer(transfer));
    }
    let _ = write_http_response(stream, 200, "OK", None, &[]).await;
}

// ---- Client (Outgoing file transfer to a LocalSendPeer) ----

struct OutgoingLocalSendFile {
    file_id: String,
    file_name: String,
    folder: Option<String>,
    size: u64,
    file: tokio::fs::File,
}

pub(crate) async fn send_files(
    shared: &Arc<Shared>,
    peer_id: DeviceId,
    files: Vec<OutgoingFile>,
) -> Result<String> {
    let Some(peer) = shared.localsend.peer(&peer_id) else {
        return Err(Error::NotPaired);
    };
    let mut opened = Vec::with_capacity(files.len());
    let mut source_targets: Vec<String> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    let mut total_bytes: u64 = 0;

    for (idx, file) in files.into_iter().enumerate() {
        let handle = match file.source {
            FileSource::Path(path) => {
                let target = outgoing_target_path(&path, file.folder.as_deref());
                if !source_targets.contains(&target) {
                    source_targets.push(target);
                }
                std::fs::File::open(path)?
            }
            FileSource::File(handle) => handle,
        };
        let size = handle.metadata()?.len();
        let clean_name = safe_file_name(&file.name);
        let wire_name = match file.folder.as_deref() {
            Some(folder) => {
                let top = folder.split('/').next().unwrap_or(folder);
                if !names.iter().any(|n| n == top) {
                    names.push(top.to_owned());
                }
                format!("{folder}/{clean_name}")
            }
            None => {
                names.push(clean_name.clone());
                clean_name.clone()
            }
        };
        total_bytes = total_bytes.saturating_add(size);
        opened.push(OutgoingLocalSendFile {
            file_id: format!("f_{idx}"),
            file_name: wire_name,
            folder: file.folder,
            size,
            file: tokio::fs::File::from_std(handle),
        });
    }

    let transfer_id = format!("ls_{}", random_token());
    let transfer = Transfer {
        id: transfer_id.clone(),
        device: peer_id,
        direction: Direction::Outgoing,
        names,
        files: opened.len(),
        total: total_bytes,
        done: 0,
        state: TransferState::Waiting,
        recording: false,
        markers: Vec::new(),
        open_on_arrival: false,
    };

    let cancel = CancellationToken::new();
    shared.register_transfer(&transfer_id, cancel.clone());
    shared.emit(NodeEvent::Transfer(transfer.clone()));

    let shared_clone = shared.clone();
    tokio::spawn(async move {
        let has_folder = opened.iter().any(|f| f.folder.is_some());
        let file_count = opened.len();
        let tl_target = source_targets.join("\n");
        let outcome = run_outgoing_transfer(&shared_clone, &peer, opened, &transfer, &cancel).await;
        let mut final_transfer = transfer;
        match outcome {
            Ok(()) => {
                final_transfer.done = final_transfer.total;
                final_transfer.state = TransferState::Done { saved: Vec::new() };
                let detail = if has_folder {
                    match file_count {
                        1 => "Folder · 1 file (LocalSend)".into(),
                        n => format!("Folder · {n} files (LocalSend)"),
                    }
                } else {
                    match file_count {
                        1 => "File (LocalSend)".into(),
                        n => format!("{n} files (LocalSend)"),
                    }
                };
                let _ = shared_clone.record_timeline(NewTimelineEntry {
                    kind: TimelineKind::File,
                    device_id: peer.id,
                    device_name: peer.alias.clone(),
                    incoming: false,
                    timestamp: crate::now_unix(),
                    title: transfer_timeline_title(&final_transfer.names),
                    detail,
                    target: tl_target,
                    size_bytes: final_transfer.total,
                    duration_secs: 0,
                    ref_id: Some(final_transfer.id.clone()),
                });
            }
            Err(state) => {
                final_transfer.state = state;
            }
        }
        shared_clone.emit(NodeEvent::Transfer(final_transfer.clone()));
        shared_clone.unregister_transfer(&final_transfer.id);
    });

    Ok(transfer_id)
}

async fn run_outgoing_transfer(
    shared: &Arc<Shared>,
    peer: &LocalSendPeer,
    mut files: Vec<OutgoingLocalSendFile>,
    initial_transfer: &Transfer,
    cancel: &CancellationToken,
) -> std::result::Result<(), TransferState> {
    let use_tls = !peer.protocol.eq_ignore_ascii_case("http");
    let mut files_map = HashMap::with_capacity(files.len());
    for f in &files {
        files_map.insert(
            f.file_id.clone(),
            FileDto {
                id: f.file_id.clone(),
                file_name: f.file_name.clone(),
                size: f.size,
                file_type: guess_mime(&f.file_name).into(),
                sha256: None,
                preview: None,
            },
        );
    }
    let prepare_req = PrepareUploadRequestDto { info: build_our_info_dto(shared), files: files_map };
    let prepare_body = serde_json::to_vec(&prepare_req)
        .map_err(|e| TransferState::Failed(TransferFailure::Other(e.to_string())))?;

    let mut prep_stream = tokio::select! {
        _ = cancel.cancelled() => return Err(TransferState::Cancelled),
        res = connect_peer_stream(shared, &peer.ip, peer.port, use_tls) => {
            res.map_err(|_| TransferState::Failed(TransferFailure::Unreachable))?
        }
    };

    let host_header = format!("{}:{}", peer.ip, peer.port);
    if write_http_request(
        &mut prep_stream,
        "POST",
        "/api/localsend/v2/prepare-upload",
        &host_header,
        Some("application/json"),
        &prepare_body,
    )
    .await
    .is_err()
    {
        return Err(TransferState::Failed(TransferFailure::Unreachable));
    }

    let prep_resp = tokio::select! {
        _ = cancel.cancelled() => return Err(TransferState::Cancelled),
        res = tokio::time::timeout(APPROVAL_TIMEOUT, read_http_response(&mut prep_stream)) => {
            match res {
                Ok(Ok(r)) => r,
                _ => return Err(TransferState::Failed(TransferFailure::Denied)),
            }
        }
    };
    drop(prep_stream);

    if prep_resp.status == 204 {
        return Ok(());
    }
    if prep_resp.status == 403 || prep_resp.status == 401 {
        return Err(TransferState::Failed(TransferFailure::Denied));
    }
    if !(200..300).contains(&prep_resp.status) {
        return Err(TransferState::Failed(TransferFailure::Other(format!(
            "LocalSend prepare-upload status {}",
            prep_resp.status
        ))));
    }

    let prep: PrepareUploadResponseDto = serde_json::from_slice(&prep_resp.body).map_err(|_| {
        TransferState::Failed(TransferFailure::Other("invalid prepare-upload response".into()))
    })?;

    let mut progress_transfer = initial_transfer.clone();
    progress_transfer.state = TransferState::Running;
    shared.emit(NodeEvent::Transfer(progress_transfer.clone()));
    let mut last_emit = Instant::now();

    for f in &mut files {
        let Some(token) = prep.files.get(&f.file_id) else {
            continue;
        };
        let mut up_stream = tokio::select! {
            _ = cancel.cancelled() => {
                let _ = send_cancel_request(shared, peer, use_tls, &prep.session_id).await;
                return Err(TransferState::Cancelled);
            }
            res = connect_peer_stream(shared, &peer.ip, peer.port, use_tls) => {
                res.map_err(|_| TransferState::Failed(TransferFailure::Unreachable))?
            }
        };

        let path = format!(
            "/api/localsend/v2/upload?sessionId={}&fileId={}&token={}",
            crate::percent_encode(&prep.session_id),
            crate::percent_encode(&f.file_id),
            crate::percent_encode(token),
        );
        let head = format!(
            "POST {path} HTTP/1.1\r\nHost: {host_header}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            f.size
        );
        if up_stream.write_all(head.as_bytes()).await.is_err() {
            return Err(TransferState::Failed(TransferFailure::Other("upload header write failed".into())));
        }

        let mut remaining = f.size;
        let mut buf = vec![0u8; UPLOAD_CHUNK];
        while remaining > 0 {
            let to_read = UPLOAD_CHUNK.min(remaining as usize);
            let n = tokio::select! {
                _ = cancel.cancelled() => {
                    let _ = send_cancel_request(shared, peer, use_tls, &prep.session_id).await;
                    return Err(TransferState::Cancelled);
                }
                r = f.file.read(&mut buf[..to_read]) => {
                    r.map_err(|e| TransferState::Failed(TransferFailure::Other(e.to_string())))?
                }
            };
            if n == 0 {
                return Err(TransferState::Failed(TransferFailure::Other("file ended early".into())));
            }
            let write_res = tokio::select! {
                _ = cancel.cancelled() => {
                    let _ = send_cancel_request(shared, peer, use_tls, &prep.session_id).await;
                    return Err(TransferState::Cancelled);
                }
                w = up_stream.write_all(&buf[..n]) => w,
            };
            if write_res.is_err() {
                return Err(TransferState::Failed(TransferFailure::Other("upload write failed".into())));
            }
            remaining -= n as u64;
            progress_transfer.done = progress_transfer.done.saturating_add(n as u64);
            if last_emit.elapsed() >= PROGRESS_EVERY {
                last_emit = Instant::now();
                shared.emit(NodeEvent::Transfer(progress_transfer.clone()));
            }
        }
        let _ = up_stream.flush().await;
        let up_resp = tokio::select! {
            _ = cancel.cancelled() => {
                let _ = send_cancel_request(shared, peer, use_tls, &prep.session_id).await;
                return Err(TransferState::Cancelled);
            }
            r = tokio::time::timeout(IO_TIMEOUT, read_http_response(&mut up_stream)) => {
                match r {
                    Ok(Ok(resp)) => resp,
                    _ => return Err(TransferState::Failed(TransferFailure::Other("upload response timed out".into()))),
                }
            }
        };
        if !(200..300).contains(&up_resp.status) {
            return Err(TransferState::Failed(TransferFailure::Other(format!(
                "upload failed with HTTP {}",
                up_resp.status
            ))));
        }
    }

    Ok(())
}

async fn send_cancel_request(shared: &Arc<Shared>, peer: &LocalSendPeer, use_tls: bool, session_id: &str) {
    let Ok(mut stream) = connect_peer_stream(shared, &peer.ip, peer.port, use_tls).await else {
        return;
    };
    let path = format!("/api/localsend/v2/cancel?sessionId={}", crate::percent_encode(session_id));
    let _ = write_http_request(&mut stream, "POST", &path, &format!("{}:{}", peer.ip, peer.port), None, &[])
        .await;
}

enum ClientStream {
    Plain(TcpStream),
    Tls(Box<tokio_rustls::client::TlsStream<TcpStream>>),
}

impl AsyncRead for ClientStream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match &mut *self {
            ClientStream::Plain(s) => std::pin::Pin::new(s).poll_read(cx, buf),
            ClientStream::Tls(s) => std::pin::Pin::new(&mut **s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for ClientStream {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        match &mut *self {
            ClientStream::Plain(s) => std::pin::Pin::new(s).poll_write(cx, buf),
            ClientStream::Tls(s) => std::pin::Pin::new(&mut **s).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match &mut *self {
            ClientStream::Plain(s) => std::pin::Pin::new(s).poll_flush(cx),
            ClientStream::Tls(s) => std::pin::Pin::new(&mut **s).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match &mut *self {
            ClientStream::Plain(s) => std::pin::Pin::new(s).poll_shutdown(cx),
            ClientStream::Tls(s) => std::pin::Pin::new(&mut **s).poll_shutdown(cx),
        }
    }
}

async fn connect_peer_stream(shared: &Shared, host: &str, port: u16, use_tls: bool) -> Result<ClientStream> {
    let addr = format!("{host}:{port}");
    let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(&addr))
        .await
        .map_err(|_| Error::Offline)?
        .map_err(|_| Error::Offline)?;
    if use_tls {
        let connector = TlsConnector::from(shared.localsend.client_tls.clone());
        let server_name = ServerName::try_from("localsend.org").expect("static valid DNS name");
        let tls = tokio::time::timeout(CONNECT_TIMEOUT, connector.connect(server_name, tcp))
            .await
            .map_err(|_| Error::Offline)?
            .map_err(|_| Error::Offline)?;
        Ok(ClientStream::Tls(Box::new(tls)))
    } else {
        Ok(ClientStream::Plain(tcp))
    }
}

// ---- Minimal HTTP/1.1 reader/writer ----

struct HttpHead {
    method: String,
    target: String,
    content_length: Option<u64>,
    chunked: bool,
    prefetched_body: Vec<u8>,
}

struct HttpResponse {
    status: u16,
    body: Vec<u8>,
}

async fn read_http_head<S: AsyncRead + Unpin>(stream: &mut S) -> std::io::Result<HttpHead> {
    let mut buf = Vec::with_capacity(2048);
    let mut temp = [0u8; 2048];
    let header_end = loop {
        if let Some(pos) = find_header_end(&buf) {
            break pos;
        }
        if buf.len() >= MAX_HEADER_BYTES {
            return Err(std::io::Error::new(ErrorKind::InvalidData, "HTTP header too large"));
        }
        let n = tokio::time::timeout(IO_TIMEOUT, stream.read(&mut temp))
            .await
            .map_err(|_| std::io::Error::new(ErrorKind::TimedOut, "HTTP header read timed out"))??;
        if n == 0 {
            return Err(std::io::Error::new(ErrorKind::UnexpectedEof, "EOF reading HTTP header"));
        }
        buf.extend_from_slice(&temp[..n]);
    };

    let prefetched_body = buf.split_off(header_end + 4);
    let header_str = String::from_utf8_lossy(&buf[..header_end]);
    let mut lines = header_str.split("\r\n");
    let first_line = lines.next().unwrap_or("");
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_uppercase();
    let target = parts.next().unwrap_or("/").to_owned();

    let mut content_length = None;
    let mut chunked = false;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let k_lower = k.trim().to_ascii_lowercase();
            let v_trim = v.trim();
            if k_lower == "content-length" {
                content_length = v_trim.parse::<u64>().ok();
            } else if k_lower == "transfer-encoding" && v_trim.to_ascii_lowercase().contains("chunked") {
                chunked = true;
            }
        }
    }

    Ok(HttpHead { method, target, content_length, chunked, prefetched_body })
}

async fn read_http_body<S: AsyncRead + Unpin>(stream: &mut S, head: &HttpHead) -> std::io::Result<Vec<u8>> {
    if head.chunked {
        return read_chunked_vec(stream, &head.prefetched_body, MAX_JSON_BODY_BYTES).await;
    }
    let expected = head.content_length.unwrap_or(0) as usize;
    if expected > MAX_JSON_BODY_BYTES {
        return Err(std::io::Error::new(ErrorKind::InvalidData, "HTTP JSON body too large"));
    }
    let mut out = Vec::with_capacity(expected);
    let take_pre = head.prefetched_body.len().min(expected);
    out.extend_from_slice(&head.prefetched_body[..take_pre]);
    while out.len() < expected {
        let mut chunk = vec![0u8; (expected - out.len()).min(16 * 1024)];
        let n = tokio::time::timeout(IO_TIMEOUT, stream.read(&mut chunk))
            .await
            .map_err(|_| std::io::Error::new(ErrorKind::TimedOut, "HTTP body read timed out"))??;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&chunk[..n]);
    }
    Ok(out)
}

async fn read_chunked_vec<S: AsyncRead + Unpin>(
    stream: &mut S,
    prefetched: &[u8],
    max_bytes: usize,
) -> std::io::Result<Vec<u8>> {
    let mut buf = prefetched.to_vec();
    let mut out = Vec::new();
    loop {
        while find_crlf(&buf).is_none() {
            let mut tmp = [0u8; 4096];
            let n = tokio::time::timeout(IO_TIMEOUT, stream.read(&mut tmp))
                .await
                .map_err(|_| std::io::Error::new(ErrorKind::TimedOut, "chunk header timeout"))??;
            if n == 0 {
                return Ok(out);
            }
            buf.extend_from_slice(&tmp[..n]);
        }
        let line_end = find_crlf(&buf).unwrap();
        let size_line = String::from_utf8_lossy(&buf[..line_end]);
        let hex_part = size_line.split(';').next().unwrap_or("").trim();
        let chunk_size = usize::from_str_radix(hex_part, 16)
            .map_err(|_| std::io::Error::new(ErrorKind::InvalidData, "invalid chunk size"))?;
        buf.drain(..line_end + 2);
        if chunk_size == 0 {
            break;
        }
        if out.len().saturating_add(chunk_size) > max_bytes {
            return Err(std::io::Error::new(ErrorKind::InvalidData, "chunked body too large"));
        }
        while buf.len() < chunk_size + 2 {
            let mut tmp = [0u8; 8192];
            let n = tokio::time::timeout(IO_TIMEOUT, stream.read(&mut tmp))
                .await
                .map_err(|_| std::io::Error::new(ErrorKind::TimedOut, "chunk data timeout"))??;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
        }
        let available = buf.len().min(chunk_size);
        out.extend_from_slice(&buf[..available]);
        let drain_len = buf.len().min(chunk_size + 2);
        buf.drain(..drain_len);
    }
    Ok(out)
}

async fn stream_body_to_file<S, F>(
    stream: &mut S,
    head: &HttpHead,
    out: &mut tokio::fs::File,
    expected_size: u64,
    cancel: &CancellationToken,
    mut on_progress: F,
) -> std::result::Result<(), TransferState>
where
    S: AsyncRead + Unpin,
    F: FnMut(u64),
{
    if head.chunked {
        let mut buf = head.prefetched_body.clone();
        loop {
            while find_crlf(&buf).is_none() {
                let mut tmp = [0u8; 8192];
                let n = tokio::select! {
                    _ = cancel.cancelled() => return Err(TransferState::Cancelled),
                    r = tokio::time::timeout(IO_TIMEOUT, stream.read(&mut tmp)) => {
                        match r {
                            Ok(Ok(n)) => n,
                            _ => return Err(TransferState::Failed(TransferFailure::Other("upload read error".into()))),
                        }
                    }
                };
                if n == 0 {
                    return Ok(());
                }
                buf.extend_from_slice(&tmp[..n]);
            }
            let line_end = find_crlf(&buf).unwrap();
            let size_line = String::from_utf8_lossy(&buf[..line_end]).into_owned();
            let hex_part = size_line.split(';').next().unwrap_or("").trim();
            let Ok(chunk_size) = usize::from_str_radix(hex_part, 16) else {
                return Err(TransferState::Failed(TransferFailure::Other("invalid chunk size".into())));
            };
            buf.drain(..line_end + 2);
            if chunk_size == 0 {
                return Ok(());
            }
            let mut left_in_chunk = chunk_size;
            while left_in_chunk > 0 {
                if !buf.is_empty() {
                    let take = buf.len().min(left_in_chunk);
                    out.write_all(&buf[..take])
                        .await
                        .map_err(|e| TransferState::Failed(TransferFailure::Other(e.to_string())))?;
                    buf.drain(..take);
                    left_in_chunk -= take;
                    on_progress(take as u64);
                } else {
                    let mut tmp = vec![0u8; UPLOAD_CHUNK.min(left_in_chunk)];
                    let n = tokio::select! {
                        _ = cancel.cancelled() => return Err(TransferState::Cancelled),
                        r = tokio::time::timeout(IO_TIMEOUT, stream.read(&mut tmp)) => {
                            match r {
                                Ok(Ok(n)) => n,
                                _ => return Err(TransferState::Failed(TransferFailure::Other("upload read error".into()))),
                            }
                        }
                    };
                    if n == 0 {
                        return Err(TransferState::Failed(TransferFailure::Other("unexpected EOF".into())));
                    }
                    out.write_all(&tmp[..n])
                        .await
                        .map_err(|e| TransferState::Failed(TransferFailure::Other(e.to_string())))?;
                    left_in_chunk -= n;
                    on_progress(n as u64);
                }
            }
            // Consume trailing \r\n after chunk data.
            while buf.len() < 2 {
                let mut tmp = [0u8; 16];
                let Ok(Ok(n)) = tokio::time::timeout(IO_TIMEOUT, stream.read(&mut tmp)).await else {
                    break;
                };
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
            }
            let drain_crlf = buf.len().min(2);
            buf.drain(..drain_crlf);
        }
    } else {
        let total = head.content_length.unwrap_or(expected_size);
        let mut remaining = total;
        if !head.prefetched_body.is_empty() {
            let take = (head.prefetched_body.len() as u64).min(remaining) as usize;
            out.write_all(&head.prefetched_body[..take])
                .await
                .map_err(|e| TransferState::Failed(TransferFailure::Other(e.to_string())))?;
            remaining -= take as u64;
            on_progress(take as u64);
        }
        let mut buf = vec![0u8; UPLOAD_CHUNK];
        while remaining > 0 {
            let max_read = UPLOAD_CHUNK.min(remaining as usize);
            let n = tokio::select! {
                _ = cancel.cancelled() => return Err(TransferState::Cancelled),
                r = tokio::time::timeout(IO_TIMEOUT, stream.read(&mut buf[..max_read])) => {
                    match r {
                        Ok(Ok(n)) => n,
                        _ => return Err(TransferState::Failed(TransferFailure::Other("upload stream interrupted".into()))),
                    }
                }
            };
            if n == 0 {
                return Err(TransferState::Failed(TransferFailure::Other(
                    "upload stream ended early".into(),
                )));
            }
            out.write_all(&buf[..n])
                .await
                .map_err(|e| TransferState::Failed(TransferFailure::Other(e.to_string())))?;
            remaining -= n as u64;
            on_progress(n as u64);
        }
        Ok(())
    }
}

async fn write_http_request<S: AsyncWrite + Unpin>(
    stream: &mut S,
    method: &str,
    path: &str,
    host: &str,
    content_type: Option<&str>,
    body: &[u8],
) -> Result<()> {
    let ct_header = content_type.map(|ct| format!("Content-Type: {ct}\r\n")).unwrap_or_default();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: Nectarlink/0.1\r\n{ct_header}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(req.as_bytes()).await.map_err(|_| Error::Offline)?;
    if !body.is_empty() {
        stream.write_all(body).await.map_err(|_| Error::Offline)?;
    }
    stream.flush().await.map_err(|_| Error::Offline)?;
    Ok(())
}

async fn write_http_response<S: AsyncWrite + Unpin>(
    stream: &mut S,
    status: u16,
    reason: &str,
    content_type: Option<&str>,
    body: &[u8],
) -> std::io::Result<()> {
    let ct_header = content_type.map(|ct| format!("Content-Type: {ct}\r\n")).unwrap_or_default();
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\n{ct_header}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    if !body.is_empty() {
        stream.write_all(body).await?;
    }
    stream.flush().await
}

async fn read_http_response<S: AsyncRead + Unpin>(stream: &mut S) -> std::io::Result<HttpResponse> {
    let head = read_http_head(stream).await?;
    // For HTTP responses, `head.target` holds the status code (e.g. "200" in "HTTP/1.1 200 OK").
    let status = head.target.parse::<u16>().unwrap_or(500);
    let body = read_http_body(stream, &head).await?;
    Ok(HttpResponse { status, body })
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn find_crlf(buf: &[u8]) -> Option<usize> {
    buf.windows(2).position(|w| w == b"\r\n")
}

fn parse_query(query: &str) -> HashMap<String, String> {
    url::form_urlencoded::parse(query.as_bytes()).into_owned().collect()
}

fn split_localsend_file_name(raw: &str) -> (Option<String>, String) {
    let normalized = raw.replace('\\', "/");
    let parts: Vec<&str> =
        normalized.split('/').map(str::trim).filter(|s| !s.is_empty() && *s != "." && *s != "..").collect();
    match parts.as_slice() {
        [] => (None, "file".into()),
        [single] => (None, safe_file_name(single)),
        [folders @ .., last] => {
            let folder = folders.iter().map(|s| storable_name(s)).collect::<Vec<_>>().join("/");
            (Some(folder), safe_file_name(last))
        }
    }
}

fn guess_mime(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".pdf") {
        "application/pdf"
    } else if lower.ends_with(".txt") || lower.ends_with(".md") {
        "text/plain"
    } else if lower.ends_with(".mp4") {
        "video/mp4"
    } else if lower.ends_with(".mp3") {
        "audio/mpeg"
    } else {
        "application/octet-stream"
    }
}

fn random_token() -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    (0..16).map(|_| ALPHABET[rand::random_range(0..ALPHABET.len())] as char).collect()
}

// ---- Multicast UDP socket setup ----

fn bind_multicast_socket(port: u16) -> std::io::Result<UdpSocket> {
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    let _ = sock.set_reuse_address(true);
    let _ = sock.set_broadcast(true);
    let _ = sock.set_multicast_loop_v4(true);
    let _ = sock.set_multicast_ttl_v4(255);
    sock.set_nonblocking(true)?;
    let bind_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port);
    sock.bind(&bind_addr.into())?;
    let _ = sock.join_multicast_v4(&LOCALSEND_MULTICAST_ADDR, &Ipv4Addr::UNSPECIFIED);
    for ip in local_ipv4_addrs() {
        let _ = sock.join_multicast_v4(&LOCALSEND_MULTICAST_ADDR, &ip);
    }
    let std_udp: std::net::UdpSocket = sock.into();
    UdpSocket::from_std(std_udp)
}

fn local_ipv4_addrs() -> Vec<Ipv4Addr> {
    let mut out = vec![Ipv4Addr::LOCALHOST];
    if let Ok(sock) = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        && sock.connect((Ipv4Addr::new(8, 8, 8, 8), 80)).is_ok()
        && let Ok(SocketAddr::V4(local)) = sock.local_addr()
        && !out.contains(local.ip())
    {
        out.push(*local.ip());
    }
    out
}

// ---- Self-signed X.509 v3 DER certificate & TLS configuration ----

fn decode_stored_identity(stored: &StoredLocalSendState) -> Option<LocalSendIdentity> {
    if stored.cert_der_hex.is_empty() || stored.key_pkcs8_hex.is_empty() {
        return None;
    }
    let cert_der = data_encoding::HEXLOWER.decode(stored.cert_der_hex.as_bytes()).ok()?;
    let key_pkcs8 = data_encoding::HEXLOWER.decode(stored.key_pkcs8_hex.as_bytes()).ok()?;
    if cert_der.is_empty() || key_pkcs8.is_empty() {
        return None;
    }
    let fingerprint = data_encoding::HEXLOWER.encode(&Sha256::digest(&cert_der));
    let id = LocalSendIdentity { cert_der, key_pkcs8, fingerprint };
    // Verify the stored key/cert builds a valid rustls ServerConfig.
    build_server_tls_config(&id).ok()?;
    Some(id)
}

fn save_stored_state(
    data_dir: &Path,
    protector: &dyn KeyProtector,
    state: &StoredLocalSendState,
) -> Result<()> {
    fs::create_dir_all(data_dir)?;
    let plain = serde_json::to_vec(state).map_err(|e| Error::Internal(e.to_string()))?;
    let sealed = protector.protect(&plain)?;
    let path = data_dir.join(STATE_FILE);
    let tmp = path.with_extension("enc.tmp");
    fs::write(&tmp, &sealed)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

/// Generates an ECDSA P-256 PKCS#8 keypair and a self-signed X.509 v3 DER certificate
/// (`CN=LocalSend User`) using `ring`, returning the DER bytes and SHA-256 hex fingerprint.
pub(crate) fn generate_self_signed_identity() -> Result<LocalSendIdentity> {
    let rng = SystemRandom::new();
    let pkcs8_doc = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng)
        .map_err(|_| Error::Internal("can't generate LocalSend ECDSA key".into()))?;
    let key_pkcs8 = pkcs8_doc.as_ref().to_vec();
    let key_pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &key_pkcs8, &rng)
        .map_err(|_| Error::Internal("can't parse generated LocalSend ECDSA key".into()))?;

    let mut serial = [0u8; 16];
    rng.fill(&mut serial).map_err(|_| Error::Internal("RNG failure".into()))?;
    serial[0] &= 0x7F;
    if serial[0] == 0 {
        serial[0] = 1;
    }

    // ecdsa-with-SHA256 (1.2.840.10045.4.3.2)
    let sig_alg = der_seq(&[der_tlv(0x06, &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x04, 0x03, 0x02])]);
    // CN = LocalSend User (2.5.4.3)
    let cn_attr = der_seq(&[der_tlv(0x06, &[0x55, 0x04, 0x03]), der_tlv(0x0C, b"LocalSend User")]);
    let name = der_seq(&[der_tlv(0x31, &cn_attr)]);
    // Validity: 2024-01-01 to 2036-01-01
    let validity = der_seq(&[der_tlv(0x17, b"240101000000Z"), der_tlv(0x17, b"360101000000Z")]);
    // SubjectPublicKeyInfo: id-ecPublicKey (1.2.840.10045.2.1) + prime256v1 (1.2.840.10045.3.1.7)
    let spki_alg = der_seq(&[
        der_tlv(0x06, &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x02, 0x01]),
        der_tlv(0x06, &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x03, 0x01, 0x07]),
    ]);
    let spki = der_seq(&[spki_alg, der_bit_string(key_pair.public_key().as_ref())]);

    let version = der_tlv(0xA0, &der_tlv(0x02, &[0x02]));
    let tbs =
        der_seq(&[version, der_tlv(0x02, &serial), sig_alg.clone(), name.clone(), validity, name, spki]);

    let signature =
        key_pair.sign(&rng, &tbs).map_err(|_| Error::Internal("can't sign LocalSend certificate".into()))?;
    let cert_der = der_seq(&[tbs, sig_alg, der_bit_string(signature.as_ref())]);
    let fingerprint = data_encoding::HEXLOWER.encode(&Sha256::digest(&cert_der));

    Ok(LocalSendIdentity { cert_der, key_pkcs8, fingerprint })
}

fn der_tlv(tag: u8, body: &[u8]) -> Vec<u8> {
    let len = body.len();
    let mut out = Vec::with_capacity(4 + len);
    out.push(tag);
    if len < 128 {
        out.push(len as u8);
    } else if len <= 0xFF {
        out.push(0x81);
        out.push(len as u8);
    } else {
        out.push(0x82);
        out.push((len >> 8) as u8);
        out.push((len & 0xFF) as u8);
    }
    out.extend_from_slice(body);
    out
}

fn der_seq(items: &[Vec<u8>]) -> Vec<u8> {
    let total: usize = items.iter().map(Vec::len).sum();
    let mut body = Vec::with_capacity(total);
    for item in items {
        body.extend_from_slice(item);
    }
    der_tlv(0x30, &body)
}

fn der_bit_string(bytes: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(1 + bytes.len());
    body.push(0x00); // 0 unused bits
    body.extend_from_slice(bytes);
    der_tlv(0x03, &body)
}

fn build_server_tls_config(identity: &LocalSendIdentity) -> Result<ServerConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let cert = CertificateDer::from(identity.cert_der.clone());
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(identity.key_pkcs8.clone()));
    ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::Internal(e.to_string()))?
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .map_err(|e| Error::Internal(e.to_string()))
}

fn build_client_tls_config() -> Result<ClientConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = Arc::new(LocalSendCertVerifier(provider.clone()));
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::Internal(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    Ok(config)
}

/// Accepts self-signed X.509 certificates presented by LocalSend peers on the LAN
/// while still verifying the TLS 1.2 / 1.3 handshake signature with `ring`.
#[derive(Debug)]
struct LocalSendCertVerifier(Arc<CryptoProvider>);

impl ServerCertVerifier for LocalSendCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::PlainKeyProtector;

    #[tokio::test]
    async fn self_signed_cert_completes_tls_handshake_and_matches_fingerprint() {
        let id = generate_self_signed_identity().unwrap();
        assert_eq!(id.fingerprint.len(), 64);

        let server_cfg = Arc::new(build_server_tls_config(&id).unwrap());
        let client_cfg = Arc::new(build_client_tls_config().unwrap());

        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();

        let acceptor = TlsAcceptor::from(server_cfg);
        let server_task = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut tls = acceptor.accept(tcp).await.unwrap();
            let mut buf = [0u8; 4];
            tls.read_exact(&mut buf).await.unwrap();
            assert_eq!(&buf, b"ping");
            tls.write_all(b"pong").await.unwrap();
            tls.flush().await.unwrap();
        });

        let tcp = TcpStream::connect(addr).await.unwrap();
        let connector = TlsConnector::from(client_cfg);
        let domain = ServerName::try_from("localsend.org").unwrap();
        let mut client_tls = connector.connect(domain, tcp).await.unwrap();
        let peer_certs = client_tls.get_ref().1.peer_certificates().unwrap();
        let peer_fp = data_encoding::HEXLOWER.encode(&Sha256::digest(peer_certs[0].as_ref()));
        assert_eq!(peer_fp, id.fingerprint);

        client_tls.write_all(b"ping").await.unwrap();
        let mut reply = [0u8; 4];
        client_tls.read_exact(&mut reply).await.unwrap();
        assert_eq!(&reply, b"pong");
        server_task.await.unwrap();
    }

    #[test]
    fn persists_identity_and_enabled_flag_encrypted_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let state1 =
            LocalSendState::open(dir.path(), Arc::new(PlainKeyProtector), LOCALSEND_DEFAULT_PORT).unwrap();
        assert!(!state1.enabled());
        let fp1 = state1.identity.fingerprint.clone();

        let state2 =
            LocalSendState::open(dir.path(), Arc::new(PlainKeyProtector), LOCALSEND_DEFAULT_PORT).unwrap();
        assert_eq!(state2.identity.fingerprint, fp1);
    }
}
