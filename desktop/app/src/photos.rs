// SPDX-License-Identifier: GPL-3.0-or-later
//! New photos from a phone (docs/protocol/photos.md): a Windows
//! notification with the preview, to open the photo, save it, or copy it.
//! The photo itself comes as a files transfer when the user asks.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

use nectarlink_core::{DeviceId, Direction, Error, NodeEvent, Photo, Transfer, TransferState};

use crate::{
    bridge::app::{describe, show_message},
    core_host,
    win::toast::{self, Toast},
};

/// The toast "device" for photos; their key is `<device ID> <photo ID>`.
pub const TOAST_GROUP: &str = "photos";
pub const ACTION_SAVE: &str = "save";
pub const ACTION_COPY: &str = "copy";

/// What the user wants done with a photo once it's here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Intent {
    /// Kept in Downloads (the usual "saved" notification says so).
    Save,
    /// Opened in its app.
    Open,
    /// Put on the clipboard; not kept.
    Copy,
}

#[derive(Debug, Default)]
struct State {
    /// Transfers bringing photos the user asked for.
    pending: HashMap<String, Intent>,
    /// Incoming transfers that finished lately, in case one finishes
    /// before the phone's answer says which it was.
    finished: HashMap<String, (Instant, Vec<PathBuf>)>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
const KEEP_FINISHED: Duration = Duration::from_secs(60);

fn state<T>(f: impl FnOnce(&mut State) -> T) -> T {
    f(STATE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_default())
}

pub fn on_event(event: &NodeEvent) {
    match event {
        NodeEvent::PhotoAdded { device, photo } => show(*device, photo),
        NodeEvent::Transfer(t) if t.direction == Direction::Incoming => finished(t),
        _ => {}
    }
}

/// Whether this transfer brings a photo the user asked to open or copy,
/// so the usual "saved" notification isn't wanted.
pub fn handles(transfer_id: &str) -> bool {
    state(|s| s.pending.get(transfer_id).is_some_and(|intent| *intent != Intent::Save))
}

fn show(device: DeviceId, photo: &Photo) {
    tracing::debug!(
        screenshot = photo.screenshot,
        preview = !photo.thumb.is_empty(),
        "a phone has a new photo"
    );
    let preview = save_preview(device, photo);
    let name = core_host::host().hub.read(|s| s.name_of(&device)).unwrap_or_else(|| "your phone".into());
    toast::show(Toast {
        device: TOAST_GROUP.into(),
        key: format!("{device} {}", photo.id),
        title: if photo.screenshot { "New screenshot".into() } else { "New photo".into() },
        body: format!("From {name}"),
        attribution: "Nectarlink".into(),
        icon: None,
        image: preview,
        actions: vec![(ACTION_SAVE.into(), "Save".into()), (ACTION_COPY.into(), "Copy".into())],
        reply: None,
        silent: true,
        progress: None,
        call: false,
    });
}

/// The preview as a file, for the notification.
fn save_preview(device: DeviceId, photo: &Photo) -> Option<PathBuf> {
    if photo.thumb.is_empty() {
        return None;
    }
    let dir = crate::notifications::images_dir();
    let path = dir.join(format!("photo-{:016x}.jpg", fingerprint(&format!("{device} {}", photo.id))));
    let written = std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, &photo.thumb));
    match written {
        Ok(()) => Some(path),
        Err(e) => {
            tracing::warn!(error = %e, "can't keep a photo preview");
            None
        }
    }
}

pub(crate) fn fingerprint(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, b| (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// The user clicked the notification (`action` None) or one of its buttons.
pub fn on_toast(key: &str, action: Option<&str>) {
    let Some((device, photo_id)) = key.split_once(' ') else { return };
    let Ok(device) = device.parse::<DeviceId>() else { return };
    let intent = match action {
        None => Intent::Open,
        Some(ACTION_SAVE) => Intent::Save,
        Some(ACTION_COPY) => Intent::Copy,
        Some(_) => return,
    };
    let Some(node) = core_host::node() else { return };
    let photo_id = photo_id.to_owned();
    core_host::spawn(async move {
        match node.fetch_photo(device, photo_id).await {
            Ok(transfer) => asked(transfer, intent),
            Err(Error::NotFound) => show_message("That photo isn't on the phone anymore."),
            Err(Error::Denied) => show_message("The phone doesn't share photos or files with this PC."),
            Err(e) => show_message(describe(&e)),
        }
    });
}

fn asked(transfer: String, intent: Intent) {
    let done = state(|s| {
        let now = Instant::now();
        s.finished.retain(|_, (at, _)| now.duration_since(*at) < KEEP_FINISHED);
        match s.finished.remove(&transfer) {
            Some((_, saved)) => Some(saved),
            None => {
                s.pending.insert(transfer, intent);
                None
            }
        }
    });
    // Already here: the usual notification came first.
    if let Some(saved) = done {
        if let (true, Some(first)) = (intent != Intent::Save, saved.first()) {
            toast::remove(crate::transfers::TOAST_GROUP, &first.to_string_lossy());
        }
        deliver(intent, &saved);
    }
}

fn finished(t: &Transfer) {
    let saved = match &t.state {
        TransferState::Done { saved } => saved.clone(),
        TransferState::Failed(_) | TransferState::Cancelled => {
            state(|s| s.pending.remove(&t.id));
            return;
        }
        _ => return,
    };
    let intent = state(|s| match s.pending.remove(&t.id) {
        Some(intent) => Some(intent),
        None => {
            s.finished.insert(t.id.clone(), (Instant::now(), saved.clone()));
            None
        }
    });
    if let Some(intent) = intent {
        deliver(intent, &saved);
    }
}

fn deliver(intent: Intent, saved: &[PathBuf]) {
    let Some(path) = saved.first() else { return };
    match intent {
        Intent::Save => {}
        Intent::Open => crate::transfers::open(path),
        Intent::Copy => copy(path),
    }
}

fn copy(path: &Path) {
    let mime = match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("png") => "image/png",
        _ => "image/jpeg",
    };
    let copied = std::fs::read(path)
        .map_err(|e| e.to_string())
        .and_then(|bytes| crate::win::clipboard::write_image(mime, &bytes));
    match copied {
        // Only wanted on the clipboard.
        Ok(()) => {
            let _ = std::fs::remove_file(path);
        }
        Err(reason) => {
            tracing::warn!(reason, "can't copy a photo");
            show_message("The photo couldn't be copied. It's in Downloads\\Nectarlink.");
        }
    }
}
