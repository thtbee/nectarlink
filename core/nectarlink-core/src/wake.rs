// SPDX-License-Identifier: MPL-2.0
//! Relayed Wake-on-LAN (`wake.relay`) magic packet broadcaster.

use std::{
    io,
    net::{Ipv4Addr, SocketAddrV4, UdpSocket},
};

use crate::Result;

/// Broadcasts a 102-byte Wake-on-LAN magic packet (`6 * 0xFF` followed by 16
/// repetitions of `mac`) on UDP ports 9 and 7 to `broadcast` (or
/// `255.255.255.255` when `None`).
pub fn send_magic_packet(mac: [u8; 6], broadcast: Option<Ipv4Addr>) -> Result<()> {
    let pkt = nectarlink_protocol::messages::magic_packet(&mac);
    let sock = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))?;
    sock.set_broadcast(true)?;
    let target = broadcast.unwrap_or(Ipv4Addr::BROADCAST);
    send_to_targets(target, |addr| sock.send_to(&pkt, addr))?;
    Ok(())
}

/// Try all destinations even when one fails, and report an error if none succeed.
fn send_to_targets(
    target: Ipv4Addr,
    mut send: impl FnMut(SocketAddrV4) -> io::Result<usize>,
) -> io::Result<()> {
    let mut result = Err(io::Error::other("no wake packets sent"));
    for port in nectarlink_protocol::messages::wake::DEFAULT_PORTS {
        result = result.or(send(SocketAddrV4::new(target, *port)).map(|_| ()));
        if target != Ipv4Addr::BROADCAST {
            result = result.or(send(SocketAddrV4::new(Ipv4Addr::BROADCAST, *port)).map(|_| ()));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_send_failures_are_reported() {
        let mut attempts = 0;
        let result = send_to_targets(Ipv4Addr::LOCALHOST, |_| {
            attempts += 1;
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        });
        assert_eq!(attempts, 4);
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn one_success_is_enough_and_all_targets_are_tried() {
        let mut attempts = 0;
        let result = send_to_targets(Ipv4Addr::LOCALHOST, |_| {
            attempts += 1;
            if attempts == 2 { Ok(102) } else { Err(io::Error::from(io::ErrorKind::PermissionDenied)) }
        });
        assert!(result.is_ok());
        assert_eq!(attempts, 4);
    }

    #[test]
    fn global_broadcast_is_not_sent_twice() {
        let mut attempts = 0;
        send_to_targets(Ipv4Addr::BROADCAST, |addr| {
            assert_eq!(*addr.ip(), Ipv4Addr::BROADCAST);
            attempts += 1;
            Ok(102)
        })
        .unwrap();
        assert_eq!(attempts, 2);
    }

    #[test]
    fn send_magic_packet_succeeds_on_loopback() {
        let mac = [0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc];
        send_magic_packet(mac, Some(Ipv4Addr::LOCALHOST)).expect("loopback magic packet succeeds");
    }
}
