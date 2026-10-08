// SPDX-License-Identifier: MPL-2.0
//! Local timeline of items shared between paired devices: files and folders,
//! clipboard items (linked by ID without duplicating content), links opened,
//! saved photos, voice recordings, and mirroring / webcam sessions.

use nectarlink_protocol::DeviceId;
use serde::{Deserialize, Serialize};

/// Default retention window in days (90 days).
pub const DEFAULT_TIMELINE_MAX_DAYS: u32 = 90;
/// Default maximum number of entries kept in the local timeline (5,000).
pub const DEFAULT_TIMELINE_MAX_ENTRIES: u32 = 5_000;
/// Default page size when querying the timeline.
pub const DEFAULT_TIMELINE_PAGE_LIMIT: u32 = 100;
/// Maximum page size allowed in a single query.
pub const MAX_TIMELINE_PAGE_LIMIT: u32 = 250;

/// What kind of item a timeline entry represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TimelineKind {
    /// One or more files or a folder transferred either way.
    File,
    /// A synced text or image clip (linked by `ref_id` to `ClipboardHistoryStore`;
    /// its content is never duplicated in SQLite).
    Clip,
    /// A web link opened on the other device or received from it.
    Link,
    /// One or more photos or videos saved from a paired phone.
    Photo,
    /// A voice recording sent from a phone to a PC.
    Recording,
    /// A screen/app mirroring or webcam session (recorded on start, updated
    /// with `duration_secs` when the stream ends).
    Session,
}

impl TimelineKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TimelineKind::File => "file",
            TimelineKind::Clip => "clip",
            TimelineKind::Link => "link",
            TimelineKind::Photo => "photo",
            TimelineKind::Recording => "recording",
            TimelineKind::Session => "session",
        }
    }

    pub fn from_str_opt(s: &str) -> Option<TimelineKind> {
        match s.trim().to_ascii_lowercase().as_str() {
            "file" | "files" | "folder" => Some(TimelineKind::File),
            "clip" | "clips" | "clipboard" => Some(TimelineKind::Clip),
            "link" | "links" => Some(TimelineKind::Link),
            "photo" | "photos" => Some(TimelineKind::Photo),
            "recording" | "recordings" => Some(TimelineKind::Recording),
            "session" | "sessions" | "mirror" | "webcam" => Some(TimelineKind::Session),
            _ => None,
        }
    }
}

/// Retention policy for the local timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineRetention {
    /// Auto-purge entries older than `max_days` days (`0` disables age limit).
    pub max_days: u32,
    /// Auto-purge oldest entries beyond `max_entries` (`0` disables count cap).
    pub max_entries: u32,
}

impl Default for TimelineRetention {
    fn default() -> Self {
        TimelineRetention { max_days: DEFAULT_TIMELINE_MAX_DAYS, max_entries: DEFAULT_TIMELINE_MAX_ENTRIES }
    }
}

/// Search, kind, device, and pagination parameters for querying the timeline.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineQuery {
    /// Filter to a single kind (`None` returns all kinds).
    pub kind: Option<TimelineKind>,
    /// Filter to a single peer device (`None` returns all devices).
    pub device: Option<DeviceId>,
    /// Case-insensitive substring search across title, detail, device name,
    /// and linked clipboard text.
    pub search: Option<String>,
    /// Number of newest matching rows to skip.
    pub offset: u32,
    /// Maximum rows to return (clamped to `1..=MAX_TIMELINE_PAGE_LIMIT`).
    pub limit: u32,
}

/// One row in the local timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineEntry {
    /// Monotonic SQLite row ID.
    pub id: i64,
    /// Kind of activity.
    pub kind: TimelineKind,
    /// The paired device involved.
    pub device_id: DeviceId,
    /// Display name of the paired device.
    pub device_name: String,
    /// `true` if received from `device_id`; `false` if sent/started from here.
    pub incoming: bool,
    /// Unix timestamp in seconds when the item occurred (or started, for sessions).
    pub timestamp: i64,
    /// Primary title (file/folder name, URL, photo name, recording name, or
    /// session name). For `Clip`, this is empty in SQLite and populated at
    /// query time from `ClipboardHistoryStore`.
    pub title: String,
    /// Secondary detail (e.g. `"Folder · 4 files"`, `"text"` / `"image"`, host,
    /// resolution, or marker count).
    pub detail: String,
    /// Local path(s) (newline-separated when multiple) or URL for links.
    pub target: String,
    /// Total byte size for files, photos, and recordings (`0` otherwise).
    pub size_bytes: u64,
    /// Duration in seconds for mirroring/webcam sessions (`0` while active or N/A).
    pub duration_secs: u64,
    /// Reference ID linking to `ClipboardHistoryStore` (for clips) or `Transfer.id`
    /// (for files, photos, and recordings).
    pub ref_id: Option<String>,
    /// For `Clip` entries: whether the referenced clip is still present in
    /// `ClipboardHistoryStore`.
    pub clip_available: bool,
    /// For image `Clip` entries: a `data:image/png;base64,...` preview resolved
    /// from `ClipboardHistoryStore` at query time (never stored in SQLite).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_data_url: Option<String>,
}

/// A single page of timeline query results.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelinePage {
    /// Entries in this page, newest first.
    pub entries: Vec<TimelineEntry>,
    /// Total number of entries matching the filter across all pages.
    pub total: u32,
    /// Whether additional older entries exist beyond `offset + entries.len()`.
    pub has_more: bool,
}

/// Parameters for inserting a new timeline row into SQLite.
#[derive(Debug, Clone)]
pub(crate) struct NewTimelineEntry {
    pub kind: TimelineKind,
    pub device_id: DeviceId,
    pub device_name: String,
    pub incoming: bool,
    pub timestamp: i64,
    pub title: String,
    pub detail: String,
    pub target: String,
    pub size_bytes: u64,
    pub duration_secs: u64,
    pub ref_id: Option<String>,
}

/// Encodes `(mime, bytes)` as a `data:<mime>;base64,...` URI for image clip previews.
pub(crate) fn encode_data_url(mime: &str, bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(6 + mime.len() + 8 + bytes.len().div_ceil(3) * 4);
    out.push_str("data:");
    out.push_str(mime);
    out.push_str(";base64,");
    let (chunks, rem) = bytes.as_chunks::<3>();
    for &[b0, b1, b2] in chunks {
        let n = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        out.push(TABLE[((n >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3F) as usize] as char);
        out.push(TABLE[((n >> 6) & 0x3F) as usize] as char);
        out.push(TABLE[(n & 0x3F) as usize] as char);
    }
    if rem.len() == 1 {
        let n = u32::from(rem[0]) << 16;
        out.push(TABLE[((n >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3F) as usize] as char);
        out.push('=');
        out.push('=');
    } else if rem.len() == 2 {
        let n = (u32::from(rem[0]) << 16) | (u32::from(rem[1]) << 8);
        out.push(TABLE[((n >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3F) as usize] as char);
        out.push(TABLE[((n >> 6) & 0x3F) as usize] as char);
        out.push('=');
    }
    out
}
