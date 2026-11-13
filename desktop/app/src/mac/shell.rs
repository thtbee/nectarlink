// SPDX-License-Identifier: GPL-3.0-or-later
//! Opening links, maps, email, locking, and sleeping the Mac.

use std::path::Path;

/// Opens a link in the default browser using macOS `open`.
pub fn open_url(url: &str) -> Result<(), String> {
    std::process::Command::new("open").arg(url).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Opens a `mailto:` link in the default mail client.
pub fn open_mailto(email: &str) -> Result<(), String> {
    let trimmed = email.trim();
    if trimmed.is_empty() {
        return Err("empty email address".into());
    }
    open_url(&format!("mailto:{trimmed}"))
}

/// Opens a street address in Apple Maps or web maps.
pub fn open_maps(address: &str) -> Result<(), String> {
    let trimmed = address.trim();
    if trimmed.is_empty() {
        return Err("empty address".into());
    }
    let url = format!("https://maps.apple.com/?q={}", urlencoding::encode(trimmed));
    open_url(&url)
}

/// Returns `true` if `scheme` is recognized.
#[allow(dead_code)]
pub fn is_url_scheme_registered(scheme: &str) -> bool {
    matches!(scheme.trim(), "http" | "https" | "mailto" | "maps")
}

/// Starts a program with arguments via `open` or direct process spawn.
#[allow(dead_code)]
pub fn run(program: &Path, arguments: &str) -> Result<(), String> {
    let mut cmd = std::process::Command::new(program);
    if !arguments.is_empty() {
        cmd.args(arguments.split_whitespace());
    }
    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Locks the Mac screen.
pub fn lock() -> Result<(), String> {
    // Legacy macOS (< Big Sur): CGSession -suspend locked the screen directly.
    // Modern macOS (11+) removed CGSession, so it falls back to osascript.
    let status = std::process::Command::new(
        "/System/Library/CoreServices/Menu Extras/User.menu/Contents/Resources/CGSession",
    )
    .arg("-suspend")
    .status();
    if status.is_ok_and(|s| s.success()) {
        return Ok(());
    }
    // Fallback to osascript with status check
    let output = std::process::Command::new("osascript")
        .args(["-e", r#"tell application "System Events" to key code 12 using {control down, command down}"#])
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// Puts the Mac to sleep.
pub fn sleep() -> Result<(), String> {
    std::process::Command::new("pmset").arg("sleepnow").spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Macs aren't hibernated on request (#355).
pub fn hibernate() -> Result<(), String> {
    Err("not on macOS".into())
}

/// Opens a directory in Finder.
pub fn open_folder(path: &Path) -> Result<(), String> {
    std::process::Command::new("open").arg(path).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Launches an application bundle or executable.
pub fn launch_app(path: &Path) -> Result<(), String> {
    std::process::Command::new("open").arg(path).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Runs a shell command.
pub fn run_command(command: &str) -> Result<(), String> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Err("empty command".into());
    }
    std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(trimmed)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("failed to run command: {e}"))
}

mod urlencoding {
    pub fn encode(s: &str) -> String {
        let mut out = String::new();
        for b in s.bytes() {
            if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
                out.push(b as char);
            } else {
                out.push_str(&format!("%{:02X}", b));
            }
        }
        out
    }
}

/// Moves `path` to the Trash in the user's home folder.
pub fn recycle_file(path: &Path) -> Result<(), String> {
    let trash = dirs::home_dir().ok_or("no home folder")?.join(".Trash");
    std::fs::create_dir_all(&trash).map_err(|e| e.to_string())?;
    let name = path.file_name().ok_or("no file name")?;
    let mut target = trash.join(name);
    let mut number = 1;
    while target.exists() {
        target = trash.join(format!("{} {number}", name.to_string_lossy()));
        number += 1;
    }
    std::fs::rename(path, &target).map_err(|e| e.to_string())
}
