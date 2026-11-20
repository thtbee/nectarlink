// SPDX-License-Identifier: MPL-2.0
//! Agent clients and their activity, in `agents.db` next to NectarLink's own
//! data. A token is stored only as its SHA-256; the token itself lives in a
//! file only its user can read, which the app writes when the client is made.

use std::{
    collections::BTreeSet,
    path::Path,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::def::{Grant, Toolset};

/// Activity is kept this many days at most.
pub const ACTIVITY_DAYS: i64 = 30;
/// And at most this many rows.
pub const ACTIVITY_ROWS: i64 = 20_000;

/// One agent installation: "Claude Code", "Cursor", "my backup script".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Client {
    pub id: String,
    /// Unique and user-visible; what `--agent` names.
    pub label: String,
    pub display_name: String,
    /// `mcp-stdio`, `cli` or `http`.
    pub kind: String,
    pub grants: BTreeSet<Grant>,
    /// Paired devices it may use; `None` means all of them.
    pub devices: Option<BTreeSet<String>>,
    pub read_only: bool,
    /// Toolsets listed to it; empty means the default set.
    pub toolsets: BTreeSet<Toolset>,
    /// Actions with a first-use confirmation the user allowed for good.
    pub remembered: BTreeSet<String>,
    pub paused: bool,
    pub created: i64,
    pub last_used: Option<i64>,
}

impl Client {
    pub fn grants_text(&self) -> Vec<String> {
        self.grants.iter().map(|g| g.to_text()).collect()
    }

    /// Whether this client holds `want` (write includes read).
    pub fn holds(&self, want: Grant) -> bool {
        self.grants.iter().any(|g| {
            g.scope == want.scope && (g.access == want.access || g.access == crate::def::Access::Write)
        })
    }
}

/// One row of the activity log. Who and what, never the content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Activity {
    pub id: i64,
    pub ts: i64,
    pub client_id: String,
    pub client_name: String,
    pub channel: String,
    pub action: String,
    pub device: Option<String>,
    pub summary: String,
    /// `ok`, `error`, or why it didn't run (`scope_denied`, `confirm_denied`…).
    pub decision: String,
    /// `none`, `pc` or `remembered`.
    pub confirm: String,
    pub error_code: Option<String>,
    pub duration_ms: i64,
}

#[derive(Debug, thiserror::Error)]
#[error("agent store: {0}")]
pub struct StoreError(String);

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError(e.to_string())
    }
}

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// A new token: 32 random bytes, as 64 hex digits.
pub fn new_token() -> String {
    let bytes: [u8; 32] = rand::random();
    data_encoding::HEXLOWER.encode(&bytes)
}

pub fn token_hash(token: &str) -> String {
    data_encoding::HEXLOWER.encode(&Sha256::digest(token.as_bytes()))
}

/// Whether a label can be used: short, plain, and safe in a file name.
pub fn label_ok(label: &str) -> bool {
    (2..=40).contains(&label.len())
        && label.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !label.starts_with('-')
}

#[derive(Debug)]
pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Store, StoreError> {
        Store::with(Connection::open(path)?)
    }

    pub fn in_memory() -> Result<Store, StoreError> {
        Store::with(Connection::open_in_memory()?)
    }

    fn with(conn: Connection) -> Result<Store, StoreError> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS agent_clients (
               id TEXT PRIMARY KEY,
               label TEXT NOT NULL UNIQUE,
               display_name TEXT NOT NULL,
               kind TEXT NOT NULL,
               token_sha256 TEXT NOT NULL UNIQUE,
               grants TEXT NOT NULL,
               devices TEXT,
               read_only INTEGER NOT NULL DEFAULT 0,
               toolsets TEXT NOT NULL DEFAULT '',
               remembered TEXT NOT NULL DEFAULT '',
               paused INTEGER NOT NULL DEFAULT 0,
               created INTEGER NOT NULL,
               last_used INTEGER
             );
             CREATE TABLE IF NOT EXISTS agent_activity (
               id INTEGER PRIMARY KEY,
               ts INTEGER NOT NULL,
               client_id TEXT NOT NULL,
               client_name TEXT NOT NULL,
               channel TEXT NOT NULL,
               action TEXT NOT NULL,
               device TEXT,
               summary TEXT NOT NULL,
               decision TEXT NOT NULL,
               confirm TEXT NOT NULL,
               error_code TEXT,
               duration_ms INTEGER NOT NULL DEFAULT 0
             );
             CREATE INDEX IF NOT EXISTS agent_activity_ts ON agent_activity(ts);",
        )?;
        Ok(Store { conn: Mutex::new(conn) })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Makes a client and returns it with its token, which isn't stored.
    pub fn create_client(
        &self,
        label: &str,
        display_name: &str,
        kind: &str,
        grants: &BTreeSet<Grant>,
    ) -> Result<(Client, String), StoreError> {
        if !label_ok(label) {
            return Err(StoreError(format!("invalid label {label:?}")));
        }
        let token = new_token();
        let id_bytes: [u8; 6] = rand::random();
        let id = format!("c_{}", data_encoding::HEXLOWER.encode(&id_bytes));
        let created = now();
        self.lock().execute(
            "INSERT INTO agent_clients (id, label, display_name, kind, token_sha256, grants, created)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, label, display_name, kind, token_hash(&token), join_grants(grants), created],
        )?;
        let client = self.client(&id)?.ok_or_else(|| StoreError("just made, now missing".into()))?;
        Ok((client, token))
    }

    /// A new token for an existing client; the old one stops working.
    pub fn rotate_token(&self, id: &str) -> Result<String, StoreError> {
        let token = new_token();
        let n = self.lock().execute(
            "UPDATE agent_clients SET token_sha256 = ?1 WHERE id = ?2",
            params![token_hash(&token), id],
        )?;
        if n == 0 {
            return Err(StoreError("no such client".into()));
        }
        Ok(token)
    }

    pub fn client(&self, id: &str) -> Result<Option<Client>, StoreError> {
        Ok(self
            .lock()
            .query_row(&format!("SELECT {COLUMNS} FROM agent_clients WHERE id = ?1"), [id], row_client)
            .optional()?)
    }

    pub fn client_by_label(&self, label: &str) -> Result<Option<Client>, StoreError> {
        Ok(self
            .lock()
            .query_row(&format!("SELECT {COLUMNS} FROM agent_clients WHERE label = ?1"), [label], row_client)
            .optional()?)
    }

    /// The client a token belongs to. The token is hashed first, so the
    /// lookup never compares the secret itself.
    pub fn client_by_token(&self, token: &str) -> Result<Option<Client>, StoreError> {
        if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(None);
        }
        Ok(self
            .lock()
            .query_row(
                &format!("SELECT {COLUMNS} FROM agent_clients WHERE token_sha256 = ?1"),
                [token_hash(&token.to_ascii_lowercase())],
                row_client,
            )
            .optional()?)
    }

    pub fn clients(&self) -> Result<Vec<Client>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!("SELECT {COLUMNS} FROM agent_clients ORDER BY created"))?;
        let rows = stmt.query_map([], row_client)?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Saves the settings a user can change on a client.
    pub fn update_client(&self, c: &Client) -> Result<(), StoreError> {
        let devices = c.devices.as_ref().map(|d| d.iter().cloned().collect::<Vec<_>>().join(","));
        let toolsets = c.toolsets.iter().map(|t| t.as_str()).collect::<Vec<_>>().join(",");
        let remembered = c.remembered.iter().cloned().collect::<Vec<_>>().join(",");
        self.lock().execute(
            "UPDATE agent_clients SET display_name = ?2, grants = ?3, devices = ?4, read_only = ?5,
               toolsets = ?6, remembered = ?7, paused = ?8 WHERE id = ?1",
            params![
                c.id,
                c.display_name,
                join_grants(&c.grants),
                devices,
                c.read_only,
                toolsets,
                remembered,
                c.paused
            ],
        )?;
        Ok(())
    }

    pub fn delete_client(&self, id: &str) -> Result<bool, StoreError> {
        Ok(self.lock().execute("DELETE FROM agent_clients WHERE id = ?1", [id])? > 0)
    }

    pub fn touch(&self, id: &str) {
        let _ =
            self.lock().execute("UPDATE agent_clients SET last_used = ?2 WHERE id = ?1", params![id, now()]);
    }

    /// Appends an activity row and prunes old ones now and then.
    pub fn log(&self, a: &Activity) -> Result<(), StoreError> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO agent_activity (ts, client_id, client_name, channel, action, device, summary,
               decision, confirm, error_code, duration_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                a.ts,
                a.client_id,
                a.client_name,
                a.channel,
                a.action,
                a.device,
                a.summary,
                a.decision,
                a.confirm,
                a.error_code,
                a.duration_ms
            ],
        )?;
        let id = conn.last_insert_rowid();
        if id % 100 == 0 {
            conn.execute("DELETE FROM agent_activity WHERE ts < ?1", [now() - ACTIVITY_DAYS * 86_400])?;
            conn.execute(
                "DELETE FROM agent_activity WHERE id <= (SELECT MAX(id) FROM agent_activity) - ?1",
                [ACTIVITY_ROWS],
            )?;
        }
        Ok(())
    }

    /// Newest first; only `client_id`'s rows when given.
    pub fn activity(&self, client_id: Option<&str>, limit: usize) -> Result<Vec<Activity>, StoreError> {
        let conn = self.lock();
        let sql =
            "SELECT id, ts, client_id, client_name, channel, action, device, summary, decision, confirm,
                     error_code, duration_ms FROM agent_activity
                   WHERE (?1 IS NULL OR client_id = ?1) ORDER BY id DESC LIMIT ?2";
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt
            .query_map(params![client_id, limit as i64], |r| {
                Ok(Activity {
                    id: r.get(0)?,
                    ts: r.get(1)?,
                    client_id: r.get(2)?,
                    client_name: r.get(3)?,
                    channel: r.get(4)?,
                    action: r.get(5)?,
                    device: r.get(6)?,
                    summary: r.get(7)?,
                    decision: r.get(8)?,
                    confirm: r.get(9)?,
                    error_code: r.get(10)?,
                    duration_ms: r.get(11)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn clear_activity(&self) -> Result<(), StoreError> {
        self.lock().execute("DELETE FROM agent_activity", [])?;
        Ok(())
    }
}

const COLUMNS: &str = "id, label, display_name, kind, grants, devices, read_only, toolsets, remembered, paused, created, last_used";

fn row_client(r: &rusqlite::Row<'_>) -> rusqlite::Result<Client> {
    let grants: String = r.get(4)?;
    let devices: Option<String> = r.get(5)?;
    let toolsets: String = r.get(7)?;
    let remembered: String = r.get(8)?;
    Ok(Client {
        id: r.get(0)?,
        label: r.get(1)?,
        display_name: r.get(2)?,
        kind: r.get(3)?,
        grants: grants.split(',').filter_map(Grant::parse).collect(),
        devices: devices.map(|d| d.split(',').filter(|s| !s.is_empty()).map(str::to_owned).collect()),
        read_only: r.get(6)?,
        toolsets: toolsets.split(',').filter_map(Toolset::parse).collect(),
        remembered: remembered.split(',').filter(|s| !s.is_empty()).map(str::to_owned).collect(),
        paused: r.get(9)?,
        created: r.get(10)?,
        last_used: r.get(11)?,
    })
}

fn join_grants(grants: &BTreeSet<Grant>) -> String {
    grants.iter().map(|g| g.to_text()).collect::<Vec<_>>().join(",")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::def::{Access, Scope};

    fn grants(list: &[(Scope, Access)]) -> BTreeSet<Grant> {
        list.iter().map(|(s, a)| Grant::new(*s, *a)).collect()
    }

    #[test]
    fn a_client_is_found_by_its_token_and_only_by_it() {
        let store = Store::in_memory().unwrap();
        let (client, token) = store
            .create_client(
                "claude-code",
                "Claude Code",
                "mcp-stdio",
                &grants(&[(Scope::Messages, Access::Write)]),
            )
            .unwrap();
        assert_eq!(token.len(), 64);
        assert_eq!(store.client_by_token(&token).unwrap().unwrap().id, client.id);
        assert_eq!(store.client_by_token(&token.to_uppercase()).unwrap().unwrap().id, client.id);
        assert!(store.client_by_token(&new_token()).unwrap().is_none());
        assert!(store.client_by_token("short").unwrap().is_none());
        // Write includes read.
        assert!(client.holds(Grant::new(Scope::Messages, Access::Read)));
        assert!(!client.holds(Grant::new(Scope::Calls, Access::Read)));

        let new = store.rotate_token(&client.id).unwrap();
        assert!(store.client_by_token(&token).unwrap().is_none(), "the old token stops working");
        assert!(store.client_by_token(&new).unwrap().is_some());
    }

    #[test]
    fn labels_are_plain_and_unique() {
        let store = Store::in_memory().unwrap();
        assert!(store.create_client("Bad Label", "x", "cli", &BTreeSet::new()).is_err());
        assert!(store.create_client("../x", "x", "cli", &BTreeSet::new()).is_err());
        store.create_client("cursor", "Cursor", "mcp-stdio", &BTreeSet::new()).unwrap();
        assert!(store.create_client("cursor", "Cursor", "mcp-stdio", &BTreeSet::new()).is_err());
    }

    #[test]
    fn settings_round_trip() {
        let store = Store::in_memory().unwrap();
        let (mut c, _) = store.create_client("script", "Script", "cli", &BTreeSet::new()).unwrap();
        c.grants = grants(&[(Scope::Files, Access::Write), (Scope::Devices, Access::Read)]);
        c.devices = Some(["abc".to_owned()].into());
        c.read_only = true;
        c.toolsets = [Toolset::Files].into();
        c.remembered = ["files.send".to_owned()].into();
        c.paused = true;
        store.update_client(&c).unwrap();
        let back = store.client(&c.id).unwrap().unwrap();
        assert_eq!(back, c);
        assert!(store.delete_client(&c.id).unwrap());
        assert!(store.client(&c.id).unwrap().is_none());
    }

    #[test]
    fn activity_is_listed_newest_first_and_per_client() {
        let store = Store::in_memory().unwrap();
        let row = |client: &str, action: &str| Activity {
            id: 0,
            ts: now(),
            client_id: client.into(),
            client_name: client.into(),
            channel: "cli".into(),
            action: action.into(),
            device: None,
            summary: String::new(),
            decision: "ok".into(),
            confirm: "none".into(),
            error_code: None,
            duration_ms: 1,
        };
        store.log(&row("a", "devices.list")).unwrap();
        store.log(&row("b", "sms.send")).unwrap();
        store.log(&row("a", "media.control")).unwrap();
        let all = store.activity(None, 10).unwrap();
        assert_eq!(
            all.iter().map(|a| a.action.as_str()).collect::<Vec<_>>(),
            ["media.control", "sms.send", "devices.list"]
        );
        assert_eq!(store.activity(Some("a"), 10).unwrap().len(), 2);
        store.clear_activity().unwrap();
        assert!(store.activity(None, 10).unwrap().is_empty());
    }
}
