// SPDX-License-Identifier: GPL-3.0-or-later
//! The Windows password, sealed so that only SYSTEM can read it (ADR 0009).
//!
//! Turning unlock on creates an RSA key in the machine's key store that only
//! SYSTEM and administrators may use, and publishes its public half. When the
//! user sets up a phone, the app seals the password to that public key before
//! the phone's key wraps it. At the lock screen the app hands the sealed blob to
//! the credential provider, which runs as SYSTEM inside LogonUI and is the only
//! thing that can open it. So the user's own processes, malware running as the
//! user included, never see the password after setting up, even with the phone's
//! key in hand.
//!
//! The blob is `SEAL_MAGIC`, a chunk count, then that many RSA-OAEP (SHA-256)
//! chunks of [`CHUNK`] bytes. Each chunk's OAEP label names the Windows user and
//! the chunk's place, so a blob made for one user, or chunks reordered, don't open.
//!
//! Shared by source: the credential provider opens, the app seals.

#![allow(unsafe_code, dead_code)]

use std::ffi::c_void;

use windows::{
    Win32::Security::Cryptography::{
        BCRYPT_ALG_HANDLE, BCRYPT_KEY_HANDLE, BCRYPT_OAEP_PADDING_INFO, BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS,
        BCRYPT_PAD_OAEP, BCRYPT_RSA_ALGORITHM, BCRYPT_RSAPUBLIC_BLOB, BCRYPT_SHA256_ALGORITHM,
        BCryptCloseAlgorithmProvider, BCryptDestroyKey, BCryptEncrypt, BCryptImportKeyPair,
        BCryptOpenAlgorithmProvider, MS_KEY_STORAGE_PROVIDER, NCRYPT_FLAGS, NCRYPT_HANDLE, NCRYPT_KEY_HANDLE,
        NCRYPT_MACHINE_KEY_FLAG, NCRYPT_PAD_OAEP_FLAG, NCRYPT_PROV_HANDLE, NCRYPT_SILENT_FLAG, NCryptDecrypt,
        NCryptFreeObject, NCryptOpenKey, NCryptOpenStorageProvider,
    },
    core::{HSTRING, PCWSTR},
};

/// The machine key's name in the Microsoft Software Key Storage Provider.
pub const KEY_NAME: &str = "NectarLink Unlock";
/// The key's size, in bits.
pub const KEY_BITS: u32 = 3072;
/// One RSA chunk: the key's size in bytes.
pub const CHUNK: usize = (KEY_BITS / 8) as usize;
/// The most an OAEP (SHA-256) chunk carries: the key's size less 2 hashes and 2.
pub const CHUNK_PLAIN: usize = CHUNK - 2 * 32 - 2;
/// Enough chunks for the longest Windows password (256 characters) in UTF-8.
pub const MAX_CHUNKS: usize = 3;
/// How a sealed password starts.
pub const SEAL_MAGIC: &[u8; 4] = b"NLS1";
/// The longest sealed password.
pub const MAX_SEALED: usize = SEAL_MAGIC.len() + 1 + MAX_CHUNKS * CHUNK;

/// The OAEP label for chunk `index` of `count`, for `user`.
fn label(user: &str, index: usize, count: usize) -> Vec<u8> {
    let mut label = b"NectarLink unlock\0".to_vec();
    label.extend_from_slice(user.as_bytes());
    label.extend_from_slice(&[0, index as u8, count as u8]);
    label
}

/// Fills `info` for an OAEP (SHA-256) operation with `label`, which must outlive it.
fn oaep(label: &[u8]) -> BCRYPT_OAEP_PADDING_INFO {
    BCRYPT_OAEP_PADDING_INFO {
        pszAlgId: BCRYPT_SHA256_ALGORITHM,
        pbLabel: label.as_ptr().cast_mut(),
        cbLabel: label.len() as u32,
    }
}

// ---- Sealing (the app, as the user) ----

struct Alg(BCRYPT_ALG_HANDLE);

impl Drop for Alg {
    fn drop(&mut self) {
        // SAFETY: opened by BCryptOpenAlgorithmProvider, closed once.
        let _ = unsafe { BCryptCloseAlgorithmProvider(self.0, 0) };
    }
}

struct PublicKey(BCRYPT_KEY_HANDLE);

impl Drop for PublicKey {
    fn drop(&mut self) {
        // SAFETY: made by BCryptImportKeyPair, destroyed once.
        let _ = unsafe { BCryptDestroyKey(self.0) };
    }
}

/// Seals `password` for `user` to the machine key's public half (`public`, a
/// `BCRYPT_RSAPUBLIC_BLOB`).
pub fn seal(public: &[u8], user: &str, password: &[u8]) -> Result<Vec<u8>, String> {
    if password.is_empty() || password.len() > MAX_CHUNKS * CHUNK_PLAIN {
        return Err("password too long to seal".into());
    }
    let mut alg = BCRYPT_ALG_HANDLE::default();
    // SAFETY: valid out pointer; closed by `Alg`.
    unsafe {
        BCryptOpenAlgorithmProvider(
            &mut alg,
            BCRYPT_RSA_ALGORITHM,
            PCWSTR::null(),
            BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS(0),
        )
    }
    .ok()
    .map_err(|e| e.message())?;
    let alg = Alg(alg);
    let mut key = BCRYPT_KEY_HANDLE::default();
    // SAFETY: `public` is read only during the call; destroyed by `PublicKey`.
    unsafe { BCryptImportKeyPair(alg.0, None, BCRYPT_RSAPUBLIC_BLOB, &mut key, public, 0) }
        .ok()
        .map_err(|e| e.message())?;
    let key = PublicKey(key);
    let chunks: Vec<&[u8]> = password.chunks(CHUNK_PLAIN).collect();
    let mut sealed = SEAL_MAGIC.to_vec();
    sealed.push(chunks.len() as u8);
    for (i, chunk) in chunks.iter().enumerate() {
        let label = label(user, i, chunks.len());
        let info = oaep(&label);
        let mut out = vec![0u8; CHUNK];
        let mut len = 0u32;
        // SAFETY: `info` and `label` outlive the call; `out` is CHUNK bytes.
        unsafe {
            BCryptEncrypt(
                key.0,
                Some(chunk),
                Some((&raw const info).cast::<c_void>()),
                None,
                Some(&mut out),
                &mut len,
                BCRYPT_PAD_OAEP,
            )
        }
        .ok()
        .map_err(|e| e.message())?;
        if len as usize != CHUNK {
            return Err("the machine key isn't the size it should be".into());
        }
        sealed.extend_from_slice(&out);
    }
    Ok(sealed)
}

// ---- Opening (the credential provider, as SYSTEM) ----

struct NHandle(NCRYPT_HANDLE);

impl Drop for NHandle {
    fn drop(&mut self) {
        // SAFETY: an NCrypt object owned here, freed once.
        let _ = unsafe { NCryptFreeObject(self.0) };
    }
}

/// Opens the machine key. Only SYSTEM and administrators may.
pub fn open_machine_key() -> Option<NCRYPT_KEY_HANDLE> {
    let mut provider = NCRYPT_PROV_HANDLE::default();
    // SAFETY: valid out pointer; freed below.
    unsafe { NCryptOpenStorageProvider(&mut provider, MS_KEY_STORAGE_PROVIDER, 0) }.ok()?;
    let provider = NHandle(NCRYPT_HANDLE(provider.0));
    let mut key = NCRYPT_KEY_HANDLE::default();
    // SAFETY: valid out pointer; the caller frees the key with `close_key`.
    unsafe {
        NCryptOpenKey(
            NCRYPT_PROV_HANDLE(provider.0.0),
            &mut key,
            &HSTRING::from(KEY_NAME),
            Default::default(),
            NCRYPT_MACHINE_KEY_FLAG | NCRYPT_SILENT_FLAG,
        )
    }
    .ok()?;
    Some(key)
}

/// Frees a key from [`open_machine_key`].
pub fn close_key(key: NCRYPT_KEY_HANDLE) {
    drop(NHandle(NCRYPT_HANDLE(key.0)));
}

/// Opens `sealed` for `user` with the machine key's private half. `None` when it
/// isn't a sealed password, was sealed for someone else, or was changed.
pub fn open(key: NCRYPT_KEY_HANDLE, user: &str, sealed: &[u8]) -> Option<Vec<u8>> {
    let body = sealed.strip_prefix(SEAL_MAGIC.as_slice())?;
    let (&count, chunks) = body.split_first()?;
    let count = count as usize;
    if count == 0 || count > MAX_CHUNKS || chunks.len() != count * CHUNK {
        return None;
    }
    let mut password = Vec::with_capacity(count * CHUNK_PLAIN);
    for (i, chunk) in chunks.chunks(CHUNK).enumerate() {
        let label = label(user, i, count);
        let info = oaep(&label);
        let mut out = vec![0u8; CHUNK];
        let mut len = 0u32;
        // SAFETY: `info` and `label` outlive the call; `out` is CHUNK bytes.
        let opened = unsafe {
            NCryptDecrypt(
                key,
                Some(chunk),
                Some((&raw const info).cast::<c_void>()),
                Some(&mut out),
                &mut len,
                NCRYPT_PAD_OAEP_FLAG | NCRYPT_FLAGS(NCRYPT_SILENT_FLAG.0),
            )
        };
        if opened.is_err() || len as usize > CHUNK_PLAIN {
            out.fill(0);
            password.fill(0);
            return None;
        }
        password.extend_from_slice(&out[..len as usize]);
        out.fill(0);
    }
    Some(password)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Security::Cryptography::{
        BCRYPT_RSA_ALGORITHM as RSA, NCRYPT_LENGTH_PROPERTY, NCryptCreatePersistedKey, NCryptExportKey,
        NCryptFinalizeKey, NCryptSetProperty,
    };

    /// A key that lives only in this process, like the machine key but for tests.
    fn ephemeral_key() -> (NCRYPT_KEY_HANDLE, Vec<u8>) {
        let mut provider = NCRYPT_PROV_HANDLE::default();
        unsafe { NCryptOpenStorageProvider(&mut provider, MS_KEY_STORAGE_PROVIDER, 0) }.unwrap();
        let mut key = NCRYPT_KEY_HANDLE::default();
        unsafe {
            NCryptCreatePersistedKey(
                provider,
                &mut key,
                RSA,
                PCWSTR::null(),
                Default::default(),
                NCRYPT_FLAGS(0),
            )
        }
        .unwrap();
        unsafe {
            NCryptSetProperty(
                NCRYPT_HANDLE(key.0),
                NCRYPT_LENGTH_PROPERTY,
                &KEY_BITS.to_le_bytes(),
                NCRYPT_FLAGS(0),
            )
        }
        .unwrap();
        unsafe { NCryptFinalizeKey(key, NCRYPT_FLAGS(0)) }.unwrap();
        let mut len = 0u32;
        unsafe { NCryptExportKey(key, None, BCRYPT_RSAPUBLIC_BLOB, None, None, &mut len, NCRYPT_FLAGS(0)) }
            .unwrap();
        let mut public = vec![0u8; len as usize];
        unsafe {
            NCryptExportKey(
                key,
                None,
                BCRYPT_RSAPUBLIC_BLOB,
                None,
                Some(&mut public),
                &mut len,
                NCRYPT_FLAGS(0),
            )
        }
        .unwrap();
        public.truncate(len as usize);
        let _ = unsafe { NCryptFreeObject(NCRYPT_HANDLE(provider.0)) };
        (key, public)
    }

    #[test]
    fn a_sealed_password_opens_only_for_its_user_and_unchanged() {
        let (key, public) = ephemeral_key();
        let user = "S-1-5-21-1-2-3-1001";
        for password in ["pw".as_bytes(), "pässwörd ✓".as_bytes(), "ü".repeat(256).as_bytes()] {
            let sealed = seal(&public, user, password).unwrap();
            assert!(sealed.len() <= MAX_SEALED);
            assert!(!sealed.windows(password.len()).any(|w| w == password), "not in the clear");
            assert_eq!(open(key, user, &sealed).as_deref(), Some(password));
            assert_eq!(open(key, "S-1-5-21-1-2-3-1002", &sealed), None, "another user");
            let mut changed = sealed.clone();
            *changed.last_mut().unwrap() ^= 1;
            assert_eq!(open(key, user, &changed), None, "changed");
        }
        // Two chunks swapped don't open either.
        let long = "x".repeat(CHUNK_PLAIN + 5);
        let sealed = seal(&public, user, long.as_bytes()).unwrap();
        let mut swapped = sealed[..5].to_vec();
        swapped.extend_from_slice(&sealed[5 + CHUNK..]);
        swapped.extend_from_slice(&sealed[5..5 + CHUNK]);
        assert_eq!(open(key, user, &swapped), None, "reordered");
        assert_eq!(open(key, user, b"NLS1"), None);
        assert_eq!(open(key, user, b"not sealed"), None);
        assert!(seal(&public, user, b"").is_err());
        assert!(seal(&public, user, &vec![b'a'; MAX_CHUNKS * CHUNK_PLAIN + 1]).is_err());
        close_key(key);
    }
}
