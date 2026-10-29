// SPDX-License-Identifier: GPL-3.0-or-later
//! Windows integration: system facts, single instance, tray, power and
//! network notifications, ringing. Window chrome lives in C++
//! (`cpp/native_window.cpp`) because it hooks Qt's native event handling.

#![allow(unsafe_code)]

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
pub mod wallpaper;
pub mod widget;
pub mod wifi;
pub mod window_capture;
pub mod wol;

use windows::{
    Win32::{
        Foundation::ERROR_SUCCESS,
        System::{
            Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
            Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS},
            Registry::{
                HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW,
            },
        },
        UI::WindowsAndMessaging::{
            SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
        },
    },
    core::{BOOL, HSTRING},
};

fn reg_dword(root: HKEY, key: &str, value: &str) -> Option<u32> {
    let mut data = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: data/size describe a live u32 buffer of the given size.
    let status = unsafe {
        RegGetValueW(
            root,
            &HSTRING::from(key),
            &HSTRING::from(value),
            RRF_RT_REG_DWORD,
            None,
            Some(std::ptr::from_mut(&mut data).cast()),
            Some(&mut size),
        )
    };
    (status == ERROR_SUCCESS).then_some(data)
}

/// Windows' per-installation machine GUID. It stays the same across app reinstalls, so a
/// hash of it lets phones recognise this PC after NectarLink is reinstalled.
pub fn machine_guid() -> Option<String> {
    reg_string(HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Cryptography", "MachineGuid")
}

pub(crate) fn reg_string(root: HKEY, key: &str, value: &str) -> Option<String> {
    // Long enough for a path (an install folder) as well as short values.
    let mut buf = [0u16; 1024];
    let mut size = std::mem::size_of_val(&buf) as u32;
    // SAFETY: buf/size describe a live UTF-16 buffer of the given byte size.
    let status = unsafe {
        RegGetValueW(
            root,
            &HSTRING::from(key),
            &HSTRING::from(value),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

const CURRENT_VERSION: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";

/// The Windows version as "10.0.<build>", which is what phones compare
/// against for features that need a minimum build.
pub fn os_version() -> String {
    let major = reg_dword(HKEY_LOCAL_MACHINE, CURRENT_VERSION, "CurrentMajorVersionNumber").unwrap_or(10);
    let minor = reg_dword(HKEY_LOCAL_MACHINE, CURRENT_VERSION, "CurrentMinorVersionNumber").unwrap_or(0);
    let build = reg_string(HKEY_LOCAL_MACHINE, CURRENT_VERSION, "CurrentBuildNumber").unwrap_or_default();
    format!("{major}.{minor}.{build}")
}

/// Whether this PC runs on a battery (so phones can show a laptop).
pub fn is_laptop() -> bool {
    let mut status = SYSTEM_POWER_STATUS::default();
    // SAFETY: status is a live, writable struct.
    let ok = unsafe { GetSystemPowerStatus(&mut status) }.is_ok();
    // 128 = "no system battery", 255 = unknown.
    ok && status.BatteryFlag != 128 && status.BatteryFlag != 255
}

/// Whether battery saver (energy saver on Windows 11) is on: decorative
/// animation should rest.
pub fn power_saver() -> bool {
    let mut status = SYSTEM_POWER_STATUS::default();
    // SAFETY: status is a live, writable struct.
    let ok = unsafe { GetSystemPowerStatus(&mut status) }.is_ok();
    ok && status.SystemStatusFlag == 1
}

/// Whether Windows is set to dark mode for apps.
pub fn system_dark() -> bool {
    reg_dword(
        HKEY_CURRENT_USER,
        r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
        "AppsUseLightTheme",
    ) == Some(0)
}

/// Whether the user turned off animations in Windows settings.
pub fn reduce_motion() -> bool {
    let mut enabled = BOOL(1);
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes a BOOL to the given pointer.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(std::ptr::from_mut(&mut enabled).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .is_ok();
    ok && !enabled.as_bool()
}

/// The primary display's work area (the screen minus the taskbar), in physical pixels:
/// `(left, top, width, height)`. Qt only reports its size, not where it starts.
pub fn work_area() -> Option<(i32, i32, i32, i32)> {
    let mut rect = windows::Win32::Foundation::RECT::default();
    // SAFETY: SPI_GETWORKAREA writes a RECT to the given pointer.
    unsafe {
        SystemParametersInfoW(
            windows::Win32::UI::WindowsAndMessaging::SPI_GETWORKAREA,
            0,
            Some(std::ptr::from_mut(&mut rect).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .ok()?;
    (rect.right > rect.left && rect.bottom > rect.top).then_some((
        rect.left,
        rect.top,
        rect.right - rect.left,
        rect.bottom - rect.top,
    ))
}

/// How far this PC's time zone is from UTC at `at_ms` (Unix milliseconds),
/// daylight saving included; 0 when Windows can't say.
pub fn local_offset_ms(at_ms: i64) -> i64 {
    use windows::Win32::{
        Foundation::{FILETIME, SYSTEMTIME},
        System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime},
    };
    // FILETIME counts 100 ns from 1601-01-01.
    let ticks = (at_ms.max(0) as u64).saturating_mul(10_000).saturating_add(116_444_736_000_000_000);
    let ft = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    // SAFETY: valid, initialized structs on the stack; no time zone means this PC's.
    let ok = unsafe {
        FileTimeToSystemTime(&ft, &mut utc).is_ok()
            && SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_ok()
    };
    if !ok {
        return 0;
    }
    let minutes = |t: &SYSTEMTIME| i64::from(t.wHour) * 60 + i64::from(t.wMinute);
    let mut diff = minutes(&local) - minutes(&utc);
    // Across midnight the calendar day differs too.
    if local.wDay != utc.wDay {
        diff += if (local.wYear, local.wMonth, local.wDay) > (utc.wYear, utc.wMonth, utc.wDay) {
            1_440
        } else {
            -1_440
        };
    }
    diff * 60_000
}

/// Free bytes on the drive holding `path`, if Windows can say.
pub fn free_space(path: &std::path::Path) -> Option<u64> {
    use windows::{Win32::Storage::FileSystem::GetDiskFreeSpaceExW, core::HSTRING};
    let mut free = 0u64;
    // SAFETY: a valid path string and an out pointer to a local.
    unsafe { GetDiskFreeSpaceExW(&HSTRING::from(path.as_os_str()), Some(&mut free), None, None) }
        .ok()
        .map(|()| free)
}

/// Fills `buf` from the system-preferred cryptographic RNG (CNG). False on failure.
pub fn secure_random(buf: &mut [u8]) -> bool {
    use windows::Win32::Security::Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom};
    // SAFETY: BCryptGenRandom writes exactly `buf.len()` bytes into `buf`.
    unsafe { BCryptGenRandom(None, buf, BCRYPT_USE_SYSTEM_PREFERRED_RNG) }.is_ok()
}

/// Runs `f` with COM available on this thread.
pub fn with_com<T>(f: impl FnOnce() -> T) -> T {
    // SAFETY: balanced below; if the thread already has COM in another mode
    // (RPC_E_CHANGED_MODE), it's usable as is and isn't uninitialized here.
    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
    let result = f();
    if initialized {
        // SAFETY: pairs with the successful CoInitializeEx above.
        unsafe { CoUninitialize() };
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_windows_version() {
        let version = os_version();
        let parts: Vec<&str> = version.split('.').collect();
        assert_eq!(parts.len(), 3, "{version}");
        // Any real build number; Windows 10 (19041+) runs the app too.
        assert!(parts[2].parse::<u32>().unwrap() >= 10240, "not a Windows 10 or 11 build: {version}");
    }
}
