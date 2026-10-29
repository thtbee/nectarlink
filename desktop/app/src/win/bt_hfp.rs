// SPDX-License-Identifier: GPL-3.0-or-later
//! Bluetooth call audio on Windows: there is none. Windows doesn't let apps be
//! a phone's hands-free unit (`docs/research/hfp.md`), so call audio stays on
//! the phone and the Calls page shows no "PC call audio" panel here (#86).

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HfpStatus {
    pub available: bool,
    pub connected: bool,
    pub device_name: String,
    /// Empty: nothing to show.
    pub detail: String,
}

pub fn probe_status(_phone_name: Option<&str>) -> HfpStatus {
    HfpStatus::default()
}

pub fn connect_audio(phone_name: Option<&str>) -> Result<HfpStatus, String> {
    Ok(probe_status(phone_name))
}

pub fn disconnect_audio() {}

/// Opens Windows Bluetooth & devices settings (`ms-settings:bluetooth`).
pub fn open_bluetooth_settings() {
    if let Err(e) = crate::win::shell::open_url("ms-settings:bluetooth") {
        tracing::warn!(error = %e, "can't open ms-settings:bluetooth");
    }
}
