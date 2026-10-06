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

/// A button on a notification.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct NotificationAction {
    pub id: String,
    pub title: String,
    /// Takes text (an inline reply).
    pub reply: bool,
}

/// A notification, as mirrored between devices
/// (docs/protocol/notifications.md).
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct Notification {
    /// This device's ID for it (Android: the StatusBarNotification key).
    pub key: String,
    /// Package name.
    pub app: String,
    pub app_name: String,
    pub title: Option<String>,
    pub text: Option<String>,
    pub sub: Option<String>,
    /// Unix milliseconds.
    pub when: i64,
    pub actions: Vec<NotificationAction>,
    /// Arrived without sound or pop-up.
    pub silent: bool,
    /// The app icon as PNG; pass it every time, the core sends it once per
    /// device and connection.
    pub icon: Option<Vec<u8>>,
    /// A picture it shows (a photo in a message, a big picture): JPEG, at
    /// most 160 KiB.
    pub image: Option<Vec<u8>>,
}

/// Something playing (or paused) on a device (docs/protocol/media.md).
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct MediaPlayer {
    /// Stable while the player exists (Android: the app's package name).
    pub id: String,
    /// User-visible app name.
    pub app: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub playing: bool,
    /// Milliseconds.
    pub duration: Option<u64>,
    /// Milliseconds, now; it moves on in real time while `playing`.
    pub position: Option<u64>,
    pub actions: Vec<MediaAction>,
    /// Identifies the artwork: the same key, the same picture.
    pub art_key: Option<String>,
    /// JPEG or PNG. Pass it every time; the core sends it once per device
    /// and connection. Arrives the same way: keep it by `art_key`.
    pub art: Option<Vec<u8>>,
}

/// Never prints what's playing (protocol v0 §11).
impl std::fmt::Debug for MediaPlayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaPlayer").field("id", &self.id).finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum MediaAction {
    Play,
    Pause,
    Next,
    Previous,
    Seek,
}

impl From<MediaAction> for core::MediaAction {
    fn from(a: MediaAction) -> Self {
        match a {
            MediaAction::Play => core::MediaAction::Play,
            MediaAction::Pause => core::MediaAction::Pause,
            MediaAction::Next => core::MediaAction::Next,
            MediaAction::Previous => core::MediaAction::Previous,
            MediaAction::Seek => core::MediaAction::Seek,
        }
    }
}

impl From<core::MediaAction> for MediaAction {
    fn from(a: core::MediaAction) -> Self {
        match a {
            core::MediaAction::Play => MediaAction::Play,
            core::MediaAction::Pause => MediaAction::Pause,
            core::MediaAction::Next => MediaAction::Next,
            core::MediaAction::Previous => MediaAction::Previous,
            core::MediaAction::Seek => MediaAction::Seek,
        }
    }
}

impl From<MediaPlayer> for core::MediaPlayer {
    fn from(p: MediaPlayer) -> Self {
        core::MediaPlayer {
            id: p.id,
            app: p.app,
            title: p.title,
            artist: p.artist,
            album: p.album,
            playing: p.playing,
            duration: p.duration,
            position: p.position,
            actions: p.actions.into_iter().map(|a| core::MediaAction::from(a).as_str().to_owned()).collect(),
            art_key: p.art_key,
            art: p.art,
        }
    }
}

impl From<core::MediaPlayer> for MediaPlayer {
    fn from(p: core::MediaPlayer) -> Self {
        MediaPlayer {
            id: p.id,
            app: p.app,
            title: p.title,
            artist: p.artist,
            album: p.album,
            playing: p.playing,
            duration: p.duration,
            position: p.position,
            actions: p.actions.iter().filter_map(|a| core::MediaAction::parse(a)).map(Into::into).collect(),
            art_key: p.art_key,
            art: p.art,
        }
    }
}

/// Never prints content (protocol v0 §11).
impl std::fmt::Debug for Notification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Notification").field("app", &self.app).finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TransferDirection {
    Outgoing,
    Incoming,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum TransferStatus {
    /// Waiting for the other device (to connect, or to continue).
    Waiting,
    Running,
    /// `saved`: where received files are, one path per item in `names`:
    /// each file, and each sent folder (empty when
    /// sending).
    Done {
        saved: Vec<String>,
    },
    /// `reason`: "denied", "unreachable", "noSpace", "interrupted" or
    /// "other".
    Failed {
        reason: String,
    },
    Cancelled,
}

/// A file transfer (docs/protocol/files.md).
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct Transfer {
    pub id: String,
    /// The other device.
    pub device_id: String,
    pub direction: TransferDirection,
    /// What was sent: file and folder names, in order.
    pub names: Vec<String>,
    /// How many files that is (folders' files included).
    pub files: u32,
    pub total: u64,
    pub done: u64,
    pub status: TransferStatus,
}

/// Never prints file names (protocol v0 §11).
impl std::fmt::Debug for Transfer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Transfer").field("id", &self.id).field("status", &self.status).finish_non_exhaustive()
    }
}

/// A file to send.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum FileToSend {
    /// An open, readable descriptor (Android: from a content URI). The core
    /// takes ownership and closes it.
    Fd {
        name: String,
        fd: i32,
        /// The folder it's in, when sending a folder: `/`-separated names,
        /// starting with the folder's own (`Trip/Day 1`).
        folder: Option<String>,
    },
    Path {
        name: String,
        path: String,
        folder: Option<String>,
    },
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
    NotificationsReset {
        id: String,
        items: Vec<Notification>,
    },
    NotificationPosted {
        id: String,
        notification: Notification,
    },
    NotificationRemoved {
        id: String,
        key: String,
    },
    /// A device put text on this phone's clipboard.
    ClipboardReceived {
        id: String,
    },
    /// A file transfer started, progressed or finished.
    Transfer {
        transfer: Transfer,
    },
    /// A device's media players (all of them; empty when nothing plays
    /// there, or media is off for it).
    MediaChanged {
        id: String,
        players: Vec<MediaPlayer>,
    },
    /// A paired phone took a photo (PCs only; phones announce theirs).
    PhotoAdded {
        id: String,
        photo_id: String,
    },
    /// A call on a paired phone changed (PCs only; phones report theirs).
    CallChanged {
        id: String,
        call_id: String,
    },
    /// A paired phone's screen started or stopped showing (PCs only).
    Mirroring {
        id: String,
        on: bool,
    },
    /// A paired phone's messages changed (PCs only).
    SmsChanged {
        id: String,
        thread: Option<String>,
    },
}

/// What a mirroring video packet holds (docs/protocol/mirror.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum VideoPacketKind {
    /// The stream's format, from `mirror_config`.
    Config,
    Frame,
    Keyframe,
}

/// What became of a video packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum MirrorSendResult {
    Queued,
    /// The network is behind: request a keyframe from the encoder (frames
    /// are dropped until one comes).
    NeedKeyframe,
    /// The PC stopped watching: stop sharing the screen.
    Closed,
}

/// This phone's screen going to one PC.
#[derive(Debug, uniffi::Object)]
pub struct MirrorStream(core::MirrorStream);

#[uniffi::export]
impl MirrorStream {
    /// Hands over a packet without waiting (a config packet waits for room).
    pub fn send(&self, kind: VideoPacketKind, time_us: u64, data: Vec<u8>) -> MirrorSendResult {
        let kind = match kind {
            VideoPacketKind::Config => core::PacketKind::Config,
            VideoPacketKind::Frame => core::PacketKind::Frame,
            VideoPacketKind::Keyframe => core::PacketKind::Keyframe,
        };
        match self.0.send(kind, time_us, data) {
            core::MirrorSend::Queued => MirrorSendResult::Queued,
            core::MirrorSend::NeedKeyframe => MirrorSendResult::NeedKeyframe,
            core::MirrorSend::Closed => MirrorSendResult::Closed,
        }
    }

    /// Ends the stream (`close` frees the object, as for every object).
    pub fn end(&self) {
        self.0.close();
    }
}

/// The bytes of a config packet for an H.264 stream of this size.
#[uniffi::export]
pub fn mirror_config(width: u32, height: u32) -> Vec<u8> {
    core::MirrorConfig { codec: "h264".into(), width, height }.to_cbor()
}

/// A conversation (docs/protocol/sms.md).
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct SmsThread {
    pub id: String,
    /// The other people's numbers (more than one: a group).
    pub addresses: Vec<String>,
    /// Their contact names, in the same order ("" when not a contact).
    pub names: Vec<String>,
    /// The latest message's text, or a description like "Photo".
    pub snippet: String,
    /// Unix milliseconds.
    pub date: i64,
    pub unread: u32,
    /// The contact's photo (one-person conversations): a JPEG of at most 16 KB.
    pub photo: Option<Vec<u8>>,
}

/// A message in a conversation.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct SmsMessage {
    pub id: String,
    pub thread: String,
    /// Who sent it (received), or the recipient (sent).
    pub address: String,
    pub body: String,
    /// Unix milliseconds.
    pub date: i64,
    pub outgoing: bool,
    /// Sent messages: "sent", "pending" or "failed".
    pub status: Option<String>,
    /// Pictures and other attachments, fetched with `sms_part`.
    pub parts: Vec<SmsPart>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SmsPart {
    pub id: String,
    pub mime: String,
    pub size: u64,
}

/// An attachment's bytes.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct SmsPartData {
    pub mime: String,
    pub data: Vec<u8>,
}

macro_rules! private_debug {
    ($($t:ty),*) => {$(
        /// Never prints numbers, names or text (protocol v0 §11).
        impl std::fmt::Debug for $t {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($t)).finish_non_exhaustive()
            }
        }
    )*};
}
private_debug!(SmsThread, SmsMessage, SmsPartData);

/// A call on this phone (docs/protocol/calls.md).
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct Call {
    /// This app's ID for the call, the same while it lasts.
    pub id: String,
    pub state: CallPhase,
    pub incoming: bool,
    pub number: Option<String>,
    /// The contact's name, when the number is a contact.
    pub name: Option<String>,
    /// The contact's photo: a JPEG of at most 64 KB, sent while ringing.
    pub photo: Option<Vec<u8>>,
    /// With `Ended`: it rang and nobody answered.
    pub missed: bool,
}

/// Never prints the number, name or photo (protocol v0 §11).
impl std::fmt::Debug for Call {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Call").field("id", &self.id).field("state", &self.state).finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CallPhase {
    Ringing,
    Active,
    Ended,
}

/// What a PC asked of a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CallCommand {
    Answer,
    /// Decline a ringing call, or hang up an active one.
    Decline,
    /// Stop the ringing.
    Silence,
}

/// A photo or screenshot that just appeared on this phone
/// (docs/protocol/photos.md).
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct Photo {
    /// This app's ID for it (a PC asks for it by this ID).
    pub id: String,
    pub name: String,
    pub size: u64,
    /// When it was taken, in Unix seconds.
    pub taken: i64,
    pub screenshot: bool,
    /// A JPEG preview of at most 96 KB, at most 512 pixels a side.
    pub thumb: Vec<u8>,
}

/// Never prints the name or picture (protocol v0 §11).
impl std::fmt::Debug for Photo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Photo").field("id", &self.id).finish_non_exhaustive()
    }
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
    #[error("it no longer exists")]
    NotFound,
    #[error("too large to send")]
    TooLarge,
    #[error("no pairing in progress")]
    NotPairing,
    #[error("invalid pairing link")]
    InvalidPairingLink,
    #[error("invalid device ID")]
    InvalidDeviceId,
    #[error("network error: {reason}")]
    Network { reason: String },
    #[error("storage error: {reason}")]
    Storage { reason: String },
    #[error("internal error: {reason}")]
    Internal { reason: String },
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
            core::Error::NotFound => NectarlinkError::NotFound,
            core::Error::TooLarge => NectarlinkError::TooLarge,
            core::Error::NotPairing => NectarlinkError::NotPairing,
            core::Error::InvalidPairingLink(_) => NectarlinkError::InvalidPairingLink,
            core::Error::Network(reason) => NectarlinkError::Network { reason },
            core::Error::Storage(reason) => NectarlinkError::Storage { reason },
            core::Error::Io(e) => NectarlinkError::Storage { reason: e.to_string() },
            core::Error::Protocol(reason) | core::Error::Internal(reason) => {
                NectarlinkError::Internal { reason }
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

impl From<core::Notification> for Notification {
    fn from(n: core::Notification) -> Self {
        Notification {
            key: n.key,
            app: n.app,
            app_name: n.app_name,
            title: n.title,
            text: n.text,
            sub: n.sub,
            when: n.when,
            actions: n
                .actions
                .into_iter()
                .map(|a| NotificationAction { id: a.id, title: a.title, reply: a.reply })
                .collect(),
            silent: n.silent,
            icon: n.icon,
            image: n.image,
        }
    }
}

impl From<Notification> for core::Notification {
    fn from(n: Notification) -> Self {
        core::Notification {
            key: n.key,
            app: n.app,
            app_name: n.app_name,
            title: n.title,
            text: n.text,
            sub: n.sub,
            when: n.when,
            actions: n
                .actions
                .into_iter()
                .map(|a| core::NotificationAction { id: a.id, title: a.title, reply: a.reply })
                .collect(),
            silent: n.silent,
            icon: n.icon,
            image: n.image,
        }
    }
}

impl From<core::Transfer> for Transfer {
    fn from(t: core::Transfer) -> Self {
        Transfer {
            id: t.id,
            device_id: t.device.to_string(),
            direction: match t.direction {
                core::Direction::Outgoing => TransferDirection::Outgoing,
                core::Direction::Incoming => TransferDirection::Incoming,
            },
            names: t.names,
            files: u32::try_from(t.files).unwrap_or(u32::MAX),
            total: t.total,
            done: t.done,
            status: match t.state {
                core::TransferState::Waiting => TransferStatus::Waiting,
                core::TransferState::Running => TransferStatus::Running,
                core::TransferState::Done { saved } => TransferStatus::Done {
                    saved: saved.into_iter().map(|p| p.to_string_lossy().into_owned()).collect(),
                },
                core::TransferState::Failed(failure) => TransferStatus::Failed {
                    reason: match failure {
                        core::TransferFailure::Denied => "denied",
                        core::TransferFailure::Unreachable => "unreachable",
                        core::TransferFailure::NoSpace => "noSpace",
                        core::TransferFailure::Interrupted => "interrupted",
                        core::TransferFailure::Other(_) => "other",
                    }
                    .into(),
                },
                core::TransferState::Cancelled => TransferStatus::Cancelled,
            },
        }
    }
}

fn file_to_send(file: FileToSend) -> Result<core::OutgoingFile> {
    Ok(match file {
        FileToSend::Path { name, path, folder } => {
            core::OutgoingFile { name, folder, source: core::FileSource::Path(path.into()) }
        }
        #[cfg(unix)]
        FileToSend::Fd { name, fd, folder } => {
            use std::os::fd::FromRawFd;
            if fd < 0 {
                return Err(NectarlinkError::Internal { reason: "invalid file descriptor".into() });
            }
            // SAFETY: the caller hands over an open descriptor it no longer
            // uses (documented on `FileToSend::Fd`); the File closes it.
            #[allow(unsafe_code)]
            let file = unsafe { std::fs::File::from_raw_fd(fd) };
            core::OutgoingFile { name, folder, source: core::FileSource::File(file) }
        }
        #[cfg(not(unix))]
        FileToSend::Fd { .. } => {
            return Err(NectarlinkError::Internal {
                reason: "file descriptors exist only on Android".into(),
            });
        }
    })
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
            NodeEvent::NotificationsReset { device, items } => Event::NotificationsReset {
                id: device.to_string(),
                items: items.into_iter().map(Into::into).collect(),
            },
            NodeEvent::NotificationPosted { device, notification } => {
                Event::NotificationPosted { id: device.to_string(), notification: notification.into() }
            }
            NodeEvent::NotificationRemoved { device, key } => {
                Event::NotificationRemoved { id: device.to_string(), key }
            }
            NodeEvent::ClipboardReceived { device } => Event::ClipboardReceived { id: device.to_string() },
            NodeEvent::Transfer(t) => Event::Transfer { transfer: t.into() },
            NodeEvent::MediaChanged { device, players } => Event::MediaChanged {
                id: device.to_string(),
                players: players.into_iter().map(Into::into).collect(),
            },
            NodeEvent::PhotoAdded { device, photo } => {
                Event::PhotoAdded { id: device.to_string(), photo_id: photo.id }
            }
            NodeEvent::Call { device, call } => {
                Event::CallChanged { id: device.to_string(), call_id: call.id }
            }
            NodeEvent::SmsChanged { device, thread } => Event::SmsChanged { id: device.to_string(), thread },
            NodeEvent::Mirroring { device, on } => Event::Mirroring { id: device.to_string(), on },
        }
    }
}

fn parse_id(id: &str) -> Result<DeviceId> {
    id.parse().map_err(|_| NectarlinkError::InvalidDeviceId)
}

// ---- Foreign interfaces (implemented in Kotlin) ----

/// Why a notification couldn't be dismissed or its action run.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum NotificationFailure {
    /// The notification or action is gone.
    #[error("no longer exists")]
    NotFound,
    /// Not possible here (e.g. notification access was revoked).
    #[error("not available")]
    Unsupported,
    /// `reason` is for logs; never put notification content in it.
    #[error("failed: {reason}")]
    Failed { reason: String },
}

impl From<uniffi::UnexpectedUniFFICallbackError> for NotificationFailure {
    fn from(e: uniffi::UnexpectedUniFFICallbackError) -> Self {
        NotificationFailure::Failed { reason: e.reason }
    }
}

impl From<NotificationFailure> for core::NotificationError {
    fn from(f: NotificationFailure) -> Self {
        match f {
            NotificationFailure::NotFound => core::NotificationError::NotFound,
            NotificationFailure::Unsupported => core::NotificationError::Unsupported,
            NotificationFailure::Failed { reason } => core::NotificationError::Failed(reason),
        }
    }
}

/// Why a media command didn't run.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum MediaFailure {
    /// The player is gone.
    #[error("no such player")]
    NotFound,
    /// The player can't do that (or media access was revoked).
    #[error("not available")]
    Unsupported,
    /// `reason` is for logs.
    #[error("failed: {reason}")]
    Failed { reason: String },
}

impl From<uniffi::UnexpectedUniFFICallbackError> for MediaFailure {
    fn from(e: uniffi::UnexpectedUniFFICallbackError) -> Self {
        MediaFailure::Failed { reason: e.reason }
    }
}

impl From<MediaFailure> for core::MediaError {
    fn from(f: MediaFailure) -> Self {
        match f {
            MediaFailure::NotFound => core::MediaError::NotFound,
            MediaFailure::Unsupported => core::MediaError::Unsupported,
            MediaFailure::Failed { reason } => core::MediaError::Failed(reason),
        }
    }
}

/// What the app does for the core: ring the phone, act on its
/// notifications.
#[uniffi::export(with_foreign)]
pub trait Platform: Send + Sync {
    /// Ring loudly, even in silent mode, until `stop_ringing`.
    fn start_ringing(&self);
    fn stop_ringing(&self);
    /// A PC dismissed this notification.
    fn dismiss_notification(&self, key: String) -> Result<(), NotificationFailure>;
    /// A PC ran this notification's action; `reply` is the text for a reply
    /// action.
    fn run_notification_action(
        &self,
        key: String,
        action: String,
        reply: Option<String>,
    ) -> Result<(), NotificationFailure>;
    /// A PC sent this text: put it on the clipboard. False if that failed.
    fn set_clipboard(&self, text: String) -> bool;
    /// A PC sent this image (`image/png` or `image/jpeg`): put it on the
    /// clipboard. False if that failed.
    fn set_clipboard_image(&self, mime: String, bytes: Vec<u8>) -> bool;
    /// A paired device asked one of this phone's media players to do
    /// something; `position` (milliseconds) is set for seeking.
    fn media_command(
        &self,
        player: String,
        action: MediaAction,
        position: Option<u64>,
    ) -> Result<(), MediaFailure>;
    /// A paired PC sent a web link (http or https, checked) to open.
    /// False if it couldn't be shown.
    fn open_link(&self, from_id: String, url: String) -> bool;
    /// A PC asked for a photo announced with `photo_taken`: open it, or
    /// `null` when it's gone.
    fn open_photo(&self, id: String) -> Option<FileToSend>;
    /// A PC asked to answer, decline or silence call `id` (the call in
    /// progress). False if the phone couldn't.
    fn call_command(&self, id: String, command: CallCommand) -> bool;
    /// A PC asked for this phone's screen: ask the user (then call
    /// `mirror_open`). False if the user couldn't be asked.
    fn mirror_requested(&self, pc_id: String, max_size: u32, fps: u32, bitrate: u32) -> bool;
    /// The PC stopped watching: stop sharing.
    fn mirror_stop_requested(&self, pc_id: String);
    /// The PC needs a keyframe.
    fn mirror_keyframe_requested(&self, pc_id: String);
    /// A PC asked for the latest conversations, newest first.
    fn sms_threads(&self, limit: u32) -> Vec<SmsThread>;
    /// A PC asked for a conversation's messages before `before` (Unix ms;
    /// the latest when null), newest first.
    fn sms_messages(&self, thread: String, before: Option<i64>, limit: u32) -> Vec<SmsMessage>;
    /// A PC asked to send a text (checked: 1–20 recipients, not empty).
    /// False if it couldn't be sent.
    fn sms_send(&self, to: Vec<String>, body: String) -> bool;
    /// A PC asked for a message's attachment; null when it's gone.
    fn sms_part(&self, id: String) -> Option<SmsPartData>;
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
    fn dismiss_notification(&self, key: &str) -> Result<(), core::NotificationError> {
        Ok(self.0.dismiss_notification(key.to_owned())?)
    }
    fn run_notification_action(
        &self,
        key: &str,
        action: &str,
        reply: Option<&str>,
    ) -> Result<(), core::NotificationError> {
        Ok(self.0.run_notification_action(key.to_owned(), action.to_owned(), reply.map(str::to_owned))?)
    }
    fn set_clipboard(&self, text: &str) -> Result<(), String> {
        if self.0.set_clipboard(text.to_owned()) { Ok(()) } else { Err("the clipboard rejected it".into()) }
    }
    fn media_command(
        &self,
        player: &str,
        action: core::MediaAction,
        position: Option<u64>,
    ) -> Result<(), core::MediaError> {
        Ok(self.0.media_command(player.to_owned(), action.into(), position)?)
    }
    fn open_link(&self, from: &DeviceId, url: &str) -> Result<(), String> {
        if self.0.open_link(from.to_string(), url.to_owned()) { Ok(()) } else { Err("not shown".into()) }
    }
    fn set_clipboard_image(&self, mime: &str, bytes: &[u8]) -> Result<(), String> {
        if self.0.set_clipboard_image(mime.to_owned(), bytes.to_vec()) {
            Ok(())
        } else {
            Err("the clipboard rejected it".into())
        }
    }
    fn mirror_requested(&self, peer: &DeviceId, options: &core::MirrorStart) -> Result<(), String> {
        if self.0.mirror_requested(peer.to_string(), options.max_size, options.fps, options.bitrate) {
            Ok(())
        } else {
            Err("the user couldn't be asked".into())
        }
    }
    fn mirror_stop_requested(&self, peer: &DeviceId) {
        self.0.mirror_stop_requested(peer.to_string());
    }
    fn mirror_keyframe_requested(&self, peer: &DeviceId) {
        self.0.mirror_keyframe_requested(peer.to_string());
    }
    fn sms_threads(&self, limit: u32) -> Result<Vec<core::SmsThread>, String> {
        Ok(self
            .0
            .sms_threads(limit)
            .into_iter()
            .map(|t| core::SmsThread {
                id: t.id,
                addresses: t.addresses,
                names: t.names,
                snippet: t.snippet,
                date: t.date,
                unread: t.unread,
                photo: t.photo,
            })
            .collect())
    }
    fn sms_messages(
        &self,
        thread: &str,
        before: Option<i64>,
        limit: u32,
    ) -> Result<Vec<core::SmsMessage>, String> {
        Ok(self
            .0
            .sms_messages(thread.to_owned(), before, limit)
            .into_iter()
            .map(|m| core::SmsMessage {
                id: m.id,
                thread: m.thread,
                address: m.address,
                body: m.body,
                date: m.date,
                outgoing: m.outgoing,
                status: m.status,
                parts: m
                    .parts
                    .into_iter()
                    .map(|p| core::SmsPart { id: p.id, mime: p.mime, size: p.size })
                    .collect(),
            })
            .collect())
    }
    fn sms_send(&self, to: &[String], body: &str) -> Result<(), String> {
        if self.0.sms_send(to.to_vec(), body.to_owned()) { Ok(()) } else { Err("not sent".into()) }
    }
    fn sms_part(&self, id: &str) -> Result<(String, Vec<u8>), String> {
        let part = self.0.sms_part(id.to_owned()).ok_or("it's gone")?;
        Ok((part.mime, part.data))
    }
    fn call_command(&self, id: &str, command: core::CallCommand) -> Result<(), String> {
        let command = match command {
            core::CallCommand::Answer => CallCommand::Answer,
            core::CallCommand::Decline => CallCommand::Decline,
            core::CallCommand::Silence => CallCommand::Silence,
        };
        if self.0.call_command(id.to_owned(), command) { Ok(()) } else { Err("the phone couldn't".into()) }
    }
    fn open_photo(&self, id: &str) -> Result<core::OutgoingFile, String> {
        let file = self.0.open_photo(id.to_owned()).ok_or("it's gone")?;
        file_to_send(file).map_err(|e| e.to_string())
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
    /// Where received files are put once complete (the app then moves them
    /// where the user finds them).
    pub downloads_dir: String,
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
            .map_err(|e| NectarlinkError::Internal { reason: e.to_string() })?;
        let mut config =
            NodeConfig::new(PathBuf::from(options.data_dir), options.device.into(), options.app_version);
        config.power = options.power.into();
        config.away_mode = options.away_mode;
        config.downloads_dir = Some(PathBuf::from(options.downloads_dir));
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

    // ---- Files ----

    /// Sends files to a paired PC; returns the transfer's ID. Progress
    /// arrives as `Event::Transfer`.
    pub async fn send_files(&self, id: String, files: Vec<FileToSend>) -> Result<String> {
        let id = parse_id(&id)?;
        let files = files.into_iter().map(file_to_send).collect::<Result<Vec<_>>>()?;
        let node = self.node.clone();
        self.run(async move { Ok(node.send_files(id, files).await?) }).await
    }

    /// Cancels a transfer in either direction.
    pub fn cancel_transfer(&self, transfer_id: String) {
        self.node.cancel_transfer(&transfer_id);
    }

    // ---- Clipboard ----

    /// Puts text on a paired PC's clipboard.
    pub async fn send_clipboard(&self, id: String, text: String) -> Result<()> {
        let id = parse_id(&id)?;
        let node = self.node.clone();
        self.run(async move { Ok(node.send_clipboard(id, text).await?) }).await
    }

    /// Puts an image (`image/png` or `image/jpeg`) on a paired PC's
    /// clipboard.
    pub async fn send_clipboard_image(&self, id: String, mime: String, bytes: Vec<u8>) -> Result<()> {
        let id = parse_id(&id)?;
        let node = self.node.clone();
        self.run(async move { Ok(node.send_clipboard_image(id, mime, bytes).await?) }).await
    }

    // ---- Media ----

    /// This phone's media players changed (most relevant first).
    pub async fn media_changed(&self, players: Vec<MediaPlayer>) {
        let node = self.node.clone();
        self.run(async move { node.media_changed(players.into_iter().map(Into::into).collect()).await })
            .await;
    }

    /// Runs a command on a paired device's media player; `position`
    /// (milliseconds) for seeking.
    pub async fn media_command(
        &self,
        id: String,
        player: String,
        action: MediaAction,
        position: Option<u64>,
    ) -> Result<()> {
        let id = parse_id(&id)?;
        let node = self.node.clone();
        self.run(async move { Ok(node.media_command(id, player, action.into(), position).await?) }).await
    }

    // ---- Notifications ----

    /// A notification appeared or changed on this phone.
    pub async fn notification_posted(&self, notification: Notification) {
        let node = self.node.clone();
        self.run(async move { node.notification_posted(notification.into()).await }).await;
    }

    pub async fn notification_removed(&self, key: String) {
        let node = self.node.clone();
        self.run(async move { node.notification_removed(key).await }).await;
    }

    /// Everything this phone shows now (listener connected), or nothing
    /// (access revoked).
    pub async fn notifications_reset(&self, items: Vec<Notification>) {
        let node = self.node.clone();
        self.run(async move { node.notifications_reset(items.into_iter().map(Into::into).collect()).await })
            .await;
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

    /// Locks a paired PC (`sleep`: puts it to sleep instead).
    pub async fn pc_power(&self, id: String, sleep: bool) -> Result<()> {
        let id = parse_id(&id)?;
        let node = self.node.clone();
        let action = if sleep { core::PowerAction::Sleep } else { core::PowerAction::Lock };
        self.run(async move { Ok(node.pc_power(id, action).await?) }).await
    }

    /// Opens a web link on a paired PC.
    pub async fn open_link(&self, id: String, url: String) -> Result<()> {
        let id = parse_id(&id)?;
        let node = self.node.clone();
        self.run(async move { Ok(node.open_link(id, url).await?) }).await
    }

    /// Opens this phone's screen stream to a PC that asked (after the user
    /// agreed).
    pub async fn mirror_open(&self, pc_id: String) -> Result<Arc<MirrorStream>> {
        let id = parse_id(&pc_id)?;
        let node = self.node.clone();
        self.run(async move { Ok(Arc::new(MirrorStream(node.mirror_open(id).await?))) }).await
    }

    /// This phone's messages changed (in `thread`, or anywhere when null):
    /// PCs that show them catch up.
    pub async fn sms_changed(&self, thread: Option<String>) {
        let node = self.node.clone();
        self.run(async move { node.sms_changed(thread).await }).await;
    }

    /// A call on this phone rang, was answered or ended: tells the PCs that
    /// show calls (and are allowed them), now and when they connect during it.
    pub async fn call_changed(&self, call: Call) -> Result<()> {
        let node = self.node.clone();
        let call = core::CallState {
            id: call.id,
            state: match call.state {
                CallPhase::Ringing => "ringing",
                CallPhase::Active => "active",
                CallPhase::Ended => "ended",
            }
            .into(),
            incoming: call.incoming,
            number: call.number,
            name: call.name,
            photo: call.photo,
            missed: call.missed,
        };
        self.run(async move { Ok(node.call_changed(call).await?) }).await
    }

    /// A photo or screenshot just appeared on this phone: tells the PCs
    /// that show photos (and are allowed them).
    pub async fn photo_taken(&self, photo: Photo) -> Result<()> {
        let node = self.node.clone();
        let photo = core::Photo {
            id: photo.id,
            name: photo.name,
            size: photo.size,
            taken: photo.taken,
            screenshot: photo.screenshot,
            thumb: photo.thumb,
        };
        self.run(async move { Ok(node.photo_taken(photo).await?) }).await
    }

    /// Reconnects to PCs that aren't connected and syncs connected ones.
    pub async fn refresh(&self) {
        let node = self.node.clone();
        self.run(async move { node.refresh().await }).await;
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
    let layer = paranoid_android::layer("Nectarlink").with_ansi(false);
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
            NectarlinkError::Internal { reason } if reason == "x"
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
