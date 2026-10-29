// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS integration: system facts, single instance, clipboard, sound,
//! notifications, volume, and shell operations.

pub mod audio;
pub mod audio_out;
pub mod autostart;
pub mod ble_beacon;
pub mod browser;
pub mod bt_hfp;
#[cfg(feature = "macos-native-camera")]
pub mod camera_output;
pub mod clipboard;
pub mod doctor;
pub mod elevated;
pub mod h264;
pub mod http;
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
pub mod wallpaper;
pub mod widget;
pub mod wifi;
pub mod window_capture;
pub mod wol;

/// Machine GUID on macOS: derived from IOPlatformSerialNumber.
pub fn machine_guid() -> Option<String> {
    let output =
        std::process::Command::new("ioreg").args(["-rd1", "-c", "IOPlatformExpertDevice"]).output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        if line.contains("IOPlatformSerialNumber")
            && let Some(pos) = line.find('=')
        {
            let val = line[pos + 1..].trim().trim_matches('"');
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

/// The macOS version as "<major>.<minor>.<patch>".
pub fn os_version() -> String {
    let output = std::process::Command::new("sw_vers").arg("-productVersion").output().ok();
    if let Some(out) = output {
        let ver = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !ver.is_empty() {
            return ver;
        }
    }
    "15.0.0".to_string()
}

/// Whether this Mac runs on a battery (MacBook).
pub fn is_laptop() -> bool {
    let output = std::process::Command::new("pmset").arg("-g").arg("batt").output().ok();
    if let Some(out) = output {
        let text = String::from_utf8_lossy(&out.stdout);
        text.contains("InternalBattery")
    } else {
        false
    }
}

/// Whether Low Power Mode is on.
pub fn power_saver() -> bool {
    let output = std::process::Command::new("pmset").arg("-g").output().ok();
    if let Some(out) = output {
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            if line.contains("lowpowermode") {
                return line.contains('1');
            }
        }
    }
    false
}

/// Whether macOS is set to dark mode.
pub fn system_dark() -> bool {
    let output =
        std::process::Command::new("defaults").args(["read", "-g", "AppleInterfaceStyle"]).output().ok();
    if let Some(out) = output { String::from_utf8_lossy(&out.stdout).trim() == "Dark" } else { false }
}

/// Whether Reduce Motion is enabled.
pub fn reduce_motion() -> bool {
    let output = std::process::Command::new("defaults")
        .args(["read", "com.apple.universalaccess", "reduceMotion"])
        .output()
        .ok();
    if let Some(out) = output { String::from_utf8_lossy(&out.stdout).trim() == "1" } else { false }
}

/// Primary display work area in physical pixels.
pub fn work_area() -> Option<(i32, i32, i32, i32)> {
    None
}

/// How far this PC's time zone is from UTC at `at_ms` (Unix milliseconds).
pub fn local_offset_ms(at_ms: i64) -> i64 {
    crate::bridge::native::ffi::mac_local_offset_ms(at_ms)
}

/// Free bytes on the drive holding `path`.
pub fn free_space(path: &std::path::Path) -> Option<u64> {
    let s = path.to_str()?;
    let mut ok = false;
    let bytes = crate::bridge::native::ffi::mac_free_space(s, &mut ok);
    ok.then_some(bytes)
}

/// Fills `buf` from cryptographic randomness.
pub fn secure_random(buf: &mut [u8]) -> bool {
    use std::io::Read;
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") { f.read_exact(buf).is_ok() } else { false }
}

/// On macOS, COM is not used. Runs `f` directly.
#[allow(dead_code)]
pub fn with_com<T>(f: impl FnOnce() -> T) -> T {
    f()
}

#[allow(dead_code)]
pub(crate) fn reg_string(_root: u32, _key: &str, _value: &str) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_space_returns_some_on_system_drive() {
        let space = free_space(std::path::Path::new("/"));
        assert!(space.is_some());
    }
}
