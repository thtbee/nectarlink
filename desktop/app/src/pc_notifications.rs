// SPDX-License-Identifier: GPL-3.0-or-later
//! PC Notifications → Phone (§3.2 P2): forwards selected PC notifications to
//! connected phones when `forward_pc_notifications_to_phone` is enabled in
//! Settings (default `false`, privacy-first).

#[cfg(target_os = "linux")]
use std::{
    io::{BufRead, BufReader},
    process::{Command, Stdio},
    sync::OnceLock,
};
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use nectarlink_core::Notification;

use crate::{core_host, settings};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
#[cfg(target_os = "linux")]
static LISTENER_STARTED: OnceLock<()> = OnceLock::new();

/// Builds a sanitized [`Notification`] representing a PC notification.
pub fn build_pc_notification(
    key: &str,
    app_name: &str,
    title: &str,
    text: &str,
    when_ms: i64,
) -> Option<Notification> {
    let clean_app = if app_name.trim().is_empty() { "PC".to_owned() } else { app_name.trim().to_owned() };
    Notification {
        key: key.to_owned(),
        app: "pc.windows".to_owned(),
        app_name: clean_app,
        title: (!title.trim().is_empty()).then(|| title.trim().to_owned()),
        text: (!text.trim().is_empty()).then(|| text.trim().to_owned()),
        sub: None,
        when: when_ms,
        actions: Vec::new(),
        silent: false,
        icon: None,
        image: None,
        live: None,
        conversation: None,
    }
    .sanitized()
}

/// Returns `true` when the notification originated from NectarLink itself or a
/// blocked system self-source so we never create a forwarding loop.
#[cfg(any(target_os = "linux", test))]
pub fn is_blocked_self_source(app_name: &str) -> bool {
    let lower = app_name.trim().to_lowercase();
    lower.is_empty()
        || lower.contains("nectarlink")
        || lower == "notify-send"
        || lower == "org.freedesktop.notifications"
}

/// Parses a single-line or compact representation of a Freedesktop `Notify` D-Bus call
/// into `(app_name, summary, body)`.
///
/// Supports `dbus-monitor` `string "..."` fields from `org.freedesktop.Notifications.Notify`.
#[cfg(any(target_os = "linux", test))]
pub fn parse_dbus_notify_line(line: &str) -> Option<(String, String, String)> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut dbus_strings = Vec::new();
    let mut rest = trimmed;
    while let Some(idx) = rest.find("string \"") {
        let after = &rest[idx + "string \"".len()..];
        if let Some((val, tail)) = take_unescaped_quoted(after) {
            dbus_strings.push(val);
            rest = tail;
        } else {
            break;
        }
    }

    // In org.freedesktop.Notifications.Notify(app_name, replaces_id, app_icon, summary, body, ...):
    // string[0] = app_name, string[1] = app_icon, string[2] = summary, string[3] = body
    if dbus_strings.len() >= 4 {
        let app_name = dbus_strings[0].trim().to_string();
        let summary = dbus_strings[2].trim().to_string();
        let body = dbus_strings[3].trim().to_string();
        if summary.is_empty() && body.is_empty() {
            return None;
        }
        return Some((app_name, summary, body));
    }

    if dbus_strings.len() == 3 {
        let app_name = dbus_strings[0].trim().to_string();
        let summary = dbus_strings[1].trim().to_string();
        let body = dbus_strings[2].trim().to_string();
        if summary.is_empty() && body.is_empty() {
            return None;
        }
        return Some((app_name, summary, body));
    }

    None
}

#[cfg(any(target_os = "linux", test))]
fn take_unescaped_quoted(s: &str) -> Option<(String, &str)> {
    let mut out = String::new();
    let mut escaped = false;
    for (idx, ch) in s.char_indices() {
        if escaped {
            out.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            return Some((out, &s[idx + 1..]));
        } else {
            out.push(ch);
        }
    }
    None
}

/// Forwards a captured PC notification if it is not from a blocked self-source,
/// has non-empty content, and `forward_pc_notifications_to_phone` is enabled.
#[cfg(any(target_os = "linux", test))]
pub fn forward_pc_toast(app_name: &str, title: &str, text: &str) -> bool {
    if is_blocked_self_source(app_name) {
        return false;
    }
    if title.trim().is_empty() && text.trim().is_empty() {
        return false;
    }
    forward_to_phones(app_name.trim(), title.trim(), text.trim())
}

/// Forwards a PC notification (`app_name`, `title`, `text`) to connected
/// phones when `forward_pc_notifications_to_phone` is enabled. Returns `true`
/// if the notification was queued for delivery.
pub fn forward_to_phones(app_name: &str, title: &str, text: &str) -> bool {
    if !settings::forward_pc_notifications_to_phone() {
        return false;
    }
    let seq = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let when_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0);
    let key = format!("pc:win:{when_ms}:{seq}");
    let Some(notification) = build_pc_notification(&key, app_name, title, text, when_ms) else {
        return false;
    };
    if let Some(node) = core_host::node() {
        core_host::spawn(async move {
            node.notification_posted(notification).await;
        });
    }
    true
}

/// Starts a background listener for desktop notifications on Linux via `dbus-monitor`.
#[cfg(target_os = "linux")]
pub fn start_listener() {
    LISTENER_STARTED.get_or_init(|| {
        if !cfg!(test) {
            let _ = std::thread::Builder::new().name("pc-notifications".into()).spawn(dbus_monitor_loop);
        }
    });
}

#[cfg(target_os = "linux")]
fn dbus_monitor_loop() {
    let Ok(mut child) = Command::new("dbus-monitor")
        .args(["--session", "type='method_call',interface='org.freedesktop.Notifications',member='Notify'"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return;
    };

    let Some(stdout) = child.stdout.take() else {
        return;
    };

    let reader = BufReader::new(stdout);
    let mut current_block = String::new();
    let mut in_notify = false;
    let mut string_count = 0usize;

    for line in reader.lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.contains("member=Notify") {
            if in_notify && let Some((app, title, body)) = parse_dbus_notify_line(&current_block) {
                let _ = forward_pc_toast(&app, &title, &body);
            }
            current_block.clear();
            in_notify = true;
            string_count = 0;
            current_block.push_str(trimmed);
            current_block.push(' ');
            continue;
        }

        if in_notify {
            if trimmed.starts_with("string \"") {
                current_block.push_str(trimmed);
                current_block.push(' ');
                string_count += 1;
                if string_count >= 4 {
                    if let Some((app, title, body)) = parse_dbus_notify_line(&current_block) {
                        let _ = forward_pc_toast(&app, &title, &body);
                    }
                    current_block.clear();
                    in_notify = false;
                    string_count = 0;
                }
            } else if trimmed.starts_with("method call") || trimmed.starts_with("signal ") {
                if let Some((app, title, body)) = parse_dbus_notify_line(&current_block) {
                    let _ = forward_pc_toast(&app, &title, &body);
                }
                current_block.clear();
                in_notify = false;
                string_count = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_to_phones_respects_privacy_toggle() {
        // Default is false (privacy-first), so forward_to_phones is a no-op.
        assert!(!settings::forward_pc_notifications_to_phone());
        assert!(!forward_to_phones("Explorer", "Download complete", "report.pdf"));

        // Verify payload construction when enabled.
        let notif = build_pc_notification(
            "pc:win:1",
            "Task Watcher",
            "Build finished",
            "cargo test succeeded",
            1_700_000_000_000,
        )
        .expect("valid PC notification");
        assert_eq!(notif.app, "pc.windows");
        assert_eq!(notif.app_name, "Task Watcher");
        assert_eq!(notif.title.as_deref(), Some("Build finished"));
        assert_eq!(notif.text.as_deref(), Some("cargo test succeeded"));
    }

    #[test]
    fn blocks_self_source_notifications() {
        assert!(is_blocked_self_source("NectarLink"));
        assert!(is_blocked_self_source("nectarlink-desktop"));
        assert!(is_blocked_self_source("notify-send"));
        assert!(is_blocked_self_source(""));
        assert!(!is_blocked_self_source("Firefox"));
        assert!(!is_blocked_self_source("Signal Desktop"));
    }

    #[test]
    fn parses_dbus_notify_lines() {
        let sample = r#"method call time=1700000.0 sender=:1.42 -> destination=org.freedesktop.Notifications serial=10 path=/org/freedesktop/Notifications; interface=org.freedesktop.Notifications; member=Notify string "Firefox" uint32 0 string "firefox" string "Download complete" string "ubuntu.iso" array [] array [] int32 -1"#;
        let parsed = parse_dbus_notify_line(sample);
        assert_eq!(
            parsed,
            Some(("Firefox".to_string(), "Download complete".to_string(), "ubuntu.iso".to_string()))
        );
    }

    #[test]
    fn forward_pc_toast_ignores_self_and_empty() {
        assert!(!forward_pc_toast("NectarLink", "Connected", "Pixel 9"));
        assert!(!forward_pc_toast("Firefox", "   ", ""));
    }
}
