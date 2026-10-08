// SPDX-License-Identifier: GPL-3.0-or-later
//! Opening links, locking and sleeping the PC.

use windows::{
    Win32::{
        System::{
            Power::SetSuspendState,
            Registry::{
                HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, KEY_READ, RegCloseKey, RegOpenKeyExW,
                RegQueryValueExW,
            },
            Shutdown::LockWorkStation,
        },
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

/// Opens a `mailto:` link in the default mail client.
pub fn open_mailto(email: &str) -> Result<(), String> {
    let trimmed = email.trim();
    if trimmed.is_empty() {
        return Err("empty email address".into());
    }
    open_url(&format!("mailto:{trimmed}"))
}

/// Opens a street address in the registered Windows maps handler if one is
/// installed, or in the default browser via a maps web URL otherwise
/// (Windows 11 deprecated the built-in Maps app, so `bingmaps:` / `ms-maps:`
/// are only used when an active handler is registered).
pub fn open_maps(address: &str) -> Result<(), String> {
    let trimmed = address.trim();
    if trimmed.is_empty() {
        return Err("empty address".into());
    }
    for scheme in ["bingmaps", "ms-maps"] {
        if is_url_scheme_registered(scheme) {
            let uri = format!("{scheme}:?where={}", nectarlink_core::clip_kind::percent_encode(trimmed));
            if open_url(&uri).is_ok() {
                return Ok(());
            }
        }
    }
    open_url(&nectarlink_core::maps_web_url(trimmed))
}

/// Returns `true` if `scheme` has an installed classic command handler or an
/// active `UserChoice` `ProgId` under `HKEY_CLASSES_ROOT`.
pub fn is_url_scheme_registered(scheme: &str) -> bool {
    let scheme = scheme.trim();
    if scheme.is_empty() || scheme.contains('\\') {
        return false;
    }
    // 1. Classic handler: HKCR\<scheme> has "URL Protocol" and shell\open\command exists.
    if key_has_value(HKEY_CLASSES_ROOT, scheme, "URL Protocol")
        && key_exists(HKEY_CLASSES_ROOT, &format!(r"{scheme}\shell\open\command"))
    {
        return true;
    }
    // 2. Modern AppX/MSIX handler: UserChoice ProgId exists and is registered in HKCR.
    let assoc_path =
        format!(r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\{scheme}\UserChoice");
    if let Some(prog_id) = read_reg_sz(HKEY_CURRENT_USER, &assoc_path, "ProgId")
        && !prog_id.is_empty()
        && key_exists(HKEY_CLASSES_ROOT, &prog_id)
    {
        return true;
    }
    false
}

fn key_exists(root: HKEY, subkey: &str) -> bool {
    let mut key = HKEY::default();
    // SAFETY: read-only RegOpenKeyExW; closed immediately on success.
    let status = unsafe { RegOpenKeyExW(root, &HSTRING::from(subkey), None, KEY_READ, &mut key) };
    if status.is_ok() {
        unsafe {
            let _ = RegCloseKey(key);
        }
        true
    } else {
        false
    }
}

fn key_has_value(root: HKEY, subkey: &str, value_name: &str) -> bool {
    let mut key = HKEY::default();
    // SAFETY: read-only RegOpenKeyExW + RegQueryValueExW; key closed below.
    if unsafe { RegOpenKeyExW(root, &HSTRING::from(subkey), None, KEY_READ, &mut key) }.is_err() {
        return false;
    }
    let exists = unsafe { RegQueryValueExW(key, &HSTRING::from(value_name), None, None, None, None) }.is_ok();
    unsafe {
        let _ = RegCloseKey(key);
    }
    exists
}

fn read_reg_sz(root: HKEY, subkey: &str, value_name: &str) -> Option<String> {
    let mut key = HKEY::default();
    // SAFETY: read-only RegOpenKeyExW + RegQueryValueExW; key closed below.
    if unsafe { RegOpenKeyExW(root, &HSTRING::from(subkey), None, KEY_READ, &mut key) }.is_err() {
        return None;
    }
    let val_h = HSTRING::from(value_name);
    let mut size: u32 = 0;
    let status = unsafe { RegQueryValueExW(key, &val_h, None, None, None, Some(&mut size)) };
    if status.is_err() || size < 2 {
        unsafe {
            let _ = RegCloseKey(key);
        }
        return None;
    }
    let mut buf = vec![0u16; (size as usize).div_ceil(2)];
    let mut byte_len = (buf.len() * 2) as u32;
    let status = unsafe {
        RegQueryValueExW(key, &val_h, None, None, Some(buf.as_mut_ptr().cast::<u8>()), Some(&mut byte_len))
    };
    unsafe {
        let _ = RegCloseKey(key);
    }
    if status.is_err() {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_scheme_registration_distinguishes_registered_and_unregistered_schemes() {
        assert!(is_url_scheme_registered("http"));
        assert!(!is_url_scheme_registered("nectarlink-nonexistent-scheme"));
        assert!(!is_url_scheme_registered(""));
    }
}
