// SPDX-License-Identifier: GPL-3.0-or-later
//! The notification-area (tray) icon and its menu, plus the system
//! broadcasts a hidden top-level window receives: resume from sleep and
//! theme/animation setting changes.
//!
//! Everything runs on the Qt GUI thread: the hidden window is created there,
//! and Qt's event loop dispatches its messages.

use std::cell::{Cell, RefCell};

use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
            DeleteObject, HBITMAP,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Shell::{
                NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
                NIN_SELECT, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
            },
            WindowsAndMessaging::{
                AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
                DestroyIcon, DestroyMenu, DestroyWindow, GetSystemMetrics, HICON, HMENU, ICONINFO, MF_GRAYED,
                MF_SEPARATOR, MF_STRING, PostMessageW, RegisterClassExW, RegisterWindowMessageW, SM_CXSMICON,
                SetForegroundWindow, TPM_BOTTOMALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx,
                WINDOW_EX_STYLE, WM_APP, WM_CONTEXTMENU, WM_NULL, WM_POWERBROADCAST, WM_SETTINGCHANGE,
                WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_POPUP,
            },
        },
    },
    core::{PCWSTR, w},
};

/// What the user or the system asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    Open,
    FindPhone,
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
    pub find_phone: String,
    pub quit: String,
}

const CALLBACK_MESSAGE: u32 = WM_APP + 1;
const ICON_ID: u32 = 1;
const CMD_OPEN: u32 = 1;
const CMD_FIND_PHONE: u32 = 2;
const CMD_QUIT: u32 = 3;
const PBT_APMRESUMEAUTOMATIC: usize = 0x12;
/// NIN_SELECT | NINF_KEY (shellapi.h): the icon was activated with the keyboard.
const NIN_KEYSELECT: u32 = NIN_SELECT | 0x1;
const SPI_SETCLIENTAREAANIMATION: usize = 0x1043;
const SPI_SETDESKWALLPAPER: usize = 0x0014;

type Handler = Box<dyn Fn(TrayEvent)>;

thread_local! {
    static HANDLER: RefCell<Option<Handler>> = RefCell::new(None);
    static LABELS: RefCell<Option<MenuLabels>> = const { RefCell::new(None) };
    static CAN_FIND_PHONE: Cell<bool> = const { Cell::new(false) };
    static TOOLTIP: RefCell<String> = const { RefCell::new(String::new()) };
    static ICON: Cell<HICON> = Cell::new(HICON::default());
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
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
            CreateWindowExW(
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
            )?
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
        let data = icon_data(self.hwnd);
        // SAFETY: removing our own icon and destroying our own window/icon.
        unsafe {
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
    let to_wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (open, find, quit) = (to_wide(&labels.open), to_wide(&labels.find_phone), to_wide(&labels.quit));
    // SAFETY: the menu and strings outlive TrackPopupMenuEx; the menu is
    // destroyed afterwards. SetForegroundWindow/WM_NULL are the documented
    // way to make the menu close when clicking elsewhere.
    let command = unsafe {
        let Ok(menu): windows::core::Result<HMENU> = CreatePopupMenu() else { return };
        let find_flags = if CAN_FIND_PHONE.with(Cell::get) { MF_STRING } else { MF_STRING | MF_GRAYED };
        let _ = AppendMenuW(menu, MF_STRING, CMD_OPEN as usize, PCWSTR(open.as_ptr()));
        let _ = AppendMenuW(menu, find_flags, CMD_FIND_PHONE as usize, PCWSTR(find.as_ptr()));
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
        CMD_OPEN => emit(TrayEvent::Open),
        CMD_FIND_PHONE => emit(TrayEvent::FindPhone),
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
