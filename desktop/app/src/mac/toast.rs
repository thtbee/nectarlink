// SPDX-License-Identifier: GPL-3.0-or-later
//! Phone notifications as macOS notifications, with action buttons, inline
//! replies, progress updates and click / dismiss events.
//!
//! The work is done by UserNotifications (`cpp/mac_notifications.mm`), which
//! only works for a process running from an `.app` bundle. From a bare binary
//! (`cargo run`) notifications fall back to plain `osascript` banners, which
//! carry no buttons, no events and no progress.

use std::{
    collections::HashMap,
    ffi::{CStr, CString, c_char},
    path::PathBuf,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

#[allow(dead_code)]
pub const AUMID: &str = "app.nectarlink.desktop";
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

impl std::fmt::Debug for Toast {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Toast").field("device", &self.device).finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    pub status: String,
    pub value: Option<f64>,
    pub label: String,
}

type OnEvent = Box<dyn Fn(ToastEvent) + Send + Sync>;
static ON_EVENT: OnceLock<OnEvent> = OnceLock::new();

/// Whether the system notification center is in use, as opposed to the
/// `osascript` fallback.
static NATIVE: AtomicBool = AtomicBool::new(false);

/// What a progress notification last showed, so an update can change its
/// bar and leave the rest as it was. Keyed by device and notification key.
static PROGRESS_TOASTS: Mutex<Option<HashMap<(String, String), Toast>>> = Mutex::new(None);

#[allow(unsafe_code)]
mod ffi {
    use std::ffi::c_char;

    /// `kind`: 0 opened, 1 action button, 2 inline reply, 3 dismissed.
    pub type EventCallback = unsafe extern "C" fn(
        kind: i32,
        device: *const c_char,
        key: *const c_char,
        action: *const c_char,
        text: *const c_char,
    );

    unsafe extern "C" {
        pub fn nl_notif_start(callback: EventCallback) -> bool;
        pub fn nl_notif_authorization() -> i32;
        pub fn nl_notif_show(
            device: *const c_char,
            key: *const c_char,
            title: *const c_char,
            subtitle: *const c_char,
            body: *const c_char,
            attachment_path: *const c_char,
            action_ids: *const *const c_char,
            action_titles: *const *const c_char,
            action_count: usize,
            reply_id: *const c_char,
            reply_title: *const c_char,
            silent: bool,
            call: bool,
            quiet: bool,
        );
        pub fn nl_notif_remove(device: *const c_char, key: *const c_char);
        pub fn nl_notif_remove_device(device: *const c_char);
        pub fn nl_notif_remove_all();
    }
}

/// Whether macOS shows this app's notifications. The user can turn them off
/// in System Settings > Notifications; until they've been asked, or when
/// running without a bundle, this says yes.
pub fn enabled() -> bool {
    if !NATIVE.load(Ordering::Acquire) {
        return true;
    }
    // SAFETY: takes no arguments and only reads the cached permission.
    #[allow(unsafe_code)]
    let state = unsafe { ffi::nl_notif_authorization() };
    state != 0
}

pub fn set_process_id() {}

/// Removes the app's notifications (when uninstalling).
pub fn unregister() {
    if NATIVE.load(Ordering::Acquire) {
        // SAFETY: takes no arguments.
        #[allow(unsafe_code)]
        unsafe {
            ffi::nl_notif_remove_all();
        }
    }
}

/// Takes over the system's notification center and asks for permission (the
/// system asks the user the first time only). `on_event` gets what the user
/// does with notifications. The icon is the app's own, which macOS takes from
/// the bundle, so it isn't written here.
pub fn start(_icon: PathBuf, on_event: impl Fn(ToastEvent) + Send + Sync + 'static) {
    let _ = ON_EVENT.set(Box::new(on_event));
    // SAFETY: the callback is a plain function that lives for the whole run.
    #[allow(unsafe_code)]
    let native = unsafe { ffi::nl_notif_start(on_native_event) };
    NATIVE.store(native, Ordering::Release);
    if native {
        tracing::info!("notifications use the macOS notification center");
    } else {
        tracing::info!("not running from an app bundle: notifications are plain banners without buttons");
    }
}

/// Called by UserNotifications on a thread of its own.
#[allow(unsafe_code)]
unsafe extern "C" fn on_native_event(
    kind: i32,
    device: *const c_char,
    key: *const c_char,
    action: *const c_char,
    text: *const c_char,
) {
    let read = |p: *const c_char| {
        if p.is_null() {
            String::new()
        } else {
            // SAFETY: a non-null pointer from the bridge is a NUL-terminated
            // string that lives for the duration of this call.
            unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
        }
    };
    let event = event_from_raw(kind, read(device), read(key), read(action), read(text));
    if let (Some(event), Some(handler)) = (event, ON_EVENT.get()) {
        handler(event);
    }
}

fn event_from_raw(kind: i32, device: String, key: String, action: String, text: String) -> Option<ToastEvent> {
    match kind {
        0 => Some(ToastEvent::Opened { device, key }),
        1 => Some(ToastEvent::Action { device, key, action }),
        2 => Some(ToastEvent::Reply { device, key, action, text }),
        3 => Some(ToastEvent::Dismissed { device, key }),
        _ => None,
    }
}

pub fn show(toast: Toast) {
    if toast.device.trim().is_empty() || toast.key.trim().is_empty() {
        return;
    }
    let id = (toast.device.clone(), toast.key.clone());
    {
        let mut shown = PROGRESS_TOASTS.lock().unwrap_or_else(|e| e.into_inner());
        let shown = shown.get_or_insert_with(HashMap::new);
        if toast.progress.is_some() {
            shown.insert(id, toast.clone());
        } else {
            shown.remove(&id);
        }
    }
    post(&toast, false);
}

/// Moves a shown notification's progress, without a new banner or a sound.
pub fn update_progress(device: &str, key: &str, progress: Progress) {
    let id = (device.to_owned(), key.to_owned());
    let mut shown = PROGRESS_TOASTS.lock().unwrap_or_else(|e| e.into_inner());
    let Some(toast) = shown.get_or_insert_with(HashMap::new).get_mut(&id) else {
        return;
    };
    toast.progress = Some(progress);
    let toast = toast.clone();
    drop(shown);
    post(&toast, true);
}

pub fn remove(device: &str, key: &str) {
    forget(|shown| {
        shown.remove(&(device.to_owned(), key.to_owned()));
    });
    if NATIVE.load(Ordering::Acquire) {
        let (device, key) = (c_string(device), c_string(key));
        // SAFETY: both are NUL-terminated and outlive the call.
        #[allow(unsafe_code)]
        unsafe {
            ffi::nl_notif_remove(device.as_ptr(), key.as_ptr());
        }
    }
}

/// Removes every notification of a device (it stopped sharing, or was unpaired).
pub fn remove_device(device: &str) {
    forget(|shown| shown.retain(|(d, _), _| d != device));
    if NATIVE.load(Ordering::Acquire) {
        let device = c_string(device);
        // SAFETY: NUL-terminated and outlives the call.
        #[allow(unsafe_code)]
        unsafe {
            ffi::nl_notif_remove_device(device.as_ptr());
        }
    }
}

fn forget(edit: impl FnOnce(&mut HashMap<(String, String), Toast>)) {
    let mut shown = PROGRESS_TOASTS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(shown) = shown.as_mut() {
        edit(shown);
    }
}

/// Posts a notification; `quiet` is an update of one that's already up.
fn post(toast: &Toast, quiet: bool) {
    let body = body_with_progress(&toast.body, toast.progress.as_ref());
    if !NATIVE.load(Ordering::Acquire) {
        if !quiet {
            banner(&toast.title, &body, &toast.attribution);
        }
        return;
    }

    let device = c_string(&toast.device);
    let key = c_string(&toast.key);
    let title = c_string(&toast.title);
    let subtitle = c_string(&toast.attribution);
    let body = c_string(&body);
    // A picture in the message, or else the phone app's icon (the caller's
    // photo for a call): macOS shows it as a thumbnail on the right.
    let attachment = toast
        .image
        .as_ref()
        .or(toast.icon.as_ref())
        .map(|p| c_string(&p.to_string_lossy()))
        .unwrap_or_default();
    let ids: Vec<CString> = toast.actions.iter().map(|(id, _)| c_string(id)).collect();
    let titles: Vec<CString> = toast.actions.iter().map(|(_, title)| c_string(title)).collect();
    let id_ptrs: Vec<*const c_char> = ids.iter().map(|s| s.as_ptr()).collect();
    let title_ptrs: Vec<*const c_char> = titles.iter().map(|s| s.as_ptr()).collect();
    let (reply_id, reply_title) = toast
        .reply
        .as_ref()
        .map(|(id, title)| (c_string(id), c_string(title)))
        .unwrap_or_default();
    // SAFETY: every pointer is to a NUL-terminated string (or an array of
    // them, `action_count` long) that outlives the call, and the bridge copies
    // what it keeps.
    #[allow(unsafe_code)]
    unsafe {
        ffi::nl_notif_show(
            device.as_ptr(),
            key.as_ptr(),
            title.as_ptr(),
            subtitle.as_ptr(),
            body.as_ptr(),
            attachment.as_ptr(),
            id_ptrs.as_ptr(),
            title_ptrs.as_ptr(),
            id_ptrs.len(),
            reply_id.as_ptr(),
            reply_title.as_ptr(),
            toast.silent,
            toast.call,
            quiet,
        );
    }
}

/// A C string for the bridge; a NUL inside the text would end it early, so it goes.
fn c_string(text: &str) -> CString {
    CString::new(text.replace('\0', "")).unwrap_or_default()
}

/// The body, and under it the progress: a bar of ten segments, then the
/// status and the label ("▰▰▰▱▱▱▱▱▱▱ Sending… 30%"). macOS has no progress bar
/// in a notification, so the text is replaced in place as it moves.
fn body_with_progress(body: &str, progress: Option<&Progress>) -> String {
    let Some(progress) = progress else {
        return body.to_owned();
    };
    let bar = progress.value.map(|value| {
        let filled = (value.clamp(0.0, 1.0) * 10.0).round() as usize;
        format!("{}{}", "▰".repeat(filled), "▱".repeat(10 - filled))
    });
    let line = [bar.as_deref(), Some(progress.status.as_str()), Some(progress.label.as_str())]
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    match (body.is_empty(), line.is_empty()) {
        (_, true) => body.to_owned(),
        (true, false) => line,
        (false, false) => format!("{body}\n{line}"),
    }
}

/// A plain banner through AppleScript, for a process without a bundle.
fn banner(title: &str, body: &str, subtitle: &str) {
    let script = banner_script(title, body, subtitle);
    let _ = std::thread::Builder::new().name("toast-banner".into()).spawn(move || {
        let _ = std::process::Command::new("osascript").args(["-e", &script]).status();
    });
}

fn banner_script(title: &str, body: &str, subtitle: &str) -> String {
    let mut script =
        format!("display notification {} with title {}", applescript_string(body), applescript_string(title));
    if !subtitle.is_empty() {
        script.push_str(&format!(" subtitle {}", applescript_string(subtitle)));
    }
    script
}

/// `text` as an AppleScript string literal. Rust's `{:?}` isn't one: it
/// writes `\u{..}` for characters AppleScript doesn't know.
fn applescript_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' | '\t' => out.push(' '),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress(value: Option<f64>, status: &str, label: &str) -> Progress {
        Progress { status: status.into(), value, label: label.into() }
    }

    #[test]
    fn events_map_from_the_bridge_kinds() {
        let d = || "dev".to_owned();
        let k = || "key".to_owned();
        assert_eq!(event_from_raw(0, d(), k(), String::new(), String::new()), Some(ToastEvent::Opened {
            device: d(),
            key: k()
        }));
        assert_eq!(
            event_from_raw(1, d(), k(), "accept".into(), String::new()),
            Some(ToastEvent::Action { device: d(), key: k(), action: "accept".into() })
        );
        assert_eq!(
            event_from_raw(2, d(), k(), "reply".into(), "on my way".into()),
            Some(ToastEvent::Reply { device: d(), key: k(), action: "reply".into(), text: "on my way".into() })
        );
        assert_eq!(
            event_from_raw(3, d(), k(), String::new(), String::new()),
            Some(ToastEvent::Dismissed { device: d(), key: k() })
        );
        assert_eq!(event_from_raw(9, d(), k(), String::new(), String::new()), None);
    }

    #[test]
    fn a_body_without_progress_is_left_alone() {
        assert_eq!(body_with_progress("hello", None), "hello");
    }

    #[test]
    fn progress_adds_a_bar_under_the_body() {
        let p = progress(Some(0.3), "Sending…", "30%");
        assert_eq!(body_with_progress("photo.jpg", Some(&p)), "photo.jpg\n▰▰▰▱▱▱▱▱▱▱ Sending… 30%");
        assert_eq!(body_with_progress("", Some(&p)), "▰▰▰▱▱▱▱▱▱▱ Sending… 30%");
    }

    #[test]
    fn progress_without_a_value_has_no_bar_and_stays_in_range() {
        let unknown = progress(None, "Waiting…", "");
        assert_eq!(body_with_progress("a", Some(&unknown)), "a\nWaiting…");
        let over = progress(Some(1.7), "", "done");
        assert_eq!(body_with_progress("a", Some(&over)), "a\n▰▰▰▰▰▰▰▰▰▰ done");
        let under = progress(Some(-1.0), "", "");
        assert_eq!(body_with_progress("a", Some(&under)), "a\n▱▱▱▱▱▱▱▱▱▱");
    }

    #[test]
    fn applescript_strings_escape_quotes_and_drop_control_characters() {
        assert_eq!(applescript_string(r#"say "hi" \o/"#), r#""say \"hi\" \\o/""#);
        assert_eq!(applescript_string("a\nb\tc\u{1b}d"), "\"a\\nb cd\"");
        assert_eq!(applescript_string("héllo 🐝"), "\"héllo 🐝\"");
    }

    #[test]
    fn the_banner_script_has_a_subtitle_only_when_there_is_one() {
        assert_eq!(banner_script("T", "B", ""), r#"display notification "B" with title "T""#);
        assert_eq!(banner_script("T", "B", "S"), r#"display notification "B" with title "T" subtitle "S""#);
    }

    #[test]
    fn a_nul_in_text_does_not_cut_the_string_short() {
        assert_eq!(c_string("a\0b").to_str().unwrap(), "ab");
    }
}
