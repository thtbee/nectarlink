// SPDX-License-Identifier: MPL-2.0
//! USB cable transport and USB auto-pairing via ADB port forwarding.
//!
//! When a phone is plugged into a PC over USB with USB debugging enabled:
//! 1. The phone listens on loopback TCP `127.0.0.1:53319` ([`DEFAULT_USB_PORT`]),
//!    or the next free port of [`USB_PORT_COUNT`] when another Android profile
//!    with NectarLink (a work profile, a cloned app) already has it: all
//!    profiles share the phone's loopback.
//! 2. The PC's ADB watcher forwards a local ephemeral TCP port to each of those
//!    phone ports (`adb forward tcp:0 tcp:53319`, ...) and opens a TCP
//!    connection to each one that answers.
//! 3. Both sides exchange a [`UsbHello`] header (`NLQU` magic + length-prefixed
//!    JSON) to learn each other's [`DeviceId`] and display name before any QUIC
//!    packets flow.
//! 4. Each side binds a loopback UDP socket (`127.0.0.1:0`) and bridges UDP
//!    datagrams over the length-prefixed TCP stream to its local iroh QUIC
//!    endpoint, so QUIC sessions (control stream, file transfers, screen
//!    mirroring, audio, camera, clipboard) run unmodified over the USB cable
//!    even when Wi-Fi is off.
//! 5. If the two devices are not paired yet, the PC automatically initiates the
//!    nearby 6-digit SAS code ceremony over the USB tunnel; the phone accepts
//!    the ceremony on a verified USB loopback connection without requiring manual
//!    host mode, and both users must confirm the 6-digit code before any trust
//!    is stored.

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use nectarlink_protocol::DeviceId;
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
};
use tokio_util::sync::CancellationToken;

use crate::{Error, Result, node::Shared};

/// Default loopback TCP port the Android app listens on for the USB tunnel.
pub const DEFAULT_USB_PORT: u16 = 53319;
/// Loopback ports, from [`DEFAULT_USB_PORT`] on, that a phone may listen on: one per
/// Android profile running NectarLink, since profiles share the phone's loopback.
pub const USB_PORT_COUNT: u16 = 4;

pub use nectarlink_adb::DEFAULT_ADB_ADDR;

const MAGIC: &[u8; 4] = b"NLQU";
const MAX_HELLO_BYTES: usize = 4096;
const HELLO_TIMEOUT: Duration = Duration::from_secs(3);
const ADB_RETRY_DELAY: Duration = Duration::from_secs(2);
/// How often, at most, the adb server is started while adb can't be reached.
const ADB_START_EVERY: Duration = Duration::from_secs(60);
/// How often, at most, a profile port nothing answered on is tried again.
const SPARE_PORT_MAX_DELAY: Duration = Duration::from_secs(30);
/// How long a tunnel may send frames without getting any back before it is
/// treated as dead. A tunnel whose far end stopped answering (a frozen app,
/// a wedged adb) never closes by itself, so without this the worker would sit
/// in it and never open a new one until the cable is replugged.
const TUNNEL_SILENCE_LIMIT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UsbHello {
    v: u8,
    id: String,
    name: String,
}

async fn write_hello(stream: &mut TcpStream, id: &DeviceId, name: &str) -> Result<()> {
    let hello = UsbHello { v: 1, id: id.to_string(), name: name.to_owned() };
    let payload = serde_json::to_vec(&hello).map_err(|e| Error::Internal(e.to_string()))?;
    if payload.is_empty() || payload.len() > MAX_HELLO_BYTES {
        return Err(Error::Protocol("invalid USB hello length".into()));
    }
    stream.write_all(MAGIC).await.map_err(|e| Error::Network(e.to_string()))?;
    stream
        .write_all(&(payload.len() as u16).to_be_bytes())
        .await
        .map_err(|e| Error::Network(e.to_string()))?;
    stream.write_all(&payload).await.map_err(|e| Error::Network(e.to_string()))?;
    stream.flush().await.map_err(|e| Error::Network(e.to_string()))?;
    Ok(())
}

async fn read_hello(stream: &mut TcpStream) -> Result<(DeviceId, String)> {
    let mut magic = [0u8; 4];
    stream.read_exact(&mut magic).await.map_err(|e| Error::Network(e.to_string()))?;
    if &magic != MAGIC {
        return Err(Error::Protocol("invalid USB tunnel magic".into()));
    }
    let mut len_buf = [0u8; 2];
    stream.read_exact(&mut len_buf).await.map_err(|e| Error::Network(e.to_string()))?;
    let len = u16::from_be_bytes(len_buf) as usize;
    if !(1..=MAX_HELLO_BYTES).contains(&len) {
        return Err(Error::Protocol(format!("invalid USB hello size {len}")));
    }
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).await.map_err(|e| Error::Network(e.to_string()))?;
    let hello: UsbHello =
        serde_json::from_slice(&payload).map_err(|e| Error::Protocol(format!("invalid USB hello: {e}")))?;
    let peer_id: DeviceId =
        hello.id.parse().map_err(|_| Error::Protocol("invalid DeviceId in USB hello".into()))?;
    let name: String = hello.name.trim().chars().take(64).collect();
    Ok((peer_id, name))
}

async fn exchange_hello(
    stream: &mut TcpStream,
    local_id: &DeviceId,
    local_name: &str,
) -> Result<(DeviceId, String)> {
    tokio::time::timeout(HELLO_TIMEOUT, async {
        write_hello(stream, local_id, local_name).await?;
        read_hello(stream).await
    })
    .await
    .map_err(|_| Error::Timeout)?
}

/// Binds the phone's USB tunnel listener: `preferred_port`, or the next free one of
/// [`USB_PORT_COUNT`] (another Android profile with NectarLink holds it; profiles share
/// the loopback). A port outside that range would never be reached by the PC, so there
/// is no listener then. `0` binds any free port (tests).
fn bind_phone_listener(preferred_port: u16) -> Option<std::net::TcpListener> {
    if preferred_port == 0 {
        return std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).ok();
    }
    let listener = (0..USB_PORT_COUNT).find_map(|offset| {
        let port = preferred_port.checked_add(offset)?;
        std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).ok()
    });
    if listener.is_none() {
        tracing::warn!(port = preferred_port, "no free USB tunnel port; USB connections are off");
    }
    listener
}

/// Starts the phone-side loopback TCP listener (`127.0.0.1:<preferred_port>`,
/// falling back to an ephemeral port if busy). Returns the bound TCP port.
pub(crate) fn start_phone_listener(shared: &Arc<Shared>, preferred_port: u16) -> Option<u16> {
    let std_listener = bind_phone_listener(preferred_port)?;
    std_listener.set_nonblocking(true).ok()?;
    let port = std_listener.local_addr().ok()?.port();
    let listener = TcpListener::from_std(std_listener).ok()?;
    shared.usb_listener_port.store(port, Ordering::Relaxed);

    let weak = Arc::downgrade(shared);
    let cancel = shared.cancel.clone();
    tokio::spawn(async move {
        loop {
            let accepted = tokio::select! {
                _ = cancel.cancelled() => return,
                res = listener.accept() => res,
            };
            let (mut stream, peer_addr) = match accepted {
                Ok(pair) => pair,
                Err(e) => {
                    tracing::debug!(error = %e, "USB listener accept failed");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            };
            if !peer_addr.ip().to_canonical().is_loopback() {
                continue;
            }
            let _ = stream.set_nodelay(true);
            let Some(node) = weak.upgrade() else { return };
            tokio::spawn(async move {
                let local_name = node.local.read().unwrap_or_else(|e| e.into_inner()).device.name.clone();
                let Ok(Ok((pc_id, pc_name))) =
                    tokio::time::timeout(HELLO_TIMEOUT, read_hello(&mut stream)).await
                else {
                    return;
                };
                if pc_id == node.id {
                    return;
                }
                let Some(iroh_port) = node.bound_v4_port() else { return };
                let Ok(udp) = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await else { return };
                let Ok(bridge_addr) = udp.local_addr() else { return };
                let bridge_port = bridge_addr.port();

                let tunnel_cancel = node.cancel.child_token();
                node.register_usb_bridge_port(bridge_port, pc_id);
                node.on_usb_tunnel_accepted(pc_id, pc_name, bridge_addr, tunnel_cancel.clone());

                if !matches!(
                    tokio::time::timeout(HELLO_TIMEOUT, write_hello(&mut stream, &node.id, &local_name))
                        .await,
                    Ok(Ok(()))
                ) {
                    node.unregister_usb_bridge_port(bridge_port);
                    node.on_usb_tunnel_disconnected(pc_id, bridge_addr).await;
                    return;
                }

                let default_target = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), iroh_port);
                relay_tcp_udp(stream, udp, default_target, tunnel_cancel).await;

                node.unregister_usb_bridge_port(bridge_port);
                node.on_usb_tunnel_disconnected(pc_id, bridge_addr).await;
            });
        }
    });
    Some(port)
}

/// Connects to a forwarded phone USB tunnel TCP address (`127.0.0.1:<port>`),
/// runs the `NLQU` handshake, spawns the UDP-over-TCP bridge, and triggers
/// session connection (if paired) or USB auto-pairing (if unpaired).
pub(crate) async fn connect_usb_tunnel(shared: &Arc<Shared>, forwarded_tcp: SocketAddr) -> Result<DeviceId> {
    let mut stream = tokio::time::timeout(HELLO_TIMEOUT, TcpStream::connect(forwarded_tcp))
        .await
        .map_err(|_| Error::Timeout)?
        .map_err(|e| Error::Network(e.to_string()))?;
    let _ = stream.set_nodelay(true);

    let local_name = shared.local.read().unwrap_or_else(|e| e.into_inner()).device.name.clone();
    let (phone_id, phone_name) = exchange_hello(&mut stream, &shared.id, &local_name).await?;
    if phone_id == shared.id {
        return Err(Error::Protocol("cannot connect USB tunnel to self".into()));
    }
    let iroh_port =
        shared.bound_v4_port().ok_or_else(|| Error::Internal("no local IPv4 QUIC port".into()))?;
    let udp = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.map_err(|e| Error::Network(e.to_string()))?;
    let bridge_addr = udp.local_addr().map_err(|e| Error::Network(e.to_string()))?;
    let bridge_port = bridge_addr.port();

    let tunnel_cancel = shared.cancel.child_token();
    shared.register_usb_bridge_port(bridge_port, phone_id);

    let node = shared.clone();
    let relay_cancel = tunnel_cancel.clone();
    let default_target = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), iroh_port);
    tokio::spawn(async move {
        relay_tcp_udp(stream, udp, default_target, relay_cancel).await;
        node.unregister_usb_bridge_port(bridge_port);
        node.on_usb_tunnel_disconnected(phone_id, bridge_addr).await;
    });

    shared.on_usb_tunnel_connected(phone_id, phone_name, bridge_addr, tunnel_cancel, true, None, None);
    Ok(phone_id)
}

/// Starts the PC-side ADB watcher that tracks USB-connected Android devices,
/// sets up `adb forward tcp:0 tcp:53319` (and the other profile ports), and
/// maintains a USB tunnel to each NectarLink profile that answers.
pub(crate) fn start_adb_watcher(shared: &Arc<Shared>, adb_addr: SocketAddr) {
    let weak = Arc::downgrade(shared);
    let cancel = shared.cancel.clone();
    tokio::spawn(async move {
        let mut active: HashMap<String, CancellationToken> = HashMap::new();
        let mut started_adb: Option<Instant> = None;
        loop {
            if cancel.is_cancelled() {
                break;
            }
            let Some(node) = weak.upgrade() else { break };
            // The loop comes round when adb can't be reached: start its server, now and then
            // at most once a minute, for an adb installed later or a server that went away.
            // A thread, not spawn_blocking, so a slow adb never holds up the runtime's shutdown.
            if started_adb.is_none_or(|at| at.elapsed() >= ADB_START_EVERY) {
                started_adb = Some(Instant::now());
                std::thread::spawn(move || nectarlink_adb::start_server(adb_addr));
            }
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            let track_cancel = cancel.child_token();
            let track_cancel_task = track_cancel.clone();
            tokio::spawn(async move {
                let _ = nectarlink_adb::track_devices(adb_addr, track_cancel_task, move |devices| {
                    let _ = tx.send(devices);
                })
                .await;
            });
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    _ = node.usb_poke.notified() => {
                        if let Ok(devices) = nectarlink_adb::list_devices(adb_addr).await {
                            reconcile_adb_devices(&node, adb_addr, &devices, &mut active);
                        }
                    }
                    maybe_devices = rx.recv() => {
                        match maybe_devices {
                            Some(devices) => reconcile_adb_devices(&node, adb_addr, &devices, &mut active),
                            None => break,
                        }
                    }
                }
            }
            track_cancel.cancel();
            drop(node);
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = tokio::time::sleep(ADB_RETRY_DELAY) => {}
            }
        }
        for (_, token) in active.drain() {
            token.cancel();
        }
    });
}

fn reconcile_adb_devices(
    shared: &Arc<Shared>,
    adb_addr: SocketAddr,
    devices: &[nectarlink_adb::AdbDevice],
    active: &mut HashMap<String, CancellationToken>,
) {
    active.retain(|serial, token| {
        let still_online = devices
            .iter()
            .any(|d| d.serial == *serial && d.is_online() && nectarlink_adb::is_serial_allowed(&d.serial));
        if !still_online {
            token.cancel();
        }
        still_online
    });

    for dev in devices {
        if !dev.is_online() || !nectarlink_adb::is_serial_allowed(&dev.serial) {
            continue;
        }
        // Wireless adb isn't a cable: the connection would go through adb's TCP over
        // Wi-Fi, which stutters where a direct one over the same Wi-Fi doesn't.
        if nectarlink_adb::is_network_serial(&dev.serial) {
            continue;
        }
        if active.contains_key(&dev.serial) {
            continue;
        }
        let token = shared.cancel.child_token();
        active.insert(dev.serial.clone(), token.clone());
        let node = shared.clone();
        let serial = dev.serial.clone();
        tokio::spawn(async move {
            run_serial_worker(node, adb_addr, serial, token).await;
        });
    }
}

/// One plugged-in phone: a tunnel per profile port (see [`USB_PORT_COUNT`]).
async fn run_serial_worker(
    shared: Arc<Shared>,
    adb_addr: SocketAddr,
    serial: String,
    cancel: CancellationToken,
) {
    let profiles = Arc::new(Mutex::new(SerialProfiles::default()));
    let mut ports = tokio::task::JoinSet::new();
    for offset in 0..USB_PORT_COUNT {
        ports.spawn(run_port_worker(
            shared.clone(),
            adb_addr,
            serial.clone(),
            DEFAULT_USB_PORT + offset,
            cancel.clone(),
            profiles.clone(),
        ));
    }
    while ports.join_next().await.is_some() {}
}

/// What the tunnels to one phone's profiles reached, keyed by phone port.
#[derive(Default)]
struct SerialProfiles {
    paired: HashMap<u16, DeviceId>,
}

impl SerialProfiles {
    /// Whether a profile other than the one on `port` is already paired with this PC.
    /// Then the phone is known here, and a second profile (work, a cloned app) is paired
    /// only by hand, rather than asking each time it's plugged in.
    fn another_is_paired(&self, port: u16) -> bool {
        self.paired.keys().any(|p| *p != port)
    }
}

async fn run_port_worker(
    shared: Arc<Shared>,
    adb_addr: SocketAddr,
    serial: String,
    phone_port: u16,
    cancel: CancellationToken,
    profiles: Arc<Mutex<SerialProfiles>>,
) {
    let mut delay = ADB_RETRY_DELAY;
    while !cancel.is_cancelled() {
        if !nectarlink_adb::is_serial_allowed(&serial) {
            return;
        }
        let mut answered = false;
        if let Ok(host_port) = nectarlink_adb::forward_tcp(adb_addr, &serial, 0, phone_port).await {
            let forwarded = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), host_port);
            let target = UsbTarget {
                phone_port,
                auto_pair: auto_pairs(&serial),
                profiles: profiles.clone(),
                serial: serial.clone(),
                adb_addr,
            };
            answered = run_forwarded_session(&shared, forwarded, &cancel, target).await;
            let _ = nectarlink_adb::kill_forward_tcp(adb_addr, &serial, host_port).await;
        }
        // The usual port is kept up; the others usually have nobody listening (one
        // profile), so they're tried less and less often until something answers.
        delay = if answered || phone_port == DEFAULT_USB_PORT {
            ADB_RETRY_DELAY
        } else {
            (delay * 2).min(SPARE_PORT_MAX_DELAY)
        };
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = shared.usb_poke.notified() => delay = ADB_RETRY_DELAY,
            _ = tokio::time::sleep(delay) => {}
        }
    }
}

/// The phone port a tunnel goes to, and what it may do once it knows who answered.
struct UsbTarget {
    phone_port: u16,
    auto_pair: bool,
    profiles: Arc<Mutex<SerialProfiles>>,
    /// The phone's adb serial, so its Elevated helper can be started over USB.
    serial: String,
    adb_addr: SocketAddr,
}

/// Whether plugging in this device should start pairing on its own. Android
/// emulators show up in adb like phones, but a developer running one almost
/// never means to pair it, and the prompt would come back on every restart.
/// They can still be paired by hand.
fn auto_pairs(serial: &str) -> bool {
    !serial.trim().starts_with("emulator-")
}

/// Runs one tunnel until it ends. Returns whether a NectarLink profile answered.
async fn run_forwarded_session(
    shared: &Arc<Shared>,
    forwarded_tcp: SocketAddr,
    cancel: &CancellationToken,
    target: UsbTarget,
) -> bool {
    let Ok(Ok(mut stream)) = tokio::time::timeout(HELLO_TIMEOUT, TcpStream::connect(forwarded_tcp)).await
    else {
        return false;
    };
    let _ = stream.set_nodelay(true);
    let local_name = shared.local.read().unwrap_or_else(|e| e.into_inner()).device.name.clone();
    let Ok((phone_id, phone_name)) = exchange_hello(&mut stream, &shared.id, &local_name).await else {
        return false;
    };
    if phone_id == shared.id {
        return true;
    }
    let Some(iroh_port) = shared.bound_v4_port() else { return true };
    let Ok(udp) = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await else { return true };
    let Ok(bridge_addr) = udp.local_addr() else { return true };
    let bridge_port = bridge_addr.port();

    let paired = shared.store.is_paired(&phone_id).unwrap_or(false);
    let auto_pair = target.auto_pair
        && !target.profiles.lock().unwrap_or_else(|e| e.into_inner()).another_is_paired(target.phone_port);
    let profiles = target.profiles.clone();
    let phone_port = target.phone_port;
    let on_verified: Box<dyn FnOnce() + Send> = Box::new(move || {
        if paired {
            profiles.lock().unwrap_or_else(|e| e.into_inner()).paired.insert(phone_port, phone_id);
        }
    });

    let tunnel_cancel = cancel.child_token();
    shared.register_usb_bridge_port(bridge_port, phone_id);
    shared.on_usb_tunnel_connected(
        phone_id,
        phone_name,
        bridge_addr,
        tunnel_cancel.clone(),
        auto_pair,
        Some((target.serial.clone(), target.adb_addr)),
        Some(on_verified),
    );

    let default_target = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), iroh_port);
    relay_tcp_udp(stream, udp, default_target, tunnel_cancel).await;

    target.profiles.lock().unwrap_or_else(|e| e.into_inner()).paired.remove(&target.phone_port);
    shared.unregister_usb_bridge_port(bridge_port);
    shared.on_usb_tunnel_disconnected(phone_id, bridge_addr).await;
    true
}

/// Bridges UDP datagrams on `udp` to length-prefixed frames (`u16` big-endian)
/// on `stream`. Pre-initializes the local UDP peer target to `default_target`
/// (`127.0.0.1:<iroh_port>`) and updates it whenever a local UDP datagram
/// arrives.
async fn relay_tcp_udp(
    stream: TcpStream,
    udp: UdpSocket,
    default_target: SocketAddr,
    cancel: CancellationToken,
) {
    relay_until_silent(stream, udp, default_target, cancel, TUNNEL_SILENCE_LIMIT).await;
}

/// [`relay_tcp_udp`] that also returns once frames have been sent for
/// `silence_limit` without any coming back.
async fn relay_until_silent(
    stream: TcpStream,
    udp: UdpSocket,
    default_target: SocketAddr,
    cancel: CancellationToken,
    silence_limit: Duration,
) {
    let (mut tcp_read, mut tcp_write) = stream.into_split();
    let udp_recv = Arc::new(udp);
    let udp_send = udp_recv.clone();
    let target = Arc::new(Mutex::new(default_target));
    let target_for_recv = target.clone();
    // Since when frames have gone out with nothing coming back.
    let unanswered: Arc<Mutex<Option<Instant>>> = Arc::default();
    let unanswered_on_send = unanswered.clone();
    let unanswered_on_recv = unanswered.clone();
    // A frame on its way out: it can sit forever on a far end that stopped reading,
    // so an answer that arrives meanwhile restarts the wait instead of ending it.
    let writing = Arc::new(AtomicBool::new(false));
    let writing_on_recv = writing.clone();

    let udp_to_tcp = async move {
        let mut buf = vec![0u8; 65535];
        loop {
            let (n, from) = udp_recv.recv_from(&mut buf).await?;
            if n == 0 || n > u16::MAX as usize {
                continue;
            }
            *target_for_recv.lock().unwrap_or_else(|e| e.into_inner()) = from;
            writing.store(true, Ordering::Relaxed);
            unanswered_on_send.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Instant::now);
            tcp_write.write_all(&(n as u16).to_be_bytes()).await?;
            tcp_write.write_all(&buf[..n]).await?;
            writing.store(false, Ordering::Relaxed);
        }
        #[allow(unreachable_code)]
        Ok::<(), std::io::Error>(())
    };

    let tcp_to_udp = async move {
        let mut buf = vec![0u8; 65535];
        loop {
            let mut len_buf = [0u8; 2];
            tcp_read.read_exact(&mut len_buf).await?;
            *unanswered_on_recv.lock().unwrap_or_else(|e| e.into_inner()) =
                writing_on_recv.load(Ordering::Relaxed).then(Instant::now);
            let len = u16::from_be_bytes(len_buf) as usize;
            if len == 0 {
                continue;
            }
            tcp_read.read_exact(&mut buf[..len]).await?;
            let dest = *target.lock().unwrap_or_else(|e| e.into_inner());
            let _ = udp_send.send_to(&buf[..len], dest).await;
        }
        #[allow(unreachable_code)]
        Ok::<(), std::io::Error>(())
    };

    let silent = async {
        let check = (silence_limit / 4).clamp(Duration::from_millis(10), Duration::from_secs(5));
        loop {
            tokio::time::sleep(check).await;
            let since = *unanswered.lock().unwrap_or_else(|e| e.into_inner());
            if since.is_some_and(|t| t.elapsed() >= silence_limit) {
                break;
            }
        }
    };

    tokio::select! {
        _ = cancel.cancelled() => {}
        _ = udp_to_tcp => {}
        _ = tcp_to_udp => {}
        _ = silent => tracing::info!("USB tunnel stopped answering; closing it"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests that bind real ports run one at a time, or one's probing takes another's port.
    static PORTS: Mutex<()> = Mutex::new(());

    /// A port with the next `USB_PORT_COUNT` ports after it free on loopback right now.
    /// From below the ephemeral ranges, where the other tests' sockets can't take one
    /// between this check and the test's bind.
    fn free_port_range() -> u16 {
        (20_000u16..32_000)
            .step_by(usize::from(USB_PORT_COUNT) + 1)
            .find(|&base| {
                (base..=base + USB_PORT_COUNT)
                    .all(|p| std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, p)).is_ok())
            })
            .expect("a free port range")
    }

    fn port_of(listener: &std::net::TcpListener) -> u16 {
        listener.local_addr().unwrap().port()
    }

    #[test]
    fn a_second_profile_takes_the_next_port() {
        let _ports = PORTS.lock().unwrap_or_else(|e| e.into_inner());
        let base = free_port_range();
        let first = bind_phone_listener(base).expect("the usual port");
        assert_eq!(port_of(&first), base);
        let second = bind_phone_listener(base).expect("the next one");
        assert_eq!(port_of(&second), base + 1);
    }

    #[test]
    fn no_listener_when_every_profile_port_is_taken() {
        let _ports = PORTS.lock().unwrap_or_else(|e| e.into_inner());
        let base = free_port_range();
        let taken: Vec<_> = (0..USB_PORT_COUNT).map(|_| bind_phone_listener(base).expect("free")).collect();
        let mut ports: Vec<u16> = taken.iter().map(port_of).collect();
        ports.sort_unstable();
        assert_eq!(ports, (base..base + USB_PORT_COUNT).collect::<Vec<_>>());
        // A port outside the range would never be reached by the PC: none at all instead.
        assert!(bind_phone_listener(base).is_none());
    }

    #[test]
    fn port_zero_binds_any_free_port() {
        let _ports = PORTS.lock().unwrap_or_else(|e| e.into_inner());
        let listener = bind_phone_listener(0).expect("any port");
        assert_ne!(port_of(&listener), 0);
    }

    #[test]
    fn a_second_profile_is_paired_by_hand_once_one_is_paired() {
        let mut profiles = SerialProfiles::default();
        assert!(!profiles.another_is_paired(DEFAULT_USB_PORT), "nothing paired yet: pairing may start");
        profiles.paired.insert(DEFAULT_USB_PORT, DeviceId([1; 32]));
        assert!(!profiles.another_is_paired(DEFAULT_USB_PORT), "the paired profile itself");
        assert!(profiles.another_is_paired(DEFAULT_USB_PORT + 1), "the work profile next to it");
    }

    #[test]
    fn emulators_are_not_auto_paired() {
        assert!(!auto_pairs("emulator-5554"));
        assert!(!auto_pairs(" emulator-5556 "));
        assert!(auto_pairs("R58M123ABC"));
        assert!(auto_pairs("192.168.1.20:37099"));
        assert!(auto_pairs("adb-R58M123ABC-xyz._adb-tls-connect._tcp"));
    }

    /// A connected loopback TCP pair: (the relay's end, the far end).
    async fn tcp_pair() -> (TcpStream, TcpStream) {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let (near, far) = tokio::join!(TcpStream::connect(listener.local_addr().unwrap()), listener.accept());
        (near.unwrap(), far.unwrap().0)
    }

    #[tokio::test]
    async fn a_tunnel_whose_far_end_stops_answering_is_closed() {
        let (near, _silent_far_end) = tcp_pair().await;
        let udp = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let bridge = udp.local_addr().unwrap();
        let relay = tokio::spawn(relay_until_silent(
            near,
            udp,
            SocketAddr::from((Ipv4Addr::LOCALHOST, 9)),
            CancellationToken::new(),
            Duration::from_millis(300),
        ));

        let sender = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        sender.send_to(b"ping", bridge).await.unwrap();

        tokio::time::timeout(Duration::from_secs(5), relay)
            .await
            .expect("the silent tunnel should have been closed")
            .unwrap();
    }

    #[tokio::test]
    async fn a_tunnel_stuck_writing_after_its_last_answer_is_closed() {
        let (near, mut far) = tcp_pair().await;
        let udp = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let bridge = udp.local_addr().unwrap();
        // Answers need somewhere to go: Windows reports a closed port on the socket itself.
        let sink = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let relay = tokio::spawn(relay_until_silent(
            near,
            udp,
            sink.local_addr().unwrap(),
            CancellationToken::new(),
            Duration::from_millis(600),
        ));

        // The far end stops reading, so the relay's writes fill the socket and block.
        // (Under 9 KiB: macOS refuses larger datagrams by default.)
        let sender = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let frame = vec![0u8; 8 * 1024];
        let flood = tokio::spawn(async move {
            loop {
                let _ = sender.send_to(&frame, bridge).await;
                tokio::task::yield_now().await;
            }
        });
        // It keeps answering while that happens, so the tunnel isn't silent yet. Its
        // last word lands while the relay is stuck in a write.
        for _ in 0..8 {
            far.write_all(&[0, 1, 7]).await.unwrap();
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        let closed = tokio::time::timeout(Duration::from_secs(5), relay).await;
        flood.abort();
        closed.expect("a tunnel stuck writing should have been closed").unwrap();
    }

    #[tokio::test]
    async fn a_tunnel_that_keeps_answering_stays_open() {
        let (near, mut far) = tcp_pair().await;
        // The far end answers every frame with a frame.
        tokio::spawn(async move {
            let mut len = [0u8; 2];
            while far.read_exact(&mut len).await.is_ok() {
                let mut body = vec![0u8; u16::from_be_bytes(len) as usize];
                if far.read_exact(&mut body).await.is_err() || far.write_all(&[0, 1, 7]).await.is_err() {
                    return;
                }
            }
        });
        let udp = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let bridge = udp.local_addr().unwrap();
        let mut relay = tokio::spawn(relay_until_silent(
            near,
            udp,
            SocketAddr::from((Ipv4Addr::LOCALHOST, 9)),
            CancellationToken::new(),
            Duration::from_millis(300),
        ));

        let sender = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let sending = async {
            for _ in 0..15 {
                sender.send_to(b"ping", bridge).await.unwrap();
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        };
        sending.await;
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut relay).await.is_err(),
            "a tunnel that is being answered must not be closed"
        );
        relay.abort();
    }
}
