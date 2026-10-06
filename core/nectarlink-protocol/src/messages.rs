// SPDX-License-Identifier: MPL-2.0
//! Message bodies for protocol v0 (protocol §5, §7, §9).

use serde::{Deserialize, Serialize};

use crate::{DeviceId, ErrorCode};

/// Message type names (the envelope's `t` field).
pub mod types {
    pub const HELLO: &str = "hello";
    pub const HELLO_UPDATE: &str = "hello.update";
    pub const PING: &str = "ping";
    pub const PONG: &str = "pong";
    pub const OK: &str = "ok";
    pub const ERROR: &str = "error";
    pub const STREAM: &str = "stream";
    pub const EVENT_BATTERY: &str = "event.battery";
    pub const EVENT_DEVICE: &str = "event.device";
    pub const DEVICE_RING: &str = "device.ring";

    pub const PAIR_REQUEST: &str = "pair.request";
    pub const PAIR_ACCEPT: &str = "pair.accept";
    pub const PAIR_COMMIT: &str = "pair.commit";
    pub const PAIR_NONCE: &str = "pair.nonce";
    pub const PAIR_REVEAL: &str = "pair.reveal";
    pub const PAIR_CONFIRM: &str = "pair.confirm";
    pub const PAIR_DONE: &str = "pair.done";
    pub const PAIR_REVOKE: &str = "pair.revoke";

    pub const NOTIFY_SNAPSHOT: &str = "notify.snapshot";
    pub const NOTIFY_POSTED: &str = "notify.posted";
    pub const NOTIFY_REMOVED: &str = "notify.removed";
    pub const NOTIFY_SYNC: &str = "notify.sync";

    pub const CLIP_SET: &str = "clip.set";
    pub const NOTIFY_DISMISS: &str = "notify.dismiss";
    pub const NOTIFY_ACTION: &str = "notify.action";
    pub const MEDIA_STATE: &str = "media.state";
    pub const MEDIA_SYNC: &str = "media.sync";
    pub const MEDIA_COMMAND: &str = "media.command";
    pub const PC_POWER: &str = "pc.power";
    pub const LINK_OPEN: &str = "link.open";
    pub const PHOTOS_NEW: &str = "photos.new";
    pub const PHOTOS_GET: &str = "photos.get";
    pub const PHOTOS_SENDING: &str = "photos.sending";
    pub const CALL_STATE: &str = "call.state";
    pub const CALL_ACTION: &str = "call.action";
    pub const SMS_THREADS: &str = "sms.threads";
    pub const SMS_MESSAGES: &str = "sms.messages";
    pub const SMS_SEND: &str = "sms.send";
    pub const SMS_PART: &str = "sms.part";
    pub const SMS_CHANGED: &str = "sms.changed";
    pub const MIRROR_START: &str = "mirror.start";
    pub const MIRROR_STOP: &str = "mirror.stop";
    pub const MIRROR_KEYFRAME: &str = "mirror.keyframe";
    pub const MIRROR_INPUT: &str = "mirror.input";
}

/// What kind of device this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    Phone,
    Tablet,
    Desktop,
    Laptop,
    #[serde(other)]
    Unknown,
}

/// A device's power level (see PLAN.md §4.6). Desktops report `NotApplicable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PowerLevel {
    Basic,
    Assist,
    Elevated,
    #[serde(rename = "n/a")]
    NotApplicable,
    /// Unknown values (e.g. from a newer peer) are treated as Basic.
    #[serde(other)]
    Unknown,
}

impl PowerLevel {
    /// The level to use for capability decisions.
    pub fn effective(self) -> PowerLevel {
        match self {
            PowerLevel::Unknown => PowerLevel::Basic,
            other => other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub name: String,
    pub kind: DeviceKind,
    pub os: String,
    pub os_ver: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Optional ARGB seed color for Material You sync.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub proto: u32,
    pub min: u32,
    pub app: String,
    pub device: DeviceInfo,
    #[serde(default)]
    pub caps: Vec<String>,
    pub power: PowerLevel,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caps: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power: Option<PowerLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<DeviceInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ping {
    /// Sender's monotonic clock in milliseconds.
    pub ts: u64,
}

/// Echoes the `ts` of the matching ping.
pub type Pong = Ping;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Battery {
    pub level: u8,
    pub charging: bool,
    /// "ac", "usb" or "wireless".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugged: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ring {
    pub on: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: ErrorCode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub msg: Option<String>,
}

/// First frame of every non-control stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamHeader {
    pub svc: String,
    pub op: String,
    pub v: u32,
}

/// An empty body (`{}`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Empty {}

// ---- Pairing (protocol §9) ----

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairRequest {
    pub device: DeviceInfo,
    #[serde(with = "serde_bytes")]
    pub proof: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairAccept {
    pub device: DeviceInfo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairCommit {
    pub device: DeviceInfo,
    #[serde(with = "serde_bytes")]
    pub c: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairNonce {
    pub device: DeviceInfo,
    #[serde(rename = "nB", with = "serde_bytes")]
    pub n_b: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairReveal {
    #[serde(rename = "nA", with = "serde_bytes")]
    pub n_a: Vec<u8>,
}

// ---- Notifications (docs/protocol/notifications.md) ----

/// Size limits from the notifications spec (§2); senders and receivers both
/// enforce them with [`Notification::sanitized`].
pub mod notify_limits {
    pub const KEY_BYTES: usize = 256;
    pub const ACTION_ID_BYTES: usize = 64;
    pub const TITLE_CHARS: usize = 256;
    pub const TEXT_CHARS: usize = 4096;
    pub const ACTION_TITLE_CHARS: usize = 64;
    pub const ACTIONS: usize = 5;
    pub const ICON_BYTES: usize = 64 * 1024;
    pub const IMAGE_BYTES: usize = 160 * 1024;
    pub const SNAPSHOT_ITEMS: usize = 100;
}

/// A button on a notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationAction {
    /// Opaque to the PC.
    pub id: String,
    pub title: String,
    /// Takes text: shown as an inline reply field.
    #[serde(default)]
    pub reply: bool,
}

/// A phone notification, as mirrored to a PC.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notification {
    /// The phone's ID for it.
    pub key: String,
    /// Package name, e.g. "com.whatsapp".
    pub app: String,
    /// User-visible app name.
    pub app_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Secondary line, e.g. a conversation or account name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,
    /// When it was posted, Unix milliseconds.
    pub when: i64,
    #[serde(default)]
    pub actions: Vec<NotificationAction>,
    /// Arrived without sound or pop-up on the phone.
    #[serde(default)]
    pub silent: bool,
    /// The app's icon (PNG), sent with an app's first notification per
    /// session.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "serde_bytes")]
    pub icon: Option<Vec<u8>>,
    /// A picture it shows (a photo in a message, a big picture), JPEG.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "serde_bytes")]
    pub image: Option<Vec<u8>>,
}

/// Never prints content: notification text must not reach logs (protocol
/// v0 §11).
impl std::fmt::Debug for Notification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Notification")
            .field("app", &self.app)
            .field("actions", &self.actions.len())
            .field("silent", &self.silent)
            .finish_non_exhaustive()
    }
}

fn truncate_chars(s: &mut String, max: usize) {
    if let Some((cut, _)) = s.char_indices().nth(max) {
        s.truncate(cut);
    }
}

fn clean_text(s: Option<String>, max: usize) -> Option<String> {
    let mut s = s?.trim().to_owned();
    truncate_chars(&mut s, max);
    (!s.is_empty()).then_some(s)
}

impl Notification {
    /// Applies the spec's limits: truncates text, drops extra actions,
    /// oversized icons and malformed actions. `None` if the notification
    /// can't be shown at all (no key, or neither a title nor a text).
    pub fn sanitized(mut self) -> Option<Notification> {
        use notify_limits::*;
        if self.key.is_empty() || self.key.len() > KEY_BYTES {
            return None;
        }
        self.title = clean_text(self.title, TITLE_CHARS);
        self.text = clean_text(self.text, TEXT_CHARS);
        self.sub = clean_text(self.sub, TITLE_CHARS);
        if self.title.is_none() && self.text.is_none() {
            return None;
        }
        truncate_chars(&mut self.app, TITLE_CHARS);
        truncate_chars(&mut self.app_name, TITLE_CHARS);
        self.actions.retain_mut(|a| {
            truncate_chars(&mut a.title, ACTION_TITLE_CHARS);
            !a.id.is_empty() && a.id.len() <= ACTION_ID_BYTES && !a.title.trim().is_empty()
        });
        self.actions.truncate(ACTIONS);
        if self.icon.as_ref().is_some_and(|i| i.is_empty() || i.len() > ICON_BYTES) {
            self.icon = None;
        }
        if self.image.as_ref().is_some_and(|i| i.is_empty() || i.len() > IMAGE_BYTES) {
            self.image = None;
        }
        Some(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotifySnapshot {
    pub items: Vec<Notification>,
}

/// Body of `notify.removed` and `notify.dismiss`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotifyKey {
    pub key: String,
}

/// Body of `notify.action`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotifyAction {
    pub key: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
}

impl std::fmt::Debug for NotifyAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotifyAction").field("action", &self.action).finish_non_exhaustive()
    }
}

// ---- Media (docs/protocol/media.md) ----

/// Limits from docs/protocol/media.md.
pub mod media_limits {
    pub const PLAYERS: usize = 8;
    pub const ID_BYTES: usize = 256;
    pub const TEXT_CHARS: usize = 256;
    pub const ART_KEY_BYTES: usize = 64;
    pub const ART_BYTES: usize = 256 * 1024;
}

/// What a player can be asked to do (`MediaPlayer::actions`,
/// `MediaCommand::action`).
pub mod media_actions {
    pub const PLAY: &str = "play";
    pub const PAUSE: &str = "pause";
    pub const NEXT: &str = "next";
    pub const PREVIOUS: &str = "previous";
    pub const SEEK: &str = "seek";
    pub const ALL: &[&str] = &[PLAY, PAUSE, NEXT, PREVIOUS, SEEK];
}

/// Something playing (or paused) on a device.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaPlayer {
    /// The device's ID for the player, stable while it exists (e.g. the
    /// app's package name).
    pub id: String,
    /// User-visible app name.
    pub app: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    #[serde(default)]
    pub playing: bool,
    /// Length, milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<u64>,
    /// Position when this was sent, milliseconds; it moves on in real time
    /// while `playing`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<u64>,
    /// Commands it takes, from [`media_actions`].
    #[serde(default)]
    pub actions: Vec<String>,
    /// Identifies the artwork; the same key means the same picture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub art_key: Option<String>,
    /// The artwork (JPEG or PNG), sent once per key and session.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "serde_bytes")]
    pub art: Option<Vec<u8>>,
}

/// Never prints what's playing (protocol v0 §11).
impl std::fmt::Debug for MediaPlayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaPlayer")
            .field("id", &self.id)
            .field("playing", &self.playing)
            .field("actions", &self.actions)
            .finish_non_exhaustive()
    }
}

impl MediaPlayer {
    /// Applies the spec's limits. `None` if it can't be shown (no ID, or
    /// nothing to show it by).
    pub fn sanitized(mut self) -> Option<MediaPlayer> {
        use media_limits::*;
        if self.id.is_empty() || self.id.len() > ID_BYTES {
            return None;
        }
        truncate_chars(&mut self.app, TEXT_CHARS);
        self.title = clean_text(self.title, TEXT_CHARS);
        self.artist = clean_text(self.artist, TEXT_CHARS);
        self.album = clean_text(self.album, TEXT_CHARS);
        if self.title.is_none() && self.app.trim().is_empty() {
            return None;
        }
        self.actions.retain(|a| media_actions::ALL.contains(&a.as_str()));
        self.actions.dedup();
        if let (Some(position), Some(duration)) = (self.position, self.duration) {
            self.position = Some(position.min(duration));
        }
        if self.art_key.as_ref().is_some_and(|k| k.is_empty() || k.len() > ART_KEY_BYTES) {
            self.art_key = None;
        }
        if self.art_key.is_none() || self.art.as_ref().is_some_and(|a| a.is_empty() || a.len() > ART_BYTES) {
            self.art = None;
        }
        Some(self)
    }
}

/// Body of `media.state`: every player, most relevant first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaState {
    pub players: Vec<MediaPlayer>,
}

/// Body of `media.command`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaCommand {
    pub player: String,
    pub action: String,
    /// For `seek`: where to, milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<u64>,
}

// ---- Actions (docs/protocol/actions.md) ----

/// Body of `pc.power`: `lock` or `sleep`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PcPower {
    pub action: String,
}

/// The longest link sent, in bytes.
pub const LINK_MAX_BYTES: usize = 4096;

// ---- Screen mirroring (docs/protocol/mirror.md) ----

pub mod mirror {
    /// Offered by phones that share their screen (with the user's consent).
    pub const CAPTURE: &str = "mirror.capture";
    /// Offered by PCs that show a phone's screen.
    pub const VIEW: &str = "mirror.view";
    pub const SERVICE: &str = "mirror";
    pub const OP_VIDEO: &str = "video";
    pub const VERSION: u32 = 1;
    /// Stream reset code: mirroring stopped.
    pub const STOPPED: u32 = 11;
    /// Offered by phones that take input from the PC while mirrored.
    pub const INPUT: &str = "mirror.input";
    /// Typed text in one message: at most this many bytes.
    pub const MAX_TEXT_BYTES: usize = 4096;
    pub const H264: &str = "h264";
    /// Offered by phones that stream all their sound while mirrored.
    pub const AUDIO: &str = "mirror.audio";
    /// Offered by phones that stream the sound of apps that allow it.
    pub const AUDIO_PLAYBACK: &str = "mirror.audio.playback";
    /// Offered by PCs that play a mirrored phone's sound.
    pub const LISTEN: &str = "mirror.listen";
    pub const OP_AUDIO: &str = "audio";
    /// 16-bit little-endian PCM, channels interleaved.
    pub const PCM: &str = "pcm_s16le";
}

/// Body of `mirror.start`: show me your screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MirrorStart {
    /// The longer side, in pixels, at most (the phone scales down).
    pub max_size: u32,
    /// Frames per second, at most.
    pub fps: u32,
    /// Bits per second to aim for.
    pub bitrate: u32,
    /// Also stream the phone's sound, when it can.
    #[serde(default)]
    pub audio: bool,
}

/// Body of `mirror.input`: the PC's mouse and keyboard, on the mirrored
/// screen. Positions are fractions of the screen (0 at the left or top, 1
/// at the right or bottom), so they hold at any size.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum MirrorInput {
    /// A finger: down, moved, up (the phone turns these into taps, long
    /// presses and swipes).
    Touch { action: TouchAction, x: f32, y: f32 },
    /// The mouse wheel at a point, in notches (positive: down / right).
    Scroll { x: f32, y: f32, dx: f32, dy: f32 },
    /// A key with no text: one of [`mirror_keys`].
    Key { key: String },
    /// Typed text, into what has the focus.
    Text { text: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TouchAction {
    Down,
    Move,
    Up,
}

/// Keys `mirror.input` can send.
pub mod mirror_keys {
    pub const BACK: &str = "back";
    pub const HOME: &str = "home";
    pub const RECENTS: &str = "recents";
    pub const ENTER: &str = "enter";
    pub const BACKSPACE: &str = "backspace";
    pub const DELETE: &str = "delete";
    pub const LEFT: &str = "left";
    pub const RIGHT: &str = "right";
    pub const UP: &str = "up";
    pub const DOWN: &str = "down";
    pub const TAB: &str = "tab";
    pub const NOTIFICATIONS: &str = "notifications";
    pub const ALL: &[&str] =
        &[BACK, HOME, RECENTS, ENTER, BACKSPACE, DELETE, LEFT, RIGHT, UP, DOWN, TAB, NOTIFICATIONS];
}

/// Never prints typed text (protocol v0 §11).
impl std::fmt::Debug for MirrorInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MirrorInput::Touch { action, .. } => write!(f, "Touch({action:?})"),
            MirrorInput::Scroll { .. } => f.write_str("Scroll"),
            MirrorInput::Key { key } => write!(f, "Key({key})"),
            MirrorInput::Text { text } => write!(f, "Text({} bytes)", text.len()),
        }
    }
}

impl MirrorInput {
    pub fn is_valid(&self) -> bool {
        let on_screen = |v: f32| v.is_finite() && (0.0..=1.0).contains(&v);
        match self {
            MirrorInput::Touch { x, y, .. } => on_screen(*x) && on_screen(*y),
            MirrorInput::Scroll { x, y, dx, dy } => {
                on_screen(*x)
                    && on_screen(*y)
                    && dx.is_finite()
                    && dy.is_finite()
                    && dx.abs() <= 100.0
                    && dy.abs() <= 100.0
            }
            MirrorInput::Key { key } => mirror_keys::ALL.contains(&key.as_str()),
            MirrorInput::Text { text } => !text.is_empty() && text.len() <= mirror::MAX_TEXT_BYTES,
        }
    }
}

/// The format of a mirroring stream (a config packet's data, in CBOR).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MirrorConfig {
    /// `h264`.
    pub codec: String,
    pub width: u32,
    pub height: u32,
}

impl MirrorConfig {
    pub fn to_cbor(&self) -> Vec<u8> {
        let mut out = Vec::new();
        ciborium::into_writer(self, &mut out).expect("writing to a Vec cannot fail");
        out
    }

    pub fn from_cbor(bytes: &[u8]) -> Result<MirrorConfig, crate::ProtocolError> {
        ciborium::from_reader(bytes).map_err(|e| crate::ProtocolError::BadMessage(e.to_string()))
    }

    /// A format this version can show: H.264, 1–8192 pixels a side.
    pub fn is_valid(&self) -> bool {
        self.codec == mirror::H264 && (1..=8192).contains(&self.width) && (1..=8192).contains(&self.height)
    }
}

/// The format of a mirrored phone's sound (an audio stream's config
/// packet, in CBOR).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MirrorAudioConfig {
    /// `pcm_s16le`.
    pub codec: String,
    /// Samples per second.
    pub rate: u32,
    pub channels: u8,
}

impl MirrorAudioConfig {
    pub fn to_cbor(&self) -> Vec<u8> {
        let mut out = Vec::new();
        ciborium::into_writer(self, &mut out).expect("writing to a Vec cannot fail");
        out
    }

    pub fn from_cbor(bytes: &[u8]) -> Result<MirrorAudioConfig, crate::ProtocolError> {
        ciborium::from_reader(bytes).map_err(|e| crate::ProtocolError::BadMessage(e.to_string()))
    }

    /// A format this version can play: PCM, 8–96 kHz, mono or stereo.
    pub fn is_valid(&self) -> bool {
        self.codec == mirror::PCM && (8_000..=96_000).contains(&self.rate) && (1..=2).contains(&self.channels)
    }

    /// Bytes in one sample frame (a sample for each channel).
    pub fn frame_bytes(&self) -> usize {
        2 * usize::from(self.channels)
    }
}

// ---- Messages (docs/protocol/sms.md) ----

pub mod sms {
    /// Offered by phones that share their text messages.
    pub const READ: &str = "sms.read";
    /// Offered by phones that send texts when a PC asks.
    pub const SEND: &str = "sms.send";
    /// Offered by PCs that show text messages.
    pub const SHOW: &str = "sms.show";
    /// Most conversations or messages in one answer.
    pub const MAX_PAGE: u32 = 100;
    /// A text sent from a PC: at most this many bytes.
    pub const MAX_SEND_BYTES: usize = 8 * 1024;
    /// At most this many recipients.
    pub const MAX_RECIPIENTS: usize = 20;
    /// A picture in a message, fetched with `sms.part`: at most this many bytes.
    pub const MAX_PART_BYTES: usize = 900 * 1024;
    /// A contact's photo with a conversation: a JPEG of at most this many bytes.
    pub const MAX_PHOTO_BYTES: usize = 16 * 1024;
}

/// A conversation, as `sms.threads` lists it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmsThread {
    pub id: String,
    /// The other people's numbers (more than one: a group).
    pub addresses: Vec<String>,
    /// Their contact names, in the same order (empty when not a contact).
    #[serde(default)]
    pub names: Vec<String>,
    /// The latest message's text (or a description, like "Photo").
    #[serde(default)]
    pub snippet: String,
    /// The latest message, in Unix milliseconds.
    pub date: i64,
    /// Unread messages.
    #[serde(default)]
    pub unread: u32,
    /// The contact's photo (one-person conversations), a JPEG.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "serde_bytes")]
    pub photo: Option<Vec<u8>>,
}

/// Never prints numbers, names or text (protocol v0 §11).
impl std::fmt::Debug for SmsThread {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmsThread")
            .field("id", &self.id)
            .field("people", &self.addresses.len())
            .field("unread", &self.unread)
            .finish_non_exhaustive()
    }
}

/// A picture (or other attachment) in a message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmsPart {
    /// Fetched with `sms.part`.
    pub id: String,
    pub mime: String,
    #[serde(default)]
    pub size: u64,
}

/// A message in a conversation.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmsMessage {
    pub id: String,
    pub thread: String,
    /// Who sent it (incoming), or the recipient (sent).
    #[serde(default)]
    pub address: String,
    #[serde(default)]
    pub body: String,
    /// Unix milliseconds.
    pub date: i64,
    /// Sent from this phone (false: received).
    #[serde(default)]
    pub outgoing: bool,
    /// For sent messages: `sent`, `pending` or `failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<SmsPart>,
}

/// Never prints numbers or text (protocol v0 §11).
impl std::fmt::Debug for SmsMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmsMessage")
            .field("id", &self.id)
            .field("thread", &self.thread)
            .field("outgoing", &self.outgoing)
            .field("parts", &self.parts.len())
            .finish_non_exhaustive()
    }
}

/// Body of `sms.threads`: the latest conversations, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmsThreadsGet {
    pub limit: u32,
}

/// Answer to `sms.threads`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmsThreads {
    pub threads: Vec<SmsThread>,
}

/// Body of `sms.messages`: a conversation's messages before `before`
/// (Unix milliseconds; the latest when missing), newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmsMessagesGet {
    pub thread: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<i64>,
    pub limit: u32,
}

/// Answer to `sms.messages`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmsMessages {
    pub messages: Vec<SmsMessage>,
}

/// Body of `sms.send`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmsSend {
    pub to: Vec<String>,
    pub body: String,
}

/// Never prints numbers or text (protocol v0 §11).
impl std::fmt::Debug for SmsSend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmsSend").field("to", &self.to.len()).field("bytes", &self.body.len()).finish()
    }
}

impl SmsSend {
    pub fn is_valid(&self) -> bool {
        (1..=sms::MAX_RECIPIENTS).contains(&self.to.len())
            && self.to.iter().all(|a| (1..=64).contains(&a.trim().len()))
            && !self.body.trim().is_empty()
            && self.body.len() <= sms::MAX_SEND_BYTES
    }
}

/// Body of `sms.part`: send this attachment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmsPartGet {
    pub id: String,
}

/// Answer to `sms.part`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmsPartData {
    pub mime: String,
    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>,
}

impl std::fmt::Debug for SmsPartData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmsPartData").field("mime", &self.mime).field("bytes", &self.data.len()).finish()
    }
}

/// Body of `sms.changed`: messages changed in this conversation (or in
/// any, when missing).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmsChanged {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
}

// ---- Calls (docs/protocol/calls.md) ----

pub mod calls {
    /// Offered by phones that report their calls.
    pub const STATE: &str = "call.state";
    /// Offered by phones that answer and decline calls when a PC asks.
    pub const CONTROL: &str = "call.control";
    /// Offered by PCs that show calls.
    pub const SHOW: &str = "call.show";
    /// Offered by phones that control a call in progress (mute, speaker,
    /// hold, keypad) when a PC asks.
    pub const IN_CALL: &str = "call.incall";
    /// A caller's photo: a JPEG of at most this many bytes.
    pub const MAX_PHOTO_BYTES: usize = 64 * 1024;
    pub const MAX_ID_BYTES: usize = 64;
    pub const MAX_TEXT_BYTES: usize = 256;

    pub const RINGING: &str = "ringing";
    pub const ACTIVE: &str = "active";
    pub const ENDED: &str = "ended";

    pub const ANSWER: &str = "answer";
    pub const DECLINE: &str = "decline";
    pub const SILENCE: &str = "silence";
    pub const MUTE: &str = "mute";
    pub const UNMUTE: &str = "unmute";
    pub const SPEAKER: &str = "speaker";
    pub const EARPIECE: &str = "earpiece";
    pub const HOLD: &str = "hold";
    pub const UNHOLD: &str = "unhold";
    /// A keypad tone: `digit` is one of `0`–`9`, `*`, `#`.
    pub const DTMF: &str = "dtmf";
    pub const VOLUME_UP: &str = "volume_up";
    pub const VOLUME_DOWN: &str = "volume_down";
}

/// A call in progress, as the phone controls it (with `call.incall`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CallControls {
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub speaker: bool,
    #[serde(default)]
    pub held: bool,
    /// The call can be put on hold.
    #[serde(default)]
    pub can_hold: bool,
}

/// Body of `call.state`: a call on the phone started ringing, was
/// answered, or ended.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallState {
    /// The phone's ID for this call, the same in each of its messages.
    pub id: String,
    /// `ringing`, `active` or `ended`.
    pub state: String,
    /// Incoming (false: the phone's user made the call).
    #[serde(default = "yes")]
    pub incoming: bool,
    /// The caller's number, when the phone knows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
    /// The contact's name, when the number is a contact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The contact's photo (JPEG), with `ringing` only.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "serde_bytes")]
    pub photo: Option<Vec<u8>>,
    /// With `ended`: it rang and nobody answered.
    #[serde(default)]
    pub missed: bool,
    /// With `active`: when it was answered, in Unix milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<i64>,
    /// With `active`, when the phone controls the call: its mute, speaker
    /// and hold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controls: Option<CallControls>,
}

fn yes() -> bool {
    true
}

/// Never prints the number, name or photo (protocol v0 §11).
impl std::fmt::Debug for CallState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallState")
            .field("id", &self.id)
            .field("state", &self.state)
            .field("incoming", &self.incoming)
            .field("missed", &self.missed)
            .finish_non_exhaustive()
    }
}

impl CallState {
    pub fn is_valid(&self) -> bool {
        let text_ok = |t: &Option<String>| t.as_ref().is_none_or(|t| t.len() <= calls::MAX_TEXT_BYTES);
        (1..=calls::MAX_ID_BYTES).contains(&self.id.len())
            && [calls::RINGING, calls::ACTIVE, calls::ENDED].contains(&self.state.as_str())
            && text_ok(&self.number)
            && text_ok(&self.name)
            && self.photo.as_ref().is_none_or(|p| p.len() <= calls::MAX_PHOTO_BYTES)
    }
}

/// Body of `call.action`: answer, decline or silence a ringing call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallAction {
    pub id: String,
    pub action: String,
    /// With `dtmf`: the key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digit: Option<String>,
}

/// Whether `digit` is a keypad key (`0`–`9`, `*`, `#`).
pub fn is_dtmf_digit(digit: &str) -> bool {
    digit.len() == 1 && digit.chars().all(|c| c.is_ascii_digit() || c == '*' || c == '#')
}

// ---- Photos (docs/protocol/photos.md) ----

pub mod photos {
    /// Offered by phones that announce new photos (and can read them).
    pub const READ: &str = "photos.read";
    /// Offered by PCs that show announced photos.
    pub const SHOW: &str = "photos.show";
    /// A photo's ID: at most this many bytes.
    pub const MAX_ID_BYTES: usize = 64;
    /// A preview: a JPEG of at most this many bytes.
    pub const MAX_THUMB_BYTES: usize = 96 * 1024;
}

/// Body of `photos.new`: a photo or screenshot just appeared on the phone.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhotoNew {
    /// The phone's ID for it; asks for the photo with `photos.get`.
    pub id: String,
    pub name: String,
    pub size: u64,
    /// When it was taken, in Unix seconds.
    pub taken: i64,
    /// A screenshot rather than a photo.
    #[serde(default)]
    pub screenshot: bool,
    /// A small JPEG preview.
    #[serde(with = "serde_bytes")]
    pub thumb: Vec<u8>,
}

/// Never prints the name or picture (protocol v0 §11).
impl std::fmt::Debug for PhotoNew {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhotoNew")
            .field("id", &self.id)
            .field("size", &self.size)
            .field("screenshot", &self.screenshot)
            .field("thumb", &self.thumb.len())
            .finish_non_exhaustive()
    }
}

impl PhotoNew {
    pub fn is_valid(&self) -> bool {
        is_valid_photo_id(&self.id)
            && is_valid_file_name(&self.name)
            && self.thumb.len() <= photos::MAX_THUMB_BYTES
    }
}

/// Whether a photo ID is 1–64 printable ASCII characters.
pub fn is_valid_photo_id(id: &str) -> bool {
    (1..=photos::MAX_ID_BYTES).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_graphic())
}

/// Body of `photos.get`: send this photo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhotoGet {
    pub id: String,
}

/// Body of `photos.sending`, the answer to `photos.get`: the files
/// transfer that brings it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhotoSending {
    pub transfer: String,
}

/// Body of `link.open`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkOpen {
    pub url: String,
}

/// Never prints the link (protocol v0 §11).
impl std::fmt::Debug for LinkOpen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinkOpen").field("bytes", &self.url.len()).finish()
    }
}

// ---- Files (docs/protocol/files.md) ----

/// The files service and its one operation, for stream headers.
pub mod files {
    pub const SERVICE: &str = "files";
    pub const OP_SEND: &str = "send";
    pub const VERSION: u32 = 1;
    pub const OFFER: &str = "files.offer";
    pub const ACCEPT: &str = "files.accept";
    pub const DONE: &str = "files.done";
    /// Stream reset code for a cancelled transfer.
    pub const CANCELLED: u32 = 10;
    pub const MAX_FILES: usize = 5000;
    pub const MAX_NAME_BYTES: usize = 255;
    /// A file's folder: at most this many bytes and this many levels.
    pub const MAX_FOLDER_BYTES: usize = 1024;
    pub const MAX_FOLDER_DEPTH: usize = 32;
}

/// One file of an offer.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub size: u64,
    /// The folder the file is in when a folder is sent: `/`-separated
    /// names, starting with the sent folder's own (`Trip/Day 1`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
}

/// Never prints the name (protocol v0 §11).
impl std::fmt::Debug for FileEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileEntry").field("size", &self.size).finish_non_exhaustive()
    }
}

/// Body of `files.offer`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilesOffer {
    pub id: String,
    pub files: Vec<FileEntry>,
}

/// Never prints file names (protocol v0 §11).
impl std::fmt::Debug for FilesOffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FilesOffer").field("id", &self.id).field("files", &self.files.len()).finish()
    }
}

/// Body of `files.accept`: bytes already received, per file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilesAccept {
    pub have: Vec<u64>,
}

/// Whether a name is a plain file name the spec allows (no path, no
/// control characters, not `.` or `..`).
pub fn is_valid_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= files::MAX_NAME_BYTES
        && name != "."
        && name != ".."
        && !name.chars().any(|c| c == '/' || c == '\\' || c.is_control())
}

/// Whether a folder is 1–32 valid names joined by `/`, in at most
/// [`files::MAX_FOLDER_BYTES`].
pub fn is_valid_folder(folder: &str) -> bool {
    folder.len() <= files::MAX_FOLDER_BYTES
        && folder.split('/').count() <= files::MAX_FOLDER_DEPTH
        && folder.split('/').all(is_valid_file_name)
}

/// Whether a transfer ID is 16–64 of `[A-Za-z0-9_-]`.
pub fn is_valid_transfer_id(id: &str) -> bool {
    (16..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

impl FilesOffer {
    pub fn is_valid(&self) -> bool {
        is_valid_transfer_id(&self.id)
            && (1..=files::MAX_FILES).contains(&self.files.len())
            && self
                .files
                .iter()
                .all(|f| is_valid_file_name(&f.name) && f.folder.as_deref().is_none_or(is_valid_folder))
    }

    pub fn total_size(&self) -> u64 {
        self.files.iter().map(|f| f.size).fold(0, u64::saturating_add)
    }
}

// ---- Clipboard (docs/protocol/clipboard.md) ----

/// Largest clipboard text sent or accepted, in UTF-8 bytes.
pub const CLIP_MAX_BYTES: usize = 512 * 1024;

/// Body of `clip.set`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipSet {
    pub text: String,
}

impl ClipSet {
    /// Whether the text is within the spec's limits.
    pub fn is_valid(&self) -> bool {
        !self.text.is_empty() && self.text.len() <= CLIP_MAX_BYTES
    }
}

/// Never prints content (protocol v0 §11).
impl std::fmt::Debug for ClipSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClipSet").field("bytes", &self.text.len()).finish()
    }
}

/// Clipboard images, which travel on their own stream.
pub mod clip {
    pub const SERVICE: &str = "clip";
    pub const OP_IMAGE: &str = "image";
    pub const VERSION: u32 = 1;
    pub const IMAGE: &str = "clip.image";
    /// The largest image sent or accepted.
    pub const MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;
    /// Image types a receiver accepts.
    pub const IMAGE_TYPES: &[&str] = &["image/png", "image/jpeg"];
}

/// Body of `clip.image`: what follows on the stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipImage {
    /// `image/png` or `image/jpeg`.
    pub mime: String,
    pub size: u64,
}

impl ClipImage {
    /// Whether the type and size are within the spec's limits.
    pub fn is_valid(&self) -> bool {
        clip::IMAGE_TYPES.contains(&self.mime.as_str()) && (1..=clip::MAX_IMAGE_BYTES).contains(&self.size)
    }
}

/// A paired device identity as stored in trust stores and exchanged in tests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerIdentity {
    pub id: DeviceId,
    pub device: DeviceInfo,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Envelope;

    fn info() -> DeviceInfo {
        DeviceInfo {
            name: "Pixel 9".into(),
            kind: DeviceKind::Phone,
            os: "android".into(),
            os_ver: "16".into(),
            model: Some("Google Pixel 9".into()),
            accent: None,
        }
    }

    #[test]
    fn hello_round_trips() {
        let hello = Hello {
            proto: 0,
            min: 0,
            app: "0.0.1".into(),
            device: info(),
            caps: vec!["core.ping".into(), "device.battery".into()],
            power: PowerLevel::Elevated,
        };
        let env = Envelope::new(types::HELLO, &hello).unwrap();
        let back: Hello = Envelope::from_cbor(&env.to_cbor()).unwrap().expect_body(types::HELLO).unwrap();
        assert_eq!(back, hello);
    }

    #[test]
    fn unknown_enum_values_are_tolerated() {
        #[derive(Serialize)]
        struct Raw<'a> {
            kind: &'a str,
            power: &'a str,
        }
        #[derive(Deserialize)]
        struct Parsed {
            kind: DeviceKind,
            power: PowerLevel,
        }
        let env = Envelope::new("x", &Raw { kind: "watch", power: "root" }).unwrap();
        let parsed: Parsed = env.body().unwrap();
        assert_eq!(parsed.kind, DeviceKind::Unknown);
        assert_eq!(parsed.power.effective(), PowerLevel::Basic);
    }

    fn notification() -> Notification {
        Notification {
            key: "0|com.example|1|null|10123".into(),
            app: "com.example".into(),
            app_name: "Example".into(),
            title: Some("Sam".into()),
            text: Some("Lunch?".into()),
            sub: None,
            when: 1_760_000_000_000,
            actions: vec![NotificationAction { id: "0".into(), title: "Reply".into(), reply: true }],
            silent: false,
            icon: Some(vec![0x89, b'P', b'N', b'G']),
            image: None,
        }
    }

    #[test]
    fn notifications_round_trip_with_binary_icons() {
        let n = notification();
        let env = Envelope::new(types::NOTIFY_POSTED, &n).unwrap();
        // The icon travels as a CBOR byte string, not an array of numbers.
        let icon = match &env.b {
            Some(ciborium::Value::Map(fields)) => fields.iter().find(|(k, _)| k.as_text() == Some("icon")),
            _ => None,
        };
        assert!(matches!(icon, Some((_, ciborium::Value::Bytes(_)))));
        let back: Notification = Envelope::from_cbor(&env.to_cbor()).unwrap().body().unwrap();
        assert_eq!(back, n);
    }

    #[test]
    fn sanitizing_enforces_the_limits() {
        let mut n = notification();
        n.title = Some("é".repeat(300));
        n.text = Some("  ".into());
        n.actions = (0..8)
            .map(|i| NotificationAction { id: i.to_string(), title: "Go".into(), reply: false })
            .collect();
        n.actions.push(NotificationAction { id: String::new(), title: "No id".into(), reply: false });
        n.icon = Some(vec![0; notify_limits::ICON_BYTES + 1]);
        n.image = Some(vec![0; notify_limits::IMAGE_BYTES + 1]);
        let n = n.sanitized().unwrap();
        assert_eq!(n.title.unwrap().chars().count(), notify_limits::TITLE_CHARS);
        assert_eq!(n.text, None, "blank text is dropped");
        assert_eq!(n.actions.len(), notify_limits::ACTIONS);
        assert_eq!(n.icon, None);
        assert_eq!(n.image, None);
    }

    #[test]
    fn notifications_without_content_or_key_are_dropped() {
        let mut n = notification();
        n.title = None;
        n.text = Some(" ".into());
        assert_eq!(n.sanitized(), None);
        let mut n = notification();
        n.key = String::new();
        assert_eq!(n.sanitized(), None);
    }

    #[test]
    fn debug_output_hides_content() {
        let shown = format!("{:?}", notification());
        assert!(!shown.contains("Lunch") && !shown.contains("Sam"), "{shown}");
        let action = NotifyAction { key: "k".into(), action: "0".into(), reply: Some("secret".into()) };
        assert!(!format!("{action:?}").contains("secret"));
    }

    #[test]
    fn file_offers_are_validated() {
        let offer = |id: &str, names: &[&str]| FilesOffer {
            id: id.into(),
            files: names.iter().map(|n| FileEntry { name: (*n).into(), size: 1, folder: None }).collect(),
        };
        assert!(offer("abcdefghijklmnop", &["photo.jpg", "Résumé (final).pdf"]).is_valid());
        for bad in ["", ".", "..", "a/b", "a\\b", "x\u{0}y", "tab\there"] {
            assert!(!offer("abcdefghijklmnop", &[bad]).is_valid(), "{bad:?}");
        }
        assert!(!offer("short", &["a"]).is_valid(), "IDs are 16+ characters");
        assert!(!offer("abcdefghijklmnop!", &["a"]).is_valid());
        assert!(!offer("abcdefghijklmnop", &[]).is_valid(), "at least one file");
        assert!(!format!("{:?}", offer("abcdefghijklmnop", &["secret.pdf"])).contains("secret"));
    }

    #[test]
    fn folders_are_validated() {
        let offer = |folder: &str| FilesOffer {
            id: "abcdefghijklmnop".into(),
            files: vec![FileEntry { name: "a.jpg".into(), size: 1, folder: Some(folder.into()) }],
        };
        for good in ["Trip", "Trip/Day 1", "Trip/Day 1/raw"] {
            assert!(offer(good).is_valid(), "{good:?}");
        }
        for bad in ["", "/Trip", "Trip/", "Trip//x", "Trip/..", "./x", "a\\b", "x\u{0}"] {
            assert!(!offer(bad).is_valid(), "{bad:?}");
        }
        assert!(!offer(&vec!["d"; files::MAX_FOLDER_DEPTH + 1].join("/")).is_valid());
        // Without a folder, the field isn't sent at all.
        let plain = FileEntry { name: "a".into(), size: 1, folder: None };
        let env = Envelope::new("x", &plain).unwrap();
        assert!(!format!("{:?}", env.b).contains("folder"));
        assert!(!format!("{:?}", offer("Secret")).contains("Secret"));
    }

    #[test]
    fn mirror_input_is_checked() {
        let touch = MirrorInput::Touch { action: TouchAction::Down, x: 0.5, y: 1.0 };
        assert!(touch.is_valid());
        let env = Envelope::new(types::MIRROR_INPUT, &touch).unwrap();
        assert_eq!(env.body::<MirrorInput>().unwrap(), touch);
        assert!(!MirrorInput::Touch { action: TouchAction::Up, x: 1.5, y: 0.0 }.is_valid());
        assert!(!MirrorInput::Touch { action: TouchAction::Up, x: f32::NAN, y: 0.0 }.is_valid());
        assert!(MirrorInput::Key { key: "back".into() }.is_valid());
        assert!(!MirrorInput::Key { key: "power".into() }.is_valid());
        assert!(!MirrorInput::Text { text: String::new() }.is_valid());
        assert!(!format!("{:?}", MirrorInput::Text { text: "secret".into() }).contains("secret"));
    }

    #[test]
    fn texts_to_send_are_checked() {
        let send = SmsSend { to: vec!["+15550100".into()], body: "On my way".into() };
        assert!(send.is_valid());
        assert!(!format!("{send:?}").contains("way"));
        assert!(!SmsSend { to: vec![], ..send.clone() }.is_valid());
        assert!(!SmsSend { body: "  ".into(), ..send.clone() }.is_valid());
        assert!(!SmsSend { body: "x".repeat(sms::MAX_SEND_BYTES + 1), ..send.clone() }.is_valid());
        assert!(!SmsSend { to: vec!["1".into(); sms::MAX_RECIPIENTS + 1], ..send }.is_valid());
    }

    #[test]
    fn calls_are_validated() {
        let call = CallState {
            id: "1".into(),
            state: calls::RINGING.into(),
            incoming: true,
            number: Some("+1 555 0100".into()),
            name: Some("Sam".into()),
            photo: Some(vec![0xff, 0xd8]),
            missed: false,
            since: None,
            controls: None,
        };
        assert!(call.is_valid());
        let shown = format!("{call:?}");
        assert!(!shown.contains("555") && !shown.contains("Sam"), "{shown}");
        assert!(!CallState { state: "on hold".into(), ..call.clone() }.is_valid());
        assert!(!CallState { id: String::new(), ..call.clone() }.is_valid());
        assert!(!CallState { photo: Some(vec![0; calls::MAX_PHOTO_BYTES + 1]), ..call.clone() }.is_valid());
        // Missing fields take their defaults: incoming, not missed.
        let env = Envelope::new("x", &CallState { number: None, name: None, photo: None, ..call }).unwrap();
        let back: CallState = env.body().unwrap();
        assert!(back.incoming && !back.missed && back.number.is_none());
    }

    #[test]
    fn photos_are_validated() {
        let photo = PhotoNew {
            id: "content:42".into(),
            name: "Screenshot_20261006.png".into(),
            size: 1000,
            taken: 1_790_000_000,
            screenshot: true,
            thumb: vec![0xff, 0xd8],
        };
        assert!(photo.is_valid());
        assert!(!format!("{photo:?}").contains("Screenshot_"));
        assert!(!PhotoNew { id: String::new(), ..photo.clone() }.is_valid());
        assert!(!PhotoNew { id: "a b".into(), ..photo.clone() }.is_valid());
        assert!(!PhotoNew { name: "../x.png".into(), ..photo.clone() }.is_valid());
        assert!(!PhotoNew { thumb: vec![0; photos::MAX_THUMB_BYTES + 1], ..photo }.is_valid());
    }

    #[test]
    fn power_level_wire_names() {
        let env = Envelope::new("x", &PowerLevel::NotApplicable).unwrap();
        assert_eq!(env.b, Some(ciborium::Value::Text("n/a".into())));
    }
}
