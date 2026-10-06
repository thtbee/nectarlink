// SPDX-License-Identifier: GPL-3.0-or-later
//! The Connection Doctor: why a phone can't reach this PC, in plain words,
//! and a button to fix what can be fixed from here (the firewall, the
//! network's profile, reconnecting).
//!
//! Checks run on a worker thread (they ask Windows a few slow questions)
//! and come back as a list for the UI.

use nectarlink_core::{ConnectionPath, LinkState};
use serde::Serialize;

use crate::{core_host, win::doctor as windows_checks};

/// How a check came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Ok,
    Warn,
    Fail,
}

/// One finding, as the UI shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    pub id: String,
    pub outcome: Outcome,
    pub title: String,
    pub detail: String,
    /// What the fix button does (see [`fix`]), if there's one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<&'static str>,
}

/// What Windows said, gathered first so the wording can be tested.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub local_address: bool,
    pub vpn: bool,
    /// Connected networks: name, and whether it's private or domain.
    pub networks: Vec<(String, bool)>,
    /// `None` when the firewall couldn't be read.
    pub firewall: Option<windows_checks::Firewall>,
    /// Paired devices: name and link.
    pub devices: Vec<(String, LinkState)>,
}

/// Asks Windows and the core (blocking; call on a worker thread).
pub fn gather() -> Facts {
    let adapters = windows_checks::adapters();
    let networks = windows_checks::networks()
        .map(|list| list.into_iter().map(|n| (network_name(&n.name), n.trusted)).collect())
        .unwrap_or_default();
    let firewall = std::env::current_exe().ok().and_then(|exe| windows_checks::firewall(&exe).ok());
    let devices = core_host::host()
        .hub
        .read(|s| s.devices.iter().map(|d| (d.info.name.clone(), d.link.clone())).collect());
    Facts { local_address: adapters.local_address, vpn: adapters.vpn, networks, firewall, devices }
}

/// A network's name as people say it. While Windows is still identifying
/// a network it calls it "Loading..." (or "Identifying..."), which says
/// nothing.
fn network_name(name: &str) -> String {
    let name = name.trim();
    if name.is_empty() || name.ends_with("...") || name.ends_with('…') {
        "This network".into()
    } else {
        name.to_owned()
    }
}

fn check(
    id: &str,
    outcome: Outcome,
    title: impl Into<String>,
    detail: impl Into<String>,
    fix: Option<&'static str>,
) -> Check {
    Check { id: id.into(), outcome, title: title.into(), detail: detail.into(), fix }
}

/// The findings, most important first.
pub fn diagnose(facts: &Facts, now_secs: i64) -> Vec<Check> {
    let mut checks = Vec::new();

    // The network itself.
    if !facts.local_address {
        checks.push(check(
            "network",
            Outcome::Fail,
            "This PC isn't on a network",
            "Connect it to the same Wi-Fi or wired network as your phone.",
            None,
        ));
    } else {
        let title = match facts.networks.first() {
            Some((name, _)) if name != "This network" => format!("Connected to {name}"),
            _ => "Connected to a network".into(),
        };
        checks.push(check("network", Outcome::Ok, title, "", None));
    }
    if let Some((name, _)) = facts.networks.iter().find(|(_, trusted)| !trusted) {
        checks.push(check(
            "profile",
            Outcome::Warn,
            format!("{name} is set as a public network"),
            "On public networks Windows keeps other devices from finding this PC. If it's your home or work \
             network, set it to Private.",
            Some("network-settings"),
        ));
    }

    // The firewall.
    match facts.firewall {
        None => {}
        Some(f) if !f.on => {
            checks.push(check("firewall", Outcome::Ok, "Windows Firewall is off for this network", "", None));
        }
        Some(f) if f.blocked => checks.push(check(
            "firewall",
            Outcome::Fail,
            "Windows Firewall blocks Nectarlink",
            "Someone chose not to allow it when Windows asked. Allow it, and your phone can connect.",
            Some("firewall"),
        )),
        Some(f) if !f.allowed => checks.push(check(
            "firewall",
            Outcome::Warn,
            "Nectarlink isn't allowed through Windows Firewall",
            "Your phone may not be able to connect. Allow it once here.",
            Some("firewall"),
        )),
        Some(_) => {
            checks.push(check("firewall", Outcome::Ok, "Windows Firewall lets your phone connect", "", None))
        }
    }

    if facts.vpn {
        checks.push(check(
            "vpn",
            Outcome::Warn,
            "A VPN is on",
            "VPNs often keep devices on your network from reaching this PC. If your phone can't connect, try \
             turning it off.",
            None,
        ));
    }

    // Each phone.
    for (i, (name, link)) in facts.devices.iter().enumerate() {
        let id = format!("device{i}");
        let finding = match link {
            LinkState::Online { path, rtt_ms } => {
                let how = match path {
                    ConnectionPath::Lan => "On this network",
                    _ => "While away",
                };
                check(&id, Outcome::Ok, format!("{name} is connected"), format!("{how} · {rtt_ms} ms"), None)
            }
            LinkState::Connecting => check(
                &id,
                Outcome::Warn,
                format!("Connecting to {name}…"),
                "This usually takes a few seconds.",
                None,
            ),
            LinkState::Offline { last_seen } => {
                let seen = last_seen.map_or_else(
                    || "It hasn't connected yet.".to_owned(),
                    |at| format!("Last seen {}.", ago(now_secs - at)),
                );
                check(
                    &id,
                    Outcome::Warn,
                    format!("{name} isn't connected"),
                    format!(
                        "{seen} Open Nectarlink on the phone and keep both on the same network. If the phone \
                         asks to run in the background, allow it."
                    ),
                    Some("reconnect"),
                )
            }
        };
        checks.push(finding);
    }

    // Failures first, then warnings, then what's fine (stable within each).
    checks.sort_by_key(|c| match c.outcome {
        Outcome::Fail => 0,
        Outcome::Warn => 1,
        Outcome::Ok => 2,
    });
    checks
}

fn ago(seconds: i64) -> String {
    let minutes = seconds / 60;
    match minutes {
        ..=0 => "just now".into(),
        1 => "a minute ago".into(),
        2..=59 => format!("{minutes} minutes ago"),
        60..=119 => "an hour ago".into(),
        120..=1439 => format!("{} hours ago", minutes / 60),
        _ => format!("{} days ago", minutes / 1440),
    }
}

/// Runs a fix (blocking: the firewall one waits for Windows' prompt).
/// Returns whether anything was done.
pub fn fix(id: &str) -> bool {
    match id {
        "firewall" => {
            let Ok(exe) = std::env::current_exe() else { return false };
            match windows_checks::fix_firewall(&exe) {
                Ok(done) => done,
                Err(e) => {
                    tracing::warn!(error = %e, "can't change the firewall");
                    false
                }
            }
        }
        "network-settings" => crate::win::shell::open_url("ms-settings:network-status").is_ok(),
        "reconnect" => {
            if let Some(node) = core_host::node() {
                core_host::spawn(async move { node.refresh().await });
            }
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> Facts {
        Facts {
            local_address: true,
            vpn: false,
            networks: vec![("Home".into(), true)],
            firewall: Some(windows_checks::Firewall { on: true, allowed: true, blocked: false }),
            devices: vec![("Pixel".into(), LinkState::Online { path: ConnectionPath::Lan, rtt_ms: 4 })],
        }
    }

    fn outcomes(checks: &[Check]) -> Vec<(&str, Outcome)> {
        checks.iter().map(|c| (c.id.as_str(), c.outcome)).collect()
    }

    #[test]
    fn all_well() {
        let checks = diagnose(&facts(), 0);
        assert!(checks.iter().all(|c| c.outcome == Outcome::Ok), "{checks:?}");
        assert_eq!(checks[0].title, "Connected to Home");
        assert_eq!(checks.last().unwrap().detail, "On this network · 4 ms");
    }

    #[test]
    fn problems_come_first_with_fixes() {
        let mut f = facts();
        f.networks = vec![("Cafe".into(), false)];
        f.firewall = Some(windows_checks::Firewall { on: true, allowed: true, blocked: true });
        f.vpn = true;
        f.devices = vec![("Pixel".into(), LinkState::Offline { last_seen: Some(1_000) })];
        let checks = diagnose(&f, 1_000 + 3 * 3600);
        assert_eq!(
            outcomes(&checks),
            [
                ("firewall", Outcome::Fail),
                ("profile", Outcome::Warn),
                ("vpn", Outcome::Warn),
                ("device0", Outcome::Warn),
                ("network", Outcome::Ok)
            ]
        );
        assert_eq!(checks[0].fix, Some("firewall"));
        assert_eq!(checks[1].fix, Some("network-settings"));
        assert!(checks[3].detail.starts_with("Last seen 3 hours ago."), "{}", checks[3].detail);
        assert_eq!(checks[3].fix, Some("reconnect"));
    }

    #[test]
    fn no_network_and_no_allow_rule() {
        let mut f = facts();
        f.local_address = false;
        f.firewall = Some(windows_checks::Firewall { on: true, allowed: false, blocked: false });
        let checks = diagnose(&f, 0);
        assert_eq!(checks[0].title, "This PC isn't on a network");
        assert_eq!(checks[1].outcome, Outcome::Warn);
        assert_eq!(checks[1].fix, Some("firewall"));
    }

    #[test]
    fn networks_being_identified_get_a_plain_name() {
        assert_eq!(network_name("Loading..."), "This network");
        assert_eq!(network_name("Identifying…"), "This network");
        assert_eq!(network_name("Home 5G"), "Home 5G");
        let mut f = facts();
        f.networks = vec![("This network".into(), false)];
        let checks = diagnose(&f, 0);
        assert_eq!(checks[0].title, "This network is set as a public network");
        let network = checks.iter().find(|c| c.id == "network").unwrap();
        assert_eq!(network.title, "Connected to a network");
    }

    #[test]
    fn times_read_naturally() {
        assert_eq!(ago(10), "just now");
        assert_eq!(ago(90), "a minute ago");
        assert_eq!(ago(600), "10 minutes ago");
        assert_eq!(ago(5400), "an hour ago");
        assert_eq!(ago(3 * 86400), "3 days ago");
    }
}
