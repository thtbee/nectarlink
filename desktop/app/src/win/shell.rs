// SPDX-License-Identifier: GPL-3.0-or-later
//! Opening links, locking and sleeping the PC.

use windows::{
    Win32::{
        System::{Power::SetSuspendState, Shutdown::LockWorkStation},
        UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
    },
    core::{HSTRING, w},
};

/// Opens a link in the default browser.
pub fn open_url(url: &str) -> Result<(), String> {
    // SAFETY: plain ShellExecute with valid strings; the result is a
    // pseudo-handle that's only compared.
    let result = unsafe { ShellExecuteW(None, w!("open"), &HSTRING::from(url), None, None, SW_SHOWNORMAL) };
    // Values above 32 mean success.
    if result.0 as isize > 32 { Ok(()) } else { Err(format!("ShellExecute returned {}", result.0 as isize)) }
}

/// Starts a program with arguments (Windows asks for permission when it
/// needs administrator rights).
pub fn run(program: &std::path::Path, arguments: &str) -> Result<(), String> {
    // SAFETY: as in `open_url`.
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(program.as_os_str()),
            &HSTRING::from(arguments),
            None,
            SW_SHOWNORMAL,
        )
    };
    if result.0 as isize > 32 { Ok(()) } else { Err(format!("ShellExecute returned {}", result.0 as isize)) }
}

/// Locks the PC (shows the sign-in screen).
pub fn lock() -> Result<(), String> {
    // SAFETY: no arguments.
    unsafe { LockWorkStation() }.map_err(|e| e.to_string())
}

/// Puts the PC to sleep (not hibernate; wake events stay allowed).
pub fn sleep() -> Result<(), String> {
    // SAFETY: plain arguments.
    if unsafe { SetSuspendState(false, false, false) } {
        Ok(())
    } else {
        Err("SetSuspendState failed".into())
    }
}
