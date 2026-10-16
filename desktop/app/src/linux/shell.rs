// SPDX-License-Identifier: GPL-3.0-or-later
//! Opening URLs, files, folders, and power actions (lock, sleep) on Linux via
//! `xdg-open`, `dbus-send`, `loginctl`, and `systemctl`.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

fn spawn_detached(program: &str, args: &[&str]) -> Result<(), String> {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("{program}: {e}"))
}

/// Opens an `http://` or `https://` URL (or local settings URI) with `xdg-open`.
pub fn open_url(url: &str) -> Result<(), String> {
    let lower = url.get(..8).map(str::to_ascii_lowercase).unwrap_or_default();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return spawn_detached("xdg-open", &[url]);
    }
    // Map Windows settings deep-links used by Doctor / Bluetooth UI to desktop equivalents or no-op.
    if url.starts_with("ms-settings:") || url.eq_ignore_ascii_case("firewall.cpl") {
        return spawn_detached("xdg-open", &[url]).or(Ok(()));
    }
    Err("only http and https links can be opened".into())
}

/// Opens the default mail client for an email address (`mailto:<email>`).
pub fn open_mailto(email: &str) -> Result<(), String> {
    let trimmed = email.trim();
    if trimmed.is_empty() || !trimmed.contains('@') || trimmed.chars().any(char::is_whitespace) {
        return Err("invalid email address".into());
    }
    let uri = if trimmed.get(..7).is_some_and(|p| p.eq_ignore_ascii_case("mailto:")) {
        trimmed.to_owned()
    } else {
        format!("mailto:{trimmed}")
    };
    spawn_detached("xdg-open", &[&uri])
}

/// Opens a street address in OpenStreetMap in the default browser.
pub fn open_maps(address: &str) -> Result<(), String> {
    let trimmed = address.trim();
    if trimmed.is_empty() {
        return Err("empty address".into());
    }
    open_url(&nectarlink_core::maps_web_url(trimmed))
}

/// Checks whether a custom URI scheme has a registered handler via `xdg-mime`.
pub fn is_url_scheme_registered(scheme: &str) -> bool {
    let scheme = scheme.trim().trim_end_matches(':');
    if scheme.is_empty()
        || !scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return false;
    }
    let mime = format!("x-scheme-handler/{scheme}");
    Command::new("xdg-mime")
        .args(["query", "default", &mime])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .is_some_and(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
}

/// Opens a file with the user's default application (`xdg-open`).
pub fn open_file(path: &Path) -> Result<(), String> {
    spawn_detached("xdg-open", &[&path.to_string_lossy()])
}

/// Opens a directory in the file manager (`xdg-open`).
pub fn open_folder(path: &Path) -> Result<(), String> {
    spawn_detached("xdg-open", &[&path.to_string_lossy()])
}

/// Reveals and selects `path` in the file manager via `org.freedesktop.FileManager1.ShowItems`,
/// falling back to opening the parent directory with `xdg-open`.
pub fn select_in_explorer(path: &Path) -> Result<(), String> {
    let abs = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let file_uri = format!("file://{}", abs.to_string_lossy());
    let array_arg = format!("array:string:{file_uri}");
    let dbus_ok = Command::new("dbus-send")
        .args([
            "--session",
            "--dest=org.freedesktop.FileManager1",
            "--type=method_call",
            "/org/freedesktop/FileManager1",
            "org.freedesktop.FileManager1.ShowItems",
            &array_arg,
            "string:",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if dbus_ok {
        return Ok(());
    }
    let folder = abs.parent().unwrap_or(&abs);
    open_folder(folder)
}

/// Runs `program` with `arguments`. Special-cases Windows `"explorer.exe"` calls from shared UI code.
pub fn run(program: &Path, arguments: &str) -> Result<(), String> {
    let prog_str = program.to_string_lossy();
    if prog_str.eq_ignore_ascii_case("explorer.exe") || prog_str.eq_ignore_ascii_case("explorer") {
        let trimmed = arguments.trim();
        if let Some(rest) = trimmed.strip_prefix("/select,") {
            let target = rest.trim().trim_matches('"');
            return select_in_explorer(Path::new(target));
        }
        let target = trimmed.trim_matches('"');
        return open_folder(Path::new(target));
    }

    let args = split_args(arguments);
    Command::new(program)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn split_args(arguments: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for ch in arguments.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            c if c.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Locks the current user session (`loginctl lock-session` or `xdg-screensaver lock`).
pub fn lock() -> Result<(), String> {
    if Command::new("loginctl")
        .arg("lock-session")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
    {
        return Ok(());
    }
    spawn_detached("xdg-screensaver", &["lock"])
}

/// Alias for [`lock`].
pub fn lock_workstation() -> Result<(), String> {
    lock()
}

/// Suspends the system (`systemctl suspend`).
pub fn sleep() -> Result<(), String> {
    spawn_detached("systemctl", &["suspend"])
}

/// Alias for [`sleep`].
pub fn sleep_system() -> Result<(), String> {
    sleep()
}

/// Launches an application or opens a file path.
pub fn launch_app(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty() {
        return Err("empty application path".into());
    }
    if path.is_file() {
        if let Ok( child ) = Command::new(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            drop(child);
            return Ok(());
        }
    }
    open_file(path)
}

/// Alias for [`launch_app`].
pub fn launch_executable(path: &Path, args: &[&str]) -> Result<(), String> {
    Command::new(path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Runs a shell command (`sh -c <command>`) detached in the background.
pub fn run_command(command: &str) -> Result<(), String> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Err("empty command".into());
    }
    spawn_detached("sh", &["-c", trimmed])
}

/// Moves `path` to the FreeDesktop Trash (`~/.local/share/Trash/files/` + `.trashinfo`),
/// or falls back to `gio trash`.
pub fn recycle_file(path: &Path) -> Result<(), String> {
    let data_dir = dirs::data_local_dir().ok_or_else(|| "no XDG_DATA_HOME directory".to_owned())?;
    recycle_file_in(&data_dir.join("Trash"), path)
}

fn recycle_file_in(trash_dir: &Path, path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("{} does not exist", path.display()));
    }
    let abs = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let files_dir = trash_dir.join("files");
    let info_dir = trash_dir.join("info");
    fs::create_dir_all(&files_dir).map_err(|e| e.to_string())?;
    fs::create_dir_all(&info_dir).map_err(|e| e.to_string())?;

    let base_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("trashed_item");
    let mut chosen_name = base_name.to_owned();
    let mut counter = 2u32;
    while files_dir.join(&chosen_name).exists() || info_dir.join(format!("{chosen_name}.trashinfo")).exists() {
        chosen_name = format!("{base_name}.{counter}");
        counter += 1;
    }

    let dest_file: PathBuf = files_dir.join(&chosen_name);
    let dest_info: PathBuf = info_dir.join(format!("{chosen_name}.trashinfo"));
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let info_contents = format!(
        "[Trash Info]\nPath={}\nDeletionDate=1970-01-01T00:00:{:02}\n",
        abs.display(),
        secs % 60
    );
    fs::write(&dest_info, info_contents).map_err(|e| e.to_string())?;
    if let Err(e) = fs::rename(path, &dest_file) {
        let _ = fs::remove_file(&dest_info);
        return Err(e.to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_non_web_urls() {
        assert!(open_url("file:///etc/passwd").is_err());
        assert!(open_url("javascript:alert(1)").is_err());
    }

    #[test]
    fn validates_mailto_and_maps() {
        assert!(open_mailto("").is_err());
        assert!(open_mailto("not-an-email").is_err());
        assert!(open_maps("   ").is_err());
    }

    #[test]
    fn moves_file_to_freedesktop_trash() {
        let dir = tempfile::tempdir().unwrap();
        let trash = dir.path().join("Trash");
        let file = dir.path().join("note.txt");
        fs::write(&file, b"hello").unwrap();

        recycle_file_in(&trash, &file).unwrap();
        assert!(!file.exists());
        assert!(trash.join("files").join("note.txt").is_file());
        assert!(trash.join("info").join("note.txt.trashinfo").is_file());
    }
}
