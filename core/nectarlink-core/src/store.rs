// SPDX-License-Identifier: MPL-2.0
//! SQLite storage: the trust store of paired devices, their last known
//! addresses and capabilities, and per-device settings.

use std::{
    collections::{BTreeSet, HashMap},
    net::SocketAddr,
    path::Path,
    sync::Mutex,
};

use nectarlink_protocol::{
    DeviceId,
    messages::{DeviceInfo, DeviceKind, Notification, PcWakeInfo, PowerLevel, ScreenShape},
};
use rusqlite::{Connection, OptionalExtension, params};

use crate::{
    Error, Result,
    timeline::{
        DEFAULT_TIMELINE_PAGE_LIMIT, MAX_TIMELINE_PAGE_LIMIT, NewTimelineEntry, TimelineEntry, TimelineKind,
        TimelinePage, TimelineQuery, TimelineRetention,
    },
};

const FILE_NAME: &str = "nectarlink.db";

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

#[derive(Debug)]
pub(crate) struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(dir: &Path) -> Result<Self> {
        Self::init(Connection::open(dir.join(FILE_NAME))?)
    }

    #[cfg(test)]
    pub fn in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrate(&conn)?;
        Ok(Store { conn: Mutex::new(conn) })
    }

    fn with<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T> {
        let conn = self.conn.lock().map_err(|_| Error::Internal("store lock poisoned".into()))?;
        f(&conn).map_err(Error::from)
    }

    /// Adds or replaces a paired device.
    pub fn upsert_peer(&self, id: &DeviceId, info: &DeviceInfo, paired_at: i64) -> Result<()> {
        let screen = encode_screen(info.screen.as_ref());
        self.with(|c| {
            c.execute(
                "INSERT INTO peers (id, name, kind, os, os_ver, model, accent, paired_at, screen)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name, kind = excluded.kind, os = excluded.os,
                    os_ver = excluded.os_ver, model = excluded.model,
                    accent = COALESCE(excluded.accent, peers.accent),
                    paired_at = excluded.paired_at,
                    screen = CASE WHEN excluded.screen = '' THEN peers.screen ELSE excluded.screen END",
                params![
                    id.as_bytes().as_slice(),
                    info.name,
                    kind_to_str(info.kind),
                    info.os,
                    info.os_ver,
                    info.model,
                    info.accent.map(i64::from),
                    paired_at,
                    screen
                ],
            )
            .map(drop)
        })
    }

    /// Updates the stored description of a device (e.g. after a rename).
    pub fn update_info(&self, id: &DeviceId, info: &DeviceInfo) -> Result<()> {
        let screen = encode_screen(info.screen.as_ref());
        self.with(|c| {
            c.execute(
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
                    screen
                ],
            )?;
            c.execute(
                "UPDATE timeline SET device_name = ?2 WHERE device_id = ?1",
                params![id.as_bytes().as_slice(), info.name],
            )
            .map(drop)
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
        self.with(|c| {
            let mut stmt = c.prepare("SELECT toggle, enabled FROM peer_toggles WHERE peer = ?1")?;
            stmt.query_map([id.as_bytes().as_slice()], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? != 0))
            })?
            .collect()
        })
    }

    pub fn set_toggle(&self, id: &DeviceId, toggle: &str, enabled: bool) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO peer_toggles (peer, toggle, enabled) VALUES (?1, ?2, ?3)
                 ON CONFLICT(peer, toggle) DO UPDATE SET enabled = excluded.enabled",
                params![id.as_bytes().as_slice(), toggle, i64::from(enabled)],
            )
            .map(drop)
        })
    }

    /// Removes a device. Returns whether it existed.
    pub fn remove_peer(&self, id: &DeviceId) -> Result<bool> {
        let peer_str = id.to_string();
        self.with(|c| {
            let _ = c.execute("DELETE FROM chat_messages WHERE peer = ?1", [&peer_str]);
            let _ = c.execute("DELETE FROM chat_threads WHERE peer = ?1", [&peer_str]);
            c.execute("DELETE FROM peers WHERE id = ?1", [id.as_bytes().as_slice()]).map(|n| n > 0)
        })
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
            let mut stmt = c.prepare(&format!("{SELECT} ORDER BY paired_at"))?;
            stmt.query_map([], row_to_peer)?.collect()
        })
    }

    // ---- Timeline ----

    pub fn timeline_retention(&self) -> Result<TimelineRetention> {
        self.with(read_retention_conn)
    }

    pub fn set_timeline_retention(&self, retention: TimelineRetention, now_unix: i64) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO meta (key, value) VALUES ('timeline_max_days', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![retention.max_days.to_string()],
            )?;
            c.execute(
                "INSERT INTO meta (key, value) VALUES ('timeline_max_entries', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![retention.max_entries.to_string()],
            )?;
            purge_timeline_conn(c, now_unix, retention)
        })
    }

    pub fn insert_timeline(&self, entry: &NewTimelineEntry, now_unix: i64) -> Result<i64> {
        self.with(|c| {
            let retention = read_retention_conn(c)?;
            c.execute(
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
            let id = c.last_insert_rowid();
            purge_timeline_conn(c, now_unix, retention)?;
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
            let mut stmt = c.prepare(&page_sql)?;
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
        }
        let thread_id = chat_thread_id(&n.app, &conv.title);
        let reply_action_id = n.actions.iter().find(|a| a.reply).map(|a| a.id.clone());
        let latest = conv.messages.last().expect("checked non-empty");
        let snippet = latest.text.clone();
        let last_sender = if latest.self_sent { None } else { latest.sender.clone() };
        let updated_ms = conv.messages.iter().map(|m| m.time).max().unwrap_or(n.when).max(n.when);
        let peer_str = peer.to_string();
        let icon_bytes = app_icon.or(n.icon.as_deref());

        self.with(|c| {
            let existing: Vec<(String, String, bool)> = {
                let mut stmt = c.prepare(
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
                let exists: Option<i64> = c
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
                    c.execute(
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
            c.execute(
                "INSERT INTO chat_threads (
                    peer, thread_id, app, app_name, notification_key, reply_action_id,
                    title, is_group, snippet, last_sender, updated_ms, unread, active, avatar, app_icon
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 1, ?13, ?14)
                 ON CONFLICT(peer, thread_id) DO UPDATE SET
                    app = excluded.app,
                    app_name = excluded.app_name,
                    notification_key = excluded.notification_key,
                    reply_action_id = excluded.reply_action_id,
                    title = excluded.title,
                    is_group = excluded.is_group,
                    snippet = excluded.snippet,
                    last_sender = excluded.last_sender,
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

            prune_chat_conn(c, &peer_str, &thread_id)?;
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
            let exists_thread: Option<i64> = c
                .query_row(
                    "SELECT 1 FROM chat_threads WHERE peer = ?1 AND thread_id = ?2",
                    params![peer_str, thread_id],
                    |r| r.get(0),
                )
                .optional()?;
            if exists_thread.is_none() {
                return Ok(false);
            }
            let dup: Option<i64> = c
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
                c.execute(
                    "INSERT INTO chat_messages (peer, thread_id, sender, text, time_ms, self_sent, avatar)
                     VALUES (?1, ?2, NULL, ?3, ?4, 1, NULL)",
                    params![peer_str, thread_id, trimmed, time_ms],
                )?;
            }
            let snippet = format!("You: {trimmed}");
            c.execute(
                "UPDATE chat_threads
                 SET snippet = ?3,
                     last_sender = NULL,
                     updated_ms = MAX(updated_ms, ?4),
                     unread = 0
                 WHERE peer = ?1 AND thread_id = ?2",
                params![peer_str, thread_id, snippet, time_ms],
            )?;
            prune_chat_messages_conn(c, &peer_str, thread_id)?;
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
            let mut stmt = c.prepare(&format!(
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
            let mut stmt = c.prepare(
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
            c.execute(
                "DELETE FROM chat_messages WHERE peer = ?1 AND thread_id = ?2",
                params![peer_str, thread_id],
            )?;
            c.execute(
                "DELETE FROM chat_threads WHERE peer = ?1 AND thread_id = ?2",
                params![peer_str, thread_id],
            )
            .map(|n| n > 0)
        })
    }

    pub fn clear_chat_threads(&self) -> Result<usize> {
        self.with(|c| {
            c.execute("DELETE FROM chat_messages", [])?;
            c.execute("DELETE FROM chat_threads", [])
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
}

const SELECT: &str = "SELECT id, name, kind, os, os_ver, model, accent, paired_at, last_seen, last_addrs, \
     caps, power, wake_info, screen FROM peers";

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
            "the database was created by a newer Nectarlink (schema {current}); please update"
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

        // Unpairing forgets the device's settings too.
        store.remove_peer(&id).unwrap();
        assert!(store.toggles(&id).unwrap().is_empty());
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
            PcWakeInfo { macs: vec!["38:a7:46:37:2e:64".into()], broadcasts: vec!["192.168.1.255".into()] };
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
}
