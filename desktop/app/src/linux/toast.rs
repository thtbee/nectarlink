// SPDX-License-Identifier: GPL-3.0-or-later
//! Phone notifications as Freedesktop desktop notifications on Linux, with
//! action buttons, inline replies, progress updates, and D-Bus signal routing.

use std::collections::HashMap;
use std::fmt;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};

pub const AUMID: &str = "Nectarlink.Desktop";
pub const CALL_ANSWER: &str = "answer";
pub const CALL_DECLINE: &str = "decline";
pub const CALL_END: &str = "end";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToastEvent {
    Opened { device: String, key: String },
    Action { device: String, key: String, action: String },
    Reply { device: String, key: String, action: String, text: String },
    Dismissed { device: String, key: String },
}

#[derive(Clone)]
pub struct Toast {
    pub device: String,
    pub key: String,
    pub title: String,
    pub body: String,
    pub attribution: String,
    pub icon: Option<PathBuf>,
    pub image: Option<PathBuf>,
    pub actions: Vec<(String, String)>,
    pub reply: Option<(String, String)>,
    pub silent: bool,
    pub progress: Option<Progress>,
    pub call: bool,
}

impl fmt::Debug for Toast {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Toast")
            .field("device", &self.device)
            .field("key", &self.key)
            .field("title", &self.title)
            .field("body", &self.body)
            .field("attribution", &self.attribution)
            .field("icon", &self.icon)
            .field("image", &self.image)
            .field("actions", &self.actions.len())
            .field("reply", &self.reply.is_some())
            .field("silent", &self.silent)
            .field("progress", &self.progress)
            .field("call", &self.call)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    pub status: String,
    pub value: Option<f64>,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MonitorSignal {
    ActionInvoked { id: u32, action_key: String },
    NotificationReplied { id: u32, text: String },
    NotificationClosed { id: u32, reason: u32 },
}

#[derive(Clone, Debug)]
struct ToastSnapshot {
    title: String,
    body: String,
    attribution: String,
    icon_path: String,
    actions: Vec<(String, String)>,
    reply: Option<(String, String)>,
    silent: bool,
    call: bool,
}

struct Toaster {
    icon: PathBuf,
    on_event: Option<Box<dyn Fn(ToastEvent) + Send + Sync + 'static>>,
    active_ids: HashMap<(String, String), u32>,
    id_to_target: HashMap<u32, (String, String)>,
    reply_actions: HashMap<u32, String>,
    snapshots: HashMap<(String, String), ToastSnapshot>,
    next_fallback_id: u32,
}

impl Toaster {
    fn new(icon: PathBuf, on_event: Option<Box<dyn Fn(ToastEvent) + Send + Sync + 'static>>) -> Self {
        Self {
            icon,
            on_event,
            active_ids: HashMap::new(),
            id_to_target: HashMap::new(),
            reply_actions: HashMap::new(),
            snapshots: HashMap::new(),
            next_fallback_id: 100_000,
        }
    }

    fn record_shown(&mut self, device: String, key: String, id: u32, snapshot: ToastSnapshot) {
        let target = (device, key);
        if let Some(old_id) = self.active_ids.insert(target.clone(), id)
            && old_id != id
        {
            self.id_to_target.remove(&old_id);
            self.reply_actions.remove(&old_id);
        }
        self.id_to_target.insert(id, target.clone());
        if let Some((reply_action, _)) = &snapshot.reply {
            self.reply_actions.insert(id, reply_action.clone());
        } else {
            self.reply_actions.remove(&id);
        }
        self.snapshots.insert(target, snapshot);
    }

    fn take_by_target(&mut self, device: &str, key: &str) -> Option<u32> {
        let target = (device.to_string(), key.to_string());
        self.snapshots.remove(&target);
        if let Some(id) = self.active_ids.remove(&target) {
            self.id_to_target.remove(&id);
            self.reply_actions.remove(&id);
            Some(id)
        } else {
            None
        }
    }

    fn take_by_device(&mut self, device: &str) -> Vec<u32> {
        let keys: Vec<(String, String)> =
            self.active_ids.keys().filter(|(d, _)| d == device).cloned().collect();
        let mut ids = Vec::new();
        for target in keys {
            if let Some(id) = self.active_ids.remove(&target) {
                self.id_to_target.remove(&id);
                self.reply_actions.remove(&id);
                ids.push(id);
            }
            self.snapshots.remove(&target);
        }
        ids
    }

    fn clear_all(&mut self) -> Vec<u32> {
        let ids: Vec<u32> = self.active_ids.values().copied().collect();
        self.active_ids.clear();
        self.id_to_target.clear();
        self.reply_actions.clear();
        self.snapshots.clear();
        ids
    }

    fn handle_signal(&mut self, signal: MonitorSignal) -> Option<ToastEvent> {
        match signal {
            MonitorSignal::ActionInvoked { id, action_key } => {
                let (device, key) = self.id_to_target.get(&id)?.clone();
                if action_key == "default" || action_key.is_empty() {
                    Some(ToastEvent::Opened { device, key })
                } else {
                    Some(ToastEvent::Action { device, key, action: action_key })
                }
            }
            MonitorSignal::NotificationReplied { id, text } => {
                let (device, key) = self.id_to_target.get(&id)?.clone();
                let action =
                    self.reply_actions.get(&id).cloned().unwrap_or_else(|| "inline-reply".to_string());
                Some(ToastEvent::Reply { device, key, action, text })
            }
            MonitorSignal::NotificationClosed { id, reason } => {
                let (device, key) = self.id_to_target.remove(&id)?;
                self.active_ids.remove(&(device.clone(), key.clone()));
                self.reply_actions.remove(&id);
                self.snapshots.remove(&(device.clone(), key.clone()));
                // Freedesktop NotificationClosed reasons:
                // 1 = expired, 2 = dismissed by user, 3 = closed by CloseNotification, 4 = undefined
                if reason == 2 { Some(ToastEvent::Dismissed { device, key }) } else { None }
            }
        }
    }
}

static TOASTER: OnceLock<Mutex<Toaster>> = OnceLock::new();
static MONITOR_STARTED: OnceLock<()> = OnceLock::new();

pub fn enabled() -> bool {
    true
}

pub fn set_process_id() {}

pub fn start<F>(icon: PathBuf, on_event: F)
where
    F: Fn(ToastEvent) + Send + Sync + 'static,
{
    if cfg!(test) {
        if !icon.exists() {
            if let Some(parent) = icon.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = super::icon::write_png(&icon, 64);
        }
    } else {
        let bg_icon = icon.clone();
        let _ = std::thread::Builder::new().name("toast-icon".into()).spawn(move || {
            if !bg_icon.exists() {
                if let Some(parent) = bg_icon.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = super::icon::write_png(&bg_icon, 128);
            }
        });
    }

    let callback: Box<dyn Fn(ToastEvent) + Send + Sync + 'static> = Box::new(on_event);
    if let Some(mutex) = TOASTER.get() {
        if let Ok(mut guard) = mutex.lock() {
            guard.icon = icon;
            guard.on_event = Some(callback);
        }
    } else {
        let _ = TOASTER.set(Mutex::new(Toaster::new(icon, Some(callback))));
    }

    if !cfg!(test) {
        MONITOR_STARTED.get_or_init(|| {
            let _ = std::thread::Builder::new().name("toasts".into()).spawn(monitor_loop);
        });
    }
    crate::pc_notifications::start_listener();
}

pub fn show(toast: Toast) {
    if toast.device.trim().is_empty() || toast.key.trim().is_empty() {
        return;
    }

    let (replaces_id, default_icon) = {
        let Some(mutex) = TOASTER.get() else {
            let icon_str = toast
                .icon
                .as_ref()
                .or(toast.image.as_ref())
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            let title = format_summary(&toast.title, &toast.attribution);
            let body = format_body(&toast.body, &toast.attribution, toast.progress.as_ref());
            let _ = send_notification(
                0,
                &icon_str,
                &title,
                &body,
                &toast.actions,
                toast.reply.as_ref(),
                toast.progress.as_ref(),
                toast.silent,
                toast.call,
            );
            return;
        };
        let Ok(guard) = mutex.lock() else {
            return;
        };
        let existing = guard.active_ids.get(&(toast.device.clone(), toast.key.clone())).copied().unwrap_or(0);
        (existing, guard.icon.clone())
    };

    let chosen_icon = toast.icon.clone().or(toast.image.clone()).unwrap_or(default_icon);
    let icon_str = chosen_icon.to_string_lossy().to_string();
    let title = format_summary(&toast.title, &toast.attribution);
    let body = format_body(&toast.body, &toast.attribution, toast.progress.as_ref());

    let notified_id = send_notification(
        replaces_id,
        &icon_str,
        &title,
        &body,
        &toast.actions,
        toast.reply.as_ref(),
        toast.progress.as_ref(),
        toast.silent,
        toast.call,
    );

    if let Some(mutex) = TOASTER.get()
        && let Ok(mut guard) = mutex.lock()
    {
        let id = notified_id.unwrap_or_else(|| {
            if replaces_id != 0 {
                replaces_id
            } else {
                guard.next_fallback_id += 1;
                guard.next_fallback_id
            }
        });
        let snapshot = ToastSnapshot {
            title: toast.title,
            body: toast.body,
            attribution: toast.attribution,
            icon_path: icon_str,
            actions: toast.actions,
            reply: toast.reply,
            silent: toast.silent,
            call: toast.call,
        };
        guard.record_shown(toast.device, toast.key, id, snapshot);
    }
}

pub fn update_progress(device: &str, key: &str, progress: Progress) {
    let Some(mutex) = TOASTER.get() else {
        return;
    };
    let (replaces_id, snapshot) = {
        let Ok(guard) = mutex.lock() else {
            return;
        };
        let target = (device.to_string(), key.to_string());
        let Some(&id) = guard.active_ids.get(&target) else {
            return;
        };
        let Some(snap) = guard.snapshots.get(&target).cloned() else {
            return;
        };
        (id, snap)
    };

    let summary = format_summary(&snapshot.title, &snapshot.attribution);
    let body = format_body(&snapshot.body, &snapshot.attribution, Some(&progress));

    let new_id = send_notification(
        replaces_id,
        &snapshot.icon_path,
        &summary,
        &body,
        &snapshot.actions,
        snapshot.reply.as_ref(),
        Some(&progress),
        true,
        snapshot.call,
    );

    if let Some(id) = new_id
        && id != replaces_id
        && let Ok(mut guard) = mutex.lock()
    {
        guard.record_shown(device.to_string(), key.to_string(), id, snapshot);
    }
}

pub fn remove(device: &str, key: &str) {
    let Some(mutex) = TOASTER.get() else {
        return;
    };
    let id = {
        let Ok(mut guard) = mutex.lock() else {
            return;
        };
        guard.take_by_target(device, key)
    };
    if let Some(id) = id {
        close_notification(id);
    }
}

pub fn remove_device(device: &str) {
    let Some(mutex) = TOASTER.get() else {
        return;
    };
    let ids = {
        let Ok(mut guard) = mutex.lock() else {
            return;
        };
        guard.take_by_device(device)
    };
    for id in ids {
        close_notification(id);
    }
}

pub fn unregister() {
    let Some(mutex) = TOASTER.get() else {
        return;
    };
    let ids = {
        let Ok(mut guard) = mutex.lock() else {
            return;
        };
        guard.clear_all()
    };
    for id in ids {
        close_notification(id);
    }
}

fn format_summary(title: &str, attribution: &str) -> String {
    if !title.trim().is_empty() {
        title.trim().to_string()
    } else if !attribution.trim().is_empty() {
        attribution.trim().to_string()
    } else {
        "NectarLink".to_string()
    }
}

fn format_body(body: &str, attribution: &str, progress: Option<&Progress>) -> String {
    let mut parts = Vec::new();
    if !body.trim().is_empty() {
        parts.push(body.trim().to_string());
    }
    if let Some(prog) = progress {
        let mut prog_line = String::new();
        if !prog.status.trim().is_empty() {
            prog_line.push_str(prog.status.trim());
        }
        if !prog.label.trim().is_empty() {
            if !prog_line.is_empty() {
                prog_line.push_str(" — ");
            }
            prog_line.push_str(prog.label.trim());
        } else if let Some(val) = prog.value {
            let pct = (val.clamp(0.0, 1.0) * 100.0).round() as i32;
            if !prog_line.is_empty() {
                prog_line.push_str(&format!(" ({pct}%)"));
            } else {
                prog_line.push_str(&format!("{pct}%"));
            }
        }
        if !prog_line.is_empty() {
            parts.push(prog_line);
        }
    }
    if !attribution.trim().is_empty() {
        parts.push(attribution.trim().to_string());
    }
    parts.join("\n")
}

fn compute_progress_percent(progress: &Progress) -> i32 {
    progress.value.map(|v| (v.clamp(0.0, 1.0) * 100.0).round() as i32).unwrap_or(0)
}

fn build_actions_gvariant(actions: &[(String, String)], reply: Option<&(String, String)>) -> String {
    let mut items: Vec<String> = Vec::new();
    items.push("'default'".to_string());
    items.push("'Open'".to_string());
    for (id, label) in actions {
        if !id.trim().is_empty() {
            items.push(escape_gvariant_string(id));
            items.push(escape_gvariant_string(label));
        }
    }
    if let Some((_action_id, placeholder)) = reply {
        let label = if placeholder.trim().is_empty() { "Reply" } else { placeholder.trim() };
        items.push("'inline-reply'".to_string());
        items.push(escape_gvariant_string(label));
    }
    format!("[{}]", items.join(", "))
}

fn build_hints_gvariant(progress: Option<&Progress>, silent: bool, call: bool) -> String {
    let mut entries = Vec::new();
    let urgency = if call {
        2
    } else if silent {
        0
    } else {
        1
    };
    entries.push(format!("'urgency': <byte {urgency}>"));
    if silent && !call {
        entries.push("'suppress-sound': <true>".to_string());
    }
    if let Some(prog) = progress {
        let pct = compute_progress_percent(prog);
        entries.push(format!("'value': <int32 {pct}>"));
    }
    format!("{{{}}}", entries.join(", "))
}

fn escape_gvariant_string(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('\'', "\\'");
    format!("'{escaped}'")
}

#[allow(clippy::too_many_arguments)]
fn send_notification(
    replaces_id: u32,
    icon_path: &str,
    summary: &str,
    body: &str,
    actions: &[(String, String)],
    reply: Option<&(String, String)>,
    progress: Option<&Progress>,
    silent: bool,
    call: bool,
) -> Option<u32> {
    if cfg!(test) {
        return None;
    }

    let actions_arg = build_actions_gvariant(actions, reply);
    let hints_arg = build_hints_gvariant(progress, silent, call);
    let expire_timeout = if call { "int32 0" } else { "int32 -1" };

    if let Ok(output) = Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.Notifications",
            "--object-path",
            "/org/freedesktop/Notifications",
            "--method",
            "org.freedesktop.Notifications.Notify",
            "NectarLink",
            &replaces_id.to_string(),
            icon_path,
            summary,
            body,
            &actions_arg,
            &hints_arg,
            expire_timeout,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        && output.status.success()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if let Some(id) = parse_notify_id(&stdout) {
            return Some(id);
        }
    }

    // Fallback to notify-send
    let mut cmd = Command::new("notify-send");
    cmd.arg("--app-name=NectarLink").arg("--print-id");
    if replaces_id != 0 {
        cmd.arg(format!("--replace-id={replaces_id}"));
    }
    if !icon_path.is_empty() {
        cmd.arg(format!("--icon={icon_path}"));
    }
    if call {
        cmd.arg("--urgency=critical");
    } else if silent {
        cmd.arg("--urgency=low");
    }
    if let Some(prog) = progress {
        let pct = compute_progress_percent(prog);
        cmd.arg(format!("--hint=int:value:{pct}"));
    }
    cmd.arg(summary);
    if !body.is_empty() {
        cmd.arg(body);
    }
    if let Ok(output) = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).output()
        && output.status.success()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if let Some(id) = parse_notify_id(&stdout) {
            return Some(id);
        }
    }

    None
}

fn close_notification(id: u32) {
    if cfg!(test) || id == 0 {
        return;
    }
    let _ = Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.Notifications",
            "--object-path",
            "/org/freedesktop/Notifications",
            "--method",
            "org.freedesktop.Notifications.CloseNotification",
            &id.to_string(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn monitor_loop() {
    let Ok(mut child) = Command::new("gdbus")
        .args([
            "monitor",
            "--session",
            "--dest",
            "org.freedesktop.Notifications",
            "--object-path",
            "/org/freedesktop/Notifications",
        ])
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
    for line in reader.lines().map_while(Result::ok) {
        if let Some(signal) = parse_monitor_line(&line) {
            let event_and_cb = TOASTER.get().and_then(|mutex| {
                let mut guard = mutex.lock().ok()?;
                let ev = guard.handle_signal(signal)?;
                Some(ev)
            });
            if let Some(event) = event_and_cb
                && let Some(mutex) = TOASTER.get()
                && let Ok(guard) = mutex.lock()
                && let Some(cb) = &guard.on_event
            {
                cb(event);
            }
        }
    }
}

/// Parses the notification ID returned by `gdbus call ... Notify` (e.g. `(uint32 42,)`)
/// or `notify-send --print-id` (e.g. `42`).
pub fn parse_notify_id(output: &str) -> Option<u32> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(id) = trimmed.parse::<u32>() {
        return Some(id);
    }
    if let Some(idx) = trimmed.find("uint32 ") {
        let rest = &trimmed[idx + "uint32 ".len()..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(id) = digits.parse::<u32>() {
            return Some(id);
        }
    }
    None
}

/// Parses a single line from `gdbus monitor --session --dest org.freedesktop.Notifications`.
///
/// Expected signal formats:
/// - `/org/freedesktop/Notifications: org.freedesktop.Notifications.ActionInvoked (uint32 12, 'reply')`
/// - `/org/freedesktop/Notifications: org.freedesktop.Notifications.NotificationReplied (uint32 12, 'Hello!')`
/// - `/org/freedesktop/Notifications: org.freedesktop.Notifications.NotificationClosed (uint32 12, uint32 2)`
pub fn parse_monitor_line(line: &str) -> Option<MonitorSignal> {
    let trimmed = line.trim();
    if let Some(idx) = trimmed.find("ActionInvoked") {
        let args = &trimmed[idx + "ActionInvoked".len()..];
        let (id, rest) = parse_first_uint32(args)?;
        let action_key = parse_quoted_arg(rest)?;
        return Some(MonitorSignal::ActionInvoked { id, action_key });
    }
    if let Some(idx) = trimmed.find("NotificationReplied") {
        let args = &trimmed[idx + "NotificationReplied".len()..];
        let (id, rest) = parse_first_uint32(args)?;
        let text = parse_quoted_arg(rest)?;
        return Some(MonitorSignal::NotificationReplied { id, text });
    }
    if let Some(idx) = trimmed.find("NotificationClosed") {
        let args = &trimmed[idx + "NotificationClosed".len()..];
        let (id, rest) = parse_first_uint32(args)?;
        let (reason, _) = parse_first_uint32(rest).unwrap_or((0, ""));
        return Some(MonitorSignal::NotificationClosed { id, reason });
    }
    None
}

fn parse_first_uint32(s: &str) -> Option<(u32, &str)> {
    let start = s.find(|c: char| c.is_ascii_digit())?;
    let after_prefix = &s[start..];
    // Skip the `32` in `uint32 ` if `start` landed on `32`
    let after_uint = if start >= 4 && &s[start - 4..start + 2] == "uint32" {
        let rest_after_type = &s[start + 2..];
        let num_start = rest_after_type.find(|c: char| c.is_ascii_digit())?;
        &rest_after_type[num_start..]
    } else {
        after_prefix
    };
    let end = after_uint.find(|c: char| !c.is_ascii_digit()).unwrap_or(after_uint.len());
    let val = after_uint[..end].parse::<u32>().ok()?;
    Some((val, &after_uint[end..]))
}

fn parse_quoted_arg(s: &str) -> Option<String> {
    let comma = s.find(',')?;
    let after_comma = s[comma + 1..].trim();
    let quote_char = after_comma.chars().next()?;
    if quote_char != '\'' && quote_char != '"' {
        return None;
    }
    let content = &after_comma[quote_char.len_utf8()..];
    let mut out = String::new();
    let mut chars = content.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else if ch == quote_char {
            return Some(out);
        } else {
            out.push(ch);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_notify_ids_from_gdbus_and_notify_send() {
        assert_eq!(parse_notify_id("(uint32 42,)\n"), Some(42));
        assert_eq!(parse_notify_id("(uint32 108,)"), Some(108));
        assert_eq!(parse_notify_id("73\n"), Some(73));
        assert_eq!(parse_notify_id(""), None);
    }

    #[test]
    fn parses_gdbus_monitor_signals() {
        let action_line = "/org/freedesktop/Notifications: org.freedesktop.Notifications.ActionInvoked (uint32 15, 'mark_read')";
        assert_eq!(
            parse_monitor_line(action_line),
            Some(MonitorSignal::ActionInvoked { id: 15, action_key: "mark_read".to_string() })
        );

        let reply_line = "/org/freedesktop/Notifications: org.freedesktop.Notifications.NotificationReplied (uint32 15, 'Sounds good\\'s!')";
        assert_eq!(
            parse_monitor_line(reply_line),
            Some(MonitorSignal::NotificationReplied { id: 15, text: "Sounds good's!".to_string() })
        );

        let closed_line = "/org/freedesktop/Notifications: org.freedesktop.Notifications.NotificationClosed (uint32 15, uint32 2)";
        assert_eq!(
            parse_monitor_line(closed_line),
            Some(MonitorSignal::NotificationClosed { id: 15, reason: 2 })
        );
    }

    #[test]
    fn toaster_tracks_lifecycle_and_routes_signals() {
        let mut toaster = Toaster::new(PathBuf::from("/tmp/icon.png"), None);
        let snap = ToastSnapshot {
            title: "Messages".to_string(),
            body: "Hi there".to_string(),
            attribution: "Messages · Pixel 9".to_string(),
            icon_path: "/tmp/icon.png".to_string(),
            actions: vec![("archive".to_string(), "Archive".to_string())],
            reply: Some(("direct_reply".to_string(), "Type a message".to_string())),
            silent: false,
            call: false,
        };

        toaster.record_shown("phone-1".to_string(), "notif-a".to_string(), 77, snap);

        assert_eq!(
            toaster.handle_signal(MonitorSignal::ActionInvoked { id: 77, action_key: "default".to_string() }),
            Some(ToastEvent::Opened { device: "phone-1".to_string(), key: "notif-a".to_string() })
        );

        assert_eq!(
            toaster.handle_signal(MonitorSignal::ActionInvoked { id: 77, action_key: "archive".to_string() }),
            Some(ToastEvent::Action {
                device: "phone-1".to_string(),
                key: "notif-a".to_string(),
                action: "archive".to_string(),
            })
        );

        assert_eq!(
            toaster
                .handle_signal(MonitorSignal::NotificationReplied { id: 77, text: "On my way".to_string() }),
            Some(ToastEvent::Reply {
                device: "phone-1".to_string(),
                key: "notif-a".to_string(),
                action: "direct_reply".to_string(),
                text: "On my way".to_string(),
            })
        );

        assert_eq!(
            toaster.handle_signal(MonitorSignal::NotificationClosed { id: 77, reason: 2 }),
            Some(ToastEvent::Dismissed { device: "phone-1".to_string(), key: "notif-a".to_string() })
        );

        assert!(toaster.active_ids.is_empty());
        assert!(toaster.id_to_target.is_empty());
    }
}
