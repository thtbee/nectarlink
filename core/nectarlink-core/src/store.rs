// SPDX-License-Identifier: MPL-2.0
//! SQLite storage: the trust store of paired devices, their last known
//! addresses and capabilities, and per-device settings.

use std::{
    collections::{BTreeSet, HashMap},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, OnceLock},
};

use nectarlink_protocol::{
    DeviceId,
    messages::{DeviceInfo, DeviceKind, Notification, PcWakeInfo, PowerLevel, ScreenShape},
};
use rusqlite::{Connection, OptionalExtension, params};

use crate::{
    CameraCaptureMode, Error, RecordingMarker, Result, TaskNotify,
    timeline::{
        DEFAULT_TIMELINE_PAGE_LIMIT, MAX_TIMELINE_PAGE_LIMIT, NewTimelineEntry, TimelineEntry, TimelineKind,
        TimelinePage, TimelineQuery, TimelineRetention,
    },
};

const FILE_NAME: &str = "nectarlink.db";

/// Maximum active (`Waiting` + `Sending`) items in `send_queue` across all peers.
pub const MAX_QUEUE_ACTIVE_ITEMS: usize = 200;
/// Maximum pending bytes across all active (`Waiting` + `Sending`) file-based and clipboard-image queue items (10 GiB).
pub const MAX_QUEUE_ACTIVE_BYTES: u64 = 10 * 1024 * 1024 * 1024;
/// Maximum size in bytes of a single queued clipboard image (`payload_blob`) (50 MiB).
pub const MAX_QUEUE_CLIPBOARD_IMAGE_BYTES: usize = 50 * 1024 * 1024;

/// Returns a file's modification time in Unix milliseconds (or `0` if unavailable).
#[must_use]
pub fn file_mtime_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

/// Kind of outgoing action stored in the persistent send queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueKind {
    Files,
    Folder,
    Clipboard,
    ClipboardImage,
    Link,
    HandoffDocument,
    Recording,
    TaskNotify,
    CameraRequest,
}

impl QueueKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            QueueKind::Files => "files",
            QueueKind::Folder => "folder",
            QueueKind::Clipboard => "clipboard",
            QueueKind::ClipboardImage => "clipboard_image",
            QueueKind::Link => "link",
            QueueKind::HandoffDocument => "handoff_document",
            QueueKind::Recording => "recording",
            QueueKind::TaskNotify => "task_notify",
            QueueKind::CameraRequest => "camera_request",
        }
    }

    #[must_use]
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "files" => Some(QueueKind::Files),
            "folder" => Some(QueueKind::Folder),
            "clipboard" => Some(QueueKind::Clipboard),
            "clipboard_image" => Some(QueueKind::ClipboardImage),
            "link" => Some(QueueKind::Link),
            "handoff_document" => Some(QueueKind::HandoffDocument),
            "recording" => Some(QueueKind::Recording),
            "task_notify" => Some(QueueKind::TaskNotify),
            "camera_request" => Some(QueueKind::CameraRequest),
            _ => None,
        }
    }

    /// Per-type time-to-live in milliseconds (`task-queue.md` §4).
    #[must_use]
    pub const fn ttl_ms(self) -> i64 {
        match self {
            QueueKind::CameraRequest => 2 * 60 * 1000,
            QueueKind::Link | QueueKind::Clipboard | QueueKind::ClipboardImage | QueueKind::TaskNotify => {
                24 * 60 * 60 * 1000
            }
            QueueKind::Files | QueueKind::Folder | QueueKind::HandoffDocument | QueueKind::Recording => {
                7 * 24 * 60 * 60 * 1000
            }
        }
    }

    /// Human-readable expiry message when an item exceeds its TTL.
    #[must_use]
    pub const fn expiry_error_message(self) -> &'static str {
        match self {
            QueueKind::CameraRequest => "Expired (camera requests expire after 2 minutes)",
            QueueKind::Link | QueueKind::Clipboard | QueueKind::ClipboardImage | QueueKind::TaskNotify => {
                "Expired (waited more than 24 hours)"
            }
            QueueKind::Files | QueueKind::Folder | QueueKind::HandoffDocument | QueueKind::Recording => {
                "Expired (waited more than 7 days)"
            }
        }
    }

    #[must_use]
    pub const fn is_clipboard(self) -> bool {
        matches!(self, QueueKind::Clipboard | QueueKind::ClipboardImage)
    }

    /// What this kind holds is the user's own content rather than a reference
    /// to a file, so it's sealed in the database and gone when it expires.
    #[must_use]
    pub const fn is_sealed(self) -> bool {
        matches!(
            self,
            QueueKind::Clipboard | QueueKind::ClipboardImage | QueueKind::Link | QueueKind::TaskNotify
        )
    }

    #[must_use]
    pub const fn is_file_based(self) -> bool {
        matches!(
            self,
            QueueKind::Files | QueueKind::Folder | QueueKind::HandoffDocument | QueueKind::Recording
        )
    }
}

/// On-disk file reference and integrity snapshot for a queued file transfer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct QueuedFileSpec {
    /// Absolute path on disk (or staged path under `<data_dir>/queue_staging/<id>/` when `staged == true`).
    pub path: PathBuf,
    /// Sanitized file name sent to the peer.
    pub name: String,
    /// Relative folder path inside a folder transfer (`None` for flat files).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    /// File size in bytes recorded at enqueue time.
    pub size: u64,
    /// Last-modified timestamp (Unix ms) recorded at enqueue time.
    pub mtime_ms: i64,
    /// `true` if the file was copied into `<data_dir>/queue_staging/<id>/` from an ephemeral FD/URI.
    #[serde(default)]
    pub staged: bool,
}

impl QueuedFileSpec {
    /// Verifies that the file still exists on disk and has the exact same `size` and `mtime_ms`
    /// recorded when it was enqueued.
    pub fn verify_integrity(&self) -> std::result::Result<(), String> {
        let meta = std::fs::metadata(&self.path)
            .map_err(|_| format!("File was deleted before sending: {}", self.name))?;
        if meta.len() != self.size || file_mtime_ms(&meta) != self.mtime_ms {
            return Err(format!("File was modified before sending: {}", self.name));
        }
        Ok(())
    }
}

/// Typed payload persisted in `send_queue.payload_json`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum QueuePayload {
    Files {
        files: Vec<QueuedFileSpec>,
    },
    Folder {
        root_name: String,
        files: Vec<QueuedFileSpec>,
    },
    Clipboard {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        html: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rtf: Option<String>,
    },
    ClipboardImage {
        mime: String,
        width: u32,
        height: u32,
    },
    Link {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    HandoffDocument {
        file: QueuedFileSpec,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        position_json: Option<String>,
    },
    Recording {
        file: QueuedFileSpec,
        markers: Vec<RecordingMarker>,
    },
    TaskNotify {
        task: TaskNotify,
    },
    CameraRequest {
        request_id: String,
        mode: CameraCaptureMode,
    },
}

impl QueuePayload {
    #[must_use]
    pub const fn kind(&self) -> QueueKind {
        match self {
            QueuePayload::Files { .. } => QueueKind::Files,
            QueuePayload::Folder { .. } => QueueKind::Folder,
            QueuePayload::Clipboard { .. } => QueueKind::Clipboard,
            QueuePayload::ClipboardImage { .. } => QueueKind::ClipboardImage,
            QueuePayload::Link { .. } => QueueKind::Link,
            QueuePayload::HandoffDocument { .. } => QueueKind::HandoffDocument,
            QueuePayload::Recording { .. } => QueueKind::Recording,
            QueuePayload::TaskNotify { .. } => QueueKind::TaskNotify,
            QueuePayload::CameraRequest { .. } => QueueKind::CameraRequest,
        }
    }

    #[must_use]
    pub fn files_vec(&self) -> Vec<QueuedFileSpec> {
        match self {
            QueuePayload::Files { files } | QueuePayload::Folder { files, .. } => files.clone(),
            QueuePayload::HandoffDocument { file, .. } | QueuePayload::Recording { file, .. } => {
                vec![file.clone()]
            }
            _ => Vec::new(),
        }
    }

    #[must_use]
    pub fn totals(&self, payload_blob: Option<&[u8]>) -> (u64, u32) {
        match self {
            QueuePayload::Files { files } | QueuePayload::Folder { files, .. } => {
                let bytes = files.iter().map(|f| f.size).fold(0u64, u64::saturating_add);
                let count = u32::try_from(files.len()).unwrap_or(u32::MAX);
                (bytes, count)
            }
            QueuePayload::HandoffDocument { file, .. } | QueuePayload::Recording { file, .. } => {
                (file.size, 1)
            }
            QueuePayload::ClipboardImage { .. } => (payload_blob.map_or(0, |b| b.len() as u64), 0),
            _ => (0, 0),
        }
    }

    #[must_use]
    /// What the database shows for this item: for sealed kinds a label that
    /// says nothing about the content, and otherwise [`Self::title_and_detail`].
    fn stored_title_and_detail(&self) -> (String, String) {
        match self {
            QueuePayload::Clipboard { .. } => ("Copied text".into(), "Clipboard text".into()),
            QueuePayload::ClipboardImage { .. } => ("Copied image".into(), "Clipboard image".into()),
            QueuePayload::Link { .. } => ("Link".into(), "Link".into()),
            QueuePayload::TaskNotify { .. } => ("Task".into(), "Task".into()),
            _ => self.title_and_detail(),
        }
    }

    pub fn title_and_detail(&self) -> (String, String) {
        match self {
            QueuePayload::Files { files } => {
                let title = match files.len() {
                    0 => "Files".into(),
                    1 => files[0].name.clone(),
                    n => format!("{} (+{} more)", files[0].name, n - 1),
                };
                let detail = match files.len() {
                    1 => "1 file".into(),
                    n => format!("{n} files"),
                };
                (title, detail)
            }
            QueuePayload::Folder { root_name, files } => {
                let title = if root_name.trim().is_empty() {
                    files
                        .first()
                        .and_then(|f| f.folder.as_deref())
                        .and_then(|f| f.split('/').find(|s| !s.is_empty()))
                        .unwrap_or("Folder")
                        .to_owned()
                } else {
                    root_name.clone()
                };
                let detail = match files.len() {
                    1 => "Folder · 1 file".into(),
                    n => format!("Folder · {n} files"),
                };
                (title, detail)
            }
            QueuePayload::Clipboard { text, .. } => {
                let collapsed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
                let title = if collapsed.chars().count() > 80 {
                    let prefix: String = collapsed.chars().take(79).collect();
                    format!("{prefix}…")
                } else if collapsed.is_empty() {
                    "Copied text".into()
                } else {
                    collapsed
                };
                (title, "Clipboard text".into())
            }
            QueuePayload::ClipboardImage { mime, width, height } => {
                let detail = if *width > 0 && *height > 0 {
                    format!("{mime} · {width}×{height}")
                } else {
                    mime.clone()
                };
                ("Copied image".into(), detail)
            }
            QueuePayload::Link { url, title } => {
                let display_title = title
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(url.as_str())
                    .to_owned();
                (display_title, url.clone())
            }
            QueuePayload::HandoffDocument { file, .. } => (file.name.clone(), "Document handoff".into()),
            QueuePayload::Recording { file, markers } => {
                let detail = match markers.len() {
                    0 => "Voice recording".into(),
                    1 => "Voice recording · 1 marker".into(),
                    n => format!("Voice recording · {n} markers"),
                };
                (file.name.clone(), detail)
            }
            QueuePayload::TaskNotify { task } => {
                let detail = if task.active { "Task started".into() } else { "Task finished".into() };
                (task.title.clone(), detail)
            }
            QueuePayload::CameraRequest { mode, .. } => {
                let title = match mode {
                    CameraCaptureMode::Photo => "Take photo with phone".into(),
                    CameraCaptureMode::Scan => "Scan document with phone".into(),
                };
                (title, "Camera request".into())
            }
        }
    }
}

/// Lifecycle state of an item in the persistent send queue.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum QueueStatus {
    Waiting,
    Sending,
    Failed { error: String },
    Expired { error: String },
}

impl QueueStatus {
    #[must_use]
    pub const fn as_db_str(&self) -> &'static str {
        match self {
            QueueStatus::Waiting => "waiting",
            QueueStatus::Sending => "sending",
            QueueStatus::Failed { .. } => "failed",
            QueueStatus::Expired { .. } => "expired",
        }
    }
}

/// An item in the persistent outgoing send queue.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct QueueItem {
    pub id: String,
    pub seq: i64,
    pub peer: DeviceId,
    pub peer_name: String,
    pub kind: QueueKind,
    pub status: QueueStatus,
    pub created_at: i64,
    pub expires_at: i64,
    pub attempts: u32,
    pub title: String,
    pub detail: String,
    pub bytes_total: u64,
    pub bytes_done: u64,
    pub files_total: u32,
    pub files_done: u32,
    pub files: Vec<QueuedFileSpec>,
}

/// Retention window for persisted notification-backed chat threads (90 days).
pub const CHAT_RETENTION_SECS: i64 = 90 * 86_400;
/// Maximum persisted notification-backed chat threads per paired phone.
pub const MAX_CHAT_THREADS_PER_PEER: usize = 200;
/// Maximum persisted messages per notification-backed chat thread.
pub const MAX_CHAT_MESSAGES_PER_THREAD: usize = 200;

/// Deterministic thread ID for a notification-backed messaging conversation.
pub fn chat_thread_id(app: &str, title: &str) -> String {
    format!("chat:{}:{}", app.trim(), title.trim().to_lowercase())
}

/// A persisted conversation from a messaging app notification (`MessagingStyle`).
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ChatThreadRecord {
    pub peer: DeviceId,
    pub thread_id: String,
    pub app: String,
    pub app_name: String,
    pub notification_key: String,
    pub reply_action_id: Option<String>,
    pub title: String,
    pub is_group: bool,
    pub snippet: String,
    pub last_sender: Option<String>,
    pub updated_ms: i64,
    pub unread: u32,
    pub active: bool,
    pub avatar: Option<Vec<u8>>,
    pub app_icon: Option<Vec<u8>>,
}

/// Never prints conversation titles, snippets, or sender names (protocol v0 §11).
impl std::fmt::Debug for ChatThreadRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatThreadRecord")
            .field("peer", &self.peer)
            .field("thread_id", &self.thread_id)
            .field("app", &self.app)
            .field("is_group", &self.is_group)
            .field("updated_ms", &self.updated_ms)
            .field("unread", &self.unread)
            .field("active", &self.active)
            .finish_non_exhaustive()
    }
}

impl ChatThreadRecord {
    /// Whether this conversation's latest notification is still active and has
    /// an inline reply action.
    pub fn can_reply(&self) -> bool {
        self.active && self.reply_action_id.is_some()
    }
}

/// One persisted message inside a [`ChatThreadRecord`].
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ChatMessageRecord {
    pub id: i64,
    pub peer: DeviceId,
    pub thread_id: String,
    pub sender: Option<String>,
    pub text: String,
    pub time_ms: i64,
    pub self_sent: bool,
    pub avatar: Option<Vec<u8>>,
}

/// Never prints message text or sender names (protocol v0 §11).
impl std::fmt::Debug for ChatMessageRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatMessageRecord")
            .field("id", &self.id)
            .field("peer", &self.peer)
            .field("thread_id", &self.thread_id)
            .field("time_ms", &self.time_ms)
            .field("self_sent", &self.self_sent)
            .finish_non_exhaustive()
    }
}

/// Schema migrations, applied in order. Index + 1 is the schema version.
const MIGRATIONS: &[&str] = &[
    // v1: trust store
    "CREATE TABLE peers (
        id          BLOB PRIMARY KEY NOT NULL CHECK (length(id) = 32),
        name        TEXT NOT NULL,
        kind        TEXT NOT NULL,
        os          TEXT NOT NULL,
        os_ver      TEXT NOT NULL,
        model       TEXT,
        accent      INTEGER,
        paired_at   INTEGER NOT NULL,
        last_seen   INTEGER,
        last_addrs  TEXT NOT NULL DEFAULT ''
    ) STRICT;",
    // v2: last known capabilities and power level (for the capability matrix
    // while a device is offline), and per-device toggles.
    "ALTER TABLE peers ADD COLUMN caps TEXT NOT NULL DEFAULT '';
     ALTER TABLE peers ADD COLUMN power TEXT NOT NULL DEFAULT 'basic';
     CREATE TABLE peer_toggles (
        peer        BLOB NOT NULL REFERENCES peers(id) ON DELETE CASCADE,
        toggle      TEXT NOT NULL,
        enabled     INTEGER NOT NULL CHECK (enabled IN (0, 1)),
        PRIMARY KEY (peer, toggle)
     ) STRICT;",
    // v3: Wake-on-LAN adapter MAC and subnet broadcast addresses per paired PC.
    "ALTER TABLE peers ADD COLUMN wake_info TEXT NOT NULL DEFAULT '';",
    // v4: measured phone/tablet front-screen geometry (`DeviceInfo.screen`).
    "ALTER TABLE peers ADD COLUMN screen TEXT NOT NULL DEFAULT '';",
    // v5: local timeline of items shared between devices (files/folders, clips
    // linked by ID without content duplication, links, saved photos, voice
    // recordings, and mirroring/webcam sessions) and key-value settings.
    "CREATE TABLE timeline (
        id            INTEGER PRIMARY KEY AUTOINCREMENT,
        kind          TEXT NOT NULL,
        device_id     BLOB NOT NULL CHECK (length(device_id) = 32),
        device_name   TEXT NOT NULL,
        incoming      INTEGER NOT NULL CHECK (incoming IN (0, 1)),
        timestamp     INTEGER NOT NULL,
        title         TEXT NOT NULL DEFAULT '',
        detail        TEXT NOT NULL DEFAULT '',
        target        TEXT NOT NULL DEFAULT '',
        size_bytes    INTEGER NOT NULL DEFAULT 0,
        duration_secs INTEGER NOT NULL DEFAULT 0,
        ref_id        TEXT
     ) STRICT;
     CREATE INDEX idx_timeline_time ON timeline(timestamp DESC, id DESC);
     CREATE INDEX idx_timeline_ref ON timeline(ref_id) WHERE ref_id IS NOT NULL;
     CREATE TABLE meta (
        key           TEXT PRIMARY KEY NOT NULL,
        value         TEXT NOT NULL
     ) STRICT;",
    // v6: unified conversations persisted from MessagingStyle notifications.
    "CREATE TABLE IF NOT EXISTS chat_threads (
        peer             TEXT NOT NULL,
        thread_id        TEXT NOT NULL,
        app              TEXT NOT NULL,
        app_name         TEXT NOT NULL,
        notification_key TEXT NOT NULL,
        reply_action_id  TEXT,
        title            TEXT NOT NULL,
        is_group         INTEGER NOT NULL DEFAULT 0,
        snippet          TEXT NOT NULL DEFAULT '',
        last_sender      TEXT,
        updated_ms       INTEGER NOT NULL,
        unread           INTEGER NOT NULL DEFAULT 0,
        active           INTEGER NOT NULL DEFAULT 1,
        avatar           BLOB,
        app_icon         BLOB,
        PRIMARY KEY (peer, thread_id)
    );
    CREATE INDEX IF NOT EXISTS idx_chat_threads_peer_updated
        ON chat_threads (peer, updated_ms DESC);

    CREATE TABLE IF NOT EXISTS chat_messages (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        peer       TEXT NOT NULL,
        thread_id  TEXT NOT NULL,
        sender     TEXT,
        text       TEXT NOT NULL,
        time_ms    INTEGER NOT NULL,
        self_sent  INTEGER NOT NULL DEFAULT 0,
        avatar     BLOB
    );
    CREATE INDEX IF NOT EXISTS idx_chat_messages_thread_time
        ON chat_messages (peer, thread_id, time_ms DESC, id DESC);",
    // v7: persistent bidirectional outgoing send queue.
    "CREATE TABLE IF NOT EXISTS send_queue (
        id              TEXT PRIMARY KEY,
        seq             INTEGER NOT NULL,
        peer_id         TEXT NOT NULL,
        kind            TEXT NOT NULL,
        status          TEXT NOT NULL,
        created_at      INTEGER NOT NULL,
        expires_at      INTEGER NOT NULL,
        next_retry_ms   INTEGER NOT NULL DEFAULT 0,
        attempts        INTEGER NOT NULL DEFAULT 0,
        title           TEXT NOT NULL,
        detail          TEXT NOT NULL,
        payload_json    TEXT NOT NULL,
        payload_blob    BLOB,
        bytes_total     INTEGER NOT NULL DEFAULT 0,
        bytes_done      INTEGER NOT NULL DEFAULT 0,
        files_total     INTEGER NOT NULL DEFAULT 0,
        files_done      INTEGER NOT NULL DEFAULT 0,
        error_msg       TEXT
    );
    CREATE INDEX IF NOT EXISTS idx_send_queue_peer_fifo
        ON send_queue(peer_id, status, seq ASC);
    CREATE INDEX IF NOT EXISTS idx_send_queue_status_wake
        ON send_queue(status, next_retry_ms ASC, expires_at ASC);",
    // v8: a device's install-stable ID, the only key used to merge re-paired entries
    "ALTER TABLE peers ADD COLUMN install_id TEXT;",
];

/// A paired device as stored on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PeerRecord {
    pub id: DeviceId,
    pub info: DeviceInfo,
    pub paired_at: i64,
    pub last_seen: Option<i64>,
    pub last_addrs: Vec<SocketAddr>,
    /// Capabilities from the device's last `hello`.
    pub caps: BTreeSet<String>,
    pub power: PowerLevel,
    /// Wake-on-LAN addresses from a paired PC's last `pc.wake_info`.
    pub wake_info: Option<PcWakeInfo>,
}

/// How a sealed `payload_json` starts: the rest is base64 of the sealed JSON.
const SEALED_PREFIX: &str = "sealed1:";

/// Seals queued content with the platform's key protector, as the clipboard
/// history is, so a copied password waiting to send isn't readable in the
/// database. On a platform without a protector (Linux) this is only encoding.
#[derive(Default)]
struct QueueSeal(OnceLock<Arc<dyn crate::identity::KeyProtector>>);

impl std::fmt::Debug for QueueSeal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("QueueSeal")
    }
}

impl QueueSeal {
    fn seal(&self, plain: &[u8]) -> Result<Vec<u8>> {
        match self.0.get() {
            Some(protector) => protector.protect(plain).map_err(|e| Error::Internal(e.to_string())),
            None => Ok(plain.to_vec()),
        }
    }

    fn open(&self, sealed: &[u8]) -> Option<Vec<u8>> {
        match self.0.get() {
            Some(protector) => protector.unprotect(sealed).ok(),
            None => Some(sealed.to_vec()),
        }
    }

    fn seal_json(&self, json: &str) -> Result<String> {
        let sealed = self.seal(json.as_bytes())?;
        Ok(format!("{SEALED_PREFIX}{}", data_encoding::BASE64.encode(&sealed)))
    }

    /// The payload in `stored`, sealed or from before sealing.
    fn payload(&self, stored: &str) -> Option<QueuePayload> {
        match stored.strip_prefix(SEALED_PREFIX) {
            Some(encoded) => {
                let sealed = data_encoding::BASE64.decode(encoded.as_bytes()).ok()?;
                serde_json::from_slice(&self.open(&sealed)?).ok()
            }
            None => serde_json::from_str(stored).ok(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct Store {
    conn: Mutex<Connection>,
    data_dir: Option<PathBuf>,
    queue_seal: QueueSeal,
    /// Each device's toggles as stored, read once: they're checked on every
    /// message (each touch while mirroring), and change only when the user
    /// flips one. Filled and cleared while `conn` is held, so a write never
    /// leaves an old copy behind.
    toggles: Mutex<HashMap<DeviceId, Arc<HashMap<String, bool>>>>,
}

impl Store {
    pub fn open(dir: &Path) -> Result<Self> {
        Self::init_with_dir(Connection::open(dir.join(FILE_NAME))?, Some(dir.to_path_buf()))
    }

    #[cfg(test)]
    pub fn in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    #[cfg(test)]
    fn init(conn: Connection) -> Result<Self> {
        Self::init_with_dir(conn, None)
    }

    fn init_with_dir(conn: Connection, data_dir: Option<PathBuf>) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        // The write-ahead log shrinks back after a checkpoint. Without a limit it
        // keeps the size of its busiest moment (as big as the database itself, seen
        // on a real install), and what was deleted stays readable in it.
        conn.pragma_update(None, "journal_size_limit", 1_048_576)?;
        conn.pragma_update(None, "temp_store", "MEMORY")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        // What's deleted is overwritten, not left in free pages.
        conn.pragma_update(None, "secure_delete", "ON")?;
        migrate(&conn)?;
        Ok(Store {
            conn: Mutex::new(conn),
            data_dir,
            queue_seal: QueueSeal::default(),
            toggles: Mutex::default(),
        })
    }

    /// Seals queued content with `protector` from now on, and seals what
    /// earlier versions left in the queue unsealed. Call once, at startup.
    pub fn seal_queue_with(&self, protector: Arc<dyn crate::identity::KeyProtector>) -> Result<usize> {
        let _ = self.queue_seal.0.set(protector);
        self.with(|c| {
            let rows: Vec<(String, String, Option<Vec<u8>>)> = {
                let mut stmt = c.prepare(
                    "SELECT id, payload_json, payload_blob FROM send_queue
                     WHERE kind IN ('clipboard', 'clipboard_image', 'link', 'task_notify')
                       AND payload_json NOT LIKE 'sealed1:%'",
                )?;
                stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            };
            let mut sealed = 0;
            for (id, json, blob) in rows {
                let Some(payload) = serde_json::from_str::<QueuePayload>(&json).ok() else { continue };
                let (title, detail) = payload.stored_title_and_detail();
                let Ok(stored) = self.queue_seal.seal_json(&json) else { continue };
                let blob = match blob.map(|b| self.queue_seal.seal(&b)).transpose() {
                    Ok(blob) => blob,
                    Err(_) => continue,
                };
                c.execute(
                    "UPDATE send_queue SET title = ?1, detail = ?2, payload_json = ?3, payload_blob = ?4
                     WHERE id = ?5",
                    params![title, detail, stored, blob, id],
                )?;
                sealed += 1;
            }
            if sealed > 0 {
                // The old rows shouldn't linger in the log or in free pages.
                let _ = c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM;");
            }
            Ok(sealed)
        })
    }

    /// Removes queued items of `kinds` whatever their state, for clearing
    /// what the user asked to forget. Returns how many went.
    pub fn remove_queue_items_of_kinds(&self, kinds: &[QueueKind]) -> Result<usize> {
        self.with(|c| {
            let mut removed = 0;
            for kind in kinds {
                removed += c.execute("DELETE FROM send_queue WHERE kind = ?1", [kind.as_str()])?;
            }
            if removed > 0 {
                let _ = c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
            }
            Ok(removed)
        })
    }

    /// Whether a queue item is still there: it may have been cleared while it was being sent.
    pub fn has_queue_item(&self, id: &str) -> bool {
        self.with(|c| {
            c.query_row("SELECT 1 FROM send_queue WHERE id = ?1", [id], |_| Ok(()))
                .optional()
                .map(|r| r.is_some())
        })
        .unwrap_or(false)
    }

    fn with<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T> {
        let conn = self.conn.lock().map_err(|_| Error::Internal("store lock poisoned".into()))?;
        f(&conn).map_err(Error::from)
    }

    fn toggle_cache(&self) -> MutexGuard<'_, HashMap<DeviceId, Arc<HashMap<String, bool>>>> {
        self.toggles.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Adds or replaces a paired device.
    pub fn upsert_peer(&self, id: &DeviceId, info: &DeviceInfo, paired_at: i64) -> Result<()> {
        self.upsert_peer_skipping(id, info, paired_at, &[])
    }

    /// Adds or replaces a paired device, leaving any peer in `skip_ids` (such as
    /// currently online peers) untouched during duplicate-install cleanup.
    pub fn upsert_peer_skipping(
        &self,
        id: &DeviceId,
        info: &DeviceInfo,
        paired_at: i64,
        skip_ids: &[DeviceId],
    ) -> Result<()> {
        let screen = encode_screen(info.screen.as_ref());
        self.with(|c| {
            c.execute(
                "INSERT INTO peers (id, name, kind, os, os_ver, model, accent, paired_at, screen, install_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name, kind = excluded.kind, os = excluded.os,
                    os_ver = excluded.os_ver, model = excluded.model,
                    accent = COALESCE(excluded.accent, peers.accent),
                    paired_at = excluded.paired_at,
                    screen = CASE WHEN excluded.screen = '' THEN peers.screen ELSE excluded.screen END,
                    install_id = COALESCE(excluded.install_id, peers.install_id)",
                params![
                    id.as_bytes().as_slice(),
                    info.name,
                    kind_to_str(info.kind),
                    info.os,
                    info.os_ver,
                    info.model,
                    info.accent.map(i64::from),
                    paired_at,
                    screen,
                    valid_install_id(info)
                ],
            )?;
            cleanup_peers(c, Some(id), skip_ids)?;
            // A stale copy of this device may be gone, toggles and all.
            self.toggle_cache().clear();
            Ok(())
        })
    }

    /// Reassigns timeline entries from `from` to `to` when deduplicating a
    /// re-paired physical device.
    pub fn reassign_timeline(&self, from: &DeviceId, to: &DeviceId) -> Result<usize> {
        self.with(|c| {
            c.execute(
                "UPDATE timeline SET device_id = ?1 WHERE device_id = ?2",
                params![to.as_bytes().as_slice(), from.as_bytes().as_slice()],
            )
        })
    }

    /// Updates the stored description of a device (e.g. after a rename).
    ///
    /// Never its install ID: that is recorded only when the user pairs the
    /// device, so a paired device can't later claim to be another one and
    /// take over its pairing and history.
    pub fn update_info(&self, id: &DeviceId, info: &DeviceInfo) -> Result<()> {
        let screen = encode_screen(info.screen.as_ref());
        // The device and its timeline name change together.
        self.with(|c| {
            let tx = c.unchecked_transaction()?;
            tx.execute(
                "UPDATE peers SET name = ?2, kind = ?3, os = ?4, os_ver = ?5, model = ?6,
                    accent = COALESCE(?7, accent),
                    screen = CASE WHEN ?8 = '' THEN screen ELSE ?8 END
                 WHERE id = ?1",
                params![
                    id.as_bytes().as_slice(),
                    info.name,
                    kind_to_str(info.kind),
                    info.os,
                    info.os_ver,
                    info.model,
                    info.accent.map(i64::from),
                    screen,
                ],
            )?;
            tx.execute(
                "UPDATE timeline SET device_name = ?2 WHERE device_id = ?1",
                params![id.as_bytes().as_slice(), info.name],
            )?;
            tx.commit()
        })
    }

    /// Records a successful connection and the peer's current direct addresses.
    pub fn record_seen(&self, id: &DeviceId, at: i64, addrs: &[SocketAddr]) -> Result<()> {
        let addrs: Vec<String> = addrs.iter().map(ToString::to_string).collect();
        self.with(|c| {
            c.execute(
                "UPDATE peers SET last_seen = ?2, last_addrs = ?3 WHERE id = ?1",
                params![id.as_bytes().as_slice(), at, addrs.join(",")],
            )
            .map(drop)
        })
    }

    /// Records the capabilities and power level a device last announced.
    /// `None` keeps the stored value.
    pub fn update_capabilities(
        &self,
        id: &DeviceId,
        caps: Option<&BTreeSet<String>>,
        power: Option<PowerLevel>,
    ) -> Result<()> {
        let caps = caps.map(|c| c.iter().map(String::as_str).collect::<Vec<_>>().join(","));
        self.with(|c| {
            c.execute(
                "UPDATE peers SET caps = coalesce(?2, caps), power = coalesce(?3, power) WHERE id = ?1",
                params![id.as_bytes().as_slice(), caps, power.map(power_to_str)],
            )
            .map(drop)
        })
    }

    /// Records or clears the Wake-on-LAN addresses reported by a paired PC.
    pub fn update_wake_info(&self, id: &DeviceId, info: Option<&PcWakeInfo>) -> Result<()> {
        let encoded = info.map(PcWakeInfo::to_storage_string).unwrap_or_default();
        self.with(|c| {
            c.execute(
                "UPDATE peers SET wake_info = ?2 WHERE id = ?1",
                params![id.as_bytes().as_slice(), encoded],
            )
            .map(drop)
        })
    }

    /// The per-device toggles the user has set. Unset toggles are absent;
    /// callers apply the defaults.
    pub fn toggles(&self, id: &DeviceId) -> Result<HashMap<String, bool>> {
        Ok((*self.cached_toggles(id)?).clone())
    }

    /// One toggle as the user set it, or `None` if they never did.
    pub fn toggle(&self, id: &DeviceId, toggle: &str) -> Result<Option<bool>> {
        Ok(self.cached_toggles(id)?.get(toggle).copied())
    }

    fn cached_toggles(&self, id: &DeviceId) -> Result<Arc<HashMap<String, bool>>> {
        if let Some(toggles) = self.toggle_cache().get(id) {
            return Ok(toggles.clone());
        }
        self.with(|c| {
            let mut stmt = c.prepare_cached("SELECT toggle, enabled FROM peer_toggles WHERE peer = ?1")?;
            let toggles: HashMap<String, bool> = stmt
                .query_map([id.as_bytes().as_slice()], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? != 0))
                })?
                .collect::<rusqlite::Result<_>>()?;
            let toggles = Arc::new(toggles);
            self.toggle_cache().insert(*id, toggles.clone());
            Ok(toggles)
        })
    }

    pub fn set_toggle(&self, id: &DeviceId, toggle: &str, enabled: bool) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO peer_toggles (peer, toggle, enabled) VALUES (?1, ?2, ?3)
                 ON CONFLICT(peer, toggle) DO UPDATE SET enabled = excluded.enabled",
                params![id.as_bytes().as_slice(), toggle, i64::from(enabled)],
            )?;
            self.toggle_cache().remove(id);
            Ok(())
        })
    }

    /// Removes a device. Returns whether it existed.
    /// The device alone; the app unpairs with [`Store::remove_peer_with_queue`].
    #[cfg(test)]
    pub fn remove_peer(&self, id: &DeviceId) -> Result<bool> {
        let peer_str = id.to_string();
        self.with(|c| {
            let _ = c.execute("DELETE FROM chat_messages WHERE peer = ?1", [&peer_str]);
            let _ = c.execute("DELETE FROM chat_threads WHERE peer = ?1", [&peer_str]);
            let removed = c.execute("DELETE FROM peers WHERE id = ?1", [id.as_bytes().as_slice()])? > 0;
            // Its toggles went with it.
            self.toggle_cache().remove(id);
            Ok(removed)
        })
    }

    /// Removes a device and its send queue together, so unpairing never drops
    /// the queue but keeps the device (or the reverse). Returns whether it
    /// existed and the queue items that went with it.
    pub fn remove_peer_with_queue(&self, id: &DeviceId) -> Result<(bool, Vec<(String, QueuePayload)>)> {
        let peer_str = id.to_string();
        let (existed, rows) = self.with(|c| {
            let tx = c.unchecked_transaction()?;
            let rows: Vec<(String, String)> = {
                let mut stmt =
                    tx.prepare_cached("SELECT id, payload_json FROM send_queue WHERE peer_id = ?1")?;
                stmt.query_map([&peer_str], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            };
            tx.execute("DELETE FROM send_queue WHERE peer_id = ?1", [&peer_str])?;
            tx.execute("DELETE FROM chat_messages WHERE peer = ?1", [&peer_str])?;
            tx.execute("DELETE FROM chat_threads WHERE peer = ?1", [&peer_str])?;
            let existed = tx.execute("DELETE FROM peers WHERE id = ?1", [id.as_bytes().as_slice()])? > 0;
            tx.commit()?;
            Ok((existed, rows))
        })?;
        // Its toggles went with it.
        self.toggle_cache().remove(id);
        let dropped: Vec<(String, QueuePayload)> = rows
            .into_iter()
            .filter_map(|(id, json)| self.queue_seal.payload(&json).map(|p| (id, p)))
            .collect();
        // After the commit: files are only removed once nothing refers to them.
        for (id, payload) in &dropped {
            cleanup_staged_payload(self.data_dir.as_deref(), id, Some(payload));
        }
        Ok((existed, dropped))
    }

    pub fn get_peer(&self, id: &DeviceId) -> Result<Option<PeerRecord>> {
        self.with(|c| {
            c.query_row(&format!("{SELECT} WHERE id = ?1"), [id.as_bytes().as_slice()], row_to_peer)
                .optional()
        })
    }

    pub fn is_paired(&self, id: &DeviceId) -> Result<bool> {
        Ok(self.get_peer(id)?.is_some())
    }

    pub fn list_peers(&self) -> Result<Vec<PeerRecord>> {
        self.with(|c| {
            let mut stmt = c.prepare_cached(&format!("{SELECT} ORDER BY paired_at"))?;
            stmt.query_map([], row_to_peer)?.collect()
        })
    }

    pub(crate) fn relay_settings(&self) -> Result<crate::relays::RelaySettings> {
        let json: Option<String> = self.with(|c| {
            c.query_row("SELECT value FROM meta WHERE key = 'relay_settings'", [], |r| r.get(0)).optional()
        })?;
        json.map(|json| serde_json::from_str(&json).map_err(|e| Error::Storage(e.to_string())))
            .transpose()
            .map(Option::unwrap_or_default)
    }

    pub(crate) fn save_relay_settings(&self, settings: &crate::relays::RelaySettings) -> Result<()> {
        let json = serde_json::to_string(settings).map_err(|e| Error::Storage(e.to_string()))?;
        self.with(|c| {
            c.execute(
                "INSERT INTO meta (key, value) VALUES ('relay_settings', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [json],
            )?;
            Ok(())
        })
    }

    // ---- Timeline ----

    pub fn timeline_retention(&self) -> Result<TimelineRetention> {
        self.with(read_retention_conn)
    }

    pub fn set_timeline_retention(&self, retention: TimelineRetention, now_unix: i64) -> Result<()> {
        self.with(|c| {
            let tx = c.unchecked_transaction()?;
            tx.execute(
                "INSERT INTO meta (key, value) VALUES ('timeline_max_days', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![retention.max_days.to_string()],
            )?;
            tx.execute(
                "INSERT INTO meta (key, value) VALUES ('timeline_max_entries', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![retention.max_entries.to_string()],
            )?;
            purge_timeline_conn(&tx, now_unix, retention)?;
            tx.commit()?;
            Ok(())
        })
    }

    pub fn insert_timeline(&self, entry: &NewTimelineEntry, now_unix: i64) -> Result<i64> {
        self.with(|c| {
            let tx = c.unchecked_transaction()?;
            let retention = read_retention_conn(&tx)?;
            tx.execute(
                "INSERT INTO timeline (
                    kind, device_id, device_name, incoming, timestamp,
                    title, detail, target, size_bytes, duration_secs, ref_id
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    entry.kind.as_str(),
                    entry.device_id.as_bytes().as_slice(),
                    entry.device_name,
                    i64::from(entry.incoming),
                    entry.timestamp,
                    entry.title,
                    entry.detail,
                    entry.target,
                    entry.size_bytes.min(i64::MAX as u64) as i64,
                    entry.duration_secs.min(i64::MAX as u64) as i64,
                    entry.ref_id,
                ],
            )?;
            let id = tx.last_insert_rowid();
            purge_timeline_conn(&tx, now_unix, retention)?;
            tx.commit()?;
            Ok(id)
        })
    }

    pub fn update_timeline_by_ref(
        &self,
        ref_id: &str,
        kind: TimelineKind,
        title: &str,
        detail: &str,
        target: &str,
        size_bytes: u64,
    ) -> Result<bool> {
        self.with(|c| {
            c.execute(
                "UPDATE timeline SET kind = ?2, title = ?3, detail = ?4, target = ?5, size_bytes = ?6
                 WHERE ref_id = ?1",
                params![ref_id, kind.as_str(), title, detail, target, size_bytes.min(i64::MAX as u64) as i64],
            )
            .map(|n| n > 0)
        })
    }

    pub fn update_timeline_kind_by_ref(&self, ref_id: &str, kind: TimelineKind) -> Result<bool> {
        self.with(|c| {
            c.execute("UPDATE timeline SET kind = ?2 WHERE ref_id = ?1", params![ref_id, kind.as_str()])
                .map(|n| n > 0)
        })
    }

    pub fn update_timeline_duration(&self, id: i64, duration_secs: u64) -> Result<bool> {
        self.with(|c| {
            c.execute(
                "UPDATE timeline SET duration_secs = ?2 WHERE id = ?1",
                params![id, duration_secs.min(i64::MAX as u64) as i64],
            )
            .map(|n| n > 0)
        })
    }

    pub fn delete_timeline(&self, id: i64) -> Result<bool> {
        self.with(|c| c.execute("DELETE FROM timeline WHERE id = ?1", [id]).map(|n| n > 0))
    }

    pub fn delete_timeline_by_ref(&self, ref_id: &str) -> Result<bool> {
        self.with(|c| c.execute("DELETE FROM timeline WHERE ref_id = ?1", [ref_id]).map(|n| n > 0))
    }

    pub fn delete_timeline_clips(&self) -> Result<usize> {
        self.with(|c| c.execute("DELETE FROM timeline WHERE kind = 'clip'", []))
    }

    pub fn clear_timeline(&self) -> Result<usize> {
        self.with(|c| c.execute("DELETE FROM timeline", []))
    }

    pub fn get_timeline(&self, id: i64) -> Result<Option<TimelineEntry>> {
        self.with(|c| {
            c.query_row(&format!("{SELECT_TIMELINE} WHERE id = ?1"), [id], row_to_timeline).optional()
        })
    }

    pub fn query_timeline(
        &self,
        query: &TimelineQuery,
        matching_clip_refs: &[String],
    ) -> Result<TimelinePage> {
        let limit = if query.limit == 0 {
            DEFAULT_TIMELINE_PAGE_LIMIT
        } else {
            query.limit.min(MAX_TIMELINE_PAGE_LIMIT)
        };
        let search_trimmed = query.search.as_deref().map(str::trim).filter(|s| !s.is_empty());
        self.with(|c| {
            let mut clauses: Vec<String> = Vec::new();
            let mut values: Vec<rusqlite::types::Value> = Vec::new();

            if let Some(kind) = query.kind {
                values.push(rusqlite::types::Value::Text(kind.as_str().to_owned()));
                clauses.push(format!("kind = ?{}", values.len()));
            }
            if let Some(device) = &query.device {
                values.push(rusqlite::types::Value::Blob(device.as_bytes().to_vec()));
                clauses.push(format!("device_id = ?{}", values.len()));
            }
            if let Some(search) = search_trimmed {
                let pattern = format!("%{}%", escape_like(&search.to_lowercase()));
                values.push(rusqlite::types::Value::Text(pattern));
                let idx = values.len();
                let mut search_or = format!(
                    "(lower(title) LIKE ?{idx} ESCAPE '\\' \
                     OR lower(detail) LIKE ?{idx} ESCAPE '\\' \
                     OR lower(device_name) LIKE ?{idx} ESCAPE '\\'"
                );
                if !matching_clip_refs.is_empty() {
                    let mut placeholders = Vec::with_capacity(matching_clip_refs.len());
                    for r in matching_clip_refs {
                        values.push(rusqlite::types::Value::Text(r.clone()));
                        placeholders.push(format!("?{}", values.len()));
                    }
                    search_or.push_str(&format!(" OR ref_id IN ({})", placeholders.join(", ")));
                }
                search_or.push(')');
                clauses.push(search_or);
            }

            let where_sql = if clauses.is_empty() {
                String::new()
            } else {
                format!(" WHERE {}", clauses.join(" AND "))
            };

            let total_sql = format!("SELECT COUNT(*) FROM timeline{where_sql}");
            let total: i64 = c.query_row(
                &total_sql,
                rusqlite::params_from_iter(values.iter()),
                |r| r.get(0),
            )?;
            let total = u32::try_from(total.max(0)).unwrap_or(u32::MAX);

            values.push(rusqlite::types::Value::Integer(i64::from(limit)));
            let limit_idx = values.len();
            values.push(rusqlite::types::Value::Integer(i64::from(query.offset)));
            let offset_idx = values.len();

            let page_sql = format!(
                "{SELECT_TIMELINE}{where_sql} ORDER BY timestamp DESC, id DESC LIMIT ?{limit_idx} OFFSET ?{offset_idx}"
            );
            let mut stmt = c.prepare_cached(&page_sql)?;
            let entries: Vec<TimelineEntry> = stmt
                .query_map(rusqlite::params_from_iter(values.iter()), row_to_timeline)?
                .collect::<rusqlite::Result<Vec<_>>>()?;

            let loaded = query.offset.saturating_add(entries.len() as u32);
            Ok(TimelinePage {
                entries,
                total,
                has_more: loaded < total,
            })
        })
    }

    // ---- Unified conversations (`MessagingStyle` notifications) ----

    pub fn upsert_chat_notification(
        &self,
        peer: &DeviceId,
        n: &Notification,
        app_icon: Option<&[u8]>,
        is_snapshot: bool,
    ) -> Result<Option<String>> {
        let Some(conv) = &n.conversation else {
            return Ok(None);
        };
        if conv.messages.is_empty() || conv.title.trim().is_empty() {
            return Ok(None);
        };
        let thread_id = chat_thread_id(&n.app, &conv.title);
        let reply_action_id = n.actions.iter().find(|a| a.reply).map(|a| a.id.clone());
        let Some(latest) = conv.messages.last() else {
            return Ok(None);
        };
        let snippet = latest.text.clone();
        let last_sender = if latest.self_sent { None } else { latest.sender.clone() };
        let updated_ms = conv.messages.iter().map(|m| m.time).max().unwrap_or(n.when).max(n.when);
        let peer_str = peer.to_string();
        let icon_bytes = app_icon.or(n.icon.as_deref());

        self.with(|c| {
            let tx = c.unchecked_transaction()?;
            let existing: Vec<(String, String, bool)> = {
                let mut stmt = tx.prepare_cached(
                    "SELECT IFNULL(sender, ''), text, self_sent
                     FROM (
                         SELECT id, sender, text, time_ms, self_sent
                         FROM chat_messages
                         WHERE peer = ?1 AND thread_id = ?2
                         ORDER BY time_ms DESC, id DESC
                         LIMIT 50
                     )
                     ORDER BY time_ms ASC, id ASC",
                )?;
                stmt.query_map(params![peer_str, thread_id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)? != 0))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
            };

            let incoming_keys: Vec<(String, String, bool)> = conv
                .messages
                .iter()
                .map(|m| (m.sender.clone().unwrap_or_default(), m.text.clone(), m.self_sent))
                .collect();

            let mut matched_prefix = 0usize;
            for p in (1..=incoming_keys.len().min(existing.len())).rev() {
                let prefix = &incoming_keys[..p];
                if existing.windows(p).any(|w| w == prefix) {
                    matched_prefix = p;
                    break;
                }
            }

            let mut new_incoming_count = 0i64;
            for msg in conv.messages.iter().skip(matched_prefix) {
                let exists: Option<i64> = tx
                    .query_row(
                        "SELECT id FROM chat_messages
                         WHERE peer = ?1 AND thread_id = ?2
                           AND IFNULL(sender, '') = IFNULL(?3, '')
                           AND text = ?4
                           AND self_sent = ?5
                           AND (time_ms = ?6 OR (self_sent != 0 AND ABS(time_ms - ?6) <= 5000))
                         LIMIT 1",
                        params![
                            peer_str,
                            thread_id,
                            msg.sender,
                            msg.text,
                            i64::from(msg.self_sent),
                            msg.time,
                        ],
                        |r| r.get(0),
                    )
                    .optional()?;
                if exists.is_none() {
                    tx.execute(
                        "INSERT INTO chat_messages (peer, thread_id, sender, text, time_ms, self_sent, avatar)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        params![
                            peer_str,
                            thread_id,
                            msg.sender,
                            msg.text,
                            msg.time,
                            i64::from(msg.self_sent),
                            msg.avatar,
                        ],
                    )?;
                    if !msg.self_sent {
                        new_incoming_count += 1;
                    }
                }
            }

            let snapshot_unread = new_incoming_count.min(conv.messages.len() as i64);
            let initial_unread = if is_snapshot { snapshot_unread } else { new_incoming_count };
            tx.execute(
                "INSERT INTO chat_threads (
                    peer, thread_id, app, app_name, notification_key, reply_action_id,
                    title, is_group, snippet, last_sender, updated_ms, unread, active, avatar, app_icon
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 1, ?13, ?14)
                 ON CONFLICT(peer, thread_id) DO UPDATE SET
                    app = excluded.app,
                    app_name = excluded.app_name,
                    notification_key = CASE WHEN excluded.updated_ms >= chat_threads.updated_ms
                        THEN excluded.notification_key ELSE chat_threads.notification_key END,
                    reply_action_id = CASE WHEN excluded.updated_ms >= chat_threads.updated_ms
                        THEN excluded.reply_action_id ELSE chat_threads.reply_action_id END,
                    title = CASE WHEN excluded.updated_ms >= chat_threads.updated_ms
                        THEN excluded.title ELSE chat_threads.title END,
                    is_group = excluded.is_group,
                    -- An older notification (a replay after reconnecting) doesn't
                    -- replace what the newest one said.
                    snippet = CASE WHEN excluded.updated_ms >= chat_threads.updated_ms
                        THEN excluded.snippet ELSE chat_threads.snippet END,
                    last_sender = CASE WHEN excluded.updated_ms >= chat_threads.updated_ms
                        THEN excluded.last_sender ELSE chat_threads.last_sender END,
                    updated_ms = MAX(chat_threads.updated_ms, excluded.updated_ms),
                    unread = CASE
                        WHEN ?15 != 0 THEN CASE
                            WHEN ?16 > 0 THEN ?16
                            ELSE chat_threads.unread
                        END
                        ELSE chat_threads.unread + ?17
                    END,
                    active = 1,
                    avatar = COALESCE(excluded.avatar, chat_threads.avatar),
                    app_icon = COALESCE(excluded.app_icon, chat_threads.app_icon)",
                params![
                    peer_str,
                    thread_id,
                    n.app,
                    n.app_name,
                    n.key,
                    reply_action_id,
                    conv.title.trim(),
                    i64::from(conv.group),
                    snippet,
                    last_sender,
                    updated_ms,
                    initial_unread,
                    conv.avatar,
                    icon_bytes,
                    i64::from(is_snapshot),
                    snapshot_unread,
                    new_incoming_count,
                ],
            )?;

            prune_chat_conn(&tx, &peer_str, &thread_id)?;
            tx.commit()?;
            Ok(Some(thread_id))
        })
    }

    pub fn append_chat_reply(
        &self,
        peer: &DeviceId,
        thread_id: &str,
        text: &str,
        time_ms: i64,
    ) -> Result<bool> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Ok(false);
        }
        let peer_str = peer.to_string();
        self.with(|c| {
            let tx = c.unchecked_transaction()?;
            let exists_thread: Option<i64> = tx
                .query_row(
                    "SELECT 1 FROM chat_threads WHERE peer = ?1 AND thread_id = ?2",
                    params![peer_str, thread_id],
                    |r| r.get(0),
                )
                .optional()?;
            if exists_thread.is_none() {
                return Ok(false);
            }
            let dup: Option<i64> = tx
                .query_row(
                    "SELECT id FROM chat_messages
                     WHERE peer = ?1 AND thread_id = ?2 AND self_sent = 1
                       AND text = ?3 AND ABS(time_ms - ?4) <= 2000
                     LIMIT 1",
                    params![peer_str, thread_id, trimmed, time_ms],
                    |r| r.get(0),
                )
                .optional()?;
            if dup.is_none() {
                tx.execute(
                    "INSERT INTO chat_messages (peer, thread_id, sender, text, time_ms, self_sent, avatar)
                     VALUES (?1, ?2, NULL, ?3, ?4, 1, NULL)",
                    params![peer_str, thread_id, trimmed, time_ms],
                )?;
            }
            let snippet = format!("You: {trimmed}");
            tx.execute(
                "UPDATE chat_threads
                 SET snippet = ?3,
                     last_sender = NULL,
                     updated_ms = MAX(updated_ms, ?4),
                     unread = 0
                 WHERE peer = ?1 AND thread_id = ?2",
                params![peer_str, thread_id, snippet, time_ms],
            )?;
            prune_chat_messages_conn(&tx, &peer_str, thread_id)?;
            tx.commit()?;
            Ok(true)
        })
    }

    pub fn append_chat_reply_by_key(
        &self,
        peer: &DeviceId,
        notification_key: &str,
        text: &str,
        time_ms: i64,
    ) -> Result<Option<String>> {
        let peer_str = peer.to_string();
        let thread_id: Option<String> = self.with(|c| {
            c.query_row(
                "SELECT thread_id FROM chat_threads
                 WHERE peer = ?1 AND notification_key = ?2
                 ORDER BY updated_ms DESC LIMIT 1",
                params![peer_str, notification_key],
                |r| r.get(0),
            )
            .optional()
        })?;
        let Some(tid) = thread_id else {
            return Ok(None);
        };
        if self.append_chat_reply(peer, &tid, text, time_ms)? { Ok(Some(tid)) } else { Ok(None) }
    }

    pub fn mark_chat_notification_removed(
        &self,
        peer: &DeviceId,
        notification_key: &str,
    ) -> Result<Option<String>> {
        let peer_str = peer.to_string();
        self.with(|c| {
            let thread_id: Option<String> = c
                .query_row(
                    "SELECT thread_id FROM chat_threads
                     WHERE peer = ?1 AND notification_key = ?2 AND active = 1
                     ORDER BY updated_ms DESC LIMIT 1",
                    params![peer_str, notification_key],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(ref tid) = thread_id {
                c.execute(
                    "UPDATE chat_threads SET active = 0, unread = 0
                     WHERE peer = ?1 AND thread_id = ?2",
                    params![peer_str, tid],
                )?;
            }
            Ok(thread_id)
        })
    }

    pub fn chat_threads(&self, peer: &DeviceId) -> Result<Vec<ChatThreadRecord>> {
        let peer_str = peer.to_string();
        let peer_id = *peer;
        self.with(|c| {
            let mut stmt = c.prepare_cached(&format!(
                "{SELECT_CHAT_THREADS} WHERE peer = ?1 ORDER BY updated_ms DESC, thread_id ASC"
            ))?;
            stmt.query_map([&peer_str], |r| row_to_chat_thread(&peer_id, r))?.collect()
        })
    }

    pub fn chat_thread(&self, peer: &DeviceId, thread_id: &str) -> Result<Option<ChatThreadRecord>> {
        let peer_str = peer.to_string();
        let peer_id = *peer;
        self.with(|c| {
            c.query_row(
                &format!("{SELECT_CHAT_THREADS} WHERE peer = ?1 AND thread_id = ?2"),
                params![peer_str, thread_id],
                |r| row_to_chat_thread(&peer_id, r),
            )
            .optional()
        })
    }

    pub fn chat_messages(
        &self,
        peer: &DeviceId,
        thread_id: &str,
        limit: usize,
    ) -> Result<Vec<ChatMessageRecord>> {
        let peer_str = peer.to_string();
        let peer_id = *peer;
        let capped = limit.clamp(1, MAX_CHAT_MESSAGES_PER_THREAD) as i64;
        self.with(|c| {
            let mut stmt = c.prepare_cached(
                "SELECT id, thread_id, sender, text, time_ms, self_sent, avatar
                 FROM chat_messages
                 WHERE peer = ?1 AND thread_id = ?2
                 ORDER BY time_ms DESC, id DESC
                 LIMIT ?3",
            )?;
            stmt.query_map(params![peer_str, thread_id, capped], |r| row_to_chat_message(&peer_id, r))?
                .collect()
        })
    }

    pub fn mark_chat_thread_read(&self, peer: &DeviceId, thread_id: &str) -> Result<bool> {
        let peer_str = peer.to_string();
        self.with(|c| {
            c.execute(
                "UPDATE chat_threads SET unread = 0 WHERE peer = ?1 AND thread_id = ?2 AND unread != 0",
                params![peer_str, thread_id],
            )
            .map(|n| n > 0)
        })
    }

    pub fn delete_chat_thread(&self, peer: &DeviceId, thread_id: &str) -> Result<bool> {
        let peer_str = peer.to_string();
        self.with(|c| {
            let tx = c.unchecked_transaction()?;
            tx.execute(
                "DELETE FROM chat_messages WHERE peer = ?1 AND thread_id = ?2",
                params![peer_str, thread_id],
            )?;
            let removed = tx.execute(
                "DELETE FROM chat_threads WHERE peer = ?1 AND thread_id = ?2",
                params![peer_str, thread_id],
            )? > 0;
            tx.commit()?;
            Ok(removed)
        })
    }

    pub fn clear_chat_threads(&self) -> Result<usize> {
        self.with(|c| {
            let tx = c.unchecked_transaction()?;
            tx.execute("DELETE FROM chat_messages", [])?;
            let removed = tx.execute("DELETE FROM chat_threads", [])?;
            tx.commit()?;
            Ok(removed)
        })
    }

    pub fn delete_timeline_kind(&self, kind: TimelineKind) -> Result<u64> {
        self.with(|c| c.execute("DELETE FROM timeline WHERE kind = ?1", [kind.as_str()]).map(|n| n as u64))
    }

    pub fn timeline_count_by_kind(&self, kind: TimelineKind) -> Result<u32> {
        self.with(|c| {
            let n: i64 =
                c.query_row("SELECT COUNT(*) FROM timeline WHERE kind = ?1", [kind.as_str()], |r| r.get(0))?;
            Ok(u32::try_from(n.max(0)).unwrap_or(u32::MAX))
        })
    }

    pub fn timeline_total_count(&self) -> Result<u32> {
        self.with(|c| {
            let n: i64 = c.query_row("SELECT COUNT(*) FROM timeline", [], |r| r.get(0))?;
            Ok(u32::try_from(n.max(0)).unwrap_or(u32::MAX))
        })
    }

    pub fn chat_cache_counts(&self) -> Result<(u32, u32)> {
        self.with(|c| {
            let threads: i64 = c.query_row("SELECT COUNT(*) FROM chat_threads", [], |r| r.get(0))?;
            let msgs: i64 = c.query_row("SELECT COUNT(*) FROM chat_messages", [], |r| r.get(0))?;
            Ok((
                u32::try_from(threads.max(0)).unwrap_or(u32::MAX),
                u32::try_from(msgs.max(0)).unwrap_or(u32::MAX),
            ))
        })
    }

    pub fn clear_all_history_and_cache(&self) -> Result<()> {
        self.with(|c| {
            let tx = c.unchecked_transaction()?;
            tx.execute("DELETE FROM timeline", [])?;
            tx.execute("DELETE FROM chat_messages", [])?;
            tx.execute("DELETE FROM chat_threads", [])?;
            tx.commit()?;
            let _ = c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
            Ok(())
        })
    }

    // ---- Persistent Outgoing Send Queue ----

    /// On node startup, resets any items left in `'sending'` state back to `'waiting'`
    /// (`next_retry_ms = 0`) so they resume cleanly when the peer reconnects.
    pub fn reset_interrupted_sending(&self) -> Result<usize> {
        self.with(|c| {
            c.execute(
                "UPDATE send_queue
                 SET status = 'waiting', next_retry_ms = 0, error_msg = NULL
                 WHERE status = 'sending'",
                [],
            )
        })
    }

    /// Resets `next_retry_ms = 0` on all `'waiting'` queue items for `peer` when the peer disconnects.
    pub fn clear_send_queue_retry_delays(&self, peer: &DeviceId) -> Result<usize> {
        let peer_str = peer.to_string();
        self.with(|c| {
            c.execute(
                "UPDATE send_queue
                 SET next_retry_ms = 0
                 WHERE peer_id = ?1 AND status = 'waiting' AND next_retry_ms != 0",
                [&peer_str],
            )
        })
    }

    /// Reassigns queued outgoing items from `from` to `to` when deduplicating a re-paired device.
    pub fn reassign_send_queue_peer(&self, from: &DeviceId, to: &DeviceId) -> Result<usize> {
        let from_str = from.to_string();
        let to_str = to.to_string();
        self.with(|c| {
            c.execute("UPDATE send_queue SET peer_id = ?1 WHERE peer_id = ?2", params![to_str, from_str])
        })
    }

    /// Enqueues a new outgoing item for `peer` in `send_queue`.
    ///
    /// - Enforces `MAX_QUEUE_ACTIVE_ITEMS` (200), `MAX_QUEUE_ACTIVE_BYTES` (10 GiB), and
    ///   `MAX_QUEUE_CLIPBOARD_IMAGE_BYTES` (50 MiB) across active (`waiting` + `sending`) items,
    ///   returning `Error::QueueFull` if exceeded.
    /// - If `payload` is `Clipboard` or `ClipboardImage` and there is already a `waiting`
    ///   clipboard item for `peer`, replaces it and assigns a new `seq = MAX(seq) + 1` so it
    ///   moves to the end of the queue.
    pub fn enqueue_send_item(
        &self,
        id: &str,
        peer: &DeviceId,
        payload: &QueuePayload,
        payload_blob: Option<&[u8]>,
        now_ms: i64,
    ) -> Result<QueueItem> {
        let kind = payload.kind();
        if kind == QueueKind::ClipboardImage {
            let blob_len = payload_blob.map_or(0, <[u8]>::len);
            if blob_len > MAX_QUEUE_CLIPBOARD_IMAGE_BYTES
                || bytes_from_len(blob_len) > MAX_QUEUE_CLIPBOARD_IMAGE_BYTES as u64
            {
                return Err(Error::QueueFull(
                    "Clipboard image exceeds the 50 MB offline queue limit.".into(),
                ));
            }
        }
        let ttl_ms = kind.ttl_ms();
        let expires_at = now_ms.saturating_add(ttl_ms);
        let (title, detail) = payload.title_and_detail();
        let (bytes_total, files_total) = payload.totals(payload_blob);
        let payload_json = serde_json::to_string(payload).map_err(|e| Error::Internal(e.to_string()))?;
        // Sealed content goes to disk under a plain label: the real title is
        // worked out from the payload when it's read back.
        let sealed_blob;
        let (stored_title, stored_detail, stored_json, stored_blob) = if kind.is_sealed() {
            let (t, d) = payload.stored_title_and_detail();
            sealed_blob = payload_blob.map(|b| self.queue_seal.seal(b)).transpose()?;
            (t, d, self.queue_seal.seal_json(&payload_json)?, sealed_blob.as_deref())
        } else {
            (title.clone(), detail.clone(), payload_json, payload_blob)
        };
        let peer_str = peer.to_string();
        let files = payload.files_vec();

        let conn = self.conn.lock().map_err(|_| Error::Internal("store lock poisoned".into()))?;
        let tx = conn.unchecked_transaction()?;
        let peer_name = lookup_peer_name_conn(&tx, peer);

        if kind.is_clipboard() {
            let existing: Option<(String, i64)> = tx
                .query_row(
                    "SELECT id, seq FROM send_queue
                     WHERE peer_id = ?1 AND status = 'waiting'
                       AND kind IN ('clipboard', 'clipboard_image')
                     ORDER BY seq ASC LIMIT 1",
                    [&peer_str],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((old_id, _old_seq)) = existing {
                tx.execute(
                    "DELETE FROM send_queue
                     WHERE peer_id = ?1 AND status = 'waiting'
                       AND kind IN ('clipboard', 'clipboard_image')
                       AND id != ?2",
                    params![peer_str, old_id],
                )?;
                if kind == QueueKind::ClipboardImage {
                    let active_file_bytes: i64 = tx.query_row(
                        "SELECT COALESCE(SUM(bytes_total), 0) FROM send_queue
                         WHERE status IN ('waiting', 'sending')
                           AND kind IN ('files', 'folder', 'handoff_document', 'recording', 'clipboard_image')
                           AND id != ?1",
                        [&old_id],
                        |r| r.get(0),
                    )?;
                    let current_bytes = u64::try_from(active_file_bytes.max(0)).unwrap_or(u64::MAX);
                    if current_bytes.saturating_add(bytes_total) > MAX_QUEUE_ACTIVE_BYTES {
                        return Err(Error::QueueFull("Queue is full (10 GB of files waiting).".into()));
                    }
                }
                let new_seq: i64 =
                    tx.query_row("SELECT COALESCE(MAX(seq), 0) + 1 FROM send_queue", [], |r| r.get(0))?;
                tx.execute(
                    "UPDATE send_queue
                     SET id = ?1, seq = ?2, kind = ?3, created_at = ?4, expires_at = ?5,
                         next_retry_ms = 0, attempts = 0, title = ?6, detail = ?7,
                         payload_json = ?8, payload_blob = ?9,
                         bytes_total = ?10, bytes_done = 0, files_total = 0, files_done = 0,
                         error_msg = NULL
                     WHERE id = ?11",
                    params![
                        id,
                        new_seq,
                        kind.as_str(),
                        now_ms,
                        expires_at,
                        stored_title,
                        stored_detail,
                        stored_json,
                        stored_blob,
                        bytes_total as i64,
                        old_id
                    ],
                )?;
                tx.commit()?;
                return Ok(QueueItem {
                    id: id.to_owned(),
                    seq: new_seq,
                    peer: *peer,
                    peer_name,
                    kind,
                    status: QueueStatus::Waiting,
                    created_at: now_ms,
                    expires_at,
                    attempts: 0,
                    title,
                    detail,
                    bytes_total,
                    bytes_done: 0,
                    files_total: 0,
                    files_done: 0,
                    files: Vec::new(),
                });
            }
        }

        let active_count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM send_queue WHERE status IN ('waiting', 'sending')",
            [],
            |r| r.get(0),
        )?;
        if usize::try_from(active_count.max(0)).unwrap_or(usize::MAX) >= MAX_QUEUE_ACTIVE_ITEMS {
            return Err(Error::QueueFull(format!(
                "Queue is full ({MAX_QUEUE_ACTIVE_ITEMS} items waiting). Send or clear existing queued items first."
            )));
        }

        if kind.is_file_based() || kind == QueueKind::ClipboardImage {
            let active_file_bytes: i64 = tx.query_row(
                "SELECT COALESCE(SUM(bytes_total), 0) FROM send_queue
                 WHERE status IN ('waiting', 'sending')
                   AND kind IN ('files', 'folder', 'handoff_document', 'recording', 'clipboard_image')",
                [],
                |r| r.get(0),
            )?;
            let current_bytes = u64::try_from(active_file_bytes.max(0)).unwrap_or(u64::MAX);
            if current_bytes.saturating_add(bytes_total) > MAX_QUEUE_ACTIVE_BYTES {
                return Err(Error::QueueFull("Queue is full (10 GB of files waiting).".into()));
            }
        }

        let seq: i64 = tx.query_row("SELECT COALESCE(MAX(seq), 0) + 1 FROM send_queue", [], |r| r.get(0))?;
        tx.execute(
            "INSERT INTO send_queue (
                id, seq, peer_id, kind, status, created_at, expires_at, next_retry_ms, attempts,
                title, detail, payload_json, payload_blob, bytes_total, bytes_done, files_total, files_done, error_msg
            ) VALUES (?1, ?2, ?3, ?4, 'waiting', ?5, ?6, 0, 0, ?7, ?8, ?9, ?10, ?11, 0, ?12, 0, NULL)",
            params![
                id,
                seq,
                peer_str,
                kind.as_str(),
                now_ms,
                expires_at,
                stored_title,
                stored_detail,
                stored_json,
                stored_blob,
                bytes_total as i64,
                i64::from(files_total),
            ],
        )?;
        tx.commit()?;

        Ok(QueueItem {
            id: id.to_owned(),
            seq,
            peer: *peer,
            peer_name,
            kind,
            status: QueueStatus::Waiting,
            created_at: now_ms,
            expires_at,
            attempts: 0,
            title,
            detail,
            bytes_total,
            bytes_done: 0,
            files_total,
            files_done: 0,
            files,
        })
    }

    /// Marks any `'waiting'` queue items whose `expires_at <= now_ms` as `'expired'`
    /// with the appropriate per-type expiry message and deletes any staged files.
    /// Returns how many items expired.
    pub fn expire_stale_queue_items(&self, now_ms: i64) -> Result<usize> {
        // Staged files stay, as for a failed item, so Retry still has them;
        // clearing finished items removes them.
        let expired: Vec<String> = self.with(|c| {
            let mut stmt = c.prepare_cached(
                "SELECT id, kind FROM send_queue WHERE status = 'waiting' AND expires_at <= ?1",
            )?;
            let stale: Vec<(String, String)> = stmt
                .query_map([now_ms], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if stale.is_empty() {
                return Ok(Vec::new());
            }
            let tx = c.unchecked_transaction()?;
            let mut out = Vec::with_capacity(stale.len());
            for (id, kind_str) in stale {
                let kind = QueueKind::from_str_opt(&kind_str).unwrap_or(QueueKind::Files);
                // Nothing keeps a clip or link past its deadline, not even for Retry.
                if kind.is_sealed() {
                    if tx.execute("DELETE FROM send_queue WHERE id = ?1", [&id])? > 0 {
                        out.push(id);
                    }
                    continue;
                }
                let msg = kind.expiry_error_message();
                let updated = tx.execute(
                    "UPDATE send_queue
                     SET status = 'expired', next_retry_ms = 0, error_msg = ?2
                     WHERE id = ?1 AND status = 'waiting'",
                    params![id, msg],
                )?;
                if updated > 0 {
                    out.push(id);
                }
            }
            tx.commit()?;
            Ok(out)
        })?;
        Ok(expired.len())
    }

    /// Lists items in `send_queue` (for `peer` when `Some`, or all peers when `None`),
    /// ordered by `seq ASC`.
    pub fn list_send_queue(&self, peer: Option<&DeviceId>) -> Result<Vec<QueueItem>> {
        self.with(|c| {
            let rows: Vec<RawQueueRow> = if let Some(p) = peer {
                let peer_str = p.to_string();
                let mut stmt =
                    c.prepare_cached(&format!("{SELECT_SEND_QUEUE} WHERE peer_id = ?1 ORDER BY seq ASC"))?;
                let mapped = stmt.query_map([&peer_str], raw_queue_row)?;
                mapped.collect::<rusqlite::Result<Vec<_>>>()?
            } else {
                let mut stmt = c.prepare_cached(&format!("{SELECT_SEND_QUEUE} ORDER BY seq ASC"))?;
                let mapped = stmt.query_map([], raw_queue_row)?;
                mapped.collect::<rusqlite::Result<Vec<_>>>()?
            };
            let mut out = Vec::with_capacity(rows.len());
            for raw in rows {
                if let Some((item, _, _, _)) = decode_queue_row(c, &self.queue_seal, raw, false) {
                    out.push(item);
                }
            }
            Ok(out)
        })
    }

    /// Returns whether `peer` has any `'waiting'` or `'sending'` items in `send_queue`.
    pub fn has_active_queue_for_peer(&self, peer: &DeviceId) -> Result<bool> {
        let peer_str = peer.to_string();
        self.with(|c| {
            let count: i64 = c.query_row(
                "SELECT COUNT(*) FROM send_queue WHERE peer_id = ?1 AND status IN ('waiting', 'sending')",
                [&peer_str],
                |r| r.get(0),
            )?;
            Ok(count > 0)
        })
    }

    /// Returns the head-of-line (`seq ASC LIMIT 1`) `'waiting'` item for `peer` along with its
    /// decoded `QueuePayload`, optional `payload_blob`, and `next_retry_ms`.
    pub fn head_waiting_item_for_peer(&self, peer: &DeviceId) -> Result<Option<DecodedQueueRow>> {
        let peer_str = peer.to_string();
        self.with(|c| {
            let raw = c
                .query_row(
                    &format!(
                        "{SELECT_SEND_QUEUE_WITH_BLOB}
                         WHERE peer_id = ?1 AND status = 'waiting'
                         ORDER BY seq ASC LIMIT 1"
                    ),
                    [&peer_str],
                    raw_queue_row_with_blob,
                )
                .optional()?;
            Ok(raw.and_then(|r| decode_queue_row(c, &self.queue_seal, r, true)))
        })
    }

    /// Returns a specific queue item by `id` along with its `QueuePayload`, `payload_blob`, and `next_retry_ms`.
    pub fn get_queue_item(&self, id: &str) -> Result<Option<DecodedQueueRow>> {
        self.with(|c| {
            let raw = c
                .query_row(
                    &format!("{SELECT_SEND_QUEUE_WITH_BLOB} WHERE id = ?1"),
                    [id],
                    raw_queue_row_with_blob,
                )
                .optional()?;
            Ok(raw.and_then(|r| decode_queue_row(c, &self.queue_seal, r, true)))
        })
    }

    /// Distinct peers that currently have at least one `'waiting'` item in `send_queue`.
    pub fn peers_with_waiting_queue(&self) -> Result<Vec<DeviceId>> {
        self.with(|c| {
            let mut stmt = c.prepare_cached(
                "SELECT DISTINCT peer_id FROM send_queue WHERE status = 'waiting' ORDER BY seq ASC",
            )?;
            let peer_strs: Vec<String> =
                stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(peer_strs.into_iter().filter_map(|s| s.parse::<DeviceId>().ok()).collect())
        })
    }

    /// Computes the earliest wakeup timestamp (`expires_at` across all waiting items, or
    /// `next_retry_ms` across waiting items for `online_peers`), or `None` if no items are waiting.
    pub fn next_queue_wake_ms(&self, online_peers: &[DeviceId]) -> Result<Option<i64>> {
        self.with(|c| {
            let min_expire: Option<i64> = c
                .query_row(
                    "SELECT MIN(expires_at) FROM send_queue WHERE status = 'waiting'",
                    [],
                    |r| r.get(0),
                )
                .optional()?
                .flatten();
            let min_retry: Option<i64> = if online_peers.is_empty() {
                None
            } else {
                let mut stmt = c.prepare_cached(
                    "SELECT peer_id, next_retry_ms FROM send_queue WHERE status = 'waiting' AND next_retry_ms > 0",
                )?;
                let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
                let online_strs: BTreeSet<String> = online_peers.iter().map(ToString::to_string).collect();
                let mut best: Option<i64> = None;
                for row in rows {
                    let (peer_str, retry_ms) = row?;
                    if online_strs.contains(&peer_str) {
                        best = Some(best.map_or(retry_ms, |b| b.min(retry_ms)));
                    }
                }
                best
            };
            Ok(match (min_expire, min_retry) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(b),
                (None, None) => None,
            })
        })
    }

    /// Transitions a queue item from `'waiting'` to `'sending'`.
    pub fn mark_queue_sending(&self, id: &str) -> Result<bool> {
        self.with(|c| {
            // Only a live item: one that just expired or failed stays that way.
            c.execute(
                "UPDATE send_queue SET status = 'sending', error_msg = NULL
                 WHERE id = ?1 AND status IN ('waiting', 'sending')",
                [id],
            )
            .map(|n| n > 0)
        })
    }

    /// Updates byte and file progress for an in-flight queue item.
    pub fn update_queue_progress(&self, id: &str, bytes_done: u64, files_done: u32) -> Result<bool> {
        self.with(|c| {
            c.execute(
                "UPDATE send_queue SET bytes_done = ?2, files_done = ?3 WHERE id = ?1",
                params![id, bytes_done as i64, i64::from(files_done)],
            )
            .map(|n| n > 0)
        })
    }

    /// Removes a completed queue item from `send_queue` and returns its decoded `QueuePayload`
    /// after cleaning up any staged files.
    pub fn complete_queue_item(&self, id: &str) -> Result<Option<QueuePayload>> {
        let payload = self.with(|c| {
            let payload_json: Option<String> = c
                .query_row("SELECT payload_json FROM send_queue WHERE id = ?1", [id], |r| r.get(0))
                .optional()?;
            c.execute("DELETE FROM send_queue WHERE id = ?1", [id])?;
            Ok(payload_json.and_then(|j| self.queue_seal.payload(&j)))
        })?;
        cleanup_staged_payload(self.data_dir.as_deref(), id, payload.as_ref());
        Ok(payload)
    }

    /// Transitions a queue item back to `'waiting'` after a disconnect or transient error,
    /// recording `attempts` and `next_retry_ms`.
    pub fn mark_queue_retry_or_wait(
        &self,
        id: &str,
        attempts: u32,
        next_retry_ms: i64,
        last_error: Option<&str>,
    ) -> Result<bool> {
        self.with(|c| {
            c.execute(
                "UPDATE send_queue
                 SET status = 'waiting', attempts = ?2, next_retry_ms = ?3, error_msg = ?4
                 WHERE id = ?1 AND status IN ('waiting', 'sending')",
                params![id, i64::from(attempts), next_retry_ms, last_error],
            )
            .map(|n| n > 0)
        })
    }

    /// Marks a queue item as permanently `'failed'` with a human-readable `error` reason.
    pub fn mark_queue_failed(&self, id: &str, error: &str) -> Result<bool> {
        self.with(|c| {
            c.execute(
                "UPDATE send_queue
                 SET status = 'failed', next_retry_ms = 0, error_msg = ?2
                 WHERE id = ?1",
                params![id, error],
            )
            .map(|n| n > 0)
        })
    }

    /// Resets a `'failed'` or `'expired'` queue item back to `'waiting'` with `attempts = 0`,
    /// `next_retry_ms = 0`, and a fresh `expires_at` window starting at `now_ms`.
    pub fn retry_queue_item(&self, id: &str, now_ms: i64) -> Result<bool> {
        let conn = self.conn.lock().map_err(|_| Error::Internal("store lock poisoned".into()))?;
        let tx = conn.unchecked_transaction()?;
        let row: Option<(String, String, String, i64)> = tx
            .query_row("SELECT peer_id, kind, status, bytes_total FROM send_queue WHERE id = ?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .optional()?;
        let Some((peer_str, kind_str, status_str, bytes_total_raw)) = row else {
            return Ok(false);
        };
        if status_str != "failed" && status_str != "expired" {
            return Ok(false);
        }
        let kind = QueueKind::from_str_opt(&kind_str).unwrap_or(QueueKind::Files);
        let bytes_total = u64::try_from(bytes_total_raw.max(0)).unwrap_or(0);
        if kind.is_clipboard() {
            tx.execute(
                "DELETE FROM send_queue
                 WHERE peer_id = ?1 AND status = 'waiting'
                   AND kind IN ('clipboard', 'clipboard_image')
                   AND id != ?2",
                params![peer_str, id],
            )?;
        }
        let active_count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM send_queue WHERE status IN ('waiting', 'sending')",
            [],
            |r| r.get(0),
        )?;
        if usize::try_from(active_count.max(0)).unwrap_or(usize::MAX) >= MAX_QUEUE_ACTIVE_ITEMS {
            return Err(Error::QueueFull(format!(
                "Queue is full ({MAX_QUEUE_ACTIVE_ITEMS} items waiting). Send or clear existing queued items first."
            )));
        }
        if kind.is_file_based() || kind == QueueKind::ClipboardImage {
            let active_file_bytes: i64 = tx.query_row(
                "SELECT COALESCE(SUM(bytes_total), 0) FROM send_queue
                 WHERE status IN ('waiting', 'sending')
                   AND kind IN ('files', 'folder', 'handoff_document', 'recording', 'clipboard_image')",
                [],
                |r| r.get(0),
            )?;
            let current_bytes = u64::try_from(active_file_bytes.max(0)).unwrap_or(u64::MAX);
            if current_bytes.saturating_add(bytes_total) > MAX_QUEUE_ACTIVE_BYTES {
                return Err(Error::QueueFull(
                    "Queue is full (10 GB of files waiting). Send or clear existing queued items first."
                        .into(),
                ));
            }
        }
        let expires_at = now_ms.saturating_add(kind.ttl_ms());
        let updated = tx.execute(
            "UPDATE send_queue
             SET status = 'waiting', attempts = 0, next_retry_ms = 0,
                 expires_at = ?2, error_msg = NULL, bytes_done = 0, files_done = 0
             WHERE id = ?1",
            params![id, expires_at],
        )?;
        tx.commit()?;
        Ok(updated > 0)
    }

    /// Resets all `'failed'` and `'expired'` items (for `peer` when `Some`, or all peers when `None`)
    /// back to `'waiting'` with fresh TTLs. Returns the count of reset items.
    pub fn retry_all_failed_queue_items(&self, peer: Option<&DeviceId>, now_ms: i64) -> Result<u32> {
        let conn = self.conn.lock().map_err(|_| Error::Internal("store lock poisoned".into()))?;
        let rows: Vec<(String, String, String, i64)> = if let Some(p) = peer {
            let peer_str = p.to_string();
            let mut stmt = conn.prepare_cached(
                "SELECT id, peer_id, kind, bytes_total FROM send_queue
                 WHERE peer_id = ?1 AND status IN ('failed', 'expired')
                 ORDER BY seq ASC",
            )?;
            let mapped = stmt.query_map([&peer_str], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            let mut stmt = conn.prepare_cached(
                "SELECT id, peer_id, kind, bytes_total FROM send_queue
                 WHERE status IN ('failed', 'expired')
                 ORDER BY seq ASC",
            )?;
            let mapped = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()?
        };
        if rows.is_empty() {
            return Ok(0);
        }
        let tx = conn.unchecked_transaction()?;
        let mut count = 0u32;
        for (id, peer_str, kind_str, bytes_total_raw) in rows {
            let kind = QueueKind::from_str_opt(&kind_str).unwrap_or(QueueKind::Files);
            let bytes_total = u64::try_from(bytes_total_raw.max(0)).unwrap_or(0);
            if kind.is_clipboard() {
                tx.execute(
                    "DELETE FROM send_queue
                     WHERE peer_id = ?1 AND status = 'waiting'
                       AND kind IN ('clipboard', 'clipboard_image')
                       AND id != ?2",
                    params![peer_str, id],
                )?;
            }
            let active_count: i64 = tx.query_row(
                "SELECT COUNT(*) FROM send_queue WHERE status IN ('waiting', 'sending')",
                [],
                |r| r.get(0),
            )?;
            if usize::try_from(active_count.max(0)).unwrap_or(usize::MAX) >= MAX_QUEUE_ACTIVE_ITEMS {
                break;
            }
            if kind.is_file_based() || kind == QueueKind::ClipboardImage {
                let active_file_bytes: i64 = tx.query_row(
                    "SELECT COALESCE(SUM(bytes_total), 0) FROM send_queue
                     WHERE status IN ('waiting', 'sending')
                       AND kind IN ('files', 'folder', 'handoff_document', 'recording', 'clipboard_image')",
                    [],
                    |r| r.get(0),
                )?;
                let current_bytes = u64::try_from(active_file_bytes.max(0)).unwrap_or(u64::MAX);
                if current_bytes.saturating_add(bytes_total) > MAX_QUEUE_ACTIVE_BYTES {
                    continue;
                }
            }
            let expires_at = now_ms.saturating_add(kind.ttl_ms());
            let updated = tx.execute(
                "UPDATE send_queue
                 SET status = 'waiting', attempts = 0, next_retry_ms = 0,
                     expires_at = ?2, error_msg = NULL, bytes_done = 0, files_done = 0
                 WHERE id = ?1",
                params![id, expires_at],
            )?;
            count = count.saturating_add(u32::try_from(updated).unwrap_or(0));
        }
        tx.commit()?;
        Ok(count)
    }

    /// Deletes a queue item by `id`, cleans up any staged files, and returns its `(QueueItem, QueuePayload)`.
    pub fn remove_queue_item(&self, id: &str) -> Result<Option<(QueueItem, QueuePayload)>> {
        let removed = self.with(|c| {
            let raw =
                c.query_row(&format!("{SELECT_SEND_QUEUE} WHERE id = ?1"), [id], raw_queue_row).optional()?;
            let Some(raw) = raw else {
                return Ok(None);
            };
            c.execute("DELETE FROM send_queue WHERE id = ?1", [id])?;
            Ok(decode_queue_row(c, &self.queue_seal, raw, false).map(|(item, payload, _, _)| (item, payload)))
        })?;
        if let Some((_, ref payload)) = removed {
            cleanup_staged_payload(self.data_dir.as_deref(), id, Some(payload));
        } else {
            cleanup_staged_payload(self.data_dir.as_deref(), id, None);
        }
        Ok(removed)
    }

    /// Removes all `'failed'` and `'expired'` items (for `peer` when `Some`, or all peers when `None`),
    /// cleans up their staged files, and returns their `(id, payload)` pairs.
    pub fn clear_finished_queue_items(&self, peer: Option<&DeviceId>) -> Result<Vec<(String, QueuePayload)>> {
        let removed: Vec<(String, QueuePayload)> = self.with(|c| {
            let rows: Vec<(String, String)> = if let Some(p) = peer {
                let peer_str = p.to_string();
                let mut stmt = c.prepare_cached(
                    "SELECT id, payload_json FROM send_queue
                     WHERE peer_id = ?1 AND status IN ('failed', 'expired')",
                )?;
                let mapped = stmt.query_map([&peer_str], |r| Ok((r.get(0)?, r.get(1)?)))?;
                mapped.collect::<rusqlite::Result<Vec<_>>>()?
            } else {
                let mut stmt = c.prepare_cached(
                    "SELECT id, payload_json FROM send_queue
                     WHERE status IN ('failed', 'expired')",
                )?;
                let mapped = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                mapped.collect::<rusqlite::Result<Vec<_>>>()?
            };
            if rows.is_empty() {
                return Ok(Vec::new());
            }
            if let Some(p) = peer {
                c.execute(
                    "DELETE FROM send_queue WHERE peer_id = ?1 AND status IN ('failed', 'expired')",
                    [p.to_string()],
                )?;
            } else {
                c.execute("DELETE FROM send_queue WHERE status IN ('failed', 'expired')", [])?;
            }
            Ok(rows
                .into_iter()
                .filter_map(|(id, json)| self.queue_seal.payload(&json).map(|p| (id, p)))
                .collect())
        })?;
        for (id, payload) in &removed {
            cleanup_staged_payload(self.data_dir.as_deref(), id, Some(payload));
        }
        Ok(removed)
    }
}

const fn bytes_from_len(len: usize) -> u64 {
    len as u64
}

fn cleanup_staged_payload(data_dir: Option<&Path>, id: &str, payload: Option<&QueuePayload>) {
    // Staged copies live in `<data_dir>/queue_staging/<id>/` and nowhere else,
    // so nothing outside it is deleted, whatever a (damaged) row says.
    let Some(dir) = data_dir else { return };
    if id.is_empty() || id == "." || id == ".." || id.contains(['/', '\\']) {
        return;
    }
    let stage = dir.join("queue_staging").join(id);
    let _ = std::fs::remove_dir_all(&stage);
    if let Some(p) = payload {
        for spec in p.files_vec() {
            let inside = spec.path.starts_with(&stage)
                && !spec.path.components().any(|c| matches!(c, std::path::Component::ParentDir));
            if spec.staged && inside {
                let _ = std::fs::remove_file(&spec.path);
            }
        }
    }
}

const SELECT_SEND_QUEUE: &str = "SELECT id, seq, peer_id, kind, status, created_at, expires_at, \
     next_retry_ms, attempts, title, detail, payload_json, bytes_total, bytes_done, \
     files_total, files_done, error_msg FROM send_queue";

const SELECT_SEND_QUEUE_WITH_BLOB: &str = "SELECT id, seq, peer_id, kind, status, created_at, expires_at, \
     next_retry_ms, attempts, title, detail, payload_json, bytes_total, bytes_done, \
     files_total, files_done, error_msg, payload_blob FROM send_queue";

struct RawQueueRow {
    id: String,
    seq: i64,
    peer_str: String,
    kind_str: String,
    status_str: String,
    created_at: i64,
    expires_at: i64,
    next_retry_ms: i64,
    attempts: i64,
    title: String,
    detail: String,
    payload_json: String,
    bytes_total: i64,
    bytes_done: i64,
    files_total: i64,
    files_done: i64,
    error_msg: Option<String>,
    payload_blob: Option<Vec<u8>>,
}

fn raw_queue_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawQueueRow> {
    Ok(RawQueueRow {
        id: row.get(0)?,
        seq: row.get(1)?,
        peer_str: row.get(2)?,
        kind_str: row.get(3)?,
        status_str: row.get(4)?,
        created_at: row.get(5)?,
        expires_at: row.get(6)?,
        next_retry_ms: row.get(7)?,
        attempts: row.get(8)?,
        title: row.get(9)?,
        detail: row.get(10)?,
        payload_json: row.get(11)?,
        bytes_total: row.get(12)?,
        bytes_done: row.get(13)?,
        files_total: row.get(14)?,
        files_done: row.get(15)?,
        error_msg: row.get(16)?,
        payload_blob: None,
    })
}

fn raw_queue_row_with_blob(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawQueueRow> {
    let mut r = raw_queue_row(row)?;
    r.payload_blob = row.get(17)?;
    Ok(r)
}

fn lookup_peer_name_conn(c: &Connection, peer: &DeviceId) -> String {
    c.query_row("SELECT name FROM peers WHERE id = ?1", [peer.as_bytes().as_slice()], |r| {
        r.get::<_, String>(0)
    })
    .ok()
    .filter(|s| !s.trim().is_empty())
    .unwrap_or_else(|| peer.short().to_owned())
}

/// Decoded row from `send_queue`: `(item, payload, payload_blob, next_retry_ms)`.
pub type DecodedQueueRow = (QueueItem, QueuePayload, Option<Vec<u8>>, i64);

fn decode_queue_row(
    c: &Connection,
    seal: &QueueSeal,
    mut raw: RawQueueRow,
    include_blob: bool,
) -> Option<DecodedQueueRow> {
    // A row this version can't read is left out of the queue; say so, so it
    // doesn't just vanish.
    let decoded = raw.peer_str.parse::<DeviceId>().ok().zip(seal.payload(&raw.payload_json));
    let Some((peer, payload)) = decoded else {
        tracing::warn!(id = %raw.id, "can't read a queued send; leaving it out of the queue");
        return None;
    };
    let kind = QueueKind::from_str_opt(&raw.kind_str).unwrap_or_else(|| payload.kind());
    if raw.payload_json.starts_with(SEALED_PREFIX) {
        (raw.title, raw.detail) = payload.title_and_detail();
        if include_blob && let Some(sealed) = raw.payload_blob.take() {
            let Some(blob) = seal.open(&sealed) else {
                tracing::warn!(id = %raw.id, "can't open a queued image; leaving it out of the queue");
                return None;
            };
            raw.payload_blob = Some(blob);
        }
    }
    let status = match raw.status_str.as_str() {
        "sending" => QueueStatus::Sending,
        "failed" => QueueStatus::Failed { error: raw.error_msg.unwrap_or_else(|| "Failed to send".into()) },
        "expired" => QueueStatus::Expired {
            error: raw.error_msg.unwrap_or_else(|| kind.expiry_error_message().into()),
        },
        _ => QueueStatus::Waiting,
    };
    let peer_name = lookup_peer_name_conn(c, &peer);
    let files = payload.files_vec();
    let item = QueueItem {
        id: raw.id,
        seq: raw.seq,
        peer,
        peer_name,
        kind,
        status,
        created_at: raw.created_at,
        expires_at: raw.expires_at,
        attempts: u32::try_from(raw.attempts.max(0)).unwrap_or(0),
        title: raw.title,
        detail: raw.detail,
        bytes_total: u64::try_from(raw.bytes_total.max(0)).unwrap_or(0),
        bytes_done: u64::try_from(raw.bytes_done.max(0)).unwrap_or(0),
        files_total: u32::try_from(raw.files_total.max(0)).unwrap_or(0),
        files_done: u32::try_from(raw.files_done.max(0)).unwrap_or(0),
        files,
    };
    let blob = if include_blob { raw.payload_blob } else { None };
    Some((item, payload, blob, raw.next_retry_ms))
}

const SELECT: &str = "SELECT id, name, kind, os, os_ver, model, accent, paired_at, last_seen, last_addrs, \
     caps, power, wake_info, screen, install_id FROM peers";

/// The device's install ID when it's well formed (others are never stored).
fn valid_install_id(info: &DeviceInfo) -> Option<&str> {
    info.install_id.as_deref().filter(|id| nectarlink_protocol::messages::is_valid_install_id(id))
}

const SELECT_TIMELINE: &str = "SELECT id, kind, device_id, device_name, incoming, timestamp, \
     title, detail, target, size_bytes, duration_secs, ref_id FROM timeline";

const SELECT_CHAT_THREADS: &str = "SELECT thread_id, app, app_name, notification_key, reply_action_id, \
     title, is_group, snippet, last_sender, updated_ms, unread, active, avatar, app_icon \
     FROM chat_threads";

fn prune_chat_messages_conn(c: &Connection, peer_str: &str, thread_id: &str) -> rusqlite::Result<()> {
    c.execute(
        "DELETE FROM chat_messages
         WHERE peer = ?1 AND thread_id = ?2
           AND id NOT IN (
               SELECT id FROM chat_messages
               WHERE peer = ?1 AND thread_id = ?2
               ORDER BY time_ms DESC, id DESC
               LIMIT ?3
           )",
        params![peer_str, thread_id, MAX_CHAT_MESSAGES_PER_THREAD as i64],
    )?;
    Ok(())
}

fn prune_chat_conn(c: &Connection, peer_str: &str, thread_id: &str) -> rusqlite::Result<()> {
    prune_chat_messages_conn(c, peer_str, thread_id)?;
    let max_updated: Option<i64> = c
        .query_row("SELECT MAX(updated_ms) FROM chat_threads WHERE peer = ?1", [peer_str], |r| r.get(0))
        .optional()?
        .flatten();
    if let Some(latest_ms) = max_updated {
        let retention_ms = CHAT_RETENTION_SECS * 1000;
        // Never later than now: one message stamped in the future (a phone
        // whose clock is wrong) would otherwise age out every other thread.
        let now_ms = crate::now_unix().saturating_mul(1000);
        let latest_ms = latest_ms.min(now_ms);
        if latest_ms > retention_ms {
            let cutoff_ms = latest_ms - retention_ms;
            c.execute(
                "DELETE FROM chat_threads WHERE peer = ?1 AND updated_ms < ?2",
                params![peer_str, cutoff_ms],
            )?;
        }
    }
    c.execute(
        "DELETE FROM chat_threads
         WHERE peer = ?1
           AND thread_id NOT IN (
               SELECT thread_id FROM chat_threads
               WHERE peer = ?1
               ORDER BY updated_ms DESC, thread_id ASC
               LIMIT ?2
           )",
        params![peer_str, MAX_CHAT_THREADS_PER_PEER as i64],
    )?;
    c.execute(
        "DELETE FROM chat_messages
         WHERE peer = ?1
           AND thread_id NOT IN (
               SELECT thread_id FROM chat_threads WHERE peer = ?1
           )",
        [peer_str],
    )?;
    Ok(())
}

fn row_to_chat_thread(peer: &DeviceId, row: &rusqlite::Row<'_>) -> rusqlite::Result<ChatThreadRecord> {
    let is_group: i64 = row.get(6)?;
    let unread: i64 = row.get(10)?;
    let active: i64 = row.get(11)?;
    Ok(ChatThreadRecord {
        peer: *peer,
        thread_id: row.get(0)?,
        app: row.get(1)?,
        app_name: row.get(2)?,
        notification_key: row.get(3)?,
        reply_action_id: row.get(4)?,
        title: row.get(5)?,
        is_group: is_group != 0,
        snippet: row.get(7)?,
        last_sender: row.get(8)?,
        updated_ms: row.get(9)?,
        unread: u32::try_from(unread.max(0)).unwrap_or(0),
        active: active != 0,
        avatar: row.get(12)?,
        app_icon: row.get(13)?,
    })
}

fn row_to_chat_message(peer: &DeviceId, row: &rusqlite::Row<'_>) -> rusqlite::Result<ChatMessageRecord> {
    let self_sent: i64 = row.get(5)?;
    Ok(ChatMessageRecord {
        id: row.get(0)?,
        peer: *peer,
        thread_id: row.get(1)?,
        sender: row.get(2)?,
        text: row.get(3)?,
        time_ms: row.get(4)?,
        self_sent: self_sent != 0,
        avatar: row.get(6)?,
    })
}

fn read_retention_conn(c: &Connection) -> rusqlite::Result<TimelineRetention> {
    let mut retention = TimelineRetention::default();
    if let Some(raw) = c
        .query_row("SELECT value FROM meta WHERE key = 'timeline_max_days'", [], |r| r.get::<_, String>(0))
        .optional()?
        && let Ok(v) = raw.parse::<u32>()
    {
        retention.max_days = v;
    }
    if let Some(raw) = c
        .query_row("SELECT value FROM meta WHERE key = 'timeline_max_entries'", [], |r| r.get::<_, String>(0))
        .optional()?
        && let Ok(v) = raw.parse::<u32>()
    {
        retention.max_entries = v;
    }
    Ok(retention)
}

fn purge_timeline_conn(c: &Connection, now_unix: i64, retention: TimelineRetention) -> rusqlite::Result<()> {
    if retention.max_days > 0 && now_unix > 0 {
        let cutoff = now_unix.saturating_sub(i64::from(retention.max_days) * 86_400);
        c.execute("DELETE FROM timeline WHERE timestamp < ?1", [cutoff])?;
    }
    if retention.max_entries > 0 {
        let boundary: Option<(i64, i64)> = c
            .query_row(
                "SELECT timestamp, id FROM timeline ORDER BY timestamp DESC, id DESC LIMIT 1 OFFSET ?1",
                [i64::from(retention.max_entries)],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((ts, id)) = boundary {
            c.execute(
                "DELETE FROM timeline WHERE timestamp < ?1 OR (timestamp = ?1 AND id <= ?2)",
                params![ts, id],
            )?;
        }
    }
    Ok(())
}

fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

fn row_to_timeline(row: &rusqlite::Row<'_>) -> rusqlite::Result<TimelineEntry> {
    let id: i64 = row.get(0)?;
    let kind_str: String = row.get(1)?;
    let dev_bytes: Vec<u8> = row.get(2)?;
    let dev_arr: [u8; 32] = dev_bytes.try_into().map_err(|_| {
        rusqlite::Error::InvalidColumnType(2, "device_id".into(), rusqlite::types::Type::Blob)
    })?;
    let incoming: i64 = row.get(4)?;
    let size_bytes: i64 = row.get(9)?;
    let duration_secs: i64 = row.get(10)?;
    let kind = TimelineKind::from_str_opt(&kind_str).unwrap_or(TimelineKind::File);
    Ok(TimelineEntry {
        id,
        kind,
        device_id: DeviceId(dev_arr),
        device_name: row.get(3)?,
        incoming: incoming != 0,
        timestamp: row.get(5)?,
        title: row.get(6)?,
        detail: row.get(7)?,
        target: row.get(8)?,
        size_bytes: u64::try_from(size_bytes.max(0)).unwrap_or(0),
        duration_secs: u64::try_from(duration_secs.max(0)).unwrap_or(0),
        ref_id: row.get(11)?,
        clip_available: false,
        image_data_url: None,
    })
}

fn encode_screen(screen: Option<&ScreenShape>) -> String {
    screen
        .cloned()
        .and_then(ScreenShape::sanitized)
        .and_then(|s| serde_json::to_string(&s).ok())
        .unwrap_or_default()
}

fn decode_screen(s: &str) -> Option<ScreenShape> {
    if s.is_empty() {
        return None;
    }
    serde_json::from_str::<ScreenShape>(s).ok().and_then(ScreenShape::sanitized)
}

fn row_to_peer(row: &rusqlite::Row<'_>) -> rusqlite::Result<PeerRecord> {
    let id: Vec<u8> = row.get(0)?;
    let id: [u8; 32] = id
        .try_into()
        .map_err(|_| rusqlite::Error::InvalidColumnType(0, "id".into(), rusqlite::types::Type::Blob))?;
    let kind: String = row.get(2)?;
    let accent: Option<i64> = row.get(6)?;
    let addrs: String = row.get(9)?;
    let caps: String = row.get(10)?;
    let power: String = row.get(11)?;
    let wake_info: String = row.get(12)?;
    let screen: String = row.get(13)?;
    Ok(PeerRecord {
        id: DeviceId(id),
        info: DeviceInfo {
            name: row.get(1)?,
            kind: kind_from_str(&kind),
            os: row.get(3)?,
            os_ver: row.get(4)?,
            model: row.get(5)?,
            accent: accent.and_then(|a| u32::try_from(a).ok()),
            screen: decode_screen(&screen),
            install_id: row.get(14)?,
        },
        paired_at: row.get(7)?,
        last_seen: row.get(8)?,
        // Skip entries that no longer parse instead of failing the whole read.
        last_addrs: addrs.split(',').filter_map(|a| a.parse().ok()).collect(),
        caps: caps.split(',').filter(|c| !c.is_empty()).map(str::to_owned).collect(),
        power: power_from_str(&power),
        wake_info: PcWakeInfo::from_storage_str(&wake_info),
    })
}

fn power_to_str(power: PowerLevel) -> &'static str {
    match power {
        PowerLevel::Basic | PowerLevel::Unknown => "basic",
        PowerLevel::Assist => "assist",
        PowerLevel::Elevated => "elevated",
        PowerLevel::NotApplicable => "n/a",
    }
}

fn power_from_str(power: &str) -> PowerLevel {
    match power {
        "assist" => PowerLevel::Assist,
        "elevated" => PowerLevel::Elevated,
        "n/a" => PowerLevel::NotApplicable,
        _ => PowerLevel::Basic,
    }
}

fn kind_to_str(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Phone => "phone",
        DeviceKind::Tablet => "tablet",
        DeviceKind::Desktop => "desktop",
        DeviceKind::Laptop => "laptop",
        DeviceKind::Unknown => "unknown",
    }
}

fn kind_from_str(kind: &str) -> DeviceKind {
    match kind {
        "phone" => DeviceKind::Phone,
        "tablet" => DeviceKind::Tablet,
        "desktop" => DeviceKind::Desktop,
        "laptop" => DeviceKind::Laptop,
        _ => DeviceKind::Unknown,
    }
}

fn migrate(conn: &Connection) -> Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let current = usize::try_from(current).unwrap_or(usize::MAX);
    if current > MIGRATIONS.len() {
        return Err(Error::Storage(format!(
            "the database was created by a newer NectarLink (schema {current}); please update"
        )));
    }
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(current) {
        let version = index + 1;
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", version as i64)?;
        tx.commit()?;
        tracing::debug!(version, "applied storage migration");
    }
    cleanup_peers(conn, None, &[])?;
    Ok(())
}

/// Merges entries for the same physical device paired again after a reinstall (which
/// creates a new identity), keeping timeline history under the current entry.
///
/// Devices count as the same only when they report the same install ID. Name and model
/// never do: two family phones of the same model often share a default name, and
/// merging them would silently unpair one.
fn cleanup_peers(
    conn: &Connection,
    keep_id: Option<&DeviceId>,
    skip_ids: &[DeviceId],
) -> rusqlite::Result<()> {
    struct PeerRow {
        id: DeviceId,
        kind: String,
        install_id: String,
        paired_at: i64,
        last_seen: Option<i64>,
    }

    let rows: Vec<PeerRow> = {
        let mut stmt = conn.prepare(
            "SELECT id, kind, install_id, paired_at, last_seen FROM peers
             WHERE install_id IS NOT NULL AND install_id != ''",
        )?;
        let mapped = stmt.query_map([], |r| {
            let bytes: Vec<u8> = r.get(0)?;
            let arr: [u8; 32] = bytes.try_into().map_err(|_| {
                rusqlite::Error::InvalidColumnType(0, "id".into(), rusqlite::types::Type::Blob)
            })?;
            Ok(PeerRow {
                id: DeviceId(arr),
                kind: r.get(1)?,
                install_id: r.get(2)?,
                paired_at: r.get(3)?,
                last_seen: r.get(4)?,
            })
        })?;
        mapped.collect::<rusqlite::Result<Vec<_>>>()?
    };

    let mut groups: Vec<Vec<PeerRow>> = Vec::new();
    for row in rows {
        if let Some(group) =
            groups.iter_mut().find(|g| g[0].install_id == row.install_id && g[0].kind == row.kind)
        {
            group.push(row);
        } else {
            groups.push(vec![row]);
        }
    }

    for group in groups {
        if group.len() <= 1 {
            continue;
        }
        let winner_id = if let Some(keep) = keep_id
            && group.iter().any(|r| &r.id == keep)
        {
            *keep
        } else {
            group.iter().max_by_key(|r| (r.last_seen.unwrap_or(0), r.paired_at)).expect("non-empty group").id
        };
        let winner_str = winner_id.to_string();
        for row in &group {
            if row.id == winner_id || skip_ids.contains(&row.id) {
                continue;
            }
            let stale_str = row.id.to_string();
            let _ = conn.execute(
                "INSERT OR IGNORE INTO peer_toggles (peer, toggle, enabled)
                 SELECT ?1, toggle, enabled FROM peer_toggles WHERE peer = ?2",
                params![winner_id.as_bytes().as_slice(), row.id.as_bytes().as_slice()],
            );
            let _ = conn.execute(
                "UPDATE timeline SET device_id = ?1 WHERE device_id = ?2",
                params![winner_id.as_bytes().as_slice(), row.id.as_bytes().as_slice()],
            );
            let _ = conn.execute(
                "UPDATE send_queue SET peer_id = ?1 WHERE peer_id = ?2",
                params![&winner_str, &stale_str],
            );
            let _ = conn.execute("DELETE FROM chat_messages WHERE peer = ?1", [&stale_str]);
            let _ = conn.execute("DELETE FROM chat_threads WHERE peer = ?1", [&stale_str]);
            conn.execute("DELETE FROM peers WHERE id = ?1", [row.id.as_bytes().as_slice()])?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nectarlink_protocol::messages::{ScreenCorners, ScreenRect};

    fn info(name: &str) -> DeviceInfo {
        DeviceInfo {
            name: name.into(),
            kind: DeviceKind::Phone,
            os: "android".into(),
            os_ver: "16".into(),
            model: None,
            accent: Some(0xFF8A5100),
            screen: Some(ScreenShape {
                v: 1,
                aspect: 0.45,
                corners: Some(ScreenCorners { tl: 0.085, tr: 0.085, br: 0.085, bl: 0.085 }),
                cutouts: vec![ScreenRect { x: 0.46, y: 0.018, w: 0.08, h: 0.036 }],
                cutout_path: Some("M 0.5 0.018 L 0.54 0.036 L 0.5 0.054 L 0.46 0.036 Z".into()),
            }),
            install_id: None,
        }
    }

    #[test]
    fn peer_lifecycle() {
        let store = Store::in_memory().unwrap();
        let id = DeviceId([1; 32]);
        assert!(!store.is_paired(&id).unwrap());

        store.upsert_peer(&id, &info("Pixel"), 100).unwrap();
        let peer = store.get_peer(&id).unwrap().unwrap();
        assert_eq!(peer.info, info("Pixel"));
        assert_eq!(peer.paired_at, 100);
        assert_eq!(peer.last_seen, None);
        assert!(peer.last_addrs.is_empty());

        let addrs: Vec<SocketAddr> =
            vec!["10.0.0.2:41641".parse().unwrap(), "[fe80::2]:41641".parse().unwrap()];
        store.record_seen(&id, 200, &addrs).unwrap();
        let mut without_accent = info("Pixel 9");
        without_accent.accent = None;
        store.update_info(&id, &without_accent).unwrap();
        let peer = store.get_peer(&id).unwrap().unwrap();
        assert_eq!(peer.last_seen, Some(200));
        assert_eq!(peer.last_addrs, addrs);
        assert_eq!(peer.info.name, "Pixel 9");
        // Stored accent is preserved when a later update omits it.
        assert_eq!(peer.info.accent, Some(0xFF8A5100));
        assert_eq!(peer.info.screen, info("Pixel 9").screen);

        assert!(store.remove_peer(&id).unwrap());
        assert!(!store.remove_peer(&id).unwrap());
        assert!(store.list_peers().unwrap().is_empty());
    }

    #[test]
    fn capabilities_and_toggles() {
        let store = Store::in_memory().unwrap();
        let id = DeviceId([3; 32]);
        store.upsert_peer(&id, &info("Pixel"), 1).unwrap();
        let peer = store.get_peer(&id).unwrap().unwrap();
        assert!(peer.caps.is_empty());
        assert_eq!(peer.power, PowerLevel::Basic);

        let caps: BTreeSet<String> = ["clip.write".to_owned(), "device.ring".to_owned()].into();
        store.update_capabilities(&id, Some(&caps), Some(PowerLevel::Elevated)).unwrap();
        // None keeps what's stored.
        store.update_capabilities(&id, None, None).unwrap();
        let peer = store.get_peer(&id).unwrap().unwrap();
        assert_eq!(peer.caps, caps);
        assert_eq!(peer.power, PowerLevel::Elevated);
        store.update_capabilities(&id, Some(&BTreeSet::new()), None).unwrap();
        assert!(store.get_peer(&id).unwrap().unwrap().caps.is_empty());

        assert!(store.toggles(&id).unwrap().is_empty());
        store.set_toggle(&id, "clipboard", false).unwrap();
        store.set_toggle(&id, "clipboard", true).unwrap();
        store.set_toggle(&id, "photos", false).unwrap();
        let toggles = store.toggles(&id).unwrap();
        assert_eq!(toggles.len(), 2);
        assert_eq!(toggles.get("clipboard"), Some(&true));
        assert_eq!(toggles.get("photos"), Some(&false));
        assert_eq!(store.toggle(&id, "photos").unwrap(), Some(false));
        assert_eq!(store.toggle(&id, "sms").unwrap(), None);
        // Read back from memory now, so a change must show at once.
        store.set_toggle(&id, "photos", true).unwrap();
        assert_eq!(store.toggle(&id, "photos").unwrap(), Some(true));

        // Unpairing forgets the device's settings too.
        store.remove_peer(&id).unwrap();
        assert!(store.toggles(&id).unwrap().is_empty());
        assert_eq!(store.toggle(&id, "photos").unwrap(), None);
        // Paired again, it starts from the defaults.
        store.upsert_peer(&id, &info("Pixel"), 2).unwrap();
        assert_eq!(store.toggle(&id, "clipboard").unwrap(), None);
    }

    #[test]
    fn upgrades_a_version_1_database() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(MIGRATIONS[0]).unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        conn.execute(
            "INSERT INTO peers (id, name, kind, os, os_ver, paired_at) VALUES (?1, 'Old', 'phone', 'android', '15', 5)",
            [[9u8; 32].as_slice()],
        )
        .unwrap();
        let store = Store::init(conn).unwrap();
        let peer = store.get_peer(&DeviceId([9; 32])).unwrap().unwrap();
        assert_eq!(peer.info.name, "Old");
        assert!(peer.caps.is_empty());
        assert_eq!(peer.power, PowerLevel::Basic);
    }

    #[test]
    fn lists_in_pairing_order_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = Store::open(dir.path()).unwrap();
            store.upsert_peer(&DeviceId([2; 32]), &info("second"), 20).unwrap();
            store.upsert_peer(&DeviceId([1; 32]), &info("first"), 10).unwrap();
        }
        let store = Store::open(dir.path()).unwrap();
        let names: Vec<_> = store.list_peers().unwrap().into_iter().map(|p| p.info.name).collect();
        assert_eq!(names, ["first", "second"]);
    }

    #[test]
    fn refuses_a_database_from_a_newer_version() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", 99).unwrap();
        assert!(matches!(Store::init(conn), Err(Error::Storage(_))));
    }

    #[test]
    fn wake_info_persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let id = DeviceId([7; 32]);
        let wake =
            PcWakeInfo { macs: vec!["00:00:5e:00:53:01".into()], broadcasts: vec!["192.168.1.255".into()] };
        {
            let store = Store::open(dir.path()).unwrap();
            store.upsert_peer(&id, &info("Studio"), 42).unwrap();
            assert_eq!(store.get_peer(&id).unwrap().unwrap().wake_info, None);
            store.update_wake_info(&id, Some(&wake)).unwrap();
        }
        {
            let store = Store::open(dir.path()).unwrap();
            assert_eq!(store.get_peer(&id).unwrap().unwrap().wake_info, Some(wake));
            store.update_wake_info(&id, None).unwrap();
            assert_eq!(store.get_peer(&id).unwrap().unwrap().wake_info, None);
        }
    }

    #[test]
    fn timeline_crud_filters_and_retention() {
        let store = Store::in_memory().unwrap();
        let dev_a = DeviceId([1; 32]);
        let dev_b = DeviceId([2; 32]);
        let now = 1_700_000_000i64;

        let file_id = store
            .insert_timeline(
                &NewTimelineEntry {
                    kind: TimelineKind::File,
                    device_id: dev_a,
                    device_name: "Pixel 9".into(),
                    incoming: true,
                    timestamp: now - 300,
                    title: "report.pdf".into(),
                    detail: "1 file".into(),
                    target: "/tmp/report.pdf".into(),
                    size_bytes: 4096,
                    duration_secs: 0,
                    ref_id: Some("tx-1".into()),
                },
                now,
            )
            .unwrap();
        let _clip_id = store
            .insert_timeline(
                &NewTimelineEntry {
                    kind: TimelineKind::Clip,
                    device_id: dev_a,
                    device_name: "Pixel 9".into(),
                    incoming: false,
                    timestamp: now - 200,
                    title: String::new(),
                    detail: "text".into(),
                    target: String::new(),
                    size_bytes: 0,
                    duration_secs: 0,
                    ref_id: Some("clip-42".into()),
                },
                now,
            )
            .unwrap();
        let session_id = store
            .insert_timeline(
                &NewTimelineEntry {
                    kind: TimelineKind::Session,
                    device_id: dev_b,
                    device_name: "Galaxy Tab".into(),
                    incoming: true,
                    timestamp: now - 100,
                    title: "Webcam".into(),
                    detail: "1920×1080 · 30 fps".into(),
                    target: String::new(),
                    size_bytes: 0,
                    duration_secs: 0,
                    ref_id: None,
                },
                now,
            )
            .unwrap();

        // Update session duration on stream end.
        assert!(store.update_timeline_duration(session_id, 95).unwrap());
        assert_eq!(store.get_timeline(session_id).unwrap().unwrap().duration_secs, 95);

        // Upgrade transfer row to Photo when saved by the photo pipeline.
        assert!(
            store
                .update_timeline_by_ref(
                    "tx-1",
                    TimelineKind::Photo,
                    "IMG_0001.jpg",
                    "Saved photo",
                    "/tmp/IMG_0001.jpg",
                    8192
                )
                .unwrap()
        );
        let updated = store.get_timeline(file_id).unwrap().unwrap();
        assert_eq!(updated.kind, TimelineKind::Photo);
        assert_eq!(updated.title, "IMG_0001.jpg");
        assert_eq!(updated.size_bytes, 8192);

        // Filter by kind.
        let photos = store
            .query_timeline(
                &TimelineQuery { kind: Some(TimelineKind::Photo), limit: 10, ..Default::default() },
                &[],
            )
            .unwrap();
        assert_eq!(photos.total, 1);
        assert_eq!(photos.entries[0].title, "IMG_0001.jpg");

        // Filter by device.
        let tab = store
            .query_timeline(&TimelineQuery { device: Some(dev_b), limit: 10, ..Default::default() }, &[])
            .unwrap();
        assert_eq!(tab.total, 1);
        assert_eq!(tab.entries[0].title, "Webcam");

        // Search matching linked clip IDs (without clip text in SQLite).
        let clips = store
            .query_timeline(
                &TimelineQuery { search: Some("secret-phrase".into()), limit: 10, ..Default::default() },
                &["clip-42".to_owned()],
            )
            .unwrap();
        assert_eq!(clips.total, 1);
        assert_eq!(clips.entries[0].kind, TimelineKind::Clip);

        // Retention auto-purge by max_entries and max_days.
        store.set_timeline_retention(TimelineRetention { max_days: 1, max_entries: 2 }, now).unwrap();
        let all = store.query_timeline(&TimelineQuery { limit: 10, ..Default::default() }, &[]).unwrap();
        assert_eq!(all.total, 2);

        assert!(store.delete_timeline(session_id).unwrap());
        assert_eq!(store.clear_timeline().unwrap(), 1);
    }

    #[test]
    fn timeline_5000_entries_paging_is_fast() {
        let store = Store::in_memory().unwrap();
        let dev = DeviceId([5; 32]);
        let base = 1_700_000_000i64;
        for i in 0..5_050i64 {
            store
                .insert_timeline(
                    &NewTimelineEntry {
                        kind: if i % 2 == 0 { TimelineKind::File } else { TimelineKind::Link },
                        device_id: dev,
                        device_name: "Pixel 9".into(),
                        incoming: i % 3 == 0,
                        timestamp: base + i,
                        title: format!("item-{i}.txt"),
                        detail: "1 file".into(),
                        target: format!("/tmp/item-{i}.txt"),
                        size_bytes: 128,
                        duration_secs: 0,
                        ref_id: None,
                    },
                    base + i,
                )
                .unwrap();
        }
        let started = std::time::Instant::now();
        let page0 = store
            .query_timeline(&TimelineQuery { offset: 0, limit: 100, ..Default::default() }, &[])
            .unwrap();
        let page25 = store
            .query_timeline(&TimelineQuery { offset: 2500, limit: 100, ..Default::default() }, &[])
            .unwrap();
        let elapsed = started.elapsed();
        // Default cap is 5,000 entries, so oldest 50 were auto-purged.
        assert_eq!(page0.total, 5_000);
        assert_eq!(page0.entries.len(), 100);
        assert!(page0.has_more);
        assert_eq!(page0.entries[0].title, "item-5049.txt");
        assert_eq!(page25.entries.len(), 100);
        assert!(elapsed.as_millis() < 200, "paging 5,000 entries took {elapsed:?}");
    }

    #[test]
    fn chat_notifications_deduplicate_reply_and_prune() {
        use nectarlink_protocol::messages::{
            Notification, NotificationAction, NotificationChatMessage, NotificationConversation,
        };
        let store = Store::in_memory().unwrap();
        let peer = DeviceId([7u8; 32]);
        let now_ms = 1_760_000_000_000i64;

        let conv = NotificationConversation {
            v: 1,
            title: "Design Team".into(),
            group: true,
            avatar: Some(vec![0xff, 0xd8, 1]),
            messages: vec![
                NotificationChatMessage {
                    sender: Some("Maya".into()),
                    text: "Standup in 5?".into(),
                    time: now_ms - 120_000,
                    self_sent: false,
                    avatar: Some(vec![0xff, 0xd8, 2]),
                },
                NotificationChatMessage {
                    sender: None,
                    text: "Joining now".into(),
                    time: now_ms - 60_000,
                    self_sent: true,
                    avatar: None,
                },
            ],
        };
        let note = Notification {
            key: "wa|design".into(),
            app: "com.whatsapp".into(),
            app_name: "WhatsApp".into(),
            title: Some("Design Team".into()),
            text: Some("Joining now".into()),
            sub: None,
            when: now_ms - 60_000,
            actions: vec![NotificationAction { id: "reply".into(), title: "Reply".into(), reply: true }],
            silent: false,
            ongoing: false,
            icon: Some(vec![0x89, b'P', b'N', b'G']),
            image: None,
            live: None,
            conversation: Some(conv),
        };

        let tid = store.upsert_chat_notification(&peer, &note, None, false).unwrap().unwrap();
        // Repeated snapshot of the exact same notification must not duplicate messages or bump unread.
        let tid_again = store.upsert_chat_notification(&peer, &note, None, true).unwrap().unwrap();
        assert_eq!(tid, tid_again);

        let threads = store.chat_threads(&peer).unwrap();
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].thread_id, tid);
        assert_eq!(threads[0].title, "Design Team");
        assert!(threads[0].is_group);
        assert_eq!(threads[0].unread, 1, "only incoming message increments unread");
        assert!(threads[0].can_reply());
        assert!(threads[0].app_icon.is_some());
        assert!(threads[0].avatar.is_some());

        let msgs = store.chat_messages(&peer, &tid, 20).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].text, "Joining now");
        assert!(msgs[0].self_sent);
        assert_eq!(msgs[1].text, "Standup in 5?");
        assert_eq!(msgs[1].sender.as_deref(), Some("Maya"));

        // Replying appends an outgoing message and resets unread count.
        assert!(store.append_chat_reply(&peer, &tid, "See you there!", now_ms + 5_000).unwrap());
        let thread = store.chat_thread(&peer, &tid).unwrap().unwrap();
        assert_eq!(thread.unread, 0);
        assert_eq!(thread.snippet, "You: See you there!");
        let msgs = store.chat_messages(&peer, &tid, 20).unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].text, "See you there!");
        assert!(msgs[0].self_sent);

        // Removing the notification clears the active reply target while keeping history.
        let cleared = store.mark_chat_notification_removed(&peer, "wa|design").unwrap();
        assert_eq!(cleared.as_deref(), Some(tid.as_str()));
        let thread = store.chat_thread(&peer, &tid).unwrap().unwrap();
        assert!(!thread.can_reply());
        assert_eq!(store.chat_messages(&peer, &tid, 20).unwrap().len(), 3);

        assert!(store.delete_chat_thread(&peer, &tid).unwrap());
        assert!(store.chat_threads(&peer).unwrap().is_empty());
    }

    #[test]
    fn data_retention_counts_and_clear_all_preserves_peers_and_meta() {
        use nectarlink_protocol::messages::{
            Notification, NotificationChatMessage, NotificationConversation,
        };
        let store = Store::in_memory().unwrap();
        let peer = DeviceId([8; 32]);
        let now = 1_700_000_000i64;

        // Seed peer, toggle, and meta.
        store.upsert_peer(&peer, &info("Pixel 9"), now).unwrap();
        store.set_toggle(&peer, "clipboard", false).unwrap();
        store.set_timeline_retention(TimelineRetention { max_days: 30, max_entries: 5000 }, now).unwrap();

        // Seed timeline entries (2 File + 1 Link).
        for (i, kind) in [TimelineKind::File, TimelineKind::File, TimelineKind::Link].into_iter().enumerate()
        {
            store
                .insert_timeline(
                    &NewTimelineEntry {
                        kind,
                        device_id: peer,
                        device_name: "Pixel 9".into(),
                        incoming: true,
                        timestamp: now + i as i64,
                        title: format!("entry-{i}"),
                        detail: "detail".into(),
                        target: String::new(),
                        size_bytes: 1024,
                        duration_secs: 0,
                        ref_id: None,
                    },
                    now + i as i64,
                )
                .unwrap();
        }

        // Seed chat conversation cache.
        let note = Notification {
            key: "wa|alex".into(),
            app: "com.whatsapp".into(),
            app_name: "WhatsApp".into(),
            title: Some("Alex".into()),
            text: Some("Hey there".into()),
            sub: None,
            when: now * 1000,
            actions: vec![],
            silent: false,
            ongoing: false,
            icon: None,
            image: None,
            live: None,
            conversation: Some(NotificationConversation {
                v: 1,
                title: "Alex".into(),
                group: false,
                avatar: None,
                messages: vec![
                    NotificationChatMessage {
                        sender: Some("Alex".into()),
                        text: "Hello!".into(),
                        time: now * 1000 - 1000,
                        self_sent: false,
                        avatar: None,
                    },
                    NotificationChatMessage {
                        sender: None,
                        text: "Hi Alex".into(),
                        time: now * 1000,
                        self_sent: true,
                        avatar: None,
                    },
                ],
            }),
        };
        store.upsert_chat_notification(&peer, &note, None, false).unwrap();

        assert_eq!(store.timeline_total_count().unwrap(), 3);
        assert_eq!(store.timeline_count_by_kind(TimelineKind::File).unwrap(), 2);
        assert_eq!(store.timeline_count_by_kind(TimelineKind::Link).unwrap(), 1);
        assert_eq!(store.chat_cache_counts().unwrap(), (1, 2));

        // Deleting only File timeline entries leaves Link entries intact.
        assert_eq!(store.delete_timeline_kind(TimelineKind::File).unwrap(), 2);
        assert_eq!(store.timeline_count_by_kind(TimelineKind::File).unwrap(), 0);
        assert_eq!(store.timeline_total_count().unwrap(), 1);

        // Clear all history and cache wipes timeline + chat tables while preserving peers, toggles, and meta.
        store.clear_all_history_and_cache().unwrap();
        assert_eq!(store.timeline_total_count().unwrap(), 0);
        assert_eq!(store.chat_cache_counts().unwrap(), (0, 0));
        assert_eq!(store.list_peers().unwrap().len(), 1);
        assert_eq!(store.toggles(&peer).unwrap().get("clipboard"), Some(&false));
        assert_eq!(store.timeline_retention().unwrap().max_days, 30);
    }

    #[test]
    fn migrates_from_every_historical_schema_version_v1_through_v6_to_v7() {
        use nectarlink_protocol::messages::{
            Notification, NotificationChatMessage, NotificationConversation, ScreenCorners, ScreenRect,
            ScreenShape,
        };

        for start_version in 1..=6usize {
            let conn = Connection::open_in_memory().unwrap();
            conn.pragma_update(None, "foreign_keys", "ON").unwrap();
            for sql in &MIGRATIONS[..start_version] {
                conn.execute_batch(sql).unwrap();
            }
            conn.pragma_update(None, "user_version", start_version as i64).unwrap();

            let peer = DeviceId([start_version as u8; 32]);
            let paired_at = 1_700_000_000i64 + start_version as i64;
            let last_seen = paired_at + 50;
            let addrs_str = "192.168.1.10:42718";

            // Seed v1 peers row.
            conn.execute(
                "INSERT INTO peers (id, name, kind, os, os_ver, model, accent, paired_at, last_seen, last_addrs)
                 VALUES (?1, ?2, 'phone', 'android', '15', 'Pixel 9', ?3, ?4, ?5, ?6)",
                params![
                    peer.as_bytes().as_slice(),
                    format!("Peer-v{start_version}"),
                    0xFF3B82F6u32 as i64,
                    paired_at,
                    last_seen,
                    addrs_str,
                ],
            )
            .unwrap();

            // Seed v2 columns & peer_toggles.
            if start_version >= 2 {
                conn.execute(
                    "UPDATE peers SET caps = ?2, power = 'assist' WHERE id = ?1",
                    params![peer.as_bytes().as_slice(), "clipboard,files"],
                )
                .unwrap();
                conn.execute(
                    "INSERT INTO peer_toggles (peer, toggle, enabled) VALUES (?1, 'clipboard', 0)",
                    params![peer.as_bytes().as_slice()],
                )
                .unwrap();
            }

            // Seed v3 wake_info column.
            if start_version >= 3 {
                let wake = PcWakeInfo {
                    macs: vec!["aa:bb:cc:dd:ee:ff".into()],
                    broadcasts: vec!["192.168.1.255".into()],
                };
                conn.execute(
                    "UPDATE peers SET wake_info = ?2 WHERE id = ?1",
                    params![peer.as_bytes().as_slice(), wake.to_storage_string()],
                )
                .unwrap();
            }

            // Seed v4 screen column.
            let expected_screen = ScreenShape {
                v: 1,
                aspect: 0.45,
                corners: Some(ScreenCorners { tl: 0.08, tr: 0.08, br: 0.08, bl: 0.08 }),
                cutouts: vec![ScreenRect { x: 0.45, y: 0.01, w: 0.1, h: 0.03 }],
                cutout_path: None,
            };
            if start_version >= 4 {
                conn.execute(
                    "UPDATE peers SET screen = ?2 WHERE id = ?1",
                    params![peer.as_bytes().as_slice(), serde_json::to_string(&expected_screen).unwrap()],
                )
                .unwrap();
            }

            // Seed v5 timeline and meta tables.
            if start_version >= 5 {
                conn.execute(
                    "INSERT INTO timeline (kind, device_id, device_name, incoming, timestamp, title, detail, target, size_bytes, duration_secs, ref_id)
                     VALUES ('file', ?1, 'Pixel 9', 1, ?2, 'photo.jpg', 'Saved', '/tmp/photo.jpg', 4096, 0, 'xfer-1')",
                    params![peer.as_bytes().as_slice(), paired_at],
                )
                .unwrap();
                conn.execute(
                    "INSERT INTO meta (key, value) VALUES ('timeline_max_days', '60'), ('timeline_max_entries', '2500')",
                    [],
                )
                .unwrap();
            }

            // Run Store::init to migrate from `start_version` -> latest (v7).
            let store = Store::init(conn).unwrap();
            let final_ver: i64 =
                store.with(|c| c.pragma_query_value(None, "user_version", |r| r.get(0))).unwrap();
            assert_eq!(final_ver as usize, MIGRATIONS.len(), "migrated from v{start_version} to v7");

            // Verify v1..v4 peer record survived intact.
            let rec = store.get_peer(&peer).unwrap().expect("peer preserved across migration");
            assert_eq!(rec.info.name, format!("Peer-v{start_version}"));
            assert_eq!(rec.info.kind, DeviceKind::Phone);
            assert_eq!(rec.info.model.as_deref(), Some("Pixel 9"));
            assert_eq!(rec.info.accent, Some(0xFF3B82F6));
            assert_eq!(rec.paired_at, paired_at);
            assert_eq!(rec.last_seen, Some(last_seen));
            assert_eq!(rec.last_addrs.len(), 1);

            if start_version >= 2 {
                assert!(rec.caps.contains("clipboard"));
                assert_eq!(rec.power, PowerLevel::Assist);
                assert_eq!(store.toggles(&peer).unwrap().get("clipboard"), Some(&false));
            } else {
                assert!(rec.caps.is_empty());
                assert_eq!(rec.power, PowerLevel::Basic);
            }

            if start_version >= 3 {
                assert_eq!(rec.wake_info.as_ref().unwrap().macs, vec!["aa:bb:cc:dd:ee:ff"]);
                assert_eq!(rec.wake_info.as_ref().unwrap().broadcasts, vec!["192.168.1.255"]);
            } else {
                assert!(rec.wake_info.is_none());
            }

            if start_version >= 4 {
                assert_eq!(rec.info.screen, Some(expected_screen));
            } else {
                assert!(rec.info.screen.is_none());
            }

            if start_version >= 5 {
                assert_eq!(store.timeline_total_count().unwrap(), 1);
                assert_eq!(
                    store.timeline_retention().unwrap(),
                    TimelineRetention { max_days: 60, max_entries: 2500 }
                );
            } else {
                assert_eq!(store.timeline_total_count().unwrap(), 0);
            }

            // Verify v5 timeline, v6 chat, and v7 send_queue tables are writable and queryable after migration.
            store
                .insert_timeline(
                    &NewTimelineEntry {
                        kind: TimelineKind::Link,
                        device_id: peer,
                        device_name: rec.info.name.clone(),
                        incoming: false,
                        timestamp: paired_at + 100,
                        title: "https://nectarlink.app".into(),
                        detail: "Sent".into(),
                        target: "https://nectarlink.app".into(),
                        size_bytes: 0,
                        duration_secs: 0,
                        ref_id: None,
                    },
                    paired_at + 100,
                )
                .unwrap();

            let note = Notification {
                key: "msg|1".into(),
                app: "com.google.android.apps.messaging".into(),
                app_name: "Messages".into(),
                title: Some("Sam".into()),
                text: Some("Migrated!".into()),
                sub: None,
                when: paired_at * 1000,
                actions: vec![],
                silent: false,
                ongoing: false,
                icon: None,
                image: None,
                live: None,
                conversation: Some(NotificationConversation {
                    v: 1,
                    title: "Sam".into(),
                    group: false,
                    avatar: None,
                    messages: vec![NotificationChatMessage {
                        sender: Some("Sam".into()),
                        text: "Migrated!".into(),
                        time: paired_at * 1000,
                        self_sent: false,
                        avatar: None,
                    }],
                }),
            };
            let tid = store.upsert_chat_notification(&peer, &note, None, false).unwrap().unwrap();
            assert_eq!(store.chat_messages(&peer, &tid, 10).unwrap().len(), 1);

            store
                .enqueue_send_item(
                    "q-mig-1",
                    &peer,
                    &QueuePayload::Link { url: "https://nectarlink.dev".into(), title: None },
                    None,
                    paired_at * 1000,
                )
                .unwrap();
            assert_eq!(store.list_send_queue(Some(&peer)).unwrap().len(), 1);
        }
    }

    #[test]
    fn cleanup_peers_merges_only_the_same_install_id() {
        let dir = tempfile::tempdir().unwrap();
        let pixel = DeviceId([0xD0; 32]);
        let old_phone = DeviceId([0xA1; 32]);
        let new_phone = DeviceId([0xA2; 32]);
        let sibling_a = DeviceId([0xB1; 32]);
        let sibling_b = DeviceId([0xB2; 32]);
        let legacy_a = DeviceId([0xC1; 32]);
        let legacy_b = DeviceId([0xC2; 32]);
        let reinstalled = "0123456789abcdef0123456789abcdef";
        let now = 1_760_000_000i64;

        {
            let conn = Connection::open(dir.path().join(FILE_NAME)).unwrap();
            migrate(&conn).unwrap();
            let insert = |id: DeviceId,
                          name: &str,
                          model: Option<&str>,
                          install: Option<&str>,
                          paired: i64| {
                conn.execute(
                    "INSERT INTO peers (id, name, kind, os, os_ver, model, accent, paired_at, last_seen, install_id)
                     VALUES (?1, ?2, 'phone', 'android', '16', ?3, NULL, ?4, ?4, ?5)",
                    params![id.as_bytes().as_slice(), name, model, paired, install],
                )
                .unwrap();
            };
            // A phone whose model isn't known yet is never removed.
            insert(pixel, "Pixel 9", None, None, now - 500);
            // The same phone paired again after a reinstall: same install ID, new identity.
            insert(old_phone, "Test Phone", Some("Example X1"), Some(reinstalled), now - 300);
            insert(new_phone, "Test Phone", Some("Example X1"), Some(reinstalled), now - 100);
            // Two family phones with the same model and default name: different devices.
            insert(
                sibling_a,
                "Family Phone",
                Some("Example X1"),
                Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
                now - 90,
            );
            insert(
                sibling_b,
                "Family Phone",
                Some("Example X1"),
                Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
                now - 80,
            );
            // Entries from app versions without install IDs are never merged on name alone.
            insert(legacy_a, "Old Phone", Some("Example X1"), None, now - 70);
            insert(legacy_b, "Old Phone", Some("Example X1"), None, now - 60);

            conn.execute(
                "INSERT INTO timeline (kind, device_id, device_name, incoming, timestamp, title, detail, target, size_bytes, duration_secs)
                 VALUES ('file', ?1, 'Test Phone', 1, ?2, 'notes.pdf', '1 file', '/tmp/notes.pdf', 1024, 0)",
                params![old_phone.as_bytes().as_slice(), now - 250],
            )
            .unwrap();
        }

        let store = Store::open(dir.path()).unwrap();
        let mut ids: Vec<DeviceId> = store.list_peers().unwrap().into_iter().map(|p| p.id).collect();
        ids.sort_by_key(|id| id.0);
        assert_eq!(
            ids,
            vec![new_phone, sibling_a, sibling_b, legacy_a, legacy_b, pixel],
            "only the reinstalled phone's old entry is merged away"
        );

        let page = store.query_timeline(&TimelineQuery { limit: 10, ..Default::default() }, &[]).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.entries[0].device_id, new_phone, "history moves to the current entry");
    }

    #[test]
    fn re_pairing_with_an_install_id_replaces_the_old_entry() {
        let store = Store::in_memory().unwrap();
        let (old, new) = (DeviceId([0x01; 32]), DeviceId([0x02; 32]));
        let mut phone = info("Test Phone");
        phone.install_id = Some("0123456789abcdef0123456789abcdef".into());
        store.upsert_peer(&old, &phone, 100).unwrap();
        store.set_toggle(&old, "clipboard", false).unwrap();
        store.set_toggle(&old, "remote_input", true).unwrap();
        store.upsert_peer(&new, &phone, 200).unwrap();
        let ids: Vec<DeviceId> = store.list_peers().unwrap().into_iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![new], "the new pairing wins");
        assert_eq!(
            store.toggle(&new, "clipboard").unwrap(),
            Some(false),
            "disabled privacy toggle is preserved across re-pairing"
        );
        assert_eq!(
            store.toggle(&new, "remote_input").unwrap(),
            Some(true),
            "enabled toggle is preserved across re-pairing"
        );

        // An online peer in skip_ids is not deleted when another peer claims the same install_id.
        let online_new = DeviceId([0x05; 32]);
        store.upsert_peer_skipping(&online_new, &phone, 250, &[new]).unwrap();
        let mut both: Vec<DeviceId> = store.list_peers().unwrap().into_iter().map(|p| p.id).collect();
        both.sort_by_key(|id| id.0);
        assert_eq!(both, vec![new, online_new], "skipped online peer is preserved");
        store.remove_peer(&online_new).unwrap();

        // Malformed IDs are never stored, so they can't merge anything.
        let mut bad = info("Other Phone");
        bad.install_id = Some("not-hex".into());
        store.upsert_peer(&DeviceId([0x03; 32]), &bad, 300).unwrap();
        store.upsert_peer(&DeviceId([0x04; 32]), &bad, 400).unwrap();
        assert_eq!(store.list_peers().unwrap().len(), 3);
    }

    #[test]
    fn an_older_notification_doesnt_replace_the_newest_snippet() {
        use nectarlink_protocol::messages::{NotificationChatMessage, NotificationConversation};
        let store = Store::in_memory().unwrap();
        let peer = DeviceId([8u8; 32]);
        let now_ms = crate::now_unix() * 1000;
        let chat = |text: &str, time: i64| Notification {
            key: "chat|Maya".into(),
            app: "com.whatsapp".into(),
            app_name: "WhatsApp".into(),
            title: Some("Maya".into()),
            text: Some(text.into()),
            sub: None,
            when: time,
            actions: Vec::new(),
            silent: false,
            ongoing: false,
            icon: None,
            image: None,
            live: None,
            conversation: Some(NotificationConversation {
                v: 1,
                title: "Maya".into(),
                group: false,
                avatar: None,
                messages: vec![NotificationChatMessage {
                    sender: Some("Maya".into()),
                    text: text.into(),
                    time,
                    self_sent: false,
                    avatar: None,
                }],
            }),
        };
        store.upsert_chat_notification(&peer, &chat("see you at 6", now_ms), None, false).unwrap();
        // The same chat's older notification, replayed after a reconnect.
        store.upsert_chat_notification(&peer, &chat("on my way", now_ms - 60_000), None, false).unwrap();
        let thread = store.chat_threads(&peer).unwrap().into_iter().next().unwrap();
        assert_eq!(thread.snippet, "see you at 6");
    }

    #[test]
    fn a_chat_stamped_in_the_future_does_not_age_out_the_others() {
        use nectarlink_protocol::messages::{NotificationChatMessage, NotificationConversation};
        let store = Store::in_memory().unwrap();
        let peer = DeviceId([7u8; 32]);
        let now_ms = crate::now_unix() * 1000;
        let chat = |title: &str, time: i64| Notification {
            key: format!("chat|{title}"),
            app: "com.whatsapp".into(),
            app_name: "WhatsApp".into(),
            title: Some(title.into()),
            text: Some("hi".into()),
            sub: None,
            when: time,
            actions: Vec::new(),
            silent: false,
            ongoing: false,
            icon: None,
            image: None,
            live: None,
            conversation: Some(NotificationConversation {
                v: 1,
                title: title.into(),
                group: false,
                avatar: None,
                messages: vec![NotificationChatMessage {
                    sender: Some(title.into()),
                    text: "hi".into(),
                    time,
                    self_sent: false,
                    avatar: None,
                }],
            }),
        };
        store.upsert_chat_notification(&peer, &chat("Maya", now_ms - 86_400_000), None, false).unwrap();
        // A phone with its clock a year ahead.
        store.upsert_chat_notification(&peer, &chat("Sam", now_ms + 365 * 86_400_000), None, false).unwrap();
        let titles: Vec<String> = store.chat_threads(&peer).unwrap().into_iter().map(|t| t.title).collect();
        assert!(titles.contains(&"Maya".to_owned()), "{titles:?}");
    }

    #[test]
    fn a_paired_device_cannot_claim_another_ones_install_id_later() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let (phone, other) = (DeviceId([0x01; 32]), DeviceId([0x02; 32]));
        let mut mine = info("My Phone");
        mine.install_id = Some("0123456789abcdef0123456789abcdef".into());
        let mut theirs = info("Their Phone");
        theirs.install_id = Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into());
        store.upsert_peer(&phone, &mine, 100).unwrap();
        store.upsert_peer(&other, &theirs, 200).unwrap();

        // On a later connection the other device reports this phone's install ID.
        let mut claimed = theirs.clone();
        claimed.install_id = mine.install_id.clone();
        store.update_info(&other, &claimed).unwrap();
        // Nor at the next start, when duplicates are merged.
        drop(store);
        let store = Store::open(dir.path()).unwrap();

        let mut peers = store.list_peers().unwrap();
        peers.sort_by_key(|p| p.id.0);
        assert_eq!(peers.iter().map(|p| p.id).collect::<Vec<_>>(), vec![phone, other], "both stay paired");
        assert_eq!(peers[1].info.install_id, theirs.install_id, "its own install ID is kept");
    }

    #[test]
    fn send_queue_fifo_clipboard_replacement_capacity_ttl_recovery_and_reassign() {
        let store = Store::in_memory().unwrap();
        let peer_a = DeviceId([0x11; 32]);
        let peer_b = DeviceId([0x22; 32]);
        let peer_a_repaired = DeviceId([0x33; 32]);
        store.upsert_peer(&peer_a, &info("Pixel 9"), 100).unwrap();
        store.upsert_peer(&peer_b, &info("Galaxy Tab"), 100).unwrap();
        store.upsert_peer(&peer_a_repaired, &info("Pixel 9"), 200).unwrap();
        let now_ms = 1_760_000_000_000i64;

        // 1. Clipboard replacement moves the replaced item to the end of the queue per peer
        // (and does not touch another peer's clip).
        let c1 = store
            .enqueue_send_item(
                "clip-a1",
                &peer_a,
                &QueuePayload::Clipboard { text: "First clip".into(), html: None, rtf: None },
                None,
                now_ms,
            )
            .unwrap();
        assert_eq!(c1.seq, 1);

        let _cb = store
            .enqueue_send_item(
                "clip-b1",
                &peer_b,
                &QueuePayload::Clipboard { text: "Peer B clip".into(), html: None, rtf: None },
                None,
                now_ms + 10,
            )
            .unwrap();

        let _link_a = store
            .enqueue_send_item(
                "link-a1",
                &peer_a,
                &QueuePayload::Link { url: "https://nectarlink.dev/docs".into(), title: None },
                None,
                now_ms + 20,
            )
            .unwrap();

        let c2 = store
            .enqueue_send_item(
                "clip-a2",
                &peer_a,
                &QueuePayload::Clipboard { text: "Latest clip".into(), html: None, rtf: None },
                None,
                now_ms + 30,
            )
            .unwrap();
        assert_eq!(c2.seq, 4, "clipboard replacement moves item to the end of the queue");
        assert_eq!(c2.title, "Latest clip");

        // Replacing text clipboard with ClipboardImage also moves to the end for the same peer.
        let c3 = store
            .enqueue_send_item(
                "clip-a3",
                &peer_a,
                &QueuePayload::ClipboardImage { mime: "image/png".into(), width: 100, height: 80 },
                Some(&[1, 2, 3, 4]),
                now_ms + 40,
            )
            .unwrap();
        assert_eq!(c3.seq, 5);
        assert_eq!(c3.kind, QueueKind::ClipboardImage);
        assert_eq!(c3.bytes_total, 4);

        // Replacing ClipboardImage with text Clipboard clears payload_blob and bytes_total.
        let c4 = store
            .enqueue_send_item(
                "clip-a4",
                &peer_a,
                &QueuePayload::Clipboard { text: "Final text clip".into(), html: None, rtf: None },
                None,
                now_ms + 50,
            )
            .unwrap();
        assert_eq!(c4.seq, 6);
        assert_eq!(c4.kind, QueueKind::Clipboard);
        assert_eq!(c4.bytes_total, 0);
        let (_, _, blob_after, _) = store.get_queue_item("clip-a4").unwrap().unwrap();
        assert!(blob_after.is_none(), "replacing ClipboardImage with text Clipboard frees payload_blob");

        let list_a = store.list_send_queue(Some(&peer_a)).unwrap();
        assert_eq!(list_a.len(), 2);
        assert_eq!(list_a[0].id, "link-a1");
        assert_eq!(list_a[1].id, "clip-a4");

        let list_b = store.list_send_queue(Some(&peer_b)).unwrap();
        assert_eq!(list_b.len(), 1);
        assert_eq!(list_b[0].id, "clip-b1");
        assert_eq!(list_b[0].title, "Peer B clip");

        // 2. Startup recovery: reset_interrupted_sending resets 'sending' back to 'waiting'.
        assert!(store.mark_queue_sending("link-a1").unwrap());
        assert_eq!(store.list_send_queue(Some(&peer_a)).unwrap()[0].status, QueueStatus::Sending);
        assert_eq!(store.reset_interrupted_sending().unwrap(), 1);
        assert_eq!(store.list_send_queue(Some(&peer_a)).unwrap()[0].status, QueueStatus::Waiting);

        // 3. Per-type TTL expiry (CameraRequest = 2 min, Link/Clip = 24h, Files = 7d).
        store
            .enqueue_send_item(
                "cam-a1",
                &peer_a,
                &QueuePayload::CameraRequest { request_id: "req-1".into(), mode: CameraCaptureMode::Photo },
                None,
                now_ms,
            )
            .unwrap();
        store
            .enqueue_send_item(
                "file-a1",
                &peer_a,
                &QueuePayload::Files {
                    files: vec![QueuedFileSpec {
                        path: PathBuf::from("/tmp/doc.pdf"),
                        name: "doc.pdf".into(),
                        folder: None,
                        size: 1024,
                        mtime_ms: now_ms,
                        staged: false,
                    }],
                },
                None,
                now_ms,
            )
            .unwrap();

        // Advance 3 minutes: only CameraRequest expires.
        let expired_2m = store.expire_stale_queue_items(now_ms + 3 * 60 * 1000).unwrap();
        assert_eq!(expired_2m, 1);
        let cam_item =
            store.list_send_queue(Some(&peer_a)).unwrap().into_iter().find(|i| i.id == "cam-a1").unwrap();
        assert_eq!(
            cam_item.status,
            QueueStatus::Expired { error: "Expired (camera requests expire after 2 minutes)".into() }
        );

        // Advance 25 hours: Link and Clipboard items expire and are removed, while Files (7d) stays Waiting.
        let expired_24h = store.expire_stale_queue_items(now_ms + 25 * 3600 * 1000).unwrap();
        assert_eq!(expired_24h, 3); // clip-a4, link-a1, clip-b1
        let file_item =
            store.list_send_queue(Some(&peer_a)).unwrap().into_iter().find(|i| i.id == "file-a1").unwrap();
        assert_eq!(file_item.status, QueueStatus::Waiting);

        // Advance 8 days: Files item expires with 7-day message.
        let expired_7d = store.expire_stale_queue_items(now_ms + 8 * 86_400 * 1000).unwrap();
        assert_eq!(expired_7d, 1);
        let file_item =
            store.list_send_queue(Some(&peer_a)).unwrap().into_iter().find(|i| i.id == "file-a1").unwrap();
        assert_eq!(
            file_item.status,
            QueueStatus::Expired { error: "Expired (waited more than 7 days)".into() }
        );

        // 4. Re-pairing peer ID reassignment moves queued items to the new peer ID:
        // the camera request and the file, as expired clips and links are gone.
        let moved = store.reassign_send_queue_peer(&peer_a, &peer_a_repaired).unwrap();
        assert_eq!(moved, 2);
        assert!(store.list_send_queue(Some(&peer_a)).unwrap().is_empty());
        assert_eq!(store.list_send_queue(Some(&peer_a_repaired)).unwrap().len(), 2);

        // Clear all finished (failed + expired) items.
        store.clear_finished_queue_items(None).unwrap();
        assert!(store.list_send_queue(None).unwrap().is_empty());

        // 5. Capacity limits: > 10 GiB file bytes and > 200 active items return Error::QueueFull.
        let huge_res = store.enqueue_send_item(
            "huge-file",
            &peer_b,
            &QueuePayload::Files {
                files: vec![QueuedFileSpec {
                    path: PathBuf::from("/tmp/huge.bin"),
                    name: "huge.bin".into(),
                    folder: None,
                    size: MAX_QUEUE_ACTIVE_BYTES + 1,
                    mtime_ms: now_ms,
                    staged: false,
                }],
            },
            None,
            now_ms,
        );
        assert!(matches!(huge_res, Err(Error::QueueFull(_))), "expected QueueFull for >10 GiB: {huge_res:?}");

        for i in 0..MAX_QUEUE_ACTIVE_ITEMS {
            store
                .enqueue_send_item(
                    &format!("item-{i}"),
                    &peer_b,
                    &QueuePayload::Link { url: format!("https://nectarlink.dev/{i}"), title: None },
                    None,
                    now_ms + i as i64,
                )
                .unwrap();
        }
        let over_cap = store.enqueue_send_item(
            "item-200",
            &peer_b,
            &QueuePayload::Link { url: "https://nectarlink.dev/overflow".into(), title: None },
            None,
            now_ms + 999,
        );
        assert!(
            matches!(over_cap, Err(Error::QueueFull(_))),
            "expected QueueFull at 201st item: {over_cap:?}"
        );
    }

    #[test]
    fn send_queue_preserved_on_clear_history_and_staged_files_deleted_on_all_terminal_paths() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let peer = DeviceId([0x44; 32]);
        store.upsert_peer(&peer, &info("Pixel 9"), 100).unwrap();
        let now_ms = 1_760_000_000_000i64;

        let stage_file = |id: &str, name: &str| -> QueuedFileSpec {
            let stage_dir = dir.path().join("queue_staging").join(id);
            std::fs::create_dir_all(&stage_dir).unwrap();
            let path = stage_dir.join(format!("0_{name}"));
            std::fs::write(&path, b"staged-bytes").unwrap();
            let meta = std::fs::metadata(&path).unwrap();
            QueuedFileSpec {
                path,
                name: name.into(),
                folder: None,
                size: meta.len(),
                mtime_ms: file_mtime_ms(&meta),
                staged: true,
            }
        };

        // Enqueue 5 staged items for the 5 terminal paths.
        for id in ["q-complete", "q-cancel", "q-expire", "q-fail-clear", "q-unpair"] {
            let spec = stage_file(id, "doc.txt");
            store
                .enqueue_send_item(id, &peer, &QueuePayload::Files { files: vec![spec] }, None, now_ms)
                .unwrap();
            assert!(dir.path().join("queue_staging").join(id).exists());
        }

        // Seed timeline and chat cache, then call clear_all_history_and_cache:
        // timeline and chat cache are cleared, while send_queue and staged files are preserved.
        store
            .insert_timeline(
                &NewTimelineEntry {
                    kind: TimelineKind::Link,
                    device_id: peer,
                    device_name: "Pixel 9".into(),
                    incoming: false,
                    timestamp: 100,
                    title: "Example".into(),
                    detail: "https://example.com".into(),
                    target: "https://example.com".into(),
                    size_bytes: 0,
                    duration_secs: 0,
                    ref_id: None,
                },
                100,
            )
            .unwrap();
        store.clear_all_history_and_cache().unwrap();
        assert_eq!(store.timeline_total_count().unwrap(), 0);
        assert_eq!(
            store.list_send_queue(Some(&peer)).unwrap().len(),
            5,
            "clear_all_history_and_cache must preserve send_queue items"
        );
        for id in ["q-complete", "q-cancel", "q-expire", "q-fail-clear", "q-unpair"] {
            assert!(
                dir.path().join("queue_staging").join(id).exists(),
                "staged directory for {id} must still exist after clear_all_history_and_cache"
            );
        }

        // Terminal path 1: Complete
        store.complete_queue_item("q-complete").unwrap();
        assert!(!dir.path().join("queue_staging").join("q-complete").exists());

        // Terminal path 2: Cancel (remove_queue_item)
        store.remove_queue_item("q-cancel").unwrap();
        assert!(!dir.path().join("queue_staging").join("q-cancel").exists());

        // Terminal path 3: Fail followed by clear_finished_queue_items
        store.mark_queue_failed("q-fail-clear", "Denied").unwrap();
        assert!(
            dir.path().join("queue_staging").join("q-fail-clear").exists(),
            "staged file remains while failed so user can retry before clearing"
        );
        store.clear_finished_queue_items(Some(&peer)).unwrap();
        assert!(!dir.path().join("queue_staging").join("q-fail-clear").exists());

        // Terminal path 4: Expire (isolate q-expire by setting its expires_at earlier than q-unpair)
        store
            .with(|c| {
                c.execute("UPDATE send_queue SET expires_at = ?1 WHERE id = 'q-expire'", [now_ms + 100])
            })
            .unwrap();
        assert_eq!(store.expire_stale_queue_items(now_ms + 200).unwrap(), 1);
        assert!(
            dir.path().join("queue_staging").join("q-expire").exists(),
            "an expired item keeps its staged file, so Retry still works"
        );
        store.clear_finished_queue_items(Some(&peer)).unwrap();
        assert!(!dir.path().join("queue_staging").join("q-expire").exists());

        // Terminal path 5: Unpair, which removes the device and its queue together.
        let (existed, dropped) = store.remove_peer_with_queue(&peer).unwrap();
        assert!(existed);
        assert!(dropped.iter().any(|(id, _)| id == "q-unpair"));
        assert!(!dir.path().join("queue_staging").join("q-unpair").exists());
        assert!(store.list_send_queue(Some(&peer)).unwrap().is_empty());
        assert!(store.get_peer(&peer).unwrap().is_none());
    }

    #[test]
    fn staging_cleanup_never_deletes_outside_the_items_staging_folder() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("precious.txt");
        std::fs::write(&outside, b"keep").unwrap();
        let staged = dir.path().join("queue_staging").join("q1").join("a.txt");
        std::fs::create_dir_all(staged.parent().unwrap()).unwrap();
        std::fs::write(&staged, b"x").unwrap();
        let spec = |path: PathBuf| QueuedFileSpec {
            path,
            name: "a.txt".into(),
            folder: None,
            size: 1,
            mtime_ms: 0,
            staged: true,
        };
        // A damaged row: a staged file outside staging, and a path escaping it.
        let payload = QueuePayload::Files {
            files: vec![
                spec(outside.clone()),
                spec(dir.path().join("queue_staging").join("q1").join("..").join("..").join("precious.txt")),
            ],
        };
        cleanup_staged_payload(Some(dir.path()), "q1", Some(&payload));
        assert!(outside.exists());
        assert!(!staged.exists(), "the item's own staging folder goes");
        cleanup_staged_payload(Some(dir.path()), "..", None);
        assert!(dir.path().join("queue_staging").exists(), "an id can't point at the parent");
    }

    #[test]
    fn a_finished_queue_item_isnt_moved_back_to_sending_or_waiting() {
        let store = Store::in_memory().unwrap();
        let peer = DeviceId([0x56; 32]);
        store.upsert_peer(&peer, &info("Pixel 9"), 100).unwrap();
        let id = "q-guard";
        let link = QueuePayload::Link { url: "https://example.com".into(), title: None };
        store.enqueue_send_item(id, &peer, &link, None, 1_000).unwrap();
        store.mark_queue_failed(id, "Denied").unwrap();
        assert!(!store.mark_queue_sending(id).unwrap(), "a failed item stays failed");
        assert!(!store.mark_queue_retry_or_wait(id, 1, 0, None).unwrap());
        assert!(store.retry_queue_item(id, 2_000).unwrap(), "Retry is the way back");
        assert!(store.mark_queue_sending(id).unwrap());
        assert!(store.mark_queue_retry_or_wait(id, 1, 0, None).unwrap());
    }

    #[test]
    fn send_queue_clipboard_image_size_cap_and_active_bytes_accounting() {
        let store = Store::in_memory().unwrap();
        let peer = DeviceId([0x55; 32]);
        store.upsert_peer(&peer, &info("Pixel 9"), 100).unwrap();
        let now_ms = 1_760_000_000_000i64;

        // 1. ClipboardImage > MAX_QUEUE_CLIPBOARD_IMAGE_BYTES (50 MiB) is rejected with QueueFull.
        let oversized = vec![0u8; MAX_QUEUE_CLIPBOARD_IMAGE_BYTES + 1];
        let res = store.enqueue_send_item(
            "img-too-big",
            &peer,
            &QueuePayload::ClipboardImage { mime: "image/png".into(), width: 100, height: 100 },
            Some(&oversized),
            now_ms,
        );
        assert!(matches!(res, Err(Error::QueueFull(_))), "expected QueueFull for >50 MiB image: {res:?}");

        // 2. ClipboardImage bytes count toward MAX_QUEUE_ACTIVE_BYTES (10 GiB).
        store
            .enqueue_send_item(
                "almost-full-file",
                &peer,
                &QueuePayload::Files {
                    files: vec![QueuedFileSpec {
                        path: PathBuf::from("/tmp/big.bin"),
                        name: "big.bin".into(),
                        folder: None,
                        size: MAX_QUEUE_ACTIVE_BYTES - 10,
                        mtime_ms: now_ms,
                        staged: false,
                    }],
                },
                None,
                now_ms,
            )
            .unwrap();
        let img_exceeding_total = store.enqueue_send_item(
            "img-overflow-total",
            &peer,
            &QueuePayload::ClipboardImage { mime: "image/png".into(), width: 10, height: 10 },
            Some(&[0u8; 20]),
            now_ms + 1,
        );
        assert!(
            matches!(img_exceeding_total, Err(Error::QueueFull(_))),
            "expected QueueFull when ClipboardImage pushes active bytes over 10 GiB: {img_exceeding_total:?}"
        );
    }

    #[test]
    fn send_queue_wake_ms_ignores_offline_peer_retries_and_clears_retry_delays() {
        let store = Store::in_memory().unwrap();
        let offline_peer = DeviceId([0x66; 32]);
        let online_peer = DeviceId([0x77; 32]);
        store.upsert_peer(&offline_peer, &info("Offline Phone"), 100).unwrap();
        store.upsert_peer(&online_peer, &info("Online Phone"), 100).unwrap();
        let now_ms = 1_760_000_000_000i64;

        let item_off = store
            .enqueue_send_item(
                "off-1",
                &offline_peer,
                &QueuePayload::Link { url: "https://nectarlink.dev/off".into(), title: None },
                None,
                now_ms,
            )
            .unwrap();
        store.mark_queue_retry_or_wait("off-1", 1, now_ms + 1_000, Some("disconnected")).unwrap();

        // When offline_peer is not in online_peers, next_queue_wake_ms ignores its next_retry_ms
        // and returns only min(expires_at).
        let wake_no_online = store.next_queue_wake_ms(&[]).unwrap();
        assert_eq!(wake_no_online, Some(item_off.expires_at));

        // When offline_peer IS in online_peers, its next_retry_ms is returned.
        let wake_with_online = store.next_queue_wake_ms(&[offline_peer]).unwrap();
        assert_eq!(wake_with_online, Some(now_ms + 1_000));

        // clear_send_queue_retry_delays resets next_retry_ms to 0 for the disconnected peer.
        assert_eq!(store.clear_send_queue_retry_delays(&offline_peer).unwrap(), 1);
        assert_eq!(
            store.next_queue_wake_ms(&[offline_peer, online_peer]).unwrap(),
            Some(item_off.expires_at)
        );
    }

    /// A protector that visibly changes the bytes, so a test can tell sealed from plain.
    struct Flip;
    impl crate::identity::KeyProtector for Flip {
        fn id(&self) -> u8 {
            9
        }
        fn protect(&self, plain: &[u8]) -> std::io::Result<Vec<u8>> {
            Ok(plain.iter().map(|b| b ^ 0x5a).collect())
        }
        fn unprotect(&self, sealed: &[u8]) -> std::io::Result<Vec<u8>> {
            self.protect(sealed)
        }
    }

    fn raw_queue_columns(store: &Store) -> Vec<(String, String, String, Option<Vec<u8>>)> {
        store
            .with(|c| {
                let mut stmt =
                    c.prepare("SELECT title, detail, payload_json, payload_blob FROM send_queue")?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
                rows.collect()
            })
            .unwrap()
    }

    #[test]
    fn queued_clips_are_sealed_on_disk_and_read_back_whole() {
        let store = Store::in_memory().unwrap();
        store.seal_queue_with(Arc::new(Flip)).unwrap();
        let peer = DeviceId([7; 32]);
        store.upsert_peer(&peer, &info("Phone"), 1).unwrap();
        let secret = "hunter2-the-password";
        let payload = QueuePayload::Clipboard { text: secret.into(), html: None, rtf: None };
        let item = store.enqueue_send_item("c1", &peer, &payload, None, 1000).unwrap();
        assert_eq!(item.title, secret, "the caller still sees what it queued");

        let (title, detail, json, _) = raw_queue_columns(&store).remove(0);
        assert!(json.starts_with(SEALED_PREFIX));
        for column in [&title, &detail, &json] {
            assert!(!column.contains(secret), "the clip leaked into {column}");
        }

        let listed = store.list_send_queue(None).unwrap();
        assert_eq!(listed[0].title, secret, "the queue shows it after opening it");
        let (_, back, _, _) = store.get_queue_item("c1").unwrap().unwrap();
        assert_eq!(back, payload);
    }

    #[test]
    fn queued_images_are_sealed_and_files_are_left_alone() {
        let store = Store::in_memory().unwrap();
        store.seal_queue_with(Arc::new(Flip)).unwrap();
        let peer = DeviceId([7; 32]);
        store.upsert_peer(&peer, &info("Phone"), 1).unwrap();
        let bytes = b"not really a png".to_vec();
        let image = QueuePayload::ClipboardImage { mime: "image/png".into(), width: 1, height: 1 };
        store.enqueue_send_item("i1", &peer, &image, Some(&bytes), 1000).unwrap();
        assert_ne!(raw_queue_columns(&store)[0].3.as_deref(), Some(bytes.as_slice()));
        let (_, _, blob, _) = store.get_queue_item("i1").unwrap().unwrap();
        assert_eq!(blob.as_deref(), Some(bytes.as_slice()));
    }

    #[test]
    fn clips_queued_before_sealing_are_sealed_at_startup() {
        let store = Store::in_memory().unwrap();
        let peer = DeviceId([7; 32]);
        store.upsert_peer(&peer, &info("Phone"), 1).unwrap();
        let secret = "from-an-older-version";
        let payload = QueuePayload::Clipboard { text: secret.into(), html: None, rtf: None };
        // No protector yet, and written the way an older version did.
        store.enqueue_send_item("old", &peer, &payload, None, 1000).unwrap();
        store
            .with(|c| {
                let json = serde_json::to_string(&payload).unwrap();
                c.execute(
                    "UPDATE send_queue SET title = ?1, payload_json = ?2 WHERE id = 'old'",
                    params![secret, json],
                )
            })
            .unwrap();
        assert_eq!(store.seal_queue_with(Arc::new(Flip)).unwrap(), 1);
        let (title, _, json, _) = raw_queue_columns(&store).remove(0);
        assert!(!title.contains(secret) && !json.contains(secret));
        assert_eq!(store.list_send_queue(None).unwrap()[0].title, secret);
    }

    #[test]
    fn clips_are_removed_at_their_deadline_and_files_stay_for_retry() {
        let store = Store::in_memory().unwrap();
        store.seal_queue_with(Arc::new(Flip)).unwrap();
        let peer = DeviceId([7; 32]);
        store.upsert_peer(&peer, &info("Phone"), 1).unwrap();
        let clip = QueuePayload::Clipboard { text: "x".into(), html: None, rtf: None };
        store.enqueue_send_item("c", &peer, &clip, None, 1000).unwrap();
        assert_eq!(store.expire_stale_queue_items(i64::MAX).unwrap(), 1);
        assert!(!store.has_queue_item("c"));
    }

    #[test]
    fn clearing_kinds_leaves_other_kinds() {
        let store = Store::in_memory().unwrap();
        let peer = DeviceId([7; 32]);
        store.upsert_peer(&peer, &info("Phone"), 1).unwrap();
        let clip = QueuePayload::Clipboard { text: "x".into(), html: None, rtf: None };
        let link = QueuePayload::Link { url: "https://example.com".into(), title: None };
        store.enqueue_send_item("c", &peer, &clip, None, 1000).unwrap();
        store.enqueue_send_item("l", &peer, &link, None, 1000).unwrap();
        assert_eq!(store.remove_queue_items_of_kinds(&[QueueKind::Clipboard]).unwrap(), 1);
        assert!(!store.has_queue_item("c") && store.has_queue_item("l"));
    }

    #[test]
    fn retry_queue_item_enforces_limits_and_deduplicates_clipboard() {
        let store = Store::in_memory().unwrap();
        let peer = DeviceId([0x88; 32]);
        store.upsert_peer(&peer, &info("Pixel 9"), 100).unwrap();
        let now_ms = 1_760_000_000_000i64;

        store
            .enqueue_send_item(
                "clip-failed",
                &peer,
                &QueuePayload::Clipboard { text: "old".into(), html: None, rtf: None },
                None,
                now_ms,
            )
            .unwrap();
        store.mark_queue_failed("clip-failed", "offline").unwrap();

        store
            .enqueue_send_item(
                "clip-waiting",
                &peer,
                &QueuePayload::Clipboard { text: "new".into(), html: None, rtf: None },
                None,
                now_ms + 1,
            )
            .unwrap();

        assert!(store.retry_queue_item("clip-failed", now_ms + 2).unwrap());
        let waiting: Vec<_> = store
            .list_send_queue(Some(&peer))
            .unwrap()
            .into_iter()
            .filter(|i| i.status == QueueStatus::Waiting)
            .collect();
        assert_eq!(waiting.len(), 1);
        assert_eq!(waiting[0].id, "clip-failed");
    }
}
