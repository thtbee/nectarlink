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
    #[cfg(target_os = "macos")]
    {
        Arc::new(keychain::KeychainKeyProtector)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
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
            let key = match decode(&bytes, protector) {
                Ok(key) => key,
                // A key written by an earlier macOS or Linux build, before
                // that platform had a protector (method 0), is upgraded in
                // place so the device keeps its identity and doesn't have to
                // pair again. Only macOS and Linux do this. On Windows a
                // method-0 file must stay refused: re-sealing one under DPAPI
                // would adopt a plain key dropped into the data folder as this
                // PC's identity, and whoever wrote it would hold the private key.
                #[cfg(any(target_os = "macos", target_os = "linux"))]
                Err(_) if protector.id() != 0 && bytes.get(MAGIC.len()) == Some(&0) => {
                    let key = decode(&bytes, &PlainKeyProtector)?;
                    write_atomically(&path, &encode(&key, protector)?)?;
                    tracing::info!("upgraded the device key to platform protection");
                    key
                }
                Err(e) => return Err(e),
            };
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

fn decode(bytes: &[u8], protector: &dyn KeyProtector) -> Result<SecretKey> {
    let corrupt = || Error::Storage("the device key file is corrupt".into());
    let rest = bytes.strip_prefix(MAGIC.as_slice()).ok_or_else(corrupt)?;
    let (&method, sealed) = rest.split_first().ok_or_else(corrupt)?;
    if method != protector.id() {
        return Err(Error::Storage(format!(
            "the device key is protected with method {method}, but this build uses {}",
            protector.id()
        )));
    }
    let plain =
        protector.unprotect(sealed).map_err(|e| Error::Storage(format!("cannot unlock device key: {e}")))?;
    let bytes: [u8; 32] = plain.as_slice().try_into().map_err(|_| corrupt())?;
    Ok(SecretKey::from_bytes(&bytes))
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

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod keychain {
    //! macOS Keychain protection: the device key and the clipboard vault are
    //! sealed with a 256-bit key that lives in the user's login Keychain, so a
    //! copy of the data directory alone can't act as this Mac.
    //!
    //! The Keychain protects the *key*, and AES-256-GCM seals each payload with
    //! it: one protector is asked for several unrelated blobs (the identity key,
    //! the clipboard index and its images), so a single Keychain item can't hold
    //! them all. Security.framework is called directly, so no crate is added.

    use std::{
        ffi::{CString, c_void},
        io, ptr,
        sync::{Mutex, OnceLock},
    };

    use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};

    use super::KeyProtector;

    /// Keychain service and account of the key-encryption key. Both are
    /// versioned: changing either locks an existing install out of its identity.
    const SERVICE: &str = "app.nectarlink.identity";
    const ACCOUNT: &str = "key-encryption-key-v1";
    /// Domain separation for the AEAD.
    const AAD: &[u8] = b"nectarlink/keychain/v1";
    const KEY_LEN: usize = 32;

    // CoreFoundation / Security status codes handled here.
    const ERR_SUCCESS: i32 = 0;
    const ERR_ITEM_NOT_FOUND: i32 = -25_300;
    const ERR_DUPLICATE_ITEM: i32 = -25_299;
    /// `kCFStringEncodingUTF8`.
    const UTF8: u32 = 0x0800_0100;

    type CFTypeRef = *const c_void;
    type CFIndex = isize;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFTypeDictionaryKeyCallBacks: c_void;
        static kCFTypeDictionaryValueCallBacks: c_void;
        static kCFBooleanTrue: CFTypeRef;
        fn CFStringCreateWithCString(alloc: CFTypeRef, text: *const i8, encoding: u32) -> CFTypeRef;
        fn CFDataCreate(alloc: CFTypeRef, bytes: *const u8, len: CFIndex) -> CFTypeRef;
        fn CFDataGetBytePtr(data: CFTypeRef) -> *const u8;
        fn CFDataGetLength(data: CFTypeRef) -> CFIndex;
        fn CFDictionaryCreate(
            alloc: CFTypeRef,
            keys: *const CFTypeRef,
            values: *const CFTypeRef,
            count: CFIndex,
            key_callbacks: *const c_void,
            value_callbacks: *const c_void,
        ) -> CFTypeRef;
        fn CFRelease(value: CFTypeRef);
    }

    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        static kSecClass: CFTypeRef;
        static kSecClassGenericPassword: CFTypeRef;
        static kSecAttrService: CFTypeRef;
        static kSecAttrAccount: CFTypeRef;
        static kSecValueData: CFTypeRef;
        static kSecReturnData: CFTypeRef;
        static kSecMatchLimit: CFTypeRef;
        static kSecMatchLimitOne: CFTypeRef;
        fn SecItemAdd(attributes: CFTypeRef, result: *mut CFTypeRef) -> i32;
        fn SecItemCopyMatching(query: CFTypeRef, result: *mut CFTypeRef) -> i32;
    }

    /// A CoreFoundation object this module owns, released on drop.
    struct Cf(CFTypeRef);

    impl Drop for Cf {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: every `Cf` holds one reference from a Create call or
                // from `SecItemCopyMatching`, released exactly once here.
                unsafe { CFRelease(self.0) };
            }
        }
    }

    /// The user (or a policy) refused Keychain access. Unsigned development
    /// builds change code identity on every rebuild, so this is the common
    /// failure there and the message should say what to do about it.
    const ERR_AUTH_FAILED: i32 = -25_293;
    const ERR_INTERACTION_NOT_ALLOWED: i32 = -25_308;

    fn failure(what: &str, status: i32) -> io::Error {
        if matches!(status, ERR_AUTH_FAILED | ERR_INTERACTION_NOT_ALLOWED) {
            return io::Error::other(format!(
                "macOS Keychain {what} was refused (OSStatus {status}); allow Nectarlink access to the login Keychain, then start it again"
            ));
        }
        io::Error::other(format!("macOS Keychain {what} failed (OSStatus {status})"))
    }

    fn cf_string(value: &str) -> io::Result<Cf> {
        let text = CString::new(value).map_err(|_| io::Error::other("Keychain name has a NUL"))?;
        // SAFETY: `text` is a live NUL-terminated string for the whole call.
        let string = unsafe { CFStringCreateWithCString(ptr::null(), text.as_ptr(), UTF8) };
        if string.is_null() {
            return Err(io::Error::other("cannot build a Keychain string"));
        }
        Ok(Cf(string))
    }

    fn cf_data(bytes: &[u8]) -> io::Result<Cf> {
        // SAFETY: `bytes` is valid for `bytes.len()` reads while it's copied.
        let data = unsafe { CFDataCreate(ptr::null(), bytes.as_ptr(), bytes.len() as CFIndex) };
        if data.is_null() {
            return Err(io::Error::other("cannot build Keychain data"));
        }
        Ok(Cf(data))
    }

    fn cf_dictionary(pairs: &[(CFTypeRef, CFTypeRef)]) -> io::Result<Cf> {
        let keys: Vec<CFTypeRef> = pairs.iter().map(|(key, _)| *key).collect();
        let values: Vec<CFTypeRef> = pairs.iter().map(|(_, value)| *value).collect();
        // SAFETY: both vectors are live for the call and hold `pairs.len()`
        // valid CoreFoundation objects, as do the type callbacks.
        let dictionary = unsafe {
            CFDictionaryCreate(
                ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                pairs.len() as CFIndex,
                ptr::addr_of!(kCFTypeDictionaryKeyCallBacks),
                ptr::addr_of!(kCFTypeDictionaryValueCallBacks),
            )
        };
        if dictionary.is_null() {
            return Err(io::Error::other("cannot build a Keychain query"));
        }
        Ok(Cf(dictionary))
    }

    /// The Keychain item that names the key-encryption key.
    fn entry(service: &Cf, account: &Cf) -> Vec<(CFTypeRef, CFTypeRef)> {
        // SAFETY: these are the Security.framework constants, always valid.
        unsafe {
            vec![
                (kSecClass, kSecClassGenericPassword),
                (kSecAttrService, service.0),
                (kSecAttrAccount, account.0),
            ]
        }
    }

    /// The key-encryption key, created on first use and cached for this process.
    ///
    /// A locked Keychain is an error rather than a silent fallback: sealing the
    /// device key without platform protection is what this module exists to
    /// stop.
    fn key_encryption_key() -> io::Result<&'static [u8; KEY_LEN]> {
        static KEY: OnceLock<[u8; KEY_LEN]> = OnceLock::new();
        static CREATE: Mutex<()> = Mutex::new(());

        if let Some(key) = KEY.get() {
            return Ok(key);
        }
        // Serialize creation: one key must win, and every later payload has to
        // be sealed with the key that is actually stored.
        let _guard = CREATE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(key) = KEY.get() {
            return Ok(key);
        }
        let key = match read_kek()? {
            Some(key) => key,
            None => {
                let fresh: [u8; KEY_LEN] = rand::random();
                // `false`: another process created the item first, and the
                // key it wrote is the one that opens this install's data.
                if write_kek(&fresh)? { fresh } else { read_kek()?.unwrap_or(fresh) }
            }
        };
        Ok(KEY.get_or_init(|| key))
    }

    fn read_kek() -> io::Result<Option<[u8; KEY_LEN]>> {
        let service = cf_string(SERVICE)?;
        let account = cf_string(ACCOUNT)?;
        let mut pairs = entry(&service, &account);
        // SAFETY: the two extra constants are valid, and `cf_dictionary` keeps
        // every element live for the call.
        unsafe {
            pairs.push((kSecReturnData, kCFBooleanTrue));
            pairs.push((kSecMatchLimit, kSecMatchLimitOne));
        }
        let query = cf_dictionary(&pairs)?;
        drop(pairs);

        let mut result: CFTypeRef = ptr::null();
        // SAFETY: `query` is a valid dictionary and `result` receives one
        // reference on success (released by `Cf`).
        let status = unsafe { SecItemCopyMatching(query.0, &mut result) };
        if status == ERR_ITEM_NOT_FOUND {
            return Ok(None);
        }
        if status != ERR_SUCCESS {
            return Err(failure("read", status));
        }
        let data = Cf(result);
        if data.0.is_null() {
            return Err(io::Error::other("the Keychain returned no key data"));
        }
        // SAFETY: `data` is a CFData returned by the Keychain.
        let (bytes, len) = unsafe { (CFDataGetBytePtr(data.0), CFDataGetLength(data.0)) };
        if bytes.is_null() || len != KEY_LEN as CFIndex {
            return Err(io::Error::other("the Keychain key has the wrong length"));
        }
        // SAFETY: the call above proved exactly `KEY_LEN` readable bytes.
        let mut key = [0u8; KEY_LEN];
        key.copy_from_slice(unsafe { std::slice::from_raw_parts(bytes, KEY_LEN) });
        Ok(Some(key))
    }

    /// Stores the key if the item doesn't exist yet; `false` means another
    /// process created it first, and its key is the one to use.
    fn write_kek(key: &[u8; KEY_LEN]) -> io::Result<bool> {
        let service = cf_string(SERVICE)?;
        let account = cf_string(ACCOUNT)?;
        let value = cf_data(key)?;
        let mut pairs = entry(&service, &account);
        // SAFETY: the constant is valid and the data object is live.
        pairs.push(unsafe { (kSecValueData, value.0) });
        let attributes = cf_dictionary(&pairs)?;
        drop(pairs);

        // SAFETY: `attributes` is a valid dictionary; no result item is wanted.
        match unsafe { SecItemAdd(attributes.0, ptr::null_mut()) } {
            ERR_SUCCESS => Ok(true),
            ERR_DUPLICATE_ITEM => Ok(false),
            status => Err(failure("write", status)),
        }
    }

    fn aead_key(key: &[u8; KEY_LEN]) -> io::Result<LessSafeKey> {
        let key = UnboundKey::new(&AES_256_GCM, key)
            .map_err(|_| io::Error::other("the Keychain key is not a valid AES-256 key"))?;
        Ok(LessSafeKey::new(key))
    }

    /// `nonce || ciphertext || tag`, with a fresh random nonce every time.
    fn seal(key: &[u8; KEY_LEN], plaintext: &[u8]) -> io::Result<Vec<u8>> {
        let key = aead_key(key)?;
        let nonce: [u8; NONCE_LEN] = rand::random();
        let mut sealed = plaintext.to_vec();
        key.seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::from(AAD), &mut sealed)
            .map_err(|_| io::Error::other("cannot seal with the Keychain key"))?;
        let mut out = Vec::with_capacity(NONCE_LEN + sealed.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&sealed);
        Ok(out)
    }

    fn open(key: &[u8; KEY_LEN], sealed: &[u8]) -> io::Result<Vec<u8>> {
        if sealed.len() < NONCE_LEN + AES_256_GCM.tag_len() {
            return Err(io::Error::other("the sealed data is too short"));
        }
        let key = aead_key(key)?;
        let (nonce, ciphertext) = sealed.split_at(NONCE_LEN);
        let nonce =
            Nonce::try_assume_unique_for_key(nonce).map_err(|_| io::Error::other("bad sealed data"))?;
        let mut out = ciphertext.to_vec();
        let plain = key
            .open_in_place(nonce, Aad::from(AAD), &mut out)
            .map_err(|_| io::Error::other("this data was sealed with another Keychain key"))?;
        Ok(plain.to_vec())
    }

    /// The device key, the clipboard index and clipboard images are sealed
    /// with a key in the login Keychain. Removing the item makes the data
    /// unreadable, which is the intended failure.
    #[derive(Debug, Default, Clone, Copy)]
    pub struct KeychainKeyProtector;

    impl KeyProtector for KeychainKeyProtector {
        /// `NLK1` method 2: Keychain key, AES-256-GCM.
        fn id(&self) -> u8 {
            2
        }

        fn protect(&self, plaintext: &[u8]) -> io::Result<Vec<u8>> {
            seal(key_encryption_key()?, plaintext)
        }

        fn unprotect(&self, ciphertext: &[u8]) -> io::Result<Vec<u8>> {
            open(key_encryption_key()?, ciphertext)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn key_of(byte: u8) -> [u8; KEY_LEN] {
            [byte; KEY_LEN]
        }

        #[test]
        fn seals_and_opens() {
            let key = key_of(7);
            let sealed = seal(&key, b"the device key").unwrap();
            assert_ne!(&sealed[NONCE_LEN..], b"the device key");
            assert_eq!(open(&key, &sealed).unwrap(), b"the device key");
        }

        #[test]
        fn every_seal_uses_a_fresh_nonce() {
            let key = key_of(7);
            assert_ne!(seal(&key, b"same").unwrap(), seal(&key, b"same").unwrap());
        }

        #[test]
        fn rejects_tampering_and_another_key() {
            let key = key_of(7);
            let mut sealed = seal(&key, b"the device key").unwrap();
            let last = sealed.len() - 1;
            sealed[last] ^= 1;
            assert!(open(&key, &sealed).is_err(), "tampered tag must fail");
            assert!(open(&key_of(8), &seal(&key, b"x").unwrap()).is_err(), "another key must fail");
            assert!(open(&key, &[0u8; 4]).is_err(), "short input must fail");
        }

        #[test]
        #[ignore = "needs an unlocked login Keychain"]
        fn keychain_round_trip() {
            let protector = KeychainKeyProtector;
            let sealed = protector.protect(b"the device key").unwrap();
            assert_eq!(protector.unprotect(&sealed).unwrap(), b"the device key");
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

    /// Stands in for a platform protector so tests never touch the real
    /// Keychain or DPAPI.
    struct TestProtector;

    impl KeyProtector for TestProtector {
        fn id(&self) -> u8 {
            9
        }
        fn protect(&self, plaintext: &[u8]) -> io::Result<Vec<u8>> {
            Ok(plaintext.iter().map(|b| b ^ 0xA5).collect())
        }
        fn unprotect(&self, ciphertext: &[u8]) -> io::Result<Vec<u8>> {
            Ok(ciphertext.iter().map(|b| b ^ 0xA5).collect())
        }
    }

    #[test]
    fn creates_then_reloads_the_same_key() {
        let dir = tempfile::tempdir().unwrap();
        let protector = TestProtector;
        let first = load_or_create(dir.path(), &protector).unwrap();
        let second = load_or_create(dir.path(), &protector).unwrap();
        assert_eq!(first.public(), second.public());
    }

    #[test]
    fn key_file_does_not_contain_the_plain_key_when_protected() {
        let dir = tempfile::tempdir().unwrap();
        let protector = TestProtector;
        let key = load_or_create(dir.path(), &protector).unwrap();
        let file = fs::read(dir.path().join(FILE_NAME)).unwrap();
        let raw = key.to_bytes();
        assert!(!file.windows(raw.len()).any(|w| w == raw), "plaintext key found on disk");
    }

    /// The in-place upgrade is macOS and Linux only (see `load_or_create`), so
    /// this test is too: on Windows the same file must stay refused.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn upgrades_a_plain_key_in_place_and_keeps_the_identity() {
        let dir = tempfile::tempdir().unwrap();
        let key = SecretKey::generate();
        let identity = key.public();
        // A file written before this platform had a protector (method 0).
        write_atomically(&dir.path().join(FILE_NAME), &encode(&key, &PlainKeyProtector).unwrap()).unwrap();

        let upgraded = load_or_create(dir.path(), &TestProtector).unwrap();
        assert_eq!(upgraded.public(), identity, "the device keeps its identity");

        let file = fs::read(dir.path().join(FILE_NAME)).unwrap();
        assert_eq!(file.get(MAGIC.len()), Some(&TestProtector.id()), "rewritten with the protector");
        assert_eq!(
            load_or_create(dir.path(), &TestProtector).unwrap().public(),
            identity,
            "the protected file opens next time"
        );
    }

    /// Windows and Android must keep refusing a method-`0` file. Re-sealing it
    /// under the platform protector would turn a plain key dropped into the
    /// data folder into this device's trusted identity.
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    #[test]
    fn a_plain_key_file_is_not_adopted_elsewhere() {
        let dir = tempfile::tempdir().unwrap();
        let key = SecretKey::generate();
        write_atomically(&dir.path().join(FILE_NAME), &encode(&key, &PlainKeyProtector).unwrap()).unwrap();
        assert!(matches!(load_or_create(dir.path(), &TestProtector), Err(Error::Storage(_))));
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
    fn a_sealed_key_is_not_opened_without_its_protector() {
        let dir = tempfile::tempdir().unwrap();
        load_or_create(dir.path(), &TestProtector).unwrap();
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

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_uses_the_keychain_protector() {
        // Constructing it must not touch the Keychain; only sealing does.
        assert_eq!(default_protector().id(), 2);
    }

    /// The real macOS path: a fresh install writes a Keychain-sealed key, the
    /// Keychain item exists, and the file alone doesn't contain the key.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "needs an unlocked login Keychain"]
    fn macos_key_file_is_sealed_with_the_keychain() {
        let dir = tempfile::tempdir().unwrap();
        let protector = default_protector();
        let key = load_or_create(dir.path(), protector.as_ref()).unwrap();
        let file = fs::read(dir.path().join(FILE_NAME)).unwrap();

        assert_eq!(&file[..MAGIC.len()], MAGIC);
        assert_eq!(file[MAGIC.len()], 2, "sealed with the Keychain protector");
        assert!(!file.windows(32).any(|w| w == key.to_bytes()), "the plain key must not be on disk");
        assert_eq!(
            load_or_create(dir.path(), protector.as_ref()).unwrap().public(),
            key.public(),
            "the Keychain key opens it again"
        );
    }

    /// Upgrading an install that stored the key unsealed (method 0).
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "needs an unlocked login Keychain"]
    fn macos_upgrades_a_plain_key_to_the_keychain() {
        let dir = tempfile::tempdir().unwrap();
        let key = SecretKey::generate();
        let identity = key.public();
        write_atomically(&dir.path().join(FILE_NAME), &encode(&key, &PlainKeyProtector).unwrap()).unwrap();
        assert_eq!(fs::read(dir.path().join(FILE_NAME)).unwrap()[MAGIC.len()], 0, "starts unsealed");

        let protector = default_protector();
        let upgraded = load_or_create(dir.path(), protector.as_ref()).unwrap();
        assert_eq!(upgraded.public(), identity, "same DeviceId, so no re-pairing");

        let file = fs::read(dir.path().join(FILE_NAME)).unwrap();
        assert_eq!(file[MAGIC.len()], 2, "re-sealed in place");
        assert!(!file.windows(32).any(|w| w == identity.as_bytes()), "plain key is gone");
        assert_eq!(load_or_create(dir.path(), protector.as_ref()).unwrap().public(), identity);
    }

    #[cfg(unix)]
    #[test]
    fn key_file_has_0600_permissions_on_unix() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        load_or_create(dir.path(), &TestProtector).unwrap();
        let meta = fs::metadata(dir.path().join(FILE_NAME)).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    }
}
