// SPDX-License-Identifier: GPL-3.0-or-later
//! Reaching a paired phone over Bluetooth when nothing else does (#313).
//!
//! While the setting is on and a paired phone is offline, this opens an
//! RFCOMM link to the phone's NectarLink service on a phone Windows is paired
//! with (Settings › Bluetooth & devices), and hands it to the core on a
//! loopback port, like an adb forward. The core checks it's really that
//! phone and runs the usual encrypted session over it (light messages only).

use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use nectarlink_core::{DeviceId, Node};
use windows::{
    Devices::{
        Bluetooth::{BluetoothDevice, BluetoothMajorClass},
        Enumeration::DeviceInformation,
    },
    Win32::Networking::WinSock::{
        INVALID_SOCKET, SD_BOTH, SEND_RECV_FLAGS, SOCK_STREAM, SOCKADDR, SOCKET, closesocket, connect, recv,
        send, shutdown, socket,
    },
    core::GUID,
};

/// [`nectarlink_core::bluetooth::SERVICE_UUID`].
const SERVICE: GUID = GUID::from_u128(0xa3f6b0e2_5c1d_4b7e_9e2a_7d4c1f8b6e53);
const AF_BTH: u16 = 32;
const BTHPROTO_RFCOMM: i32 = 3;
/// How often to look while a paired phone is offline, and the longest wait
/// after links that didn't work.
const FIRST_WAIT: Duration = Duration::from_secs(20);
const LONGEST_WAIT: Duration = Duration::from_secs(300);

static ENABLED: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);

#[repr(C, packed(1))]
struct SockaddrBth {
    address_family: u16,
    bt_addr: u64,
    service_class_id: GUID,
    port: u32,
}

/// Turns looking for phones over Bluetooth on or off.
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// Starts the loop (once); it only acts while [`set_enabled`] is on.
pub fn start(node: Node) {
    if STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    crate::core_host::spawn(async move {
        // Which Bluetooth device turned out to be which phone.
        let known: Arc<Mutex<HashMap<DeviceId, u64>>> = Arc::default();
        let mut wait = FIRST_WAIT;
        loop {
            tokio::time::sleep(wait).await;
            if !ENABLED.load(Ordering::Relaxed) {
                wait = FIRST_WAIT;
                continue;
            }
            let wanted = node.bluetooth_wanted();
            if wanted.is_empty() {
                wait = FIRST_WAIT;
                continue;
            }
            let connected = try_phones(&node, &wanted, &known).await;
            wait = if connected { FIRST_WAIT } else { (wait * 2).min(LONGEST_WAIT) };
        }
    });
}

/// Tries the paired Bluetooth phones, likeliest first. Whether one connected.
async fn try_phones(
    node: &Node,
    wanted: &[(DeviceId, String)],
    known: &Mutex<HashMap<DeviceId, u64>>,
) -> bool {
    let Ok(Ok(devices)) = tokio::task::spawn_blocking(paired_phones).await else { return false };
    let order = {
        let known = known.lock().unwrap_or_else(|e| e.into_inner());
        likeliest_first(&devices, wanted, &known)
    };
    for addr in order {
        let Ok(Ok(link)) = tokio::task::spawn_blocking(move || Link::open(addr)).await else { continue };
        let Ok(forward) = link.forward() else { continue };
        match node.connect_bluetooth(forward).await {
            Ok(phone) => {
                known.lock().unwrap_or_else(|e| e.into_inner()).insert(phone, addr);
                tracing::info!(phone = %phone.short(), "connected over Bluetooth");
                return true;
            }
            Err(e) => tracing::debug!(error = %e, "a Bluetooth link wasn't used"),
        }
    }
    false
}

/// Bluetooth addresses to try: ones known to be a wanted phone, then ones
/// named like a wanted phone, then the other phones.
fn likeliest_first(
    devices: &[(u64, String)],
    wanted: &[(DeviceId, String)],
    known: &HashMap<DeviceId, u64>,
) -> Vec<u64> {
    let rank = |(addr, name): &(u64, String)| {
        if wanted.iter().any(|(id, _)| known.get(id) == Some(addr)) {
            0
        } else if wanted.iter().any(|(_, n)| n.trim().eq_ignore_ascii_case(name.trim())) {
            1
        } else {
            2
        }
    };
    let mut ranked: Vec<(u8, u64)> = devices.iter().map(|d| (rank(d), d.0)).collect();
    ranked.sort_by_key(|(r, _)| *r);
    ranked.into_iter().map(|(_, addr)| addr).collect()
}

/// Phones Windows is paired with over Bluetooth: `(address, name)`.
fn paired_phones() -> windows::core::Result<Vec<(u64, String)>> {
    let selector = BluetoothDevice::GetDeviceSelectorFromPairingState(true)?;
    let infos = DeviceInformation::FindAllAsyncAqsFilter(&selector)?.join()?;
    let mut phones = Vec::new();
    for info in infos {
        let Ok(device) = BluetoothDevice::FromIdAsync(&info.Id()?)?.join() else { continue };
        let is_phone = device
            .ClassOfDevice()
            .and_then(|c| c.MajorClass())
            .is_ok_and(|c| c == BluetoothMajorClass::Phone);
        if is_phone {
            phones.push((device.BluetoothAddress()?, device.Name()?.to_string()));
        }
    }
    Ok(phones)
}

/// An RFCOMM link to a phone's NectarLink service.
struct Link(SOCKET);

// The socket is only used from one thread at a time, or for one direction each.
unsafe impl Send for Link {}
unsafe impl Sync for Link {}

impl Link {
    fn open(bt_addr: u64) -> std::io::Result<Link> {
        // Starts Winsock, which std does on first use.
        drop(TcpListener::bind(("127.0.0.1", 0)));
        // SAFETY: plain Winsock calls on a socket this function owns.
        unsafe {
            let s = socket(i32::from(AF_BTH), SOCK_STREAM, BTHPROTO_RFCOMM)?;
            if s == INVALID_SOCKET {
                return Err(std::io::Error::last_os_error());
            }
            let addr = SockaddrBth { address_family: AF_BTH, bt_addr, service_class_id: SERVICE, port: 0 };
            let len = std::mem::size_of::<SockaddrBth>() as i32;
            if connect(s, std::ptr::from_ref(&addr).cast::<SOCKADDR>(), len) != 0 {
                let e = std::io::Error::last_os_error();
                closesocket(s);
                return Err(e);
            }
            Ok(Link(s))
        }
    }

    /// Exposes the link on a loopback port for the core, which connects once.
    fn forward(self) -> std::io::Result<SocketAddr> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let addr = listener.local_addr()?;
        std::thread::Builder::new().name("bt-link".into()).spawn(move || {
            let Ok((core, _)) = listener.accept() else { return };
            let _ = core.set_nodelay(true);
            self.pipe(core);
        })?;
        Ok(addr)
    }

    /// Copies both ways until either side ends.
    fn pipe(self, core: TcpStream) {
        let link = Arc::new(self);
        let Ok(mut core_read) = core.try_clone() else { return };
        let up = {
            let link = link.clone();
            let mut core_write = core;
            std::thread::spawn(move || {
                let mut buf = vec![0u8; 16 * 1024];
                // SAFETY: recv into a buffer this thread owns.
                loop {
                    let n = unsafe { recv(link.0, &mut buf, SEND_RECV_FLAGS(0)) };
                    if n <= 0 || core_write.write_all(&buf[..n as usize]).is_err() {
                        break;
                    }
                }
                let _ = core_write.shutdown(Shutdown::Both);
            })
        };
        let mut buf = vec![0u8; 16 * 1024];
        while let Ok(n) = core_read.read(&mut buf) {
            if n == 0 || !link.send_all(&buf[..n]) {
                break;
            }
        }
        // SAFETY: ends the link for the other thread too.
        unsafe { shutdown(link.0, SD_BOTH) };
        let _ = core_read.shutdown(Shutdown::Both);
        let _ = up.join();
    }

    fn send_all(&self, mut data: &[u8]) -> bool {
        while !data.is_empty() {
            // SAFETY: send from a slice this call borrows.
            let n = unsafe { send(self.0, data, SEND_RECV_FLAGS(0)) };
            if n <= 0 {
                return false;
            }
            data = &data[n as usize..];
        }
        true
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        // SAFETY: the socket this link owns, closed once.
        unsafe { closesocket(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_service_matches_the_core() {
        assert_eq!(format!("{SERVICE:?}").to_lowercase(), nectarlink_core::bluetooth::SERVICE_UUID);
    }

    #[test]
    fn known_and_named_phones_come_first() {
        let pixel = DeviceId([1; 32]);
        let devices =
            vec![(10, "Headset".to_owned()), (20, "Pixel 9".to_owned()), (30, "Old phone".to_owned())];
        let wanted = vec![(pixel, "pixel 9".to_owned())];
        assert_eq!(likeliest_first(&devices, &wanted, &HashMap::new()), vec![20, 10, 30]);
        let known = HashMap::from([(pixel, 30)]);
        assert_eq!(likeliest_first(&devices, &wanted, &known), vec![30, 20, 10]);
    }

    #[test]
    fn the_address_has_the_size_windows_expects() {
        assert_eq!(std::mem::size_of::<SockaddrBth>(), 30);
    }
}
