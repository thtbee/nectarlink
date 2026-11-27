// SPDX-License-Identifier: MPL-2.0
//! A call's sound on a PC (`docs/protocol/calls.md` §2.6): the phone carries
//! the other party's voice to the PC (`downlink` stream) and the PC's
//! microphone into the call (`uplink` stream), each its own QUIC stream of
//! 16 kHz mono 16-bit PCM packets, after the PC's `call.audio.start`.
//!
//! The same streams carry a PC's "Test call audio" with no call
//! (`call.audio.test.start`): the PC's microphone goes to the phone's speaker
//! and straight back. It has its own stream ops and stop signals, so a test
//! never touches a call's sound.

use std::sync::Arc;

use iroh::endpoint::{RecvStream, SendStream, VarInt};
use nectarlink_protocol::{
    DeviceId, Envelope, ErrorCode, PacketKind,
    messages::{CallAudioStart, MirrorAudioConfig, StreamHeader, calls, types},
    read_video_packet, video_packet_header, write_frame,
};

use crate::{Error, MirrorStream, Result, events::NodeEvent, node::Shared, session::Session};

/// The device toggle that lets a call's sound move between these devices.
pub(crate) const TOGGLE: &str = "call_audio";

/// Whether calls, and their sound moving, are on for `peer`.
fn allowed(shared: &Shared, peer: &DeviceId) -> bool {
    shared.toggle_on(peer, crate::calls::TOGGLE) && shared.toggle_on(peer, TOGGLE)
}

/// Whether a stream's format is the call's: 16 kHz mono.
fn is_call_format(config: &MirrorAudioConfig) -> bool {
    config.is_valid() && config.rate == calls::AUDIO_RATE && config.channels == 1
}

/// Reads a stream's first packet: the call's format, or nothing.
async fn read_config(recv: &mut RecvStream) -> Option<MirrorAudioConfig> {
    match read_video_packet(recv).await {
        Ok(Some(p)) if p.kind == PacketKind::Config => {
            MirrorAudioConfig::from_cbor(&p.data).ok().filter(is_call_format)
        }
        _ => None,
    }
}

fn refuse(mut send: SendStream, mut recv: RecvStream) {
    let _ = send.reset(VarInt::from_u32(calls::AUDIO_STOPPED));
    let _ = recv.stop(VarInt::from_u32(calls::AUDIO_STOPPED));
}

// ---- The PC ----

/// Asks the phone to carry call `id`'s sound here. The phone answers once
/// its side is ready; the `downlink` stream follows.
pub(crate) async fn start(shared: &Shared, session: &Session, id: String) -> Result<()> {
    if !allowed(shared, &session.peer) {
        return Err(Error::Denied);
    }
    if !shared.store.get_peer(&session.peer)?.is_some_and(|p| p.caps.contains(calls::AUDIO)) {
        return Err(Error::Unsupported);
    }
    // A call's sound comes first: a test in progress ends.
    shared.stop_call_audio_test(&session.peer);
    let env = Envelope::new(types::CALL_AUDIO_START, &CallAudioStart { id })?;
    session.request(env, crate::session::REQUEST_TIMEOUT).await?.expect(types::OK)?;
    Ok(())
}

pub(crate) async fn stop(session: &Session) -> Result<()> {
    session.send(Envelope::empty(types::CALL_AUDIO_STOP)).await
}

/// The phone opened its `downlink` stream (header already read). The
/// platform's sink takes it only while this PC asked for the call's sound.
pub(crate) async fn receive_downlink(
    shared: Arc<Shared>,
    peer: DeviceId,
    send: SendStream,
    mut recv: RecvStream,
) {
    if !allowed(&shared, &peer) {
        return refuse(send, recv);
    }
    let Some(sink) = shared.platform.call_audio_sink(&peer) else {
        return refuse(send, recv);
    };
    let Some(config) = read_config(&mut recv).await else {
        tracing::warn!("a phone sent call sound in a format this PC doesn't take");
        return refuse(send, recv);
    };
    sink.config(config);
    pump(&shared, peer, send, recv, |time_us, data| sink.audio(time_us, data)).await;
    sink.ended();
}

// ---- The phone ----

pub(crate) async fn handle(shared: &Arc<Shared>, session: &Arc<Session>, env: &Envelope) -> Result<bool> {
    let peer = session.peer;
    let platform = shared.platform.clone();
    match env.t.as_str() {
        types::CALL_AUDIO_START => {
            let start: CallAudioStart = env.body()?;
            let reply = if !allowed(shared, &peer) {
                Envelope::error(ErrorCode::Denied, "calls are off for this device")
            } else if !shared.we_offer(calls::AUDIO) {
                Envelope::error(ErrorCode::Unsupported, "this phone doesn't carry a call's sound")
            } else if !start.is_valid()
                || !shared.calls.get().is_some_and(|c| c.id == start.id && c.state == calls::ACTIVE)
            {
                Envelope::error(ErrorCode::NotFound, "no such call in progress")
            } else {
                // A call's sound comes first: a test in progress ends.
                shared.stop_call_audio_test(&peer);
                match tokio::task::spawn_blocking(move || platform.call_audio_requested(&peer, &start.id))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
                {
                    Ok(()) => Envelope::empty(types::OK),
                    Err(reason) => {
                        tracing::warn!(reason, "can't carry the call's sound");
                        Envelope::error(ErrorCode::Unsupported, "the phone can't reach the call's sound")
                    }
                }
            };
            session.send(reply.reply_to(env.id)).await?;
        }
        types::CALL_AUDIO_STOP => {
            // Either side: the phone stops carrying the sound, the PC stops playing it.
            shared.stop_call_audio(&peer);
            tokio::task::spawn_blocking(move || platform.call_audio_stop_requested(&peer));
            if env.id.is_some() {
                session.send(Envelope::empty(types::OK).reply_to(env.id)).await?;
            }
        }
        types::CALL_AUDIO_TEST_START => {
            let reply = if !allowed(shared, &peer) {
                Envelope::error(ErrorCode::Denied, "calls are off for this device")
            } else if !shared.we_offer(calls::AUDIO_TEST) {
                Envelope::error(ErrorCode::Unsupported, "this phone doesn't loop a test sound back")
            } else if shared.call_audio_stops.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&peer) {
                // A call's sound is on this PC: the test would cut into it.
                Envelope::error(ErrorCode::Busy, "a call's sound is on the PC")
            } else {
                shared.arm_call_audio_test(&peer);
                Envelope::empty(types::OK)
            };
            session.send(reply.reply_to(env.id)).await?;
        }
        types::CALL_AUDIO_TEST_STOP => {
            shared.stop_call_audio_test(&peer);
            if env.id.is_some() {
                session.send(Envelope::empty(types::OK).reply_to(env.id)).await?;
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// The PC opened its `uplink` stream (header already read): its microphone,
/// handed on as [`NodeEvent::CallAudioUplink`] for the phone to put into the call.
pub(crate) async fn receive_uplink(
    shared: Arc<Shared>,
    peer: DeviceId,
    send: SendStream,
    mut recv: RecvStream,
) {
    if !allowed(&shared, &peer) || !shared.we_offer(calls::AUDIO) {
        return refuse(send, recv);
    }
    if read_config(&mut recv).await.is_none() {
        tracing::warn!("a PC sent its microphone in a format the call doesn't take");
        return refuse(send, recv);
    }
    let events = shared.clone();
    pump(&shared, peer, send, recv, move |_, pcm| {
        events.emit(NodeEvent::CallAudioUplink { device: peer, pcm })
    })
    .await;
}

// ---- Both ----

/// Opens this device's side of the call's sound: `downlink` from a phone,
/// `uplink` from a PC. Write a config packet first, then the sound.
pub(crate) async fn open(shared: &Arc<Shared>, session: &Session, uplink: bool) -> Result<MirrorStream> {
    if !allowed(shared, &session.peer) {
        return Err(Error::Denied);
    }
    let (mut send, recv) = session.conn.open_bi().await.map_err(crate::error::net)?;
    let op = if uplink { calls::OP_UPLINK } else { calls::OP_DOWNLINK };
    write_stream_header(&mut send, op).await?;
    Ok(MirrorStream::spawn_stream(send, recv, true, false, None))
}

/// Hands each sound packet on until the stream ends or `call.audio.stop`.
async fn pump(
    shared: &Arc<Shared>,
    peer: DeviceId,
    send: SendStream,
    recv: RecvStream,
    audio: impl FnMut(u64, Vec<u8>),
) {
    let stopped = shared.new_call_audio_stop(&peer);
    pump_until(&stopped, send, recv, audio).await;
    shared.clear_call_audio_stop(&peer, &stopped);
}

/// [`pump`] until `stopped` fires.
async fn pump_until(
    stopped: &tokio::sync::Notify,
    mut send: SendStream,
    mut recv: RecvStream,
    mut audio: impl FnMut(u64, Vec<u8>),
) {
    loop {
        let packet = tokio::select! {
            _ = stopped.notified() => break,
            packet = read_video_packet(&mut recv) => packet,
        };
        match packet {
            Ok(Some(p)) if p.kind == PacketKind::Config => {
                if !MirrorAudioConfig::from_cbor(&p.data).is_ok_and(|c| is_call_format(&c)) {
                    break;
                }
            }
            Ok(Some(p)) => audio(p.time_us, p.data),
            Ok(None) => break,
            Err(e) => {
                tracing::debug!(error = %e, "call sound stream ended");
                break;
            }
        }
    }
    let _ = recv.stop(VarInt::from_u32(calls::AUDIO_STOPPED));
    let _ = send.finish();
}

// ---- The test: no call, the PC's microphone to the phone and back ----

/// Asks the phone for a test: it plays what the PC sends and sends it back.
/// Then open the PC's side with [`open_test`]; the phone's side arrives
/// through [`Platform::call_audio_test_sink`](crate::Platform::call_audio_test_sink).
pub(crate) async fn test_start(shared: &Shared, session: &Session) -> Result<()> {
    if !allowed(shared, &session.peer) {
        return Err(Error::Denied);
    }
    if !shared.store.get_peer(&session.peer)?.is_some_and(|p| p.caps.contains(calls::AUDIO_TEST)) {
        return Err(Error::Unsupported);
    }
    shared.arm_call_audio_test(&session.peer);
    let asked = session
        .request(Envelope::empty(types::CALL_AUDIO_TEST_START), crate::session::REQUEST_TIMEOUT)
        .await
        .and_then(|reply| Ok(reply.expect(types::OK).map(drop)?));
    if asked.is_err() {
        shared.stop_call_audio_test(&session.peer);
    }
    asked
}

pub(crate) async fn test_stop(session: &Session) -> Result<()> {
    session.send(Envelope::empty(types::CALL_AUDIO_TEST_STOP)).await
}

/// Opens the PC's microphone stream of a test the phone agreed to.
pub(crate) async fn open_test(shared: &Arc<Shared>, session: &Session) -> Result<MirrorStream> {
    if !allowed(shared, &session.peer) {
        return Err(Error::Denied);
    }
    if shared.call_audio_test_signal(&session.peer).is_none() {
        return Err(Error::NotFound);
    }
    let (mut send, recv) = session.conn.open_bi().await.map_err(crate::error::net)?;
    write_stream_header(&mut send, calls::OP_TEST_UPLINK).await?;
    Ok(MirrorStream::spawn_stream(send, recv, true, false, None))
}

async fn write_stream_header(send: &mut SendStream, op: &str) -> Result<()> {
    let header = StreamHeader { svc: calls::AUDIO_SERVICE.into(), op: op.into(), v: calls::AUDIO_VERSION };
    write_frame(send, &Envelope::new(types::STREAM, &header)?.to_cbor()).await?;
    Ok(())
}

/// The PC's `test-uplink` stream arrived at the phone (header already read):
/// each packet goes to the phone's speaker ([`NodeEvent::CallAudioTestUplink`])
/// and straight back down a `test-downlink` stream, as it came.
pub(crate) async fn receive_test_uplink(
    shared: Arc<Shared>,
    peer: DeviceId,
    mut send: SendStream,
    mut recv: RecvStream,
) {
    let Some(stopped) = shared.call_audio_test_signal(&peer) else { return refuse(send, recv) };
    if !allowed(&shared, &peer) || !shared.we_offer(calls::AUDIO_TEST) {
        return refuse(send, recv);
    }
    let Some(config) = read_config(&mut recv).await else {
        tracing::warn!("a PC sent a test sound in a format the call doesn't take");
        return refuse(send, recv);
    };
    let Some(session) = shared.session(&peer) else { return refuse(send, recv) };
    let Ok((mut echo, echo_recv)) = session.conn.open_bi().await else { return refuse(send, recv) };
    let opened = async {
        write_stream_header(&mut echo, calls::OP_TEST_DOWNLINK).await?;
        write_packet(&mut echo, PacketKind::Config, 0, &config.to_cbor()).await
    }
    .await;
    if opened.is_err() {
        return refuse(send, recv);
    }
    loop {
        let packet = tokio::select! {
            _ = stopped.notified() => break,
            packet = read_video_packet(&mut recv) => packet,
        };
        match packet {
            Ok(Some(p)) if p.kind == PacketKind::Config => {
                if !MirrorAudioConfig::from_cbor(&p.data).is_ok_and(|c| is_call_format(&c)) {
                    break;
                }
            }
            Ok(Some(p)) => {
                shared.emit(NodeEvent::CallAudioTestUplink { device: peer, pcm: p.data.clone() });
                // Waits if the PC isn't reading: a test needs no more than that.
                let sent = tokio::select! {
                    _ = stopped.notified() => break,
                    sent = write_packet(&mut echo, PacketKind::Frame, p.time_us, &p.data) => sent,
                };
                if sent.is_err() {
                    break;
                }
            }
            Ok(None) | Err(_) => break,
        }
    }
    shared.emit(NodeEvent::CallAudioTestEnded { device: peer });
    let _ = recv.stop(VarInt::from_u32(calls::AUDIO_STOPPED));
    let _ = send.finish();
    let _ = echo.finish();
    drop(echo_recv);
    shared.clear_call_audio_test(&peer, &stopped);
}

async fn write_packet(send: &mut SendStream, kind: PacketKind, time_us: u64, data: &[u8]) -> Result<()> {
    let header = video_packet_header(kind, time_us, data.len())?;
    send.write_all(&header).await.map_err(crate::error::net)?;
    send.write_all(data).await.map_err(crate::error::net)?;
    Ok(())
}

/// The phone's `test-downlink` stream arrived at the PC (header already
/// read): the PC's own sound back, for its call output. The platform's sink
/// takes it only while this PC asked for a test.
pub(crate) async fn receive_test_downlink(
    shared: Arc<Shared>,
    peer: DeviceId,
    send: SendStream,
    mut recv: RecvStream,
) {
    let Some(stopped) = shared.call_audio_test_signal(&peer) else { return refuse(send, recv) };
    if !allowed(&shared, &peer) {
        return refuse(send, recv);
    }
    let Some(sink) = shared.platform.call_audio_test_sink(&peer) else { return refuse(send, recv) };
    let Some(config) = read_config(&mut recv).await else {
        tracing::warn!("a phone sent a test sound in a format this PC doesn't take");
        return refuse(send, recv);
    };
    sink.config(config);
    pump_until(&stopped, send, recv, |time_us, data| sink.audio(time_us, data)).await;
    sink.ended();
    shared.clear_call_audio_test(&peer, &stopped);
}

impl Shared {
    fn new_call_audio_stop(&self, peer: &DeviceId) -> Arc<tokio::sync::Notify> {
        let signal = Arc::new(tokio::sync::Notify::new());
        // A PC has one stream in (downlink), a phone one (uplink): a new one replaces the old.
        if let Some(prev) =
            self.call_audio_stops.lock().unwrap_or_else(|e| e.into_inner()).insert(*peer, signal.clone())
        {
            prev.notify_one();
        }
        signal
    }

    fn clear_call_audio_stop(&self, peer: &DeviceId, signal: &Arc<tokio::sync::Notify>) {
        let mut stops = self.call_audio_stops.lock().unwrap_or_else(|e| e.into_inner());
        if stops.get(peer).is_some_and(|cur| Arc::ptr_eq(cur, signal)) {
            stops.remove(peer);
        }
    }

    /// A test with `peer` begins (either side): its streams are taken from
    /// now on, and an earlier test ends.
    pub(crate) fn arm_call_audio_test(&self, peer: &DeviceId) {
        let signal = Arc::new(tokio::sync::Notify::new());
        let prev = self.call_audio_tests.lock().unwrap_or_else(|e| e.into_inner()).insert(*peer, signal);
        if let Some(prev) = prev {
            prev.notify_one();
        }
    }

    /// The stop signal of the test under way with `peer`, if any.
    fn call_audio_test_signal(&self, peer: &DeviceId) -> Option<Arc<tokio::sync::Notify>> {
        self.call_audio_tests.lock().unwrap_or_else(|e| e.into_inner()).get(peer).cloned()
    }

    fn clear_call_audio_test(&self, peer: &DeviceId, signal: &Arc<tokio::sync::Notify>) {
        let mut tests = self.call_audio_tests.lock().unwrap_or_else(|e| e.into_inner());
        if tests.get(peer).is_some_and(|cur| Arc::ptr_eq(cur, signal)) {
            tests.remove(peer);
        }
    }

    /// Ends the test with `peer`, if any.
    pub(crate) fn stop_call_audio_test(&self, peer: &DeviceId) {
        if let Some(signal) = self.call_audio_tests.lock().unwrap_or_else(|e| e.into_inner()).remove(peer) {
            signal.notify_one();
        }
    }

    /// Ends the call sound arriving from `peer`, if any.
    pub(crate) fn stop_call_audio(&self, peer: &DeviceId) {
        if let Some(signal) = self.call_audio_stops.lock().unwrap_or_else(|e| e.into_inner()).get(peer) {
            signal.notify_one();
        }
    }
}
