// SPDX-License-Identifier: MPL-2.0
//! Biometric signing for Linux session unlock (docs/protocol/linux-unlock.md).

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use iroh::{
    TransportAddr,
    endpoint::{PathEvent, PathEventStream},
};
use n0_future::StreamExt;
use nectarlink_protocol::{
    DeviceId, Envelope, ErrorCode, LinuxUnlockChallenge,
    messages::{
        LinuxUnlockEnroll, LinuxUnlockEnrolled, LinuxUnlockForget, LinuxUnlockSigned, linux_unlock, types,
    },
};

use crate::{
    Error, Result,
    node::Shared,
    session::Session,
    unlock::{self, KeyRefusal, Requests},
};

pub const PC: &str = linux_unlock::CAP_PC;
pub const KEY: &str = linux_unlock::CAP_KEY;
pub const REQUEST_TTL: Duration = Duration::from_secs(30);
const ENROLL_TTL: Duration = unlock::ENROLL_TTL;

/// The names confirmed during enrollment and retained on the phone.
#[derive(Clone, PartialEq, Eq)]
pub struct Ask {
    pub pc_name: String,
    pub user_name: String,
    pub uid: u32,
}

impl std::fmt::Debug for Ask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ask").finish_non_exhaustive()
    }
}

struct ActivePrompt {
    peer: DeviceId,
    revoked: AtomicBool,
}

impl ActivePrompt {
    fn is_revoked(&self) -> bool {
        self.revoked.load(Ordering::Acquire)
    }
}

#[derive(Default)]
struct ActivityState {
    active: Weak<ActivePrompt>,
    cleanup: HashMap<DeviceId, Weak<()>>,
}

#[derive(Default)]
pub(crate) struct Activity {
    state: Mutex<ActivityState>,
    failures: Mutex<HashMap<DeviceId, Requests>>,
}

impl Activity {
    fn try_begin(&self, peer: DeviceId) -> Option<Arc<ActivePrompt>> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.active.strong_count() != 0
            || state.cleanup.get(&peer).is_some_and(|cleanup| cleanup.strong_count() != 0)
        {
            return None;
        }
        let current = Arc::new(ActivePrompt { peer, revoked: AtomicBool::new(false) });
        state.active = Arc::downgrade(&current);
        Some(current)
    }

    /// End an in-flight request and keep new requests out until all old key deletions finish.
    pub(crate) fn begin_cleanup(&self, peer: DeviceId) -> Arc<()> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(active) = state.active.upgrade()
            && active.peer == peer
        {
            active.revoked.store(true, Ordering::Release);
        }
        state.cleanup.retain(|_, cleanup| cleanup.strong_count() != 0);
        if let Some(cleanup) = state.cleanup.get(&peer).and_then(Weak::upgrade) {
            return cleanup;
        }
        let cleanup = Arc::new(());
        state.cleanup.insert(peer, Arc::downgrade(&cleanup));
        cleanup
    }

    pub(crate) fn clear_failures(&self, peer: &DeviceId) {
        self.failures.lock().unwrap_or_else(|e| e.into_inner()).remove(peer);
    }
}

fn has_local_path(shared: &Shared, session: &Session) -> bool {
    session
        .selected_ip_path()
        .is_some_and(|(addr, usb)| usb || shared.is_unlock_local_ip(&session.peer, addr.ip()))
}

async fn local_path_lost(shared: &Shared, session: &Session, mut events: PathEventStream) {
    while let Some(event) = events.next().await {
        match event {
            PathEvent::Selected { remote_addr, .. } => {
                if !matches!(remote_addr, TransportAddr::Ip(addr)
                    if session.is_usb_addr(&addr) || shared.is_unlock_local_ip(&session.peer, addr.ip()))
                {
                    return;
                }
            }
            PathEvent::Closed { .. } if !has_local_path(shared, session) => return,
            PathEvent::Lagged { .. } => return,
            _ => {}
        }
    }
}

fn check(shared: &Shared, session: &Session, local_cap: &str, peer_cap: &str) -> Result<()> {
    if !session.is_alive() {
        return Err(Error::Offline);
    }
    if !shared.store.is_paired(&session.peer)? {
        return Err(Error::NotPaired);
    }
    if !shared.we_offer(local_cap) || !shared.offers(&session.peer, peer_cap)? {
        return Err(Error::Unsupported);
    }
    if !shared.toggle_on(&session.peer, unlock::TOGGLE) || !has_local_path(shared, session) {
        return Err(Error::Denied);
    }
    Ok(())
}

async fn prompt(
    shared: &Shared,
    session: &Session,
    timeout: Duration,
    run: impl FnOnce(&ActivePrompt) -> Result<Vec<u8>, KeyRefusal> + Send + 'static,
) -> Result<Vec<u8>, Envelope> {
    let prompts = &shared.linux_unlock_activity;
    let current = prompts
        .try_begin(session.peer)
        .ok_or_else(|| Envelope::error(ErrorCode::Busy, "phone unlock prompt in progress"))?;
    let paths = session.path_events();
    check(shared, session, KEY, PC).map_err(gate_reply)?;
    if prompts
        .failures
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&session.peer)
        .is_some_and(|attempts| attempts.locked_out(Instant::now()).is_some())
    {
        return Err(Envelope::error(ErrorCode::Denied, "too many tries"));
    }

    // The platform owns the slot until it finishes, including after a timeout.
    let platform_current = current.clone();
    let platform_answer = async {
        tokio::time::timeout(
            timeout,
            tokio::task::spawn_blocking(move || {
                if platform_current.is_revoked() {
                    return Err(KeyRefusal::Declined);
                }
                run(&platform_current)
            }),
        )
        .await
    };
    let answer = tokio::select! {
        biased;
        _ = local_path_lost(shared, session, paths) => {
            current.revoked.store(true, Ordering::Release);
            return Err(gate_reply(Error::Denied));
        }
        answer = platform_answer => answer,
    };
    if current.is_revoked() {
        return Err(gate_reply(Error::Denied));
    }
    check(shared, session, KEY, PC).map_err(gate_reply)?;

    let mut failures = prompts.failures.lock().unwrap_or_else(|e| e.into_inner());
    match answer {
        Ok(Ok(Ok(bytes))) => {
            failures.remove(&session.peer);
            Ok(bytes)
        }
        Ok(Ok(Err(refusal @ (KeyRefusal::Unsupported | KeyRefusal::KeyGone)))) => Err(refusal.reply()),
        _ => {
            failures.entry(session.peer).or_default().failed(Instant::now());
            Err(KeyRefusal::Declined.reply())
        }
    }
}

fn gate_reply(error: Error) -> Envelope {
    match error {
        Error::Unsupported => Envelope::error(ErrorCode::Unsupported, "Linux phone unlock unavailable"),
        Error::NotPaired => Envelope::error(ErrorCode::Unpaired, "device is no longer paired"),
        _ => Envelope::error(ErrorCode::Denied, "Linux phone unlock is off or the connection is not local"),
    }
}

async fn respond(shared: &Shared, session: &Session, env: &Envelope) -> Result<Envelope, Envelope> {
    let (platform, peer) = (shared.platform.clone(), session.peer);
    if env.t == types::LINUX_UNLOCK_FORGET {
        if !shared.store.is_paired(&peer).unwrap_or(false) || !shared.offers(&peer, PC).unwrap_or(false) {
            return Err(gate_reply(Error::Denied));
        }
        let uid = match env.body() {
            Ok(LinuxUnlockForget::User { uid }) => Some(uid),
            Ok(LinuxUnlockForget::All) => None,
            Err(_) => return Err(Envelope::error(ErrorCode::BadMessage, "invalid Linux key removal")),
        };
        let cleanup = shared.linux_unlock_activity.begin_cleanup(peer);
        tokio::task::spawn_blocking(move || {
            let _cleanup = cleanup;
            platform.linux_unlock_forget(&peer, uid);
        })
        .await
        .map_err(|_| Envelope::error(ErrorCode::Internal, "Linux key removal failed"))?;
        return Ok(Envelope::empty(types::OK));
    }

    check(shared, session, KEY, PC).map_err(gate_reply)?;
    let invalid = || Envelope::error(ErrorCode::BadMessage, "invalid Linux phone unlock request");
    let encode_error = |_| Envelope::error(ErrorCode::Internal, "failed to encode Linux phone unlock reply");
    match env.t.as_str() {
        types::LINUX_UNLOCK_ENROLL => {
            let req: LinuxUnlockEnroll = env.body().map_err(|_| invalid())?;
            if !req.is_valid() || req.pc != peer || req.phone != shared.id {
                return Err(invalid());
            }
            let pc_name = unlock::shown_name(&shared.peer_name(&peer));
            let ask = Ask {
                pc_name: if pc_name.trim().is_empty() { peer.short() } else { pc_name },
                user_name: req.user_name,
                uid: req.uid,
            };
            let public_key = prompt(shared, session, ENROLL_TTL, move |current| {
                let key = platform.linux_unlock_enroll(&peer, &ask)?;
                if current.is_revoked() {
                    platform.linux_unlock_forget(&peer, Some(ask.uid));
                    return Err(KeyRefusal::Declined);
                }
                Ok(key)
            })
            .await?;
            let enrolled = LinuxUnlockEnrolled { public_key };
            if !enrolled.is_valid() {
                return Err(Envelope::error(ErrorCode::Internal, "invalid Linux public key"));
            }
            Envelope::new(types::LINUX_UNLOCK_ENROLLED, &enrolled).map_err(encode_error)
        }
        types::LINUX_UNLOCK_SIGN => {
            let req: LinuxUnlockChallenge = env.body().map_err(|_| invalid())?;
            if req.pc != peer || req.phone != shared.id {
                return Err(invalid());
            }
            let signature = prompt(shared, session, REQUEST_TTL, move |_| {
                platform.linux_unlock_sign(&peer, req.uid, &req.signing_bytes())
            })
            .await?;
            let signed = LinuxUnlockSigned { signature };
            if !signed.is_valid() {
                return Err(Envelope::error(ErrorCode::Internal, "invalid Linux signature"));
            }
            Envelope::new(types::LINUX_UNLOCK_SIGNED, &signed).map_err(encode_error)
        }
        _ => unreachable!("Linux phone unlock message was already matched"),
    }
}

pub(crate) async fn handle(shared: &Shared, session: &Session, env: &Envelope) -> Result<bool> {
    if !matches!(
        env.t.as_str(),
        types::LINUX_UNLOCK_ENROLL | types::LINUX_UNLOCK_SIGN | types::LINUX_UNLOCK_FORGET
    ) {
        return Ok(false);
    }
    if env.id.is_some() && env.re.is_none() {
        let reply = match respond(shared, session, env).await {
            Ok(reply) | Err(reply) => reply,
        };
        session.send(reply.reply_to(env.id)).await?;
    }
    Ok(true)
}

async fn request(shared: &Shared, session: &Session, env: Envelope, timeout: Duration) -> Result<Envelope> {
    let current = shared.linux_unlock_activity.try_begin(session.peer).ok_or(Error::Denied)?;
    let paths = session.path_events();
    check(shared, session, PC, KEY)?;
    let reply = tokio::select! {
        biased;
        _ = local_path_lost(shared, session, paths) => return Err(Error::Denied),
        reply = session.request(env, timeout) => reply?,
    };
    if current.is_revoked() {
        return Err(Error::Denied);
    }
    check(shared, session, PC, KEY)?;
    Ok(reply)
}

pub(crate) async fn enroll(shared: &Shared, session: &Session, uid: u32, user_name: &str) -> Result<Vec<u8>> {
    let req = LinuxUnlockEnroll { pc: shared.id, phone: session.peer, uid, user_name: user_name.to_owned() };
    if !req.is_valid() {
        return Err(Error::Protocol("invalid Linux account label".into()));
    }
    let reply =
        request(shared, session, Envelope::new(types::LINUX_UNLOCK_ENROLL, &req)?, ENROLL_TTL).await?;
    let enrolled: LinuxUnlockEnrolled = reply.expect_body(types::LINUX_UNLOCK_ENROLLED)?;
    if !enrolled.is_valid() {
        return Err(Error::Protocol("invalid Linux public key".into()));
    }
    Ok(enrolled.public_key)
}

pub(crate) async fn sign(
    shared: &Shared,
    session: &Session,
    challenge: LinuxUnlockChallenge,
) -> Result<Vec<u8>> {
    let reply =
        request(shared, session, Envelope::new(types::LINUX_UNLOCK_SIGN, &challenge)?, REQUEST_TTL).await?;
    let signed: LinuxUnlockSigned = reply.expect_body(types::LINUX_UNLOCK_SIGNED)?;
    if !signed.is_valid() {
        return Err(Error::Protocol("invalid Linux signature".into()));
    }
    Ok(signed.signature)
}

pub(crate) async fn forget(session: &Session, uid: Option<u32>) -> Result<()> {
    let req = uid.map_or(LinuxUnlockForget::All, |uid| LinuxUnlockForget::User { uid });
    let reply = session
        .request(Envelope::new(types::LINUX_UNLOCK_FORGET, &req)?, crate::session::REQUEST_TIMEOUT)
        .await?;
    Ok(reply.expect(types::OK)?)
}
