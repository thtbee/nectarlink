// SPDX-License-Identifier: GPL-3.0-or-later
//! Bluetooth call audio on macOS: not yet. Call audio stays on the phone, and
//! the Calls page shows no "PC call audio" panel here.

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HfpStatus {
    pub available: bool,
    pub connected: bool,
    pub device_name: String,
    pub detail: String,
}

pub fn probe_status(_phone_name: Option<&str>) -> HfpStatus {
    HfpStatus::default()
}

pub fn connect_audio(phone_name: Option<&str>) -> Result<HfpStatus, String> {
    Ok(probe_status(phone_name))
}

pub fn disconnect_audio() {}

pub fn open_bluetooth_settings() {
    let _ = std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.BluetoothSettings")
        .spawn();
}
