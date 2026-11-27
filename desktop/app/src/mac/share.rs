// SPDX-License-Identifier: GPL-3.0-or-later
//! The Mac's share sheet for files (`cpp/mac_share.mm`).

use std::path::PathBuf;

/// Opens the share sheet for `paths`, which have to stay on disk until the app
/// that's picked in it has read them. Has to run on Qt's thread (the main one).
pub fn show(paths: &[PathBuf]) -> Result<(), String> {
    use std::ffi::{CString, c_char, c_int};

    #[allow(unsafe_code)] // The shim in `cpp/mac_share.mm`.
    unsafe extern "C" {
        fn nl_mac_share_files(paths: *const *const c_char, count: c_int) -> c_int;
    }

    let names: Vec<CString> =
        paths.iter().filter_map(|p| CString::new(p.to_string_lossy().into_owned()).ok()).collect();
    if names.is_empty() {
        return Err("nothing to share".to_owned());
    }
    let pointers: Vec<*const c_char> = names.iter().map(|n| n.as_ptr()).collect();
    let count = c_int::try_from(pointers.len()).map_err(|e| e.to_string())?;
    // SAFETY: the pointers are NUL-terminated strings that outlive the call, and the
    // shim only reads them; it runs on the main thread, as AppKit needs.
    #[allow(unsafe_code)]
    let shown = unsafe { nl_mac_share_files(pointers.as_ptr(), count) };
    if shown == 1 { Ok(()) } else { Err("no window to open it from".to_owned()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_window_there_is_no_sheet() {
        // The shim links, and says no rather than crash when no window is up.
        assert!(show(&[PathBuf::from("/tmp/Picture.jpg")]).is_err());
        assert!(show(&[]).is_err());
    }
}
