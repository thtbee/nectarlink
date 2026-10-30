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
    pub const MEDIA_WATCH: &str = "media.watch";
    pub const MEDIA_FILE: &str = "media.file";
    pub const MEDIA_FILE_META: &str = "media.file.meta";
    pub const PC_POWER: &str = "pc.power";
    pub const PC_WAKE_INFO: &str = "pc.wake_info";
    pub const LINK_OPEN: &str = "link.open";
    pub const TASK_NOTIFY: &str = "task.notify";
    pub const PHOTOS_NEW: &str = "photos.new";
    pub const PHOTOS_ALBUMS: &str = "photos.albums";
    pub const PHOTOS_ALBUMS_LIST: &str = "photos.albums.list";
    pub const PHOTOS_LIST: &str = "photos.list";
    pub const PHOTOS_ITEMS: &str = "photos.items";
    pub const PHOTOS_THUMBS: &str = "photos.thumbs";
    pub const PHOTOS_THUMBS_LIST: &str = "photos.thumbs.list";
    pub const PHOTOS_GET: &str = "photos.get";
    pub const PHOTOS_SENDING: &str = "photos.sending";
    pub const PHOTOS_CHANGED: &str = "photos.changed";
    pub const CALL_STATE: &str = "call.state";
    pub const CALL_ACTION: &str = "call.action";
    pub const CALL_LOG: &str = "call.log";
    pub const CALL_LOG_CHANGED: &str = "call.log.changed";
    pub const CALL_DIAL: &str = "call.dial";
    pub const CONTACTS_LIST: &str = "contacts.list";
    pub const CONTACTS_CHANGED: &str = "contacts.changed";
    pub const SMS_THREADS: &str = "sms.threads";
    pub const SMS_MESSAGES: &str = "sms.messages";
    pub const SMS_SEND: &str = "sms.send";
    pub const SMS_PART: &str = "sms.part";
    pub const SMS_CHANGED: &str = "sms.changed";
    pub const MIRROR_START: &str = "mirror.start";
    pub const MIRROR_STOP: &str = "mirror.stop";
    pub const MIRROR_POWER: &str = "mirror.power";
    pub const MIRROR_KEYFRAME: &str = "mirror.keyframe";
    pub const MIRROR_RESIZE: &str = "mirror.resize";
    pub const MIRROR_INPUT: &str = "mirror.input";
    pub const MIRROR_APPS: &str = "mirror.apps";
    pub const REMOTE_CHECK: &str = "remote.check";
    pub const REMOTE_INPUT: &str = "remote.input";
    pub const PHONE_TOGGLES: &str = "phone.toggles";
    pub const PHONE_TOGGLE_SET: &str = "phone.toggle.set";
    pub const DECK_LAYOUT: &str = "deck.layout";
    pub const DECK_STATE: &str = "deck.state";
    pub const DECK_PRESS: &str = "deck.press";
    pub const PC_AUDIO_SET: &str = "pc.audio.set";
    pub const STORAGE_LIST: &str = "storage.list";
    pub const STORAGE_ENTRIES: &str = "storage.entries";
    pub const STORAGE_READ: &str = "storage.read";
    pub const STORAGE_READ_META: &str = "storage.read.meta";
    pub const STORAGE_WRITE: &str = "storage.write";
    pub const STORAGE_WRITE_ACCEPT: &str = "storage.write.accept";
    pub const STORAGE_WRITE_DONE: &str = "storage.write.done";
    pub const STORAGE_MKDIR: &str = "storage.mkdir";
    pub const STORAGE_RENAME: &str = "storage.rename";
    pub const STORAGE_DELETE: &str = "storage.delete";
    pub const STORAGE_CHANGED: &str = "storage.changed";
    pub const ELEVATED_USB_START: &str = "elevated.usb.start";
    pub const ELEVATED_USB_READY: &str = "elevated.usb.ready";
    pub const MIC_START: &str = "mic.start";
    pub const MIC_STOP: &str = "mic.stop";
    pub const WEBCAM_START: &str = "webcam.start";
    pub const WEBCAM_STOP: &str = "webcam.stop";
    pub const WEBCAM_KEYFRAME: &str = "webcam.keyframe";
    pub const WEBCAM_OK: &str = "webcam.ok";
    pub const CAMERA_CAPTURE_REQUEST: &str = "camera.capture.request";
    pub const CAMERA_CAPTURE_OK: &str = "camera.capture.ok";
    pub const CAMERA_CAPTURE_CANCEL: &str = "camera.capture.cancel";
    pub const FILES_RESUME: &str = "files.resume";
    pub const FILES_CANCEL: &str = "files.cancel";
    pub const WAKE_RELAY: &str = "wake.relay";
    pub const AUDIO_PCM: &str = "audio.pcm";
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

/// Limits for measured display geometry (`DeviceInfo.screen`, protocol §5.1).
pub mod screen_limits {
    pub const VERSION: u32 = 1;
    pub const MIN_ASPECT: f32 = 0.2;
    pub const MAX_ASPECT: f32 = 5.0;
    pub const MAX_CORNER_RADIUS: f32 = 0.5;
    pub const MAX_CUTOUTS: usize = 8;
    pub const MAX_CUTOUT_PATH_BYTES: usize = 4096;
}

fn approx_eq_f32(a: f32, b: f32) -> bool {
    a.to_bits() == b.to_bits() || (a - b).abs() <= 1e-5
}

fn round_norm(v: f32) -> f32 {
    (v * 10000.0).round() / 10000.0
}

/// Normalized `[0.0, 1.0]` bounding rectangle on the upright portrait display.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ScreenRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl PartialEq for ScreenRect {
    fn eq(&self, other: &Self) -> bool {
        approx_eq_f32(self.x, other.x)
            && approx_eq_f32(self.y, other.y)
            && approx_eq_f32(self.w, other.w)
            && approx_eq_f32(self.h, other.h)
    }
}
impl Eq for ScreenRect {}

impl ScreenRect {
    pub fn sanitized(self) -> Option<Self> {
        if !self.x.is_finite()
            || !self.y.is_finite()
            || !self.w.is_finite()
            || !self.h.is_finite()
            || self.w <= 0.0
            || self.h <= 0.0
        {
            return None;
        }
        let x = round_norm(self.x.clamp(0.0, 1.0));
        let y = round_norm(self.y.clamp(0.0, 1.0));
        let w = round_norm(self.w.clamp(0.0, (1.0 - x).max(0.0)));
        let h = round_norm(self.h.clamp(0.0, (1.0 - y).max(0.0)));
        (w > 0.0 && h > 0.0).then_some(Self { x, y, w, h })
    }
}

/// Normalized rounded-corner radii `[tl, tr, br, bl]` as fractions of upright screen width (`0.0..=0.5`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ScreenCorners {
    pub tl: f32,
    pub tr: f32,
    pub br: f32,
    pub bl: f32,
}

impl PartialEq for ScreenCorners {
    fn eq(&self, other: &Self) -> bool {
        approx_eq_f32(self.tl, other.tl)
            && approx_eq_f32(self.tr, other.tr)
            && approx_eq_f32(self.br, other.br)
            && approx_eq_f32(self.bl, other.bl)
    }
}
impl Eq for ScreenCorners {}

impl ScreenCorners {
    pub fn sanitized(self) -> Option<Self> {
        if !self.tl.is_finite() || !self.tr.is_finite() || !self.br.is_finite() || !self.bl.is_finite() {
            return None;
        }
        Some(Self {
            tl: round_norm(self.tl.clamp(0.0, screen_limits::MAX_CORNER_RADIUS)),
            tr: round_norm(self.tr.clamp(0.0, screen_limits::MAX_CORNER_RADIUS)),
            br: round_norm(self.br.clamp(0.0, screen_limits::MAX_CORNER_RADIUS)),
            bl: round_norm(self.bl.clamp(0.0, screen_limits::MAX_CORNER_RADIUS)),
        })
    }
}

/// Measured front-display geometry of a phone or tablet (`DeviceInfo.screen`, protocol §5.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenShape {
    /// Schema version (currently `1`).
    pub v: u32,
    /// Upright display aspect ratio (`width / height`, e.g. `0.45` for 9:20).
    pub aspect: f32,
    /// Measured corner radii as fractions of screen width (`Display.getRoundedCorner()` on API 31+).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corners: Option<ScreenCorners>,
    /// Measured cutout bounding boxes (`DisplayCutout.getBoundingRects()`, normalized `0.0..=1.0`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cutouts: Vec<ScreenRect>,
    /// Measured cutout vector path (`DisplayCutout.getCutoutPath()` on API 31+, normalized SVG path in `0.0..=1.0`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cutout_path: Option<String>,
}

impl PartialEq for ScreenShape {
    fn eq(&self, other: &Self) -> bool {
        self.v == other.v
            && approx_eq_f32(self.aspect, other.aspect)
            && self.corners == other.corners
            && self.cutouts == other.cutouts
            && self.cutout_path == other.cutout_path
    }
}
impl Eq for ScreenShape {}

impl ScreenShape {
    pub fn sanitized(self) -> Option<Self> {
        if self.v != screen_limits::VERSION
            || !self.aspect.is_finite()
            || !(screen_limits::MIN_ASPECT..=screen_limits::MAX_ASPECT).contains(&self.aspect)
        {
            return None;
        }
        let cutout_path = self.cutout_path.and_then(|s| {
            let trimmed = s.trim();
            if trimmed.is_empty() || trimmed.len() > screen_limits::MAX_CUTOUT_PATH_BYTES {
                return None;
            }
            let valid_chars = trimmed.chars().all(|c| {
                c.is_ascii_digit()
                    || c.is_ascii_whitespace()
                    || matches!(
                        c,
                        'M' | 'L'
                            | 'H'
                            | 'V'
                            | 'C'
                            | 'S'
                            | 'Q'
                            | 'T'
                            | 'A'
                            | 'Z'
                            | 'm'
                            | 'l'
                            | 'h'
                            | 'v'
                            | 'c'
                            | 's'
                            | 'q'
                            | 't'
                            | 'a'
                            | 'z'
                            | '.'
                            | '-'
                            | '+'
                            | ','
                    )
            });
            valid_chars.then(|| trimmed.to_owned())
        });
        let cutouts = self
            .cutouts
            .into_iter()
            .filter_map(ScreenRect::sanitized)
            .take(screen_limits::MAX_CUTOUTS)
            .collect();
        Some(Self {
            v: screen_limits::VERSION,
            aspect: round_norm(self.aspect),
            corners: self.corners.and_then(ScreenCorners::sanitized),
            cutouts,
            cutout_path,
        })
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
    /// Optional measured front-display geometry for phones/tablets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen: Option<ScreenShape>,
    /// Identifies the physical device across app reinstalls (a new identity key each
    /// time), so a re-paired device replaces its old entry instead of appearing twice.
    /// A salted hash, never the raw platform ID; see `install_id_from`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_id: Option<String>,
}

/// A device's name or model, at most, in characters.
pub const MAX_DEVICE_NAME_CHARS: usize = 64;
/// A device's OS name or version, at most, in characters.
pub const MAX_DEVICE_OS_CHARS: usize = 32;

/// `text` without control characters or the invisible marks that reorder
/// text (a name could otherwise show reversed or disguised), cut to
/// `max_chars` and trimmed.
fn clean_device_text(text: &str, max_chars: usize) -> String {
    let is_direction_mark = |c: char| matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}');
    let clean: String =
        text.chars().filter(|c| !c.is_control() && !is_direction_mark(*c)).take(max_chars).collect();
    clean.trim().to_owned()
}

impl DeviceInfo {
    /// What another device says about itself, made safe to store and show.
    pub fn sanitized(mut self) -> Self {
        self.name = clean_device_text(&self.name, MAX_DEVICE_NAME_CHARS);
        self.os = clean_device_text(&self.os, MAX_DEVICE_OS_CHARS);
        self.os_ver = clean_device_text(&self.os_ver, MAX_DEVICE_OS_CHARS);
        self.model =
            self.model.map(|m| clean_device_text(&m, MAX_DEVICE_NAME_CHARS)).filter(|m| !m.is_empty());
        self.screen = self.screen.and_then(ScreenShape::sanitized);
        self.install_id = self.install_id.filter(|id| is_valid_install_id(id));
        self
    }
}

/// Length of an install ID in hex characters (128 bits).
pub const INSTALL_ID_LEN: usize = 32;

/// Whether `id` is a well-formed install ID: lowercase hex, [`INSTALL_ID_LEN`] long.
pub fn is_valid_install_id(id: &str) -> bool {
    id.len() == INSTALL_ID_LEN && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
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
    /// Estimated minutes until full when charging (`BatteryManager.computeChargeTimeRemaining()`), if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_in: Option<u16>,
    /// Cellular signal strength in bars (`0..=4`), if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell_bars: Option<u8>,
    /// Cellular network generation/type (`"5G"`, `"LTE"`, `"4G"`, `"3G"`, `"2G"`), if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell_type: Option<String>,
    /// Wi-Fi signal strength in bars (`0..=4`), if connected to Wi-Fi and known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wifi_bars: Option<u8>,
}

impl Battery {
    pub fn sanitized(mut self) -> Self {
        self.level = self.level.min(100);
        if !self.charging {
            self.full_in = None;
        } else if let Some(mins) = self.full_in
            && (mins == 0 || mins > 24 * 60)
        {
            self.full_in = None;
        }
        self.cell_bars = self.cell_bars.map(|b| b.min(4));
        self.cell_type =
            clean_text(self.cell_type.map(|s| s.chars().filter(|c| !c.is_control()).collect()), 8);
        self.wifi_bars = self.wifi_bars.map(|b| b.min(4));
        self
    }
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
    pub const LIVE_VERSION: u32 = 1;
    pub const CHIP_CHARS: usize = 32;
    pub const SEGMENTS: usize = 16;
    pub const POINTS: usize = 16;
    pub const CONVERSATION_VERSION: u32 = 1;
    pub const CONVERSATION_MESSAGES: usize = 25;
    pub const SENDER_CHARS: usize = 128;
    pub const AVATAR_BYTES: usize = 16 * 1024;
}

fn default_live_v() -> u32 {
    notify_limits::LIVE_VERSION
}

fn default_conversation_v() -> u32 {
    notify_limits::CONVERSATION_VERSION
}

/// One segment of a segmented progress bar (`Notification.ProgressStyle.Segment` on Android 16+).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveSegment {
    /// Relative length of this segment (`> 0`).
    pub length: u32,
    /// Optional RGB/ARGB color; `None` when using the default theme color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<u32>,
}

/// One milestone point along a progress bar (`Notification.ProgressStyle.Point` on Android 16+).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LivePoint {
    /// Position along `0..=max`.
    pub position: u32,
    /// Optional RGB/ARGB color; `None` when using the default theme color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<u32>,
}

/// Live Update metadata on a mirrored notification (`docs/protocol/notifications.md` §2.2).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationLive {
    /// Schema version (currently `1`).
    #[serde(default = "default_live_v")]
    pub v: u32,
    /// Current progress (`0..=max`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<u32>,
    /// Maximum progress (`> 0` when determinate progress is present).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<u32>,
    /// True when progress is indeterminate.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub indeterminate: bool,
    /// Short critical status pill text (`Notification.getShortCriticalText()`, e.g. `"5 min"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chip: Option<String>,
    /// Optional segments of an Android 16 `ProgressStyle` bar.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub segments: Vec<LiveSegment>,
    /// Optional milestone points along an Android 16 `ProgressStyle` bar.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<LivePoint>,
    /// True when the notification shows a live elapsed/countdown timer (`EXTRA_SHOW_CHRONOMETER`)
    /// relative to `Notification.when` (Unix milliseconds).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub chronometer: bool,
    /// True when the chronometer counts down toward `Notification.when` (`EXTRA_CHRONOMETER_COUNT_DOWN`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub countdown: bool,
}

impl Default for NotificationLive {
    fn default() -> Self {
        Self {
            v: notify_limits::LIVE_VERSION,
            progress: None,
            max: None,
            indeterminate: false,
            chip: None,
            segments: Vec::new(),
            points: Vec::new(),
            chronometer: false,
            countdown: false,
        }
    }
}

/// Never prints `chip` text in logs (protocol v0 §11).
impl std::fmt::Debug for NotificationLive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotificationLive")
            .field("v", &self.v)
            .field("has_chip", &self.chip.is_some())
            .field("progress", &self.progress)
            .field("max", &self.max)
            .field("indeterminate", &self.indeterminate)
            .field("segments", &self.segments.len())
            .field("points", &self.points.len())
            .field("chronometer", &self.chronometer)
            .field("countdown", &self.countdown)
            .finish()
    }
}

impl NotificationLive {
    /// Applies limits and drops unknown schema versions (`v != 1`) or empty metadata.
    pub fn sanitized(mut self) -> Option<Self> {
        use notify_limits::*;
        if self.v != LIVE_VERSION {
            return None;
        }
        self.chip =
            clean_text(self.chip.map(|s| s.chars().filter(|c| !c.is_control()).collect()), CHIP_CHARS);
        self.segments.retain(|s| s.length > 0);
        self.segments.truncate(SEGMENTS);
        self.points.truncate(POINTS);
        if self.indeterminate {
            self.progress = None;
            self.max = None;
            self.segments.clear();
            self.points.clear();
        } else {
            if !self.segments.is_empty() && self.max.unwrap_or(0) == 0 {
                let sum = self.segments.iter().map(|s| s.length).fold(0u32, u32::saturating_add);
                if sum > 0 {
                    self.max = Some(sum);
                }
            }
            if self.max == Some(0) {
                self.max = None;
                self.progress = None;
            }
            if let Some(max) = self.max {
                self.progress = Some(self.progress.unwrap_or(0).min(max));
                self.points.retain(|p| p.position <= max);
            } else {
                self.progress = None;
                self.points.clear();
            }
        }
        if !self.chronometer {
            self.countdown = false;
        }
        if self.progress.is_none()
            && self.max.is_none()
            && !self.indeterminate
            && self.chip.is_none()
            && self.segments.is_empty()
            && self.points.is_empty()
            && !self.chronometer
        {
            return None;
        }
        Some(self)
    }
}

/// One message in a `Notification.MessagingStyle` conversation (`docs/protocol/notifications.md` §2.3).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationChatMessage {
    /// Sender display name (`None` when sent by the phone's user).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<String>,
    /// Message text.
    pub text: String,
    /// Message timestamp, Unix milliseconds.
    pub time: i64,
    /// True when sent by the phone's user (`MessagingStyle.user` or `Person == null`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub self_sent: bool,
    /// Optional sender avatar (JPEG/PNG, at most [`notify_limits::AVATAR_BYTES`]).
    #[serde(default, skip_serializing_if = "Option::is_none", with = "serde_bytes")]
    pub avatar: Option<Vec<u8>>,
}

/// Never prints sender names or message text (protocol v0 §11).
impl std::fmt::Debug for NotificationChatMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotificationChatMessage")
            .field("time", &self.time)
            .field("self_sent", &self.self_sent)
            .field("has_avatar", &self.avatar.is_some())
            .finish_non_exhaustive()
    }
}

/// `Notification.MessagingStyle` conversation metadata on a mirrored notification (`docs/protocol/notifications.md` §2.3).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationConversation {
    /// Schema version (currently `1`).
    #[serde(default = "default_conversation_v")]
    pub v: u32,
    /// Conversation title (group name or 1:1 contact name).
    pub title: String,
    /// True when `MessagingStyle.isGroupConversation` is true.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub group: bool,
    /// Conversation or contact avatar (JPEG/PNG, at most [`notify_limits::AVATAR_BYTES`]).
    #[serde(default, skip_serializing_if = "Option::is_none", with = "serde_bytes")]
    pub avatar: Option<Vec<u8>>,
    /// Messages in chronological order (oldest first, at most [`notify_limits::CONVERSATION_MESSAGES`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<NotificationChatMessage>,
}

/// Never prints conversation title or messages (protocol v0 §11).
impl std::fmt::Debug for NotificationConversation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotificationConversation")
            .field("v", &self.v)
            .field("group", &self.group)
            .field("messages", &self.messages.len())
            .field("has_avatar", &self.avatar.is_some())
            .finish_non_exhaustive()
    }
}

impl NotificationConversation {
    /// Applies limits and drops unknown schema versions (`v != 1`) or empty conversations.
    pub fn sanitized(mut self) -> Option<Self> {
        use notify_limits::*;
        if self.v != CONVERSATION_VERSION {
            return None;
        }
        self.title = clean_text(Some(self.title), TITLE_CHARS)?;
        if self.avatar.as_ref().is_some_and(|a| a.is_empty() || a.len() > AVATAR_BYTES) {
            self.avatar = None;
        }
        let mut cleaned = Vec::with_capacity(self.messages.len().min(CONVERSATION_MESSAGES));
        let skip = self.messages.len().saturating_sub(CONVERSATION_MESSAGES);
        for mut m in self.messages.into_iter().skip(skip) {
            let Some(text) = clean_text(Some(m.text), TEXT_CHARS) else {
                continue;
            };
            m.text = text;
            m.sender = if m.self_sent { None } else { clean_text(m.sender, SENDER_CHARS) };
            if m.avatar.as_ref().is_some_and(|a| a.is_empty() || a.len() > AVATAR_BYTES) {
                m.avatar = None;
            }
            cleaned.push(m);
        }
        if cleaned.is_empty() {
            return None;
        }
        self.messages = cleaned;
        Some(self)
    }
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
    /// Live Update metadata (ongoing progress, timer, or Android 16 Live Update).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live: Option<NotificationLive>,
    /// `Notification.MessagingStyle` conversation metadata (RCS, WhatsApp, Telegram, Signal, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<NotificationConversation>,
}

/// Never prints content: notification text must not reach logs (protocol
/// v0 §11).
impl std::fmt::Debug for Notification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Notification")
            .field("app", &self.app)
            .field("actions", &self.actions.len())
            .field("silent", &self.silent)
            .field("live", &self.live.is_some())
            .field("conversation", &self.conversation.is_some())
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
        self.live = self.live.and_then(NotificationLive::sanitized);
        self.conversation = self.conversation.and_then(NotificationConversation::sanitized);
        if self.title.is_none() && self.text.is_none() && self.conversation.is_none() {
            return None;
        }
        if self.title.is_none()
            && let Some(conv) = &self.conversation
        {
            self.title = Some(conv.title.clone());
        }
        if self.text.is_none()
            && let Some(conv) = &self.conversation
            && let Some(last) = conv.messages.last()
        {
            self.text = Some(last.text.clone());
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

/// Play on phone (`docs/protocol/media.md` §4).
pub mod media_watch {
    /// Offered by phones that show a PC's playback streamed to them.
    pub const WATCH: &str = "media.watch";
    /// Offered by PCs that stream a player's picture and sound on request.
    pub const STREAM: &str = "media.stream";
    /// Stream service and operation for reading an offered file.
    pub const SERVICE: &str = "media";
    pub const OP_FILE: &str = "file";
    pub const VERSION: u32 = 1;
    /// The most one `media.file` read returns.
    pub const MAX_READ: u64 = 4 * 1024 * 1024;
}

/// A video file a PC player plays, offered with `media.watch` for the phone
/// to play itself (`docs/protocol/media.md` §4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchFile {
    /// Its file name, for the phone's player to tell the format by.
    pub name: String,
    /// Its size in bytes.
    pub size: u64,
    /// Where the PC had got to, milliseconds.
    #[serde(default)]
    pub position: u64,
}

impl WatchFile {
    pub fn is_valid(&self) -> bool {
        (1..=media_limits::TEXT_CHARS).contains(&self.name.chars().count())
            && !self.name.contains(['/', '\\'])
            && self.size > 0
    }
}

/// Body of `media.file` on a `media/file` stream: read `length` bytes of
/// the offered file from `offset`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaFileRead {
    pub offset: u64,
    pub length: u64,
}

/// Body of `media.file.meta`: the file's size and how many bytes follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaFileMeta {
    pub size: u64,
    pub length: u64,
}

/// Body of `media.watch`: a PC offers to stream what a player of its own
/// plays to the phone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaWatch {
    /// The PC's ID for the player, as in `media.state`.
    pub player: String,
    /// What it plays, to show with the offer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// A video file the phone plays itself, read from the PC, instead of
    /// watching the PC's stream.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<WatchFile>,
}

impl MediaWatch {
    pub fn is_valid(&self) -> bool {
        (1..=media_limits::ID_BYTES).contains(&self.player.len())
            && self.title.as_ref().is_none_or(|t| t.chars().count() <= media_limits::TEXT_CHARS)
            && self.file.as_ref().is_none_or(WatchFile::is_valid)
    }
}

// ---- Actions (docs/protocol/actions.md) ----

/// Body of `pc.power`: `lock` or `sleep`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PcPower {
    pub action: String,
}

/// The longest link sent, in bytes.
pub const LINK_MAX_BYTES: usize = 4096;

/// Constants and limits for Wake-on-LAN (`docs/protocol/actions.md` §2.3).
pub mod wake {
    /// Offered by PCs that share their adapter addresses (`pc.wake_info`).
    pub const CAP: &str = "pc.wake";
    /// Maximum hardware addresses in a `pc.wake_info` message.
    pub const MAX_ADAPTERS: usize = 8;
    /// Maximum directed subnet broadcast addresses in a `pc.wake_info` message.
    pub const MAX_BROADCASTS: usize = 16;
    /// Size of a standard Wake-on-LAN magic packet (`6 + 16 * 6`).
    pub const MAGIC_PACKET_BYTES: usize = 102;
    /// Standard UDP ports for Wake-on-LAN magic packets.
    pub const DEFAULT_PORTS: &[u16] = &[9, 7];
}

/// Parses a 6-byte hardware (MAC) address written as `"aa:bb:cc:dd:ee:ff"` or
/// `"AA-BB-CC-DD-EE-FF"`. Rejects all-zero (`00:00:00:00:00:00`) and broadcast
/// (`ff:ff:ff:ff:ff:ff`) addresses.
pub fn parse_mac(s: &str) -> Option<[u8; 6]> {
    let s = s.trim();
    let sep = if s.contains(':') { ':' } else { '-' };
    let mut bytes = [0u8; 6];
    let mut parts = s.split(sep);
    for slot in &mut bytes {
        let part = parts.next()?;
        if part.len() != 2 || !part.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        *slot = u8::from_str_radix(part, 16).ok()?;
    }
    if parts.next().is_some() || bytes == [0; 6] || bytes == [0xff; 6] {
        return None;
    }
    Some(bytes)
}

/// Formats a 6-byte hardware (MAC) address as lowercase `"aa:bb:cc:dd:ee:ff"`.
pub fn format_mac(mac: &[u8; 6]) -> String {
    format!("{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}", mac[0], mac[1], mac[2], mac[3], mac[4], mac[5])
}

/// Builds the standard 102-byte Wake-on-LAN magic packet: 6 bytes of `0xFF`
/// followed by `mac` repeated 16 times.
pub fn magic_packet(mac: &[u8; 6]) -> [u8; wake::MAGIC_PACKET_BYTES] {
    let mut pkt = [0xffu8; wake::MAGIC_PACKET_BYTES];
    for i in 0..16 {
        let start = 6 + i * 6;
        pkt[start..start + 6].copy_from_slice(mac);
    }
    pkt
}

/// Computes the directed IPv4 subnet broadcast address (`ip | !mask`) from an
/// adapter's IPv4 address and on-link prefix length (`1..=30`). Rejects
/// unspecified, loopback, link-local (`169.254.0.0/16`), multicast and
/// broadcast addresses.
pub fn ipv4_broadcast(ip: std::net::Ipv4Addr, prefix_len: u8) -> Option<std::net::Ipv4Addr> {
    if !(1..=30).contains(&prefix_len)
        || ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_broadcast()
    {
        return None;
    }
    let mask = u32::MAX << (32 - u32::from(prefix_len));
    let bcast = std::net::Ipv4Addr::from(u32::from(ip) | !mask);
    (!bcast.is_unspecified() && !bcast.is_loopback() && !bcast.is_multicast()).then_some(bcast)
}

fn parse_wake_broadcast(s: &str) -> Option<std::net::Ipv4Addr> {
    let ip: std::net::Ipv4Addr = s.trim().parse().ok()?;
    if ip.is_unspecified() || ip.is_loopback() || ip.is_link_local() || ip.is_multicast() {
        return None;
    }
    Some(ip)
}

/// Body of `pc.wake_info` (`docs/protocol/actions.md` §2.3): the PC's
/// wake-capable adapter MAC addresses and directed subnet broadcast addresses.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PcWakeInfo {
    /// Hardware addresses (`"aa:bb:cc:dd:ee:ff"`), preferred adapter first.
    /// Empty when the user turned `pc_actions` off for this phone.
    #[serde(default)]
    pub macs: Vec<String>,
    /// Directed IPv4 subnet broadcast addresses (e.g. `"192.168.1.255"`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub broadcasts: Vec<String>,
}

/// Never prints hardware or IP addresses in logs (protocol v0 §11).
impl std::fmt::Debug for PcWakeInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PcWakeInfo")
            .field("macs", &self.macs.len())
            .field("broadcasts", &self.broadcasts.len())
            .finish()
    }
}

impl PcWakeInfo {
    pub fn is_valid(&self) -> bool {
        self.macs.len() <= wake::MAX_ADAPTERS
            && self.broadcasts.len() <= wake::MAX_BROADCASTS
            && self.macs.iter().all(|m| parse_mac(m).is_some())
            && self.broadcasts.iter().all(|b| parse_wake_broadcast(b).is_some())
    }

    /// Normalizes MAC addresses to lowercase `"aa:bb:cc:dd:ee:ff"`, filters out
    /// invalid entries, deduplicates preserving order, and enforces protocol
    /// bounds.
    pub fn sanitized(self) -> Self {
        let mut macs = Vec::new();
        for m in self.macs {
            if let Some(bytes) = parse_mac(&m) {
                let norm = format_mac(&bytes);
                if !macs.contains(&norm) {
                    macs.push(norm);
                    if macs.len() >= wake::MAX_ADAPTERS {
                        break;
                    }
                }
            }
        }
        if macs.is_empty() {
            return Self::default();
        }
        let mut broadcasts = Vec::new();
        for b in self.broadcasts {
            if let Some(ip) = parse_wake_broadcast(&b) {
                let norm = ip.to_string();
                if !broadcasts.contains(&norm) {
                    broadcasts.push(norm);
                    if broadcasts.len() >= wake::MAX_BROADCASTS {
                        break;
                    }
                }
            }
        }
        Self { macs, broadcasts }
    }

    /// Encodes as `"mac1,mac2|bcast1,bcast2"` for the `peers.wake_info` column.
    pub fn to_storage_string(&self) -> String {
        let clean = self.clone().sanitized();
        if clean.macs.is_empty() {
            String::new()
        } else {
            format!("{}|{}", clean.macs.join(","), clean.broadcasts.join(","))
        }
    }

    /// Decodes from the `peers.wake_info` column (`None` when empty or invalid).
    pub fn from_storage_str(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        let (macs_part, bcasts_part) = s.split_once('|').unwrap_or((s, ""));
        let info = Self {
            macs: macs_part.split(',').filter(|p| !p.is_empty()).map(str::to_owned).collect(),
            broadcasts: bcasts_part.split(',').filter(|p| !p.is_empty()).map(str::to_owned).collect(),
        }
        .sanitized();
        (!info.macs.is_empty()).then_some(info)
    }
}

/// Relayed Wake-on-LAN request (`wake.relay`): sent to an online paired peer on
/// the target LAN asking it to broadcast a UDP magic packet for `mac`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WakeRelay {
    pub mac: [u8; 6],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub broadcast: Option<String>,
}

/// Low-latency PCM audio frame (`audio.pcm`) streamed from a PC to a phone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioPcmFrame {
    pub sample_rate: u32,
    pub channels: u8,
    #[serde(with = "serde_bytes")]
    pub pcm: Vec<u8>,
}

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
    pub const H265: &str = "h265";
    pub const AV1: &str = "av1";
    /// Offered by phones that stream all their sound while mirrored.
    pub const AUDIO: &str = "mirror.audio";
    /// Offered by phones that stream the sound of apps that allow it.
    pub const AUDIO_PLAYBACK: &str = "mirror.audio.playback";
    /// Offered by PCs that play a mirrored phone's sound.
    pub const LISTEN: &str = "mirror.listen";
    pub const OP_AUDIO: &str = "audio";
    /// 16-bit little-endian PCM, channels interleaved.
    pub const PCM: &str = "pcm_s16le";
    /// Offered by phones that run an app on a display of its own, shown in
    /// a window of its own (Elevated).
    pub const VIRTUAL_DISPLAY: &str = "mirror.virtual_display";
    /// The session of the phone's own screen; app windows have others.
    pub const SCREEN: u32 = 0;
    /// Apps in a `mirror.apps` answer, at most.
    pub const MAX_APPS: usize = 500;
    /// An app's icon (PNG), at most.
    pub const MAX_ICON_BYTES: usize = 8 * 1024;
}

/// Whether `name` looks like an Android package name.
pub fn is_package_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name.split('.').count() >= 2
        && name.split('.').all(|part| {
            part.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
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
    /// The PC's number for this mirroring: [`mirror::SCREEN`] for the
    /// phone's screen, any other for an app window.
    #[serde(default)]
    pub session: u32,
    /// An app (package name) to run on a display of its own and show,
    /// instead of the phone's screen (with a session other than SCREEN).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// Keep the phone awake while mirroring its screen (`session == SCREEN`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stay_awake: bool,
    /// Turn the phone's physical screen off while mirroring (`session == SCREEN`, Elevated).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub screen_off: bool,
    /// Play on phone: show what this PC player (its ID, as in `media.state`)
    /// plays instead of the PC's screen (`session == SCREEN`, sent to a PC
    /// that offered it with `media.watch`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watch: Option<String>,
}

impl MirrorStart {
    /// The screen without an app, or an app with a session of its own.
    pub fn is_valid(&self) -> bool {
        let watch_ok = self
            .watch
            .as_ref()
            .is_none_or(|p| self.app.is_none() && (1..=media_limits::ID_BYTES).contains(&p.len()));
        watch_ok
            && match &self.app {
                None => self.session == mirror::SCREEN,
                Some(app) => {
                    self.session != mirror::SCREEN
                        && is_package_name(app)
                        && !self.stay_awake
                        && !self.screen_off
                }
            }
    }
}

/// Body of `mirror.power`: change `stay_awake` or `screen_off` while mirroring
/// the phone's screen (`session == SCREEN`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MirrorPower {
    #[serde(default)]
    pub stay_awake: bool,
    #[serde(default)]
    pub screen_off: bool,
}

/// Which mirroring a `mirror.stop`, `mirror.keyframe` or `mirror.input` is
/// about (its body, or part of it; missing means the screen).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MirrorSession {
    #[serde(default)]
    pub session: u32,
}

/// Body of `mirror.resize`: resize an app window's display (`session != 0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MirrorResize {
    pub session: u32,
    pub width: u32,
    pub height: u32,
}

impl MirrorResize {
    pub fn is_valid(&self) -> bool {
        self.session != mirror::SCREEN
            && (64..=4096).contains(&self.width)
            && (64..=4096).contains(&self.height)
    }
}

/// An app a PC can open in a window (in a `mirror.apps` answer).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhoneApp {
    /// Package name.
    pub pkg: String,
    pub label: String,
    /// A small icon (PNG), at most [`mirror::MAX_ICON_BYTES`].
    #[serde(default, skip_serializing_if = "Option::is_none", with = "serde_bytes")]
    pub icon: Option<Vec<u8>>,
}

impl std::fmt::Debug for PhoneApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhoneApp")
            .field("pkg", &self.pkg)
            .field("icon", &self.icon.as_ref().map(Vec::len))
            .finish_non_exhaustive()
    }
}

/// Body of a `mirror.apps` answer: the phone's apps, by name.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PhoneApps {
    pub apps: Vec<PhoneApp>,
}

/// Body of `mirror.input`: the PC's mouse, keyboard, and gamepad on the mirrored
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
    /// Gamepad controller state (XInput-style buttons bitmask, stick axes, and triggers).
    Gamepad { buttons: u16, lx: i16, ly: i16, rx: i16, ry: i16, lt: u8, rt: u8 },
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
    pub const KEYBOARD_ON: &str = "keyboard_on";
    pub const KEYBOARD_OFF: &str = "keyboard_off";
    pub const ALL: &[&str] = &[
        BACK,
        HOME,
        RECENTS,
        ENTER,
        BACKSPACE,
        DELETE,
        LEFT,
        RIGHT,
        UP,
        DOWN,
        TAB,
        NOTIFICATIONS,
        KEYBOARD_ON,
        KEYBOARD_OFF,
    ];
}

/// `mirror.input`: the input's fields, and the session when it isn't the
/// screen (phones that predate sessions ignore it).
pub fn mirror_input_envelope(
    input: &MirrorInput,
    session: u32,
) -> Result<crate::Envelope, crate::ProtocolError> {
    let mut env = crate::Envelope::new(types::MIRROR_INPUT, input)?;
    if session != mirror::SCREEN
        && let Some(ciborium::Value::Map(fields)) = env.b.as_mut()
    {
        fields.push((ciborium::Value::Text("session".into()), ciborium::Value::Integer(session.into())));
    }
    Ok(env)
}

/// Never prints typed text (protocol v0 §11).
impl std::fmt::Debug for MirrorInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MirrorInput::Touch { action, .. } => write!(f, "Touch({action:?})"),
            MirrorInput::Scroll { .. } => f.write_str("Scroll"),
            MirrorInput::Key { key } => write!(f, "Key({key})"),
            MirrorInput::Text { text } => write!(f, "Text({} bytes)", text.len()),
            MirrorInput::Gamepad { buttons, .. } => write!(f, "Gamepad({buttons:#06x})"),
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
            MirrorInput::Gamepad { .. } => true,
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
    /// Which mirroring this stream is (see [`MirrorStart::session`]).
    #[serde(default)]
    pub session: u32,
}

impl MirrorConfig {
    pub fn to_cbor(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let _ = ciborium::into_writer(self, &mut out);
        out
    }

    pub fn from_cbor(bytes: &[u8]) -> Result<MirrorConfig, crate::ProtocolError> {
        ciborium::from_reader(bytes).map_err(|e| crate::ProtocolError::BadMessage(e.to_string()))
    }

    /// A format this version can show: H.264, H.265, or AV1, 1-8192 pixels a side.
    pub fn is_valid(&self) -> bool {
        matches!(self.codec.as_str(), mirror::H264 | mirror::H265 | mirror::AV1)
            && (1..=8192).contains(&self.width)
            && (1..=8192).contains(&self.height)
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
        let _ = ciborium::into_writer(self, &mut out);
        out
    }

    pub fn from_cbor(bytes: &[u8]) -> Result<MirrorAudioConfig, crate::ProtocolError> {
        ciborium::from_reader(bytes).map_err(|e| crate::ProtocolError::BadMessage(e.to_string()))
    }

    /// A format this version can play: PCM, 8-96 kHz, mono or stereo.
    pub fn is_valid(&self) -> bool {
        self.codec == mirror::PCM && (8_000..=96_000).contains(&self.rate) && (1..=2).contains(&self.channels)
    }

    /// Bytes in one sample frame (a sample for each channel).
    pub fn frame_bytes(&self) -> usize {
        2 * usize::from(self.channels)
    }
}

// ---- Phone as webcam (docs/protocol/webcam.md) ----

/// Phone as microphone (`docs/protocol/mic.md`).
/// Elevated over USB (`docs/protocol/elevated-usb.md`): a phone without
/// wireless debugging asks the PC it's plugged into to start its Elevated
/// helper over USB debugging.
pub mod elevated {
    /// Offered by PCs that can start a USB-connected phone's helper.
    pub const USB: &str = "elevated.usb";
    /// The only package the PC runs anything for.
    pub const PACKAGE: &str = "app.nectarlink";
    /// The helper the PC starts, with `app_process`.
    pub const HELPER_CLASS: &str = "app.nectarlink.android.elevated.InputServer";
    /// Grants a request may carry, at most.
    pub const MAX_GRANTS: usize = 8;
}

/// Body of `elevated.usb.start`: start the helper from `apk` with `token`,
/// after running `grants` (each one of the phone's own, see [`is_valid_grant`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElevatedUsbStart {
    pub apk: String,
    pub token: String,
    #[serde(default)]
    pub grants: Vec<String>,
}

impl ElevatedUsbStart {
    pub fn is_valid(&self) -> bool {
        is_valid_app_apk(&self.apk)
            && (32..=64).contains(&self.token.len())
            && self.token.bytes().all(|b| b.is_ascii_hexdigit())
            && self.grants.len() <= elevated::MAX_GRANTS
            && self.grants.iter().all(|g| is_valid_grant(g))
    }
}

/// Body of `elevated.usb.ready`: the helper listens on this loopback port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElevatedUsbReady {
    pub port: u16,
}

/// Characters a path or command the PC passes to the phone's shell may use:
/// nothing the shell would read as more than a word.
fn is_shell_word(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._/=~+-:".contains(&b))
}

/// The app's own installed APK (`pm path` gives `/data/app/<...>/base.apk`).
pub fn is_valid_app_apk(apk: &str) -> bool {
    apk.len() <= 512
        && apk.starts_with("/data/app/")
        && apk.ends_with("/base.apk")
        && !apk.contains("..")
        && apk.contains(elevated::PACKAGE)
        && is_shell_word(apk)
}

/// NectarLink's package. Debug builds of the PC app also accept its local
/// test build (`app.nectarlink.dev`, the `applicationIdSuffix` of a
/// side-by-side build); release builds accept no other package.
pub fn is_own_package(p: &str) -> bool {
    p == elevated::PACKAGE || (cfg!(debug_assertions) && p.strip_prefix(elevated::PACKAGE) == Some(".dev"))
}

/// A grant the phone may ask for itself, and nothing else: a permission or
/// app op for NectarLink's own package, or its own notification listener.
pub fn is_valid_grant(grant: &str) -> bool {
    let words: Vec<&str> = grant.split(' ').collect();
    if !words.iter().all(|w| is_shell_word(w)) {
        return false;
    }
    match words.as_slice() {
        ["pm", "grant", p, perm] => is_own_package(p) && perm.starts_with("android.permission."),
        ["appops", "set", p, op, "allow"] => {
            is_own_package(p) && op.bytes().all(|b| b.is_ascii_uppercase() || b == b'_')
        }
        ["cmd", "notification", "allow_dnd", p] => is_own_package(p),
        ["cmd", "notification", "allow_listener", component] => {
            component.split_once('/').is_some_and(|(p, rest)| is_own_package(p) && !rest.is_empty())
        }
        _ => false,
    }
}

pub mod mic {
    /// Offered by phones that stream their microphone.
    pub const STREAM: &str = "mic.stream";
    /// Offered by PCs that take a phone's microphone.
    pub const LISTEN: &str = "mic.listen";
    pub const SERVICE: &str = "mic";
    pub const OP_PCM: &str = "pcm";
    pub const VERSION: u32 = 1;
    /// Stream reset code: the microphone stopped.
    pub const STOPPED: u32 = 13;
}

pub mod webcam {
    /// Offered by phones that stream their camera as a webcam.
    pub const STREAM: &str = "camera.stream";
    /// Offered by PCs that receive and decode a phone's webcam stream.
    pub const VIRTUAL: &str = "camera.virtual";
    /// Receiver can decode HEVC webcam streams. Absence means H.264 only.
    pub const HEVC: &str = "camera.virtual.hevc";
    /// Offered by PCs where the Windows virtual camera COM add-on is registered.
    pub const ADDON_VCAM: &str = "addon.vcam";
    pub const SERVICE: &str = "webcam";
    pub const OP_VIDEO: &str = "video";
    pub const VERSION: u32 = 1;
    /// Stream reset code: webcam stopped.
    pub const STOPPED: u32 = 12;
    pub const H264: &str = "h264";
    pub const H265: &str = "h265";
    pub const CAMERA_BACK: &str = "back";
    pub const CAMERA_FRONT: &str = "front";
}

fn default_webcam_camera() -> String {
    webcam::CAMERA_BACK.to_owned()
}

fn default_webcam_fps() -> u32 {
    30
}

fn default_webcam_bitrate() -> u32 {
    6_000_000
}

/// Body of `webcam.start`: ask a paired phone to stream its camera (or update
/// camera/resolution while already streaming).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebcamStart {
    /// `"back"` or `"front"`.
    #[serde(default = "default_webcam_camera")]
    pub camera: String,
    /// Requested frame width in pixels (e.g. 1280, 1920, 3840).
    pub width: u32,
    /// Requested frame height in pixels (e.g. 720, 1080, 2160).
    pub height: u32,
    /// Target frames per second (`1..=60`).
    #[serde(default = "default_webcam_fps")]
    pub fps: u32,
    /// Target bitrate in bits per second (`100_000..=50_000_000`).
    #[serde(default = "default_webcam_bitrate")]
    pub bitrate: u32,
}

impl Default for WebcamStart {
    fn default() -> Self {
        Self {
            camera: default_webcam_camera(),
            width: 1280,
            height: 720,
            fps: default_webcam_fps(),
            bitrate: default_webcam_bitrate(),
        }
    }
}

impl WebcamStart {
    pub fn is_valid(&self) -> bool {
        (self.camera == webcam::CAMERA_BACK || self.camera == webcam::CAMERA_FRONT)
            && (320..=3840).contains(&self.width)
            && (240..=2160).contains(&self.height)
            && (1..=60).contains(&self.fps)
            && (100_000..=50_000_000).contains(&self.bitrate)
    }
}

/// The format of a webcam stream (a `PacketKind::Config` packet's payload, in CBOR).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebcamConfig {
    /// `"h264"` or `"h265"` (only when the receiver advertises webcam::HEVC).
    pub codec: String,
    pub width: u32,
    pub height: u32,
    /// `"back"` or `"front"`.
    #[serde(default = "default_webcam_camera")]
    pub camera: String,
    /// Frames per second.
    #[serde(default = "default_webcam_fps")]
    pub fps: u32,
    /// The highest resolution the camera in use can stream, by its short
    /// side: 2160 when it can do 4K, else 1080. Phones before 0.1.5 don't
    /// say, and fall back to 1080p themselves when asked for 4K it can't do.
    #[serde(default = "default_webcam_max_height")]
    pub max_height: u32,
}

fn default_webcam_max_height() -> u32 {
    2160
}

impl WebcamConfig {
    pub fn to_cbor(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let _ = ciborium::into_writer(self, &mut out);
        out
    }

    pub fn from_cbor(bytes: &[u8]) -> Result<WebcamConfig, crate::ProtocolError> {
        ciborium::from_reader(bytes).map_err(|e| crate::ProtocolError::BadMessage(e.to_string()))
    }

    pub fn is_valid(&self) -> bool {
        matches!(self.codec.as_str(), webcam::H264 | webcam::H265)
            && (1..=8192).contains(&self.width)
            && (1..=8192).contains(&self.height)
            && (self.camera == webcam::CAMERA_BACK || self.camera == webcam::CAMERA_FRONT)
            && (1..=120).contains(&self.fps)
    }
}

// ---- Continuity Camera (docs/protocol/v0.md §Continuity Camera) ----

pub mod continuity_camera {
    /// Offered by phones that can capture a still photo or scan a document on request.
    pub const CAPTURE: &str = "camera.capture";
    /// Offered by PCs that can request and receive Continuity Camera captures.
    pub const RECEIVE: &str = "camera.continuity";

    /// The `stream` header's `svc` for a Continuity Camera result upload.
    pub const SERVICE: &str = "camera";
    /// The `stream` header's `op` for sending a captured photo or scanned document.
    pub const OP_RESULT: &str = "result";
    pub const VERSION: u32 = 1;

    /// Stream frame type sent before the raw image bytes on a `camera` / `result` stream.
    pub const RESULT_META: &str = "camera.capture.meta";

    /// Capture mode string values.
    pub const MODE_PHOTO: &str = "photo";
    pub const MODE_SCAN: &str = "scan";

    /// Maximum image payload size for a Continuity Camera capture (32 MiB).
    pub const MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;
    /// Maximum length of a request ID in bytes.
    pub const MAX_ID_BYTES: usize = 64;
    /// Maximum length of a file name in bytes.
    pub const MAX_NAME_BYTES: usize = 255;
}

/// Mode requested for a Continuity Camera capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CameraCaptureMode {
    /// Capture a single high-resolution photo.
    Photo,
    /// Capture and perspective-crop / contrast-enhance a document scan.
    Scan,
}

impl CameraCaptureMode {
    /// Wire/display string (`"photo"` or `"scan"`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Photo => continuity_camera::MODE_PHOTO,
            Self::Scan => continuity_camera::MODE_SCAN,
        }
    }

    /// Parses `"photo"` or `"scan"` (case-insensitive).
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            continuity_camera::MODE_PHOTO => Some(Self::Photo),
            continuity_camera::MODE_SCAN => Some(Self::Scan),
            _ => None,
        }
    }
}

/// `camera.capture.request`: PC asks a paired phone to open its camera UI and capture a photo or scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraCaptureRequest {
    pub request_id: String,
    pub mode: CameraCaptureMode,
}

/// `camera.capture.ok`: Phone acknowledges that the capture UI was launched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraCaptureOk {
    pub request_id: String,
}

/// `camera.capture.cancel`: Either side cancels an in-flight capture request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraCaptureCancel {
    pub request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// `camera.capture.meta`: Header frame sent on a `camera` / `result` stream before the raw image bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraCaptureResultMeta {
    pub request_id: String,
    pub mode: CameraCaptureMode,
    pub file_name: String,
    /// `"image/jpeg"` or `"image/png"`.
    pub mime: String,
    pub size: u64,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
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
    /// Most image attachments in one `sms.send` message.
    pub const MAX_ATTACHMENTS: usize = 1;
    /// An image sent with `sms.send` (MMS): at most this many bytes.
    pub const MAX_ATTACHMENT_BYTES: usize = MAX_PART_BYTES;
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

/// An image attachment sent with `sms.send` (MMS).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmsAttachment {
    /// `"image/jpeg"` or `"image/png"`.
    pub mime: String,
    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>,
}

impl std::fmt::Debug for SmsAttachment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmsAttachment").field("mime", &self.mime).field("bytes", &self.data.len()).finish()
    }
}

impl SmsAttachment {
    pub fn is_valid(&self) -> bool {
        matches!(
            self.mime.as_str(),
            "image/jpeg"
                | "image/png"
                | "image/gif"
                | "image/webp"
                | "audio/mp4"
                | "audio/m4a"
                | "audio/aac"
                | "audio/mpeg"
                | "audio/ogg"
                | "audio/wav"
                | "audio/amr"
                | "audio/3gpp"
        ) && !self.data.is_empty()
            && self.data.len() <= sms::MAX_ATTACHMENT_BYTES
    }
}

/// Body of `sms.send`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmsSend {
    pub to: Vec<String>,
    #[serde(default)]
    pub body: String,
    /// Optional image attachment(s) to send as MMS.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<SmsAttachment>,
}

/// Never prints numbers or text (protocol v0 §11).
impl std::fmt::Debug for SmsSend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmsSend")
            .field("to", &self.to.len())
            .field("bytes", &self.body.len())
            .field("attachments", &self.attachments.len())
            .finish()
    }
}

impl SmsSend {
    pub fn is_valid(&self) -> bool {
        (1..=sms::MAX_RECIPIENTS).contains(&self.to.len())
            && self.to.iter().all(|a| (1..=64).contains(&a.trim().len()))
            && (!self.body.trim().is_empty() || !self.attachments.is_empty())
            && self.body.len() <= sms::MAX_SEND_BYTES
            && self.attachments.len() <= sms::MAX_ATTACHMENTS
            && self.attachments.iter().all(SmsAttachment::is_valid)
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
    /// Offered by phones that list recent calls from the call log.
    pub const LOG: &str = "call.log";
    /// Offered by phones that place calls (or open the dialer) when a PC asks.
    pub const DIAL: &str = "call.dial";
    /// A caller's photo: a JPEG of at most this many bytes.
    pub const MAX_PHOTO_BYTES: usize = 64 * 1024;
    /// A contact's photo on a call log entry: a JPEG of at most this many bytes.
    pub const MAX_LOG_PHOTO_BYTES: usize = 16 * 1024;
    /// Most call log entries in one answer.
    pub const MAX_LOG_PAGE: u32 = 100;
    pub const MAX_ID_BYTES: usize = 64;
    pub const MAX_TEXT_BYTES: usize = 256;

    pub const RINGING: &str = "ringing";
    pub const ACTIVE: &str = "active";
    pub const ENDED: &str = "ended";

    pub const DIR_INCOMING: &str = "incoming";
    pub const DIR_OUTGOING: &str = "outgoing";
    pub const DIR_MISSED: &str = "missed";
    pub const DIR_REJECTED: &str = "rejected";

    pub const ANSWER: &str = "answer";
    pub const DECLINE: &str = "decline";
    pub const SILENCE: &str = "silence";
    pub const MUTE: &str = "mute";
    pub const UNMUTE: &str = "unmute";
    pub const SPEAKER: &str = "speaker";
    pub const EARPIECE: &str = "earpiece";
    pub const HOLD: &str = "hold";
    pub const UNHOLD: &str = "unhold";
    /// A keypad tone: `digit` is one of `0`-`9`, `*`, `#`.
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

/// Whether `digit` is a keypad key (`0`-`9`, `*`, `#`).
pub fn is_dtmf_digit(digit: &str) -> bool {
    digit.len() == 1 && digit.chars().all(|c| c.is_ascii_digit() || c == '*' || c == '#')
}

/// One call in the phone's call history (`call.log`).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallLogEntry {
    pub id: String,
    /// Who called, or who was called (empty when withheld).
    #[serde(default)]
    pub number: String,
    /// The contact's name, when the number is a contact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// `incoming`, `outgoing`, `missed` or `rejected`.
    pub direction: String,
    /// When the call started, in Unix milliseconds.
    pub date: i64,
    /// Seconds on the call (`0` when missed or not answered).
    #[serde(default)]
    pub duration: u32,
    /// The contact's photo (JPEG, at most 16 KiB).
    #[serde(default, skip_serializing_if = "Option::is_none", with = "serde_bytes")]
    pub photo: Option<Vec<u8>>,
}

/// Never prints the number, name or photo (protocol v0 §11).
impl std::fmt::Debug for CallLogEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallLogEntry")
            .field("id", &self.id)
            .field("direction", &self.direction)
            .field("duration", &self.duration)
            .finish_non_exhaustive()
    }
}

/// Body of `call.log`: recent calls before `before` (Unix milliseconds; the
/// latest when missing), newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallLogGet {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<i64>,
    pub limit: u32,
}

/// Answer to `call.log`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallLog {
    pub entries: Vec<CallLogEntry>,
}

/// Body of `call.log.changed`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallLogChanged {}

/// Body of `call.dial`: ask the phone to call `number`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallDial {
    pub number: String,
}

/// Never prints the number (protocol v0 §11).
impl std::fmt::Debug for CallDial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallDial").field("bytes", &self.number.len()).finish()
    }
}

impl CallDial {
    pub fn is_valid(&self) -> bool {
        !self.number.trim().is_empty() && self.number.len() <= calls::MAX_TEXT_BYTES
    }
}

// ---- Contacts (docs/protocol/contacts.md) ----

pub mod contacts {
    /// Offered by phones that share their contacts.
    pub const READ: &str = "contacts.read";
    /// Offered by PCs that show contacts.
    pub const SHOW: &str = "contacts.show";
    /// Most contacts in one answer.
    pub const MAX_PAGE: u32 = 200;
    /// A search query: at most this many bytes.
    pub const MAX_QUERY_BYTES: usize = 256;
    /// Most phone numbers on one contact.
    pub const MAX_NUMBERS: usize = 16;
    /// A contact's photo: a JPEG of at most this many bytes.
    pub const MAX_PHOTO_BYTES: usize = 16 * 1024;
}

/// One phone number on a contact.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactNumber {
    pub number: String,
    /// Optional type/label, e.g. `"mobile"`, `"home"`, `"work"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Never prints the number or label (protocol v0 §11).
impl std::fmt::Debug for ContactNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContactNumber").finish_non_exhaustive()
    }
}

/// A contact on the phone (`contacts.list`).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contact {
    pub id: String,
    pub name: String,
    pub numbers: Vec<ContactNumber>,
    /// Favorite / starred on the phone.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub starred: bool,
    /// Small JPEG thumbnail (at most 16 KiB).
    #[serde(default, skip_serializing_if = "Option::is_none", with = "serde_bytes")]
    pub photo: Option<Vec<u8>>,
}

/// Never prints the name, numbers or photo (protocol v0 §11).
impl std::fmt::Debug for Contact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Contact")
            .field("id", &self.id)
            .field("numbers", &self.numbers.len())
            .field("starred", &self.starred)
            .finish_non_exhaustive()
    }
}

/// Body of `contacts.list` request.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactsListGet {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub offset: u32,
    pub limit: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// Never prints the search query (protocol v0 §11).
impl std::fmt::Debug for ContactsListGet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContactsListGet")
            .field("has_query", &self.query.is_some())
            .field("offset", &self.offset)
            .field("limit", &self.limit)
            .finish()
    }
}

/// Answer to `contacts.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactsList {
    pub contacts: Vec<Contact>,
}

/// Body of `contacts.changed`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactsChanged {}

// ---- Photos (docs/protocol/photos.md) ----

pub mod photos {
    /// Offered by phones that announce new photos (and can read them).
    pub const READ: &str = "photos.read";
    /// Offered by PCs that show announced photos and the phone's gallery.
    pub const SHOW: &str = "photos.show";
    /// A photo's ID: at most this many bytes.
    pub const MAX_ID_BYTES: usize = 64;
    /// A preview or thumbnail: a JPEG of at most this many bytes.
    pub const MAX_THUMB_BYTES: usize = 96 * 1024;
    /// Most items in one `photos.list` answer.
    pub const MAX_PAGE: u32 = 200;
    /// Most thumbnails requested in one `photos.thumbs` batch.
    pub const MAX_THUMB_BATCH: usize = 24;
    /// Most full items requested in one `photos.get` transfer.
    pub const MAX_GET_ITEMS: usize = 200;
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

/// Whether a photo ID is 1-64 printable ASCII characters.
pub fn is_valid_photo_id(id: &str) -> bool {
    (1..=photos::MAX_ID_BYTES).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_graphic())
}

/// One album on the phone (`photos.albums.list`).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhotoAlbum {
    /// Album bucket identifier on the phone.
    pub id: String,
    /// Display name, e.g. `"Camera"`, `"Screenshots"`.
    pub name: String,
    /// Number of photos and videos in the album.
    pub count: u32,
    /// Newest item's ID, for its cover thumbnail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<String>,
}

/// Never prints the album's name (protocol v0 §11).
impl std::fmt::Debug for PhotoAlbum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhotoAlbum").field("id", &self.id).field("count", &self.count).finish_non_exhaustive()
    }
}

/// Body of `photos.albums` request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhotoAlbumsGet {}

/// Answer to `photos.albums` (`photos.albums.list`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhotoAlbumsList {
    #[serde(default)]
    pub albums: Vec<PhotoAlbum>,
}

/// Body of `photos.list`: photos and videos on the phone (or in `album`),
/// newest first, after the item at (`before`, `before_id`) when given (the
/// last item of the previous page), else from the latest.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhotoListGet {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    /// The previous page's last `date` (Unix milliseconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<i64>,
    /// The previous page's last `id`, so items sharing its date aren't
    /// skipped or repeated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_id: Option<String>,
    pub limit: u32,
}

impl std::fmt::Debug for PhotoListGet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhotoListGet")
            .field("has_album", &self.album.is_some())
            .field("before", &self.before)
            .field("before_id", &self.before_id)
            .field("limit", &self.limit)
            .finish()
    }
}

/// One photo or video in `photos.items`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhotoItem {
    pub id: String,
    pub name: String,
    /// When it was taken (Unix milliseconds), or else when it was saved.
    pub date: i64,
    pub size: u64,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    /// Video duration in milliseconds (`None` for photos).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<u32>,
    /// Album bucket identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
}

/// Never prints the file name (protocol v0 §11).
impl std::fmt::Debug for PhotoItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhotoItem")
            .field("id", &self.id)
            .field("date", &self.date)
            .field("size", &self.size)
            .field("video", &self.duration.is_some())
            .finish_non_exhaustive()
    }
}

/// Answer to `photos.list` (`photos.items`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhotoItems {
    #[serde(default)]
    pub items: Vec<PhotoItem>,
}

/// Body of `photos.thumbs`: ask for small JPEG thumbnails for these IDs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhotoThumbsGet {
    pub ids: Vec<String>,
}

/// One thumbnail in `photos.thumbs.list`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhotoThumb {
    pub id: String,
    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>,
}

/// Never prints thumbnail bytes (protocol v0 §11).
impl std::fmt::Debug for PhotoThumb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhotoThumb").field("id", &self.id).field("bytes", &self.data.len()).finish()
    }
}

/// Answer to `photos.thumbs` (`photos.thumbs.list`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhotoThumbsList {
    #[serde(default)]
    pub thumbs: Vec<PhotoThumb>,
}

/// Body of `photos.get`: send one (`id`) or several (`ids`) photos or videos.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhotoGet {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ids: Vec<String>,
}

impl PhotoGet {
    /// Requested item IDs in order (`ids` when non-empty, otherwise `[id]`).
    pub fn requested_ids(&self) -> Vec<String> {
        if !self.ids.is_empty() {
            self.ids.clone()
        } else if !self.id.is_empty() {
            vec![self.id.clone()]
        } else {
            Vec::new()
        }
    }
}

/// Body of `photos.sending`, the answer to `photos.get`: the files
/// transfer that brings it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhotoSending {
    pub transfer: String,
}

/// Body of `photos.changed`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhotosChanged {}

/// Body of `link.open`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkOpen {
    pub url: String,
    /// Optional human-readable title (e.g. the active video's title from the sender's media player).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// Never prints the link or title (protocol v0 §11).
impl std::fmt::Debug for LinkOpen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinkOpen")
            .field("bytes", &self.url.len())
            .field("has_title", &self.title.is_some())
            .finish()
    }
}

/// Limits for `task.notify` (`docs/protocol/actions.md`).
pub mod task_notify {
    /// Maximum UTF-8 bytes in a task identifier.
    pub const MAX_ID_BYTES: usize = 64;
    /// Maximum Unicode scalar values in a task title.
    pub const MAX_TITLE_CHARS: usize = 120;
    /// Maximum concurrent active tasks tracked per Node.
    pub const MAX_ACTIVE: usize = 32;
}

/// Body of `task.notify`: live or completed PC task status sent to the phone.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskNotify {
    /// Stable identifier for this watched task on the sender.
    pub id: String,
    /// Human-readable task name (e.g. `"cargo build --release"`).
    pub title: String,
    /// `true` while the task is running; `false` when it has finished or been dismissed.
    pub active: bool,
    /// Elapsed wall-clock time in milliseconds since the task started.
    #[serde(default)]
    pub elapsed_ms: u64,
    /// Process exit code when the task has finished (`active == false`).
    /// When `active == false` and `exit_code == None`, the ongoing notification is dismissed silently.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

/// Never prints the task title in logs (protocol v0 §11).
impl std::fmt::Debug for TaskNotify {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskNotify")
            .field("id", &self.id)
            .field("active", &self.active)
            .field("elapsed_ms", &self.elapsed_ms)
            .field("exit_code", &self.exit_code)
            .finish_non_exhaustive()
    }
}

impl TaskNotify {
    /// Whether `id` and `title` satisfy protocol bounds.
    pub fn is_valid(&self) -> bool {
        let id_ok = (1..=task_notify::MAX_ID_BYTES).contains(&self.id.len())
            && self.id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':'));
        let title_trimmed = self.title.trim();
        let title_ok = !title_trimmed.is_empty()
            && self.title.chars().count() <= task_notify::MAX_TITLE_CHARS
            && !self.title.chars().any(char::is_control);
        id_ok && title_ok && (!self.active || self.exit_code.is_none())
    }

    /// Clamps and sanitizes a [`TaskNotify`] within protocol bounds, or returns `None` if invalid.
    pub fn sanitized(mut self) -> Option<Self> {
        let id: String = self
            .id
            .bytes()
            .filter(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':'))
            .take(task_notify::MAX_ID_BYTES)
            .map(char::from)
            .collect();
        if id.is_empty() {
            return None;
        }
        let filtered_title: String = self.title.chars().filter(|c| !c.is_control()).collect();
        let mut title = filtered_title.trim().to_owned();
        truncate_chars(&mut title, task_notify::MAX_TITLE_CHARS);
        if title.is_empty() {
            return None;
        }
        self.id = id;
        self.title = title;
        if self.active {
            self.exit_code = None;
        }
        Some(self)
    }

    /// Formats `elapsed_ms` as `"Took 38s"`, `"Took 4m 12s"`, or `"Took 1h 04m"`.
    pub fn format_took(elapsed_ms: u64) -> String {
        let total_secs = elapsed_ms / 1000;
        let hours = total_secs / 3600;
        let mins = (total_secs % 3600) / 60;
        let secs = total_secs % 60;
        if hours > 0 {
            format!("Took {hours}h {mins:02}m")
        } else if mins > 0 {
            format!("Took {mins}m {secs:02}s")
        } else {
            format!("Took {secs}s")
        }
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
    pub const RESUME: &str = "files.resume";
    pub const CANCEL: &str = "files.cancel";
    /// Stream reset code for a cancelled transfer.
    pub const CANCELLED: u32 = 10;
    /// Stream reset code for a manually paused transfer (receiver keeps `.part` files).
    pub const PAUSED: u32 = 11;
    pub const MAX_FILES: usize = 5000;
    pub const MAX_NAME_BYTES: usize = 255;
    /// A file's folder: at most this many bytes and this many levels.
    pub const MAX_FOLDER_BYTES: usize = 1024;
    pub const MAX_FOLDER_DEPTH: usize = 32;
    /// Offered by PCs that save and convert voice recordings (`docs/protocol/recorder.md`).
    pub const RECORDER: &str = "recorder";
    /// Most markers on one voice recording.
    pub const MAX_MARKERS: usize = 256;
    /// Most UTF-8 bytes in a marker's label.
    pub const MAX_MARKER_LABEL_BYTES: usize = 256;
}

/// A timestamped marker placed during a voice recording (`docs/protocol/recorder.md`).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingMarker {
    /// Elapsed milliseconds from the start of the recording (excluding pauses).
    pub at_ms: u64,
    /// Optional user label for the marker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Never prints the marker's label (protocol v0 §11).
impl std::fmt::Debug for RecordingMarker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordingMarker")
            .field("at_ms", &self.at_ms)
            .field("has_label", &self.label.is_some())
            .finish()
    }
}

impl RecordingMarker {
    pub fn is_valid(&self) -> bool {
        self.label.as_ref().is_none_or(|l| {
            !l.trim().is_empty()
                && l.len() <= files::MAX_MARKER_LABEL_BYTES
                && !l.chars().any(char::is_control)
        })
    }
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
    /// Optional lowercase 64-character hex BLAKE3 digest of the file's full bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blake3: Option<String>,
}

/// Never prints the name (protocol v0 §11).
impl std::fmt::Debug for FileEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileEntry").field("size", &self.size).finish_non_exhaustive()
    }
}

impl FileEntry {
    pub fn is_valid(&self) -> bool {
        is_valid_file_name(&self.name)
            && self.folder.as_deref().is_none_or(is_valid_folder)
            && self.blake3.as_deref().is_none_or(is_valid_blake3_hex)
    }
}

/// Body of `files.offer`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilesOffer {
    pub id: String,
    pub files: Vec<FileEntry>,
    /// True when this transfer is a voice recording (`docs/protocol/recorder.md`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub recording: bool,
    /// Timestamped markers placed during the recording.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub markers: Vec<RecordingMarker>,
    /// True when the sender asked the receiver to open this single safe document
    /// with the OS default application once it finishes arriving (Rich Handoff).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub open_on_arrival: bool,
}

/// Never prints file names (protocol v0 §11).
impl std::fmt::Debug for FilesOffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FilesOffer")
            .field("id", &self.id)
            .field("files", &self.files.len())
            .field("recording", &self.recording)
            .field("markers", &self.markers.len())
            .field("open_on_arrival", &self.open_on_arrival)
            .finish()
    }
}

/// Body of `files.accept`: bytes already received, per file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilesAccept {
    pub have: Vec<u64>,
}

/// Body of `files.resume` and `files.cancel` control messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilesControl {
    pub id: String,
}

impl FilesControl {
    pub fn is_valid(&self) -> bool {
        is_valid_transfer_id(&self.id)
    }
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

/// Whether a folder is 1-32 valid names joined by `/`, in at most
/// [`files::MAX_FOLDER_BYTES`].
pub fn is_valid_folder(folder: &str) -> bool {
    folder.len() <= files::MAX_FOLDER_BYTES
        && folder.split('/').count() <= files::MAX_FOLDER_DEPTH
        && folder.split('/').all(is_valid_file_name)
}

/// Whether a string is a 64-character ASCII hexadecimal BLAKE3 digest.
pub fn is_valid_blake3_hex(hex: &str) -> bool {
    hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Whether a transfer ID is 16-64 of `[A-Za-z0-9_-]`.
pub fn is_valid_transfer_id(id: &str) -> bool {
    (16..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

impl FilesOffer {
    pub fn is_valid(&self) -> bool {
        let files_ok = if self.recording {
            self.files.len() == 1
                && self.files[0].folder.is_none()
                && self.markers.len() <= files::MAX_MARKERS
                && self.markers.iter().all(RecordingMarker::is_valid)
        } else {
            (1..=files::MAX_FILES).contains(&self.files.len()) && self.markers.is_empty()
        };
        is_valid_transfer_id(&self.id) && files_ok && self.files.iter().all(FileEntry::is_valid)
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtf: Option<String>,
}

impl ClipSet {
    /// Whether the text and optional rich-text fields are within the spec's limits.
    pub fn is_valid(&self) -> bool {
        if self.text.is_empty() {
            return false;
        }
        let html_len = self.html.as_ref().map_or(0, String::len);
        let rtf_len = self.rtf.as_ref().map_or(0, String::len);
        self.text.len().saturating_add(html_len).saturating_add(rtf_len) <= CLIP_MAX_BYTES
    }
}

/// Never prints content (protocol v0 §11).
impl std::fmt::Debug for ClipSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClipSet")
            .field("bytes", &self.text.len())
            .field("has_html", &self.html.is_some())
            .field("has_rtf", &self.rtf.is_some())
            .finish()
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

/// Limits and wire constants for `remote.*` (`docs/protocol/remote.md`).
pub mod remote {
    /// One-byte QUIC datagram channel tag (`docs/protocol/v0.md` §4).
    pub const DATAGRAM_TAG: u8 = 0x01;
    /// Largest datagram body accepted after [`DATAGRAM_TAG`].
    pub const MAX_DATAGRAM_BYTES: usize = 256;
    /// Largest relative pointer step (`dx` or `dy`), in logical pixels.
    pub const MAX_MOVE_DELTA: f32 = 4000.0;
    /// Largest scroll step (`dx` or `dy`), in wheel notches.
    pub const MAX_SCROLL_DELTA: f32 = 200.0;
    /// Most UTF-8 bytes in one `Text` event.
    pub const MAX_TEXT_BYTES: usize = 256;
    /// Most modifiers on a key press.
    pub const MAX_MODS: usize = 4;
    /// Stream service and operation when falling back from datagrams.
    pub const SERVICE: &str = "remote";
    pub const OP_MOTION: &str = "motion";
    pub const VERSION: u32 = 1;
}

/// A mouse button on the PC (`docs/protocol/remote.md` §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// What to do with a [`MouseButton`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ButtonAction {
    Down,
    Up,
    Click,
}

/// A modifier held during a [`RemoteInput::Key`] press.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyMod {
    Ctrl,
    Alt,
    Shift,
    Win,
}

/// A presentation remote action (`docs/protocol/remote.md` §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlideAction {
    Next,
    Previous,
    Start,
    Stop,
    Black,
}

/// Keys and shortcuts accepted by [`RemoteInput::Key`].
pub mod remote_keys {
    use super::KeyMod;

    pub const ENTER: &str = "enter";
    pub const BACKSPACE: &str = "backspace";
    pub const TAB: &str = "tab";
    pub const ESCAPE: &str = "escape";
    pub const SPACE: &str = "space";
    pub const LEFT: &str = "left";
    pub const RIGHT: &str = "right";
    pub const UP: &str = "up";
    pub const DOWN: &str = "down";
    pub const HOME: &str = "home";
    pub const END: &str = "end";
    pub const PAGE_UP: &str = "page_up";
    pub const PAGE_DOWN: &str = "page_down";
    pub const DELETE: &str = "delete";
    pub const F1: &str = "f1";
    pub const F2: &str = "f2";
    pub const F3: &str = "f3";
    pub const F4: &str = "f4";
    pub const F5: &str = "f5";
    pub const F6: &str = "f6";
    pub const F7: &str = "f7";
    pub const F8: &str = "f8";
    pub const F9: &str = "f9";
    pub const F10: &str = "f10";
    pub const F11: &str = "f11";
    pub const F12: &str = "f12";
    pub const WIN: &str = "win";

    pub const NAMED: &[&str] = &[
        ENTER, BACKSPACE, TAB, ESCAPE, SPACE, LEFT, RIGHT, UP, DOWN, HOME, END, PAGE_UP, PAGE_DOWN, DELETE,
        F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12, WIN,
    ];

    /// Common shortcut aliases (`mods` must be empty when using an alias).
    pub const COPY: &str = "copy";
    pub const PASTE: &str = "paste";
    pub const CUT: &str = "cut";
    pub const UNDO: &str = "undo";
    pub const SELECT_ALL: &str = "select_all";
    pub const TASK_VIEW: &str = "task_view";
    pub const LOCK: &str = "lock";

    pub const SHORTCUTS: &[&str] = &[COPY, PASTE, CUT, UNDO, SELECT_ALL, TASK_VIEW, LOCK];

    /// Whether `key` and `mods` form a valid [`super::RemoteInput::Key`].
    pub fn is_valid(key: &str, mods: &[KeyMod]) -> bool {
        if mods.len() > super::remote::MAX_MODS {
            return false;
        }
        // Reject duplicate modifiers.
        for (i, m) in mods.iter().enumerate() {
            if mods[i + 1..].contains(m) {
                return false;
            }
        }
        if NAMED.contains(&key) {
            return true;
        }
        if SHORTCUTS.contains(&key) {
            return mods.is_empty();
        }
        // Single ASCII lowercase letter or digit is valid when combined with at
        // least one modifier (e.g. Ctrl+C, Win+L, Alt+1).
        if let [b] = key.as_bytes() {
            return !mods.is_empty() && (b.is_ascii_lowercase() || b.is_ascii_digit());
        }
        false
    }
}

/// An input event sent from a phone to a PC (`docs/protocol/remote.md`).
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum RemoteInput {
    /// Relative pointer motion in logical pixels (`dx` right, `dy` down).
    /// Travels in QUIC datagrams (or the `remote/motion` stream), never on the
    /// control stream.
    Move { dx: f32, dy: f32 },
    /// Mouse button press, release or click.
    Button { button: MouseButton, action: ButtonAction },
    /// Smooth or stepped scroll in wheel notches (`dx` right, `dy` vertical).
    Scroll { dx: f32, dy: f32 },
    /// Typed Unicode text.
    Text { text: String },
    /// A named key, modified key, or shortcut.
    Key {
        key: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        mods: Vec<KeyMod>,
    },
    /// A presentation remote action.
    Slide { action: SlideAction },
    /// Laser pointer position on the PC's primary screen (`0.0..=1.0`).
    Laser {
        on: bool,
        #[serde(default)]
        x: f32,
        #[serde(default)]
        y: f32,
    },
}

/// Never prints typed text (protocol v0 §11).
impl std::fmt::Debug for RemoteInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RemoteInput::Move { dx, dy } => write!(f, "Move({dx}, {dy})"),
            RemoteInput::Button { button, action } => write!(f, "Button({button:?}, {action:?})"),
            RemoteInput::Scroll { dx, dy } => write!(f, "Scroll({dx}, {dy})"),
            RemoteInput::Text { text } => write!(f, "Text({} bytes)", text.len()),
            RemoteInput::Key { key, mods } if mods.is_empty() => write!(f, "Key({key})"),
            RemoteInput::Key { key, mods } => write!(f, "Key({mods:?}+{key})"),
            RemoteInput::Slide { action } => write!(f, "Slide({action:?})"),
            RemoteInput::Laser { on: false, .. } => f.write_str("Laser(off)"),
            RemoteInput::Laser { on: true, x, y } => write!(f, "Laser({x}, {y})"),
        }
    }
}

impl RemoteInput {
    pub fn is_valid(&self) -> bool {
        match self {
            RemoteInput::Move { dx, dy } => {
                dx.is_finite()
                    && dy.is_finite()
                    && dx.abs() <= remote::MAX_MOVE_DELTA
                    && dy.abs() <= remote::MAX_MOVE_DELTA
            }
            RemoteInput::Button { .. } => true,
            RemoteInput::Scroll { dx, dy } => {
                dx.is_finite()
                    && dy.is_finite()
                    && dx.abs() <= remote::MAX_SCROLL_DELTA
                    && dy.abs() <= remote::MAX_SCROLL_DELTA
            }
            RemoteInput::Text { text } => {
                !text.is_empty()
                    && text.len() <= remote::MAX_TEXT_BYTES
                    && !text.chars().any(char::is_control)
            }
            RemoteInput::Key { key, mods } => remote_keys::is_valid(key, mods),
            RemoteInput::Slide { .. } => true,
            RemoteInput::Laser { on, x, y } => {
                x.is_finite()
                    && y.is_finite()
                    && (!*on || ((0.0..=1.0).contains(x) && (0.0..=1.0).contains(y)))
            }
        }
    }

    /// Whether this event may travel in a lossy QUIC datagram.
    pub fn is_datagram_allowed(&self) -> bool {
        matches!(self, RemoteInput::Move { .. } | RemoteInput::Scroll { .. } | RemoteInput::Laser { .. })
    }

    /// Whether this event may travel in a `remote.input` control request.
    /// Pointer motion (`Move`) never uses the control stream.
    pub fn is_control_allowed(&self) -> bool {
        !matches!(self, RemoteInput::Move { .. })
    }

    /// Encodes a datagram prefixed with [`remote::DATAGRAM_TAG`].
    pub fn to_datagram(&self) -> Vec<u8> {
        let mut out = vec![remote::DATAGRAM_TAG];
        let _ = ciborium::into_writer(self, &mut out);
        out
    }

    /// Decodes and validates a datagram prefixed with [`remote::DATAGRAM_TAG`].
    pub fn from_datagram(bytes: &[u8]) -> Result<Self, crate::ProtocolError> {
        if bytes.first().copied() != Some(remote::DATAGRAM_TAG) {
            return Err(crate::ProtocolError::BadMessage("unknown datagram channel tag".into()));
        }
        let body = &bytes[1..];
        if body.is_empty() || body.len() > remote::MAX_DATAGRAM_BYTES {
            return Err(crate::ProtocolError::FrameTooLarge(body.len()));
        }
        let input: Self =
            ciborium::from_reader(body).map_err(|e| crate::ProtocolError::BadMessage(e.to_string()))?;
        if !input.is_valid() || !input.is_datagram_allowed() {
            return Err(crate::ProtocolError::BadMessage("invalid remote input datagram".into()));
        }
        Ok(input)
    }
}

// ---- Phone toggles (docs/protocol/toggles.md) ----

/// Capability IDs for `phone.toggles` and `phone.toggle.set`.
pub mod toggles {
    pub const READ: &str = "toggles.read";
    pub const RINGER: &str = "toggles.ringer";
    pub const VOLUME: &str = "toggles.volume";
    pub const FLASHLIGHT: &str = "toggles.flashlight";
    pub const DND: &str = "toggles.dnd";
    pub const BRIGHTNESS: &str = "toggles.brightness";
    pub const WIFI: &str = "toggles.wifi";
    pub const BLUETOOTH: &str = "toggles.bluetooth";
    pub const SHOW: &str = "toggles.show";
}

/// Toggle IDs in `phone.toggle.set`.
pub mod toggle_ids {
    pub const DND: &str = "dnd";
    pub const RINGER: &str = "ringer";
    pub const FLASHLIGHT: &str = "flashlight";
    pub const VOLUME: &str = "volume";
    pub const BRIGHTNESS: &str = "brightness";
    pub const WIFI: &str = "wifi";
    pub const BLUETOOTH: &str = "bluetooth";
    pub const HOTSPOT: &str = "hotspot";
    pub const PC_ACTIVE: &str = "pc_active";
    pub const ALL: &[&str] =
        &[DND, RINGER, FLASHLIGHT, VOLUME, BRIGHTNESS, WIFI, BLUETOOTH, HOTSPOT, PC_ACTIVE];
}

/// Ringer modes for [`PhoneToggles::ringer`] and `phone.toggle.set` (`id = "ringer"`).
pub mod ringer_modes {
    pub const RING: &str = "ring";
    pub const VIBRATE: &str = "vibrate";
    pub const SILENT: &str = "silent";
}

/// Body of `phone.toggles` (`docs/protocol/toggles.md` §2.1): a phone's
/// current quick settings state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhoneToggles {
    pub dnd: bool,
    /// `"ring"`, `"vibrate"` or `"silent"`.
    pub ringer: String,
    /// `None` when the phone has no flash unit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flashlight: Option<bool>,
    /// Media volume, `0..=100`.
    pub volume: u8,
    /// Screen brightness, `0..=100`.
    pub brightness: u8,
    pub wifi: bool,
    pub bluetooth: bool,
}

impl PhoneToggles {
    pub fn is_valid(&self) -> bool {
        matches!(self.ringer.as_str(), ringer_modes::RING | ringer_modes::VIBRATE | ringer_modes::SILENT)
            && self.volume <= 100
            && self.brightness <= 100
    }

    /// Validates `ringer` and clamps `volume` and `brightness` to `0..=100`.
    pub fn sanitized(mut self) -> Option<Self> {
        if !matches!(self.ringer.as_str(), ringer_modes::RING | ringer_modes::VIBRATE | ringer_modes::SILENT)
        {
            return None;
        }
        self.volume = self.volume.min(100);
        self.brightness = self.brightness.min(100);
        Some(self)
    }
}

/// A toggle value in [`PhoneToggleSet`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PhoneToggleValue {
    Bool(bool),
    Level(u8),
    Mode(String),
}

/// Body of `phone.toggle.set` (`docs/protocol/toggles.md` §2.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhoneToggleSet {
    pub id: String,
    pub value: PhoneToggleValue,
}

impl PhoneToggleSet {
    /// Checks that `id` is a known toggle and `value` has the type and range
    /// that toggle accepts.
    pub fn is_valid(&self) -> bool {
        match (self.id.as_str(), &self.value) {
            (
                toggle_ids::DND
                | toggle_ids::FLASHLIGHT
                | toggle_ids::WIFI
                | toggle_ids::BLUETOOTH
                | toggle_ids::HOTSPOT
                | toggle_ids::PC_ACTIVE,
                PhoneToggleValue::Bool(_),
            ) => true,
            (toggle_ids::VOLUME | toggle_ids::BRIGHTNESS, PhoneToggleValue::Level(n)) => *n <= 100,
            (toggle_ids::RINGER, PhoneToggleValue::Mode(m)) => {
                matches!(m.as_str(), ringer_modes::RING | ringer_modes::VIBRATE | ringer_modes::SILENT)
            }
            _ => false,
        }
    }
}

// ---- Deck (docs/protocol/deck.md) ----

pub mod deck {
    /// Offered by PCs that share their Deck layout and live state and run Deck actions.
    pub const ACTIONS: &str = "deck.actions";
    /// Offered by PCs that share their active audio output devices and accept `pc.audio.set`.
    pub const PC_AUDIO: &str = "pc.audio";
    /// Maximum pages in a `DeckLayout`.
    pub const MAX_PAGES: usize = 8;
    /// Maximum tiles on a single `DeckPage`.
    pub const MAX_TILES_PER_PAGE: usize = 24;
    /// Maximum bytes in a page or tile ID.
    pub const MAX_ID_BYTES: usize = 64;
    /// Maximum UTF-8 bytes in a page name or tile label.
    pub const MAX_LABEL_BYTES: usize = 64;
    /// Maximum active audio output devices in a `DeckState`.
    pub const MAX_OUTPUT_DEVICES: usize = 32;
    /// Maximum bytes in an audio endpoint ID.
    pub const MAX_DEVICE_ID_BYTES: usize = 256;
    /// Maximum UTF-8 bytes in an audio endpoint friendly name.
    pub const MAX_DEVICE_NAME_BYTES: usize = 128;
}

/// Action kinds for [`DeckTile::kind`] (`docs/protocol/deck.md` §2.4).
pub mod deck_kinds {
    pub const MEDIA_PLAY_PAUSE: &str = "media_play_pause";
    pub const MEDIA_NEXT: &str = "media_next";
    pub const MEDIA_PREVIOUS: &str = "media_previous";
    pub const VOLUME_UP: &str = "volume_up";
    pub const VOLUME_DOWN: &str = "volume_down";
    pub const VOLUME_MUTE: &str = "volume_mute";
    pub const MIC_MUTE: &str = "mic_mute";
    pub const LOCK_PC: &str = "lock_pc";
    pub const SHOW_DESKTOP: &str = "show_desktop";
    pub const SWITCH_WINDOW: &str = "switch_window";
    pub const SCREENSHOT: &str = "screenshot";
    pub const SHORTCUT: &str = "shortcut";
    pub const OPEN_URL: &str = "open_url";
    pub const TYPE_TEXT: &str = "type_text";
    pub const LAUNCH_APP: &str = "launch_app";
    pub const RUN_COMMAND: &str = "run_command";

    pub const ALL: &[&str] = &[
        MEDIA_PLAY_PAUSE,
        MEDIA_NEXT,
        MEDIA_PREVIOUS,
        VOLUME_UP,
        VOLUME_DOWN,
        VOLUME_MUTE,
        MIC_MUTE,
        LOCK_PC,
        SHOW_DESKTOP,
        SWITCH_WINDOW,
        SCREENSHOT,
        SHORTCUT,
        OPEN_URL,
        TYPE_TEXT,
        LAUNCH_APP,
        RUN_COMMAND,
    ];

    /// Default icon for an action kind.
    pub fn default_icon(kind: &str) -> &'static str {
        use super::deck_icons::*;
        match kind {
            MEDIA_PLAY_PAUSE => PLAY,
            MEDIA_NEXT => SKIP_NEXT,
            MEDIA_PREVIOUS => SKIP_PREVIOUS,
            VOLUME_UP => VOLUME_UP,
            VOLUME_DOWN => VOLUME_DOWN,
            VOLUME_MUTE => VOLUME_OFF,
            MIC_MUTE => MIC,
            LOCK_PC => LOCK,
            SHOW_DESKTOP => DESKTOP,
            SWITCH_WINDOW => SWITCH_WINDOW,
            SCREENSHOT => SCREENSHOT,
            SHORTCUT => SHORTCUT,
            OPEN_URL => GLOBE,
            TYPE_TEXT => TEXT,
            LAUNCH_APP => APP,
            RUN_COMMAND => TERMINAL,
            _ => SPARKLE,
        }
    }

    /// Default color for an action kind.
    pub fn default_color(kind: &str) -> &'static str {
        use super::deck_colors::*;
        match kind {
            MEDIA_PLAY_PAUSE => AMBER,
            MEDIA_NEXT | MEDIA_PREVIOUS => SLATE,
            VOLUME_UP | VOLUME_DOWN | VOLUME_MUTE => TEAL,
            MIC_MUTE => CORAL,
            LOCK_PC => RED,
            SHOW_DESKTOP | SWITCH_WINDOW => BLUE,
            SCREENSHOT => VIOLET,
            SHORTCUT => AMBER,
            OPEN_URL => BLUE,
            TYPE_TEXT => GREEN,
            LAUNCH_APP => TEAL,
            RUN_COMMAND => CORAL,
            _ => AMBER,
        }
    }

    /// Default label for an action kind.
    pub fn default_label(kind: &str) -> &'static str {
        match kind {
            MEDIA_PLAY_PAUSE => "Play / pause",
            MEDIA_NEXT => "Next",
            MEDIA_PREVIOUS => "Previous",
            VOLUME_UP => "Volume up",
            VOLUME_DOWN => "Volume down",
            VOLUME_MUTE => "Mute audio",
            MIC_MUTE => "Mic mute",
            LOCK_PC => "Lock PC",
            SHOW_DESKTOP => "Show desktop",
            SWITCH_WINDOW => "Switch window",
            SCREENSHOT => "Screenshot",
            SHORTCUT => "Shortcut",
            OPEN_URL => "Open website",
            TYPE_TEXT => "Type text",
            LAUNCH_APP => "Launch app",
            RUN_COMMAND => "Run command",
            _ => "Action",
        }
    }
}

/// Fixed icon identifiers for [`DeckTile::icon`] (`docs/protocol/deck.md` §2.4).
pub mod deck_icons {
    pub const PLAY: &str = "play";
    pub const PAUSE: &str = "pause";
    pub const SKIP_NEXT: &str = "skip_next";
    pub const SKIP_PREVIOUS: &str = "skip_previous";
    pub const VOLUME_UP: &str = "volume_up";
    pub const VOLUME_DOWN: &str = "volume_down";
    pub const VOLUME_OFF: &str = "volume_off";
    pub const MIC: &str = "mic";
    pub const MIC_OFF: &str = "mic_off";
    pub const LOCK: &str = "lock";
    pub const DESKTOP: &str = "desktop";
    pub const SWITCH_WINDOW: &str = "switch_window";
    pub const SCREENSHOT: &str = "screenshot";
    pub const SHORTCUT: &str = "shortcut";
    pub const GLOBE: &str = "globe";
    pub const TEXT: &str = "text";
    pub const APP: &str = "app";
    pub const TERMINAL: &str = "terminal";
    pub const SPARKLE: &str = "sparkle";
    pub const STAR: &str = "star";

    pub const ALL: &[&str] = &[
        PLAY,
        PAUSE,
        SKIP_NEXT,
        SKIP_PREVIOUS,
        VOLUME_UP,
        VOLUME_DOWN,
        VOLUME_OFF,
        MIC,
        MIC_OFF,
        LOCK,
        DESKTOP,
        SWITCH_WINDOW,
        SCREENSHOT,
        SHORTCUT,
        GLOBE,
        TEXT,
        APP,
        TERMINAL,
        SPARKLE,
        STAR,
    ];
}

/// Fixed color identifiers for [`DeckTile::color`] (`docs/protocol/deck.md` §2.4).
pub mod deck_colors {
    pub const AMBER: &str = "amber";
    pub const CORAL: &str = "coral";
    pub const RED: &str = "red";
    pub const TEAL: &str = "teal";
    pub const GREEN: &str = "green";
    pub const BLUE: &str = "blue";
    pub const VIOLET: &str = "violet";
    pub const SLATE: &str = "slate";

    pub const ALL: &[&str] = &[AMBER, CORAL, RED, TEAL, GREEN, BLUE, VIOLET, SLATE];
}

/// Whether a Deck page or tile ID is 1..=64 ASCII alphanumeric, `_`, `-`, or `.` characters.
pub fn is_valid_deck_id(id: &str) -> bool {
    (1..=deck::MAX_ID_BYTES).contains(&id.len())
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

fn is_valid_deck_label(s: &str) -> bool {
    let trimmed = s.trim();
    !trimmed.is_empty() && s.len() <= deck::MAX_LABEL_BYTES && !s.chars().any(char::is_control)
}

fn sanitize_deck_label(s: &str) -> Option<String> {
    let filtered: String = s.chars().filter(|c| !c.is_control()).collect();
    let trimmed = filtered.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut end = trimmed.len().min(deck::MAX_LABEL_BYTES);
    while end > 0 && !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    let out = trimmed[..end].trim().to_owned();
    (!out.is_empty()).then_some(out)
}

/// One tile in a [`DeckPage`] (`docs/protocol/deck.md` §2.1).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeckTile {
    pub id: String,
    pub label: String,
    pub icon: String,
    pub color: String,
    pub kind: String,
}

/// Never prints the user label (protocol v0 §11).
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

impl DeckTile {
    pub fn is_valid(&self) -> bool {
        is_valid_deck_id(&self.id)
            && is_valid_deck_label(&self.label)
            && deck_icons::ALL.contains(&self.icon.as_str())
            && deck_colors::ALL.contains(&self.color.as_str())
            && deck_kinds::ALL.contains(&self.kind.as_str())
    }

    pub fn sanitized(self) -> Option<Self> {
        if !is_valid_deck_id(&self.id) || !deck_kinds::ALL.contains(&self.kind.as_str()) {
            return None;
        }
        let raw_label = sanitize_deck_label(&self.label)?;
        let label = match (self.kind.as_str(), raw_label.as_str()) {
            (deck_kinds::MEDIA_PLAY_PAUSE, "Play / Pause") => "Play / pause".into(),
            (deck_kinds::VOLUME_UP, "Volume Up") => "Volume up".into(),
            (deck_kinds::VOLUME_DOWN, "Volume Down") => "Volume down".into(),
            (deck_kinds::VOLUME_MUTE, "Mute Audio") => "Mute audio".into(),
            (deck_kinds::MIC_MUTE, "Mic Mute") => "Mic mute".into(),
            (deck_kinds::SHOW_DESKTOP, "Show Desktop") => "Show desktop".into(),
            (deck_kinds::SWITCH_WINDOW, "Switch Window") => "Switch window".into(),
            (deck_kinds::OPEN_URL, "Open Website") => "Open website".into(),
            (deck_kinds::TYPE_TEXT, "Type Text") => "Type text".into(),
            (deck_kinds::LAUNCH_APP, "Launch App") => "Launch app".into(),
            (deck_kinds::RUN_COMMAND, "Run Command") => "Run command".into(),
            _ => raw_label,
        };
        let icon = if deck_icons::ALL.contains(&self.icon.as_str()) {
            self.icon
        } else {
            deck_kinds::default_icon(&self.kind).into()
        };
        let color = if deck_colors::ALL.contains(&self.color.as_str()) {
            self.color
        } else {
            deck_kinds::default_color(&self.kind).into()
        };
        Some(DeckTile { id: self.id, label, icon, color, kind: self.kind })
    }
}

/// One page of tiles in [`DeckLayout`] (`docs/protocol/deck.md` §2.1).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeckPage {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub tiles: Vec<DeckTile>,
}

/// Never prints the page name (protocol v0 §11).
impl std::fmt::Debug for DeckPage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeckPage")
            .field("id", &self.id)
            .field("tiles", &self.tiles.len())
            .finish_non_exhaustive()
    }
}

/// Body of `deck.layout`: pages and tiles configured on the PC (`docs/protocol/deck.md` §2.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeckLayout {
    pub pages: Vec<DeckPage>,
}

impl DeckLayout {
    pub fn is_valid(&self) -> bool {
        if !(1..=deck::MAX_PAGES).contains(&self.pages.len()) {
            return false;
        }
        let mut page_ids = std::collections::BTreeSet::new();
        let mut tile_ids = std::collections::BTreeSet::new();
        for page in &self.pages {
            if !is_valid_deck_id(&page.id)
                || !is_valid_deck_label(&page.name)
                || page.tiles.len() > deck::MAX_TILES_PER_PAGE
                || !page_ids.insert(page.id.as_str())
            {
                return false;
            }
            for tile in &page.tiles {
                if !tile.is_valid() || !tile_ids.insert(tile.id.as_str()) {
                    return false;
                }
            }
        }
        true
    }

    pub fn sanitized(self) -> Option<Self> {
        let mut page_ids = std::collections::BTreeSet::new();
        let mut tile_ids = std::collections::BTreeSet::new();
        let mut pages = Vec::new();
        for page in self.pages {
            if pages.len() >= deck::MAX_PAGES {
                break;
            }
            if !is_valid_deck_id(&page.id) || !page_ids.insert(page.id.clone()) {
                continue;
            }
            let Some(name) = sanitize_deck_label(&page.name) else {
                continue;
            };
            let mut tiles = Vec::new();
            for tile in page.tiles {
                if tiles.len() >= deck::MAX_TILES_PER_PAGE {
                    break;
                }
                if let Some(clean) = tile.sanitized()
                    && tile_ids.insert(clean.id.clone())
                {
                    tiles.push(clean);
                }
            }
            pages.push(DeckPage { id: page.id, name, tiles });
        }
        (!pages.is_empty()).then_some(DeckLayout { pages })
    }

    /// Finds a tile by ID across all pages.
    pub fn tile(&self, id: &str) -> Option<&DeckTile> {
        self.pages.iter().flat_map(|p| p.tiles.iter()).find(|t| t.id == id)
    }

    /// The default single-page Deck layout (`docs/protocol/deck.md`).
    pub fn default_layout() -> Self {
        let tile = |id: &str, kind: &str| DeckTile {
            id: id.into(),
            label: deck_kinds::default_label(kind).into(),
            icon: deck_kinds::default_icon(kind).into(),
            color: deck_kinds::default_color(kind).into(),
            kind: kind.into(),
        };
        DeckLayout {
            pages: vec![DeckPage {
                id: "main".into(),
                name: "Main".into(),
                tiles: vec![
                    tile("play_pause", deck_kinds::MEDIA_PLAY_PAUSE),
                    tile("prev_track", deck_kinds::MEDIA_PREVIOUS),
                    tile("next_track", deck_kinds::MEDIA_NEXT),
                    tile("vol_down", deck_kinds::VOLUME_DOWN),
                    tile("vol_up", deck_kinds::VOLUME_UP),
                    tile("vol_mute", deck_kinds::VOLUME_MUTE),
                    tile("mic_mute", deck_kinds::MIC_MUTE),
                    tile("show_desktop", deck_kinds::SHOW_DESKTOP),
                    tile("switch_window", deck_kinds::SWITCH_WINDOW),
                    tile("screenshot", deck_kinds::SCREENSHOT),
                    tile("lock_pc", deck_kinds::LOCK_PC),
                ],
            }],
        }
    }
}

impl Default for DeckLayout {
    fn default() -> Self {
        Self::default_layout()
    }
}

/// One active audio output endpoint on the PC (`docs/protocol/deck.md` §2.2).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioOutputDevice {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_default: bool,
}

/// Never prints the user's audio device name or system ID in logs (protocol v0 §11).
impl std::fmt::Debug for AudioOutputDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioOutputDevice").field("is_default", &self.is_default).finish_non_exhaustive()
    }
}

impl AudioOutputDevice {
    pub fn is_valid(&self) -> bool {
        let id = self.id.trim();
        let name = self.name.trim();
        !id.is_empty()
            && self.id.len() <= deck::MAX_DEVICE_ID_BYTES
            && !self.id.chars().any(char::is_control)
            && !name.is_empty()
            && self.name.len() <= deck::MAX_DEVICE_NAME_BYTES
            && !self.name.chars().any(char::is_control)
    }

    pub fn sanitized(self) -> Option<Self> {
        let id: String = self.id.chars().filter(|c| !c.is_control()).collect();
        let id = id.trim();
        if id.is_empty() || id.len() > deck::MAX_DEVICE_ID_BYTES {
            return None;
        }
        let name: String = self.name.chars().filter(|c| !c.is_control()).collect();
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        let mut end = name.len().min(deck::MAX_DEVICE_NAME_BYTES);
        while end > 0 && !name.is_char_boundary(end) {
            end -= 1;
        }
        let name = name[..end].trim().to_owned();
        if name.is_empty() {
            return None;
        }
        Some(AudioOutputDevice { id: id.to_owned(), name, is_default: self.is_default })
    }
}

/// Body of `deck.state`: live PC state reflected on Deck tiles (`docs/protocol/deck.md` §2.2).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeckState {
    #[serde(default)]
    pub playing: bool,
    #[serde(default)]
    pub volume: u8,
    #[serde(default)]
    pub muted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mic_muted: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output_devices: Vec<AudioOutputDevice>,
}

impl DeckState {
    pub fn is_valid(&self) -> bool {
        self.volume <= 100
            && self.output_devices.len() <= deck::MAX_OUTPUT_DEVICES
            && self.output_devices.iter().all(AudioOutputDevice::is_valid)
    }

    pub fn sanitized(mut self) -> Option<Self> {
        self.volume = self.volume.min(100);
        self.output_devices = self
            .output_devices
            .into_iter()
            .filter_map(AudioOutputDevice::sanitized)
            .take(deck::MAX_OUTPUT_DEVICES)
            .collect();
        Some(self)
    }

    /// Friendly name of the default audio output device, if reported.
    pub fn default_output_name(&self) -> Option<&str> {
        self.output_devices
            .iter()
            .find(|d| d.is_default)
            .or_else(|| self.output_devices.first())
            .map(|d| d.name.as_str())
    }

    /// Short live status text for a tile of `kind`, if `kind` is a live tile.
    pub fn tile_status(&self, kind: &str) -> Option<String> {
        match kind {
            deck_kinds::MEDIA_PLAY_PAUSE => {
                Some(if self.playing { "Playing".into() } else { "Paused".into() })
            }
            deck_kinds::VOLUME_UP | deck_kinds::VOLUME_DOWN => Some(if self.muted {
                format!("Muted · {}%", self.volume)
            } else {
                format!("{}%", self.volume)
            }),
            deck_kinds::VOLUME_MUTE => {
                Some(if self.muted { "Muted".into() } else { format!("{}%", self.volume) })
            }
            deck_kinds::MIC_MUTE => match self.mic_muted {
                Some(true) => Some("Muted".into()),
                Some(false) => Some("Live".into()),
                None => Some("No mic".into()),
            },
            _ => None,
        }
    }
}

/// Body of `deck.press`: asks the PC to run a tile's action (`docs/protocol/deck.md` §2.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeckPress {
    pub tile: String,
}

impl DeckPress {
    pub fn is_valid(&self) -> bool {
        is_valid_deck_id(&self.tile)
    }
}

/// Body of `pc.audio.set`: asks the PC to change its master speaker volume
/// (`0..=100`) and/or mute state (`docs/protocol/deck.md` §2.5).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PcAudioSet {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted: Option<bool>,
}

impl PcAudioSet {
    pub fn is_valid(&self) -> bool {
        let vol_ok = self.volume.is_none_or(|v| v <= 100 || v == 200 || v == 201);
        (self.volume.is_some() || self.muted.is_some()) && vol_ok
    }
}

// ---- Storage (docs/protocol/storage.md) ----

pub mod storage {
    /// Offered by phones that list folders and read files in shared storage.
    pub const READ: &str = "storage.read";
    /// Offered by phones that create folders, write/upload files, rename and delete entries.
    pub const WRITE: &str = "storage.write";
    /// Offered by PCs that mount phone storage in File Explorer.
    pub const MOUNT: &str = "storage.mount";
    pub const SERVICE: &str = "storage";
    pub const OP_READ: &str = "read";
    pub const OP_WRITE: &str = "write";
    pub const VERSION: u32 = 1;
    /// Maximum UTF-8 bytes in a relative storage path.
    pub const MAX_PATH_BYTES: usize = 1024;
    /// Maximum `/`-separated path segments.
    pub const MAX_SEGMENTS: usize = 32;
    /// Maximum UTF-8 bytes in a single entry name.
    pub const MAX_NAME_BYTES: usize = 255;
    /// Maximum bytes in a write upload ID.
    pub const MAX_ID_BYTES: usize = 64;
}

/// Whether `name` is a valid single storage entry name (`1..=255` UTF-8 bytes,
/// not `"."` or `".."`, no `/`, `\`, NUL, or control characters).
pub fn is_valid_storage_name(name: &str) -> bool {
    (1..=storage::MAX_NAME_BYTES).contains(&name.len())
        && name != "."
        && name != ".."
        && !name.chars().any(|c| c == '/' || c == '\\' || c.is_control())
}

/// Whether `path` is a valid non-empty relative storage path (`"DCIM/Camera"`,
/// `"report.pdf"`): `1..=32` `/`-separated segments, at most 1 024 bytes, no
/// leading or trailing `/`, no `.` or `..` segments, no `\` or control chars.
pub fn is_valid_storage_path(path: &str) -> bool {
    if path.is_empty() || path.len() > storage::MAX_PATH_BYTES || path.starts_with('/') || path.ends_with('/')
    {
        return false;
    }
    let mut count = 0usize;
    for seg in path.split('/') {
        count += 1;
        if count > storage::MAX_SEGMENTS || !is_valid_storage_name(seg) {
            return false;
        }
    }
    count >= 1
}

/// Whether `path` is a valid storage directory path (`""` for the shared root,
/// or a valid relative path).
pub fn is_valid_storage_dir_path(path: &str) -> bool {
    path.is_empty() || is_valid_storage_path(path)
}

/// Whether `id` is a valid upload ID (`1..=64` printable ASCII characters
/// without `/` or `\`).
pub fn is_valid_storage_id(id: &str) -> bool {
    (1..=storage::MAX_ID_BYTES).contains(&id.len())
        && id.bytes().all(|b| (0x21..=0x7e).contains(&b) && b != b'/' && b != b'\\')
}

/// Body of `storage.list` (`docs/protocol/storage.md` §3.1).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageList {
    pub path: String,
}

impl std::fmt::Debug for StorageList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageList").finish_non_exhaustive()
    }
}

impl StorageList {
    pub fn is_valid(&self) -> bool {
        is_valid_storage_dir_path(&self.path)
    }
}

/// One directory entry in `storage.entries` (`docs/protocol/storage.md` §3).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageEntry {
    pub name: String,
    pub size: u64,
    /// Last-modified time in Unix milliseconds.
    pub modified: i64,
    pub is_dir: bool,
}

/// Never prints entry names in logs (protocol v0 §11).
impl std::fmt::Debug for StorageEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageEntry")
            .field("size", &self.size)
            .field("modified", &self.modified)
            .field("is_dir", &self.is_dir)
            .finish_non_exhaustive()
    }
}

impl StorageEntry {
    pub fn is_valid(&self) -> bool {
        is_valid_storage_name(&self.name) && self.modified >= 0
    }

    pub fn sanitized(mut self) -> Option<Self> {
        if !is_valid_storage_name(&self.name) {
            return None;
        }
        self.modified = self.modified.max(0);
        if self.is_dir {
            self.size = 0;
        }
        Some(self)
    }
}

/// Body of `storage.entries`: immediate children of a listed folder.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageEntries {
    #[serde(default)]
    pub entries: Vec<StorageEntry>,
}

/// Body of `storage.read` on a `storage/read` stream (`docs/protocol/storage.md` §4.1).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageRead {
    pub path: String,
    #[serde(default)]
    pub offset: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<u64>,
}

impl std::fmt::Debug for StorageRead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageRead")
            .field("offset", &self.offset)
            .field("length", &self.length)
            .finish_non_exhaustive()
    }
}

impl StorageRead {
    pub fn is_valid(&self) -> bool {
        is_valid_storage_path(&self.path)
    }
}

/// Body of `storage.read.meta`: precedes the raw file bytes on a `storage/read` stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageReadMeta {
    pub size: u64,
    pub modified: i64,
    pub length: u64,
}

/// Body of `storage.write` on a `storage/write` stream (`docs/protocol/storage.md` §4.2).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageWriteOffer {
    pub id: String,
    pub path: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<i64>,
}

impl std::fmt::Debug for StorageWriteOffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageWriteOffer")
            .field("id", &self.id)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl StorageWriteOffer {
    pub fn is_valid(&self) -> bool {
        is_valid_storage_id(&self.id)
            && is_valid_storage_path(&self.path)
            && self.modified.is_none_or(|m| m >= 0)
    }
}

/// Body of `storage.write.accept`: how many bytes the phone already has for this upload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageWriteAccept {
    pub have: u64,
}

/// Body of `storage.write.done`: sent by the phone after committing the uploaded file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageWriteDone {
    pub size: u64,
    pub modified: i64,
}

/// Body of `storage.mkdir` (`docs/protocol/storage.md` §3.2).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageMkdir {
    pub path: String,
}

impl std::fmt::Debug for StorageMkdir {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageMkdir").finish_non_exhaustive()
    }
}

impl StorageMkdir {
    pub fn is_valid(&self) -> bool {
        is_valid_storage_path(&self.path)
    }
}

/// Body of `storage.rename` (`docs/protocol/storage.md` §3.3).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageRename {
    pub from: String,
    pub to: String,
}

impl std::fmt::Debug for StorageRename {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageRename").finish_non_exhaustive()
    }
}

impl StorageRename {
    pub fn is_valid(&self) -> bool {
        is_valid_storage_path(&self.from) && is_valid_storage_path(&self.to) && self.from != self.to
    }
}

/// Body of `storage.delete` (`docs/protocol/storage.md` §3.4).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageDelete {
    pub path: String,
    #[serde(default)]
    pub confirmed: bool,
}

impl std::fmt::Debug for StorageDelete {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageDelete").field("confirmed", &self.confirmed).finish_non_exhaustive()
    }
}

impl StorageDelete {
    pub fn is_valid(&self) -> bool {
        is_valid_storage_path(&self.path)
    }
}

/// Body of `storage.changed` (`docs/protocol/storage.md` §3.5).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageChanged {
    pub path: String,
}

impl std::fmt::Debug for StorageChanged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageChanged").finish_non_exhaustive()
    }
}

impl StorageChanged {
    pub fn is_valid(&self) -> bool {
        is_valid_storage_dir_path(&self.path)
    }
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
            screen: None,
            install_id: None,
        }
    }

    #[test]
    fn elevated_usb_only_runs_the_apps_own_helper_and_grants() {
        let apk = "/data/app/~~kG3x_Q==/app.nectarlink-Ab1-cD2==/base.apk";
        let start = ElevatedUsbStart {
            apk: apk.into(),
            token: "0123456789abcdef0123456789abcdef0123456789abcdef".into(),
            grants: vec![
                "pm grant app.nectarlink android.permission.WRITE_SECURE_SETTINGS".into(),
                "appops set app.nectarlink RECEIVE_SENSITIVE_NOTIFICATIONS allow".into(),
                "cmd notification allow_dnd app.nectarlink".into(),
                "cmd notification allow_listener app.nectarlink/app.nectarlink.android.notifications.NotificationListener"
                    .into(),
            ],
        };
        assert!(start.is_valid());

        for apk in [
            "/data/app/other.app-1/base.apk",
            "/sdcard/app.nectarlink/base.apk",
            "/data/app/app.nectarlink-1/../../evil/base.apk",
            "/data/app/app.nectarlink-1/base.apk; reboot",
            "/data/app/app.nectarlink-1/base.apk$(id)",
        ] {
            assert!(!ElevatedUsbStart { apk: apk.into(), ..start.clone() }.is_valid(), "{apk}");
        }
        for grant in [
            "pm grant com.other android.permission.WRITE_SECURE_SETTINGS",
            "pm grant app.nectarlink android.permission.X; rm -rf /sdcard",
            "pm grant app.nectarlink android.permission.X && reboot",
            "appops set app.nectarlink.evil SYSTEM_ALERT_WINDOW allow",
            "cmd notification allow_listener com.other/x.Listener",
            "cmd notification allow_dnd com.other",
            "settings put global adb_enabled 1",
            "reboot",
        ] {
            assert!(!ElevatedUsbStart { grants: vec![grant.into()], ..start.clone() }.is_valid(), "{grant}");
        }
        assert!(
            !ElevatedUsbStart { token: "not-hex-token-not-hex-token-not-hex".into(), ..start.clone() }
                .is_valid()
        );
        assert!(!ElevatedUsbStart { grants: vec![start.grants[0].clone(); 9], ..start.clone() }.is_valid());
        // The side-by-side test build grants itself, in debug builds only.
        let dev = ElevatedUsbStart {
            apk: "/data/app/~~kG3x_Q==/app.nectarlink.dev-Ab1-cD2==/base.apk".into(),
            grants: vec![
                "pm grant app.nectarlink.dev android.permission.WRITE_SECURE_SETTINGS".into(),
                "cmd notification allow_listener app.nectarlink.dev/app.nectarlink.android.notifications.NotificationListener".into(),
            ],
            ..start
        };
        assert_eq!(dev.is_valid(), cfg!(debug_assertions));
    }

    #[test]
    fn device_names_are_cleaned_and_capped() {
        assert_eq!(info().sanitized(), info(), "ordinary info is unchanged");

        let mut dev = info();
        // A right-to-left override would show "Pixel\u{202E}gnp.exe" as "Pixelexe.png".
        dev.name = " Pixel\u{202E}gnp.exe\u{0007}\n ".into();
        dev.os_ver = "1".repeat(100);
        dev.model = Some("\u{2066}\u{2069}".into());
        let clean = dev.sanitized();
        assert_eq!(clean.name, "Pixelgnp.exe");
        assert_eq!(clean.os_ver.chars().count(), MAX_DEVICE_OS_CHARS);
        assert_eq!(clean.model, None);

        let long = DeviceInfo { name: "é".repeat(200), ..info() }.sanitized();
        assert_eq!(long.name.chars().count(), MAX_DEVICE_NAME_CHARS);
    }

    #[test]
    fn screen_shape_and_battery_full_in_round_trip_and_sanitize() {
        let screen = ScreenShape {
            v: 1,
            aspect: 0.45,
            corners: Some(ScreenCorners { tl: 0.088, tr: 0.088, br: 0.088, bl: 0.088 }),
            cutouts: vec![ScreenRect { x: 0.46, y: 0.018, w: 0.08, h: 0.036 }],
            cutout_path: Some("M 0.5 0.018 L 0.54 0.036 L 0.5 0.054 L 0.46 0.036 Z".into()),
        };
        let mut dev = info();
        dev.screen = Some(screen.clone());
        let env = Envelope::new(types::EVENT_DEVICE, &dev).unwrap();
        let back: DeviceInfo = Envelope::from_cbor(&env.to_cbor()).unwrap().body().unwrap();
        assert_eq!(back.clone().sanitized(), dev);

        // Unknown schema version or out-of-range aspect drops screen cleanly.
        assert!(ScreenShape { v: 2, ..screen.clone() }.sanitized().is_none());
        assert!(ScreenShape { aspect: 0.05, ..screen.clone() }.sanitized().is_none());
        assert!(ScreenShape { aspect: f32::NAN, ..screen.clone() }.sanitized().is_none());

        // Malformed SVG path characters or oversized rect coordinates are sanitized.
        let dirty = ScreenShape {
            v: 1,
            aspect: 0.4495,
            corners: Some(ScreenCorners { tl: 0.9, tr: -0.1, br: 0.08, bl: 0.08 }),
            cutouts: vec![
                ScreenRect { x: -0.1, y: 0.02, w: 0.2, h: 0.04 },
                ScreenRect { x: 0.5, y: 0.5, w: 0.0, h: 0.1 },
            ],
            cutout_path: Some("<script>alert(1)</script>".into()),
        }
        .sanitized()
        .unwrap();
        assert_eq!(dirty.corners, Some(ScreenCorners { tl: 0.5, tr: 0.0, br: 0.08, bl: 0.08 }));
        assert_eq!(dirty.cutouts, vec![ScreenRect { x: 0.0, y: 0.02, w: 0.2, h: 0.04 }]);
        assert!(dirty.cutout_path.is_none());

        // Battery with full_in round-trips and sanitizes when not charging or out of range.
        let b = Battery {
            level: 68,
            charging: true,
            plugged: Some("usb".into()),
            full_in: Some(34),
            cell_bars: None,
            cell_type: None,
            wifi_bars: None,
        };
        let env = Envelope::new(types::EVENT_BATTERY, &b).unwrap();
        let back: Battery = Envelope::from_cbor(&env.to_cbor()).unwrap().body().unwrap();
        assert_eq!(back.sanitized(), b);
        assert_eq!(
            Battery {
                level: 105,
                charging: false,
                plugged: None,
                full_in: Some(25),
                cell_bars: None,
                cell_type: None,
                wifi_bars: None,
            }
            .sanitized(),
            Battery {
                level: 100,
                charging: false,
                plugged: None,
                full_in: None,
                cell_bars: None,
                cell_type: None,
                wifi_bars: None,
            }
        );
    }

    #[test]
    fn battery_signal_strength_round_trips_sanitizes_and_is_backwards_compatible() {
        let valid = Battery {
            level: 85,
            charging: false,
            plugged: None,
            full_in: None,
            cell_bars: Some(4),
            cell_type: Some("5G".into()),
            wifi_bars: Some(3),
        };
        let env = Envelope::new(types::EVENT_BATTERY, &valid).unwrap();
        let back: Battery = Envelope::from_cbor(&env.to_cbor()).unwrap().body().unwrap();
        assert_eq!(back.sanitized(), valid);

        // Clamping bars > 4 to 4, stripping control characters, trimming, truncating cell_type to 8 chars.
        let dirty = Battery {
            level: 50,
            charging: false,
            plugged: None,
            full_in: None,
            cell_bars: Some(9),
            cell_type: Some("  5G\n\tUltraWideband  ".into()),
            wifi_bars: Some(255),
        }
        .sanitized();
        assert_eq!(dirty.cell_bars, Some(4));
        assert_eq!(dirty.cell_type.as_deref(), Some("5GUltraW"));
        assert_eq!(dirty.wifi_bars, Some(4));

        // Blank or control-only cell_type becomes None.
        let empty_type = Battery {
            level: 50,
            charging: false,
            plugged: None,
            full_in: None,
            cell_bars: Some(0),
            cell_type: Some(" \n\0\t ".into()),
            wifi_bars: Some(0),
        }
        .sanitized();
        assert_eq!(empty_type.cell_bars, Some(0));
        assert_eq!(empty_type.cell_type, None);
        assert_eq!(empty_type.wifi_bars, Some(0));

        // Backwards-compatible decoding when signal fields are omitted on the wire.
        #[derive(Serialize)]
        struct LegacyBattery {
            level: u8,
            charging: bool,
        }
        let legacy_env =
            Envelope::new(types::EVENT_BATTERY, &LegacyBattery { level: 77, charging: false }).unwrap();
        let decoded: Battery = Envelope::from_cbor(&legacy_env.to_cbor()).unwrap().body().unwrap();
        assert_eq!(
            decoded,
            Battery {
                level: 77,
                charging: false,
                plugged: None,
                full_in: None,
                cell_bars: None,
                cell_type: None,
                wifi_bars: None,
            }
        );
        // None signal fields are omitted on the wire.
        let none_env = Envelope::new(types::EVENT_BATTERY, &decoded).unwrap();
        let debug_b = format!("{:?}", none_env.b);
        assert!(
            !debug_b.contains("cell_bars")
                && !debug_b.contains("cell_type")
                && !debug_b.contains("wifi_bars")
        );
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
            live: None,
            conversation: None,
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
    fn conversation_notifications_are_sanitized_and_hide_content_in_debug() {
        let mut n = notification();
        n.conversation = Some(NotificationConversation {
            v: 1,
            title: "Design Team".into(),
            group: true,
            avatar: Some(vec![0xff, 0xd8]),
            messages: (0..30)
                .map(|i| NotificationChatMessage {
                    sender: Some(format!("Sender {i}")),
                    text: format!("Message {i}"),
                    time: 1_760_000_000_000 + i,
                    self_sent: i % 2 == 0,
                    avatar: Some(vec![0; if i == 29 { notify_limits::AVATAR_BYTES + 1 } else { 4 }]),
                })
                .collect(),
        });
        let shown = format!("{n:?} {:?}", n.conversation);
        assert!(!shown.contains("Design Team") && !shown.contains("Message 29"), "{shown}");
        let clean = n.sanitized().unwrap();
        let conv = clean.conversation.unwrap();
        assert_eq!(conv.messages.len(), notify_limits::CONVERSATION_MESSAGES);
        assert_eq!(conv.messages[0].text, "Message 5", "keeps newest messages");
        assert_eq!(conv.messages[1].sender, None, "self-sent messages drop sender name");
        assert_eq!(conv.messages.last().unwrap().avatar, None, "oversized avatar is dropped");
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
            files: names
                .iter()
                .map(|n| FileEntry { name: (*n).into(), size: 1, folder: None, blake3: None })
                .collect(),
            recording: false,
            markers: Vec::new(),
            open_on_arrival: false,
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
    fn file_blake3_and_control_are_validated() {
        let good_hex = "a".repeat(64);
        let offer_with_hash = |blake3: Option<String>| FilesOffer {
            id: "abcdefghijklmnop".into(),
            files: vec![FileEntry { name: "a.bin".into(), size: 10, folder: None, blake3 }],
            recording: false,
            markers: Vec::new(),
            open_on_arrival: false,
        };
        assert!(offer_with_hash(None).is_valid());
        assert!(offer_with_hash(Some(good_hex)).is_valid());
        assert!(offer_with_hash(Some("0123456789abcdefABCDEF".repeat(2) + &"0".repeat(20))).is_valid());
        for bad in ["", "abcd", &"a".repeat(63), &"a".repeat(65), &(String::from("g") + &"a".repeat(63))] {
            assert!(!offer_with_hash(Some(bad.into())).is_valid(), "{bad:?}");
        }
        assert!(FilesControl { id: "abcdefghijklmnop".into() }.is_valid());
        assert!(!FilesControl { id: "short".into() }.is_valid());
        assert!(!FilesControl { id: "abcdefghijklmnop!".into() }.is_valid());
    }

    #[test]
    fn folders_are_validated() {
        let offer = |folder: &str| FilesOffer {
            id: "abcdefghijklmnop".into(),
            files: vec![FileEntry {
                name: "a.jpg".into(),
                size: 1,
                folder: Some(folder.into()),
                blake3: None,
            }],
            recording: false,
            markers: Vec::new(),
            open_on_arrival: false,
        };
        for good in ["Trip", "Trip/Day 1", "Trip/Day 1/raw"] {
            assert!(offer(good).is_valid(), "{good:?}");
        }
        for bad in ["", "/Trip", "Trip/", "Trip//x", "Trip/..", "./x", "a\\b", "x\u{0}"] {
            assert!(!offer(bad).is_valid(), "{bad:?}");
        }
        assert!(!offer(&vec!["d"; files::MAX_FOLDER_DEPTH + 1].join("/")).is_valid());
        // Without a folder or blake3, the fields aren't sent at all.
        let plain = FileEntry { name: "a".into(), size: 1, folder: None, blake3: None };
        let env = Envelope::new("x", &plain).unwrap();
        assert!(!format!("{:?}", env.b).contains("folder"));
        assert!(!format!("{:?}", env.b).contains("blake3"));
        assert!(!format!("{:?}", offer("Secret")).contains("Secret"));
    }

    #[test]
    fn recording_offers_and_markers_are_validated() {
        let rec = FilesOffer {
            id: "abcdefghijklmnop".into(),
            files: vec![FileEntry { name: "Recording.m4a".into(), size: 4096, folder: None, blake3: None }],
            recording: true,
            markers: vec![
                RecordingMarker { at_ms: 1200, label: None },
                RecordingMarker { at_ms: 5400, label: Some("Action item".into()) },
            ],
            open_on_arrival: false,
        };
        assert!(rec.is_valid());
        assert!(!format!("{:?}", rec.markers[1]).contains("Action item"));
        let env = Envelope::new(files::OFFER, &rec).unwrap();
        let back: FilesOffer = Envelope::from_cbor(&env.to_cbor()).unwrap().body().unwrap();
        assert_eq!(back, rec);

        // Recordings must be a single file without a folder.
        assert!(
            !FilesOffer {
                files: vec![
                    FileEntry { name: "a.m4a".into(), size: 1, folder: None, blake3: None },
                    FileEntry { name: "b.m4a".into(), size: 1, folder: None, blake3: None },
                ],
                ..rec.clone()
            }
            .is_valid()
        );
        assert!(
            !FilesOffer {
                files: vec![FileEntry {
                    name: "a.m4a".into(),
                    size: 1,
                    folder: Some("dir".into()),
                    blake3: None,
                }],
                ..rec.clone()
            }
            .is_valid()
        );
        // Non-recordings cannot carry markers.
        assert!(!FilesOffer { recording: false, ..rec.clone() }.is_valid());
        // Blank or control-character marker labels are rejected.
        assert!(
            !FilesOffer {
                markers: vec![RecordingMarker { at_ms: 0, label: Some("   ".into()) }],
                ..rec.clone()
            }
            .is_valid()
        );
        assert!(
            !FilesOffer {
                markers: vec![RecordingMarker { at_ms: 0, label: Some("bad\nlabel".into()) }],
                ..rec
            }
            .is_valid()
        );
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
        let send =
            SmsSend { to: vec!["+15550100".into()], body: "On my way".into(), attachments: Vec::new() };
        assert!(send.is_valid());
        assert!(!format!("{send:?}").contains("way"));
        assert!(!SmsSend { to: vec![], ..send.clone() }.is_valid());
        assert!(!SmsSend { body: "  ".into(), ..send.clone() }.is_valid());
        assert!(!SmsSend { body: "x".repeat(sms::MAX_SEND_BYTES + 1), ..send.clone() }.is_valid());
        assert!(!SmsSend { to: vec!["1".into(); sms::MAX_RECIPIENTS + 1], ..send.clone() }.is_valid());
        let with_image = SmsSend {
            body: String::new(),
            attachments: vec![SmsAttachment { mime: "image/png".into(), data: vec![1, 2, 3] }],
            ..send.clone()
        };
        assert!(with_image.is_valid(), "an image attachment can be sent without body text");
        let bad_mime = SmsSend {
            attachments: vec![SmsAttachment { mime: "text/plain".into(), data: vec![1] }],
            ..send.clone()
        };
        assert!(!bad_mime.is_valid());
        let oversized = SmsSend {
            attachments: vec![SmsAttachment {
                mime: "image/jpeg".into(),
                data: vec![0; sms::MAX_ATTACHMENT_BYTES + 1],
            }],
            ..send
        };
        assert!(!oversized.is_valid());
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

    #[test]
    fn call_log_dial_and_contacts_round_trip_and_hide_private_fields() {
        let dial = CallDial { number: "+1 555 0100".into() };
        assert!(dial.is_valid());
        assert!(!format!("{dial:?}").contains("555"));
        assert!(!CallDial { number: "   ".into() }.is_valid());
        assert!(!CallDial { number: "1".repeat(calls::MAX_TEXT_BYTES + 1) }.is_valid());

        let entry = CallLogEntry {
            id: "10".into(),
            number: "+15550100".into(),
            name: Some("Ada Lovelace".into()),
            direction: calls::DIR_MISSED.into(),
            date: 1_760_000_000_000,
            duration: 0,
            photo: Some(vec![0xff, 0xd8]),
        };
        assert!(!format!("{entry:?}").contains("555") && !format!("{entry:?}").contains("Ada"));
        let env = Envelope::new(types::CALL_LOG, &CallLog { entries: vec![entry.clone()] }).unwrap();
        let back: CallLog = Envelope::from_cbor(&env.to_cbor()).unwrap().body().unwrap();
        assert_eq!(back.entries, vec![entry]);

        let contact = Contact {
            id: "42".into(),
            name: "Ada Lovelace".into(),
            numbers: vec![ContactNumber { number: "+15550100".into(), label: Some("mobile".into()) }],
            starred: true,
            photo: Some(vec![0xff, 0xd8]),
        };
        assert!(!format!("{contact:?}").contains("Ada") && !format!("{contact:?}").contains("555"));
        let get = ContactsListGet { query: Some("Ada".into()), offset: 0, limit: 50 };
        assert!(!format!("{get:?}").contains("Ada"));
        let env =
            Envelope::new(types::CONTACTS_LIST, &ContactsList { contacts: vec![contact.clone()] }).unwrap();
        let back: ContactsList = Envelope::from_cbor(&env.to_cbor()).unwrap().body().unwrap();
        assert_eq!(back.contacts, vec![contact]);
    }

    #[test]
    fn gallery_messages_round_trip_and_hide_private_fields() {
        let album = PhotoAlbum {
            id: "b1".into(),
            name: "Private Vacation".into(),
            count: 12,
            cover: Some("p1".into()),
        };
        assert!(!format!("{album:?}").contains("Vacation"));
        let env = Envelope::new(types::PHOTOS_ALBUMS_LIST, &PhotoAlbumsList { albums: vec![album.clone()] })
            .unwrap();
        let back: PhotoAlbumsList = Envelope::from_cbor(&env.to_cbor()).unwrap().body().unwrap();
        assert_eq!(back.albums, vec![album]);

        let item = PhotoItem {
            id: "p1".into(),
            name: "secret_photo.jpg".into(),
            date: 1_790_000_000_000,
            size: 2048,
            width: 1920,
            height: 1080,
            duration: Some(5000),
            album: Some("b1".into()),
        };
        assert!(!format!("{item:?}").contains("secret_photo"));
        let env = Envelope::new(types::PHOTOS_ITEMS, &PhotoItems { items: vec![item.clone()] }).unwrap();
        let back: PhotoItems = Envelope::from_cbor(&env.to_cbor()).unwrap().body().unwrap();
        assert_eq!(back.items, vec![item]);

        let thumb = PhotoThumb { id: "p1".into(), data: vec![0xff, 0xd8, 0xff] };
        let env = Envelope::new(types::PHOTOS_THUMBS_LIST, &PhotoThumbsList { thumbs: vec![thumb.clone()] })
            .unwrap();
        let back: PhotoThumbsList = Envelope::from_cbor(&env.to_cbor()).unwrap().body().unwrap();
        assert_eq!(back.thumbs, vec![thumb]);

        let single = PhotoGet { id: "p1".into(), ids: Vec::new() };
        assert_eq!(single.requested_ids(), vec!["p1".to_owned()]);
        let multi = PhotoGet { id: String::new(), ids: vec!["p1".into(), "p2".into()] };
        assert_eq!(multi.requested_ids(), vec!["p1".to_owned(), "p2".to_owned()]);
    }

    #[test]
    fn remote_input_validates_and_round_trips() {
        // Datagram round-trip for move, scroll, and laser.
        let mv = RemoteInput::Move { dx: 12.5, dy: -8.0 };
        assert!(mv.is_valid() && mv.is_datagram_allowed() && !mv.is_control_allowed());
        let dgram = mv.to_datagram();
        assert_eq!(dgram[0], remote::DATAGRAM_TAG);
        assert_eq!(RemoteInput::from_datagram(&dgram).unwrap(), mv);

        let laser = RemoteInput::Laser { on: true, x: 0.25, y: 0.75 };
        assert_eq!(RemoteInput::from_datagram(&laser.to_datagram()).unwrap(), laser);
        let laser_off = RemoteInput::Laser { on: false, x: 0.0, y: 0.0 };
        assert_eq!(RemoteInput::from_datagram(&laser_off.to_datagram()).unwrap(), laser_off);

        // Button or key in a datagram is rejected.
        let click = RemoteInput::Button { button: MouseButton::Left, action: ButtonAction::Click };
        assert!(click.is_valid() && click.is_control_allowed() && !click.is_datagram_allowed());
        assert!(RemoteInput::from_datagram(&click.to_datagram()).is_err());

        // Bounds checks.
        assert!(!RemoteInput::Move { dx: 5000.0, dy: 0.0 }.is_valid());
        assert!(!RemoteInput::Move { dx: f32::NAN, dy: 0.0 }.is_valid());
        assert!(!RemoteInput::Scroll { dx: 0.0, dy: 250.0 }.is_valid());
        assert!(!RemoteInput::Laser { on: true, x: 1.2, y: 0.5 }.is_valid());
        assert!(!RemoteInput::Laser { on: true, x: -0.1, y: 0.5 }.is_valid());

        // Text and keys.
        let text = RemoteInput::Text { text: "Hello 🐝 नमस्ते".into() };
        assert!(text.is_valid());
        assert!(!format!("{text:?}").contains("Hello"));
        assert!(!RemoteInput::Text { text: String::new() }.is_valid());
        assert!(!RemoteInput::Text { text: "bad\nnewline".into() }.is_valid());
        assert!(!RemoteInput::Text { text: "x".repeat(remote::MAX_TEXT_BYTES + 1) }.is_valid());

        for k in remote_keys::NAMED {
            assert!(RemoteInput::Key { key: (*k).into(), mods: vec![] }.is_valid(), "{k}");
        }
        for s in remote_keys::SHORTCUTS {
            assert!(RemoteInput::Key { key: (*s).into(), mods: vec![] }.is_valid(), "{s}");
            assert!(!RemoteInput::Key { key: (*s).into(), mods: vec![KeyMod::Ctrl] }.is_valid(), "{s}");
        }
        assert!(RemoteInput::Key { key: "c".into(), mods: vec![KeyMod::Ctrl] }.is_valid());
        assert!(RemoteInput::Key { key: "z".into(), mods: vec![KeyMod::Ctrl, KeyMod::Shift] }.is_valid());
        assert!(
            !RemoteInput::Key { key: "c".into(), mods: vec![] }.is_valid(),
            "unmodified letter uses Text"
        );
        assert!(!RemoteInput::Key { key: "c".into(), mods: vec![KeyMod::Ctrl, KeyMod::Ctrl] }.is_valid());
        assert!(!RemoteInput::Key { key: "f13".into(), mods: vec![] }.is_valid());
        assert!(!RemoteInput::Key { key: "power".into(), mods: vec![] }.is_valid());

        // Slide round-trip on the control stream.
        let slide = RemoteInput::Slide { action: SlideAction::Next };
        let env = Envelope::new(types::REMOTE_INPUT, &slide).unwrap();
        assert_eq!(env.body::<RemoteInput>().unwrap(), slide);
    }

    #[test]
    fn phone_toggles_validate_and_round_trip() {
        let state = PhoneToggles {
            dnd: false,
            ringer: ringer_modes::RING.into(),
            flashlight: Some(true),
            volume: 65,
            brightness: 50,
            wifi: true,
            bluetooth: true,
        };
        assert!(state.is_valid());
        let env = Envelope::new(types::PHONE_TOGGLES, &state).unwrap();
        let back: PhoneToggles = Envelope::from_cbor(&env.to_cbor()).unwrap().body().unwrap();
        assert_eq!(back, state);

        // Without flash hardware, `flashlight` is omitted on the wire.
        let no_flash = PhoneToggles { flashlight: None, ..state.clone() };
        let env = Envelope::new(types::PHONE_TOGGLES, &no_flash).unwrap();
        assert!(!format!("{:?}", env.b).contains("flashlight"));
        let back: PhoneToggles = Envelope::from_cbor(&env.to_cbor()).unwrap().body().unwrap();
        assert_eq!(back, no_flash);

        // Invalid ringer mode is rejected; out-of-range volume/brightness clamp in sanitized().
        assert!(!PhoneToggles { ringer: "loud".into(), ..state.clone() }.is_valid());
        assert_eq!(PhoneToggles { ringer: "loud".into(), ..state.clone() }.sanitized(), None);
        assert!(!PhoneToggles { volume: 120, ..state.clone() }.is_valid());
        assert_eq!(PhoneToggles { volume: 120, brightness: 200, ..state }.sanitized().unwrap().volume, 100);

        // `phone.toggle.set` validation for every toggle ID and value kind.
        for id in [toggle_ids::DND, toggle_ids::FLASHLIGHT, toggle_ids::WIFI, toggle_ids::BLUETOOTH] {
            let set = PhoneToggleSet { id: id.into(), value: PhoneToggleValue::Bool(true) };
            assert!(set.is_valid(), "{id}");
            let env = Envelope::new(types::PHONE_TOGGLE_SET, &set).unwrap();
            assert_eq!(Envelope::from_cbor(&env.to_cbor()).unwrap().body::<PhoneToggleSet>().unwrap(), set);
            assert!(!PhoneToggleSet { id: id.into(), value: PhoneToggleValue::Level(1) }.is_valid());
            assert!(!PhoneToggleSet { id: id.into(), value: PhoneToggleValue::Mode("on".into()) }.is_valid());
        }
        for id in [toggle_ids::VOLUME, toggle_ids::BRIGHTNESS] {
            for level in [0, 50, 100] {
                let set = PhoneToggleSet { id: id.into(), value: PhoneToggleValue::Level(level) };
                assert!(set.is_valid());
                let env = Envelope::new(types::PHONE_TOGGLE_SET, &set).unwrap();
                assert_eq!(
                    Envelope::from_cbor(&env.to_cbor()).unwrap().body::<PhoneToggleSet>().unwrap(),
                    set
                );
            }
            assert!(!PhoneToggleSet { id: id.into(), value: PhoneToggleValue::Level(101) }.is_valid());
            assert!(!PhoneToggleSet { id: id.into(), value: PhoneToggleValue::Bool(true) }.is_valid());
        }
        for mode in [ringer_modes::RING, ringer_modes::VIBRATE, ringer_modes::SILENT] {
            let set =
                PhoneToggleSet { id: toggle_ids::RINGER.into(), value: PhoneToggleValue::Mode(mode.into()) };
            assert!(set.is_valid());
            let env = Envelope::new(types::PHONE_TOGGLE_SET, &set).unwrap();
            assert_eq!(Envelope::from_cbor(&env.to_cbor()).unwrap().body::<PhoneToggleSet>().unwrap(), set);
        }
        assert!(
            !PhoneToggleSet { id: toggle_ids::RINGER.into(), value: PhoneToggleValue::Mode("mute".into()) }
                .is_valid()
        );
        assert!(!PhoneToggleSet { id: "airplane".into(), value: PhoneToggleValue::Bool(true) }.is_valid());
    }

    #[test]
    fn magic_packet_has_sync_header_and_sixteen_mac_copies() {
        let mac = parse_mac("00-00-5E-00-53-01").unwrap();
        assert_eq!(mac, [0x00, 0x00, 0x5e, 0x00, 0x53, 0x01]);
        assert_eq!(format_mac(&mac), "00:00:5e:00:53:01");
        let pkt = magic_packet(&mac);
        assert_eq!(pkt.len(), 102);
        assert_eq!(&pkt[..6], &[0xff; 6]);
        for i in 0..16 {
            assert_eq!(&pkt[6 + i * 6..12 + i * 6], &mac);
        }
    }

    #[test]
    fn ipv4_broadcast_from_ip_and_prefix() {
        use std::net::Ipv4Addr;
        assert_eq!(ipv4_broadcast(Ipv4Addr::new(192, 168, 1, 42), 24), Some(Ipv4Addr::new(192, 168, 1, 255)));
        assert_eq!(ipv4_broadcast(Ipv4Addr::new(10, 0, 12, 5), 16), Some(Ipv4Addr::new(10, 0, 255, 255)));
        assert_eq!(ipv4_broadcast(Ipv4Addr::new(172, 16, 5, 10), 20), Some(Ipv4Addr::new(172, 16, 15, 255)));
        assert_eq!(ipv4_broadcast(Ipv4Addr::new(192, 168, 10, 1), 30), Some(Ipv4Addr::new(192, 168, 10, 3)));
        // Invalid prefix lengths or non-routable LAN addresses.
        assert_eq!(ipv4_broadcast(Ipv4Addr::new(192, 168, 1, 42), 0), None);
        assert_eq!(ipv4_broadcast(Ipv4Addr::new(192, 168, 1, 42), 31), None);
        assert_eq!(ipv4_broadcast(Ipv4Addr::new(192, 168, 1, 42), 32), None);
        assert_eq!(ipv4_broadcast(Ipv4Addr::UNSPECIFIED, 24), None);
        assert_eq!(ipv4_broadcast(Ipv4Addr::LOCALHOST, 8), None);
        assert_eq!(ipv4_broadcast(Ipv4Addr::new(169, 254, 10, 20), 16), None);
        assert_eq!(ipv4_broadcast(Ipv4Addr::new(224, 0, 0, 1), 24), None);
        assert_eq!(ipv4_broadcast(Ipv4Addr::BROADCAST, 24), None);
    }

    #[test]
    fn wake_info_validates_sanitizes_and_round_trips() {
        let info = PcWakeInfo {
            macs: vec!["00:00:5e:00:53:01".into(), "00:00:5e:00:53:02".into()],
            broadcasts: vec!["192.168.1.255".into()],
        };
        assert!(info.is_valid());
        let debug = format!("{info:?}");
        assert!(!debug.contains("38:a7") && !debug.contains("192.168"), "{debug}");

        let env = Envelope::new(types::PC_WAKE_INFO, &info).unwrap();
        let back: PcWakeInfo = Envelope::from_cbor(&env.to_cbor()).unwrap().body().unwrap();
        assert_eq!(back, info);

        let stored = info.to_storage_string();
        assert_eq!(PcWakeInfo::from_storage_str(&stored), Some(info.clone()));
        assert_eq!(PcWakeInfo::from_storage_str(""), None);

        // Empty wake_info (sent when pc_actions is turned off) is valid on the wire.
        let empty = PcWakeInfo::default();
        assert!(empty.is_valid());
        assert_eq!(empty.to_storage_string(), "");

        // Invalid MACs and broadcast addresses are rejected by is_valid and stripped by sanitized.
        for bad_mac in [
            "",
            "00:00:00:00:00:00",
            "ff:ff:ff:ff:ff:ff",
            "FF-FF-FF-FF-FF-FF",
            "00:00:5e:00:53",
            "00:00:5e:00:53:01:00",
            "00:00:5e:00:53:zz",
        ] {
            assert!(!PcWakeInfo { macs: vec![bad_mac.into()], broadcasts: vec![] }.is_valid(), "{bad_mac}");
        }
        for bad_ip in ["", "0.0.0.0", "127.0.0.1", "169.254.1.255", "224.0.0.251", "not-an-ip"] {
            assert!(
                !PcWakeInfo { macs: vec!["00:00:5e:00:53:01".into()], broadcasts: vec![bad_ip.into()] }
                    .is_valid(),
                "{bad_ip}"
            );
        }

        let messy = PcWakeInfo {
            macs: vec!["00-00-5E-00-53-01".into(), "00:00:5e:00:53:01".into(), "00:00:00:00:00:00".into()],
            broadcasts: vec!["192.168.1.255".into(), "192.168.1.255".into(), "127.0.0.1".into()],
        }
        .sanitized();
        assert_eq!(messy.macs, vec!["00:00:5e:00:53:01"]);
        assert_eq!(messy.broadcasts, vec!["192.168.1.255"]);
    }

    #[test]
    fn deck_layout_state_and_press_validate_sanitize_and_round_trip() {
        let layout = DeckLayout::default_layout();
        assert!(layout.is_valid());
        assert_eq!(layout.pages.len(), 1);
        assert!(layout.tile("play_pause").is_some());
        assert!(layout.tile("mic_mute").is_some());
        assert!(layout.tile("lock_pc").is_some());
        assert!(layout.tile("screenshot").is_some());
        assert!(layout.tile("show_desktop").is_some());

        // Debug output hides user labels and page names.
        let dbg = format!("{layout:?}");
        assert!(!dbg.contains("Play / pause") && !dbg.contains("Main"), "{dbg}");

        let env = Envelope::new(types::DECK_LAYOUT, &layout).unwrap();
        let back: DeckLayout = Envelope::from_cbor(&env.to_cbor()).unwrap().body().unwrap();
        assert_eq!(back, layout);

        // Duplicate tile IDs across pages are invalid and deduplicated by sanitized().
        let dup_tile = layout.pages[0].tiles[0].clone();
        let with_dup = DeckLayout {
            pages: vec![
                layout.pages[0].clone(),
                DeckPage { id: "page2".into(), name: "Second".into(), tiles: vec![dup_tile] },
            ],
        };
        assert!(!with_dup.is_valid());
        let clean = with_dup.sanitized().unwrap();
        assert!(clean.is_valid());
        assert!(clean.pages[1].tiles.is_empty());

        // Unknown icon/color fall back to the action kind's default in sanitized().
        let fallback = DeckTile {
            id: "cmd1".into(),
            label: "  Build\nnow  ".into(),
            icon: "unknown_icon".into(),
            color: "unknown_color".into(),
            kind: deck_kinds::RUN_COMMAND.into(),
        }
        .sanitized()
        .unwrap();
        assert_eq!(fallback.label, "Buildnow");
        assert_eq!(fallback.icon, deck_icons::TERMINAL);
        assert_eq!(fallback.color, deck_colors::CORAL);

        // Empty pages list is rejected.
        assert!(!DeckLayout { pages: vec![] }.is_valid());
        assert_eq!(DeckLayout { pages: vec![] }.sanitized(), None);

        // Live state round-trip and status formatting.
        let state = DeckState {
            playing: true,
            volume: 72,
            muted: false,
            mic_muted: Some(true),
            output_devices: vec![
                AudioOutputDevice {
                    id: "out:speakers".into(),
                    name: "Speakers (Realtek Audio)".into(),
                    is_default: true,
                },
                AudioOutputDevice {
                    id: "out:headphones".into(),
                    name: "Headphones".into(),
                    is_default: false,
                },
            ],
        };
        assert!(state.is_valid());
        assert_eq!(state.default_output_name(), Some("Speakers (Realtek Audio)"));
        assert!(!format!("{:?}", state.output_devices[0]).contains("Realtek"));
        assert_eq!(state.tile_status(deck_kinds::MEDIA_PLAY_PAUSE).as_deref(), Some("Playing"));
        assert_eq!(state.tile_status(deck_kinds::VOLUME_UP).as_deref(), Some("72%"));
        assert_eq!(state.tile_status(deck_kinds::MIC_MUTE).as_deref(), Some("Muted"));
        assert_eq!(state.tile_status(deck_kinds::LOCK_PC), None);
        let env = Envelope::new(types::DECK_STATE, &state).unwrap();
        let back: DeckState = Envelope::from_cbor(&env.to_cbor()).unwrap().body().unwrap();
        assert_eq!(back, state);

        // PC audio set validation and round-trip.
        let audio_set = PcAudioSet { volume: Some(65), muted: Some(false) };
        assert!(audio_set.is_valid());
        let env = Envelope::new(types::PC_AUDIO_SET, &audio_set).unwrap();
        assert_eq!(Envelope::from_cbor(&env.to_cbor()).unwrap().body::<PcAudioSet>().unwrap(), audio_set);
        assert!(!PcAudioSet { volume: None, muted: None }.is_valid());
        assert!(!PcAudioSet { volume: Some(101), muted: None }.is_valid());

        // Press validation.
        let press = DeckPress { tile: "play_pause".into() };
        assert!(press.is_valid());
        let env = Envelope::new(types::DECK_PRESS, &press).unwrap();
        assert_eq!(Envelope::from_cbor(&env.to_cbor()).unwrap().body::<DeckPress>().unwrap(), press);
        assert!(!DeckPress { tile: String::new() }.is_valid());
        assert!(!DeckPress { tile: "bad tile!".into() }.is_valid());
    }

    #[test]
    fn storage_paths_and_messages_validate_and_round_trip() {
        assert!(is_valid_storage_dir_path(""));
        assert!(!is_valid_storage_path(""));
        for good in ["DCIM", "DCIM/Camera/IMG_001.jpg", "Download/Report (2026) ✓.pdf", ".hidden/file.txt"]
        {
            assert!(is_valid_storage_path(good), "{good}");
            assert!(is_valid_storage_dir_path(good), "{good}");
        }
        for bad in [
            "/",
            "/DCIM",
            "DCIM/",
            "DCIM//Camera",
            ".",
            "..",
            "../etc/passwd",
            "DCIM/../secret",
            "DCIM/.",
            "DCIM\\Camera",
            "C:\\Windows",
            "bad\0name",
            "bad\nname",
        ] {
            assert!(!is_valid_storage_path(bad), "{bad}");
            assert!(!is_valid_storage_dir_path(bad), "{bad}");
        }
        let deep = vec!["a"; storage::MAX_SEGMENTS + 1].join("/");
        assert!(!is_valid_storage_path(&deep));
        let long_seg = "x".repeat(storage::MAX_NAME_BYTES + 1);
        assert!(!is_valid_storage_path(&long_seg));

        let entry = StorageEntry {
            name: "photo.jpg".into(),
            size: 123_456,
            modified: 1_750_000_000_000,
            is_dir: false,
        };
        assert!(entry.is_valid());
        assert!(!format!("{entry:?}").contains("photo.jpg"));
        let dir_entry =
            StorageEntry { name: "DCIM".into(), size: 999, modified: -5, is_dir: true }.sanitized().unwrap();
        assert_eq!(dir_entry.size, 0);
        assert_eq!(dir_entry.modified, 0);

        let entries = StorageEntries { entries: vec![entry, dir_entry] };
        let env = Envelope::new(types::STORAGE_ENTRIES, &entries).unwrap();
        let back: StorageEntries = Envelope::from_cbor(&env.to_cbor()).unwrap().body().unwrap();
        assert_eq!(back, entries);

        let read = StorageRead { path: "DCIM/photo.jpg".into(), offset: 4096, length: Some(65536) };
        assert!(read.is_valid());
        assert!(!format!("{read:?}").contains("photo.jpg"));
        let env = Envelope::new(types::STORAGE_READ, &read).unwrap();
        assert_eq!(Envelope::from_cbor(&env.to_cbor()).unwrap().body::<StorageRead>().unwrap(), read);

        let write = StorageWriteOffer {
            id: "up-1".into(),
            path: "Download/notes.txt".into(),
            size: 512,
            modified: Some(1_750_000_000_000),
        };
        assert!(write.is_valid());
        assert!(!format!("{write:?}").contains("notes.txt"));
        let env = Envelope::new(types::STORAGE_WRITE, &write).unwrap();
        assert_eq!(Envelope::from_cbor(&env.to_cbor()).unwrap().body::<StorageWriteOffer>().unwrap(), write);

        let rename = StorageRename { from: "a.txt".into(), to: "b.txt".into() };
        assert!(rename.is_valid());
        assert!(!StorageRename { from: "a.txt".into(), to: "a.txt".into() }.is_valid());
    }

    #[test]
    fn play_on_phone_messages_validate() {
        let offer = MediaWatch { player: "Chrome".into(), title: Some("A video".into()), file: None };
        assert!(offer.is_valid());
        assert!(!MediaWatch { player: String::new(), title: None, file: None }.is_valid());
        assert!(!MediaWatch { player: "p".repeat(257), title: None, file: None }.is_valid());
        assert!(!MediaWatch { player: "Chrome".into(), title: Some("t".repeat(257)), file: None }.is_valid());

        // A file offer: a plain name and a size; it goes over the wire as is.
        let file = WatchFile { name: "Holiday.mp4".into(), size: 1_000, position: 61_500 };
        let with_file = MediaWatch { player: "vlc.exe".into(), title: None, file: Some(file.clone()) };
        assert!(with_file.is_valid());
        let env = Envelope::new(types::MEDIA_WATCH, &with_file).unwrap();
        assert_eq!(Envelope::from_cbor(&env.to_cbor()).unwrap().body::<MediaWatch>().unwrap(), with_file);
        for bad in [
            WatchFile { name: String::new(), ..file.clone() },
            WatchFile { name: "a/b.mp4".into(), ..file.clone() },
            WatchFile { name: "a\\b.mp4".into(), ..file.clone() },
            WatchFile { size: 0, ..file.clone() },
        ] {
            assert!(!MediaWatch { file: Some(bad.clone()), ..with_file.clone() }.is_valid(), "{bad:?}");
        }

        let screen = MirrorStart {
            max_size: 1280,
            fps: 30,
            bitrate: 4_000_000,
            audio: true,
            session: mirror::SCREEN,
            app: None,
            stay_awake: false,
            screen_off: false,
            watch: Some("Chrome".into()),
        };
        assert!(screen.is_valid());
        // Watching is of the PC's screen session only, never with an app.
        assert!(!MirrorStart { session: 3, ..screen.clone() }.is_valid());
        assert!(
            !MirrorStart { session: 3, app: Some("com.example.app".into()), ..screen.clone() }.is_valid()
        );
        assert!(!MirrorStart { watch: Some(String::new()), ..screen.clone() }.is_valid());
        // Older PCs and phones leave it out.
        let plain = MirrorStart { watch: None, ..screen };
        let mut bytes = Vec::new();
        ciborium::into_writer(&plain, &mut bytes).unwrap();
        let back: MirrorStart = ciborium::from_reader(bytes.as_slice()).unwrap();
        assert_eq!(back, plain);
    }

    #[test]
    fn webcam_messages_validate_and_round_trip() {
        let start = WebcamStart::default();
        assert!(start.is_valid());
        let env = Envelope::new(types::WEBCAM_START, &start).unwrap();
        let back: WebcamStart = Envelope::from_cbor(&env.to_cbor()).unwrap().body().unwrap();
        assert_eq!(back, start);

        let front_1080 = WebcamStart {
            camera: webcam::CAMERA_FRONT.into(),
            width: 1920,
            height: 1080,
            fps: 30,
            bitrate: 8_000_000,
        };
        assert!(front_1080.is_valid());
        assert!(!WebcamStart { camera: "telephoto".into(), ..start.clone() }.is_valid());
        assert!(!WebcamStart { width: 100, ..start.clone() }.is_valid());
        assert!(!WebcamStart { fps: 0, ..start }.is_valid());

        let cfg = WebcamConfig {
            codec: webcam::H264.into(),
            width: 1280,
            height: 720,
            camera: webcam::CAMERA_BACK.into(),
            fps: 30,
            max_height: 1080,
        };
        assert!(cfg.is_valid());
        assert_eq!(WebcamConfig::from_cbor(&cfg.to_cbor()).unwrap(), cfg);
        let hevc = WebcamConfig { codec: webcam::H265.into(), ..cfg.clone() };
        assert!(hevc.is_valid());
        assert_eq!(WebcamConfig::from_cbor(&hevc.to_cbor()).unwrap(), hevc);
        assert!(!WebcamConfig { codec: "vp8".into(), ..cfg.clone() }.is_valid());
        assert!(!WebcamConfig { width: 0, ..cfg }.is_valid());

        // A phone before 0.1.5 doesn't say what its camera can do: 4K stays offered.
        #[derive(Serialize)]
        struct Before {
            codec: &'static str,
            width: u32,
            height: u32,
            camera: &'static str,
            fps: u32,
        }
        let mut old = Vec::new();
        let before =
            Before { codec: webcam::H264, width: 1920, height: 1080, camera: webcam::CAMERA_FRONT, fps: 30 };
        ciborium::into_writer(&before, &mut old).unwrap();
        assert_eq!(WebcamConfig::from_cbor(&old).unwrap().max_height, 2160);
    }

    #[test]
    fn live_notification_round_trips_and_sanitizes() {
        let live = NotificationLive {
            v: 1,
            progress: Some(72),
            max: Some(100),
            indeterminate: false,
            chip: Some("4 min".into()),
            segments: vec![
                LiveSegment { length: 50, color: Some(0xFF4C_AF50) },
                LiveSegment { length: 50, color: None },
            ],
            points: vec![LivePoint { position: 50, color: Some(0xFFFF_C107) }],
            chronometer: true,
            countdown: true,
        };
        let mut n = notification();
        n.live = Some(live.clone());
        assert!(!format!("{live:?}").contains("4 min"));
        let env = Envelope::new(types::NOTIFY_POSTED, &n).unwrap();
        let back: Notification = Envelope::from_cbor(&env.to_cbor()).unwrap().body().unwrap();
        assert_eq!(back.sanitized(), Some(n));

        // Unknown schema version or empty live metadata drops cleanly.
        assert!(NotificationLive { v: 2, ..live.clone() }.sanitized().is_none());
        assert!(NotificationLive::default().sanitized().is_none());

        // Indeterminate clears determinate progress/max; progress > max clamps to max.
        let ind = NotificationLive {
            v: 1,
            progress: Some(40),
            max: Some(100),
            indeterminate: true,
            ..Default::default()
        }
        .sanitized()
        .unwrap();
        assert_eq!(ind.progress, None);
        assert_eq!(ind.max, None);
        assert!(ind.indeterminate);

        let clamped = NotificationLive {
            v: 1,
            progress: Some(150),
            max: Some(100),
            chip: Some("  Arriving\nsoon  ".into()),
            segments: vec![LiveSegment { length: 0, color: None }, LiveSegment { length: 100, color: None }],
            countdown: true,
            chronometer: false,
            ..Default::default()
        }
        .sanitized()
        .unwrap();
        assert_eq!(clamped.progress, Some(100));
        assert_eq!(clamped.chip.as_deref(), Some("Arrivingsoon"));
        assert_eq!(clamped.segments, vec![LiveSegment { length: 100, color: None }]);
        assert!(!clamped.countdown);
    }

    #[test]
    fn task_notify_validates_round_trips_and_formats_duration() {
        let running = TaskNotify {
            id: "task-1234".into(),
            title: "cargo build --release".into(),
            active: true,
            elapsed_ms: 38_400,
            exit_code: None,
        };
        assert!(running.is_valid());
        assert!(!format!("{running:?}").contains("cargo build"));
        let env = Envelope::new(types::TASK_NOTIFY, &running).unwrap();
        let back: TaskNotify = Envelope::from_cbor(&env.to_cbor()).unwrap().body().unwrap();
        assert_eq!(back, running);

        let done = TaskNotify {
            id: "task-1234".into(),
            title: "cargo build --release".into(),
            active: false,
            elapsed_ms: 252_000,
            exit_code: Some(0),
        };
        assert!(done.is_valid());
        assert_eq!(done.clone().sanitized(), Some(done));

        // Active tasks cannot carry an exit code.
        let dirty = TaskNotify { exit_code: Some(1), ..running.clone() };
        assert!(!dirty.is_valid());
        assert_eq!(dirty.sanitized().unwrap().exit_code, None);

        // Blank title or invalid ID is rejected.
        assert!(!TaskNotify { id: "".into(), ..running.clone() }.is_valid());
        assert!(!TaskNotify { title: "   ".into(), ..running }.is_valid());

        assert_eq!(TaskNotify::format_took(38_000), "Took 38s");
        assert_eq!(TaskNotify::format_took(252_000), "Took 4m 12s");
        assert_eq!(TaskNotify::format_took(3_840_000), "Took 1h 04m");
    }

    #[test]
    fn continuity_camera_messages_round_trip() {
        let req = CameraCaptureRequest { request_id: "req-1".into(), mode: CameraCaptureMode::Scan };
        let env = Envelope::new(types::CAMERA_CAPTURE_REQUEST, &req).unwrap();
        let back: CameraCaptureRequest = Envelope::from_cbor(&env.to_cbor()).unwrap().body().unwrap();
        assert_eq!(back, req);

        let ok = CameraCaptureOk { request_id: "req-1".into() };
        let env_ok = Envelope::new(types::CAMERA_CAPTURE_OK, &ok).unwrap();
        let back_ok: CameraCaptureOk = Envelope::from_cbor(&env_ok.to_cbor()).unwrap().body().unwrap();
        assert_eq!(back_ok, ok);

        let cancel =
            CameraCaptureCancel { request_id: "req-1".into(), reason: Some("user dismissed".into()) };
        let env_cancel = Envelope::new(types::CAMERA_CAPTURE_CANCEL, &cancel).unwrap();
        let back_cancel: CameraCaptureCancel =
            Envelope::from_cbor(&env_cancel.to_cbor()).unwrap().body().unwrap();
        assert_eq!(back_cancel, cancel);

        let meta = CameraCaptureResultMeta {
            request_id: "req-1".into(),
            mode: CameraCaptureMode::Photo,
            file_name: "IMG_20260409.jpg".into(),
            mime: "image/jpeg".into(),
            size: 12345,
            width: 1920,
            height: 1080,
        };
        let env_meta = Envelope::new(continuity_camera::RESULT_META, &meta).unwrap();
        let back_meta: CameraCaptureResultMeta =
            Envelope::from_cbor(&env_meta.to_cbor()).unwrap().body().unwrap();
        assert_eq!(back_meta, meta);
    }

    #[test]
    fn clip_set_rich_text_validates_and_round_trips() {
        let plain = ClipSet { text: "Hello".into(), html: None, rtf: None };
        assert!(plain.is_valid());
        let env_plain = Envelope::new(types::CLIP_SET, &plain).unwrap();
        assert!(!format!("{:?}", env_plain.b).contains("html"));
        assert!(!format!("{:?}", env_plain.b).contains("rtf"));
        let back_plain: ClipSet = Envelope::from_cbor(&env_plain.to_cbor()).unwrap().body().unwrap();
        assert_eq!(back_plain, plain);

        let rich = ClipSet {
            text: "Hello world".into(),
            html: Some("<b>Hello</b> world".into()),
            rtf: Some(r"{\rtf1\ansi\b Hello\b0  world}".into()),
        };
        assert!(rich.is_valid());
        assert!(!format!("{rich:?}").contains("Hello"));
        let env_rich = Envelope::new(types::CLIP_SET, &rich).unwrap();
        let back_rich: ClipSet = Envelope::from_cbor(&env_rich.to_cbor()).unwrap().body().unwrap();
        assert_eq!(back_rich, rich);

        let oversized =
            ClipSet { text: "a".repeat(CLIP_MAX_BYTES), html: Some("<b>a</b>".into()), rtf: None };
        assert!(!oversized.is_valid());
    }

    #[test]
    fn mirror_input_gamepad_round_trips_and_validates() {
        let gp = MirrorInput::Gamepad {
            buttons: 0x1001,
            lx: -12345,
            ly: 23456,
            rx: 0,
            ry: -32768,
            lt: 128,
            rt: 255,
        };
        assert!(gp.is_valid());
        assert_eq!(format!("{gp:?}"), "Gamepad(0x1001)");
        let env = mirror_input_envelope(&gp, mirror::SCREEN).unwrap();
        let back: MirrorInput = Envelope::from_cbor(&env.to_cbor()).unwrap().body().unwrap();
        assert_eq!(back, gp);

        let env_app = mirror_input_envelope(&gp, 2).unwrap();
        let back_app: MirrorInput = Envelope::from_cbor(&env_app.to_cbor()).unwrap().body().unwrap();
        assert_eq!(back_app, gp);
    }

    #[test]
    fn fuzz_all_message_bodies_cbor_and_json_10k() {
        fn next_u64(seed: &mut u64) -> u64 {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 7;
            *seed ^= *seed << 17;
            *seed
        }

        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut buf = Vec::with_capacity(256);
        for i in 0..10_000usize {
            let len = (next_u64(&mut seed) as usize) % 192;
            buf.clear();
            for _ in 0..len {
                buf.push((next_u64(&mut seed) & 0xFF) as u8);
            }
            // Also mix in structured CBOR map prefixes so deeper serde paths run.
            if i % 2 == 1 && !buf.is_empty() {
                buf[0] = 0xA0 | ((buf[0] & 0x0F).min(8));
            }

            if let Ok(env) = Envelope::from_cbor(&buf) {
                let _ = format!("{env:?}");
                let _ = env.body::<Hello>().map(|m| format!("{m:?}"));
                let _ = env.body::<HelloUpdate>().map(|m| format!("{m:?}"));
                let _ = env.body::<DeviceInfo>().map(|m| m.sanitized());
                let _ = env.body::<ScreenShape>().map(|m| m.sanitized());
                let _ = env.body::<Battery>().map(|m| m.sanitized());
                let _ = env.body::<Notification>().map(|m| m.sanitized());
                let _ = env.body::<NotificationLive>().map(|m| m.sanitized());
                let _ = env.body::<ClipSet>().map(|m| (m.is_valid(), format!("{m:?}")));
                let _ = env.body::<MirrorInput>().map(|m| (m.is_valid(), format!("{m:?}")));
                let _ = env.body::<TaskNotify>().map(|m| (m.is_valid(), m.sanitized()));
            }

            // Directly fuzz CBOR deserialization across message body types.
            macro_rules! try_decode {
                ($($ty:ty),+ $(,)?) => {{
                    $(
                        if let Ok(v) = ciborium::from_reader::<$ty, _>(buf.as_slice()) {
                            let _ = format!("{v:?}");
                        }
                    )+
                }};
            }

            match i % 6 {
                0 => try_decode!(Hello, HelloUpdate, DeviceInfo, ScreenShape, Ping, Battery, Ring, ErrorBody),
                1 => try_decode!(StreamHeader, PairRequest, PairAccept, PairCommit, PairNonce, PairReveal),
                2 => try_decode!(
                    Notification,
                    NotificationLive,
                    NotifySnapshot,
                    NotifyKey,
                    NotifyAction,
                    ClipSet
                ),
                3 => try_decode!(MediaPlayer, MediaState, MediaCommand, PcPower, PcWakeInfo),
                4 => try_decode!(
                    MirrorStart,
                    MirrorPower,
                    MirrorSession,
                    MirrorResize,
                    PhoneApp,
                    PhoneApps,
                    MirrorInput
                ),
                _ => try_decode!(
                    MirrorConfig,
                    MirrorAudioConfig,
                    WebcamStart,
                    WebcamConfig,
                    CallState,
                    CallAction,
                    CallLog,
                    CallDial,
                    SmsThreads,
                    SmsMessages,
                    SmsSend,
                    ContactsListGet,
                    ContactsList,
                    PhotoNew,
                    PhotoAlbumsList,
                    PhotoListGet,
                    PhotoItems,
                    PhotoThumbsGet,
                    PhotoThumbsList,
                    PhotoGet,
                    LinkOpen,
                    TaskNotify,
                    CameraCaptureRequest,
                    CameraCaptureOk,
                    CameraCaptureCancel,
                    CameraCaptureResultMeta,
                ),
            }
        }
    }

    #[test]
    fn fuzz_screen_shape_decode_and_sanitize_10k() {
        fn next_u64(seed: &mut u64) -> u64 {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 7;
            *seed ^= *seed << 17;
            *seed
        }
        fn next_f32(seed: &mut u64) -> f32 {
            let r = next_u64(seed);
            match r % 10 {
                0 => f32::from_bits(r as u32), // includes NaN, Inf, subnormals
                1 => 0.0,
                2 => -0.0,
                3 => 1.0,
                4 => -1.0,
                5 => ((r >> 8) as f32) / ((u64::MAX >> 8) as f32), // 0.0..=1.0
                6 => (((r >> 8) as f32) / ((u64::MAX >> 8) as f32)) * 6.0 - 0.5, // -0.5..=5.5
                _ => f32::from_bits((r >> 32) as u32),
            }
        }

        let mut seed = 0xD1B5_4A32_D192_ED03u64;
        for i in 0..10_000usize {
            let v = if i % 5 == 0 { (next_u64(&mut seed) % 4) as u32 } else { screen_limits::VERSION };
            let aspect = next_f32(&mut seed);
            let corners = (next_u64(&mut seed) & 1 == 0).then(|| ScreenCorners {
                tl: next_f32(&mut seed),
                tr: next_f32(&mut seed),
                br: next_f32(&mut seed),
                bl: next_f32(&mut seed),
            });
            let cutout_count = (next_u64(&mut seed) % 8) as usize;
            let mut cutouts = Vec::with_capacity(cutout_count);
            for _ in 0..cutout_count {
                cutouts.push(ScreenRect {
                    x: next_f32(&mut seed),
                    y: next_f32(&mut seed),
                    w: next_f32(&mut seed),
                    h: next_f32(&mut seed),
                });
            }
            let cutout_path = match i % 4 {
                0 => None,
                1 => Some("M 0.45 0.01 L 0.55 0.01 Z".to_owned()),
                2 => Some(format!("M {} {} L <script>", next_f32(&mut seed), next_f32(&mut seed))),
                _ => {
                    let len = (next_u64(&mut seed) as usize) % 600;
                    let s: String = (0..len)
                        .map(|_| {
                            let b = (next_u64(&mut seed) & 0x7F) as u8;
                            b as char
                        })
                        .collect();
                    Some(s)
                }
            };

            let shape = ScreenShape { v, aspect, corners, cutouts, cutout_path };
            if let Some(clean) = shape.sanitized() {
                assert_eq!(clean.v, screen_limits::VERSION);
                assert!(clean.aspect.is_finite());
                assert!((screen_limits::MIN_ASPECT..=screen_limits::MAX_ASPECT).contains(&clean.aspect));
                if let Some(c) = clean.corners {
                    for r in [c.tl, c.tr, c.br, c.bl] {
                        assert!(r.is_finite() && (0.0..=screen_limits::MAX_CORNER_RADIUS).contains(&r));
                    }
                }
                assert!(clean.cutouts.len() <= screen_limits::MAX_CUTOUTS);
                for rect in &clean.cutouts {
                    assert!(rect.x.is_finite() && (0.0..=1.0).contains(&rect.x));
                    assert!(rect.y.is_finite() && (0.0..=1.0).contains(&rect.y));
                    assert!(rect.w.is_finite() && rect.w > 0.0 && rect.x + rect.w <= 1.0002);
                    assert!(rect.h.is_finite() && rect.h > 0.0 && rect.y + rect.h <= 1.0002);
                }
                if let Some(ref path) = clean.cutout_path {
                    assert!(!path.is_empty() && path.len() <= screen_limits::MAX_CUTOUT_PATH_BYTES);
                }
                // Idempotence and CBOR round-trip of sanitized ScreenShape.
                let again = clean.clone().sanitized().expect("sanitized shape stays valid");
                assert_eq!(clean, again);
                let mut cbor = Vec::new();
                ciborium::into_writer(&clean, &mut cbor).unwrap();
                let from_cbor: ScreenShape = ciborium::from_reader(cbor.as_slice()).unwrap();
                assert_eq!(from_cbor.sanitized().unwrap(), clean);
            }
        }
    }

    #[test]
    fn wake_relay_and_audio_pcm_cbor_roundtrip() {
        let req =
            WakeRelay { mac: [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff], broadcast: Some("192.168.1.255".into()) };
        let env = crate::Envelope::new(types::WAKE_RELAY, &req).unwrap();
        let decoded: WakeRelay = crate::Envelope::from_cbor(&env.to_cbor()).unwrap().body().unwrap();
        assert_eq!(decoded, req);

        let frame = AudioPcmFrame { sample_rate: 48_000, channels: 2, pcm: vec![0x01, 0x02, 0x03, 0x04] };
        let env = crate::Envelope::new(types::AUDIO_PCM, &frame).unwrap();
        let decoded: AudioPcmFrame = crate::Envelope::from_cbor(&env.to_cbor()).unwrap().body().unwrap();
        assert_eq!(decoded, frame);
    }
}
