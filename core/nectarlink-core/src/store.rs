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
    messages::{DeviceInfo, DeviceKind, PcWakeInfo, PowerLevel},
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

const SELECT: &str = "SELECT id, name, kind, os, os_ver, model, accent, paired_at, last_seen, last_addrs, \
     caps, power, wake_info FROM peers";

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
}
