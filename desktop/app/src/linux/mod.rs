// SPDX-License-Identifier: GPL-3.0-or-later
//! Linux platform integration (`desktop/app/src/linux/`), matching the public
//! API of `desktop/app/src/win/`.

#![allow(dead_code, unsafe_code)]

pub mod audio;
pub mod audio_out;
pub mod autostart;
pub mod ble_beacon;
pub mod bt_hfp;
pub mod clipboard;
pub mod doctor;
pub mod elevated;
pub mod h264;
pub mod http;
pub mod icon;
pub mod image;
pub mod input;
pub mod jumplist;
pub mod media_sessions;
pub mod mp4;
pub mod net;
pub mod share_target;
pub mod shell;
pub mod shortcut;
pub mod single_instance;
pub mod smtc;
pub mod sound;
pub mod toast;
pub mod tray;
pub mod vcam;
pub mod vmic;
pub mod wallpaper;
pub mod widget;
pub mod wol;

use std::{fs, io::Read, process::Command};

/// A stable machine identifier from `/etc/machine-id` or `/var/lib/dbus/machine-id`.
pub fn machine_guid() -> Option<String> {
    for path in ["/etc/machine-id", "/var/lib/dbus/machine-id"] {
        if let Ok(contents) = fs::read_to_string(path) {
            let trimmed = contents.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_owned());
            }
        }
    }
    None
}

/// Human-readable Linux distribution and kernel version (e.g. `"Ubuntu 24.04 LTS (6.8.0)"`).
pub fn os_version() -> String {
    let distro = fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|text| parse_os_release_pretty_name(&text))
        .unwrap_or_else(|| "Linux".to_owned());
    let kernel = fs::read_to_string("/proc/sys/kernel/osrelease")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    match kernel {
        Some(k) => format!("{distro} ({k})"),
        None => distro,
    }
}

fn parse_os_release_pretty_name(contents: &str) -> Option<String> {
    for line in contents.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("PRETTY_NAME=") {
            let unquoted = rest.trim().trim_matches('"').trim_matches('\'').trim();
            if !unquoted.is_empty() {
                return Some(unquoted.to_owned());
            }
        }
    }
    None
}

/// Returns a synthetic build number (`22621`) so Windows 11 feature gates in shared UI stay enabled.
pub fn os_build_number() -> u32 {
    22621
}

/// Whether this PC is a laptop (battery present in `/sys/class/power_supply` or mobile DMI chassis).
pub fn is_laptop() -> bool {
    if let Ok(entries) = fs::read_dir("/sys/class/power_supply") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("BAT") {
                return true;
            }
            if let Ok(kind) = fs::read_to_string(entry.path().join("type"))
                && kind.trim().eq_ignore_ascii_case("Battery")
            {
                return true;
            }
        }
    }
    if let Ok(chassis) = fs::read_to_string("/sys/class/dmi/id/chassis_type") {
        let c = chassis.trim();
        // 8 = Portable, 9 = Laptop, 10 = Notebook, 14 = Sub Notebook,
        // 30 = Tablet, 31 = Convertible, 32 = Detachable
        if matches!(c, "8" | "9" | "10" | "14" | "30" | "31" | "32") {
            return true;
        }
    }
    false
}

/// Whether power-saver mode is active (`/sys/firmware/acpi/platform_profile` or `powerprofilesctl`).
pub fn power_saver() -> bool {
    if let Ok(profile) = fs::read_to_string("/sys/firmware/acpi/platform_profile")
        && profile.trim().eq_ignore_ascii_case("low-power")
    {
        return true;
    }
    if let Ok(output) = Command::new("powerprofilesctl").arg("get").output()
        && output.status.success()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        if text.trim().eq_ignore_ascii_case("power-saver") {
            return true;
        }
    }
    false
}

/// Parses GTK_THEME, gsettings color-scheme, and kdeglobals ColorScheme to decide dark mode.
pub(crate) fn parse_system_dark_sources(
    gtk_theme: Option<&str>,
    gsettings_color_scheme: Option<&str>,
    kdeglobals: Option<&str>,
) -> bool {
    if let Some(gtk) = gtk_theme {
        let lower = gtk.to_ascii_lowercase();
        if lower.contains("dark") {
            return true;
        }
        if lower.contains("light") {
            return false;
        }
    }
    if let Some(gs) = gsettings_color_scheme {
        let text = gs.to_ascii_lowercase();
        if text.contains("prefer-dark") {
            return true;
        }
        if text.contains("prefer-light") || text.contains("default") {
            return false;
        }
    }
    if let Some(kde) = kdeglobals {
        for line in kde.lines() {
            if let Some(scheme) = line.trim().strip_prefix("ColorScheme=") {
                return scheme.to_ascii_lowercase().contains("dark");
            }
        }
    }
    true
}

/// Whether the desktop prefers a dark color scheme.
pub fn system_dark() -> bool {
    let gtk_theme = std::env::var("GTK_THEME").ok();
    let gsettings = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "color-scheme"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
    let kdeglobals = dirs::config_dir().and_then(|c| fs::read_to_string(c.join("kdeglobals")).ok());
    parse_system_dark_sources(gtk_theme.as_deref(), gsettings.as_deref(), kdeglobals.as_deref())
}

/// Whether desktop animations are turned off.
pub fn reduce_motion() -> bool {
    if let Ok(output) =
        Command::new("gsettings").args(["get", "org.gnome.desktop.interface", "enable-animations"]).output()
        && output.status.success()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        if text.trim() == "false" {
            return true;
        }
    }
    false
}

/// Returns the usable desktop work area `(x, y, width, height)` in physical pixels.
pub fn work_area() -> Option<(i32, i32, i32, i32)> {
    if let Ok(output) = Command::new("xprop").args(["-root", "_NET_WORKAREA"]).output()
        && output.status.success()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        if let Some((_, rhs)) = text.split_once('=') {
            let nums: Vec<i32> =
                rhs.split(',').filter_map(|p| p.trim().parse::<i32>().ok()).take(4).collect();
            if let [x, y, w, h] = nums.as_slice()
                && *w > 0
                && *h > 0
            {
                return Some((*x, *y, *w, *h));
            }
        }
    }
    Some((0, 0, 1920, 1080))
}

/// Fills `buf` with cryptographically strong random bytes from `/dev/urandom`.
pub fn secure_random(buf: &mut [u8]) -> bool {
    if buf.is_empty() {
        return true;
    }
    let Ok(mut file) = fs::File::open("/dev/urandom") else {
        return false;
    };
    file.read_exact(buf).is_ok()
}

/// Runs `f` on the current thread (no-op COM wrapper on Linux).
pub fn with_com<T>(f: impl FnOnce() -> T) -> T {
    f()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_os_info_without_panicking() {
        assert!(!os_version().is_empty());
        assert_eq!(os_build_number(), 22621);
        let _ = machine_guid();
        let _ = is_laptop();
        let _ = power_saver();
        let _ = system_dark();
        let _ = reduce_motion();
        assert!(work_area().is_some());
    }

    #[test]
    fn parses_os_release_pretty_name() {
        let sample = "NAME=\"Ubuntu\"\nPRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\nID=ubuntu\n";
        assert_eq!(parse_os_release_pretty_name(sample).as_deref(), Some("Ubuntu 24.04.1 LTS"));
    }

    #[test]
    fn parses_system_dark_across_gtk_gnome_and_kde_sources() {
        assert!(parse_system_dark_sources(Some("Adwaita-dark"), Some("'prefer-light'"), None));
        assert!(!parse_system_dark_sources(Some("Adwaita-light"), Some("'prefer-dark'"), None));
        assert!(parse_system_dark_sources(None, Some("'prefer-dark'\n"), None));
        assert!(!parse_system_dark_sources(None, Some("'prefer-light'\n"), None));
        assert!(!parse_system_dark_sources(None, Some("'default'\n"), None));
        assert!(parse_system_dark_sources(None, None, Some("[General]\nColorScheme=BreezeDark\n")));
        assert!(!parse_system_dark_sources(None, None, Some("[General]\nColorScheme=BreezeLight\n")));
        assert!(parse_system_dark_sources(None, None, None));
    }

    #[test]
    fn fills_secure_random_bytes() {
        let mut buf = [0u8; 32];
        assert!(secure_random(&mut buf));
        assert!(buf.iter().any(|&b| b != 0));
    }

    #[test]
    fn runs_closure_with_com() {
        assert_eq!(with_com(|| 42), 42);
    }
}
