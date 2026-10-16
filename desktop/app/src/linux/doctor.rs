use std::fs;
use std::io::Result;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Copy)]
pub struct Firewall {
    pub on: bool,
    pub allowed: bool,
    pub blocked: bool,
}

#[derive(Debug, Clone)]
pub struct Network {
    pub name: String,
    pub trusted: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct Adapters {
    pub local_address: bool,
    pub vpn: bool,
}

pub fn firewall(_exe: &Path) -> Result<Firewall> {
    Ok(Firewall {
        on: false,
        allowed: true,
        blocked: false,
    })
}

pub fn networks() -> Result<Vec<Network>> {
    let mut list = Vec::new();
    if let Ok(entries) = fs::read_dir("/sys/class/net") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == "lo" {
                continue;
            }
            let state = fs::read_to_string(entry.path().join("operstate"))
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            if state == "up" {
                list.push(Network {
                    name,
                    trusted: true,
                });
            }
        }
    }
    Ok(list)
}

pub fn adapters() -> Adapters {
    let mut local_address = false;
    let mut vpn = false;

    if let Ok(entries) = fs::read_dir("/sys/class/net") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if name == "lo" {
                continue;
            }
            let state = fs::read_to_string(entry.path().join("operstate"))
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            if state != "up" && state != "unknown" {
                continue;
            }
            if name.starts_with("tun")
                || name.starts_with("tap")
                || name.starts_with("wg")
                || name.starts_with("tailscale")
                || name.starts_with("nordlynx")
                || name.starts_with("ppp")
            {
                vpn = true;
            } else {
                local_address = true;
            }
        }
    }

    Adapters { local_address, vpn }
}

pub fn fix_firewall(_exe: &Path) -> Result<bool> {
    Ok(true)
}

pub fn open_firewall_settings() -> bool {
    Command::new("gufw")
        .spawn()
        .or_else(|_| Command::new("firewall-config").spawn())
        .is_ok()
}
