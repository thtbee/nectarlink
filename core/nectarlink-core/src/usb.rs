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
//!    the ceremony without requiring manual host mode only on a tunnel adb
//!    opened, and both users must confirm the 6-digit code before any trust
//!    is stored.
//!
//! Any app on the phone can reach the tunnel's loopback port, so the phone can't
//! tell adb from another app by the connection alone. Before it opens a tunnel to
//! a phone it isn't paired with, the PC sends a one-time nonce through adb (a
//! broadcast to a receiver only the shell may reach, see [`Wake`]) and puts the
//! same nonce in its hello. The phone takes pairing without pairing mode only on
//! a tunnel whose hello carried a nonce it was given that way ([`UsbNonces`]).

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
pub(crate) const HELLO_TIMEOUT: Duration = Duration::from_secs(3);
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
/// How much a tunnel gathers per write, and reads ahead, between UDP and TCP.
const RELAY_BATCH_BYTES: usize = 256 * 1024;
/// Room for one framed datagram: its 2-byte length and up to 65,535 bytes.
const FRAME_ROOM: usize = 2 + u16::MAX as usize;
const _: () = assert!(RELAY_BATCH_BYTES >= FRAME_ROOM);

/// The phone app's id, and the pieces of it the PC starts through adb. Both
/// are protected by `android.permission.DUMP`, which only the shell holds.
const PHONE_PACKAGE: &str = "app.nectarlink";
const WAKE_ACTION: &str = "app.nectarlink.action.USB_WAKE";
const WAKE_RECEIVER: &str = "app.nectarlink.android.service.UsbWakeReceiver";
const WAKE_SERVICE: &str = "app.nectarlink.android.service.ConnectionService";
/// How often, at most, a phone nobody answers on is woken.
const WAKE_EVERY: Duration = Duration::from_secs(15);
/// How long to leave a phone alone after waking it failed (the app isn't installed, say),
/// or after [`WAKES_BEFORE_SLOWING`] wakes that nothing answered.
const WAKE_AFTER_ERROR: Duration = Duration::from_secs(300);
/// Wakes that go through without the app answering before they slow down.
const WAKES_BEFORE_SLOWING: u32 = 4;

/// What the PC asks of a phone's app through adb.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wake {
    /// Start the app's process and its core, so the tunnel's port answers. Runs even
    /// when Android won't let the app start a foreground service.
    Process,
    /// Start the connection service in the foreground, which shows the app is connected.
    Service,
}

impl Wake {
    fn command(self) -> String {
        match self {
            // `-f 32`: also reaches an app that was force-stopped.
            Wake::Process => {
                format!("am broadcast -n {PHONE_PACKAGE}/{WAKE_RECEIVER} -a {WAKE_ACTION} -f 32")
            }
            Wake::Service => {
                format!("am start-foreground-service -n {PHONE_PACKAGE}/{WAKE_SERVICE} -a {WAKE_ACTION}")
            }
        }
    }
}

/// The wake broadcast, carrying a nonce the PC's next hello repeats. Only the shell
/// may send it, so a hello with that nonce came from whoever holds adb. Older apps
/// ignore the extra.
fn nonce_command(nonce: &str) -> String {
    format!("{} --es {NONCE_EXTRA} {nonce}", Wake::Process.command())
}

/// The wake broadcast's extra holding the nonce.
const NONCE_EXTRA: &str = "nonce";
/// Random bytes in a nonce; it travels as twice as many lowercase hex digits.
const NONCE_BYTES: usize = 16;
/// How long a phone keeps a nonce it was given: a PC sends it right before the hello.
const NONCE_TTL: Duration = Duration::from_secs(120);
/// Nonces a phone keeps at most; the oldest goes first.
const MAX_NONCES: usize = 4;

/// A fresh nonce for one tunnel, as lowercase hex.
fn new_nonce() -> String {
    let bytes: [u8; NONCE_BYTES] = rand::random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn parse_nonce(nonce: &str) -> Option<[u8; NONCE_BYTES]> {
    let digits = nonce.as_bytes();
    if digits.len() != NONCE_BYTES * 2 || !digits.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    let mut out = [0u8; NONCE_BYTES];
    let nibble = |digit: u8| char::from(digit).to_digit(16).map(|v| v as u8);
    for (byte, [high, low]) in out.iter_mut().zip(digits.as_chunks::<2>().0) {
        *byte = (nibble(*high)? << 4) | nibble(*low)?;
    }
    Some(out)
}

/// The nonces this phone was given through adb, each good for one tunnel within
/// [`NONCE_TTL`].
#[derive(Default)]
pub(crate) struct UsbNonces {
    given: Mutex<Vec<([u8; NONCE_BYTES], Instant)>>,
}

impl UsbNonces {
    /// Keeps `nonce`, if it is one. Returns whether it was.
    pub(crate) fn allow(&self, nonce: &str, now: Instant) -> bool {
        let Some(nonce) = parse_nonce(nonce) else { return false };
        let mut given = self.given.lock().unwrap_or_else(|e| e.into_inner());
        given.retain(|(_, at)| now.saturating_duration_since(*at) < NONCE_TTL);
        if given.len() >= MAX_NONCES {
            given.remove(0);
        }
        given.push((nonce, now));
        true
    }

    /// Whether `nonce` is one this phone was given and hasn't used yet. A match is
    /// used up.
    pub(crate) fn take(&self, nonce: &str, now: Instant) -> bool {
        let Some(nonce) = parse_nonce(nonce) else { return false };
        let mut given = self.given.lock().unwrap_or_else(|e| e.into_inner());
        given.retain(|(_, at)| now.saturating_duration_since(*at) < NONCE_TTL);
        // Compared without stopping early, so timing says nothing about how much matched.
        let mut found = None;
        for (i, (kept, _)) in given.iter().enumerate() {
            let diff = kept.iter().zip(&nonce).fold(0u8, |acc, (a, b)| acc | (a ^ b));
            if diff == 0 {
                found = Some(i);
            }
        }
        found.map(|i| given.remove(i)).is_some()
    }
}

/// Gives the phone `nonce` through adb. Returns whether `am` says it went through.
async fn send_nonce(adb_addr: SocketAddr, serial: &str, nonce: &str) -> bool {
    match nectarlink_adb::shell_output(adb_addr, serial, &nonce_command(nonce), 1024).await {
        Ok(output) => {
            let ok = wake_went_through(&output);
            tracing::debug!(%serial, ok, "gave a USB phone a tunnel nonce");
            ok
        }
        Err(e) => {
            tracing::debug!(%serial, error = %e, "couldn't give a USB phone a tunnel nonce");
            false
        }
    }
}

/// Whether what `am` printed says it went through.
fn wake_went_through(output: &str) -> bool {
    !output.contains("Error") && !output.contains("Exception")
}

/// Wakes a phone no more often than [`WAKE_EVERY`], and rarely after a failure.
#[derive(Default)]
struct WakeGate {
    next: Option<Instant>,
    /// Wakes in a row that went through but nothing answered after. `am
    /// broadcast` reports success even with no such app on the phone, or one
    /// that ignores the wake (USB only off), so those slow down too.
    unanswered: u32,
}

impl WakeGate {
    fn due(&self, now: Instant) -> bool {
        self.next.is_none_or(|next| now >= next)
    }

    fn note(&mut self, now: Instant, went_through: bool) {
        if went_through {
            self.unanswered += 1;
        }
        let quick = went_through && self.unanswered < WAKES_BEFORE_SLOWING;
        self.next = Some(now + if quick { WAKE_EVERY } else { WAKE_AFTER_ERROR });
    }

    /// The phone answered: the next time nobody does, wake it at once.
    fn answered(&mut self) {
        *self = WakeGate::default();
    }
}

async fn wake_phone(adb_addr: SocketAddr, serial: &str, wake: Wake) -> bool {
    match nectarlink_adb::shell_output(adb_addr, serial, &wake.command(), 1024).await {
        Ok(output) => {
            let ok = wake_went_through(&output);
            tracing::debug!(%serial, ?wake, ok, output = output.trim(), "woke a USB phone's app");
            ok
        }
        Err(e) => {
            tracing::debug!(%serial, ?wake, error = %e, "couldn't wake a USB phone's app");
            false
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UsbHello {
    v: u8,
    id: String,
    name: String,
    /// The phone only connects through the cable: the PC starts its service
    /// while the cable is connected, and wakes it when it isn't running.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    usb_only: bool,
    /// From a PC: the nonce it just gave the phone through adb (see [`UsbNonces`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    nonce: Option<String>,
}

/// What the other end's hello said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PeerHello {
    pub id: DeviceId,
    pub name: String,
    pub usb_only: bool,
    pub nonce: Option<String>,
}

pub(crate) async fn write_hello(
    stream: &mut TcpStream,
    id: &DeviceId,
    name: &str,
    usb_only: bool,
    nonce: Option<&str>,
) -> Result<()> {
    let hello = UsbHello {
        v: 1,
        id: id.to_string(),
        name: name.to_owned(),
        usb_only,
        nonce: nonce.map(str::to_owned),
    };
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

pub(crate) async fn read_hello(stream: &mut TcpStream) -> Result<PeerHello> {
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
    parse_hello(&payload)
}

fn parse_hello(payload: &[u8]) -> Result<PeerHello> {
    let hello: UsbHello =
        serde_json::from_slice(payload).map_err(|e| Error::Protocol(format!("invalid USB hello: {e}")))?;
    let id: DeviceId =
        hello.id.parse().map_err(|_| Error::Protocol("invalid DeviceId in USB hello".into()))?;
    let name: String = hello.name.trim().chars().take(64).collect();
    Ok(PeerHello { id, name, usb_only: hello.usb_only, nonce: hello.nonce })
}

/// PC: sends this end's hello (with `nonce`, if the phone was just given one) and
/// reads the phone's.
pub(crate) async fn exchange_hello(
    stream: &mut TcpStream,
    local_id: &DeviceId,
    local_name: &str,
    nonce: Option<&str>,
) -> Result<PeerHello> {
    tokio::time::timeout(HELLO_TIMEOUT, async {
        write_hello(stream, local_id, local_name, false, nonce).await?;
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
                let Ok(Ok(hello)) = tokio::time::timeout(HELLO_TIMEOUT, read_hello(&mut stream)).await else {
                    return;
                };
                let (pc_id, pc_name) = (hello.id, hello.name);
                if pc_id == node.id {
                    return;
                }
                // Opened by adb, which gave this phone the nonce first; anything else on the
                // phone could have opened it otherwise.
                let from_adb =
                    hello.nonce.as_deref().is_some_and(|nonce| node.usb_nonces.take(nonce, Instant::now()));
                let Some(iroh_port) = node.bound_v4_port() else { return };
                let Ok(udp) = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await else { return };
                let Ok(bridge_addr) = udp.local_addr() else { return };
                let bridge_port = bridge_addr.port();

                let tunnel_cancel = node.cancel.child_token();
                node.register_usb_bridge_port(bridge_port, pc_id);
                node.on_usb_tunnel_accepted(pc_id, pc_name, bridge_addr, tunnel_cancel.clone(), from_adb);

                if !matches!(
                    tokio::time::timeout(
                        HELLO_TIMEOUT,
                        write_hello(&mut stream, &node.id, &local_name, node.usb_only(), None)
                    )
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
///
/// `nonce` is one the phone was just given through adb, so it may pair over this tunnel.
pub(crate) async fn connect_usb_tunnel(
    shared: &Arc<Shared>,
    forwarded_tcp: SocketAddr,
    nonce: Option<&str>,
) -> Result<DeviceId> {
    let mut stream = tokio::time::timeout(HELLO_TIMEOUT, TcpStream::connect(forwarded_tcp))
        .await
        .map_err(|_| Error::Timeout)?
        .map_err(|e| Error::Network(e.to_string()))?;
    let _ = stream.set_nodelay(true);

    let local_name = shared.local.read().unwrap_or_else(|e| e.into_inner()).device.name.clone();
    let PeerHello { id: phone_id, name: phone_name, .. } =
        exchange_hello(&mut stream, &shared.id, &local_name, nonce).await?;
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
            tracing::debug!("watching adb for USB phones");
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
            tracing::debug!("lost the adb watch; starting it again");
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
            tracing::info!(%serial, "a USB phone is gone from adb");
            token.cancel();
        }
        still_online
    });

    for dev in devices {
        if !dev.is_online() || !nectarlink_adb::is_serial_allowed(&dev.serial) {
            if !active.contains_key(&dev.serial) {
                tracing::debug!(serial = %dev.serial, online = dev.is_online(), "adb lists a device that isn't used");
            }
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
        tracing::info!(serial = %dev.serial, "a USB phone showed up in adb");
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
    let mut wake_gate = WakeGate::default();
    // The nonce the phone was just given, for the next tunnel's hello.
    let mut nonce: Option<String> = None;
    while !cancel.is_cancelled() {
        if !nectarlink_adb::is_serial_allowed(&serial) {
            return;
        }
        let mut answered = false;
        let forward = nectarlink_adb::forward_tcp(adb_addr, &serial, 0, phone_port).await;
        if let Err(e) = &forward {
            tracing::debug!(%serial, phone_port, error = %e, "adb wouldn't forward the tunnel port");
        }
        if let Ok(host_port) = forward {
            let forwarded = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), host_port);
            let target = UsbTarget {
                phone_port,
                auto_pair: auto_pairs(&serial),
                profiles: profiles.clone(),
                serial: serial.clone(),
                adb_addr,
            };
            let started = Instant::now();
            let outcome = run_forwarded_session(&shared, forwarded, &cancel, target, nonce.take()).await;
            if outcome == Tunnel::Ended {
                tracing::info!(%serial, phone_port, secs = started.elapsed().as_secs(), "a USB tunnel ended");
            }
            let _ = nectarlink_adb::kill_forward_tcp(adb_addr, &serial, host_port).await;
            answered = outcome != Tunnel::NoAnswer;
            if outcome == Tunnel::NeedsNonce {
                // A phone this PC isn't paired with: it takes pairing only on a tunnel adb
                // opened. The next hello carries the nonce whether or not `am` took it, so
                // a phone that didn't get it gets a tunnel all the same, without pairing.
                let fresh = new_nonce();
                send_nonce(adb_addr, &serial, &fresh).await;
                nonce = Some(fresh);
                wake_gate.answered();
                continue;
            }
        }
        // Nobody on the usual port: the app may not be running (a phone that only
        // connects through the cable stops while it's idle), or too frozen to answer.
        // Also when this app started with the cable already in.
        if answered {
            wake_gate.answered();
        } else if phone_port == DEFAULT_USB_PORT && wake_gate.due(Instant::now()) {
            let went_through = wake_phone(adb_addr, &serial, Wake::Process).await;
            wake_gate.note(Instant::now(), went_through);
            if went_through {
                // Soon enough for the app to have started.
                delay = ADB_RETRY_DELAY;
            }
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

/// How a tunnel to a phone port went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tunnel {
    /// No NectarLink profile answered.
    NoAnswer,
    /// A profile answered, and the tunnel ran until it ended.
    Ended,
    /// A profile this PC isn't paired with answered a hello without a nonce: open
    /// the tunnel again with one, so the phone can tell adb opened it.
    NeedsNonce,
}

/// Runs one tunnel until it ends. `nonce` is one the phone was just given through adb.
async fn run_forwarded_session(
    shared: &Arc<Shared>,
    forwarded_tcp: SocketAddr,
    cancel: &CancellationToken,
    target: UsbTarget,
    nonce: Option<String>,
) -> Tunnel {
    let stream = tokio::time::timeout(HELLO_TIMEOUT, TcpStream::connect(forwarded_tcp)).await;
    let Ok(Ok(mut stream)) = stream else {
        tracing::debug!(port = target.phone_port, "nothing answered on the forwarded tunnel port");
        return Tunnel::NoAnswer;
    };
    let _ = stream.set_nodelay(true);
    let local_name = shared.local.read().unwrap_or_else(|e| e.into_inner()).device.name.clone();
    let hello = exchange_hello(&mut stream, &shared.id, &local_name, nonce.as_deref()).await;
    let PeerHello { id: phone_id, name: phone_name, usb_only: phone_usb_only, .. } = match hello {
        Ok(hello) => hello,
        Err(e) => {
            // The port is open (adb listens on it) but the app behind it didn't answer.
            tracing::debug!(port = target.phone_port, error = %e, "the phone didn't answer the tunnel's hello");
            return Tunnel::NoAnswer;
        }
    };
    tracing::info!(peer = %phone_id.short(), port = target.phone_port, "a USB tunnel is up");
    if phone_id == shared.id {
        return Tunnel::Ended;
    }
    let paired = shared.store.is_paired(&phone_id).unwrap_or(false);
    // A paired phone's sessions are checked by their keys, so its tunnel needs no nonce,
    // and the phone isn't woken through adb each time the cable goes in.
    if !paired && nonce.is_none() {
        return Tunnel::NeedsNonce;
    }
    let Some(iroh_port) = shared.bound_v4_port() else { return Tunnel::Ended };
    let Ok(udp) = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await else { return Tunnel::Ended };
    let Ok(bridge_addr) = udp.local_addr() else { return Tunnel::Ended };
    let bridge_port = bridge_addr.port();

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
    if phone_usb_only {
        // Connected: it should run in the foreground, with its notification. Android
        // may refuse the app that, from the background; the shell may ask.
        let (adb_addr, serial) = (target.adb_addr, target.serial.clone());
        tokio::spawn(async move {
            wake_phone(adb_addr, &serial, Wake::Service).await;
        });
    }

    let default_target = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), iroh_port);
    relay_tcp_udp(stream, udp, default_target, tunnel_cancel).await;

    target.profiles.lock().unwrap_or_else(|e| e.into_inner()).paired.remove(&target.phone_port);
    shared.unregister_usb_bridge_port(bridge_port);
    shared.on_usb_tunnel_disconnected(phone_id, bridge_addr).await;
    Tunnel::Ended
}

/// Bridges UDP datagrams on `udp` to length-prefixed frames (`u16` big-endian)
/// on `stream`. Pre-initializes the local UDP peer target to `default_target`
/// (`127.0.0.1:<iroh_port>`) and updates it whenever a local UDP datagram
/// arrives.
pub(crate) async fn relay_tcp_udp(
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
    let (tcp_read, mut tcp_write) = stream.into_split();
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
        // Datagrams already waiting go out with the first, framed one after
        // another in a single write: one write per batch, where a length and a
        // datagram written apart made two (and two TCP segments, with Nagle off).
        let mut batch = vec![0u8; RELAY_BATCH_BYTES];
        loop {
            let (n, mut from) = udp_recv.recv_from(&mut batch[2..FRAME_ROOM]).await?;
            let mut len = 0;
            if n > 0 {
                batch[..2].copy_from_slice(&(n as u16).to_be_bytes());
                len = 2 + n;
            }
            let mut failed = None;
            while batch.len() - len >= FRAME_ROOM {
                match udp_recv.try_recv_from(&mut batch[len + 2..len + FRAME_ROOM]) {
                    Ok((0, _)) => {}
                    Ok((n, at)) => {
                        batch[len..len + 2].copy_from_slice(&(n as u16).to_be_bytes());
                        len += 2 + n;
                        from = at;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    // Reported after what's gathered goes out, as it would have been.
                    Err(e) => {
                        failed = Some(e);
                        break;
                    }
                }
            }
            if len > 0 {
                *target_for_recv.lock().unwrap_or_else(|e| e.into_inner()) = from;
                writing.store(true, Ordering::Relaxed);
                unanswered_on_send.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Instant::now);
                tcp_write.write_all(&batch[..len]).await?;
                writing.store(false, Ordering::Relaxed);
            }
            if let Some(e) = failed {
                return Err(e);
            }
        }
        #[allow(unreachable_code)]
        Ok::<(), std::io::Error>(())
    };

    let tcp_to_udp = async move {
        // Frames come out of a buffer: two reads per frame were two system calls each.
        let mut tcp_read = tokio::io::BufReader::with_capacity(RELAY_BATCH_BYTES, tcp_read);
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

    #[test]
    fn a_phone_is_woken_once_in_a_while_and_rarely_after_it_failed() {
        let t0 = Instant::now();
        let mut gate = WakeGate::default();
        assert!(gate.due(t0), "the first time is at once: the cable may have been in when this started");
        gate.note(t0, true);
        assert!(!gate.due(t0 + WAKE_EVERY - Duration::from_secs(1)));
        assert!(gate.due(t0 + WAKE_EVERY));
        gate.note(t0 + WAKE_EVERY, false);
        assert!(!gate.due(t0 + WAKE_EVERY + WAKE_EVERY * 2), "after an error, not for a good while");
        assert!(gate.due(t0 + WAKE_EVERY + WAKE_AFTER_ERROR));
    }

    #[test]
    fn wakes_nothing_answers_slow_down_until_it_answers() {
        // No app, or one that ignores the wake: `am` still says it went through.
        let t0 = Instant::now();
        let mut gate = WakeGate::default();
        let mut now = t0;
        for _ in 1..WAKES_BEFORE_SLOWING {
            gate.note(now, true);
            now += WAKE_EVERY;
            assert!(gate.due(now), "quick at first");
        }
        gate.note(now, true);
        assert!(!gate.due(now + WAKE_EVERY), "then only once in a while");
        assert!(gate.due(now + WAKE_AFTER_ERROR));
        gate.answered();
        assert!(gate.due(now), "an app that answered is woken at once next time");
    }

    #[test]
    fn waking_names_the_apps_own_protected_components_and_reads_am_errors() {
        let process = Wake::Process.command();
        assert!(
            process
                .starts_with("am broadcast -n app.nectarlink/app.nectarlink.android.service.UsbWakeReceiver")
        );
        assert!(Wake::Service.command().starts_with("am start-foreground-service -n app.nectarlink/"));
        // Nothing but letters, digits and the punctuation a shell reads as one word.
        for command in [process, Wake::Service.command()] {
            assert!(command.chars().all(|c| c.is_ascii_alphanumeric() || " ._/=-:".contains(c)), "{command}");
        }
        assert!(wake_went_through("Broadcast completed: result=0\n"));
        assert!(wake_went_through("Starting service: Intent { act=app.nectarlink.action.USB_WAKE }"));
        assert!(!wake_went_through("Error: Not found; no service started."));
        assert!(!wake_went_through("java.lang.SecurityException: Permission Denial"));
    }

    fn hello(usb_only: bool, nonce: Option<&str>) -> UsbHello {
        UsbHello { v: 1, id: "x".into(), name: "n".into(), usb_only, nonce: nonce.map(str::to_owned) }
    }

    #[test]
    fn a_hello_without_the_usb_only_flag_reads_as_off() {
        let old: UsbHello = serde_json::from_str(r#"{"v":1,"id":"x","name":"Pixel"}"#).unwrap();
        assert!(!old.usb_only);
        let off = serde_json::to_string(&hello(false, None)).unwrap();
        assert!(!off.contains("usb_only"), "older apps get what they always got: {off}");
        let on = serde_json::to_string(&hello(true, None)).unwrap();
        assert!(on.contains(r#""usb_only":true"#));
    }

    #[test]
    fn a_hello_reads_with_and_without_a_nonce() {
        let id = DeviceId([7; 32]).to_string();
        let old = parse_hello(format!(r#"{{"v":1,"id":"{id}","name":" Desktop "}}"#).as_bytes()).unwrap();
        assert_eq!(
            old,
            PeerHello { id: DeviceId([7; 32]), name: "Desktop".into(), usb_only: false, nonce: None }
        );
        let nonce = "00112233445566778899aabbccddeeff";
        let new = parse_hello(
            format!(r#"{{"v":1,"id":"{id}","name":"Desktop","nonce":"{nonce}","later":[1,2]}}"#).as_bytes(),
        )
        .unwrap();
        assert_eq!(new.nonce.as_deref(), Some(nonce), "and a field from a later version is ignored");

        let without = serde_json::to_string(&hello(false, None)).unwrap();
        assert!(!without.contains("nonce"), "a hello without one is what it always was: {without}");
        let with = serde_json::to_string(&hello(false, Some(nonce))).unwrap();
        assert!(with.contains(&format!(r#""nonce":"{nonce}""#)), "{with}");
    }

    #[test]
    fn a_nonce_is_good_once() {
        let nonces = UsbNonces::default();
        let t0 = Instant::now();
        let nonce = new_nonce();
        assert!(nonces.allow(&nonce, t0));
        assert!(!nonces.take("ffeeddccbbaa99887766554433221100", t0), "one it wasn't given");
        assert!(nonces.take(&nonce.to_uppercase(), t0 + Duration::from_secs(1)), "hex in either case");
        assert!(!nonces.take(&nonce, t0 + Duration::from_secs(1)), "used up");
    }

    #[test]
    fn a_nonce_expires() {
        let nonces = UsbNonces::default();
        let t0 = Instant::now();
        let (early, late) = (new_nonce(), new_nonce());
        assert!(nonces.allow(&early, t0));
        assert!(nonces.allow(&late, t0 + Duration::from_secs(60)));
        assert!(!nonces.take(&early, t0 + NONCE_TTL), "too old");
        assert!(nonces.take(&late, t0 + NONCE_TTL), "still fresh");
    }

    #[test]
    fn only_a_few_nonces_are_kept() {
        let nonces = UsbNonces::default();
        let t0 = Instant::now();
        let given: Vec<String> = (0..=MAX_NONCES).map(|_| new_nonce()).collect();
        for nonce in &given {
            assert!(nonces.allow(nonce, t0));
        }
        assert!(!nonces.take(&given[0], t0), "the oldest made room");
        for nonce in &given[1..] {
            assert!(nonces.take(nonce, t0));
        }
    }

    #[test]
    fn only_a_nonce_shaped_string_is_one() {
        let nonces = UsbNonces::default();
        let t0 = Instant::now();
        for bad in
            ["", "0011", "+0112233445566778899aabbccddeeff", "00112233445566778899aabbccddeefg", "\u{e9}"]
        {
            assert!(!nonces.allow(bad, t0), "{bad:?}");
            assert!(!nonces.take(bad, t0), "{bad:?}");
        }
        let nonce = new_nonce();
        assert_eq!(nonce.len(), NONCE_BYTES * 2);
        assert!(nonce.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)), "{nonce}");
        assert_ne!(nonce, new_nonce());
    }

    #[test]
    fn the_nonce_rides_on_the_apps_protected_wake_broadcast() {
        let nonce = new_nonce();
        let command = nonce_command(&nonce);
        assert!(command.starts_with(&Wake::Process.command()), "{command}");
        assert!(command.ends_with(&format!(" --es nonce {nonce}")), "{command}");
        assert!(command.chars().all(|c| c.is_ascii_alphanumeric() || " ._/=-:".contains(c)), "{command}");
    }

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

    /// How fast a tunnel carries datagrams each way, and in how many TCP reads
    /// they reach the far end. Opt-in:
    /// `cargo test --release -p nectarlink-core --lib usb::tests::relay_throughput -- --ignored --nocapture`.
    #[ignore = "benchmark"]
    #[tokio::test(flavor = "multi_thread")]
    async fn relay_throughput() {
        use std::sync::atomic::AtomicUsize;
        const DATAGRAMS: usize = 200_000;
        const SIZE: usize = 1400;
        // In flight at most: under the 64 KiB a UDP socket buffers by default on
        // Windows, so none is dropped (QUIC's congestion window does this for real).
        const WINDOW: usize = 32;
        let (near, mut far) = tcp_pair().await;
        let udp = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let bridge = udp.local_addr().unwrap();
        let relay = tokio::spawn(relay_until_silent(
            near,
            udp,
            SocketAddr::from((Ipv4Addr::LOCALHOST, 9)),
            CancellationToken::new(),
            Duration::from_secs(3600),
        ));

        // UDP into the tunnel.
        let peer = Arc::new(UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap());
        let arrived = Arc::new(AtomicUsize::new(0));
        let start = Instant::now();
        let send = {
            let (peer, arrived) = (peer.clone(), arrived.clone());
            tokio::spawn(async move {
                let datagram = vec![7u8; SIZE];
                for sent in 0..DATAGRAMS {
                    while sent - arrived.load(Ordering::Relaxed) >= WINDOW {
                        tokio::task::yield_now().await;
                    }
                    peer.send_to(&datagram, bridge).await.unwrap();
                }
            })
        };
        let (mut reads, mut bytes) = (0usize, 0usize);
        let mut buf = vec![0u8; 1 << 20];
        while bytes < DATAGRAMS * (SIZE + 2) {
            let n = far.read(&mut buf).await.unwrap();
            assert!(n > 0);
            reads += 1;
            bytes += n;
            arrived.store(bytes / (SIZE + 2), Ordering::Relaxed);
        }
        let up = start.elapsed();
        send.await.unwrap();
        println!(
            "[BENCH] UDP to TCP: {DATAGRAMS} datagrams in {up:?} = {:.0} MiB/s, {:.1} datagrams per TCP read",
            (DATAGRAMS * SIZE) as f64 / up.as_secs_f64() / (1 << 20) as f64,
            DATAGRAMS as f64 / reads as f64,
        );

        // TCP into the tunnel, to the address the datagrams came from.
        let got = Arc::new(AtomicUsize::new(0));
        let start = Instant::now();
        let write = {
            let got = got.clone();
            tokio::spawn(async move {
                let mut frame = Vec::with_capacity(SIZE + 2);
                frame.extend_from_slice(&(SIZE as u16).to_be_bytes());
                frame.extend_from_slice(&[9u8; SIZE]);
                let mut chunk = Vec::new();
                let mut written = 0;
                while written < DATAGRAMS {
                    while written - got.load(Ordering::Relaxed) >= WINDOW / 2 {
                        tokio::task::yield_now().await;
                    }
                    chunk.clear();
                    let n = (WINDOW / 2).min(DATAGRAMS - written);
                    for _ in 0..n {
                        chunk.extend_from_slice(&frame);
                    }
                    far.write_all(&chunk).await.unwrap();
                    written += n;
                }
                far
            })
        };
        let mut buf = vec![0u8; 65536];
        let _ = tokio::time::timeout(Duration::from_secs(30), async {
            while got.load(Ordering::Relaxed) < DATAGRAMS {
                peer.recv_from(&mut buf).await.unwrap();
                got.fetch_add(1, Ordering::Relaxed);
            }
        })
        .await;
        let got = got.load(Ordering::Relaxed);
        let down = start.elapsed();
        println!(
            "[BENCH] TCP to UDP: {got} of {DATAGRAMS} datagrams in {down:?} = {:.0} MiB/s",
            (got * SIZE) as f64 / down.as_secs_f64() / (1 << 20) as f64
        );
        write.abort();
        relay.abort();
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
