// SPDX-License-Identifier: MPL-2.0
//! AudioLink: managed audio routing between paired devices and virtual
//! microphone status (`docs/protocol/audio-route.md`).
//!
//! A controller sends each device an `audio.route.set` snapshot for one
//! `generation`. Sources then send managed PCM (an `NLRP` datagram, or an
//! `audio.route.pcm` envelope when the frame doesn't fit), and sinks report
//! `audio.route.levels`. A receiver takes PCM only for its current
//! generation, from a source enrolled in one of its lanes, and only from that
//! source itself.
//!
//! Everything needs the `media` toggle; a phone's own audio needs
//! `mirroring` too. `mic.status` and `mic.setup` need the `mic` toggle. The
//! legacy paths (`NLPA`, `audio.pcm`, `pc.audio.set`, `mic/pcm`,
//! `mirror/audio`) are untouched.

use std::sync::Arc;

use nectarlink_protocol::{
    DeviceId, Envelope, ErrorCode,
    messages::{DeviceKind, NLRP_MAGIC, types},
};

pub use nectarlink_protocol::messages::{
    AudioLaneLevel, AudioLatencyPreset, AudioPlaybackPolicy, AudioRouteLane, AudioRouteLevels, AudioRoutePcm,
    AudioRouteRole, AudioRouteSet, AudioRouteStatus, CAP_AUDIO_ROUTE_CONTROLLER as AUDIO_ROUTE_CONTROLLER,
    CAP_AUDIO_ROUTE_SINK as AUDIO_ROUTE_SINK, CAP_AUDIO_ROUTE_SOURCE as AUDIO_ROUTE_SOURCE,
    CAP_MIC_STATUS as MIC_STATUS, MicSetupResponse, MicStatus,
};

use crate::{Error, Result, events::NodeEvent, node::Shared, session::Session};

/// The toggle every managed route message needs.
const MEDIA: &str = crate::media::TOGGLE;
/// The extra toggle for a phone's own audio in a route.
const MIRRORING: &str = "mirroring";

/// The largest frame sent as an `audio.route.pcm` envelope when it doesn't
/// fit a datagram: about 85 ms of 48 kHz stereo. Bigger frames are refused so
/// audio never crowds the control stream.
pub const MAX_FALLBACK_PCM_BYTES: usize = 16 * 1024;

/// The route a controller last set on this device.
#[derive(Debug, Clone)]
pub(crate) struct ActiveRoute {
    /// Who set it: this device itself for a local controller.
    pub controller: DeviceId,
    pub set: AudioRouteSet,
}

/// [`AudioRouteLevels`] with exact (bitwise) float equality, so it can ride in
/// [`NodeEvent`].
#[derive(Debug, Clone)]
pub struct RouteLevels(pub AudioRouteLevels);

impl PartialEq for RouteLevels {
    fn eq(&self, other: &Self) -> bool {
        self.0.generation == other.0.generation
            && self.0.levels.len() == other.0.levels.len()
            && self.0.levels.iter().zip(&other.0.levels).all(|(a, b)| {
                a.source_id == b.source_id && a.peak.to_bits() == b.peak.to_bits() && a.active == b.active
            })
    }
}

impl Eq for RouteLevels {}

impl std::ops::Deref for RouteLevels {
    type Target = AudioRouteLevels;
    fn deref(&self) -> &AudioRouteLevels {
        &self.0
    }
}

/// The capability this device needs for `role`, if any.
fn role_capability(role: AudioRouteRole) -> Option<&'static str> {
    match role {
        AudioRouteRole::Idle => None,
        AudioRouteRole::Source => Some(AUDIO_ROUTE_SOURCE),
        AudioRouteRole::Sink | AudioRouteRole::PhonePlayback => Some(AUDIO_ROUTE_SINK),
    }
}

fn is_phone_kind(kind: DeviceKind) -> bool {
    matches!(kind, DeviceKind::Phone | DeviceKind::Tablet)
}

fn local_is_phone(shared: &Shared) -> bool {
    is_phone_kind(shared.local.read().unwrap_or_else(|e| e.into_inner()).device.kind)
}

fn peer_is_phone(shared: &Shared, peer: &DeviceId) -> bool {
    shared.store.get_peer(peer).ok().flatten().is_some_and(|p| is_phone_kind(p.info.kind))
}

fn peer_offers(shared: &Shared, peer: &DeviceId, cap: &str) -> bool {
    shared.offers(peer, cap).unwrap_or(false)
}

fn lock_route(shared: &Shared) -> std::sync::MutexGuard<'_, Option<ActiveRoute>> {
    shared.audio_route.lock().unwrap_or_else(|e| e.into_inner())
}

/// Cuts `text` to at most `max` bytes on a character boundary.
fn bounded(mut text: String, max: usize) -> String {
    if text.len() > max {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

// ---- Sending ----

/// Sends `set` to `peer` and returns the status it answers with.
pub(crate) async fn set_route(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    set: AudioRouteSet,
) -> Result<AudioRouteStatus> {
    if !set.is_valid() {
        return Err(Error::Protocol("invalid audio route".into()));
    }
    if !shared.toggle_on(&session.peer, MEDIA) {
        return Err(Error::Denied);
    }
    if !shared.we_offer(AUDIO_ROUTE_CONTROLLER)
        || role_capability(set.role).is_some_and(|cap| !peer_offers(shared, &session.peer, cap))
    {
        return Err(Error::Unsupported);
    }
    let status: AudioRouteStatus = session
        .request(Envelope::new(types::AUDIO_ROUTE_SET, &set)?, crate::session::REQUEST_TIMEOUT)
        .await?
        .expect_body(types::AUDIO_ROUTE_STATUS)?;
    if !status.is_valid() {
        return Err(Error::Protocol("invalid audio route status".into()));
    }
    Ok(status)
}

/// Sends one managed frame: a datagram when it fits, else a bounded envelope
/// that is dropped rather than queued behind control messages.
pub(crate) fn send_pcm(shared: &Shared, session: &Session, pcm: AudioRoutePcm) -> Result<()> {
    if !pcm.is_valid() {
        return Err(Error::Protocol("invalid managed audio frame".into()));
    }
    if !shared.toggle_on(&session.peer, MEDIA) {
        return Err(Error::Denied);
    }
    if !shared.we_offer(AUDIO_ROUTE_SOURCE) || !peer_offers(shared, &session.peer, AUDIO_ROUTE_SINK) {
        return Err(Error::Unsupported);
    }
    let datagram = pcm.to_datagram()?;
    if session.conn.max_datagram_size().is_some_and(|max| datagram.len() <= max)
        && session.send_datagram(bytes::Bytes::from(datagram)).is_ok()
    {
        return Ok(());
    }
    if pcm.pcm.len() > MAX_FALLBACK_PCM_BYTES {
        return Err(Error::Protocol("managed audio frame too large for the control stream".into()));
    }
    session.try_send_realtime(Envelope::new(types::AUDIO_ROUTE_PCM, &pcm)?)
}

/// Sends a sink's lane levels to its controller (lossy, like audio).
pub(crate) fn send_levels(shared: &Shared, session: &Session, levels: &AudioRouteLevels) -> Result<()> {
    if !levels.is_valid() {
        return Err(Error::Protocol("invalid audio route levels".into()));
    }
    if !shared.toggle_on(&session.peer, MEDIA) {
        return Err(Error::Denied);
    }
    if !shared.we_offer(AUDIO_ROUTE_SINK) || !peer_offers(shared, &session.peer, AUDIO_ROUTE_CONTROLLER) {
        return Err(Error::Unsupported);
    }
    session.try_send_realtime(Envelope::new(types::AUDIO_ROUTE_LEVELS, levels)?)
}

/// Tells a controller this device's route changed (`audio.route.status`
/// without a request).
pub(crate) async fn send_status(shared: &Shared, session: &Session, status: &AudioRouteStatus) -> Result<()> {
    if !status.is_valid() {
        return Err(Error::Protocol("invalid audio route status".into()));
    }
    if !shared.toggle_on(&session.peer, MEDIA) {
        return Err(Error::Denied);
    }
    if !peer_offers(shared, &session.peer, AUDIO_ROUTE_CONTROLLER) {
        return Err(Error::Unsupported);
    }
    session.send(Envelope::new(types::AUDIO_ROUTE_STATUS, status)?).await
}

/// Asks a PC for its virtual microphone readiness.
pub(crate) async fn request_mic_status(shared: &Shared, session: &Session) -> Result<MicStatus> {
    if !shared.toggle_on(&session.peer, crate::mic::TOGGLE) {
        return Err(Error::Denied);
    }
    if !peer_offers(shared, &session.peer, MIC_STATUS) {
        return Err(Error::Unsupported);
    }
    let status: MicStatus = session
        .request(Envelope::empty(types::MIC_STATUS), crate::session::REQUEST_TIMEOUT)
        .await?
        .expect_body(types::MIC_STATUS)?;
    if !status.is_valid() {
        return Err(Error::Protocol("invalid mic.status".into()));
    }
    Ok(status)
}

/// Asks a PC to open its virtual microphone setup.
pub(crate) async fn request_mic_setup(shared: &Shared, session: &Session) -> Result<MicSetupResponse> {
    if !shared.toggle_on(&session.peer, crate::mic::TOGGLE) {
        return Err(Error::Denied);
    }
    if !peer_offers(shared, &session.peer, MIC_STATUS) {
        return Err(Error::Unsupported);
    }
    let reply: MicSetupResponse = session
        .request(Envelope::empty(types::MIC_SETUP), crate::session::REQUEST_TIMEOUT)
        .await?
        .expect_body(types::MIC_SETUP)?;
    if !reply.is_valid() {
        return Err(Error::Protocol("invalid mic.setup".into()));
    }
    Ok(reply)
}

/// PC side: reports virtual microphone readiness to a peer (`mic.status`
/// event).
pub(crate) async fn send_mic_status(shared: &Shared, session: &Session, status: &MicStatus) -> Result<()> {
    if !status.is_valid() {
        return Err(Error::Protocol("invalid mic.status".into()));
    }
    if !shared.toggle_on(&session.peer, crate::mic::TOGGLE) {
        return Err(Error::Denied);
    }
    if !shared.we_offer(MIC_STATUS) {
        return Err(Error::Unsupported);
    }
    session.send(Envelope::new(types::MIC_STATUS, status)?).await
}

/// Records the route a local controller applied to this device itself, so
/// PCM from its enrolled sources is accepted here. `None` stops it.
pub(crate) fn set_local(shared: &Shared, local: DeviceId, set: Option<AudioRouteSet>) -> Result<()> {
    if let Some(set) = &set
        && !set.is_valid()
    {
        return Err(Error::Protocol("invalid audio route".into()));
    }
    *lock_route(shared) =
        set.filter(|s| s.role != AudioRouteRole::Idle).map(|set| ActiveRoute { controller: local, set });
    Ok(())
}

/// The route set here, if any.
pub(crate) fn current(shared: &Shared) -> Option<AudioRouteSet> {
    lock_route(shared).as_ref().map(|r| r.set.clone())
}

/// `peer` was unpaired: a route it controlled stops.
pub(crate) fn forget(shared: &Shared, peer: &DeviceId) {
    let mut route = lock_route(shared);
    if route.as_ref().is_some_and(|r| r.controller == *peer) {
        *route = None;
    }
}

// ---- Receiving ----

/// Whether a managed frame from `peer` belongs to the route set here.
fn takes_pcm(shared: &Shared, peer: &DeviceId, pcm: &AudioRoutePcm) -> bool {
    if !shared.we_offer(AUDIO_ROUTE_SINK)
        || !peer_offers(shared, peer, AUDIO_ROUTE_SOURCE)
        || !shared.toggle_on(peer, MEDIA)
        || !pcm.is_valid()
    {
        return false;
    }
    // A source sends only its own audio: its lane is its device ID.
    if pcm.source_id != peer.to_string() {
        return false;
    }
    let enrolled = lock_route(shared).as_ref().is_some_and(|r| {
        r.set.generation == pcm.generation
            && matches!(r.set.role, AudioRouteRole::Sink | AudioRouteRole::PhonePlayback)
            && r.set.lanes.iter().any(|lane| lane.source_id == pcm.source_id)
    });
    enrolled && (!peer_is_phone(shared, peer) || shared.toggle_on(peer, MIRRORING))
}

fn deliver_pcm(shared: &Shared, peer: DeviceId, pcm: AudioRoutePcm) {
    if !takes_pcm(shared, &peer, &pcm) {
        return;
    }
    shared.platform.on_audio_route_pcm(peer, pcm.clone());
    shared.emit(NodeEvent::AudioRoutePcm { device: peer, pcm });
}

/// An `NLRP` datagram from `session`'s peer.
pub(crate) fn handle_datagram(shared: &Arc<Shared>, session: &Session, bytes: &[u8]) {
    debug_assert!(bytes.starts_with(NLRP_MAGIC));
    match AudioRoutePcm::from_datagram(bytes) {
        Ok(pcm) => deliver_pcm(shared, session.peer, pcm),
        Err(_) => tracing::debug!(peer = %session.peer.short(), "dropped a malformed managed audio datagram"),
    }
}

/// Applies an incoming `audio.route.set`.
async fn receive_set(
    shared: &Arc<Shared>,
    peer: DeviceId,
    env: &Envelope,
) -> std::result::Result<AudioRouteStatus, (ErrorCode, String)> {
    if !shared.toggle_on(&peer, MEDIA) {
        return Err((ErrorCode::Denied, "media is off for this device".into()));
    }
    if !peer_offers(shared, &peer, AUDIO_ROUTE_CONTROLLER) {
        return Err((ErrorCode::Denied, "that device doesn't control audio routes".into()));
    }
    let set = match env.body::<AudioRouteSet>() {
        Ok(set) if set.is_valid() => set,
        _ => return Err((ErrorCode::BadMessage, "invalid audio route".into())),
    };
    if role_capability(set.role).is_some_and(|cap| !shared.we_offer(cap)) {
        return Err((ErrorCode::Unsupported, "this device can't take that audio role".into()));
    }
    if set.include_phone_audio
        && set.role != AudioRouteRole::Idle
        && local_is_phone(shared)
        && !shared.toggle_on(&peer, MIRRORING)
    {
        return Err((ErrorCode::Denied, "phone audio is off for this device".into()));
    }
    let platform = shared.platform.clone();
    let applied = set.clone();
    let result = tokio::task::spawn_blocking(move || platform.on_audio_route_set(peer, applied))
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
    match result {
        Ok(status) if status.is_valid() => {
            let mut route = lock_route(shared);
            if set.role == AudioRouteRole::Idle {
                // Idle stops only a route this controller set (or none).
                if route.as_ref().is_none_or(|r| r.controller == peer) {
                    *route = None;
                }
            } else {
                *route = Some(ActiveRoute { controller: peer, set });
            }
            Ok(status)
        }
        Ok(_) => Err((ErrorCode::Internal, "invalid audio route status".into())),
        Err(reason) => {
            tracing::debug!(peer = %peer.short(), "couldn't apply an audio route");
            let reason = bounded(reason, nectarlink_protocol::messages::audio_route::MAX_REASON_BYTES);
            Err((
                ErrorCode::Internal,
                if reason.trim().is_empty() { "couldn't apply the route".into() } else { reason },
            ))
        }
    }
}

/// Handles `audio.route.*`, `mic.status` and `mic.setup`. `false` if `env`
/// isn't one of them.
pub(crate) async fn handle(shared: &Arc<Shared>, session: &Arc<Session>, env: &Envelope) -> Result<bool> {
    let peer = session.peer;
    match env.t.as_str() {
        types::AUDIO_ROUTE_SET => {
            let reply = match receive_set(shared, peer, env).await {
                Ok(status) => Envelope::new(types::AUDIO_ROUTE_STATUS, &status)?,
                Err((code, msg)) => Envelope::error(code, msg),
            };
            if env.id.is_some() {
                session.send(reply.reply_to(env.id)).await?;
            }
        }
        types::AUDIO_ROUTE_STATUS => {
            // A reply to our request is matched by `re` before it gets here.
            if shared.we_offer(AUDIO_ROUTE_CONTROLLER)
                && shared.toggle_on(&peer, MEDIA)
                && let Ok(status) = env.body::<AudioRouteStatus>()
                && status.is_valid()
            {
                shared.emit(NodeEvent::AudioRouteStatus { device: peer, status });
            }
        }
        types::AUDIO_ROUTE_LEVELS => {
            if shared.we_offer(AUDIO_ROUTE_CONTROLLER)
                && shared.toggle_on(&peer, MEDIA)
                && peer_offers(shared, &peer, AUDIO_ROUTE_SINK)
                && let Ok(levels) = env.body::<AudioRouteLevels>()
                && levels.is_valid()
            {
                let platform = shared.platform.clone();
                let copy = levels.clone();
                shared.runtime.spawn_blocking(move || platform.on_audio_route_levels(peer, copy));
                shared.emit(NodeEvent::AudioRouteLevels { device: peer, levels: RouteLevels(levels) });
            }
        }
        types::AUDIO_ROUTE_PCM => {
            if let Ok(pcm) = env.body::<AudioRoutePcm>()
                && pcm.pcm.len() <= MAX_FALLBACK_PCM_BYTES
            {
                deliver_pcm(shared, peer, pcm);
            }
        }
        types::MIC_STATUS if env.id.is_some() && env.b.is_none() => {
            let reply = if !shared.toggle_on(&peer, crate::mic::TOGGLE) {
                Envelope::error(ErrorCode::Denied, "the microphone is off for this device")
            } else if !shared.we_offer(MIC_STATUS) {
                Envelope::error(ErrorCode::Unsupported, "no virtual microphone here")
            } else {
                let platform = shared.platform.clone();
                match tokio::task::spawn_blocking(move || platform.on_mic_status_request(peer))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
                {
                    Ok(status) if status.is_valid() => Envelope::new(types::MIC_STATUS, &status)?,
                    Ok(_) => Envelope::error(ErrorCode::Internal, "invalid microphone status"),
                    Err(_) => Envelope::error(ErrorCode::Unsupported, "no virtual microphone status here"),
                }
            };
            session.send(reply.reply_to(env.id)).await?;
        }
        types::MIC_STATUS => {
            if shared.toggle_on(&peer, crate::mic::TOGGLE)
                && peer_offers(shared, &peer, MIC_STATUS)
                && let Ok(status) = env.body::<MicStatus>()
                && status.is_valid()
            {
                shared.emit(NodeEvent::MicStatus { device: peer, status });
            }
            if env.id.is_some() {
                session.send(Envelope::empty(types::OK).reply_to(env.id)).await?;
            }
        }
        types::MIC_SETUP => {
            if env.id.is_none() {
                return Ok(true);
            }
            let reply = if !shared.toggle_on(&peer, crate::mic::TOGGLE) {
                Envelope::error(ErrorCode::Denied, "the microphone is off for this device")
            } else if !shared.we_offer(MIC_STATUS) {
                Envelope::error(ErrorCode::Unsupported, "no virtual microphone here")
            } else {
                let platform = shared.platform.clone();
                match tokio::task::spawn_blocking(move || platform.on_mic_setup_request(peer))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
                {
                    Ok(reply) if reply.is_valid() => Envelope::new(types::MIC_SETUP, &reply)?,
                    Ok(_) => Envelope::error(ErrorCode::Internal, "invalid microphone setup reply"),
                    Err(_) => Envelope::error(ErrorCode::Unsupported, "no virtual microphone setup here"),
                }
            };
            session.send(reply.reply_to(env.id)).await?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_compare_floats_bit_for_bit() {
        let levels = |peak: f32| {
            RouteLevels(AudioRouteLevels {
                generation: 3,
                levels: vec![AudioLaneLevel { source_id: "a".into(), peak, active: true }],
            })
        };
        assert_eq!(levels(0.5), levels(0.5));
        assert_ne!(levels(0.5), levels(0.25));
        assert_ne!(levels(0.0), levels(-0.0));
    }

    #[test]
    fn each_role_needs_its_capability() {
        assert_eq!(role_capability(AudioRouteRole::Idle), None);
        assert_eq!(role_capability(AudioRouteRole::Source), Some(AUDIO_ROUTE_SOURCE));
        assert_eq!(role_capability(AudioRouteRole::Sink), Some(AUDIO_ROUTE_SINK));
        assert_eq!(role_capability(AudioRouteRole::PhonePlayback), Some(AUDIO_ROUTE_SINK));
    }

    #[test]
    fn reasons_are_cut_on_a_character_boundary() {
        assert_eq!(bounded("héllo".into(), 2), "h");
        assert_eq!(bounded("ok".into(), 256), "ok");
    }
}
