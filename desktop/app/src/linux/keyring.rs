// SPDX-License-Identifier: GPL-3.0-or-later
//! Protects the device key (and the other files sealed with the platform
//! `KeyProtector`) with a wrapping key kept in the desktop's Secret Service
//! (GNOME Keyring, KWallet's `org.freedesktop.secrets` bridge, KeePassXC).
//!
//! The wrapping key is created on first use and stored once per user. Data is
//! sealed with AES-256-GCM. Until the device key is sealed, a keyring that
//! isn't ready only means starting unprotected, with no prompt and no wait.
//! Once it is sealed, a keyring that is locked, missing or not running stops
//! the core from starting until it's back, and nothing on disk is touched.

use std::{collections::HashMap, io, path::Path, sync::Arc, time::Duration};

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
/// How long one call may take, so a keyring that hangs while it starts can't
/// hold up startup. Waiting for the user to answer a prompt isn't a call.
const CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// The wrapping key's item in the keyring.
const APPLICATION: &str = "nectarlink";
const KIND: &str = "device-key-wrap";

/// Why the wrapping key isn't available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    /// No Secret Service answers on the session bus.
    Unreachable,
    /// The keyring is locked, or unlocking it was declined.
    Locked,
    /// The keyring has no wrapping key, though the device key was sealed with one.
    Missing,
}

/// Whether the device key in `data_dir` is sealed with this protector.
pub fn is_sealed(data_dir: &Path) -> bool {
    nectarlink_core::stored_key_method(data_dir) == Some(METHOD_ID)
}

/// The protector to start the core with. `sealed` says whether the device key
/// is already sealed (see [`is_sealed`]), and `prompt` whether the keyring may
/// ask the user to unlock it, which blocks until they answer.
///
/// `Ok(None)` starts the core unprotected: the key isn't sealed yet and the
/// keyring isn't ready without asking, so it's sealed on a later start. A
/// sealed key never gets `Ok(None)`, and never a newly created wrapping key,
/// which would leave what the old one sealed unreadable.
pub fn open(sealed: bool, prompt: bool) -> Result<Option<Arc<dyn KeyProtector>>, Problem> {
    match wrapping_key(KIND, sealed, prompt && sealed) {
        Ok(key) => Ok(Some(Arc::new(SecretServiceProtector::from_key(&key)))),
        Err(problem) if sealed => Err(problem),
        Err(problem) => {
            tracing::info!(?problem, "Secret Service not ready; the device key stays unprotected for now");
            Ok(None)
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

fn dbus(e: zbus::Error) -> Problem {
    tracing::debug!(error = %e, "Secret Service call failed");
    Problem::Unreachable
}

fn proxy<'a>(conn: &'a Connection, path: &'a str, iface: &'a str) -> Result<Proxy<'a>, Problem> {
    Proxy::new(conn, SERVICE, path, iface).map_err(dbus)
}

/// Waits, with no time limit, for the user to answer a keyring prompt, or
/// dismisses it at once when `ask` is false. `Ok(false)` if not accepted.
fn run_prompt(conn: &Connection, prompt: &OwnedObjectPath, ask: bool) -> Result<bool, Problem> {
    if prompt.as_str() == "/" {
        return Ok(true);
    }
    let p = proxy(conn, prompt.as_str(), PROMPT_IFACE)?;
    if !ask {
        let _ = p.call_method("Dismiss", &());
        return Ok(false);
    }
    let mut done = p.receive_signal("Completed").map_err(dbus)?;
    p.call_method("Prompt", &("",)).map_err(dbus)?;
    let Some(msg) = done.next() else { return Ok(false) };
    let (dismissed, _result): (bool, OwnedValue) = msg.body().deserialize().map_err(dbus)?;
    Ok(!dismissed)
}

/// Unlocks `objects`, asking the user, and returns the ones that ended up unlocked.
fn unlock(
    conn: &Connection,
    service: &Proxy<'_>,
    objects: &[OwnedObjectPath],
) -> Result<Vec<OwnedObjectPath>, Problem> {
    let paths: Vec<ObjectPath<'_>> = objects.iter().map(|o| o.as_ref()).collect();
    let (unlocked, prompt): (Vec<OwnedObjectPath>, OwnedObjectPath) =
        service.call("Unlock", &(&paths,)).map_err(dbus)?;
    if run_prompt(conn, &prompt, true)? && unlocked.is_empty() {
        // The prompt unlocked them; ask again for the list.
        let (unlocked, _): (Vec<OwnedObjectPath>, OwnedObjectPath) =
            service.call("Unlock", &(&paths,)).map_err(dbus)?;
        return Ok(unlocked);
    }
    Ok(unlocked)
}

fn is_locked(conn: &Connection, collection: &OwnedObjectPath) -> Result<bool, Problem> {
    proxy(conn, collection.as_str(), COLLECTION_IFACE)?.get_property("Locked").map_err(dbus)
}

/// The keyring's locked collections.
fn locked_collections(conn: &Connection, service: &Proxy<'_>) -> Result<Vec<OwnedObjectPath>, Problem> {
    let all: Vec<OwnedObjectPath> = service.get_property("Collections").map_err(dbus)?;
    Ok(all.into_iter().filter(|c| is_locked(conn, c).unwrap_or(false)).collect())
}

type Secret<'a> = (ObjectPath<'a>, Vec<u8>, Vec<u8>, &'a str);

/// Reads the wrapping key stored under `kind`. Creates it on first use, but
/// only while nothing is `sealed` with an earlier one. Prompts only if `ask`.
fn wrapping_key(kind: &str, sealed: bool, ask: bool) -> Result<[u8; KEY_LEN], Problem> {
    let conn = zbus::blocking::connection::Builder::session()
        .map_err(dbus)?
        .method_timeout(CALL_TIMEOUT)
        .build()
        .map_err(dbus)?;
    let service = proxy(&conn, SERVICE_PATH, SERVICE_IFACE)?;
    let (_, session): (OwnedValue, OwnedObjectPath) =
        service.call("OpenSession", &("plain", Value::from(""))).map_err(dbus)?;

    let attributes: HashMap<&str, &str> = HashMap::from([("application", APPLICATION), ("kind", kind)]);
    let (unlocked, locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) =
        service.call("SearchItems", &(&attributes,)).map_err(dbus)?;
    let mut found = unlocked;
    if found.is_empty() && !locked.is_empty() {
        if !ask {
            return Err(Problem::Locked);
        }
        found = unlock(&conn, &service, &locked)?;
        if found.is_empty() {
            return Err(Problem::Locked);
        }
    }
    if found.is_empty() && sealed {
        // Some keyrings can't search a locked store, so finding nothing may
        // only mean it's locked: unlock it and look again.
        let collections = locked_collections(&conn, &service)?;
        if !collections.is_empty() {
            if !ask || unlock(&conn, &service, &collections)?.is_empty() {
                return Err(Problem::Locked);
            }
            let (unlocked, _): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) =
                service.call("SearchItems", &(&attributes,)).map_err(dbus)?;
            found = unlocked;
        }
    }

    if let Some(item) = found.first() {
        let item = proxy(&conn, item.as_str(), ITEM_IFACE)?;
        let (_, _, value, _): (OwnedObjectPath, Vec<u8>, Vec<u8>, String) =
            item.call("GetSecret", &(&session,)).map_err(dbus)?;
        // Never replace a stored key that doesn't look like ours: the data it
        // sealed would be lost.
        return <[u8; KEY_LEN]>::try_from(value.as_slice()).map_err(|_| {
            tracing::warn!("the stored wrapping key is malformed");
            Problem::Missing
        });
    }
    if sealed {
        return Err(Problem::Missing);
    }

    let collection: OwnedObjectPath = service.call("ReadAlias", &("default",)).map_err(dbus)?;
    if collection.as_str() == "/" {
        tracing::info!("no default keyring");
        return Err(Problem::Missing);
    }
    if is_locked(&conn, &collection)? {
        return Err(Problem::Locked);
    }

    let mut key = [0u8; KEY_LEN];
    SystemRandom::new().fill(&mut key).map_err(|_| Problem::Unreachable)?;
    let properties: HashMap<&str, Value<'_>> = HashMap::from([
        ("org.freedesktop.Secret.Item.Label", Value::from("NectarLink device key protection")),
        ("org.freedesktop.Secret.Item.Attributes", Value::from(attributes)),
    ]);
    let secret: Secret<'_> = (session.as_ref(), Vec::new(), key.to_vec(), "application/octet-stream");
    let collection_proxy = proxy(&conn, collection.as_str(), COLLECTION_IFACE)?;
    let (_item, prompt): (OwnedObjectPath, OwnedObjectPath) =
        collection_proxy.call("CreateItem", &(properties, secret, false)).map_err(dbus)?;
    if !run_prompt(&conn, &prompt, ask)? {
        return Err(Problem::Locked);
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
    /// own items so it never touches the real wrapping key. It locks the
    /// default keyring and leaves it locked, so run it in a throwaway session
    /// (with a short `XDG_RUNTIME_DIR` and an empty `XDG_DATA_HOME`):
    /// `dbus-run-session -- sh -c 'echo -n pw | gnome-keyring-daemon --unlock --components=secrets; gdbus call --session --dest org.freedesktop.secrets --object-path /org/freedesktop/secrets --method org.freedesktop.Secret.Service.SetAlias default /org/freedesktop/secrets/collection/login; cargo test -- --ignored secret_service'`.
    #[test]
    #[ignore = "needs a Secret Service on the session bus"]
    fn secret_service_round_trip() {
        let kind = "device-key-wrap-test";
        let first = wrapping_key(kind, false, false).expect("create the key without asking");
        let second = wrapping_key(kind, true, false).expect("read it back");
        assert_eq!(first, second, "the stored key is stable");
        assert_ne!(first, [0u8; KEY_LEN]);

        // A sealed key never gets a new wrapping key.
        assert_eq!(wrapping_key("device-key-wrap-absent", true, false), Err(Problem::Missing));

        let conn = Connection::session().unwrap();
        let service = proxy(&conn, SERVICE_PATH, SERVICE_IFACE).unwrap();

        // Locked: no prompt, no wait. A sealed key waits; an unsealed one starts plain.
        let collection: OwnedObjectPath = service.call("ReadAlias", &("default",)).unwrap();
        let _: (Vec<OwnedObjectPath>, OwnedObjectPath) =
            service.call("Lock", &(vec![collection.as_ref()],)).unwrap();
        assert_eq!(wrapping_key(kind, true, false), Err(Problem::Locked));
        assert_eq!(wrapping_key("device-key-wrap-absent", false, false), Err(Problem::Locked));
        assert_eq!(
            wrapping_key("device-key-wrap-absent", true, false),
            Err(Problem::Locked),
            "finding nothing in a locked keyring isn't proof the key is missing"
        );
        assert!(matches!(open(false, true), Ok(None)), "never asks before the key is sealed");
        assert!(is_locked(&conn, &collection).unwrap(), "a locked keyring stays locked without asking");

        // Best effort: the keyring is still locked, which is why this wants a throwaway session.
        let attributes: HashMap<&str, &str> = HashMap::from([("application", APPLICATION), ("kind", kind)]);
        let (unlocked, locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) =
            service.call("SearchItems", &(&attributes,)).unwrap();
        for item in unlocked.into_iter().chain(locked) {
            let item = proxy(&conn, item.as_str(), ITEM_IFACE).unwrap();
            let _ = item.call::<_, _, OwnedObjectPath>("Delete", &());
        }
    }
}
