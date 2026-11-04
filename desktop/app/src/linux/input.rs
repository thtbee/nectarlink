// SPDX-License-Identifier: GPL-3.0-or-later
//! Synthesizes mouse and keyboard input on Linux for a paired phone acting as
//! a touchpad, keyboard, or presentation remote (`docs/protocol/remote.md`),
//! as well as reverse PC mirror touch injection.
//!
//! Input backends are tried in order of lowest overhead and broadest display-server
//! support:
//! 1. In-process `/dev/uinput` virtual input device (works on both X11 and Wayland).
//! 2. On Wayland: in-process `org.freedesktop.portal.RemoteDesktop` D-Bus session.
//! 3. On X11 / XWayland: in-process `libX11.so.6` + `libXtst.so.6` (`XTest`).
//! 4. Subprocess fallbacks (`ydotool`, `wtype`, `xdotool`), piping text via stdin
//!    so typed strings never appear in `/proc/<pid>/cmdline`.

use std::{
    collections::HashMap,
    ffi::{CStr, CString, c_char, c_int, c_uint, c_ulong, c_void},
    io::Write,
    process::{Command, Stdio},
    sync::{Mutex, OnceLock},
};

use nectarlink_core::{ButtonAction, KeyMod, MouseButton, PowerAction, SlideAction, TouchAction};

// ---- Linux `/dev/uinput` kernel ABI constants ----
const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_REL: u16 = 0x02;

const SYN_REPORT: u16 = 0x00;

const REL_X: u16 = 0x00;
const REL_Y: u16 = 0x01;
const REL_HWHEEL: u16 = 0x06;
const REL_WHEEL: u16 = 0x08;

const BTN_LEFT: u16 = 0x110;
const BTN_RIGHT: u16 = 0x111;
const BTN_MIDDLE: u16 = 0x112;

const BUS_USB: u16 = 0x03;

const UI_DEV_CREATE: c_ulong = 0x5501;
const UI_DEV_DESTROY: c_ulong = 0x5502;
const UI_DEV_SETUP: c_ulong = 0x405c_5503;
const UI_SET_EVBIT: c_ulong = 0x4004_5564;
const UI_SET_KEYBIT: c_ulong = 0x4004_5565;
const UI_SET_RELBIT: c_ulong = 0x4004_5566;

#[repr(C)]
struct InputId {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

#[repr(C)]
struct UinputSetup {
    id: InputId,
    name: [u8; 80],
    ff_effects_max: u32,
}

#[repr(C)]
struct InputEvent {
    tv_sec: libc::time_t,
    tv_usec: libc::suseconds_t,
    type_: u16,
    code: u16,
    value: i32,
}

struct UinputDevice {
    fd: c_int,
}

impl UinputDevice {
    fn open() -> Option<Self> {
        let paths = [c"/dev/uinput", c"/dev/input/uinput"];
        let mut fd = -1;
        for path in paths {
            // SAFETY: `path` is a valid null-terminated C string.
            let opened =
                unsafe { libc::open(path.as_ptr(), libc::O_WRONLY | libc::O_NONBLOCK | libc::O_CLOEXEC) };
            if opened >= 0 {
                fd = opened;
                break;
            }
        }
        if fd < 0 {
            return None;
        }

        // SAFETY: `fd` is an open `/dev/uinput` descriptor; ioctl configures supported event bits.
        unsafe {
            if libc::ioctl(fd, UI_SET_EVBIT, c_int::from(EV_SYN)) < 0
                || libc::ioctl(fd, UI_SET_EVBIT, c_int::from(EV_KEY)) < 0
                || libc::ioctl(fd, UI_SET_EVBIT, c_int::from(EV_REL)) < 0
            {
                let _ = libc::close(fd);
                return None;
            }

            for rel in [REL_X, REL_Y, REL_WHEEL, REL_HWHEEL] {
                if libc::ioctl(fd, UI_SET_RELBIT, c_int::from(rel)) < 0 {
                    let _ = libc::close(fd);
                    return None;
                }
            }

            for btn in [BTN_LEFT, BTN_RIGHT, BTN_MIDDLE] {
                if libc::ioctl(fd, UI_SET_KEYBIT, c_int::from(btn)) < 0 {
                    let _ = libc::close(fd);
                    return None;
                }
            }

            for key in 1..=248 {
                let _ = libc::ioctl(fd, UI_SET_KEYBIT, key as c_int);
            }

            let mut setup = UinputSetup {
                id: InputId { bustype: BUS_USB, vendor: 0x1d6b, product: 0x0104, version: 1 },
                name: [0u8; 80],
                ff_effects_max: 0,
            };
            let dev_name = b"NectarLink Virtual Input";
            setup.name[..dev_name.len()].copy_from_slice(dev_name);

            if libc::ioctl(fd, UI_DEV_SETUP, &setup) < 0 || libc::ioctl(fd, UI_DEV_CREATE) < 0 {
                let _ = libc::close(fd);
                return None;
            }
        }

        Some(Self { fd })
    }

    fn emit(&self, type_: u16, code: u16, value: i32) -> bool {
        let ev = InputEvent { tv_sec: 0, tv_usec: 0, type_, code, value };
        let size = std::mem::size_of::<InputEvent>();
        // SAFETY: `ev` is a repr(C) `input_event` struct written atomically to `self.fd`.
        let written = unsafe { libc::write(self.fd, (&raw const ev).cast::<c_void>(), size) };
        written == size as isize
    }

    fn syn(&self) -> bool {
        self.emit(EV_SYN, SYN_REPORT, 0)
    }

    fn move_relative(&self, dx: i32, dy: i32) -> bool {
        let mut ok = true;
        if dx != 0 {
            ok &= self.emit(EV_REL, REL_X, dx);
        }
        if dy != 0 {
            ok &= self.emit(EV_REL, REL_Y, dy);
        }
        ok && self.syn()
    }

    fn button(&self, btn: u16, down: bool) -> bool {
        self.emit(EV_KEY, btn, i32::from(down)) && self.syn()
    }

    fn scroll(&self, nx: i32, ny: i32) -> bool {
        let mut ok = true;
        if ny != 0 {
            // In evdev, positive REL_WHEEL is scroll up; our `ny > 0` is scroll down.
            ok &= self.emit(EV_REL, REL_WHEEL, -ny);
        }
        if nx != 0 {
            ok &= self.emit(EV_REL, REL_HWHEEL, nx);
        }
        ok && self.syn()
    }

    fn key_combo(&self, keycode: u16, mods: &[u16]) -> bool {
        let mut ok = true;
        for &m in mods {
            ok &= self.emit(EV_KEY, m, 1);
        }
        ok &= self.emit(EV_KEY, keycode, 1);
        ok &= self.syn();
        ok &= self.emit(EV_KEY, keycode, 0);
        for &m in mods.iter().rev() {
            ok &= self.emit(EV_KEY, m, 0);
        }
        ok && self.syn()
    }
}

impl Drop for UinputDevice {
    fn drop(&mut self) {
        if self.fd >= 0 {
            // SAFETY: `self.fd` is a valid uinput descriptor owned by `self`.
            unsafe {
                let _ = libc::ioctl(self.fd, UI_DEV_DESTROY);
                let _ = libc::close(self.fd);
            }
        }
    }
}

fn with_uinput<F: FnOnce(&UinputDevice) -> bool>(f: F) -> bool {
    static UINPUT: OnceLock<Mutex<Option<UinputDevice>>> = OnceLock::new();
    let mutex = UINPUT.get_or_init(|| Mutex::new(UinputDevice::open()));
    let guard = mutex.lock().unwrap_or_else(|e| e.into_inner());
    guard.as_ref().is_some_and(f)
}

fn evdev_button_code(button: MouseButton) -> u16 {
    match button {
        MouseButton::Left => BTN_LEFT,
        MouseButton::Right => BTN_RIGHT,
        MouseButton::Middle => BTN_MIDDLE,
    }
}

pub(crate) fn evdev_key_for_mod(m: KeyMod) -> u16 {
    match m {
        KeyMod::Ctrl => 29,  // KEY_LEFTCTRL
        KeyMod::Shift => 42, // KEY_LEFTSHIFT
        KeyMod::Alt => 56,   // KEY_LEFTALT
        KeyMod::Win => 125,  // KEY_LEFTMETA
    }
}

/// Maps a named X11 keysym (as returned by [`resolve_key`]) to a Linux evdev `KEY_*` code.
/// Letters and digits aren't here: where they are depends on the layout ([`key_code_for_sym`]).
pub(crate) fn evdev_key_for_sym(sym: &str) -> Option<u16> {
    let code = match sym {
        "Escape" => 1,
        "BackSpace" => 14,
        "Tab" => 15,
        "Return" => 28,
        "space" => 57,
        "F1" => 59,
        "F2" => 60,
        "F3" => 61,
        "F4" => 62,
        "F5" => 63,
        "F6" => 64,
        "F7" => 65,
        "F8" => 66,
        "F9" => 67,
        "F10" => 68,
        "F11" => 87,
        "F12" => 88,
        "Print" => 99,
        "Home" => 102,
        "Up" => 103,
        "Prior" => 104,
        "Left" => 105,
        "Right" => 106,
        "End" => 107,
        "Down" => 108,
        "Next" => 109,
        "Insert" => 110,
        "Delete" => 111,
        "XF86AudioMute" => 113,
        "XF86AudioLowerVolume" => 114,
        "XF86AudioRaiseVolume" => 115,
        "XF86AudioNext" => 163,
        "XF86AudioPlay" => 164,
        "XF86AudioPrev" => 165,
        _ => return None,
    };
    Some(code)
}

/// The modifier keys a [`KeyPress`](super::keymap::KeyPress) needs held.
fn press_mods(press: &super::keymap::KeyPress) -> Vec<u16> {
    let mut mods = Vec::with_capacity(2);
    if press.shift {
        mods.push(evdev_key_for_mod(KeyMod::Shift));
    }
    if press.altgr {
        mods.push(super::keymap::KEY_RIGHTALT);
    }
    mods
}

/// The evdev key for a shortcut's key name. Named keys (arrows, F-keys,
/// Enter) are in the same place on every layout; a letter isn't (Z is
/// where Y is on a German keyboard), so letters go through the user's
/// layout, and without a known layout there is no key code to send.
fn key_code_for_sym(sym: &str) -> Option<u16> {
    let mut chars = sym.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) => {
            super::keymap::current()?.press_for(ch).filter(|p| !p.shift && !p.altgr).map(|p| p.code)
        }
        _ => evdev_key_for_sym(sym),
    }
}

/// Converts a Unicode character to its X11 keysym (`XK_*` or `0x0100_0000 | codepoint`).
pub(crate) fn char_to_x11_keysym(ch: char) -> u32 {
    match ch {
        '\n' => 0xff0d,     // XK_Return
        '\t' => 0xff09,     // XK_Tab
        '\u{08}' => 0xff08, // XK_BackSpace
        '\u{1b}' => 0xff1b, // XK_Escape
        ' '..='~' | '\u{a0}'..='\u{ff}' => ch as u32,
        c => 0x0100_0000 | (c as u32),
    }
}

// ---- Wayland `org.freedesktop.portal.RemoteDesktop` backend ----
struct RemoteDesktopPortal {
    conn: zbus::blocking::Connection,
    session_path: zbus::zvariant::OwnedObjectPath,
}

impl RemoteDesktopPortal {
    fn connect() -> Option<Self> {
        let conn = zbus::blocking::Connection::session().ok()?;
        if !super::dbus_name_has_owner(&conn, "org.freedesktop.portal.Desktop") {
            return None;
        }
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface("org.freedesktop.portal.Request")
            .and_then(|b| b.member("Response"))
            .map(|b| b.build())
            .ok()?;
        let mut iter = zbus::blocking::MessageIterator::for_match_rule(rule, &conn, None).ok()?;

        let pid = std::process::id();
        let session_token = format!("nectarlink_rd_sess_{pid}");
        let create_token = format!("nectarlink_rd_create_{pid}");

        let mut create_opts: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::new();
        create_opts.insert("handle_token", zbus::zvariant::Value::from(create_token));
        create_opts.insert("session_handle_token", zbus::zvariant::Value::from(session_token));

        let create_reply = conn
            .call_method(
                Some("org.freedesktop.portal.Desktop"),
                "/org/freedesktop/portal/desktop",
                Some("org.freedesktop.portal.RemoteDesktop"),
                "CreateSession",
                &(&create_opts,),
            )
            .ok()?;
        let create_req_path =
            create_reply.body().deserialize::<zbus::zvariant::OwnedObjectPath>().ok().map(|p| p.to_string());
        let create_results = Self::wait_portal_response(&mut iter, create_req_path.as_deref())?;
        let session_str =
            create_results.get("session_handle").and_then(|v| <&str>::try_from(v).ok())?.to_owned();
        let session_path = zbus::zvariant::OwnedObjectPath::try_from(session_str).ok()?;

        // SelectDevices: types = 3 (1 = KEYBOARD | 2 = POINTER).
        let select_token = format!("nectarlink_rd_select_{pid}");
        let mut select_opts: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::new();
        select_opts.insert("handle_token", zbus::zvariant::Value::from(select_token));
        select_opts.insert("types", zbus::zvariant::Value::U32(3));

        let select_reply = conn
            .call_method(
                Some("org.freedesktop.portal.Desktop"),
                "/org/freedesktop/portal/desktop",
                Some("org.freedesktop.portal.RemoteDesktop"),
                "SelectDevices",
                &(&session_path, &select_opts),
            )
            .ok()?;
        let select_req_path =
            select_reply.body().deserialize::<zbus::zvariant::OwnedObjectPath>().ok().map(|p| p.to_string());
        Self::wait_portal_response(&mut iter, select_req_path.as_deref())?;

        // Start session.
        let start_token = format!("nectarlink_rd_start_{pid}");
        let mut start_opts: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::new();
        start_opts.insert("handle_token", zbus::zvariant::Value::from(start_token));

        let start_reply = conn
            .call_method(
                Some("org.freedesktop.portal.Desktop"),
                "/org/freedesktop/portal/desktop",
                Some("org.freedesktop.portal.RemoteDesktop"),
                "Start",
                &(&session_path, "", &start_opts),
            )
            .ok()?;
        let start_req_path =
            start_reply.body().deserialize::<zbus::zvariant::OwnedObjectPath>().ok().map(|p| p.to_string());
        Self::wait_portal_response(&mut iter, start_req_path.as_deref())?;

        Some(Self { conn, session_path })
    }

    fn wait_portal_response(
        iter: &mut zbus::blocking::MessageIterator,
        expected_path: Option<&str>,
    ) -> Option<HashMap<String, zbus::zvariant::OwnedValue>> {
        for msg in iter.take(32) {
            let msg = msg.ok()?;
            if let Some(expected) = expected_path
                && msg.header().path().map(|p| p.as_str()) != Some(expected)
            {
                continue;
            }
            if let Ok((code, results)) =
                msg.body().deserialize::<(u32, HashMap<String, zbus::zvariant::OwnedValue>)>()
            {
                return (code == 0).then_some(results);
            }
        }
        None
    }

    fn move_relative(&self, dx: f64, dy: f64) -> bool {
        let opts: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::new();
        self.conn
            .call_method(
                Some("org.freedesktop.portal.Desktop"),
                "/org/freedesktop/portal/desktop",
                Some("org.freedesktop.portal.RemoteDesktop"),
                "NotifyPointerMotion",
                &(&self.session_path, &opts, dx, dy),
            )
            .is_ok()
    }

    fn move_absolute(&self, x: f64, y: f64) -> bool {
        let opts: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::new();
        self.conn
            .call_method(
                Some("org.freedesktop.portal.Desktop"),
                "/org/freedesktop/portal/desktop",
                Some("org.freedesktop.portal.RemoteDesktop"),
                "NotifyPointerMotionAbsolute",
                &(&self.session_path, &opts, 0u32, x, y),
            )
            .is_ok()
    }

    fn button(&self, evdev_btn: i32, down: bool) -> bool {
        let opts: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::new();
        let state: u32 = u32::from(down);
        self.conn
            .call_method(
                Some("org.freedesktop.portal.Desktop"),
                "/org/freedesktop/portal/desktop",
                Some("org.freedesktop.portal.RemoteDesktop"),
                "NotifyPointerButton",
                &(&self.session_path, &opts, evdev_btn, state),
            )
            .is_ok()
    }

    fn scroll(&self, nx: i32, ny: i32) -> bool {
        let opts: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::new();
        let mut ok = true;
        if ny != 0 {
            ok &= self
                .conn
                .call_method(
                    Some("org.freedesktop.portal.Desktop"),
                    "/org/freedesktop/portal/desktop",
                    Some("org.freedesktop.portal.RemoteDesktop"),
                    "NotifyPointerAxisDiscrete",
                    &(&self.session_path, &opts, 0u32, ny),
                )
                .is_ok();
        }
        if nx != 0 {
            ok &= self
                .conn
                .call_method(
                    Some("org.freedesktop.portal.Desktop"),
                    "/org/freedesktop/portal/desktop",
                    Some("org.freedesktop.portal.RemoteDesktop"),
                    "NotifyPointerAxisDiscrete",
                    &(&self.session_path, &opts, 1u32, nx),
                )
                .is_ok();
        }
        ok
    }

    fn keycode(&self, evdev_key: i32, down: bool) -> bool {
        let opts: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::new();
        let state: u32 = u32::from(down);
        self.conn
            .call_method(
                Some("org.freedesktop.portal.Desktop"),
                "/org/freedesktop/portal/desktop",
                Some("org.freedesktop.portal.RemoteDesktop"),
                "NotifyKeyboardKeycode",
                &(&self.session_path, &opts, evdev_key, state),
            )
            .is_ok()
    }

    fn keysym(&self, keysym: i32, down: bool) -> bool {
        let opts: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::new();
        let state: u32 = u32::from(down);
        self.conn
            .call_method(
                Some("org.freedesktop.portal.Desktop"),
                "/org/freedesktop/portal/desktop",
                Some("org.freedesktop.portal.RemoteDesktop"),
                "NotifyKeyboardKeysym",
                &(&self.session_path, &opts, keysym, state),
            )
            .is_ok()
    }
}

fn with_wayland_portal<F: FnOnce(&RemoteDesktopPortal) -> bool + Send>(f: F) -> bool {
    if !has_wayland_display() {
        return false;
    }
    super::non_tokio(|| {
        static PORTAL: OnceLock<Mutex<Option<RemoteDesktopPortal>>> = OnceLock::new();
        let mutex = PORTAL.get_or_init(|| Mutex::new(RemoteDesktopPortal::connect()));
        let guard = mutex.lock().unwrap_or_else(|e| e.into_inner());
        guard.as_ref().is_some_and(f)
    })
}

fn has_remote_desktop_portal() -> bool {
    super::non_tokio(|| {
        let Ok(conn) = zbus::blocking::Connection::session() else {
            return false;
        };
        if !super::dbus_name_has_owner(&conn, "org.freedesktop.portal.Desktop") {
            return false;
        }
        let Ok(msg) = conn.call_method(
            Some("org.freedesktop.portal.Desktop"),
            "/org/freedesktop/portal/desktop",
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.freedesktop.portal.RemoteDesktop", "AvailableDeviceTypes"),
        ) else {
            return false;
        };
        if let Ok(val) = msg.body().deserialize::<zbus::zvariant::OwnedValue>()
            && let Ok(types) = u32::try_from(val)
        {
            return types != 0;
        }
        true
    })
}

// ---- X11 `libX11.so.6` + `libXtst.so.6` (`XTest`) backend ----
struct X11TestApi {
    _x11_lib: *mut c_void,
    _xtst_lib: *mut c_void,
    open_display: unsafe extern "C" fn(*const c_char) -> *mut c_void,
    flush: unsafe extern "C" fn(*mut c_void) -> c_int,
    string_to_keysym: unsafe extern "C" fn(*const c_char) -> c_ulong,
    keysym_to_keycode: unsafe extern "C" fn(*mut c_void, c_ulong) -> u8,
    keycode_to_keysym: Option<unsafe extern "C" fn(*mut c_void, u8, c_int) -> c_ulong>,
    query_extension:
        unsafe extern "C" fn(*mut c_void, *mut c_int, *mut c_int, *mut c_int, *mut c_int) -> c_int,
    fake_motion: unsafe extern "C" fn(*mut c_void, c_int, c_int, c_int, c_ulong) -> c_int,
    fake_relative_motion: unsafe extern "C" fn(*mut c_void, c_int, c_int, c_ulong) -> c_int,
    fake_button: unsafe extern "C" fn(*mut c_void, c_uint, c_int, c_ulong) -> c_int,
    fake_key: unsafe extern "C" fn(*mut c_void, c_uint, c_int, c_ulong) -> c_int,
}

// SAFETY: Function pointers loaded from libX11.so.6 and libXtst.so.6 are process-wide.
unsafe impl Send for X11TestApi {}
unsafe impl Sync for X11TestApi {}

struct X11Display(*mut c_void);

// SAFETY: Access to the Display pointer is serialized through a Mutex.
unsafe impl Send for X11Display {}

unsafe fn load_sym<T>(handle: *mut c_void, name: &CStr) -> Option<T> {
    let raw = unsafe { libc::dlsym(handle, name.as_ptr()) };
    if raw.is_null() {
        return None;
    }
    Some(unsafe { std::mem::transmute_copy(&raw) })
}

fn x11_test_api() -> Option<&'static X11TestApi> {
    static API: OnceLock<Option<X11TestApi>> = OnceLock::new();
    API.get_or_init(|| unsafe {
        let x11_lib = libc::dlopen(c"libX11.so.6".as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL);
        if x11_lib.is_null() {
            return None;
        }
        let xtst_lib = libc::dlopen(c"libXtst.so.6".as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL);
        if xtst_lib.is_null() {
            let _ = libc::dlclose(x11_lib);
            return None;
        }
        let api = X11TestApi {
            _x11_lib: x11_lib,
            _xtst_lib: xtst_lib,
            open_display: load_sym(x11_lib, c"XOpenDisplay")?,
            flush: load_sym(x11_lib, c"XFlush")?,
            string_to_keysym: load_sym(x11_lib, c"XStringToKeysym")?,
            keysym_to_keycode: load_sym(x11_lib, c"XKeysymToKeycode")?,
            keycode_to_keysym: load_sym(x11_lib, c"XKeycodeToKeysym"),
            query_extension: load_sym(xtst_lib, c"XTestQueryExtension")?,
            fake_motion: load_sym(xtst_lib, c"XTestFakeMotionEvent")?,
            fake_relative_motion: load_sym(xtst_lib, c"XTestFakeRelativeMotionEvent")?,
            fake_button: load_sym(xtst_lib, c"XTestFakeButtonEvent")?,
            fake_key: load_sym(xtst_lib, c"XTestFakeKeyEvent")?,
        };
        Some(api)
    })
    .as_ref()
}

fn with_xtest<F: FnOnce(&X11TestApi, *mut c_void)>(f: F) -> bool {
    if std::env::var_os("DISPLAY").is_none() {
        return false;
    }
    let Some(api) = x11_test_api() else {
        return false;
    };
    static DISPLAY_CONN: OnceLock<Mutex<Option<X11Display>>> = OnceLock::new();
    let mutex = DISPLAY_CONN.get_or_init(|| Mutex::new(None));
    let mut guard = mutex.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        // SAFETY: XOpenDisplay(NULL) connects to $DISPLAY; XTestQueryExtension checks XTEST support.
        let dpy = unsafe { (api.open_display)(std::ptr::null()) };
        if dpy.is_null() {
            return false;
        }
        let mut ev = 0;
        let mut err = 0;
        let mut maj = 0;
        let mut min = 0;
        let ok = unsafe { (api.query_extension)(dpy, &mut ev, &mut err, &mut maj, &mut min) };
        if ok == 0 {
            return false;
        }
        *guard = Some(X11Display(dpy));
    }
    if let Some(X11Display(dpy)) = *guard {
        f(api, dpy);
        // SAFETY: dpy is a valid open X11 Display pointer protected by DISPLAY_CONN mutex.
        unsafe {
            let _ = (api.flush)(dpy);
        }
        true
    } else {
        false
    }
}

fn uinput_writable() -> bool {
    std::fs::OpenOptions::new().write(true).open("/dev/uinput").is_ok()
        || std::fs::OpenOptions::new().write(true).open("/dev/input/uinput").is_ok()
}

/// Returns `true` when input synthesis is supported in the current desktop session.
pub fn is_available() -> bool {
    if uinput_writable() {
        return true;
    }
    if has_x11_display() && (x11_test_api().is_some() || has_executable_in_path("xdotool")) {
        return true;
    }
    if has_wayland_display()
        && (has_executable_in_path("ydotool")
            || has_executable_in_path("wtype")
            || has_remote_desktop_portal())
    {
        return true;
    }
    false
}

fn has_executable_in_path(binary: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(binary).is_file()))
}

/// Fractional remainders for sub-pixel pointer motion and sub-notch scrolling,
/// plus which mouse buttons are currently held down by remote input.
#[derive(Default, Debug, Clone, PartialEq)]
struct InputState {
    move_rem_x: f32,
    move_rem_y: f32,
    scroll_rem_x: f32,
    scroll_rem_y: f32,
    held_left: bool,
    held_right: bool,
    held_middle: bool,
}

impl InputState {
    fn accumulate_move(&mut self, dx: f32, dy: f32) -> (i32, i32) {
        let total_x = self.move_rem_x + dx;
        let total_y = self.move_rem_y + dy;
        let ix = total_x.round() as i32;
        let iy = total_y.round() as i32;
        self.move_rem_x = total_x - (ix as f32);
        self.move_rem_y = total_y - (iy as f32);
        (ix, iy)
    }

    fn accumulate_scroll(&mut self, dx: f32, dy: f32) -> (i32, i32) {
        let total_x = self.scroll_rem_x + dx;
        let total_y = self.scroll_rem_y + dy;
        let nx = total_x.round() as i32;
        let ny = total_y.round() as i32;
        self.scroll_rem_x = total_x - (nx as f32);
        self.scroll_rem_y = total_y - (ny as f32);
        (nx, ny)
    }

    fn record_button(&mut self, button: MouseButton, action: ButtonAction) {
        let slot = match button {
            MouseButton::Left => &mut self.held_left,
            MouseButton::Right => &mut self.held_right,
            MouseButton::Middle => &mut self.held_middle,
        };
        *slot = matches!(action, ButtonAction::Down);
    }

    fn take_held_and_reset(&mut self) -> (bool, bool, bool) {
        let held = (self.held_left, self.held_right, self.held_middle);
        self.held_left = false;
        self.held_right = false;
        self.held_middle = false;
        self.move_rem_x = 0.0;
        self.move_rem_y = 0.0;
        self.scroll_rem_x = 0.0;
        self.scroll_rem_y = 0.0;
        held
    }
}

static STATE: Mutex<InputState> = Mutex::new(InputState {
    move_rem_x: 0.0,
    move_rem_y: 0.0,
    scroll_rem_x: 0.0,
    scroll_rem_y: 0.0,
    held_left: false,
    held_right: false,
    held_middle: false,
});

fn has_display_env() -> bool {
    std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

fn has_wayland_display() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty())
}

fn has_x11_display() -> bool {
    std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty())
}

fn run_quiet(program: &str, args: &[&str]) -> bool {
    if !has_display_env() {
        return false;
    }
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn run_with_stdin(program: &str, args: &[&str], input: &[u8]) -> bool {
    if !has_display_env() {
        return false;
    }
    let Ok(mut child) = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    if let Some(mut stdin) = child.stdin.take()
        && stdin.write_all(input).is_err()
    {
        let _ = child.kill();
        let _ = child.wait();
        return false;
    }
    child.wait().is_ok_and(|s| s.success())
}

fn display_dimensions() -> (i32, i32) {
    let mut sx = 0;
    let mut sy = 0;
    let mut sw = 0;
    let mut sh = 0;
    if crate::bridge::native::ffi::linux_screen_geometry(&mut sx, &mut sy, &mut sw, &mut sh)
        && sw > 0
        && sh > 0
    {
        return (sw, sh);
    }
    if let Some((x, y, w, h)) = super::work_area() {
        let total_w = (x.max(0) + w).max(1);
        let total_h = (y.max(0) + h).max(1);
        if total_w > 0 && total_h > 0 {
            return (total_w, total_h);
        }
    }
    if has_display_env()
        && let Ok(out) = Command::new("xdotool")
            .arg("getdisplaygeometry")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
        && out.status.success()
    {
        let text = String::from_utf8_lossy(&out.stdout);
        let mut parts = text.split_whitespace().filter_map(|s| s.parse::<i32>().ok());
        if let (Some(w), Some(h)) = (parts.next(), parts.next())
            && w > 0
            && h > 0
        {
            return (w, h);
        }
    }
    (1920, 1080)
}

fn normalized_to_pixels(x: f32, y: f32, width: i32, height: i32) -> (i32, i32) {
    let max_x = (width - 1).max(1) as f32;
    let max_y = (height - 1).max(1) as f32;
    let px = (x.clamp(0.0, 1.0) * max_x).round() as i32;
    let py = (y.clamp(0.0, 1.0) * max_y).round() as i32;
    (px, py)
}

/// Moves the mouse cursor relatively by `(dx, dy)` pixels, keeping sub-pixel
/// fractions for the next move.
pub fn move_relative(dx: f32, dy: f32) {
    if !dx.is_finite() || !dy.is_finite() {
        return;
    }
    let (ix, iy) = {
        let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
        s.accumulate_move(dx, dy)
    };
    if ix == 0 && iy == 0 {
        return;
    }
    if with_uinput(|u| u.move_relative(ix, iy)) {
        return;
    }
    if with_wayland_portal(|p| p.move_relative(f64::from(ix), f64::from(iy))) {
        return;
    }
    let sx = ix.to_string();
    let sy = iy.to_string();
    if has_wayland_display() && run_quiet("ydotool", &["mousemove", "-x", &sx, "-y", &sy]) {
        return;
    }
    if with_xtest(|api, dpy| unsafe {
        let _ = (api.fake_relative_motion)(dpy, ix, iy, 0);
    }) {
        return;
    }
    if !run_quiet("xdotool", &["mousemove_relative", "--", &sx, &sy]) {
        let _ = run_quiet("ydotool", &["mousemove", "-x", &sx, "-y", &sy]);
    }
}

/// Moves the mouse cursor to normalized coordinates `(x, y)` on the primary
/// display (`0.0..=1.0`).
pub fn move_absolute(x: f32, y: f32) {
    if !x.is_finite() || !y.is_finite() {
        return;
    }
    let (w, h) = display_dimensions();
    let (px, py) = normalized_to_pixels(x, y, w, h);
    if with_wayland_portal(|p| p.move_absolute(f64::from(px), f64::from(py))) {
        return;
    }
    let sx = px.to_string();
    let sy = py.to_string();
    if has_wayland_display() && run_quiet("ydotool", &["mousemove", "--absolute", "-x", &sx, "-y", &sy]) {
        return;
    }
    if with_xtest(|api, dpy| unsafe {
        let _ = (api.fake_motion)(dpy, -1, px, py, 0);
    }) {
        return;
    }
    if !run_quiet("xdotool", &["mousemove", "--", &sx, &sy]) {
        let _ = run_quiet("ydotool", &["mousemove", "--absolute", "-x", &sx, "-y", &sy]);
    }
}

/// Injects a normalized touch event (`0.0..=1.0`) from reverse PC mirroring as
/// absolute mouse movement + left button press/release.
pub fn mirror_touch(action: TouchAction, x: f32, y: f32) {
    if !x.is_finite() || !y.is_finite() {
        return;
    }
    let (w, h) = display_dimensions();
    let (px, py) = normalized_to_pixels(x, y, w, h);
    let sx = px.to_string();
    let sy = py.to_string();

    match action {
        TouchAction::Down => {
            if let Ok(mut s) = STATE.lock() {
                s.held_left = true;
            }
            if !has_wayland_display()
                && with_xtest(|api, dpy| unsafe {
                    let _ = (api.fake_motion)(dpy, -1, px, py, 0);
                    let _ = (api.fake_button)(dpy, 1, 1, 0);
                })
            {
                return;
            }
            if has_wayland_display()
                || !run_quiet("xdotool", &["mousemove", "--", &sx, &sy, "mousedown", "1"])
            {
                move_absolute(x, y);
                mouse_button(MouseButton::Left, ButtonAction::Down);
            }
        }
        TouchAction::Move => {
            move_absolute(x, y);
        }
        TouchAction::Up => {
            if let Ok(mut s) = STATE.lock() {
                s.held_left = false;
            }
            if !has_wayland_display()
                && with_xtest(|api, dpy| unsafe {
                    let _ = (api.fake_motion)(dpy, -1, px, py, 0);
                    let _ = (api.fake_button)(dpy, 1, 0, 0);
                })
            {
                return;
            }
            if has_wayland_display() || !run_quiet("xdotool", &["mousemove", "--", &sx, &sy, "mouseup", "1"])
            {
                move_absolute(x, y);
                mouse_button(MouseButton::Left, ButtonAction::Up);
            }
        }
    }
}

fn x11_button_num(button: MouseButton) -> c_uint {
    match button {
        MouseButton::Left => 1,
        MouseButton::Middle => 2,
        MouseButton::Right => 3,
    }
}

fn xdotool_button_id(button: MouseButton) -> &'static str {
    match button {
        MouseButton::Left => "1",
        MouseButton::Middle => "2",
        MouseButton::Right => "3",
    }
}

/// Presses, releases or clicks a mouse button.
pub fn mouse_button(button: MouseButton, action: ButtonAction) {
    {
        let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
        s.record_button(button, action);
    }
    let evdev_btn = evdev_button_code(button);
    if with_uinput(|u| match action {
        ButtonAction::Click => u.button(evdev_btn, true) && u.button(evdev_btn, false),
        ButtonAction::Down => u.button(evdev_btn, true),
        ButtonAction::Up => u.button(evdev_btn, false),
    }) {
        return;
    }
    if with_wayland_portal(|p| match action {
        ButtonAction::Click => p.button(i32::from(evdev_btn), true) && p.button(i32::from(evdev_btn), false),
        ButtonAction::Down => p.button(i32::from(evdev_btn), true),
        ButtonAction::Up => p.button(i32::from(evdev_btn), false),
    }) {
        return;
    }
    let ydo_code = match (button, action) {
        (MouseButton::Left, ButtonAction::Click) => "0xC0",
        (MouseButton::Left, ButtonAction::Down) => "0x40",
        (MouseButton::Left, ButtonAction::Up) => "0x80",
        (MouseButton::Right, ButtonAction::Click) => "0xC1",
        (MouseButton::Right, ButtonAction::Down) => "0x41",
        (MouseButton::Right, ButtonAction::Up) => "0x81",
        (MouseButton::Middle, ButtonAction::Click) => "0xC2",
        (MouseButton::Middle, ButtonAction::Down) => "0x42",
        (MouseButton::Middle, ButtonAction::Up) => "0x82",
    };
    if has_wayland_display() && run_quiet("ydotool", &["click", ydo_code]) {
        return;
    }
    let btn_num = x11_button_num(button);
    if with_xtest(|api, dpy| unsafe {
        match action {
            ButtonAction::Click => {
                let _ = (api.fake_button)(dpy, btn_num, 1, 0);
                let _ = (api.fake_button)(dpy, btn_num, 0, 0);
            }
            ButtonAction::Down => {
                let _ = (api.fake_button)(dpy, btn_num, 1, 0);
            }
            ButtonAction::Up => {
                let _ = (api.fake_button)(dpy, btn_num, 0, 0);
            }
        }
    }) {
        return;
    }
    let btn = xdotool_button_id(button);
    let subcmd = match action {
        ButtonAction::Click => "click",
        ButtonAction::Down => "mousedown",
        ButtonAction::Up => "mouseup",
    };
    if !run_quiet("xdotool", &[subcmd, btn]) {
        let _ = run_quiet("ydotool", &["click", ydo_code]);
    }
}

/// Releases any mouse buttons still held down by remote input.
pub fn release_held_buttons() {
    let (left, right, middle) = {
        let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
        s.take_held_and_reset()
    };
    if !left && !right && !middle {
        return;
    }
    if with_uinput(|u| {
        let mut ok = true;
        if left {
            ok &= u.button(BTN_LEFT, false);
        }
        if middle {
            ok &= u.button(BTN_MIDDLE, false);
        }
        if right {
            ok &= u.button(BTN_RIGHT, false);
        }
        ok
    }) {
        return;
    }
    if with_wayland_portal(|p| {
        let mut ok = true;
        if left {
            ok &= p.button(i32::from(BTN_LEFT), false);
        }
        if middle {
            ok &= p.button(i32::from(BTN_MIDDLE), false);
        }
        if right {
            ok &= p.button(i32::from(BTN_RIGHT), false);
        }
        ok
    }) {
        return;
    }
    if has_wayland_display() {
        let mut handled = true;
        if left {
            handled &= run_quiet("ydotool", &["click", "0x80"]);
        }
        if middle {
            handled &= run_quiet("ydotool", &["click", "0x82"]);
        }
        if right {
            handled &= run_quiet("ydotool", &["click", "0x81"]);
        }
        if handled {
            return;
        }
    }
    if !has_wayland_display()
        && with_xtest(|api, dpy| unsafe {
            if left {
                let _ = (api.fake_button)(dpy, 1, 0, 0);
            }
            if middle {
                let _ = (api.fake_button)(dpy, 2, 0, 0);
            }
            if right {
                let _ = (api.fake_button)(dpy, 3, 0, 0);
            }
        })
    {
        return;
    }
    if left && !run_quiet("xdotool", &["mouseup", "1"]) {
        let _ = run_quiet("ydotool", &["click", "0x80"]);
    }
    if middle && !run_quiet("xdotool", &["mouseup", "2"]) {
        let _ = run_quiet("ydotool", &["click", "0x82"]);
    }
    if right && !run_quiet("xdotool", &["mouseup", "3"]) {
        let _ = run_quiet("ydotool", &["click", "0x81"]);
    }
}

/// Scrolls the mouse wheel smoothly (`dy > 0` scrolls down, `dx > 0` scrolls right).
pub fn scroll(dx: f32, dy: f32) {
    if !dx.is_finite() || !dy.is_finite() {
        return;
    }
    let (nx, ny) = {
        let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
        s.accumulate_scroll(dx, dy)
    };
    if nx == 0 && ny == 0 {
        return;
    }
    if with_uinput(|u| u.scroll(nx, ny)) {
        return;
    }
    if with_wayland_portal(|p| p.scroll(nx, ny)) {
        return;
    }
    if has_wayland_display() {
        let mut ydo_ok = true;
        if ny != 0 {
            let wheel_y = (-ny).to_string();
            ydo_ok &= run_quiet("ydotool", &["mousemove", "--wheel", "-x", "0", "-y", &wheel_y]);
        }
        if nx != 0 {
            let wheel_x = nx.to_string();
            ydo_ok &= run_quiet("ydotool", &["mousemove", "--wheel", "-x", &wheel_x, "-y", "0"]);
        }
        if ydo_ok {
            return;
        }
    }
    if with_xtest(|api, dpy| unsafe {
        if ny != 0 {
            let btn: c_uint = if ny > 0 { 5 } else { 4 };
            for _ in 0..ny.unsigned_abs() {
                let _ = (api.fake_button)(dpy, btn, 1, 0);
                let _ = (api.fake_button)(dpy, btn, 0, 0);
            }
        }
        if nx != 0 {
            let btn: c_uint = if nx > 0 { 7 } else { 6 };
            for _ in 0..nx.unsigned_abs() {
                let _ = (api.fake_button)(dpy, btn, 1, 0);
                let _ = (api.fake_button)(dpy, btn, 0, 0);
            }
        }
    }) {
        return;
    }
    if ny != 0 {
        // X11 button 4 = wheel up, button 5 = wheel down.
        let btn = if ny > 0 { "5" } else { "4" };
        let count = ny.unsigned_abs().to_string();
        if !run_quiet("xdotool", &["click", "--repeat", &count, "--delay", "1", btn]) {
            let wheel_y = (-ny).to_string();
            let _ = run_quiet("ydotool", &["mousemove", "--wheel", "-x", "0", "-y", &wheel_y]);
        }
    }
    if nx != 0 {
        // X11 button 6 = wheel left, button 7 = wheel right.
        let btn = if nx > 0 { "7" } else { "6" };
        let count = nx.unsigned_abs().to_string();
        if !run_quiet("xdotool", &["click", "--repeat", &count, "--delay", "1", btn]) {
            let wheel_x = nx.to_string();
            let _ = run_quiet("ydotool", &["mousemove", "--wheel", "-x", &wheel_x, "-y", "0"]);
        }
    }
}

/// Types arbitrary Unicode text (filtering carriage returns `\r`) without
/// exposing the text on any subprocess command line (`ps` / `/proc/<pid>/cmdline`).
pub fn type_text(text: &str) {
    let cleaned: String = text.chars().filter(|&c| c != '\r').collect();
    if cleaned.is_empty() {
        return;
    }

    // 1. `/dev/uinput`, pressing the keys that type each character on the
    //    user's layout. Without a known layout, skip it: a guessed layout
    //    types the wrong characters.
    let presses: Option<Vec<super::keymap::KeyPress>> =
        super::keymap::current().and_then(|km| cleaned.chars().map(|ch| km.press_for(ch)).collect());
    if let Some(ref presses) = presses
        && with_uinput(|u| presses.iter().all(|p| u.key_combo(p.code, &press_mods(p))))
    {
        return;
    }

    // 2. On Wayland, try `org.freedesktop.portal.RemoteDesktop.NotifyKeyboardKeysym`.
    if with_wayland_portal(|p| {
        cleaned.chars().all(|ch| {
            let ks = char_to_x11_keysym(ch) as i32;
            p.keysym(ks, true) && p.keysym(ks, false)
        })
    }) {
        return;
    }

    // 3. On X11, try in-process XTest key injection when all characters map to keycodes.
    let mut xtest_typed = false;
    let _ = with_xtest(|api, dpy| unsafe {
        let shift_ks = (api.string_to_keysym)(c"Shift_L".as_ptr());
        let shift_kc = c_uint::from((api.keysym_to_keycode)(dpy, shift_ks));
        let mut steps: Vec<(c_uint, bool)> = Vec::with_capacity(cleaned.len());
        for ch in cleaned.chars() {
            let ks = c_ulong::from(char_to_x11_keysym(ch));
            let kc = (api.keysym_to_keycode)(dpy, ks);
            if kc == 0 {
                return;
            }
            let needs_shift = if let Some(kc_to_ks) = api.keycode_to_keysym {
                let unshifted = kc_to_ks(dpy, kc, 0);
                let shifted = kc_to_ks(dpy, kc, 1);
                unshifted != ks && shifted == ks
            } else {
                ch.is_ascii_uppercase()
                    || matches!(
                        ch,
                        '!' | '@'
                            | '#'
                            | '$'
                            | '%'
                            | '^'
                            | '&'
                            | '*'
                            | '('
                            | ')'
                            | '_'
                            | '+'
                            | '{'
                            | '}'
                            | '|'
                            | ':'
                            | '"'
                            | '<'
                            | '>'
                            | '?'
                            | '~'
                    )
            };
            steps.push((c_uint::from(kc), needs_shift));
        }
        for (kc, shift) in steps {
            if shift && shift_kc != 0 {
                let _ = (api.fake_key)(dpy, shift_kc, 1, 0);
            }
            let _ = (api.fake_key)(dpy, kc, 1, 0);
            let _ = (api.fake_key)(dpy, kc, 0, 0);
            if shift && shift_kc != 0 {
                let _ = (api.fake_key)(dpy, shift_kc, 0, 0);
            }
        }
        xtest_typed = true;
    });
    if xtest_typed {
        return;
    }

    // 4. Pipe text over stdin (`wtype -` or `xdotool type --file -`) so it is never on argv.
    if has_wayland_display() && run_with_stdin("wtype", &["-"], cleaned.as_bytes()) {
        return;
    }
    if !run_with_stdin(
        "xdotool",
        &["type", "--clearmodifiers", "--delay", "0", "--file", "-"],
        cleaned.as_bytes(),
    ) {
        let _ = run_with_stdin("wtype", &["-"], cleaned.as_bytes());
    }
}

/// Resolves a protocol key or shortcut name into `(keysym, extra_mods)`.
pub(crate) fn resolve_key(key: &str) -> Option<(String, &'static [KeyMod])> {
    const CTRL: &[KeyMod] = &[KeyMod::Ctrl];
    const WIN: &[KeyMod] = &[KeyMod::Win];
    const NONE: &[KeyMod] = &[];

    let trimmed = key.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    let (sym, mods): (&str, &'static [KeyMod]) = match lower.as_str() {
        "copy" => ("c", CTRL),
        "paste" => ("v", CTRL),
        "cut" => ("x", CTRL),
        "undo" => ("z", CTRL),
        "redo" => ("y", CTRL),
        "select_all" => ("a", CTRL),
        "task_view" => ("Tab", WIN),
        "enter" | "return" => ("Return", NONE),
        "backspace" => ("BackSpace", NONE),
        "tab" => ("Tab", NONE),
        "escape" | "esc" => ("Escape", NONE),
        "space" => ("space", NONE),
        "delete" | "del" => ("Delete", NONE),
        "insert" | "ins" => ("Insert", NONE),
        "up" => ("Up", NONE),
        "down" => ("Down", NONE),
        "left" => ("Left", NONE),
        "right" => ("Right", NONE),
        "home" => ("Home", NONE),
        "end" => ("End", NONE),
        "page_up" | "pageup" | "pgup" | "prior" => ("Prior", NONE),
        "page_down" | "pagedown" | "pgdn" | "next" => ("Next", NONE),
        "print" | "printscreen" | "sysrq" => ("Print", NONE),
        "f1" => ("F1", NONE),
        "f2" => ("F2", NONE),
        "f3" => ("F3", NONE),
        "f4" => ("F4", NONE),
        "f5" => ("F5", NONE),
        "f6" => ("F6", NONE),
        "f7" => ("F7", NONE),
        "f8" => ("F8", NONE),
        "f9" => ("F9", NONE),
        "f10" => ("F10", NONE),
        "f11" => ("F11", NONE),
        "f12" => ("F12", NONE),
        "volume_up" | "xf86audioraisevolume" => ("XF86AudioRaiseVolume", NONE),
        "volume_down" | "xf86audiolowervolume" => ("XF86AudioLowerVolume", NONE),
        "volume_mute" | "xf86audiomute" => ("XF86AudioMute", NONE),
        "play_pause" | "xf86audioplay" => ("XF86AudioPlay", NONE),
        "next_track" | "xf86audionext" => ("XF86AudioNext", NONE),
        "prev_track" | "xf86audioprev" => ("XF86AudioPrev", NONE),
        s if s.len() == 1 => {
            let b = s.as_bytes()[0];
            if b.is_ascii_lowercase() || b.is_ascii_digit() {
                return Some((s.to_owned(), NONE));
            }
            return None;
        }
        _ => return None,
    };
    Some((sym.to_owned(), mods))
}

fn mod_name(m: KeyMod) -> &'static str {
    match m {
        KeyMod::Ctrl => "ctrl",
        KeyMod::Alt => "alt",
        KeyMod::Shift => "shift",
        KeyMod::Win => "super",
    }
}

/// Formats an `xdotool key` combo string (e.g. `"ctrl+shift+k"` or `"super+Tab"`)
/// for a protocol key and modifier list.
pub(crate) fn format_xdotool_combo(key: &str, mods: &[KeyMod]) -> Option<String> {
    let (sym, preset_mods) = resolve_key(key)?;
    let mut all_mods: Vec<KeyMod> = Vec::with_capacity(4);
    for &m in preset_mods.iter().chain(mods.iter()) {
        if !all_mods.contains(&m) {
            all_mods.push(m);
        }
    }
    if all_mods.is_empty() {
        return Some(sym);
    }
    let mut parts: Vec<&str> = all_mods.iter().map(|&m| mod_name(m)).collect();
    parts.push(&sym);
    Some(parts.join("+"))
}

fn mod_x11_keysym_name(m: KeyMod) -> &'static CStr {
    match m {
        KeyMod::Ctrl => c"Control_L",
        KeyMod::Alt => c"Alt_L",
        KeyMod::Shift => c"Shift_L",
        KeyMod::Win => c"Super_L",
    }
}

/// Presses a named key or shortcut with optional modifiers.
pub fn press_key(key: &str, mods: &[KeyMod]) {
    if key.eq_ignore_ascii_case("lock") {
        let _ = crate::links::power(PowerAction::Lock);
        return;
    }
    let Some((sym, preset_mods)) = resolve_key(key) else {
        return;
    };
    let mut all_mods: Vec<KeyMod> = Vec::with_capacity(4);
    for &m in preset_mods.iter().chain(mods.iter()) {
        if !all_mods.contains(&m) {
            all_mods.push(m);
        }
    }

    if let Some(evdev_code) = key_code_for_sym(&sym) {
        let mod_codes: Vec<u16> = all_mods.iter().map(|&m| evdev_key_for_mod(m)).collect();
        if with_uinput(|u| u.key_combo(evdev_code, &mod_codes)) {
            return;
        }
        if with_wayland_portal(|p| {
            let mut ok = true;
            for &mc in &mod_codes {
                ok &= p.keycode(i32::from(mc), true);
            }
            ok &= p.keycode(i32::from(evdev_code), true);
            ok &= p.keycode(i32::from(evdev_code), false);
            for &mc in mod_codes.iter().rev() {
                ok &= p.keycode(i32::from(mc), false);
            }
            ok
        }) {
            return;
        }
    }

    if has_wayland_display() {
        let mut wtype_args: Vec<String> = Vec::new();
        for &m in &all_mods {
            let wm = match m {
                KeyMod::Ctrl => "ctrl",
                KeyMod::Alt => "alt",
                KeyMod::Shift => "shift",
                KeyMod::Win => "logo",
            };
            wtype_args.push("-M".into());
            wtype_args.push(wm.into());
        }
        wtype_args.push("-k".into());
        wtype_args.push(sym.clone());
        let refs: Vec<&str> = wtype_args.iter().map(String::as_str).collect();
        if run_quiet("wtype", &refs) {
            return;
        }
    }
    if let Ok(c_sym) = CString::new(sym.as_str()) {
        let mut injected = false;
        let _ = with_xtest(|api, dpy| unsafe {
            let ks = (api.string_to_keysym)(c_sym.as_ptr());
            if ks == 0 {
                return;
            }
            let kc = (api.keysym_to_keycode)(dpy, ks);
            if kc == 0 {
                return;
            }
            let mut mod_kcs: Vec<c_uint> = Vec::with_capacity(all_mods.len());
            for &m in &all_mods {
                let mks = (api.string_to_keysym)(mod_x11_keysym_name(m).as_ptr());
                let mkc = (api.keysym_to_keycode)(dpy, mks);
                if mkc != 0 {
                    mod_kcs.push(c_uint::from(mkc));
                }
            }
            for &mkc in &mod_kcs {
                let _ = (api.fake_key)(dpy, mkc, 1, 0);
            }
            let _ = (api.fake_key)(dpy, c_uint::from(kc), 1, 0);
            let _ = (api.fake_key)(dpy, c_uint::from(kc), 0, 0);
            for &mkc in mod_kcs.iter().rev() {
                let _ = (api.fake_key)(dpy, mkc, 0, 0);
            }
            injected = true;
        });
        if injected {
            return;
        }
    }
    if let Some(combo) = format_xdotool_combo(key, mods) {
        let _ = run_quiet("xdotool", &["key", "--clearmodifiers", &combo]);
    }
}

/// Executes a presentation slide action.
pub fn slide(action: SlideAction) {
    let key = match action {
        SlideAction::Next => "page_down",
        SlideAction::Previous => "page_up",
        SlideAction::Start => "f5",
        SlideAction::Stop => "escape",
        SlideAction::Black => "b",
    };
    press_key(key, &[]);
}

/// Parses idle milliseconds from `xprintidle` output (`"8801\n"`) or D-Bus
/// `GetIdletime` / `GetSessionIdleTime` output (`"(uint64 8801,)"` or `"(uint32 8,)"`).
pub(crate) fn parse_idle_output_ms(raw: &str, is_seconds: bool) -> Option<u64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(val) = trimmed.parse::<u64>() {
        return Some(if is_seconds { val.saturating_mul(1000) } else { val });
    }
    for token in trimmed.split(|c: char| !c.is_ascii_digit()) {
        if !token.is_empty()
            && token != "64"
            && token != "32"
            && let Ok(val) = token.parse::<u64>()
        {
            return Some(if is_seconds { val.saturating_mul(1000) } else { val });
        }
    }
    None
}

fn query_idle_ms() -> Option<u64> {
    if !has_display_env() {
        return None;
    }

    // 1. On X11, prefer `xprintidle` first (always returns milliseconds). Under
    // Wayland it would ask XWayland, which sees no input from native windows.
    if has_x11_display()
        && !has_wayland_display()
        && let Ok(out) = Command::new("xprintidle").stdin(Stdio::null()).stderr(Stdio::null()).output()
        && out.status.success()
        && let Some(ms) = parse_idle_output_ms(&String::from_utf8_lossy(&out.stdout), false)
    {
        return Some(ms);
    }

    // 2. Query D-Bus idle monitors in-process via `zbus::blocking::Connection::session()`.
    if let Some(ms) = super::non_tokio(|| {
        let conn = zbus::blocking::Connection::session().ok()?;
        // GNOME Mutter IdleMonitor returns u64 milliseconds.
        if super::dbus_name_has_owner(&conn, "org.gnome.Mutter.IdleMonitor")
            && let Ok(reply) = conn.call_method(
                Some("org.gnome.Mutter.IdleMonitor"),
                "/org/gnome/Mutter/IdleMonitor/Core",
                Some("org.gnome.Mutter.IdleMonitor"),
                "GetIdletime",
                &(),
            )
        {
            let body = reply.body();
            if let Ok(ms) = body.deserialize::<u64>().or_else(|_| body.deserialize::<(u64,)>().map(|(v,)| v))
            {
                return Some(ms);
            }
        }

        // Freedesktop ScreenSaver (`GetSessionIdleTime` returns u32).
        if super::dbus_name_has_owner(&conn, "org.freedesktop.ScreenSaver") {
            for path in ["/org/freedesktop/ScreenSaver", "/ScreenSaver"] {
                if let Ok(reply) = conn.call_method(
                    Some("org.freedesktop.ScreenSaver"),
                    path,
                    Some("org.freedesktop.ScreenSaver"),
                    "GetSessionIdleTime",
                    &(),
                ) {
                    let body = reply.body();
                    if let Ok(val) =
                        body.deserialize::<u32>().or_else(|_| body.deserialize::<(u32,)>().map(|(v,)| v))
                    {
                        return Some(u64::from(val));
                    }
                }
            }
        }
        None
    }) {
        return Some(ms);
    }

    // 3. Fallback to `xprintidle` on Wayland with XWayland if D-Bus idle monitors were unavailable.
    if !has_x11_display()
        && let Ok(out) = Command::new("xprintidle").stdin(Stdio::null()).stderr(Stdio::null()).output()
        && out.status.success()
        && let Some(ms) = parse_idle_output_ms(&String::from_utf8_lossy(&out.stdout), false)
    {
        return Some(ms);
    }

    None
}

/// Returns `true` when the user has moved the mouse or pressed a key on this PC
/// within the last `max_idle_ms` milliseconds.
pub fn is_user_active_within_ms(max_idle_ms: u32) -> bool {
    match query_idle_ms() {
        Some(idle_ms) => idle_ms <= u64::from(max_idle_ms),
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_named_keys_and_shortcuts() {
        assert_eq!(resolve_key("enter"), Some(("Return".to_owned(), &[][..])));
        assert_eq!(resolve_key("up"), Some(("Up".to_owned(), &[][..])));
        assert_eq!(resolve_key("copy"), Some(("c".to_owned(), &[KeyMod::Ctrl][..])));
        assert_eq!(resolve_key("paste"), Some(("v".to_owned(), &[KeyMod::Ctrl][..])));
        assert_eq!(resolve_key("task_view"), Some(("Tab".to_owned(), &[KeyMod::Win][..])));
        assert_eq!(resolve_key("a"), Some(("a".to_owned(), &[][..])));
        assert_eq!(resolve_key("A"), Some(("a".to_owned(), &[][..])));
        assert_eq!(resolve_key("5"), Some(("5".to_owned(), &[][..])));
        assert_eq!(resolve_key("f5"), Some(("F5".to_owned(), &[][..])));
        assert_eq!(resolve_key("play_pause"), Some(("XF86AudioPlay".to_owned(), &[][..])));
        assert_eq!(resolve_key("volume_up"), Some(("XF86AudioRaiseVolume".to_owned(), &[][..])));
        assert_eq!(resolve_key("print"), Some(("Print".to_owned(), &[][..])));
        assert!(resolve_key("unknown_key").is_none());
    }

    #[test]
    fn maps_evdev_keys_and_unicode_keysyms() {
        assert_eq!(evdev_key_for_sym("Return"), Some(28));
        assert_eq!(evdev_key_for_sym("Escape"), Some(1));
        assert_eq!(evdev_key_for_sym("F5"), Some(63));
        assert_eq!(evdev_key_for_sym("XF86AudioPlay"), Some(164));
        assert_eq!(evdev_key_for_mod(KeyMod::Win), 125);

        assert_eq!(char_to_x11_keysym('a'), 0x61);
        assert_eq!(char_to_x11_keysym('\n'), 0xff0d);
        assert_eq!(char_to_x11_keysym('é'), 0x00e9);
        assert_eq!(char_to_x11_keysym('€'), 0x0100_20ac);
    }

    #[test]
    fn formats_xdotool_combos_with_preset_and_explicit_modifiers() {
        assert_eq!(format_xdotool_combo("copy", &[]), Some("ctrl+c".to_owned()));
        assert_eq!(
            format_xdotool_combo("k", &[KeyMod::Ctrl, KeyMod::Shift]),
            Some("ctrl+shift+k".to_owned())
        );
        assert_eq!(format_xdotool_combo("tab", &[KeyMod::Alt]), Some("alt+Tab".to_owned()));
        assert_eq!(format_xdotool_combo("d", &[KeyMod::Win]), Some("super+d".to_owned()));
        assert_eq!(format_xdotool_combo("f5", &[]), Some("F5".to_owned()));
    }

    #[test]
    fn accumulates_subpixel_move_and_scroll_remainders() {
        let mut s = InputState::default();
        assert_eq!(s.accumulate_move(0.4, -0.4), (0, 0));
        assert_eq!(s.accumulate_move(0.4, -0.4), (1, -1));
        assert_eq!(s.accumulate_move(0.4, -0.4), (0, 0));

        assert_eq!(s.accumulate_scroll(0.3, 0.3), (0, 0));
        assert_eq!(s.accumulate_scroll(0.3, 0.3), (1, 1));
    }

    #[test]
    fn tracks_held_mouse_buttons_and_resets_on_release() {
        let mut s = InputState::default();
        s.record_button(MouseButton::Left, ButtonAction::Down);
        s.record_button(MouseButton::Right, ButtonAction::Click);
        assert!(s.held_left);
        assert!(!s.held_right);

        let held = s.take_held_and_reset();
        assert_eq!(held, (true, false, false));
        assert!(!s.held_left);
    }

    #[test]
    fn maps_normalized_coordinates_and_parses_idle_output() {
        assert_eq!(normalized_to_pixels(0.0, 0.0, 1920, 1080), (0, 0));
        assert_eq!(normalized_to_pixels(1.0, 1.0, 1920, 1080), (1919, 1079));
        assert_eq!(normalized_to_pixels(0.5, 0.5, 1921, 1081), (960, 540));

        assert_eq!(parse_idle_output_ms("8801\n", false), Some(8801));
        assert_eq!(parse_idle_output_ms("(uint64 4250,)", false), Some(4250));
        assert_eq!(parse_idle_output_ms("(uint32 19,)", true), Some(19_000));
        assert_eq!(parse_idle_output_ms("12", true), Some(12_000));
    }
}
