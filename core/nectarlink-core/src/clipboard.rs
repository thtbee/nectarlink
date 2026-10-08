// SPDX-License-Identifier: MPL-2.0
//! The clipboard service (docs/protocol/clipboard.md): copied text and
//! images sent to a paired device, which puts them on its own clipboard.
//! Text goes on the control stream; images on a stream of their own.

use std::{sync::Arc, time::Duration};

use iroh::endpoint::{RecvStream, SendStream};
use nectarlink_protocol::{
    DeviceId, Envelope, ErrorCode,
    messages::{ClipImage, ClipSet, StreamHeader, clip, types},
    read_frame, write_frame,
};

use crate::{Error, Result, error::net, events::NodeEvent, node::Shared, session::Session};

/// How long an image may take to arrive, or to be answered: generous, for
/// a large screenshot on a slow link.
const IMAGE_TIMEOUT: Duration = Duration::from_secs(60);
/// Remote capability for receiving images.
const IMAGE_CAPABILITY: &str = "clip.image";

/// The device toggle that allows the clipboard for a device.
pub(crate) const TOGGLE: &str = "clipboard";

/// Handles `clip.set`. Returns false for other message types.
pub(crate) async fn handle(shared: &Arc<Shared>, session: &Arc<Session>, env: &Envelope) -> Result<bool> {
    if env.t != types::CLIP_SET {
        return Ok(false);
    }
    let peer = session.peer;
    let reply = if !shared.toggle_on(&peer, TOGGLE) {
        Envelope::error(ErrorCode::Denied, "the clipboard is off for this device")
    } else {
        match env.body::<ClipSet>() {
            Ok(clip) if clip.is_valid() => {
                let platform = shared.platform.clone();
                let text = clip.text;
                let written = {
                    let text = text.clone();
                    tokio::task::spawn_blocking(move || platform.set_clipboard(&text)).await
                };
                match written {
                    Ok(Ok(())) => {
                        let peer_name = shared.peer_name(&peer);
                        if let Some((clip_id, evicted)) =
                            shared.clipboard_history.record_text_with_id(&text, &peer_name, true)
                        {
                            shared.record_timeline_clip(peer, &peer_name, true, "text", clip_id, evicted);
                            shared.emit(NodeEvent::ClipboardHistoryChanged);
                        }
                        let suggestion = crate::clip_kind::classify_clip(&text);
                        if let Some(ref s) = suggestion {
                            tracing::debug!(from = %peer.short(), kind = ?s.kind, "classified received clipboard text");
                        }
                        *shared.last_clip_suggestion.lock().unwrap_or_else(|e| e.into_inner()) =
                            suggestion.map(|s| (peer, s));
                        shared.emit(NodeEvent::ClipboardReceived { device: peer });
                        Envelope::empty(types::OK)
                    }
                    Ok(Err(reason)) => {
                        tracing::warn!(reason, "can't write the clipboard");
                        Envelope::error(ErrorCode::Internal, "can't write the clipboard")
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "clipboard writer failed");
                        Envelope::error(ErrorCode::Internal, "can't write the clipboard")
                    }
                }
            }
            _ => Envelope::error(ErrorCode::BadMessage, "clipboard text is empty or too large"),
        }
    };
    session.send(reply.reply_to(env.id)).await?;
    Ok(true)
}

/// Sends an image on its own stream and waits for the receiver's answer.
pub(crate) async fn send_image(
    shared: &Arc<Shared>,
    session: &Session,
    mime: String,
    bytes: Vec<u8>,
) -> Result<()> {
    let peer = session.peer;
    let image = ClipImage { mime, size: bytes.len() as u64 };
    if image.size > clip::MAX_IMAGE_BYTES {
        return Err(Error::TooLarge);
    }
    if !image.is_valid() {
        return Err(Error::Internal("not a PNG or JPEG image".into()));
    }
    if !shared.toggle_on(&peer, TOGGLE) {
        return Err(Error::Denied);
    }
    let takes_images = shared.store.get_peer(&peer)?.is_some_and(|p| p.caps.contains(IMAGE_CAPABILITY));
    if !takes_images {
        return Err(Error::Unsupported);
    }
    let exchange = async {
        let (mut send, mut recv) = session.conn.open_bi().await.map_err(net)?;
        let header = StreamHeader { svc: clip::SERVICE.into(), op: clip::OP_IMAGE.into(), v: clip::VERSION };
        write_frame(&mut send, &Envelope::new(types::STREAM, &header)?.to_cbor()).await?;
        write_frame(&mut send, &Envelope::new(clip::IMAGE, &image)?.to_cbor()).await?;
        // A receiver that refuses stops reading; its answer still comes.
        let written = send.write_all(&bytes).await;
        let _ = send.finish();
        let reply = read_frame(&mut recv).await?.ok_or(Error::Offline)?;
        Envelope::from_cbor(&reply)?.expect(types::OK)?;
        written.map_err(net)?;
        Ok::<(), Error>(())
    };
    tokio::time::timeout(IMAGE_TIMEOUT, exchange).await.map_err(|_| Error::Timeout)??;
    let peer_name = shared.peer_name(&peer);
    if let Some((clip_id, evicted)) =
        shared.clipboard_history.record_image_with_id(&image.mime, &bytes, &peer_name, false)
    {
        shared.record_timeline_clip(peer, &peer_name, false, &image.mime, clip_id, evicted);
        shared.emit(NodeEvent::ClipboardHistoryChanged);
    }
    Ok(())
}

/// Receives an image (the stream header was read) and puts it on the
/// clipboard.
pub(crate) async fn receive_image(
    shared: Arc<Shared>,
    peer: DeviceId,
    mut send: SendStream,
    mut recv: RecvStream,
) {
    let reply = match tokio::time::timeout(IMAGE_TIMEOUT, take_image(&shared, peer, &mut recv)).await {
        Ok(Ok(())) => Envelope::empty(types::OK),
        Ok(Err((code, msg))) => Envelope::error(code, msg),
        Err(_) => Envelope::error(ErrorCode::Internal, "the image took too long"),
    };
    if reply.t != types::OK {
        let _ = recv.stop(0u32.into());
    }
    let _ = write_frame(&mut send, &reply.to_cbor()).await;
    let _ = send.finish();
    let _ = send.stopped().await;
}

async fn take_image(
    shared: &Arc<Shared>,
    peer: DeviceId,
    recv: &mut RecvStream,
) -> std::result::Result<(), (ErrorCode, &'static str)> {
    let image = read_frame(recv)
        .await
        .ok()
        .flatten()
        .and_then(|bytes| Envelope::from_cbor(&bytes).ok())
        .and_then(|env| env.expect_body::<ClipImage>(clip::IMAGE).ok())
        .filter(ClipImage::is_valid)
        .ok_or((ErrorCode::BadMessage, "invalid image"))?;
    if !shared.toggle_on(&peer, TOGGLE) {
        return Err((ErrorCode::Denied, "the clipboard is off for this device"));
    }
    let size = usize::try_from(image.size).map_err(|_| (ErrorCode::BadMessage, "invalid image"))?;
    let mut bytes = Vec::with_capacity(size.min(64 * 1024));
    let mut buf = vec![0u8; 64 * 1024];
    while bytes.len() < size {
        let want = (size - bytes.len()).min(buf.len());
        match recv.read(&mut buf[..want]).await {
            Ok(Some(n)) if n > 0 => bytes.extend_from_slice(&buf[..n]),
            _ => return Err((ErrorCode::BadMessage, "the image ended early")),
        }
    }
    let platform = shared.platform.clone();
    let mime = image.mime;
    let written = {
        let mime = mime.clone();
        let bytes = bytes.clone();
        tokio::task::spawn_blocking(move || platform.set_clipboard_image(&mime, &bytes)).await
    };
    match written {
        Ok(Ok(())) => {
            let peer_name = shared.peer_name(&peer);
            if let Some((clip_id, evicted)) =
                shared.clipboard_history.record_image_with_id(&mime, &bytes, &peer_name, true)
            {
                shared.record_timeline_clip(peer, &peer_name, true, &mime, clip_id, evicted);
                shared.emit(NodeEvent::ClipboardHistoryChanged);
            }
            shared.emit(NodeEvent::ClipboardReceived { device: peer });
            Ok(())
        }
        Ok(Err(reason)) => {
            tracing::warn!(reason, "can't put an image on the clipboard");
            Err((ErrorCode::Internal, "can't write the clipboard"))
        }
        Err(e) => {
            tracing::warn!(error = %e, "clipboard writer failed");
            Err((ErrorCode::Internal, "can't write the clipboard"))
        }
    }
}
