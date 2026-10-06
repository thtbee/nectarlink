// SPDX-License-Identifier: MPL-2.0
//! Screen mirroring (docs/protocol/mirror.md): a PC asks a phone for its
//! screen; once the phone's user agrees, the phone streams encoded video
//! on a stream of its own, which the PC decodes and shows.
//!
//! The phone side keeps latency low over throughput: when the network
//! can't keep up, frames are dropped (never queued for long) and the
//! encoder is asked for a keyframe to start clean from.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use iroh::endpoint::{RecvStream, SendStream, VarInt};
use nectarlink_protocol::{
    DeviceId, Envelope, ErrorCode, PacketKind,
    messages::{MirrorConfig, MirrorStart, StreamHeader, mirror, types},
    read_video_packet, video_packet_header, write_frame,
};
use tokio::sync::mpsc;

use crate::{Error, Result, events::NodeEvent, node::Shared, session::Session};

/// The device toggle that allows mirroring for a device.
pub(crate) const TOGGLE: &str = "mirroring";
/// Packets waiting to go out, at most. Beyond this the network is behind:
/// sending more would only add delay.
const QUEUE: usize = 4;

/// What the PC's app gets of a mirroring stream. Called from the core's
/// threads; must return quickly (hand the work to a thread of its own).
pub trait MirrorSink: Send + Sync {
    /// The stream's format: first, and again when it changes.
    fn config(&self, config: MirrorConfig);
    /// Encoded video (Annex B), `keyframe` when it starts fresh.
    fn packet(&self, keyframe: bool, time_us: u64, data: Vec<u8>);
    /// The stream ended (the phone stopped, or the connection dropped).
    fn ended(&self);
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
    let env = Envelope::new(types::MIRROR_START, &options)?;
    session.request(env, crate::session::REQUEST_TIMEOUT).await?.expect(types::OK)?;
    Ok(())
}

pub(crate) async fn stop(session: &Session) -> Result<()> {
    session
        .request(Envelope::empty(types::MIRROR_STOP), crate::session::REQUEST_TIMEOUT)
        .await?
        .expect(types::OK)?;
    Ok(())
}

pub(crate) async fn request_keyframe(session: &Session) {
    let _ = session.send(Envelope::empty(types::MIRROR_KEYFRAME)).await;
}

/// A phone opened a video stream (its header already read).
pub(crate) async fn receive(shared: Arc<Shared>, peer: DeviceId, mut send: SendStream, mut recv: RecvStream) {
    let shows = shared.local_capabilities().iter().any(|c| c == mirror::VIEW);
    let sink = shows.then(|| shared.platform.mirror_sink(&peer)).flatten();
    let Some(sink) = sink.filter(|_| shared.toggle_on(&peer, TOGGLE)) else {
        let _ = send.reset(VarInt::from_u32(mirror::STOPPED));
        let _ = recv.stop(VarInt::from_u32(mirror::STOPPED));
        return;
    };
    shared.emit(NodeEvent::Mirroring { device: peer, on: true });
    let stopped = shared.new_mirror_stop(&peer);
    loop {
        let packet = tokio::select! {
            _ = stopped.notified() => break,
            packet = read_video_packet(&mut recv) => packet,
        };
        match packet {
            Ok(Some(p)) if p.kind == PacketKind::Config => match MirrorConfig::from_cbor(&p.data) {
                Ok(config) if config.is_valid() => sink.config(config),
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
    shared.emit(NodeEvent::Mirroring { device: peer, on: false });
}

// ---- The phone ----

/// Handles `mirror.*` on a session. Returns false for other types.
pub(crate) async fn handle(shared: &Arc<Shared>, session: &Arc<Session>, env: &Envelope) -> Result<bool> {
    let peer = session.peer;
    let platform = shared.platform.clone();
    match env.t.as_str() {
        types::MIRROR_START => {
            let options: MirrorStart = env.body()?;
            let reply = if !shared.toggle_on(&peer, TOGGLE) {
                Envelope::error(ErrorCode::Denied, "mirroring is off for this device")
            } else if !shared.local_capabilities().iter().any(|c| c == mirror::CAPTURE) {
                Envelope::error(ErrorCode::Unsupported, "this device doesn't share its screen")
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
            // The PC's side: stop showing; the phone's: stop sharing.
            shared.stop_showing(&peer);
            tokio::task::spawn_blocking(move || platform.mirror_stop_requested(&peer));
            session.send(Envelope::empty(types::OK).reply_to(env.id)).await?;
        }
        types::MIRROR_KEYFRAME => {
            tokio::task::spawn_blocking(move || platform.mirror_keyframe_requested(&peer));
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// What became of a packet handed to [`MirrorStream::send`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirrorSend {
    Queued,
    /// Dropped to keep the delay low: send a keyframe next (frames are
    /// dropped until one comes).
    NeedKeyframe,
    /// The PC stopped watching, or the connection is gone: stop sharing.
    Closed,
}

/// A phone's video stream to one PC.
#[derive(Debug)]
pub struct MirrorStream {
    queue: mpsc::Sender<(PacketKind, u64, Vec<u8>)>,
    closed: Arc<AtomicBool>,
    /// Frames are being dropped until a keyframe.
    resyncing: Mutex<bool>,
}

impl MirrorStream {
    /// Queues a packet without waiting. A config packet waits for room:
    /// the PC can't decode without it.
    pub fn send(&self, kind: PacketKind, time_us: u64, data: Vec<u8>) -> MirrorSend {
        if self.closed.load(Ordering::Acquire) {
            return MirrorSend::Closed;
        }
        let mut resyncing = self.resyncing.lock().unwrap_or_else(|e| e.into_inner());
        if *resyncing && kind == PacketKind::Frame {
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
            Err(()) => {
                *resyncing = true;
                MirrorSend::NeedKeyframe
            }
        }
    }

    /// Ends the stream (the phone stopped sharing).
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire) || self.queue.is_closed()
    }
}

/// Opens a video stream to a PC that asked for the screen.
pub(crate) async fn open(shared: &Arc<Shared>, session: &Session) -> Result<MirrorStream> {
    if !shared.toggle_on(&session.peer, TOGGLE) {
        return Err(Error::Denied);
    }
    let (mut send, mut recv) = session.conn.open_bi().await.map_err(crate::error::net)?;
    let header =
        StreamHeader { svc: mirror::SERVICE.into(), op: mirror::OP_VIDEO.into(), v: mirror::VERSION };
    write_frame(&mut send, &Envelope::new(types::STREAM, &header)?.to_cbor()).await?;
    // Video goes out as soon as it's written.
    let _ = send.set_priority(1);
    let (queue, mut packets) = mpsc::channel::<(PacketKind, u64, Vec<u8>)>(QUEUE);
    let closed = Arc::new(AtomicBool::new(false));
    let done = closed.clone();
    tokio::spawn(async move {
        let mut ended = Box::pin(async move {
            // The PC stops reading (STOP_SENDING) when it's done watching.
            let _ = recv.read_to_end(64).await;
        });
        loop {
            let next = tokio::select! {
                _ = &mut ended => break,
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
    });
    Ok(MirrorStream { queue, closed, resyncing: Mutex::new(false) })
}

impl Shared {
    /// A new stop signal for the PC's reader of `peer`'s stream.
    fn new_mirror_stop(&self, peer: &DeviceId) -> Arc<tokio::sync::Notify> {
        let signal = Arc::new(tokio::sync::Notify::new());
        self.mirror_stops.lock().unwrap_or_else(|e| e.into_inner()).insert(*peer, signal.clone());
        signal
    }

    /// Stops showing `peer`'s screen here (kept until the reader sees it).
    pub(crate) fn stop_showing(&self, peer: &DeviceId) {
        if let Some(signal) = self.mirror_stops.lock().unwrap_or_else(|e| e.into_inner()).remove(peer) {
            signal.notify_one();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn a_full_queue_drops_frames_until_a_keyframe() {
        let (queue, mut packets) = mpsc::channel(2);
        let stream = MirrorStream { queue, closed: Arc::default(), resyncing: Mutex::new(false) };
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
}
