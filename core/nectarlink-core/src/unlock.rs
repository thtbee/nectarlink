// SPDX-License-Identifier: MPL-2.0
//! Unlock the PC with the phone, the PC's half (docs/adr/0009-unlock-with-phone.md;
//! proposed, not wired up). The user's Windows password is kept only sealed
//! under a key `K` the PC doesn't keep: the phone holds `K`, wrapped by a
//! Keystore key that needs the owner's fingerprint for every use, and returns
//! it for one unlock at a time.
//!
//! This module is the part that can be tested on its own: sealing and opening
//! the password (AES-256-GCM, bound to the PC, the phone and the Windows user),
//! and the rules for unlock requests: single use, a short expiry, only the
//! enrolled phone may answer, and a lockout after repeated failures.

use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

use nectarlink_protocol::DeviceId;
use ring::{
    aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey},
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Serialize};

/// Bytes in `K`.
pub const KEY_LEN: usize = 32;
/// How long the phone has to answer a request.
pub const REQUEST_TTL: Duration = Duration::from_secs(30);
/// Failed unlocks allowed within [`FAILURE_WINDOW`] before a lockout.
pub const MAX_FAILURES: usize = 5;
pub const FAILURE_WINDOW: Duration = Duration::from_secs(15 * 60);
pub const LOCKOUT: Duration = Duration::from_secs(15 * 60);
/// Separates this use of the key from any other.
const DOMAIN: &[u8] = b"nectarlink-unlock-v1\0";

/// Secret bytes (`K`, or the password): overwritten when dropped, never printed.
pub struct Secret(Vec<u8>);

impl Secret {
    pub fn new(bytes: Vec<u8>) -> Secret {
        Secret(bytes)
    }

    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.0.fill(0);
        // Best effort without unsafe code: black_box keeps the wipe from being
        // optimized away as a store nobody reads.
        std::hint::black_box(&self.0);
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Secret({} bytes)", self.0.len())
    }
}

/// What a sealed password belongs to: opening it for any other PC, phone or
/// user fails, even with the right key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub pc: DeviceId,
    pub phone: DeviceId,
    /// The Windows user's security identifier (`S-1-5-21-…`).
    pub user: String,
}

impl Binding {
    fn aad(&self) -> Vec<u8> {
        let user = self.user.as_bytes();
        let mut aad = Vec::with_capacity(DOMAIN.len() + 64 + 4 + user.len());
        aad.extend_from_slice(DOMAIN);
        aad.extend_from_slice(&self.pc.0);
        aad.extend_from_slice(&self.phone.0);
        aad.extend_from_slice(&(user.len() as u32).to_be_bytes());
        aad.extend_from_slice(user);
        aad
    }
}

/// The password, sealed under `K`: what the PC keeps on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sealed {
    pub nonce: [u8; NONCE_LEN],
    pub ciphertext: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VaultError {
    #[error("no secure random numbers")]
    Random,
    #[error("the key isn't 32 bytes")]
    KeyLength,
    /// The key is wrong, or the sealed password was changed, or it belongs
    /// to another PC, phone or user.
    #[error("can't open the sealed password")]
    Open,
}

/// A new random `K`, for enrolling.
pub fn new_key() -> Result<Secret, VaultError> {
    let mut key = vec![0u8; KEY_LEN];
    SystemRandom::new().fill(&mut key).map_err(|_| VaultError::Random)?;
    Ok(Secret(key))
}

fn aead_key(key: &Secret) -> Result<LessSafeKey, VaultError> {
    if key.expose().len() != KEY_LEN {
        return Err(VaultError::KeyLength);
    }
    let unbound = UnboundKey::new(&AES_256_GCM, key.expose()).map_err(|_| VaultError::KeyLength)?;
    Ok(LessSafeKey::new(unbound))
}

/// Seals `password` under `key` for `binding`, with a fresh random nonce.
pub fn seal(password: &Secret, key: &Secret, binding: &Binding) -> Result<Sealed, VaultError> {
    let aead = aead_key(key)?;
    let mut nonce = [0u8; NONCE_LEN];
    SystemRandom::new().fill(&mut nonce).map_err(|_| VaultError::Random)?;
    let mut buffer = password.expose().to_vec();
    let sealed = aead.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(nonce),
        Aad::from(binding.aad()),
        &mut buffer,
    );
    if sealed.is_err() {
        // Don't leave the plaintext copy behind.
        drop(Secret(buffer));
        return Err(VaultError::Open);
    }
    Ok(Sealed { nonce, ciphertext: buffer })
}

/// Opens `sealed` with `key` for `binding`.
pub fn open(sealed: &Sealed, key: &Secret, binding: &Binding) -> Result<Secret, VaultError> {
    let aead = aead_key(key)?;
    let mut buffer = sealed.ciphertext.clone();
    let opened =
        aead.open_in_place(Nonce::assume_unique_for_key(sealed.nonce), Aad::from(binding.aad()), &mut buffer);
    match opened {
        Ok(plain) => {
            let len = plain.len();
            buffer.truncate(len);
            Ok(Secret(buffer))
        }
        Err(_) => {
            drop(Secret(buffer));
            Err(VaultError::Open)
        }
    }
}

// ---- Requests ----

/// One unlock request's ID: random, so it can't be guessed.
pub type RequestId = [u8; 16];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub phone: DeviceId,
    pub user: String,
    pub expires: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// Too many failures; unlocking with the phone works again at `until`.
    LockedOut {
        until: Instant,
    },
    /// Not a request this PC made, or one already answered.
    Unknown,
    Expired,
    /// An answer from a device other than the one asked.
    WrongDevice,
    Random,
}

/// The PC's unlock requests and its record of failures.
#[derive(Debug, Default)]
pub struct Requests {
    pending: HashMap<RequestId, Request>,
    failures: VecDeque<Instant>,
    locked_until: Option<Instant>,
}

impl Requests {
    pub fn new() -> Requests {
        Requests::default()
    }

    /// Whether unlocking with the phone is locked out at `now`, and until when.
    pub fn locked_out(&self, now: Instant) -> Option<Instant> {
        self.locked_until.filter(|until| now < *until)
    }

    /// Starts a request to `phone` for `user`, unless locked out.
    pub fn begin(&mut self, phone: DeviceId, user: &str, now: Instant) -> Result<RequestId, Refused> {
        if let Some(until) = self.locked_out(now) {
            return Err(Refused::LockedOut { until });
        }
        self.pending.retain(|_, r| now < r.expires);
        let mut id = [0u8; 16];
        SystemRandom::new().fill(&mut id).map_err(|_| Refused::Random)?;
        self.pending.insert(id, Request { phone, user: user.to_owned(), expires: now + REQUEST_TTL });
        Ok(id)
    }

    /// Takes the answer to `id`, from `from`: each request can be answered
    /// once, before it expires, and only by the phone it was sent to. What it
    /// returns is then opened with the vault; report how that went with
    /// [`succeeded`](Self::succeeded) or [`failed`](Self::failed).
    pub fn answer(&mut self, id: &RequestId, from: DeviceId, now: Instant) -> Result<Request, Refused> {
        if let Some(until) = self.locked_out(now) {
            self.pending.remove(id);
            return Err(Refused::LockedOut { until });
        }
        let request = self.pending.remove(id).ok_or(Refused::Unknown)?;
        if now >= request.expires {
            return Err(Refused::Expired);
        }
        if request.phone != from {
            self.failed(now);
            return Err(Refused::WrongDevice);
        }
        Ok(request)
    }

    /// An unlock failed (the key didn't open the password, or a wrong device
    /// answered). Too many within the window lock it out for a while.
    pub fn failed(&mut self, now: Instant) {
        while self.failures.front().is_some_and(|at| now.duration_since(*at) >= FAILURE_WINDOW) {
            self.failures.pop_front();
        }
        self.failures.push_back(now);
        if self.failures.len() >= MAX_FAILURES {
            self.locked_until = Some(now + LOCKOUT);
            self.failures.clear();
            self.pending.clear();
        }
    }

    /// An unlock worked: earlier failures no longer count.
    pub fn succeeded(&mut self) {
        self.failures.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding() -> Binding {
        Binding { pc: DeviceId([1; 32]), phone: DeviceId([2; 32]), user: "S-1-5-21-1-2-3-1001".into() }
    }

    fn password() -> Secret {
        Secret::new(b"correct horse battery staple".to_vec())
    }

    #[test]
    fn a_sealed_password_opens_only_with_its_key_and_binding() {
        let key = new_key().unwrap();
        let sealed = seal(&password(), &key, &binding()).unwrap();
        assert_ne!(sealed.ciphertext.as_slice(), password().expose(), "not stored in the clear");
        assert_eq!(open(&sealed, &key, &binding()).unwrap().expose(), password().expose());

        let other_key = new_key().unwrap();
        assert_eq!(open(&sealed, &other_key, &binding()).unwrap_err(), VaultError::Open);
        for other in [
            Binding { pc: DeviceId([9; 32]), ..binding() },
            Binding { phone: DeviceId([9; 32]), ..binding() },
            Binding { user: "S-1-5-21-1-2-3-1002".into(), ..binding() },
        ] {
            assert_eq!(open(&sealed, &key, &other).unwrap_err(), VaultError::Open, "{other:?}");
        }
    }

    #[test]
    fn a_changed_sealed_password_doesnt_open() {
        let key = new_key().unwrap();
        let mut sealed = seal(&password(), &key, &binding()).unwrap();
        sealed.ciphertext[0] ^= 1;
        assert_eq!(open(&sealed, &key, &binding()).unwrap_err(), VaultError::Open);
        let mut sealed = seal(&password(), &key, &binding()).unwrap();
        sealed.nonce[0] ^= 1;
        assert_eq!(open(&sealed, &key, &binding()).unwrap_err(), VaultError::Open);
    }

    #[test]
    fn each_seal_uses_a_new_nonce_and_keys_must_be_32_bytes() {
        let key = new_key().unwrap();
        let a = seal(&password(), &key, &binding()).unwrap();
        let b = seal(&password(), &key, &binding()).unwrap();
        assert_ne!(a.nonce, b.nonce);
        assert_ne!(a.ciphertext, b.ciphertext);
        assert_eq!(
            seal(&password(), &Secret::new(vec![0; 16]), &binding()).unwrap_err(),
            VaultError::KeyLength
        );
    }

    #[test]
    fn secrets_are_never_printed() {
        assert_eq!(format!("{:?}", password()), "Secret(28 bytes)");
    }

    #[test]
    fn a_request_is_answered_once_in_time_by_its_phone() {
        let now = Instant::now();
        let phone = DeviceId([2; 32]);
        let mut requests = Requests::new();
        let id = requests.begin(phone, "S-1", now).unwrap();
        let request = requests.answer(&id, phone, now + Duration::from_secs(5)).unwrap();
        assert_eq!((request.phone, request.user.as_str()), (phone, "S-1"));
        assert_eq!(requests.answer(&id, phone, now).unwrap_err(), Refused::Unknown, "only once");

        let late = requests.begin(phone, "S-1", now).unwrap();
        assert_eq!(requests.answer(&late, phone, now + REQUEST_TTL).unwrap_err(), Refused::Expired);

        let stolen = requests.begin(phone, "S-1", now).unwrap();
        assert_eq!(requests.answer(&stolen, DeviceId([7; 32]), now).unwrap_err(), Refused::WrongDevice);
        assert_eq!(
            requests.answer(&stolen, phone, now).unwrap_err(),
            Refused::Unknown,
            "used up by the wrong device"
        );

        assert_eq!(requests.answer(&[0; 16], phone, now).unwrap_err(), Refused::Unknown);
    }

    #[test]
    fn repeated_failures_lock_it_out_for_a_while() {
        let start = Instant::now();
        let phone = DeviceId([2; 32]);
        let mut requests = Requests::new();
        for i in 0..MAX_FAILURES - 1 {
            requests.failed(start + Duration::from_secs(i as u64));
        }
        assert!(requests.begin(phone, "S-1", start).is_ok(), "not yet");
        let pending = requests.begin(phone, "S-1", start).unwrap();
        requests.failed(start + Duration::from_secs(10));
        let at = start + Duration::from_secs(11);
        assert!(matches!(requests.begin(phone, "S-1", at), Err(Refused::LockedOut { .. })));
        assert!(
            matches!(requests.answer(&pending, phone, at), Err(Refused::LockedOut { .. })),
            "pending requests are dropped too"
        );
        let after = start + Duration::from_secs(10) + LOCKOUT;
        assert!(requests.begin(phone, "S-1", after).is_ok(), "works again after the lockout");
    }

    #[test]
    fn old_failures_and_a_success_dont_count() {
        let start = Instant::now();
        let phone = DeviceId([2; 32]);
        let mut requests = Requests::new();
        for i in 0..MAX_FAILURES - 1 {
            requests.failed(start + Duration::from_secs(i as u64));
        }
        // Long after: the earlier ones fell out of the window.
        requests.failed(start + FAILURE_WINDOW + Duration::from_secs(60));
        assert!(requests.begin(phone, "S-1", start + FAILURE_WINDOW + Duration::from_secs(61)).is_ok());

        let mut requests = Requests::new();
        for i in 0..MAX_FAILURES - 1 {
            requests.failed(start + Duration::from_secs(i as u64));
        }
        requests.succeeded();
        requests.failed(start + Duration::from_secs(10));
        assert!(requests.begin(phone, "S-1", start + Duration::from_secs(11)).is_ok());
    }
}
