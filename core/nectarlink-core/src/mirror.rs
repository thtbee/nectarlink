// SPDX-License-Identifier: MPL-2.0
//! Screen mirroring (docs/protocol/mirror.md): a PC asks a phone for its
//! screen; once the phone's user agrees, the phone streams encoded video
//! on a stream of its own, which the PC decodes and shows.
//!
//! The phone side keeps latency low over throughput: when the network
//! can't keep up, frames are dropped (never queued for long) and the
//! encoder is asked for a keyframe to start clean from.
//!
//! When the PC asks for it and the phone can, the phone's sound comes on a
//! second stream (PCM), dropped rather than delayed in the same way.
//!
//! Each mirroring is a session the PC numbers: 0 for the phone's screen,
//! others for apps the phone runs on displays of their own (Elevated), each
//! in a window of its own on the PC.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use iroh::endpoint::{RecvStream, SendStream, VarInt};
use nectarlink_protocol::{
    DeviceId, Envelope, ErrorCode, PacketKind,
    messages::{
        MirrorAudioConfig, MirrorConfig, MirrorInput, MirrorPower, MirrorResize, MirrorSession, MirrorStart,
        PhoneApp, PhoneApps, StreamHeader, mirror, types,
    },
    read_video_packet, video_packet_header, write_frame,
};
use tokio::sync::mpsc;

use crate::{Error, Result, events::NodeEvent, node::Shared, session::Session};

/// The device toggle that allows mirroring for a device.
pub(crate) const TOGGLE: &str = "mirroring";
/// Packets waiting to go out, at most. Beyond this the network is behind:
/// sending more would only add delay.
const QUEUE: usize = 4;
/// Sound packets waiting to go out, at most (each is a few milliseconds).
const AUDIO_QUEUE: usize = 24;
/// Icons in a `mirror.apps` answer, at most, in bytes: the rest go without
/// (the answer stays well inside a frame).
const ICONS_BUDGET: usize = 768 * 1024;

/// What the PC's app gets of a mirroring stream. Called from the core's
/// threads; must return quickly (hand the work to a thread of its own).
pub trait MirrorSink: Send + Sync {
    /// The stream's format: first, and again when it changes.
    fn config(&self, config: MirrorConfig);
    /// Encoded video (Annex B), `keyframe` when it starts fresh.
    fn packet(&self, keyframe: bool, time_us: u64, data: Vec<u8>);
    /// The stream ended (the phone stopped, or the connection dropped).
    fn ended(&self);
    /// The sound's format: first on the sound stream, and when it changes.
    fn audio_config(&self, _config: MirrorAudioConfig) {}
    /// Sound, in that format.
    fn audio(&self, _time_us: u64, _data: Vec<u8>) {}
    /// The sound stream ended.
    fn audio_ended(&self) {}
}

impl std::fmt::Debug for dyn MirrorSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MirrorSink")
    }
}

// ---- The PC ----

pub(crate) async fn start(shared: &Shared, session: &Session, options: MirrorStart) -> Result<()> {
    if !shared.toggle_on(&session.peer, TOGGLE) {
        return Err(Error::Denied);
    }
    if !options.is_valid() {
        return Err(Error::Protocol("invalid mirroring options".into()));
    }
    let env = Envelope::new(types::MIRROR_START, &options)?;
    session.request(env, crate::session::REQUEST_TIMEOUT).await?.expect(types::OK)?;
    Ok(())
}

pub(crate) async fn stop(session: &Session, mirroring: u32) -> Result<()> {
    let env = Envelope::new(types::MIRROR_STOP, &MirrorSession { session: mirroring })?;
    session.request(env, crate::session::REQUEST_TIMEOUT).await?.expect(types::OK)?;
    Ok(())
}

/// Mouse and keyboard on a phone's mirrored screen or app window (not
/// answered: input is only worth it right away).
pub(crate) async fn input(
    shared: &Shared,
    session: &Session,
    mirroring: u32,
    input: MirrorInput,
) -> Result<()> {
    if !shared.toggle_on(&session.peer, TOGGLE) {
        return Err(Error::Denied);
    }
    if !input.is_valid() {
        return Err(Error::Protocol("invalid input".into()));
    }
    session.send(input_envelope(&input, mirroring)?).await
}

fn input_envelope(input: &MirrorInput, mirroring: u32) -> Result<Envelope> {
    Ok(nectarlink_protocol::messages::mirror_input_envelope(input, mirroring)?)
}

pub(crate) async fn request_keyframe(session: &Session, mirroring: u32) {
    if let Ok(env) = Envelope::new(types::MIRROR_KEYFRAME, &MirrorSession { session: mirroring }) {
        let _ = session.send(env).await;
    }
}

/// Resize an app window's display (`mirroring` != 0) on a phone.
pub(crate) async fn resize(
    shared: &Shared,
    session: &Session,
    mirroring: u32,
    width: u32,
    height: u32,
) -> Result<()> {
    if !shared.toggle_on(&session.peer, TOGGLE) {
        return Err(Error::Denied);
    }
    let msg = MirrorResize { session: mirroring, width, height };
    if !msg.is_valid() {
        return Err(Error::Protocol("invalid resize".into()));
    }
    session.send(Envelope::new(types::MIRROR_RESIZE, &msg)?).await
}

/// Update `stay_awake` and `screen_off` on a phone while mirroring its screen (`session == 0`).
pub(crate) async fn power(
    shared: &Shared,
    session: &Session,
    stay_awake: bool,
    screen_off: bool,
) -> Result<()> {
    if !shared.toggle_on(&session.peer, TOGGLE) {
        return Err(Error::Denied);
    }
    let msg = MirrorPower { stay_awake, screen_off };
    session.send(Envelope::new(types::MIRROR_POWER, &msg)?).await
}

/// The apps a phone can open in windows of their own, by name.
pub(crate) async fn apps(shared: &Shared, session: &Session) -> Result<Vec<PhoneApp>> {
    if !shared.toggle_on(&session.peer, TOGGLE) {
        return Err(Error::Denied);
    }
    let env = Envelope::empty(types::MIRROR_APPS);
    let PhoneApps { mut apps } =
        session.request(env, crate::session::REQUEST_TIMEOUT).await?.expect_body(types::MIRROR_APPS)?;
    apps.retain(|a| nectarlink_protocol::messages::is_package_name(&a.pkg));
    apps.truncate(mirror::MAX_APPS);
    for app in &mut apps {
        app.label = app.label.chars().take(100).collect();
        if app.icon.as_ref().is_some_and(|i| i.len() > mirror::MAX_ICON_BYTES) {
            app.icon = None;
        }
    }
    apps.sort_by_cached_key(|a| a.label.to_lowercase());
    Ok(apps)
}

/// A phone opened a video stream (its header already read). Its first
/// packet says which mirroring it is.
pub(crate) async fn receive(shared: Arc<Shared>, peer: DeviceId, mut send: SendStream, mut recv: RecvStream) {
    let refuse = |mut send: SendStream, mut recv: RecvStream| {
        let _ = send.reset(VarInt::from_u32(mirror::STOPPED));
        let _ = recv.stop(VarInt::from_u32(mirror::STOPPED));
    };
    let shows = shared.local_capabilities().iter().any(|c| c == mirror::VIEW);
    if !shows || !shared.toggle_on(&peer, TOGGLE) {
        return refuse(send, recv);
    }
    let first = match read_video_packet(&mut recv).await {
        Ok(Some(p)) if p.kind == PacketKind::Config => {
            MirrorConfig::from_cbor(&p.data).ok().filter(|c| c.is_valid())
        }
        _ => None,
    };
    let Some(config) = first else {
        tracing::warn!("a phone sent a video format this PC can't show");
        return refuse(send, recv);
    };
    let mirroring = config.session;
    let Some(sink) = shared.platform.mirror_sink(&peer, mirroring) else {
        return refuse(send, recv);
    };
    // Ready to be stopped before anyone hears it's showing: a stop sent the
    // moment it appears must not be lost.
    let stopped = shared.new_mirror_stop(&peer, mirroring, mirror::OP_VIDEO);
    let started_at = std::time::Instant::now();
    let title = if mirroring == mirror::SCREEN {
        "Screen mirroring".to_owned()
    } else {
        format!("App window #{mirroring}")
    };
    let tl_row = shared.record_timeline(crate::timeline::NewTimelineEntry {
        kind: crate::TimelineKind::Session,
        device_id: peer,
        device_name: shared.peer_name(&peer),
        incoming: true,
        timestamp: crate::now_unix(),
        title,
        detail: format!("{}×{}", config.width, config.height),
        target: "mirror".into(),
        size_bytes: 0,
        duration_secs: 0,
        ref_id: None,
    });
    shared.emit(NodeEvent::Mirroring { device: peer, session: mirroring, on: true });
    sink.config(config);
    loop {
        let packet = tokio::select! {
            _ = stopped.notified() => break,
            packet = read_video_packet(&mut recv) => packet,
        };
        match packet {
            Ok(Some(p)) if p.kind == PacketKind::Config => match MirrorConfig::from_cbor(&p.data) {
                Ok(config) if config.is_valid() && config.session == mirroring => sink.config(config),
                _ => {
                    tracing::warn!("a phone sent a video format this PC can't show");
                    break;
                }
            },
            Ok(Some(p)) => sink.packet(p.kind == PacketKind::Keyframe, p.time_us, p.data),
            Ok(None) => break,
            Err(e) => {
                tracing::debug!(error = %e, "mirroring stream ended");
                break;
            }
        }
    }
    let _ = recv.stop(VarInt::from_u32(mirror::STOPPED));
    let _ = send.finish();
    sink.ended();
    if let Some(row_id) = tl_row {
        shared.finish_timeline_session(row_id, started_at.elapsed().as_secs().max(1));
    }
    shared.emit(NodeEvent::Mirroring { device: peer, session: mirroring, on: false });
}

/// A phone opened a sound stream (its header already read).
pub(crate) async fn receive_audio(
    shared: Arc<Shared>,
    peer: DeviceId,
    mut send: SendStream,
    mut recv: RecvStream,
) {
    // Sound comes with the screen only.
    let listens = shared.local_capabilities().iter().any(|c| c == mirror::LISTEN);
    let sink = listens.then(|| shared.platform.mirror_sink(&peer, mirror::SCREEN)).flatten();
    let Some(sink) = sink.filter(|_| shared.toggle_on(&peer, TOGGLE)) else {
        let _ = send.reset(VarInt::from_u32(mirror::STOPPED));
        let _ = recv.stop(VarInt::from_u32(mirror::STOPPED));
        return;
    };
    let stopped = shared.new_mirror_stop(&peer, mirror::SCREEN, mirror::OP_AUDIO);
    loop {
        let packet = tokio::select! {
            _ = stopped.notified() => break,
            packet = read_video_packet(&mut recv) => packet,
        };
        match packet {
            Ok(Some(p)) if p.kind == PacketKind::Config => match MirrorAudioConfig::from_cbor(&p.data) {
                Ok(config) if config.is_valid() => sink.audio_config(config),
                _ => {
                    tracing::warn!("a phone sent a sound format this PC can't play");
                    break;
                }
            },
            Ok(Some(p)) => sink.audio(p.time_us, p.data),
            Ok(None) => break,
            Err(e) => {
                tracing::debug!(error = %e, "mirrored sound ended");
                break;
            }
        }
    }
    let _ = recv.stop(VarInt::from_u32(mirror::STOPPED));
    let _ = send.finish();
    sink.audio_ended();
}

// ---- The phone ----

/// Handles `mirror.*` on a session. Returns false for other types.
pub(crate) async fn handle(shared: &Arc<Shared>, session: &Arc<Session>, env: &Envelope) -> Result<bool> {
    let peer = session.peer;
    let platform = shared.platform.clone();
    match env.t.as_str() {
        types::MIRROR_START => {
            let options: MirrorStart = env.body()?;
            let needs = if options.app.is_some() { mirror::VIRTUAL_DISPLAY } else { mirror::CAPTURE };
            let reply = if !shared.toggle_on(&peer, TOGGLE) {
                Envelope::error(ErrorCode::Denied, "mirroring is off for this device")
            } else if !options.is_valid() {
                Envelope::error(ErrorCode::BadMessage, "invalid mirroring options")
            } else if !shared.local_capabilities().iter().any(|c| c == needs) {
                Envelope::error(ErrorCode::Unsupported, "this device doesn't share that")
            } else {
                match tokio::task::spawn_blocking(move || platform.mirror_requested(&peer, &options))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
                {
                    Ok(()) => Envelope::empty(types::OK),
                    Err(reason) => {
                        tracing::warn!(reason, "can't ask to share the screen");
                        Envelope::error(ErrorCode::Internal, "the phone couldn't ask")
                    }
                }
            };
            session.send(reply.reply_to(env.id)).await?;
        }
        types::MIRROR_STOP => {
            let MirrorSession { session: mirroring } = env.body()?;
            // The PC's side: stop showing; the phone's: stop sharing.
            shared.stop_showing(&peer, mirroring);
            shared.emit(NodeEvent::Mirroring { device: peer, session: mirroring, on: false });
            tokio::task::spawn_blocking(move || platform.mirror_stop_requested(&peer, mirroring));
            session.send(Envelope::empty(types::OK).reply_to(env.id)).await?;
        }
        types::MIRROR_KEYFRAME => {
            let MirrorSession { session: mirroring } = env.body()?;
            tokio::task::spawn_blocking(move || platform.mirror_keyframe_requested(&peer, mirroring));
        }
        types::MIRROR_RESIZE => {
            let resize: MirrorResize = env.body()?;
            let allowed = shared.toggle_on(&peer, TOGGLE)
                && shared.local_capabilities().iter().any(|c| c == mirror::VIRTUAL_DISPLAY);
            if allowed && resize.is_valid() {
                tokio::task::spawn_blocking(move || {
                    platform.mirror_resize_requested(&peer, resize.session, resize.width, resize.height);
                });
            }
        }
        types::MIRROR_POWER => {
            let power: MirrorPower = env.body()?;
            let allowed = shared.toggle_on(&peer, TOGGLE)
                && shared.local_capabilities().iter().any(|c| c == mirror::CAPTURE);
            if allowed {
                tokio::task::spawn_blocking(move || {
                    platform.mirror_power_requested(&peer, power.stay_awake, power.screen_off);
                });
            }
        }
        types::MIRROR_INPUT => {
            let input: MirrorInput = env.body()?;
            let MirrorSession { session: mirroring } = env.body()?;
            // App windows take input as they're Elevated; the screen, with mirror.input.
            let needs = if mirroring == mirror::SCREEN { mirror::INPUT } else { mirror::VIRTUAL_DISPLAY };
            let allowed =
                shared.toggle_on(&peer, TOGGLE) && shared.local_capabilities().iter().any(|c| c == needs);
            if allowed && input.is_valid() {
                platform.mirror_input(&peer, mirroring, input);
            }
        }
        types::MIRROR_APPS => {
            let reply = if !shared.toggle_on(&peer, TOGGLE) {
                Envelope::error(ErrorCode::Denied, "mirroring is off for this device")
            } else if !shared.local_capabilities().iter().any(|c| c == mirror::VIRTUAL_DISPLAY) {
                Envelope::error(ErrorCode::Unsupported, "this device doesn't open apps in windows")
            } else {
                match tokio::task::spawn_blocking(move || platform.phone_apps())
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
                {
                    Ok(apps) => Envelope::new(types::MIRROR_APPS, &PhoneApps { apps: fit_apps(apps) })?,
                    Err(reason) => {
                        tracing::warn!(reason, "can't list the apps");
                        Envelope::error(ErrorCode::Internal, "the phone couldn't list its apps")
                    }
                }
            };
            session.send(reply.reply_to(env.id)).await?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// Apps within the limits: at most [`mirror::MAX_APPS`], icons small and
/// within [`ICONS_BUDGET`] all together.
fn fit_apps(mut apps: Vec<PhoneApp>) -> Vec<PhoneApp> {
    apps.retain(|a| nectarlink_protocol::messages::is_package_name(&a.pkg));
    apps.truncate(mirror::MAX_APPS);
    let mut budget = ICONS_BUDGET;
    for app in &mut apps {
        app.label = app.label.chars().take(100).collect();
        match app.icon.as_ref().map(Vec::len) {
            Some(len) if len <= mirror::MAX_ICON_BYTES && len <= budget => budget -= len,
            _ => app.icon = None,
        }
    }
    apps
}

/// What became of a packet handed to [`MirrorStream::send`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirrorSend {
    Queued,
    /// Sound dropped to keep the delay low (the next packet may go).
    Dropped,
    /// Dropped to keep the delay low: send a keyframe next (frames are
    /// dropped until one comes).
    NeedKeyframe,
    /// The PC stopped watching, or the connection is gone: stop sharing.
    Closed,
}

/// A phone's video (or sound) stream to one PC.
#[derive(Debug)]
pub struct MirrorStream {
    queue: mpsc::Sender<(PacketKind, u64, Vec<u8>)>,
    closed: Arc<AtomicBool>,
    /// Wakes the sending task when the stream is closed, so it finishes the
    /// stream even if no more packets come.
    stop: Arc<tokio::sync::Notify>,
    /// Frames are being dropped until a keyframe.
    resyncing: Mutex<bool>,
    /// Sound: each packet stands alone, so a dropped one needs no keyframe.
    audio: bool,
}

impl MirrorStream {
    /// Queues a packet without waiting. A config packet waits for room:
    /// the PC can't decode without it.
    pub fn send(&self, kind: PacketKind, time_us: u64, data: Vec<u8>) -> MirrorSend {
        if self.closed.load(Ordering::Acquire) {
            return MirrorSend::Closed;
        }
        let mut resyncing = self.resyncing.lock().unwrap_or_else(|e| e.into_inner());
        if *resyncing && kind == PacketKind::Frame && !self.audio {
            return MirrorSend::NeedKeyframe;
        }
        let sent = if kind == PacketKind::Config {
            self.queue.blocking_send((kind, time_us, data)).map_err(|_| ())
        } else {
            self.queue.try_send((kind, time_us, data)).map_err(|e| match e {
                mpsc::error::TrySendError::Full(_) => (),
                mpsc::error::TrySendError::Closed(_) => self.closed.store(true, Ordering::Release),
            })
        };
        match sent {
            Ok(()) => {
                if kind == PacketKind::Keyframe {
                    *resyncing = false;
                }
                MirrorSend::Queued
            }
            Err(()) if self.closed.load(Ordering::Acquire) => MirrorSend::Closed,
            Err(()) if self.audio => MirrorSend::Dropped,
            Err(()) => {
                *resyncing = true;
                MirrorSend::NeedKeyframe
            }
        }
    }

    /// Ends the stream (the phone stopped sharing).
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.stop.notify_one();
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire) || self.queue.is_closed()
    }
}

/// Opens a video stream to a PC that asked for the screen, or (`audio`) a
/// sound stream to one that asked for the sound too.
pub(crate) async fn open(shared: &Arc<Shared>, session: &Session, audio: bool) -> Result<MirrorStream> {
    if !shared.toggle_on(&session.peer, TOGGLE) {
        return Err(Error::Denied);
    }
    let (mut send, recv) = session.conn.open_bi().await.map_err(crate::error::net)?;
    let op = if audio { mirror::OP_AUDIO } else { mirror::OP_VIDEO };
    let header = StreamHeader { svc: mirror::SERVICE.into(), op: op.into(), v: mirror::VERSION };
    write_frame(&mut send, &Envelope::new(types::STREAM, &header)?.to_cbor()).await?;
    let on_end = if !audio {
        shared
            .record_timeline(crate::timeline::NewTimelineEntry {
                kind: crate::TimelineKind::Session,
                device_id: session.peer,
                device_name: shared.peer_name(&session.peer),
                incoming: false,
                timestamp: crate::now_unix(),
                title: "Screen mirroring".into(),
                detail: String::new(),
                target: "mirror".into(),
                size_bytes: 0,
                duration_secs: 0,
                ref_id: None,
            })
            .map(|id| (shared.clone(), id, std::time::Instant::now()))
    } else {
        None
    };
    Ok(MirrorStream::spawn_stream(send, recv, audio, on_end))
}

impl MirrorStream {
    pub(crate) fn spawn_stream(
        mut send: SendStream,
        mut recv: RecvStream,
        audio: bool,
        on_end: Option<(Arc<Shared>, i64, std::time::Instant)>,
    ) -> MirrorStream {
        // Sound and video go out as soon as they're written; sound first, as a
        // gap in it is the more noticeable.
        let _ = send.set_priority(if audio { 2 } else { 1 });
        let (queue, mut packets) =
            mpsc::channel::<(PacketKind, u64, Vec<u8>)>(if audio { AUDIO_QUEUE } else { QUEUE });
        let closed = Arc::new(AtomicBool::new(false));
        let done = closed.clone();
        let stop = Arc::new(tokio::sync::Notify::new());
        let stopped = stop.clone();
        tokio::spawn(async move {
            let mut ended = Box::pin(async move {
                // The PC stops reading (STOP_SENDING) when it's done watching.
                let _ = recv.read_to_end(64).await;
            });
            loop {
                let next = tokio::select! {
                    _ = &mut ended => break,
                    _ = stopped.notified() => break,
                    next = packets.recv() => next,
                };
                let Some((kind, time, data)) = next else { break };
                if done.load(Ordering::Acquire) {
                    break;
                }
                let Ok(header) = video_packet_header(kind, time, data.len()) else { continue };
                let written = async {
                    send.write_all(&header).await?;
                    send.write_all(&data).await
                };
                if written.await.is_err() {
                    break;
                }
            }
            done.store(true, Ordering::Release);
            let _ = send.finish();
            if let Some((shared, row_id, started_at)) = on_end {
                shared.finish_timeline_session(row_id, started_at.elapsed().as_secs().max(1));
            }
        });
        MirrorStream { queue, closed, stop, resyncing: Mutex::new(false), audio }
    }
}

impl Shared {
    /// A new stop signal for the PC's reader of `peer`'s video or sound
    /// stream (`op`) of a mirroring.
    fn new_mirror_stop(&self, peer: &DeviceId, mirroring: u32, op: &'static str) -> Arc<tokio::sync::Notify> {
        let signal = Arc::new(tokio::sync::Notify::new());
        self.mirror_stops
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert((*peer, mirroring, op), signal.clone());
        signal
    }

    /// Stops showing (and playing) a mirroring of `peer` here (kept until
    /// the readers see it).
    pub(crate) fn stop_showing(&self, peer: &DeviceId, mirroring: u32) {
        let mut stops = self.mirror_stops.lock().unwrap_or_else(|e| e.into_inner());
        for op in [mirror::OP_VIDEO, mirror::OP_AUDIO] {
            if let Some(signal) = stops.remove(&(*peer, mirroring, op)) {
                signal.notify_one();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn a_full_queue_drops_frames_until_a_keyframe() {
        let (queue, mut packets) = mpsc::channel(2);
        let stream = MirrorStream {
            queue,
            closed: Arc::default(),
            stop: Arc::default(),
            resyncing: Mutex::new(false),
            audio: false,
        };
        let send = |kind| tokio::task::block_in_place(|| stream.send(kind, 0, vec![1]));
        assert_eq!(send(PacketKind::Keyframe), MirrorSend::Queued);
        assert_eq!(send(PacketKind::Frame), MirrorSend::Queued);
        assert_eq!(send(PacketKind::Frame), MirrorSend::NeedKeyframe, "full");
        packets.recv().await;
        // Room again, but the decoder needs a fresh start first.
        assert_eq!(send(PacketKind::Frame), MirrorSend::NeedKeyframe);
        assert_eq!(send(PacketKind::Keyframe), MirrorSend::Queued);
        packets.recv().await;
        assert_eq!(send(PacketKind::Frame), MirrorSend::Queued);
        drop(packets);
        assert_eq!(send(PacketKind::Keyframe), MirrorSend::Closed);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sound_drops_only_what_doesnt_fit() {
        let (queue, mut packets) = mpsc::channel(1);
        let stream = MirrorStream {
            queue,
            closed: Arc::default(),
            stop: Arc::default(),
            resyncing: Mutex::new(false),
            audio: true,
        };
        let send = |kind| tokio::task::block_in_place(|| stream.send(kind, 0, vec![1]));
        assert_eq!(send(PacketKind::Frame), MirrorSend::Queued);
        assert_eq!(send(PacketKind::Frame), MirrorSend::Dropped, "full");
        packets.recv().await;
        // No keyframe to wait for: the next one goes.
        assert_eq!(send(PacketKind::Frame), MirrorSend::Queued);
    }
}
