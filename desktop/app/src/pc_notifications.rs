// SPDX-License-Identifier: GPL-3.0-or-later
//! PC Notifications → Phone (§3.2 P2): forwards selected PC notifications to
//! connected phones when `forward_pc_notifications_to_phone` is enabled in
//! Settings (default `false`, privacy-first).

#[cfg(target_os = "linux")]
use std::sync::OnceLock;
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
        ongoing: false,
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

/// Deserializes a Freedesktop `org.freedesktop.Notifications.Notify` D-Bus method call
/// message into `(app_name, summary, body)`, preserving multi-line notification bodies.
#[cfg(target_os = "linux")]
pub fn extract_notify_call(msg: &zbus::message::Message) -> Option<(String, String, String)> {
    let header = msg.header();
    if header.message_type() != zbus::message::Type::MethodCall {
        return None;
    }
    if header.interface().map(|i| i.as_str()) != Some("org.freedesktop.Notifications") {
        return None;
    }
    if header.member().map(|m| m.as_str()) != Some("Notify") {
        return None;
    }
    type NotifyBody = (
        String,
        u32,
        String,
        String,
        String,
        Vec<String>,
        std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
        i32,
    );
    let (app_name, _replaces_id, _app_icon, summary, body, _actions, hints, _timeout) =
        msg.body().deserialize::<NotifyBody>().ok()?;
    if hints.contains_key("x-canonical-private-synchronous")
        || hints.get("transient").and_then(|v| bool::try_from(v).ok()).unwrap_or(false)
    {
        return None;
    }
    let app_name = app_name.trim().to_string();
    let summary = summary.trim().to_string();
    let body = body.trim().to_string();
    if summary.is_empty() && body.is_empty() {
        return None;
    }
    Some((app_name, summary, body))
}

/// Starts a background listener for desktop notifications on Linux via `zbus`.
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
    use std::time::Duration;

    loop {
        if !settings::forward_pc_notifications_to_phone() {
            std::thread::sleep(Duration::from_secs(1));
            continue;
        }

        let Ok(conn) = zbus::blocking::Connection::session() else {
            std::thread::sleep(Duration::from_secs(5));
            continue;
        };

        let rules = vec!["type='method_call',interface='org.freedesktop.Notifications',member='Notify'"];
        if conn
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus.Monitoring"),
                "BecomeMonitor",
                &(rules, 0u32),
            )
            .is_err()
        {
            std::thread::sleep(Duration::from_secs(5));
            continue;
        }

        for msg in zbus::blocking::MessageIterator::from(conn) {
            if !settings::forward_pc_notifications_to_phone() {
                break;
            }
            match msg {
                Ok(msg) => {
                    if let Some((app, title, body)) = extract_notify_call(&msg) {
                        let _ = forward_pc_toast(&app, &title, &body);
                    }
                }
                Err(_) => break,
            }
        }

        std::thread::sleep(Duration::from_secs(2));
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
    fn forward_pc_toast_ignores_self_and_empty() {
        assert!(!forward_pc_toast("NectarLink", "Connected", "Pixel 9"));
        assert!(!forward_pc_toast("Firefox", "   ", ""));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn extract_notify_call_preserves_multiline_body() {
        let hints: std::collections::HashMap<&str, zbus::zvariant::Value<'_>> =
            std::collections::HashMap::new();
        let actions: Vec<&str> = vec!["default", "Open"];
        let msg = zbus::message::Message::method_call("/org/freedesktop/Notifications", "Notify")
            .unwrap()
            .interface("org.freedesktop.Notifications")
            .unwrap()
            .destination("org.freedesktop.Notifications")
            .unwrap()
            .build(&(
                "Signal Desktop",
                0u32,
                "signal-desktop",
                "Alice",
                "Hey there!\nAre we still on for 7pm?\nLet me know.",
                actions,
                hints,
                -1i32,
            ))
            .unwrap();

        let extracted = extract_notify_call(&msg);
        assert_eq!(
            extracted,
            Some((
                "Signal Desktop".to_string(),
                "Alice".to_string(),
                "Hey there!\nAre we still on for 7pm?\nLet me know.".to_string(),
            ))
        );
    }
}
