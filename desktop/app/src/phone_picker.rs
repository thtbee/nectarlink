// SPDX-License-Identifier: GPL-3.0-or-later
//! Picking a file from the phone for the Messages composer
//! (`docs/protocol/photos.md`, `docs/protocol/storage.md`).
//!
//! The picker has a tab per way of choosing: **Photos** (the phone's gallery,
//! newest first, with thumbnails) and **Files** (its folders). A pick is
//! brought to this PC, staged as the message's attachment exactly as a file
//! chosen on this PC is ([`crate::messages::attach_file`]), and the copy made
//! here is deleted. Photos come as an ordinary files transfer
//! (`photos.get`); files are read through `storage.read`.
//!
//! Adding a tab (the phone's camera, say) means another list in [`State`],
//! the calls that fill it, and a tab in `PhonePickerSheet.qml`.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Mutex,
};

use nectarlink_core::{DeviceId, Error, LinkState, Node, NodeEvent, PhotoItem, StorageEntry};
use serde_json::{Value, json};

use crate::{
    core_host,
    i18n::tr,
    messages::{self, Status},
    photos,
    state::Changes,
};

/// Photos asked for at a time.
const PAGE: u32 = 60;
/// Thumbnails asked for at a time (the protocol's limit).
const THUMB_BATCH: usize = 24;
/// Pages asked for in a row while a page has only videos, which can't be sent.
const SKIPPED_PAGES: usize = 5;
/// The most a file from the phone may be: [`messages::attach_file`] reads no more.
const LARGEST_FILE: u64 = 50 * 1024 * 1024;
const STREAM_CHUNK: usize = 64 * 1024;

#[derive(Debug, Default)]
struct Gallery {
    status: Status,
    /// Photos (not videos), newest first.
    items: Vec<PhotoItem>,
    /// Where the next page starts: the last photo or video listed.
    cursor: Option<(i64, String)>,
    more: bool,
    loading_older: bool,
    /// Photos whose thumbnail is on disk.
    thumbs: HashSet<String>,
}

#[derive(Debug, Default)]
struct Folders {
    status: Status,
    /// The folder shown, relative to the phone's storage ("" for its root).
    path: String,
    entries: Vec<StorageEntry>,
    /// Bumped on each folder asked for, so a slow answer isn't shown late.
    visit: u64,
}

#[derive(Debug, Default)]
struct State {
    device: Option<DeviceId>,
    /// Bumped when the picker opens or closes, so late answers are dropped.
    generation: u64,
    gallery: Gallery,
    folders: Folders,
    /// The photo or file being brought over (its photo ID or path), and a
    /// number telling its answer from an older one's.
    picking: Option<(u64, String)>,
    last_pick: u64,
    /// Counts the files staged, so the sheet can close on one.
    picked: u64,
    /// Why the last pick failed ("" when it didn't).
    problem: String,
    /// Bumped as thumbnails arrive.
    thumb_revision: u64,
}

impl State {
    /// Starts bringing `key` over: the number its answer carries, or `None`
    /// while another pick is under way.
    fn begin(&mut self, key: String) -> Option<u64> {
        if self.picking.is_some() {
            return None;
        }
        self.last_pick += 1;
        self.picking = Some((self.last_pick, key));
        self.problem.clear();
        Some(self.last_pick)
    }

    /// Whether `token` is the pick under way; it ends.
    fn finish(&mut self, token: u64) -> bool {
        if self.picking.as_ref().is_some_and(|(t, _)| *t == token) {
            self.picking = None;
            return true;
        }
        false
    }
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn state<T>(f: impl FnOnce(&mut State) -> T) -> T {
    f(STATE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_default())
}

fn changed() {
    core_host::host().hub.changed(Changes::MESSAGES);
}

/// What the sheet shows.
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub device: Option<DeviceId>,
    pub photos_status: Status,
    pub photos: Value,
    pub photos_more: bool,
    pub photos_loading_older: bool,
    pub thumb_revision: u64,
    pub files_status: Status,
    pub folder: String,
    pub crumbs: Value,
    pub entries: Value,
    /// The photo ID or path being brought over, or "".
    pub busy: String,
    pub problem: String,
    pub picked: u64,
}

pub fn view() -> View {
    state(|s| View {
        device: s.device,
        photos_status: s.gallery.status,
        photos: photo_rows(&s.gallery.items),
        photos_more: s.gallery.more,
        photos_loading_older: s.gallery.loading_older,
        thumb_revision: s.thumb_revision,
        files_status: s.folders.status,
        folder: s.folders.path.clone(),
        crumbs: crumbs(&s.folders.path),
        entries: entry_rows(&s.folders.path, &s.folders.entries),
        busy: s.picking.as_ref().map(|(_, key)| key.clone()).unwrap_or_default(),
        problem: s.problem.clone(),
        picked: s.picked,
    })
}

fn photo_rows(items: &[PhotoItem]) -> Value {
    Value::Array(
        items
            .iter()
            .map(|i| json!({ "id": i.id, "name": i.name, "date": i.date, "width": i.width, "height": i.height }))
            .collect(),
    )
}

/// `folder` and `name` as one path on the phone.
fn join(folder: &str, name: &str) -> String {
    if folder.is_empty() { name.to_owned() } else { format!("{folder}/{name}") }
}

/// The folders from the root down to `path`: `[{ name, path }]`, the root's
/// name empty.
fn crumbs(path: &str) -> Value {
    let mut out = vec![json!({ "name": "", "path": "" })];
    let mut at = String::new();
    for name in path.split('/').filter(|n| !n.is_empty()) {
        at = join(&at, name);
        out.push(json!({ "name": name, "path": at }));
    }
    Value::Array(out)
}

/// The folder's entries, folders first as the phone lists them, without
/// the hidden ones (a dot first). A file says whether a message can carry it.
fn entry_rows(folder: &str, entries: &[StorageEntry]) -> Value {
    Value::Array(
        entries
            .iter()
            .filter(|e| !e.name.starts_with('.'))
            .map(|e| {
                json!({
                    "name": e.name,
                    "path": join(folder, &e.name),
                    "isDir": e.is_dir,
                    "size": messages::format_attachment_size(usize::try_from(e.size).unwrap_or(usize::MAX)),
                    "modified": e.modified,
                    "canAttach": !e.is_dir && messages::is_attachable_name(&e.name),
                })
            })
            .collect(),
    )
}

/// Opens the picker for `device`, with nothing loaded yet (each tab loads
/// when it's first shown).
pub fn open(device: DeviceId) {
    state(|s| {
        *s = State {
            device: Some(device),
            generation: s.generation.wrapping_add(1),
            last_pick: s.last_pick,
            picked: s.picked,
            ..State::default()
        };
    });
    changed();
}

/// The picker closed: what's still on its way is dropped.
pub fn close() {
    state(|s| {
        s.generation = s.generation.wrapping_add(1);
        s.picking = None;
    });
    changed();
}

pub fn on_event(event: &NodeEvent) {
    let device = state(|s| s.device);
    match event {
        NodeEvent::LinkChanged { device: d, link: LinkState::Online { .. } } if Some(*d) == device => {
            if state(|s| s.gallery.status == Status::Offline) {
                load_photos();
            }
            if let Some(path) =
                state(|s| (s.folders.status == Status::Offline).then(|| s.folders.path.clone()))
            {
                open_folder(path);
            }
        }
        NodeEvent::DeviceRemoved(d) if Some(*d) == device => close(),
        _ => {}
    }
}

fn why(error: &Error) -> String {
    match error {
        Error::Denied => tr!(
            "PhonePicker",
            "The phone doesn't share that with this PC. Allow it in the NectarLink app on the phone."
        ),
        Error::Offline | Error::NotPaired | Error::Timeout => {
            tr!("PhonePicker", "The phone isn't connected.")
        }
        Error::NotFound => tr!("PhonePicker", "That file is gone from the phone."),
        Error::TooLarge => tr!("PhonePicker", "That file is too large to attach."),
        _ => tr!("PhonePicker", "The phone couldn't send that file."),
    }
}

// ---- Photos ----

/// Lists the phone's latest photos (when the Photos tab is shown for the
/// first time, or to try again).
pub fn load_photos() {
    let started = state(|s| {
        if s.gallery.status == Status::Loading || s.gallery.status == Status::Ready {
            return false;
        }
        s.gallery = Gallery { status: Status::Loading, ..Gallery::default() };
        true
    });
    if started {
        changed();
        list_photos(None);
    }
}

/// Lists the photos before the last one shown.
pub fn load_older_photos() {
    let before = state(|s| {
        if s.gallery.loading_older || !s.gallery.more {
            return None;
        }
        s.gallery.loading_older = true;
        s.gallery.cursor.clone()
    });
    if before.is_some() {
        changed();
        list_photos(before);
    }
}

fn list_photos(mut before: Option<(i64, String)>) {
    let Some((device, generation)) = state(|s| s.device.map(|d| (d, s.generation))) else { return };
    let Some(node) = core_host::node() else {
        gallery_failed(generation, &Error::Offline);
        return;
    };
    core_host::spawn(async move {
        let mut shown = Vec::new();
        let mut cursor = None;
        let mut more = false;
        for _ in 0..SKIPPED_PAGES {
            let page = match node.photo_list(device, None, before.clone(), PAGE).await {
                Ok(page) => page,
                Err(e) => {
                    tracing::debug!(error = %e, "can't list photos for the picker");
                    gallery_failed(generation, &e);
                    return;
                }
            };
            more = page.len() as u32 >= PAGE;
            cursor = page.last().map(|i| (i.date, i.id.clone()));
            shown.extend(page.into_iter().filter(|i| i.duration.is_none()));
            // A page of videos only shows nothing: go on to the next.
            if !shown.is_empty() || !more {
                break;
            }
            before = cursor.clone();
        }
        let ids: Vec<String> = shown.iter().map(|i| i.id.clone()).collect();
        let applied = state(|s| {
            if s.generation != generation {
                return false;
            }
            let ready = ids.iter().filter(|id| photos::thumb_path(device, id).exists()).cloned();
            s.gallery.thumbs.extend(ready);
            let known: HashSet<String> = s.gallery.items.iter().map(|i| i.id.clone()).collect();
            s.gallery.items.extend(shown.into_iter().filter(|i| !known.contains(&i.id)));
            s.gallery.cursor = cursor;
            s.gallery.more = more;
            s.gallery.loading_older = false;
            s.gallery.status = Status::Ready;
            true
        });
        if !applied {
            return;
        }
        changed();
        fetch_thumbs(&node, device, generation, ids).await;
    });
}

fn gallery_failed(generation: u64, error: &Error) {
    let applied = state(|s| {
        if s.generation != generation {
            return false;
        }
        s.gallery.loading_older = false;
        // A later page that fails leaves the photos already shown.
        if s.gallery.items.is_empty() {
            s.gallery.status = Status::of(error);
        } else {
            s.problem = why(error);
        }
        true
    });
    if applied {
        changed();
    }
}

/// Brings the thumbnails not on disk yet, a batch at a time.
async fn fetch_thumbs(node: &Node, device: DeviceId, generation: u64, ids: Vec<String>) {
    let missing: Vec<String> =
        state(|s| ids.into_iter().filter(|id| !s.gallery.thumbs.contains(id)).collect());
    for batch in missing.chunks(THUMB_BATCH) {
        if state(|s| s.generation != generation) {
            return;
        }
        let thumbs = match node.photo_thumbs(device, batch.to_vec()).await {
            Ok(thumbs) => thumbs,
            Err(e) => {
                tracing::debug!(error = %e, "can't fetch thumbnails for the picker");
                return;
            }
        };
        let saved: Vec<String> = thumbs
            .into_iter()
            .filter(|t| photos::write_thumb(device, &t.id, &t.data).is_some())
            .map(|t| t.id)
            .collect();
        state(|s| {
            if s.generation == generation {
                s.gallery.thumbs.extend(saved);
                s.thumb_revision += 1;
            }
        });
        changed();
    }
}

/// The thumbnail's file URL, or "" when it hasn't come.
pub fn thumb_url(id: &str) -> String {
    let device = state(|s| s.gallery.thumbs.contains(id).then_some(s.device)).flatten();
    device.map(|d| crate::icons::file_url(&photos::thumb_path(d, id))).unwrap_or_default()
}

/// Brings photo `id` over and attaches it.
pub fn pick_photo(id: String) {
    let Some((device, token)) = begin(id.clone()) else { return };
    let Some(node) = core_host::node() else {
        failed(token, &Error::Offline);
        return;
    };
    core_host::spawn(async move {
        match node.fetch_photo(device, id).await {
            // The transfer's end brings it to `downloaded`.
            Ok(transfer) => photos::attach_requested(transfer, token),
            Err(e) => {
                tracing::debug!(error = %e, "can't fetch the photo to attach");
                failed(token, &e);
            }
        }
    });
}

// ---- Files ----

/// Lists a folder of the phone ("" for its root).
pub fn open_folder(path: String) {
    let Some((device, generation, visit)) = state(|s| {
        let device = s.device?;
        s.folders.visit += 1;
        s.folders.status = Status::Loading;
        s.folders.path = path.clone();
        s.folders.entries.clear();
        Some((device, s.generation, s.folders.visit))
    }) else {
        return;
    };
    changed();
    let Some(node) = core_host::node() else {
        folder_listed(generation, visit, Err(Error::Offline));
        return;
    };
    core_host::spawn(async move {
        let result = node.storage_list(device, path).await;
        folder_listed(generation, visit, result);
    });
}

fn folder_listed(generation: u64, visit: u64, result: Result<Vec<StorageEntry>, Error>) {
    let applied = state(|s| {
        if s.generation != generation || s.folders.visit != visit {
            return false;
        }
        match result {
            Ok(entries) => {
                s.folders.entries = entries;
                s.folders.status = Status::Ready;
            }
            Err(e) => {
                tracing::debug!(error = %e, "can't list the phone's folder for the picker");
                s.folders.status = Status::of(&e);
            }
        }
        true
    });
    if applied {
        changed();
    }
}

/// Brings the file at `path` (in the folder shown) over and attaches it.
pub fn pick_file(path: String) {
    let Some((name, size)) = state(|s| {
        s.folders
            .entries
            .iter()
            .find(|e| !e.is_dir && join(&s.folders.path, &e.name) == path)
            .map(|e| (e.name.clone(), e.size))
    }) else {
        return;
    };
    if !messages::is_attachable_name(&name) {
        return;
    }
    // Staged as the picture or voice note it is, with the usual reason it can't go.
    if size > LARGEST_FILE {
        messages::stage_error(
            name,
            tr!(
                "Messages",
                "Attachment is %1 (MMS supports up to 900 KB)",
                messages::format_attachment_size(usize::try_from(size).unwrap_or(usize::MAX))
            ),
        );
        state(|s| s.picked += 1);
        changed();
        return;
    }
    let Some((device, token)) = begin(path.clone()) else { return };
    let Some(node) = core_host::node() else {
        failed(token, &Error::Offline);
        return;
    };
    core_host::spawn(async move {
        let dir = pick_dir().join(format!("{token}"));
        let dest = dir.join(photos::sanitize_drag_filename(&name, ""));
        match read_to_file(&node, device, path, &dest).await {
            Ok(()) => downloaded(token, dest),
            Err(e) => {
                tracing::debug!(error = %e, "can't read the file to attach");
                let _ = tokio::fs::remove_dir_all(&dir).await;
                failed(token, &e);
            }
        }
    });
}

/// Writes the phone's file `remote` to `dest`.
async fn read_to_file(node: &Node, device: DeviceId, remote: String, dest: &Path) -> Result<(), Error> {
    use tokio::io::AsyncWriteExt;

    let (meta, mut recv) = node.storage_read_stream(device, remote, 0, None).await?;
    if meta.length > LARGEST_FILE {
        return Err(Error::TooLarge);
    }
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut file = tokio::fs::File::create(dest).await?;
    let mut written = 0u64;
    while let Some(chunk) = recv.read_chunk(STREAM_CHUNK).await.map_err(|e| Error::Network(e.to_string()))? {
        written += chunk.len() as u64;
        if written > meta.length {
            return Err(Error::Protocol("more bytes than announced".into()));
        }
        file.write_all(&chunk).await?;
    }
    if written != meta.length {
        return Err(Error::Network("the file came in part".into()));
    }
    file.flush().await?;
    Ok(())
}

// ---- Staging ----

/// Where files from the phone wait to be staged.
fn pick_dir() -> PathBuf {
    core_host::host().data_dir.join("cache").join("phone-picker")
}

fn begin(key: String) -> Option<(DeviceId, u64)> {
    let begun = state(|s| Some((s.device?, s.begin(key)?)));
    if begun.is_some() {
        changed();
    }
    begun
}

/// A pick that didn't arrive.
pub(crate) fn failed(token: u64, error: &Error) {
    let current = state(|s| {
        let current = s.finish(token);
        if current {
            s.problem = why(error);
        }
        current
    });
    if current {
        changed();
    }
}

/// The file picked is on this PC: stages it as the attachment and deletes
/// it (and what it came in).
pub(crate) fn downloaded(token: u64, path: PathBuf) {
    let current = state(|s| s.finish(token));
    core_host::spawn(async move {
        let staged = current && {
            let path = path.clone();
            tokio::task::spawn_blocking(move || {
                let kept = messages::attach_file(path.to_string_lossy().into_owned());
                // A picture too big for a message is staged too, to say so.
                kept || messages::has_attachment()
            })
            .await
            .unwrap_or(false)
        };
        let _ = tokio::fs::remove_file(&path).await;
        if let Some(dir) = path.parent().filter(|d| d.starts_with(pick_dir())) {
            let _ = tokio::fs::remove_dir(dir).await;
        }
        if !current {
            return;
        }
        state(|s| {
            if staged {
                s.picked += 1;
            } else {
                s.problem = tr!("PhonePicker", "That file can't be attached.");
            }
        });
        changed();
    });
}

/// Removes what a pick left behind in a run that was cut short.
pub fn clean_up() {
    let _ = std::fs::remove_dir_all(pick_dir());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, is_dir: bool) -> StorageEntry {
        StorageEntry { name: name.into(), size: 2048, modified: 7, is_dir }
    }

    #[test]
    fn crumbs_run_from_the_root_down() {
        assert_eq!(crumbs(""), json!([{ "name": "", "path": "" }]));
        assert_eq!(
            crumbs("DCIM/Camera"),
            json!([{ "name": "", "path": "" }, { "name": "DCIM", "path": "DCIM" }, { "name": "Camera", "path": "DCIM/Camera" }])
        );
    }

    #[test]
    fn entries_hide_dot_files_and_say_which_can_be_attached() {
        let rows = entry_rows(
            "DCIM",
            &[
                entry("Camera", true),
                entry(".thumbnails", true),
                entry("a.jpg", false),
                entry("b.pdf", false),
            ],
        );
        let rows = rows.as_array().unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0]["path"], "DCIM/Camera");
        assert_eq!(rows[0]["canAttach"], false, "a folder is opened, not attached");
        assert_eq!(rows[1]["path"], "DCIM/a.jpg");
        assert_eq!(rows[1]["canAttach"], true);
        assert_eq!(rows[1]["size"], "2 KB");
        assert_eq!(rows[2]["canAttach"], false);
        assert_eq!(entry_rows("", &[entry("a.jpg", false)])[0]["path"], "a.jpg");
    }

    #[test]
    fn one_pick_at_a_time_and_a_late_answer_is_told_apart() {
        let mut s = State::default();
        let first = s.begin("a".into()).unwrap();
        assert_eq!(s.begin("b".into()), None, "a second pick waits for the first");
        assert!(!s.finish(first + 1), "someone else's answer changes nothing");
        assert!(s.picking.is_some());
        assert!(s.finish(first));
        assert!(!s.finish(first), "an answer counts once");
        let second = s.begin("b".into()).unwrap();
        assert_ne!(first, second, "an answer for the old pick can't end the new one");
        assert!(!s.finish(first));
    }

    #[test]
    fn a_new_pick_forgets_the_last_problem() {
        let mut s = State { problem: "no".into(), ..State::default() };
        s.begin("a".into());
        assert!(s.problem.is_empty());
    }

    #[test]
    fn photos_are_listed_with_what_the_grid_needs() {
        let item = PhotoItem {
            id: "7".into(),
            name: "IMG.jpg".into(),
            date: 9,
            size: 1,
            width: 4,
            height: 3,
            duration: None,
            album: None,
        };
        assert_eq!(
            photo_rows(&[item]),
            json!([{ "id": "7", "name": "IMG.jpg", "date": 9, "width": 4, "height": 3 }])
        );
    }
}
