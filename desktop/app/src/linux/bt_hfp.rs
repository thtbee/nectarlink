// SPDX-License-Identifier: GPL-3.0-or-later
//! Paired Bluetooth device inspection and Hands-Free Profile (HFP) call audio
//! routing on Linux via BlueZ (`bluetoothctl`) and PipeWire/PulseAudio
//! (`pactl set-card-profile`).

use std::process::Command;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HfpStatus {
    pub available: bool,
    pub connected: bool,
    pub device_name: String,
    pub detail: String,
}

static ACTIVE_MAC: OnceLock<Mutex<Option<(String, String)>>> = OnceLock::new();

fn active_slot() -> &'static Mutex<Option<(String, String)>> {
    ACTIVE_MAC.get_or_init(|| Mutex::new(None))
}

/// Parses `bluetoothctl devices` / `bluetoothctl devices Paired` output lines
/// of the form `"Device AA:BB:CC:DD:EE:FF Pixel 9 Pro"` into `(mac, name)` pairs.
pub(crate) fn parse_bluetoothctl_devices(output: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for raw_line in output.lines() {
        let line = raw_line.trim();
        let Some(rest) = line.strip_prefix("Device ") else {
            continue;
        };
        let rest = rest.trim();
        let mut parts = rest.splitn(2, char::is_whitespace);
        let Some(mac) = parts.next().map(str::trim).filter(|m| !m.is_empty()) else {
            continue;
        };
        let name = parts.next().map(str::trim).unwrap_or_default().to_owned();
        out.push((mac.to_owned(), name));
    }
    out
}

/// Parses `bluetoothctl info <mac>` output to determine whether `Connected: yes`
/// is present.
pub(crate) fn parse_bluetoothctl_connected(info_output: &str) -> bool {
    info_output.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.strip_prefix("Connected:").map(|v| v.trim().eq_ignore_ascii_case("yes")).unwrap_or(false)
    })
}

/// Formats a Bluetooth MAC address (`AA:BB:CC:DD:EE:FF`) into its PipeWire /
/// PulseAudio BlueZ card name (`bluez_card.AA_BB_CC_DD_EE_FF`).
pub(crate) fn bluez_card_name_for_mac(mac: &str) -> String {
    let underscored: String =
        mac.trim().chars().map(|c| if c == ':' || c == '-' { '_' } else { c }).collect();
    format!("bluez_card.{underscored}")
}

pub fn match_endpoint<'a>(
    endpoints: &'a [(String, String)],
    phone_name: Option<&str>,
) -> Option<&'a (String, String)> {
    if endpoints.is_empty() {
        return None;
    }
    if let Some(needle) = phone_name.map(str::trim).filter(|s| !s.is_empty()) {
        let needle_lower = needle.to_ascii_lowercase();
        if let Some(found) = endpoints.iter().find(|(_, name)| {
            let hay = name.trim().to_ascii_lowercase();
            !hay.is_empty() && (hay.contains(&needle_lower) || needle_lower.contains(&hay))
        }) {
            return Some(found);
        }
    }
    endpoints.first()
}

fn enumerate_paired_devices() -> Vec<(String, String)> {
    if let Ok(out) = Command::new("bluetoothctl").args(["devices", "Paired"]).output()
        && out.status.success()
    {
        let parsed = parse_bluetoothctl_devices(&String::from_utf8_lossy(&out.stdout));
        if !parsed.is_empty() {
            return parsed;
        }
    }
    if let Ok(out) = Command::new("bluetoothctl").arg("devices").output()
        && out.status.success()
    {
        return parse_bluetoothctl_devices(&String::from_utf8_lossy(&out.stdout));
    }
    Vec::new()
}

fn query_device_connected(mac: &str) -> bool {
    Command::new("bluetoothctl")
        .args(["info", mac])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| parse_bluetoothctl_connected(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or(false)
}

pub fn probe_status(phone_name: Option<&str>) -> HfpStatus {
    let endpoints = enumerate_paired_devices();
    if let Some((mac, name)) = match_endpoint(&endpoints, phone_name) {
        let display = if name.trim().is_empty() {
            phone_name.map(str::trim).filter(|s| !s.is_empty()).unwrap_or(mac.as_str()).to_owned()
        } else {
            name.clone()
        };
        let connected = query_device_connected(mac);
        let detail = if connected {
            format!("Bluetooth connected ({display}).")
        } else {
            format!("Bluetooth paired with {display} (not currently connected).")
        };
        return HfpStatus { available: true, connected, device_name: display, detail };
    }

    HfpStatus {
        available: false,
        connected: false,
        device_name: phone_name.unwrap_or_default().to_string(),
        detail: "Pair your phone over system Bluetooth to route call audio through this PC.".to_string(),
    }
}

pub fn connect_audio(phone_name: Option<&str>) -> Result<HfpStatus, String> {
    let endpoints = enumerate_paired_devices();
    let Some((mac, name)) = match_endpoint(&endpoints, phone_name) else {
        return Ok(probe_status(phone_name));
    };
    let display = if name.trim().is_empty() {
        phone_name.map(str::trim).filter(|s| !s.is_empty()).unwrap_or(mac.as_str()).to_owned()
    } else {
        name.clone()
    };

    let _ = Command::new("bluetoothctl").args(["connect", mac]).status();
    let card = bluez_card_name_for_mac(mac);
    let _ = Command::new("pactl")
        .args(["set-card-profile", &card, "headset-head-unit"])
        .status()
        .and_then(|s| if s.success() { Ok(s) } else { Err(std::io::Error::other("profile")) })
        .or_else(|_| {
            Command::new("pactl").args(["set-card-profile", &card, "headset-head-unit-cvsd"]).status()
        });

    let connected = query_device_connected(mac);
    if let Ok(mut guard) = active_slot().lock() {
        *guard = Some((mac.clone(), display.clone()));
    }
    let detail = if connected {
        format!("Bluetooth HFP call audio connected ({display}).")
    } else {
        format!("Bluetooth paired with {display} (not currently connected).")
    };
    Ok(HfpStatus { available: true, connected, device_name: display, detail })
}

pub fn disconnect_audio() {
    if let Ok(mut guard) = active_slot().lock()
        && let Some((mac, _)) = guard.take()
    {
        let card = bluez_card_name_for_mac(&mac);
        let _ = Command::new("pactl").args(["set-card-profile", &card, "a2dp-sink"]).status();
    }
}

pub fn open_bluetooth_settings() {
    let _ = Command::new("blueman-manager")
        .spawn()
        .or_else(|_| Command::new("gnome-control-center").arg("bluetooth").spawn())
        .or_else(|_| Command::new("xdg-open").arg("settings://bluetooth").spawn());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_endpoint_prefers_phone_name_case_insensitive() {
        let list = vec![
            ("id-1".to_string(), "Galaxy Buds".to_string()),
            ("id-2".to_string(), "Pixel 9 Pro".to_string()),
        ];
        let matched = match_endpoint(&list, Some("pixel 9")).unwrap();
        assert_eq!(matched.0, "id-2");
    }

    #[test]
    fn match_endpoint_falls_back_to_first_when_unmatched() {
        let list = vec![("id-1".to_string(), "Phone A".to_string())];
        let matched = match_endpoint(&list, Some("Other Phone")).unwrap();
        assert_eq!(matched.0, "id-1");
    }

    #[test]
    fn parse_bluetoothctl_devices_extracts_mac_and_name() {
        let sample = "\
Device 11:22:33:44:55:66 Galaxy S24 Ultra
Device AA:BB:CC:DD:EE:FF Pixel 9 Pro
Invalid line
";
        let list = parse_bluetoothctl_devices(sample);
        assert_eq!(
            list,
            vec![
                ("11:22:33:44:55:66".to_string(), "Galaxy S24 Ultra".to_string()),
                ("AA:BB:CC:DD:EE:FF".to_string(), "Pixel 9 Pro".to_string()),
            ]
        );
    }

    #[test]
    fn parse_bluetoothctl_connected_detects_connection_state() {
        let connected_info = "\
Device AA:BB:CC:DD:EE:FF (public)
\tName: Pixel 9 Pro
\tPaired: yes
\tConnected: yes
";
        let disconnected_info = "\
Device AA:BB:CC:DD:EE:FF (public)
\tPaired: yes
\tConnected: no
";
        assert!(parse_bluetoothctl_connected(connected_info));
        assert!(!parse_bluetoothctl_connected(disconnected_info));
    }

    #[test]
    fn bluez_card_name_for_mac_replaces_colons_with_underscores() {
        assert_eq!(bluez_card_name_for_mac("AA:BB:CC:DD:EE:FF"), "bluez_card.AA_BB_CC_DD_EE_FF");
    }
}
