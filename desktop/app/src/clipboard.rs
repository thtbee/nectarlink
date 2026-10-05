// SPDX-License-Identifier: GPL-3.0-or-later
//! The clipboard between this PC and paired phones
//! (docs/protocol/clipboard.md): what the user copies goes to connected
//! phones (unless turned off in Settings), "Send clipboard" sends it on
//! demand, and text from a phone lands on the Windows clipboard.

use std::sync::atomic::{AtomicBool, Ordering};

use nectarlink_core::{DeviceId, Error, FeatureState, LinkState, NodeEvent};

use crate::{
    bridge::app::{describe, show_message},
    core_host,
    win::clipboard::{self, Clip},
};

/// The "Send what you copy to your phone" preference.
static AUTO_SEND: AtomicBool = AtomicBool::new(true);

pub fn set_auto_send(on: bool) {
    AUTO_SEND.store(on, Ordering::Relaxed);
}

/// Starts watching the Windows clipboard.
pub fn start() {
    clipboard::start(on_copy);
}

/// Connected phones that take this PC's clipboard (capable and allowed).
fn receivers() -> Vec<DeviceId> {
    core_host::host().hub.read(|s| {
        s.devices
            .iter()
            .filter(|d| matches!(d.link, LinkState::Online { .. }))
            .filter(|d| {
                s.matrices.get(&d.id).and_then(|m| m.state("clipboard.pc_to_phone"))
                    == Some(FeatureState::Available)
            })
            .map(|d| d.id)
            .collect()
    })
}

/// The user copied text on this PC.
fn on_copy(text: String) {
    if !AUTO_SEND.load(Ordering::Relaxed) {
        return;
    }
    let Some(node) = core_host::node() else { return };
    for device in receivers() {
        let (node, text) = (node.clone(), text.clone());
        core_host::spawn(async move {
            // Automatic: nothing to tell the user if a phone can't take it.
            if let Err(e) = node.send_clipboard(device, text).await {
                tracing::debug!(device = %device.short(), error = %e, "clipboard not sent");
            }
        });
    }
}

/// "Send clipboard": sends what's copied to one device, and says how it went.
pub fn send_now(device: DeviceId) {
    let Some(node) = core_host::node() else { return };
    let name = core_host::host().hub.read(|s| s.name_of(&device)).unwrap_or_else(|| "the phone".into());
    let text = match clipboard::read() {
        Clip::Text(text) => text,
        Clip::Private => {
            show_message("What you copied is marked private by its app, so it wasn't sent.");
            return;
        }
        Clip::Empty => {
            show_message("There's no text on the clipboard.");
            return;
        }
    };
    core_host::spawn(async move {
        match node.send_clipboard(device, text).await {
            Ok(()) => show_message(format!("Sent to {name}.")),
            Err(Error::TooLarge) => show_message("That's too much text to send at once."),
            Err(Error::Denied) => show_message(format!("The clipboard is turned off for {name}.")),
            Err(e) => show_message(describe(&e)),
        }
    });
}

/// Feedback for clipboard events.
pub fn on_event(event: &NodeEvent) {
    if let NodeEvent::ClipboardReceived { device } = event {
        let name = core_host::host().hub.read(|s| s.name_of(device)).unwrap_or_else(|| "your phone".into());
        show_message(format!("Copied from {name}."));
    }
}
