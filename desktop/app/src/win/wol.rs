// SPDX-License-Identifier: GPL-3.0-or-later
//! Reads this PC's physical Ethernet and Wi-Fi adapters with
//! `GetAdaptersAddresses` and checks (read-only) whether Wake on Magic Packet
//! is enabled in the network adapter class registry. Never modifies any
//! adapter or power settings.

use std::net::Ipv4Addr;

use nectarlink_core::{PcWakeInfo, format_mac, ipv4_broadcast, parse_mac};
use windows::{
    Win32::{
        Foundation::ERROR_SUCCESS,
        NetworkManagement::IpHelper::{
            GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_MULTICAST, GetAdaptersAddresses, IP_ADAPTER_ADDRESSES_LH,
        },
        Networking::WinSock::AF_UNSPEC,
        System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW},
    },
    core::HSTRING,
};

const IF_TYPE_ETHERNET_CSMACD: u32 = 6;
const IF_TYPE_IEEE80211: u32 = 71;

const NET_CLASS_KEY: &str = r"SYSTEM\CurrentControlSet\Control\Class\{4d36e972-e325-11ce-bfc1-08002be10318}";

/// NCF_VIRTUAL = 0x1, NCF_PHYSICAL = 0x4.
const NCF_VIRTUAL: u32 = 0x01;
const NCF_PHYSICAL: u32 = 0x04;
/// In `PnPCapabilities`, bit 0x10 means "Allow this device to wake the computer" is unchecked.
const PNP_DISABLE_WAKE: u32 = 0x10;

/// Whether Wake on Magic Packet is enabled on a physical adapter.
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

/// One physical Ethernet or Wi-Fi adapter that is currently up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WakeAdapter {
    /// Friendly name + description, e.g. `"Ethernet (Realtek PCIe GbE Family Controller)"`.
    pub label: String,
    /// True for wired Ethernet (`IfType == 6`), false for Wi-Fi (`IfType == 71`).
    pub wired: bool,
    /// Normalized MAC (`"aa:bb:cc:dd:ee:ff"`).
    pub mac: String,
    /// Directed IPv4 subnet broadcast addresses on this adapter.
    pub broadcasts: Vec<String>,
    /// Whether Magic Packet wake is enabled according to the registry.
    pub support: WakeSupport,
}

/// Summary of this PC's Wake-on-LAN readiness and the `pc.wake_info` payload
/// to send to paired phones.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WakeStatus {
    pub info: PcWakeInfo,
    pub adapters: Vec<WakeAdapter>,
}

impl WakeStatus {
    /// The preferred adapter (wired Ethernet first), if any.
    pub fn primary(&self) -> Option<&WakeAdapter> {
        self.adapters.first()
    }

    /// `"enabled"`, `"disabled"`, `"unknown"`, or `"none"`.
    pub fn state_str(&self) -> &'static str {
        match self.primary() {
            Some(a) => a.support.as_str(),
            None => "none",
        }
    }
}

#[derive(Debug, Default)]
struct RegAdapterInfo {
    net_cfg_id: String,
    characteristics: Option<u32>,
    device_instance_id: Option<String>,
    wake_on_magic_packet: Option<bool>,
    pnp_wake_allowed: Option<bool>,
}

impl RegAdapterInfo {
    fn is_physical_hardware(&self) -> bool {
        if let Some(chars) = self.characteristics
            && ((chars & NCF_VIRTUAL) != 0 || (chars & NCF_PHYSICAL) == 0)
        {
            return false;
        }
        if let Some(inst) = &self.device_instance_id {
            let upper = inst.to_ascii_uppercase();
            if upper.starts_with("ROOT\\") || upper.starts_with("SWD\\") || upper.starts_with("UMB\\") {
                return false;
            }
        }
        true
    }

    fn wake_support(&self) -> WakeSupport {
        if self.pnp_wake_allowed == Some(false) || self.wake_on_magic_packet == Some(false) {
            return WakeSupport::Disabled;
        }
        if self.wake_on_magic_packet == Some(true) {
            return WakeSupport::Enabled;
        }
        WakeSupport::Unknown
    }
}

fn reg_dword(subkey: &str, value: &str) -> Option<u32> {
    let mut data = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: data/size describe a live u32 buffer.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            &HSTRING::from(subkey),
            &HSTRING::from(value),
            RRF_RT_REG_DWORD,
            None,
            Some(std::ptr::from_mut(&mut data).cast()),
            Some(&mut size),
        )
    };
    (status == ERROR_SUCCESS).then_some(data)
}

fn reg_string(subkey: &str, value: &str) -> Option<String> {
    let mut buf = [0u16; 256];
    let mut size = std::mem::size_of_val(&buf) as u32;
    // SAFETY: buf/size describe a live UTF-16 buffer.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            &HSTRING::from(subkey),
            &HSTRING::from(value),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

fn reg_bool_flag(subkey: &str, value: &str) -> Option<bool> {
    if let Some(s) = reg_string(subkey, value) {
        return match s.trim().to_ascii_lowercase().as_str() {
            "1" | "enabled" | "true" => Some(true),
            "0" | "disabled" | "false" => Some(false),
            _ => None,
        };
    }
    reg_dword(subkey, value).map(|v| v != 0)
}

fn read_registry_adapters() -> Vec<RegAdapterInfo> {
    let mut out = Vec::new();
    for idx in 0..100 {
        let subkey = format!(r"{NET_CLASS_KEY}\{idx:04}");
        let Some(net_cfg_id) = reg_string(&subkey, "NetCfgInstanceId") else {
            continue;
        };
        let characteristics = reg_dword(&subkey, "Characteristics");
        let device_instance_id = reg_string(&subkey, "DeviceInstanceID");
        let wake_on_magic_packet = reg_bool_flag(&subkey, "*WakeOnMagicPacket")
            .or_else(|| reg_bool_flag(&subkey, "WakeOnMagicPacket"));
        let pnp_wake_allowed = reg_dword(&subkey, "PnPCapabilities").map(|pnp| (pnp & PNP_DISABLE_WAKE) == 0);
        out.push(RegAdapterInfo {
            net_cfg_id,
            characteristics,
            device_instance_id,
            wake_on_magic_packet,
            pnp_wake_allowed,
        });
    }
    out
}

fn looks_virtual_name(friendly: &str, description: &str) -> bool {
    let f = friendly.to_ascii_lowercase();
    let d = description.to_ascii_lowercase();
    const VIRTUAL_WORDS: &[&str] = &[
        "virtual",
        "vethernet",
        "hyper-v",
        "wsl",
        "vmware",
        "virtualbox",
        "vbox",
        "loopback",
        "tap-",
        "tap-windows",
        "tuntap",
        "wintun",
        "vpn",
        "wireguard",
        "openvpn",
        "tailscale",
        "zerotier",
        "bluetooth",
        "wan miniport",
        "ndis",
        "teredo",
        "isatap",
        "docker",
        "npcap",
        "wi-fi direct",
        "hosted network",
    ];
    VIRTUAL_WORDS.iter().any(|w| f.contains(w) || d.contains(w))
}

/// Reads physical, up Ethernet and Wi-Fi adapters and their Wake-on-LAN status.
pub fn query_wake_status() -> WakeStatus {
    let reg_entries = read_registry_adapters();
    let mut size: u32 = 32 * 1024;
    let mut buffer = vec![0u8; size as usize];
    let mut adapters: Vec<WakeAdapter> = Vec::new();

    // SAFETY: `buffer` is sized as `GetAdaptersAddresses` requests and walked
    // via its `Next` and `FirstUnicastAddress` pointers while alive.
    unsafe {
        let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST;
        let mut status = GetAdaptersAddresses(
            u32::from(AF_UNSPEC.0),
            flags,
            None,
            Some(buffer.as_mut_ptr().cast()),
            &mut size,
        );
        if status == 111 {
            // ERROR_BUFFER_OVERFLOW
            buffer = vec![0u8; size as usize];
            status = GetAdaptersAddresses(
                u32::from(AF_UNSPEC.0),
                flags,
                None,
                Some(buffer.as_mut_ptr().cast()),
                &mut size,
            );
        }
        if status != 0 {
            return WakeStatus::default();
        }

        let mut ptr = buffer.as_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        while let Some(a) = ptr.as_ref() {
            ptr = a.Next;

            // Only operational (Up) Ethernet or Wi-Fi adapters with a 6-byte MAC.
            if a.OperStatus.0 != 1 {
                continue;
            }
            let wired = match a.IfType {
                IF_TYPE_ETHERNET_CSMACD => true,
                IF_TYPE_IEEE80211 => false,
                _ => continue,
            };
            if a.PhysicalAddressLength != 6 {
                continue;
            }
            let mut raw_mac = [0u8; 6];
            raw_mac.copy_from_slice(&a.PhysicalAddress[..6]);
            let mac = format_mac(&raw_mac);
            if parse_mac(&mac).is_none() {
                continue;
            }

            let guid = a.AdapterName.to_string().unwrap_or_default();
            let friendly = a.FriendlyName.to_string().unwrap_or_default();
            let description = a.Description.to_string().unwrap_or_default();
            if looks_virtual_name(&friendly, &description) {
                continue;
            }

            let reg = reg_entries.iter().find(|r| r.net_cfg_id.eq_ignore_ascii_case(&guid));
            if let Some(reg) = reg
                && !reg.is_physical_hardware()
            {
                continue;
            }
            let support = reg.map_or(WakeSupport::Unknown, RegAdapterInfo::wake_support);

            let mut broadcasts = Vec::new();
            let mut unicast = a.FirstUnicastAddress;
            while let Some(u) = unicast.as_ref() {
                unicast = u.Next;
                let addr = u.Address.lpSockaddr;
                if !addr.is_null() && (*addr).sa_family.0 == 2 {
                    let bytes = &(*addr).sa_data;
                    let ip = Ipv4Addr::new(bytes[2] as u8, bytes[3] as u8, bytes[4] as u8, bytes[5] as u8);
                    if let Some(bcast) = ipv4_broadcast(ip, u.OnLinkPrefixLength) {
                        let s = bcast.to_string();
                        if !broadcasts.contains(&s) {
                            broadcasts.push(s);
                        }
                    }
                }
            }

            let label = match (friendly.trim(), description.trim()) {
                ("", d) => d.to_owned(),
                (f, "") => f.to_owned(),
                (f, d) if f.eq_ignore_ascii_case(d) => f.to_owned(),
                (f, d) => format!("{f} ({d})"),
            };

            adapters.push(WakeAdapter { label, wired, mac, broadcasts, support });
        }
    }

    // Prefer wired Ethernet over Wi-Fi, then adapters with an IPv4 broadcast,
    // then adapters where Wake on Magic Packet is enabled.
    adapters.sort_by_key(|a| {
        (
            !a.wired,
            a.broadcasts.is_empty(),
            match a.support {
                WakeSupport::Enabled => 0u8,
                WakeSupport::Unknown => 1,
                WakeSupport::Disabled => 2,
            },
        )
    });

    let mut macs = Vec::new();
    let mut broadcasts = Vec::new();
    for a in &adapters {
        if !macs.contains(&a.mac) {
            macs.push(a.mac.clone());
        }
        for b in &a.broadcasts {
            if !broadcasts.contains(b) {
                broadcasts.push(b.clone());
            }
        }
    }
    let info = PcWakeInfo { macs, broadcasts }.sanitized();

    WakeStatus { info, adapters }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_virtual_adapter_names() {
        assert!(looks_virtual_name("vEthernet (WSL (Hyper-V firewall))", "Hyper-V Virtual Ethernet Adapter"));
        assert!(looks_virtual_name("Tailscale", "Tailscale Tunnel"));
        assert!(looks_virtual_name("Ethernet 2", "VMware Virtual Ethernet Adapter for VMnet1"));
        assert!(!looks_virtual_name("Ethernet", "Realtek PCIe GbE Family Controller"));
        assert!(!looks_virtual_name("Wi-Fi", "Intel(R) Wi-Fi 6E AX211 160MHz"));
    }

    #[test]
    fn queries_wake_status_without_admin() {
        let status = query_wake_status();
        assert!(status.info.is_valid());
        for a in &status.adapters {
            assert!(parse_mac(&a.mac).is_some());
            assert!(!a.label.is_empty());
        }
    }
}
