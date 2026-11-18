// SPDX-License-Identifier: GPL-3.0-or-later
//! The credential LogonUI hands to Windows to unlock a session: a
//! `KERB_INTERACTIVE_UNLOCK_LOGON` packed into one buffer, its strings right after
//! the struct and its string pointers turned into offsets from the start, as
//! Microsoft's credential provider samples (`KerbInteractiveUnlockLogonPack`) do.

#![allow(unsafe_code)]

use std::mem::size_of;

use windows::{
    Win32::{
        Foundation::{HANDLE, LUID},
        Security::Authentication::Identity::{
            KERB_INTERACTIVE_LOGON, KERB_INTERACTIVE_UNLOCK_LOGON, KerbWorkstationUnlockLogon, LSA_STRING,
            LSA_UNICODE_STRING, LsaConnectUntrusted, LsaDeregisterLogonProcess,
            LsaLookupAuthenticationPackage,
        },
    },
    core::{PSTR, PWSTR},
};

/// Why a credential couldn't be packed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackError {
    /// A name or the password is longer than a Windows string can be.
    TooLong,
}

/// Packs `domain\user` and `password` (UTF-16, no terminator) for unlocking a
/// workstation. The result holds the password: wipe it after use.
pub fn pack_unlock(domain: &str, user: &str, password: &[u16]) -> Result<Vec<u8>, PackError> {
    let domain: Vec<u16> = domain.encode_utf16().collect();
    let user: Vec<u16> = user.encode_utf16().collect();
    let header = size_of::<KERB_INTERACTIVE_UNLOCK_LOGON>();
    let text = 2 * (domain.len() + user.len() + password.len());
    let mut buffer = vec![0u8; header + text];
    let mut offset = header;
    let mut place = |s: &[u16], buffer: &mut [u8]| -> Result<LSA_UNICODE_STRING, PackError> {
        let bytes = s.len() * 2;
        let length = u16::try_from(bytes).map_err(|_| PackError::TooLong)?;
        for (i, unit) in s.iter().enumerate() {
            buffer[offset + 2 * i..offset + 2 * i + 2].copy_from_slice(&unit.to_le_bytes());
        }
        // A packed string's "pointer" is its offset from the start of the buffer.
        let string =
            LSA_UNICODE_STRING { Length: length, MaximumLength: length, Buffer: PWSTR(offset as *mut u16) };
        offset += bytes;
        Ok(string)
    };
    let logon = KERB_INTERACTIVE_UNLOCK_LOGON {
        Logon: KERB_INTERACTIVE_LOGON {
            MessageType: KerbWorkstationUnlockLogon,
            LogonDomainName: place(&domain, &mut buffer)?,
            UserName: place(&user, &mut buffer)?,
            Password: place(password, &mut buffer)?,
        },
        LogonId: LUID::default(),
    };
    // SAFETY: `logon` is a plain `repr(C)` value of exactly `header` bytes, and
    // `buffer` has at least that many.
    unsafe {
        std::ptr::copy_nonoverlapping((&raw const logon).cast::<u8>(), buffer.as_mut_ptr(), header);
    }
    Ok(buffer)
}

/// The ID Windows gives the Negotiate package, which LogonUI expects unlock
/// credentials for.
pub fn negotiate_package() -> windows::core::Result<u32> {
    let mut lsa = HANDLE::default();
    // SAFETY: `lsa` is a valid out pointer; the handle is released below.
    unsafe { LsaConnectUntrusted(&mut lsa) }.ok()?;
    let name = b"Negotiate";
    let lsa_name = LSA_STRING {
        Length: name.len() as u16,
        MaximumLength: name.len() as u16,
        Buffer: PSTR(name.as_ptr().cast_mut()),
    };
    let mut package = 0u32;
    // SAFETY: `lsa` is connected; `lsa_name` points at `name`, which outlives the call.
    let looked_up = unsafe { LsaLookupAuthenticationPackage(lsa, &lsa_name, &mut package) };
    // SAFETY: `lsa` came from LsaConnectUntrusted above.
    let _ = unsafe { LsaDeregisterLogonProcess(lsa) };
    looked_up.ok()?;
    Ok(package)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_at(buffer: &[u8], s: &LSA_UNICODE_STRING) -> String {
        let start = s.Buffer.0 as usize;
        let units: Vec<u16> = buffer[start..start + usize::from(s.Length)]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        String::from_utf16(&units).unwrap()
    }

    #[test]
    fn strings_follow_the_struct_at_the_offsets_it_names() {
        let password: Vec<u16> = "pässwörd 🐝".encode_utf16().collect();
        let packed = pack_unlock("MicrosoftAccount", "me@example.com", &password).unwrap();
        let header = size_of::<KERB_INTERACTIVE_UNLOCK_LOGON>();
        // SAFETY: `packed` starts with a whole KERB_INTERACTIVE_UNLOCK_LOGON.
        let logon: KERB_INTERACTIVE_UNLOCK_LOGON =
            unsafe { std::ptr::read_unaligned(packed.as_ptr().cast::<KERB_INTERACTIVE_UNLOCK_LOGON>()) };
        assert_eq!(logon.Logon.MessageType, KerbWorkstationUnlockLogon);
        assert_eq!(logon.Logon.LogonDomainName.Buffer.0 as usize, header, "strings start right after it");
        assert_eq!(text_at(&packed, &logon.Logon.LogonDomainName), "MicrosoftAccount");
        assert_eq!(text_at(&packed, &logon.Logon.UserName), "me@example.com");
        assert_eq!(text_at(&packed, &logon.Logon.Password), "pässwörd 🐝");
        assert_eq!(logon.Logon.Password.MaximumLength, logon.Logon.Password.Length);
        assert_eq!(packed.len(), header + 2 * (16 + 14 + password.len()));
    }

    #[test]
    fn a_string_too_long_for_windows_is_refused() {
        let long = vec![b'a' as u16; 40_000];
        assert_eq!(pack_unlock(".", "bee", &long), Err(PackError::TooLong));
    }
}
