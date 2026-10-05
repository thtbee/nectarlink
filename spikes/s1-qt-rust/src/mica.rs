// SPDX-License-Identifier: GPL-3.0-or-later
//! Applies the Windows 11 Mica backdrop to this thread's top-level windows.
//! Requires `native::ffi::enable_window_alpha()` before Qt starts and a
//! transparent QML root. Mica's light or dark tint follows the window's
//! color scheme, which QML sets to match the app theme.

#[cfg(windows)]
#[allow(unsafe_code)]
pub fn apply_to_process_windows() {
    use windows::{
        Win32::{
            Foundation::{HWND, LPARAM},
            Graphics::Dwm::{
                DWM_SYSTEMBACKDROP_TYPE, DWMSBT_MAINWINDOW, DWMWA_SYSTEMBACKDROP_TYPE,
                DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
            },
            System::Threading::GetCurrentThreadId,
            UI::{Controls::MARGINS, WindowsAndMessaging::EnumThreadWindows},
        },
        core::BOOL,
    };

    unsafe extern "system" fn apply(hwnd: HWND, _: LPARAM) -> BOOL {
        let backdrop: DWM_SYSTEMBACKDROP_TYPE = DWMSBT_MAINWINDOW;
        let margins = MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 };
        // SAFETY: hwnd comes from EnumThreadWindows; the attribute buffer is a
        // live DWM_SYSTEMBACKDROP_TYPE of the size passed.
        unsafe {
            // Best effort: on failure the window keeps its opaque frame.
            let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_SYSTEMBACKDROP_TYPE,
                std::ptr::from_ref(&backdrop).cast(),
                std::mem::size_of::<DWM_SYSTEMBACKDROP_TYPE>() as u32,
            );
        }
        BOOL(1)
    }

    // SAFETY: the callback only calls DWM functions on the enumerated handles.
    unsafe {
        let _ = EnumThreadWindows(GetCurrentThreadId(), Some(apply), LPARAM(0));
    }
}

#[cfg(not(windows))]
pub fn apply_to_process_windows() {}
