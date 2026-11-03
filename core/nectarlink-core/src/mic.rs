// SPDX-License-Identifier: MPL-2.0
//! Phone as microphone (`docs/protocol/mic.md`): a phone streams its
//! microphone as 16-bit PCM on a dedicated QUIC stream (`mic/pcm`), after
//! its user accepts a PC's `mic.start` (or starts it on the phone).

use std::sync::Arc;

use iroh::endpoint::{RecvStream, SendStream, VarInt};
use nectarlink_protocol::{
    DeviceId, Envelope, ErrorCode, PacketKind,
    messages::{MirrorAudioConfig, StreamHeader, mic, types},
    read_video_packet, write_frame,
};

use crate::{Error, MirrorStream, Result, events::NodeEvent, node::Shared, session::Session};

/// The device toggle that allows a device's microphone on this PC (or this
/// phone's microphone for a PC).
pub const TOGGLE: &str = "mic";

/// What the PC's app gets of a phone's microphone. Called from the core's
/// threads; must return quickly.
pub trait MicSink: Send + Sync {
    /// The sound's format: first, and again if it changes.
    fn config(&self, config: MirrorAudioConfig);
    /// Sound, in that format.
    fn audio(&self, time_us: u64, data: Vec<u8>);
    /// The stream ended (the phone stopped, or the connection dropped).
    fn ended(&self);
}

impl std::fmt::Debug for dyn MicSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MicSink")
    }
}

// ---- The PC ----

pub(crate) async fn start(shared: &Shared, session: &Session) -> Result<()> {
    if !shared.toggle_on(&session.peer, TOGGLE) {
        return Err(Error::Denied);
    }
    if !shared.store.get_peer(&session.peer)?.is_some_and(|p| p.caps.contains(mic::STREAM)) {
        return Err(Error::Unsupported);
    }
    let env = Envelope::empty(types::MIC_START);
    session.request(env, crate::session::REQUEST_TIMEOUT).await?.expect(types::OK)?;
    Ok(())
}

pub(crate) async fn stop(session: &Session) -> Result<()> {
    session.send(Envelope::empty(types::MIC_STOP)).await
}

/// A phone opened a `mic/pcm` stream (its stream header already read).
pub(crate) async fn receive(shared: Arc<Shared>, peer: DeviceId, mut send: SendStream, mut recv: RecvStream) {
    let refuse = |mut send: SendStream, mut recv: RecvStream| {
        let _ = send.reset(VarInt::from_u32(mic::STOPPED));
        let _ = recv.stop(VarInt::from_u32(mic::STOPPED));
    };
    let listens = shared.we_offer(mic::LISTEN);
    if !listens || !shared.toggle_on(&peer, TOGGLE) {
        return refuse(send, recv);
    }
    let first = match read_video_packet(&mut recv).await {
        Ok(Some(p)) if p.kind == PacketKind::Config => {
            MirrorAudioConfig::from_cbor(&p.data).ok().filter(MirrorAudioConfig::is_valid)
        }
        _ => None,
    };
    let Some(config) = first else {
        tracing::warn!("a phone sent a microphone format this PC can't play");
        return refuse(send, recv);
    };
    let Some(sink) = shared.platform.mic_sink(&peer) else {
        return refuse(send, recv);
    };
    let stopped = shared.new_mic_stop(&peer);
    shared.emit(NodeEvent::Mic { device: peer, on: true });
    sink.config(config);
    loop {
        let packet = tokio::select! {
            _ = stopped.notified() => break,
            packet = read_video_packet(&mut recv) => packet,
        };
        match packet {
            Ok(Some(p)) if p.kind == PacketKind::Config => match MirrorAudioConfig::from_cbor(&p.data) {
                Ok(config) if config.is_valid() => sink.config(config),
                _ => {
                    tracing::warn!("a phone sent a microphone format this PC can't play");
                    break;
                }
            },
            Ok(Some(p)) => sink.audio(p.time_us, p.data),
            Ok(None) => break,
            Err(e) => {
                tracing::debug!(error = %e, "microphone stream ended");
                break;
            }
        }
    }
    let _ = recv.stop(VarInt::from_u32(mic::STOPPED));
    let _ = send.finish();
    if shared.clear_mic_stop(&peer, &stopped) {
        sink.ended();
        shared.emit(NodeEvent::Mic { device: peer, on: false });
    }
}

// ---- The phone ----

pub(crate) async fn handle(shared: &Arc<Shared>, session: &Arc<Session>, env: &Envelope) -> Result<bool> {
    let peer = session.peer;
    let platform = shared.platform.clone();
    match env.t.as_str() {
        types::MIC_START => {
            let reply = if !shared.toggle_on(&peer, TOGGLE) {
                Envelope::error(ErrorCode::Denied, "the microphone is off for this device")
            } else if !shared.we_offer(mic::STREAM) {
                Envelope::error(ErrorCode::Unsupported, "this device doesn't stream its microphone")
            } else {
                match tokio::task::spawn_blocking(move || platform.mic_requested(&peer))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
                {
                    Ok(()) => Envelope::empty(types::OK),
                    Err(reason) => {
                        tracing::warn!(reason, "can't ask to start the microphone");
                        Envelope::error(ErrorCode::Internal, "the phone couldn't ask")
                    }
                }
            };
            session.send(reply.reply_to(env.id)).await?;
        }
        types::MIC_STOP => {
            // The PC's side: stop listening; the phone's: stop recording. A phone that
            // declines before any sound came has no stream to stop, but the PC still
            // hears that the microphone is off, so it stops waiting.
            if !shared.stop_mic(&peer) {
                shared.emit(NodeEvent::Mic { device: peer, on: false });
            }
            tokio::task::spawn_blocking(move || platform.mic_stop_requested(&peer));
            if env.id.is_some() {
                session.send(Envelope::empty(types::OK).reply_to(env.id)).await?;
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// Opens this phone's microphone stream to a connected PC: write a config
/// packet first, then the sound.
pub(crate) async fn open(shared: &Arc<Shared>, session: &Session) -> Result<MirrorStream> {
    if !shared.toggle_on(&session.peer, TOGGLE) {
        return Err(Error::Denied);
    }
    let (mut send, recv) = session.conn.open_bi().await.map_err(crate::error::net)?;
    let header = StreamHeader { svc: mic::SERVICE.into(), op: mic::OP_PCM.into(), v: mic::VERSION };
    write_frame(&mut send, &Envelope::new(types::STREAM, &header)?.to_cbor()).await?;
    let on_end = shared
        .record_timeline(crate::timeline::NewTimelineEntry {
            kind: crate::TimelineKind::Session,
            device_id: session.peer,
            device_name: shared.peer_name(&session.peer),
            incoming: false,
            timestamp: crate::now_unix(),
            title: "Phone microphone".into(),
            detail: String::new(),
            target: "mic".into(),
            size_bytes: 0,
            duration_secs: 0,
            ref_id: None,
        })
        .map(|id| (shared.clone(), id, std::time::Instant::now()));
    Ok(MirrorStream::spawn_stream(send, recv, true, false, on_end))
}

impl Shared {
    fn new_mic_stop(&self, peer: &DeviceId) -> Arc<tokio::sync::Notify> {
        let signal = Arc::new(tokio::sync::Notify::new());
        if let Some(prev) =
            self.mic_stops.lock().unwrap_or_else(|e| e.into_inner()).insert(*peer, signal.clone())
        {
            prev.notify_one();
        }
        signal
    }

    fn clear_mic_stop(&self, peer: &DeviceId, signal: &Arc<tokio::sync::Notify>) -> bool {
        let mut stops = self.mic_stops.lock().unwrap_or_else(|e| e.into_inner());
        if stops.get(peer).is_some_and(|cur| Arc::ptr_eq(cur, signal)) {
            stops.remove(peer);
            true
        } else {
            false
        }
    }

    /// Stops the stream from `peer`, if one is open; whether one was.
    pub(crate) fn stop_mic(&self, peer: &DeviceId) -> bool {
        let Some(signal) = self.mic_stops.lock().unwrap_or_else(|e| e.into_inner()).get(peer).cloned() else {
            return false;
        };
        signal.notify_one();
        true
    }
}
