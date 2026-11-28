// SPDX-License-Identifier: GPL-3.0-or-later
//! The chat with each paired device (#698): notes typed here show on the
//! device and the other way round. The notes live on the timeline (see
//! `bridge::chat`); this sends, changes and deletes them, and shows a
//! notification for one that comes in while its chat isn't in front.

use std::sync::Mutex;

use nectarlink_core::{DeviceId, Error, NodeEvent};

use crate::{
    bridge::app::{describe, show_message},
    core_host,
    i18n::tr,
    win::toast::{self, Toast, ToastEvent},
};

/// The notifications' group; each device's latest note is keyed by its ID.
pub const TOAST_GROUP: &str = "nectarlink-chat";

/// Where the Chat page is: the device it shows, and whether it's in front.
struct Front {
    shown: Option<DeviceId>,
    in_front: bool,
}

static FRONT: Mutex<Front> = Mutex::new(Front { shown: None, in_front: false });

fn front<T>(f: impl FnOnce(&mut Front) -> T) -> T {
    f(&mut FRONT.lock().unwrap_or_else(|e| e.into_inner()))
}

/// The note ID in a chat note's timeline `ref_id` (`chat:<id>`).
pub fn note_id(ref_id: &str) -> Option<&str> {
    ref_id.strip_prefix("chat:").filter(|id| !id.is_empty())
}

/// The device whose chat the Chat page shows.
pub fn set_shown(device: Option<DeviceId>) {
    front(|f| f.shown = device);
    clear_toast_if_in_front();
}

/// Whether the Chat page is in front (shown, in the active window).
pub fn set_in_front(on: bool) {
    front(|f| f.in_front = on);
    clear_toast_if_in_front();
}

/// The device whose chat is in front, if one is.
fn in_front() -> Option<DeviceId> {
    front(|f| f.shown.filter(|_| f.in_front))
}

/// Its notes are being read: their notification has done its job.
fn clear_toast_if_in_front() {
    if let Some(device) = in_front() {
        toast::remove(TOAST_GROUP, &device.to_string());
    }
}

/// Sends `text` to `peer`; `done` gets the text back when it wasn't sent.
pub fn send(peer: DeviceId, text: String, done: impl FnOnce(Option<String>) + Send + 'static) {
    let Some(node) = core_host::node() else { return done(Some(text)) };
    core_host::spawn(async move {
        match node.send_chat(peer, text.clone()).await {
            Ok(_) => done(None),
            Err(e) => {
                show_message(explain(&e));
                done(Some(text));
            }
        }
    });
}

/// Changes the text of a note this PC sent `peer`.
pub fn edit(peer: DeviceId, id: String, text: String) {
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        match node.edit_chat(peer, id, text).await {
            Ok(()) => {}
            Err(Error::NotFound) => show_message(tr!("Chat", "That note was deleted.")),
            Err(e) => show_message(explain(&e)),
        }
    });
}

/// Deletes a note in the chat with `peer`, here and there.
pub fn delete(peer: DeviceId, id: String) {
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        match node.delete_chat(peer, id).await {
            // Gone already.
            Ok(()) | Err(Error::NotFound) => {}
            Err(e) => show_message(explain(&e)),
        }
    });
}

/// A failed chat command in a sentence.
fn explain(error: &Error) -> String {
    match error {
        Error::Unsupported => tr!("Chat", "Update NectarLink on the device to chat."),
        Error::Protocol(_) => tr!("Chat", "A note can have up to 8192 characters."),
        other => describe(other),
    }
}

pub fn on_event(event: &NodeEvent) {
    if let NodeEvent::ChatReceived { device, text, .. } = event {
        received(*device, text);
    }
}

/// A note came in: a notification, unless its chat is being read.
fn received(device: DeviceId, text: &str) {
    if in_front() == Some(device) || crate::notifications::quiet_now() {
        return;
    }
    let name = core_host::host().hub.read(|s| s.name_of(&device)).unwrap_or_default();
    toast::show(Toast {
        device: TOAST_GROUP.into(),
        key: device.to_string(),
        title: if name.is_empty() { tr!("Chat", "New note") } else { name },
        body: text.to_owned(),
        attribution: "NectarLink".into(),
        icon: None,
        image: None,
        actions: Vec::new(),
        reply: None,
        silent: false,
        progress: None,
        call: false,
    });
}

/// What the user did with a chat notification: clicked, it opens the chat.
pub fn on_toast(event: &ToastEvent) {
    if let ToastEvent::Opened { key, .. } = event {
        match key.parse::<DeviceId>() {
            Ok(device) => crate::bridge::app::show_chat(device),
            Err(_) => crate::bridge::app::request_activation(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_id_comes_from_its_timeline_ref() {
        assert_eq!(note_id("chat:3f9a"), Some("3f9a"));
        assert_eq!(note_id("chat:"), None);
        assert_eq!(note_id("clip-1"), None);
    }
}
