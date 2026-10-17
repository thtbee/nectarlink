// SPDX-License-Identifier: GPL-3.0-or-later
//! The clipboard between this PC and paired phones
//! (docs/protocol/clipboard.md): what the user copies (text or an image)
//! goes to connected phones (unless turned off in Settings), "Send
//! clipboard" sends it on demand, and what a phone sends lands on the
//! Windows clipboard.

use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use data_encoding::BASE64;
use nectarlink_core::{
    CLIP_MAX_IMAGE_BYTES, ClipKind, ClipSuggestion, ClipboardItemKind, DeviceId, Error, FeatureState,
    LinkState, Node, NodeEvent,
};
use serde_json::json;

use crate::{
    bridge::app::{describe, open_dialer, show_message, show_message_with_action},
    core_host,
    state::Changes,
    win::clipboard::{self, Clip},
};

/// Toast group and action ID for smart clipboard context chips.
pub const TOAST_GROUP: &str = "clipboard";
pub const ACTION_CLIP_SUGGESTION: &str = "clip_action";

/// The "Send what you copy to your phone" preference.
static AUTO_SEND: AtomicBool = AtomicBool::new(true);

/// The "Keep clipboard history" preference.
static HISTORY_ENABLED: AtomicBool = AtomicBool::new(true);

/// The "Suggest actions for copied text" preference.
static SUGGEST_ACTIONS: AtomicBool = AtomicBool::new(true);

/// Most recent incoming clipboard context suggestion, kept only in memory so
/// Windows toast XML arguments never carry the copied text.
static LAST_SUGGESTION: Mutex<Option<(DeviceId, ClipSuggestion)>> = Mutex::new(None);

/// Small PNG previews of history images as data URLs, by entry ID. Made
/// off the UI thread and kept only in memory, so nothing decrypted is
/// written to disk.
static THUMBS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);
static MAKING_THUMBS: AtomicBool = AtomicBool::new(false);
/// The longest side of a preview, in pixels (the sheet shows them at most
/// 180 × 80, so this stays sharp at 150% scaling).
const THUMB_SIZE: u32 = 240;

fn thumbs<T>(f: impl FnOnce(&mut HashMap<String, String>) -> T) -> T {
    f(THUMBS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new))
}

/// Returns the cached scaled PNG data URL for a clipboard history image ID, or
/// schedules background generation if not yet cached.
pub fn thumb_for_clip(id: &str) -> Option<String> {
    if !HISTORY_ENABLED.load(Ordering::Relaxed) {
        return None;
    }
    let cached = thumbs(|t| t.get(id).cloned());
    if let Some(url) = cached {
        return (!url.is_empty()).then_some(url);
    }
    if let Some(node) = core_host::node() {
        make_thumbs(node, vec![id.to_owned()]);
    }
    None
}

pub fn set_auto_send(on: bool) {
    AUTO_SEND.store(on, Ordering::Relaxed);
}

pub fn set_suggest_actions(on: bool) {
    SUGGEST_ACTIONS.store(on, Ordering::Relaxed);
    if !on {
        *LAST_SUGGESTION.lock().unwrap_or_else(|e| e.into_inner()) = None;
        crate::win::toast::remove_device(TOAST_GROUP);
    }
}

pub fn set_history_enabled(on: bool) {
    HISTORY_ENABLED.store(on, Ordering::Relaxed);
    if let Some(node) = core_host::node() {
        let _ = node.set_clipboard_history_enabled(on);
    }
    core_host::host().hub.changed(Changes::CLIPBOARD);
}

pub fn apply_history_setting(node: &Node) {
    let _ = node.set_clipboard_history_enabled(HISTORY_ENABLED.load(Ordering::Relaxed));
    core_host::host().hub.changed(Changes::CLIPBOARD);
}

/// Encrypted local clipboard history serialized as a JSON array for QML.
/// Cheap enough for the UI thread: image previews come from memory, and
/// missing ones are made in the background (which refreshes the list).
pub fn history_json() -> String {
    if !HISTORY_ENABLED.load(Ordering::Relaxed) {
        thumbs(HashMap::clear);
        return "[]".into();
    }
    let Some(node) = core_host::node() else {
        return "[]".into();
    };
    let entries = node.clipboard_history(None);
    let (cached, missing) = thumbs(|t| {
        t.retain(|id, _| entries.iter().any(|e| &e.id == id));
        let missing: Vec<String> = entries
            .iter()
            .filter(|e| e.kind == ClipboardItemKind::Image && !t.contains_key(&e.id))
            .map(|e| e.id.clone())
            .collect();
        (t.clone(), missing)
    });
    if !missing.is_empty() {
        make_thumbs(node, missing);
    }
    let items: Vec<serde_json::Value> = entries
        .into_iter()
        .filter(|entry| {
            !(entry.kind == ClipboardItemKind::Text
                && nectarlink_core::is_likely_binary_or_corrupted_image_text(&entry.text))
        })
        .map(|entry| {
            let kind = match entry.kind {
                ClipboardItemKind::Text => "text",
                ClipboardItemKind::Image => "image",
            };
            json!({
                "id": entry.id,
                "kind": kind,
                "text": entry.text,
                "imageDataUrl": cached.get(&entry.id).cloned().unwrap_or_default(),
                "deviceName": entry.device_name,
                "incoming": entry.incoming,
                "timestamp": entry.timestamp,
                "pinned": entry.pinned,
            })
        })
        .collect();
    serde_json::Value::Array(items).to_string()
}

/// Decrypts and shrinks the given history images on a worker thread, then
/// refreshes the list. An image that can't be read gets an empty preview
/// (the sheet shows a plain image tile) so it isn't retried every refresh.
fn make_thumbs(node: Node, ids: Vec<String>) {
    if MAKING_THUMBS.swap(true, Ordering::AcqRel) {
        return; // the running worker refreshes the list, which retries the rest
    }
    std::thread::spawn(move || {
        for id in ids {
            let url = node
                .clipboard_history_image(&id)
                .and_then(|(_, bytes)| crate::win::image::decode(&bytes).ok())
                .and_then(|bitmap| {
                    crate::win::image::encode_png(&crate::win::image::scale_to(&bitmap, THUMB_SIZE)).ok()
                })
                .map(|png| format!("data:image/png;base64,{}", BASE64.encode(&png)))
                .unwrap_or_default();
            thumbs(|t| t.insert(id, url));
        }
        MAKING_THUMBS.store(false, Ordering::Release);
        core_host::host().hub.changed(Changes::CLIPBOARD | Changes::TIMELINE);
    });
}

pub fn copy_history_item(id: &str) {
    let Some(node) = core_host::node() else { return };
    match node.copy_clipboard_history(id) {
        Ok(()) => show_message("Copied to clipboard"),
        Err(e) => show_message(describe(&e)),
    }
}

pub fn pin_history_item(id: &str, pinned: bool) {
    if let Some(node) = core_host::node() {
        let _ = node.pin_clipboard_history(id, pinned);
    }
}

pub fn delete_history_item(id: &str) {
    if let Some(node) = core_host::node() {
        let _ = node.delete_clipboard_history(id);
    }
}

pub fn clear_history() {
    if let Some(node) = core_host::node() {
        let _ = node.clear_clipboard_history();
    }
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
    Text { text: String, html: Option<String>, rtf: Option<String> },
    Png(Vec<u8>),
}

impl Content {
    /// Text as is, an image as PNG (converted off the async threads).
    async fn from_clip(clip: Clip) -> Option<Content> {
        match clip {
            Clip::Text { text, html, rtf } => Some(Content::Text { text, html, rtf }),
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
            Content::Text { text, html, rtf } => {
                node.send_clipboard_rich(device, text.clone(), html.clone(), rtf.clone()).await
            }
            Content::Png(png) if png.len() as u64 > CLIP_MAX_IMAGE_BYTES => Err(Error::TooLarge),
            Content::Png(png) => node.send_clipboard_image(device, "image/png".into(), png.clone()).await,
        }
    }
}

/// "Send clipboard": sends what's copied to one device, and says how it went.
pub fn send_now(device: DeviceId) {
    let Some(node) = core_host::node() else { return };
    let (name, is_online) = core_host::host().hub.read(|s| {
        let name = s.name_of(&device).unwrap_or_else(|| "the phone".into());
        let online = s.device(&device).is_some_and(|d| matches!(d.link, LinkState::Online { .. }));
        (name, online)
    });
    let clip = clipboard::read();
    let image = matches!(clip, Clip::Image(_));
    match clip {
        Clip::Private => {
            return show_message("What you copied is marked private by its app, so it wasn't sent.");
        }
        Clip::Empty => {
            if let Some(files) = clipboard::read_files() {
                crate::transfers::send(device, files);
                if is_online {
                    show_message(format!("Sending copied files to {name}…"));
                }
                return;
            }
            return show_message("There's no text, image, or file on the clipboard.");
        }
        Clip::Text { .. } | Clip::Image(_) => {}
    }
    core_host::spawn(async move {
        let Some(content) = Content::from_clip(clip).await else {
            return show_message("That image can't be read.");
        };
        match content.send(&node, device).await {
            Ok(()) => {
                if !crate::bridge::app::show_offline_queued_toast(device, None) {
                    show_message(format!("Sent to {name}."));
                }
            }
            Err(Error::TooLarge) if image => show_message("That image is too large to send."),
            Err(Error::TooLarge) => show_message("That's too much text to send at once."),
            Err(Error::Unsupported) if image => {
                show_message(format!("Update NectarLink on {name} to send it images."))
            }
            Err(Error::Denied) => show_message(format!("The clipboard is turned off for {name}.")),
            Err(e) => show_message(describe(&e)),
        }
    });
}

/// Sends a saved clipboard history item to `device`, queueing if offline.
pub fn send_history_item(device: DeviceId, id: &str) {
    let Some(node) = core_host::node() else { return };
    let id = id.to_owned();
    let name = core_host::host().hub.read(|s| s.name_of(&device)).unwrap_or_else(|| "the phone".into());
    core_host::spawn(async move {
        match node.resend_clipboard_history(device, &id).await {
            Ok(()) => {
                if !crate::bridge::app::show_offline_queued_toast(device, None) {
                    show_message(format!("Sent to {name}."));
                }
            }
            Err(Error::NotFound) => show_message("That clip is no longer in clipboard history."),
            Err(e) => show_message(describe(&e)),
        }
    });
}

/// Places received file paths onto the Windows clipboard (`CF_HDROP`) when
/// `clipboard_auto_copy_received_files` is enabled, so the user can immediately
/// press `Ctrl+V` in File Explorer or any desktop app to paste them.
pub fn copy_received_files_to_clipboard(paths: &[std::path::PathBuf]) -> bool {
    if paths.is_empty() || !crate::settings::clipboard_auto_copy_received_files() {
        return false;
    }
    clipboard::write_files(paths)
}

/// Feedback for clipboard events.
pub fn on_event(event: &NodeEvent) {
    if let NodeEvent::ClipboardReceived { device } = event {
        let name = core_host::host().hub.read(|s| s.name_of(device)).unwrap_or_else(|| "your phone".into());
        let suggestion = if SUGGEST_ACTIONS.load(Ordering::Relaxed) {
            core_host::node()
                .and_then(|node| node.last_clip_suggestion())
                .filter(|(sug_dev, _)| sug_dev == device)
                .map(|(_, sug)| sug)
        } else {
            None
        };
        if let Some(suggestion) = suggestion {
            let label = suggestion.action_label();
            *LAST_SUGGESTION.lock().unwrap_or_else(|e| e.into_inner()) = Some((*device, suggestion));
            show_message_with_action(format!("Copied from {name}."), label);
            crate::win::toast::show(crate::win::toast::Toast {
                device: TOAST_GROUP.into(),
                key: "clip_suggestion".into(),
                title: format!("Copied from {name}"),
                body: String::new(),
                attribution: String::new(),
                icon: None,
                image: None,
                actions: vec![(ACTION_CLIP_SUGGESTION.into(), label.into())],
                reply: None,
                silent: true,
                progress: None,
                call: false,
            });
        } else {
            *LAST_SUGGESTION.lock().unwrap_or_else(|e| e.into_inner()) = None;
            show_message(format!("Copied from {name}."));
        }
    }
}

/// Runs the action for the most recently offered clipboard suggestion (from
/// the in-app toast chip or the Windows toast action button).
pub fn run_last_suggestion() {
    let Some((device, suggestion)) = LAST_SUGGESTION.lock().unwrap_or_else(|e| e.into_inner()).clone() else {
        return;
    };
    crate::win::toast::remove(TOAST_GROUP, "clip_suggestion");
    let result = match suggestion.kind {
        ClipKind::WebLink => crate::win::shell::open_url(&suggestion.target),
        ClipKind::StreetAddress => crate::win::shell::open_maps(&suggestion.target),
        ClipKind::PhoneNumber => {
            open_dialer(device, &suggestion.target);
            Ok(())
        }
        ClipKind::TrackingNumber => {
            crate::win::shell::open_url(&nectarlink_core::tracking_search_url(&suggestion.target))
        }
        ClipKind::Email => crate::win::shell::open_mailto(&suggestion.target),
    };
    if let Err(e) = result {
        tracing::debug!(kind = ?suggestion.kind, error = %e, "clipboard suggestion action failed");
    }
}
