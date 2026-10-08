// SPDX-License-Identifier: GPL-3.0-or-later
//! Photos and videos from a phone (docs/protocol/photos.md):
//! - Windows notification with preview when a new photo or screenshot is taken,
//!   to open, save, or copy it.
//! - The Photos gallery page: albums, paged items newest-first, on-demand
//!   batched thumbnails cached on disk, in-app viewer, Save (to a chosen
//!   folder or `Downloads\Nectarlink`), Copy to clipboard, and Open in the
//!   Windows Photos app or default video player.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

use nectarlink_core::{
    DeviceId, Direction, Error, FeatureState, LinkState, NodeEvent, Photo, PhotoAlbum, PhotoItem, PhotoThumb,
    Transfer, TransferState,
};
use serde_json::{Value, json};

use crate::{
    bridge::app::{describe, show_message},
    core_host,
    messages::Status,
    state::Changes,
    win::toast::{self, Toast},
};

/// The toast "device" for photos; their key is `<device ID> <photo ID>`.
pub const TOAST_GROUP: &str = "photos";
pub const ACTION_SAVE: &str = "save";
pub const ACTION_COPY: &str = "copy";

/// Items requested per gallery page, and thumbnails requested per batch.
const PAGE: u32 = 80;
const THUMB_BATCH: usize = 24;

/// What the user wants done with a transfer bringing one or more photos/videos.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Intent {
    /// Kept in Downloads from a toast (the usual "saved" notification says so).
    ToastSave,
    /// Opened in its default app (Windows Photos or video player).
    Open { item_id: Option<String> },
    /// Put on the clipboard; not kept in Downloads.
    Copy { item_id: Option<String>, notify_ui: bool },
    /// Downloaded for the in-app viewer (moved into the gallery cache).
    View { device: DeviceId, item_id: String },
    /// Saved from the gallery to `Downloads\Nectarlink` or a user-chosen folder.
    GallerySave { ids: Vec<String>, folder: Option<PathBuf> },
}

#[derive(Debug, Default)]
struct State {
    /// Transfers bringing photos the user asked for.
    pending: HashMap<String, Intent>,
    /// Transfers whose completion is handled here (so `transfers.rs` suppresses
    /// its generic "saved in Downloads" toast).
    handled: HashMap<String, Instant>,
    /// Transfers that bring a file the user doesn't keep (the viewer's, or
    /// one only for the clipboard): not listed with the others.
    private: HashMap<String, Instant>,
    /// Incoming transfers that finished lately, in case one finishes
    /// before the phone's answer says which it was.
    finished: HashMap<String, (Instant, Vec<PathBuf>)>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
const KEEP_FINISHED: Duration = Duration::from_secs(60);
/// How long a private transfer stays out of the list (longer than any
/// photo or video takes to arrive).
const KEEP_PRIVATE: Duration = Duration::from_secs(6 * 60 * 60);

fn state<T>(f: impl FnOnce(&mut State) -> T) -> T {
    f(STATE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_default())
}

pub fn on_event(event: &NodeEvent) {
    let page_device = gallery_state(|s| s.device);
    match event {
        NodeEvent::PhotoAdded { device, photo } => {
            show(*device, photo);
            if !photo.thumb.is_empty() {
                let _ = write_thumb(*device, &photo.id, &photo.thumb);
            }
            if Some(*device) == page_device {
                reload_quiet();
            }
        }
        NodeEvent::PhotosChanged { device } if Some(*device) == page_device => {
            reload_quiet();
        }
        NodeEvent::LinkChanged { device, link: LinkState::Online { .. } } if Some(*device) == page_device => {
            if gallery_state(|s| s.status != Status::Ready) {
                reload();
            }
        }
        NodeEvent::LinkChanged { device, link: LinkState::Offline { .. } }
            if Some(*device) == page_device =>
        {
            gallery_state(|s| {
                s.status = Status::Offline;
                s.loading_older = false;
            });
            gallery_changed();
        }
        NodeEvent::Capabilities(matrix) if Some(matrix.device) == page_device => {
            match matrix.state("files.recent_photos") {
                Some(FeatureState::Available)
                    if gallery_state(|s| !matches!(s.status, Status::Ready | Status::Loading)) =>
                {
                    reload();
                }
                Some(FeatureState::Locked { .. } | FeatureState::Unsupported { .. })
                    if gallery_state(|s| s.status == Status::Ready) =>
                {
                    reload();
                }
                _ => {}
            }
        }
        NodeEvent::Transfer(t) if t.direction == Direction::Incoming => finished(t),
        _ => {}
    }
}

/// Whether this transfer brings a photo the user asked to open, copy, view, or
/// save from the gallery, so the usual "saved" toast notification isn't wanted.
/// Whether this transfer brings a file the user doesn't keep, so it isn't
/// listed with the other transfers.
pub fn is_private(transfer_id: &str) -> bool {
    state(|s| s.private.contains_key(transfer_id))
}

pub fn handles(transfer_id: &str) -> bool {
    state(|s| {
        s.handled.contains_key(transfer_id)
            || s.pending.get(transfer_id).is_some_and(|intent| *intent != Intent::ToastSave)
    })
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
    fingerprint_bytes(text.as_bytes())
}

/// FNV-1a: a stable name for cached files.
pub(crate) fn fingerprint_bytes(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, b| (hash ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
}

/// The user clicked the notification (`action` None) or one of its buttons.
pub fn on_toast(key: &str, action: Option<&str>) {
    let Some((device, photo_id)) = key.split_once(' ') else { return };
    let Ok(device) = device.parse::<DeviceId>() else { return };
    let intent = match action {
        None => Intent::Open { item_id: Some(photo_id.to_owned()) },
        Some(ACTION_SAVE) => Intent::ToastSave,
        Some(ACTION_COPY) => Intent::Copy { item_id: Some(photo_id.to_owned()), notify_ui: false },
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
    let private = matches!(intent, Intent::View { .. } | Intent::Copy { .. });
    let id = transfer.clone();
    let done = state(|s| {
        let now = Instant::now();
        s.finished.retain(|_, (at, _)| now.duration_since(*at) < KEEP_FINISHED);
        s.handled.retain(|_, at| now.duration_since(*at) < KEEP_FINISHED);
        s.private.retain(|_, at| now.duration_since(*at) < KEEP_PRIVATE);
        if intent != Intent::ToastSave {
            s.handled.insert(transfer.clone(), now);
        }
        if private {
            s.private.insert(transfer.clone(), now);
        }
        match s.finished.remove(&transfer) {
            Some((_, saved)) => Some(saved),
            None => {
                s.pending.insert(transfer, intent.clone());
                None
            }
        }
    });
    if private {
        // It may have started before the phone said which transfer it was.
        core_host::host().hub.update(|s| s.forget_transfer(&id));
    }
    // Already here: the usual notification came first.
    if let Some(saved) = done {
        if let (true, Some(first)) = (intent != Intent::ToastSave, saved.first()) {
            toast::remove(crate::transfers::TOAST_GROUP, &first.to_string_lossy());
        }
        deliver(intent, &saved);
    }
}

fn finished(t: &Transfer) {
    let saved = match &t.state {
        TransferState::Done { saved } => saved.clone(),
        TransferState::Failed(_) | TransferState::Cancelled => {
            let removed = state(|s| s.pending.remove(&t.id));
            if removed.is_some() {
                gallery_state(|g| {
                    g.saving = false;
                    g.busy_item = None;
                });
                gallery_changed();
            }
            return;
        }
        _ => return,
    };
    let intent = state(|s| {
        let now = Instant::now();
        s.handled.retain(|_, at| now.duration_since(*at) < KEEP_FINISHED);
        match s.pending.remove(&t.id) {
            Some(intent) => {
                if intent != Intent::ToastSave {
                    s.handled.insert(t.id.clone(), now);
                }
                Some(intent)
            }
            None => {
                s.finished.insert(t.id.clone(), (now, saved.clone()));
                None
            }
        }
    });
    if let Some(intent) = intent {
        deliver(intent, &saved);
    }
}

fn deliver(intent: Intent, saved: &[PathBuf]) {
    let Some(first) = saved.first() else {
        gallery_state(|g| {
            g.saving = false;
            g.busy_item = None;
        });
        gallery_changed();
        return;
    };
    match intent {
        Intent::ToastSave => {}
        Intent::Open { item_id } => {
            if let Some(id) = item_id {
                gallery_state(|g| {
                    g.full_files.insert(id, first.clone());
                    if g.busy_item.is_some() {
                        g.busy_item = None;
                    }
                    g.saving = false;
                });
                gallery_changed();
            }
            crate::transfers::open(first);
        }
        Intent::Copy { item_id: _, notify_ui } => {
            gallery_state(|g| {
                g.saving = false;
                g.busy_item = None;
            });
            gallery_changed();
            copy_file_to_clipboard(first, true, notify_ui);
        }
        Intent::View { device, item_id } => {
            let ext = first.extension().and_then(|e| e.to_str()).filter(|e| !e.is_empty()).unwrap_or("jpg");
            let dir = thumbs_dir(device);
            let cached = dir.join(format!("full-{:016x}.{ext}", fingerprint(&item_id)));
            let final_path =
                if std::fs::create_dir_all(&dir).and_then(|()| move_or_copy(first, &cached)).is_ok() {
                    cached
                } else {
                    first.clone()
                };
            gallery_state(|g| {
                g.full_files.insert(item_id.clone(), final_path);
                if g.busy_item.as_deref() == Some(item_id.as_str()) {
                    g.busy_item = None;
                }
            });
            gallery_changed();
        }
        Intent::GallerySave { ids, folder } => {
            let mut final_paths = Vec::with_capacity(saved.len());
            let dest_label = match &folder {
                Some(dir) => {
                    let _ = std::fs::create_dir_all(dir);
                    for src in saved {
                        let name = src.file_name().and_then(|n| n.to_str()).unwrap_or("photo.jpg");
                        let dst = free_path(dir, name);
                        if dst == *src || move_or_copy(src, &dst).is_ok() {
                            final_paths.push(dst);
                        } else {
                            final_paths.push(src.clone());
                        }
                    }
                    folder_display_name(dir)
                }
                None => {
                    final_paths.extend(saved.iter().cloned());
                    r"Downloads\Nectarlink".to_owned()
                }
            };
            gallery_state(|g| {
                if ids.len() == final_paths.len() {
                    for (id, path) in ids.into_iter().zip(final_paths.iter().cloned()) {
                        g.full_files.insert(id, path);
                    }
                }
                g.saving = false;
                g.busy_item = None;
            });
            gallery_changed();
            match final_paths.as_slice() {
                [one] => {
                    let name = one.file_name().and_then(|n| n.to_str()).unwrap_or("Photo");
                    show_message(format!("Saved {name} to {dest_label}."));
                }
                many if !many.is_empty() => {
                    show_message(format!("Saved {} items to {dest_label}.", many.len()));
                }
                _ => {}
            }
        }
    }
}

fn move_or_copy(src: &Path, dst: &Path) -> std::io::Result<()> {
    if src == dst {
        return Ok(());
    }
    if std::fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    std::fs::copy(src, dst)?;
    let _ = std::fs::remove_file(src);
    Ok(())
}

fn free_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let path = Path::new(name);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(name);
    let ext = path.extension().and_then(|e| e.to_str());
    for n in 2..10_000u32 {
        let candidate = match ext {
            Some(ext) => dir.join(format!("{stem} ({n}).{ext}")),
            None => dir.join(format!("{stem} ({n})")),
        };
        if !candidate.exists() {
            return candidate;
        }
    }
    first
}

fn copy_file_to_clipboard(path: &Path, remove_after: bool, notify_ui: bool) {
    let mime = match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("png") => "image/png",
        _ => "image/jpeg",
    };
    let copied = std::fs::read(path)
        .map_err(|e| e.to_string())
        .and_then(|bytes| crate::win::clipboard::write_image(mime, &bytes));
    match copied {
        Ok(()) => {
            if remove_after {
                let _ = std::fs::remove_file(path);
            }
            if notify_ui {
                show_message("Copied to clipboard.");
            }
        }
        Err(reason) => {
            tracing::warn!(reason, "can't copy a photo");
            if remove_after {
                show_message("The photo couldn't be copied. It's in Downloads\\Nectarlink.");
            } else {
                show_message("The photo couldn't be copied to the clipboard.");
            }
        }
    }
}

// ---- Photos gallery page ----

#[derive(Debug, Clone, PartialEq)]
pub struct ItemRow {
    pub id: String,
    pub name: String,
    pub date: i64,
    pub prev_date: i64,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    pub duration: u32,
    pub is_video: bool,
    pub album: String,
    pub thumb: String,
    pub full_url: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub device: Option<DeviceId>,
    pub status: Status,
    pub albums: Value,
    pub album: String,
    pub rows: Vec<ItemRow>,
    pub more: bool,
    pub loading_older: bool,
    pub saving: bool,
    pub busy_item: String,
    pub save_folder: String,
}

#[derive(Debug, Default)]
struct GalleryState {
    device: Option<DeviceId>,
    status: Status,
    albums: Vec<PhotoAlbum>,
    album: String,
    items: Vec<PhotoItem>,
    more: bool,
    loading_older: bool,
    thumbs: HashMap<String, PathBuf>,
    failed_thumbs: HashSet<String>,
    full_files: HashMap<String, PathBuf>,
    wanted_thumbs: Vec<String>,
    fetching_thumbs: bool,
    saving: bool,
    busy_item: Option<String>,
    save_folder: Option<PathBuf>,
    generation: u64,
}

static GALLERY: Mutex<Option<GalleryState>> = Mutex::new(None);

fn gallery_state<T>(f: impl FnOnce(&mut GalleryState) -> T) -> T {
    f(GALLERY.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_default())
}

fn gallery_changed() {
    core_host::host().hub.changed(Changes::PHOTOS);
}

/// Drops in-memory gallery items, album lists, and thumbnail path maps while
/// the window is closed to the tray; reopening the Photos page reloads them.
pub fn release_idle_resources() {
    let mut released = false;
    gallery_state(|g| {
        if !g.saving && (g.device.is_some() || !g.items.is_empty() || !g.albums.is_empty()) {
            let save_folder = g.save_folder.take();
            let generation = g.generation.wrapping_add(1);
            *g = GalleryState { save_folder, generation, ..GalleryState::default() };
            released = true;
        }
    });
    if released {
        gallery_changed();
    }
}

fn thumbs_dir(device: DeviceId) -> PathBuf {
    core_host::host().data_dir.join("cache").join("photos-thumbs").join(device.to_string())
}

/// How much the gallery keeps on disk per phone: thumbnails, and full
/// photos opened in the viewer. The least recently written go first.
const THUMBS_BUDGET: u64 = 64 << 20;
const FULL_BUDGET: u64 = 512 << 20;

/// Trims a phone's gallery cache to its budgets.
fn prune_cache(device: DeviceId) {
    let Ok(entries) = std::fs::read_dir(thumbs_dir(device)) else { return };
    let (mut thumbs, mut full): (Vec<_>, Vec<_>) = entries
        .filter_map(|e| {
            let e = e.ok()?;
            let meta = e.metadata().ok().filter(|m| m.is_file())?;
            Some((meta.modified().ok()?, meta.len(), e.path()))
        })
        .partition(|(_, _, path)| {
            path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("thumb-"))
        });
    for (files, budget) in [(&mut thumbs, THUMBS_BUDGET), (&mut full, FULL_BUDGET)] {
        // Newest first; delete beyond the budget.
        files.sort_by_key(|f| std::cmp::Reverse(f.0));
        let mut kept = 0u64;
        for (_, len, path) in files.iter() {
            kept += len;
            if kept > budget {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

pub(crate) fn thumb_path(device: DeviceId, id: &str) -> PathBuf {
    thumbs_dir(device).join(format!("thumb-{:016x}.jpg", fingerprint(id)))
}

pub(crate) fn write_thumb(device: DeviceId, id: &str, jpeg: &[u8]) -> Option<PathBuf> {
    if jpeg.is_empty() {
        return None;
    }
    let dir = thumbs_dir(device);
    let path = thumb_path(device, id);
    std::fs::create_dir_all(&dir).ok()?;
    std::fs::write(&path, jpeg).ok()?;
    gallery_state(|s| {
        if s.device == Some(device) {
            s.thumbs.insert(id.to_owned(), path.clone());
        }
    });
    Some(path)
}

fn populate_disk_thumbs(device: DeviceId, ids: impl IntoIterator<Item = String>) {
    let mut found = Vec::new();
    for id in ids {
        let already = gallery_state(|s| s.thumbs.contains_key(&id));
        if already {
            continue;
        }
        let path = thumb_path(device, &id);
        if path.exists() {
            found.push((id, path));
        }
    }
    if !found.is_empty() {
        gallery_state(|s| {
            if s.device == Some(device) {
                for (id, path) in found {
                    s.thumbs.insert(id, path);
                }
            }
        });
    }
}

fn folder_display_name(dir: &Path) -> String {
    if let Some(home) = dirs::home_dir()
        && let Ok(rel) = dir.strip_prefix(&home)
        && !rel.as_os_str().is_empty()
    {
        return rel.to_string_lossy().into_owned();
    }
    dir.to_string_lossy().into_owned()
}

pub fn view() -> View {
    gallery_state(|s| {
        let mut prev_date = 0i64;
        let rows: Vec<ItemRow> = s
            .items
            .iter()
            .map(|item| {
                let thumb = s.thumbs.get(&item.id).map(|p| crate::icons::file_url(p)).unwrap_or_default();
                let full_url = s
                    .full_files
                    .get(&item.id)
                    .filter(|p| p.exists())
                    .map(|p| crate::icons::file_url(p))
                    .unwrap_or_default();
                let row = ItemRow {
                    id: item.id.clone(),
                    name: item.name.clone(),
                    date: item.date,
                    prev_date,
                    size: item.size,
                    width: item.width,
                    height: item.height,
                    duration: item.duration.unwrap_or(0),
                    is_video: item.duration.is_some() || item.id.starts_with("video:"),
                    album: item.album.clone().unwrap_or_default(),
                    thumb,
                    full_url,
                };
                prev_date = item.date;
                row
            })
            .collect();
        let albums = Value::Array(
            s.albums
                .iter()
                .map(|a| {
                    let cover = a.cover.clone().unwrap_or_default();
                    let thumb = (!cover.is_empty())
                        .then(|| s.thumbs.get(&cover))
                        .flatten()
                        .map(|p| crate::icons::file_url(p))
                        .unwrap_or_default();
                    json!({
                        "id": a.id,
                        "name": a.name,
                        "count": a.count,
                        "cover": cover,
                        "thumb": thumb,
                    })
                })
                .collect(),
        );
        let save_folder = s
            .save_folder
            .as_deref()
            .map(folder_display_name)
            .unwrap_or_else(|| r"Downloads\Nectarlink".to_owned());
        View {
            device: s.device,
            status: s.status,
            albums,
            album: s.album.clone(),
            rows,
            more: s.more,
            loading_older: s.loading_older,
            saving: s.saving,
            busy_item: s.busy_item.clone().unwrap_or_default(),
            save_folder,
        }
    })
}

/// Opens the gallery for `device` (reloads albums and items).
pub fn open_device(device: DeviceId) {
    let same = gallery_state(|s| {
        let same = s.device == Some(device);
        if !same {
            let save_folder = s.save_folder.take();
            *s = GalleryState {
                device: Some(device),
                save_folder,
                generation: s.generation + 1,
                ..GalleryState::default()
            };
        }
        same
    });
    if !same {
        gallery_changed();
        core_host::spawn(async move {
            let _ = tokio::task::spawn_blocking(move || prune_cache(device)).await;
        });
    }
    reload();
}

/// Switches to `album` (`""` for All photos) and loads its items.
pub fn select_album(album: String) {
    let changed_album = gallery_state(|s| {
        if s.album == album && !s.items.is_empty() {
            return false;
        }
        s.album = album;
        s.items.clear();
        s.more = false;
        s.loading_older = false;
        s.wanted_thumbs.clear();
        s.generation += 1;
        true
    });
    if changed_album {
        gallery_changed();
        load_items(None);
    }
}

pub fn reload() {
    load_albums();
    load_items(None);
}

fn reload_quiet() {
    load_albums();
    load_items(None);
}

pub fn load_older() {
    let before = gallery_state(|s| {
        if s.loading_older || !s.more {
            return None;
        }
        s.loading_older = true;
        s.items.last().map(|i| (i.date, i.id.clone()))
    });
    if before.is_some() {
        gallery_changed();
        load_items(before);
    }
}

fn load_albums() {
    let Some((device, generation)) = gallery_state(|s| s.device.map(|d| (d, s.generation))) else {
        return;
    };
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        let Ok(albums) = node.photo_albums(device).await else {
            return;
        };
        let covers: Vec<String> =
            albums.iter().filter_map(|a| a.cover.clone()).filter(|c| !c.is_empty()).collect();
        populate_disk_thumbs(device, covers.clone());
        gallery_state(|s| {
            if s.device != Some(device) || s.generation != generation {
                return;
            }
            s.albums = albums;
        });
        gallery_changed();
        // Queue missing cover thumbnails so album covers appear promptly.
        for cover in covers {
            need_thumb(cover);
        }
    });
}

fn load_items(before: Option<(i64, String)>) {
    let Some((device, album, generation)) = gallery_state(|s| {
        if before.is_none() && s.items.is_empty() {
            s.status = Status::Loading;
        }
        let album = (!s.album.is_empty()).then(|| s.album.clone());
        s.device.map(|d| (d, album, s.generation))
    }) else {
        return;
    };
    gallery_changed();
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        let first_page = before.is_none();
        let result = node.photo_list(device, album, before, PAGE).await;
        if let Ok(ref page) = result {
            populate_disk_thumbs(device, page.iter().map(|i| i.id.clone()));
        }
        gallery_state(|s| {
            s.loading_older = false;
            if s.device != Some(device) || s.generation != generation {
                return;
            }
            match result {
                Ok(page) => {
                    let full = page.len() as u32 >= PAGE;
                    if first_page {
                        // A refresh: keep the older pages already loaded
                        // beyond this one.
                        let oldest = page.last().map(|i| i.date).unwrap_or(i64::MAX);
                        let fresh: HashSet<&str> = page.iter().map(|i| i.id.as_str()).collect();
                        let older: Vec<PhotoItem> = s
                            .items
                            .drain(..)
                            .filter(|i| full && i.date <= oldest && !fresh.contains(i.id.as_str()))
                            .collect();
                        s.items = page;
                        s.items.extend(older);
                        s.more = full;
                    } else {
                        let existing: HashSet<String> = s.items.iter().map(|i| i.id.clone()).collect();
                        s.items.extend(page.into_iter().filter(|i| !existing.contains(&i.id)));
                        s.more = full;
                    }
                    s.status = Status::Ready;
                }
                Err(e) => {
                    tracing::debug!(error = %e, "can't list photos");
                    s.status = Status::of(&e);
                }
            }
        });
        gallery_changed();
    });
}

/// Requests a thumbnail for `id` (called by visible tiles in `GridView`).
pub fn need_thumb(id: String) {
    if id.is_empty() {
        return;
    }
    let Some(device) = gallery_state(|s| s.device) else { return };
    if gallery_state(|s| s.thumbs.contains_key(&id) || s.failed_thumbs.contains(&id)) {
        return;
    }
    let disk_path = thumb_path(device, &id);
    if disk_path.exists() {
        gallery_state(|s| {
            if s.device == Some(device) {
                s.thumbs.insert(id, disk_path);
            }
        });
        gallery_changed();
        return;
    }
    let start_pump = gallery_state(|s| {
        if s.device != Some(device) {
            return None;
        }
        if !s.wanted_thumbs.contains(&id) {
            s.wanted_thumbs.push(id);
        }
        if !s.fetching_thumbs {
            s.fetching_thumbs = true;
            Some((device, s.generation))
        } else {
            None
        }
    });
    if let Some((device, generation)) = start_pump {
        pump_thumbs(device, generation);
    }
}

/// Drops a pending thumbnail request when a tile scrolls off-screen before
/// being fetched.
pub fn drop_thumb(id: &str) {
    if id.is_empty() {
        return;
    }
    gallery_state(|s| {
        s.wanted_thumbs.retain(|wanted| wanted != id);
    });
}

fn pump_thumbs(device: DeviceId, generation: u64) {
    let Some(node) = core_host::node() else {
        gallery_state(|s| s.fetching_thumbs = false);
        return;
    };
    core_host::spawn(async move {
        loop {
            // Brief coalesce window so all tiles becoming visible in the same
            // frame are requested in one batch, and tiles scrolled past are
            // dropped before network I/O.
            tokio::time::sleep(Duration::from_millis(25)).await;
            let batch: Vec<String> = gallery_state(|s| {
                if s.device != Some(device) || s.generation != generation || s.wanted_thumbs.is_empty() {
                    s.fetching_thumbs = false;
                    return Vec::new();
                }
                let take = s.wanted_thumbs.len().min(THUMB_BATCH);
                s.wanted_thumbs.drain(..take).collect()
            });
            if batch.is_empty() {
                return;
            }
            match node.photo_thumbs(device, batch.clone()).await {
                Ok(thumbs) => {
                    save_thumb_batch(device, generation, &batch, thumbs);
                    gallery_changed();
                }
                Err(e) => {
                    tracing::debug!(error = %e, "can't fetch photo thumbnails");
                    gallery_state(|s| {
                        for id in &batch {
                            s.failed_thumbs.insert(id.clone());
                        }
                        s.fetching_thumbs = false;
                    });
                    return;
                }
            }
        }
    });
}

fn save_thumb_batch(device: DeviceId, generation: u64, requested: &[String], thumbs: Vec<PhotoThumb>) {
    let dir = thumbs_dir(device);
    let _ = std::fs::create_dir_all(&dir);
    let mut saved = HashMap::new();
    for t in thumbs {
        if t.data.is_empty() {
            continue;
        }
        let path = thumb_path(device, &t.id);
        if std::fs::write(&path, &t.data).is_ok() {
            saved.insert(t.id, path);
        }
    }
    gallery_state(|s| {
        if s.device != Some(device) || s.generation != generation {
            return;
        }
        for id in requested {
            if !saved.contains_key(id) {
                s.failed_thumbs.insert(id.clone());
            }
        }
        s.thumbs.extend(saved);
    });
}

/// Ensures the full-resolution image for `id` is downloaded for the in-app
/// viewer (for still photos; videos are opened in the default player on click).
pub fn ensure_full(id: String) {
    if id.is_empty() || id.starts_with("video:") {
        return;
    }
    let Some(device) = gallery_state(|s| {
        if s.full_files.get(&id).is_some_and(|p| p.exists()) {
            return None;
        }
        let is_video = s.items.iter().find(|i| i.id == id).is_some_and(|i| i.duration.is_some());
        if is_video {
            return None;
        }
        let dir = s.device.map(thumbs_dir)?;
        for ext in ["jpg", "jpeg", "png", "webp", "heic"] {
            let candidate = dir.join(format!("full-{:016x}.{ext}", fingerprint(&id)));
            if candidate.exists() {
                s.full_files.insert(id.clone(), candidate);
                return None;
            }
        }
        s.busy_item = Some(id.clone());
        s.device
    }) else {
        gallery_changed();
        return;
    };
    gallery_changed();
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        match node.fetch_photo(device, id.clone()).await {
            Ok(transfer) => asked(transfer, Intent::View { device, item_id: id }),
            Err(e) => {
                tracing::debug!(error = %e, "can't fetch full photo for viewer");
                gallery_state(|s| {
                    if s.busy_item.as_deref() == Some(id.as_str()) {
                        s.busy_item = None;
                    }
                });
                gallery_changed();
            }
        }
    });
}

/// Opens `id` in the Windows Photos app (or default video player), downloading
/// it first if needed.
pub fn open_item(id: String) {
    if id.is_empty() {
        return;
    }
    if let Some(existing) = gallery_state(|s| s.full_files.get(&id).filter(|p| p.exists()).cloned()) {
        crate::transfers::open(&existing);
        return;
    }
    let Some(device) = gallery_state(|s| {
        s.saving = true;
        s.busy_item = Some(id.clone());
        s.device
    }) else {
        return;
    };
    gallery_changed();
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        match node.fetch_photo(device, id.clone()).await {
            Ok(transfer) => asked(transfer, Intent::Open { item_id: Some(id) }),
            Err(e) => {
                gallery_state(|s| {
                    s.saving = false;
                    s.busy_item = None;
                });
                gallery_changed();
                show_photo_error(&e);
            }
        }
    });
}

/// Copies one image `id` to the PC clipboard, downloading it first if needed.
pub fn copy_item(id: String) {
    if id.is_empty() {
        return;
    }
    if let Some(existing) = gallery_state(|s| s.full_files.get(&id).filter(|p| p.exists()).cloned()) {
        copy_file_to_clipboard(&existing, false, true);
        return;
    }
    let Some(device) = gallery_state(|s| {
        s.saving = true;
        s.busy_item = Some(id.clone());
        s.device
    }) else {
        return;
    };
    gallery_changed();
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        match node.fetch_photo(device, id.clone()).await {
            Ok(transfer) => asked(transfer, Intent::Copy { item_id: Some(id), notify_ui: true }),
            Err(e) => {
                gallery_state(|s| {
                    s.saving = false;
                    s.busy_item = None;
                });
                gallery_changed();
                show_photo_error(&e);
            }
        }
    });
}

/// Sets the preferred destination folder for saving gallery items (`None`
/// resets to `Downloads\Nectarlink`).
pub fn set_save_folder(folder: Option<PathBuf>) {
    gallery_state(|s| s.save_folder = folder);
    gallery_changed();
}

/// Downloads and saves `ids` to `folder` (or the remembered `save_folder`, or
/// default `Downloads\Nectarlink`).
pub fn save_items(ids: Vec<String>, folder: Option<PathBuf>) {
    let ids: Vec<String> = ids.into_iter().filter(|id| !id.is_empty()).collect();
    if ids.is_empty() {
        return;
    }
    let Some((device, target_folder)) = gallery_state(|s| {
        if folder.is_some() {
            s.save_folder = folder.clone();
        }
        s.saving = true;
        s.device.map(|d| (d, folder.or_else(|| s.save_folder.clone())))
    }) else {
        return;
    };
    gallery_changed();
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        match node.fetch_photos(device, ids.clone()).await {
            Ok(transfer) => asked(transfer, Intent::GallerySave { ids, folder: target_folder }),
            Err(e) => {
                gallery_state(|s| {
                    s.saving = false;
                    s.busy_item = None;
                });
                gallery_changed();
                show_photo_error(&e);
            }
        }
    });
}

fn show_photo_error(error: &Error) {
    match error {
        Error::NotFound => show_message("That photo isn't on the phone anymore."),
        Error::Denied => show_message("The phone doesn't share photos or files with this PC."),
        other => show_message(describe(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gallery_view_formats_rows_and_prev_dates() {
        gallery_state(|s| {
            *s = GalleryState {
                status: Status::Ready,
                album: "bucket:1".into(),
                albums: vec![PhotoAlbum {
                    id: "bucket:1".into(),
                    name: "Camera".into(),
                    count: 2,
                    cover: Some("media:2".into()),
                }],
                items: vec![
                    PhotoItem {
                        id: "media:2".into(),
                        name: "IMG_0002.jpg".into(),
                        date: 1_710_000_000_000,
                        size: 120_000,
                        width: 1920,
                        height: 1080,
                        duration: None,
                        album: Some("bucket:1".into()),
                    },
                    PhotoItem {
                        id: "video:1".into(),
                        name: "VID_0001.mp4".into(),
                        date: 1_709_900_000_000,
                        size: 4_500_000,
                        width: 1280,
                        height: 720,
                        duration: Some(14_500),
                        album: Some("bucket:1".into()),
                    },
                ],
                more: true,
                ..GalleryState::default()
            };
        });

        let v = view();
        assert_eq!(v.status, Status::Ready);
        assert_eq!(v.album, "bucket:1");
        assert!(v.more);
        assert_eq!(v.rows.len(), 2);
        assert_eq!(v.rows[0].id, "media:2");
        assert_eq!(v.rows[0].prev_date, 0);
        assert!(!v.rows[0].is_video);
        assert_eq!(v.rows[1].id, "video:1");
        assert_eq!(v.rows[1].prev_date, 1_710_000_000_000);
        assert!(v.rows[1].is_video);
        assert_eq!(v.rows[1].duration, 14_500);
    }

    #[test]
    fn free_path_picks_unused_numbered_name() {
        let dir = tempfile::tempdir().unwrap();
        let first = free_path(dir.path(), "IMG_1.jpg");
        assert_eq!(first, dir.path().join("IMG_1.jpg"));
        std::fs::write(&first, b"a").unwrap();

        let second = free_path(dir.path(), "IMG_1.jpg");
        assert_eq!(second, dir.path().join("IMG_1 (2).jpg"));
    }
}
