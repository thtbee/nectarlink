// SPDX-License-Identifier: GPL-3.0-or-later
//! The Connection Doctor: why a phone can't reach this PC, in plain words,
//! and a button to fix what can be fixed from here (the firewall, the
//! network's profile, reconnecting).
//!
//! Checks run on a worker thread (they ask the OS a few slow questions)
//! and come back as a list for the UI.

use crate::i18n::tr;
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

/// One paired device's connection state and discovery diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceFact {
    pub name: String,
    pub model: Option<String>,
    pub os: String,
    pub link: LinkState,
    pub mdns_seen_ago_secs: Option<u64>,
    /// The addresses it last announced over mDNS.
    pub mdns_addrs: Vec<std::net::SocketAddr>,
    pub dial_failures: u32,
}

/// Whether this PC can reach phones on a USB cable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdbFact {
    /// adb was found, in an SDK folder or on `PATH`.
    pub installed: bool,
    /// An adb server answers.
    pub running: bool,
}

/// What the OS said, gathered first so the wording can be tested.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub local_address: bool,
    pub vpn: bool,
    /// Connected networks: name, and whether it's private or domain.
    pub networks: Vec<(String, bool)>,
    /// `None` when the firewall couldn't be read.
    pub firewall: Option<windows_checks::Firewall>,
    /// Paired devices: link and discovery diagnostics.
    pub devices: Vec<DeviceFact>,
    /// adb, which phones on a USB cable connect through; `None` when USB is off.
    pub adb: Option<AdbFact>,
    /// Optional helper tools missing from `PATH` on Linux.
    #[cfg(target_os = "linux")]
    pub missing_tools: Vec<&'static str>,
    /// Linux-specific system readiness checks: `(id, title, ok, detail, fix)`.
    #[cfg(target_os = "linux")]
    pub linux_checks: Vec<(String, String, bool, String, Option<&'static str>)>,
}

/// Asks the OS and the core (blocking; call on a worker thread).
pub fn gather() -> Facts {
    let adapters = windows_checks::adapters();
    let networks = windows_checks::networks()
        .map(|list| list.into_iter().map(|n| (network_name(&n.name), n.trusted)).collect())
        .unwrap_or_default();
    let firewall = std::env::current_exe().ok().and_then(|exe| windows_checks::firewall(&exe).ok());
    let node = core_host::node();
    let devices = core_host::host().hub.read(|s| {
        s.devices
            .iter()
            .map(|d| {
                let diag = node.as_ref().map(|n| n.peer_diagnostics(&d.id)).unwrap_or_default();
                DeviceFact {
                    name: d.info.name.clone(),
                    model: d.info.model.clone(),
                    os: d.info.os.clone(),
                    link: d.link.clone(),
                    mdns_seen_ago_secs: diag.mdns_seen_ago_secs,
                    mdns_addrs: diag.mdns_addrs,
                    dial_failures: diag.dial_failures,
                }
            })
            .collect()
    });
    let adb = core_host::adb_enabled().then(|| AdbFact {
        installed: nectarlink_core::locate_adb().is_some(),
        running: nectarlink_core::adb_server_answers(nectarlink_core::DEFAULT_ADB_ADDR),
    });
    Facts {
        local_address: adapters.local_address,
        vpn: adapters.vpn,
        networks,
        firewall,
        devices,
        adb,
        #[cfg(target_os = "linux")]
        missing_tools: windows_checks::missing_tools(),
        #[cfg(target_os = "linux")]
        linux_checks: windows_checks::linux_checks(),
    }
}

/// A network's name as people say it. While Windows is still identifying
/// a network it calls it "Loading..." (or "Identifying..."), which says
/// nothing.
fn network_name(name: &str) -> String {
    let name = name.trim();
    if name.is_empty() || name.ends_with("...") || name.ends_with('…') {
        tr!("Doctor", "This network")
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

fn match_oem(raw: &str) -> Option<(&'static str, String)> {
    let lower = raw.to_ascii_lowercase();
    if lower.contains("samsung") || lower.contains("galaxy") || lower.contains("sm-") {
        Some((
            "Samsung",
            tr!(
                "Doctor",
                "On Samsung: open Settings → Apps → NectarLink → Battery and choose Unrestricted, then in \
             Device care → Battery → Background usage limits make sure NectarLink is never put to sleep."
            ),
        ))
    } else if lower.contains("xiaomi") || lower.contains("redmi") || lower.contains("poco") {
        Some((
            "Xiaomi",
            tr!(
                "Doctor",
                "On Xiaomi / HyperOS / MIUI: open Settings → Apps → Manage apps → NectarLink, turn on \
             Autostart, and set Battery saver to No restrictions."
            ),
        ))
    } else if lower.contains("oneplus") {
        Some((
            "OnePlus",
            tr!(
                "Doctor",
                "On OnePlus / OxygenOS: open Settings → Apps → App management → NectarLink → Battery usage and \
             turn on Allow background activity and Allow auto-launch."
            ),
        ))
    } else if lower.contains("oppo") || lower.contains("realme") || lower.contains("rmx") {
        Some((
            "Oppo / Realme",
            tr!(
                "Doctor",
                "On Oppo / Realme: open Settings → Apps → App management → NectarLink → Battery usage and turn \
             on Allow background activity and Allow auto-launch."
            ),
        ))
    } else if lower.contains("vivo") || lower.contains("iqoo") {
        Some((
            "Vivo",
            tr!(
                "Doctor",
                "On Vivo / OriginOS / Funtouch: open Settings → Battery → Background power consumption \
             management → NectarLink and choose Unrestricted, and allow Autostart."
            ),
        ))
    } else if lower.contains("huawei") || lower.contains("honor") {
        Some((
            "Huawei",
            tr!(
                "Doctor",
                "On Huawei / Honor: open Settings → Battery → App launch, turn off Manage automatically for \
             NectarLink, and keep Auto-launch, Secondary launch, and Run in background on."
            ),
        ))
    } else if lower.contains("pixel") || lower.contains("google") {
        Some((
            "Pixel",
            tr!(
                "Doctor",
                "On Pixel: open Settings → Apps → See all apps → NectarLink → App battery usage, tap Allow \
             background usage, and choose Unrestricted."
            ),
        ))
    } else {
        None
    }
}

/// Returns the `(oem_label, guide_text)` for a phone's manufacturer when recognized.
pub fn oem_battery_guide(model: Option<&str>, name: &str) -> Option<(&'static str, String)> {
    model.and_then(match_oem).or_else(|| match_oem(name))
}

/// The findings, most important first.
pub fn diagnose(facts: &Facts, now_secs: i64) -> Vec<Check> {
    let mut checks = Vec::new();

    // The network itself.
    if !facts.local_address {
        checks.push(check(
            "network",
            Outcome::Fail,
            tr!("Doctor", "This PC isn't on a network"),
            tr!("Doctor", "Connect it to the same Wi-Fi or wired network as your phone."),
            None,
        ));
    } else {
        let title = match facts.networks.first() {
            Some((name, _)) if *name != tr!("Doctor", "This network") => {
                tr!("Doctor", "Connected to %1", name)
            }
            _ => tr!("Doctor", "Connected to a network"),
        };
        checks.push(check("network", Outcome::Ok, title, "", None));
    }
    if let Some((name, _)) = facts.networks.iter().find(|(_, trusted)| !trusted) {
        #[cfg(target_os = "linux")]
        let profile_detail = tr!(
            "Doctor",
            "On public networks the firewall keeps other devices from finding this PC. If it's your home or work \
             network, open network settings and set its firewall zone to Home or Trusted."
        );
        #[cfg(not(target_os = "linux"))]
        let profile_detail = tr!(
            "Doctor",
            "On public networks Windows keeps other devices from finding this PC. If it's your home or work \
             network, set it to Private."
        );
        checks.push(check(
            "profile",
            Outcome::Warn,
            tr!("Doctor", "%1 is set as a public network", name),
            profile_detail,
            Some("network-settings"),
        ));
    }

    // The firewall.
    match facts.firewall {
        None => {}
        Some(f) if !f.on => {
            #[cfg(target_os = "linux")]
            let title = tr!("Doctor", "The firewall is off for this network");
            #[cfg(not(target_os = "linux"))]
            let title = tr!("Doctor", "Windows Firewall is off for this network");
            checks.push(check("firewall", Outcome::Ok, title, "", None));
        }
        Some(f) if f.blocked => {
            #[cfg(target_os = "linux")]
            let (title, detail) = (
                tr!("Doctor", "The firewall blocks NectarLink"),
                tr!(
                    "Doctor",
                    "A firewall rule blocks incoming NectarLink connections. Allow it, and your phone can connect."
                ),
            );
            #[cfg(not(target_os = "linux"))]
            let (title, detail) = (
                tr!("Doctor", "Windows Firewall blocks NectarLink"),
                tr!(
                    "Doctor",
                    "Someone chose not to allow it when Windows asked. Allow it, and your phone can connect."
                ),
            );
            checks.push(check("firewall", Outcome::Fail, title, detail, Some("firewall")));
        }
        Some(f) if !f.allowed => {
            #[cfg(target_os = "linux")]
            let title = tr!("Doctor", "NectarLink isn't allowed through the firewall");
            #[cfg(not(target_os = "linux"))]
            let title = tr!("Doctor", "NectarLink isn't allowed through Windows Firewall");
            checks.push(check(
                "firewall",
                Outcome::Warn,
                title,
                tr!("Doctor", "Your phone may not be able to connect. Allow it once here."),
                Some("firewall"),
            ));
        }
        Some(_) => {
            #[cfg(target_os = "linux")]
            let title = tr!("Doctor", "The firewall lets your phone connect");
            #[cfg(not(target_os = "linux"))]
            let title = tr!("Doctor", "Windows Firewall lets your phone connect");
            checks.push(check("firewall", Outcome::Ok, title, "", None));
        }
    }

    if facts.vpn {
        checks.push(check(
            "vpn",
            Outcome::Warn,
            tr!("Doctor", "A VPN is on"),
            tr!("Doctor", "VPNs often keep devices on your network from reaching this PC. If your phone can't connect, try \
             turning it off."),
            None,
        ));
    }

    // USB: phones on a cable connect through adb (#322). Not having it only
    // matters while a phone isn't reachable another way.
    match facts.adb {
        Some(AdbFact { running: true, .. }) => checks.push(check(
            "adb",
            Outcome::Ok,
            tr!("Doctor", "Ready for a USB cable"),
            tr!("Doctor", "adb is running, so a phone with USB debugging on connects over a cable."),
            None,
        )),
        Some(AdbFact { installed: true, running: false }) => checks.push(check(
            "adb",
            Outcome::Warn,
            tr!("Doctor", "adb isn't running"),
            tr!(
                "Doctor",
                "Phones on a USB cable connect through adb. NectarLink starts it every minute or so; \
                 start it now if a phone on a cable isn't showing up."
            ),
            Some("start-adb"),
        )),
        Some(AdbFact { installed: false, .. })
            if facts.devices.iter().any(|d| !matches!(d.link, LinkState::Online { .. })) =>
        {
            checks.push(check(
                "adb",
                Outcome::Warn,
                tr!("Doctor", "A USB cable needs adb"),
                tr!(
                    "Doctor",
                    "To connect over a cable, NectarLink can get Android's USB tools from Google \
                     for you, and the phone needs USB debugging on. Wi-Fi works without them."
                ),
                Some("get-adb"),
            ));
        }
        _ => {}
    }

    #[cfg(target_os = "linux")]
    if !facts.missing_tools.is_empty() {
        let list = facts.missing_tools.join(", ");
        checks.push(check(
            "linux-tools",
            Outcome::Warn,
            format!("Optional helper tools not found: {list}"),
            format!(
                "Install {list} with your package manager for background clipboard sync and opening files or links."
            ),
            None,
        ));
    }

    #[cfg(target_os = "linux")]
    for (id, title, ok, detail, fix) in &facts.linux_checks {
        checks.push(check(
            id,
            if *ok { Outcome::Ok } else { Outcome::Warn },
            title.clone(),
            detail.clone(),
            *fix,
        ));
    }

    // Each phone.
    for (i, d) in facts.devices.iter().enumerate() {
        let name = &d.name;
        let ap_isolated = (!matches!(d.link, LinkState::Online { .. })
            || matches!(d.link, LinkState::Online { path: ConnectionPath::Relay, .. }))
            && d.mdns_seen_ago_secs.is_some_and(|s| s <= 120)
            && d.dial_failures >= 1;
        // A network numbered outside the private ranges (#131): NectarLink
        // before 0.1.5 refused it without Connect over mobile data, which
        // looks the same from here as a router keeping devices apart.
        let direct =
            matches!(d.link, LinkState::Online { path: ConnectionPath::Lan | ConnectionPath::Usb, .. });
        let outside_private = !direct
            && d.mdns_seen_ago_secs.is_some_and(|s| s <= 120)
            && !d.mdns_addrs.is_empty()
            && d.mdns_addrs.iter().all(|a| !nectarlink_core::is_lan_ip(a.ip()));
        if outside_private {
            let id = if i == 0 { "outside-private".to_owned() } else { format!("outside-private-{i}") };
            let outcome =
                if matches!(d.link, LinkState::Online { .. }) { Outcome::Warn } else { Outcome::Fail };
            checks.push(check(
                &id,
                outcome,
                tr!("Doctor", "%1 is on this Wi-Fi at an address outside the usual home ranges", name),
                tr!(
                    "Doctor",
                    "%1 announces itself at %2, which isn't in a private range such as 192.168.x.x. \
                     NectarLink connects on such a network once both apps are up to date. Until then, \
                     turn on Connect over mobile data on both, or connect with a USB cable.",
                    name,
                    d.mdns_addrs[0].ip()
                ),
                Some("reconnect"),
            ));
        } else if ap_isolated {
            let ap_id = if i == 0 { "ap-isolation".to_owned() } else { format!("ap-isolation-{i}") };
            let outcome =
                if matches!(d.link, LinkState::Online { .. }) { Outcome::Warn } else { Outcome::Fail };
            checks.push(check(
                &ap_id,
                outcome,
                tr!(
                    "Doctor",
                    "%1 is on this Wi-Fi, but the router blocks direct connections (AP isolation)",
                    name
                ),
                tr!(
                    "Doctor",
                    "This PC can see %1 announce itself on the network, but the router is blocking \
                     devices from talking directly to each other (often called AP Isolation, Client \
                     Isolation, or Guest Wi-Fi). Move both devices off any Guest Wi-Fi network, turn off AP \
                     / Client Isolation in your router's Wi-Fi settings, or connect with a USB cable.",
                    name
                ),
                Some("reconnect"),
            ));
        }

        let id = format!("device{i}");
        let finding = match &d.link {
            LinkState::Online { path, rtt_ms } => {
                let how = match path {
                    ConnectionPath::Lan => tr!("Doctor", "On this network"),
                    ConnectionPath::Usb => tr!("Doctor", "On USB"),
                    ConnectionPath::Relay => tr!("Doctor", "While away"),
                };
                check(
                    &id,
                    Outcome::Ok,
                    tr!("Doctor", "%1 is connected", name),
                    tr!("Doctor", "%1 · %2 ms", how, rtt_ms),
                    None,
                )
            }
            LinkState::Connecting => check(
                &id,
                Outcome::Warn,
                tr!("Doctor", "Connecting to %1…", name),
                tr!("Doctor", "This usually takes a few seconds."),
                None,
            ),
            LinkState::Offline { last_seen } => {
                let seen = last_seen.map_or_else(
                    || tr!("Doctor", "It hasn't connected yet."),
                    |at| tr!("Doctor", "Last seen %1.", ago(now_secs - at)),
                );
                check(
                    &id,
                    Outcome::Warn,
                    tr!("Doctor", "%1 isn't connected", name),
                    tr!(
                        "Doctor",
                        "%1 Open NectarLink on the phone and keep both on the same network. If the phone \
                         asks to run in the background, allow it.",
                        seen
                    ),
                    Some("reconnect"),
                )
            }
        };
        checks.push(finding);

        if matches!(d.link, LinkState::Offline { .. })
            && (d.os.eq_ignore_ascii_case("android") || d.os.is_empty())
            && let Some((oem, guide)) = oem_battery_guide(d.model.as_deref(), name)
        {
            checks.push(check(
                &format!("oem-battery-{i}"),
                Outcome::Warn,
                tr!("Doctor", "Keep %1 reachable in the background (%2)", name, oem),
                guide,
                None,
            ));
        }
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
        ..=0 => tr!("Doctor", "just now"),
        1 => tr!("Doctor", "a minute ago"),
        2..=59 => tr!("Doctor", "%1 minutes ago", minutes),
        60..=119 => tr!("Doctor", "an hour ago"),
        120..=1439 => tr!("Doctor", "%1 hours ago", minutes / 60),
        _ => tr!("Doctor", "%1 days ago", minutes / 1440),
    }
}

/// Runs a fix (blocking: the firewall one waits for the elevation prompt).
/// Returns whether anything was done.
pub fn fix(id: &str) -> bool {
    match id {
        "firewall" => {
            let Ok(exe) = std::env::current_exe() else {
                return windows_checks::open_firewall_settings();
            };
            let fixed = match windows_checks::fix_firewall(&exe) {
                Ok(true) => windows_checks::firewall(&exe).map(|f| f.allowed && !f.blocked).unwrap_or(true),
                Ok(false) => false,
                Err(e) => {
                    tracing::warn!(error = %e, "can't change the firewall");
                    false
                }
            };
            if !fixed {
                let _ = windows_checks::open_firewall_settings();
            }
            true
        }
        "firewall-settings" => windows_checks::open_firewall_settings(),
        #[cfg(target_os = "linux")]
        "network-settings" => windows_checks::open_network_settings(),
        #[cfg(not(target_os = "linux"))]
        "network-settings" => crate::win::shell::open_url("ms-settings:network-status").is_ok(),
        // The USB watcher finds the server on its next round, within seconds.
        "start-adb" => {
            nectarlink_core::start_adb_server(nectarlink_core::DEFAULT_ADB_ADDR);
            nectarlink_core::adb_server_answers(nectarlink_core::DEFAULT_ADB_ADDR)
        }
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

    fn device_fact(name: &str, link: LinkState) -> DeviceFact {
        DeviceFact {
            name: name.into(),
            model: None,
            os: "android".into(),
            link,
            mdns_seen_ago_secs: None,
            mdns_addrs: Vec::new(),
            dial_failures: 0,
        }
    }

    fn facts() -> Facts {
        Facts {
            local_address: true,
            vpn: false,
            networks: vec![("Home".into(), true)],
            firewall: Some(windows_checks::Firewall { on: true, allowed: true, blocked: false }),
            devices: vec![device_fact("Pixel", LinkState::Online { path: ConnectionPath::Lan, rtt_ms: 4 })],
            adb: None,
            #[cfg(target_os = "linux")]
            missing_tools: Vec::new(),
            #[cfg(target_os = "linux")]
            linux_checks: Vec::new(),
        }
    }

    fn outcomes(checks: &[Check]) -> Vec<(&str, Outcome)> {
        checks.iter().map(|c| (c.id.as_str(), c.outcome)).collect()
    }

    #[test]
    fn adb_is_reported_for_usb_and_only_nags_while_a_phone_is_unreachable() {
        let adb = |installed, running| {
            let mut f = facts();
            f.adb = Some(AdbFact { installed, running });
            f
        };
        let find = |f: &Facts| diagnose(f, 0).into_iter().find(|c| c.id == "adb");

        let ready = find(&adb(true, true)).unwrap();
        assert_eq!(ready.outcome, Outcome::Ok);
        assert_eq!(ready.fix, None);

        let stopped = find(&adb(true, false)).unwrap();
        assert_eq!(stopped.outcome, Outcome::Warn);
        assert_eq!(stopped.fix, Some("start-adb"));

        // Not installed: nothing while the phone is connected another way...
        assert!(find(&adb(false, false)).is_none());
        // ...and a warning once it isn't, whose button sets USB up (Install on a phone).
        let mut offline = adb(false, false);
        offline.devices = vec![device_fact("Pixel", LinkState::Offline { last_seen: None })];
        let missing = find(&offline).unwrap();
        assert_eq!(missing.outcome, Outcome::Warn);
        assert_eq!(missing.fix, Some("get-adb"));

        // USB turned off (NECTARLINK_NO_ADB): no line at all.
        let mut off = facts();
        off.devices = vec![device_fact("Pixel", LinkState::Offline { last_seen: None })];
        assert!(find(&off).is_none());
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
        f.devices = vec![device_fact("Phone", LinkState::Offline { last_seen: Some(1_000) })];
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
    fn ap_isolation_detected_when_mdns_seen_and_direct_dial_fails() {
        let mut f = facts();
        f.devices = vec![DeviceFact {
            name: "Galaxy S24".into(),
            model: Some("SM-S921B".into()),
            os: "android".into(),
            link: LinkState::Offline { last_seen: Some(900) },
            mdns_seen_ago_secs: Some(15),
            mdns_addrs: vec!["192.168.1.55:4242".parse().unwrap()],
            dial_failures: 2,
        }];
        let checks = diagnose(&f, 1_000);
        let ap = checks.iter().find(|c| c.id == "ap-isolation").expect("ap-isolation check present");
        assert_eq!(ap.outcome, Outcome::Fail);
        assert!(ap.title.contains("AP isolation"), "{}", ap.title);
        assert!(ap.detail.contains("Guest Wi-Fi"), "{}", ap.detail);
        assert_eq!(ap.fix, Some("reconnect"));

        // Relay-connected with recent mDNS and failed direct dial warns about AP isolation.
        f.devices[0].link = LinkState::Online { path: ConnectionPath::Relay, rtt_ms: 48 };
        let checks_relay = diagnose(&f, 1_000);
        let ap_relay =
            checks_relay.iter().find(|c| c.id == "ap-isolation").expect("ap-isolation warn on relay");
        assert_eq!(ap_relay.outcome, Outcome::Warn);

        // A phone announcing only a public address is a network outside the
        // private ranges (#131), not AP isolation.
        f.devices[0].link = LinkState::Offline { last_seen: Some(900) };
        f.devices[0].mdns_addrs = vec!["192.1.1.208:49806".parse().unwrap()];
        let checks_public = diagnose(&f, 1_000);
        assert!(!checks_public.iter().any(|c| c.id == "ap-isolation"));
        let outside =
            checks_public.iter().find(|c| c.id == "outside-private").expect("outside-private check present");
        assert_eq!(outside.outcome, Outcome::Fail);
        assert!(outside.detail.contains("192.1.1.208"), "{}", outside.detail);
        // Once it's connected on this network, there's nothing to report.
        f.devices[0].link = LinkState::Online { path: ConnectionPath::Lan, rtt_ms: 4 };
        assert!(!diagnose(&f, 1_000).iter().any(|c| c.id == "outside-private"));
        f.devices[0].link = LinkState::Online { path: ConnectionPath::Relay, rtt_ms: 48 };
        f.devices[0].mdns_addrs = vec!["192.168.1.55:4242".parse().unwrap()];

        // Stale mDNS (> 120s) does not trigger AP isolation.
        f.devices[0].mdns_seen_ago_secs = Some(121);
        let checks_stale = diagnose(&f, 1_000);
        assert!(!checks_stale.iter().any(|c| c.id == "ap-isolation"));
    }

    #[test]
    fn oem_battery_guides_match_all_seven_manufacturers_individually() {
        let cases = [
            (Some("SM-S928B"), "Phone", "Samsung", "Device care"),
            (Some("23127PN0CC"), "Xiaomi 14", "Xiaomi", "HyperOS / MIUI"),
            (Some("CPH2581"), "OnePlus 12", "OnePlus", "OxygenOS"),
            (Some("RMX3888"), "My Phone", "Oppo / Realme", "On Oppo / Realme:"),
            (Some("V2307A"), "vivo X100", "Vivo", "OriginOS / Funtouch"),
            (Some("ALN-AL00"), "Huawei Mate 60", "Huawei", "On Huawei / Honor:"),
            (Some("Pixel 9 Pro"), "Phone", "Pixel", "On Pixel:"),
        ];
        for (model, name, expected_oem, expected_snippet) in cases {
            let (oem, guide) = oem_battery_guide(model, name)
                .unwrap_or_else(|| panic!("expected guide for {model:?} / {name}"));
            assert_eq!(oem, expected_oem);
            assert!(guide.contains(expected_snippet), "{oem}: {guide}");

            let mut f = facts();
            f.devices = vec![DeviceFact {
                name: name.into(),
                model: model.map(str::to_owned),
                os: "android".into(),
                link: LinkState::Offline { last_seen: Some(500) },
                mdns_seen_ago_secs: None,
                mdns_addrs: Vec::new(),
                dial_failures: 0,
            }];
            let checks = diagnose(&f, 600);
            let oem_checks: Vec<_> = checks.iter().filter(|c| c.id.starts_with("oem-battery-")).collect();
            assert_eq!(oem_checks.len(), 1, "only the matching OEM's guide is shown");
            assert_eq!(oem_checks[0].outcome, Outcome::Warn);
            assert!(oem_checks[0].title.contains(expected_oem), "{}", oem_checks[0].title);
            assert!(oem_checks[0].detail.contains(expected_snippet), "{}", oem_checks[0].detail);
        }
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

    #[cfg(target_os = "linux")]
    #[test]
    fn includes_linux_readiness_checks_and_platform_titles() {
        let mut f = facts();
        f.linux_checks = vec![
            (
                "linux-appindicator".into(),
                "GNOME system tray extension not detected".into(),
                false,
                "Install gnome-shell-extension-appindicator".into(),
                None,
            ),
            (
                "linux-fuse".into(),
                "Phone storage can open in your file manager".into(),
                true,
                String::new(),
                None,
            ),
        ];
        let checks = diagnose(&f, 0);
        let fw = checks.iter().find(|c| c.id == "firewall").expect("firewall check");
        assert_eq!(fw.title, "The firewall lets your phone connect");
        let appind = checks.iter().find(|c| c.id == "linux-appindicator").expect("appindicator check");
        assert_eq!(appind.outcome, Outcome::Warn);
        let fuse = checks.iter().find(|c| c.id == "linux-fuse").expect("fuse check");
        assert_eq!(fuse.outcome, Outcome::Ok);
    }
}
