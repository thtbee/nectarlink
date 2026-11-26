// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS connection doctor integrations.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Firewall {
    pub on: bool,
    pub allowed: bool,
    pub blocked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Network {
    pub name: String,
    pub trusted: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Adapters {
    pub local_address: bool,
    pub vpn: Option<String>,
}

pub fn firewall(_exe: &Path) -> Result<Firewall, String> {
    Err("firewall check is not supported on macOS".into())
}

pub fn networks() -> Result<Vec<Network>, String> {
    Err("network profile check is not supported on macOS".into())
}

pub fn adapters() -> Adapters {
    Adapters { local_address: true, vpn: None }
}

pub fn fix_firewall(_exe: &Path) -> Result<bool, String> {
    Err("automatic firewall configuration is not supported on macOS".into())
}

pub fn open_firewall_settings() -> bool {
    let _ = std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Firewall")
        .spawn();
    true
}

#[allow(dead_code)]
pub fn diagnose() -> Vec<(String, bool, String)> {
    Vec::new()
}
