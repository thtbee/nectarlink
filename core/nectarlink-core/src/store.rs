// SPDX-License-Identifier: MPL-2.0
//! SQLite storage: the trust store of paired devices and their last known
//! addresses.

use std::{net::SocketAddr, path::Path, sync::Mutex};

use nectarlink_protocol::{
    DeviceId,
    messages::{DeviceInfo, DeviceKind},
};
use rusqlite::{Connection, OptionalExtension, params};

use crate::{Error, Result};

const FILE_NAME: &str = "nectarlink.db";

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
];

/// A paired device as stored on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PeerRecord {
    pub id: DeviceId,
    pub info: DeviceInfo,
    pub paired_at: i64,
    pub last_seen: Option<i64>,
    pub last_addrs: Vec<SocketAddr>,
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
        self.with(|c| {
            c.execute(
                "INSERT INTO peers (id, name, kind, os, os_ver, model, accent, paired_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name, kind = excluded.kind, os = excluded.os,
                    os_ver = excluded.os_ver, model = excluded.model, accent = excluded.accent,
                    paired_at = excluded.paired_at",
                params![
                    id.as_bytes().as_slice(),
                    info.name,
                    kind_to_str(info.kind),
                    info.os,
                    info.os_ver,
                    info.model,
                    info.accent.map(i64::from),
                    paired_at
                ],
            )
            .map(drop)
        })
    }

    /// Updates the stored description of a device (e.g. after a rename).
    pub fn update_info(&self, id: &DeviceId, info: &DeviceInfo) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE peers SET name = ?2, kind = ?3, os = ?4, os_ver = ?5, model = ?6, accent = ?7
                 WHERE id = ?1",
                params![
                    id.as_bytes().as_slice(),
                    info.name,
                    kind_to_str(info.kind),
                    info.os,
                    info.os_ver,
                    info.model,
                    info.accent.map(i64::from)
                ],
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

    /// Removes a device. Returns whether it existed.
    pub fn remove_peer(&self, id: &DeviceId) -> Result<bool> {
        self.with(|c| c.execute("DELETE FROM peers WHERE id = ?1", [id.as_bytes().as_slice()]).map(|n| n > 0))
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
}

const SELECT: &str =
    "SELECT id, name, kind, os, os_ver, model, accent, paired_at, last_seen, last_addrs FROM peers";

fn row_to_peer(row: &rusqlite::Row<'_>) -> rusqlite::Result<PeerRecord> {
    let id: Vec<u8> = row.get(0)?;
    let id: [u8; 32] = id
        .try_into()
        .map_err(|_| rusqlite::Error::InvalidColumnType(0, "id".into(), rusqlite::types::Type::Blob))?;
    let kind: String = row.get(2)?;
    let accent: Option<i64> = row.get(6)?;
    let addrs: String = row.get(9)?;
    Ok(PeerRecord {
        id: DeviceId(id),
        info: DeviceInfo {
            name: row.get(1)?,
            kind: kind_from_str(&kind),
            os: row.get(3)?,
            os_ver: row.get(4)?,
            model: row.get(5)?,
            accent: accent.and_then(|a| u32::try_from(a).ok()),
        },
        paired_at: row.get(7)?,
        last_seen: row.get(8)?,
        // Skip entries that no longer parse instead of failing the whole read.
        last_addrs: addrs.split(',').filter_map(|a| a.parse().ok()).collect(),
    })
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

    fn info(name: &str) -> DeviceInfo {
        DeviceInfo {
            name: name.into(),
            kind: DeviceKind::Phone,
            os: "android".into(),
            os_ver: "16".into(),
            model: None,
            accent: Some(0xFF8A5100),
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
        store.update_info(&id, &info("Pixel 9")).unwrap();
        let peer = store.get_peer(&id).unwrap().unwrap();
        assert_eq!(peer.last_seen, Some(200));
        assert_eq!(peer.last_addrs, addrs);
        assert_eq!(peer.info.name, "Pixel 9");

        assert!(store.remove_peer(&id).unwrap());
        assert!(!store.remove_peer(&id).unwrap());
        assert!(store.list_peers().unwrap().is_empty());
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
}
