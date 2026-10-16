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
    #[cfg(target_os = "linux")]
    {
        if keyring::SecretServiceKeyProtector::is_available() {
            Arc::new(keyring::SecretServiceKeyProtector)
        } else {
            Arc::new(PlainKeyProtector)
        }
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Arc::new(PlainKeyProtector)
    }
}

#[cfg(target_os = "linux")]
pub use keyring::SecretServiceKeyProtector;

/// Loads the device key from `dir`, creating and saving a new one on first
/// launch.
pub(crate) fn load_or_create(dir: &Path, protector: &dyn KeyProtector) -> Result<SecretKey> {
    let path = dir.join(FILE_NAME);
    match fs::read(&path) {
        Ok(bytes) => decode(&bytes, protector),
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

pub(crate) fn unprotect_with_known(method: u8, sealed: &[u8]) -> Option<io::Result<Vec<u8>>> {
    match method {
        0 => Some(PlainKeyProtector.unprotect(sealed)),
        #[cfg(windows)]
        1 => Some(dpapi::DpapiKeyProtector.unprotect(sealed)),
        #[cfg(target_os = "linux")]
        2 => Some(keyring::SecretServiceKeyProtector.unprotect(sealed)),
        _ => None,
    }
}

fn decode(bytes: &[u8], protector: &dyn KeyProtector) -> Result<SecretKey> {
    let corrupt = || Error::Storage("the device key file is corrupt".into());
    let rest = bytes.strip_prefix(MAGIC.as_slice()).ok_or_else(corrupt)?;
    let (&method, sealed) = rest.split_first().ok_or_else(corrupt)?;
    let plain = if method == protector.id() {
        protector.unprotect(sealed)
    } else if let Some(res) = unprotect_with_known(method, sealed) {
        res
    } else {
        return Err(Error::Storage(format!(
            "the device key is protected with method {method}, but this build uses {}",
            protector.id()
        )));
    }
    .map_err(|e| Error::Storage(format!("cannot unlock device key: {e}")))?;
    let bytes: [u8; 32] = plain.as_slice().try_into().map_err(|_| corrupt())?;
    Ok(SecretKey::from_bytes(&bytes))
}

/// Writes via a temporary file and rename, so a crash never leaves a
/// half-written key behind.
fn write_atomically(path: &Path, data: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut file = fs::File::create(&tmp)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(data)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)
}

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
mod keyring {
    //! Linux kernel user/session keyring (`keyctl`) backed ChaCha20-Poly1305
    //! key protector (`id = 2`).

    use std::{ffi::CStr, fs, io};

    use ring::aead::{Aad, CHACHA20_POLY1305, LessSafeKey, Nonce, UnboundKey};

    use super::KeyProtector;

    const KEY_TYPE: &CStr = c"user";
    const KEY_DESC: &CStr = c"nectarlink:identity-master-key";
    const KEY_SPEC_USER_KEYRING: libc::c_int = -4;
    const KEY_SPEC_SESSION_KEYRING: libc::c_int = -3;
    const KEYCTL_SEARCH: libc::c_int = 10;
    const KEYCTL_READ: libc::c_int = 11;

    const KEY_LEN: usize = 32;
    const NONCE_LEN: usize = 12;
    const TAG_LEN: usize = 16;
    const ENTROPY: &[u8] = b"nectarlink/identity/v1";

    /// Linux kernel keyring protector (`id = 2`).
    #[derive(Debug, Default, Clone, Copy)]
    pub struct SecretServiceKeyProtector;

    impl SecretServiceKeyProtector {
        /// Probes whether the Linux kernel user or session keyring is accessible.
        pub fn is_available() -> bool {
            get_or_create_wrapping_key().is_ok()
        }

        pub(super) fn protect_with_key(wrapping_key: &[u8; KEY_LEN], plaintext: &[u8]) -> io::Result<Vec<u8>> {
            let unbound = UnboundKey::new(&CHACHA20_POLY1305, wrapping_key)
                .map_err(|_| io::Error::other("invalid wrapping key"))?;
            let key = LessSafeKey::new(unbound);
            let nonce_bytes: [u8; NONCE_LEN] = rand::random();
            let nonce = Nonce::assume_unique_for_key(nonce_bytes);
            let mut in_out = plaintext.to_vec();
            key.seal_in_place_append_tag(nonce, Aad::from(ENTROPY), &mut in_out)
                .map_err(|_| io::Error::other("failed to seal device key"))?;
            let mut out = Vec::with_capacity(NONCE_LEN + in_out.len());
            out.extend_from_slice(&nonce_bytes);
            out.extend_from_slice(&in_out);
            Ok(out)
        }

        pub(super) fn unprotect_with_key(
            wrapping_key: &[u8; KEY_LEN],
            ciphertext: &[u8],
        ) -> io::Result<Vec<u8>> {
            if ciphertext.len() < NONCE_LEN + TAG_LEN {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "ciphertext too short"));
            }
            let (nonce_slice, body) = ciphertext.split_at(NONCE_LEN);
            let nonce_bytes: [u8; NONCE_LEN] =
                nonce_slice.try_into().map_err(|_| io::Error::other("invalid nonce"))?;
            let unbound = UnboundKey::new(&CHACHA20_POLY1305, wrapping_key)
                .map_err(|_| io::Error::other("invalid wrapping key"))?;
            let key = LessSafeKey::new(unbound);
            let nonce = Nonce::assume_unique_for_key(nonce_bytes);
            let mut in_out = body.to_vec();
            let plain = key
                .open_in_place(nonce, Aad::from(ENTROPY), &mut in_out)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "authentication failed"))?;
            Ok(plain.to_vec())
        }
    }

    fn read_key_by_serial(serial: libc::c_int) -> io::Result<[u8; KEY_LEN]> {
        let mut buf = [0u8; KEY_LEN];
        // SAFETY: `buf` is a valid mutable buffer of `KEY_LEN` bytes for the duration of the syscall.
        let n = unsafe {
            libc::syscall(
                libc::SYS_keyctl,
                KEYCTL_READ,
                serial,
                buf.as_mut_ptr().cast::<libc::c_char>(),
                buf.len(),
            )
        };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        if n as usize != KEY_LEN {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "unexpected keyring key length"));
        }
        Ok(buf)
    }

    fn search_and_read_key(keyring: libc::c_int) -> io::Result<[u8; KEY_LEN]> {
        // SAFETY: `KEY_TYPE` and `KEY_DESC` are NUL-terminated static C strings.
        let serial = unsafe {
            libc::syscall(
                libc::SYS_keyctl,
                KEYCTL_SEARCH,
                keyring,
                KEY_TYPE.as_ptr(),
                KEY_DESC.as_ptr(),
                0 as libc::c_int,
            )
        };
        if serial <= 0 {
            return Err(io::Error::last_os_error());
        }
        read_key_by_serial(serial as libc::c_int)
    }

    fn add_key_to_keyring(keyring: libc::c_int, key: &[u8; KEY_LEN]) -> io::Result<libc::c_int> {
        // SAFETY: `KEY_TYPE` and `KEY_DESC` are NUL-terminated static C strings, and `key` is a valid buffer.
        let serial = unsafe {
            libc::syscall(
                libc::SYS_add_key,
                KEY_TYPE.as_ptr(),
                KEY_DESC.as_ptr(),
                key.as_ptr().cast::<libc::c_void>(),
                key.len(),
                keyring,
            )
        };
        if serial <= 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(serial as libc::c_int)
    }

    fn derive_seed_key() -> [u8; KEY_LEN] {
        let machine_id = fs::read_to_string("/etc/machine-id")
            .or_else(|_| fs::read_to_string("/var/lib/dbus/machine-id"))
            .unwrap_or_default();
        // SAFETY: `getuid` is always safe to call on POSIX systems.
        let uid = unsafe { libc::getuid() };
        let mut hasher = blake3::Hasher::new_derive_key("nectarlink 2026 linux keyring master key v1");
        hasher.update(machine_id.trim().as_bytes());
        hasher.update(&uid.to_le_bytes());
        *hasher.finalize().as_bytes()
    }

    fn get_or_create_wrapping_key() -> io::Result<[u8; KEY_LEN]> {
        if let Ok(key) = search_and_read_key(KEY_SPEC_USER_KEYRING) {
            return Ok(key);
        }
        if let Ok(key) = search_and_read_key(KEY_SPEC_SESSION_KEYRING) {
            return Ok(key);
        }
        let seed = derive_seed_key();
        let serial = add_key_to_keyring(KEY_SPEC_USER_KEYRING, &seed)
            .or_else(|_| add_key_to_keyring(KEY_SPEC_SESSION_KEYRING, &seed))?;
        read_key_by_serial(serial)
    }

    impl KeyProtector for SecretServiceKeyProtector {
        fn id(&self) -> u8 {
            2
        }

        fn protect(&self, plaintext: &[u8]) -> io::Result<Vec<u8>> {
            let wrapping_key = get_or_create_wrapping_key()?;
            Self::protect_with_key(&wrapping_key, plaintext)
        }

        fn unprotect(&self, ciphertext: &[u8]) -> io::Result<Vec<u8>> {
            let wrapping_key = get_or_create_wrapping_key()?;
            Self::unprotect_with_key(&wrapping_key, ciphertext)
        }
    }
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

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_secret_service_protector_round_trips_and_detects_tampering() {
        let wrapping_key = [0x5a_u8; 32];
        let secret = [0x3c_u8; 32];
        let sealed = SecretServiceKeyProtector::protect_with_key(&wrapping_key, &secret).unwrap();
        assert_eq!(sealed.len(), 12 + secret.len() + 16);
        assert!(!sealed.windows(secret.len()).any(|w| w == secret));

        let opened = SecretServiceKeyProtector::unprotect_with_key(&wrapping_key, &sealed).unwrap();
        assert_eq!(opened, secret);

        // Tampering with ciphertext or tag must fail authentication.
        let mut tampered = sealed.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 0x01;
        assert!(SecretServiceKeyProtector::unprotect_with_key(&wrapping_key, &tampered).is_err());

        // Short ciphertext must fail.
        assert!(SecretServiceKeyProtector::unprotect_with_key(&wrapping_key, &[0u8; 10]).is_err());

        // If the kernel keyring is available in this environment, verify end-to-end protect/unprotect.
        if SecretServiceKeyProtector::is_available() {
            let p = SecretServiceKeyProtector;
            assert_eq!(p.id(), 2);
            let ct = p.protect(&secret).unwrap();
            assert_eq!(p.unprotect(&ct).unwrap(), secret);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_backward_compatibility_with_plain_key_protector() {
        let dir = tempfile::tempdir().unwrap();
        // Create key using PlainKeyProtector (id = 0).
        let created = load_or_create(dir.path(), &PlainKeyProtector).unwrap();
        // Reload using SecretServiceKeyProtector (id = 2) or default_protector():
        // decode() must fall back via unprotect_with_known(0, ...) and load the same key.
        let reloaded = load_or_create(dir.path(), &SecretServiceKeyProtector).unwrap();
        assert_eq!(created.public(), reloaded.public());
    }
}
