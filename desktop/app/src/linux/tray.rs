// SPDX-License-Identifier: GPL-3.0-or-later
//! Linux system tray, global hotkeys, active external window tracking/focus
//! for Continuity Camera & File Shelf, and D-Bus sleep/resume + XDG Desktop
//! Portal appearance change listeners.

use std::{
    collections::HashMap,
    env,
    io::Result,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
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
    ToggleNotificationsPause,
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
    pub pause_notifications: String,
    pub resume_notifications: String,
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
static HOTKEY_GENERATION: AtomicU64 = AtomicU64::new(1);

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

/// Returns `true` when a system tray host (Qt `QSystemTrayIcon` or D-Bus
/// `org.kde.StatusNotifierWatcher` / `org.freedesktop.Notifications`)
/// is available in the current desktop session.
pub fn is_tray_available() -> bool {
    if !has_desktop_session() {
        return false;
    }
    if !cfg!(test) && crate::bridge::native::ffi::linux_tray_is_available() {
        return true;
    }
    if let Ok(conn) = zbus::blocking::Connection::session()
        && let Ok(dbus) = zbus::blocking::fdo::DBusProxy::new(&conn)
    {
        for name in [
            "org.kde.StatusNotifierWatcher",
            "org.freedesktop.StatusNotifierWatcher",
            "org.freedesktop.Notifications",
        ] {
            if let Ok(bus_name) = zbus::names::BusName::try_from(name)
                && dbus.name_has_owner(bus_name).unwrap_or(false)
            {
                return true;
            }
        }
    }
    false
}

fn is_numeric_window_id(hwnd: isize) -> bool {
    if hwnd <= 0 {
        return false;
    }
    if let Ok(map) = window_id_map().lock()
        && let Some(raw) = map.get(&hwnd)
    {
        let trimmed = raw.trim();
        return trimmed.starts_with("0x")
            || trimmed.starts_with("0X")
            || trimmed.bytes().all(|b| b.is_ascii_digit());
    }
    true
}

type XErrorHandler = Option<unsafe extern "C" fn(*mut libc::c_void, *mut libc::c_void) -> libc::c_int>;

static X11_ERROR_LOCK: Mutex<()> = Mutex::new(());
static X11_HAD_ERROR: AtomicBool = AtomicBool::new(false);

unsafe extern "C" fn x11_ignore_bad_access(_dpy: *mut libc::c_void, _err: *mut libc::c_void) -> libc::c_int {
    X11_HAD_ERROR.store(true, Ordering::Release);
    0
}

#[repr(C)]
struct XWindowAttributes {
    x: libc::c_int,
    y: libc::c_int,
    width: libc::c_int,
    height: libc::c_int,
    border_width: libc::c_int,
    depth: libc::c_int,
    visual: *mut libc::c_void,
    root: libc::c_ulong,
    class: libc::c_int,
    bit_gravity: libc::c_int,
    win_gravity: libc::c_int,
    backing_store: libc::c_int,
    backing_planes: libc::c_ulong,
    backing_pixel: libc::c_ulong,
    save_under: libc::c_int,
    colormap: libc::c_ulong,
    map_installed: libc::c_int,
    map_state: libc::c_int,
    all_event_masks: libc::c_long,
    your_event_mask: libc::c_long,
    do_not_propagate_mask: libc::c_long,
    override_redirect: libc::c_int,
    screen: *mut libc::c_void,
    _pad: [u8; 64],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct XClientMessageEvent {
    event_type: libc::c_int,
    serial: libc::c_ulong,
    send_event: libc::c_int,
    display: *mut libc::c_void,
    window: libc::c_ulong,
    message_type: libc::c_ulong,
    format: libc::c_int,
    data_l: [libc::c_long; 5],
}

struct X11Lib {
    x_open_display: unsafe extern "C" fn(*const libc::c_char) -> *mut libc::c_void,
    x_close_display: unsafe extern "C" fn(*mut libc::c_void) -> libc::c_int,
    x_default_root_window: unsafe extern "C" fn(*mut libc::c_void) -> libc::c_ulong,
    x_intern_atom: unsafe extern "C" fn(*mut libc::c_void, *const libc::c_char, libc::c_int) -> libc::c_ulong,
    x_get_window_property: unsafe extern "C" fn(
        *mut libc::c_void,
        libc::c_ulong,
        libc::c_ulong,
        libc::c_long,
        libc::c_long,
        libc::c_int,
        libc::c_ulong,
        *mut libc::c_ulong,
        *mut libc::c_int,
        *mut libc::c_ulong,
        *mut libc::c_ulong,
        *mut *mut libc::c_uchar,
    ) -> libc::c_int,
    x_free: unsafe extern "C" fn(*mut libc::c_void) -> libc::c_int,
    x_fetch_name:
        unsafe extern "C" fn(*mut libc::c_void, libc::c_ulong, *mut *mut libc::c_char) -> libc::c_int,
    x_get_window_attributes:
        unsafe extern "C" fn(*mut libc::c_void, libc::c_ulong, *mut XWindowAttributes) -> libc::c_int,
    x_send_event: unsafe extern "C" fn(
        *mut libc::c_void,
        libc::c_ulong,
        libc::c_int,
        libc::c_long,
        *mut XEvent,
    ) -> libc::c_int,
    x_set_input_focus:
        unsafe extern "C" fn(*mut libc::c_void, libc::c_ulong, libc::c_int, libc::c_ulong) -> libc::c_int,
    x_raise_window: unsafe extern "C" fn(*mut libc::c_void, libc::c_ulong) -> libc::c_int,
    x_sync: unsafe extern "C" fn(*mut libc::c_void, libc::c_int) -> libc::c_int,
    x_set_error_handler: unsafe extern "C" fn(XErrorHandler) -> XErrorHandler,
}

static X11_LIB: OnceLock<Option<X11Lib>> = OnceLock::new();

unsafe fn load_x11_sym<T>(lib: *mut libc::c_void, name: &std::ffi::CStr) -> Option<T> {
    let ptr = unsafe { libc::dlsym(lib, name.as_ptr()) };
    if ptr.is_null() { None } else { Some(unsafe { std::mem::transmute_copy(&ptr) }) }
}

fn x11_lib() -> Option<&'static X11Lib> {
    X11_LIB
        .get_or_init(|| unsafe {
            let lib = libc::dlopen(c"libX11.so.6".as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL);
            if lib.is_null() {
                return None;
            }
            Some(X11Lib {
                x_open_display: load_x11_sym(lib, c"XOpenDisplay")?,
                x_close_display: load_x11_sym(lib, c"XCloseDisplay")?,
                x_default_root_window: load_x11_sym(lib, c"XDefaultRootWindow")?,
                x_intern_atom: load_x11_sym(lib, c"XInternAtom")?,
                x_get_window_property: load_x11_sym(lib, c"XGetWindowProperty")?,
                x_free: load_x11_sym(lib, c"XFree")?,
                x_fetch_name: load_x11_sym(lib, c"XFetchName")?,
                x_get_window_attributes: load_x11_sym(lib, c"XGetWindowAttributes")?,
                x_send_event: load_x11_sym(lib, c"XSendEvent")?,
                x_set_input_focus: load_x11_sym(lib, c"XSetInputFocus")?,
                x_raise_window: load_x11_sym(lib, c"XRaiseWindow")?,
                x_sync: load_x11_sym(lib, c"XSync")?,
                x_set_error_handler: load_x11_sym(lib, c"XSetErrorHandler")?,
            })
        })
        .as_ref()
}

/// Temporarily installs `x11_ignore_bad_access` around an X11 operation, flushes
/// errors with `XSync`, and immediately restores the previous error handler so
/// Qt's process-wide X11 error handler is never clobbered.
unsafe fn with_x11_error_guard<R>(
    x_set_error_handler: unsafe extern "C" fn(XErrorHandler) -> XErrorHandler,
    x_sync: unsafe extern "C" fn(*mut libc::c_void, libc::c_int) -> libc::c_int,
    dpy: *mut libc::c_void,
    op: impl FnOnce() -> R,
) -> (R, bool) {
    let _guard = X11_ERROR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    X11_HAD_ERROR.store(false, Ordering::Release);
    let prev = unsafe { x_set_error_handler(Some(x11_ignore_bad_access)) };
    let out = op();
    let _ = unsafe { x_sync(dpy, 0) };
    let _ = unsafe { x_set_error_handler(prev) };
    let had_err = X11_HAD_ERROR.load(Ordering::Acquire);
    (out, had_err)
}

fn x11_window_title(x11: &X11Lib, dpy: *mut libc::c_void, win: libc::c_ulong) -> Option<String> {
    unsafe {
        let net_wm_name = (x11.x_intern_atom)(dpy, c"_NET_WM_NAME".as_ptr(), 0);
        if net_wm_name != 0 {
            let mut actual_type: libc::c_ulong = 0;
            let mut actual_format: libc::c_int = 0;
            let mut nitems: libc::c_ulong = 0;
            let mut bytes_after: libc::c_ulong = 0;
            let mut prop: *mut libc::c_uchar = std::ptr::null_mut();
            let status = (x11.x_get_window_property)(
                dpy,
                win,
                net_wm_name,
                0,
                1024,
                0,
                0, // AnyPropertyType
                &mut actual_type,
                &mut actual_format,
                &mut nitems,
                &mut bytes_after,
                &mut prop,
            );
            if status == 0 && !prop.is_null() {
                let text = if actual_format == 8 && nitems > 0 {
                    let slice = std::slice::from_raw_parts(prop, nitems as usize);
                    let end = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
                    Some(String::from_utf8_lossy(&slice[..end]).trim().to_owned())
                } else {
                    None
                };
                let _ = (x11.x_free)(prop.cast());
                if let Some(t) = text.filter(|s| !s.is_empty()) {
                    return Some(t);
                }
            }
        }

        let mut name_ptr: *mut libc::c_char = std::ptr::null_mut();
        if (x11.x_fetch_name)(dpy, win, &mut name_ptr) != 0 && !name_ptr.is_null() {
            let title = std::ffi::CStr::from_ptr(name_ptr).to_string_lossy().trim().to_owned();
            let _ = (x11.x_free)(name_ptr.cast());
            if !title.is_empty() {
                return Some(title);
            }
        }
        None
    }
}

fn x11_query_active_window() -> Option<(isize, String)> {
    let x11 = x11_lib()?;
    unsafe {
        let dpy = (x11.x_open_display)(std::ptr::null());
        if dpy.is_null() {
            return None;
        }
        let root = (x11.x_default_root_window)(dpy);
        let net_active = (x11.x_intern_atom)(dpy, c"_NET_ACTIVE_WINDOW".as_ptr(), 0);
        let (res, had_err) = with_x11_error_guard(x11.x_set_error_handler, x11.x_sync, dpy, || {
            if net_active == 0 {
                return None;
            }
            let mut actual_type: libc::c_ulong = 0;
            let mut actual_format: libc::c_int = 0;
            let mut nitems: libc::c_ulong = 0;
            let mut bytes_after: libc::c_ulong = 0;
            let mut prop: *mut libc::c_uchar = std::ptr::null_mut();
            let status = (x11.x_get_window_property)(
                dpy,
                root,
                net_active,
                0,
                1,
                0,
                0, // AnyPropertyType
                &mut actual_type,
                &mut actual_format,
                &mut nitems,
                &mut bytes_after,
                &mut prop,
            );
            if status != 0 || prop.is_null() {
                return None;
            }
            let win = if actual_format == 32 && nitems >= 1 { *(prop as *const libc::c_ulong) } else { 0 };
            let _ = (x11.x_free)(prop.cast());
            if win == 0 {
                return None;
            }
            let title = x11_window_title(x11, dpy, win)?;
            Some((win, title))
        });
        let _ = (x11.x_close_display)(dpy);
        if had_err {
            return None;
        }
        let (win, title) = res?;
        if is_nectarlink_title(&title) {
            return None;
        }
        let hwnd = win as isize;
        if hwnd == 0 {
            return None;
        }
        if let Ok(mut map) = window_id_map().lock() {
            map.insert(hwnd, format!("0x{win:x}"));
        }
        Some((hwnd, title))
    }
}

fn x11_is_window_alive_and_visible(hwnd: isize) -> bool {
    if !is_numeric_window_id(hwnd) {
        return false;
    }
    let Some(x11) = x11_lib() else {
        return false;
    };
    unsafe {
        let dpy = (x11.x_open_display)(std::ptr::null());
        if dpy.is_null() {
            return false;
        }
        const IS_VIEWABLE: libc::c_int = 2;
        let (viewable, had_err) = with_x11_error_guard(x11.x_set_error_handler, x11.x_sync, dpy, || {
            let mut attrs: XWindowAttributes = std::mem::zeroed();
            let status = (x11.x_get_window_attributes)(dpy, hwnd as libc::c_ulong, &mut attrs);
            status != 0 && attrs.map_state == IS_VIEWABLE
        });
        let _ = (x11.x_close_display)(dpy);
        viewable && !had_err
    }
}

fn x11_focus_window(hwnd: isize) -> bool {
    if !is_numeric_window_id(hwnd) {
        return false;
    }
    let Some(x11) = x11_lib() else {
        return false;
    };
    unsafe {
        let dpy = (x11.x_open_display)(std::ptr::null());
        if dpy.is_null() {
            return false;
        }
        let root = (x11.x_default_root_window)(dpy);
        let net_active = (x11.x_intern_atom)(dpy, c"_NET_ACTIVE_WINDOW".as_ptr(), 0);
        const IS_VIEWABLE: libc::c_int = 2;
        const CLIENT_MESSAGE: libc::c_int = 33;
        const REVERT_TO_POINTER_ROOT: libc::c_int = 1;
        const SUBSTRUCTURE_REDIRECT_MASK: libc::c_long = 1 << 20;
        const SUBSTRUCTURE_NOTIFY_MASK: libc::c_long = 1 << 19;

        let (ok, had_err) = with_x11_error_guard(x11.x_set_error_handler, x11.x_sync, dpy, || {
            let win = hwnd as libc::c_ulong;
            let mut attrs: XWindowAttributes = std::mem::zeroed();
            if (x11.x_get_window_attributes)(dpy, win, &mut attrs) == 0 || attrs.map_state != IS_VIEWABLE {
                return false;
            }
            if net_active != 0 {
                let mut ev: XEvent = std::mem::zeroed();
                ev.xclient = XClientMessageEvent {
                    event_type: CLIENT_MESSAGE,
                    serial: 0,
                    send_event: 1,
                    display: dpy,
                    window: win,
                    message_type: net_active,
                    format: 32,
                    data_l: [2, 0, 0, 0, 0],
                };
                let _ = (x11.x_send_event)(
                    dpy,
                    root,
                    0,
                    SUBSTRUCTURE_REDIRECT_MASK | SUBSTRUCTURE_NOTIFY_MASK,
                    &mut ev,
                );
            }
            let _ = (x11.x_raise_window)(dpy, win);
            let _ = (x11.x_set_input_focus)(dpy, win, REVERT_TO_POINTER_ROOT, 0);
            true
        });
        let _ = (x11.x_close_display)(dpy);
        ok && !had_err
    }
}

/// Queries the currently active external window via X11 `_NET_ACTIVE_WINDOW`
/// (in-process), `kdotool` (KDE Wayland), or `xdotool`, ignoring NectarLink's own windows.
pub fn query_active_window() -> Option<(isize, String)> {
    if has_x11_display()
        && let Some((hwnd, title)) = x11_query_active_window()
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
    if has_x11_display()
        && let Some(raw_id) = run_command_stdout("xdotool", &["getactivewindow"])
        && let Some(hwnd) = parse_window_handle(&raw_id)
        && let Some(title) = run_command_stdout("xdotool", &["getwindowname", &raw_id])
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
/// moment a Continuity Camera request begins. Returns `None` honestly when no
/// live external window can be inspected.
pub fn snapshot_target_window() -> Option<(isize, String)> {
    if let Some((hwnd, title)) = query_active_window() {
        let mut guard = LAST_EXTERNAL_WINDOW.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some((hwnd, title.clone()));
        return Some((hwnd, title));
    }
    let cached = LAST_EXTERNAL_WINDOW.lock().unwrap_or_else(|e| e.into_inner()).clone()?;
    if is_window_alive_and_visible(cached.0) { Some(cached) } else { None }
}

/// Returns true if `hwnd` still refers to an existing, visible top-level window.
pub fn is_window_alive_and_visible(hwnd: isize) -> bool {
    if hwnd == 0 {
        return false;
    }
    if has_x11_display() && x11_is_window_alive_and_visible(hwnd) {
        return true;
    }
    let raw_id = raw_window_id(hwnd);
    if has_wayland_display()
        && let Some(title) = run_command_stdout("kdotool", &["getwindowname", &raw_id])
        && !title.is_empty()
    {
        return true;
    }
    if has_x11_display()
        && let Some(title) = run_command_stdout("xdotool", &["getwindowname", &raw_id])
        && !title.is_empty()
    {
        return true;
    }
    false
}

/// Restores and brings `hwnd` to the foreground so a paste shortcut can be
/// synthesized into it. Returns false if the window is gone or cannot be focused.
pub fn focus_window(hwnd: isize) -> bool {
    if !is_window_alive_and_visible(hwnd) {
        return false;
    }
    if has_x11_display() && x11_focus_window(hwnd) {
        return true;
    }
    let raw_id = raw_window_id(hwnd);
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
    false
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
    if trimmed.is_empty() || trimmed.starts_with('-') || !has_x11_display() {
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
    drop(specs);
    HOTKEY_GENERATION.fetch_add(1, Ordering::Release);
}

const X11_SHIFT_MASK: u32 = 1 << 0;
const X11_LOCK_MASK: u32 = 1 << 1;
const X11_CONTROL_MASK: u32 = 1 << 2;
const X11_MOD1_MASK: u32 = 1 << 3;
const X11_MOD2_MASK: u32 = 1 << 4;
const X11_MOD4_MASK: u32 = 1 << 6;
const X11_CLEAN_MOD_MASK: u32 = X11_SHIFT_MASK | X11_CONTROL_MASK | X11_MOD1_MASK | X11_MOD4_MASK;

/// Converts `MOD_*` flags from [`parse_hotkey`] into an X11 modifier state mask.
pub(crate) fn mods_to_x11_mask(mods: u32) -> u32 {
    let mut mask = 0u32;
    if (mods & MOD_SHIFT) != 0 {
        mask |= X11_SHIFT_MASK;
    }
    if (mods & MOD_CONTROL) != 0 {
        mask |= X11_CONTROL_MASK;
    }
    if (mods & MOD_ALT) != 0 {
        mask |= X11_MOD1_MASK;
    }
    if (mods & MOD_WIN) != 0 {
        mask |= X11_MOD4_MASK;
    }
    mask
}

/// Converts a virtual key code from [`parse_hotkey`] into an X11 `KeySym` (`<X11/keysymdef.h>`).
pub(crate) fn vk_to_x11_keysym(vk: u32) -> Option<libc::c_ulong> {
    let keysym: u64 = match vk {
        0x20 => 0x0020,                               // XK_space
        0x09 => 0xFF09,                               // XK_Tab
        0x0D => 0xFF0D,                               // XK_Return
        0x1B => 0xFF1B,                               // XK_Escape
        0x2D => 0xFF63,                               // XK_Insert
        0x2E => 0xFFFF,                               // XK_Delete
        0x24 => 0xFF50,                               // XK_Home
        0x23 => 0xFF57,                               // XK_End
        0x21 => 0xFF55,                               // XK_Prior (PageUp)
        0x22 => 0xFF56,                               // XK_Next (PageDown)
        0x25 => 0xFF51,                               // XK_Left
        0x26 => 0xFF52,                               // XK_Up
        0x27 => 0xFF53,                               // XK_Right
        0x28 => 0xFF54,                               // XK_Down
        0xBB => 0x003D,                               // XK_equal ('+'/'=' key)
        0xBD => 0x002D,                               // XK_minus
        0xBC => 0x002C,                               // XK_comma
        0xBE => 0x002E,                               // XK_period
        0x30..=0x39 => u64::from(vk),                 // XK_0..=XK_9
        0x41..=0x5A => u64::from(vk + 32),            // XK_a..=XK_z
        0x70..=0x7B => 0xFFBE + u64::from(vk - 0x70), // XK_F1..=XK_F12
        _ => return None,
    };
    Some(keysym as libc::c_ulong)
}

#[repr(C)]
#[derive(Clone, Copy)]
struct XKeyEvent {
    event_type: libc::c_int,
    serial: libc::c_ulong,
    send_event: libc::c_int,
    display: *mut libc::c_void,
    window: libc::c_ulong,
    root: libc::c_ulong,
    subwindow: libc::c_ulong,
    time: libc::c_ulong,
    x: libc::c_int,
    y: libc::c_int,
    x_root: libc::c_int,
    y_root: libc::c_int,
    state: libc::c_uint,
    keycode: libc::c_uint,
    same_screen: libc::c_int,
}

#[repr(C)]
union XEvent {
    event_type: libc::c_int,
    xkey: XKeyEvent,
    xclient: XClientMessageEvent,
    pad: [libc::c_long; 24],
}

fn run_x11_hotkey_loop(stop: &AtomicBool, handler: &SharedHandler) {
    if !has_x11_display() {
        return;
    }
    // SAFETY: Dynamically loads libX11.so.6 and resolves standard Xlib symbols.
    unsafe {
        let lib = libc::dlopen(c"libX11.so.6".as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL);
        if lib.is_null() {
            tracing::debug!("libX11.so.6 not available for global hotkeys");
            return;
        }

        type FnXOpenDisplay = unsafe extern "C" fn(*const libc::c_char) -> *mut libc::c_void;
        type FnXCloseDisplay = unsafe extern "C" fn(*mut libc::c_void) -> libc::c_int;
        type FnXDefaultRootWindow = unsafe extern "C" fn(*mut libc::c_void) -> libc::c_ulong;
        type FnXKeysymToKeycode = unsafe extern "C" fn(*mut libc::c_void, libc::c_ulong) -> libc::c_uchar;
        type FnXGrabKey = unsafe extern "C" fn(
            *mut libc::c_void,
            libc::c_int,
            libc::c_uint,
            libc::c_ulong,
            libc::c_int,
            libc::c_int,
            libc::c_int,
        ) -> libc::c_int;
        type FnXUngrabKey =
            unsafe extern "C" fn(*mut libc::c_void, libc::c_int, libc::c_uint, libc::c_ulong) -> libc::c_int;
        type FnXPending = unsafe extern "C" fn(*mut libc::c_void) -> libc::c_int;
        type FnXNextEvent = unsafe extern "C" fn(*mut libc::c_void, *mut XEvent) -> libc::c_int;
        type FnXSync = unsafe extern "C" fn(*mut libc::c_void, libc::c_int) -> libc::c_int;
        type FnXConnectionNumber = unsafe extern "C" fn(*mut libc::c_void) -> libc::c_int;
        type FnXSetErrorHandler = unsafe extern "C" fn(XErrorHandler) -> XErrorHandler;

        macro_rules! sym {
            ($name:literal, $ty:ty) => {{
                let ptr = libc::dlsym(lib, $name.as_ptr());
                if ptr.is_null() {
                    return;
                }
                std::mem::transmute::<*mut libc::c_void, $ty>(ptr)
            }};
        }

        let x_open_display = sym!(c"XOpenDisplay", FnXOpenDisplay);
        let x_close_display = sym!(c"XCloseDisplay", FnXCloseDisplay);
        let x_default_root_window = sym!(c"XDefaultRootWindow", FnXDefaultRootWindow);
        let x_keysym_to_keycode = sym!(c"XKeysymToKeycode", FnXKeysymToKeycode);
        let x_grab_key = sym!(c"XGrabKey", FnXGrabKey);
        let x_ungrab_key = sym!(c"XUngrabKey", FnXUngrabKey);
        let x_pending = sym!(c"XPending", FnXPending);
        let x_next_event = sym!(c"XNextEvent", FnXNextEvent);
        let x_sync = sym!(c"XSync", FnXSync);
        let x_connection_number = sym!(c"XConnectionNumber", FnXConnectionNumber);
        let x_set_error_handler = sym!(c"XSetErrorHandler", FnXSetErrorHandler);

        let dpy = x_open_display(std::ptr::null());
        if dpy.is_null() {
            tracing::debug!("XOpenDisplay failed for global hotkeys");
            return;
        }
        let root = x_default_root_window(dpy);
        let fd = x_connection_number(dpy);

        const KEY_PRESS: libc::c_int = 2;
        const GRAB_MODE_ASYNC: libc::c_int = 1;
        const LOCK_COMBOS: [u32; 4] = [0, X11_LOCK_MASK, X11_MOD2_MASK, X11_LOCK_MASK | X11_MOD2_MASK];

        let mut applied_gen: u64 = 0;
        // (x11_clean_mods, keycode, TrayEvent)
        let mut grabbed: Vec<(u32, u32, TrayEvent)> = Vec::new();
        let mut last_fired: Option<(TrayEvent, Instant)> = None;

        while !stop.load(Ordering::Acquire) {
            let current_gen = HOTKEY_GENERATION.load(Ordering::Acquire);
            if current_gen != applied_gen {
                let (photo, scan, palette, shelf) =
                    HOTKEY_SPECS.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let targets = [
                    (photo.as_str(), TrayEvent::TakePhoto),
                    (scan.as_str(), TrayEvent::ScanDocument),
                    (palette.as_str(), TrayEvent::CommandPalette),
                    (shelf.as_str(), TrayEvent::Shelf),
                ];

                let _ = with_x11_error_guard(x_set_error_handler, x_sync, dpy, || {
                    for &(mods, keycode, _) in &grabbed {
                        for extra in LOCK_COMBOS {
                            let _ = x_ungrab_key(dpy, keycode as libc::c_int, mods | extra, root);
                        }
                    }
                    grabbed.clear();

                    for (spec, ev) in targets {
                        let Some((mods, vk)) = parse_hotkey(spec) else { continue };
                        let Some(keysym) = vk_to_x11_keysym(vk) else { continue };
                        let keycode = u32::from(x_keysym_to_keycode(dpy, keysym));
                        if keycode == 0 {
                            continue;
                        }
                        let x11_mods = mods_to_x11_mask(mods);
                        if x11_mods == 0 {
                            continue;
                        }
                        for extra in LOCK_COMBOS {
                            let _ = x_grab_key(
                                dpy,
                                keycode as libc::c_int,
                                x11_mods | extra,
                                root,
                                1, // owner_events = True
                                GRAB_MODE_ASYNC,
                                GRAB_MODE_ASYNC,
                            );
                        }
                        grabbed.push((x11_mods, keycode, ev));
                    }
                });
                applied_gen = current_gen;
            }

            while x_pending(dpy) > 0 {
                let mut event: XEvent = std::mem::zeroed();
                let _ = x_next_event(dpy, &mut event);
                if event.event_type == KEY_PRESS {
                    let clean_mods = event.xkey.state & X11_CLEAN_MOD_MASK;
                    let keycode = event.xkey.keycode;
                    if let Some(&(_, _, tray_ev)) =
                        grabbed.iter().find(|&&(m, kc, _)| m == clean_mods && kc == keycode)
                    {
                        let now = Instant::now();
                        let debounced = last_fired.is_some_and(|(prev_ev, prev_t)| {
                            prev_ev == tray_ev
                                && now.saturating_duration_since(prev_t) < Duration::from_millis(220)
                        });
                        if !debounced {
                            last_fired = Some((tray_ev, now));
                            record_foreground_window();
                            handler.emit(tray_ev);
                        }
                    }
                }
            }

            let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
            let _ = libc::poll(&mut pfd, 1, 150);
        }

        let _ = with_x11_error_guard(x_set_error_handler, x_sync, dpy, || {
            for &(mods, keycode, _) in &grabbed {
                for extra in LOCK_COMBOS {
                    let _ = x_ungrab_key(dpy, keycode as libc::c_int, mods | extra, root);
                }
            }
        });
        let _ = x_close_display(dpy);
    }
}

fn spawn_x11_hotkey_thread(stop: Arc<AtomicBool>, handler: Arc<SharedHandler>) -> Option<JoinHandle<()>> {
    if !has_x11_display() {
        return None;
    }
    std::thread::Builder::new()
        .name("tray-x11-hotkeys".into())
        .spawn(move || run_x11_hotkey_loop(&stop, &handler))
        .ok()
}

/// Maps an `org.freedesktop.portal.GlobalShortcuts` shortcut identifier to [`TrayEvent`].
pub(crate) fn parse_portal_shortcut_id(id: &str) -> Option<TrayEvent> {
    match id.trim() {
        "take-photo" => Some(TrayEvent::TakePhoto),
        "scan-document" => Some(TrayEvent::ScanDocument),
        "command-palette" => Some(TrayEvent::CommandPalette),
        "shelf" => Some(TrayEvent::Shelf),
        _ => None,
    }
}

/// Converts a NectarLink hotkey spec (e.g. `"Ctrl+Alt+C"`) into an XDG GlobalShortcuts
/// `preferred_trigger` string (e.g. `"CTRL+ALT+c"`).
pub(crate) fn hotkey_to_portal_trigger(spec: &str) -> Option<String> {
    let (mods, vk) = parse_hotkey(spec)?;
    let mut parts: Vec<String> = Vec::new();
    if (mods & MOD_CONTROL) != 0 {
        parts.push("CTRL".into());
    }
    if (mods & MOD_ALT) != 0 {
        parts.push("ALT".into());
    }
    if (mods & MOD_SHIFT) != 0 {
        parts.push("SHIFT".into());
    }
    if (mods & MOD_WIN) != 0 {
        parts.push("LOGO".into());
    }
    let key_name: String = match vk {
        0x20 => "space".into(),
        0x09 => "Tab".into(),
        0x0D => "Return".into(),
        0x1B => "Escape".into(),
        0x2D => "Insert".into(),
        0x2E => "Delete".into(),
        0x24 => "Home".into(),
        0x23 => "End".into(),
        0x21 => "Page_Up".into(),
        0x22 => "Page_Down".into(),
        0x25 => "Left".into(),
        0x26 => "Up".into(),
        0x27 => "Right".into(),
        0x28 => "Down".into(),
        0xBB => "equal".into(),
        0xBD => "minus".into(),
        0xBC => "comma".into(),
        0xBE => "period".into(),
        0x30..=0x39 => char::from(vk as u8).to_string(),
        0x41..=0x5A => char::from((vk as u8).to_ascii_lowercase()).to_string(),
        0x70..=0x7B => format!("F{}", vk - 0x70 + 1),
        _ => return None,
    };
    parts.push(key_name);
    Some(parts.join("+"))
}

/// The hotkeys as [`update_hotkeys`] last set them.
fn hotkey_specs() -> (String, String, String, String) {
    HOTKEY_SPECS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The portal shortcut IDs NectarLink binds for `specs` (those with a key).
fn portal_shortcut_ids(specs: &(String, String, String, String)) -> Vec<&'static str> {
    let (photo, scan, palette, shelf) = specs;
    [("take-photo", photo), ("scan-document", scan), ("command-palette", palette), ("shelf", shelf)]
        .into_iter()
        .filter(|(_, spec)| hotkey_to_portal_trigger(spec).is_some_and(|t| !t.is_empty()))
        .map(|(id, _)| id)
        .collect()
}

/// Whether the portal already holds every shortcut in `wanted`, from a
/// `ListShortcuts` answer's IDs. Binding again would make KDE open its
/// shortcut dialog at every start.
fn portal_has_shortcuts(listed: &[String], wanted: &[&str]) -> bool {
    wanted.iter().all(|id| listed.iter().any(|l| l == id))
}

/// The IDs in a `ListShortcuts` answer's `shortcuts` (`a(sa{sv})`).
fn listed_shortcut_ids(value: &zbus::zvariant::Value<'_>) -> Vec<String> {
    use zbus::zvariant::Value;
    let Value::Array(list) = value else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|item| match item {
            Value::Value(inner) => match &**inner {
                Value::Structure(entry) => entry.fields().first().cloned(),
                _ => None,
            }
            .and_then(|id| if let Value::Str(id) = id { Some(id.to_string()) } else { None }),
            Value::Structure(entry) => match entry.fields().first() {
                Some(Value::Str(id)) => Some(id.to_string()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn bind_portal_global_shortcuts(
    conn: &zbus::blocking::Connection,
    session_path: zbus::zvariant::ObjectPath<'_>,
    bind_seq: &mut u64,
    specs: &(String, String, String, String),
) -> bool {
    use zbus::zvariant::Value;

    let (photo, scan, palette, shelf) = specs;
    let specs = [
        ("take-photo", "Take photo with phone", photo.as_str()),
        ("scan-document", "Scan document with phone", scan.as_str()),
        ("command-palette", "Open Command Palette", palette.as_str()),
        ("shelf", "Open File Shelf", shelf.as_str()),
    ];

    let mut shortcuts: Vec<(&str, HashMap<&str, Value<'_>>)> = Vec::new();
    let mut triggers: Vec<String> = Vec::new();
    for (_, _, spec) in specs {
        triggers.push(hotkey_to_portal_trigger(spec).unwrap_or_default());
    }
    for ((id, desc, _), trigger) in specs.iter().zip(triggers.iter()) {
        if trigger.is_empty() {
            continue;
        }
        let mut meta: HashMap<&str, Value<'_>> = HashMap::new();
        meta.insert("description", Value::from(*desc));
        meta.insert("preferred_trigger", Value::from(trigger.as_str()));
        shortcuts.push((*id, meta));
    }
    if shortcuts.is_empty() {
        return true;
    }

    *bind_seq += 1;
    let token = format!("nectarlink_gs_bind_{}", *bind_seq);
    let mut options: HashMap<&str, Value<'_>> = HashMap::new();
    options.insert("handle_token", Value::from(token.as_str()));

    conn.call_method(
        Some("org.freedesktop.portal.Desktop"),
        "/org/freedesktop/portal/desktop",
        Some("org.freedesktop.portal.GlobalShortcuts"),
        "BindShortcuts",
        &(&session_path, shortcuts, "", options),
    )
    .is_ok()
}

fn try_run_wayland_portal_shortcuts(stop: &AtomicBool, handler: &SharedHandler) -> bool {
    use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

    let Ok(conn) = zbus::blocking::Connection::session() else {
        return false;
    };
    if !super::dbus_name_has_owner(&conn, "org.freedesktop.portal.Desktop") {
        return false;
    }
    let Ok(dbus) = zbus::blocking::fdo::DBusProxy::new(&conn) else {
        return false;
    };
    let Ok(activated_rule) = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.freedesktop.portal.GlobalShortcuts")
        .and_then(|b| b.member("Activated"))
        .and_then(|b| b.path("/org/freedesktop/portal/desktop"))
        .map(|b| b.build())
    else {
        return false;
    };
    if dbus.add_match_rule(activated_rule).is_err() {
        return false;
    }
    if let Ok(resp_rule) = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.freedesktop.portal.Request")
        .and_then(|b| b.member("Response"))
        .map(|b| b.build())
    {
        let _ = dbus.add_match_rule(resp_rule);
    }

    let pid = std::process::id();
    let req_token = format!("nectarlink_gs_req_{pid}");
    let session_token = format!("nectarlink_gs_sess_{pid}");
    let mut create_opts: HashMap<&str, Value<'_>> = HashMap::new();
    create_opts.insert("handle_token", Value::from(req_token.as_str()));
    create_opts.insert("session_handle_token", Value::from(session_token.as_str()));

    let Ok(reply) = conn.call_method(
        Some("org.freedesktop.portal.Desktop"),
        "/org/freedesktop/portal/desktop",
        Some("org.freedesktop.portal.GlobalShortcuts"),
        "CreateSession",
        &(create_opts,),
    ) else {
        return false;
    };

    let predicted_session_path =
        conn.unique_name().map(|u| u.as_str().trim_start_matches(':').replace('.', "_")).and_then(|sender| {
            ObjectPath::try_from(format!("/org/freedesktop/portal/desktop/session/{sender}/{session_token}"))
                .ok()
                .map(OwnedObjectPath::from)
        });

    let mut session_path: Option<OwnedObjectPath> = reply
        .body()
        .deserialize::<OwnedObjectPath>()
        .ok()
        .filter(|p| p.as_str().contains("/session/"))
        .or(predicted_session_path);

    let mut bind_seq: u64 = 0;
    // The hotkeys the portal holds for this session; None until its list is in.
    let mut applied: Option<(String, String, String, String)> = None;
    let list_token = format!("nectarlink_gs_list_{pid}");
    let mut listed = false;
    // Asks what the portal already has (KDE keeps an app's shortcuts between
    // runs); without an answer, binds as before.
    let list = |conn: &zbus::blocking::Connection,
                sp: &OwnedObjectPath,
                bind_seq: &mut u64,
                applied: &mut Option<(String, String, String, String)>| {
        let mut opts: HashMap<&str, Value<'_>> = HashMap::new();
        opts.insert("handle_token", Value::from(list_token.as_str()));
        let asked = conn
            .call_method(
                Some("org.freedesktop.portal.Desktop"),
                "/org/freedesktop/portal/desktop",
                Some("org.freedesktop.portal.GlobalShortcuts"),
                "ListShortcuts",
                &(sp.as_ref(), opts),
            )
            .is_ok();
        if !asked {
            let specs = hotkey_specs();
            let _ = bind_portal_global_shortcuts(conn, sp.as_ref(), bind_seq, &specs);
            *applied = Some(specs);
        }
    };

    if let Some(ref sp) = session_path {
        list(&conn, sp, &mut bind_seq, &mut applied);
        listed = true;
    }

    for msg in zbus::blocking::MessageIterator::from(&conn) {
        if stop.load(Ordering::Acquire) {
            break;
        }
        let Ok(msg) = msg else {
            break;
        };
        let hdr = msg.header();
        let iface = hdr.interface().map(|i| i.as_str());
        let member = hdr.member().map(|m| m.as_str());

        if iface == Some("org.freedesktop.portal.Request")
            && member == Some("Response")
            && let Ok((code, results)) = msg.body().deserialize::<(u32, HashMap<String, OwnedValue>)>()
        {
            let is_list = hdr.path().is_some_and(|p| p.as_str().ends_with(&format!("/{list_token}")));
            if is_list {
                if applied.is_none()
                    && let Some(ref sp) = session_path
                {
                    let ids = results.get("shortcuts").map(|v| listed_shortcut_ids(v)).unwrap_or_default();
                    let specs = hotkey_specs();
                    if code != 0 || !portal_has_shortcuts(&ids, &portal_shortcut_ids(&specs)) {
                        let _ = bind_portal_global_shortcuts(&conn, sp.as_ref(), &mut bind_seq, &specs);
                    }
                    applied = Some(specs);
                }
            } else {
                if code != 0 && session_path.is_none() {
                    return false;
                }
                if session_path.is_none()
                    && let Some(val) = results.get("session_handle")
                {
                    if let Ok(s) = <&str>::try_from(val)
                        && let Ok(op) = ObjectPath::try_from(s)
                    {
                        session_path = Some(OwnedObjectPath::from(op));
                    } else if let Ok(op) = <ObjectPath<'_>>::try_from(&**val) {
                        session_path = Some(OwnedObjectPath::from(op));
                    }
                }
            }
        }

        if !listed && let Some(ref sp) = session_path {
            list(&conn, sp, &mut bind_seq, &mut applied);
            listed = true;
        }

        // The user changed a hotkey: bind the new set (KDE then asks to confirm it).
        if let (Some(have), Some(sp)) = (&applied, &session_path) {
            let specs = hotkey_specs();
            if *have != specs {
                let _ = bind_portal_global_shortcuts(&conn, sp.as_ref(), &mut bind_seq, &specs);
                applied = Some(specs);
            }
        }

        if iface == Some("org.freedesktop.portal.GlobalShortcuts")
            && member == Some("Activated")
            && let Ok((_sess, shortcut_id, _ts, _opts)) =
                msg.body().deserialize::<(OwnedObjectPath, String, u64, HashMap<String, OwnedValue>)>()
            && let Some(ev) = parse_portal_shortcut_id(&shortcut_id)
        {
            record_foreground_window();
            handler.emit(ev);
        }
    }
    true
}

fn spawn_wayland_global_shortcuts_listener(stop: Arc<AtomicBool>, handler: Arc<SharedHandler>) {
    let _ = std::thread::Builder::new().name("tray-wayland-shortcuts".into()).spawn(move || {
        if !try_run_wayland_portal_shortcuts(&stop, &handler) && !stop.load(Ordering::Acquire) {
            run_x11_hotkey_loop(&stop, &handler);
        }
    });
}

/// Returns true when an `org.freedesktop.portal.Settings.SettingChanged`
/// signal (`namespace`, `key`) affects desktop appearance (`"color-scheme"`,
/// `"contrast"`, or `"accent-color"`).
pub fn is_portal_appearance_setting(namespace: &str, key: &str) -> bool {
    namespace.trim() == "org.freedesktop.appearance"
        && matches!(key.trim(), "color-scheme" | "contrast" | "accent-color")
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

type TrayEventHandler = Box<dyn Fn(TrayEvent) + Send + Sync>;

struct SharedHandler {
    inner: Mutex<Option<TrayEventHandler>>,
}

impl SharedHandler {
    fn new(handler: impl Fn(TrayEvent) + Send + Sync + 'static) -> Arc<Self> {
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
    if let Some(dir) = dirs::runtime_dir().or_else(dirs::cache_dir).filter(|p| p.is_absolute()) {
        dir.join("nectarlink").join("tray-icon.png")
    } else {
        let uid = unsafe { libc::geteuid() };
        env::temp_dir().join(format!("nectarlink-{uid}")).join("tray-icon.png")
    }
}

fn spawn_login1_sleep_listener(stop: Arc<AtomicBool>, handler: Arc<SharedHandler>) {
    let _ = std::thread::Builder::new().name("tray-login1-monitor".into()).spawn(move || {
        let Ok(conn) = zbus::blocking::Connection::system() else { return };
        let Ok(rule) = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface("org.freedesktop.login1.Manager")
            .and_then(|b| b.member("PrepareForSleep"))
            .map(|b| b.build())
        else {
            return;
        };
        let Ok(dbus) = zbus::blocking::fdo::DBusProxy::new(&conn) else { return };
        if dbus.add_match_rule(rule).is_err() {
            return;
        }
        for msg in zbus::blocking::MessageIterator::from(&conn) {
            if stop.load(Ordering::Acquire) {
                break;
            }
            let Ok(msg) = msg else { break };
            let hdr = msg.header();
            if hdr.interface().map(|i| i.as_str()) == Some("org.freedesktop.login1.Manager")
                && hdr.member().map(|m| m.as_str()) == Some("PrepareForSleep")
                && let Ok(going_to_sleep) = msg.body().deserialize::<bool>()
                && !going_to_sleep
            {
                handler.emit(TrayEvent::Resumed);
            }
        }
    });
}

fn spawn_portal_appearance_listener(stop: Arc<AtomicBool>, handler: Arc<SharedHandler>) {
    let _ = std::thread::Builder::new().name("tray-portal-monitor".into()).spawn(move || {
        let Ok(conn) = zbus::blocking::Connection::session() else { return };
        let Ok(rule) = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface("org.freedesktop.portal.Settings")
            .and_then(|b| b.member("SettingChanged"))
            .map(|b| b.build())
        else {
            return;
        };
        let Ok(dbus) = zbus::blocking::fdo::DBusProxy::new(&conn) else { return };
        if dbus.add_match_rule(rule).is_err() {
            return;
        }
        for msg in zbus::blocking::MessageIterator::from(&conn) {
            if stop.load(Ordering::Acquire) {
                break;
            }
            let Ok(msg) = msg else { break };
            let hdr = msg.header();
            if hdr.interface().map(|i| i.as_str()) == Some("org.freedesktop.portal.Settings")
                && hdr.member().map(|m| m.as_str()) == Some("SettingChanged")
                && let Ok((namespace, key, _val)) =
                    msg.body().deserialize::<(String, String, zbus::zvariant::OwnedValue)>()
                && is_portal_appearance_setting(&namespace, &key)
            {
                handler.emit(TrayEvent::AppearanceChanged);
            }
        }
    });
}

pub(crate) fn parse_tray_helper_action(line: &str) -> Option<TrayEvent> {
    match line.trim() {
        "OPEN" => Some(TrayEvent::Open),
        "COMMAND_PALETTE" => Some(TrayEvent::CommandPalette),
        "SHELF" => Some(TrayEvent::Shelf),
        "TAKE_PHOTO" => Some(TrayEvent::TakePhoto),
        "SCAN_DOCUMENT" => Some(TrayEvent::ScanDocument),
        "FIND_PHONE" => Some(TrayEvent::FindPhone),
        "OPEN_LINK" => Some(TrayEvent::OpenLinkOnPhone),
        "PAUSE_NOTIFICATIONS" => Some(TrayEvent::ToggleNotificationsPause),
        "QUIT" => Some(TrayEvent::Quit),
        _ => None,
    }
}

pub struct Tray {
    tooltip: Mutex<String>,
    #[allow(dead_code)]
    labels: Mutex<MenuLabels>,
    find_phone_enabled: Mutex<bool>,
    icon_path: PathBuf,
    stop: Arc<AtomicBool>,
    handler: Arc<SharedHandler>,
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
    pub fn create(
        tooltip: &str,
        labels: MenuLabels,
        handler: impl Fn(TrayEvent) + Send + Sync + 'static,
    ) -> Result<Tray> {
        let icon_path = tray_icon_cache_path();
        if let Err(e) = super::icon::write_png(&icon_path, 32) {
            tracing::debug!(error = %e, path = %icon_path.display(), "could not write tray icon PNG");
        }

        let stop = Arc::new(AtomicBool::new(false));
        let shared_handler = SharedHandler::new(handler);
        let mut threads = Vec::new();

        if !cfg!(test) && has_desktop_session() {
            record_foreground_window();

            // 0. Qt QSystemTrayIcon + QMenu on the main GUI thread.
            crate::bridge::native::ffi::linux_tray_setup(
                &icon_path.to_string_lossy(),
                tooltip,
                &labels.open,
                &labels.command_palette,
                &labels.shelf,
                &labels.take_photo,
                &labels.scan_document,
                &labels.find_phone,
                &labels.open_link,
                &labels.pause_notifications,
                &labels.quit,
            );
            let stop_clone = Arc::clone(&stop);
            let handler_clone = Arc::clone(&shared_handler);
            if let Ok(handle) = std::thread::Builder::new().name("tray-qt-actions".into()).spawn(move || {
                while !stop_clone.load(Ordering::Acquire) {
                    let action = crate::bridge::native::ffi::linux_tray_wait_action(200);
                    if stop_clone.load(Ordering::Acquire) {
                        break;
                    }
                    if !action.is_empty()
                        && let Some(ev) = parse_tray_helper_action(&action)
                    {
                        if matches!(
                            ev,
                            TrayEvent::CommandPalette | TrayEvent::TakePhoto | TrayEvent::ScanDocument
                        ) {
                            record_foreground_window();
                        }
                        handler_clone.emit(ev);
                    }
                }
            }) {
                threads.push(handle);
            }

            // 1. System bus org.freedesktop.login1.Manager PrepareForSleep listener via zbus.
            spawn_login1_sleep_listener(Arc::clone(&stop), Arc::clone(&shared_handler));

            // 2. Session bus org.freedesktop.portal.Settings SettingChanged listener via zbus.
            spawn_portal_appearance_listener(Arc::clone(&stop), Arc::clone(&shared_handler));
        }

        if !cfg!(test) && has_wayland_display() {
            spawn_wayland_global_shortcuts_listener(Arc::clone(&stop), Arc::clone(&shared_handler));
        } else if let Some(handle) = spawn_x11_hotkey_thread(Arc::clone(&stop), Arc::clone(&shared_handler)) {
            threads.push(handle);
        }

        Ok(Tray {
            tooltip: Mutex::new(tooltip.to_string()),
            labels: Mutex::new(labels),
            find_phone_enabled: Mutex::new(false),
            icon_path,
            stop,
            handler: shared_handler,
            threads: Mutex::new(threads),
        })
    }

    pub fn set_tooltip(&self, text: &str) {
        if let Ok(mut guard) = self.tooltip.lock() {
            *guard = text.to_string();
        }
        if !cfg!(test) {
            crate::bridge::native::ffi::linux_tray_set_tooltip(text);
        }
    }

    /// Says Pause or Resume in the menu, whichever comes next.
    pub fn set_notifications_paused(&self, paused: bool) {
        let labels = self.labels.lock().unwrap_or_else(|e| e.into_inner());
        let label = if paused { &labels.resume_notifications } else { &labels.pause_notifications };
        crate::bridge::native::ffi::linux_tray_set_pause_label(label);
    }

    /// Whether "Find my phone" is offered (a phone is paired).
    pub fn set_find_phone_enabled(&self, enabled: bool) {
        if let Ok(mut guard) = self.find_phone_enabled.lock() {
            *guard = enabled;
        }
        if !cfg!(test) {
            crate::bridge::native::ffi::linux_tray_set_find_phone_enabled(enabled);
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.handler.clear();
        if !cfg!(test) {
            crate::bridge::native::ffi::linux_tray_teardown();
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
        assert!(!is_window_alive_and_visible(0));
        assert!(!focus_window(0));
        // Non-existent window handle must honestly report false rather than faking focus.
        {
            let mut guard = LAST_EXTERNAL_WINDOW.lock().unwrap_or_else(|e| e.into_inner());
            *guard = Some((0x7FFF_4242, "Nonexistent Window".into()));
        }
        assert!(!is_window_alive_and_visible(0x7FFF_4242));
        assert!(!focus_window(0x7FFF_4242));

        let labels = MenuLabels {
            open: "Open".into(),
            command_palette: "Command Palette".into(),
            shelf: "Shelf".into(),
            take_photo: "Take photo".into(),
            scan_document: "Scan document".into(),
            find_phone: "Find phone".into(),
            open_link: "Open link".into(),
            pause_notifications: "Pause".into(),
            resume_notifications: "Resume".into(),
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

    #[test]
    fn portal_shortcut_mapping_and_triggers_work() {
        assert_eq!(parse_portal_shortcut_id("take-photo"), Some(TrayEvent::TakePhoto));
        assert_eq!(parse_portal_shortcut_id("scan-document"), Some(TrayEvent::ScanDocument));
        assert_eq!(parse_portal_shortcut_id("command-palette"), Some(TrayEvent::CommandPalette));
        assert_eq!(parse_portal_shortcut_id("shelf"), Some(TrayEvent::Shelf));
        assert_eq!(parse_portal_shortcut_id("unknown"), None);

        assert_eq!(hotkey_to_portal_trigger("Ctrl+Alt+C").as_deref(), Some("CTRL+ALT+c"));
        assert_eq!(hotkey_to_portal_trigger("Ctrl+Alt+Space").as_deref(), Some("CTRL+ALT+space"));
        assert_eq!(hotkey_to_portal_trigger("Super+Shift+F9").as_deref(), Some("SHIFT+LOGO+F9"));
        assert!(hotkey_to_portal_trigger("none").is_none());
    }

    #[test]
    fn shortcuts_the_portal_already_has_are_not_bound_again() {
        use zbus::zvariant::Value;
        let specs =
            ("Ctrl+Alt+C".to_owned(), String::new(), "Ctrl+Alt+Space".to_owned(), "Ctrl+Alt+S".to_owned());
        let wanted = portal_shortcut_ids(&specs);
        assert_eq!(wanted, ["take-photo", "command-palette", "shelf"]);

        // As the portal sends it: a(sa{sv}).
        let entry = |id: &str| (id.to_owned(), std::collections::HashMap::<String, Value<'_>>::new());
        let listed = Value::from(vec![entry("take-photo"), entry("command-palette"), entry("shelf")]);
        assert_eq!(listed.value_signature().to_string(), "a(sa{sv})");
        let ids = listed_shortcut_ids(&listed);
        assert!(portal_has_shortcuts(&ids, &wanted));
        assert!(!portal_has_shortcuts(&ids[..2], &wanted));
        assert!(listed_shortcut_ids(&Value::from(3u32)).is_empty());
    }

    #[test]
    fn x11_hotkey_mapping_covers_modifiers_and_virtual_keys() {
        let (mods, vk) = parse_hotkey("Ctrl+Alt+Space").expect("Ctrl+Alt+Space");
        assert_eq!(mods_to_x11_mask(mods), X11_CONTROL_MASK | X11_MOD1_MASK);
        assert_eq!(vk_to_x11_keysym(vk), Some(0x0020));

        let (mods, vk) = parse_hotkey("Super+Shift+C").expect("Super+Shift+C");
        assert_eq!(mods_to_x11_mask(mods), X11_MOD4_MASK | X11_SHIFT_MASK);
        assert_eq!(vk_to_x11_keysym(vk), Some(u64::from(b'c')));

        let (_, vk_f1) = parse_hotkey("Ctrl+F1").expect("Ctrl+F1");
        assert_eq!(vk_to_x11_keysym(vk_f1), Some(0xFFBE));
        let (_, vk_f12) = parse_hotkey("Ctrl+F12").expect("Ctrl+F12");
        assert_eq!(vk_to_x11_keysym(vk_f12), Some(0xFFBE + 11));
        assert_eq!(vk_to_x11_keysym(0x0D), Some(0xFF0D));
        assert_eq!(vk_to_x11_keysym(0x1B), Some(0xFF1B));
        assert_eq!(vk_to_x11_keysym(0xFFFF), None);

        let before = HOTKEY_GENERATION.load(Ordering::Acquire);
        update_hotkeys("Ctrl+Alt+C", "Ctrl+Alt+D", "Ctrl+Alt+Space", "Ctrl+Alt+S");
        assert!(HOTKEY_GENERATION.load(Ordering::Acquire) > before);
    }
}
