// SPDX-License-Identifier: GPL-3.0-or-later
use std::collections::HashMap;
use std::process::{Child, Command, Stdio};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use nectarlink_core::DeviceId;

pub const COMPANY_ID: u16 = 0xFFFF;
pub const MAGIC: &[u8; 5] = b"NCTR\x01";
pub const NEAR_RSSI_DBM: i16 = -72;
pub const WALK_AWAY_RSSI_DBM: i16 = -85;

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const POLL_STEP: Duration = Duration::from_millis(100);

static RUNNING: AtomicBool = AtomicBool::new(false);
static WORKER: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

#[derive(Debug, Default)]
pub struct RssiTracker {
    peers: HashMap<[u8; 8], PeerRssiState>,
}

#[derive(Debug, Clone, Default)]
struct PeerRssiState {
    samples: [i16; 4],
    count: usize,
    idx: usize,
    armed_near: bool,
}

impl RssiTracker {
    pub fn record(&mut self, prefix: [u8; 8], rssi: i16) -> bool {
        let state = self.peers.entry(prefix).or_default();
        state.samples[state.idx] = rssi;
        state.idx = (state.idx + 1) % state.samples.len();
        state.count = (state.count + 1).min(state.samples.len());
        if state.count < state.samples.len() {
            return false;
        }
        let sum: i32 = state.samples.iter().map(|&v| i32::from(v)).sum();
        let avg = (sum / (state.samples.len() as i32)) as i16;
        if avg >= NEAR_RSSI_DBM {
            state.armed_near = true;
            false
        } else if state.armed_near && avg <= WALK_AWAY_RSSI_DBM {
            state.armed_near = false;
            true
        } else {
            false
        }
    }
}

pub fn build_manufacturer_payload(device_id: &DeviceId) -> [u8; 13] {
    let mut out = [0u8; 13];
    out[0..5].copy_from_slice(MAGIC);
    out[5..13].copy_from_slice(&device_id.as_bytes()[0..8]);
    out
}

/// Validates the `NCTR\x01` magic header and extracts the peer device ID prefix.
/// Accepts either the 13-byte payload (5-byte magic + 8-byte prefix) or a 12-byte
/// payload (5-byte magic + 7-byte prefix, zero-padded in the 8th byte).
pub fn parse_manufacturer_payload(bytes: &[u8]) -> Option<[u8; 8]> {
    if bytes.len() < 12 || &bytes[0..5] != MAGIC {
        return None;
    }
    let mut prefix = [0u8; 8];
    if bytes.len() >= 13 {
        prefix.copy_from_slice(&bytes[5..13]);
    } else {
        prefix[0..7].copy_from_slice(&bytes[5..12]);
    }
    Some(prefix)
}

pub fn start(device_id: DeviceId) {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let handle = thread::Builder::new().name("ble-beacon".into()).spawn(move || run_worker(device_id)).ok();
    if let Ok(mut guard) = WORKER.lock() {
        *guard = handle;
    }
}

pub fn stop() {
    RUNNING.store(false, Ordering::SeqCst);
    let handle = WORKER.lock().ok().and_then(|mut guard| guard.take());
    if let Some(h) = handle {
        let _ = h.join();
    }
}

fn walk_away_lock_enabled() -> bool {
    let rules = crate::settings::automation_rules();
    !crate::automations::matching_rules(&rules, crate::automations::TRIGGER_PHONE_WALKED_AWAY).is_empty()
}

fn start_bluetoothctl_advertisement(payload: &[u8; 13]) -> Option<Child> {
    let hex_bytes = payload.iter().map(|b| format!("0x{b:02x}")).collect::<Vec<_>>().join(" ");
    let mut child = Command::new("bluetoothctl")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        let script =
            format!("advertise.manufacturer 0xffff {hex_bytes}\nadvertise.name off\nadvertise on\nscan on\n");
        let _ = stdin.write_all(script.as_bytes());
        let _ = stdin.flush();
        child.stdin = Some(stdin);
    }
    Some(child)
}

fn stop_bluetoothctl_advertisement(child: Option<Child>) {
    let Some(mut child) = child else { return };
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        let _ = stdin.write_all(b"scan off\nadvertise off\nquit\n");
        let _ = stdin.flush();
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn is_adapter_powered() -> bool {
    if let Ok(out) = Command::new("busctl")
        .args(["--system", "get-property", "org.bluez", "/org/bluez/hci0", "org.bluez.Adapter1", "Powered"])
        .output()
        && out.status.success()
    {
        let stdout = String::from_utf8_lossy(&out.stdout);
        return stdout.trim().ends_with("true");
    }
    if let Ok(out) = Command::new("bluetoothctl").arg("show").output()
        && out.status.success()
    {
        let stdout = String::from_utf8_lossy(&out.stdout);
        return stdout.lines().any(|l| {
            l.trim().strip_prefix("Powered:").is_some_and(|v| v.trim().eq_ignore_ascii_case("yes"))
        });
    }
    false
}

fn fallback_peer_prefix(id_str: &str) -> [u8; 8] {
    let mut out = [0u8; 8];
    let bytes = id_str.as_bytes();
    let start = bytes.len().saturating_sub(8);
    for (dst, &src) in out.iter_mut().zip(&bytes[start..]) {
        *dst = src;
    }
    out
}

fn parse_rssi_from_info(info: &str) -> Option<i16> {
    for line in info.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("RSSI:") else {
            continue;
        };
        let rest = rest.trim();
        if let Some(open) = rest.find('(')
            && let Some(close) = rest[open + 1..].find(')')
            && let Ok(val) = rest[open + 1..open + 1 + close].trim().parse::<i16>()
        {
            return Some(val);
        }
        if let Some(tok) = rest.split_whitespace().next()
            && let Ok(val) = tok.parse::<i16>()
        {
            return Some(val);
        }
    }
    None
}

fn parse_manufacturer_from_info(info: &str) -> Option<Vec<u8>> {
    let mut in_ffff_block = false;
    let mut collected = Vec::new();
    for line in info.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("ManufacturerData Key:") {
            if !collected.is_empty() {
                break;
            }
            let lower = rest.to_ascii_lowercase();
            in_ffff_block = lower.contains("0xffff") || lower.contains("65535");
            continue;
        }
        if in_ffff_block && let Some(rest) = trimmed.strip_prefix("ManufacturerData Value:") {
            for tok in rest.split_whitespace() {
                let hex = tok.strip_prefix("0x").unwrap_or(tok);
                if hex.len() == 2
                    && let Ok(b) = u8::from_str_radix(hex, 16)
                {
                    collected.push(b);
                } else {
                    break;
                }
            }
        }
    }
    if collected.is_empty() { None } else { Some(collected) }
}

fn poll_bluez_devices(own_prefix: &[u8; 8], tracker: &mut RssiTracker) {
    let Ok(out) = Command::new("bluetoothctl").arg("devices").output() else {
        return;
    };
    if !out.status.success() {
        return;
    }
    let devices = super::bt_hfp::parse_bluetoothctl_devices(&String::from_utf8_lossy(&out.stdout));
    for (mac, _) in devices {
        let Ok(info_out) = Command::new("bluetoothctl").args(["info", &mac]).output() else {
            continue;
        };
        if !info_out.status.success() {
            continue;
        }
        let info = String::from_utf8_lossy(&info_out.stdout);
        let Some(rssi) = parse_rssi_from_info(&info) else {
            continue;
        };
        let connected = super::bt_hfp::parse_bluetoothctl_connected(&info);
        let paired = info
            .lines()
            .any(|l| l.trim().strip_prefix("Paired:").is_some_and(|v| v.trim().eq_ignore_ascii_case("yes")));

        let peer_prefix = if let Some(mfg_bytes) = parse_manufacturer_from_info(&info)
            && let Some(prefix) = parse_manufacturer_payload(&mfg_bytes)
        {
            if &prefix == own_prefix {
                continue;
            }
            Some(prefix)
        } else if connected && paired && walk_away_lock_enabled() {
            Some(fallback_peer_prefix(&mac))
        } else {
            None
        };

        if let Some(prefix) = peer_prefix
            && tracker.record(prefix, rssi)
        {
            tracing::info!("BLE walk-away threshold reached; locking workstation and firing trigger");
            let _ = super::shell::lock_workstation();
            crate::automations::fire_trigger(crate::automations::TRIGGER_PHONE_WALKED_AWAY);
        }
    }
}

fn run_worker(device_id: DeviceId) {
    if !is_adapter_powered() {
        tracing::debug!("BLE beacon: /org/bluez/hci0 unavailable or not powered; exiting worker");
        RUNNING.store(false, Ordering::SeqCst);
        return;
    }

    let payload = build_manufacturer_payload(&device_id);
    let mut own_prefix = [0u8; 8];
    own_prefix.copy_from_slice(&device_id.as_bytes()[0..8]);

    let adv_child = start_bluetoothctl_advertisement(&payload);
    let mut tracker = RssiTracker::default();

    while RUNNING.load(Ordering::Relaxed) {
        poll_bluez_devices(&own_prefix, &mut tracker);

        let steps = (POLL_INTERVAL.as_millis() / POLL_STEP.as_millis()).max(1);
        for _ in 0..steps {
            if !RUNNING.load(Ordering::Relaxed) {
                break;
            }
            thread::sleep(POLL_STEP);
        }
    }

    stop_bluetoothctl_advertisement(adv_child);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_13_byte_manufacturer_payload_with_magic_and_id_prefix() {
        let mut raw = [0u8; 32];
        for (i, b) in raw.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(7).wrapping_add(3);
        }
        let id = DeviceId(raw);
        let payload = build_manufacturer_payload(&id);
        assert_eq!(payload.len(), 13);
        assert_eq!(&payload[0..5], b"NCTR\x01");
        assert_eq!(&payload[5..13], &raw[0..8]);
        assert_eq!(COMPANY_ID, 0xFFFF);
    }

    #[test]
    fn manufacturer_payload_roundtrips_through_parser() {
        let mut raw = [0u8; 32];
        for (i, b) in raw.iter_mut().enumerate() {
            *b = (i as u8).wrapping_add(0x10);
        }
        let id = DeviceId(raw);
        let payload = build_manufacturer_payload(&id);
        let parsed = parse_manufacturer_payload(&payload).expect("valid payload should parse");
        assert_eq!(&parsed[..], &raw[0..8]);

        // Also supports 12-byte payloads (5-byte magic + 7-byte prefix).
        let parsed_12 = parse_manufacturer_payload(&payload[0..12]).expect("12-byte payload should parse");
        assert_eq!(&parsed_12[0..7], &raw[0..7]);
        assert_eq!(parsed_12[7], 0);
    }

    #[test]
    fn parser_rejects_short_or_wrong_magic_payloads() {
        assert_eq!(parse_manufacturer_payload(b""), None);
        assert_eq!(parse_manufacturer_payload(b"NCTR\x01short"), None);
        assert_eq!(parse_manufacturer_payload(b"NCTR\x0212345678"), None);
        assert_eq!(parse_manufacturer_payload(b"XXXX\x0112345678"), None);
    }

    #[test]
    fn rssi_tracker_fires_walk_away_only_after_armed_near() {
        let mut tracker = RssiTracker::default();
        let peer = [1, 2, 3, 4, 5, 6, 7, 8];
        for _ in 0..4 {
            assert!(!tracker.record(peer, -90));
        }
        for _ in 0..4 {
            assert!(!tracker.record(peer, -65));
        }
        assert!(!tracker.record(peer, -88));
        assert!(!tracker.record(peer, -90));
        assert!(!tracker.record(peer, -92));
        assert!(tracker.record(peer, -90));
        assert!(!tracker.record(peer, -92));
    }

    #[test]
    fn rssi_tracker_arms_at_near_threshold_and_fires_once_on_walk_away() {
        let mut tracker = RssiTracker::default();
        let peer = [9, 8, 7, 6, 5, 4, 3, 2];
        // Exactly at NEAR_RSSI_DBM (-72) for 4 samples arms the tracker.
        for _ in 0..4 {
            assert!(!tracker.record(peer, NEAR_RSSI_DBM));
        }
        // Dropping to WALK_AWAY_RSSI_DBM (-85) across 4 samples triggers once.
        assert!(!tracker.record(peer, WALK_AWAY_RSSI_DBM));
        assert!(!tracker.record(peer, WALK_AWAY_RSSI_DBM));
        assert!(!tracker.record(peer, WALK_AWAY_RSSI_DBM));
        assert!(tracker.record(peer, WALK_AWAY_RSSI_DBM));
        // Subsequent distant samples do not re-trigger without re-arming near.
        for _ in 0..4 {
            assert!(!tracker.record(peer, -95));
        }
    }

    #[test]
    fn start_and_stop_never_panic_when_ble_radio_is_absent_or_present() {
        let id = DeviceId([0x42; 32]);
        start(id);
        stop();
    }

    #[test]
    fn start_and_stop_are_idempotent_without_bluez() {
        let id = DeviceId([0x24; 32]);
        start(id);
        start(id);
        stop();
        stop();
    }
}
