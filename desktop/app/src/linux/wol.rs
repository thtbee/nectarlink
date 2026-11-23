// SPDX-License-Identifier: GPL-3.0-or-later
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;
use std::process::Command;

use nectarlink_core::PcWakeInfo;

const ARPHRD_ETHER: &str = "1";
const SIOCETHTOOL: libc::c_ulong = 0x8946;
const ETHTOOL_GWOL: u32 = 0x0000_0005;
const WAKE_MAGIC: u32 = 1 << 5;

#[repr(C)]
struct EthtoolWolInfo {
    cmd: u32,
    supported: u32,
    wolopts: u32,
    sopass: [u8; 6],
}

#[repr(C)]
struct IfReq {
    ifr_name: [u8; libc::IFNAMSIZ],
    ifr_data: *mut libc::c_void,
    _pad: [u8; 16],
}

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
        self.adapters.iter().find(|a| a.wired).or_else(|| self.adapters.first())
    }

    pub fn state_str(&self) -> &'static str {
        match self.primary() {
            Some(primary) => primary.support.as_str(),
            None => "none",
        }
    }
}

pub fn query_wake_status() -> WakeStatus {
    let default_iface = default_route().map(|(iface, _)| iface);
    let mut adapters = enumerate_wake_adapters();
    adapters.sort_by_key(|a| {
        (
            Some(a.label.as_str()) != default_iface.as_deref(),
            !a.wired,
            a.broadcasts.is_empty(),
            match a.support {
                WakeSupport::Enabled => 0u8,
                WakeSupport::Unknown => 1,
                WakeSupport::Disabled => 2,
            },
            a.label.clone(),
        )
    });

    let mut macs = Vec::new();
    let mut broadcast_set = BTreeSet::new();
    for adapter in &adapters {
        if !macs.contains(&adapter.mac) {
            macs.push(adapter.mac.clone());
        }
        for bcast in &adapter.broadcasts {
            broadcast_set.insert(bcast.clone());
        }
    }

    WakeStatus { info: PcWakeInfo { macs, broadcasts: broadcast_set.into_iter().collect() }, adapters }
}

pub fn is_virtual_interface_name(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    lower == "lo"
        || lower.starts_with("tun")
        || lower.starts_with("tap")
        || lower.starts_with("tailscale")
        || lower.starts_with("wg")
        || lower.starts_with("docker")
        || lower.starts_with("veth")
        || lower.starts_with("br-")
        || lower.starts_with("virbr")
        || lower.starts_with("nordlynx")
        || lower.starts_with("zt")
        || lower.starts_with("vmnet")
        || lower.starts_with("vboxnet")
}

fn is_physical_ethernet_interface(name: &str, path: &Path) -> bool {
    if is_virtual_interface_name(name) {
        return false;
    }
    if !path.join("device").exists() {
        return false;
    }
    let Ok(if_type) = fs::read_to_string(path.join("type")) else {
        return false;
    };
    if_type.trim() == ARPHRD_ETHER
}

fn enumerate_wake_adapters() -> Vec<WakeAdapter> {
    let mut result = Vec::new();
    let Ok(entries) = fs::read_dir("/sys/class/net") else {
        return result;
    };
    let broadcasts_by_iface = query_ipv4_broadcasts();

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        if !is_physical_ethernet_interface(&name, &path) {
            continue;
        }
        let operstate =
            fs::read_to_string(path.join("operstate")).unwrap_or_default().trim().to_ascii_lowercase();
        if operstate != "up" && operstate != "unknown" {
            continue;
        }
        let raw_mac = fs::read_to_string(path.join("address")).unwrap_or_default();
        let Some(mac) = normalize_mac(&raw_mac) else {
            continue;
        };
        let wired = !path.join("wireless").exists() && !path.join("phy80211").exists();
        let support = probe_adapter_wake_support(&name, &path);
        let broadcasts = broadcasts_by_iface.get(&name).cloned().unwrap_or_default();

        result.push(WakeAdapter { label: name, wired, mac, broadcasts, support });
    }
    result
}

pub fn parse_ethtool_wol(output: &str) -> Option<bool> {
    let mut supports_magic = false;
    let mut active_modes: Option<&str> = None;

    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Supports Wake-on:") {
            if rest.trim().contains('g') {
                supports_magic = true;
            }
        } else if let Some(rest) = trimmed.strip_prefix("Wake-on:") {
            active_modes = Some(rest.trim());
        }
    }

    let modes = active_modes?;
    if modes.contains('g') {
        Some(true)
    } else if supports_magic || modes.contains('d') {
        Some(false)
    } else {
        None
    }
}

fn probe_ethtool_ioctl(iface_name: &str) -> Option<WakeSupport> {
    let name_bytes = iface_name.as_bytes();
    if name_bytes.is_empty() || name_bytes.len() >= libc::IFNAMSIZ || name_bytes.contains(&0) {
        return None;
    }

    // SAFETY: opening an unprivileged AF_INET datagram socket for SIOCETHTOOL.
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return None;
    }

    let mut wol = EthtoolWolInfo { cmd: ETHTOOL_GWOL, supported: 0, wolopts: 0, sopass: [0u8; 6] };
    let mut ifr = IfReq {
        ifr_name: [0u8; libc::IFNAMSIZ],
        ifr_data: (&raw mut wol).cast::<libc::c_void>(),
        _pad: [0u8; 16],
    };
    ifr.ifr_name[..name_bytes.len()].copy_from_slice(name_bytes);

    // SAFETY: `ifr` points to our valid stack `EthtoolWolInfo` and `fd` is open.
    let rc = unsafe { libc::ioctl(fd, SIOCETHTOOL as _, &mut ifr) };
    // SAFETY: closing the socket opened above.
    unsafe {
        let _ = libc::close(fd);
    }

    if rc < 0 {
        return None;
    }
    if (wol.wolopts & WAKE_MAGIC) != 0 {
        Some(WakeSupport::Enabled)
    } else if (wol.supported & WAKE_MAGIC) != 0 || wol.supported != 0 {
        Some(WakeSupport::Disabled)
    } else {
        None
    }
}

fn probe_adapter_wake_support(iface_name: &str, iface_sys_path: &Path) -> WakeSupport {
    if let Some(support) = probe_ethtool_ioctl(iface_name) {
        return support;
    }

    let wakeup_path = iface_sys_path.join("device/power/wakeup");
    let sysfs_state = fs::read_to_string(wakeup_path).ok();
    if sysfs_state.as_deref().map(str::trim) == Some("enabled") {
        return WakeSupport::Enabled;
    }

    if let Ok(output) = Command::new("ethtool").arg(iface_name).output()
        && output.status.success()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        match parse_ethtool_wol(&stdout) {
            Some(true) => return WakeSupport::Enabled,
            Some(false) => return WakeSupport::Disabled,
            None => {}
        }
    }

    match sysfs_state.as_deref().map(str::trim) {
        Some("enabled") => WakeSupport::Enabled,
        Some("disabled") => WakeSupport::Disabled,
        _ => WakeSupport::Unknown,
    }
}

/// Parses `/proc/net/route` to find the preferred default route (`Destination == 00000000`)
/// and its IPv4 gateway address, choosing the lowest-metric `RTF_UP` entry.
pub fn parse_proc_net_route(route_table: &str) -> Option<(String, Option<Ipv4Addr>)> {
    const RTF_UP: u32 = 0x0001;
    const RTF_GATEWAY: u32 = 0x0002;

    let mut best: Option<(u32, String, Option<Ipv4Addr>)> = None;
    for line in route_table.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 8 {
            continue;
        }
        let iface = fields[0];
        if is_virtual_interface_name(iface) {
            continue;
        }
        let Ok(dest) = u32::from_str_radix(fields[1], 16) else {
            continue;
        };
        let Ok(gw_raw) = u32::from_str_radix(fields[2], 16) else {
            continue;
        };
        let Ok(flags) = u32::from_str_radix(fields[3], 16) else {
            continue;
        };
        let metric = fields[6].parse::<u32>().unwrap_or(u32::MAX);
        let Ok(mask) = u32::from_str_radix(fields[7], 16) else {
            continue;
        };
        if dest != 0 || mask != 0 || (flags & RTF_UP) == 0 {
            continue;
        }
        let gateway = if (flags & RTF_GATEWAY) != 0 && gw_raw != 0 {
            Some(Ipv4Addr::from(gw_raw.to_ne_bytes()))
        } else {
            None
        };
        if best.as_ref().is_none_or(|(best_metric, _, _)| metric < *best_metric) {
            best = Some((metric, iface.to_string(), gateway));
        }
    }
    best.map(|(_, iface, gw)| (iface, gw))
}

/// Returns the default-route interface name and optional gateway IPv4 from `/proc/net/route`.
pub fn default_route() -> Option<(String, Option<Ipv4Addr>)> {
    let contents = fs::read_to_string("/proc/net/route").ok()?;
    parse_proc_net_route(&contents)
}

/// Parses `/proc/net/arp` to resolve `target_ip` to a normalized lowercase MAC (`aa:bb:cc:dd:ee:ff`).
pub fn parse_proc_net_arp(arp_table: &str, target_ip: Ipv4Addr) -> Option<String> {
    for line in arp_table.lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 6 {
            continue;
        }
        let Ok(ip) = cols[0].parse::<Ipv4Addr>() else {
            continue;
        };
        if ip != target_ip {
            continue;
        }
        // Flags == 0x0 means incomplete ARP entry.
        if cols[2].eq_ignore_ascii_case("0x0") {
            continue;
        }
        if let Some(mac) = normalize_mac(cols[3]) {
            return Some(mac);
        }
    }
    None
}

/// Parses `ip neigh` output as a fallback when `/proc/net/arp` does not contain `target_ip`.
pub fn parse_ip_neigh(output: &str, target_ip: Ipv4Addr) -> Option<String> {
    for line in output.lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let Some(first) = tokens.first() else {
            continue;
        };
        let Ok(ip) = first.parse::<Ipv4Addr>() else {
            continue;
        };
        if ip != target_ip {
            continue;
        }
        for window in tokens.windows(2) {
            if window[0] == "lladdr"
                && let Some(mac) = normalize_mac(window[1])
            {
                return Some(mac);
            }
        }
    }
    None
}

/// Resolves the default gateway's MAC address in-process from `/proc/net/route` + `/proc/net/arp`,
/// falling back to `ip neigh` only if `/proc/net/arp` misses it.
pub fn query_gateway_mac() -> Option<String> {
    let (_, Some(gw_ip)) = default_route()? else {
        return None;
    };
    if let Ok(arp) = fs::read_to_string("/proc/net/arp")
        && let Some(mac) = parse_proc_net_arp(&arp, gw_ip)
    {
        return Some(mac);
    }
    let out = Command::new("ip").args(["neigh", "show", &gw_ip.to_string()]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    parse_ip_neigh(&String::from_utf8_lossy(&out.stdout), gw_ip)
}

fn query_ipv4_broadcasts() -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();

    // First query in-process via `getifaddrs`.
    let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: `getifaddrs` allocates and populates a linked list freed by `freeifaddrs`.
    if unsafe { libc::getifaddrs(&mut ifap) } == 0 && !ifap.is_null() {
        let mut cur = ifap;
        // SAFETY: walking the valid `ifaddrs` list before `freeifaddrs`.
        unsafe {
            while let Some(ifa) = cur.as_ref() {
                cur = ifa.ifa_next;
                if ifa.ifa_name.is_null() || ifa.ifa_addr.is_null() || ifa.ifa_netmask.is_null() {
                    continue;
                }
                if i32::from((*ifa.ifa_addr).sa_family) != libc::AF_INET {
                    continue;
                }
                let Ok(name) = std::ffi::CStr::from_ptr(ifa.ifa_name).to_str() else {
                    continue;
                };
                let sin_addr = (*ifa.ifa_addr.cast::<libc::sockaddr_in>()).sin_addr.s_addr;
                let sin_mask = (*ifa.ifa_netmask.cast::<libc::sockaddr_in>()).sin_addr.s_addr;
                let ip = Ipv4Addr::from(u32::from_be(sin_addr));
                let prefix = u32::from_be(sin_mask).leading_ones() as u8;
                if let Some(bcast) = subnet_broadcast(ip, prefix) {
                    let list = map.entry(name.to_string()).or_default();
                    let s = bcast.to_string();
                    if !list.contains(&s) {
                        list.push(s);
                    }
                }
            }
            libc::freeifaddrs(ifap);
        }
        if !map.is_empty() {
            return map;
        }
    }

    let Ok(output) = Command::new("ip").args(["-4", "-o", "addr", "show"]).output() else {
        return map;
    };
    if !output.status.success() {
        return map;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 4 {
            continue;
        }
        let iface = parts[1].to_string();
        for window in parts.windows(2) {
            if window[0] == "inet"
                && let Some((ip_str, prefix_str)) = window[1].split_once('/')
                && let (Ok(ip), Ok(prefix)) = (ip_str.parse::<Ipv4Addr>(), prefix_str.parse::<u8>())
                && let Some(bcast) = subnet_broadcast(ip, prefix)
            {
                let list = map.entry(iface.clone()).or_default();
                let s = bcast.to_string();
                if !list.contains(&s) {
                    list.push(s);
                }
            }
        }
    }
    map
}

/// All non-loopback IPv4 and IPv6 unicast addresses currently assigned to UP
/// interfaces on this machine (including Tailscale / VPN tunnels).
pub fn interface_ips() -> Vec<IpAddr> {
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

pub fn normalize_mac(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let parts: Vec<&str> = trimmed.split([':', '-']).collect();
    if parts.len() != 6 {
        return None;
    }
    let mut bytes = [0u8; 6];
    for (dst, part) in bytes.iter_mut().zip(parts) {
        if part.is_empty() || part.len() > 2 {
            return None;
        }
        *dst = u8::from_str_radix(part, 16).ok()?;
    }
    format_mac(&bytes)
}

pub(crate) fn format_mac(bytes: &[u8]) -> Option<String> {
    if bytes.len() != 6 {
        return None;
    }
    if bytes.iter().all(|&b| b == 0) || bytes.iter().all(|&b| b == 0xFF) {
        return None;
    }
    Some(format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5]
    ))
}

pub(crate) fn subnet_broadcast(ip: Ipv4Addr, prefix_len: u8) -> Option<Ipv4Addr> {
    if prefix_len == 0 || prefix_len >= 32 {
        return None;
    }
    let ip_u32 = u32::from(ip);
    let host_mask = (1u32 << (32 - prefix_len)) - 1;
    let bcast = ip_u32 | host_mask;
    Some(Ipv4Addr::from(bcast))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_valid_mac_lowercase_and_rejects_invalid() {
        assert_eq!(format_mac(&[0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]), Some("aa:bb:cc:dd:ee:ff".to_string()));
        assert_eq!(normalize_mac("AA:BB:CC:DD:EE:FF"), Some("aa:bb:cc:dd:ee:ff".to_string()));
        assert_eq!(normalize_mac("00-11-22-33-44-55"), Some("00:11:22:33:44:55".to_string()));
        assert_eq!(format_mac(&[0, 0, 0, 0, 0, 0]), None);
        assert_eq!(format_mac(&[0xff; 6]), None);
        assert_eq!(format_mac(&[1, 2, 3]), None);
        assert_eq!(normalize_mac("00:00:00:00:00:00"), None);
        assert_eq!(normalize_mac("ff:ff:ff:ff:ff:ff"), None);
    }

    #[test]
    fn excludes_virtual_and_tunnel_interface_names() {
        for virt in
            ["lo", "tun0", "tap0", "tailscale0", "wg0", "docker0", "veth1a2b3c", "br-123456", "virbr0"]
        {
            assert!(is_virtual_interface_name(virt), "expected {virt} to be excluded");
        }
        for phys in ["eth0", "enp3s0", "eno1", "wlan0", "wlp2s0"] {
            assert!(!is_virtual_interface_name(phys), "expected {phys} to be allowed");
        }
    }

    #[test]
    fn calculates_subnet_broadcast() {
        assert_eq!(
            subnet_broadcast(Ipv4Addr::new(192, 168, 1, 42), 24),
            Some(Ipv4Addr::new(192, 168, 1, 255))
        );
        assert_eq!(subnet_broadcast(Ipv4Addr::new(10, 0, 12, 5), 16), Some(Ipv4Addr::new(10, 0, 255, 255)));
        assert_eq!(subnet_broadcast(Ipv4Addr::new(192, 168, 1, 1), 32), None);
        assert_eq!(subnet_broadcast(Ipv4Addr::new(192, 168, 1, 1), 0), None);
    }

    #[test]
    fn parses_ethtool_magic_packet_enabled_and_disabled() {
        let enabled_output = "\
Settings for enp3s0:
\tSupported ports: [ TP ]
\tSupports Wake-on: pumbg
\tWake-on: g
\tLink detected: yes
";
        assert_eq!(parse_ethtool_wol(enabled_output), Some(true));

        let disabled_output = "\
Settings for enp3s0:
\tSupported ports: [ TP ]
\tSupports Wake-on: pumbg
\tWake-on: d
\tLink detected: yes
";
        assert_eq!(parse_ethtool_wol(disabled_output), Some(false));

        let unicast_only_output = "\
Settings for enp3s0:
\tSupports Wake-on: pumbg
\tWake-on: u
";
        assert_eq!(parse_ethtool_wol(unicast_only_output), Some(false));

        assert_eq!(parse_ethtool_wol("Settings for lo:\n\tLink detected: yes\n"), None);
    }

    #[test]
    fn parses_proc_net_route_and_arp_tables() {
        let route_table = "\
Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT
wlan0\t00000000\t0101A8C0\t0003\t0\t0\t600\t00000000\t0\t0\t0
enp3s0\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0
tun0\t00000000\t0100000A\t0003\t0\t0\t50\t00000000\t0\t0\t0
";
        let (iface, gw) = parse_proc_net_route(route_table).expect("default route");
        assert_eq!(iface, "enp3s0");
        assert_eq!(gw, Some(Ipv4Addr::new(192, 168, 1, 1)));

        let arp_table = "\
IP address       HW type     Flags       HW address            Mask     Device
192.168.1.50     0x1         0x0         00:00:00:00:00:00     *        enp3s0
192.168.1.1      0x1         0x2         DC:A6:32:12:34:56     *        enp3s0
";
        assert_eq!(
            parse_proc_net_arp(arp_table, Ipv4Addr::new(192, 168, 1, 1)),
            Some("dc:a6:32:12:34:56".to_string())
        );
        assert_eq!(parse_proc_net_arp(arp_table, Ipv4Addr::new(192, 168, 1, 50)), None);

        let ip_neigh = "192.168.1.1 dev enp3s0 lladdr DC:A6:32:12:34:56 REACHABLE\n";
        assert_eq!(
            parse_ip_neigh(ip_neigh, Ipv4Addr::new(192, 168, 1, 1)),
            Some("dc:a6:32:12:34:56".to_string())
        );
    }
}
