// SPDX-License-Identifier: MPL-2.0
//! The clipboard service (docs/protocol/clipboard.md): copied text sent to
//! a paired device, which puts it on its own clipboard.

use std::sync::Arc;

use nectarlink_protocol::{
    Envelope, ErrorCode,
    messages::{ClipSet, types},
};

use crate::{Result, events::NodeEvent, node::Shared, session::Session};

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
                let written = tokio::task::spawn_blocking(move || platform.set_clipboard(&clip.text)).await;
                match written {
                    Ok(Ok(())) => {
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
