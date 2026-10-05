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
    pub const MAX_FILES: usize = 1000;
    pub const MAX_NAME_BYTES: usize = 255;
}

/// One file of an offer.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub size: u64,
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

/// Whether a transfer ID is 16–64 of `[A-Za-z0-9_-]`.
pub fn is_valid_transfer_id(id: &str) -> bool {
    (16..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

impl FilesOffer {
    pub fn is_valid(&self) -> bool {
        is_valid_transfer_id(&self.id)
            && (1..=files::MAX_FILES).contains(&self.files.len())
            && self.files.iter().all(|f| is_valid_file_name(&f.name))
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
        let n = n.sanitized().unwrap();
        assert_eq!(n.title.unwrap().chars().count(), notify_limits::TITLE_CHARS);
        assert_eq!(n.text, None, "blank text is dropped");
        assert_eq!(n.actions.len(), notify_limits::ACTIONS);
        assert_eq!(n.icon, None);
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
            files: names.iter().map(|n| FileEntry { name: (*n).into(), size: 1 }).collect(),
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
    fn power_level_wire_names() {
        let env = Envelope::new("x", &PowerLevel::NotApplicable).unwrap();
        assert_eq!(env.b, Some(ciborium::Value::Text("n/a".into())));
    }
}
