// SPDX-License-Identifier: GPL-3.0-or-later
//! Where paired devices can be dialled besides the local network: their
//! Tailscale addresses (read from the local Tailscale CLI) and an address the
//! user typed for a device ("connect by address").
//!
//! Addresses are the user's own data: shown in the app, never logged.

use std::{
    collections::BTreeSet,
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

use nectarlink_core::{DeviceId, LinkState, Node};

use crate::{core_host, tailscale};

/// The port NectarLink desktops prefer, and the one assumed for a peer whose
/// port isn't known yet.
pub const DEFAULT_PORT: u16 = 7718;

/// How long the user can type in a host name: a DNS name is at most 253 bytes.
const MAX_HOST_BYTES: usize = 253;

/// The periodic refresh while a paired device is offline starts here and
/// doubles up to [`MAX_REFRESH`].
const MIN_REFRESH: Duration = Duration::from_secs(30);
const MAX_REFRESH: Duration = Duration::from_secs(300);

/// A manual address as typed: a host (IP or DNS name) and maybe a port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualAddr {
    pub host: String,
    pub port: Option<u16>,
}

/// Why a typed address isn't usable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManualAddrError {
    Empty,
    Invalid,
    BadPort,
}

impl std::fmt::Display for ManualAddrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("Enter an IP address or hostname."),
            Self::Invalid => f.write_str("Enter a valid IPv4, IPv6, or Tailscale hostname."),
            Self::BadPort => f.write_str("Port must be between 1 and 65535."),
        }
    }
}

/// Parses `192.168.1.20`, `192.168.1.20:7718`, `fd7a::1`, `[fd7a::1]:7718`,
/// `phone.tail1234.ts.net` or `phone:7718`.
pub fn parse_manual_addr(text: &str) -> Result<ManualAddr, ManualAddrError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(ManualAddrError::Empty);
    }
    if text.len() > MAX_HOST_BYTES + 8 || text.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(ManualAddrError::Invalid);
    }
    let parse_port =
        |port: &str| port.parse::<u16>().ok().filter(|p| *p != 0).ok_or(ManualAddrError::BadPort);
    if let Some(rest) = text.strip_prefix('[') {
        let (host, after) = rest.split_once(']').ok_or(ManualAddrError::Invalid)?;
        let ip: std::net::Ipv6Addr = host.parse().map_err(|_| ManualAddrError::Invalid)?;
        let port = match after {
            "" => None,
            _ => Some(parse_port(after.strip_prefix(':').ok_or(ManualAddrError::Invalid)?)?),
        };
        return Ok(ManualAddr { host: ip.to_string(), port });
    }
    if let Ok(ip) = text.parse::<IpAddr>() {
        return Ok(ManualAddr { host: ip.to_string(), port: None });
    }
    let (host, port) = match text.rsplit_once(':') {
        Some((host, port)) => (host, Some(parse_port(port)?)),
        None => (text, None),
    };
    if let Ok(ip) = host.parse::<std::net::Ipv4Addr>() {
        return Ok(ManualAddr { host: ip.to_string(), port });
    }
    let host = host.trim_end_matches('.');
    let label_ok = |label: &str| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    };
    if host.is_empty() || host.len() > MAX_HOST_BYTES || !host.split('.').all(label_ok) {
        return Err(ManualAddrError::Invalid);
    }
    Ok(ManualAddr { host: host.to_ascii_lowercase(), port })
}

impl ManualAddr {
    /// Both A and AAAA records (or the literal IP), at the typed port or `default_port`.
    /// Blocking: a DNS name is looked up.
    pub fn resolve(&self, default_port: u16) -> Vec<SocketAddr> {
        let port = self.port.unwrap_or(default_port);
        let mut found: Vec<SocketAddr> = match self.host.parse::<IpAddr>() {
            Ok(ip) => vec![SocketAddr::new(ip, port)],
            Err(_) => (self.host.as_str(), port).to_socket_addrs().map(Iterator::collect).unwrap_or_default(),
        };
        found.retain(|a| {
            let ip = a.ip().to_canonical();
            !ip.is_loopback() && !ip.is_unspecified() && !ip.is_multicast()
        });
        let mut seen = BTreeSet::new();
        found.retain(|a| seen.insert(*a));
        // The core keeps at most eight.
        found.truncate(8);
        found
    }
}

/// The port a paired device was last reached on, from its stored addresses,
/// or [`DEFAULT_PORT`].
fn known_port(node: &Node, peer: &DeviceId) -> u16 {
    node.peer_addr_diagnostics(peer)
        .ok()
        .and_then(|diag| {
            diag.candidates
                .iter()
                .filter(|c| c.source != nectarlink_core::CandidateSource::Manual)
                .max_by_key(|c| (c.selected, c.last_seen))
                .map(|c| c.addr.port())
        })
        .filter(|p| *p != 0)
        .unwrap_or(DEFAULT_PORT)
}

/// Resolves and applies the manual address saved for `peer` (blocking).
/// `None` or an empty string clears it.
pub fn apply_manual(node: &Node, peer: DeviceId, text: Option<&str>) -> Result<usize, ManualAddrError> {
    let Some(text) = text.filter(|t| !t.trim().is_empty()) else {
        let _ = node.set_manual_addrs(peer, &[]);
        return Ok(0);
    };
    let parsed = parse_manual_addr(text)?;
    let addrs = parsed.resolve(known_port(node, &peer));
    if let Err(e) = node.set_manual_addrs(peer, &addrs) {
        tracing::debug!(peer = %peer.short(), error = %e, "can't set a manual address");
        return Err(ManualAddrError::Invalid);
    }
    Ok(addrs.len())
}

/// Saves `text` as the manual address of `peer` and applies it in the
/// background. An empty `text` clears it.
pub fn set_manual(peer: DeviceId, text: &str) -> Result<(), ManualAddrError> {
    let text = text.trim();
    if !text.is_empty() {
        parse_manual_addr(text)?;
    }
    crate::settings::set_manual_addr(&peer.to_string(), (!text.is_empty()).then(|| text.to_owned()));
    let saved = crate::settings::manual_addrs();
    let dir = core_host::host().data_dir.clone();
    let owned = (!text.is_empty()).then(|| text.to_owned());
    core_host::spawn(async move {
        let _ = tokio::task::spawn_blocking(move || {
            if let Err(e) = crate::settings::Settings::update(&dir, |s| s.manual_addrs = saved) {
                tracing::warn!(error = %e, "can't save a manual address");
            }
            if let Some(node) = core_host::node() {
                let _ = apply_manual(&node, peer, owned.as_deref());
            }
        })
        .await;
    });
    Ok(())
}

/// Resolves every saved manual address again (on start and on a route
/// change, since a DNS name can move).
pub fn reapply_all_manual(node: &Node) {
    let paired: BTreeSet<DeviceId> =
        node.paired_devices().unwrap_or_default().into_iter().map(|d| d.id).collect();
    for (id, text) in crate::settings::manual_addrs() {
        let Ok(peer) = id.parse::<DeviceId>() else { continue };
        if paired.contains(&peer) {
            let _ = apply_manual(node, peer, Some(&text));
        }
    }
}

/// What Tailscale said last, for the Doctor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TailscaleFacts {
    /// The CLI answered.
    pub running: bool,
    /// This device's tailnet addresses that one of its interfaces has.
    pub local_ips: Vec<IpAddr>,
    /// How many paired devices were found on the tailnet.
    pub matched_peers: usize,
}

static LAST: Mutex<Option<TailscaleFacts>> = Mutex::new(None);

/// The last Tailscale refresh's outcome, if one ran.
pub fn tailscale_facts() -> Option<TailscaleFacts> {
    LAST.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// This PC's own interface addresses.
fn local_interface_ips() -> Vec<IpAddr> {
    crate::win::wol::interface_ips()
}

/// A device name as a host name label: `Kia's Pixel 8` → `kias-pixel-8`.
fn host_label(name: &str) -> String {
    let mut label = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            label.push(c.to_ascii_lowercase());
        } else if (c.is_whitespace() || c == '-' || c == '_') && !label.ends_with('-') {
            label.push('-');
        }
    }
    label.trim_matches('-').to_owned()
}

/// The Tailscale addresses that plausibly belong to one paired device: a node
/// whose tailnet address the device has used before, or whose host name is
/// the device's name.
fn tailnet_addrs_for(
    inventory: &tailscale::TailscaleInventory,
    name: &str,
    known_ips: &BTreeSet<IpAddr>,
    port: u16,
) -> Vec<SocketAddr> {
    let label = host_label(name);
    let mut out = Vec::new();
    for node in inventory.peers.values() {
        let by_ip = node.tailscale_ips.iter().any(|ip| known_ips.contains(&ip.to_canonical()));
        let by_name = !label.is_empty()
            && (node.host_name.as_deref().is_some_and(|h| host_label(h) == label)
                || node
                    .dns_name
                    .as_deref()
                    .and_then(|d| d.split('.').next())
                    .is_some_and(|first| host_label(first) == label));
        if by_ip || by_name {
            out.extend(
                node.tailscale_ips
                    .iter()
                    .filter(|ip| tailscale::is_tailscale_ip(**ip))
                    .map(|ip| SocketAddr::new(*ip, port)),
            );
        }
    }
    out.sort();
    out.dedup();
    out.truncate(8);
    out
}

/// Reads Tailscale's status and tells the core this PC's tailnet addresses and
/// the ones its paired devices plausibly have (blocking: runs the CLI).
pub fn refresh_tailscale_blocking(node: &Node) -> TailscaleFacts {
    let inventory = match tailscale::discover() {
        Ok(inventory) => inventory,
        Err(e) => {
            tracing::debug!(error = %e, "Tailscale status unavailable");
            let facts = TailscaleFacts::default();
            *LAST.lock().unwrap_or_else(|e| e.into_inner()) = Some(facts.clone());
            core_host::host().runtime.block_on(node.set_local_tailscale_ips(Vec::new()));
            return facts;
        }
    };
    let local: Vec<IpAddr> =
        tailscale::correlate_local_tailscale_ips(&inventory, local_interface_ips()).into_iter().collect();
    core_host::host().runtime.block_on(node.set_local_tailscale_ips(local.clone()));
    let mut matched = 0;
    if !local.is_empty() {
        for device in node.paired_devices().unwrap_or_default() {
            let diag = node.peer_addr_diagnostics(&device.id).ok();
            let known_ips: BTreeSet<IpAddr> =
                diag.iter().flat_map(|d| d.candidates.iter().map(|c| c.addr.ip().to_canonical())).collect();
            let port = known_port(node, &device.id);
            let addrs = tailnet_addrs_for(&inventory, &device.info.name, &known_ips, port);
            if !addrs.is_empty() && node.add_tailscale_candidates(device.id, &addrs).unwrap_or(0) > 0 {
                matched += 1;
            }
        }
    }
    let facts = TailscaleFacts { running: true, local_ips: local, matched_peers: matched };
    *LAST.lock().unwrap_or_else(|e| e.into_inner()) = Some(facts.clone());
    facts
}

static REFRESHING: AtomicBool = AtomicBool::new(false);

/// Refreshes Tailscale and manual addresses in the background: on start, a
/// network or route change, a resume, or when asked. Overlapping asks run once.
pub fn refresh() {
    if REFRESHING.swap(true, Ordering::AcqRel) {
        return;
    }
    core_host::spawn(async {
        let done = tokio::task::spawn_blocking(|| {
            if let Some(node) = core_host::node() {
                refresh_tailscale_blocking(&node);
                reapply_all_manual(&node);
            }
        })
        .await;
        REFRESHING.store(false, Ordering::Release);
        if done.is_err() {
            tracing::warn!("Tailscale refresh failed");
        }
    });
}

fn any_paired_offline() -> bool {
    core_host::host().hub.read(|s| s.devices.iter().any(|d| !matches!(d.link, LinkState::Online { .. })))
}

/// Refreshes once now, then periodically while a paired device is offline
/// (backing off to every five minutes), and after the PC wakes from sleep.
pub fn start() {
    refresh();
    core_host::spawn(async {
        let mut wait = MIN_REFRESH;
        loop {
            let (mono, wall) = (Instant::now(), SystemTime::now());
            tokio::time::sleep(wait).await;
            // A wall clock that jumped past the monotonic one: the PC slept.
            let slept =
                wall.elapsed().unwrap_or_default().saturating_sub(mono.elapsed()) > Duration::from_secs(5);
            if slept {
                wait = MIN_REFRESH;
                refresh();
            } else if any_paired_offline() {
                refresh();
                wait = (wait * 2).min(MAX_REFRESH);
            } else {
                wait = MIN_REFRESH;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ips_bracketed_ipv6_and_host_names_with_ports() {
        let ok = |text: &str, host: &str, port: Option<u16>| {
            assert_eq!(parse_manual_addr(text), Ok(ManualAddr { host: host.into(), port }), "{text}");
        };
        ok("192.168.1.20", "192.168.1.20", None);
        ok(" 100.82.1.3:7718 ", "100.82.1.3", Some(7718));
        ok("fd7a:115c:a1e0::2", "fd7a:115c:a1e0::2", None);
        ok("[fd7a:115c:a1e0::2]", "fd7a:115c:a1e0::2", None);
        ok("[fd7a:115c:a1e0::2]:41641", "fd7a:115c:a1e0::2", Some(41641));
        ok("Phone.Tail1234.ts.net.", "phone.tail1234.ts.net", None);
        ok("phone:9000", "phone", Some(9000));
        assert_eq!(parse_manual_addr("  "), Err(ManualAddrError::Empty));
        assert_eq!(parse_manual_addr("phone:0"), Err(ManualAddrError::BadPort));
        assert_eq!(parse_manual_addr("phone:99999"), Err(ManualAddrError::BadPort));
        assert_eq!(parse_manual_addr("[fd7a::1"), Err(ManualAddrError::Invalid));
        assert_eq!(parse_manual_addr("[fd7a::1]7718"), Err(ManualAddrError::Invalid));
        assert_eq!(parse_manual_addr("bad host"), Err(ManualAddrError::Invalid));
        assert_eq!(parse_manual_addr("-bad.example"), Err(ManualAddrError::Invalid));
        assert_eq!(parse_manual_addr("a..b"), Err(ManualAddrError::Invalid));
    }

    #[test]
    fn literal_addresses_resolve_without_dns_and_skip_loopback() {
        let addr = parse_manual_addr("100.82.1.3").unwrap();
        assert_eq!(addr.resolve(41641), ["100.82.1.3:41641".parse().unwrap()]);
        let addr = parse_manual_addr("[fd7a:115c:a1e0::2]:9").unwrap();
        assert_eq!(addr.resolve(7718), ["[fd7a:115c:a1e0::2]:9".parse().unwrap()]);
        assert!(parse_manual_addr("127.0.0.1").unwrap().resolve(7718).is_empty());
    }

    #[test]
    fn matches_tailnet_nodes_by_known_address_or_host_name() {
        let inventory = tailscale::parse_status_json(
            r#"{"Self": {"TailscaleIPs": ["100.65.1.2"]},
                "Peer": {
                  "a": {"HostName": "kias-pixel", "TailscaleIPs": ["100.82.1.3", "fd7a:115c:a1e0::2"]},
                  "b": {"HostName": "laptop", "DNSName": "work-pc.example.ts.net.", "TailscaleIPs": ["100.99.2.1"]},
                  "c": {"HostName": "other", "TailscaleIPs": ["100.70.0.1", "192.168.1.9"]}
                }}"#,
        )
        .unwrap();
        let none = BTreeSet::new();
        assert_eq!(
            tailnet_addrs_for(&inventory, "Kia's Pixel", &none, 7718),
            ["100.82.1.3:7718".parse::<SocketAddr>().unwrap(), "[fd7a:115c:a1e0::2]:7718".parse().unwrap()]
        );
        assert_eq!(
            tailnet_addrs_for(&inventory, "Work PC", &none, 9000),
            ["100.99.2.1:9000".parse::<SocketAddr>().unwrap()]
        );
        let known = BTreeSet::from(["100.70.0.1".parse().unwrap()]);
        assert_eq!(
            tailnet_addrs_for(&inventory, "Renamed", &known, 1),
            ["100.70.0.1:1".parse::<SocketAddr>().unwrap()],
            "only tailnet addresses are kept"
        );
        assert!(tailnet_addrs_for(&inventory, "Unknown", &none, 7718).is_empty());
    }
}
