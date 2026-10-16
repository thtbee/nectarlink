// SPDX-License-Identifier: GPL-3.0-or-later
//! XDG autostart management (`~/.config/autostart/nectarlink.desktop`) on Linux.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

const DESKTOP_FILE_NAME: &str = "nectarlink.desktop";

/// The command line written into the autostart entry's `Exec=` line.
pub fn command(exe: &Path) -> String {
    format!("\"{}\" --minimized", exe.display())
}

fn autostart_file_path_in(config_dir: &Path) -> PathBuf {
    config_dir.join("autostart").join(DESKTOP_FILE_NAME)
}

fn autostart_file_path() -> io::Result<PathBuf> {
    let config_dir = dirs::config_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no XDG config directory"))?;
    Ok(autostart_file_path_in(&config_dir))
}

fn render_desktop_entry(exe: &Path) -> String {
    let cmd = command(exe);
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=NectarLink\n\
         Comment=Link your Android phone and PC\n\
         Exec={cmd}\n\
         Icon=nectarlink\n\
         Terminal=false\n\
         Categories=Utility;Network;\n\
         StartupNotify=false\n\
         X-GNOME-Autostart-enabled=true\n"
    )
}

fn set_in(config_dir: &Path, enabled: bool, exe: &Path) -> io::Result<()> {
    let path = autostart_file_path_in(config_dir);
    if enabled {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, render_desktop_entry(exe))
    } else {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }
}

fn is_enabled_in(config_dir: &Path) -> bool {
    let path = autostart_file_path_in(config_dir);
    let Ok(contents) = fs::read_to_string(path) else {
        return false;
    };
    !contents.lines().any(|line| {
        let t = line.trim();
        t.eq_ignore_ascii_case("Hidden=true") || t.eq_ignore_ascii_case("X-GNOME-Autostart-enabled=false")
    })
}

/// Adds or removes the XDG autostart `.desktop` entry for `exe`.
pub fn set(enabled: bool, exe: &Path) -> io::Result<()> {
    let path = autostart_file_path()?;
    if let Some(config_dir) = path.parent().and_then(Path::parent) {
        set_in(config_dir, enabled, exe)
    } else {
        Err(io::Error::new(io::ErrorKind::NotFound, "invalid XDG autostart path"))
    }
}

/// Alias for checking whether the XDG autostart entry is currently present and enabled.
pub fn is_enabled() -> bool {
    dirs::config_dir().as_deref().is_some_and(is_enabled_in)
}

/// Alias for enabling or disabling XDG autostart for the current executable.
pub fn set_enabled(enabled: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    set(enabled, &exe).map_err(|e| e.to_string())
}

/// Whether `exe` is installed in a standard Linux application location (rather
/// than run directly from a `target/` build directory).
pub fn is_installed(exe: &Path) -> bool {
    if std::env::var_os("APPIMAGE").is_some() {
        return true;
    }
    let s = exe.to_string_lossy();
    if s.contains("/target/debug/") || s.contains("/target/release/") {
        return false;
    }
    exe.starts_with("/usr/bin")
        || exe.starts_with("/usr/local/bin")
        || exe.starts_with("/opt")
        || dirs::data_local_dir().is_some_and(|d| exe.starts_with(d))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_removes_xdg_autostart_desktop_file() {
        let dir = tempfile::tempdir().unwrap();
        let exe = Path::new("/usr/bin/nectarlink-desktop");

        assert!(!is_enabled_in(dir.path()));
        set_in(dir.path(), true, exe).unwrap();
        assert!(is_enabled_in(dir.path()));

        let desktop = fs::read_to_string(autostart_file_path_in(dir.path())).unwrap();
        assert!(desktop.contains("Exec=\"/usr/bin/nectarlink-desktop\" --minimized"));

        set_in(dir.path(), false, exe).unwrap();
        assert!(!is_enabled_in(dir.path()));
        // Removing again is a no-op.
        set_in(dir.path(), false, exe).unwrap();
    }

    #[test]
    fn detects_installed_vs_build_paths() {
        assert!(is_installed(Path::new("/usr/bin/nectarlink-desktop")));
        assert!(is_installed(Path::new("/opt/nectarlink/nectarlink-desktop")));
        assert!(!is_installed(Path::new("/home/user/repo/target/debug/nectarlink-desktop")));
        assert!(!is_installed(Path::new("/home/user/repo/target/release/nectarlink-desktop")));
    }
}
