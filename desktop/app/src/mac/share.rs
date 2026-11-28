// SPDX-License-Identifier: GPL-3.0-or-later
//! The macOS share picker (`NSSharingServicePicker`), for the Photos page's
//! Share button (#174): `cpp/mac_share.mm`.

#![allow(unsafe_code)]

use std::{
    ffi::{CString, c_char},
    os::unix::ffi::OsStrExt,
    path::PathBuf,
};

unsafe extern "C" {
    fn mac_share_files(paths: *const *const c_char, count: usize) -> bool;
}

/// Shows the share picker for `paths` over the app's window. Call it on the
/// UI thread.
pub fn show(paths: &[PathBuf]) -> Result<(), String> {
    let paths: Vec<CString> = paths.iter().filter_map(|p| CString::new(p.as_os_str().as_bytes()).ok()).collect();
    if paths.is_empty() {
        return Err("nothing to share".to_owned());
    }
    let pointers: Vec<*const c_char> = paths.iter().map(|p| p.as_ptr()).collect();
    // SAFETY: `pointers` holds `pointers.len()` valid NUL-terminated strings,
    // which `paths` keeps alive for the call; the function copies them.
    if unsafe { mac_share_files(pointers.as_ptr(), pointers.len()) } {
        Ok(())
    } else {
        Err("the share picker didn't open".to_owned())
    }
}
