// SPDX-License-Identifier: GPL-3.0-or-later
//! Windows integration: system facts, single instance, tray, power and
//! network notifications, ringing. Window chrome lives in C++
//! (`cpp/native_window.cpp`) because it hooks Qt's native event handling.

#![allow(unsafe_code)]

pub mod autostart;
pub mod clipboard;
pub mod doctor;
pub mod http;
pub mod icon;
pub mod image;
pub mod media_sessions;
pub mod net;
pub mod shell;
pub mod shortcut;
pub mod single_instance;
pub mod smtc;
pub mod sound;
pub mod toast;
pub mod tray;
pub mod wallpaper;

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

fn reg_string(root: HKEY, key: &str, value: &str) -> Option<String> {
    let mut buf = [0u16; 64];
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
        assert!(parts[2].parse::<u32>().unwrap() >= 22000, "Nectarlink targets Windows 11: {version}");
    }
}
