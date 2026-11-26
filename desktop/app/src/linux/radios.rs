// SPDX-License-Identifier: GPL-3.0-or-later
//! This PC's Wi-Fi and Bluetooth, for phones (#636): Wi-Fi through
//! NetworkManager's `WirelessEnabled` (what `nmcli radio wifi` switches), and
//! Bluetooth through BlueZ's adapter `Powered`. Either is left out when the
//! PC has no such device or the service isn't running.

use std::collections::HashMap;

use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const NM: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const BLUEZ: &str = "org.bluez";
const ADAPTER: &str = "org.bluez.Adapter1";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

type ManagedObjects = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

/// Whether a network interface here is a Wi-Fi one.
fn has_wifi_device() -> bool {
    std::fs::read_dir("/sys/class/net")
        .map(|dir| {
            dir.flatten().any(|i| i.path().join("wireless").exists() || i.path().join("phy80211").exists())
        })
        .unwrap_or(false)
}

fn get_bool(
    conn: &zbus::blocking::Connection,
    dest: &str,
    path: &str,
    iface: &str,
    name: &str,
) -> Option<bool> {
    let reply = conn.call_method(Some(dest), path, Some(PROPERTIES), "Get", &(iface, name)).ok()?;
    let value: OwnedValue = reply.body().deserialize().ok()?;
    bool::try_from(value).ok()
}

fn set_bool(
    conn: &zbus::blocking::Connection,
    dest: &str,
    path: &str,
    iface: &str,
    name: &str,
    on: bool,
) -> Result<(), String> {
    conn.call_method(Some(dest), path, Some(PROPERTIES), "Set", &(iface, name, Value::from(on)))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// BlueZ's first adapter, by path.
fn adapter(conn: &zbus::blocking::Connection) -> Option<OwnedObjectPath> {
    let reply = conn
        .call_method(Some(BLUEZ), "/", Some("org.freedesktop.DBus.ObjectManager"), "GetManagedObjects", &())
        .ok()?;
    let objects: ManagedObjects = reply.body().deserialize().ok()?;
    objects
        .into_iter()
        .filter(|(_, ifaces)| ifaces.contains_key(ADAPTER))
        .map(|(path, _)| path)
        .min_by(|a, b| a.as_str().cmp(b.as_str()))
}

/// Whether Wi-Fi and Bluetooth are on, each `None` where it can't be told.
pub fn read() -> (Option<bool>, Option<bool>) {
    super::non_tokio(|| {
        let Ok(conn) = zbus::blocking::Connection::system() else { return (None, None) };
        let wifi = has_wifi_device().then(|| get_bool(&conn, NM, NM_PATH, NM, "WirelessEnabled")).flatten();
        let bluetooth =
            adapter(&conn).and_then(|path| get_bool(&conn, BLUEZ, path.as_str(), ADAPTER, "Powered"));
        (wifi, bluetooth)
    })
}

/// Whether this PC has a radio NectarLink can report.
pub fn available() -> bool {
    let (wifi, bluetooth) = read();
    wifi.is_some() || bluetooth.is_some()
}

/// Turns Wi-Fi and/or Bluetooth on or off. NetworkManager and BlueZ decide
/// whether this user may (polkit and BlueZ's D-Bus policy).
pub fn set(wifi: Option<bool>, bluetooth: Option<bool>) -> Result<(), String> {
    super::non_tokio(|| {
        let conn = zbus::blocking::Connection::system().map_err(|e| e.to_string())?;
        if let Some(on) = wifi {
            set_bool(&conn, NM, NM_PATH, NM, "WirelessEnabled", on)?;
        }
        if let Some(on) = bluetooth {
            let path = adapter(&conn).ok_or("no Bluetooth adapter")?;
            set_bool(&conn, BLUEZ, path.as_str(), ADAPTER, "Powered", on)?;
        }
        Ok(())
    })
}
