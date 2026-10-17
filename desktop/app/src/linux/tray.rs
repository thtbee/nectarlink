// SPDX-License-Identifier: GPL-3.0-or-later
//! Linux system tray, global hotkeys, active external window tracking/focus
//! for Continuity Camera & File Shelf, and D-Bus sleep/resume + XDG Desktop
//! Portal appearance change listeners.

use std::{
    collections::HashMap,
    env,
    io::{BufRead, BufReader, Result, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
};

/// What the user or the system asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    Open,
    CommandPalette,
    Shelf,
    TakePhoto,
    ScanDocument,
    FindPhone,
    /// Open the link on the clipboard on the phone.
    OpenLinkOnPhone,
    Quit,
    /// The PC woke from sleep.
    Resumed,
    /// Dark mode, the accent color, the wallpaper or the animation setting
    /// changed.
    AppearanceChanged,
}

/// Menu texts (translated by the caller).
#[derive(Debug, Clone)]
pub struct MenuLabels {
    pub open: String,
    pub command_palette: String,
    pub shelf: String,
    pub take_photo: String,
    pub scan_document: String,
    pub find_phone: String,
    pub open_link: String,
    pub quit: String,
}

pub const MOD_ALT: u32 = 0x0001;
pub const MOD_CONTROL: u32 = 0x0002;
pub const MOD_SHIFT: u32 = 0x0004;
pub const MOD_WIN: u32 = 0x0008;
pub const MOD_NOREPEAT: u32 = 0x4000;

static LAST_EXTERNAL_WINDOW: Mutex<Option<(isize, String)>> = Mutex::new(None);
static WINDOW_ID_MAP: OnceLock<Mutex<HashMap<isize, String>>> = OnceLock::new();
static HOTKEY_SPECS: Mutex<(String, String, String, String)> =
    Mutex::new((String::new(), String::new(), String::new(), String::new()));

fn window_id_map() -> &'static Mutex<HashMap<isize, String>> {
    WINDOW_ID_MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

fn has_x11_display() -> bool {
    env::var_os("DISPLAY").is_some_and(|v| !v.is_empty())
}

fn has_wayland_display() -> bool {
    env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty())
}

fn has_desktop_session() -> bool {
    env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some_and(|v| !v.is_empty())
        || has_x11_display()
        || has_wayland_display()
}

fn is_nectarlink_title(title: &str) -> bool {
    let trimmed = title.trim();
    trimmed.is_empty() || trimmed.get(..10).is_some_and(|prefix| prefix.eq_ignore_ascii_case("nectarlink"))
}

fn parse_window_handle(raw: &str) -> Option<isize> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let handle = if let Some(hex) = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")) {
        isize::from_str_radix(hex, 16).ok()?
    } else if let Ok(num) = trimmed.parse::<isize>() {
        num
    } else {
        // Deterministic non-zero hash for non-numeric window identifiers (e.g. KWin UUIDs).
        let mut hash: usize = 0xcbf2_9ce4_8422_2325;
        for b in trimmed.as_bytes() {
            hash ^= usize::from(*b);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        let h = (hash as isize).wrapping_abs();
        if h == 0 { 1 } else { h }
    };
    if handle == 0 {
        return None;
    }
    if let Ok(mut map) = window_id_map().lock() {
        map.insert(handle, trimmed.to_owned());
    }
    Some(handle)
}

fn raw_window_id(hwnd: isize) -> String {
    if let Ok(map) = window_id_map().lock()
        && let Some(raw) = map.get(&hwnd)
    {
        return raw.clone();
    }
    hwnd.to_string()
}

fn run_command_stdout(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

/// Queries the currently active external window via `xdotool` (X11/XWayland)
/// or `kdotool` (KDE Wayland), ignoring NectarLink's own windows.
pub fn query_active_window() -> Option<(isize, String)> {
    if has_x11_display()
        && let Some(raw_id) = run_command_stdout("xdotool", &["getactivewindow"])
        && let Some(hwnd) = parse_window_handle(&raw_id)
        && let Some(title) = run_command_stdout("xdotool", &["getwindowname", &raw_id])
        && !is_nectarlink_title(&title)
    {
        return Some((hwnd, title));
    }
    if has_wayland_display()
        && let Some(raw_id) = run_command_stdout("kdotool", &["getactivewindow"])
        && let Some(hwnd) = parse_window_handle(&raw_id)
        && let Some(title) = run_command_stdout("kdotool", &["getwindowname", &raw_id])
        && !is_nectarlink_title(&title)
    {
        return Some((hwnd, title));
    }
    None
}

/// Records the current foreground window if it belongs to an external
/// user-facing application (not NectarLink itself).
pub fn record_foreground_window() {
    if let Some((hwnd, title)) = query_active_window() {
        let mut guard = LAST_EXTERNAL_WINDOW.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some((hwnd, title));
    }
}

/// Captures the last active external foreground window `(hwnd, title)` at the
/// moment a Continuity Camera request begins.
pub fn snapshot_target_window() -> Option<(isize, String)> {
    if let Some((hwnd, title)) = query_active_window() {
        let mut guard = LAST_EXTERNAL_WINDOW.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some((hwnd, title.clone()));
        return Some((hwnd, title));
    }
    LAST_EXTERNAL_WINDOW.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Returns true if `hwnd` still refers to an existing, visible top-level window.
pub fn is_window_alive_and_visible(hwnd: isize) -> bool {
    if hwnd == 0 {
        return false;
    }
    let raw_id = raw_window_id(hwnd);
    if has_x11_display()
        && let Some(title) = run_command_stdout("xdotool", &["getwindowname", &raw_id])
        && !title.is_empty()
    {
        return true;
    }
    if has_wayland_display()
        && let Some(title) = run_command_stdout("kdotool", &["getwindowname", &raw_id])
        && !title.is_empty()
    {
        return true;
    }
    LAST_EXTERNAL_WINDOW
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .is_some_and(|(recorded_hwnd, _)| *recorded_hwnd == hwnd)
}

/// Restores and brings `hwnd` to the foreground so a paste shortcut can be
/// synthesized into it. Returns false if the window is gone.
pub fn focus_window(hwnd: isize) -> bool {
    if !is_window_alive_and_visible(hwnd) {
        return false;
    }
    let raw_id = raw_window_id(hwnd);
    if has_x11_display() {
        if Command::new("xdotool")
            .args(["windowactivate", &raw_id])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
        {
            return true;
        }
        if Command::new("wmctrl")
            .args(["-i", "-a", &raw_id])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
        {
            return true;
        }
    }
    if has_wayland_display()
        && Command::new("kdotool")
            .args(["windowactivate", &raw_id])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    {
        return true;
    }
    LAST_EXTERNAL_WINDOW
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .is_some_and(|(recorded_hwnd, _)| *recorded_hwnd == hwnd)
}

/// If the window manager prevented `focus_window` from making `hwnd` the
/// active window on X11, sends `Ctrl+V` directly to `hwnd`.
pub fn send_paste_fallback_if_not_foreground(hwnd: isize) {
    if hwnd == 0 || !has_x11_display() {
        return;
    }
    if query_active_window().is_some_and(|(active, _)| active == hwnd) {
        return;
    }
    let raw_id = raw_window_id(hwnd);
    let _ = Command::new("xdotool")
        .args(["key", "--window", &raw_id, "--clearmodifiers", "ctrl+v"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Applies `_NET_WM_STATE_ABOVE` and `_NET_WM_STATE_SKIP_TASKBAR` to a
/// top-level window matching `title` on X11 so the Shelf stays above normal
/// windows and off the taskbar without blocking or panicking.
pub fn apply_no_activate_style_by_title(title: &str) {
    let trimmed = title.trim();
    if trimmed.is_empty() || !has_x11_display() {
        return;
    }
    let Some(ids) = run_command_stdout("xdotool", &["search", "--name", trimmed]) else {
        return;
    };
    for id in ids.lines().map(str::trim).filter(|s| !s.is_empty()) {
        let _ = Command::new("xprop")
            .args([
                "-id",
                id,
                "-f",
                "_NET_WM_STATE",
                "32a",
                "-set",
                "_NET_WM_STATE",
                "_NET_WM_STATE_ABOVE,_NET_WM_STATE_SKIP_TASKBAR",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// Parses a hotkey string such as `"Ctrl+Alt+C"`, `"Ctrl+Alt+Space"` or `"Ctrl+Shift+F9"` into
/// `(HOT_KEY_MODIFIERS bits including MOD_NOREPEAT, virtual_key_code)`.
pub fn parse_hotkey(spec: &str) -> Option<(u32, u32)> {
    let trimmed = spec.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
        return None;
    }
    let mut mods: u32 = 0;
    let mut key_vk: Option<u32> = None;

    for raw_part in trimmed.split('+') {
        let part = raw_part.trim();
        if part.is_empty() {
            return None;
        }
        let lower = part.to_ascii_lowercase();
        match lower.as_str() {
            "ctrl" | "control" => mods |= MOD_CONTROL,
            "alt" => mods |= MOD_ALT,
            "shift" => mods |= MOD_SHIFT,
            "win" | "super" | "meta" => mods |= MOD_WIN,
            _ => {
                if key_vk.is_some() {
                    return None;
                }
                let vk = match lower.as_str() {
                    "space" | "spacebar" => 0x20,
                    "tab" => 0x09,
                    "enter" | "return" => 0x0D,
                    "esc" | "escape" => 0x1B,
                    "insert" | "ins" => 0x2D,
                    "delete" | "del" => 0x2E,
                    "home" => 0x24,
                    "end" => 0x23,
                    "pageup" | "pgup" => 0x21,
                    "pagedown" | "pgdn" => 0x22,
                    "left" => 0x25,
                    "up" => 0x26,
                    "right" => 0x27,
                    "down" => 0x28,
                    "plus" | "=" => 0xBB,
                    "minus" | "-" => 0xBD,
                    "comma" | "," => 0xBC,
                    "period" | "." => 0xBE,
                    _ if part.len() == 1 => {
                        let b = part.as_bytes()[0];
                        if b.is_ascii_alphabetic() {
                            u32::from(b.to_ascii_uppercase())
                        } else if b.is_ascii_digit() {
                            u32::from(b)
                        } else {
                            return None;
                        }
                    }
                    _ => {
                        let num_str = lower.strip_prefix('f')?;
                        let n: u32 = num_str.parse().ok()?;
                        if (1..=12).contains(&n) {
                            0x70 + (n - 1)
                        } else {
                            return None;
                        }
                    }
                };
                key_vk = Some(vk);
            }
        }
    }

    let vk = key_vk?;
    if mods == 0 {
        return None;
    }
    Some((mods | MOD_NOREPEAT, vk))
}

/// Updates the Continuity Camera, Command Palette, and Shelf global hotkeys
/// (`photo_spec`, `scan_spec`, `palette_spec`, and `shelf_spec`).
pub fn update_hotkeys(photo_spec: &str, scan_spec: &str, palette_spec: &str, shelf_spec: &str) {
    let mut specs = HOTKEY_SPECS.lock().unwrap_or_else(|e| e.into_inner());
    *specs = (
        photo_spec.trim().to_owned(),
        scan_spec.trim().to_owned(),
        palette_spec.trim().to_owned(),
        shelf_spec.trim().to_owned(),
    );
}

/// Returns true when a line from `gdbus monitor` or `dbus-monitor` indicates
/// `org.freedesktop.login1.Manager.PrepareForSleep(false)` (system waking up).
pub fn is_prepare_for_sleep_resume_line(line: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.contains("PrepareForSleep") {
        return false;
    }
    trimmed.contains("(false") || trimmed.contains("boolean false") || trimmed.ends_with(" false")
}

/// Returns true when a line from `gdbus monitor` or `dbus-monitor` indicates
/// an `org.freedesktop.portal.Settings.SettingChanged` signal for
/// `"org.freedesktop.appearance"` (`"color-scheme"`, `"contrast"`, or `"accent-color"`).
pub fn is_portal_appearance_changed_line(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.contains("SettingChanged")
        && trimmed.contains("org.freedesktop.appearance")
        && (trimmed.contains("color-scheme")
            || trimmed.contains("contrast")
            || trimmed.contains("accent-color"))
}

type TrayEventHandler = Box<dyn Fn(TrayEvent)>;

struct SharedHandler {
    inner: Mutex<Option<TrayEventHandler>>,
}

// SAFETY: Access to `inner` is serialized by its `Mutex`, and all invocations
// either run on the owning thread or forward thread-safe `TrayEvent` variants.
unsafe impl Send for SharedHandler {}
unsafe impl Sync for SharedHandler {}

impl SharedHandler {
    fn new(handler: impl Fn(TrayEvent) + 'static) -> Arc<Self> {
        Arc::new(Self { inner: Mutex::new(Some(Box::new(handler))) })
    }

    fn emit(&self, event: TrayEvent) {
        if let Ok(guard) = self.inner.lock()
            && let Some(cb) = guard.as_ref()
        {
            cb(event);
        }
    }

    fn clear(&self) {
        if let Ok(mut guard) = self.inner.lock() {
            *guard = None;
        }
    }
}

fn tray_icon_cache_path() -> PathBuf {
    dirs::cache_dir().unwrap_or_else(env::temp_dir).join("nectarlink").join("tray-icon.png")
}

fn spawn_dbus_monitor_reader(
    mut child: Child,
    stop: Arc<AtomicBool>,
    handler: Arc<SharedHandler>,
    is_login1: bool,
) -> Option<JoinHandle<()>> {
    let stdout = child.stdout.take()?;
    std::thread::Builder::new()
        .name(if is_login1 { "tray-login1-monitor".into() } else { "tray-portal-monitor".into() })
        .spawn(move || {
            let reader = BufReader::new(stdout);
            let mut awaiting_sleep_bool = false;
            for line in reader.lines() {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                let Ok(line) = line else { break };
                if is_login1 {
                    if is_prepare_for_sleep_resume_line(&line) {
                        awaiting_sleep_bool = false;
                        handler.emit(TrayEvent::Resumed);
                    } else if line.contains("PrepareForSleep") {
                        awaiting_sleep_bool = !line.contains("true");
                    } else if awaiting_sleep_bool {
                        let trimmed = line.trim();
                        if trimmed.contains("boolean false") {
                            awaiting_sleep_bool = false;
                            handler.emit(TrayEvent::Resumed);
                        } else if trimmed.contains("boolean true") {
                            awaiting_sleep_bool = false;
                        }
                    }
                } else if is_portal_appearance_changed_line(&line) {
                    handler.emit(TrayEvent::AppearanceChanged);
                }
            }
        })
        .ok()
}

const TRAY_HELPER_SCRIPT: &str = r#"
import os, sys, threading
import gi
gi.require_version('Gtk', '3.0')
from gi.repository import Gtk, GLib

icon_path = sys.argv[1]
tooltip = sys.argv[2]
labels = sys.argv[3:11]
icon_dir = os.path.dirname(icon_path)
icon_name = os.path.splitext(os.path.basename(icon_path))[0]

def emit(action):
    try:
        sys.stdout.write(action + "\n")
        sys.stdout.flush()
    except Exception:
        pass

menu = Gtk.Menu()
def add_item(label, action):
    item = Gtk.MenuItem(label=label)
    item.connect('activate', lambda _: emit(action))
    menu.append(item)
    return item

open_item = add_item(labels[0], 'OPEN')
add_item(labels[1], 'COMMAND_PALETTE')
add_item(labels[2], 'SHELF')
menu.append(Gtk.SeparatorMenuItem())
add_item(labels[3], 'TAKE_PHOTO')
add_item(labels[4], 'SCAN_DOCUMENT')
find_phone_item = add_item(labels[5], 'FIND_PHONE')
find_phone_item.set_sensitive(False)
add_item(labels[6], 'OPEN_LINK')
menu.append(Gtk.SeparatorMenuItem())
add_item(labels[7], 'QUIT')
menu.show_all()

ind = None
status_icon = None
try:
    try:
        gi.require_version('AyatanaAppIndicator3', '0.1')
        from gi.repository import AyatanaAppIndicator3 as AppIndicator
    except Exception:
        gi.require_version('AppIndicator3', '0.1')
        from gi.repository import AppIndicator3 as AppIndicator
    ind = AppIndicator.Indicator.new_with_path(
        'nectarlink',
        icon_name,
        AppIndicator.IndicatorCategory.APPLICATION_STATUS,
        icon_dir,
    )
    ind.set_status(AppIndicator.IndicatorStatus.ACTIVE)
    ind.set_title(tooltip)
    ind.set_icon_full(icon_path, tooltip)
    ind.set_menu(menu)
    try:
        ind.set_secondary_activate_target(open_item)
    except Exception:
        pass
except Exception:
    status_icon = Gtk.StatusIcon.new_from_file(icon_path)
    status_icon.set_tooltip_text(tooltip)
    status_icon.set_visible(True)
    status_icon.connect('activate', lambda _: emit('OPEN'))
    status_icon.connect(
        'popup-menu',
        lambda icon, button, time: menu.popup(None, None, Gtk.StatusIcon.position_menu, icon, button, time),
    )

def apply_cmd(line):
    parts = line.rstrip('\r\n').split('\t', 1)
    if len(parts) != 2:
        return False
    cmd, val = parts
    if cmd == 'TOOLTIP':
        if ind is not None:
            ind.set_title(val)
            ind.set_icon_full(icon_path, val)
        if status_icon is not None:
            status_icon.set_tooltip_text(val)
    elif cmd == 'FIND_PHONE':
        find_phone_item.set_sensitive(val == '1')
    return False

def stdin_reader():
    try:
        for line in sys.stdin:
            GLib.idle_add(apply_cmd, line)
    except Exception:
        pass
    GLib.idle_add(Gtk.main_quit)

t = threading.Thread(target=stdin_reader, daemon=True)
t.start()
Gtk.main()
"#;

pub(crate) fn parse_tray_helper_action(line: &str) -> Option<TrayEvent> {
    match line.trim() {
        "OPEN" => Some(TrayEvent::Open),
        "COMMAND_PALETTE" => Some(TrayEvent::CommandPalette),
        "SHELF" => Some(TrayEvent::Shelf),
        "TAKE_PHOTO" => Some(TrayEvent::TakePhoto),
        "SCAN_DOCUMENT" => Some(TrayEvent::ScanDocument),
        "FIND_PHONE" => Some(TrayEvent::FindPhone),
        "OPEN_LINK" => Some(TrayEvent::OpenLinkOnPhone),
        "QUIT" => Some(TrayEvent::Quit),
        _ => None,
    }
}

pub struct Tray {
    tooltip: Mutex<String>,
    labels: Mutex<MenuLabels>,
    find_phone_enabled: Mutex<bool>,
    icon_path: PathBuf,
    stop: Arc<AtomicBool>,
    handler: Arc<SharedHandler>,
    helper_stdin: Mutex<Option<ChildStdin>>,
    children: Mutex<Vec<Child>>,
    threads: Mutex<Vec<JoinHandle<()>>>,
}

impl std::fmt::Debug for Tray {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tray")
            .field("tooltip", &self.tooltip)
            .field("find_phone_enabled", &self.find_phone_enabled)
            .field("icon_path", &self.icon_path)
            .finish()
    }
}

impl Tray {
    pub fn create(tooltip: &str, labels: MenuLabels, handler: impl Fn(TrayEvent) + 'static) -> Result<Tray> {
        let icon_path = tray_icon_cache_path();
        if let Err(e) = super::icon::write_png(&icon_path, 32) {
            tracing::debug!(error = %e, path = %icon_path.display(), "could not write tray icon PNG");
        }

        let stop = Arc::new(AtomicBool::new(false));
        let shared_handler = SharedHandler::new(handler);
        let mut helper_stdin = None;
        let mut children = Vec::new();
        let mut threads = Vec::new();

        if !cfg!(test) && has_desktop_session() {
            record_foreground_window();

            // 0. StatusNotifierItem / Gtk.StatusIcon system tray indicator helper.
            let tray_child = Command::new("python3")
                .args([
                    "-u",
                    "-c",
                    TRAY_HELPER_SCRIPT,
                    &icon_path.to_string_lossy(),
                    tooltip,
                    &labels.open,
                    &labels.command_palette,
                    &labels.shelf,
                    &labels.take_photo,
                    &labels.scan_document,
                    &labels.find_phone,
                    &labels.open_link,
                    &labels.quit,
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn();

            if let Ok(mut child) = tray_child {
                helper_stdin = child.stdin.take();
                let stdout = child.stdout.take();
                children.push(child);
                if let Some(stdout) = stdout {
                    let reader = BufReader::new(stdout);
                    let stop_clone = Arc::clone(&stop);
                    let handler_clone = Arc::clone(&shared_handler);
                    if let Ok(handle) =
                        std::thread::Builder::new().name("tray-sni-helper".into()).spawn(move || {
                            for line in reader.lines() {
                                if stop_clone.load(Ordering::Acquire) {
                                    break;
                                }
                                let Ok(line) = line else { break };
                                if let Some(ev) = parse_tray_helper_action(&line) {
                                    if matches!(
                                        ev,
                                        TrayEvent::CommandPalette
                                            | TrayEvent::TakePhoto
                                            | TrayEvent::ScanDocument
                                    ) {
                                        record_foreground_window();
                                    }
                                    handler_clone.emit(ev);
                                }
                            }
                        })
                    {
                        threads.push(handle);
                    }
                }
            }

            // 1. System bus org.freedesktop.login1.Manager PrepareForSleep monitor.
            let login1_child = Command::new("gdbus")
                .args([
                    "monitor",
                    "--system",
                    "--dest",
                    "org.freedesktop.login1",
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .or_else(|_| {
                    Command::new("dbus-monitor")
                        .args([
                            "--system",
                            "type='signal',interface='org.freedesktop.login1.Manager',member='PrepareForSleep'",
                        ])
                        .stdin(Stdio::null())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::null())
                        .spawn()
                });
            if let Ok(mut child) = login1_child {
                let stdout = child.stdout.take();
                children.push(child);
                if let Some(stdout) = stdout {
                    let fake_child_reader = BufReader::new(stdout);
                    let stop_clone = Arc::clone(&stop);
                    let handler_clone = Arc::clone(&shared_handler);
                    if let Ok(handle) =
                        std::thread::Builder::new().name("tray-login1-monitor".into()).spawn(move || {
                            let mut awaiting_sleep_bool = false;
                            for line in fake_child_reader.lines() {
                                if stop_clone.load(Ordering::Acquire) {
                                    break;
                                }
                                let Ok(line) = line else { break };
                                if is_prepare_for_sleep_resume_line(&line) {
                                    awaiting_sleep_bool = false;
                                    handler_clone.emit(TrayEvent::Resumed);
                                } else if line.contains("PrepareForSleep") {
                                    awaiting_sleep_bool = !line.contains("true");
                                } else if awaiting_sleep_bool {
                                    let trimmed = line.trim();
                                    if trimmed.contains("boolean false") {
                                        awaiting_sleep_bool = false;
                                        handler_clone.emit(TrayEvent::Resumed);
                                    } else if trimmed.contains("boolean true") {
                                        awaiting_sleep_bool = false;
                                    }
                                }
                            }
                        })
                    {
                        threads.push(handle);
                    }
                }
            }

            // 2. Session bus org.freedesktop.portal.Settings SettingChanged monitor.
            let portal_child = Command::new("gdbus")
                .args([
                    "monitor",
                    "--session",
                    "--dest",
                    "org.freedesktop.portal.Desktop",
                    "--object-path",
                    "/org/freedesktop/portal/desktop",
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn();
            if let Ok(mut child) = portal_child {
                let stdout = child.stdout.take();
                children.push(child);
                if let Some(stdout) = stdout {
                    let reader = BufReader::new(stdout);
                    let stop_clone = Arc::clone(&stop);
                    let handler_clone = Arc::clone(&shared_handler);
                    if let Ok(handle) =
                        std::thread::Builder::new().name("tray-portal-monitor".into()).spawn(move || {
                            for line in reader.lines() {
                                if stop_clone.load(Ordering::Acquire) {
                                    break;
                                }
                                let Ok(line) = line else { break };
                                if is_portal_appearance_changed_line(&line) {
                                    handler_clone.emit(TrayEvent::AppearanceChanged);
                                }
                            }
                        })
                    {
                        threads.push(handle);
                    }
                }
            }
        }

        let _ = spawn_dbus_monitor_reader;

        Ok(Tray {
            tooltip: Mutex::new(tooltip.to_string()),
            labels: Mutex::new(labels),
            find_phone_enabled: Mutex::new(false),
            icon_path,
            stop,
            handler: shared_handler,
            helper_stdin: Mutex::new(helper_stdin),
            children: Mutex::new(children),
            threads: Mutex::new(threads),
        })
    }

    pub fn set_tooltip(&self, text: &str) {
        if let Ok(mut guard) = self.tooltip.lock() {
            *guard = text.to_string();
        }
        if let Ok(mut guard) = self.helper_stdin.lock()
            && let Some(stdin) = guard.as_mut()
        {
            let sanitized = text.replace(['\r', '\n', '\t'], " ");
            let _ = writeln!(stdin, "TOOLTIP\t{sanitized}");
            let _ = stdin.flush();
        }
    }

    /// Whether "Find my phone" is offered (a phone is paired).
    pub fn set_find_phone_enabled(&self, enabled: bool) {
        if let Ok(mut guard) = self.find_phone_enabled.lock() {
            *guard = enabled;
        }
        if let Ok(mut guard) = self.helper_stdin.lock()
            && let Some(stdin) = guard.as_mut()
        {
            let flag = if enabled { "1" } else { "0" };
            let _ = writeln!(stdin, "FIND_PHONE\t{flag}");
            let _ = stdin.flush();
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.handler.clear();
        if let Ok(mut stdin_guard) = self.helper_stdin.lock() {
            let _ = stdin_guard.take();
        }
        if let Ok(mut children) = self.children.lock() {
            for child in children.iter_mut() {
                let _ = child.kill();
                let _ = child.wait();
            }
            children.clear();
        }
        if let Ok(mut threads) = self.threads.lock() {
            for handle in threads.drain(..) {
                let _ = handle.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hotkey_recognizes_modifiers_and_keys() {
        let (mods, vk) = parse_hotkey("Ctrl+Alt+C").expect("Ctrl+Alt+C");
        assert_eq!(mods, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT);
        assert_eq!(vk, u32::from(b'C'));

        let (mods, vk) = parse_hotkey("Ctrl+Alt+Space").expect("Ctrl+Alt+Space");
        assert_eq!(mods, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT);
        assert_eq!(vk, 0x20);

        let (mods, vk) = parse_hotkey(" ctrl + shift + f10 ").expect("ctrl+shift+f10");
        assert_eq!(mods, MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT);
        assert_eq!(vk, 0x79);

        let (mods, vk) = parse_hotkey("Super+Shift+F12").expect("Super+Shift+F12");
        assert_eq!(mods, MOD_WIN | MOD_SHIFT | MOD_NOREPEAT);
        assert_eq!(vk, 0x70 + 11);

        let (mods, vk) = parse_hotkey("Alt+Tab").expect("Alt+Tab");
        assert_eq!(mods, MOD_ALT | MOD_NOREPEAT);
        assert_eq!(vk, 0x09);

        let (_, vk) = parse_hotkey("Ctrl+Enter").expect("Ctrl+Enter");
        assert_eq!(vk, 0x0D);
        let (_, vk) = parse_hotkey("Ctrl+Esc").expect("Ctrl+Esc");
        assert_eq!(vk, 0x1B);
        let (_, vk) = parse_hotkey("Ctrl+Ins").expect("Ctrl+Ins");
        assert_eq!(vk, 0x2D);
        let (_, vk) = parse_hotkey("Ctrl+Del").expect("Ctrl+Del");
        assert_eq!(vk, 0x2E);
        let (_, vk) = parse_hotkey("Ctrl+Home").expect("Ctrl+Home");
        assert_eq!(vk, 0x24);
        let (_, vk) = parse_hotkey("Ctrl+End").expect("Ctrl+End");
        assert_eq!(vk, 0x23);
        let (_, vk) = parse_hotkey("Ctrl+PgUp").expect("Ctrl+PgUp");
        assert_eq!(vk, 0x21);
        let (_, vk) = parse_hotkey("Ctrl+PgDn").expect("Ctrl+PgDn");
        assert_eq!(vk, 0x22);
        let (_, vk) = parse_hotkey("Ctrl+Left").expect("Ctrl+Left");
        assert_eq!(vk, 0x25);
        let (_, vk) = parse_hotkey("Ctrl+Up").expect("Ctrl+Up");
        assert_eq!(vk, 0x26);
        let (_, vk) = parse_hotkey("Ctrl+Right").expect("Ctrl+Right");
        assert_eq!(vk, 0x27);
        let (_, vk) = parse_hotkey("Ctrl+Down").expect("Ctrl+Down");
        assert_eq!(vk, 0x28);
        let (_, vk) = parse_hotkey("Ctrl+Plus").expect("Ctrl+Plus");
        assert_eq!(vk, 0xBB);
        let (_, vk) = parse_hotkey("Ctrl+Minus").expect("Ctrl+Minus");
        assert_eq!(vk, 0xBD);
        let (_, vk) = parse_hotkey("Ctrl+Comma").expect("Ctrl+Comma");
        assert_eq!(vk, 0xBC);
        let (_, vk) = parse_hotkey("Ctrl+Period").expect("Ctrl+Period");
        assert_eq!(vk, 0xBE);
    }

    #[test]
    fn parse_hotkey_rejects_missing_modifier_or_key() {
        assert!(parse_hotkey("").is_none());
        assert!(parse_hotkey("none").is_none());
        assert!(parse_hotkey("NONE").is_none());
        assert!(parse_hotkey("C").is_none());
        assert!(parse_hotkey("F5").is_none());
        assert!(parse_hotkey("Ctrl+Alt").is_none());
        assert!(parse_hotkey("Ctrl+Alt+C+D").is_none());
        assert!(parse_hotkey("Ctrl+F13").is_none());
        assert!(parse_hotkey("Ctrl+UnknownKey").is_none());
    }

    #[test]
    fn sleep_and_portal_appearance_parsers_match_signals() {
        assert!(is_prepare_for_sleep_resume_line(
            "/org/freedesktop/login1: org.freedesktop.login1.Manager.PrepareForSleep (false,)"
        ));
        assert!(!is_prepare_for_sleep_resume_line(
            "/org/freedesktop/login1: org.freedesktop.login1.Manager.PrepareForSleep (true,)"
        ));

        assert!(is_portal_appearance_changed_line(
            "/org/freedesktop/portal/desktop: org.freedesktop.portal.Settings.SettingChanged ('org.freedesktop.appearance', 'color-scheme', <uint32 1>)"
        ));
        assert!(is_portal_appearance_changed_line(
            "/org/freedesktop/portal/desktop: org.freedesktop.portal.Settings.SettingChanged ('org.freedesktop.appearance', 'contrast', <uint32 0>)"
        ));
        assert!(!is_portal_appearance_changed_line(
            "/org/freedesktop/portal/desktop: org.freedesktop.portal.Settings.SettingChanged ('org.gnome.desktop.interface', 'font-name', <'Sans 11'>)"
        ));
    }

    #[test]
    fn tray_lifecycle_and_window_tracking_work() {
        {
            let mut guard = LAST_EXTERNAL_WINDOW.lock().unwrap_or_else(|e| e.into_inner());
            *guard = Some((0x4242, "LibreOffice Writer".into()));
        }
        assert!(is_window_alive_and_visible(0x4242));
        assert!(focus_window(0x4242));
        let snap = snapshot_target_window().expect("snapshot window");
        assert!(!snap.1.is_empty());

        let labels = MenuLabels {
            open: "Open".into(),
            command_palette: "Command Palette".into(),
            shelf: "Shelf".into(),
            take_photo: "Take photo".into(),
            scan_document: "Scan document".into(),
            find_phone: "Find phone".into(),
            open_link: "Open link".into(),
            quit: "Quit".into(),
        };
        let tray = Tray::create("NectarLink", labels, |_| {}).expect("create tray");
        tray.set_tooltip("NectarLink · Pixel 9");
        tray.set_find_phone_enabled(true);
        assert!(tray.icon_path.exists());
        drop(tray);
    }

    #[test]
    fn parse_tray_helper_action_maps_all_menu_items() {
        assert_eq!(parse_tray_helper_action("OPEN\n"), Some(TrayEvent::Open));
        assert_eq!(parse_tray_helper_action("COMMAND_PALETTE"), Some(TrayEvent::CommandPalette));
        assert_eq!(parse_tray_helper_action("SHELF"), Some(TrayEvent::Shelf));
        assert_eq!(parse_tray_helper_action("TAKE_PHOTO"), Some(TrayEvent::TakePhoto));
        assert_eq!(parse_tray_helper_action("SCAN_DOCUMENT"), Some(TrayEvent::ScanDocument));
        assert_eq!(parse_tray_helper_action("FIND_PHONE"), Some(TrayEvent::FindPhone));
        assert_eq!(parse_tray_helper_action("OPEN_LINK"), Some(TrayEvent::OpenLinkOnPhone));
        assert_eq!(parse_tray_helper_action("QUIT"), Some(TrayEvent::Quit));
        assert_eq!(parse_tray_helper_action("UNKNOWN"), None);
    }
}
