// SPDX-License-Identifier: GPL-3.0-or-later
//! Protects the device key (and the other files sealed with the platform
//! `KeyProtector`) with a wrapping key kept in the desktop's Secret Service
//! (GNOME Keyring, KWallet's `org.freedesktop.secrets` bridge, KeePassXC).
//!
//! The wrapping key is created on first use and stored once per user. Data is
//! sealed with AES-256-GCM. Without a Secret Service, [`protector`] returns
//! `None` and the caller keeps the unprotected default.

use std::{
    collections::HashMap,
    io,
    sync::{Arc, mpsc},
    time::Duration,
};

use nectarlink_core::KeyProtector;
use ring::{
    aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey},
    rand::{SecureRandom, SystemRandom},
};
use zbus::{
    blocking::{Connection, Proxy},
    zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value},
};

/// The method id written into the key file for this protector.
const METHOD_ID: u8 = 2;
/// Starts every sealed blob; data without it is read back as stored by an
/// older, unprotected build and is sealed the next time it is written.
const MAGIC: &[u8; 8] = b"NLSS\x01\0\0\0";
const AAD: &[u8] = b"nectarlink/seal/v1";
const KEY_LEN: usize = 32;

const SERVICE: &str = "org.freedesktop.secrets";
const SERVICE_PATH: &str = "/org/freedesktop/secrets";
const SERVICE_IFACE: &str = "org.freedesktop.Secret.Service";
const COLLECTION_IFACE: &str = "org.freedesktop.Secret.Collection";
const ITEM_IFACE: &str = "org.freedesktop.Secret.Item";
const PROMPT_IFACE: &str = "org.freedesktop.Secret.Prompt";
/// How long to wait for the user to unlock the keyring.
const PROMPT_TIMEOUT: Duration = Duration::from_secs(90);

/// The wrapping key's item in the keyring.
const APPLICATION: &str = "nectarlink";
const KIND: &str = "device-key-wrap";

/// The protector backed by the Secret Service, or `None` when there is no
/// usable service (no session bus, no keyring, locked and not unlocked, or a
/// stored key that can't be read). The reason is logged, never any key
/// material.
pub fn protector() -> Option<Arc<dyn KeyProtector>> {
    match wrapping_key(KIND) {
        Ok(key) => Some(Arc::new(SecretServiceProtector::from_key(&key))),
        Err(e) => {
            tracing::info!(error = %e, "Secret Service unavailable; a device key it sealed can't be opened until it is");
            None
        }
    }
}

/// Seals data with a wrapping key; see the module docs.
pub struct SecretServiceProtector {
    key: LessSafeKey,
}

impl SecretServiceProtector {
    fn from_key(key: &[u8; KEY_LEN]) -> Self {
        let unbound = UnboundKey::new(&AES_256_GCM, key).expect("a 32-byte key fits AES-256-GCM");
        Self { key: LessSafeKey::new(unbound) }
    }
}

impl KeyProtector for SecretServiceProtector {
    fn id(&self) -> u8 {
        METHOD_ID
    }

    fn protect(&self, plaintext: &[u8]) -> io::Result<Vec<u8>> {
        let mut nonce = [0u8; NONCE_LEN];
        SystemRandom::new().fill(&mut nonce).map_err(|_| io::Error::other("no random numbers"))?;
        let mut sealed = plaintext.to_vec();
        self.key
            .seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::from(AAD), &mut sealed)
            .map_err(|_| io::Error::other("cannot seal"))?;
        let mut out = Vec::with_capacity(MAGIC.len() + NONCE_LEN + sealed.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&sealed);
        Ok(out)
    }

    fn unprotect(&self, ciphertext: &[u8]) -> io::Result<Vec<u8>> {
        let Some(rest) = ciphertext.strip_prefix(MAGIC.as_slice()) else {
            // Written before the keyring was used.
            return Ok(ciphertext.to_vec());
        };
        let (nonce, sealed) = rest
            .split_at_checked(NONCE_LEN)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "sealed data is too short"))?;
        let nonce = Nonce::try_assume_unique_for_key(nonce)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad nonce"))?;
        let mut buf = sealed.to_vec();
        let plain = self
            .key
            .open_in_place(nonce, Aad::from(AAD), &mut buf)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "wrong wrapping key or damaged data"))?;
        Ok(plain.to_vec())
    }
}

fn dbus(e: zbus::Error) -> io::Error {
    io::Error::other(format!("Secret Service: {e}"))
}

fn proxy<'a>(conn: &'a Connection, path: &'a str, iface: &'a str) -> io::Result<Proxy<'a>> {
    Proxy::new(conn, SERVICE, path, iface).map_err(dbus)
}

/// Waits for the user to answer a keyring prompt. `Ok(false)` if dismissed.
fn run_prompt(conn: &Connection, prompt: &OwnedObjectPath) -> io::Result<bool> {
    if prompt.as_str() == "/" {
        return Ok(true);
    }
    let owned = prompt.clone();
    let conn = conn.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new().name("keyring-prompt".into()).spawn(move || {
        let result = (|| -> io::Result<bool> {
            let p = proxy(&conn, owned.as_str(), PROMPT_IFACE)?;
            let mut done = p.receive_signal("Completed").map_err(dbus)?;
            p.call_method("Prompt", &("",)).map_err(dbus)?;
            let msg = done.next().ok_or_else(|| io::Error::other("the keyring prompt went away"))?;
            let (dismissed, _result): (bool, OwnedValue) = msg.body().deserialize().map_err(dbus)?;
            Ok(!dismissed)
        })();
        let _ = tx.send(result);
    })?;
    rx.recv_timeout(PROMPT_TIMEOUT)
        .map_err(|_| io::Error::other("timed out waiting for the keyring prompt"))?
}

/// Unlocks `objects`, returning the ones that ended up unlocked.
fn unlock(
    conn: &Connection,
    service: &Proxy<'_>,
    objects: &[OwnedObjectPath],
) -> io::Result<Vec<OwnedObjectPath>> {
    let paths: Vec<ObjectPath<'_>> = objects.iter().map(|o| o.as_ref()).collect();
    let (unlocked, prompt): (Vec<OwnedObjectPath>, OwnedObjectPath) =
        service.call("Unlock", &(paths,)).map_err(dbus)?;
    if run_prompt(conn, &prompt)? && unlocked.is_empty() {
        // The prompt unlocked them; ask again for the list.
        let paths: Vec<ObjectPath<'_>> = objects.iter().map(|o| o.as_ref()).collect();
        let (unlocked, _): (Vec<OwnedObjectPath>, OwnedObjectPath) =
            service.call("Unlock", &(paths,)).map_err(dbus)?;
        return Ok(unlocked);
    }
    Ok(unlocked)
}

type Secret<'a> = (ObjectPath<'a>, Vec<u8>, Vec<u8>, &'a str);

/// Reads the wrapping key stored under `kind`, creating it on first use.
fn wrapping_key(kind: &str) -> io::Result<[u8; KEY_LEN]> {
    let conn = Connection::session().map_err(dbus)?;
    let service = proxy(&conn, SERVICE_PATH, SERVICE_IFACE)?;
    let (_, session): (OwnedValue, OwnedObjectPath) =
        service.call("OpenSession", &("plain", Value::from(""))).map_err(dbus)?;

    let attributes: HashMap<&str, &str> = HashMap::from([("application", APPLICATION), ("kind", kind)]);
    let (unlocked, locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) =
        service.call("SearchItems", &(&attributes,)).map_err(dbus)?;
    let mut found = unlocked;
    if found.is_empty() && !locked.is_empty() {
        found = unlock(&conn, &service, &locked)?;
        if found.is_empty() {
            return Err(io::Error::other("the keyring is locked"));
        }
    }

    if let Some(item) = found.first() {
        let item = proxy(&conn, item.as_str(), ITEM_IFACE)?;
        let (_, _, value, _): (OwnedObjectPath, Vec<u8>, Vec<u8>, String) =
            item.call("GetSecret", &(&session,)).map_err(dbus)?;
        // Never replace a stored key that doesn't look like ours: the data it
        // sealed would be lost.
        return <[u8; KEY_LEN]>::try_from(value.as_slice())
            .map_err(|_| io::Error::other("the stored wrapping key is malformed"));
    }

    let collection: OwnedObjectPath = service.call("ReadAlias", &("default",)).map_err(dbus)?;
    if collection.as_str() == "/" {
        return Err(io::Error::other("no default keyring"));
    }
    if !unlock(&conn, &service, std::slice::from_ref(&collection))?.contains(&collection) {
        return Err(io::Error::other("the keyring is locked"));
    }

    let mut key = [0u8; KEY_LEN];
    SystemRandom::new().fill(&mut key).map_err(|_| io::Error::other("no random numbers"))?;
    let properties: HashMap<&str, Value<'_>> = HashMap::from([
        ("org.freedesktop.Secret.Item.Label", Value::from("NectarLink device key protection")),
        ("org.freedesktop.Secret.Item.Attributes", Value::from(attributes)),
    ]);
    let secret: Secret<'_> = (session.as_ref(), Vec::new(), key.to_vec(), "application/octet-stream");
    let collection_proxy = proxy(&conn, collection.as_str(), COLLECTION_IFACE)?;
    let (_item, prompt): (OwnedObjectPath, OwnedObjectPath) =
        collection_proxy.call("CreateItem", &(properties, secret, false)).map_err(dbus)?;
    if !run_prompt(&conn, &prompt)? {
        return Err(io::Error::other("storing the wrapping key was declined"));
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn protector_with(byte: u8) -> SecretServiceProtector {
        SecretServiceProtector::from_key(&[byte; KEY_LEN])
    }

    #[test]
    fn sealed_data_round_trips_and_hides_the_plaintext() {
        let p = protector_with(1);
        let plain = b"a device key, 32 bytes long....!";
        let sealed = p.protect(plain).unwrap();
        assert!(sealed.starts_with(MAGIC));
        assert!(!sealed.windows(plain.len()).any(|w| w == plain), "plaintext must not appear");
        assert_eq!(p.unprotect(&sealed).unwrap(), plain);
    }

    #[test]
    fn the_same_data_seals_differently_each_time() {
        let p = protector_with(1);
        assert_ne!(p.protect(b"same").unwrap(), p.protect(b"same").unwrap());
    }

    #[test]
    fn data_stored_before_the_keyring_is_read_back_as_is() {
        let p = protector_with(1);
        assert_eq!(p.unprotect(b"{\"history\":[]}").unwrap(), b"{\"history\":[]}");
        assert_eq!(p.unprotect(b"").unwrap(), b"");
    }

    #[test]
    fn tampering_or_the_wrong_key_is_an_error_not_garbage() {
        let sealed = protector_with(1).protect(b"secret").unwrap();
        assert!(protector_with(2).unprotect(&sealed).is_err(), "wrong key");
        let mut damaged = sealed.clone();
        *damaged.last_mut().unwrap() ^= 1;
        assert!(protector_with(1).unprotect(&damaged).is_err(), "flipped bit");
        assert!(protector_with(1).unprotect(&sealed[..MAGIC.len() + 4]).is_err(), "truncated");
    }

    /// Talks to the real Secret Service of the logged-in session, using its
    /// own item so it never touches the real wrapping key. Run with
    /// `cargo test -- --ignored secret_service_round_trip`.
    #[test]
    #[ignore = "needs a Secret Service on the session bus"]
    fn secret_service_round_trip() {
        let kind = "device-key-wrap-test";
        let first = wrapping_key(kind).expect("create or read the key");
        let second = wrapping_key(kind).expect("read it back");
        assert_eq!(first, second, "the stored key is stable");
        assert_ne!(first, [0u8; KEY_LEN]);

        // Remove the test item again.
        let conn = Connection::session().unwrap();
        let service = proxy(&conn, SERVICE_PATH, SERVICE_IFACE).unwrap();
        let attributes: HashMap<&str, &str> = HashMap::from([("application", APPLICATION), ("kind", kind)]);
        let (unlocked, _): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) =
            service.call("SearchItems", &(&attributes,)).unwrap();
        for item in unlocked {
            let item = proxy(&conn, item.as_str(), ITEM_IFACE).unwrap();
            let _: OwnedObjectPath = item.call("Delete", &()).unwrap();
        }
    }
}
