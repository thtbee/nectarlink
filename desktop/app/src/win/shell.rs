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

/// Launches a `.exe` or `.lnk` file on the PC via `ShellExecuteW` with no
/// command-line arguments (`lpParameters = None`).
pub fn launch_app(path: &std::path::Path) -> Result<(), String> {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    if ext != "exe" && ext != "lnk" {
        return Err("only .exe and .lnk files can be launched".into());
    }
    let file = HSTRING::from(path.as_os_str());
    let dir = path.parent().map(|p| HSTRING::from(p.as_os_str()));
    // SAFETY: plain ShellExecuteW with `lpParameters = None` so no arguments are passed.
    let result = unsafe {
        match &dir {
            Some(d) => ShellExecuteW(None, w!("open"), &file, None, d, SW_SHOWNORMAL),
            None => ShellExecuteW(None, w!("open"), &file, None, None, SW_SHOWNORMAL),
        }
    };
    if result.0 as isize > 32 { Ok(()) } else { Err(format!("ShellExecute returned {}", result.0 as isize)) }
}

/// Runs a PC-configured shell command (`cmd.exe /C <command>`) without flashing
/// a console window.
pub fn run_command(command: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Err("empty command".into());
    }
    // Exactly as typed: Rust's usual argument quoting is for programs that
    // parse their command line like C programs do, which cmd.exe doesn't.
    std::process::Command::new("cmd.exe")
        .raw_arg("/C ")
        .raw_arg(trimmed)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("failed to run command: {e}"))
}
