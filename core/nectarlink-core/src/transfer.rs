// SPDX-License-Identifier: MPL-2.0
//! The files service (docs/protocol/files.md): sending files to a paired
//! device on a stream of their own, receiving them, progress, cancelling,
//! and resuming after a dropped connection.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use iroh::endpoint::{ReadError, RecvStream, SendStream, VarInt, WriteError};
use nectarlink_protocol::{
    DeviceId, Envelope, ErrorCode,
    messages::{
        ErrorBody, FileEntry, FilesAccept, FilesOffer, StreamHeader, clip, files, is_valid_file_name, types,
    },
    read_frame, write_frame,
};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use crate::{Error, Result, events::NodeEvent, node::Shared, session::Session};

/// The device toggle that allows files for a device.
pub(crate) const TOGGLE: &str = "files";

/// How long a sender keeps trying to reach the device before giving up.
const RETRY_FOR: Duration = Duration::from_secs(10 * 60);
/// Waiting for the other side's answer or confirmation.
const REPLY_TIMEOUT: Duration = Duration::from_secs(30);
/// Progress events, at most this often per transfer.
const PROGRESS_EVERY: Duration = Duration::from_millis(200);
/// Partly received files are kept this long for resuming.
const KEEP_PARTIAL: Duration = Duration::from_secs(60 * 60);
const CHUNK: usize = 256 * 1024;

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
    /// Received files are where `saved` says (one path per file).
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
    /// File names, in order.
    pub names: Vec<String>,
    /// Bytes in total and done so far.
    pub total: u64,
    pub done: u64,
    pub state: TransferState,
}

/// Never prints file names (protocol v0 §11).
impl std::fmt::Debug for Transfer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Transfer")
            .field("id", &self.id)
            .field("direction", &self.direction)
            .field("files", &self.names.len())
            .field("done", &self.done)
            .field("total", &self.total)
            .field("state", &self.state)
            .finish()
    }
}

/// A file to send.
#[derive(Debug)]
pub struct OutgoingFile {
    /// The name the other device sees (no path).
    pub name: String,
    pub source: FileSource,
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
    if files.is_empty() || files.len() > files::MAX_FILES {
        return Err(Error::Protocol("send between 1 and 1000 files".into()));
    }
    if !shared.store.is_paired(&peer)? {
        return Err(Error::NotPaired);
    }
    if !shared.toggle_on(&peer, TOGGLE) {
        return Err(Error::Denied);
    }
    let mut opened = Vec::with_capacity(files.len());
    for file in files {
        let handle = match file.source {
            FileSource::Path(path) => std::fs::File::open(path)?,
            FileSource::File(handle) => handle,
        };
        let size = handle.metadata()?.len();
        opened.push((safe_file_name(&file.name), size, tokio::fs::File::from_std(handle)));
    }
    let transfer = Transfer {
        id: new_id(),
        device: peer,
        direction: Direction::Outgoing,
        names: opened.iter().map(|(name, ..)| name.clone()).collect(),
        total: opened.iter().map(|(_, size, _)| *size).fold(0, u64::saturating_add),
        done: 0,
        state: TransferState::Waiting,
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
    mut files: Vec<(String, u64, tokio::fs::File)>,
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
    files: &mut [(String, u64, tokio::fs::File)],
    cancel: &CancellationToken,
) -> Attempt {
    let Ok((mut send, mut recv)) = session.conn.open_bi().await else { return Attempt::Retry };
    let header = StreamHeader { svc: files::SERVICE.into(), op: files::OP_SEND.into(), v: files::VERSION };
    let offer = FilesOffer {
        id: reporter.transfer.id.clone(),
        files: files.iter().map(|(name, size, _)| FileEntry { name: name.clone(), size: *size }).collect(),
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
    let mut done: u64 = files.iter().zip(&have).map(|((_, size, _), have)| (*have).min(*size)).sum();
    reporter.progress(done);
    let mut buf = vec![0u8; CHUNK];
    for ((_, size, file), have) in files.iter_mut().zip(&have) {
        let start = (*have).min(*size);
        if file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
            return Attempt::Fatal(TransferFailure::Other("can't read a file".into()));
        }
        let mut left = *size - start;
        while left > 0 {
            let want = buf.len().min(usize::try_from(left).unwrap_or(usize::MAX));
            let read = tokio::select! {
                _ = cancel.cancelled() => {
                    let _ = send.reset(VarInt::from_u32(files::CANCELLED));
                    return Attempt::Cancelled;
                }
                read = file.read(&mut buf[..want]) => read,
            };
            let n = match read {
                Ok(0) => {
                    return Attempt::Fatal(TransferFailure::Other("a file got shorter while sending".into()));
                }
                Ok(n) => n,
                Err(e) => {
                    return Attempt::Fatal(TransferFailure::Other(format!(
                        "can't read a file: {}",
                        e.kind()
                    )));
                }
            };
            let written = tokio::select! {
                _ = cancel.cancelled() => {
                    let _ = send.reset(VarInt::from_u32(files::CANCELLED));
                    return Attempt::Cancelled;
                }
                written = send.write_all(&buf[..n]) => written,
            };
            if let Err(e) = written {
                return match e {
                    WriteError::Stopped(code) => stopped(code),
                    _ => Attempt::Retry,
                };
            }
            left -= n as u64;
            done += n as u64;
            reporter.progress(done);
        }
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
    if !shared.toggle_on(&peer, TOGGLE) {
        return refuse(&mut send, ErrorCode::Denied, "files are off for this device").await;
    }
    let dir = shared.incoming_dir().join(&offer.id);
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
        names: offer.files.iter().map(|f| f.name.clone()).collect(),
        total: offer.total_size(),
        done: have.iter().sum(),
        state: TransferState::Running,
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
            }
            state
        }
    };
    reporter.set(state);
    shared.unregister_transfer(&offer.id);
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
                Ok(None) => return Err(TransferState::Waiting),
                Err(ReadError::Reset(code)) if code.into_inner() == u64::from(files::CANCELLED) => {
                    return Err(TransferState::Cancelled);
                }
                // The connection dropped: the sender resumes when it's back.
                Err(_) => return Err(TransferState::Waiting),
            };
            if let Err(e) = out.write_all(&bytes).await {
                let no_space = e.kind() == std::io::ErrorKind::StorageFull;
                let code = if no_space { ErrorCode::Busy } else { ErrorCode::Internal };
                let _ = recv.stop(VarInt::from_u32(code.close_code()));
                return Err(TransferState::Failed(if no_space {
                    TransferFailure::NoSpace
                } else {
                    TransferFailure::Other(format!("can't write: {}", e.kind()))
                }));
            }
            left -= bytes.len() as u64;
            done += bytes.len() as u64;
            reporter.progress(done);
        }
        out.flush().await.ok();
    }
    Ok(())
}

/// Moves complete files where the user finds them; returns their paths.
async fn store(
    shared: &Shared,
    offer: &FilesOffer,
    part: &impl Fn(usize) -> PathBuf,
    dir: &Path,
) -> std::io::Result<Vec<PathBuf>> {
    let target = shared.downloads_dir.clone();
    let names: Vec<String> = offer.files.iter().map(|f| f.name.clone()).collect();
    let parts: Vec<PathBuf> = (0..names.len()).map(part).collect();
    let dir = dir.to_owned();
    tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&target)?;
        let mut saved = Vec::with_capacity(names.len());
        for (name, part) in names.iter().zip(parts) {
            let to = unique_path(&target, name);
            if std::fs::rename(&part, &to).is_err() {
                // Another volume: copy, then drop the part file.
                std::fs::copy(&part, &to)?;
                std::fs::remove_file(&part)?;
            }
            saved.push(to);
        }
        let _ = std::fs::remove_dir_all(&dir);
        Ok(saved)
    })
    .await
    .map_err(std::io::Error::other)?
}

/// Deletes partly received files nobody resumed in time.
pub(crate) fn clean_incoming(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|t| t.elapsed().unwrap_or_default() > KEEP_PARTIAL);
        if stale {
            let _ = std::fs::remove_dir_all(entry.path());
        }
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
    fn ids_are_valid() {
        assert!(nectarlink_protocol::messages::is_valid_transfer_id(&new_id()));
        assert_ne!(new_id(), new_id());
    }
}
