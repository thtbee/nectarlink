// SPDX-License-Identifier: GPL-3.0-or-later
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Result;
use std::net::Ipv4Addr;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

pub const DEFAULT_PORT: u16 = 7718;
pub const MDNS_PORT: u16 = 5353;

static FIREWALL_VERIFIED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Adapters {
    pub local_address: bool,
    pub vpn: bool,
}

pub fn parse_port_file(contents: &str) -> Option<u16> {
    contents.split_whitespace().find_map(|tok| tok.parse::<u16>().ok().filter(|&p| p > 0))
}

pub fn active_node_port() -> u16 {
    if let Some(node) = crate::core_host::node()
        && let Some(port) = node.local_bound_addrs().into_iter().map(|a| a.port()).find(|&p| p > 0)
    {
        return port;
    }
    let port_file = crate::core_host::host().data_dir.join("port");
    fs::read_to_string(port_file).ok().and_then(|s| parse_port_file(&s)).unwrap_or(DEFAULT_PORT)
}

fn record_firewall_verified(port: u16) {
    FIREWALL_VERIFIED.store(true, Ordering::SeqCst);
    let verified_path = crate::core_host::host().data_dir.join("firewall_verified");
    let _ = fs::write(verified_path, format!("{port}\n"));
}

fn is_firewall_verified(port: u16) -> bool {
    if FIREWALL_VERIFIED.load(Ordering::SeqCst) {
        return true;
    }
    let verified_path = crate::core_host::host().data_dir.join("firewall_verified");
    if let Ok(content) = fs::read_to_string(verified_path)
        && content.lines().any(|l| l.trim().parse::<u16>().ok() == Some(port))
    {
        FIREWALL_VERIFIED.store(true, Ordering::SeqCst);
        return true;
    }
    false
}

pub fn parse_ufw_status(output: &str) -> Option<Firewall> {
    parse_ufw_status_for_port(output, DEFAULT_PORT)
}

pub fn parse_ufw_status_for_port(output: &str, port: u16) -> Option<Firewall> {
    let lower = output.to_ascii_lowercase();
    if lower.contains("status: inactive") {
        return Some(Firewall { on: false, allowed: true, blocked: false });
    }
    if !lower.contains("status: active") {
        return None;
    }

    let port_str = port.to_string();
    let port_tcp = format!("{port}/tcp");
    let port_udp = format!("{port}/udp");
    let port_colon = format!("{port}:");

    let mut tcp_allowed = false;
    let mut udp_allowed = false;
    let mut blocked = false;

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("Status:")
            || trimmed.starts_with("To ")
            || trimmed.starts_with("--")
        {
            continue;
        }
        let tokens: Vec<&str> = trimmed.split_whitespace().collect();
        if tokens.len() < 2 {
            continue;
        }
        let port_spec = tokens[0].to_ascii_lowercase();
        let touches_port = port_spec == port_str
            || port_spec == port_tcp
            || port_spec == port_udp
            || port_spec.starts_with(&port_colon);
        if !touches_port {
            continue;
        }
        let upper_line = trimmed.to_ascii_uppercase();
        if upper_line.contains("DENY") || upper_line.contains("REJECT") {
            blocked = true;
        } else if upper_line.contains("ALLOW") {
            if port_spec == port_str {
                tcp_allowed = true;
                udp_allowed = true;
            } else if port_spec == port_tcp || port_spec.ends_with("/tcp") {
                tcp_allowed = true;
            } else if port_spec == port_udp || port_spec.ends_with("/udp") {
                udp_allowed = true;
            }
        }
    }

    let allowed = !blocked && tcp_allowed && udp_allowed;
    Some(Firewall { on: true, allowed, blocked })
}

pub fn parse_firewalld_output(state: &str, ports: &str) -> Firewall {
    parse_firewalld_output_for_port(state, ports, "", DEFAULT_PORT)
}

pub fn parse_firewalld_output_for_port(state: &str, ports: &str, rich_rules: &str, port: u16) -> Firewall {
    if !state.trim().eq_ignore_ascii_case("running") {
        return Firewall { on: false, allowed: true, blocked: false };
    }
    let port_tcp = format!("{port}/tcp");
    let port_dash = format!("{port}-");
    let port_attr = format!("port=\"{port}\"");

    let mut blocked = false;
    let mut rich_allowed = false;
    for line in rich_rules.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.contains(&port_attr) || lower.contains("port=\"7718\"") {
            if lower.contains("reject") || lower.contains("drop") {
                blocked = true;
            } else if lower.contains("accept") {
                rich_allowed = true;
            }
        }
    }

    let port_allowed = ports.split_whitespace().any(|p| {
        p.eq_ignore_ascii_case(&port_tcp)
            || p.starts_with(&port_dash)
            || p.eq_ignore_ascii_case("7718/tcp")
            || p.starts_with("7718-")
    });
    Firewall { on: true, allowed: !blocked && (port_allowed || rich_allowed), blocked }
}

fn check_ufw_config_fallback(port: u16) -> Option<Firewall> {
    let conf = fs::read_to_string("/etc/ufw/ufw.conf").ok()?;
    let enabled = conf.lines().any(|l| {
        let t = l.trim();
        t.eq_ignore_ascii_case("ENABLED=yes") || t.eq_ignore_ascii_case("ENABLED=\"yes\"")
    });
    if !enabled {
        return Some(Firewall { on: false, allowed: true, blocked: false });
    }
    let port_str = port.to_string();
    for rules_path in ["/etc/ufw/user.rules", "/lib/ufw/user.rules"] {
        if let Ok(rules) = fs::read_to_string(rules_path) {
            let touches = |l: &str| l.contains(&port_str) || l.contains("7718");
            let has_drop =
                rules.lines().any(|l| touches(l) && (l.contains("-j DROP") || l.contains("-j REJECT")));
            let has_accept = rules.lines().any(|l| touches(l) && l.contains("-j ACCEPT"));
            return Some(Firewall { on: true, allowed: !has_drop && has_accept, blocked: has_drop });
        }
    }
    // Non-root users cannot read `/etc/ufw/user.rules` (0640 root:root); consult verified state.
    let verified = is_firewall_verified(port) || is_firewall_verified(DEFAULT_PORT);
    Some(Firewall { on: true, allowed: verified, blocked: false })
}

fn is_service_active(service: &str) -> bool {
    Command::new("systemctl")
        .args(["is-active", "--quiet", service])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn has_command(cmd: &str) -> bool {
    if cmd.is_empty() || cmd.contains('/') || cmd.contains('\0') {
        return false;
    }
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    let extra = [Path::new("/usr/sbin"), Path::new("/sbin"), Path::new("/usr/bin"), Path::new("/bin")];
    std::env::split_paths(&path_var).chain(extra.into_iter().map(Path::to_path_buf)).any(|dir| {
        let candidate = dir.join(cmd);
        fs::metadata(&candidate).is_ok_and(|m| {
            if !m.is_file() {
                return false;
            }
            #[cfg(unix)]
            {
                m.mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                true
            }
        })
    })
}

pub fn firewall(_exe: &Path) -> Result<Firewall> {
    let port = active_node_port();

    // 1. Try ufw status
    if let Ok(out) = Command::new("ufw").arg("status").output()
        && out.status.success()
    {
        let stdout = String::from_utf8_lossy(&out.stdout);
        if let Some(fw) = parse_ufw_status_for_port(&stdout, port) {
            return Ok(fw);
        }
    }
    if is_service_active("ufw")
        && let Some(fw) = check_ufw_config_fallback(port)
    {
        return Ok(fw);
    }

    // 2. Try firewalld (firewall-cmd --state, --list-ports, --list-rich-rules)
    if let Ok(state_out) = Command::new("firewall-cmd").arg("--state").output() {
        let state_str = String::from_utf8_lossy(&state_out.stdout);
        if state_str.trim().eq_ignore_ascii_case("running") {
            let ports_str = Command::new("firewall-cmd")
                .arg("--list-ports")
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                .unwrap_or_default();
            let rich_str = Command::new("firewall-cmd")
                .arg("--list-rich-rules")
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                .unwrap_or_default();
            let mut fw = parse_firewalld_output_for_port(&state_str, &ports_str, &rich_str, port);
            if !fw.allowed
                && !fw.blocked
                && (is_firewall_verified(port) || is_firewall_verified(DEFAULT_PORT))
            {
                fw.allowed = true;
            }
            return Ok(fw);
        }
    }

    // 3. Try nftables
    if is_service_active("nftables") {
        let ruleset = Command::new("nft")
            .args(["list", "ruleset"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .or_else(|| fs::read_to_string("/etc/nftables.conf").ok());
        if let Some(conf) = ruleset {
            let lower = conf.to_ascii_lowercase();
            let has_policy_drop = lower.contains("policy drop");
            let port_str = port.to_string();
            let has_port = lower.contains(&port_str) || lower.contains("7718");
            if has_policy_drop || has_port {
                let verified = is_firewall_verified(port) || is_firewall_verified(DEFAULT_PORT);
                return Ok(Firewall { on: true, allowed: has_port || verified, blocked: false });
            }
        }
    }

    Ok(Firewall { on: false, allowed: true, blocked: false })
}

/// Parses `nmcli -t -f GENERAL.CONNECTION,GENERAL.ZONE device show` output into a
/// map from connection name to firewall zone.
pub fn parse_nmcli_device_zones(stdout: &str) -> HashMap<String, String> {
    let mut zones = HashMap::new();
    let mut current_conn: Option<String> = None;
    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some(conn) = trimmed.strip_prefix("GENERAL.CONNECTION:") {
            let name = super::wifi::unescape_nmcli(conn.trim());
            if !name.is_empty() && name != "--" {
                current_conn = Some(name);
            } else {
                current_conn = None;
            }
        } else if let Some(zone) = trimmed.strip_prefix("GENERAL.ZONE:") {
            let zone_trimmed = zone.trim();
            if let Some(conn) = current_conn.take()
                && !zone_trimmed.is_empty()
                && zone_trimmed != "--"
            {
                zones.insert(conn, zone_trimmed.to_ascii_lowercase());
            }
        }
    }
    zones
}

/// Parses `nmcli -t -f DEVICE,NAME,TYPE,STATE connection show --active` output and combines it
/// with firewall zones from `device show`, marking `public` firewall zones as `trusted: false`.
pub fn parse_nmcli_active_networks(active_stdout: &str, zones: &HashMap<String, String>) -> Vec<Network> {
    let mut list = Vec::new();
    for line in active_stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Split from the right: `...:TYPE:STATE`, and from the left: `DEVICE:NAME`.
        let Some((rest, state)) = trimmed.rsplit_once(':') else {
            continue;
        };
        let Some((dev_and_name, kind)) = rest.rsplit_once(':') else {
            continue;
        };
        let Some((device, raw_name)) = dev_and_name.split_once(':') else {
            continue;
        };
        if state != "activated" && state != "activating" {
            continue;
        }
        if device == "lo"
            || kind == "loopback"
            || kind == "bridge"
            || kind == "tun"
            || kind == "wireguard"
            || super::wol::is_virtual_interface_name(device)
        {
            continue;
        }
        let name = super::wifi::unescape_nmcli(raw_name.trim());
        if name.is_empty() {
            continue;
        }
        let trusted =
            !zones.get(&name).is_some_and(|z| z.eq_ignore_ascii_case("public") || z.starts_with("public"));
        if !list.iter().any(|n: &Network| n.name == name) {
            list.push(Network { name, trusted });
        }
    }
    list
}

fn interfaces_with_ipv4() -> (Vec<String>, bool) {
    let mut ipv4_ifaces = Vec::new();
    let mut vpn = false;
    let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();

    // SAFETY: `getifaddrs` allocates and populates a linked list freed by `freeifaddrs`.
    if unsafe { libc::getifaddrs(&mut ifap) } == 0 && !ifap.is_null() {
        let mut cur = ifap;
        // SAFETY: iterating valid `ifaddrs` nodes before `freeifaddrs`.
        unsafe {
            while let Some(ifa) = cur.as_ref() {
                cur = ifa.ifa_next;
                if ifa.ifa_name.is_null() {
                    continue;
                }
                let flags = ifa.ifa_flags as i32;
                if (flags & libc::IFF_UP) == 0 || (flags & libc::IFF_LOOPBACK) != 0 {
                    continue;
                }
                let Ok(name) = std::ffi::CStr::from_ptr(ifa.ifa_name).to_str() else {
                    continue;
                };
                let lower = name.to_ascii_lowercase();
                if lower == "lo" {
                    continue;
                }
                let is_vpn = lower.starts_with("tun")
                    || lower.starts_with("tap")
                    || lower.starts_with("wg")
                    || lower.starts_with("tailscale")
                    || lower.starts_with("nordlynx")
                    || lower.starts_with("ppp")
                    || lower.starts_with("zt");
                if is_vpn {
                    vpn = true;
                    continue;
                }
                if !ifa.ifa_addr.is_null() && i32::from((*ifa.ifa_addr).sa_family) == libc::AF_INET {
                    let sin_addr = (*ifa.ifa_addr.cast::<libc::sockaddr_in>()).sin_addr.s_addr;
                    let ip = Ipv4Addr::from(u32::from_be(sin_addr));
                    if !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified() {
                        let owned = name.to_string();
                        if !ipv4_ifaces.contains(&owned) {
                            ipv4_ifaces.push(owned);
                        }
                    }
                }
            }
            libc::freeifaddrs(ifap);
        }
    }

    (ipv4_ifaces, vpn)
}

pub fn networks() -> Result<Vec<Network>> {
    if let Ok(active_out) = Command::new("nmcli")
        .args(["-t", "-f", "DEVICE,NAME,TYPE,STATE", "connection", "show", "--active"])
        .output()
        && active_out.status.success()
    {
        let zones = Command::new("nmcli")
            .args(["-t", "-f", "GENERAL.CONNECTION,GENERAL.ZONE", "device", "show"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| parse_nmcli_device_zones(&String::from_utf8_lossy(&o.stdout)))
            .unwrap_or_default();
        let active_str = String::from_utf8_lossy(&active_out.stdout);
        let parsed = parse_nmcli_active_networks(&active_str, &zones);
        if !parsed.is_empty() {
            return Ok(parsed);
        }
    }

    let (ipv4_ifaces, _) = interfaces_with_ipv4();
    let list = ipv4_ifaces.into_iter().map(|name| Network { name, trusted: true }).collect();
    Ok(list)
}

pub fn adapters() -> Adapters {
    let (ipv4_ifaces, mut vpn) = interfaces_with_ipv4();
    let local_address = !ipv4_ifaces.is_empty();

    if !vpn && let Ok(entries) = fs::read_dir("/sys/class/net") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            let state = fs::read_to_string(entry.path().join("operstate"))
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            if (state == "up" || state == "unknown")
                && (name.starts_with("tun")
                    || name.starts_with("tap")
                    || name.starts_with("wg")
                    || name.starts_with("tailscale")
                    || name.starts_with("nordlynx")
                    || name.starts_with("ppp")
                    || name.starts_with("zt"))
            {
                vpn = true;
                break;
            }
        }
    }

    Adapters { local_address, vpn }
}

/// Returns the names of optional session-relevant helper commands that are not
/// currently installed on `PATH` (`wl-copy`/`wl-paste` on Wayland, `xclip` on
/// X11, and `xdg-open`).
pub fn missing_tools() -> Vec<&'static str> {
    let mut missing = Vec::new();
    if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty()) {
        for tool in ["wl-copy", "wl-paste"] {
            if !has_command(tool) {
                missing.push(tool);
            }
        }
    }
    if std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty()) && !has_command("xclip") {
        missing.push("xclip");
    }
    if !has_command("xdg-open") {
        missing.push("xdg-open");
    }
    missing
}

fn session_dbus_has_owner(bus_name: &str) -> bool {
    super::non_tokio(|| {
        let Ok(conn) = zbus::blocking::Connection::session() else {
            return false;
        };
        super::dbus_name_has_owner(&conn, bus_name)
    })
}

fn has_libxtst() -> bool {
    const CANDIDATES: &[&str] = &[
        "/usr/lib/x86_64-linux-gnu/libXtst.so.6",
        "/lib/x86_64-linux-gnu/libXtst.so.6",
        "/usr/lib64/libXtst.so.6",
        "/usr/lib/libXtst.so.6",
        "/usr/lib/aarch64-linux-gnu/libXtst.so.6",
    ];
    CANDIDATES.iter().any(|p| Path::new(p).exists()) || has_command("xdotool")
}

fn has_v4l2loopback() -> bool {
    if Path::new("/sys/module/v4l2loopback").exists() {
        return true;
    }
    for bin in ["modinfo", "/sbin/modinfo", "/usr/sbin/modinfo"] {
        if Command::new(bin)
            .arg("v4l2loopback")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
        {
            return true;
        }
    }
    false
}

/// Linux-specific checks for features that need something installed or
/// allowed first: `(id, title, ok, detail, fix)`. These don't stop the phone
/// from connecting; the Doctor lists them after the connection checks.
pub fn linux_checks() -> Vec<(String, String, bool, String, Option<&'static str>)> {
    use crate::i18n::tr;
    let mut out = Vec::new();
    let mut check = |id: &str, ok: bool, ready: String, needs: String, how: String| {
        out.push((
            id.to_owned(),
            if ok { ready } else { needs },
            ok,
            if ok { String::new() } else { how },
            None,
        ));
    };

    // GNOME hides tray icons unless the AppIndicator extension is on.
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    if desktop.to_ascii_uppercase().contains("GNOME") {
        check(
            "linux-appindicator",
            session_dbus_has_owner("org.kde.StatusNotifierWatcher"),
            tr!("Doctor", "NectarLink is in the top bar"),
            tr!("Doctor", "NectarLink can't show its icon in the top bar"),
            tr!(
                "Doctor",
                "GNOME shows app icons there only with the AppIndicator extension. Install it (gnome-shell-extension-appindicator) and turn it on in Extensions."
            ),
        );
    }

    let uinput_writable = OpenOptions::new().write(true).open("/dev/uinput").is_ok();
    let portal_available = session_dbus_has_owner("org.freedesktop.portal.Desktop");
    let x11_xtst = std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty()) && has_libxtst();
    check(
        "linux-remote-input",
        uinput_writable || portal_available || x11_xtst,
        tr!("Doctor", "Your phone can control this PC"),
        tr!("Doctor", "Your phone can't control this PC yet"),
        tr!(
            "Doctor",
            "The touchpad and keyboard on your phone need the desktop portal (xdg-desktop-portal) or access to /dev/uinput."
        ),
    );

    check(
        "linux-fuse",
        Path::new("/dev/fuse").exists() && (has_command("fusermount3") || has_command("fusermount")),
        tr!("Doctor", "Phone storage can open in your file manager"),
        tr!("Doctor", "Phone storage needs one more step"),
        tr!("Doctor", "Install fuse3 to browse your phone's files in your file manager."),
    );

    check(
        "linux-audio",
        has_command("pactl") || has_command("wpctl") || has_command("pw-play") || has_command("aplay"),
        tr!("Doctor", "Sound is ready"),
        tr!("Doctor", "Calls and alerts can't play sound yet"),
        tr!("Doctor", "Install pipewire-pulse (or pulseaudio-utils) for call audio and Find my PC."),
    );

    // Sharing this PC's screen on Wayland goes through the ScreenCast portal
    // and GStreamer's PipeWire source; on X11 it reads the screen directly.
    if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty()) {
        check(
            "linux-screencast",
            portal_available && crate::pc_mirror::pipewire_capture_installed(),
            tr!("Doctor", "Your phone can show this PC's screen"),
            tr!("Doctor", "Showing this PC's screen needs one more step"),
            tr!(
                "Doctor",
                "Install gstreamer1.0-pipewire and gstreamer1.0-tools, and the desktop portal (xdg-desktop-portal) if it's missing."
            ),
        );
    }

    check(
        "linux-v4l2loopback",
        has_v4l2loopback(),
        tr!("Doctor", "Phone as webcam is ready"),
        tr!("Doctor", "Phone as webcam needs one more step"),
        tr!("Doctor", "Install v4l2loopback-dkms, then restart, so apps can use your phone as a camera."),
    );

    out
}

pub fn build_ufw_fix_script(node_port: u16) -> String {
    let mut ports = vec![DEFAULT_PORT];
    if node_port != DEFAULT_PORT && node_port != MDNS_PORT && node_port > 0 {
        ports.push(node_port);
    }
    let port_cmds: Vec<String> = ports
        .iter()
        .map(|p| {
            format!(
                "ufw allow from \"$cidr\" to any port {p} proto tcp && ufw allow from \"$cidr\" to any port {p} proto udp"
            )
        })
        .collect();
    let joined = port_cmds.join(" && ");
    let ts_udp: Vec<String> =
        ports.iter().map(|p| format!("ufw allow from \"$cidr\" to any port {p} proto udp")).collect();
    let ts_joined = ts_udp.join(" && ");
    format!(
        "for cidr in 192.168.0.0/16 10.0.0.0/8 172.16.0.0/12 fe80::/10; do \
           {joined} && \
           ufw allow from \"$cidr\" to any port {MDNS_PORT} proto udp || exit 1; \
         done && \
         for cidr in 100.64.0.0/10 fd7a:115c:a1e0::/48; do \
           ({ts_joined}) || case \"$cidr\" in fd7a:*) ;; *) exit 1 ;; esac; \
         done"
    )
}

pub fn build_firewalld_fix_script(node_port: u16) -> String {
    let mut ports = vec![DEFAULT_PORT];
    if node_port != DEFAULT_PORT && node_port != MDNS_PORT && node_port > 0 {
        ports.push(node_port);
    }
    let mut cmds = Vec::new();
    for cidr in ["192.168.0.0/16", "10.0.0.0/8", "172.16.0.0/12"] {
        for &p in &ports {
            cmds.push(format!(
                "firewall-cmd --permanent --add-rich-rule='rule family=\"ipv4\" source address=\"{cidr}\" port port=\"{p}\" protocol=\"tcp\" accept'"
            ));
            cmds.push(format!(
                "firewall-cmd --permanent --add-rich-rule='rule family=\"ipv4\" source address=\"{cidr}\" port port=\"{p}\" protocol=\"udp\" accept'"
            ));
        }
        cmds.push(format!(
            "firewall-cmd --permanent --add-rich-rule='rule family=\"ipv4\" source address=\"{cidr}\" port port=\"{MDNS_PORT}\" protocol=\"udp\" accept'"
        ));
    }
    for &p in &ports {
        cmds.push(format!(
            "firewall-cmd --permanent --add-rich-rule='rule family=\"ipv4\" source address=\"100.64.0.0/10\" port port=\"{p}\" protocol=\"udp\" accept'"
        ));
        cmds.push(format!(
            "firewall-cmd --permanent --add-rich-rule='rule family=\"ipv6\" source address=\"fe80::/10\" port port=\"{p}\" protocol=\"tcp\" accept'"
        ));
        cmds.push(format!(
            "firewall-cmd --permanent --add-rich-rule='rule family=\"ipv6\" source address=\"fe80::/10\" port port=\"{p}\" protocol=\"udp\" accept'"
        ));
        cmds.push(format!(
            "firewall-cmd --permanent --add-rich-rule='rule family=\"ipv6\" source address=\"fd7a:115c:a1e0::/48\" port port=\"{p}\" protocol=\"udp\" accept'"
        ));
    }
    cmds.push(format!(
        "firewall-cmd --permanent --add-rich-rule='rule family=\"ipv6\" source address=\"fe80::/10\" port port=\"{MDNS_PORT}\" protocol=\"udp\" accept'"
    ));
    cmds.push("firewall-cmd --reload".into());
    cmds.join(" && ")
}

pub fn fix_firewall(_exe: &Path) -> Result<bool> {
    let port = active_node_port();
    if is_service_active("ufw") || has_command("ufw") {
        let script = build_ufw_fix_script(port);
        let status = super::elevated::run_elevated(Path::new("sh"), &["-c", &script]);
        if matches!(status, Ok(0)) {
            record_firewall_verified(port);
            return Ok(true);
        }
        return Ok(false);
    }
    if is_service_active("firewalld") || has_command("firewall-cmd") {
        let script = build_firewalld_fix_script(port);
        let status = super::elevated::run_elevated(Path::new("sh"), &["-c", &script]);
        if matches!(status, Ok(0)) {
            record_firewall_verified(port);
            return Ok(true);
        }
        return Ok(false);
    }
    Ok(false)
}

pub fn open_firewall_settings() -> bool {
    Command::new("gufw")
        .spawn()
        .or_else(|_| Command::new("firewall-config").spawn())
        .or_else(|_| Command::new("plasma-firewall").spawn())
        .or_else(|_| Command::new("gnome-control-center").arg("network").spawn())
        .is_ok()
}

pub fn open_network_settings() -> bool {
    Command::new("nm-connection-editor")
        .spawn()
        .or_else(|_| Command::new("gnome-control-center").arg("network").spawn())
        .or_else(|_| Command::new("gnome-control-center").arg("wifi").spawn())
        .or_else(|_| Command::new("plasma-nm").spawn())
        .or_else(|_| Command::new("systemsettings").arg("kcm_networkmanagement").spawn())
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ufw_inactive_active_allowed_and_blocked() {
        assert_eq!(
            parse_ufw_status("Status: inactive\n"),
            Some(Firewall { on: false, allowed: true, blocked: false })
        );

        let allowed_output = "\
Status: active

To                         Action      From
--                         ------      ----
22/tcp                     ALLOW       Anywhere
7718/tcp                   ALLOW       Anywhere
7718/udp                   ALLOW       Anywhere
";
        assert_eq!(
            parse_ufw_status(allowed_output),
            Some(Firewall { on: true, allowed: true, blocked: false })
        );

        let combined_allowed_output = "\
Status: active

To                         Action      From
--                         ------      ----
7718                       ALLOW       Anywhere
";
        assert_eq!(
            parse_ufw_status(combined_allowed_output),
            Some(Firewall { on: true, allowed: true, blocked: false })
        );

        let subnet_scoped_allowed_output = "\
Status: active

To                         Action      From
--                         ------      ----
7718/tcp                   ALLOW       192.168.0.0/16
7718/udp                   ALLOW       192.168.0.0/16
5353/udp                   ALLOW       192.168.0.0/16
";
        assert_eq!(
            parse_ufw_status(subnet_scoped_allowed_output),
            Some(Firewall { on: true, allowed: true, blocked: false })
        );

        let blocked_output = "\
Status: active

To                         Action      From
--                         ------      ----
7718/tcp                   DENY        Anywhere
";
        assert_eq!(
            parse_ufw_status(blocked_output),
            Some(Firewall { on: true, allowed: false, blocked: true })
        );

        let unconfigured_active = "\
Status: active

To                         Action      From
--                         ------      ----
22/tcp                     ALLOW       Anywhere
";
        assert_eq!(
            parse_ufw_status(unconfigured_active),
            Some(Firewall { on: true, allowed: false, blocked: false })
        );

        assert_eq!(parse_ufw_status("ufw: command not found"), None);
    }

    #[test]
    fn parses_firewalld_running_ports_and_scoped_rich_rules() {
        assert_eq!(
            parse_firewalld_output("not running\n", ""),
            Firewall { on: false, allowed: true, blocked: false }
        );
        assert_eq!(
            parse_firewalld_output("running\n", "80/tcp 443/tcp\n"),
            Firewall { on: true, allowed: false, blocked: false }
        );
        assert_eq!(
            parse_firewalld_output("running\n", "5353/udp 7718/tcp 7718/udp\n"),
            Firewall { on: true, allowed: true, blocked: false }
        );

        let rich = "rule family=\"ipv4\" source address=\"192.168.0.0/16\" port port=\"49152\" protocol=\"tcp\" accept\n";
        assert_eq!(
            parse_firewalld_output_for_port("running\n", "", rich, 49152),
            Firewall { on: true, allowed: true, blocked: false }
        );
    }

    #[test]
    fn builds_scoped_ufw_and_firewalld_scripts_with_dynamic_port() {
        let ufw_script = build_ufw_fix_script(45678);
        assert!(ufw_script.contains("port 7718 proto tcp"));
        assert!(ufw_script.contains("port 45678 proto tcp"));
        assert!(ufw_script.contains("port 5353 proto udp"));
        assert!(ufw_script.contains("fe80::/10"));

        let fwd_script = build_firewalld_fix_script(45678);
        assert!(fwd_script.contains("--add-rich-rule='rule family=\"ipv4\" source address=\"192.168.0.0/16\" port port=\"7718\" protocol=\"tcp\" accept'"));
        assert!(fwd_script.contains("port=\"45678\""));
        assert!(fwd_script.contains("family=\"ipv6\" source address=\"fe80::/10\""));
        assert!(fwd_script.ends_with("firewall-cmd --reload"));
    }

    #[test]
    fn parses_nmcli_networks_and_marks_public_zone_untrusted() {
        let dev_show = "\
GENERAL.CONNECTION:Cafe\\:Free
GENERAL.ZONE:public

GENERAL.CONNECTION:Home WiFi
GENERAL.ZONE:home
";
        let zones = parse_nmcli_device_zones(dev_show);
        let active = "\
wlan0:Cafe\\:Free:802-11-wireless:activated
eth0:Home WiFi:802-3-ethernet:activated
lo:lo:loopback:activated
tun0:wg0:tun:activated
";
        let nets = parse_nmcli_active_networks(active, &zones);
        assert_eq!(
            nets,
            vec![
                Network { name: "Cafe:Free".into(), trusted: false },
                Network { name: "Home WiFi".into(), trusted: true },
            ]
        );
    }
}
