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
    /// A note typed in the chat between two devices (#698): its text in
    /// `detail`, its ID in `ref_id`.
    Message,
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
            TimelineKind::Message => "message",
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
            "message" | "messages" | "chat" => Some(TimelineKind::Message),
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

/// Formats a clipboard text string for display in the Timeline UI:
/// - Local file paths (including `"C:\..."` quoted paths from Explorer's "Copy as path")
///   return `("Copied <filename>", Some(unquoted_path))`.
/// - Web/geo URLs (`http://`, `https://`, `geo:`) return `("Link · <domain>", Some(raw_url))`.
/// - Long or multi-line text returns `("Copied text: <first words>", Some(raw_first_line))`.
#[must_use]
pub fn format_clip_display(raw_text: &str) -> (String, Option<String>) {
    let first_line =
        raw_text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_else(|| raw_text.trim());
    let unquoted = first_line
        .strip_prefix('"')
        .map(|s| s.strip_suffix('"').unwrap_or(s))
        .or_else(|| first_line.strip_prefix('\'').map(|s| s.strip_suffix('\'').unwrap_or(s)))
        .unwrap_or(first_line)
        .trim();

    // Up to 8 characters: `geo:0,0` is shorter than that.
    let lower_prefix: String = unquoted.chars().take(8).collect::<String>().to_ascii_lowercase();
    if (lower_prefix.starts_with("http://")
        || lower_prefix.starts_with("https://")
        || lower_prefix.starts_with("geo:"))
        && !unquoted.contains(char::is_whitespace)
    {
        if lower_prefix.starts_with("geo:") {
            let (title, raw) = crate::handoff::timeline_link_title_and_detail(unquoted, None);
            return (title, Some(raw));
        }
        let host = unquoted
            .split("://")
            .nth(1)
            .unwrap_or(unquoted)
            .split(['/', '?', '#'])
            .next()
            .unwrap_or(unquoted);
        let clean_host = host.strip_prefix("www.").unwrap_or(host);
        if !clean_host.is_empty() {
            return (format!("Link · {clean_host}"), Some(unquoted.to_owned()));
        }
    }

    let bytes = unquoted.as_bytes();
    let is_win_drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/');
    let is_unc = unquoted.starts_with("\\\\") && unquoted.len() > 3;
    let is_unix_path = (unquoted.starts_with('/') && !unquoted.starts_with("//") && unquoted.len() > 2)
        || unquoted.starts_with("~/");
    if is_win_drive || is_unc || is_unix_path {
        let cleaned = unquoted.trim_end_matches(['\\', '/']);
        if let Some(name) = cleaned.rsplit(['\\', '/']).find(|seg| !seg.is_empty())
            && !name.ends_with(':')
        {
            return (format!("Copied file {name}"), Some(unquoted.to_owned()));
        }
    }

    let collapsed: String = unquoted.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > 44 {
        let mut snippet = String::new();
        for word in collapsed.split_whitespace() {
            if !snippet.is_empty() && snippet.chars().count() + 1 + word.chars().count() > 38 {
                break;
            }
            if !snippet.is_empty() {
                snippet.push(' ');
            }
            snippet.push_str(word);
        }
        if snippet.is_empty() {
            snippet = collapsed.chars().take(38).collect();
        }
        return (format!("Copied text: {snippet}…"), Some(collapsed));
    }

    (collapsed, None)
}

/// What a [`TimelineKind::Session`] entry was. Its `title` is English for
/// whoever reads the store directly; an app shows its own words for these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    /// A phone's screen, mirrored on this device.
    PhoneScreen,
    /// One of a phone's apps in a window of its own (its mirroring session).
    AppWindow(u32),
    /// This device's screen, shared with a phone.
    PcScreen,
    /// A phone's camera used as a webcam.
    Webcam,
    /// A phone's microphone used on this device.
    Microphone,
}

/// The title the core gives an app window's session entry.
const APP_WINDOW_TITLE: &str = "App window #";

impl TimelineEntry {
    /// What this entry was, if it is a session; from what the core recorded
    /// (`target` and the direction), so it holds for entries made before
    /// this existed too.
    #[must_use]
    pub fn session_kind(&self) -> Option<SessionKind> {
        if self.kind != TimelineKind::Session {
            return None;
        }
        match self.target.as_str() {
            "webcam" => Some(SessionKind::Webcam),
            "mic" => Some(SessionKind::Microphone),
            "mirror" if !self.incoming => Some(SessionKind::PcScreen),
            "mirror" => Some(
                self.title
                    .strip_prefix(APP_WINDOW_TITLE)
                    .and_then(|n| n.parse().ok())
                    .map_or(SessionKind::PhoneScreen, SessionKind::AppWindow),
            ),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn sessions_say_what_they_were_whatever_their_title() {
        use super::{SessionKind, TimelineEntry, TimelineKind};
        let entry = |kind, incoming, title: &str, target: &str| TimelineEntry {
            id: 1,
            kind,
            device_id: nectarlink_protocol::DeviceId([1; 32]),
            device_name: "Pixel".into(),
            incoming,
            timestamp: 0,
            title: title.into(),
            detail: String::new(),
            target: target.into(),
            size_bytes: 0,
            duration_secs: 0,
            ref_id: None,
            clip_available: false,
            image_data_url: None,
        };
        let session =
            |incoming, title, target| entry(TimelineKind::Session, incoming, title, target).session_kind();
        assert_eq!(session(true, "Phone webcam", "webcam"), Some(SessionKind::Webcam));
        assert_eq!(session(false, "Phone microphone", "mic"), Some(SessionKind::Microphone));
        assert_eq!(session(true, "Screen mirroring", "mirror"), Some(SessionKind::PhoneScreen));
        assert_eq!(session(false, "Screen mirroring", "mirror"), Some(SessionKind::PcScreen));
        assert_eq!(session(true, "App window #3", "mirror"), Some(SessionKind::AppWindow(3)));
        // The title isn't what decides.
        assert_eq!(session(true, "Bildschirm", "webcam"), Some(SessionKind::Webcam));
        assert_eq!(session(true, "Something", "other"), None);
        assert_eq!(entry(TimelineKind::File, true, "Phone webcam", "webcam").session_kind(), None);
    }

    use super::*;

    #[test]
    fn formats_quoted_paths_urls_and_text_for_timeline() {
        let (t1, d1) = format_clip_display("\"C:\\Users\\Alice\\Videos\\Sample Clip.mp4\"");
        assert_eq!(t1, "Copied file Sample Clip.mp4");
        assert_eq!(d1.as_deref(), Some("C:\\Users\\Alice\\Videos\\Sample Clip.mp4"));

        let (t2, d2) = format_clip_display("https://example.com/posts/12345");
        assert_eq!(t2, "Link · example.com");
        assert_eq!(d2.as_deref(), Some("https://example.com/posts/12345"));

        let (geo_title, geo_raw) = format_clip_display("geo:0,0");
        assert_eq!(geo_raw.as_deref(), Some("geo:0,0"), "a short map link is still a link: {geo_title}");
        let (t3, d3) = format_clip_display("Short note");
        assert_eq!(t3, "Short note");
        assert_eq!(d3, None);

        let long_note =
            "This is a much longer copied paragraph that should be truncated cleanly with a colon";
        let (t4, d4) = format_clip_display(long_note);
        assert!(t4.starts_with("Copied text: "), "expected colon prefix, got {t4}");
        assert_eq!(d4.as_deref(), Some(long_note));
    }
}
