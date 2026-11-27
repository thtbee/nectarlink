// SPDX-License-Identifier: GPL-3.0-or-later
//! Bringing the Island's window back when Windows hid it behind Qt's back:
//! a full-screen app, a display change or a session switch can do that, and
//! only the platform can show it again.

use windows::{
    Win32::{
        Foundation::{HWND, LPARAM},
        UI::WindowsAndMessaging::{
            EnumWindows, GetWindowTextW, GetWindowThreadProcessId, HWND_TOPMOST, IsWindowVisible,
            SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos, ShowWindow,
        },
    },
    core::BOOL,
};

/// Shows the top-level window titled `title` again if Windows hid it, keeping
/// it topmost without taking focus.
pub fn ensure_visible(title: &str) {
    let wanted: Vec<u16> = title.encode_utf16().collect();
    if wanted.is_empty() {
        return;
    }

    unsafe extern "system" fn each(hwnd: HWND, found: LPARAM) -> BOOL {
        // SAFETY: `found` points to the Vec passed below, alive for the call.
        let found = unsafe { &mut *(found.0 as *mut Vec<HWND>) };
        found.push(hwnd);
        BOOL(1)
    }

    let mut handles: Vec<HWND> = Vec::new();
    // SAFETY: the callback only pushes to `handles`, which outlives the call.
    let _ = unsafe { EnumWindows(Some(each), LPARAM(&mut handles as *mut Vec<HWND> as isize)) };

    let pid = std::process::id();
    for hwnd in handles {
        let mut owner = 0u32;
        // SAFETY: plain query on a window handle.
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&mut owner));
        }
        if owner != pid {
            continue;
        }
        let mut buffer = [0u16; 128];
        // SAFETY: `buffer` is a writable, appropriately sized text buffer.
        let len = unsafe { GetWindowTextW(hwnd, &mut buffer) }.max(0) as usize;
        if len == 0 || buffer[..len] != wanted[..] {
            continue;
        }
        // SAFETY: `hwnd` is one of ours, with the Island's own title.
        unsafe {
            if !IsWindowVisible(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            }
            let _ =
                SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }
}
