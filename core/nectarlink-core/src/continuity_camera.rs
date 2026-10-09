// SPDX-License-Identifier: MPL-2.0
//! Continuity Camera: a paired PC asks a phone to take a photo or scan a
//! document (`camera.capture.request`), and the phone streams the captured
//! image back on a dedicated `camera` / `result` stream.

use std::{sync::Arc, time::Duration};

use iroh::endpoint::{RecvStream, SendStream};
use nectarlink_protocol::{
    DeviceId, Envelope, ErrorCode,
    messages::{
        CameraCaptureCancel, CameraCaptureMode, CameraCaptureOk, CameraCaptureRequest,
        CameraCaptureResultMeta, StreamHeader, continuity_camera, types,
    },
    read_frame, write_frame,
};

use crate::{Error, NodeEvent, Result, error::net, node::Shared, session::Session};

/// Per-device toggle that gates Continuity Camera along with photo sharing.
pub const TOGGLE: &str = "photos";

const RESULT_TIMEOUT: Duration = Duration::from_secs(60);
const CHUNK_BYTES: usize = 64 * 1024;

/// Whether a Continuity Camera `request_id` meets the protocol bounds.
pub fn valid_request_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= continuity_camera::MAX_ID_BYTES && !id.chars().any(char::is_control)
}

fn detail_for_mode(mode: CameraCaptureMode) -> &'static str {
    match mode {
        CameraCaptureMode::Photo => "Continuity Camera · Photo",
        CameraCaptureMode::Scan => "Continuity Camera · Document scan",
    }
}

/// Asks a paired phone to open its camera UI and capture a photo or scan a document.
pub(crate) async fn request(
    shared: &Shared,
    peer: DeviceId,
    request_id: String,
    mode: CameraCaptureMode,
) -> Result<()> {
    if !valid_request_id(&request_id) {
        return Err(Error::Protocol("invalid continuity camera request_id".into()));
    }
    if !shared.store.is_paired(&peer)? {
        return Err(Error::NotPaired);
    }
    if !shared.toggle_on(&peer, TOGGLE) {
        return Err(Error::Denied);
    }
    let session = shared.session(&peer).ok_or(Error::Offline)?;
    let env = Envelope::new(types::CAMERA_CAPTURE_REQUEST, &CameraCaptureRequest { request_id, mode })?;
    session.send(env).await
}

/// Cancels an in-flight Continuity Camera capture request on either side.
pub(crate) async fn cancel(
    shared: &Shared,
    peer: DeviceId,
    request_id: String,
    reason: Option<String>,
) -> Result<()> {
    if !valid_request_id(&request_id) {
        return Err(Error::Protocol("invalid continuity camera request_id".into()));
    }
    if !shared.store.is_paired(&peer)? {
        return Err(Error::NotPaired);
    }
    let session = shared.session(&peer).ok_or(Error::Offline)?;
    let env = Envelope::new(types::CAMERA_CAPTURE_CANCEL, &CameraCaptureCancel { request_id, reason })?;
    session.send(env).await
}

/// Handles `camera.capture.*` messages on the control stream.
pub(crate) async fn handle(shared: &Arc<Shared>, session: &Arc<Session>, env: &Envelope) -> Result<bool> {
    match env.t.as_str() {
        types::CAMERA_CAPTURE_REQUEST => {
            let req: CameraCaptureRequest = env.body()?;
            if !valid_request_id(&req.request_id) {
                return Err(Error::Protocol("invalid continuity camera request_id".into()));
            }
            if !shared.toggle_on(&session.peer, TOGGLE) {
                let cancel = Envelope::new(
                    types::CAMERA_CAPTURE_CANCEL,
                    &CameraCaptureCancel {
                        request_id: req.request_id,
                        reason: Some("photos sharing is turned off".into()),
                    },
                )?;
                let _ = session.send(cancel).await;
                return Ok(true);
            }
            match shared.platform.camera_capture_requested(&session.peer, &req) {
                Ok(()) => {
                    let ok = Envelope::new(
                        types::CAMERA_CAPTURE_OK,
                        &CameraCaptureOk { request_id: req.request_id.clone() },
                    )?;
                    let _ = session.send(ok).await;
                    shared.emit(NodeEvent::CameraCaptureRequested {
                        device: session.peer,
                        request_id: req.request_id,
                        mode: req.mode,
                    });
                }
                Err(reason) => {
                    let cancel = Envelope::new(
                        types::CAMERA_CAPTURE_CANCEL,
                        &CameraCaptureCancel { request_id: req.request_id, reason: Some(reason) },
                    )?;
                    let _ = session.send(cancel).await;
                }
            }
            Ok(true)
        }
        types::CAMERA_CAPTURE_OK => {
            let ok: CameraCaptureOk = env.body()?;
            if !valid_request_id(&ok.request_id) {
                return Err(Error::Protocol("invalid continuity camera request_id".into()));
            }
            Ok(true)
        }
        types::CAMERA_CAPTURE_CANCEL => {
            let cancel: CameraCaptureCancel = env.body()?;
            if !valid_request_id(&cancel.request_id) {
                return Err(Error::Protocol("invalid continuity camera request_id".into()));
            }
            shared.platform.camera_capture_cancelled(&session.peer, &cancel.request_id);
            shared.emit(NodeEvent::CameraCaptureCancelled {
                device: session.peer,
                request_id: cancel.request_id,
                reason: cancel.reason,
            });
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Sends a captured photo or scanned document from the phone to the requesting PC.
pub(crate) async fn send_result(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    meta: CameraCaptureResultMeta,
    bytes: Vec<u8>,
) -> Result<()> {
    if !valid_request_id(&meta.request_id) {
        return Err(Error::Protocol("invalid continuity camera request_id".into()));
    }
    if meta.file_name.trim().is_empty() || meta.file_name.len() > continuity_camera::MAX_NAME_BYTES {
        return Err(Error::Protocol("invalid continuity camera file_name".into()));
    }
    if !matches!(meta.mime.as_str(), "image/jpeg" | "image/png") {
        return Err(Error::Protocol("unsupported continuity camera mime".into()));
    }
    if meta.size > continuity_camera::MAX_IMAGE_BYTES
        || bytes.len() as u64 > continuity_camera::MAX_IMAGE_BYTES
    {
        return Err(Error::TooLarge);
    }
    if meta.size == 0 || bytes.len() as u64 != meta.size {
        return Err(Error::Protocol("continuity camera size mismatch".into()));
    }
    if !shared.toggle_on(&session.peer, TOGGLE) {
        return Err(Error::Denied);
    }

    let safe_name = crate::transfer::safe_file_name(&meta.file_name);
    let clean_meta = CameraCaptureResultMeta { file_name: safe_name.clone(), ..meta };

    let exchange = async {
        let (mut send, mut recv) = session.conn.open_bi().await.map_err(net)?;
        let header = StreamHeader {
            svc: continuity_camera::SERVICE.into(),
            op: continuity_camera::OP_RESULT.into(),
            v: continuity_camera::VERSION,
        };
        write_frame(&mut send, &Envelope::new(types::STREAM, &header)?.to_cbor()).await?;
        write_frame(&mut send, &Envelope::new(continuity_camera::RESULT_META, &clean_meta)?.to_cbor())
            .await?;
        for chunk in bytes.chunks(CHUNK_BYTES) {
            if let Err(e) = send.write_all(chunk).await {
                let _ = send.finish();
                if let Ok(Some(reply_bytes)) = read_frame(&mut recv).await
                    && let Ok(env) = Envelope::from_cbor(&reply_bytes)
                {
                    env.expect(types::OK)?;
                }
                return Err(net(e));
            }
        }
        let _ = send.finish();
        let reply = read_frame(&mut recv).await?.ok_or(Error::Offline)?;
        Envelope::from_cbor(&reply)?.expect(types::OK)?;
        Ok::<(), Error>(())
    };

    tokio::time::timeout(RESULT_TIMEOUT, exchange).await.map_err(|_| Error::Timeout)??;

    let _ = shared.record_timeline(crate::timeline::NewTimelineEntry {
        kind: crate::TimelineKind::Photo,
        device_id: session.peer,
        device_name: shared.peer_name(&session.peer),
        incoming: false,
        timestamp: crate::now_unix(),
        title: safe_name,
        detail: detail_for_mode(clean_meta.mode).into(),
        target: String::new(),
        size_bytes: clean_meta.size,
        duration_secs: 0,
        ref_id: None,
    });

    Ok(())
}

/// Receives a Continuity Camera capture result stream on the PC.
pub(crate) async fn receive_result(
    shared: Arc<Shared>,
    peer: DeviceId,
    mut send: SendStream,
    mut recv: RecvStream,
) {
    let reply = match tokio::time::timeout(RESULT_TIMEOUT, take_result(&shared, peer, &mut recv)).await {
        Ok(Ok(())) => Envelope::empty(types::OK),
        Ok(Err((code, msg))) => Envelope::error(code, msg),
        Err(_) => Envelope::error(ErrorCode::Internal, "continuity camera transfer timed out"),
    };
    if reply.t != types::OK {
        let _ = recv.stop(0u32.into());
    }
    let _ = write_frame(&mut send, &reply.to_cbor()).await;
    let _ = send.finish();
    let _ = tokio::time::timeout(Duration::from_secs(2), send.stopped()).await;
}

async fn take_result(
    shared: &Arc<Shared>,
    peer: DeviceId,
    recv: &mut RecvStream,
) -> std::result::Result<(), (ErrorCode, &'static str)> {
    let meta = read_frame(recv)
        .await
        .ok()
        .flatten()
        .and_then(|bytes| Envelope::from_cbor(&bytes).ok())
        .and_then(|env| env.expect_body::<CameraCaptureResultMeta>(continuity_camera::RESULT_META).ok())
        .ok_or((ErrorCode::BadMessage, "invalid continuity camera metadata"))?;

    if !valid_request_id(&meta.request_id) {
        return Err((ErrorCode::BadMessage, "invalid request_id"));
    }
    if !matches!(meta.mime.as_str(), "image/jpeg" | "image/png") {
        return Err((ErrorCode::BadMessage, "unsupported continuity camera format"));
    }
    if meta.size == 0 || meta.size > continuity_camera::MAX_IMAGE_BYTES {
        return Err((ErrorCode::BadMessage, "invalid continuity camera size"));
    }
    if !shared.toggle_on(&peer, TOGGLE) {
        return Err((ErrorCode::Denied, "photos sharing is turned off"));
    }

    let size = usize::try_from(meta.size).map_err(|_| (ErrorCode::BadMessage, "image too large"))?;
    let mut data = Vec::with_capacity(size.min(CHUNK_BYTES));
    let mut buf = vec![0u8; CHUNK_BYTES];
    while data.len() < size {
        let want = (size - data.len()).min(buf.len());
        match recv.read(&mut buf[..want]).await {
            Ok(Some(n)) if n > 0 => data.extend_from_slice(&buf[..n]),
            _ => return Err((ErrorCode::BadMessage, "continuity camera stream ended early")),
        }
    }

    let default_ext = if meta.mime == "image/png" { "png" } else { "jpg" };
    let fallback_name = match meta.mode {
        CameraCaptureMode::Photo => format!("Photo.{default_ext}"),
        CameraCaptureMode::Scan => format!("Scan.{default_ext}"),
    };
    let raw_name = if meta.file_name.trim().is_empty() { &fallback_name } else { &meta.file_name };
    let safe_name = crate::transfer::safe_file_name(raw_name);

    let saved_path = match tokio::fs::create_dir_all(&shared.downloads_dir).await {
        Ok(()) => {
            let dest = crate::transfer::unique_path(&shared.downloads_dir, &safe_name);
            match tokio::fs::write(&dest, &data).await {
                Ok(()) => Some(dest),
                Err(e) => {
                    tracing::warn!(error = %e, "can't save continuity camera capture");
                    None
                }
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "can't create downloads dir for continuity camera capture");
            None
        }
    };

    let target = saved_path.as_ref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let _ = shared.record_timeline(crate::timeline::NewTimelineEntry {
        kind: crate::TimelineKind::Photo,
        device_id: peer,
        device_name: shared.peer_name(&peer),
        incoming: true,
        timestamp: crate::now_unix(),
        title: safe_name.clone(),
        detail: detail_for_mode(meta.mode).into(),
        target,
        size_bytes: meta.size,
        duration_secs: 0,
        ref_id: None,
    });

    shared.emit(NodeEvent::CameraCaptureReceived {
        device: peer,
        request_id: meta.request_id,
        mode: meta.mode,
        file_name: safe_name,
        mime: meta.mime,
        width: meta.width,
        height: meta.height,
        data,
        saved_path,
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_id_validation() {
        assert!(valid_request_id("req-1234"));
        assert!(!valid_request_id(""));
        assert!(!valid_request_id("bad\nid"));
        assert!(!valid_request_id(&"x".repeat(continuity_camera::MAX_ID_BYTES + 1)));
    }
}
