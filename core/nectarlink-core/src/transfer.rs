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
        ErrorBody, FileEntry, FilesAccept, FilesOffer, StreamHeader, clip, files, is_valid_file_name,
        is_valid_folder, mirror, types,
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
    /// Anything else; for logs, never holds file names.
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferState {
    /// Waiting for the other device: to connect (sending), or to continue
    /// after the connection dropped (receiving; the sender resumes it).
    Waiting,
    Running,
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
/// characters it rejects, or trailing dots and spaces.
fn storable_name(name: &str) -> String {
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
    (2..).map(|n| dir.join(format!("{stem} ({n}){ext}"))).find(|p| !p.exists()).expect("some name is free")
}

fn new_id() -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    (0..24).map(|_| ALPHABET[rand::random_range(0..ALPHABET.len())] as char).collect()
}

/// Publishes a transfer's progress, at most every [`PROGRESS_EVERY`].
struct Reporter {
    shared: Arc<Shared>,
    transfer: Transfer,
    last: Instant,
}

impl Reporter {
    fn new(shared: Arc<Shared>, transfer: Transfer) -> Reporter {
        Reporter { shared, transfer, last: Instant::now() - PROGRESS_EVERY }
    }

    fn set(&mut self, state: TransferState) {
        self.transfer.state = state;
        self.shared.emit(NodeEvent::Transfer(self.transfer.clone()));
        self.last = Instant::now();
    }

    fn progress(&mut self, done: u64) {
        self.transfer.done = done;
        if self.last.elapsed() >= PROGRESS_EVERY {
            self.set(self.transfer.state.clone());
        }
    }
}

// ---- Sending ----

/// Starts sending files; progress and the outcome arrive as
/// [`NodeEvent::Transfer`]. Returns the transfer's ID.
pub(crate) async fn send(shared: &Arc<Shared>, peer: DeviceId, files: Vec<OutgoingFile>) -> Result<String> {
    send_inner(shared, peer, files, false, Vec::new()).await
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
    send_inner(shared, peer, vec![file], true, clean_markers).await
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

async fn send_inner(
    shared: &Arc<Shared>,
    peer: DeviceId,
    files: Vec<OutgoingFile>,
    recording: bool,
    markers: Vec<RecordingMarker>,
) -> Result<String> {
    if files.is_empty() || files.len() > files::MAX_FILES || (recording && files.len() != 1) {
        return Err(Error::Protocol("send between 1 and 5000 files".into()));
    }
    if !shared.store.is_paired(&peer)? {
        return Err(Error::NotPaired);
    }
    let toggle = if recording { RECORDINGS_TOGGLE } else { TOGGLE };
    if !shared.toggle_on(&peer, toggle) {
        return Err(Error::Denied);
    }
    let mut opened = Vec::with_capacity(files.len());
    for file in files {
        if file.folder.as_deref().is_some_and(|f| !is_valid_folder(f)) {
            return Err(Error::Protocol("invalid folder name".into()));
        }
        let handle = match file.source {
            FileSource::Path(path) => std::fs::File::open(path)?,
            FileSource::File(handle) => handle,
        };
        let entry = FileEntry {
            name: safe_file_name(&file.name),
            size: handle.metadata()?.len(),
            folder: file.folder,
        };
        opened.push((entry, tokio::fs::File::from_std(handle)));
    }
    let entries: Vec<FileEntry> = opened.iter().map(|(entry, _)| entry.clone()).collect();
    let id = new_id();
    // The whole offer has to fit in one frame.
    let offer = Envelope::new(
        files::OFFER,
        &FilesOffer { id: id.clone(), files: entries.clone(), recording, markers: markers.clone() },
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
    };
    let id = transfer.id.clone();
    // The user's cancel only: shutting down interrupts a transfer (so it
    // can resume later), it doesn't cancel it.
    let cancel = CancellationToken::new();
    shared.register_transfer(&id, cancel.clone());
    let mut reporter = Reporter::new(shared.clone(), transfer);
    reporter.set(TransferState::Waiting);
    tokio::spawn(async move {
        let state = send_until_done(&mut reporter, opened, &cancel).await;
        reporter.set(state);
        reporter.shared.unregister_transfer(&reporter.transfer.id);
    });
    Ok(id)
}

enum Attempt {
    Done,
    Cancelled,
    Fatal(TransferFailure),
    /// The connection failed; try again once it's back.
    Retry,
}

async fn send_until_done(
    reporter: &mut Reporter,
    mut files: Vec<(FileEntry, tokio::fs::File)>,
    cancel: &CancellationToken,
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
                _ = tokio::time::sleep(Duration::from_secs(1)) => continue,
            }
        };
        match attempt(reporter, &session, &mut files, cancel).await {
            Attempt::Done => return TransferState::Done { saved: Vec::new() },
            Attempt::Cancelled => return TransferState::Cancelled,
            Attempt::Fatal(failure) => return TransferState::Failed(failure),
            Attempt::Retry => {
                // Made it this far: give the device a fresh window to come back.
                give_up_at = Instant::now() + RETRY_FOR;
                tracing::debug!(id = %reporter.transfer.id, "transfer interrupted; will resume");
                reporter.set(TransferState::Waiting);
                tokio::select! {
                    _ = cancel.cancelled() => return TransferState::Cancelled,
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                }
            }
        }
    }
}

async fn attempt(
    reporter: &mut Reporter,
    session: &Session,
    files: &mut [(FileEntry, tokio::fs::File)],
    cancel: &CancellationToken,
) -> Attempt {
    let Ok((mut send, mut recv)) = session.conn.open_bi().await else { return Attempt::Retry };
    let header = StreamHeader { svc: files::SERVICE.into(), op: files::OP_SEND.into(), v: files::VERSION };
    let offer = FilesOffer {
        id: reporter.transfer.id.clone(),
        files: files.iter().map(|(entry, _)| entry.clone()).collect(),
        recording: reporter.transfer.recording,
        markers: reporter.transfer.markers.clone(),
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
    let have = match read_reply(&mut recv, cancel).await {
        Reply::Envelope(env) if env.t == files::ACCEPT => match env.body::<FilesAccept>() {
            Ok(accept) if accept.have.len() == files.len() => accept.have,
            _ => return Attempt::Fatal(TransferFailure::Other("bad answer to the offer".into())),
        },
        Reply::Envelope(env) => return Attempt::Fatal(refusal(&env)),
        Reply::Cancelled => {
            let _ = send.reset(VarInt::from_u32(files::CANCELLED));
            return Attempt::Cancelled;
        }
        Reply::Failed => return Attempt::Retry,
    };

    reporter.set(TransferState::Running);
    let mut done: u64 = files.iter().zip(&have).map(|((entry, _), have)| (*have).min(entry.size)).sum();
    reporter.progress(done);
    // What's left of each file, read on a thread of its own that keeps a
    // few blocks ready; it stops when this attempt ends and drops them.
    let mut plan = Vec::with_capacity(files.len());
    for ((entry, file), have) in files.iter().zip(&have) {
        let start = (*have).min(entry.size);
        let Ok(clone) = file.try_clone().await else {
            return Attempt::Fatal(TransferFailure::Other("can't read a file".into()));
        };
        plan.push((clone.into_std().await, start, entry.size - start));
    }
    let (blocks, mut ready) = tokio::sync::mpsc::channel(READ_AHEAD);
    tokio::task::spawn_blocking(move || read_ahead(plan, &blocks));
    loop {
        let block = tokio::select! {
            _ = cancel.cancelled() => {
                let _ = send.reset(VarInt::from_u32(files::CANCELLED));
                return Attempt::Cancelled;
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
    if send.finish().is_err() {
        return Attempt::Retry;
    }
    match read_reply(&mut recv, cancel).await {
        Reply::Envelope(env) if env.t == files::DONE => Attempt::Done,
        Reply::Envelope(env) => Attempt::Fatal(refusal(&env)),
        Reply::Cancelled => Attempt::Cancelled,
        Reply::Failed => Attempt::Retry,
    }
}

/// What a stream reset with `code` by the receiver means.
fn stopped(code: VarInt) -> Attempt {
    match u32::try_from(code.into_inner()).unwrap_or(u32::MAX) {
        files::CANCELLED => Attempt::Cancelled,
        c if c == ErrorCode::Busy.close_code() => Attempt::Fatal(TransferFailure::NoSpace),
        c if c == ErrorCode::Denied.close_code() => Attempt::Fatal(TransferFailure::Denied),
        _ => Attempt::Retry,
    }
}

fn refusal(env: &Envelope) -> TransferFailure {
    match env.body::<ErrorBody>().map(|e| e.code) {
        Ok(ErrorCode::Denied) => TransferFailure::Denied,
        Ok(ErrorCode::Busy) => TransferFailure::NoSpace,
        Ok(code) => TransferFailure::Other(format!("refused: {code}")),
        Err(_) => TransferFailure::Other(format!("unexpected {}", env.t)),
    }
}

enum Reply {
    Envelope(Envelope),
    Cancelled,
    Failed,
}

async fn read_reply(recv: &mut RecvStream, cancel: &CancellationToken) -> Reply {
    let frame = tokio::select! {
        _ = cancel.cancelled() => return Reply::Cancelled,
        frame = tokio::time::timeout(REPLY_TIMEOUT, read_frame(recv)) => frame,
    };
    match frame {
        Ok(Ok(Some(bytes))) => Envelope::from_cbor(&bytes).map_or(Reply::Failed, Reply::Envelope),
        Ok(Err(nectarlink_protocol::ProtocolError::Io(e))) if is_cancel(&e) => Reply::Cancelled,
        _ => Reply::Failed,
    }
}

/// Whether an I/O error from a stream is the other side cancelling.
fn is_cancel(e: &std::io::Error) -> bool {
    e.get_ref().and_then(|inner| inner.downcast_ref::<ReadError>()).is_some_and(
        |r| matches!(r, ReadError::Reset(code) if code.into_inner() == u64::from(files::CANCELLED)),
    )
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
        if !shared.local_capabilities().iter().any(|c| c == RECORDER) {
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
    let mut have = Vec::with_capacity(offer.files.len());
    for (i, file) in offer.files.iter().enumerate() {
        let len = tokio::fs::metadata(part(i)).await.map(|m| m.len()).unwrap_or(0);
        have.push(len.min(file.size));
    }
    let Ok(accept) = Envelope::new(files::ACCEPT, &FilesAccept { have: have.clone() }) else { return };
    if write_frame(&mut send, &accept.to_cbor()).await.is_err() {
        return;
    }

    let cancel = CancellationToken::new();
    shared.register_transfer(&offer.id, cancel.clone());
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
    };
    let mut reporter = Reporter::new(shared.clone(), transfer);
    reporter.set(TransferState::Running);
    let state = match receive_bytes(&mut reporter, &offer, &have, &part, &mut send, &mut recv, &cancel).await
    {
        Ok(()) => match store(&shared, &offer, &part, &dir).await {
            Ok(saved) => {
                let _ = write_frame(&mut send, &Envelope::empty(files::DONE).to_cbor()).await;
                let _ = send.finish();
                // Let the confirmation reach the sender before the stream goes.
                let _ = tokio::time::timeout(Duration::from_secs(2), send.stopped()).await;
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
                let _ = tokio::fs::remove_dir_all(&dir).await;
                if let Some(parent) = dir.parent() {
                    let _ = tokio::fs::remove_dir(parent).await;
                }
            }
            state
        }
    };
    reporter.set(state);
    shared.unregister_transfer(&offer.id);
}

#[allow(clippy::too_many_arguments)]
/// Reads each file's remaining bytes, from where the receiver is, in
/// blocks; stops early when the sending side is gone.
fn read_ahead(
    plan: Vec<(std::fs::File, u64, u64)>,
    blocks: &tokio::sync::mpsc::Sender<std::result::Result<Bytes, TransferFailure>>,
) {
    use std::io::{Read, Seek, SeekFrom};
    for (mut file, start, mut left) in plan {
        if let Err(e) = file.seek(SeekFrom::Start(start)) {
            let failure = TransferFailure::Other(format!("can't read a file: {}", e.kind()));
            let _ = blocks.blocking_send(Err(failure));
            return;
        }
        while left > 0 {
            let mut block = vec![0u8; BLOCK.min(usize::try_from(left).unwrap_or(usize::MAX))];
            if let Err(e) = file.read_exact(&mut block) {
                let failure = if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    TransferFailure::Other("a file got shorter while sending".into())
                } else {
                    TransferFailure::Other(format!("can't read a file: {}", e.kind()))
                };
                let _ = blocks.blocking_send(Err(failure));
                return;
            }
            left -= block.len() as u64;
            if blocks.blocking_send(Ok(Bytes::from(block))).is_err() {
                return;
            }
        }
    }
}

async fn receive_bytes(
    reporter: &mut Reporter,
    offer: &FilesOffer,
    have: &[u64],
    part: &impl Fn(usize) -> PathBuf,
    send: &mut SendStream,
    recv: &mut RecvStream,
    cancel: &CancellationToken,
) -> std::result::Result<(), TransferState> {
    let mut done: u64 = have.iter().sum();
    for (i, (file, have)) in offer.files.iter().zip(have).enumerate() {
        let mut out =
            tokio::fs::OpenOptions::new().create(true).append(true).open(part(i)).await.map_err(|e| {
                TransferState::Failed(TransferFailure::Other(format!("can't write: {}", e.kind())))
            })?;
        // A part file longer than the file (it changed) starts over.
        if *have < out.metadata().await.map(|m| m.len()).unwrap_or(0) {
            out.set_len(*have).await.ok();
        }
        // Gathered in memory and written in large pieces. Progress counts
        // what's on disk, which is also where a resumed transfer continues.
        let mut pending: Vec<u8> = Vec::with_capacity(WRITE_BUFFER);
        let mut left = file.size - have;
        while left > 0 {
            let max = CHUNK.min(usize::try_from(left).unwrap_or(usize::MAX));
            let chunk = tokio::select! {
                _ = cancel.cancelled() => {
                    let _ = recv.stop(VarInt::from_u32(files::CANCELLED));
                    let _ = send.reset(VarInt::from_u32(files::CANCELLED));
                    return Err(TransferState::Cancelled);
                }
                chunk = recv.read_chunk(max) => chunk,
            };
            let bytes = match chunk {
                Ok(Some(bytes)) => bytes,
                Err(ReadError::Reset(code)) if code.into_inner() == u64::from(files::CANCELLED) => {
                    return Err(TransferState::Cancelled);
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
            left -= bytes.len() as u64;
            done += bytes.len() as u64;
            pending.extend_from_slice(&bytes);
            if pending.len() >= WRITE_BUFFER || left == 0 {
                write_out(&mut out, &mut pending, recv).await?;
                reporter.progress(done);
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
        std::fs::create_dir_all(&target)?;
        // A sent folder lands under a name of its own: `Trip`, or
        // `Trip (2)` when there already is one.
        let mut folders: HashMap<String, PathBuf> = HashMap::new();
        let mut saved = Vec::with_capacity(entries.len());
        for (entry, part) in entries.iter().zip(parts) {
            let at = match entry.folder.as_deref() {
                None => target.clone(),
                Some(folder) => {
                    let mut names = folder.split('/').map(storable_name);
                    let top = names.next().unwrap_or_else(|| "Folder".into());
                    let mut at = match folders.get(&top) {
                        Some(at) => at.clone(),
                        None => {
                            let at = unique_path(&target, &top);
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
            if std::fs::rename(&part, &to).is_err() {
                // Another volume: copy, then drop the part file.
                std::fs::copy(&part, &to)?;
                std::fs::remove_file(&part)?;
            }
            if entry.folder.is_none() {
                saved.push(to);
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        if let Some(parent) = dir.parent() {
            let _ = std::fs::remove_dir(parent);
        }
        Ok(saved)
    })
    .await
    .map_err(std::io::Error::other)?
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
}
