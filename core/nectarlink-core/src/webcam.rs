// SPDX-License-Identifier: MPL-2.0
//! Phone as webcam (`docs/protocol/webcam.md`): a phone streams encoded H.264
//! video from its camera on a dedicated QUIC stream (`webcam/video`), either
//! started directly from the phone's Webcam screen or after the phone's user
//! accepts a PC's `webcam.start` request.

use std::sync::Arc;

use iroh::endpoint::{RecvStream, SendStream, VarInt};
use nectarlink_protocol::{
    DeviceId, Envelope, ErrorCode, PacketKind,
    messages::{StreamHeader, WebcamConfig, WebcamStart, types, webcam},
    read_video_packet, write_frame,
};

use crate::{Error, MirrorStream, Result, events::NodeEvent, node::Shared, session::Session};

/// The device toggle that allows webcam streaming for a device.
pub const TOGGLE: &str = "webcam";

/// What the PC's app gets of a phone's webcam stream. Called from the core's
/// threads; must return quickly (hand decoding off to a dedicated thread).
pub trait WebcamSink: Send + Sync {
    /// The stream's format: first, and again whenever camera or resolution changes.
    fn config(&self, config: WebcamConfig);
    /// Encoded video (Annex B H.264), `keyframe` when it starts fresh (SPS+PPS+IDR).
    fn packet(&self, keyframe: bool, time_us: u64, data: Vec<u8>);
    /// The stream ended (the phone stopped, or the connection dropped).
    fn ended(&self);
}

impl std::fmt::Debug for dyn WebcamSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WebcamSink")
    }
}

// ---- The PC ----

pub(crate) async fn start(shared: &Shared, session: &Session, options: WebcamStart) -> Result<()> {
    if !shared.toggle_on(&session.peer, TOGGLE) {
        return Err(Error::Denied);
    }
    if !options.is_valid() {
        return Err(Error::Protocol("invalid webcam options".into()));
    }
    let env = Envelope::new(types::WEBCAM_START, &options)?;
    session.request(env, crate::session::REQUEST_TIMEOUT).await?.expect(types::WEBCAM_OK)?;
    Ok(())
}

pub(crate) async fn stop(session: &Session) -> Result<()> {
    session.send(Envelope::empty(types::WEBCAM_STOP)).await
}

pub(crate) async fn request_keyframe(session: &Session) {
    let _ = session.send(Envelope::empty(types::WEBCAM_KEYFRAME)).await;
}

/// A phone opened a `webcam/video` stream (its stream header already read).
pub(crate) async fn receive(shared: Arc<Shared>, peer: DeviceId, mut send: SendStream, mut recv: RecvStream) {
    let refuse = |mut send: SendStream, mut recv: RecvStream| {
        let _ = send.reset(VarInt::from_u32(webcam::STOPPED));
        let _ = recv.stop(VarInt::from_u32(webcam::STOPPED));
    };
    let supports = shared.local_capabilities().iter().any(|c| c == webcam::VIRTUAL);
    if !supports || !shared.toggle_on(&peer, TOGGLE) {
        return refuse(send, recv);
    }
    let first = match read_video_packet(&mut recv).await {
        Ok(Some(p)) if p.kind == PacketKind::Config => {
            WebcamConfig::from_cbor(&p.data).ok().filter(WebcamConfig::is_valid)
        }
        _ => None,
    };
    let Some(config) = first else {
        tracing::warn!("a phone sent a webcam format this PC can't decode");
        return refuse(send, recv);
    };
    let Some(sink) = shared.platform.webcam_sink(&peer) else {
        return refuse(send, recv);
    };
    let stopped = shared.new_webcam_stop(&peer);
    shared.emit(NodeEvent::Webcam { device: peer, on: true });
    sink.config(config);
    loop {
        let packet = tokio::select! {
            _ = stopped.notified() => break,
            packet = read_video_packet(&mut recv) => packet,
        };
        match packet {
            Ok(Some(p)) if p.kind == PacketKind::Config => match WebcamConfig::from_cbor(&p.data) {
                Ok(config) if config.is_valid() => sink.config(config),
                _ => {
                    tracing::warn!("a phone sent a webcam format this PC can't decode");
                    break;
                }
            },
            Ok(Some(p)) => sink.packet(p.kind == PacketKind::Keyframe, p.time_us, p.data),
            Ok(None) => break,
            Err(e) => {
                tracing::debug!(error = %e, "webcam stream ended");
                break;
            }
        }
    }
    let _ = recv.stop(VarInt::from_u32(webcam::STOPPED));
    let _ = send.finish();
    sink.ended();
    if shared.clear_webcam_stop(&peer, &stopped) {
        shared.emit(NodeEvent::Webcam { device: peer, on: false });
    }
}

// ---- The phone ----

pub(crate) async fn handle(shared: &Arc<Shared>, session: &Arc<Session>, env: &Envelope) -> Result<bool> {
    let peer = session.peer;
    let platform = shared.platform.clone();
    match env.t.as_str() {
        types::WEBCAM_START => {
            let options: WebcamStart = env.body()?;
            let reply = if !shared.toggle_on(&peer, TOGGLE) {
                Envelope::error(ErrorCode::Denied, "webcam is off for this device")
            } else if !options.is_valid() {
                Envelope::error(ErrorCode::BadMessage, "invalid webcam options")
            } else if !shared.local_capabilities().iter().any(|c| c == webcam::STREAM) {
                Envelope::error(ErrorCode::Unsupported, "this device doesn't stream a camera")
            } else {
                match tokio::task::spawn_blocking(move || platform.webcam_requested(&peer, &options))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
                {
                    Ok(()) => Envelope::empty(types::WEBCAM_OK),
                    Err(reason) => {
                        tracing::warn!(reason, "can't ask to start the webcam");
                        Envelope::error(ErrorCode::Internal, "the phone couldn't start the webcam")
                    }
                }
            };
            session.send(reply.reply_to(env.id)).await?;
        }
        types::WEBCAM_STOP => {
            shared.stop_webcam(&peer);
            tokio::task::spawn_blocking(move || platform.webcam_stop_requested(&peer));
            if env.id.is_some() {
                session.send(Envelope::empty(types::OK).reply_to(env.id)).await?;
            }
        }
        types::WEBCAM_KEYFRAME => {
            tokio::task::spawn_blocking(move || platform.webcam_keyframe_requested(&peer));
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// Opens a webcam video stream from this phone to a connected PC.
pub(crate) async fn open(shared: &Arc<Shared>, session: &Session) -> Result<MirrorStream> {
    if !shared.toggle_on(&session.peer, TOGGLE) {
        return Err(Error::Denied);
    }
    let (mut send, recv) = session.conn.open_bi().await.map_err(crate::error::net)?;
    let header =
        StreamHeader { svc: webcam::SERVICE.into(), op: webcam::OP_VIDEO.into(), v: webcam::VERSION };
    write_frame(&mut send, &Envelope::new(types::STREAM, &header)?.to_cbor()).await?;
    Ok(MirrorStream::spawn_stream(send, recv, false))
}

impl Shared {
    fn new_webcam_stop(&self, peer: &DeviceId) -> Arc<tokio::sync::Notify> {
        let signal = Arc::new(tokio::sync::Notify::new());
        if let Some(prev) =
            self.webcam_stops.lock().unwrap_or_else(|e| e.into_inner()).insert(*peer, signal.clone())
        {
            prev.notify_one();
        }
        signal
    }

    fn clear_webcam_stop(&self, peer: &DeviceId, signal: &Arc<tokio::sync::Notify>) -> bool {
        let mut stops = self.webcam_stops.lock().unwrap_or_else(|e| e.into_inner());
        if stops.get(peer).is_some_and(|cur| Arc::ptr_eq(cur, signal)) {
            stops.remove(peer);
            true
        } else {
            false
        }
    }

    pub(crate) fn stop_webcam(&self, peer: &DeviceId) {
        if let Some(signal) = self.webcam_stops.lock().unwrap_or_else(|e| e.into_inner()).remove(peer) {
            signal.notify_one();
            self.emit(NodeEvent::Webcam { device: *peer, on: false });
        }
    }
}
