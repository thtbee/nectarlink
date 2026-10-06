// SPDX-License-Identifier: GPL-3.0-or-later
//! Starting Nectarlink when the user signs in, so phones can reach this PC
//! without opening it first: a value under the user's `Run` key that starts
//! it in the tray.

use std::path::Path;

use windows::{
    Win32::{
        Foundation::ERROR_FILE_NOT_FOUND,
        System::Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegDeleteValueW, RegOpenKeyExW,
            RegSetValueExW,
        },
    },
    core::HSTRING,
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE: &str = "Nectarlink";

/// The command the `Run` value holds.
pub fn command(exe: &Path) -> String {
    format!("\"{}\" --minimized", exe.display())
}

/// Starts this copy of the app at sign-in, or stops doing so.
pub fn set(enabled: bool, exe: &Path) -> windows::core::Result<()> {
    let mut key = HKEY::default();
    // SAFETY: opens a key under HKCU with a valid path; closed below.
    unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(RUN_KEY), None, KEY_SET_VALUE, &mut key) }
        .ok()?;
    let result = if enabled {
        let wide: Vec<u16> = command(exe).encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: the data is a null-terminated UTF-16 string of the given size.
        unsafe {
            RegSetValueExW(
                key,
                &HSTRING::from(VALUE),
                None,
                REG_SZ,
                Some(std::slice::from_raw_parts(wide.as_ptr().cast::<u8>(), wide.len() * 2)),
            )
        }
        .ok()
    } else {
        // SAFETY: deletes one value of the open key.
        let deleted = unsafe { RegDeleteValueW(key, &HSTRING::from(VALUE)) };
        if deleted == ERROR_FILE_NOT_FOUND { Ok(()) } else { deleted.ok() }
    };
    // SAFETY: the key was opened above.
    unsafe {
        let _ = RegCloseKey(key);
    }
    result
}

/// Whether an executable is an installed copy (under Program Files), as
/// opposed to a build being run from its folder.
pub fn is_installed(exe: &Path) -> bool {
    let exe = exe.to_string_lossy().to_lowercase();
    ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"]
        .iter()
        .filter_map(|var| std::env::var(var).ok())
        .any(|dir| exe.starts_with(&format!("{}\\", dir.to_lowercase().trim_end_matches('\\'))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_minimized_from_the_exe() {
        assert_eq!(
            command(Path::new(r"C:\Program Files\Nectarlink\nectarlink-desktop.exe")),
            r#""C:\Program Files\Nectarlink\nectarlink-desktop.exe" --minimized"#
        );
    }

    #[test]
    fn builds_are_not_installed_copies() {
        assert!(!is_installed(Path::new(
            r"C:\Users\me\src\nectarlink\target\release\nectarlink-desktop.exe"
        )));
        if let Ok(dir) = std::env::var("ProgramFiles") {
            assert!(is_installed(&Path::new(&dir).join("Nectarlink").join("nectarlink-desktop.exe")));
        }
    }
}
