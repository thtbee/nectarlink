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
    /// Whether this phone has stored Wake-on-LAN addresses for this PC.
    pub can_wake: bool,
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

/// A timestamped marker within a voice recording (docs/protocol/recorder.md).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RecordingMarker {
    pub at_ms: u64,
    pub label: Option<String>,
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
    /// True when this transfer is a voice recording from the phone's recorder.
    pub recording: bool,
    /// Markers captured during the recording.
    pub markers: Vec<RecordingMarker>,
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
    /// The local clipboard history changed.
    ClipboardHistoryChanged,
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
    /// A paired phone's call history changed (PCs only).
    CallLogChanged {
        id: String,
    },
    /// A paired phone's contacts changed (PCs only).
    ContactsChanged {
        id: String,
    },
    /// A paired phone's screen (session 0) or app window started or
    /// stopped showing (PCs only).
    Mirroring {
        id: String,
        session: u32,
        on: bool,
    },
    /// A paired phone's messages changed (PCs only).
    SmsChanged {
        id: String,
        thread: Option<String>,
    },
    /// A paired phone's photo library changed (PCs only).
    PhotosChanged {
        id: String,
    },
    /// A paired phone asked to control this PC while `remote_input` is off
    /// (PCs only).
    RemoteInputRequested {
        id: String,
    },
    /// A paired phone's quick settings state (PCs only).
    PhoneToggles {
        id: String,
        toggles: PhoneToggles,
    },
    /// A paired PC's stored Wake-on-LAN addresses changed (phones only).
    WakeInfoChanged {
        id: String,
        can_wake: bool,
    },
    /// A paired PC's Deck layout arrived or changed (phones only).
    DeckLayout {
        id: String,
        layout: DeckLayout,
    },
    /// A paired PC's live Deck state arrived or changed (phones only).
    DeckState {
        id: String,
        state: DeckState,
    },
    /// A paired phone's camera started or stopped streaming as a webcam
    /// (PCs only).
    Webcam {
        id: String,
        on: bool,
    },
    /// A paired PC asked to browse this phone's storage while `storage` is off
    /// (phones only).
    StorageRequested {
        id: String,
    },
    /// A folder on a paired phone changed (PCs only).
    StorageChanged {
        id: String,
        path: String,
    },
}

/// One tile on a PC's Deck (`docs/protocol/deck.md`).
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct DeckTile {
    pub id: String,
    pub label: String,
    pub icon: String,
    pub color: String,
    pub kind: String,
}

impl std::fmt::Debug for DeckTile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeckTile")
            .field("id", &self.id)
            .field("icon", &self.icon)
            .field("color", &self.color)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl From<core::DeckTile> for DeckTile {
    fn from(t: core::DeckTile) -> Self {
        DeckTile { id: t.id, label: t.label, icon: t.icon, color: t.color, kind: t.kind }
    }
}

/// One page of tiles on a PC's Deck (`docs/protocol/deck.md`).
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct DeckPage {
    pub id: String,
    pub name: String,
    pub tiles: Vec<DeckTile>,
}

impl std::fmt::Debug for DeckPage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeckPage")
            .field("id", &self.id)
            .field("tiles", &self.tiles.len())
            .finish_non_exhaustive()
    }
}

impl From<core::DeckPage> for DeckPage {
    fn from(p: core::DeckPage) -> Self {
        DeckPage { id: p.id, name: p.name, tiles: p.tiles.into_iter().map(Into::into).collect() }
    }
}

/// A PC's Deck layout (`docs/protocol/deck.md`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DeckLayout {
    pub pages: Vec<DeckPage>,
}

impl From<core::DeckLayout> for DeckLayout {
    fn from(l: core::DeckLayout) -> Self {
        DeckLayout { pages: l.pages.into_iter().map(Into::into).collect() }
    }
}

/// An active audio output device on a paired PC (`docs/protocol/deck.md`).
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct AudioOutputDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

impl std::fmt::Debug for AudioOutputDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioOutputDevice").field("is_default", &self.is_default).finish_non_exhaustive()
    }
}

impl From<core::AudioOutputDevice> for AudioOutputDevice {
    fn from(d: core::AudioOutputDevice) -> Self {
        AudioOutputDevice { id: d.id, name: d.name, is_default: d.is_default }
    }
}

/// Live PC state reflected on Deck tiles (`docs/protocol/deck.md`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DeckState {
    pub playing: bool,
    pub volume: u8,
    pub muted: bool,
    pub mic_muted: Option<bool>,
    pub output_devices: Vec<AudioOutputDevice>,
}

impl From<core::DeckState> for DeckState {
    fn from(s: core::DeckState) -> Self {
        DeckState {
            playing: s.playing,
            volume: s.volume,
            muted: s.muted,
            mic_muted: s.mic_muted,
            output_devices: s.output_devices.into_iter().map(Into::into).collect(),
        }
    }
}

/// The recognized kind of a text clip (`docs/protocol/clipboard.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ClipKind {
    WebLink,
    StreetAddress,
    PhoneNumber,
    TrackingNumber,
    Email,
}

impl From<core::ClipKind> for ClipKind {
    fn from(k: core::ClipKind) -> Self {
        match k {
            core::ClipKind::WebLink => ClipKind::WebLink,
            core::ClipKind::StreetAddress => ClipKind::StreetAddress,
            core::ClipKind::PhoneNumber => ClipKind::PhoneNumber,
            core::ClipKind::TrackingNumber => ClipKind::TrackingNumber,
            core::ClipKind::Email => ClipKind::Email,
        }
    }
}

/// A context chip suggestion for a received text clip.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct ClipSuggestion {
    pub kind: ClipKind,
    pub action_label: String,
    pub target: String,
}

/// Never prints the target text (protocol v0 §11).
impl std::fmt::Debug for ClipSuggestion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClipSuggestion").field("kind", &self.kind).finish_non_exhaustive()
    }
}

impl From<core::ClipSuggestion> for ClipSuggestion {
    fn from(s: core::ClipSuggestion) -> Self {
        ClipSuggestion { kind: s.kind.into(), action_label: s.action_label().into(), target: s.target }
    }
}

/// This phone's quick settings state (docs/protocol/toggles.md).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PhoneToggles {
    pub dnd: bool,
    /// `"ring"`, `"vibrate"` or `"silent"`.
    pub ringer: String,
    /// `None` when the phone has no camera flash.
    pub flashlight: Option<bool>,
    /// Media volume, `0..=100`.
    pub volume: u8,
    /// Screen brightness, `0..=100`.
    pub brightness: u8,
    pub wifi: bool,
    pub bluetooth: bool,
}

impl From<core::PhoneToggles> for PhoneToggles {
    fn from(t: core::PhoneToggles) -> Self {
        PhoneToggles {
            dnd: t.dnd,
            ringer: t.ringer,
            flashlight: t.flashlight,
            volume: t.volume,
            brightness: t.brightness,
            wifi: t.wifi,
            bluetooth: t.bluetooth,
        }
    }
}

impl From<PhoneToggles> for core::PhoneToggles {
    fn from(t: PhoneToggles) -> Self {
        core::PhoneToggles {
            dnd: t.dnd,
            ringer: t.ringer,
            flashlight: t.flashlight,
            volume: t.volume,
            brightness: t.brightness,
            wifi: t.wifi,
            bluetooth: t.bluetooth,
        }
    }
}

/// The value to set for a phone quick setting (`phone.toggle.set`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum PhoneToggleValue {
    Bool { on: bool },
    Level { level: u8 },
    Mode { mode: String },
}

/// What a mirroring video packet holds (docs/protocol/mirror.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum VideoPacketKind {
    /// The stream's format, from `mirror_config`.
    Config,
    Frame,
    Keyframe,
}

/// The PC's mouse or keyboard on the mirrored screen. Positions are
/// fractions of the screen (0 to 1).
#[derive(Clone, PartialEq, uniffi::Enum)]
pub enum MirrorInputEvent {
    Touch {
        action: TouchPhase,
        x: f32,
        y: f32,
    },
    /// The mouse wheel, in notches (positive: down / right).
    Scroll {
        x: f32,
        y: f32,
        dx: f32,
        dy: f32,
    },
    /// "back", "home", "recents", "enter", "backspace", "delete", "left",
    /// "right", "up", "down", "tab" or "notifications".
    Key {
        key: String,
    },
    Text {
        text: String,
    },
}

/// Never prints typed text (protocol v0 §11).
impl std::fmt::Debug for MirrorInputEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MirrorInputEvent::Text { text } => write!(f, "Text({} bytes)", text.len()),
            MirrorInputEvent::Touch { action, .. } => write!(f, "Touch({action:?})"),
            MirrorInputEvent::Scroll { .. } => f.write_str("Scroll"),
            MirrorInputEvent::Key { key } => write!(f, "Key({key})"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TouchPhase {
    Down,
    Move,
    Up,
}

impl From<core::MirrorInput> for MirrorInputEvent {
    fn from(input: core::MirrorInput) -> Self {
        match input {
            core::MirrorInput::Touch { action, x, y } => MirrorInputEvent::Touch {
                action: match action {
                    core::TouchAction::Down => TouchPhase::Down,
                    core::TouchAction::Move => TouchPhase::Move,
                    core::TouchAction::Up => TouchPhase::Up,
                },
                x,
                y,
            },
            core::MirrorInput::Scroll { x, y, dx, dy } => MirrorInputEvent::Scroll { x, y, dx, dy },
            core::MirrorInput::Key { key } => MirrorInputEvent::Key { key },
            core::MirrorInput::Text { text } => MirrorInputEvent::Text { text },
        }
    }
}

/// What became of a video (or sound) packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum MirrorSendResult {
    Queued,
    /// Sound dropped: the network is behind (the next packet may go).
    Dropped,
    /// The network is behind: request a keyframe from the encoder (frames
    /// are dropped until one comes).
    NeedKeyframe,
    /// The PC stopped watching: stop sharing the screen.
    Closed,
}

/// This phone's screen (or its sound) going to one PC.
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
            core::MirrorSend::Dropped => MirrorSendResult::Dropped,
            core::MirrorSend::NeedKeyframe => MirrorSendResult::NeedKeyframe,
            core::MirrorSend::Closed => MirrorSendResult::Closed,
        }
    }

    /// Ends the stream (`close` frees the object, as for every object).
    pub fn end(&self) {
        self.0.close();
    }
}

/// The bytes of a config packet for an H.264 stream of this size, for the
/// screen (session 0) or an app window.
#[uniffi::export]
pub fn mirror_config(width: u32, height: u32, session: u32) -> Vec<u8> {
    core::MirrorConfig { codec: "h264".into(), width, height, session }.to_cbor()
}

/// What a PC asked to mirror, and how (docs/protocol/mirror.md).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MirrorOptions {
    /// The longer side, in pixels, at most.
    pub max_size: u32,
    pub fps: u32,
    pub bitrate: u32,
    /// The sound too (the screen only).
    pub audio: bool,
    /// 0 for the screen; another for an app window.
    pub session: u32,
    /// The app (package name) to show in a window of its own.
    pub app: Option<String>,
}

/// An app a PC can open in a window (docs/protocol/mirror.md).
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct PhoneApp {
    /// Package name.
    pub pkg: String,
    pub label: String,
    /// A small PNG icon (at most 8 KiB).
    pub icon: Option<Vec<u8>>,
}

impl std::fmt::Debug for PhoneApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhoneApp").field("pkg", &self.pkg).finish_non_exhaustive()
    }
}

/// The bytes of a config packet for 16-bit PCM sound.
#[uniffi::export]
pub fn mirror_audio_config(rate: u32, channels: u8) -> Vec<u8> {
    core::MirrorAudioConfig { codec: core::MIRROR_PCM.into(), rate, channels }.to_cbor()
}

/// The bytes of a config packet for an H.264 webcam stream (`docs/protocol/webcam.md`).
#[uniffi::export]
pub fn webcam_config(width: u32, height: u32, camera: String, fps: u32) -> Vec<u8> {
    core::WebcamConfig { codec: core::WEBCAM_H264.into(), width, height, camera, fps }.to_cbor()
}

/// What a PC asked the phone's camera to stream (`docs/protocol/webcam.md`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct WebcamOptions {
    /// `"back"` or `"front"`.
    pub camera: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate: u32,
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
private_debug!(SmsThread, SmsMessage, SmsPartData, CallLogEntry, ContactNumber, Contact);

/// One call in this phone's call history (docs/protocol/calls.md).
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct CallLogEntry {
    pub id: String,
    /// Who called, or who was called ("" when withheld).
    pub number: String,
    /// The contact's name, when the number is a contact.
    pub name: Option<String>,
    /// "incoming", "outgoing", "missed" or "rejected".
    pub direction: String,
    /// Unix milliseconds.
    pub date: i64,
    /// Seconds on the call (0 when missed or not answered).
    pub duration: u32,
    /// The contact's photo: a JPEG of at most 16 KB.
    pub photo: Option<Vec<u8>>,
}

/// One phone number on a contact (docs/protocol/contacts.md).
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactNumber {
    pub number: String,
    pub label: Option<String>,
}

/// A contact on this phone (docs/protocol/contacts.md).
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct Contact {
    pub id: String,
    pub name: String,
    pub numbers: Vec<ContactNumber>,
    pub starred: bool,
    /// A JPEG of at most 16 KB.
    pub photo: Option<Vec<u8>>,
}

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
    /// With `Active`: when it was answered, in Unix milliseconds.
    pub since: Option<i64>,
    /// With `Active`, when this app controls the call (its in-call service
    /// is bound): mute, speaker and hold.
    pub controls: Option<CallControls>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct CallControls {
    pub muted: bool,
    pub speaker: bool,
    pub held: bool,
    pub can_hold: bool,
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
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CallCommand {
    Answer,
    /// Decline a ringing call, or hang up an active one.
    Decline,
    /// Stop the ringing.
    Silence,
    /// Mute (true) or unmute the microphone on the call.
    Mute {
        on: bool,
    },
    /// The speaker (true) or the earpiece.
    Speaker {
        on: bool,
    },
    /// Put the call on hold (true) or take it off.
    Hold {
        on: bool,
    },
    /// A keypad tone: "0"–"9", "*" or "#".
    Dtmf {
        digit: String,
    },
    /// The call's volume up (true) or down.
    Volume {
        up: bool,
    },
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

/// An album (folder) in this phone's gallery (docs/protocol/photos.md).
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct PhotoAlbum {
    pub id: String,
    pub name: String,
    pub count: u32,
    /// Newest item's ID in the album, for its cover thumbnail.
    pub cover: Option<String>,
}

/// A photo or video in this phone's gallery (docs/protocol/photos.md).
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct PhotoItem {
    pub id: String,
    pub name: String,
    /// Unix milliseconds.
    pub date: i64,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    /// Video duration in milliseconds; `None` for still photos.
    pub duration: Option<u32>,
    pub album: Option<String>,
}

/// A JPEG thumbnail for a gallery item (docs/protocol/photos.md).
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct PhotoThumb {
    pub id: String,
    /// JPEG of at most 32 KiB, ~256 px on the longer side.
    pub data: Vec<u8>,
}

/// One file or folder in a phone directory listing (`docs/protocol/storage.md`).
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct StorageEntry {
    pub name: String,
    pub size: u64,
    /// Unix milliseconds.
    pub modified: i64,
    pub is_dir: bool,
}

/// An opened file on this phone ready for ranged reading (`storage.read`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct StorageReadFile {
    pub source: FileToSend,
    pub size: u64,
    /// Unix milliseconds.
    pub modified: i64,
}

/// Result of writing a file into this phone's storage (`storage.write`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct StorageWriteDone {
    pub size: u64,
    /// Unix milliseconds.
    pub modified: i64,
}

/// Whether a clipboard history item is text or an image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ClipboardItemKind {
    Text,
    Image,
}

impl From<core::ClipboardItemKind> for ClipboardItemKind {
    fn from(k: core::ClipboardItemKind) -> Self {
        match k {
            core::ClipboardItemKind::Text => ClipboardItemKind::Text,
            core::ClipboardItemKind::Image => ClipboardItemKind::Image,
        }
    }
}

/// One clip in the local clipboard history.
#[derive(Clone, PartialEq, Eq, uniffi::Record)]
pub struct ClipboardHistoryEntry {
    pub id: String,
    pub kind: ClipboardItemKind,
    pub text: String,
    pub mime: Option<String>,
    pub device_name: String,
    pub incoming: bool,
    pub timestamp: i64,
    pub pinned: bool,
}

impl From<core::ClipboardHistoryEntry> for ClipboardHistoryEntry {
    fn from(e: core::ClipboardHistoryEntry) -> Self {
        ClipboardHistoryEntry {
            id: e.id,
            kind: e.kind.into(),
            text: e.text,
            mime: e.mime,
            device_name: e.device_name,
            incoming: e.incoming,
            timestamp: e.timestamp,
            pinned: e.pinned,
        }
    }
}

private_debug!(PhotoAlbum, PhotoItem, PhotoThumb, StorageEntry, ClipboardHistoryEntry);

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
            can_wake: d.can_wake,
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

impl From<core::RecordingMarker> for RecordingMarker {
    fn from(m: core::RecordingMarker) -> Self {
        RecordingMarker { at_ms: m.at_ms, label: m.label }
    }
}

impl From<RecordingMarker> for core::RecordingMarker {
    fn from(m: RecordingMarker) -> Self {
        core::RecordingMarker { at_ms: m.at_ms, label: m.label }
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
            recording: t.recording,
            markers: t.markers.into_iter().map(Into::into).collect(),
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
            NodeEvent::ClipboardHistoryChanged => Event::ClipboardHistoryChanged,
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
            NodeEvent::CallLogChanged { device } => Event::CallLogChanged { id: device.to_string() },
            NodeEvent::ContactsChanged { device } => Event::ContactsChanged { id: device.to_string() },
            NodeEvent::SmsChanged { device, thread } => Event::SmsChanged { id: device.to_string(), thread },
            NodeEvent::PhotosChanged { device } => Event::PhotosChanged { id: device.to_string() },
            NodeEvent::Mirroring { device, session, on } => {
                Event::Mirroring { id: device.to_string(), session, on }
            }
            NodeEvent::Webcam { device, on } => Event::Webcam { id: device.to_string(), on },
            NodeEvent::RemoteInputRequested { device } => {
                Event::RemoteInputRequested { id: device.to_string() }
            }
            NodeEvent::PhoneToggles { device, toggles } => {
                Event::PhoneToggles { id: device.to_string(), toggles: toggles.into() }
            }
            NodeEvent::WakeInfoChanged { device, can_wake } => {
                Event::WakeInfoChanged { id: device.to_string(), can_wake }
            }
            NodeEvent::DeckLayout { device, layout } => {
                Event::DeckLayout { id: device.to_string(), layout: layout.into() }
            }
            NodeEvent::DeckState { device, state } => {
                Event::DeckState { id: device.to_string(), state: state.into() }
            }
            NodeEvent::StorageRequested { device } => Event::StorageRequested { id: device.to_string() },
            NodeEvent::StorageChanged { device, path } => {
                Event::StorageChanged { id: device.to_string(), path }
            }
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

/// Why a phone storage operation failed (`docs/protocol/storage.md`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum StorageFailure {
    #[error("no longer exists")]
    NotFound,
    #[error("access denied")]
    Denied,
    #[error("invalid path: {reason}")]
    Invalid { reason: String },
    #[error("not enough space")]
    NoSpace,
    #[error("not available")]
    Unsupported,
    #[error("failed: {reason}")]
    Failed { reason: String },
}

impl From<uniffi::UnexpectedUniFFICallbackError> for StorageFailure {
    fn from(e: uniffi::UnexpectedUniFFICallbackError) -> Self {
        StorageFailure::Failed { reason: e.reason }
    }
}

impl From<StorageFailure> for core::StorageError {
    fn from(f: StorageFailure) -> Self {
        match f {
            StorageFailure::NotFound => core::StorageError::NotFound,
            StorageFailure::Denied => core::StorageError::Denied,
            StorageFailure::Invalid { reason } => core::StorageError::Invalid(reason),
            StorageFailure::NoSpace => core::StorageError::NoSpace,
            StorageFailure::Unsupported => core::StorageError::Unsupported,
            StorageFailure::Failed { reason } => core::StorageError::Failed(reason),
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
    /// A PC asked for a photo or video from the gallery: open it, or
    /// `null` when it's gone.
    fn open_photo(&self, id: String) -> Option<FileToSend>;
    /// A PC asked for this phone's photo albums (folders), Camera and
    /// Screenshots first, then by count.
    fn photo_albums(&self) -> Vec<PhotoAlbum>;
    /// A PC asked for photos and videos in `album` (or all when null),
    /// newest first by date, then by media ID (highest first). When
    /// `before` is set, only items after the previous page's last one:
    /// older than `before`, or of that date with an ID below `before_id`
    /// (none of that date when `before_id` is empty).
    fn photo_list(
        &self,
        album: Option<String>,
        before: Option<i64>,
        before_id: String,
        limit: u32,
    ) -> Vec<PhotoItem>;
    /// A PC asked for small JPEG thumbnails (at most 32 KiB each) for
    /// these gallery item IDs.
    fn photo_thumbs(&self, ids: Vec<String>) -> Vec<PhotoThumb>;
    /// A PC asked to answer, decline or silence call `id` (the call in
    /// progress). False if the phone couldn't.
    fn call_command(&self, id: String, command: CallCommand) -> bool;
    /// A PC asked for recent calls before `before` (Unix ms; the latest when
    /// null), newest first.
    fn call_log(&self, before: Option<i64>, limit: u32) -> Vec<CallLogEntry>;
    /// A PC asked to call `number` (or open the dialer with it filled in).
    /// False if the phone couldn't.
    fn call_dial(&self, number: String) -> bool;
    /// A PC asked for contacts matching `query` (or all with phone numbers
    /// when null), favorites first then alphabetical, skipping `offset`.
    fn contacts(&self, query: Option<String>, offset: u32, limit: u32) -> Vec<Contact>;
    /// A PC asked for this phone's screen (and, with `audio`, its sound):
    /// ask the user (then call `mirror_open`, and `mirror_open_audio` for
    /// the sound). With `app` (and a `session` other than 0): run that app
    /// on a display of its own and stream it with `mirror_open`, its config
    /// packets carrying the session. False if it can't.
    fn mirror_requested(&self, pc_id: String, options: MirrorOptions) -> bool;
    /// The PC stopped watching the screen (session 0) or an app window.
    fn mirror_stop_requested(&self, pc_id: String, session: u32);
    /// The PC needs a keyframe.
    fn mirror_keyframe_requested(&self, pc_id: String, session: u32);
    /// The PC resized an app window (`session` != 0) to `width × height`.
    fn mirror_resize_requested(&self, pc_id: String, session: u32, width: u32, height: u32);
    /// The PC's mouse or keyboard on the mirrored screen (only while this
    /// phone offers `mirror.input`) or an app window. Return quickly.
    fn mirror_input(&self, pc_id: String, session: u32, input: MirrorInputEvent);
    /// A PC asked for this phone's camera as a webcam: start streaming (or
    /// ask the user) and call `webcam_open`. False if it can't or the user
    /// declined.
    fn webcam_requested(&self, pc_id: String, options: WebcamOptions) -> bool;
    /// The PC stopped watching the webcam stream.
    fn webcam_stop_requested(&self, pc_id: String);
    /// The PC needs a fresh H.264 keyframe on the webcam stream.
    fn webcam_keyframe_requested(&self, pc_id: String);
    /// A PC asked for the apps it may open in windows (launchable ones).
    fn phone_apps(&self) -> Vec<PhoneApp>;
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
    /// A PC asked to change one of this phone's quick settings (`id`: `"dnd"`,
    /// `"ringer"`, `"flashlight"`, `"volume"`, `"brightness"`, `"wifi"`,
    /// `"bluetooth"`). False if the phone couldn't.
    fn set_phone_toggle(&self, id: String, value: PhoneToggleValue) -> bool;
    /// A PC asked to list a folder in this phone's storage (`""` is root).
    fn storage_list(&self, path: String) -> Result<Vec<StorageEntry>, StorageFailure>;
    /// A PC asked to open a file in this phone's storage for ranged reading.
    fn storage_open_read(&self, path: String) -> Result<StorageReadFile, StorageFailure>;
    /// A PC finished uploading a file to `staged_path`: move/copy it into
    /// `path` in this phone's storage.
    fn storage_write(
        &self,
        path: String,
        staged_path: String,
        modified: Option<i64>,
    ) -> Result<StorageWriteDone, StorageFailure>;
    /// A PC asked to create a folder at `path` in this phone's storage.
    fn storage_mkdir(&self, path: String) -> Result<(), StorageFailure>;
    /// A PC asked to rename or move `from` to `to` in this phone's storage.
    fn storage_rename(&self, from: String, to: String) -> Result<(), StorageFailure>;
    /// A PC asked to delete `path` in this phone's storage (`confirmed`: the
    /// PC user confirmed permanent deletion if trash isn't available).
    fn storage_delete(&self, path: String, confirmed: bool) -> Result<(), StorageFailure>;
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
        let options = MirrorOptions {
            max_size: options.max_size,
            fps: options.fps,
            bitrate: options.bitrate,
            audio: options.audio,
            session: options.session,
            app: options.app.clone(),
        };
        if self.0.mirror_requested(peer.to_string(), options) {
            Ok(())
        } else {
            Err("the phone couldn't".into())
        }
    }
    fn mirror_stop_requested(&self, peer: &DeviceId, session: u32) {
        self.0.mirror_stop_requested(peer.to_string(), session);
    }
    fn mirror_keyframe_requested(&self, peer: &DeviceId, session: u32) {
        self.0.mirror_keyframe_requested(peer.to_string(), session);
    }
    fn mirror_resize_requested(&self, peer: &DeviceId, session: u32, width: u32, height: u32) {
        self.0.mirror_resize_requested(peer.to_string(), session, width, height);
    }
    fn mirror_input(&self, peer: &DeviceId, session: u32, input: core::MirrorInput) {
        self.0.mirror_input(peer.to_string(), session, input.into());
    }
    fn webcam_requested(&self, peer: &DeviceId, options: &core::WebcamStart) -> Result<(), String> {
        let options = WebcamOptions {
            camera: options.camera.clone(),
            width: options.width,
            height: options.height,
            fps: options.fps,
            bitrate: options.bitrate,
        };
        if self.0.webcam_requested(peer.to_string(), options) {
            Ok(())
        } else {
            Err("the phone couldn't".into())
        }
    }
    fn webcam_stop_requested(&self, peer: &DeviceId) {
        self.0.webcam_stop_requested(peer.to_string());
    }
    fn webcam_keyframe_requested(&self, peer: &DeviceId) {
        self.0.webcam_keyframe_requested(peer.to_string());
    }
    fn phone_apps(&self) -> Result<Vec<core::PhoneApp>, String> {
        Ok(self
            .0
            .phone_apps()
            .into_iter()
            .map(|a| core::PhoneApp { pkg: a.pkg, label: a.label, icon: a.icon })
            .collect())
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
            core::CallCommand::Mute(on) => CallCommand::Mute { on },
            core::CallCommand::Speaker(on) => CallCommand::Speaker { on },
            core::CallCommand::Hold(on) => CallCommand::Hold { on },
            core::CallCommand::Dtmf(digit) => CallCommand::Dtmf { digit: digit.to_string() },
            core::CallCommand::Volume(up) => CallCommand::Volume { up },
        };
        if self.0.call_command(id.to_owned(), command) { Ok(()) } else { Err("the phone couldn't".into()) }
    }
    fn call_log(&self, before: Option<i64>, limit: u32) -> Result<Vec<core::CallLogEntry>, String> {
        Ok(self
            .0
            .call_log(before, limit)
            .into_iter()
            .map(|e| core::CallLogEntry {
                id: e.id,
                number: e.number,
                name: e.name,
                direction: e.direction,
                date: e.date,
                duration: e.duration,
                photo: e.photo,
            })
            .collect())
    }
    fn call_dial(&self, number: &str) -> Result<(), String> {
        if self.0.call_dial(number.to_owned()) { Ok(()) } else { Err("the phone couldn't".into()) }
    }
    fn contacts(&self, query: Option<&str>, offset: u32, limit: u32) -> Result<Vec<core::Contact>, String> {
        Ok(self
            .0
            .contacts(query.map(str::to_owned), offset, limit)
            .into_iter()
            .map(|c| core::Contact {
                id: c.id,
                name: c.name,
                numbers: c
                    .numbers
                    .into_iter()
                    .map(|n| core::ContactNumber { number: n.number, label: n.label })
                    .collect(),
                starred: c.starred,
                photo: c.photo,
            })
            .collect())
    }
    fn open_photo(&self, id: &str) -> Result<core::OutgoingFile, String> {
        let file = self.0.open_photo(id.to_owned()).ok_or("it's gone")?;
        file_to_send(file).map_err(|e| e.to_string())
    }
    fn photo_albums(&self) -> Result<Vec<core::PhotoAlbum>, String> {
        Ok(self
            .0
            .photo_albums()
            .into_iter()
            .map(|a| core::PhotoAlbum { id: a.id, name: a.name, count: a.count, cover: a.cover })
            .collect())
    }
    fn photo_list(
        &self,
        album: Option<&str>,
        before: Option<(i64, &str)>,
        limit: u32,
    ) -> Result<Vec<core::PhotoItem>, String> {
        let (date, id) = before.map_or((None, String::new()), |(date, id)| (Some(date), id.to_owned()));
        Ok(self
            .0
            .photo_list(album.map(str::to_owned), date, id, limit)
            .into_iter()
            .map(|i| core::PhotoItem {
                id: i.id,
                name: i.name,
                date: i.date,
                size: i.size,
                width: i.width,
                height: i.height,
                duration: i.duration,
                album: i.album,
            })
            .collect())
    }
    fn photo_thumbs(&self, ids: &[String]) -> Result<Vec<core::PhotoThumb>, String> {
        Ok(self
            .0
            .photo_thumbs(ids.to_vec())
            .into_iter()
            .map(|t| core::PhotoThumb { id: t.id, data: t.data })
            .collect())
    }
    fn set_phone_toggle(&self, id: &str, value: &core::PhoneToggleValue) -> Result<(), String> {
        let value = match value {
            core::PhoneToggleValue::Bool(on) => PhoneToggleValue::Bool { on: *on },
            core::PhoneToggleValue::Level(level) => PhoneToggleValue::Level { level: *level },
            core::PhoneToggleValue::Mode(mode) => PhoneToggleValue::Mode { mode: mode.clone() },
        };
        if self.0.set_phone_toggle(id.to_owned(), value) { Ok(()) } else { Err("the phone couldn't".into()) }
    }
    fn storage_list(&self, path: &str) -> Result<Vec<core::StorageEntry>, core::StorageError> {
        let items = self.0.storage_list(path.to_owned())?;
        Ok(items
            .into_iter()
            .map(|e| core::StorageEntry {
                name: e.name,
                size: e.size,
                modified: e.modified,
                is_dir: e.is_dir,
            })
            .collect())
    }
    fn storage_open_read(&self, path: &str) -> Result<core::StorageReadFile, core::StorageError> {
        let file = self.0.storage_open_read(path.to_owned())?;
        let outgoing = file_to_send(file.source).map_err(|e| core::StorageError::Failed(e.to_string()))?;
        Ok(core::StorageReadFile { source: outgoing.source, size: file.size, modified: file.modified })
    }
    fn storage_write(
        &self,
        path: &str,
        staged: &std::path::Path,
        modified: Option<i64>,
    ) -> Result<core::StorageWriteDone, core::StorageError> {
        let staged_str = staged.to_string_lossy().into_owned();
        let done = self.0.storage_write(path.to_owned(), staged_str, modified)?;
        Ok(core::StorageWriteDone { size: done.size, modified: done.modified })
    }
    fn storage_mkdir(&self, path: &str) -> Result<(), core::StorageError> {
        Ok(self.0.storage_mkdir(path.to_owned())?)
    }
    fn storage_rename(&self, from: &str, to: &str) -> Result<(), core::StorageError> {
        Ok(self.0.storage_rename(from.to_owned(), to.to_owned())?)
    }
    fn storage_delete(&self, path: &str, confirmed: bool) -> Result<(), core::StorageError> {
        Ok(self.0.storage_delete(path.to_owned(), confirmed)?)
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

    /// Sends a voice recording and its markers to a paired PC; returns the
    /// transfer's ID. Progress arrives as `Event::Transfer`.
    pub async fn send_recording(
        &self,
        id: String,
        file: FileToSend,
        markers: Vec<RecordingMarker>,
    ) -> Result<String> {
        let id = parse_id(&id)?;
        let file = file_to_send(file)?;
        let markers = markers.into_iter().map(Into::into).collect();
        let node = self.node.clone();
        self.run(async move { Ok(node.send_recording(id, file, markers).await?) }).await
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

    /// Whether the local clipboard history is enabled.
    pub fn clipboard_history_enabled(&self) -> bool {
        self.node.clipboard_history_enabled()
    }

    /// Turns the local clipboard history on or off.
    pub fn set_clipboard_history_enabled(&self, enabled: bool) -> Result<()> {
        Ok(self.node.set_clipboard_history_enabled(enabled)?)
    }

    /// Lists the local clipboard history (pinned first, then newest first),
    /// optionally filtered by `query`.
    pub fn clipboard_history(&self, query: Option<String>) -> Vec<ClipboardHistoryEntry> {
        self.node.clipboard_history(query.as_deref()).into_iter().map(Into::into).collect()
    }

    /// Decrypts and returns the image bytes for an image entry in the
    /// clipboard history, or `None` if not found.
    pub fn clipboard_history_image(&self, id: String) -> Option<Vec<u8>> {
        self.node.clipboard_history_image(&id).map(|(_, bytes)| bytes)
    }

    /// Pins or unpins an entry in the clipboard history.
    pub fn pin_clipboard_history(&self, id: String, pinned: bool) -> bool {
        self.node.pin_clipboard_history(&id, pinned)
    }

    /// Deletes one entry from the clipboard history.
    pub fn delete_clipboard_history(&self, id: String) -> bool {
        self.node.delete_clipboard_history(&id)
    }

    /// Clears the entire clipboard history and deletes its encrypted files from disk.
    pub fn clear_clipboard_history(&self) -> Result<()> {
        Ok(self.node.clear_clipboard_history()?)
    }

    /// Copies a clipboard history entry back onto this device's OS clipboard.
    pub fn copy_clipboard_history(&self, id: String) -> Result<()> {
        Ok(self.node.copy_clipboard_history(&id)?)
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

    /// Sends Wake-on-LAN magic packets for a paired PC over UDP to its stored
    /// subnet broadcasts and `255.255.255.255` on ports 9 and 7, repeated a
    /// few times over ~2 s.
    pub async fn wake(&self, id: String) -> Result<()> {
        let id = parse_id(&id)?;
        let node = self.node.clone();
        self.run(async move { Ok(node.wake(id).await?) }).await
    }

    /// Opens a web link on a paired PC.
    pub async fn open_link(&self, id: String, url: String) -> Result<()> {
        let id = parse_id(&id)?;
        let node = self.node.clone();
        self.run(async move { Ok(node.open_link(id, url).await?) }).await
    }

    // ---- Remote input (touchpad, keyboard, presentation) ----

    /// Checks whether a paired PC allows remote mouse/keyboard input from this
    /// phone (`NectarlinkError::Denied` when `remote_input` is off on the PC,
    /// which also triggers a one-time confirmation prompt there).
    pub async fn remote_check(&self, id: String) -> Result<()> {
        let id = parse_id(&id)?;
        let node = self.node.clone();
        self.run(async move { Ok(node.remote_check(id).await?) }).await
    }

    /// Moves a paired PC's mouse cursor by `(dx, dy)` logical pixels over
    /// QUIC datagrams (or the `remote/motion` stream fallback).
    pub async fn remote_move(&self, id: String, dx: f32, dy: f32) -> Result<()> {
        let id = parse_id(&id)?;
        let node = self.node.clone();
        self.run(async move { Ok(node.remote_move(id, dx, dy).await?) }).await
    }

    /// Scrolls on a paired PC (`dy` > 0 scrolls down, `dx` > 0 scrolls right,
    /// in wheel notches). When `fast` is true, sends over QUIC datagrams.
    pub async fn remote_scroll(&self, id: String, dx: f32, dy: f32, fast: bool) -> Result<()> {
        let id = parse_id(&id)?;
        let node = self.node.clone();
        self.run(async move {
            if fast {
                Ok(node.remote_scroll_fast(id, dx, dy).await?)
            } else {
                Ok(node.remote_input(id, core::RemoteInput::Scroll { dx, dy }).await?)
            }
        })
        .await
    }

    /// Presses, releases or clicks a mouse button (`"left"`, `"right"` or
    /// `"middle"`; `action`: `"down"`, `"up"` or `"click"`) on a paired PC.
    pub async fn remote_button(&self, id: String, button: String, action: String) -> Result<()> {
        let id = parse_id(&id)?;
        let button = match button.as_str() {
            "left" => core::MouseButton::Left,
            "right" => core::MouseButton::Right,
            "middle" => core::MouseButton::Middle,
            _ => return Err(NectarlinkError::Internal { reason: format!("invalid mouse button: {button}") }),
        };
        let action = match action.as_str() {
            "down" => core::ButtonAction::Down,
            "up" => core::ButtonAction::Up,
            "click" => core::ButtonAction::Click,
            _ => {
                return Err(NectarlinkError::Internal { reason: format!("invalid button action: {action}") });
            }
        };
        let node = self.node.clone();
        self.run(
            async move { Ok(node.remote_input(id, core::RemoteInput::Button { button, action }).await?) },
        )
        .await
    }

    /// Types Unicode text on a paired PC.
    pub async fn remote_text(&self, id: String, text: String) -> Result<()> {
        let id = parse_id(&id)?;
        let node = self.node.clone();
        self.run(async move { Ok(node.remote_input(id, core::RemoteInput::Text { text }).await?) }).await
    }

    /// Presses a named key or shortcut (`key`) with optional modifiers
    /// (`"ctrl"`, `"alt"`, `"shift"`, `"win"`) on a paired PC.
    pub async fn remote_key(&self, id: String, key: String, mods: Vec<String>) -> Result<()> {
        let id = parse_id(&id)?;
        let mut parsed_mods = Vec::with_capacity(mods.len());
        for m in mods {
            let km = match m.as_str() {
                "ctrl" => core::KeyMod::Ctrl,
                "alt" => core::KeyMod::Alt,
                "shift" => core::KeyMod::Shift,
                "win" => core::KeyMod::Win,
                _ => return Err(NectarlinkError::Internal { reason: format!("invalid key modifier: {m}") }),
            };
            parsed_mods.push(km);
        }
        let node = self.node.clone();
        self.run(async move {
            Ok(node.remote_input(id, core::RemoteInput::Key { key, mods: parsed_mods }).await?)
        })
        .await
    }

    /// Sends a presentation slide command (`"next"`, `"previous"`, `"start"`,
    /// `"stop"` or `"black"`) to a paired PC.
    pub async fn remote_slide(&self, id: String, action: String) -> Result<()> {
        let id = parse_id(&id)?;
        let action = match action.as_str() {
            "next" => core::SlideAction::Next,
            "previous" | "prev" => core::SlideAction::Previous,
            "start" => core::SlideAction::Start,
            "stop" => core::SlideAction::Stop,
            "black" => core::SlideAction::Black,
            _ => return Err(NectarlinkError::Internal { reason: format!("invalid slide action: {action}") }),
        };
        let node = self.node.clone();
        self.run(async move { Ok(node.remote_input(id, core::RemoteInput::Slide { action }).await?) }).await
    }

    /// Moves or hides the laser pointer overlay on a paired PC (`x`, `y` in
    /// `0.0..=1.0`).
    pub async fn remote_laser(&self, id: String, on: bool, x: f32, y: f32) -> Result<()> {
        let id = parse_id(&id)?;
        let node = self.node.clone();
        self.run(async move { Ok(node.remote_laser(id, on, x, y).await?) }).await
    }

    // ---- Deck (macro pad) ----

    /// The latest Deck layout reported by a connected PC, if any.
    pub fn deck_layout(&self, id: String) -> Result<Option<DeckLayout>> {
        let id = parse_id(&id)?;
        Ok(self.node.deck_layout(id).map(Into::into))
    }

    /// The latest live Deck state reported by a connected PC, if any.
    pub fn deck_state(&self, id: String) -> Result<Option<DeckState>> {
        let id = parse_id(&id)?;
        Ok(self.node.deck_state(id).map(Into::into))
    }

    /// Asks a paired PC to run the action bound to `tile` (`deck.press`).
    pub async fn deck_press(&self, id: String, tile: String) -> Result<()> {
        let id = parse_id(&id)?;
        let node = self.node.clone();
        self.run(async move { Ok(node.deck_press(id, tile).await?) }).await
    }

    /// Asks a paired PC to set its master volume (`0..=100`) and/or mute state (`pc.audio.set`).
    pub async fn set_pc_audio(&self, id: String, volume: Option<u8>, muted: Option<bool>) -> Result<()> {
        let id = parse_id(&id)?;
        let node = self.node.clone();
        self.run(async move { Ok(node.set_pc_audio(id, volume, muted).await?) }).await
    }

    /// The context chip suggestion for the most recently received text clip, if any.
    pub fn last_clip_suggestion(&self) -> Option<ClipSuggestion> {
        self.node.last_clip_suggestion().map(|(_, s)| s.into())
    }

    /// Opens this phone's screen stream to a PC that asked (after the user
    /// agreed).
    pub async fn mirror_open(&self, pc_id: String) -> Result<Arc<MirrorStream>> {
        let id = parse_id(&pc_id)?;
        let node = self.node.clone();
        self.run(async move { Ok(Arc::new(MirrorStream(node.mirror_open(id).await?))) }).await
    }

    /// Opens this phone's sound stream to a PC that asked for it with the
    /// screen.
    pub async fn mirror_open_audio(&self, pc_id: String) -> Result<Arc<MirrorStream>> {
        let id = parse_id(&pc_id)?;
        let node = self.node.clone();
        self.run(async move { Ok(Arc::new(MirrorStream(node.mirror_open_audio(id).await?))) }).await
    }

    /// Opens this phone's camera stream to a paired PC (`docs/protocol/webcam.md`).
    pub async fn webcam_open(&self, pc_id: String) -> Result<Arc<MirrorStream>> {
        let id = parse_id(&pc_id)?;
        let node = self.node.clone();
        self.run(async move { Ok(Arc::new(MirrorStream(node.webcam_open(id).await?))) }).await
    }

    /// Stops this phone's webcam stream to a paired PC and tells the PC.
    pub async fn webcam_stop(&self, pc_id: String) -> Result<()> {
        let id = parse_id(&pc_id)?;
        let node = self.node.clone();
        self.run(async move {
            node.webcam_stop(id).await;
            Ok(())
        })
        .await
    }

    /// This phone's messages changed (in `thread`, or anywhere when null):
    /// PCs that show them catch up.
    pub async fn sms_changed(&self, thread: Option<String>) {
        let node = self.node.clone();
        self.run(async move { node.sms_changed(thread).await }).await;
    }

    /// This phone's call history changed: PCs that show it catch up.
    pub async fn call_log_changed(&self) {
        let node = self.node.clone();
        self.run(async move { node.call_log_changed().await }).await;
    }

    /// This phone's contacts changed: PCs that show them catch up.
    pub async fn contacts_changed(&self) {
        let node = self.node.clone();
        self.run(async move { node.contacts_changed().await }).await;
    }

    /// This phone's photo library changed: PCs that show it catch up.
    pub async fn photos_changed(&self) {
        let node = self.node.clone();
        self.run(async move { node.photos_changed().await }).await;
    }

    /// A folder on this phone changed (`""` for root): PCs that have it open
    /// in File Explorer refresh it (`storage.changed`, debounced).
    pub async fn storage_changed(&self, path: String) {
        let node = self.node.clone();
        self.run(async move { node.storage_changed(path).await }).await;
    }

    /// Directory paths currently watched by any connected PC (most recently
    /// listed folders).
    pub fn storage_open_folders(&self) -> Vec<String> {
        self.node.storage_open_folders()
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
            since: call.since,
            controls: call.controls.map(|c| core::CallControls {
                muted: c.muted,
                speaker: c.speaker,
                held: c.held,
                can_hold: c.can_hold,
            }),
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

    /// This phone's quick settings state (on startup and whenever any toggle
    /// changes): tells the PCs that show toggles (and are allowed them).
    pub async fn toggles_changed(&self, toggles: PhoneToggles) -> Result<()> {
        let node = self.node.clone();
        let toggles: core::PhoneToggles = toggles.into();
        self.run(async move { Ok(node.toggles_changed(toggles).await?) }).await
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

/// Classifies a text clip into a single context chip suggestion (`None` for
/// OTPs or plain text).
#[uniffi::export]
pub fn classify_clip(text: String) -> Option<ClipSuggestion> {
    core::classify_clip(&text).map(Into::into)
}

/// Builds a web search URL for a parcel tracking number.
#[uniffi::export]
pub fn tracking_search_url(tracking_number: String) -> String {
    core::tracking_search_url(&tracking_number)
}

/// Builds a web maps URL for a street address.
#[uniffi::export]
pub fn maps_web_url(address: String) -> String {
    core::maps_web_url(&address)
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
