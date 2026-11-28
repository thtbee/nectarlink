// SPDX-License-Identifier: MPL-2.0
//! The chat between two paired devices (#698, docs/protocol/chat.md): notes
//! typed on one show on the other, beside the files, links and clips they
//! already share. Each note is a timeline entry of kind `message`, its text
//! in `detail` and `chat:<id>` in `ref_id`, so history, search and the
//! timeline's retention apply to it as to everything else shared.
//!
//! A note is changed only by the device that wrote it, and deleted by
//! either; both act only on notes of the chat with the device that asks.

use std::sync::Arc;

use nectarlink_protocol::{
    DeviceId, Envelope, ErrorCode,
    messages::{ChatDelete, ChatEdit, ChatMessage, types},
};

use crate::{
    Result, TimelineKind, events::NodeEvent, node::Shared, session::Session, timeline::NewTimelineEntry,
};

/// The timeline `ref_id` of the note `id`: kept apart from other entries' refs.
pub(crate) fn ref_id(id: &str) -> String {
    format!("chat:{id}")
}

/// A fresh note ID: 32 random hex digits.
pub(crate) fn new_id() -> String {
    format!("{:032x}", rand::random::<u128>())
}

/// Puts a note in the chat with `peer` on this device's timeline.
pub(crate) fn record(shared: &Shared, peer: DeviceId, incoming: bool, id: &str, text: &str) -> Option<i64> {
    shared.record_timeline(NewTimelineEntry {
        kind: TimelineKind::Message,
        device_id: peer,
        device_name: String::new(),
        incoming,
        timestamp: crate::now_unix(),
        title: String::new(),
        detail: text.to_owned(),
        target: String::new(),
        size_bytes: 0,
        duration_secs: 0,
        ref_id: Some(ref_id(id)),
    })
}

/// Handles a `chat.*` message on a session. Returns false for other types.
pub(crate) async fn handle(shared: &Arc<Shared>, session: &Arc<Session>, env: &Envelope) -> Result<bool> {
    let peer = session.peer;
    match env.t.as_str() {
        types::CHAT_MESSAGE => {
            let note: ChatMessage = env.body()?;
            let reply = if !note.is_valid() {
                Envelope::error(ErrorCode::BadMessage, "invalid chat note")
            } else {
                // Sent again after a lost reply: it's here already.
                if !shared.store.chat_note_exists(&peer, &ref_id(&note.id))? {
                    record(shared, peer, true, &note.id, &note.text);
                    shared.emit(NodeEvent::ChatReceived { device: peer, id: note.id, text: note.text });
                }
                Envelope::empty(types::OK)
            };
            if env.id.is_some() {
                session.send(reply.reply_to(env.id)).await?;
            }
        }
        types::CHAT_EDIT => {
            let edit: ChatEdit = env.body()?;
            // Only a note that device wrote, in the chat with it.
            if edit.is_valid() && shared.store.edit_chat_note(&peer, &ref_id(&edit.id), true, &edit.text)? {
                shared.emit(NodeEvent::TimelineChanged);
            }
        }
        types::CHAT_DELETE => {
            let delete: ChatDelete = env.body()?;
            if delete.is_valid() && shared.store.delete_chat_note(&peer, &ref_id(&delete.id))? {
                shared.emit(NodeEvent::TimelineChanged);
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}
