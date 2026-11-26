// SPDX-License-Identifier: GPL-3.0-or-later
//! This PC's own battery, for phones: a laptop's level and whether it
//! charges show on the PC's card, in the PC widget and on the Desk clock.
//! Read once a minute; the core passes it on only when it changes.

use std::time::Duration;

use nectarlink_core::Battery;

/// How often the battery is read.
const EVERY: Duration = Duration::from_secs(60);

/// Reads the battery now and then for as long as `node` runs.
pub fn spawn(node: nectarlink_core::Node) {
    crate::core_host::spawn(async move {
        loop {
            let battery = tokio::task::spawn_blocking(read).await.ok().flatten();
            if let Some(battery) = battery {
                node.update_battery(battery).await;
            }
            tokio::time::sleep(EVERY).await;
        }
    });
}

fn battery(level: u8, charging: bool, plugged: bool) -> Battery {
    Battery {
        level: level.min(100),
        charging,
        plugged: plugged.then(|| "ac".to_owned()),
        full_in: None,
        cell_bars: None,
        cell_type: None,
        wifi_bars: None,
    }
}

/// The battery, or `None` on a PC without one (or one that can't be read).
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn read() -> Option<Battery> {
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    let mut status = SYSTEM_POWER_STATUS::default();
    // SAFETY: status is a live, writable struct.
    unsafe { GetSystemPowerStatus(&mut status) }.ok()?;
    from_power_status(status.ACLineStatus, status.BatteryFlag, status.BatteryLifePercent)
}

/// Windows' `SYSTEM_POWER_STATUS` fields as a battery.
#[cfg(any(windows, test))]
fn from_power_status(ac_line: u8, flag: u8, percent: u8) -> Option<Battery> {
    // 128: no system battery; 255: unknown.
    if flag & 128 != 0 || flag == 255 || percent > 100 {
        return None;
    }
    let plugged = ac_line == 1;
    Some(battery(percent, flag & 8 != 0, plugged))
}

#[cfg(target_os = "linux")]
pub fn read() -> Option<Battery> {
    let mut supplies = Vec::new();
    for entry in std::fs::read_dir("/sys/class/power_supply").ok()?.flatten() {
        let path = entry.path();
        let file = |name: &str| std::fs::read_to_string(path.join(name)).ok().map(|s| s.trim().to_owned());
        supplies.push(Supply {
            kind: file("type").unwrap_or_default(),
            scope: file("scope").unwrap_or_default(),
            online: file("online").as_deref() == Some("1"),
            status: file("status").unwrap_or_default(),
            capacity: file("capacity").and_then(|c| c.parse().ok()),
            energy_now: file("energy_now").or_else(|| file("charge_now")).and_then(|c| c.parse().ok()),
            energy_full: file("energy_full").or_else(|| file("charge_full")).and_then(|c| c.parse().ok()),
        });
    }
    from_supplies(&supplies)
}

/// One entry of `/sys/class/power_supply`.
#[cfg(any(target_os = "linux", test))]
#[derive(Debug, Default)]
struct Supply {
    kind: String,
    /// "Device" for a mouse's or headset's battery, not the PC's.
    scope: String,
    online: bool,
    status: String,
    capacity: Option<u8>,
    energy_now: Option<u64>,
    energy_full: Option<u64>,
}

/// The PC's batteries together (some laptops have two), and whether it's plugged in.
#[cfg(any(target_os = "linux", test))]
fn from_supplies(supplies: &[Supply]) -> Option<Battery> {
    let batteries: Vec<&Supply> =
        supplies.iter().filter(|s| s.kind == "Battery" && !s.scope.eq_ignore_ascii_case("Device")).collect();
    if batteries.is_empty() {
        return None;
    }
    let (now, full) = batteries
        .iter()
        .fold((0u64, 0u64), |(n, f), b| (n + b.energy_now.unwrap_or(0), f + b.energy_full.unwrap_or(0)));
    let level = if batteries.len() > 1 && full > 0 {
        (now * 100 / full).min(100) as u8
    } else {
        batteries
            .iter()
            .find_map(|b| b.capacity)
            .or_else(|| (full > 0).then(|| (now * 100 / full).min(100) as u8))?
    };
    let charging = batteries.iter().any(|b| b.status == "Charging");
    let plugged = supplies.iter().any(|s| (s.kind == "Mains" || s.kind == "USB") && s.online)
        || charging
        || batteries.iter().all(|b| b.status == "Full" || b.status == "Not charging");
    Some(battery(level, charging, plugged))
}

#[cfg(target_os = "macos")]
pub fn read() -> Option<Battery> {
    let out = std::process::Command::new("pmset").args(["-g", "batt"]).output().ok()?;
    from_pmset(&String::from_utf8_lossy(&out.stdout))
}

/// `pmset -g batt`, as in:
/// ```text
/// Now drawing from 'AC Power'
///  -InternalBattery-0 (id=1234)  87%; charging; 1:02 remaining present: true
/// ```
#[cfg(any(target_os = "macos", test))]
fn from_pmset(text: &str) -> Option<Battery> {
    let line = text.lines().find(|l| l.contains("InternalBattery"))?;
    let percent = line.split('%').next()?.rsplit(|c: char| !c.is_ascii_digit()).next()?.parse::<u8>().ok()?;
    let state = line.split(';').nth(1).map(str::trim).unwrap_or_default();
    let plugged = text.contains("'AC Power'");
    Some(battery(percent, state == "charging", plugged))
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub fn read() -> Option<Battery> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_desktop_without_a_battery_sends_none() {
        assert!(from_power_status(1, 128, 255).is_none());
        assert!(from_power_status(255, 255, 255).is_none());
    }

    #[test]
    fn windows_laptop_charging_and_on_battery() {
        let b = from_power_status(1, 8 | 2, 64).unwrap();
        assert_eq!((b.level, b.charging, b.plugged.as_deref()), (64, true, Some("ac")));
        let b = from_power_status(0, 1, 87).unwrap();
        assert_eq!((b.level, b.charging, b.plugged), (87, false, None));
    }

    fn supply(kind: &str, status: &str, capacity: Option<u8>) -> Supply {
        Supply { kind: kind.into(), status: status.into(), capacity, ..Default::default() }
    }

    #[test]
    fn linux_mouse_battery_is_not_the_pcs() {
        let mouse = Supply { scope: "Device".into(), ..supply("Battery", "Discharging", Some(40)) };
        assert!(from_supplies(&[mouse, supply("Mains", "", None)]).is_none());
    }

    #[test]
    fn linux_laptop_on_battery_and_plugged_in() {
        let ac = |online| Supply { online, ..supply("Mains", "", None) };
        let b = from_supplies(&[supply("Battery", "Discharging", Some(73)), ac(false)]).unwrap();
        assert_eq!((b.level, b.charging, b.plugged), (73, false, None));
        let b = from_supplies(&[supply("Battery", "Charging", Some(20)), ac(true)]).unwrap();
        assert_eq!((b.level, b.charging, b.plugged.as_deref()), (20, true, Some("ac")));
        let b = from_supplies(&[supply("Battery", "Full", Some(100)), ac(true)]).unwrap();
        assert_eq!((b.level, b.charging), (100, false));
    }

    #[test]
    fn linux_two_batteries_count_together() {
        let bat = |now, full| Supply {
            energy_now: Some(now),
            energy_full: Some(full),
            ..supply("Battery", "Discharging", Some(99))
        };
        let b = from_supplies(&[bat(10, 40), bat(30, 40)]).unwrap();
        assert_eq!(b.level, 50);
    }

    #[test]
    fn pmset_output() {
        let charging = "Now drawing from 'AC Power'\n -InternalBattery-0 (id=4653155)\t87%; charging; 0:42 remaining present: true\n";
        let b = from_pmset(charging).unwrap();
        assert_eq!((b.level, b.charging, b.plugged.as_deref()), (87, true, Some("ac")));
        let unplugged = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=4653155)\t5%; discharging; 0:20 remaining present: true\n";
        let b = from_pmset(unplugged).unwrap();
        assert_eq!((b.level, b.charging, b.plugged), (5, false, None));
        assert!(from_pmset("Now drawing from 'AC Power'\n").is_none());
    }
}
