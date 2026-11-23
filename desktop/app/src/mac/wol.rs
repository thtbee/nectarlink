// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS Wake-on-LAN integration stubs.

use nectarlink_core::PcWakeInfo;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeSupport {
    Enabled,
    Disabled,
    Unknown,
}

impl WakeSupport {
    pub fn as_str(self) -> &'static str {
        match self {
            WakeSupport::Enabled => "enabled",
            WakeSupport::Disabled => "disabled",
            WakeSupport::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WakeAdapter {
    pub label: String,
    pub wired: bool,
    pub mac: String,
    pub broadcasts: Vec<String>,
    pub support: WakeSupport,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WakeStatus {
    pub info: PcWakeInfo,
    pub adapters: Vec<WakeAdapter>,
}

impl WakeStatus {
    pub fn primary(&self) -> Option<&WakeAdapter> {
        self.adapters.first()
    }

    pub fn state_str(&self) -> &'static str {
        match self.primary() {
            Some(a) => a.support.as_str(),
            None => "none",
        }
    }
}

pub fn query_wake_status() -> WakeStatus {
    WakeStatus::default()
}

/// All non-loopback IPv4 and IPv6 unicast addresses currently assigned to UP
/// interfaces on this Mac (including Tailscale / utun tunnels).
#[allow(unsafe_code)]
pub fn interface_ips() -> Vec<std::net::IpAddr> {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    let mut out = Vec::new();
    let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: `getifaddrs` allocates a linked list freed by `freeifaddrs`.
    if unsafe { libc::getifaddrs(&mut ifap) } == 0 && !ifap.is_null() {
        let mut cur = ifap;
        // SAFETY: iterating valid `ifaddrs` nodes before `freeifaddrs`.
        unsafe {
            while let Some(ifa) = cur.as_ref() {
                cur = ifa.ifa_next;
                let flags = ifa.ifa_flags as i32;
                if (flags & libc::IFF_UP) == 0 || (flags & libc::IFF_LOOPBACK) != 0 || ifa.ifa_addr.is_null()
                {
                    continue;
                }
                let ip = match i32::from((*ifa.ifa_addr).sa_family) {
                    libc::AF_INET => {
                        let sin = *ifa.ifa_addr.cast::<libc::sockaddr_in>();
                        IpAddr::V4(Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr)))
                    }
                    libc::AF_INET6 => {
                        let sin6 = *ifa.ifa_addr.cast::<libc::sockaddr_in6>();
                        IpAddr::V6(Ipv6Addr::from(sin6.sin6_addr.s6_addr))
                    }
                    _ => continue,
                };
                if !ip.is_loopback() && !ip.is_unspecified() && !out.contains(&ip) {
                    out.push(ip);
                }
            }
            libc::freeifaddrs(ifap);
        }
    }
    out
}

#[allow(dead_code)]
pub fn wake(_mac: &[u8]) -> Result<(), String> {
    Err("wake-on-lan is not supported on macOS".into())
}
