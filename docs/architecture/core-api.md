# `nectarlink-core` public interface (Phase 0 draft)

This is the boundary between the shared Rust core and the two apps. The
desktop app calls it directly from Rust (and exposes it to QML through
the cxx-qt bridge in `desktop/app`); the Android app calls it through UniFFI-generated Kotlin.

Design rules:

- **One entry point**, `Node`, owning its own async runtime. Apps never touch
  tokio or iroh directly.
- **Commands are async methods; state changes are events.** UIs subscribe to a
  single event stream and render from it. No polling.
- **FFI-friendly types only** at the boundary: plain structs and enums, owned
  strings, byte vectors, no lifetimes or generics. That keeps the UniFFI and
  cxx-qt layers thin.
- **Platform features are injected**, not called directly: the core defines
  traits (battery source, notification sink…) that each app implements.

## Types (sketch)

```rust
pub struct NodeConfig {
    pub data_dir: PathBuf,             // keys, trust store, SQLite database
    pub device: DeviceInfo,
    pub away_mode: bool,               // allow relays
    pub relay_urls: Vec<String>,       // empty = defaults
}

pub struct DeviceId(pub [u8; 32]);     // displayed as z-base-32

pub struct PairedDevice {
    pub id: DeviceId,
    pub info: DeviceInfo,
    pub paired_at: i64,                // unix seconds
    pub link: LinkState,
}

pub enum LinkState {
    Offline { last_seen: Option<i64> },
    Connecting,
    Online { path: ConnectionPath, rtt_ms: u32 },
}

pub enum ConnectionPath { Lan, Usb, Relay }

pub enum PairingEvent {
    QrReady { uri: String, expires_at: i64 },
    SasCode { code: String },          // nearby flow: show and ask to confirm
    Paired { device: PairedDevice },
    Failed { reason: PairingError },
}

pub enum NodeEvent {
    DeviceAdded(PairedDevice),
    DeviceRemoved(DeviceId),
    LinkChanged { device: DeviceId, link: LinkState },
    PeerInfoChanged { device: DeviceId, info: DeviceInfo },
    Battery { device: DeviceId, level: u8, charging: bool },
    Capabilities(CapabilityMatrix),    // see capabilities.md
    Ring { device: DeviceId, on: bool },
    Pairing(PairingEvent),
}
```

## `Node`

```rust
impl Node {
    /// Loads or creates the device key and trust store, starts networking.
    pub async fn start(config: NodeConfig, platform: Arc<dyn Platform>) -> Result<Node>;
    pub async fn shutdown(&self);

    pub fn device_id(&self) -> DeviceId;
    pub fn events(&self) -> EventReceiver;          // one stream per subscriber

    // Pairing
    pub async fn pairing_start_qr(&self) -> Result<PairingSession>;     // PC side
    pub async fn pairing_scan_qr(&self, uri: String) -> Result<PairingSession>; // phone side
    pub async fn pairing_start_nearby(&self, peer: DiscoveredDevice) -> Result<PairingSession>;
    pub fn discovered_devices(&self) -> Vec<DiscoveredDevice>;

    // Devices
    pub fn paired_devices(&self) -> Vec<PairedDevice>;
    pub async fn unpair(&self, id: DeviceId) -> Result<()>;
    pub async fn ring(&self, id: DeviceId, on: bool) -> Result<()>;
    pub async fn set_device_toggle(&self, id: DeviceId, toggle: String, on: bool) -> Result<()>;

    // Local state the app reports to the core
    pub fn update_local_info(&self, info: DeviceInfo);
    pub fn update_power_level(&self, level: PowerLevel);
    pub fn update_local_capabilities(&self, caps: Vec<String>);
    pub fn set_away_mode(&self, enabled: bool);
}

impl PairingSession {
    pub async fn confirm_sas(&self, matches: bool) -> Result<()>;
    pub async fn cancel(&self);
}
```

## Platform trait (implemented by each app)

```rust
pub trait Platform: Send + Sync {
    fn battery(&self) -> Option<BatteryState>;
    fn start_ringing(&self);
    fn stop_ringing(&self);
    /// Called when the core needs a user decision while the app is in the
    /// background (e.g. an incoming nearby pairing request).
    fn notify_user(&self, request: UserPrompt);
}
```

Later services add more injected traits (notification source and sink,
clipboard, media session, file access…), each in its own module, so the
platform surface grows feature by feature.

## Threading

- `Node` runs on its own multi-threaded tokio runtime, created in `start`.
- Methods are safe to call from any thread.
- **Desktop:** the bridge (`desktop/app/src/core_host.rs`) reads `events()` on a background task and hands
  each event to the Qt GUI thread through cxx-qt's thread queue. This is the
  only place core events cross into Qt.
- **Android:** UniFFI exposes `events()` as a callback interface; the Kotlin
  side converts it into a `Flow` collected by view-models.

## Errors

A single `Error` enum crosses the boundary, with variants that the UI can
act on (`NotPaired`, `Offline`, `Denied`, `VersionTooOld { side }`,
`Unsupported`, `Io(String)`, `Internal(String)`). Internal error details are
logged without personal content.

## Storage

- `data_dir/identity.key`: the device key, encrypted with the platform key
  store.
- `data_dir/nectarlink.db`: SQLite with the trust store, per-device settings
  and caches. Schema migrations run in `Node::start`.
