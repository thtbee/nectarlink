// SPDX-License-Identifier: GPL-3.0-or-later
use std::fs;
use std::io::Result;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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

pub fn parse_ufw_status(output: &str) -> Option<Firewall> {
    let lower = output.to_ascii_lowercase();
    if lower.contains("status: inactive") {
        return Some(Firewall { on: false, allowed: true, blocked: false });
    }
    if !lower.contains("status: active") {
        return None;
    }

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
        let touches_7718 = port_spec == "7718"
            || port_spec == "7718/tcp"
            || port_spec == "7718/udp"
            || port_spec.starts_with("7718:");
        if !touches_7718 {
            continue;
        }
        let upper_line = trimmed.to_ascii_uppercase();
        if upper_line.contains("DENY") || upper_line.contains("REJECT") {
            blocked = true;
        } else if upper_line.contains("ALLOW") {
            if port_spec == "7718" {
                tcp_allowed = true;
                udp_allowed = true;
            } else if port_spec == "7718/tcp" || port_spec.ends_with("/tcp") {
                tcp_allowed = true;
            } else if port_spec == "7718/udp" || port_spec.ends_with("/udp") {
                udp_allowed = true;
            }
        }
    }

    let allowed = !blocked && tcp_allowed && udp_allowed;
    Some(Firewall { on: true, allowed, blocked })
}

pub fn parse_firewalld_output(state: &str, ports: &str) -> Firewall {
    if !state.trim().eq_ignore_ascii_case("running") {
        return Firewall { on: false, allowed: true, blocked: false };
    }
    let allowed =
        ports.split_whitespace().any(|p| p.eq_ignore_ascii_case("7718/tcp") || p.starts_with("7718-"));
    Firewall { on: true, allowed, blocked: false }
}

fn check_ufw_config_fallback() -> Option<Firewall> {
    let conf = fs::read_to_string("/etc/ufw/ufw.conf").ok()?;
    let enabled = conf.lines().any(|l| {
        let t = l.trim();
        t.eq_ignore_ascii_case("ENABLED=yes") || t.eq_ignore_ascii_case("ENABLED=\"yes\"")
    });
    if !enabled {
        return Some(Firewall { on: false, allowed: true, blocked: false });
    }
    if let Ok(rules) = fs::read_to_string("/etc/ufw/user.rules") {
        let has_drop_7718 =
            rules.lines().any(|l| l.contains("7718") && (l.contains("-j DROP") || l.contains("-j REJECT")));
        let has_accept_7718 = rules.lines().any(|l| l.contains("7718") && l.contains("-j ACCEPT"));
        return Some(Firewall {
            on: true,
            allowed: !has_drop_7718 && has_accept_7718,
            blocked: has_drop_7718,
        });
    }
    Some(Firewall { on: true, allowed: false, blocked: false })
}

fn is_service_active(service: &str) -> bool {
    Command::new("systemctl").args(["is-active", "--quiet", service]).status().is_ok_and(|s| s.success())
}

fn has_command(cmd: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {cmd} >/dev/null 2>&1")])
        .status()
        .is_ok_and(|s| s.success())
}

pub fn firewall(_exe: &Path) -> Result<Firewall> {
    // 1. Try ufw status
    if let Ok(out) = Command::new("ufw").arg("status").output()
        && out.status.success()
    {
        let stdout = String::from_utf8_lossy(&out.stdout);
        if let Some(fw) = parse_ufw_status(&stdout) {
            return Ok(fw);
        }
    }
    if is_service_active("ufw")
        && let Some(fw) = check_ufw_config_fallback()
    {
        return Ok(fw);
    }

    // 2. Try firewalld (firewall-cmd --state & firewall-cmd --list-ports)
    if let Ok(state_out) = Command::new("firewall-cmd").arg("--state").output() {
        let state_str = String::from_utf8_lossy(&state_out.stdout);
        if state_str.trim().eq_ignore_ascii_case("running") {
            let ports_str = Command::new("firewall-cmd")
                .arg("--list-ports")
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                .unwrap_or_default();
            return Ok(parse_firewalld_output(&state_str, &ports_str));
        }
    }

    // 3. Try nftables
    if is_service_active("nftables")
        && let Ok(conf) = fs::read_to_string("/etc/nftables.conf")
    {
        let lower = conf.to_ascii_lowercase();
        let has_policy_drop = lower.contains("policy drop");
        let has_7718 = lower.contains("7718");
        if has_policy_drop || has_7718 {
            return Ok(Firewall { on: true, allowed: has_7718, blocked: false });
        }
    }

    Ok(Firewall { on: false, allowed: true, blocked: false })
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
                list.push(Network { name, trusted: true });
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
    if is_service_active("ufw") || has_command("ufw") {
        let status = super::elevated::run_elevated(
            Path::new("sh"),
            &["-c", "ufw allow 7718/tcp && ufw allow 7718/udp && ufw allow 5353/udp"],
        );
        return Ok(matches!(status, Ok(0)));
    }
    if is_service_active("firewalld") || has_command("firewall-cmd") {
        let status = super::elevated::run_elevated(
            Path::new("sh"),
            &[
                "-c",
                "firewall-cmd --permanent --add-port=7718/tcp --add-port=7718/udp --add-port=5353/udp && firewall-cmd --reload",
            ],
        );
        return Ok(matches!(status, Ok(0)));
    }
    if is_service_active("nftables") {
        let status = super::elevated::run_elevated(
            Path::new("sh"),
            &[
                "-c",
                "nft add rule inet filter input tcp dport 7718 accept; nft add rule inet filter input udp dport { 5353, 7718 } accept",
            ],
        );
        return Ok(matches!(status, Ok(0)));
    }
    Ok(true)
}

pub fn open_firewall_settings() -> bool {
    Command::new("gufw")
        .spawn()
        .or_else(|_| Command::new("firewall-config").spawn())
        .or_else(|_| Command::new("plasma-firewall").spawn())
        .or_else(|_| Command::new("gnome-control-center").arg("network").spawn())
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
    fn parses_firewalld_running_and_not_running() {
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
    }
}
