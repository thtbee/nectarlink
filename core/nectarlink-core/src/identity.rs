// SPDX-License-Identifier: MPL-2.0
//! The device key: created on first launch, stored encrypted at rest.

use std::{
    fs,
    io::{self, Write},
    path::Path,
    sync::Arc,
};

use iroh::SecretKey;

use crate::{Error, Result};

const FILE_NAME: &str = "identity.key";
const MAGIC: &[u8; 4] = b"NLK1";

/// Protects the device key at rest. Each platform provides one: DPAPI on
/// Windows, Android Keystore wrapping on Android (injected by the app).
pub trait KeyProtector: Send + Sync {
    /// A stable one-byte identifier written into the key file.
    fn id(&self) -> u8;
    fn protect(&self, plaintext: &[u8]) -> io::Result<Vec<u8>>;
    fn unprotect(&self, ciphertext: &[u8]) -> io::Result<Vec<u8>>;
}

/// No protection. Only used where no platform protector exists (tests,
/// headless tools); a warning is logged when it is used for a real key.
#[derive(Debug, Default, Clone, Copy)]
pub struct PlainKeyProtector;

impl KeyProtector for PlainKeyProtector {
    fn id(&self) -> u8 {
        0
    }
    fn protect(&self, plaintext: &[u8]) -> io::Result<Vec<u8>> {
        Ok(plaintext.to_vec())
    }
    fn unprotect(&self, ciphertext: &[u8]) -> io::Result<Vec<u8>> {
        Ok(ciphertext.to_vec())
    }
}

/// The best protector available on this platform.
pub fn default_protector() -> Arc<dyn KeyProtector> {
    #[cfg(windows)]
    {
        Arc::new(dpapi::DpapiKeyProtector)
    }
    #[cfg(not(windows))]
    {
        Arc::new(PlainKeyProtector)
    }
}

/// Loads the device key from `dir`, creating and saving a new one on first
/// launch.
pub(crate) fn load_or_create(dir: &Path, protector: &dyn KeyProtector) -> Result<SecretKey> {
    let path = dir.join(FILE_NAME);
    match fs::read(&path) {
        Ok(bytes) => {
            let (key, legacy) = decode(&bytes, protector)?;
            if legacy {
                // Stored without protection by an older build: seal it now.
                write_atomically(&path, &encode(&key, protector)?)?;
            }
            Ok(key)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let key = SecretKey::generate();
            if protector.id() == 0 {
                tracing::warn!("device key is stored without platform protection");
            }
            write_atomically(&path, &encode(&key, protector)?)?;
            tracing::info!(device = %crate::device_id(&key.public()).short(), "created device identity");
            Ok(key)
        }
        Err(e) => Err(e.into()),
    }
}

fn encode(key: &SecretKey, protector: &dyn KeyProtector) -> Result<Vec<u8>> {
    let sealed = protector.protect(&key.to_bytes())?;
    let mut out = Vec::with_capacity(MAGIC.len() + 1 + sealed.len());
    out.extend_from_slice(MAGIC);
    out.push(protector.id());
    out.extend_from_slice(&sealed);
    Ok(out)
}

/// Returns the key, and whether the file held it without protection while
/// `protector` can now seal it (the caller re-seals it).
fn decode(bytes: &[u8], protector: &dyn KeyProtector) -> Result<(SecretKey, bool)> {
    let corrupt = || Error::Storage("the device key file is corrupt".into());
    let rest = bytes.strip_prefix(MAGIC.as_slice()).ok_or_else(corrupt)?;
    let (&method, sealed) = rest.split_first().ok_or_else(corrupt)?;
    let legacy = method == PlainKeyProtector.id() && protector.id() != PlainKeyProtector.id();
    if method != protector.id() && !legacy {
        return Err(Error::Storage(format!(
            "the device key is protected with method {method}, but this build uses {}",
            protector.id()
        )));
    }
    let reader: &dyn KeyProtector = if legacy { &PlainKeyProtector } else { protector };
    let plain =
        reader.unprotect(sealed).map_err(|e| Error::Storage(format!("cannot unlock device key: {e}")))?;
    let bytes: [u8; 32] = plain.as_slice().try_into().map_err(|_| corrupt())?;
    Ok((SecretKey::from_bytes(&bytes), legacy))
}

/// Writes via a temporary file and rename, so a crash never leaves a
/// half-written key behind.
fn write_atomically(path: &Path, data: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        // A fresh file, private from the start: a leftover (or a link) at
        // the temporary path is removed rather than written through.
        let _ = fs::remove_file(&tmp);
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(data)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod dpapi {
    //! Windows Data Protection API: encrypts with a key tied to the current
    //! Windows user account.

    use std::{io, ptr, slice};

    use windows::Win32::{
        Foundation::{HLOCAL, LocalFree},
        Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
        },
    };

    use super::KeyProtector;

    /// Extra entropy so other apps of the same user can't trivially decrypt
    /// our blob with a bare `CryptUnprotectData` call.
    const ENTROPY: &[u8] = b"nectarlink/identity/v1";

    #[derive(Debug, Default, Clone, Copy)]
    pub struct DpapiKeyProtector;

    fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 }
    }

    /// Copies a DPAPI output blob into a Vec and frees the original.
    ///
    /// # Safety
    /// `out` must have been filled by a successful DPAPI call.
    unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        // SAFETY: DPAPI returns a valid buffer of `cbData` bytes allocated with LocalAlloc.
        let data = unsafe { slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
        // SAFETY: the buffer was allocated by DPAPI with LocalAlloc and is freed exactly once.
        unsafe { LocalFree(Some(HLOCAL(out.pbData.cast()))) };
        data
    }

    impl KeyProtector for DpapiKeyProtector {
        fn id(&self) -> u8 {
            1
        }

        fn protect(&self, plaintext: &[u8]) -> io::Result<Vec<u8>> {
            let input = blob(plaintext);
            let entropy = blob(ENTROPY);
            let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: ptr::null_mut() };
            // SAFETY: all pointers reference live buffers for the duration of the call.
            unsafe {
                CryptProtectData(
                    &input,
                    windows::core::w!("NectarLink device key"),
                    Some(&entropy),
                    None,
                    None,
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut out,
                )
            }
            .map_err(io::Error::other)?;
            // SAFETY: the call succeeded, so `out` holds a DPAPI-allocated buffer.
            Ok(unsafe { take(out) })
        }

        fn unprotect(&self, ciphertext: &[u8]) -> io::Result<Vec<u8>> {
            let input = blob(ciphertext);
            let entropy = blob(ENTROPY);
            let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: ptr::null_mut() };
            // SAFETY: all pointers reference live buffers for the duration of the call.
            unsafe {
                CryptUnprotectData(
                    &input,
                    None,
                    Some(&entropy),
                    None,
                    None,
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut out,
                )
            }
            .map_err(io::Error::other)?;
            // SAFETY: the call succeeded, so `out` holds a DPAPI-allocated buffer.
            Ok(unsafe { take(out) })
        }
    }
}

/// The install ID a device reports to its peers ([`DeviceInfo::install_id`]): a salted
/// BLAKE3 hash of a platform ID that survives app reinstalls (Android's `ANDROID_ID`,
/// Windows' `MachineGuid`). The raw ID never leaves the device; the salt keeps the hash
/// specific to NectarLink, so it can't be matched against other apps' identifiers.
///
/// [`DeviceInfo::install_id`]: nectarlink_protocol::messages::DeviceInfo::install_id
pub fn install_id_from(platform_id: &str) -> Option<String> {
    let raw = platform_id.trim();
    if raw.is_empty() {
        return None;
    }
    let mut hasher = blake3::Hasher::new_derive_key("nectarlink 2026 install id v1");
    hasher.update(raw.as_bytes());
    let hex = hasher.finalize().to_hex();
    Some(hex[..nectarlink_protocol::messages::INSTALL_ID_LEN].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_then_reloads_the_same_key() {
        let dir = tempfile::tempdir().unwrap();
        let protector = default_protector();
        let first = load_or_create(dir.path(), protector.as_ref()).unwrap();
        let second = load_or_create(dir.path(), protector.as_ref()).unwrap();
        assert_eq!(first.public(), second.public());
    }

    #[test]
    fn key_file_does_not_contain_the_plain_key_when_protected() {
        let dir = tempfile::tempdir().unwrap();
        let protector = default_protector();
        let key = load_or_create(dir.path(), protector.as_ref()).unwrap();
        let file = fs::read(dir.path().join(FILE_NAME)).unwrap();
        if protector.id() != 0 {
            let raw = key.to_bytes();
            assert!(!file.windows(raw.len()).any(|w| w == raw), "plaintext key found on disk");
        }
    }

    #[test]
    fn rejects_corrupt_and_mismatched_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(FILE_NAME), b"garbage").unwrap();
        assert!(matches!(load_or_create(dir.path(), &PlainKeyProtector), Err(Error::Storage(_))));

        let mut wrong_method = MAGIC.to_vec();
        wrong_method.push(42);
        wrong_method.extend_from_slice(&[0; 32]);
        fs::write(dir.path().join(FILE_NAME), wrong_method).unwrap();
        assert!(matches!(load_or_create(dir.path(), &PlainKeyProtector), Err(Error::Storage(_))));
    }

    /// Seals by XOR with a non-zero byte, under a method id other than 0.
    struct SealingProtector;

    impl KeyProtector for SealingProtector {
        fn id(&self) -> u8 {
            7
        }
        fn protect(&self, plaintext: &[u8]) -> io::Result<Vec<u8>> {
            Ok(plaintext.iter().map(|b| b ^ 0x5a).collect())
        }
        fn unprotect(&self, ciphertext: &[u8]) -> io::Result<Vec<u8>> {
            Ok(ciphertext.iter().map(|b| b ^ 0x5a).collect())
        }
    }

    #[test]
    fn an_unprotected_key_is_sealed_when_a_protector_becomes_available() {
        let dir = tempfile::tempdir().unwrap();
        let before = load_or_create(dir.path(), &PlainKeyProtector).unwrap();
        let path = dir.path().join(FILE_NAME);
        assert_eq!(fs::read(&path).unwrap()[MAGIC.len()], 0);

        let after = load_or_create(dir.path(), &SealingProtector).unwrap();
        assert_eq!(before.public(), after.public(), "same identity after sealing");
        let file = fs::read(&path).unwrap();
        assert_eq!(file[MAGIC.len()], 7, "re-sealed under the new method");
        assert!(!file.windows(32).any(|w| w == before.to_bytes()), "plain key no longer on disk");
        assert_eq!(load_or_create(dir.path(), &SealingProtector).unwrap().public(), before.public());
    }

    #[test]
    fn a_sealed_key_is_not_opened_without_its_protector() {
        let dir = tempfile::tempdir().unwrap();
        load_or_create(dir.path(), &SealingProtector).unwrap();
        // Nothing is overwritten when the protector is gone: the error says why.
        let before = fs::read(dir.path().join(FILE_NAME)).unwrap();
        assert!(matches!(load_or_create(dir.path(), &PlainKeyProtector), Err(Error::Storage(_))));
        assert_eq!(fs::read(dir.path().join(FILE_NAME)).unwrap(), before);
    }

    #[test]
    fn install_ids_are_stable_salted_and_well_formed() {
        let a = install_id_from("9774d56d682e549c").unwrap();
        assert_eq!(a, install_id_from(" 9774d56d682e549c ").unwrap(), "stable for the same device");
        assert_ne!(a, install_id_from("9774d56d682e549d").unwrap(), "differs per device");
        assert!(nectarlink_protocol::messages::is_valid_install_id(&a));
        assert!(!a.contains("9774d56d682e549c"), "the raw ID never appears");
        assert_eq!(install_id_from("  "), None);
    }

    #[cfg(unix)]
    #[test]
    fn key_file_has_0600_permissions_on_unix() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let protector = default_protector();
        load_or_create(dir.path(), protector.as_ref()).unwrap();
        let meta = fs::metadata(dir.path().join(FILE_NAME)).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    }
}
