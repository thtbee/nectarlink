// SPDX-License-Identifier: MPL-2.0
//! UniFFI bindings for `nectarlink-core`, used by the Android app.
//!
//! The boundary uses plain records and enums (see
//! docs/architecture/core-api.md). The node runs on its own Tokio runtime;
//! every async method executes there, so the foreign side can await it from
//! any executor (Kotlin coroutines). Events are delivered to a foreign
//! listener on a core thread; the listener should hand them off quickly.

use std::{path::PathBuf, sync::Arc, time::Duration};

use nectarlink_core as core;
use nectarlink_core::{
    ConnectionPath, DeviceId, LinkState, Node, NodeConfig, NodeEvent, PairingEvent,
    features::{CapabilityMatrix, FeatureState},
};
use tokio::{runtime::Runtime, sync::broadcast::error::RecvError};

uniffi::setup_scaffolding!();

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

// ---- Records and enums ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DeviceKind {
    Phone,
    Tablet,
    Desktop,
    Laptop,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DeviceInfo {
    pub name: String,
    pub kind: DeviceKind,
    /// "android", "windows" or "macos".
    pub os: String,
    pub os_version: String,
    pub model: Option<String>,
    /// ARGB seed color for Material You sync.
    pub accent: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PowerLevel {
    Basic,
    Assist,
    Elevated,
    /// Desktops.
    NotApplicable,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Battery {
    /// 0–100.
    pub level: u8,
    pub charging: bool,
    /// "ac", "usb" or "wireless".
    pub plugged: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum Link {
    /// `last_seen` is in Unix seconds.
    Offline {
        last_seen: Option<i64>,
    },
    Connecting,
    Online {
        relayed: bool,
        rtt_ms: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PairedDevice {
    /// The device ID (z-base-32).
    pub id: String,
    pub info: DeviceInfo,
    /// Unix seconds.
    pub paired_at: i64,
    pub link: Link,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DiscoveredDevice {
    pub id: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum PairingFailure {
    Rejected,
    Declined,
    Expired,
    Unreachable,
    Other { message: String },
}

/// What unlocks a feature (see docs/architecture/capabilities.md).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Upgrade {
    /// "raisePower", "grantPermission", "enableAddon", "enablePath",
    /// "enableToggle" or "updateApp".
    pub action: String,
    /// E.g. "elevated", "notification_access", "clipboard", "phone".
    pub target: String,
    /// Roughly how long it takes; 0 for instant.
    pub minutes: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum FeatureStatus {
    Available,
    Partial {
        limit: String,
        upgrade: Option<Upgrade>,
    },
    Locked {
        upgrade: Upgrade,
    },
    /// `reason`: "deviceKinds", "android:<release>", "windows:<build>" or
    /// "notOnThisDevice".
    Unsupported {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Feature {
    pub id: String,
    pub status: FeatureStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DeviceToggle {
    pub name: String,
    pub enabled: bool,
}

/// Everything the UI needs to know about, as it happens.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum Event {
    DeviceAdded {
        device: PairedDevice,
    },
    DeviceRemoved {
        id: String,
    },
    LinkChanged {
        id: String,
        link: Link,
    },
    PeerInfoChanged {
        id: String,
        info: DeviceInfo,
    },
    PeerPowerChanged {
        id: String,
        power: PowerLevel,
    },
    Battery {
        id: String,
        battery: Battery,
    },
    /// A device asked this one to start (or stop) ringing.
    Ring {
        id: String,
        on: bool,
    },
    Discovered {
        device: DiscoveredDevice,
    },
    DiscoveryExpired {
        id: String,
    },
    /// Show this 6-digit code and ask the user whether it matches.
    PairingCode {
        peer: String,
        code: String,
    },
    Paired {
        device: PairedDevice,
    },
    PairingFailed {
        failure: PairingFailure,
    },
    Capabilities {
        id: String,
        features: Vec<Feature>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum NectarlinkError {
    #[error("device is not paired")]
    NotPaired,
    #[error("device is offline")]
    Offline,
    #[error("the operation was denied")]
    Denied,
    #[error("declined")]
    Declined,
    #[error("timed out")]
    Timeout,
    /// `this_device`: whether this app (true) or the other device's app
    /// (false) needs an update.
    #[error("an app is too old for this connection")]
    VersionTooOld { this_device: bool },
    #[error("not supported by the other device")]
    Unsupported,
    #[error("no pairing in progress")]
    NotPairing,
    #[error("invalid pairing link")]
    InvalidPairingLink,
    #[error("invalid device ID")]
    InvalidDeviceId,
    #[error("network error: {message}")]
    Network { message: String },
    #[error("storage error: {message}")]
    Storage { message: String },
    #[error("internal error: {message}")]
    Internal { message: String },
}

impl From<core::Error> for NectarlinkError {
    fn from(e: core::Error) -> Self {
        match e {
            core::Error::NotPaired => NectarlinkError::NotPaired,
            core::Error::Offline => NectarlinkError::Offline,
            core::Error::Denied => NectarlinkError::Denied,
            core::Error::Declined => NectarlinkError::Declined,
            core::Error::Timeout => NectarlinkError::Timeout,
            core::Error::VersionTooOld { side } => {
                NectarlinkError::VersionTooOld { this_device: side == core::Side::Local }
            }
            core::Error::Unsupported => NectarlinkError::Unsupported,
            core::Error::NotPairing => NectarlinkError::NotPairing,
            core::Error::InvalidPairingLink(_) => NectarlinkError::InvalidPairingLink,
            core::Error::Network(message) => NectarlinkError::Network { message },
            core::Error::Storage(message) => NectarlinkError::Storage { message },
            core::Error::Io(e) => NectarlinkError::Storage { message: e.to_string() },
            core::Error::Protocol(message) | core::Error::Internal(message) => {
                NectarlinkError::Internal { message }
            }
        }
    }
}

type Result<T, E = NectarlinkError> = std::result::Result<T, E>;

// ---- Conversions ----

impl From<core::DeviceKind> for DeviceKind {
    fn from(kind: core::DeviceKind) -> Self {
        match kind {
            core::DeviceKind::Phone => DeviceKind::Phone,
            core::DeviceKind::Tablet => DeviceKind::Tablet,
            core::DeviceKind::Desktop => DeviceKind::Desktop,
            core::DeviceKind::Laptop => DeviceKind::Laptop,
            core::DeviceKind::Unknown => DeviceKind::Unknown,
        }
    }
}

impl From<DeviceKind> for core::DeviceKind {
    fn from(kind: DeviceKind) -> Self {
        match kind {
            DeviceKind::Phone => core::DeviceKind::Phone,
            DeviceKind::Tablet => core::DeviceKind::Tablet,
            DeviceKind::Desktop => core::DeviceKind::Desktop,
            DeviceKind::Laptop => core::DeviceKind::Laptop,
            DeviceKind::Unknown => core::DeviceKind::Unknown,
        }
    }
}

impl From<core::DeviceInfo> for DeviceInfo {
    fn from(i: core::DeviceInfo) -> Self {
        DeviceInfo {
            name: i.name,
            kind: i.kind.into(),
            os: i.os,
            os_version: i.os_ver,
            model: i.model,
            accent: i.accent,
        }
    }
}

impl From<DeviceInfo> for core::DeviceInfo {
    fn from(i: DeviceInfo) -> Self {
        core::DeviceInfo {
            name: i.name,
            kind: i.kind.into(),
            os: i.os,
            os_ver: i.os_version,
            model: i.model,
            accent: i.accent,
        }
    }
}

impl From<core::PowerLevel> for PowerLevel {
    fn from(p: core::PowerLevel) -> Self {
        match p.effective() {
            core::PowerLevel::Assist => PowerLevel::Assist,
            core::PowerLevel::Elevated => PowerLevel::Elevated,
            core::PowerLevel::NotApplicable => PowerLevel::NotApplicable,
            core::PowerLevel::Basic | core::PowerLevel::Unknown => PowerLevel::Basic,
        }
    }
}

impl From<PowerLevel> for core::PowerLevel {
    fn from(p: PowerLevel) -> Self {
        match p {
            PowerLevel::Basic => core::PowerLevel::Basic,
            PowerLevel::Assist => core::PowerLevel::Assist,
            PowerLevel::Elevated => core::PowerLevel::Elevated,
            PowerLevel::NotApplicable => core::PowerLevel::NotApplicable,
        }
    }
}

impl From<core::Battery> for Battery {
    fn from(b: core::Battery) -> Self {
        Battery { level: b.level, charging: b.charging, plugged: b.plugged }
    }
}

impl From<Battery> for core::Battery {
    fn from(b: Battery) -> Self {
        core::Battery { level: b.level.min(100), charging: b.charging, plugged: b.plugged }
    }
}

impl From<LinkState> for Link {
    fn from(l: LinkState) -> Self {
        match l {
            LinkState::Offline { last_seen } => Link::Offline { last_seen },
            LinkState::Connecting => Link::Connecting,
            LinkState::Online { path, rtt_ms } => {
                Link::Online { relayed: path == ConnectionPath::Relay, rtt_ms }
            }
        }
    }
}

impl From<core::PairedDevice> for PairedDevice {
    fn from(d: core::PairedDevice) -> Self {
        PairedDevice {
            id: d.id.to_string(),
            info: d.info.into(),
            paired_at: d.paired_at,
            link: d.link.into(),
        }
    }
}

impl From<core::DiscoveredDevice> for DiscoveredDevice {
    fn from(d: core::DiscoveredDevice) -> Self {
        DiscoveredDevice { id: d.id.to_string(), name: d.name }
    }
}

impl From<core::PairingFailure> for PairingFailure {
    fn from(f: core::PairingFailure) -> Self {
        match f {
            core::PairingFailure::Rejected => PairingFailure::Rejected,
            core::PairingFailure::Declined => PairingFailure::Declined,
            core::PairingFailure::Expired => PairingFailure::Expired,
            core::PairingFailure::Unreachable => PairingFailure::Unreachable,
            core::PairingFailure::Other(message) => PairingFailure::Other { message },
        }
    }
}

fn upgrade(u: core::features::Upgrade) -> Upgrade {
    let (action, target) = u.action.describe();
    Upgrade { action: action.into(), target, minutes: u.effort.minutes() }
}

impl From<FeatureState> for FeatureStatus {
    fn from(state: FeatureState) -> Self {
        match state {
            FeatureState::Available => FeatureStatus::Available,
            FeatureState::Partial { limit, upgrade: u } => {
                FeatureStatus::Partial { limit: limit.into(), upgrade: u.map(upgrade) }
            }
            FeatureState::Locked { upgrade: u } => FeatureStatus::Locked { upgrade: upgrade(u) },
            FeatureState::Unsupported { reason } => FeatureStatus::Unsupported { reason: reason.describe() },
        }
    }
}

fn features(matrix: &CapabilityMatrix) -> Vec<Feature> {
    matrix.features.iter().map(|(id, state)| Feature { id: (*id).into(), status: (*state).into() }).collect()
}

impl From<NodeEvent> for Event {
    fn from(event: NodeEvent) -> Self {
        match event {
            NodeEvent::DeviceAdded(device) => Event::DeviceAdded { device: device.into() },
            NodeEvent::DeviceRemoved(id) => Event::DeviceRemoved { id: id.to_string() },
            NodeEvent::LinkChanged { device, link } => {
                Event::LinkChanged { id: device.to_string(), link: link.into() }
            }
            NodeEvent::PeerInfoChanged { device, info } => {
                Event::PeerInfoChanged { id: device.to_string(), info: info.into() }
            }
            NodeEvent::PeerPowerChanged { device, power } => {
                Event::PeerPowerChanged { id: device.to_string(), power: power.into() }
            }
            NodeEvent::Battery { device, battery } => {
                Event::Battery { id: device.to_string(), battery: battery.into() }
            }
            NodeEvent::Ring { device, on } => Event::Ring { id: device.to_string(), on },
            NodeEvent::Discovered(device) => Event::Discovered { device: device.into() },
            NodeEvent::DiscoveryExpired(id) => Event::DiscoveryExpired { id: id.to_string() },
            NodeEvent::Pairing(PairingEvent::SasCode { peer, code }) => {
                Event::PairingCode { peer: peer.to_string(), code }
            }
            NodeEvent::Pairing(PairingEvent::Paired(device)) => Event::Paired { device: device.into() },
            NodeEvent::Pairing(PairingEvent::Failed(failure)) => {
                Event::PairingFailed { failure: failure.into() }
            }
            NodeEvent::Capabilities(matrix) => {
                Event::Capabilities { id: matrix.device.to_string(), features: features(&matrix) }
            }
        }
    }
}

fn parse_id(id: &str) -> Result<DeviceId> {
    id.parse().map_err(|_| NectarlinkError::InvalidDeviceId)
}

// ---- Foreign interfaces (implemented in Kotlin) ----

/// What the app does for the core: ring the phone.
#[uniffi::export(with_foreign)]
pub trait Platform: Send + Sync {
    /// Ring loudly, even in silent mode, until `stop_ringing`.
    fn start_ringing(&self);
    fn stop_ringing(&self);
}

/// Encrypts the device key at rest (Android: a Keystore key).
#[uniffi::export(with_foreign)]
pub trait KeyProtector: Send + Sync {
    fn protect(&self, plaintext: Vec<u8>) -> Result<Vec<u8>>;
    fn unprotect(&self, ciphertext: Vec<u8>) -> Result<Vec<u8>>;
}

/// Receives core events on a core thread. Hand them off; don't block.
#[uniffi::export(with_foreign)]
pub trait EventListener: Send + Sync {
    fn on_event(&self, event: Event);
}

struct PlatformAdapter(Arc<dyn Platform>);

impl std::fmt::Debug for PlatformAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PlatformAdapter")
    }
}

impl core::Platform for PlatformAdapter {
    fn start_ringing(&self) {
        self.0.start_ringing();
    }
    fn stop_ringing(&self) {
        self.0.stop_ringing();
    }
}

struct KeyProtectorAdapter(Arc<dyn KeyProtector>);

/// Key files written with the Android Keystore protector.
const ANDROID_KEYSTORE_PROTECTOR: u8 = 2;

impl core::KeyProtector for KeyProtectorAdapter {
    fn id(&self) -> u8 {
        ANDROID_KEYSTORE_PROTECTOR
    }
    fn protect(&self, plaintext: &[u8]) -> std::io::Result<Vec<u8>> {
        self.0.protect(plaintext.to_vec()).map_err(std::io::Error::other)
    }
    fn unprotect(&self, ciphertext: &[u8]) -> std::io::Result<Vec<u8>> {
        self.0.unprotect(ciphertext.to_vec()).map_err(std::io::Error::other)
    }
}

// ---- The node ----

#[derive(Debug, Clone, uniffi::Record)]
pub struct NodeOptions {
    /// App-private directory for the identity and database.
    pub data_dir: String,
    pub device: DeviceInfo,
    pub app_version: String,
    pub power: PowerLevel,
    /// Allow connections through relays when away from home.
    pub away_mode: bool,
}

/// The Nectarlink engine. One per app process.
#[derive(uniffi::Object)]
pub struct NectarlinkNode {
    runtime: Runtime,
    node: Node,
}

impl std::fmt::Debug for NectarlinkNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NectarlinkNode").field("node", &self.node).finish_non_exhaustive()
    }
}

impl NectarlinkNode {
    /// Runs `fut` on the node's runtime and waits for it from the caller's
    /// executor.
    async fn run<T: Send + 'static>(&self, fut: impl Future<Output = T> + Send + 'static) -> T {
        match self.runtime.spawn(fut).await {
            Ok(value) => value,
            Err(e) => std::panic::resume_unwind(e.into_panic()),
        }
    }
}

#[uniffi::export]
impl NectarlinkNode {
    /// Loads or creates this device's identity and trust store, starts
    /// networking and starts delivering events to `listener`. Blocks for
    /// the (short) startup; call it off the main thread.
    #[uniffi::constructor]
    pub fn start(
        options: NodeOptions,
        platform: Arc<dyn Platform>,
        key_protector: Arc<dyn KeyProtector>,
        listener: Arc<dyn EventListener>,
    ) -> Result<Arc<Self>> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("nectarlink-core")
            .enable_all()
            .build()
            .map_err(|e| NectarlinkError::Internal { message: e.to_string() })?;
        let mut config =
            NodeConfig::new(PathBuf::from(options.data_dir), options.device.into(), options.app_version);
        config.power = options.power.into();
        config.away_mode = options.away_mode;
        config.key_protector = Some(Arc::new(KeyProtectorAdapter(key_protector)));
        let node = runtime.block_on(Node::start(config, Arc::new(PlatformAdapter(platform))))?;

        let mut events = node.events();
        runtime.spawn(async move {
            loop {
                match events.recv().await {
                    Ok(event) => listener.on_event(event.into()),
                    Err(RecvError::Lagged(missed)) => tracing::warn!(missed, "event listener fell behind"),
                    Err(RecvError::Closed) => return,
                }
            }
        });
        Ok(Arc::new(NectarlinkNode { runtime, node }))
    }

    /// This device's ID (z-base-32).
    pub fn device_id(&self) -> String {
        self.node.device_id().to_string()
    }

    /// Stops networking and tells connected devices.
    pub async fn shutdown(&self) {
        let node = self.node.clone();
        self.run(async move {
            if tokio::time::timeout(SHUTDOWN_TIMEOUT, node.shutdown()).await.is_err() {
                tracing::warn!("shutdown timed out");
            }
        })
        .await;
    }

    // ---- Pairing ----

    /// Enters pairing mode and returns the link to show as a QR code.
    pub async fn pairing_start_qr(&self) -> Result<String> {
        let node = self.node.clone();
        self.run(async move { Ok(node.pairing_start_qr().await?.to_uri()) }).await
    }

    /// Pairs with the device whose pairing link was scanned.
    pub async fn pairing_join(&self, link: String) -> Result<()> {
        let node = self.node.clone();
        self.run(async move { Ok(node.pairing_join(&link).await?) }).await
    }

    /// Starts nearby pairing with a discovered device in pairing mode; the
    /// code to compare arrives as `Event::PairingCode`.
    pub async fn pairing_start_nearby(&self, peer: String) -> Result<()> {
        let peer = parse_id(&peer)?;
        let node = self.node.clone();
        self.run(async move { Ok(node.pairing_start_nearby(peer).await?) }).await
    }

    /// Answers the code comparison.
    pub fn pairing_confirm(&self, codes_match: bool) -> Result<()> {
        Ok(self.node.pairing_confirm(codes_match)?)
    }

    pub fn pairing_cancel(&self) {
        self.node.pairing_cancel();
    }

    pub fn discovered_devices(&self) -> Vec<DiscoveredDevice> {
        self.node.discovered_devices().into_iter().map(Into::into).collect()
    }

    // ---- Paired devices ----

    pub fn paired_devices(&self) -> Result<Vec<PairedDevice>> {
        Ok(self.node.paired_devices()?.into_iter().map(Into::into).collect())
    }

    pub async fn unpair(&self, id: String) -> Result<()> {
        let id = parse_id(&id)?;
        let node = self.node.clone();
        self.run(async move { Ok(node.unpair(id).await?) }).await
    }

    pub async fn ring(&self, id: String, on: bool) -> Result<()> {
        let id = parse_id(&id)?;
        let node = self.node.clone();
        self.run(async move { Ok(node.ring(id, on).await?) }).await
    }

    pub fn capabilities(&self, id: String) -> Result<Vec<Feature>> {
        Ok(features(&self.node.capabilities(parse_id(&id)?)?))
    }

    pub fn device_toggles(&self, id: String) -> Result<Vec<DeviceToggle>> {
        Ok(self
            .node
            .device_toggles(parse_id(&id)?)?
            .into_iter()
            .map(|(name, enabled)| DeviceToggle { name: name.into(), enabled })
            .collect())
    }

    pub fn set_device_toggle(&self, id: String, name: String, enabled: bool) -> Result<()> {
        Ok(self.node.set_device_toggle(parse_id(&id)?, &name, enabled)?)
    }

    // ---- This device ----

    pub async fn update_battery(&self, battery: Battery) {
        let node = self.node.clone();
        self.run(async move { node.update_battery(battery.into()).await }).await;
    }

    pub async fn update_device_info(&self, info: DeviceInfo) {
        let node = self.node.clone();
        self.run(async move { node.update_device_info(info.into()).await }).await;
    }

    /// Updates this phone's power level and the capabilities it offers on
    /// top of the built-in ones (they depend on permissions and power).
    pub async fn update_power(&self, power: PowerLevel, capabilities: Vec<String>) {
        let node = self.node.clone();
        self.run(async move { node.update_power(power.into(), capabilities).await }).await;
    }

    /// Call when connectivity changes (Android doesn't tell native code).
    pub async fn network_changed(&self) {
        let node = self.node.clone();
        self.run(async move { node.network_changed().await }).await;
    }
}

/// Sends core logs to the platform log (logcat on Android). Call once,
/// before starting the node. `filter` uses tracing's syntax, e.g. "info".
#[uniffi::export]
pub fn init_logging(filter: String) {
    use tracing_subscriber::{EnvFilter, prelude::*};
    let filter = EnvFilter::try_new(&filter).unwrap_or_else(|_| EnvFilter::new("info"));
    #[cfg(target_os = "android")]
    let layer = paranoid_android::layer("Nectarlink");
    #[cfg(not(target_os = "android"))]
    let layer = tracing_subscriber::fmt::layer();
    let _ = tracing_subscriber::registry().with(filter).with(layer).try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_map_to_actionable_variants() {
        assert_eq!(NectarlinkError::from(core::Error::Offline), NectarlinkError::Offline);
        assert_eq!(
            NectarlinkError::from(core::Error::VersionTooOld { side: core::Side::Remote }),
            NectarlinkError::VersionTooOld { this_device: false }
        );
        assert!(matches!(
            NectarlinkError::from(core::Error::Internal("x".into())),
            NectarlinkError::Internal { message } if message == "x"
        ));
    }

    #[test]
    fn device_info_round_trips() {
        let info = DeviceInfo {
            name: "Pixel".into(),
            kind: DeviceKind::Phone,
            os: "android".into(),
            os_version: "16".into(),
            model: Some("Google Pixel 9".into()),
            accent: Some(0xFF8A5100),
        };
        assert_eq!(DeviceInfo::from(core::DeviceInfo::from(info.clone())), info);
    }

    #[test]
    fn batteries_are_clamped() {
        let b: core::Battery = Battery { level: 140, charging: true, plugged: None }.into();
        assert_eq!(b.level, 100);
    }
}
