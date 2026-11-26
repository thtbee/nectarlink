// SPDX-License-Identifier: GPL-3.0-or-later
//! Reaching a paired phone over Bluetooth when nothing else does (#313).
//!
//! While the setting is on and a paired phone is offline, this asks BlueZ to
//! connect NectarLink's RFCOMM service on phones the PC is paired with
//! (`org.bluez.Device1.ConnectProfile`). BlueZ looks the service up on the
//! phone and hands the connected socket to the profile registered here
//! (`org.bluez.Profile1.NewConnection`), which exposes it to the core on a
//! loopback port, like an adb forward. The core checks it's really that phone
//! and runs the usual encrypted session over it (light messages only).

use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    os::unix::net::UnixStream,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use nectarlink_core::{DeviceId, Node};
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const PROFILE_PATH: &str = "/app/nectarlink/BluetoothLink";
const FIRST_WAIT: Duration = Duration::from_secs(20);
const LONGEST_WAIT: Duration = Duration::from_secs(300);

static ENABLED: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);

type ManagedObjects = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

/// Turns looking for phones over Bluetooth on or off.
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// Starts the loop (once); it only acts while [`set_enabled`] is on.
pub fn start(node: Node) {
    if STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    let spawned = std::thread::Builder::new().name("bt-link".into()).spawn(move || run(node));
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "can't start looking for phones over Bluetooth");
    }
}

/// The profile BlueZ hands connected RFCOMM sockets to.
struct Profile {
    node: Node,
    /// The phone `ConnectProfile` was last called for, by its BlueZ path.
    last: Arc<Mutex<Option<OwnedObjectPath>>>,
    /// Whether the link it handed over was taken by the core.
    connected: Arc<AtomicBool>,
}

#[zbus::interface(name = "org.bluez.Profile1")]
impl Profile {
    fn release(&self) {}

    fn new_connection(
        &self,
        device: OwnedObjectPath,
        fd: zbus::zvariant::OwnedFd,
        _properties: HashMap<String, OwnedValue>,
    ) {
        let socket = UnixStream::from(std::os::fd::OwnedFd::from(fd));
        let Ok(listener) = TcpListener::bind(("127.0.0.1", 0)) else { return };
        let Ok(forward) = listener.local_addr() else { return };
        let piped = std::thread::Builder::new().name("bt-link-pipe".into()).spawn(move || {
            if let Ok((core, _)) = listener.accept() {
                let _ = core.set_nodelay(true);
                pipe(socket, core);
            }
        });
        if piped.is_err() {
            return;
        }
        let node = self.node.clone();
        let connected = self.connected.clone();
        *self.last.lock().unwrap_or_else(|e| e.into_inner()) = Some(device);
        crate::core_host::spawn(async move {
            match node.connect_bluetooth(forward).await {
                Ok(phone) => {
                    connected.store(true, Ordering::SeqCst);
                    tracing::info!(phone = %phone.short(), "connected over Bluetooth");
                }
                Err(e) => tracing::debug!(error = %e, "a Bluetooth link wasn't used"),
            }
        });
    }

    fn request_disconnection(&self, _device: OwnedObjectPath) {}
}

fn run(node: Node) {
    let last = Arc::new(Mutex::new(None));
    let connected = Arc::new(AtomicBool::new(false));
    let profile = Profile { node: node.clone(), last: last.clone(), connected: connected.clone() };
    let conn = match zbus::blocking::connection::Builder::system()
        .and_then(|b| b.serve_at(PROFILE_PATH, profile))
        .and_then(|b| b.build())
    {
        Ok(conn) => conn,
        Err(e) => {
            tracing::info!(error = %e, "no D-Bus system bus: no Bluetooth links");
            return;
        }
    };
    let mut registered = false;
    // Which BlueZ device turned out to be which phone.
    let mut known: HashMap<DeviceId, OwnedObjectPath> = HashMap::new();
    let mut wait = FIRST_WAIT;
    loop {
        std::thread::sleep(wait);
        if !ENABLED.load(Ordering::Relaxed) {
            wait = FIRST_WAIT;
            continue;
        }
        let wanted = node.bluetooth_wanted();
        if wanted.is_empty() {
            wait = FIRST_WAIT;
            continue;
        }
        if !registered {
            registered = register(&conn);
            if !registered {
                wait = LONGEST_WAIT;
                continue;
            }
        }
        let phones = paired_phones(&conn);
        let mut linked = false;
        for path in likeliest_first(&phones, &wanted, &known) {
            connected.store(false, Ordering::SeqCst);
            if connect_profile(&conn, &path).is_err() {
                continue;
            }
            // The core takes the link a moment after BlueZ hands it over.
            for _ in 0..50 {
                if connected.load(Ordering::SeqCst) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            if connected.load(Ordering::SeqCst) {
                let still = node.bluetooth_wanted();
                if let Some((id, _)) = wanted.iter().find(|(id, _)| !still.iter().any(|(s, _)| s == id)) {
                    known.insert(*id, path);
                }
                linked = true;
                break;
            }
        }
        *last.lock().unwrap_or_else(|e| e.into_inner()) = None;
        wait = if linked { FIRST_WAIT } else { (wait * 2).min(LONGEST_WAIT) };
    }
}

fn register(conn: &zbus::blocking::Connection) -> bool {
    let path = zbus::zvariant::ObjectPath::try_from(PROFILE_PATH).expect("a valid object path");
    let options: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::from([
        ("Name", zbus::zvariant::Value::from("NectarLink")),
        ("Role", zbus::zvariant::Value::from("client")),
    ]);
    let result = conn.call_method(
        Some("org.bluez"),
        "/org/bluez",
        Some("org.bluez.ProfileManager1"),
        "RegisterProfile",
        &(path, nectarlink_core::bluetooth::SERVICE_UUID, options),
    );
    if let Err(e) = &result {
        tracing::info!(error = %e, "BlueZ didn't take the Bluetooth link profile");
    }
    result.is_ok()
}

fn connect_profile(conn: &zbus::blocking::Connection, device: &OwnedObjectPath) -> zbus::Result<()> {
    conn.call_method(
        Some("org.bluez"),
        device.as_str(),
        Some("org.bluez.Device1"),
        "ConnectProfile",
        &(nectarlink_core::bluetooth::SERVICE_UUID,),
    )
    .map(drop)
}

/// Paired phones BlueZ knows: `(object path, name)`.
fn paired_phones(conn: &zbus::blocking::Connection) -> Vec<(OwnedObjectPath, String)> {
    let Ok(reply) = conn.call_method(
        Some("org.bluez"),
        "/",
        Some("org.freedesktop.DBus.ObjectManager"),
        "GetManagedObjects",
        &(),
    ) else {
        return Vec::new();
    };
    let Ok(objects) = reply.body().deserialize::<ManagedObjects>() else { return Vec::new() };
    phones_in(&objects)
}

fn phones_in(objects: &ManagedObjects) -> Vec<(OwnedObjectPath, String)> {
    let text = |props: &HashMap<String, OwnedValue>, key: &str| {
        props.get(key).and_then(|v| String::try_from(v.clone()).ok()).unwrap_or_default()
    };
    objects
        .iter()
        .filter_map(|(path, interfaces)| {
            let device = interfaces.get("org.bluez.Device1")?;
            let paired = device.get("Paired").and_then(|v| bool::try_from(v.clone()).ok()).unwrap_or(false);
            let icon = text(device, "Icon");
            (paired && (icon.is_empty() || icon.starts_with("phone")))
                .then(|| (path.clone(), text(device, "Alias")))
        })
        .collect()
}

/// Devices to try: one known to be a wanted phone, then ones named like a
/// wanted phone, then the other phones.
fn likeliest_first(
    phones: &[(OwnedObjectPath, String)],
    wanted: &[(DeviceId, String)],
    known: &HashMap<DeviceId, OwnedObjectPath>,
) -> Vec<OwnedObjectPath> {
    let rank = |(path, name): &(OwnedObjectPath, String)| {
        if wanted.iter().any(|(id, _)| known.get(id) == Some(path)) {
            0
        } else if wanted.iter().any(|(_, n)| n.trim().eq_ignore_ascii_case(name.trim())) {
            1
        } else {
            2
        }
    };
    let mut ranked: Vec<(u8, OwnedObjectPath)> = phones.iter().map(|p| (rank(p), p.0.clone())).collect();
    ranked.sort_by_key(|(r, _)| *r);
    ranked.into_iter().map(|(_, p)| p).collect()
}

/// Copies both ways between the RFCOMM socket and the core until either ends.
fn pipe(link: UnixStream, core: TcpStream) {
    let (Ok(mut link_read), Ok(mut core_read)) = (link.try_clone(), core.try_clone()) else { return };
    let (mut link_write, mut core_write) = (link, core);
    let up = std::thread::spawn(move || {
        copy(&mut link_read, &mut core_write);
        let _ = core_write.shutdown(Shutdown::Both);
        let _ = link_read.shutdown(Shutdown::Both);
    });
    copy(&mut core_read, &mut link_write);
    let _ = link_write.shutdown(Shutdown::Both);
    let _ = core_read.shutdown(Shutdown::Both);
    let _ = up.join();
}

fn copy(from: &mut impl Read, to: &mut impl Write) {
    let mut buf = vec![0u8; 16 * 1024];
    while let Ok(n) = from.read(&mut buf) {
        if n == 0 || to.write_all(&buf[..n]).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(paired: bool, icon: &str, alias: &str) -> HashMap<String, HashMap<String, OwnedValue>> {
        let value = |v: zbus::zvariant::Value<'_>| OwnedValue::try_from(v).unwrap();
        HashMap::from([(
            "org.bluez.Device1".to_owned(),
            HashMap::from([
                ("Paired".to_owned(), value(paired.into())),
                ("Icon".to_owned(), value(icon.into())),
                ("Alias".to_owned(), value(alias.into())),
            ]),
        )])
    }

    #[test]
    fn only_paired_phones_are_tried() {
        let path = |p: &str| OwnedObjectPath::try_from(p).unwrap();
        let objects: ManagedObjects = HashMap::from([
            (path("/org/bluez/hci0/dev_1"), device(true, "phone", "Pixel 9")),
            (path("/org/bluez/hci0/dev_2"), device(true, "audio-headset", "Buds")),
            (path("/org/bluez/hci0/dev_3"), device(false, "phone", "Someone else's")),
        ]);
        assert_eq!(phones_in(&objects), vec![(path("/org/bluez/hci0/dev_1"), "Pixel 9".to_owned())]);
    }

    #[test]
    fn known_and_named_phones_come_first() {
        let path = |p: &str| OwnedObjectPath::try_from(p).unwrap();
        let pixel = DeviceId([1; 32]);
        let phones = vec![(path("/a"), "Old".to_owned()), (path("/b"), "Pixel 9".to_owned())];
        let wanted = vec![(pixel, "pixel 9".to_owned())];
        assert_eq!(likeliest_first(&phones, &wanted, &HashMap::new()), vec![path("/b"), path("/a")]);
        let known = HashMap::from([(pixel, path("/a"))]);
        assert_eq!(likeliest_first(&phones, &wanted, &known), vec![path("/a"), path("/b")]);
    }
}
