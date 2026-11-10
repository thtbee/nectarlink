// SPDX-License-Identifier: MPL-2.0
//! The files service (docs/protocol/files.md): sending files to a paired
//! device on a stream of their own, receiving them, progress, cancelling,
//! and resuming after a dropped connection.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use bytes::Bytes;
use iroh::endpoint::{ReadError, RecvStream, SendStream, VarInt, WriteError};
pub use nectarlink_protocol::messages::{RecordingMarker, files::RECORDER};
use nectarlink_protocol::{
    DeviceId, Envelope, ErrorCode,
    messages::{
        ErrorBody, FileEntry, FilesAccept, FilesControl, FilesOffer, StreamHeader, clip, files,
        is_valid_file_name, is_valid_folder, mirror, types,
    },
    read_frame, write_frame,
};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

use crate::{Error, Result, events::NodeEvent, node::Shared, session::Session};

/// The device toggle that allows files for a device.
pub(crate) const TOGGLE: &str = "files";
/// The device toggle that allows voice recordings for a device.
pub(crate) const RECORDINGS_TOGGLE: &str = "recordings";

/// How long a sender keeps trying to reach the device before giving up.
const RETRY_FOR: Duration = Duration::from_secs(10 * 60);
/// Waiting for the other side's answer or confirmation.
const REPLY_TIMEOUT: Duration = Duration::from_secs(30);
/// Progress events, at most this often per transfer.
const PROGRESS_EVERY: Duration = Duration::from_millis(200);
/// Queue progress updates and `QueueChanged` events, at most this often per transfer.
const QUEUE_PROGRESS_EVERY: Duration = Duration::from_millis(1000);
/// Partly received files are kept this long for resuming.
const KEEP_PARTIAL: Duration = Duration::from_secs(60 * 60);
const CHUNK: usize = 256 * 1024;
/// The sender reads ahead in blocks of this size, this many at a time, so
/// the disk is read while the network sends.
const BLOCK: usize = 1024 * 1024;
const READ_AHEAD: usize = 8;
/// The receiver writes the disk in pieces this large (QUIC delivers a few
/// kilobytes at a time).
const WRITE_BUFFER: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Outgoing,
    Incoming,
}

/// Why a transfer didn't complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferFailure {
    /// The other device has files turned off for this one.
    Denied,
    /// The other device couldn't be reached for a while.
    Unreachable,
    /// The receiving device ran out of space.
    NoSpace,
    /// Stopped by this device shutting down; partly received files are kept,
    /// so it can still resume.
    Interrupted,
    /// Received bytes did not match the sender's BLAKE3 digest.
    Integrity,
    /// Anything else; for logs, never holds file names.
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferState {
    /// An incoming transfer waiting for the user to accept or decline
    /// (e.g. from a nearby LocalSend device).
    Requested,
    /// Waiting for the other device: to connect (sending), or to continue
    /// after the connection dropped (receiving; the sender resumes it).
    Waiting,
    Running,
    /// Manually paused by either side; partial files are kept on the receiver
    /// until resumed or cancelled.
    Paused,
    /// Received files are where `saved` says: one path per item, each
    /// file and each sent folder (as [`Transfer::names`] lists them).
    Done {
        saved: Vec<PathBuf>,
    },
    Failed(TransferFailure),
    Cancelled,
}

impl TransferState {
    pub fn is_finished(&self) -> bool {
        matches!(self, TransferState::Done { .. } | TransferState::Failed(_) | TransferState::Cancelled)
    }
}

/// A transfer as the UI shows it.
#[derive(Clone, PartialEq, Eq)]
pub struct Transfer {
    pub id: String,
    pub device: DeviceId,
    pub direction: Direction,
    /// What was sent, as the user picked it: file and folder names, in order.
    pub names: Vec<String>,
    /// How many files that is (a folder's files included).
    pub files: usize,
    /// Bytes in total and done so far.
    pub total: u64,
    pub done: u64,
    pub state: TransferState,
    /// True when this transfer is a voice recording (`docs/protocol/recorder.md`).
    pub recording: bool,
    /// Timestamped markers placed during the recording.
    pub markers: Vec<RecordingMarker>,
    /// True when the sender handed off a single document that should open with
    /// the default app once it arrives (`docs/protocol/files.md`).
    pub open_on_arrival: bool,
}

/// Never prints file names (protocol v0 §11).
impl std::fmt::Debug for Transfer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Transfer")
            .field("id", &self.id)
            .field("direction", &self.direction)
            .field("items", &self.names.len())
            .field("files", &self.files)
            .field("done", &self.done)
            .field("total", &self.total)
            .field("state", &self.state)
            .field("recording", &self.recording)
            .field("markers", &self.markers.len())
            .field("open_on_arrival", &self.open_on_arrival)
            .finish()
    }
}

/// A file to send.
#[derive(Debug)]
pub struct OutgoingFile {
    /// The name the other device sees (no path).
    pub name: String,
    /// The folder it's in, when sending a folder: `/`-separated names,
    /// starting with the sent folder's own.
    pub folder: Option<String>,
    pub source: FileSource,
}

/// The files to send for paths the user picked: files as they are, and
/// folders with everything in them (empty folders and links aside).
pub fn outgoing_paths(paths: &[PathBuf]) -> Result<Vec<OutgoingFile>> {
    let mut out = Vec::new();
    for path in paths {
        let meta = std::fs::symlink_metadata(path)?;
        if meta.is_dir() {
            let name =
                path.file_name().map_or_else(|| "Folder".into(), |n| safe_file_name(&n.to_string_lossy()));
            add_folder(path, &name, 1, &mut out)?;
        } else if meta.is_file() {
            let name =
                path.file_name().map_or_else(|| "file".into(), |n| safe_file_name(&n.to_string_lossy()));
            out.push(OutgoingFile { name, folder: None, source: FileSource::Path(path.clone()) });
        }
        if out.len() > files::MAX_FILES {
            return Err(Error::TooLarge);
        }
    }
    Ok(out)
}

fn add_folder(dir: &Path, folder: &str, depth: usize, out: &mut Vec<OutgoingFile>) -> Result<()> {
    if depth > files::MAX_FOLDER_DEPTH || folder.len() > files::MAX_FOLDER_BYTES {
        return Err(Error::TooLarge);
    }
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<std::io::Result<_>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        // Not followed: links (and junctions) could lead anywhere.
        let kind = entry.file_type()?;
        let name = safe_file_name(&entry.file_name().to_string_lossy());
        if kind.is_dir() {
            add_folder(&entry.path(), &format!("{folder}/{name}"), depth + 1, out)?;
        } else if kind.is_file() {
            out.push(OutgoingFile {
                name,
                folder: Some(folder.to_owned()),
                source: FileSource::Path(entry.path()),
            });
            if out.len() > files::MAX_FILES {
                return Err(Error::TooLarge);
            }
        }
    }
    Ok(())
}

/// What a transfer's files are, as the user picked them: each file outside
/// a folder, and each folder once.
fn items(entries: &[FileEntry]) -> Vec<String> {
    let mut items: Vec<String> = Vec::new();
    let mut last_folder: Option<&str> = None;
    for entry in entries {
        match entry.folder.as_deref().map(|f| f.split('/').next().unwrap_or(f)) {
            Some(top) if last_folder == Some(top) => {}
            Some(top) => {
                items.push(top.to_owned());
                last_folder = Some(top);
            }
            None => {
                items.push(entry.name.clone());
                last_folder = None;
            }
        }
    }
    items
}

/// A name Windows can store: no reserved device names (`CON`, `COM1`…),
/// characters it rejects (including `:`, which would make the name a drive
/// prefix or an alternate data stream), or trailing dots and spaces.
pub fn storable_name(name: &str) -> String {
    let mut clean: String = name
        .chars()
        .map(|c| if matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') { '_' } else { c })
        .collect();
    while clean.ends_with(['.', ' ']) {
        clean.pop();
    }
    let stem = clean.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        clean.insert(0, '_');
    }
    if clean.is_empty() { "file".into() } else { clean }
}

#[derive(Debug)]
pub enum FileSource {
    Path(PathBuf),
    /// An open file (e.g. a descriptor the Android app got for a shared
    /// item). Must be seekable for resuming.
    File(std::fs::File),
}

/// A file name the spec allows, from whatever the sender has.
pub fn safe_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let mut clean: String =
        base.chars().map(|c| if c.is_control() { '_' } else { c }).collect::<String>().trim().to_owned();
    while clean.len() > files::MAX_NAME_BYTES {
        clean.pop();
    }
    if is_valid_file_name(&clean) { clean } else { "file".into() }
}

/// `dir/name`, or `dir/name (2).ext` and so on if that's taken.
pub(crate) fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(dot) if dot > 0 => (&name[..dot], &name[dot..]),
        _ => (name, ""),
    };
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .unwrap_or_else(|| dir.join(format!("{stem} ({}){ext}", new_id())))
}

pub(crate) fn new_id() -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    (0..24).map(|_| ALPHABET[rand::random_range(0..ALPHABET.len())] as char).collect()
}

pub(crate) fn cleanup_staging_id(shared: &Shared, id: &str) {
    let dir = shared.queue_staging_dir().join(id);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Publishes a transfer's progress, at most every [`PROGRESS_EVERY`].
struct Reporter {
    shared: Arc<Shared>,
    transfer: Transfer,
    last: Instant,
    last_queue: Instant,
    cumulative_sizes: Vec<u64>,
    update_queue: bool,
}

impl Reporter {
    fn new(shared: Arc<Shared>, transfer: Transfer) -> Reporter {
        let now = Instant::now();
        Reporter {
            shared,
            transfer,
            last: now,
            last_queue: now,
            cumulative_sizes: Vec::new(),
            update_queue: false,
        }
    }

    fn with_queue(shared: Arc<Shared>, transfer: Transfer, file_sizes: &[u64]) -> Reporter {
        let cumulative_sizes = file_sizes
            .iter()
            .scan(0u64, |acc, &s| {
                *acc = acc.saturating_add(s);
                Some(*acc)
            })
            .collect();
        let now = Instant::now();
        Reporter { shared, transfer, last: now, last_queue: now, cumulative_sizes, update_queue: true }
    }

    fn files_completed(&self, done: u64) -> u32 {
        u32::try_from(self.cumulative_sizes.iter().filter(|&&end| done >= end).count()).unwrap_or(u32::MAX)
    }

    fn set(&mut self, state: TransferState) {
        self.transfer.state = state;
        self.shared.emit(NodeEvent::Transfer(self.transfer.clone()));
        self.last = Instant::now();
    }

    fn progress(&mut self, done: u64) {
        let advanced = done > self.transfer.done;
        self.transfer.done = done;
        if advanced && self.last.elapsed() >= PROGRESS_EVERY {
            if self.update_queue && self.last_queue.elapsed() >= QUEUE_PROGRESS_EVERY {
                let files_done = self.files_completed(done);
                let _ = self.shared.store.update_queue_progress(&self.transfer.id, done, files_done);
                self.shared.emit(NodeEvent::QueueChanged);
                self.last_queue = Instant::now();
            }
            self.shared.emit(NodeEvent::Transfer(self.transfer.clone()));
            self.last = Instant::now();
        }
    }
}

static PHOTO_TRANSFERS: std::sync::Mutex<Option<std::collections::HashMap<String, Instant>>> =
    std::sync::Mutex::new(None);

/// Marks an incoming transfer ID as a requested photo transfer (`photos.get`).
pub(crate) fn mark_photo_transfer(shared: &Shared, transfer_id: &str) {
    if shared.store.update_timeline_kind_by_ref(transfer_id, crate::TimelineKind::Photo).unwrap_or(false) {
        shared.emit(NodeEvent::TimelineChanged);
    } else {
        let now = Instant::now();
        let mut guard = PHOTO_TRANSFERS.lock().unwrap_or_else(|e| e.into_inner());
        let map = guard.get_or_insert_with(std::collections::HashMap::new);
        map.retain(|_, t| now.duration_since(*t) < RETRY_FOR);
        map.insert(transfer_id.to_owned(), now);
    }
}

static QUIET_TRANSFERS: std::sync::Mutex<Option<std::collections::HashMap<String, Instant>>> =
    std::sync::Mutex::new(None);

/// Marks an incoming transfer as quiet: it's saved like any other, but leaves
/// nothing in the Timeline (a photo backup's downloads). An entry already
/// recorded for it (the transfer finished before this) is removed.
pub(crate) fn mark_quiet_transfer(shared: &Shared, transfer_id: &str) {
    let now = Instant::now();
    {
        let mut guard = QUIET_TRANSFERS.lock().unwrap_or_else(|e| e.into_inner());
        let map = guard.get_or_insert_with(std::collections::HashMap::new);
        map.retain(|_, t| now.duration_since(*t) < RETRY_FOR);
        map.insert(transfer_id.to_owned(), now);
    }
    if shared.store.delete_timeline_by_ref(transfer_id).unwrap_or(false) {
        // It had finished already, so nothing will take the mark.
        take_quiet_transfer(transfer_id);
        shared.emit(NodeEvent::TimelineChanged);
    }
}

/// Whether a transfer that just finished was quiet, forgetting it. A failed
/// one stays marked: it resumes under the same ID.
fn take_quiet_transfer(transfer_id: &str) -> bool {
    let now = Instant::now();
    QUIET_TRANSFERS.lock().unwrap_or_else(|e| e.into_inner()).as_mut().is_some_and(|map| {
        map.retain(|_, t| now.duration_since(*t) < RETRY_FOR);
        map.remove(transfer_id).is_some()
    })
}

fn take_photo_transfer(transfer_id: &str) -> bool {
    let now = Instant::now();
    PHOTO_TRANSFERS.lock().unwrap_or_else(|e| e.into_inner()).as_mut().is_some_and(|map| {
        map.retain(|_, t| now.duration_since(*t) < RETRY_FOR);
        map.remove(transfer_id).is_some()
    })
}

pub(crate) fn transfer_timeline_title(names: &[String]) -> String {
    match names.len() {
        0 => "Files".into(),
        1 => names[0].clone(),
        n => format!("{} (+{} more)", names[0], n - 1),
    }
}

fn transfer_timeline_detail(entries: &[FileEntry], recording: bool, markers: &[RecordingMarker]) -> String {
    if recording {
        return match markers.len() {
            0 => "Voice recording".into(),
            1 => "Voice recording · 1 marker".into(),
            n => format!("Voice recording · {n} markers"),
        };
    }
    let has_folder = entries.iter().any(|e| e.folder.is_some());
    if has_folder {
        match entries.len() {
            1 => "Folder · 1 file".into(),
            n => format!("Folder · {n} files"),
        }
    } else {
        match entries.len() {
            1 => "File".into(),
            n => format!("{n} files"),
        }
    }
}

// ---- Sending ----

/// Starts sending files; progress and the outcome arrive as
/// [`NodeEvent::Transfer`]. Returns the transfer's ID.
pub(crate) async fn send(shared: &Arc<Shared>, peer: DeviceId, files: Vec<OutgoingFile>) -> Result<String> {
    enqueue_outgoing(shared, peer, files, false, Vec::new(), false).await
}

/// Starts sending a handed-off document that should open with the default app
/// on the receiving device once it arrives (`docs/protocol/files.md`).
pub(crate) async fn send_handoff(
    shared: &Arc<Shared>,
    peer: DeviceId,
    files: Vec<OutgoingFile>,
) -> Result<String> {
    let open_on_arrival = files.len() == 1
        && files[0].folder.is_none()
        && crate::handoff::is_safe_handoff_document(&files[0].name);
    enqueue_outgoing(shared, peer, files, false, Vec::new(), open_on_arrival).await
}

/// Starts sending photos requested via `photos.get`.
pub(crate) async fn send_photos_files(
    shared: &Arc<Shared>,
    peer: DeviceId,
    files: Vec<OutgoingFile>,
) -> Result<String> {
    send_inner(shared, peer, files, false, true, Vec::new(), false).await
}

/// Starts sending a voice recording and its markers (`docs/protocol/recorder.md`);
/// progress and the outcome arrive as [`NodeEvent::Transfer`]. Returns the transfer's ID.
pub(crate) async fn send_recording(
    shared: &Arc<Shared>,
    peer: DeviceId,
    mut file: OutgoingFile,
    markers: Vec<RecordingMarker>,
) -> Result<String> {
    file.folder = None;
    let clean_markers = sanitize_markers(markers);
    enqueue_outgoing(shared, peer, vec![file], true, clean_markers, false).await
}

fn sanitize_markers(markers: Vec<RecordingMarker>) -> Vec<RecordingMarker> {
    markers
        .into_iter()
        .take(files::MAX_MARKERS)
        .map(|m| {
            let label = m.label.and_then(|raw| {
                let mut clean: String = raw
                    .chars()
                    .map(|c| if c.is_control() { ' ' } else { c })
                    .collect::<String>()
                    .trim()
                    .to_owned();
                while clean.len() > files::MAX_MARKER_LABEL_BYTES {
                    clean.pop();
                }
                let trimmed = clean.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_owned())
            });
            RecordingMarker { at_ms: m.at_ms, label }
        })
        .collect()
}

pub(crate) fn outgoing_target_path(path: &Path, folder: Option<&str>) -> String {
    if let Some(f) = folder {
        let depth = f.split('/').filter(|s| !s.is_empty()).count();
        let mut p = path;
        for _ in 0..depth {
            if let Some(parent) = p.parent() {
                p = parent;
            }
        }
        p.to_string_lossy().into_owned()
    } else {
        path.to_string_lossy().into_owned()
    }
}

async fn enqueue_outgoing(
    shared: &Arc<Shared>,
    peer: DeviceId,
    files: Vec<OutgoingFile>,
    recording: bool,
    markers: Vec<RecordingMarker>,
    open_on_arrival: bool,
) -> Result<String> {
    if files.is_empty() || files.len() > files::MAX_FILES || (recording && files.len() != 1) {
        return Err(Error::Protocol("send between 1 and 5000 files".into()));
    }
    if !shared.store.is_paired(&peer)? {
        if !recording && shared.localsend.has_peer(&peer) {
            return crate::localsend::send_files(shared, peer, files).await;
        }
        return Err(Error::NotPaired);
    }
    let toggle = if recording { RECORDINGS_TOGGLE } else { TOGGLE };
    if !shared.toggle_on(&peer, toggle) {
        return Err(Error::Denied);
    }

    let id = new_id();
    let mut specs = Vec::with_capacity(files.len());
    let mut entries = Vec::with_capacity(files.len());

    for (idx, file) in files.into_iter().enumerate() {
        if file.folder.as_deref().is_some_and(|f| !is_valid_folder(f)) {
            cleanup_staging_id(shared, &id);
            return Err(Error::Protocol("invalid folder name".into()));
        }
        let clean_name = safe_file_name(&file.name);
        match file.source {
            FileSource::Path(path) => {
                let path_for_meta = path.clone();
                let meta_res = tokio::task::spawn_blocking(move || -> std::io::Result<(u64, i64)> {
                    let opened_file = std::fs::File::open(&path_for_meta)?;
                    let meta = opened_file.metadata()?;
                    Ok((meta.len(), crate::file_mtime_ms(&meta)))
                })
                .await
                .map_err(|e| Error::Internal(e.to_string()));
                let (size, mtime_ms) = match meta_res {
                    Ok(Ok(v)) => v,
                    Ok(Err(e)) => {
                        cleanup_staging_id(shared, &id);
                        return Err(e.into());
                    }
                    Err(e) => {
                        cleanup_staging_id(shared, &id);
                        return Err(e);
                    }
                };
                specs.push(crate::QueuedFileSpec {
                    path,
                    name: clean_name.clone(),
                    folder: file.folder.clone(),
                    size,
                    mtime_ms,
                    staged: false,
                });
                entries.push(FileEntry {
                    name: clean_name,
                    size,
                    folder: file.folder,
                    blake3: Some("0".repeat(64)),
                });
            }
            FileSource::File(handle) => {
                let stage_dir = shared.queue_staging_dir().join(&id);
                let stage_path = stage_dir.join(format!("{idx}_{}", storable_name(&clean_name)));
                let stage_path_clone = stage_path.clone();
                let staged_res = tokio::task::spawn_blocking(move || -> std::io::Result<(u64, i64)> {
                    use std::io::{Seek, SeekFrom, Write};
                    std::fs::create_dir_all(&stage_dir)?;
                    let mut src = handle;
                    let _ = src.seek(SeekFrom::Start(0));
                    let mut dst = std::fs::File::create(&stage_path_clone)?;
                    std::io::copy(&mut src, &mut dst)?;
                    dst.flush()?;
                    let meta = dst.metadata()?;
                    Ok((meta.len(), crate::file_mtime_ms(&meta)))
                })
                .await
                .map_err(|e| Error::Internal(e.to_string()));
                let (size, mtime_ms) = match staged_res {
                    Ok(Ok(v)) => v,
                    Ok(Err(e)) => {
                        cleanup_staging_id(shared, &id);
                        return Err(e.into());
                    }
                    Err(e) => {
                        cleanup_staging_id(shared, &id);
                        return Err(e);
                    }
                };
                specs.push(crate::QueuedFileSpec {
                    path: stage_path,
                    name: clean_name.clone(),
                    folder: file.folder.clone(),
                    size,
                    mtime_ms,
                    staged: true,
                });
                entries.push(FileEntry {
                    name: clean_name,
                    size,
                    folder: file.folder,
                    blake3: Some("0".repeat(64)),
                });
            }
        }
    }

    let open_on_arrival = open_on_arrival
        && !recording
        && entries.len() == 1
        && entries[0].folder.is_none()
        && crate::handoff::is_safe_handoff_document(&entries[0].name);

    let offer = Envelope::new(
        files::OFFER,
        &FilesOffer {
            id: id.clone(),
            files: entries.clone(),
            recording,
            markers: markers.clone(),
            open_on_arrival,
        },
    )
    .map_err(|e| {
        cleanup_staging_id(shared, &id);
        Error::Protocol(e.to_string())
    })?;
    if offer.to_cbor().len() > nectarlink_protocol::MAX_FRAME_LEN {
        cleanup_staging_id(shared, &id);
        return Err(Error::TooLarge);
    }

    let payload = if recording {
        let spec =
            specs.into_iter().next().ok_or_else(|| Error::Protocol("missing recording file".into()))?;
        crate::QueuePayload::Recording { file: spec, markers: markers.clone() }
    } else if open_on_arrival {
        let spec = specs.into_iter().next().ok_or_else(|| Error::Protocol("missing handoff file".into()))?;
        crate::QueuePayload::HandoffDocument { file: spec, position_json: None }
    } else if specs.iter().any(|s| s.folder.is_some()) {
        let root_name = specs
            .iter()
            .find_map(|s| s.folder.as_deref().map(|f| f.split('/').next().unwrap_or(f).to_owned()))
            .unwrap_or_else(|| "Folder".into());
        crate::QueuePayload::Folder { root_name, files: specs }
    } else {
        crate::QueuePayload::Files { files: specs }
    };

    if let Err(e) = shared.store.enqueue_send_item(&id, &peer, &payload, None, crate::now_unix_ms()) {
        cleanup_staging_id(shared, &id);
        return Err(e);
    }

    let cancel = CancellationToken::new();
    let _ = shared.register_transfer_control(&id, peer, Direction::Outgoing, cancel);

    let transfer = Transfer {
        id: id.clone(),
        device: peer,
        direction: Direction::Outgoing,
        names: items(&entries),
        files: entries.len(),
        total: entries.iter().map(|e| e.size).fold(0, u64::saturating_add),
        done: 0,
        state: TransferState::Waiting,
        recording,
        markers,
        open_on_arrival,
    };
    shared.emit(NodeEvent::QueueChanged);
    shared.emit(NodeEvent::Transfer(transfer));
    shared.queue_notify.notify_waiters();
    Ok(id)
}

fn failure_reason_string(failure: &TransferFailure) -> String {
    match failure {
        TransferFailure::Denied => "Refused by receiving device".into(),
        TransferFailure::Unreachable => "Device unreachable".into(),
        TransferFailure::NoSpace => "Not enough storage space on receiving device".into(),
        TransferFailure::Interrupted => "Interrupted by shutdown".into(),
        TransferFailure::Integrity => "BLAKE3 integrity check failed".into(),
        TransferFailure::Other(msg) => msg.clone(),
    }
}

pub(crate) async fn deliver_queued_files(
    shared: &Arc<Shared>,
    item: &crate::QueueItem,
    payload: &crate::QueuePayload,
) {
    let (specs, recording, markers, open_on_arrival) = match payload {
        crate::QueuePayload::Files { files } => (files.clone(), false, Vec::new(), false),
        crate::QueuePayload::Folder { files, .. } => (files.clone(), false, Vec::new(), false),
        crate::QueuePayload::HandoffDocument { file, .. } => (vec![file.clone()], false, Vec::new(), true),
        crate::QueuePayload::Recording { file, markers } => {
            (vec![file.clone()], true, markers.clone(), false)
        }
        _ => return,
    };

    let peer = item.peer;
    let prelim_entries: Vec<FileEntry> = specs
        .iter()
        .map(|s| FileEntry {
            name: safe_file_name(&s.name),
            size: s.size,
            folder: s.folder.clone(),
            blake3: None,
        })
        .collect();
    let file_sizes: Vec<u64> = specs.iter().map(|s| s.size).collect();
    let transfer = Transfer {
        id: item.id.clone(),
        device: peer,
        direction: Direction::Outgoing,
        names: items(&prelim_entries),
        files: prelim_entries.len(),
        total: file_sizes.iter().copied().fold(0, u64::saturating_add),
        done: item.bytes_done,
        state: TransferState::Waiting,
        recording,
        markers: markers.clone(),
        open_on_arrival,
    };

    let (cancel, mut pause_rx) = shared.ensure_transfer_control(&item.id, peer, Direction::Outgoing);
    if cancel.is_cancelled() {
        let _ = shared.store.remove_queue_item(&item.id);
        cleanup_staging_id(shared, &item.id);
        shared.emit(NodeEvent::QueueChanged);
        let mut t = transfer;
        t.state = TransferState::Cancelled;
        shared.emit(NodeEvent::Transfer(t));
        shared.unregister_transfer(&item.id);
        return;
    }

    let specs_to_verify = specs.clone();
    let verify_err = tokio::task::spawn_blocking(move || {
        for spec in &specs_to_verify {
            if let Err(reason) = spec.verify_integrity() {
                return Some(reason);
            }
        }
        None
    })
    .await
    .unwrap_or_else(|e| Some(e.to_string()));
    if let Some(reason) = verify_err {
        let _ = shared.store.mark_queue_failed(&item.id, &reason);
        shared.emit(NodeEvent::QueueChanged);
        let mut t = transfer;
        t.state = TransferState::Failed(TransferFailure::Other(reason));
        shared.emit(NodeEvent::Transfer(t));
        shared.unregister_transfer(&item.id);
        return;
    }

    let toggle = if recording { RECORDINGS_TOGGLE } else { TOGGLE };
    if !shared.toggle_on(&peer, toggle) {
        let reason = "File sharing is disabled for this device".to_owned();
        let _ = shared.store.mark_queue_failed(&item.id, &reason);
        shared.emit(NodeEvent::QueueChanged);
        let mut t = transfer;
        t.state = TransferState::Failed(TransferFailure::Denied);
        shared.emit(NodeEvent::Transfer(t));
        shared.unregister_transfer(&item.id);
        return;
    }

    let _ = shared.store.mark_queue_sending(&item.id);
    shared.emit(NodeEvent::QueueChanged);

    let mut opened = Vec::with_capacity(specs.len());
    let mut source_targets: Vec<String> = Vec::new();
    for spec in &specs {
        let path = &spec.path;
        if !spec.staged {
            let target = outgoing_target_path(path, spec.folder.as_deref());
            if !source_targets.contains(&target) {
                source_targets.push(target);
            }
        }
        let path_to_hash = path.clone();
        let name_for_err = spec.name.clone();
        let res = tokio::task::spawn_blocking(move || -> std::result::Result<(u64, String), String> {
            let handle = std::fs::File::open(&path_to_hash)
                .map_err(|_| format!("File was deleted before sending: {name_for_err}"))?;
            let (_handle, size, digest) =
                hash_whole_file(handle).map_err(|_| format!("Cannot read source file: {name_for_err}"))?;
            Ok((size, digest))
        })
        .await;
        let (size, digest) = match res {
            Ok(Ok(v)) => v,
            Ok(Err(reason)) => {
                let _ = shared.store.mark_queue_failed(&item.id, &reason);
                shared.emit(NodeEvent::QueueChanged);
                let mut t = transfer;
                t.state = TransferState::Failed(TransferFailure::Other(reason));
                shared.emit(NodeEvent::Transfer(t));
                shared.unregister_transfer(&item.id);
                return;
            }
            Err(_) => {
                let reason = format!("Cannot read source file: {}", spec.name);
                let _ = shared.store.mark_queue_failed(&item.id, &reason);
                shared.emit(NodeEvent::QueueChanged);
                let mut t = transfer;
                t.state = TransferState::Failed(TransferFailure::Other(reason));
                shared.emit(NodeEvent::Transfer(t));
                shared.unregister_transfer(&item.id);
                return;
            }
        };
        let entry = FileEntry {
            name: safe_file_name(&spec.name),
            size,
            folder: spec.folder.clone(),
            blake3: Some(digest),
        };
        opened.push((entry, OutgoingDataSource::Path(path.clone())));
    }

    let entries: Vec<FileEntry> = opened.iter().map(|(e, _)| e.clone()).collect();
    let tl_detail = if open_on_arrival {
        "Handoff document".into()
    } else {
        transfer_timeline_detail(&entries, recording, &markers)
    };
    let tl_target = source_targets.join("\n");
    let tl_kind = if recording { crate::TimelineKind::Recording } else { crate::TimelineKind::File };

    let mut reporter = Reporter::with_queue(shared.clone(), transfer, &file_sizes);
    let shutdown = shared.cancel.clone();
    let mut attempts = item.attempts;
    let mut furthest = 0;
    const RETRY_BACKOFF_MS: [i64; 5] = [1_000, 2_000, 4_000, 8_000, 15_000];

    let mut store_errors = 0u32;
    let state = loop {
        if cancel.is_cancelled() {
            break TransferState::Cancelled;
        }
        if shutdown.is_cancelled() {
            break TransferState::Failed(TransferFailure::Interrupted);
        }
        let current = match shared.store.get_queue_item(&item.id) {
            Ok(Some((current, _, _, _))) => current,
            // Removed from the queue: the user cancelled it.
            Ok(None) => break TransferState::Cancelled,
            // The database was busy for a moment; that's not a cancel.
            Err(e) if store_errors < 20 => {
                store_errors += 1;
                tracing::debug!(error = %e, "can't read the queue item yet");
                tokio::time::sleep(Duration::from_millis(250)).await;
                continue;
            }
            Err(e) => {
                break TransferState::Failed(TransferFailure::Other(format!(
                    "can't read the send queue: {e}"
                )));
            }
        };
        store_errors = 0;
        if let crate::QueueStatus::Failed { error } | crate::QueueStatus::Expired { error } = current.status {
            break TransferState::Failed(TransferFailure::Other(error));
        }

        while *pause_rx.borrow_and_update() {
            if reporter.transfer.state != TransferState::Paused {
                reporter.set(TransferState::Paused);
            }
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = shutdown.cancelled() => break,
                res = pause_rx.changed() => {
                    if res.is_err() {
                        break;
                    }
                }
            }
        }
        if cancel.is_cancelled() {
            break TransferState::Cancelled;
        }
        if shutdown.is_cancelled() {
            break TransferState::Failed(TransferFailure::Interrupted);
        }
        // Resumed: show it now, not once the receiver answers, so a pause
        // from the receiver before then still reads as a change.
        if reporter.transfer.state == TransferState::Paused {
            reporter.set(TransferState::Running);
        }

        let Some(session) = shared.session(&peer) else {
            let _ = shared.store.mark_queue_retry_or_wait(&item.id, 0, 0, None);
            let _ = shared.store.clear_send_queue_retry_delays(&peer);
            shared.emit(NodeEvent::QueueChanged);
            if reporter.transfer.state != TransferState::Waiting {
                reporter.set(TransferState::Waiting);
            }
            shared.mark_transfer_idle(&item.id);
            return;
        };

        if current.status != crate::QueueStatus::Sending {
            let _ = shared.store.mark_queue_sending(&item.id);
            shared.emit(NodeEvent::QueueChanged);
        }

        match attempt(&mut reporter, &session, &mut opened, &cancel, &mut pause_rx).await {
            Attempt::Done => break TransferState::Done { saved: Vec::new() },
            Attempt::Cancelled => break TransferState::Cancelled,
            Attempt::Paused => {
                if shared.transfer_paused_by_attempt(&item.id)
                    && reporter.transfer.state != TransferState::Paused
                {
                    reporter.set(TransferState::Paused);
                }
            }
            Attempt::Fatal(failure) => break TransferState::Failed(failure),
            Attempt::Retry => {
                if session.is_alive() {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                let peer_disconnected = !session.is_alive() || shared.session(&peer).is_none();
                if peer_disconnected {
                    let _ = shared.store.mark_queue_retry_or_wait(&item.id, 0, 0, None);
                    let _ = shared.store.clear_send_queue_retry_delays(&peer);
                    shared.emit(NodeEvent::QueueChanged);
                    if reporter.transfer.state != TransferState::Waiting {
                        reporter.set(TransferState::Waiting);
                    }
                    shared.mark_transfer_idle(&item.id);
                    return;
                } else {
                    // Only attempts that get nowhere count: on a poor link a big file
                    // can take many, each getting further.
                    if reporter.transfer.done > furthest {
                        furthest = reporter.transfer.done;
                        attempts = 0;
                    }
                    attempts += 1;
                    if attempts >= 5 {
                        break TransferState::Failed(TransferFailure::Other(
                            "Transfer failed after 5 attempts".into(),
                        ));
                    }
                    let delay_ms = RETRY_BACKOFF_MS[(attempts as usize - 1).min(RETRY_BACKOFF_MS.len() - 1)];
                    let next_retry = crate::now_unix_ms() + delay_ms;
                    let _ = shared.store.mark_queue_retry_or_wait(
                        &item.id,
                        attempts,
                        next_retry,
                        Some("Connection interrupted; retrying"),
                    );
                    shared.emit(NodeEvent::QueueChanged);
                    reporter.set(TransferState::Waiting);
                    let session_cancel = session.cancel.clone();
                    tokio::select! {
                        _ = cancel.cancelled() => break TransferState::Cancelled,
                        _ = shutdown.cancelled() => break TransferState::Failed(TransferFailure::Interrupted),
                        _ = session_cancel.cancelled() => {}
                        Ok(()) = pause_rx.changed() => {}
                        _ = tokio::time::sleep(Duration::from_millis(delay_ms as u64)) => {}
                    }
                }
            }
        }
    };

    match &state {
        TransferState::Done { .. } => {
            let _ = shared.record_timeline(crate::timeline::NewTimelineEntry {
                kind: tl_kind,
                device_id: peer,
                device_name: shared.peer_name(&peer),
                incoming: false,
                timestamp: crate::now_unix(),
                title: transfer_timeline_title(&reporter.transfer.names),
                detail: tl_detail,
                target: tl_target,
                size_bytes: reporter.transfer.total,
                duration_secs: 0,
                ref_id: Some(item.id.clone()),
            });
            let _ = shared.store.complete_queue_item(&item.id);
            cleanup_staging_id(shared, &item.id);
            shared.emit(NodeEvent::QueueChanged);
            reporter.set(state);
        }
        TransferState::Cancelled => {
            let _ = shared.store.remove_queue_item(&item.id);
            cleanup_staging_id(shared, &item.id);
            if let Some(session) = shared.session(&peer)
                && let Ok(env) = Envelope::new(types::FILES_CANCEL, &FilesControl { id: item.id.clone() })
            {
                let _ = session.send(env).await;
            }
            shared.emit(NodeEvent::QueueChanged);
            reporter.set(state);
        }
        TransferState::Failed(TransferFailure::Interrupted) => {
            // Keep in SQLite so next Node::start recovers it.
            reporter.set(state);
        }
        TransferState::Failed(failure) => {
            let reason = failure_reason_string(failure);
            let _ = shared.store.mark_queue_failed(&item.id, &reason);
            shared.emit(NodeEvent::QueueChanged);
            reporter.set(state);
        }
        _ => {
            reporter.set(state);
        }
    }
    shared.unregister_transfer(&item.id);
}

async fn send_inner(
    shared: &Arc<Shared>,
    peer: DeviceId,
    files: Vec<OutgoingFile>,
    recording: bool,
    is_photo: bool,
    markers: Vec<RecordingMarker>,
    open_on_arrival: bool,
) -> Result<String> {
    if files.is_empty() || files.len() > files::MAX_FILES || (recording && files.len() != 1) {
        return Err(Error::Protocol("send between 1 and 5000 files".into()));
    }
    if !shared.store.is_paired(&peer)? {
        if !recording && !is_photo && shared.localsend.has_peer(&peer) {
            return crate::localsend::send_files(shared, peer, files).await;
        }
        return Err(Error::NotPaired);
    }
    let toggle = if recording { RECORDINGS_TOGGLE } else { TOGGLE };
    if !shared.toggle_on(&peer, toggle) {
        return Err(Error::Denied);
    }
    let mut opened = Vec::with_capacity(files.len());
    let mut source_targets: Vec<String> = Vec::new();
    for file in files {
        if file.folder.as_deref().is_some_and(|f| !is_valid_folder(f)) {
            return Err(Error::Protocol("invalid folder name".into()));
        }
        let (size, digest, data_source) = match file.source {
            FileSource::Path(path) => {
                let target = outgoing_target_path(&path, file.folder.as_deref());
                if !source_targets.contains(&target) {
                    source_targets.push(target);
                }
                let path_to_hash = path.clone();
                let (size, digest) =
                    tokio::task::spawn_blocking(move || -> std::io::Result<(u64, String)> {
                        let handle = std::fs::File::open(&path_to_hash)?;
                        let (_handle, size, digest) = hash_whole_file(handle)?;
                        Ok((size, digest))
                    })
                    .await
                    .map_err(|e| Error::Internal(e.to_string()))??;
                (size, digest, OutgoingDataSource::Path(path))
            }
            FileSource::File(handle) => {
                let (handle, size, digest) = tokio::task::spawn_blocking(move || hash_whole_file(handle))
                    .await
                    .map_err(|e| Error::Internal(e.to_string()))??;
                (size, digest, OutgoingDataSource::File(tokio::fs::File::from_std(handle)))
            }
        };
        let entry =
            FileEntry { name: safe_file_name(&file.name), size, folder: file.folder, blake3: Some(digest) };
        opened.push((entry, data_source));
    }
    let entries: Vec<FileEntry> = opened.iter().map(|(entry, _)| entry.clone()).collect();
    let id = new_id();
    let open_on_arrival = open_on_arrival
        && !recording
        && entries.len() == 1
        && entries[0].folder.is_none()
        && crate::handoff::is_safe_handoff_document(&entries[0].name);
    // The whole offer has to fit in one frame.
    let offer = Envelope::new(
        files::OFFER,
        &FilesOffer {
            id: id.clone(),
            files: entries.clone(),
            recording,
            markers: markers.clone(),
            open_on_arrival,
        },
    )
    .map_err(|e| Error::Protocol(e.to_string()))?;
    if offer.to_cbor().len() > nectarlink_protocol::MAX_FRAME_LEN {
        return Err(Error::TooLarge);
    }
    let transfer = Transfer {
        id,
        device: peer,
        direction: Direction::Outgoing,
        names: items(&entries),
        files: entries.len(),
        total: entries.iter().map(|e| e.size).fold(0, u64::saturating_add),
        done: 0,
        state: TransferState::Waiting,
        recording,
        markers,
        open_on_arrival,
    };
    let id = transfer.id.clone();
    let tl_detail = if open_on_arrival {
        "Handoff document".into()
    } else {
        transfer_timeline_detail(&entries, recording, &transfer.markers)
    };
    let tl_target = source_targets.join("\n");
    let tl_kind = if recording {
        crate::TimelineKind::Recording
    } else if is_photo {
        crate::TimelineKind::Photo
    } else {
        crate::TimelineKind::File
    };
    // The user's cancel only: shutting down interrupts a transfer (so it
    // can resume later), it doesn't cancel it.
    let cancel = CancellationToken::new();
    let mut pause_rx = shared.register_transfer_control(&id, peer, Direction::Outgoing, cancel.clone());
    let mut reporter = Reporter::new(shared.clone(), transfer);
    reporter.set(TransferState::Waiting);
    tokio::spawn(async move {
        if is_photo {
            // Let the `photos.sending` reply go out on the control stream first
            // so the requester can tag the transfer ID before it finishes.
            tokio::task::yield_now().await;
        }
        let state = send_until_done(&mut reporter, opened, &cancel, &mut pause_rx).await;
        if matches!(state, TransferState::Done { .. }) {
            let _ = reporter.shared.record_timeline(crate::timeline::NewTimelineEntry {
                kind: tl_kind,
                device_id: peer,
                device_name: reporter.shared.peer_name(&peer),
                incoming: false,
                timestamp: crate::now_unix(),
                title: transfer_timeline_title(&reporter.transfer.names),
                detail: tl_detail,
                target: tl_target,
                size_bytes: reporter.transfer.total,
                duration_secs: 0,
                ref_id: Some(reporter.transfer.id.clone()),
            });
        }
        reporter.set(state);
        reporter.shared.unregister_transfer(&reporter.transfer.id);
    });
    Ok(id)
}

enum OutgoingDataSource {
    Path(PathBuf),
    File(tokio::fs::File),
}

enum ReadSource {
    Path(PathBuf),
    File(std::fs::File),
}

enum Attempt {
    Done,
    Cancelled,
    Paused,
    Fatal(TransferFailure),
    /// The connection failed; try again once it's back.
    Retry,
}

async fn send_until_done(
    reporter: &mut Reporter,
    mut files: Vec<(FileEntry, OutgoingDataSource)>,
    cancel: &CancellationToken,
    pause_rx: &mut tokio::sync::watch::Receiver<bool>,
) -> TransferState {
    let peer = reporter.transfer.device;
    let shutdown = reporter.shared.cancel.clone();
    let mut give_up_at = Instant::now() + RETRY_FOR;
    loop {
        if cancel.is_cancelled() {
            return TransferState::Cancelled;
        }
        if shutdown.is_cancelled() {
            return TransferState::Failed(TransferFailure::Interrupted);
        }
        while *pause_rx.borrow_and_update() {
            if reporter.transfer.state != TransferState::Paused {
                reporter.set(TransferState::Paused);
            }
            tokio::select! {
                _ = cancel.cancelled() => return TransferState::Cancelled,
                _ = shutdown.cancelled() => return TransferState::Failed(TransferFailure::Interrupted),
                res = pause_rx.changed() => {
                    if res.is_err() {
                        return TransferState::Cancelled;
                    }
                }
            }
            give_up_at = Instant::now() + RETRY_FOR;
        }
        // Resumed: as in the queue's loop above.
        if reporter.transfer.state == TransferState::Paused {
            reporter.set(TransferState::Running);
        }
        let Some(session) = reporter.shared.session(&peer) else {
            if Instant::now() >= give_up_at {
                return TransferState::Failed(TransferFailure::Unreachable);
            }
            if reporter.transfer.state != TransferState::Waiting {
                reporter.set(TransferState::Waiting);
            }
            tokio::select! {
                _ = cancel.cancelled() => return TransferState::Cancelled,
                _ = shutdown.cancelled() => return TransferState::Failed(TransferFailure::Interrupted),
                Ok(()) = pause_rx.changed() => continue,
                _ = tokio::time::sleep(Duration::from_secs(1)) => continue,
            }
        };
        let sent_before = reporter.transfer.done;
        match attempt(reporter, &session, &mut files, cancel, pause_rx).await {
            Attempt::Done => return TransferState::Done { saved: Vec::new() },
            Attempt::Cancelled => return TransferState::Cancelled,
            Attempt::Paused => {
                give_up_at = Instant::now() + RETRY_FOR;
                if reporter.shared.transfer_paused_by_attempt(&reporter.transfer.id)
                    && reporter.transfer.state != TransferState::Paused
                {
                    reporter.set(TransferState::Paused);
                }
            }
            Attempt::Fatal(failure) => return TransferState::Failed(failure),
            Attempt::Retry => {
                // Got further: give the device a fresh window to come back. A
                // device that keeps reconnecting without taking any bytes
                // doesn't keep the transfer going forever.
                if reporter.transfer.done > sent_before {
                    give_up_at = Instant::now() + RETRY_FOR;
                } else if Instant::now() >= give_up_at {
                    return TransferState::Failed(TransferFailure::Unreachable);
                }
                tracing::debug!(id = %reporter.transfer.id, "transfer interrupted; will resume");
                reporter.set(TransferState::Waiting);
                tokio::select! {
                    _ = cancel.cancelled() => return TransferState::Cancelled,
                    _ = shutdown.cancelled() => return TransferState::Failed(TransferFailure::Interrupted),
                    Ok(()) = pause_rx.changed() => {}
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                }
            }
        }
    }
}

async fn wait_paused(rx: &mut tokio::sync::watch::Receiver<bool>) {
    while !*rx.borrow_and_update() {
        if rx.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

async fn attempt(
    reporter: &mut Reporter,
    session: &Session,
    files: &mut [(FileEntry, OutgoingDataSource)],
    cancel: &CancellationToken,
    pause_rx: &mut tokio::sync::watch::Receiver<bool>,
) -> Attempt {
    let Ok((mut send, mut recv)) = session.conn.open_bi().await else { return Attempt::Retry };
    let header = StreamHeader { svc: files::SERVICE.into(), op: files::OP_SEND.into(), v: files::VERSION };
    let offer = FilesOffer {
        id: reporter.transfer.id.clone(),
        files: files.iter().map(|(entry, _)| entry.clone()).collect(),
        recording: reporter.transfer.recording,
        markers: reporter.transfer.markers.clone(),
        open_on_arrival: reporter.transfer.open_on_arrival,
    };
    let (Ok(header), Ok(offer)) =
        (Envelope::new(types::STREAM, &header), Envelope::new(files::OFFER, &offer))
    else {
        return Attempt::Fatal(TransferFailure::Other("can't encode the offer".into()));
    };
    if write_frame(&mut send, &header.to_cbor()).await.is_err()
        || write_frame(&mut send, &offer.to_cbor()).await.is_err()
    {
        return Attempt::Retry;
    }
    let have = match read_reply(&mut recv, cancel, pause_rx).await {
        Reply::Envelope(env) if env.t == files::ACCEPT => match env.body::<FilesAccept>() {
            Ok(accept) if accept.have.len() == files.len() => accept.have,
            _ => return Attempt::Fatal(TransferFailure::Other("bad answer to the offer".into())),
        },
        Reply::Envelope(env) => return Attempt::Fatal(refusal(&env)),
        Reply::Cancelled => {
            let _ = send.reset(VarInt::from_u32(files::CANCELLED));
            return Attempt::Cancelled;
        }
        Reply::Paused => {
            let _ = send.reset(VarInt::from_u32(files::PAUSED));
            let _ = recv.stop(VarInt::from_u32(files::PAUSED));
            return Attempt::Paused;
        }
        Reply::Failed => return Attempt::Retry,
    };

    let mut done: u64 = files.iter().zip(&have).map(|((entry, _), have)| (*have).min(entry.size)).sum();
    reporter.transfer.done = done;
    if reporter.update_queue && done > 0 {
        let files_done = reporter.files_completed(done);
        let _ = reporter.shared.store.update_queue_progress(&reporter.transfer.id, done, files_done);
        reporter.shared.emit(NodeEvent::QueueChanged);
    }
    reporter.set(TransferState::Running);
    // What's left of each file, read on a thread of its own that keeps a
    // few blocks ready; it stops when this attempt ends and drops them.
    let mut plan = Vec::with_capacity(files.len());
    for ((entry, source), have) in files.iter().zip(&have) {
        let start = (*have).min(entry.size);
        let read_source = match source {
            OutgoingDataSource::Path(path) => ReadSource::Path(path.clone()),
            OutgoingDataSource::File(file) => {
                let Ok(clone) = file.try_clone().await else {
                    return Attempt::Fatal(TransferFailure::Other("can't read a file".into()));
                };
                ReadSource::File(clone.into_std().await)
            }
        };
        plan.push((read_source, start, entry.size - start));
    }
    let (blocks, mut ready) = tokio::sync::mpsc::channel(READ_AHEAD);
    tokio::task::spawn_blocking(move || read_ahead(plan, &blocks));
    loop {
        if *pause_rx.borrow_and_update() {
            let _ = send.reset(VarInt::from_u32(files::PAUSED));
            let _ = recv.stop(VarInt::from_u32(files::PAUSED));
            return Attempt::Paused;
        }
        let block = tokio::select! {
            _ = cancel.cancelled() => {
                let _ = send.reset(VarInt::from_u32(files::CANCELLED));
                return Attempt::Cancelled;
            }
            _ = wait_paused(pause_rx) => {
                let _ = send.reset(VarInt::from_u32(files::PAUSED));
                let _ = recv.stop(VarInt::from_u32(files::PAUSED));
                return Attempt::Paused;
            }
            block = ready.recv() => block,
        };
        let block = match block {
            None => break,
            Some(Ok(block)) => block,
            Some(Err(failure)) => return Attempt::Fatal(failure),
        };
        let n = block.len() as u64;
        let written = tokio::select! {
            _ = cancel.cancelled() => {
                let _ = send.reset(VarInt::from_u32(files::CANCELLED));
                return Attempt::Cancelled;
            }
            _ = wait_paused(pause_rx) => {
                let _ = send.reset(VarInt::from_u32(files::PAUSED));
                let _ = recv.stop(VarInt::from_u32(files::PAUSED));
                return Attempt::Paused;
            }
            written = send.write_chunk(block) => written,
        };
        if let Err(e) = written {
            return match e {
                WriteError::Stopped(code) => stopped(code),
                _ => Attempt::Retry,
            };
        }
        done += n;
        reporter.progress(done);
    }
    if *pause_rx.borrow_and_update() {
        let _ = send.reset(VarInt::from_u32(files::PAUSED));
        let _ = recv.stop(VarInt::from_u32(files::PAUSED));
        return Attempt::Paused;
    }
    if send.finish().is_err() {
        return Attempt::Retry;
    }
    // Written isn't sent: QUIC holds megabytes, which take minutes on a slow link.
    // The receiver's answer can only be due once it has them. A connection
    // that dies ends this wait, as does the receiver stopping the stream.
    tokio::select! {
        _ = cancel.cancelled() => {
            let _ = send.reset(VarInt::from_u32(files::CANCELLED));
            return Attempt::Cancelled;
        }
        _ = wait_paused(pause_rx) => {
            let _ = send.reset(VarInt::from_u32(files::PAUSED));
            let _ = recv.stop(VarInt::from_u32(files::PAUSED));
            return Attempt::Paused;
        }
        _ = send.stopped() => {}
    }
    match read_reply(&mut recv, cancel, pause_rx).await {
        Reply::Envelope(env) if env.t == files::DONE => Attempt::Done,
        Reply::Envelope(env) => Attempt::Fatal(refusal(&env)),
        Reply::Cancelled => Attempt::Cancelled,
        Reply::Paused => {
            let _ = send.reset(VarInt::from_u32(files::PAUSED));
            let _ = recv.stop(VarInt::from_u32(files::PAUSED));
            Attempt::Paused
        }
        Reply::Failed => Attempt::Retry,
    }
}

/// What a stream reset with `code` by the receiver means.
fn stopped(code: VarInt) -> Attempt {
    match u32::try_from(code.into_inner()).unwrap_or(u32::MAX) {
        files::CANCELLED => Attempt::Cancelled,
        files::PAUSED => Attempt::Paused,
        c if c == ErrorCode::Busy.close_code() => Attempt::Fatal(TransferFailure::NoSpace),
        c if c == ErrorCode::Denied.close_code() => Attempt::Fatal(TransferFailure::Denied),
        _ => Attempt::Retry,
    }
}

fn refusal(env: &Envelope) -> TransferFailure {
    match env.body::<ErrorBody>() {
        Ok(ErrorBody { code: ErrorCode::Denied, .. }) => TransferFailure::Denied,
        Ok(ErrorBody { code: ErrorCode::Busy, .. }) => TransferFailure::NoSpace,
        Ok(ErrorBody { code: ErrorCode::BadMessage, msg: Some(m) })
            if m.to_ascii_lowercase().contains("integrity") =>
        {
            TransferFailure::Integrity
        }
        Ok(ErrorBody { code, .. }) => TransferFailure::Other(format!("refused: {code}")),
        Err(_) => TransferFailure::Other(format!("unexpected {}", env.t)),
    }
}

enum Reply {
    Envelope(Envelope),
    Cancelled,
    Paused,
    Failed,
}

async fn read_reply(
    recv: &mut RecvStream,
    cancel: &CancellationToken,
    pause_rx: &mut tokio::sync::watch::Receiver<bool>,
) -> Reply {
    if *pause_rx.borrow_and_update() {
        return Reply::Paused;
    }
    let frame = tokio::select! {
        _ = cancel.cancelled() => return Reply::Cancelled,
        _ = wait_paused(pause_rx) => return Reply::Paused,
        frame = tokio::time::timeout(REPLY_TIMEOUT, read_frame(recv)) => frame,
    };
    match frame {
        Ok(Ok(Some(bytes))) => Envelope::from_cbor(&bytes).map_or(Reply::Failed, Reply::Envelope),
        Ok(Err(nectarlink_protocol::ProtocolError::Io(e))) if is_cancel(&e) => Reply::Cancelled,
        Ok(Err(nectarlink_protocol::ProtocolError::Io(e))) if is_pause(&e) => Reply::Paused,
        _ => Reply::Failed,
    }
}

/// Whether an I/O error from a stream is the other side cancelling.
fn is_cancel(e: &std::io::Error) -> bool {
    e.get_ref().and_then(|inner| inner.downcast_ref::<ReadError>()).is_some_and(
        |r| matches!(r, ReadError::Reset(code) if code.into_inner() == u64::from(files::CANCELLED)),
    )
}

/// Whether an I/O error from a stream is the other side pausing.
fn is_pause(e: &std::io::Error) -> bool {
    e.get_ref()
        .and_then(|inner| inner.downcast_ref::<ReadError>())
        .is_some_and(|r| matches!(r, ReadError::Reset(code) if code.into_inner() == u64::from(files::PAUSED)))
}

/// Handles `files.resume` and `files.cancel` control messages.
pub(crate) async fn handle_control(shared: &Arc<Shared>, peer: DeviceId, env: &Envelope) -> Result<bool> {
    match env.t.as_str() {
        types::FILES_RESUME => {
            let ctrl: FilesControl = env.body()?;
            if !ctrl.is_valid() {
                return Err(Error::Protocol("invalid files.resume".into()));
            }
            shared.resume_transfer_from_peer(&ctrl.id, peer);
            Ok(true)
        }
        types::FILES_CANCEL => {
            let ctrl: FilesControl = env.body()?;
            if !ctrl.is_valid() {
                return Err(Error::Protocol("invalid files.cancel".into()));
            }
            shared.cancel_transfer_from_peer(&ctrl.id, peer).await;
            Ok(true)
        }
        _ => Ok(false),
    }
}

// ---- Receiving ----

/// Handles a stream the other device opened (its header not yet read).
pub(crate) async fn accept_stream(
    shared: Arc<Shared>,
    session: Arc<Session>,
    mut send: SendStream,
    mut recv: RecvStream,
) {
    let header = match tokio::time::timeout(REPLY_TIMEOUT, read_frame(&mut recv)).await {
        Ok(Ok(Some(bytes))) => {
            Envelope::from_cbor(&bytes).ok().and_then(|e| e.expect_body::<StreamHeader>(types::STREAM).ok())
        }
        _ => None,
    };
    match header {
        Some(h) if h.svc == files::SERVICE && h.op == files::OP_SEND && h.v == files::VERSION => {
            receive(shared, session.peer, send, recv).await;
        }
        Some(h) if h.svc == clip::SERVICE && h.op == clip::OP_IMAGE && h.v == clip::VERSION => {
            crate::clipboard::receive_image(shared, session.peer, send, recv).await;
        }
        Some(h) if h.svc == mirror::SERVICE && h.op == mirror::OP_VIDEO && h.v == mirror::VERSION => {
            crate::mirror::receive(shared, session.peer, send, recv).await;
        }
        Some(h) if h.svc == mirror::SERVICE && h.op == mirror::OP_AUDIO && h.v == mirror::VERSION => {
            crate::mirror::receive_audio(shared, session.peer, send, recv).await;
        }
        Some(h)
            if h.svc == nectarlink_protocol::messages::media_watch::SERVICE
                && h.op == nectarlink_protocol::messages::media_watch::OP_FILE
                && h.v == nectarlink_protocol::messages::media_watch::VERSION =>
        {
            crate::media::serve_file(shared, session.peer, send, recv).await;
        }
        Some(h)
            if h.svc == nectarlink_protocol::messages::mic::SERVICE
                && h.op == nectarlink_protocol::messages::mic::OP_PCM
                && h.v == nectarlink_protocol::messages::mic::VERSION =>
        {
            crate::mic::receive(shared, session.peer, send, recv).await;
        }
        Some(h)
            if h.svc == nectarlink_protocol::messages::webcam::SERVICE
                && h.op == nectarlink_protocol::messages::webcam::OP_VIDEO
                && h.v == nectarlink_protocol::messages::webcam::VERSION =>
        {
            crate::webcam::receive(shared, session.peer, send, recv).await;
        }
        Some(h)
            if h.svc == nectarlink_protocol::messages::continuity_camera::SERVICE
                && h.op == nectarlink_protocol::messages::continuity_camera::OP_RESULT
                && h.v == nectarlink_protocol::messages::continuity_camera::VERSION =>
        {
            crate::continuity_camera::receive_result(shared, session.peer, send, recv).await;
        }
        Some(h)
            if h.svc == nectarlink_protocol::messages::remote::SERVICE
                && h.op == nectarlink_protocol::messages::remote::OP_MOTION
                && h.v == 1 =>
        {
            crate::remote::receive_motion(shared, session.peer, send, recv).await;
        }
        Some(h)
            if h.svc == nectarlink_protocol::messages::storage::SERVICE
                && h.op == nectarlink_protocol::messages::storage::OP_READ
                && h.v == nectarlink_protocol::messages::storage::VERSION =>
        {
            crate::storage::serve_read(shared, session.peer, send, recv).await;
        }
        Some(h)
            if h.svc == nectarlink_protocol::messages::storage::SERVICE
                && h.op == nectarlink_protocol::messages::storage::OP_WRITE
                && h.v == nectarlink_protocol::messages::storage::VERSION =>
        {
            crate::storage::serve_write(shared, session.peer, send, recv).await;
        }
        _ => {
            let reply = Envelope::error(ErrorCode::Unsupported, "unknown stream");
            let _ = write_frame(&mut send, &reply.to_cbor()).await;
            let _ = send.finish();
        }
    }
}

async fn refuse(send: &mut SendStream, code: ErrorCode, msg: &str) {
    let _ = write_frame(send, &Envelope::error(code, msg).to_cbor()).await;
    let _ = send.finish();
}

/// A file's size and BLAKE3 digest (hex), read from the start; the file is
/// handed back rewound, ready to send.
pub(crate) fn hash_whole_file(mut f: std::fs::File) -> std::io::Result<(std::fs::File, u64, String)> {
    use std::io::{Seek, SeekFrom};
    let size = f.metadata()?.len();
    f.seek(SeekFrom::Start(0))?;
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(&mut f)?;
    f.seek(SeekFrom::Start(0))?;
    Ok((f, size, hasher.finalize().to_hex().to_string()))
}

fn hash_file_prefix(path: &Path, mut left: u64) -> std::io::Result<blake3::Hasher> {
    use std::io::Read;
    let mut hasher = blake3::Hasher::new();
    if left == 0 {
        return Ok(hasher);
    }
    let mut f = std::fs::File::open(path)?;
    let mut buf = vec![0u8; 64 * 1024];
    while left > 0 {
        let to_read = buf.len().min(usize::try_from(left).unwrap_or(usize::MAX));
        let n = f.read(&mut buf[..to_read])?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "part file shorter than expected",
            ));
        }
        hasher.update(&buf[..n]);
        left -= n as u64;
    }
    Ok(hasher)
}

/// Holds an incoming transfer's turn on its part files (`Shared::incoming_turn`).
pub(crate) struct IncomingTurn {
    pub(crate) shared: Arc<Shared>,
    pub(crate) id: String,
    /// Fires when a newer attempt at this transfer arrives.
    pub(crate) stop: CancellationToken,
    pub(crate) held: Option<tokio::sync::OwnedMutexGuard<()>>,
}

impl Drop for IncomingTurn {
    fn drop(&mut self) {
        let mut turns = self.shared.incoming_turns.lock().unwrap_or_else(|e| e.into_inner());
        self.held = None;
        // Nobody else holds or waits for it: the entry can go.
        if turns.get(&self.id).is_some_and(|(lock, _)| Arc::strong_count(lock) == 1) {
            turns.remove(&self.id);
        }
    }
}

async fn receive(shared: Arc<Shared>, peer: DeviceId, mut send: SendStream, mut recv: RecvStream) {
    let offer = match tokio::time::timeout(REPLY_TIMEOUT, read_frame(&mut recv)).await {
        Ok(Ok(Some(bytes))) => {
            Envelope::from_cbor(&bytes).ok().and_then(|e| e.expect_body::<FilesOffer>(files::OFFER).ok())
        }
        _ => None,
    };
    let Some(offer) = offer.filter(FilesOffer::is_valid) else {
        return refuse(&mut send, ErrorCode::BadMessage, "invalid offer").await;
    };
    if offer.recording {
        if !shared.we_offer(RECORDER) {
            return refuse(&mut send, ErrorCode::Unsupported, "recordings aren't supported").await;
        }
        if !shared.toggle_on(&peer, RECORDINGS_TOGGLE) {
            return refuse(&mut send, ErrorCode::Denied, "recordings are off for this device").await;
        }
    } else if !shared.toggle_on(&peer, TOGGLE) {
        return refuse(&mut send, ErrorCode::Denied, "files are off for this device").await;
    }
    let dir = shared.incoming_dir().join(peer.to_string()).join(&offer.id);
    if let Err(e) = tokio::fs::create_dir_all(&dir).await {
        tracing::warn!(error = %e, "can't store incoming files");
        return refuse(&mut send, ErrorCode::Internal, "can't store files").await;
    }
    let part = |i: usize| dir.join(format!("{i}.part"));
    // Before looking at what's on disk: the attempt before this one may still be writing.
    let turn = shared.incoming_turn(&offer.id).await;
    let mut have = Vec::with_capacity(offer.files.len());
    for (i, file) in offer.files.iter().enumerate() {
        let p = part(i);
        let len = tokio::fs::metadata(&p).await.map(|m| m.len()).unwrap_or(0);
        let valid_len = if len == file.size && file.size > 0 {
            if let Some(expected) = file.blake3.clone() {
                let matches = tokio::task::spawn_blocking(move || {
                    hash_file_prefix(&p, len).map(|h| h.finalize().to_hex().to_string())
                })
                .await
                .ok()
                .and_then(std::result::Result::ok)
                .is_some_and(|actual| actual.eq_ignore_ascii_case(&expected));
                if matches { file.size } else { 0 }
            } else {
                file.size
            }
        } else {
            len.min(file.size)
        };
        have.push(valid_len);
    }
    // What's still to come has to fit where it's received and where it's
    // saved; a transfer that runs out of room partway helps nobody. The
    // sender reads Busy as "not enough space" (TransferFailure::NoSpace).
    let to_come = offer.total_size().saturating_sub(have.iter().sum());
    let fits = |at: &Path| free_space(at).is_none_or(|free| free >= to_come.saturating_add(SPACE_MARGIN));
    if !fits(&dir) || !fits(&shared.downloads_dir) {
        tracing::warn!(bytes = to_come, "not enough space for incoming files");
        return refuse(&mut send, ErrorCode::Busy, "not enough space").await;
    }
    let Ok(accept) = Envelope::new(files::ACCEPT, &FilesAccept { have: have.clone() }) else { return };
    if write_frame(&mut send, &accept.to_cbor()).await.is_err() {
        return;
    }

    let cancel = CancellationToken::new();
    let mut pause_rx = shared.register_transfer_control(&offer.id, peer, Direction::Incoming, cancel.clone());
    let open_on_arrival = offer.open_on_arrival
        && !offer.recording
        && offer.files.len() == 1
        && offer.files[0].folder.is_none()
        && crate::handoff::is_safe_handoff_document(&offer.files[0].name);
    let transfer = Transfer {
        id: offer.id.clone(),
        device: peer,
        direction: Direction::Incoming,
        names: items(&offer.files),
        files: offer.files.len(),
        total: offer.total_size(),
        done: have.iter().sum(),
        state: TransferState::Running,
        recording: offer.recording,
        markers: offer.markers.clone(),
        open_on_arrival,
    };
    let mut reporter = Reporter::new(shared.clone(), transfer);
    reporter.set(TransferState::Running);
    let state = match receive_bytes(
        &mut reporter,
        &offer,
        &have,
        &part,
        &mut send,
        &mut recv,
        &cancel,
        &mut pause_rx,
        &turn.stop,
    )
    .await
    {
        // Paused just as the last bytes came in: tell the sender, which is
        // waiting for the confirmation, as any other pause does.
        Ok(()) if *pause_rx.borrow() => {
            let _ = recv.stop(VarInt::from_u32(files::PAUSED));
            let _ = send.reset(VarInt::from_u32(files::PAUSED));
            TransferState::Paused
        }
        Ok(()) => match store(&shared, &offer, &part, &dir).await {
            Ok(saved) => {
                let _ = write_frame(&mut send, &Envelope::empty(files::DONE).to_cbor()).await;
                let _ = send.finish();
                // Let the confirmation reach the sender before the stream goes.
                let _ = tokio::time::timeout(Duration::from_secs(2), send.stopped()).await;
                let kind = if offer.recording {
                    crate::TimelineKind::Recording
                } else if take_photo_transfer(&offer.id) {
                    crate::TimelineKind::Photo
                } else {
                    crate::TimelineKind::File
                };
                let detail = if kind == crate::TimelineKind::Photo {
                    match offer.files.len() {
                        1 => "Photo".into(),
                        n => format!("{n} photos"),
                    }
                } else if open_on_arrival {
                    "Handoff document".into()
                } else {
                    transfer_timeline_detail(&offer.files, offer.recording, &offer.markers)
                };
                let target =
                    saved.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>().join("\n");
                if !take_quiet_transfer(&offer.id) {
                    let _ = shared.record_timeline(crate::timeline::NewTimelineEntry {
                        kind,
                        device_id: peer,
                        device_name: shared.peer_name(&peer),
                        incoming: true,
                        timestamp: crate::now_unix(),
                        title: transfer_timeline_title(&reporter.transfer.names),
                        detail,
                        target,
                        size_bytes: reporter.transfer.total,
                        duration_secs: 0,
                        ref_id: Some(offer.id.clone()),
                    });
                }
                TransferState::Done { saved }
            }
            Err(e) => {
                tracing::warn!(error = %e, "can't save received files");
                refuse(&mut send, ErrorCode::Internal, "can't save files").await;
                TransferState::Failed(TransferFailure::Other(format!("can't save: {}", e.kind())))
            }
        },
        Err(state) => {
            if state == TransferState::Cancelled {
                let _ = take_photo_transfer(&offer.id);
                let _ = take_quiet_transfer(&offer.id);
                let _ = tokio::fs::remove_dir_all(&dir).await;
                if let Some(parent) = dir.parent() {
                    let _ = tokio::fs::remove_dir(parent).await;
                }
            }
            state
        }
    };
    if state == TransferState::Paused {
        reporter.transfer.state = TransferState::Paused;
        shared.mark_transfer_paused(&offer.id, reporter.transfer.clone());
        reporter.set(TransferState::Paused);
        return;
    }
    reporter.set(state);
    shared.unregister_transfer(&offer.id);
}

#[allow(clippy::too_many_arguments)]
/// Reads each file's remaining bytes, from where the receiver is, in
/// blocks; stops early when the sending side is gone.
fn read_ahead(
    plan: Vec<(ReadSource, u64, u64)>,
    blocks: &tokio::sync::mpsc::Sender<std::result::Result<Bytes, TransferFailure>>,
) {
    for (source, mut at, mut left) in plan {
        // Read by position (never through a shared cursor): an older attempt's
        // reader may still hold the same open file.
        let file = match source {
            ReadSource::Path(path) => match std::fs::File::open(&path) {
                Ok(f) => f,
                Err(e) => {
                    let failure = TransferFailure::Other(format!("can't read a file: {}", e.kind()));
                    let _ = blocks.blocking_send(Err(failure));
                    return;
                }
            },
            ReadSource::File(f) => f,
        };
        while left > 0 {
            let mut block = vec![0u8; BLOCK.min(usize::try_from(left).unwrap_or(usize::MAX))];
            if let Err(e) = read_exact_at(&file, &mut block, at) {
                let failure = if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    TransferFailure::Other("a file got shorter while sending".into())
                } else {
                    TransferFailure::Other(format!("can't read a file: {}", e.kind()))
                };
                let _ = blocks.blocking_send(Err(failure));
                return;
            }
            at += block.len() as u64;
            left -= block.len() as u64;
            if blocks.blocking_send(Ok(Bytes::from(block))).is_err() {
                return;
            }
        }
    }
}

/// Fills `block` from `at`, whatever the file's position is. Handles from
/// `try_clone` share one position, and a reader from an attempt that just ended
/// can still be going: moving it under the next one would send the wrong bytes.
fn read_exact_at(file: &std::fs::File, block: &mut [u8], at: u64) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::FileExt::read_exact_at(file, block, at)
    }
    #[cfg(windows)]
    {
        let mut filled = 0;
        while filled < block.len() {
            let offset = at + filled as u64;
            match std::os::windows::fs::FileExt::seek_read(file, &mut block[filled..], offset) {
                Ok(0) => return Err(std::io::ErrorKind::UnexpectedEof.into()),
                Ok(n) => filled += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
async fn receive_bytes(
    reporter: &mut Reporter,
    offer: &FilesOffer,
    have: &[u64],
    part: &impl Fn(usize) -> PathBuf,
    send: &mut SendStream,
    recv: &mut RecvStream,
    cancel: &CancellationToken,
    pause_rx: &mut tokio::sync::watch::Receiver<bool>,
    superseded: &CancellationToken,
) -> std::result::Result<(), TransferState> {
    let mut done: u64 = have.iter().sum();
    for (i, (file, have)) in offer.files.iter().zip(have).enumerate() {
        let part_path = part(i);
        let mut out =
            tokio::fs::OpenOptions::new().create(true).append(true).open(&part_path).await.map_err(|e| {
                TransferState::Failed(TransferFailure::Other(format!("can't write: {}", e.kind())))
            })?;
        // A part file longer than `have` (it changed or failed a prior check) starts from `have`.
        if *have < out.metadata().await.map(|m| m.len()).unwrap_or(0) {
            // Appending after leftover bytes would put this file's data in the wrong place.
            out.set_len(*have).await.map_err(|e| {
                TransferState::Failed(TransferFailure::Other(format!("can't truncate: {}", e.kind())))
            })?;
        }
        let mut hasher = if file.blake3.is_some() {
            let have_bytes = *have;
            let p = part_path.clone();
            let seeded = tokio::task::spawn_blocking(move || hash_file_prefix(&p, have_bytes))
                .await
                .map_err(|e| TransferState::Failed(TransferFailure::Other(e.to_string())))?
                .map_err(|e| {
                    TransferState::Failed(TransferFailure::Other(format!("can't read part: {}", e.kind())))
                })?;
            Some(seeded)
        } else {
            None
        };
        // Gathered in memory and written in large pieces. Progress counts
        // what's on disk, which is also where a resumed transfer continues.
        let mut pending: Vec<u8> = Vec::with_capacity(WRITE_BUFFER);
        let mut left = file.size - have;
        while left > 0 {
            if *pause_rx.borrow_and_update() {
                if write_out(&mut out, &mut pending, recv).await.is_ok() {
                    reporter.progress(done);
                }
                let _ = recv.stop(VarInt::from_u32(files::PAUSED));
                let _ = send.reset(VarInt::from_u32(files::PAUSED));
                return Err(TransferState::Paused);
            }
            let max = CHUNK.min(usize::try_from(left).unwrap_or(usize::MAX));
            let chunk = tokio::select! {
                _ = cancel.cancelled() => {
                    let _ = recv.stop(VarInt::from_u32(files::CANCELLED));
                    let _ = send.reset(VarInt::from_u32(files::CANCELLED));
                    return Err(TransferState::Cancelled);
                }
                // A newer attempt at this transfer is waiting for the part file.
                _ = superseded.cancelled() => {
                    if write_out(&mut out, &mut pending, recv).await.is_ok() {
                        reporter.progress(done);
                    }
                    return Err(TransferState::Waiting);
                }
                _ = wait_paused(pause_rx) => {
                    if write_out(&mut out, &mut pending, recv).await.is_ok() {
                        reporter.progress(done);
                    }
                    let _ = recv.stop(VarInt::from_u32(files::PAUSED));
                    let _ = send.reset(VarInt::from_u32(files::PAUSED));
                    return Err(TransferState::Paused);
                }
                chunk = recv.read_chunk(max) => chunk,
            };
            let bytes = match chunk {
                Ok(Some(bytes)) => bytes,
                Err(ReadError::Reset(code)) if code.into_inner() == u64::from(files::CANCELLED) => {
                    return Err(TransferState::Cancelled);
                }
                Err(ReadError::Reset(code)) if code.into_inner() == u64::from(files::PAUSED) => {
                    if write_out(&mut out, &mut pending, recv).await.is_ok() {
                        reporter.progress(done);
                    }
                    return Err(TransferState::Paused);
                }
                // The stream or connection ended early: keep what arrived;
                // the sender resumes from there when it's back.
                Ok(None) | Err(_) => {
                    if write_out(&mut out, &mut pending, recv).await.is_ok() {
                        reporter.progress(done);
                    }
                    return Err(TransferState::Waiting);
                }
            };
            if let Some(ref mut h) = hasher {
                h.update(&bytes);
            }
            left -= bytes.len() as u64;
            done += bytes.len() as u64;
            pending.extend_from_slice(&bytes);
            if pending.len() >= WRITE_BUFFER || left == 0 {
                write_out(&mut out, &mut pending, recv).await?;
                reporter.progress(done);
            }
        }
        if let (Some(expected), Some(h)) = (file.blake3.as_deref(), hasher.take()) {
            let actual = h.finalize().to_hex();
            if !actual.as_str().eq_ignore_ascii_case(expected) {
                refuse(send, ErrorCode::BadMessage, "blake3 integrity check failed").await;
                return Err(TransferState::Failed(TransferFailure::Integrity));
            }
            // A resumed file was hashed partly from disk and partly as it
            // arrived: check what's actually there before it's saved.
            if *have > 0 {
                let (p, size, want) = (part_path.clone(), file.size, expected.to_owned());
                let on_disk = tokio::task::spawn_blocking(move || {
                    hash_file_prefix(&p, size).map(|h| h.finalize().to_hex().to_string())
                })
                .await
                .ok()
                .and_then(std::result::Result::ok)
                .is_some_and(|actual| actual.eq_ignore_ascii_case(&want));
                if !on_disk {
                    refuse(send, ErrorCode::BadMessage, "blake3 integrity check failed").await;
                    return Err(TransferState::Failed(TransferFailure::Integrity));
                }
            }
        }
    }
    Ok(())
}

/// Writes what's gathered; a failure stops the stream and says why.
async fn write_out(
    out: &mut tokio::fs::File,
    pending: &mut Vec<u8>,
    recv: &mut RecvStream,
) -> std::result::Result<(), TransferState> {
    let written = out.write_all(pending).await;
    let written = match written {
        Ok(()) => out.flush().await,
        Err(e) => Err(e),
    };
    pending.clear();
    written.map_err(|e| {
        let no_space = e.kind() == std::io::ErrorKind::StorageFull;
        let code = if no_space { ErrorCode::Busy } else { ErrorCode::Internal };
        let _ = recv.stop(VarInt::from_u32(code.close_code()));
        TransferState::Failed(if no_space {
            TransferFailure::NoSpace
        } else {
            TransferFailure::Other(format!("can't write: {}", e.kind()))
        })
    })
}

/// Moves complete files where the user finds them; returns their paths.
async fn store(
    shared: &Shared,
    offer: &FilesOffer,
    part: &impl Fn(usize) -> PathBuf,
    dir: &Path,
) -> std::io::Result<Vec<PathBuf>> {
    let target = shared.downloads_dir.clone();
    let entries = offer.files.clone();
    let parts: Vec<PathBuf> = (0..entries.len()).map(part).collect();
    let dir = dir.to_owned();
    tokio::task::spawn_blocking(move || {
        let saved = save_files(&target, &entries, parts)?;
        let _ = std::fs::remove_dir_all(&dir);
        if let Some(parent) = dir.parent() {
            let _ = std::fs::remove_dir(parent);
        }
        Ok(saved)
    })
    .await
    .map_err(std::io::Error::other)?
}

/// Moves the received part files into `target`, under their own names.
/// When one can't be saved, the ones already moved go back to their part
/// files: the sender hears it failed, and a retry resumes instead of leaving
/// a second copy of each.
fn save_files(target: &Path, entries: &[FileEntry], parts: Vec<PathBuf>) -> std::io::Result<Vec<PathBuf>> {
    // One at a time: two transfers saving `photo.jpg` together would both
    // find the same free name, and the second would replace the first.
    static SAVING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _saving = SAVING.lock().unwrap_or_else(|e| e.into_inner());
    std::fs::create_dir_all(target)?;
    // A sent folder lands under a name of its own: `Trip`, or
    // `Trip (2)` when there already is one.
    let mut folders: HashMap<String, PathBuf> = HashMap::new();
    let mut saved = Vec::with_capacity(entries.len());
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    let result = (|| {
        for (entry, part) in entries.iter().zip(parts) {
            let at = match entry.folder.as_deref() {
                None => target.to_path_buf(),
                Some(folder) => {
                    let mut names = folder.split('/').map(storable_name);
                    let top = names.next().unwrap_or_else(|| "Folder".into());
                    let mut at = match folders.get(&top) {
                        Some(at) => at.clone(),
                        None => {
                            let at = unique_path(target, &top);
                            std::fs::create_dir(&at)?;
                            folders.insert(top, at.clone());
                            saved.push(at.clone());
                            at
                        }
                    };
                    at.extend(names);
                    std::fs::create_dir_all(&at)?;
                    at
                }
            };
            let to = unique_path(&at, &storable_name(&entry.name));
            move_file(&part, &to)?;
            mark_from_elsewhere(&to);
            moved.push((to.clone(), part));
            if entry.folder.is_none() {
                saved.push(to);
            }
        }
        Ok(())
    })();
    if let Err(e) = result {
        for (to, part) in moved.iter().rev() {
            if let Err(e) = move_file(to, part) {
                tracing::warn!(error = %e, "can't put a received file back");
            }
        }
        // Folders made here hold nothing of anyone else's.
        for folder in folders.values() {
            let _ = std::fs::remove_dir_all(folder);
        }
        return Err(e);
    }
    Ok(saved)
}

/// Marks a received file as coming from another device, the way a browser
/// marks a download: on Windows that's the Internet zone (Mark of the Web),
/// so SmartScreen checks a program before it runs and Office opens a
/// document in Protected View. Best effort: a drive without alternate data
/// streams (FAT32, say) takes no mark.
pub(crate) fn mark_from_elsewhere(path: &Path) {
    #[cfg(windows)]
    {
        let mut stream = path.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        if let Err(e) = std::fs::write(&stream, "[ZoneTransfer]\r\nZoneId=3\r\n") {
            tracing::debug!(error = %e, "can't mark a received file as from elsewhere");
        }
    }
    #[cfg(not(windows))]
    let _ = path;
}

/// Room kept free beyond an incoming transfer's size.
pub(crate) const SPACE_MARGIN: u64 = 64 * 1024 * 1024;

/// Bytes free for this user on the drive holding `path` (`None` when it
/// can't tell, which never blocks a transfer).
#[allow(unsafe_code)]
pub(crate) fn free_space(path: &Path) -> Option<u64> {
    #[cfg(windows)]
    {
        use windows::{Win32::Storage::FileSystem::GetDiskFreeSpaceExW, core::HSTRING};
        let mut free = 0u64;
        // SAFETY: a valid path string and an out pointer to a local.
        unsafe { GetDiskFreeSpaceExW(&HSTRING::from(path.as_os_str()), Some(&mut free), None, None) }
            .ok()
            .map(|()| free)
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
        // SAFETY: statvfs is plain data; zeroed is a valid value for it.
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: `c_path` is NUL-terminated and `stat` is writable.
        if unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) } != 0 {
            return None;
        }
        let block = if stat.f_frsize > 0 { stat.f_frsize as u64 } else { stat.f_bsize as u64 };
        Some((stat.f_bavail as u64).saturating_mul(block))
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = path;
        None
    }
}

/// Moves `from` to `to`, copying when they're on different volumes.
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if std::fs::rename(from, to).is_err() {
        if let Err(e) = std::fs::copy(from, to) {
            let _ = std::fs::remove_file(to);
            return Err(e);
        }
        std::fs::remove_file(from)?;
    }
    Ok(())
}

/// Deletes partly received files nobody resumed in time.
pub(crate) fn clean_incoming(dir: &Path) {
    let is_stale = |entry: &std::fs::DirEntry| {
        entry
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|t| t.elapsed().unwrap_or_default() > KEEP_PARTIAL)
    };
    let Ok(peers) = std::fs::read_dir(dir) else { return };
    for peer in peers.flatten() {
        let path = peer.path();
        // Writes to the phone's storage from a PC: `incoming/storage/<peer>/<id>.part`,
        // each resumable on its own (the folder's own age says nothing about them).
        if peer.file_name() == "storage" {
            for peer in std::fs::read_dir(&path).into_iter().flatten().flatten() {
                for part in std::fs::read_dir(peer.path()).into_iter().flatten().flatten() {
                    if is_stale(&part) {
                        let _ = std::fs::remove_file(part.path());
                    }
                }
                let _ = std::fs::remove_dir(peer.path());
            }
            let _ = std::fs::remove_dir(&path);
            continue;
        }
        if peer.file_name() == "localsend" {
            for session in std::fs::read_dir(&path).into_iter().flatten().flatten() {
                if is_stale(&session) {
                    let _ = std::fs::remove_dir_all(session.path());
                }
            }
            let _ = std::fs::remove_dir(&path);
            continue;
        }
        // Transfers from before they were kept per peer: `incoming/<offer>`.
        if peer.file_name().to_str().and_then(|n| n.parse::<DeviceId>().ok()).is_none() {
            if is_stale(&peer) {
                let _ = std::fs::remove_dir_all(&path);
            }
            continue;
        }
        if let Ok(offers) = std::fs::read_dir(&path) {
            for offer in offers.flatten() {
                if is_stale(&offer) {
                    let _ = std::fs::remove_dir_all(offer.path());
                }
            }
        }
        let _ = std::fs::remove_dir(&path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_ahead_ignores_where_another_handle_moved_the_file() {
        use std::io::{Seek, SeekFrom, Write};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        let data: Vec<u8> = (0..BLOCK * 3).map(|i| (i % 251) as u8).collect();
        std::fs::File::create(&path).unwrap().write_all(&data).unwrap();
        let mut other = std::fs::File::open(&path).unwrap();
        let clone = other.try_clone().unwrap();
        let start = 100u64;
        let want = data.len() as u64 - start;

        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let reader =
            std::thread::spawn(move || read_ahead(vec![(ReadSource::File(clone), start, want)], &tx));
        let mut got = Vec::new();
        while let Some(block) = rx.blocking_recv() {
            got.extend_from_slice(&block.expect("reads to the end"));
            // The reader of an attempt that just ended, sharing this position.
            other.seek(SeekFrom::End(0)).unwrap();
        }
        reader.join().unwrap();
        assert!(got == data[100..], "the bytes came from where they should");
    }

    #[cfg(windows)]
    #[test]
    fn received_files_are_marked_as_from_the_internet_zone() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("setup.exe");
        std::fs::write(&file, b"MZ").unwrap();
        mark_from_elsewhere(&file);
        let mut stream = file.into_os_string();
        stream.push(":Zone.Identifier");
        assert_eq!(std::fs::read_to_string(stream).unwrap(), "[ZoneTransfer]\r\nZoneId=3\r\n");
    }

    #[test]
    fn free_space_is_read_for_a_real_folder() {
        let dir = tempfile::tempdir().unwrap();
        assert!(free_space(dir.path()).is_some_and(|free| free > 0));
        assert_eq!(free_space(&dir.path().join("not-there")), None, "unknown never blocks");
    }

    #[test]
    fn files_go_back_to_their_part_files_when_one_cant_be_saved() {
        let dir = tempfile::tempdir().unwrap();
        let (incoming, target) = (dir.path().join("incoming"), dir.path().join("Downloads"));
        std::fs::create_dir_all(&incoming).unwrap();
        let entry = |name: &str, folder: Option<&str>| FileEntry {
            name: name.into(),
            size: 1,
            folder: folder.map(Into::into),
            blake3: None,
        };
        let entries = [entry("a.txt", None), entry("b.txt", Some("Trip")), entry("c.txt", None)];
        let parts: Vec<PathBuf> = (0..3).map(|i| incoming.join(format!("{i}.part"))).collect();
        std::fs::write(&parts[0], b"a").unwrap();
        std::fs::write(&parts[1], b"b").unwrap();
        // The third never arrived, so saving fails on it.

        assert!(save_files(&target, &entries, parts.clone()).is_err());
        assert_eq!(std::fs::read(&parts[0]).unwrap(), b"a", "back where a retry resumes");
        assert_eq!(std::fs::read(&parts[1]).unwrap(), b"b");
        assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0, "nothing half-delivered");

        std::fs::write(&parts[2], b"c").unwrap();
        let saved = save_files(&target, &entries, parts).unwrap();
        assert_eq!(saved, [target.join("a.txt"), target.join("Trip"), target.join("c.txt")]);
        assert_eq!(std::fs::read(target.join("Trip").join("b.txt")).unwrap(), b"b");
    }

    #[test]
    fn storage_writes_are_swept_one_part_file_at_a_time() {
        let dir = tempfile::tempdir().unwrap();
        let peer = dir.path().join("storage").join("peer");
        std::fs::create_dir_all(&peer).unwrap();
        let fresh = peer.join("fresh.part");
        let stale = peer.join("stale.part");
        std::fs::write(&fresh, b"resume me").unwrap();
        std::fs::write(&stale, b"old").unwrap();
        let old = std::time::SystemTime::now() - KEEP_PARTIAL - Duration::from_secs(60);
        std::fs::File::options().write(true).open(&stale).unwrap().set_modified(old).unwrap();

        clean_incoming(dir.path());
        assert!(fresh.exists(), "a recent write stays resumable");
        assert!(!stale.exists());

        std::fs::remove_file(&fresh).unwrap();
        clean_incoming(dir.path());
        assert!(!dir.path().join("storage").exists(), "emptied folders go");

        let ls = dir.path().join("localsend");
        let ls_fresh = ls.join("lss_fresh");
        let ls_stale = ls.join("lss_stale");
        std::fs::create_dir_all(&ls_fresh).unwrap();
        std::fs::create_dir_all(&ls_stale).unwrap();
        std::fs::File::options().write(true).open(&ls_stale).ok();
        let _ = filetime_backdate_dir(&ls_stale, old);
        clean_incoming(dir.path());
        assert!(ls_fresh.exists(), "a recent LocalSend session stays intact");
        std::fs::remove_dir_all(&ls_fresh).unwrap();
        std::fs::remove_dir_all(&ls_stale).ok();
        clean_incoming(dir.path());
        assert!(!ls.exists(), "emptied localsend folder goes");
    }

    fn filetime_backdate_dir(path: &Path, time: std::time::SystemTime) -> std::io::Result<()> {
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
            let f = std::fs::OpenOptions::new()
                .write(true)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                .open(path)?;
            f.set_modified(time)
        }
        #[cfg(not(windows))]
        {
            let f = std::fs::File::open(path)?;
            f.set_modified(time)
        }
    }

    #[test]
    fn names_are_made_safe() {
        assert_eq!(safe_file_name("photo.jpg"), "photo.jpg");
        assert_eq!(safe_file_name(r"C:\Users\x\report.pdf"), "report.pdf");
        assert_eq!(safe_file_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_file_name("a\nb"), "a_b");
        assert_eq!(safe_file_name(".."), "file");
        assert_eq!(safe_file_name(""), "file");
        assert!(safe_file_name(&"é".repeat(300)).len() <= files::MAX_NAME_BYTES);
    }

    #[test]
    fn taken_names_get_a_number() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(unique_path(dir.path(), "a.txt"), dir.path().join("a.txt"));
        std::fs::write(dir.path().join("a.txt"), "x").unwrap();
        std::fs::write(dir.path().join("a (2).txt"), "x").unwrap();
        assert_eq!(unique_path(dir.path(), "a.txt"), dir.path().join("a (3).txt"));
        std::fs::write(dir.path().join("README"), "x").unwrap();
        assert_eq!(unique_path(dir.path(), "README"), dir.path().join("README (2)"));
        std::fs::write(dir.path().join(".env"), "x").unwrap();
        assert_eq!(unique_path(dir.path(), ".env"), dir.path().join(".env (2)"));
    }

    #[test]
    fn names_windows_can_store() {
        assert_eq!(storable_name("photo.jpg"), "photo.jpg");
        assert_eq!(storable_name("CON"), "_CON");
        assert_eq!(storable_name("nul.txt"), "_nul.txt");
        assert_eq!(storable_name("com1"), "_com1");
        assert_eq!(storable_name("COMPUTER"), "COMPUTER");
        assert_eq!(storable_name("what?: \"x\"."), "what__ _x_");
        assert_eq!(storable_name(". "), "file");
    }

    #[test]
    fn items_are_files_and_folders() {
        let entry = |name: &str, folder: Option<&str>| FileEntry {
            name: name.into(),
            size: 1,
            folder: folder.map(Into::into),
            blake3: None,
        };
        let entries = [
            entry("a.txt", None),
            entry("1.jpg", Some("Trip")),
            entry("2.jpg", Some("Trip/Day 2")),
            entry("b.txt", None),
            entry("x", Some("Other")),
        ];
        assert_eq!(items(&entries), ["a.txt", "Trip", "b.txt", "Other"]);
    }

    #[test]
    fn folders_are_walked() {
        let dir = tempfile::tempdir().unwrap();
        let trip = dir.path().join("Trip");
        std::fs::create_dir_all(trip.join("Day 1")).unwrap();
        std::fs::create_dir_all(trip.join("empty")).unwrap();
        std::fs::write(trip.join("Day 1").join("b.jpg"), "b").unwrap();
        std::fs::write(trip.join("a.jpg"), "a").unwrap();
        std::fs::write(dir.path().join("note.txt"), "n").unwrap();
        let files = outgoing_paths(&[dir.path().join("note.txt"), trip]).unwrap();
        let listed: Vec<(&str, Option<&str>)> =
            files.iter().map(|f| (f.name.as_str(), f.folder.as_deref())).collect();
        assert_eq!(listed, [("note.txt", None), ("b.jpg", Some("Trip/Day 1")), ("a.jpg", Some("Trip"))]);
    }

    #[test]
    fn ids_are_valid() {
        assert!(nectarlink_protocol::messages::is_valid_transfer_id(&new_id()));
        assert_ne!(new_id(), new_id());
    }

    #[test]
    fn read_ahead_opens_paths_on_demand() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.bin");
        let b = dir.path().join("b.bin");
        std::fs::write(&a, b"hello ").unwrap();
        std::fs::write(&b, b"world").unwrap();
        let plan = vec![(ReadSource::Path(a), 0, 6), (ReadSource::Path(b), 0, 5)];
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        read_ahead(plan, &tx);
        drop(tx);
        let mut out = Vec::new();
        while let Some(block) = rx.blocking_recv() {
            out.extend_from_slice(&block.unwrap());
        }
        assert_eq!(out, b"hello world");
    }
}
