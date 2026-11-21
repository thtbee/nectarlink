// SPDX-License-Identifier: GPL-3.0-or-later
//! Who is signed in to the session LogonUI is showing. Since Windows 10 the
//! lock screen usually asks providers for `CPUS_LOGON` (sign-in and unlock were
//! merged), so this is how the provider keeps to unlocking: it offers a tile
//! only to the account already signed in to this session. At the first
//! sign-in after a restart nobody is, so there's no tile.

#![allow(unsafe_code)]

use windows::{
    Win32::{
        Foundation::{HLOCAL, LocalFree},
        Security::{Authorization::ConvertSidToStringSidW, LookupAccountNameW, PSID, SID_NAME_USE},
        System::RemoteDesktop::{
            WTS_CURRENT_SERVER_HANDLE, WTS_CURRENT_SESSION, WTS_INFO_CLASS, WTSDomainName, WTSFreeMemory,
            WTSQuerySessionInformationW, WTSUserName,
        },
    },
    core::{HSTRING, PCWSTR, PWSTR},
};

fn session_string(class: WTS_INFO_CLASS) -> Option<String> {
    let mut buf = PWSTR::null();
    let mut len = 0u32;
    // SAFETY: Windows allocates `buf`, freed with WTSFreeMemory below.
    unsafe {
        WTSQuerySessionInformationW(
            Some(WTS_CURRENT_SERVER_HANDLE),
            WTS_CURRENT_SESSION,
            class,
            &mut buf,
            &mut len,
        )
    }
    .ok()?;
    // SAFETY: a NUL-terminated string Windows returned.
    let s = unsafe { buf.to_string() }.ok();
    // SAFETY: allocated by WTSQuerySessionInformationW.
    unsafe { WTSFreeMemory(buf.0.cast()) };
    s.filter(|s| !s.is_empty())
}

/// The security identifier of the account signed in to this session, if any.
pub fn signed_in_sid() -> Option<String> {
    let user = session_string(WTSUserName)?;
    let account = match session_string(WTSDomainName) {
        Some(domain) => format!(r"{domain}\{user}"),
        None => user,
    };
    let account = HSTRING::from(account);
    let mut sid = vec![0u8; 256];
    let mut sid_len = sid.len() as u32;
    let mut domain = [0u16; 256];
    let mut domain_len = domain.len() as u32;
    let mut kind = SID_NAME_USE::default();
    // SAFETY: the buffers and their lengths match.
    unsafe {
        LookupAccountNameW(
            PCWSTR::null(),
            &account,
            Some(PSID(sid.as_mut_ptr().cast())),
            &mut sid_len,
            Some(PWSTR(domain.as_mut_ptr())),
            &mut domain_len,
            &mut kind,
        )
    }
    .ok()?;
    let mut text = PWSTR::null();
    // SAFETY: `sid` holds the SID LookupAccountNameW wrote; `text` is freed below.
    unsafe { ConvertSidToStringSidW(PSID(sid.as_mut_ptr().cast()), &mut text) }.ok()?;
    // SAFETY: a valid string from ConvertSidToStringSidW.
    let result = unsafe { text.to_string() }.ok();
    // SAFETY: allocated by ConvertSidToStringSidW with LocalAlloc.
    unsafe { LocalFree(Some(HLOCAL(text.0.cast()))) };
    result
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_test_runners_session_has_its_user() {
        let sid = super::signed_in_sid().expect("tests run in a signed-in session");
        assert!(crate::wire::user_ok(&sid), "{sid}");
    }
}
