// SPDX-License-Identifier: MPL-2.0
//! Phone as touchpad, keyboard and presentation remote (`docs/protocol/remote.md`).

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};

use bytes::Bytes;
use iroh::endpoint::{RecvStream, SendStream};
pub use nectarlink_protocol::messages::{
    ButtonAction, KeyMod, MouseButton, RemoteInput, SlideAction, remote_keys,
};
use nectarlink_protocol::{
    DeviceId, Envelope, ErrorCode,
    messages::{StreamHeader, remote, types},
    read_frame, write_frame,
};

use crate::{Error, NodeEvent, Result, node::Shared, session::Session};

/// Capability ID announced by a desktop that can inject mouse, keyboard and
/// presentation input (`docs/protocol/remote.md` §1).
pub const INPUT_INJECT: &str = "input.inject";

/// Per-device toggle name on the PC (off by default).
pub(crate) const TOGGLE: &str = "remote_input";

const TIMEOUT: Duration = Duration::from_secs(5);

/// Maximum rate of incoming datagrams (`move`, `scroll`, `laser`) per peer.
const DATAGRAM_RATE_PER_SEC: f64 = 240.0;
const DATAGRAM_BURST: f64 = 60.0;

/// Maximum rate of incoming control-stream `remote.input` requests per peer.
const CONTROL_RATE_PER_SEC: f64 = 60.0;
const CONTROL_BURST: f64 = 30.0;

/// Simple token bucket for per-peer input rate limiting.
#[derive(Debug, Clone)]
pub(crate) struct TokenBucket {
    tokens: f64,
    capacity: f64,
    rate_per_sec: f64,
    last: Instant,
}

impl TokenBucket {
    pub fn new(capacity: f64, rate_per_sec: f64, now: Instant) -> Self {
        Self { tokens: capacity, capacity, rate_per_sec, last: now }
    }

    pub fn take(&mut self, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + elapsed * self.rate_per_sec).min(self.capacity);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[derive(Debug)]
struct PeerLimiter {
    datagram: TokenBucket,
    control: TokenBucket,
}

impl PeerLimiter {
    fn new(now: Instant) -> Self {
        Self {
            datagram: TokenBucket::new(DATAGRAM_BURST, DATAGRAM_RATE_PER_SEC, now),
            control: TokenBucket::new(CONTROL_BURST, CONTROL_RATE_PER_SEC, now),
        }
    }
}

/// Per-node state for remote input: rate limiters and one-time prompt tracking.
#[derive(Debug, Default)]
pub(crate) struct RemoteState {
    peers: HashMap<DeviceId, PeerLimiter>,
    /// Peers that already triggered the one-time prompt while `remote_input`
    /// was off.
    prompted: HashSet<DeviceId>,
}

impl RemoteState {
    pub fn allow_datagram(&mut self, peer: DeviceId, now: Instant) -> bool {
        self.peers.entry(peer).or_insert_with(|| PeerLimiter::new(now)).datagram.take(now)
    }

    pub fn allow_control(&mut self, peer: DeviceId, now: Instant) -> bool {
        self.peers.entry(peer).or_insert_with(|| PeerLimiter::new(now)).control.take(now)
    }

    /// Records that `peer` asked for remote input while the toggle was off.
    /// Returns `true` the first time for that peer.
    pub fn mark_prompted(&mut self, peer: DeviceId) -> bool {
        self.prompted.insert(peer)
    }

    pub fn remove_peer(&mut self, peer: &DeviceId) {
        self.peers.remove(peer);
        self.prompted.remove(peer);
    }
}

fn peer_offers(shared: &Shared, peer: &DeviceId, cap: &str) -> Result<bool> {
    Ok(shared.store.get_peer(peer)?.is_some_and(|p| p.caps.contains(cap)))
}

fn we_offer(shared: &Shared, cap: &str) -> bool {
    shared.local_capabilities().iter().any(|c| c == cap)
}

fn check_allowed_with(
    shared: &Shared,
    peer: &DeviceId,
    allow_deck: bool,
) -> std::result::Result<(), (ErrorCode, &'static str)> {
    let supported =
        we_offer(shared, INPUT_INJECT) || (allow_deck && we_offer(shared, crate::deck::DECK_ACTIONS));
    if !supported {
        return Err((ErrorCode::Unsupported, "remote input is not supported here"));
    }
    if !shared.toggle_on(peer, TOGGLE) {
        let first_time = shared.remote.lock().unwrap_or_else(|e| e.into_inner()).mark_prompted(*peer);
        if first_time {
            shared.emit(NodeEvent::RemoteInputRequested { device: *peer });
        }
        return Err((ErrorCode::Denied, "remote input is turned off for this device"));
    }
    Ok(())
}

fn check_allowed(shared: &Shared, peer: &DeviceId) -> std::result::Result<(), (ErrorCode, &'static str)> {
    check_allowed_with(shared, peer, false)
}

/// Checks whether `peer` (a PC) currently accepts remote input from this phone.
pub(crate) async fn check(shared: &Arc<Shared>, session: &Arc<Session>) -> Result<()> {
    if !peer_offers(shared, &session.peer, INPUT_INJECT)?
        && !peer_offers(shared, &session.peer, crate::deck::DECK_ACTIONS)?
    {
        return Err(Error::Unsupported);
    }
    session.request(Envelope::empty(types::REMOTE_CHECK), TIMEOUT).await?.expect(types::OK)?;
    Ok(())
}

/// Sends a remote input event to `peer` (a PC).
///
/// Discrete actions travel on the control stream. If `input` is
/// [`RemoteInput::Move`], permission is checked first and the motion itself is
/// sent as a QUIC datagram, never on the control stream.
pub(crate) async fn input(shared: &Arc<Shared>, session: &Arc<Session>, event: RemoteInput) -> Result<()> {
    if !event.is_valid() {
        return Err(Error::Protocol("invalid remote input".into()));
    }
    if !peer_offers(shared, &session.peer, INPUT_INJECT)? {
        return Err(Error::Unsupported);
    }
    if let RemoteInput::Move { dx, dy } = event {
        check(shared, session).await?;
        return move_pointer(shared, session, dx, dy).await;
    }
    let env = Envelope::new(types::REMOTE_INPUT, &event)?;
    session.request(env, TIMEOUT).await?.expect(types::OK)?;
    Ok(())
}

/// Sends relative pointer motion `(dx, dy)` in a QUIC datagram (or a dedicated
/// `remote/motion` stream if datagrams are unavailable), never on the control
/// stream.
pub(crate) async fn move_pointer(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    dx: f32,
    dy: f32,
) -> Result<()> {
    let event = RemoteInput::Move { dx, dy };
    if !event.is_valid() {
        return Err(Error::Protocol("pointer motion out of bounds".into()));
    }
    send_datagram(shared, session, &event).await
}

/// Sends smooth scroll `(dx, dy)` in a QUIC datagram.
pub(crate) async fn scroll_fast(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    dx: f32,
    dy: f32,
) -> Result<()> {
    let event = RemoteInput::Scroll { dx, dy };
    if !event.is_valid() {
        return Err(Error::Protocol("scroll delta out of bounds".into()));
    }
    send_datagram(shared, session, &event).await
}

/// Sends a laser pointer update. While `on` is `true`, positions travel in
/// QUIC datagrams; when `on` is `false`, a datagram and a control-stream
/// `remote.input` are both sent so the dot reliably hides.
pub(crate) async fn laser(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    on: bool,
    x: f32,
    y: f32,
) -> Result<()> {
    let event = RemoteInput::Laser { on, x, y };
    if !event.is_valid() {
        return Err(Error::Protocol("laser coordinates out of bounds".into()));
    }
    if on {
        send_datagram(shared, session, &event).await
    } else {
        let _ = send_datagram(shared, session, &event).await;
        input(shared, session, event).await
    }
}

async fn send_datagram(shared: &Arc<Shared>, session: &Arc<Session>, event: &RemoteInput) -> Result<()> {
    if !peer_offers(shared, &session.peer, INPUT_INJECT)? {
        return Err(Error::Unsupported);
    }
    let bytes = Bytes::from(event.to_datagram());
    if session.send_datagram(bytes).is_ok() {
        return Ok(());
    }
    // Fallback if QUIC datagrams are disabled on this transport: open a
    // dedicated stream (still never the control stream).
    let (mut send, _recv) = session.open_bi().await?;
    let header =
        StreamHeader { svc: remote::SERVICE.into(), op: remote::OP_MOTION.into(), v: remote::VERSION };
    let header = Envelope::new(types::STREAM, &header)?;
    write_frame(&mut send, &header.to_cbor()).await?;
    let frame = Envelope::new(types::REMOTE_INPUT, event)?;
    write_frame(&mut send, &frame.to_cbor()).await?;
    let _ = send.finish();
    Ok(())
}

/// Handles incoming `remote.*` requests on the control stream.
pub(crate) async fn handle(shared: &Arc<Shared>, session: &Arc<Session>, env: &Envelope) -> Result<bool> {
    let reply = match env.t.as_str() {
        types::REMOTE_CHECK => match check_allowed_with(shared, &session.peer, true) {
            Ok(()) => Envelope::empty(types::OK),
            Err((code, msg)) => Envelope::error(code, msg),
        },
        types::REMOTE_INPUT => match check_allowed(shared, &session.peer) {
            Err((code, msg)) => Envelope::error(code, msg),
            Ok(()) => match env.body::<RemoteInput>() {
                Ok(event) if event.is_control_allowed() && event.is_valid() => {
                    let allowed = shared
                        .remote
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .allow_control(session.peer, Instant::now());
                    if !allowed {
                        Envelope::error(ErrorCode::Denied, "rate limit exceeded")
                    } else {
                        let platform = shared.platform.clone();
                        let peer = session.peer;
                        match tokio::task::spawn_blocking(move || platform.remote_input(&peer, event)).await {
                            Ok(Ok(())) => Envelope::empty(types::OK),
                            Ok(Err(e)) => Envelope::error(ErrorCode::Internal, e),
                            Err(_) => Envelope::error(ErrorCode::Internal, "input task failed"),
                        }
                    }
                }
                _ => Envelope::error(ErrorCode::BadMessage, "invalid remote input"),
            },
        },
        _ => return Ok(false),
    };
    session.send(reply.reply_to(env.id)).await?;
    Ok(true)
}

/// Handles an incoming QUIC datagram from `peer`.
pub(crate) fn handle_datagram(shared: &Arc<Shared>, peer: DeviceId, bytes: &[u8]) {
    if check_allowed(shared, &peer).is_err() {
        return;
    }
    let Ok(event) = RemoteInput::from_datagram(bytes) else {
        return;
    };
    let allowed =
        shared.remote.lock().unwrap_or_else(|e| e.into_inner()).allow_datagram(peer, Instant::now());
    if !allowed {
        return;
    }
    let _ = shared.platform.remote_input(&peer, event);
}

/// Handles a fallback `remote/motion` stream when QUIC datagrams aren't used.
pub(crate) async fn receive_motion(
    shared: Arc<Shared>,
    peer: DeviceId,
    mut send: SendStream,
    mut recv: RecvStream,
) {
    while let Ok(Some(bytes)) = read_frame(&mut recv).await {
        if check_allowed(&shared, &peer).is_err() {
            break;
        }
        let Ok(env) = Envelope::from_cbor(&bytes) else { break };
        let Ok(event) = env.expect_body::<RemoteInput>(types::REMOTE_INPUT) else { break };
        if !event.is_datagram_allowed() || !event.is_valid() {
            break;
        }
        let allowed =
            shared.remote.lock().unwrap_or_else(|e| e.into_inner()).allow_datagram(peer, Instant::now());
        if allowed {
            let _ = shared.platform.remote_input(&peer, event);
        }
    }
    let _ = send.finish();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_bucket_enforces_burst_and_refills() {
        let start = Instant::now();
        let mut b = TokenBucket::new(3.0, 10.0, start);
        assert!(b.take(start));
        assert!(b.take(start));
        assert!(b.take(start));
        assert!(!b.take(start), "burst exhausted");

        // 100 ms later at 10/s refills 1 token.
        let later = start + Duration::from_millis(100);
        assert!(b.take(later));
        assert!(!b.take(later));
    }

    #[test]
    fn prompt_is_emitted_once_per_peer() {
        let mut state = RemoteState::default();
        let a = DeviceId([1; 32]);
        let b = DeviceId([2; 32]);
        assert!(state.mark_prompted(a));
        assert!(!state.mark_prompted(a), "second ask from same peer does not re-prompt");
        assert!(state.mark_prompted(b));
        state.remove_peer(&a);
        assert!(state.mark_prompted(a), "unpairing resets prompt state");
    }
}
