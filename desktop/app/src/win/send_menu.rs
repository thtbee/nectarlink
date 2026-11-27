// SPDX-License-Identifier: GPL-3.0-or-later
//! "Send with NectarLink" in the first level of File Explorer's right-click
//! menu (#451): `nectarlink_shellmenu.dll`, declared in the sparse package,
//! shows the phones this writes to the registry. Kept with the "Send to"
//! shortcuts, by the same preference.

use windows::{
    Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, WIN32_ERROR},
        System::Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_MULTI_SZ, REG_OPTION_NON_VOLATILE, REG_SZ,
            REG_VALUE_TYPE, RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW,
        },
    },
    core::HSTRING,
};

use nectarlink_core::DeviceId;

use crate::i18n::tr;

/// What the menu shows, shared with the DLL by source (it reads, this writes).
#[path = "../../../shellmenu/src/menu.rs"]
#[allow(dead_code)]
mod menu;

/// Shows `phones` in the menu; none hides it.
pub fn write(phones: &[(DeviceId, String)]) -> windows::core::Result<()> {
    if phones.is_empty() {
        return clear();
    }
    let phones: Vec<menu::Phone> = phones
        .iter()
        .map(|(id, name)| menu::Phone {
            id: id.to_string(),
            name: name.clone(),
            title: tr!("SendTo", "Send to %1 with NectarLink", name),
        })
        .collect();
    let mut key = HKEY::default();
    // SAFETY: creates or opens a key under HKCU with a valid path; closed below.
    unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &HSTRING::from(menu::KEY),
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
    }
    .ok()?;
    let title = tr!("SendTo", "Send with NectarLink");
    let result = set(key, menu::TITLE, REG_SZ, &[title])
        .and_then(|()| set(key, menu::PHONES, REG_MULTI_SZ, &menu::encode(&phones)));
    // SAFETY: the key was opened above.
    unsafe {
        let _ = RegCloseKey(key);
    }
    result
}

/// Takes the entry out of the menu.
pub fn clear() -> windows::core::Result<()> {
    // SAFETY: deletes a key under HKCU with a valid path.
    let deleted = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(menu::KEY)) };
    if deleted == ERROR_FILE_NOT_FOUND { Ok(()) } else { deleted.ok() }
}

/// Sets a string value (`REG_SZ`, the first of `strings`) or a multi-string one.
fn set(key: HKEY, name: &str, kind: REG_VALUE_TYPE, strings: &[String]) -> windows::core::Result<()> {
    let wide = utf16(kind, strings);
    // SAFETY: the data is the value's UTF-16, of the given size.
    let status: WIN32_ERROR = unsafe {
        RegSetValueExW(
            key,
            &HSTRING::from(name),
            None,
            kind,
            Some(std::slice::from_raw_parts(wide.as_ptr().cast::<u8>(), wide.len() * 2)),
        )
    };
    status.ok()
}

/// The UTF-16 of a value: each string ends with a null, a multi-string with
/// one more.
fn utf16(kind: REG_VALUE_TYPE, strings: &[String]) -> Vec<u16> {
    let strings = if kind == REG_MULTI_SZ { strings } else { &strings[..strings.len().min(1)] };
    let mut wide: Vec<u16> =
        strings.iter().flat_map(|s| s.encode_utf16().chain(std::iter::once(0))).collect();
    if kind == REG_MULTI_SZ {
        wide.push(0);
    }
    wide
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_end_as_the_registry_expects() {
        assert_eq!(utf16(REG_SZ, &["ab".into(), "cd".into()]), [97, 98, 0]);
        assert_eq!(utf16(REG_MULTI_SZ, &["ab".into(), "c".into()]), [97, 98, 0, 99, 0, 0]);
    }
}
