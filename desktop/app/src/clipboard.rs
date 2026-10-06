// SPDX-License-Identifier: GPL-3.0-or-later
//! The clipboard between this PC and paired phones
//! (docs/protocol/clipboard.md): what the user copies (text or an image)
//! goes to connected phones (unless turned off in Settings), "Send
//! clipboard" sends it on demand, and what a phone sends lands on the
//! Windows clipboard.

use std::sync::atomic::{AtomicBool, Ordering};

use nectarlink_core::{CLIP_MAX_IMAGE_BYTES, DeviceId, Error, FeatureState, LinkState, Node, NodeEvent};

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

/// The user copied text or an image on this PC.
fn on_copy(clip: Clip) {
    if !AUTO_SEND.load(Ordering::Relaxed) {
        return;
    }
    let Some(node) = core_host::node() else { return };
    let devices = receivers();
    if devices.is_empty() {
        return;
    }
    core_host::spawn(async move {
        let Some(content) = Content::from_clip(clip).await else { return };
        for device in devices {
            // Automatic: nothing to tell the user if a phone can't take it.
            if let Err(e) = content.send(&node, device).await {
                tracing::debug!(device = %device.short(), error = %e, "clipboard not sent");
            }
        }
    });
}

/// What goes to a phone.
enum Content {
    Text(String),
    Png(Vec<u8>),
}

impl Content {
    /// Text as is, an image as PNG (converted off the async threads).
    async fn from_clip(clip: Clip) -> Option<Content> {
        match clip {
            Clip::Text(text) => Some(Content::Text(text)),
            Clip::Image(image) => match tokio::task::spawn_blocking(move || image.to_png()).await {
                Ok(Ok(png)) => Some(Content::Png(png)),
                Ok(Err(reason)) => {
                    tracing::warn!(reason, "can't read the copied image");
                    None
                }
                Err(_) => None,
            },
            Clip::Private | Clip::Empty => None,
        }
    }

    async fn send(&self, node: &Node, device: DeviceId) -> Result<(), Error> {
        match self {
            Content::Text(text) => node.send_clipboard(device, text.clone()).await,
            Content::Png(png) if png.len() as u64 > CLIP_MAX_IMAGE_BYTES => Err(Error::TooLarge),
            Content::Png(png) => node.send_clipboard_image(device, "image/png".into(), png.clone()).await,
        }
    }
}

/// "Send clipboard": sends what's copied to one device, and says how it went.
pub fn send_now(device: DeviceId) {
    let Some(node) = core_host::node() else { return };
    let name = core_host::host().hub.read(|s| s.name_of(&device)).unwrap_or_else(|| "the phone".into());
    let clip = clipboard::read();
    let image = matches!(clip, Clip::Image(_));
    match clip {
        Clip::Private => {
            return show_message("What you copied is marked private by its app, so it wasn't sent.");
        }
        Clip::Empty => return show_message("There's no text or image on the clipboard."),
        Clip::Text(_) | Clip::Image(_) => {}
    }
    core_host::spawn(async move {
        let Some(content) = Content::from_clip(clip).await else {
            return show_message("That image can't be read.");
        };
        match content.send(&node, device).await {
            Ok(()) => show_message(format!("Sent to {name}.")),
            Err(Error::TooLarge) if image => show_message("That image is too large to send."),
            Err(Error::TooLarge) => show_message("That's too much text to send at once."),
            Err(Error::Unsupported) if image => {
                show_message(format!("Update Nectarlink on {name} to send it images."))
            }
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
