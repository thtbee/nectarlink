// SPDX-License-Identifier: GPL-3.0-or-later
//! The notification-area (tray) icon and its menu, plus the system
//! broadcasts a hidden top-level window receives: resume from sleep and
//! theme/animation setting changes.
//!
//! Everything runs on the Qt GUI thread: the hidden window is created there,
//! and Qt's event loop dispatches its messages.

use std::{
    cell::{Cell, RefCell},
    sync::{
        Mutex,
        atomic::{AtomicIsize, Ordering},
    },
};

use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
            DeleteObject, HBITMAP,
        },
        System::{
            LibraryLoader::GetModuleHandleW,
            Threading::{AttachThreadInput, GetCurrentProcessId, GetCurrentThreadId},
        },
        UI::{
            Input::KeyboardAndMouse::{
                HOT_KEY_MODIFIERS, INPUT, INPUT_0, INPUT_MOUSE, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
                MOD_SHIFT, MOD_WIN, MOUSEEVENTF_MOVE, MOUSEINPUT, RegisterHotKey, SendInput,
                UnregisterHotKey,
            },
            Shell::{
                NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
                NIN_SELECT, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
            },
            WindowsAndMessaging::{
                AppendMenuW, BringWindowToTop, CreateIconIndirect, CreatePopupMenu, CreateWindowExW,
                DefWindowProcW, DestroyIcon, DestroyMenu, DestroyWindow, GW_HWNDNEXT, GWL_EXSTYLE,
                GetClassNameW, GetForegroundWindow, GetSystemMetrics, GetTopWindow, GetWindow,
                GetWindowLongW, GetWindowTextW, GetWindowThreadProcessId, HICON, HMENU, ICONINFO, IsIconic,
                IsWindow, IsWindowVisible, KillTimer, MF_GRAYED, MF_SEPARATOR, MF_STRING, PostMessageW,
                RegisterClassExW, RegisterWindowMessageW, SM_CXSMICON, SW_RESTORE, SendMessageW,
                SetForegroundWindow, SetTimer, ShowWindow, TPM_BOTTOMALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON,
                TrackPopupMenuEx, WINDOW_EX_STYLE, WM_APP, WM_CONTEXTMENU, WM_HOTKEY, WM_NULL,
                WM_POWERBROADCAST, WM_SETTINGCHANGE, WM_TIMER, WNDCLASSEXW, WS_EX_NOACTIVATE,
                WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
            },
        },
    },
    core::{PCWSTR, w},
};

/// What the user or the system asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    Open,
    CommandPalette,
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
    pub take_photo: String,
    pub scan_document: String,
    pub find_phone: String,
    pub open_link: String,
    pub quit: String,
}

const CALLBACK_MESSAGE: u32 = WM_APP + 1;
const WM_UPDATE_HOTKEYS: u32 = WM_APP + 2;
const WM_PASTE_MSG: u32 = 0x0302;
const TIMER_TRACK_FG: usize = 1;
const HOTKEY_PHOTO_ID: i32 = 101;
const HOTKEY_SCAN_ID: i32 = 102;
const HOTKEY_PALETTE_ID: i32 = 103;
const ICON_ID: u32 = 1;
const CMD_OPEN: u32 = 1;
const CMD_FIND_PHONE: u32 = 2;
const CMD_QUIT: u32 = 3;
const CMD_OPEN_LINK: u32 = 4;
const CMD_TAKE_PHOTO: u32 = 5;
const CMD_SCAN_DOCUMENT: u32 = 6;
const CMD_COMMAND_PALETTE: u32 = 7;
const PBT_APMRESUMEAUTOMATIC: usize = 0x12;
/// NIN_SELECT | NINF_KEY (shellapi.h): the icon was activated with the keyboard.
const NIN_KEYSELECT: u32 = NIN_SELECT | 0x1;
const SPI_SETCLIENTAREAANIMATION: usize = 0x1043;
const SPI_SETDESKWALLPAPER: usize = 0x014;

type Handler = Box<dyn Fn(TrayEvent)>;

static TRAY_HWND: AtomicIsize = AtomicIsize::new(0);
static LAST_EXTERNAL_FOREGROUND: Mutex<Option<(isize, String)>> = Mutex::new(None);
static HOTKEY_SPECS: Mutex<(String, String, String)> =
    Mutex::new((String::new(), String::new(), String::new()));

thread_local! {
    static HANDLER: RefCell<Option<Handler>> = RefCell::new(None);
    static LABELS: RefCell<Option<MenuLabels>> = const { RefCell::new(None) };
    static CAN_FIND_PHONE: Cell<bool> = const { Cell::new(false) };
    static TOOLTIP: RefCell<String> = const { RefCell::new(String::new()) };
    static ICON: Cell<HICON> = Cell::new(HICON::default());
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
}

fn window_title(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    // SAFETY: `buf` is a live stack UTF-16 buffer.
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
    if len <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..len as usize]).trim().to_owned()
}

fn window_class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 128];
    // SAFETY: `buf` is a live stack UTF-16 buffer.
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    if len <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..len as usize])
}

unsafe fn inspect_external_window(hwnd: HWND, self_pid: u32) -> Option<(isize, String, bool)> {
    if hwnd.0.is_null() {
        return None;
    }
    // SAFETY: Standard Win32 queries on a candidate top-level HWND.
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() || !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return None;
        }
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 || pid == self_pid {
            return None;
        }
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if (ex_style & WS_EX_TOOLWINDOW.0) != 0 || (ex_style & WS_EX_NOACTIVATE.0) != 0 {
            return None;
        }
        let class_name = window_class_name(hwnd);
        if matches!(
            class_name.as_str(),
            "Shell_TrayWnd"
                | "NotifyIconOverflowWindow"
                | "TopLevelWindowForOverflowXamlIsland"
                | "Progman"
                | "WorkerW"
                | "MultitaskingViewFrame"
                | "Windows.UI.Core.CoreWindow"
        ) {
            return None;
        }
        let title = window_title(hwnd);
        if title.is_empty() || title == "Default IME" || title == "MSCTFIME UI" {
            return None;
        }
        let is_topmost = (ex_style & WS_EX_TOPMOST.0) != 0;
        Some((hwnd.0 as isize, title, is_topmost))
    }
}

unsafe fn find_top_external_window(self_pid: u32) -> Option<(isize, String, bool)> {
    // SAFETY: Walking top-level windows in Z-order via GetTopWindow / GetWindow.
    unsafe {
        let mut cur = GetTopWindow(None).ok()?;
        for _ in 0..256 {
            if let Some(found) = inspect_external_window(cur, self_pid) {
                return Some(found);
            }
            match GetWindow(cur, GW_HWNDNEXT) {
                Ok(next) if !next.0.is_null() => cur = next,
                _ => break,
            }
        }
        None
    }
}

/// Records the current foreground window if it belongs to an external
/// user-facing application (not Nectarlink itself and not the Windows shell tray).
pub fn record_foreground_window() {
    // SAFETY: Standard Win32 queries on the foreground and Z-order HWNDs.
    unsafe {
        let self_pid = GetCurrentProcessId();
        let fg = GetForegroundWindow();
        if let Some((hwnd, title, _)) = inspect_external_window(fg, self_pid) {
            let mut guard = LAST_EXTERNAL_FOREGROUND.lock().unwrap_or_else(|e| e.into_inner());
            *guard = Some((hwnd, title));
            return;
        }
        if let Some((hwnd, title, is_topmost)) = find_top_external_window(self_pid) {
            let mut guard = LAST_EXTERNAL_FOREGROUND.lock().unwrap_or_else(|e| e.into_inner());
            let current_alive = guard.as_ref().is_some_and(|(h, _)| is_window_alive_and_visible(*h));
            if is_topmost || !current_alive {
                *guard = Some((hwnd, title));
            }
        }
    }
}

/// Captures the last active external foreground window `(hwnd, title)` at the
/// moment a Continuity Camera request begins, if that window is still alive
/// and visible right now.
pub fn snapshot_target_window() -> Option<(isize, String)> {
    record_foreground_window();
    let recorded = LAST_EXTERNAL_FOREGROUND.lock().unwrap_or_else(|e| e.into_inner()).clone()?;
    let (hwnd, saved_title) = recorded;
    if !is_window_alive_and_visible(hwnd) {
        return None;
    }
    let live_title = window_title(HWND(hwnd as *mut _));
    let title = if live_title.is_empty() { saved_title } else { live_title };
    Some((hwnd, title))
}

/// Returns true if `hwnd` still refers to an existing, visible top-level window.
pub fn is_window_alive_and_visible(hwnd: isize) -> bool {
    if hwnd == 0 {
        return false;
    }
    let h = HWND(hwnd as *mut _);
    // SAFETY: `IsWindow` and `IsWindowVisible` safely inspect any handle value.
    unsafe { IsWindow(Some(h)).as_bool() && IsWindowVisible(h).as_bool() }
}

/// Restores (if minimized) and brings `hwnd` to the foreground so a paste
/// shortcut can be synthesized into it. Returns false if the window is gone.
pub fn focus_window(hwnd: isize) -> bool {
    if !is_window_alive_and_visible(hwnd) {
        return false;
    }
    let h = HWND(hwnd as *mut _);
    // SAFETY: `h` was just verified to be a live visible window.
    unsafe {
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        let fg = GetForegroundWindow();
        if fg == h {
            return true;
        }
        // Synthesize a zero-delta mouse input so Windows unlocks SetForegroundWindow
        // for this thread, and attach thread input to the current foreground thread.
        let noop = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: 0,
                    dwFlags: MOUSEEVENTF_MOVE,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let _ = SendInput(&[noop], std::mem::size_of::<INPUT>() as i32);

        let cur_tid = GetCurrentThreadId();
        let fg_tid = if !fg.0.is_null() { GetWindowThreadProcessId(fg, None) } else { 0 };
        let attached = if fg_tid != 0 && fg_tid != cur_tid {
            AttachThreadInput(cur_tid, fg_tid, true).as_bool()
        } else {
            false
        };
        let _ = BringWindowToTop(h);
        let _ = SetForegroundWindow(h);
        if attached {
            let _ = AttachThreadInput(cur_tid, fg_tid, false);
        }
        is_window_alive_and_visible(hwnd)
    }
}

/// If the OS foreground lock prevented `SetForegroundWindow` from making `hwnd`
/// the foreground window, sends `WM_PASTE` directly to `hwnd` so it still receives
/// the paste action.
pub fn send_paste_fallback_if_not_foreground(hwnd: isize) {
    if !is_window_alive_and_visible(hwnd) {
        return;
    }
    let h = HWND(hwnd as *mut _);
    // SAFETY: `h` is a valid live window handle.
    unsafe {
        if GetForegroundWindow() != h {
            let _ = SendMessageW(h, WM_PASTE_MSG, Some(WPARAM(0)), Some(LPARAM(0)));
        }
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
            "ctrl" | "control" => mods |= MOD_CONTROL.0,
            "alt" => mods |= MOD_ALT.0,
            "shift" => mods |= MOD_SHIFT.0,
            "win" | "super" | "meta" => mods |= MOD_WIN.0,
            "space" | "spacebar" => {
                if key_vk.is_some() {
                    return None;
                }
                key_vk = Some(0x20);
            }
            _ => {
                if key_vk.is_some() {
                    return None;
                }
                let vk = if part.len() == 1 {
                    let b = part.as_bytes()[0];
                    if b.is_ascii_alphabetic() {
                        u32::from(b.to_ascii_uppercase())
                    } else if b.is_ascii_digit() {
                        u32::from(b)
                    } else {
                        return None;
                    }
                } else {
                    let num_str = lower.strip_prefix('f')?;
                    let n: u32 = num_str.parse().ok()?;
                    if (1..=12).contains(&n) {
                        0x70 + (n - 1)
                    } else {
                        return None;
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
    Some((mods | MOD_NOREPEAT.0, vk))
}

/// Updates the Continuity Camera and Command Palette global hotkeys (`photo_spec`, `scan_spec`, and `palette_spec`).
pub fn update_hotkeys(photo_spec: &str, scan_spec: &str, palette_spec: &str) {
    {
        let mut specs = HOTKEY_SPECS.lock().unwrap_or_else(|e| e.into_inner());
        *specs = (photo_spec.trim().to_owned(), scan_spec.trim().to_owned(), palette_spec.trim().to_owned());
    }
    let raw = TRAY_HWND.load(Ordering::Acquire);
    if raw != 0 {
        // SAFETY: Posting WM_UPDATE_HOTKEYS to the tray window wakes the tray thread to apply hotkeys.
        unsafe {
            let _ = PostMessageW(Some(HWND(raw as *mut _)), WM_UPDATE_HOTKEYS, WPARAM(0), LPARAM(0));
        }
    }
}

unsafe fn apply_hotkeys_on_window(hwnd: HWND) {
    let (photo_spec, scan_spec, palette_spec) =
        HOTKEY_SPECS.lock().unwrap_or_else(|e| e.into_inner()).clone();
    // SAFETY: Unregistering and registering hotkeys on the thread that owns `hwnd`.
    unsafe {
        let _ = UnregisterHotKey(Some(hwnd), HOTKEY_PHOTO_ID);
        let _ = UnregisterHotKey(Some(hwnd), HOTKEY_SCAN_ID);
        let _ = UnregisterHotKey(Some(hwnd), HOTKEY_PALETTE_ID);
        if let Some((mods, vk)) = parse_hotkey(&photo_spec)
            && let Err(e) = RegisterHotKey(Some(hwnd), HOTKEY_PHOTO_ID, HOT_KEY_MODIFIERS(mods), vk)
        {
            tracing::debug!(hotkey = %photo_spec, error = %e, "could not register Take Photo hotkey");
        }
        if let Some((mods, vk)) = parse_hotkey(&scan_spec)
            && let Err(e) = RegisterHotKey(Some(hwnd), HOTKEY_SCAN_ID, HOT_KEY_MODIFIERS(mods), vk)
        {
            tracing::debug!(hotkey = %scan_spec, error = %e, "could not register Scan Document hotkey");
        }
        if let Some((mods, vk)) = parse_hotkey(&palette_spec)
            && let Err(e) = RegisterHotKey(Some(hwnd), HOTKEY_PALETTE_ID, HOT_KEY_MODIFIERS(mods), vk)
        {
            tracing::debug!(hotkey = %palette_spec, error = %e, "could not register Command Palette hotkey");
        }
    }
}

/// The tray icon. Removed when dropped. Create and drop on the GUI thread.
#[derive(Debug)]
pub struct Tray {
    hwnd: HWND,
}

impl Tray {
    pub fn create(
        tooltip: &str,
        labels: MenuLabels,
        handler: impl Fn(TrayEvent) + 'static,
    ) -> windows::core::Result<Tray> {
        HANDLER.with(|h| *h.borrow_mut() = Some(Box::new(handler)));
        LABELS.with(|l| *l.borrow_mut() = Some(labels));
        TOOLTIP.with(|t| *t.borrow_mut() = tooltip.to_owned());
        // SAFETY: standard window class registration and creation; the
        // window procedure only touches this thread's thread-locals.
        let hwnd = unsafe {
            let instance = GetModuleHandleW(None)?;
            let class = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(window_proc),
                hInstance: instance.into(),
                lpszClassName: w!("Nectarlink.TrayWindow"),
                ..Default::default()
            };
            // Registering twice (a second Tray) fails harmlessly.
            RegisterClassExW(&class);
            TASKBAR_CREATED.with(|t| t.set(RegisterWindowMessageW(w!("TaskbarCreated"))));
            // A hidden top-level window (not message-only) so it also gets
            // power and setting broadcasts.
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW | WINDOW_EX_STYLE(0),
                w!("Nectarlink.TrayWindow"),
                w!("Nectarlink"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance.into()),
                None,
            )?;
            TRAY_HWND.store(hwnd.0 as isize, Ordering::Release);
            let _ = SetTimer(Some(hwnd), TIMER_TRACK_FG, 250, None);
            record_foreground_window();
            apply_hotkeys_on_window(hwnd);
            hwnd
        };
        ICON.with(|i| i.set(create_icon().unwrap_or_default()));
        add_icon(hwnd);
        Ok(Tray { hwnd })
    }

    pub fn set_tooltip(&self, text: &str) {
        TOOLTIP.with(|t| *t.borrow_mut() = text.to_owned());
        let mut data = icon_data(self.hwnd);
        data.uFlags = NIF_TIP | NIF_SHOWTIP;
        // SAFETY: data is fully initialized for this window's icon.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }

    /// Whether "Find my phone" is offered (a phone is paired).
    pub fn set_find_phone_enabled(&self, enabled: bool) {
        CAN_FIND_PHONE.with(|c| c.set(enabled));
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        TRAY_HWND.store(0, Ordering::Release);
        let data = icon_data(self.hwnd);
        // SAFETY: removing our own icon, hotkeys, timer, and destroying our own window/icon.
        unsafe {
            let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_PHOTO_ID);
            let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_SCAN_ID);
            let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_PALETTE_ID);
            let _ = KillTimer(Some(self.hwnd), TIMER_TRACK_FG);
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            let _ = DestroyWindow(self.hwnd);
            let icon = ICON.with(|i| i.replace(HICON::default()));
            if !icon.is_invalid() {
                let _ = DestroyIcon(icon);
            }
        }
        HANDLER.with(|h| *h.borrow_mut() = None);
    }
}

fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
        uCallbackMessage: CALLBACK_MESSAGE,
        hIcon: ICON.with(Cell::get),
        ..Default::default()
    };
    TOOLTIP.with(|t| {
        let wide: Vec<u16> = t.borrow().encode_utf16().take(data.szTip.len() - 1).collect();
        data.szTip[..wide.len()].copy_from_slice(&wide);
    });
    data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
    data
}

fn add_icon(hwnd: HWND) {
    let data = icon_data(hwnd);
    // SAFETY: data is fully initialized for this window's icon.
    unsafe {
        if !Shell_NotifyIconW(NIM_ADD, &data).as_bool() {
            tracing::warn!("can't add the tray icon");
            return;
        }
        let _ = Shell_NotifyIconW(NIM_SETVERSION, &data);
    }
}

/// Builds an HICON from the mark rendered at the small-icon size.
fn create_icon() -> windows::core::Result<HICON> {
    // SAFETY: GetSystemMetrics has no preconditions.
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.clamp(16, 64) as usize;
    let rgba = crate::mark::render(size);
    // SAFETY: a top-down 32-bit DIB of `size`×`size`; we write exactly
    // size*size*4 bytes into its pixel buffer, then hand both bitmaps to
    // CreateIconIndirect, which copies them, and delete ours.
    unsafe {
        let header = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size as i32,
            biHeight: -(size as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let info = BITMAPINFO { bmiHeader: header, ..Default::default() };
        let mut bits = std::ptr::null_mut();
        let color: HBITMAP = CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0)?;
        let pixels = std::slice::from_raw_parts_mut(bits.cast::<u8>(), size * size * 4);
        for (dst, src) in pixels.as_chunks_mut::<4>().0.iter_mut().zip(rgba.as_chunks::<4>().0) {
            // RGBA → BGRA, straight alpha (what icons expect).
            *dst = [src[2], src[1], src[0], src[3]];
        }
        let mask = CreateBitmap(size as i32, size as i32, 1, 1, None);
        let icon = CreateIconIndirect(&ICONINFO {
            fIcon: true.into(),
            hbmMask: mask,
            hbmColor: color,
            ..Default::default()
        });
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}

fn emit(event: TrayEvent) {
    HANDLER.with(|h| {
        if let Some(handler) = h.borrow().as_ref() {
            handler(event);
        }
    });
}

fn show_menu(hwnd: HWND, x: i32, y: i32) {
    let Some(labels) = LABELS.with(|l| l.borrow().clone()) else { return };
    record_foreground_window();
    let to_wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (open, find, quit) = (to_wide(&labels.open), to_wide(&labels.find_phone), to_wide(&labels.quit));
    let palette = to_wide(&labels.command_palette);
    let take_photo = to_wide(&labels.take_photo);
    let scan_doc = to_wide(&labels.scan_document);
    let link = to_wide(&labels.open_link);
    // SAFETY: the menu and strings outlive TrackPopupMenuEx; the menu is
    // destroyed afterwards. SetForegroundWindow/WM_NULL are the documented
    // way to make the menu close when clicking elsewhere.
    let command = unsafe {
        let Ok(menu): windows::core::Result<HMENU> = CreatePopupMenu() else { return };
        let find_flags = if CAN_FIND_PHONE.with(Cell::get) { MF_STRING } else { MF_STRING | MF_GRAYED };
        let _ = AppendMenuW(menu, find_flags, CMD_TAKE_PHOTO as usize, PCWSTR(take_photo.as_ptr()));
        let _ = AppendMenuW(menu, find_flags, CMD_SCAN_DOCUMENT as usize, PCWSTR(scan_doc.as_ptr()));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, CMD_OPEN as usize, PCWSTR(open.as_ptr()));
        let _ = AppendMenuW(menu, MF_STRING, CMD_COMMAND_PALETTE as usize, PCWSTR(palette.as_ptr()));
        let _ = AppendMenuW(menu, find_flags, CMD_FIND_PHONE as usize, PCWSTR(find.as_ptr()));
        let _ = AppendMenuW(menu, find_flags, CMD_OPEN_LINK as usize, PCWSTR(link.as_ptr()));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, CMD_QUIT as usize, PCWSTR(quit.as_ptr()));
        let _ = SetForegroundWindow(hwnd);
        let command =
            TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN).0, x, y, hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        command.0 as u32
    };
    match command {
        CMD_TAKE_PHOTO => emit(TrayEvent::TakePhoto),
        CMD_SCAN_DOCUMENT => emit(TrayEvent::ScanDocument),
        CMD_OPEN => emit(TrayEvent::Open),
        CMD_COMMAND_PALETTE => emit(TrayEvent::CommandPalette),
        CMD_FIND_PHONE => emit(TrayEvent::FindPhone),
        CMD_OPEN_LINK => emit(TrayEvent::OpenLinkOnPhone),
        CMD_QUIT => emit(TrayEvent::Quit),
        _ => {}
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        CALLBACK_MESSAGE => {
            // NOTIFYICON_VERSION_4: the event is in LOWORD(lParam) and the
            // anchor point in wParam.
            let event = (lparam.0 & 0xFFFF) as u32;
            match event {
                NIN_SELECT | NIN_KEYSELECT => emit(TrayEvent::Open),
                WM_CONTEXTMENU => {
                    let x = (wparam.0 & 0xFFFF) as i16 as i32;
                    let y = ((wparam.0 >> 16) & 0xFFFF) as i16 as i32;
                    show_menu(hwnd, x, y);
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == TIMER_TRACK_FG {
                record_foreground_window();
            }
            LRESULT(0)
        }
        WM_UPDATE_HOTKEYS => {
            // SAFETY: `hwnd` is owned by this thread.
            unsafe { apply_hotkeys_on_window(hwnd) };
            LRESULT(0)
        }
        WM_HOTKEY => {
            record_foreground_window();
            match wparam.0 as i32 {
                HOTKEY_PHOTO_ID => emit(TrayEvent::TakePhoto),
                HOTKEY_SCAN_ID => emit(TrayEvent::ScanDocument),
                HOTKEY_PALETTE_ID => emit(TrayEvent::CommandPalette),
                _ => {}
            }
            LRESULT(0)
        }
        WM_POWERBROADCAST => {
            if wparam.0 == PBT_APMRESUMEAUTOMATIC {
                emit(TrayEvent::Resumed);
            }
            LRESULT(1)
        }
        WM_SETTINGCHANGE => {
            // SAFETY: for WM_SETTINGCHANGE, lParam is null or a valid
            // null-terminated string for the duration of the message.
            let area =
                if lparam.0 == 0 { None } else { unsafe { PCWSTR(lparam.0 as *const u16).to_string().ok() } };
            if area.as_deref() == Some("ImmersiveColorSet")
                || wparam.0 == SPI_SETCLIENTAREAANIMATION
                || wparam.0 == SPI_SETDESKWALLPAPER
            {
                emit(TrayEvent::AppearanceChanged);
            }
            LRESULT(0)
        }
        // Explorer restarted: the tray is new and our icon must be re-added.
        m if m != 0 && m == TASKBAR_CREATED.with(Cell::get) => {
            add_icon(hwnd);
            LRESULT(0)
        }
        // SAFETY: default handling for everything else.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_and_invalid_hotkey_specs() {
        let (mods, vk) = parse_hotkey("Ctrl+Alt+C").expect("Ctrl+Alt+C");
        assert_eq!(mods, MOD_CONTROL.0 | MOD_ALT.0 | MOD_NOREPEAT.0);
        assert_eq!(vk, u32::from(b'C'));

        let (mods, vk) = parse_hotkey("Ctrl+Alt+Space").expect("Ctrl+Alt+Space");
        assert_eq!(mods, MOD_CONTROL.0 | MOD_ALT.0 | MOD_NOREPEAT.0);
        assert_eq!(vk, 0x20);

        let (mods, vk) = parse_hotkey(" ctrl + shift + f10 ").expect("ctrl+shift+f10");
        assert_eq!(mods, MOD_CONTROL.0 | MOD_SHIFT.0 | MOD_NOREPEAT.0);
        assert_eq!(vk, 0x79);

        assert!(parse_hotkey("").is_none());
        assert!(parse_hotkey("none").is_none());
        assert!(parse_hotkey("C").is_none());
        assert!(parse_hotkey("Ctrl+Alt").is_none());
        assert!(parse_hotkey("Ctrl+Alt+C+D").is_none());
    }
}
