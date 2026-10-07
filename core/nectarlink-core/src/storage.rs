// SPDX-License-Identifier: MPL-2.0
//! Phone storage in File Explorer (`docs/protocol/storage.md`).
//!
//! A phone that offers `storage.read` / `storage.write` lets a paired PC
//! (while the `storage` per-device toggle is on) list directories, read file
//! ranges on a dedicated stream, upload files with resume on a dedicated
//! stream, and create folders, rename and delete entries.

use std::{
    collections::{HashMap, HashSet},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use bytes::Bytes;
use iroh::endpoint::{RecvStream, SendStream};
pub use nectarlink_protocol::messages::{
    StorageChanged, StorageDelete, StorageEntries, StorageEntry, StorageList, StorageMkdir, StorageRead,
    StorageReadMeta, StorageRename, StorageWriteAccept, StorageWriteDone, StorageWriteOffer,
    is_valid_storage_dir_path, is_valid_storage_id, is_valid_storage_name, is_valid_storage_path,
    storage::{MOUNT as STORAGE_MOUNT, READ as STORAGE_READ, WRITE as STORAGE_WRITE},
};
use nectarlink_protocol::{
    DeviceId, Envelope, ErrorCode, MAX_FRAME_LEN,
    messages::{StreamHeader, storage, types},
    read_frame, write_frame,
};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

use crate::{
    Error, NodeEvent, Result,
    node::Shared,
    session::{REQUEST_TIMEOUT, Session},
    transfer::FileSource,
};

/// Per-device toggle name (off by default on the phone).
pub const TOGGLE: &str = "storage";

const STREAM_TIMEOUT: Duration = Duration::from_secs(30);
const CHUNK_SIZE: usize = 64 * 1024;
const MAX_OPEN_FOLDERS_PER_PEER: usize = 256;
const CHANGED_DEBOUNCE: Duration = Duration::from_millis(250);

/// Why a phone storage operation failed on the platform side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageError {
    NotFound,
    Denied,
    Invalid(String),
    NoSpace,
    Unsupported,
    Failed(String),
}

impl StorageError {
    pub fn into_code_and_msg(self) -> (ErrorCode, String) {
        match self {
            StorageError::NotFound => (ErrorCode::NotFound, "not found".into()),
            StorageError::Denied => (ErrorCode::Denied, "access denied".into()),
            StorageError::Invalid(m) => (ErrorCode::BadMessage, m),
            StorageError::NoSpace => (ErrorCode::Busy, "not enough space".into()),
            StorageError::Unsupported => (ErrorCode::Unsupported, "storage not available".into()),
            StorageError::Failed(m) => (ErrorCode::Internal, m),
        }
    }
}

/// An opened file on the phone ready for ranged reading (`storage.read`).
#[derive(Debug)]
pub struct StorageReadFile {
    pub source: FileSource,
    pub size: u64,
    /// Last-modified time in Unix milliseconds.
    pub modified: i64,
}

/// Per-node state for the storage service: one-time prompt tracking, folders
/// each connected PC has listed, and debounce timestamps for `storage.changed`.
#[derive(Debug, Default)]
pub(crate) struct StorageState {
    prompted: HashSet<DeviceId>,
    open_folders: HashMap<DeviceId, HashSet<String>>,
    last_changed: HashMap<(DeviceId, String), Instant>,
}

impl StorageState {
    pub fn mark_prompted(&mut self, peer: DeviceId) -> bool {
        self.prompted.insert(peer)
    }

    pub fn clear_prompted(&mut self, peer: &DeviceId) {
        self.prompted.remove(peer);
    }

    pub fn record_open_folder(&mut self, peer: DeviceId, path: &str) {
        let set = self.open_folders.entry(peer).or_default();
        if set.len() >= MAX_OPEN_FOLDERS_PER_PEER && !set.contains(path) {
            set.clear();
        }
        set.insert(path.to_owned());
    }

    pub fn open_folders_all(&self) -> Vec<String> {
        let mut out: HashSet<String> = HashSet::new();
        for set in self.open_folders.values() {
            for p in set {
                out.insert(p.clone());
            }
        }
        let mut v: Vec<String> = out.into_iter().collect();
        v.sort();
        v
    }

    pub fn peers_watching(&mut self, path: &str, now: Instant) -> Vec<DeviceId> {
        let mut peers = Vec::new();
        for (peer, folders) in &self.open_folders {
            if folders.contains(path) {
                let key = (*peer, path.to_owned());
                let should_send = self
                    .last_changed
                    .get(&key)
                    .is_none_or(|prev| now.saturating_duration_since(*prev) >= CHANGED_DEBOUNCE);
                if should_send {
                    self.last_changed.insert(key, now);
                    peers.push(*peer);
                }
            }
        }
        peers
    }

    pub fn remove_peer(&mut self, peer: &DeviceId) {
        self.prompted.remove(peer);
        self.open_folders.remove(peer);
        self.last_changed.retain(|(p, _), _| p != peer);
    }
}

fn peer_offers(shared: &Shared, peer: &DeviceId, cap: &str) -> Result<bool> {
    Ok(shared.store.get_peer(peer)?.is_some_and(|p| p.caps.contains(cap)))
}

fn we_offer(shared: &Shared, cap: &str) -> bool {
    shared.local_capabilities().iter().any(|c| c == cap)
}

/// Checks if the PC user explicitly turned `storage` off for `peer`.
fn pc_allows(shared: &Shared, peer: &DeviceId) -> Result<bool> {
    Ok(shared.store.toggles(peer)?.get(TOGGLE).copied().unwrap_or(true))
}

fn check_phone_allowed(
    shared: &Shared,
    peer: &DeviceId,
    cap: &str,
) -> std::result::Result<(), (ErrorCode, String)> {
    if !we_offer(shared, cap) {
        return Err((ErrorCode::Unsupported, format!("{cap} is not offered by this device")));
    }
    if !shared.toggle_on(peer, TOGGLE) {
        let first_time = shared.storage.lock().unwrap_or_else(|e| e.into_inner()).mark_prompted(*peer);
        if first_time {
            shared.emit(NodeEvent::StorageRequested { device: *peer });
        }
        return Err((ErrorCode::Denied, "storage access is turned off for this device".into()));
    }
    Ok(())
}

fn parent_dir_of(path: &str) -> &str {
    match path.rfind('/') {
        Some(idx) => &path[..idx],
        None => "",
    }
}

/// Notifies connected PCs that have `path` open that its contents changed.
pub(crate) async fn notify_changed(shared: &Arc<Shared>, path: String) {
    if !is_valid_storage_dir_path(&path) {
        return;
    }
    let peers = {
        let mut st = shared.storage.lock().unwrap_or_else(|e| e.into_inner());
        st.peers_watching(&path, Instant::now())
    };
    if peers.is_empty() {
        return;
    }
    let Ok(env) = Envelope::new(types::STORAGE_CHANGED, &StorageChanged { path }) else {
        return;
    };
    for peer in peers {
        if shared.toggle_on(&peer, TOGGLE)
            && let Some(session) = shared.session(&peer)
        {
            let _ = session.send(env.clone()).await;
        }
    }
}

async fn notify_parent_changed(shared: &Arc<Shared>, path: &str) {
    notify_changed(shared, parent_dir_of(path).to_owned()).await;
}

/// Handles incoming `storage.*` messages on the control stream.
pub(crate) async fn handle(shared: &Arc<Shared>, session: &Arc<Session>, env: &Envelope) -> Result<bool> {
    let peer = session.peer;
    match env.t.as_str() {
        types::STORAGE_LIST => {
            let req = match env.body::<StorageList>() {
                Ok(r) if r.is_valid() => r,
                _ => {
                    let reply =
                        Envelope::error(ErrorCode::BadMessage, "invalid storage.list path").reply_to(env.id);
                    session.send(reply).await?;
                    return Ok(true);
                }
            };
            if let Err((code, msg)) = check_phone_allowed(shared, &peer, STORAGE_READ) {
                session.send(Envelope::error(code, msg).reply_to(env.id)).await?;
                return Ok(true);
            }
            shared.storage.lock().unwrap_or_else(|e| e.into_inner()).record_open_folder(peer, &req.path);
            let platform = shared.platform.clone();
            let path = req.path;
            let res = tokio::task::spawn_blocking(move || platform.storage_list(&path))
                .await
                .unwrap_or_else(|e| Err(StorageError::Failed(e.to_string())));
            match res {
                Ok(raw_entries) => {
                    let mut entries: Vec<StorageEntry> =
                        raw_entries.into_iter().filter_map(StorageEntry::sanitized).collect();
                    // Sort directories first, then case-insensitively by name.
                    entries.sort_by(|a, b| {
                        b.is_dir
                            .cmp(&a.is_dir)
                            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                            .then_with(|| a.name.cmp(&b.name))
                    });
                    // Ensure the frame fits within MAX_FRAME_LEN.
                    let reply = loop {
                        let body = StorageEntries { entries: entries.clone() };
                        let candidate = Envelope::new(types::STORAGE_ENTRIES, &body)?.reply_to(env.id);
                        if candidate.to_cbor().len() <= MAX_FRAME_LEN || entries.is_empty() {
                            break candidate;
                        }
                        let new_len = (entries.len() * 3 / 4).max(entries.len().saturating_sub(1));
                        entries.truncate(new_len);
                    };
                    session.send(reply).await?;
                }
                Err(err) => {
                    let (code, msg) = err.into_code_and_msg();
                    session.send(Envelope::error(code, msg).reply_to(env.id)).await?;
                }
            }
            Ok(true)
        }
        types::STORAGE_MKDIR => {
            let req = match env.body::<StorageMkdir>() {
                Ok(r) if r.is_valid() => r,
                _ => {
                    let reply =
                        Envelope::error(ErrorCode::BadMessage, "invalid storage.mkdir path").reply_to(env.id);
                    session.send(reply).await?;
                    return Ok(true);
                }
            };
            if let Err((code, msg)) = check_phone_allowed(shared, &peer, STORAGE_WRITE) {
                session.send(Envelope::error(code, msg).reply_to(env.id)).await?;
                return Ok(true);
            }
            let platform = shared.platform.clone();
            let path = req.path.clone();
            let res = tokio::task::spawn_blocking(move || platform.storage_mkdir(&path))
                .await
                .unwrap_or_else(|e| Err(StorageError::Failed(e.to_string())));
            match res {
                Ok(()) => {
                    session.send(Envelope::empty(types::OK).reply_to(env.id)).await?;
                    notify_parent_changed(shared, &req.path).await;
                }
                Err(err) => {
                    let (code, msg) = err.into_code_and_msg();
                    session.send(Envelope::error(code, msg).reply_to(env.id)).await?;
                }
            }
            Ok(true)
        }
        types::STORAGE_RENAME => {
            let req = match env.body::<StorageRename>() {
                Ok(r) if r.is_valid() => r,
                _ => {
                    let reply = Envelope::error(ErrorCode::BadMessage, "invalid storage.rename paths")
                        .reply_to(env.id);
                    session.send(reply).await?;
                    return Ok(true);
                }
            };
            if let Err((code, msg)) = check_phone_allowed(shared, &peer, STORAGE_WRITE) {
                session.send(Envelope::error(code, msg).reply_to(env.id)).await?;
                return Ok(true);
            }
            let platform = shared.platform.clone();
            let from = req.from.clone();
            let to = req.to.clone();
            let res = tokio::task::spawn_blocking(move || platform.storage_rename(&from, &to))
                .await
                .unwrap_or_else(|e| Err(StorageError::Failed(e.to_string())));
            match res {
                Ok(()) => {
                    session.send(Envelope::empty(types::OK).reply_to(env.id)).await?;
                    notify_parent_changed(shared, &req.from).await;
                    if parent_dir_of(&req.from) != parent_dir_of(&req.to) {
                        notify_parent_changed(shared, &req.to).await;
                    }
                }
                Err(err) => {
                    let (code, msg) = err.into_code_and_msg();
                    session.send(Envelope::error(code, msg).reply_to(env.id)).await?;
                }
            }
            Ok(true)
        }
        types::STORAGE_DELETE => {
            let req = match env.body::<StorageDelete>() {
                Ok(r) if r.is_valid() => r,
                _ => {
                    let reply = Envelope::error(ErrorCode::BadMessage, "invalid storage.delete path")
                        .reply_to(env.id);
                    session.send(reply).await?;
                    return Ok(true);
                }
            };
            if let Err((code, msg)) = check_phone_allowed(shared, &peer, STORAGE_WRITE) {
                session.send(Envelope::error(code, msg).reply_to(env.id)).await?;
                return Ok(true);
            }
            let platform = shared.platform.clone();
            let path = req.path.clone();
            let confirmed = req.confirmed;
            let res = tokio::task::spawn_blocking(move || platform.storage_delete(&path, confirmed))
                .await
                .unwrap_or_else(|e| Err(StorageError::Failed(e.to_string())));
            match res {
                Ok(()) => {
                    session.send(Envelope::empty(types::OK).reply_to(env.id)).await?;
                    notify_parent_changed(shared, &req.path).await;
                }
                Err(err) => {
                    let (code, msg) = err.into_code_and_msg();
                    session.send(Envelope::error(code, msg).reply_to(env.id)).await?;
                }
            }
            Ok(true)
        }
        types::STORAGE_CHANGED => {
            if let Ok(changed) = env.body::<StorageChanged>()
                && changed.is_valid()
                && pc_allows(shared, &peer).unwrap_or(true)
            {
                shared.emit(NodeEvent::StorageChanged { device: peer, path: changed.path });
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Serves an incoming `storage/read` stream on the phone (`docs/protocol/storage.md` §4.1).
pub(crate) async fn serve_read(
    shared: Arc<Shared>,
    peer: DeviceId,
    mut send: SendStream,
    mut recv: RecvStream,
) {
    let Ok(Ok(Some(bytes))) = tokio::time::timeout(STREAM_TIMEOUT, read_frame(&mut recv)).await else {
        return;
    };
    let Ok(env) = Envelope::from_cbor(&bytes) else {
        return;
    };
    let req = match env.expect_body::<StorageRead>(types::STORAGE_READ) {
        Ok(r) if r.is_valid() => r,
        _ => {
            let reply = Envelope::error(ErrorCode::BadMessage, "invalid storage.read").reply_to(env.id);
            let _ = write_frame(&mut send, &reply.to_cbor()).await;
            let _ = send.finish();
            return;
        }
    };
    if let Err((code, msg)) = check_phone_allowed(&shared, &peer, STORAGE_READ) {
        let reply = Envelope::error(code, msg).reply_to(env.id);
        let _ = write_frame(&mut send, &reply.to_cbor()).await;
        let _ = send.finish();
        return;
    }

    let platform = shared.platform.clone();
    let path = req.path.clone();
    let opened = tokio::task::spawn_blocking(move || platform.storage_open_read(&path))
        .await
        .unwrap_or_else(|e| Err(StorageError::Failed(e.to_string())));

    let file = match opened {
        Ok(f) => f,
        Err(err) => {
            let (code, msg) = err.into_code_and_msg();
            let reply = Envelope::error(code, msg).reply_to(env.id);
            let _ = write_frame(&mut send, &reply.to_cbor()).await;
            let _ = send.finish();
            return;
        }
    };

    let avail = file.size.saturating_sub(req.offset);
    let to_send = req.length.map_or(avail, |l| l.min(avail));
    let meta = StorageReadMeta { size: file.size, modified: file.modified.max(0), length: to_send };
    let Ok(reply) = Envelope::new(types::STORAGE_READ_META, &meta).map(|e| e.reply_to(env.id)) else {
        return;
    };
    if write_frame(&mut send, &reply.to_cbor()).await.is_err() {
        return;
    }
    if to_send == 0 {
        let _ = send.finish();
        return;
    }

    let offset = req.offset;
    let source = file.source;
    let (tx, mut rx) = tokio::sync::mpsc::channel::<std::result::Result<Bytes, String>>(8);
    tokio::task::spawn_blocking(move || {
        let mut std_file: std::fs::File = match source {
            FileSource::Path(p) => match std::fs::File::open(p) {
                Ok(f) => f,
                Err(e) => {
                    let _ = tx.blocking_send(Err(e.to_string()));
                    return;
                }
            },
            FileSource::File(f) => f,
        };
        if offset > 0 && std_file.seek(SeekFrom::Start(offset)).is_err() {
            let _ = tx.blocking_send(Err("seek failed".into()));
            return;
        }
        let mut remaining = to_send;
        while remaining > 0 {
            let want = (remaining as usize).min(CHUNK_SIZE);
            let mut buf = vec![0u8; want];
            match std_file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    buf.truncate(n);
                    remaining -= n as u64;
                    if tx.blocking_send(Ok(Bytes::from(buf))).is_err() {
                        return;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    let _ = tx.blocking_send(Err(e.to_string()));
                    return;
                }
            }
        }
    });

    while let Some(chunk) = rx.recv().await {
        match chunk {
            Ok(bytes) => {
                if send.write_chunk(bytes).await.is_err() {
                    return;
                }
            }
            Err(_) => return,
        }
    }
    let _ = send.finish();
}

fn staging_file_path(shared: &Shared, peer: &DeviceId, offer: &StorageWriteOffer) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    offer.path.hash(&mut hasher);
    offer.size.hash(&mut hasher);
    let path_hash = hasher.finish();
    shared
        .incoming_dir()
        .join("storage")
        .join(peer.to_string())
        .join(format!("{}-{path_hash:016x}.part", offer.id))
}

/// Serves an incoming `storage/write` stream on the phone (`docs/protocol/storage.md` §4.2).
pub(crate) async fn serve_write(
    shared: Arc<Shared>,
    peer: DeviceId,
    mut send: SendStream,
    mut recv: RecvStream,
) {
    let Ok(Ok(Some(bytes))) = tokio::time::timeout(STREAM_TIMEOUT, read_frame(&mut recv)).await else {
        return;
    };
    let Ok(env) = Envelope::from_cbor(&bytes) else {
        return;
    };
    let offer = match env.expect_body::<StorageWriteOffer>(types::STORAGE_WRITE) {
        Ok(o) if o.is_valid() => o,
        _ => {
            let reply = Envelope::error(ErrorCode::BadMessage, "invalid storage.write").reply_to(env.id);
            let _ = write_frame(&mut send, &reply.to_cbor()).await;
            let _ = send.finish();
            return;
        }
    };
    if let Err((code, msg)) = check_phone_allowed(&shared, &peer, STORAGE_WRITE) {
        let reply = Envelope::error(code, msg).reply_to(env.id);
        let _ = write_frame(&mut send, &reply.to_cbor()).await;
        let _ = send.finish();
        return;
    }

    let part_path = staging_file_path(&shared, &peer, &offer);
    if let Some(parent) = part_path.parent()
        && tokio::fs::create_dir_all(parent).await.is_err()
    {
        let reply = Envelope::error(ErrorCode::Internal, "cannot create staging directory").reply_to(env.id);
        let _ = write_frame(&mut send, &reply.to_cbor()).await;
        let _ = send.finish();
        return;
    }

    let existing_len = tokio::fs::metadata(&part_path).await.map(|m| m.len()).unwrap_or(0);
    let have = if existing_len <= offer.size { existing_len } else { 0 };

    let mut file = match tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(have == 0)
        .open(&part_path)
        .await
    {
        Ok(f) => f,
        Err(_) => {
            let reply = Envelope::error(ErrorCode::Internal, "cannot open staging file").reply_to(env.id);
            let _ = write_frame(&mut send, &reply.to_cbor()).await;
            let _ = send.finish();
            return;
        }
    };
    if have > 0 && file.seek(SeekFrom::Start(have)).await.is_err() {
        let _ = tokio::fs::remove_file(&part_path).await;
        return;
    }

    let Ok(accept) =
        Envelope::new(types::STORAGE_WRITE_ACCEPT, &StorageWriteAccept { have }).map(|e| e.reply_to(env.id))
    else {
        return;
    };
    if write_frame(&mut send, &accept.to_cbor()).await.is_err() {
        return;
    }

    let mut received = have;
    while received < offer.size {
        match tokio::time::timeout(STREAM_TIMEOUT, recv.read_chunk(CHUNK_SIZE)).await {
            Ok(Ok(Some(chunk))) => {
                if chunk.is_empty() {
                    break;
                }
                let next = received.saturating_add(chunk.len() as u64);
                if next > offer.size {
                    let _ = file.flush().await;
                    drop(file);
                    let _ = tokio::fs::remove_file(&part_path).await;
                    let reply = Envelope::error(ErrorCode::BadMessage, "too many bytes sent");
                    let _ = write_frame(&mut send, &reply.to_cbor()).await;
                    let _ = send.finish();
                    return;
                }
                if file.write_all(&chunk).await.is_err() {
                    let _ = file.flush().await;
                    let reply = Envelope::error(ErrorCode::Busy, "write failed");
                    let _ = write_frame(&mut send, &reply.to_cbor()).await;
                    let _ = send.finish();
                    return;
                }
                received = next;
            }
            Ok(Ok(None)) => break,
            _ => {
                let _ = file.flush().await;
                return;
            }
        }
    }

    if file.flush().await.is_err() || file.sync_all().await.is_err() {
        return;
    }
    drop(file);

    if received != offer.size {
        // Keep the partial staging file so a retry with the same `id` resumes.
        return;
    }

    let platform = shared.platform.clone();
    let dest_path = offer.path.clone();
    let staged = part_path.clone();
    let modified = offer.modified;
    let res = tokio::task::spawn_blocking(move || platform.storage_write(&dest_path, &staged, modified))
        .await
        .unwrap_or_else(|e| Err(StorageError::Failed(e.to_string())));
    let _ = tokio::fs::remove_file(&part_path).await;

    match res {
        Ok(done) => {
            if let Ok(env) = Envelope::new(types::STORAGE_WRITE_DONE, &done) {
                let _ = write_frame(&mut send, &env.to_cbor()).await;
            }
            let _ = send.finish();
            // Let the confirmation reach the sender before the stream closes.
            let _ = tokio::time::timeout(Duration::from_secs(2), send.stopped()).await;
            notify_parent_changed(&shared, &offer.path).await;
        }
        Err(err) => {
            let (code, msg) = err.into_code_and_msg();
            let reply = Envelope::error(code, msg);
            let _ = write_frame(&mut send, &reply.to_cbor()).await;
            let _ = send.finish();
            let _ = tokio::time::timeout(Duration::from_secs(2), send.stopped()).await;
        }
    }
}

// ---- PC-side client operations ----

pub(crate) fn new_upload_id() -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    (0..24).map(|_| ALPHABET[rand::random_range(0..ALPHABET.len())] as char).collect()
}

pub(crate) async fn list(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    path: String,
) -> Result<Vec<StorageEntry>> {
    if !is_valid_storage_dir_path(&path) {
        return Err(Error::Protocol("invalid storage directory path".into()));
    }
    if !peer_offers(shared, &session.peer, STORAGE_READ)? {
        return Err(Error::Unsupported);
    }
    if !pc_allows(shared, &session.peer)? {
        return Err(Error::Denied);
    }
    let req = Envelope::new(types::STORAGE_LIST, &StorageList { path })?;
    let reply = session.request(req, REQUEST_TIMEOUT).await?;
    let body: StorageEntries = reply.expect_body(types::STORAGE_ENTRIES)?;
    Ok(body.entries.into_iter().filter_map(StorageEntry::sanitized).collect())
}

pub(crate) async fn mkdir(shared: &Arc<Shared>, session: &Arc<Session>, path: String) -> Result<()> {
    if !is_valid_storage_path(&path) {
        return Err(Error::Protocol("invalid storage path".into()));
    }
    if !peer_offers(shared, &session.peer, STORAGE_WRITE)? {
        return Err(Error::Unsupported);
    }
    if !pc_allows(shared, &session.peer)? {
        return Err(Error::Denied);
    }
    let req = Envelope::new(types::STORAGE_MKDIR, &StorageMkdir { path })?;
    session.request(req, REQUEST_TIMEOUT).await?.expect(types::OK)?;
    Ok(())
}

pub(crate) async fn rename(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    from: String,
    to: String,
) -> Result<()> {
    let body = StorageRename { from, to };
    if !body.is_valid() {
        return Err(Error::Protocol("invalid storage rename path".into()));
    }
    if !peer_offers(shared, &session.peer, STORAGE_WRITE)? {
        return Err(Error::Unsupported);
    }
    if !pc_allows(shared, &session.peer)? {
        return Err(Error::Denied);
    }
    let req = Envelope::new(types::STORAGE_RENAME, &body)?;
    session.request(req, REQUEST_TIMEOUT).await?.expect(types::OK)?;
    Ok(())
}

pub(crate) async fn delete(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    path: String,
    confirmed: bool,
) -> Result<()> {
    let body = StorageDelete { path, confirmed };
    if !body.is_valid() {
        return Err(Error::Protocol("invalid storage delete path".into()));
    }
    if !peer_offers(shared, &session.peer, STORAGE_WRITE)? {
        return Err(Error::Unsupported);
    }
    if !pc_allows(shared, &session.peer)? {
        return Err(Error::Denied);
    }
    let req = Envelope::new(types::STORAGE_DELETE, &body)?;
    session.request(req, REQUEST_TIMEOUT).await?.expect(types::OK)?;
    Ok(())
}

/// Opens a ranged read stream for `path` on `session`, returning the
/// [`StorageReadMeta`] and the raw QUIC [`RecvStream`].
pub(crate) async fn open_read_stream(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    path: String,
    offset: u64,
    length: Option<u64>,
) -> Result<(StorageReadMeta, RecvStream)> {
    let req = StorageRead { path, offset, length };
    if !req.is_valid() {
        return Err(Error::Protocol("invalid storage read path".into()));
    }
    if !peer_offers(shared, &session.peer, STORAGE_READ)? {
        return Err(Error::Unsupported);
    }
    if !pc_allows(shared, &session.peer)? {
        return Err(Error::Denied);
    }

    let (mut send, mut recv) = session.open_bi().await?;
    let header =
        StreamHeader { svc: storage::SERVICE.into(), op: storage::OP_READ.into(), v: storage::VERSION };
    let header_env = Envelope::new(types::STREAM, &header)?;
    let req_env = Envelope::new(types::STORAGE_READ, &req)?.with_id(1);
    write_frame(&mut send, &header_env.to_cbor()).await?;
    write_frame(&mut send, &req_env.to_cbor()).await?;
    let _ = send.finish();

    let reply_bytes = match tokio::time::timeout(STREAM_TIMEOUT, read_frame(&mut recv)).await {
        Ok(Ok(Some(b))) => b,
        Ok(Ok(None)) => return Err(Error::Offline),
        Ok(Err(e)) => return Err(e.into()),
        Err(_) => return Err(Error::Timeout),
    };
    let reply = Envelope::from_cbor(&reply_bytes)?;
    let meta: StorageReadMeta = reply.expect_body(types::STORAGE_READ_META)?;
    Ok((meta, recv))
}

/// Uploads `source` to `path` on `session` with resumable upload ID `id`.
/// If `stop_after` is `Some(limit)`, stops sending after `limit` total bytes
/// (used by tests to simulate an interrupted upload before resuming).
pub(crate) async fn write_with_id(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    id: String,
    path: String,
    source: FileSource,
    modified: Option<i64>,
    stop_after: Option<u64>,
) -> Result<StorageWriteDone> {
    if !is_valid_storage_id(&id) || !is_valid_storage_path(&path) {
        return Err(Error::Protocol("invalid storage write offer".into()));
    }
    if !peer_offers(shared, &session.peer, STORAGE_WRITE)? {
        return Err(Error::Unsupported);
    }
    if !pc_allows(shared, &session.peer)? {
        return Err(Error::Denied);
    }

    let (std_file, size, file_mtime) =
        tokio::task::spawn_blocking(move || -> Result<(std::fs::File, u64, Option<i64>)> {
            let f = match source {
                FileSource::Path(p) => std::fs::File::open(&p)?,
                FileSource::File(f) => f,
            };
            let meta = f.metadata()?;
            if !meta.is_file() {
                return Err(Error::Protocol("storage write source must be a regular file".into()));
            }
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .and_then(|d| i64::try_from(d.as_millis()).ok());
            Ok((f, meta.len(), mtime))
        })
        .await
        .map_err(|e| Error::Internal(e.to_string()))??;

    let offer = StorageWriteOffer { id, path, size, modified: modified.or(file_mtime) };
    if !offer.is_valid() {
        return Err(Error::Protocol("invalid storage write offer".into()));
    }

    let (mut send, mut recv) = session.open_bi().await?;
    let header =
        StreamHeader { svc: storage::SERVICE.into(), op: storage::OP_WRITE.into(), v: storage::VERSION };
    let header_env = Envelope::new(types::STREAM, &header)?;
    let offer_env = Envelope::new(types::STORAGE_WRITE, &offer)?.with_id(1);
    write_frame(&mut send, &header_env.to_cbor()).await?;
    write_frame(&mut send, &offer_env.to_cbor()).await?;

    let accept_bytes = match tokio::time::timeout(STREAM_TIMEOUT, read_frame(&mut recv)).await {
        Ok(Ok(Some(b))) => b,
        Ok(Ok(None)) => return Err(Error::Offline),
        Ok(Err(e)) => return Err(e.into()),
        Err(_) => return Err(Error::Timeout),
    };
    let accept_env = Envelope::from_cbor(&accept_bytes)?;
    let accept: StorageWriteAccept = accept_env.expect_body(types::STORAGE_WRITE_ACCEPT)?;
    let start = accept.have.min(size);
    let target_end = stop_after.map_or(size, |limit| limit.min(size));

    if target_end > start {
        let to_read = target_end - start;
        let (tx, mut rx) = tokio::sync::mpsc::channel::<std::result::Result<Bytes, std::io::Error>>(8);
        tokio::task::spawn_blocking(move || {
            let mut f = std_file;
            if start > 0
                && let Err(e) = f.seek(SeekFrom::Start(start))
            {
                let _ = tx.blocking_send(Err(e));
                return;
            }
            let mut rem = to_read;
            while rem > 0 {
                let want = (rem as usize).min(CHUNK_SIZE);
                let mut buf = vec![0u8; want];
                match f.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        buf.truncate(n);
                        rem -= n as u64;
                        if tx.blocking_send(Ok(Bytes::from(buf))).is_err() {
                            return;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => {
                        let _ = tx.blocking_send(Err(e));
                        return;
                    }
                }
            }
        });

        while let Some(chunk) = rx.recv().await {
            let bytes = chunk?;
            send.write_chunk(bytes).await.map_err(|_| Error::Offline)?;
        }
    }

    if stop_after.is_some_and(|limit| limit < size) {
        let _ = send.finish();
        return Err(Error::Offline);
    }

    send.finish().map_err(|_| Error::Offline)?;
    let done_bytes = match tokio::time::timeout(STREAM_TIMEOUT, read_frame(&mut recv)).await {
        Ok(Ok(Some(b))) => b,
        Ok(Ok(None)) => return Err(Error::Offline),
        Ok(Err(e)) => return Err(e.into()),
        Err(_) => return Err(Error::Timeout),
    };
    let done_env = Envelope::from_cbor(&done_bytes)?;
    let done: StorageWriteDone = done_env.expect_body(types::STORAGE_WRITE_DONE)?;
    Ok(done)
}

// ---- FolderStorage: filesystem-backed phone storage provider ----

/// Filesystem-backed storage root that enforces relative path validation,
/// blocks `/data` and private directories, and rejects symlinks resolving
/// outside the canonical root.
#[derive(Debug, Clone)]
pub struct FolderStorage {
    root: PathBuf,
    blocked_prefixes: Vec<PathBuf>,
    trash_dir: Option<PathBuf>,
}

impl FolderStorage {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            blocked_prefixes: vec![
                PathBuf::from("/data/data"),
                PathBuf::from("/data/user"),
                PathBuf::from("/data/system"),
                PathBuf::from("/data/app"),
                PathBuf::from("/data/misc"),
            ],
            trash_dir: None,
        }
    }

    /// Configures an optional trash directory where unconfirmed deletes of
    /// media files (`.jpg`, `.jpeg`, `.png`, `.webp`, `.gif`, `.mp4`, `.mov`,
    /// `.mp3`, `.m4a`, `.wav`, `.flac`) are moved, matching Android's
    /// `MediaStore` trash behavior.
    pub fn with_trash_dir(mut self, trash: impl Into<PathBuf>) -> Self {
        self.trash_dir = Some(trash.into());
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn canonical_root(&self) -> std::result::Result<PathBuf, StorageError> {
        let canon = std::fs::canonicalize(&self.root).map_err(|_| StorageError::NotFound)?;
        if self.is_blocked(&canon) {
            return Err(StorageError::Denied);
        }
        Ok(canon)
    }

    fn is_blocked(&self, canon: &Path) -> bool {
        if canon == Path::new("/data") {
            return true;
        }
        self.blocked_prefixes.iter().any(|p| canon.starts_with(p))
    }

    fn ensure_inside(&self, canon_root: &Path, candidate: &Path) -> std::result::Result<(), StorageError> {
        if !candidate.starts_with(canon_root) || self.is_blocked(candidate) {
            return Err(StorageError::Denied);
        }
        Ok(())
    }

    /// Resolves an existing directory or file relative to `root`, rejecting
    /// `..`, absolute paths, and any symlink pointing outside `canonical_root`.
    pub fn resolve_existing(
        &self,
        rel: &str,
        allow_root: bool,
    ) -> std::result::Result<PathBuf, StorageError> {
        let valid = if allow_root { is_valid_storage_dir_path(rel) } else { is_valid_storage_path(rel) };
        if !valid {
            return Err(StorageError::Invalid("invalid storage path".into()));
        }
        let canon_root = self.canonical_root()?;
        if rel.is_empty() {
            return Ok(canon_root);
        }
        let mut cur = canon_root.clone();
        for seg in rel.split('/') {
            cur.push(seg);
            let meta = std::fs::symlink_metadata(&cur).map_err(|_| StorageError::NotFound)?;
            if meta.file_type().is_symlink() {
                let target = std::fs::canonicalize(&cur).map_err(|_| StorageError::Denied)?;
                self.ensure_inside(&canon_root, &target)?;
            }
        }
        let canon = std::fs::canonicalize(&cur).map_err(|_| StorageError::NotFound)?;
        self.ensure_inside(&canon_root, &canon)?;
        Ok(canon)
    }

    /// Resolves a destination path (whose final segment or parent directories
    /// might not exist yet) inside `root`, rejecting any symlink along the
    /// existing prefix that points outside `canonical_root`.
    pub fn resolve_target(&self, rel: &str) -> std::result::Result<PathBuf, StorageError> {
        if !is_valid_storage_path(rel) {
            return Err(StorageError::Invalid("invalid storage path".into()));
        }
        let canon_root = self.canonical_root()?;
        let mut cur = canon_root.clone();
        for seg in rel.split('/') {
            cur.push(seg);
            if let Ok(meta) = std::fs::symlink_metadata(&cur)
                && meta.file_type().is_symlink()
            {
                let target = std::fs::canonicalize(&cur).map_err(|_| StorageError::Denied)?;
                self.ensure_inside(&canon_root, &target)?;
            }
        }
        Ok(cur)
    }

    pub fn list(&self, path: &str) -> std::result::Result<Vec<StorageEntry>, StorageError> {
        let dir = self.resolve_existing(path, true)?;
        let canon_root = self.canonical_root()?;
        let meta = std::fs::metadata(&dir).map_err(|_| StorageError::NotFound)?;
        if !meta.is_dir() {
            return Err(StorageError::NotFound);
        }
        let read_dir = std::fs::read_dir(&dir).map_err(|_| StorageError::Denied)?;
        let mut entries = Vec::new();
        for item in read_dir.flatten() {
            let Ok(name) = item.file_name().into_string() else {
                continue;
            };
            if !is_valid_storage_name(&name) || name.starts_with(".nectarlink") {
                continue;
            }
            let item_path = item.path();
            let Ok(canon) = std::fs::canonicalize(&item_path) else {
                continue;
            };
            if self.ensure_inside(&canon_root, &canon).is_err() {
                continue;
            }
            let Ok(md) = std::fs::metadata(&canon) else {
                continue;
            };
            let is_dir = md.is_dir();
            if !is_dir && !md.is_file() {
                continue;
            }
            let modified = system_time_ms(md.modified().ok());
            entries.push(StorageEntry { name, size: if is_dir { 0 } else { md.len() }, modified, is_dir });
        }
        Ok(entries)
    }

    pub fn open_read(&self, path: &str) -> std::result::Result<StorageReadFile, StorageError> {
        let resolved = self.resolve_existing(path, false)?;
        let md = std::fs::metadata(&resolved).map_err(|_| StorageError::NotFound)?;
        if !md.is_file() {
            return Err(StorageError::NotFound);
        }
        Ok(StorageReadFile {
            source: FileSource::Path(resolved),
            size: md.len(),
            modified: system_time_ms(md.modified().ok()),
        })
    }

    pub fn write(
        &self,
        path: &str,
        staged: &Path,
        modified: Option<i64>,
    ) -> std::result::Result<StorageWriteDone, StorageError> {
        let dest = self.resolve_target(path)?;
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| StorageError::Failed(e.to_string()))?;
        }
        let canon_root = self.canonical_root()?;
        if let Some(parent) = dest.parent() {
            let canon_parent = std::fs::canonicalize(parent).map_err(|_| StorageError::Denied)?;
            self.ensure_inside(&canon_root, &canon_parent)?;
        }
        if std::fs::rename(staged, &dest).is_err() {
            std::fs::copy(staged, &dest).map_err(|e| StorageError::Failed(e.to_string()))?;
        }
        if let Some(ms) = modified.filter(|m| *m >= 0)
            && let Some(st) = UNIX_EPOCH.checked_add(Duration::from_millis(ms as u64))
            && let Ok(f) = std::fs::OpenOptions::new().write(true).open(&dest)
        {
            let _ = f.set_modified(st);
        }
        let md = std::fs::metadata(&dest).map_err(|e| StorageError::Failed(e.to_string()))?;
        Ok(StorageWriteDone {
            size: md.len(),
            modified: modified.unwrap_or_else(|| system_time_ms(md.modified().ok())),
        })
    }

    pub fn mkdir(&self, path: &str) -> std::result::Result<(), StorageError> {
        let dest = self.resolve_target(path)?;
        std::fs::create_dir_all(&dest).map_err(|e| StorageError::Failed(e.to_string()))?;
        let canon_root = self.canonical_root()?;
        let canon = std::fs::canonicalize(&dest).map_err(|_| StorageError::Denied)?;
        self.ensure_inside(&canon_root, &canon)
    }

    pub fn rename(&self, from: &str, to: &str) -> std::result::Result<(), StorageError> {
        let src = self.resolve_existing(from, false)?;
        let dst = self.resolve_target(to)?;
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(|e| StorageError::Failed(e.to_string()))?;
            let canon_root = self.canonical_root()?;
            let canon_parent = std::fs::canonicalize(parent).map_err(|_| StorageError::Denied)?;
            self.ensure_inside(&canon_root, &canon_parent)?;
        }
        std::fs::rename(&src, &dst).map_err(|e| StorageError::Failed(e.to_string()))
    }

    pub fn delete(&self, path: &str, confirmed: bool) -> std::result::Result<(), StorageError> {
        let target = self.resolve_existing(path, false)?;
        let md = std::fs::metadata(&target).map_err(|_| StorageError::NotFound)?;
        if let Some(trash_dir) = &self.trash_dir
            && md.is_file()
            && is_media_extension(&target)
        {
            std::fs::create_dir_all(trash_dir).map_err(|e| StorageError::Failed(e.to_string()))?;
            let name = target
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "trashed".into());
            let dest = trash_dir.join(name);
            if std::fs::rename(&target, &dest).is_err() {
                std::fs::copy(&target, &dest).map_err(|e| StorageError::Failed(e.to_string()))?;
                std::fs::remove_file(&target).map_err(|e| StorageError::Failed(e.to_string()))?;
            }
            return Ok(());
        }
        if !confirmed {
            return Err(StorageError::Denied);
        }
        if md.is_dir() {
            std::fs::remove_dir_all(&target).map_err(|e| StorageError::Failed(e.to_string()))
        } else {
            std::fs::remove_file(&target).map_err(|e| StorageError::Failed(e.to_string()))
        }
    }
}

fn is_media_extension(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|ext| {
        matches!(
            ext.to_ascii_lowercase().as_str(),
            "jpg"
                | "jpeg"
                | "png"
                | "webp"
                | "gif"
                | "heic"
                | "mp4"
                | "mov"
                | "mkv"
                | "mp3"
                | "m4a"
                | "wav"
                | "flac"
                | "ogg"
        )
    })
}

fn system_time_ms(t: Option<SystemTime>) -> i64 {
    t.and_then(|st| st.duration_since(UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_storage_rejects_traversal_and_symlinks_outside_root() {
        let root_dir = tempfile::tempdir().unwrap();
        let outside_dir = tempfile::tempdir().unwrap();
        std::fs::write(outside_dir.path().join("secret.txt"), b"top secret").unwrap();
        std::fs::create_dir_all(root_dir.path().join("Documents")).unwrap();
        std::fs::write(root_dir.path().join("Documents/notes.txt"), b"hello world").unwrap();

        let storage = FolderStorage::new(root_dir.path());

        // Valid operations inside root succeed.
        let root_entries = storage.list("").unwrap();
        assert_eq!(root_entries.len(), 1);
        assert_eq!(root_entries[0].name, "Documents");
        assert!(root_entries[0].is_dir);

        // Path traversal and absolute paths are rejected.
        for bad in [
            "..",
            "../secret.txt",
            "Documents/../../secret.txt",
            "/etc/passwd",
            "/data/data",
            "Documents\\notes.txt",
        ] {
            assert!(storage.list(bad).is_err(), "{bad}");
            assert!(storage.open_read(bad).is_err(), "{bad}");
            assert!(storage.mkdir(bad).is_err(), "{bad}");
            assert!(storage.delete(bad, true).is_err(), "{bad}");
        }

        // Non-media file deletion requires confirmation (`confirmed = true`);
        // media files move to trash when a trash dir is configured.
        let trash_dir = tempfile::tempdir().unwrap();
        let storage = storage.with_trash_dir(trash_dir.path());
        assert_eq!(storage.delete("Documents/notes.txt", false), Err(StorageError::Denied));
        std::fs::write(root_dir.path().join("Documents/photo.jpg"), b"jpeg").unwrap();
        storage.delete("Documents/photo.jpg", false).unwrap();
        assert!(trash_dir.path().join("photo.jpg").exists());
    }
}
