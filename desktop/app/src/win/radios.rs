// SPDX-License-Identifier: GPL-3.0-or-later
//! This PC's Wi-Fi and Bluetooth radios, for phones (#636): whether each is
//! on, and turning them on or off, through `Windows.Devices.Radios` (the
//! same switches as Windows' quick settings and airplane mode).

#![allow(unsafe_code)]

use windows::{
    Devices::Radios::{Radio, RadioAccessStatus, RadioKind, RadioState},
    Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize},
};

/// COM on the calling thread for as long as it lives (WinRT needs it).
struct Com(bool);

impl Com {
    fn new() -> Self {
        // SAFETY: COM for this thread, undone on drop if this call did it.
        Com(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok())
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: balances a successful CoInitializeEx on this thread.
            unsafe { CoUninitialize() };
        }
    }
}

fn radios() -> Vec<Radio> {
    Radio::GetRadiosAsync()
        .and_then(|op| op.join())
        .map(|list| list.into_iter().collect())
        .unwrap_or_default()
}

/// Whether a radio of `kind` is on: `None` with no such radio, or one whose
/// state Windows doesn't know.
fn state_of(radios: &[Radio], kind: RadioKind) -> Option<bool> {
    let states: Vec<RadioState> =
        radios.iter().filter(|r| r.Kind().is_ok_and(|k| k == kind)).filter_map(|r| r.State().ok()).collect();
    if states.is_empty() {
        return None;
    }
    if states.contains(&RadioState::On) {
        return Some(true);
    }
    // Off, or Disabled: off in hardware or by airplane mode.
    states.iter().any(|s| *s == RadioState::Off || *s == RadioState::Disabled).then_some(false)
}

/// Whether Wi-Fi and Bluetooth are on, each `None` where this PC has no such
/// radio. Blocks a moment.
pub fn read() -> (Option<bool>, Option<bool>) {
    let _com = Com::new();
    let radios = radios();
    (state_of(&radios, RadioKind::WiFi), state_of(&radios, RadioKind::Bluetooth))
}

/// Whether this PC has a radio NectarLink can report.
pub fn available() -> bool {
    let (wifi, bluetooth) = read();
    wifi.is_some() || bluetooth.is_some()
}

/// Turns Wi-Fi and/or Bluetooth on or off. Blocks a moment.
pub fn set(wifi: Option<bool>, bluetooth: Option<bool>) -> Result<(), String> {
    let _com = Com::new();
    let access = Radio::RequestAccessAsync().and_then(|op| op.join()).map_err(|e| e.to_string())?;
    if access != RadioAccessStatus::Allowed {
        return Err(format!("Windows doesn't let apps switch radios ({access:?})"));
    }
    let radios = radios();
    for (kind, on) in [(RadioKind::WiFi, wifi), (RadioKind::Bluetooth, bluetooth)] {
        let Some(on) = on else { continue };
        let mut found = false;
        for radio in radios.iter().filter(|r| r.Kind().is_ok_and(|k| k == kind)) {
            found = true;
            let target = if on { RadioState::On } else { RadioState::Off };
            let status = radio.SetStateAsync(target).and_then(|op| op.join()).map_err(|e| e.to_string())?;
            if status != RadioAccessStatus::Allowed {
                return Err(format!("Windows refused switching the radio ({status:?})"));
            }
        }
        if !found {
            return Err("this PC has no such radio".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    /// Reads this PC's radios through Windows; a PC without any gets `None`s.
    #[test]
    fn reads_this_pcs_radios() {
        let (wifi, bluetooth) = super::read();
        eprintln!("wifi {wifi:?} bluetooth {bluetooth:?}");
        assert_eq!(super::available(), wifi.is_some() || bluetooth.is_some());
    }
}
