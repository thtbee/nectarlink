// SPDX-License-Identifier: GPL-3.0-or-later
use std::collections::BTreeSet;
use std::fs;
use std::net::Ipv4Addr;
use std::path::Path;
use std::process::Command;

use nectarlink_core::PcWakeInfo;

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
    let mut adapters = enumerate_wake_adapters();
    adapters.sort_by_key(|a| (!a.wired, a.label.clone()));

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

fn enumerate_wake_adapters() -> Vec<WakeAdapter> {
    let mut result = Vec::new();
    let Ok(entries) = fs::read_dir("/sys/class/net") else {
        return result;
    };
    let broadcasts_by_iface = query_ipv4_broadcasts();

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "lo"
            || name.starts_with("docker")
            || name.starts_with("veth")
            || name.starts_with("br-")
            || name.starts_with("virbr")
            || name.starts_with("tun")
            || name.starts_with("tap")
            || name.starts_with("wg")
        {
            continue;
        }
        let path = entry.path();
        let operstate =
            fs::read_to_string(path.join("operstate")).unwrap_or_default().trim().to_ascii_lowercase();
        if operstate != "up" && operstate != "unknown" {
            continue;
        }
        let raw_mac = fs::read_to_string(path.join("address")).unwrap_or_default();
        let bytes: Vec<u8> =
            raw_mac.trim().split(':').filter_map(|p| u8::from_str_radix(p, 16).ok()).collect();
        let Some(mac) = format_mac(&bytes) else {
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

fn probe_adapter_wake_support(iface_name: &str, iface_sys_path: &Path) -> WakeSupport {
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
    let wakeup_path = iface_sys_path.join("device/power/wakeup");
    if let Ok(state) = fs::read_to_string(wakeup_path) {
        return match state.trim() {
            "enabled" => WakeSupport::Enabled,
            "disabled" => WakeSupport::Disabled,
            _ => WakeSupport::Unknown,
        };
    }
    WakeSupport::Unknown
}

fn query_ipv4_broadcasts() -> std::collections::HashMap<String, Vec<String>> {
    let mut map: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
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

pub(crate) fn format_mac(bytes: &[u8]) -> Option<String> {
    if bytes.len() != 6 {
        return None;
    }
    if bytes.iter().all(|&b| b == 0) || bytes.iter().all(|&b| b == 0xFF) {
        return None;
    }
    Some(format!(
        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
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
    fn formats_valid_mac_and_rejects_invalid() {
        assert_eq!(format_mac(&[0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]), Some("AA:BB:CC:DD:EE:FF".to_string()));
        assert_eq!(format_mac(&[0, 0, 0, 0, 0, 0]), None);
        assert_eq!(format_mac(&[0xff; 6]), None);
        assert_eq!(format_mac(&[1, 2, 3]), None);
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
}
