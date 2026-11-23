// SPDX-License-Identifier: MPL-2.0
//! Unlock the PC with the phone (docs/adr/0009-unlock-with-phone.md,
//! docs/protocol/unlock.md). The user's Windows password is kept only sealed
//! under a key `K` the PC doesn't keep: the phone holds `K`, wrapped by a
//! Keystore key that needs the owner's fingerprint for every use, and returns
//! it for one unlock at a time.
//!
//! Here: sealing and opening the password (AES-256-GCM, bound to the PC, the
//! phone and the Windows user), the rules for unlock requests (single use, a
//! short expiry, only the enrolled phone may answer, a lockout after repeated
//! failures), and the messages on both sides. Only on a direct local path,
//! and only with the `unlock` toggle on at both ends.

use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};

use nectarlink_protocol::{
    DeviceId, Envelope, ErrorCode,
    messages::{UnlockEnroll, UnlockForget, UnlockRelease, UnlockRequest, UnlockWrapped, types, unlock},
};
use ring::{
    aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey},
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Serialize};

use crate::{ConnectionPath, Error, Result, node::Shared, session::Session};

/// Bytes in `K`.
pub const KEY_LEN: usize = 32;
/// How long the phone has to answer a request.
pub const REQUEST_TTL: Duration = Duration::from_secs(30);
/// How long the phone's owner has to agree to enrolling.
pub const ENROLL_TTL: Duration = Duration::from_secs(90);
/// Capability a phone offers when it can hold a key behind a biometric.
pub const KEY: &str = unlock::CAP_KEY;
/// Capability a PC offers when it can unlock its locked session with a phone.
pub const PC: &str = unlock::CAP_PC;
pub const FROM_PHONE: &str = unlock::CAP_FROM_PHONE;
/// The per-device toggle, off by default on both sides.
pub const TOGGLE: &str = unlock::TOGGLE;
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

// ---- On the wire ----

/// What the phone shows its owner before using its key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    pub pc_name: String,
    pub user_name: String,
    /// The Windows user's security identifier.
    pub user: String,
}

/// Why the phone didn't wrap or unwrap a key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyRefusal {
    /// The owner said no, or didn't answer in time.
    #[error("declined on the phone")]
    Declined,
    /// No strong biometric set up, or no secure key storage.
    #[error("this phone can't keep the key behind a fingerprint")]
    Unsupported,
    /// The key is gone (a fingerprint was added, or the app's data was
    /// cleared): the PC must enroll again.
    #[error("the phone's key is gone; turn unlocking on again")]
    KeyGone,
}

impl KeyRefusal {
    pub(crate) fn reply(&self) -> Envelope {
        match self {
            KeyRefusal::Declined => Envelope::error(ErrorCode::Denied, "declined"),
            KeyRefusal::Unsupported => Envelope::error(ErrorCode::Unsupported, "no strong biometric"),
            KeyRefusal::KeyGone => Envelope::error(ErrorCode::NotFound, "key gone"),
        }
    }
}

/// Whether unlocking may happen over `session`: the toggle is on here for the
/// other device, and the two are on the same network right now.
fn allowed(shared: &Shared, session: &Session) -> bool {
    shared.toggle_on(&session.peer, TOGGLE) && on_local_network(shared, session)
}

/// Whether `session` runs over the USB cable or a direct path to an address on this
/// network (`Shared::is_unlock_local_ip`). Never a relay, a tailnet or an internet
/// address, even with away mode on, and not while no path is selected.
fn on_local_network(shared: &Shared, session: &Session) -> bool {
    if session.path() == ConnectionPath::Usb {
        return true;
    }
    session.selected_direct_addr().is_some_and(|addr| shared.is_unlock_local_ip(&session.peer, addr.ip()))
}

fn binding(pc: DeviceId, phone: DeviceId, user: &str) -> Binding {
    Binding { pc, phone, user: user.to_owned() }
}

/// The PC's own name, as the phone shows it.
fn pc_name(shared: &Shared) -> String {
    shown_name(&shared.local.read().unwrap_or_else(|e| e.into_inner()).device.name)
}

/// `name` without control characters, short enough for a prompt.
pub(crate) fn shown_name(name: &str) -> String {
    name.chars().filter(|c| !c.is_control()).take(unlock::MAX_NAME / 4).collect()
}

/// Handles `unlock.*` messages. Returns whether the message was one of them.
pub(crate) async fn handle(shared: &Arc<Shared>, session: &Arc<Session>, env: &Envelope) -> Result<bool> {
    let reply = match env.t.as_str() {
        // Phone side: wrap a key for this PC.
        types::UNLOCK_ENROLL => match env.body::<UnlockEnroll>() {
            Ok(enroll) if enroll.is_valid() => {
                if !allowed(shared, session) {
                    Envelope::error(ErrorCode::Denied, "unlocking is off")
                } else {
                    let ask = Ask {
                        pc_name: shown_name(&shared.peer_name(&session.peer)),
                        user_name: enroll.user_name,
                        user: enroll.user,
                    };
                    let key = Secret::new(enroll.key);
                    let (platform, pc) = (shared.platform.clone(), session.peer);
                    let wrapped = tokio::time::timeout(
                        ENROLL_TTL,
                        tokio::task::spawn_blocking(move || platform.unlock_wrap(&pc, &ask, key.expose())),
                    )
                    .await;
                    match wrapped {
                        Ok(Ok(Ok(wrapped)))
                            if !wrapped.is_empty() && wrapped.len() <= unlock::MAX_WRAPPED =>
                        {
                            Envelope::new(types::UNLOCK_WRAPPED, &UnlockWrapped { wrapped })?
                        }
                        Ok(Ok(Err(refusal))) => refusal.reply(),
                        _ => KeyRefusal::Declined.reply(),
                    }
                }
            }
            _ => Envelope::error(ErrorCode::BadMessage, "invalid unlock.enroll"),
        },
        // Phone side: release the key for one unlock.
        types::UNLOCK_REQUEST => match env.body::<UnlockRequest>() {
            Ok(request) if request.is_valid() => {
                let now = Instant::now();
                let prompts_locked = lock(&shared.unlock_prompts)
                    .get(&session.peer)
                    .is_some_and(|p| p.locked_out(now).is_some());
                if !allowed(shared, session) {
                    Envelope::error(ErrorCode::Denied, "unlocking is off")
                } else if prompts_locked {
                    // Too many prompts from this PC went unconfirmed: don't ask again for now.
                    Envelope::error(ErrorCode::Denied, "too many tries")
                } else {
                    // The phone names the PC as it knows it, not as the PC says.
                    let ask = Ask {
                        pc_name: shown_name(&shared.peer_name(&session.peer)),
                        user_name: request.user_name,
                        user: request.user,
                    };
                    let (platform, pc, wrapped) = (shared.platform.clone(), session.peer, request.wrapped);
                    let key = tokio::time::timeout(
                        REQUEST_TTL,
                        tokio::task::spawn_blocking(move || platform.unlock_unwrap(&pc, &ask, &wrapped)),
                    )
                    .await;
                    let mut prompts = lock(&shared.unlock_prompts);
                    let counted = prompts.entry(session.peer).or_default();
                    match key {
                        // Turned off, or off the network, while the owner was asked: the key
                        // stays here.
                        Ok(Ok(Ok(_))) if !allowed(shared, session) => {
                            Envelope::error(ErrorCode::Denied, "unlocking is off")
                        }
                        Ok(Ok(Ok(key))) if key.len() == unlock::KEY_LEN => {
                            counted.succeeded();
                            let key = Secret::new(key);
                            Envelope::new(
                                types::UNLOCK_RELEASE,
                                &UnlockRelease { request: request.request, key: key.expose().to_vec() },
                            )?
                        }
                        Ok(Ok(Err(KeyRefusal::Declined))) | Err(_) => {
                            counted.failed(Instant::now());
                            KeyRefusal::Declined.reply()
                        }
                        Ok(Ok(Err(refusal))) => refusal.reply(),
                        _ => KeyRefusal::Declined.reply(),
                    }
                }
            }
            _ => Envelope::error(ErrorCode::BadMessage, "invalid unlock.request"),
        },
        types::UNLOCK_FROM_PHONE => {
            if env.b.is_some() {
                Envelope::error(ErrorCode::BadMessage, "invalid unlock.from-phone")
            } else if !allowed(shared, session) || !shared.platform.unlock_from_phone(session.peer) {
                Envelope::error(ErrorCode::Denied, "unlock needs an enrolled phone and a locked session")
            } else {
                Envelope::empty(types::UNLOCK_FROM_PHONE)
            }
        }
        // Either side: the other one turned it off.
        types::UNLOCK_FORGET => {
            // No user means every user; a body that doesn't parse, or names a user
            // that isn't valid, means nothing.
            let Ok(forget) = env.body::<UnlockForget>() else {
                tracing::warn!(peer = %session.peer.short(), "ignored an unlock.forget that doesn't parse");
                return Ok(true);
            };
            if forget.user.as_deref().is_some_and(|u| !unlock::user_ok_pub(u)) {
                tracing::warn!(peer = %session.peer.short(), "ignored an unlock.forget for an invalid user");
                return Ok(true);
            }
            let user = forget.user.as_deref();
            let _ = shared.store.remove_unlock(&session.peer, user);
            let (platform, pc, user) = (shared.platform.clone(), session.peer, user.map(str::to_owned));
            let _ = tokio::task::spawn_blocking(move || platform.unlock_forget(&pc, user.as_deref())).await;
            return Ok(true);
        }
        _ => return Ok(false),
    };
    session.send(reply.reply_to(env.id)).await?;
    Ok(true)
}

pub(crate) async fn request_from_phone(shared: &Shared, session: &Session) -> Result<()> {
    if !allowed(shared, session) {
        return Err(Error::Denied);
    }
    if !shared.offers(&session.peer, FROM_PHONE).unwrap_or(false) {
        return Err(Error::Unsupported);
    }
    let reply =
        session.request(Envelope::empty(types::UNLOCK_FROM_PHONE), crate::session::REQUEST_TIMEOUT).await?;
    Ok(reply.expect(types::UNLOCK_FROM_PHONE)?)
}

/// PC side: enrolls `user` to unlock with `phone`. The phone's owner agrees
/// and confirms with a strong biometric; the password is then kept only
/// sealed under a key the phone holds.
pub(crate) async fn enroll(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    user: &str,
    user_name: &str,
    password: &Secret,
) -> Result<()> {
    if !shared.offers(&session.peer, KEY)? || !shared.we_offer(PC) {
        return Err(Error::Unsupported);
    }
    if !allowed(shared, session) {
        return Err(Error::Denied);
    }
    let key = new_key().map_err(|e| Error::Internal(e.to_string()))?;
    let enroll = UnlockEnroll {
        pc_name: pc_name(shared),
        user_name: user_name.to_owned(),
        user: user.to_owned(),
        key: key.expose().to_vec(),
    };
    if !enroll.is_valid() {
        return Err(Error::Protocol("invalid user for unlocking".into()));
    }
    let env = Envelope::new(types::UNLOCK_ENROLL, &enroll)?;
    drop(Secret::new(enroll.key));
    let reply = session.request(env, ENROLL_TTL + Duration::from_secs(5)).await?;
    let wrapped: UnlockWrapped = reply.expect_body(types::UNLOCK_WRAPPED)?;
    if !wrapped.is_valid() {
        return Err(Error::Protocol("invalid unlock.wrapped".into()));
    }
    let sealed = seal(password, &key, &binding(shared.id, session.peer, user))
        .map_err(|e| Error::Internal(e.to_string()))?;
    shared.store.save_unlock(
        &session.peer,
        &crate::store::UnlockEntry {
            user: user.to_owned(),
            user_name: user_name.to_owned(),
            sealed,
            wrapped: wrapped.wrapped,
            enrolled_at: crate::now_unix(),
        },
    )?;
    Ok(())
}

/// PC side: asks `phone` for the key to `user`'s password and opens it. The
/// phone's owner confirms with their biometric.
pub(crate) async fn password(shared: &Arc<Shared>, session: &Arc<Session>, user: &str) -> Result<Secret> {
    if !shared.offers(&session.peer, KEY)? || !shared.we_offer(PC) {
        return Err(Error::Unsupported);
    }
    // Not a no from the phone's owner, so the PC can go on to another phone
    // (#535): one with unlocking off can't, one away from this network is away.
    if !shared.toggle_on(&session.peer, TOGGLE) {
        return Err(Error::Unsupported);
    }
    if !on_local_network(shared, session) {
        return Err(Error::Offline);
    }
    let entry = shared.store.unlock_entry(&session.peer, user)?.ok_or(Error::NotFound)?;
    let id = lock(&shared.unlock_requests).begin(session.peer, user, Instant::now()).map_err(refused)?;
    let request = UnlockRequest {
        request: id.to_vec(),
        pc_name: pc_name(shared),
        user_name: entry.user_name.clone(),
        user: user.to_owned(),
        wrapped: entry.wrapped.clone(),
    };
    let reply = session.request(Envelope::new(types::UNLOCK_REQUEST, &request)?, REQUEST_TTL).await;
    let release: UnlockRelease = match reply.and_then(|r| Ok(r.expect_body(types::UNLOCK_RELEASE)?)) {
        Ok(release) => release,
        Err(e) => {
            // The request is used up; a key that's gone means enrolling again. A
            // declined or unanswered prompt counts toward the lockout, so the lock
            // screen can't keep asking the phone's owner.
            let now = Instant::now();
            let mut requests = lock(&shared.unlock_requests);
            let _ = requests.answer(&id, session.peer, now);
            if matches!(e, Error::Denied | Error::Timeout) {
                requests.failed(now);
            }
            return Err(e);
        }
    };
    let key = Secret::new(release.key);
    // Unlocking may have been turned off, the phone forgotten, or the network left
    // while the phone asked its owner: then the key opens nothing.
    let still_enrolled = shared
        .store
        .unlock_entry(&session.peer, user)?
        .is_some_and(|now| now.enrolled_at == entry.enrolled_at && now.wrapped == entry.wrapped);
    if !still_enrolled || !allowed(shared, session) {
        let _ = lock(&shared.unlock_requests).answer(&id, session.peer, Instant::now());
        return Err(Error::Denied);
    }
    let now = Instant::now();
    let mut requests = lock(&shared.unlock_requests);
    if release.request.as_slice() != id.as_slice() || key.expose().len() != KEY_LEN {
        requests.failed(now);
        return Err(Error::Protocol("invalid unlock.release".into()));
    }
    requests.answer(&id, session.peer, now).map_err(refused)?;
    match open(&entry.sealed, &key, &binding(shared.id, session.peer, user)) {
        Ok(password) => {
            requests.succeeded();
            Ok(password)
        }
        Err(_) => {
            requests.failed(now);
            Err(Error::Denied)
        }
    }
}

/// PC side: forgets `user` (or everyone) for `phone`, and tells the phone to
/// drop its keys if it's connected.
pub(crate) async fn forget(shared: &Arc<Shared>, phone: &DeviceId, user: Option<&str>) -> Result<()> {
    shared.store.remove_unlock(phone, user)?;
    if let Some(session) = shared.session(phone) {
        let forget = UnlockForget { user: user.map(str::to_owned) };
        let _ = session.send(Envelope::new(types::UNLOCK_FORGET, &forget)?).await;
    }
    Ok(())
}

fn refused(refused: Refused) -> Error {
    match refused {
        Refused::LockedOut { .. } | Refused::WrongDevice => Error::Denied,
        Refused::Expired => Error::Timeout,
        Refused::Unknown => Error::Protocol("unknown unlock request".into()),
        Refused::Random => Error::Internal("no secure random numbers".into()),
    }
}

fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
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
