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
pub type Attempt = tokio_util::sync::CancellationToken;

/// The names confirmed during enrollment and retained on the phone.
#[derive(Clone, PartialEq, Eq)]
pub struct Ask {
    pub pc_name: String,
    pub user_name: String,
    pub uid: u32,
    pub timeout: Duration,
}

impl std::fmt::Debug for Ask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ask").finish_non_exhaustive()
    }
}

pub(crate) struct ActivePrompt {
    peer: DeviceId,
    request: Option<(usize, u64)>,
    cancellation: Attempt,
    started: AtomicBool,
}

pub(crate) enum Incoming {
    Prompt { current: Arc<ActivePrompt>, paths: PathEventStream },
    Forget { uid: Option<u32>, cleanup: Arc<()> },
}

#[derive(Default)]
struct ActivityState {
    active: Weak<ActivePrompt>,
    outgoing: HashMap<DeviceId, Weak<ActivePrompt>>,
    cleanup: HashMap<DeviceId, Weak<()>>,
}

#[derive(Default)]
pub(crate) struct Activity {
    state: Mutex<ActivityState>,
    failures: Mutex<HashMap<DeviceId, Requests>>,
}

impl Activity {
    fn try_begin(&self, peer: DeviceId, request: Option<(usize, u64)>) -> Option<Arc<ActivePrompt>> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.cleanup.get(&peer).is_some_and(|cleanup| cleanup.strong_count() != 0) {
            return None;
        }
        let slot = if request.is_some() {
            &mut state.active
        } else {
            state.outgoing.retain(|_, prompt| prompt.strong_count() != 0);
            state.outgoing.entry(peer).or_default()
        };
        if slot.strong_count() != 0 {
            return None;
        }
        let current = Arc::new(ActivePrompt {
            peer,
            request,
            cancellation: Attempt::new(),
            started: AtomicBool::new(false),
        });
        *slot = Arc::downgrade(&current);
        Some(current)
    }

    pub(crate) fn cancel_request(&self, session: &Session, id: u64) {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(active) = state.active.upgrade()
            && active.peer == session.peer
            && active.request == Some((session.conn.stable_id(), id))
        {
            active.cancellation.cancel();
        }
    }

    /// End an in-flight request and keep new requests out until all old key deletions finish.
    pub(crate) fn begin_cleanup(&self, peer: DeviceId) -> Arc<()> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(active) = state.active.upgrade()
            && active.peer == peer
        {
            active.cancellation.cancel();
        }
        if let Some(outgoing) = state.outgoing.get(&peer).and_then(Weak::upgrade) {
            outgoing.cancellation.cancel();
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
    current: Arc<ActivePrompt>,
    paths: PathEventStream,
    timeout: Duration,
    run: impl FnOnce(&Attempt) -> Result<Vec<u8>, KeyRefusal> + Send + 'static,
) -> Result<Vec<u8>, Envelope> {
    let prompts = &shared.linux_unlock_activity;
    let cancel_on_exit = current.cancellation.clone().drop_guard();
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
                if platform_current.cancellation.is_cancelled() {
                    return Err(KeyRefusal::Declined);
                }
                platform_current.started.store(true, Ordering::Release);
                run(&platform_current.cancellation)
            }),
        )
        .await
    };
    let answer = tokio::select! {
        biased;
        _ = current.cancellation.cancelled() => None,
        _ = session.cancel.cancelled() => None,
        _ = local_path_lost(shared, session, paths) => None,
        answer = platform_answer => Some(answer),
    };
    if answer.is_none() || current.cancellation.is_cancelled() {
        current.cancellation.cancel();
        if current.started.load(Ordering::Acquire) {
            prompts
                .failures
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entry(session.peer)
                .or_default()
                .failed(Instant::now());
        }
        return Err(gate_reply(Error::Denied));
    }
    check(shared, session, KEY, PC).map_err(gate_reply)?;

    let mut failures = prompts.failures.lock().unwrap_or_else(|e| e.into_inner());
    match answer {
        Some(Ok(Ok(Ok(bytes)))) => {
            failures.remove(&session.peer);
            cancel_on_exit.disarm();
            Ok(bytes)
        }
        Some(Ok(Ok(Err(refusal @ (KeyRefusal::Unsupported | KeyRefusal::KeyGone))))) => Err(refusal.reply()),
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

pub(crate) fn prepare_background(
    shared: &Shared,
    session: &Session,
    env: &Envelope,
) -> Result<Option<Incoming>, Envelope> {
    let Some(id) = env.id.filter(|_| env.re.is_none()) else { return Ok(None) };
    let peer = session.peer;
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
        return Ok(Some(Incoming::Forget { uid, cleanup }));
    }
    if matches!(env.t.as_str(), types::LINUX_UNLOCK_ENROLL | types::LINUX_UNLOCK_SIGN) {
        let paths = session.path_events();
        check(shared, session, KEY, PC).map_err(gate_reply)?;
        // Reserve this slot before the reader can see a following cancel message.
        let current = shared
            .linux_unlock_activity
            .try_begin(peer, Some((session.conn.stable_id(), id)))
            .ok_or_else(|| Envelope::error(ErrorCode::Busy, "phone unlock prompt in progress"))?;
        return Ok(Some(Incoming::Prompt { current, paths }));
    }
    Ok(None)
}

async fn respond(
    shared: &Shared,
    session: &Session,
    env: &Envelope,
    incoming: Incoming,
) -> Result<Envelope, Envelope> {
    let (platform, peer) = (shared.platform.clone(), session.peer);
    let (current, paths) = match incoming {
        Incoming::Prompt { current, paths } => (current, paths),
        Incoming::Forget { uid, cleanup } => {
            tokio::task::spawn_blocking(move || {
                let _cleanup = cleanup;
                platform.linux_unlock_forget(&peer, uid);
            })
            .await
            .map_err(|_| Envelope::error(ErrorCode::Internal, "Linux key removal failed"))?;
            return Ok(Envelope::empty(types::OK));
        }
    };

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
            let deadline = Instant::now() + ENROLL_TTL;
            let public_key = prompt(shared, session, current, paths, ENROLL_TTL, move |attempt| {
                let timeout = deadline.saturating_duration_since(Instant::now());
                // Leave time to return the result after Android finishes committing the key.
                let timeout = timeout.checked_sub(Duration::from_secs(1)).ok_or(KeyRefusal::Declined)?;
                let ask = Ask {
                    pc_name: if pc_name.trim().is_empty() { peer.short() } else { pc_name },
                    user_name: req.user_name,
                    uid: req.uid,
                    timeout,
                };
                platform.linux_unlock_enroll(&peer, &ask, attempt)
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
            if req.uid == 0 || req.pc != peer || req.phone != shared.id {
                return Err(invalid());
            }
            let signature = prompt(shared, session, current, paths, REQUEST_TTL, move |attempt| {
                platform.linux_unlock_sign(&peer, req.uid, &req.signing_bytes(), attempt)
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

pub(crate) async fn handle(
    shared: &Shared,
    session: &Session,
    env: &Envelope,
    incoming: Option<Incoming>,
) -> Result<bool> {
    if env.t == types::LINUX_UNLOCK_CANCEL {
        if env.id.is_none()
            && env.re.is_none()
            && let Ok(id) = env.body::<u64>()
        {
            shared.linux_unlock_activity.cancel_request(session, id);
        }
        return Ok(true);
    }
    if !matches!(
        env.t.as_str(),
        types::LINUX_UNLOCK_ENROLL | types::LINUX_UNLOCK_SIGN | types::LINUX_UNLOCK_FORGET
    ) {
        return Ok(false);
    }
    if let Some(incoming) = incoming {
        let reply = match respond(shared, session, env, incoming).await {
            Ok(reply) | Err(reply) => reply,
        };
        session.send(reply.reply_to(env.id)).await?;
    }
    Ok(true)
}

async fn request(shared: &Shared, session: &Session, env: Envelope, timeout: Duration) -> Result<Envelope> {
    let current = shared.linux_unlock_activity.try_begin(session.peer, None).ok_or(Error::Denied)?;
    let paths = session.path_events();
    check(shared, session, PC, KEY)?;
    let reply = tokio::select! {
        biased;
        _ = current.cancellation.cancelled() => return Err(Error::Denied),
        _ = local_path_lost(shared, session, paths) => return Err(Error::Denied),
        reply = session.request(env, timeout) => reply?,
    };
    if current.cancellation.is_cancelled() {
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
    let timeout = ENROLL_TTL + Duration::from_secs(5);
    let reply = request(shared, session, Envelope::new(types::LINUX_UNLOCK_ENROLL, &req)?, timeout).await?;
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
    if challenge.uid == 0 {
        return Err(Error::Denied);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outgoing_attempts_only_serialize_against_the_same_phone() {
        let activity = Activity::default();
        let first = DeviceId([1; 32]);
        let second = DeviceId([2; 32]);
        let first_attempt = activity.try_begin(first, None).unwrap();
        assert!(activity.try_begin(first, None).is_none());
        let second_attempt = activity.try_begin(second, None).unwrap();
        let _cleanup = activity.begin_cleanup(first);
        assert!(first_attempt.cancellation.is_cancelled());
        assert!(!second_attempt.cancellation.is_cancelled());
    }
}
