// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS integration: system facts, single instance, clipboard, sound,
//! notifications, volume, and shell operations.

pub mod audio;
pub mod audio_out;
pub mod autostart;
pub mod ble_beacon;
pub mod browser;
pub mod bt_hfp;
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
/// Whether this Mac runs on its battery right now.
pub fn on_battery() -> bool {
    std::process::Command::new("pmset")
        .args(["-g", "batt"])
        .output()
        .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).contains("'Battery Power'"))
}

/// Whether the session is locked, so nothing the phone sends should pop up on
/// this Mac.
///
/// `CGSessionCopyCurrentDictionary` is the public CoreGraphics view of the
/// current login session. Measured on macOS 26.6: while the screen is locked the
/// dictionary carries `CGSSessionScreenIsLocked = true`; when it is unlocked the
/// key is absent entirely, and `kCGSSessionOnConsoleKey` stays true throughout,
/// so only the screen-lock key can answer this.
///
/// A dictionary that can't be read counts as unlocked, like Linux when
/// `LockedHint` is missing: better to show a notification than to hide every one.
#[allow(unsafe_code)]
pub fn session_locked() -> bool {
    let dictionary = unsafe { CGSessionCopyCurrentDictionary() };
    if dictionary.is_null() {
        return false;
    }
    let screen_locked = dictionary_bool(dictionary, "CGSSessionScreenIsLocked");
    // SAFETY: CGSessionCopyCurrentDictionary returns one reference we own.
    unsafe { CFRelease(dictionary) };
    is_locked(screen_locked)
}

/// The lock decision on its own, so it can be tested without CoreFoundation.
/// Only `true` means locked: the key is absent while the screen is unlocked.
fn is_locked(screen_locked: Option<bool>) -> bool {
    screen_locked == Some(true)
}

type CFTypeRef = *const std::ffi::c_void;

#[allow(unsafe_code)]
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    /// A +1 dictionary describing the current login session, or null.
    fn CGSessionCopyCurrentDictionary() -> CFTypeRef;
}

#[allow(unsafe_code)]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDictionaryGetValue(dictionary: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
    fn CFBooleanGetValue(boolean: CFTypeRef) -> u8;
    fn CFGetTypeID(value: CFTypeRef) -> usize;
    fn CFBooleanGetTypeID() -> usize;
    fn CFStringCreateWithCString(alloc: CFTypeRef, text: *const std::ffi::c_char, encoding: u32)
    -> CFTypeRef;
    fn CFRelease(value: CFTypeRef);
}

/// A boolean entry of `dictionary`, or `None` when the key is absent or holds
/// something other than a CFBoolean.
#[allow(unsafe_code)]
fn dictionary_bool(dictionary: CFTypeRef, key: &str) -> Option<bool> {
    /// `kCFStringEncodingUTF8`.
    const UTF8: u32 = 0x0800_0100;
    let key = std::ffi::CString::new(key).ok()?;
    // SAFETY: `key` is a live NUL-terminated string and the call copies it.
    let key_ref = unsafe { CFStringCreateWithCString(std::ptr::null(), key.as_ptr(), UTF8) };
    if key_ref.is_null() {
        return None;
    }
    // SAFETY: both are live CoreFoundation objects; the value stays borrowed.
    let value = unsafe { CFDictionaryGetValue(dictionary, key_ref) };
    // SAFETY: `key_ref` came from CFStringCreateWithCString and is ours.
    unsafe { CFRelease(key_ref) };
    if value.is_null() {
        return None;
    }
    // The session dictionary holds strings, numbers and booleans, and reading a
    // non-boolean as one would be unsound, so check the type first.
    // SAFETY: `value` is a non-null CoreFoundation object owned by the dictionary.
    if unsafe { CFGetTypeID(value) } != unsafe { CFBooleanGetTypeID() } {
        return None;
    }
    // SAFETY: the check above proved `value` is a CFBoolean.
    Some(unsafe { CFBooleanGetValue(value) != 0 })
}

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

    #[test]
    fn locked_when_the_screen_is_locked() {
        assert!(is_locked(Some(true)));
    }

    /// The key is absent, not `false`, while the screen is unlocked (measured).
    #[test]
    fn unlocked_when_the_key_is_absent() {
        assert!(!is_locked(None));
    }

    #[test]
    fn unlocked_when_the_key_says_so() {
        assert!(!is_locked(Some(false)));
    }

    #[test]
    fn reads_this_machines_session_without_panicking() {
        // Whatever this machine's state, the call must answer, not crash.
        let _ = session_locked();
    }
}
