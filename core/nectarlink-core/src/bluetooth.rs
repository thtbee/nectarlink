// SPDX-License-Identifier: GPL-3.0-or-later
//! The session over Bluetooth, for when a phone and a PC reach each other no
//! other way: no shared network, no mobile data, no cable.
//!
//! The core doesn't speak Bluetooth itself; each platform carries the bytes:
//! - The phone listens for RFCOMM links on [`SERVICE_UUID`] and pipes each one
//!   into the loopback listener [`start_phone_listener`] opens.
//! - The PC opens an RFCOMM link to a phone it's bonded with (paired in the
//!   system's Bluetooth settings), exposes it on a loopback TCP port, and hands
//!   that to [`connect`].
//!
//! From there it's the USB tunnel's design: the same hello and the same
//! UDP-over-stream bridge, so the QUIC session, its encryption and the
//! device's identity are unchanged. Three things differ:
//! - Only devices already paired in NectarLink: nothing pairs over it, and a
//!   Bluetooth bond never stands in for NectarLink's own pairing.
//! - A cable always wins: a link isn't set up beside a USB tunnel, and a cable
//!   that comes along replaces it. Any other path the connection finds (the
//!   local network) is used instead of it as soon as it works.
//! - Bluetooth is slow, so only light messages go over it ([`allows`]):
//!   notifications, texts, calls without their sound, clipboard text, quick
//!   settings and small commands. No data streams and no datagrams.

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{Arc, atomic::Ordering},
};

use nectarlink_protocol::{
    DeviceId,
    messages::{DeviceKind, types},
};
use tokio::net::{TcpListener, TcpStream, UdpSocket};

use crate::{
    Error, Result,
    node::Shared,
    usb::{HELLO_TIMEOUT, exchange_hello, read_hello, relay_tcp_udp, write_hello},
};

/// The RFCOMM service both sides use (an SDP record on the phone).
pub const SERVICE_UUID: &str = "a3f6b0e2-5c1d-4b7e-9e2a-7d4c1f8b6e53";

/// Message families that move bulk data or need a stream: screen and camera
/// video, photos, phone storage, files, sound, the pointer.
const HEAVY_FAMILIES: &[&str] = &[
    "mirror.",
    "photos.",
    "storage.",
    "webcam.",
    "camera.",
    "pointer.",
    "mic.",
    "stream.",
    "files.",
    "audio.",
    "elevated.",
];

/// Whether a message of type `t` may go over Bluetooth. Replies always may.
pub fn allows(t: &str) -> bool {
    !(HEAVY_FAMILIES.iter().any(|family| t.starts_with(family))
        || matches!(
            t,
            types::MEDIA_WATCH
                | types::MEDIA_FILE
                | types::MEDIA_FILE_META
                | types::CALL_AUDIO_START
                | types::CALL_AUDIO_STOP
                // Picture messages: their parts.
                | types::SMS_PART
        ))
}

/// Whether a link from `peer` may carry its session now.
fn may_carry(shared: &Shared, peer: &DeviceId) -> bool {
    *peer != shared.id
        && shared.store.is_paired(peer).unwrap_or(false)
        && shared.reconnects(peer)
        && !shared.has_cable(peer)
}

/// The paired phones a Bluetooth link would help now: offline, reconnecting
/// on, and not on a cable. With their names, to try the likeliest first.
pub(crate) fn wanted(shared: &Shared) -> Vec<(DeviceId, String)> {
    let Ok(peers) = shared.store.list_peers() else { return Vec::new() };
    peers
        .into_iter()
        .filter(|p| matches!(p.info.kind, DeviceKind::Phone | DeviceKind::Tablet))
        .filter(|p| shared.session(&p.id).is_none() && may_carry(shared, &p.id))
        .map(|p| (p.id, p.info.name))
        .collect()
}

/// Phone: opens (once) the loopback listener that each incoming Bluetooth
/// link is piped into, and returns its port.
pub(crate) fn start_phone_listener(shared: &Arc<Shared>) -> Option<u16> {
    let started = shared.bt_listener_port.load(Ordering::Relaxed);
    if started != 0 {
        return Some(started);
    }
    let std_listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).ok()?;
    std_listener.set_nonblocking(true).ok()?;
    let port = std_listener.local_addr().ok()?.port();
    if shared.bt_listener_port.compare_exchange(0, port, Ordering::Relaxed, Ordering::Relaxed).is_err() {
        return Some(shared.bt_listener_port.load(Ordering::Relaxed));
    }
    let listener = TcpListener::from_std(std_listener).ok()?;
    let weak = Arc::downgrade(shared);
    let cancel = shared.cancel.clone();
    tokio::spawn(async move {
        loop {
            let accepted = tokio::select! {
                _ = cancel.cancelled() => return,
                res = listener.accept() => res,
            };
            let Ok((stream, from)) = accepted else {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                continue;
            };
            if !from.ip().to_canonical().is_loopback() {
                continue;
            }
            let Some(node) = weak.upgrade() else { return };
            tokio::spawn(serve_phone_link(node, stream));
        }
    });
    tracing::info!("listening for Bluetooth links");
    Some(port)
}

/// Phone: one link from a PC, until it ends.
async fn serve_phone_link(node: Arc<Shared>, mut stream: TcpStream) {
    let _ = stream.set_nodelay(true);
    let Ok(Ok((pc_id, pc_name, _))) = tokio::time::timeout(HELLO_TIMEOUT, read_hello(&mut stream)).await
    else {
        return;
    };
    if !may_carry(&node, &pc_id) {
        tracing::debug!(peer = %pc_id.short(), "refusing a Bluetooth link");
        return;
    }
    let Some(iroh_port) = node.bound_v4_port() else { return };
    let Ok(udp) = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await else { return };
    let Ok(bridge_addr) = udp.local_addr() else { return };
    let bridge_port = bridge_addr.port();
    let local_name = node.local.read().unwrap_or_else(|e| e.into_inner()).device.name.clone();

    let tunnel_cancel = node.cancel.child_token();
    node.bt_bridge_ports.lock().unwrap_or_else(|e| e.into_inner()).insert(bridge_port);
    node.register_usb_bridge_port(bridge_port, pc_id);
    node.on_usb_tunnel_accepted(pc_id, pc_name, bridge_addr, tunnel_cancel.clone());
    tracing::info!(peer = %pc_id.short(), "Bluetooth link up");

    if matches!(
        tokio::time::timeout(HELLO_TIMEOUT, write_hello(&mut stream, &node.id, &local_name, false)).await,
        Ok(Ok(()))
    ) {
        let default_target = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), iroh_port);
        relay_tcp_udp(stream, udp, default_target, tunnel_cancel).await;
    }
    node.unregister_usb_bridge_port(bridge_port);
    node.on_usb_tunnel_disconnected(pc_id, bridge_addr).await;
    node.bt_bridge_ports.lock().unwrap_or_else(|e| e.into_inner()).remove(&bridge_port);
    tracing::info!(peer = %pc_id.short(), "Bluetooth link down");
}

/// PC: runs the session over a link to a phone exposed at `forwarded_tcp`.
pub(crate) async fn connect(shared: &Arc<Shared>, forwarded_tcp: SocketAddr) -> Result<DeviceId> {
    let mut stream = tokio::time::timeout(HELLO_TIMEOUT, TcpStream::connect(forwarded_tcp))
        .await
        .map_err(|_| Error::Timeout)?
        .map_err(|e| Error::Network(e.to_string()))?;
    let _ = stream.set_nodelay(true);
    let local_name = shared.local.read().unwrap_or_else(|e| e.into_inner()).device.name.clone();
    let (phone_id, phone_name, _) = exchange_hello(&mut stream, &shared.id, &local_name).await?;
    if !may_carry(shared, &phone_id) {
        return Err(Error::NotPaired);
    }
    if shared.session(&phone_id).is_some() {
        return Err(Error::Denied);
    }
    let iroh_port =
        shared.bound_v4_port().ok_or_else(|| Error::Internal("no local IPv4 QUIC port".into()))?;
    let udp = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.map_err(|e| Error::Network(e.to_string()))?;
    let bridge_addr = udp.local_addr().map_err(|e| Error::Network(e.to_string()))?;
    let bridge_port = bridge_addr.port();

    let tunnel_cancel = shared.cancel.child_token();
    shared.bt_bridge_ports.lock().unwrap_or_else(|e| e.into_inner()).insert(bridge_port);
    shared.register_usb_bridge_port(bridge_port, phone_id);
    let node = shared.clone();
    let relay_cancel = tunnel_cancel.clone();
    let default_target = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), iroh_port);
    tokio::spawn(async move {
        relay_tcp_udp(stream, udp, default_target, relay_cancel).await;
        node.unregister_usb_bridge_port(bridge_port);
        node.on_usb_tunnel_disconnected(phone_id, bridge_addr).await;
        node.bt_bridge_ports.lock().unwrap_or_else(|e| e.into_inner()).remove(&bridge_port);
        tracing::info!(peer = %phone_id.short(), "Bluetooth link down");
    });
    shared.on_usb_tunnel_connected(phone_id, phone_name, bridge_addr, tunnel_cancel, false, None, None);
    tracing::info!(peer = %phone_id.short(), "Bluetooth link up");
    Ok(phone_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_messages_pass_and_bulk_ones_dont() {
        for t in [
            types::PING,
            types::NOTIFY_POSTED,
            types::NOTIFY_ACTION,
            types::SMS_SEND,
            types::SMS_MESSAGES,
            types::CALL_STATE,
            types::CALL_ACTION,
            types::CLIP_SET,
            types::PHONE_TOGGLE_SET,
            types::MEDIA_COMMAND,
            types::LINK_OPEN,
            types::DEVICE_RING,
        ] {
            assert!(allows(t), "{t} should pass");
        }
        for t in [
            types::MIRROR_START,
            types::PHOTOS_GET,
            types::STORAGE_LIST,
            types::WEBCAM_START,
            types::MEDIA_WATCH,
            types::MEDIA_FILE_META,
            types::CALL_AUDIO_START,
            types::SMS_PART,
            types::FILES_RESUME,
            types::AUDIO_PCM,
        ] {
            assert!(!allows(t), "{t} shouldn't pass");
        }
    }
}
