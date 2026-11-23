// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    collections::{HashMap, HashSet},
    io::{self, SeekFrom},
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use iroh::{PublicKey, SecretKey};
use nectarlink_core::FileSource;
use serde::{Deserialize, Serialize};
use tokio::{
    fs::File,
    io::{AsyncSeekExt, AsyncWriteExt, BufWriter},
    sync::Mutex,
    time::Instant,
};

use super::{
    Event, Report, Route, Selection,
    protocol::{self, COMPLETE, COMPLETE_INDEX, Cipher, ERROR_INDEX, PULL, PUSH, open, read_index, seal},
};

pub(super) const WINDOW: usize = 8;

#[derive(PartialEq, Serialize, Deserialize)]
struct FileInfo {
    name: String,
    size: u64,
}

enum Offer {
    Send { file: Arc<std::fs::File>, info: FileInfo },
    Receive(PathBuf),
}

struct Incoming {
    info: FileInfo,
    file: BufWriter<File>,
    position: u64,
    temp: tempfile::TempPath,
    chunks: Chunks,
}

pub(super) struct Session {
    host: SecretKey,
    ticket: SecretKey,
    offer: Offer,
    deadline: Instant,
    limit: usize,
    guests: Mutex<Guests>,
    report: Report,
}

#[derive(Default)]
struct Guests {
    entries: HashMap<PublicKey, Arc<Mutex<Guest>>>,
    completed: usize,
    failed: usize,
}

struct Guest {
    route: Route,
    key: [u8; 32],
    ack: Bytes,
    cipher: Cipher,
    sent: Chunks,
    incoming: Option<Incoming>,
    finished: bool,
    saved: Option<PathBuf>,
    error: Option<Bytes>,
}

#[derive(Default)]
struct Chunks {
    seen: HashSet<usize>,
    bytes: u64,
}

impl Chunks {
    fn record(&mut self, index: usize, length: usize, total: u64, report: Option<&Report>) {
        if self.seen.insert(index) {
            let before = self.bytes;
            self.bytes += length as u64;
            if let Some(report) = report
                && (before == 0 || before * 100 / total != self.bytes * 100 / total)
            {
                report(Event::Progress(self.bytes, total));
            }
        }
    }
}

impl Session {
    pub(super) async fn new(
        selection: Selection,
        host: SecretKey,
        ticket: SecretKey,
        deadline: Instant,
        limit: usize,
        report: Report,
    ) -> Result<Self> {
        let offer = match selection {
            Selection::Send { source, name } => {
                let (file, name) = match source {
                    FileSource::Path(path) => {
                        let file = File::open(&path).await.context("Could not open the selected file.")?;
                        let name =
                            name.or_else(|| path.file_name().map(|name| name.to_string_lossy().into_owned()));
                        (file, name)
                    }
                    FileSource::File(file) => (File::from_std(file), name),
                };
                let metadata = file.metadata().await?;
                let size = metadata.len();
                if !metadata.is_file() || !Route::Lan.supports_size(size) {
                    bail!("Choose a regular file supported by AnySend.");
                }
                let name = name.context("The file has no name.")?;
                Offer::Send { file: Arc::new(file.into_std().await), info: FileInfo { name, size } }
            }
            Selection::Receive(directory) => {
                if !tokio::fs::metadata(&directory).await.is_ok_and(|meta| meta.is_dir()) {
                    bail!("Choose an existing destination folder.");
                }
                Offer::Receive(directory)
            }
        };
        Ok(Self { host, ticket, offer, deadline, limit, guests: Mutex::default(), report })
    }

    pub(super) fn probe(&self, challenge: &[u8]) -> Bytes {
        Bytes::copy_from_slice(&self.host.sign(&[protocol::PROBE, challenge].concat()).to_bytes())
    }

    async fn claim(&self, route: Route, id: PublicKey, bytes: &[u8]) -> Vec<Bytes> {
        let denied = || vec![Bytes::from_static(&[0])];
        let Some(key) = protocol::verify_claim(self.host.public(), self.ticket.public(), id, route, bytes)
        else {
            return denied();
        };
        let mut guests = self.guests.lock().await;
        if Instant::now() >= self.deadline {
            return denied();
        }
        if let Some(guest) = guests.entries.get(&id).cloned() {
            drop(guests);
            let guest = guest.lock().await;
            return if guest.route == route && guest.key == key { vec![guest.ack.clone()] } else { denied() };
        }
        if guests.entries.len() >= self.limit {
            return denied();
        }
        let Ok((cipher, ack)) = protocol::accept_claim(&self.host, &self.ticket, id, route, key) else {
            return denied();
        };
        guests.entries.insert(
            id,
            Arc::new(Mutex::new(Guest {
                route,
                key,
                ack: ack.clone(),
                cipher,
                sent: Chunks::default(),
                incoming: None,
                finished: false,
                saved: None,
                error: None,
            })),
        );
        if guests.entries.len() == 1 {
            (self.report)(Event::Started(route));
        }
        self.report_guests(&guests);
        vec![ack]
    }

    pub(super) async fn handle(&self, route: Route, id: PublicKey, bytes: &[u8]) -> Vec<Bytes> {
        if Instant::now() >= self.deadline {
            return Vec::new();
        }
        if let Some((&1, claim)) = bytes.split_first() {
            return self.claim(route, id, claim).await;
        }
        let guest = self.guests.lock().await.entries.get(&id).cloned();
        let Some(guest) = guest else { return Vec::new() };
        let mut guest = guest.lock().await;
        if guest.route != route || Instant::now() >= self.deadline {
            return Vec::new();
        }
        let was_finished = guest.finished;
        let result = guest.packet(self, bytes).await;
        if Instant::now() >= self.deadline {
            return Vec::new();
        }
        match result {
            Ok(replies) => {
                if !was_finished && guest.finished {
                    let saved = guest.saved.take();
                    let mut guests = self.guests.lock().await;
                    guests.completed += 1;
                    if let Some(path) = &saved {
                        (self.report)(Event::Received(path.clone()));
                    }
                    self.report_guests(&guests);
                    if guests.completed == self.limit {
                        (self.report)(Event::Finished(saved.filter(|_| self.limit == 1)));
                    }
                }
                replies
            }
            Err(error) => {
                let message = error.to_string();
                guest.incoming = None;
                let reply = seal(&guest.cipher, ERROR_INDEX, message.as_bytes()).ok();
                guest.error = reply.clone();
                let mut guests = self.guests.lock().await;
                guests.failed += 1;
                self.report_guests(&guests);
                if guests.completed + guests.failed == self.limit {
                    (self.report)(Event::Failed(message));
                }
                reply.into_iter().collect()
            }
        }
    }

    fn report_guests(&self, guests: &Guests) {
        (self.report)(Event::Guests {
            connected: guests.entries.len(),
            completed: guests.completed,
            failed: guests.failed,
        });
    }

    fn progress(&self) -> Option<&Report> {
        (self.limit == 1).then_some(&self.report)
    }
}

impl Guest {
    async fn packet(&mut self, session: &Session, bytes: &[u8]) -> Result<Vec<Bytes>> {
        let Some((&kind, data)) = bytes.split_first() else { return Ok(Vec::new()) };
        if let Some(error) = &self.error {
            return Ok(vec![error.clone()]);
        }
        match kind {
            2 if data.len() == 36 => {
                let Some(range) = open(&self.cipher, data, PULL) else { return Ok(Vec::new()) };
                let first = read_index(&range).expect("8-byte request");
                let count = read_index(&range[4..]).expect("8-byte request") as usize;
                self.read(session, first, count).await
            }
            3 if let Offer::Receive(directory) = &session.offer
                && let Some((header, frame)) = data.split_at_checked(4)
                && let Some(plaintext) = open(&self.cipher, frame, &[PUSH, header].concat()) =>
            {
                let index = read_index(header).expect("4-byte index");
                if !self.finished {
                    self.write(directory, index, &plaintext, session).await?;
                }
                Ok(vec![seal(&self.cipher, index, &[])?])
            }
            4 if let Offer::Send { info, .. } = &session.offer
                && data.len() == 28
                && open(&self.cipher, data, COMPLETE).is_some()
                && self.sent.bytes == info.size =>
            {
                self.finished = true;
                Ok(vec![seal(&self.cipher, COMPLETE_INDEX, &[])?])
            }
            _ => Ok(Vec::new()),
        }
    }

    async fn read(&mut self, session: &Session, index: u32, count: usize) -> Result<Vec<Bytes>> {
        if count == 0 || count > WINDOW {
            return Ok(Vec::new());
        }
        if index == 0 {
            let metadata = match &session.offer {
                Offer::Send { info, .. } => serde_json::to_vec(info)?,
                Offer::Receive(_) => serde_json::to_vec(&serde_json::json!({
                    "direction": "upload",
                    "maxSize": self.route.file_limit(),
                }))?,
            };
            return Ok(vec![seal(&self.cipher, 0, &metadata)?]);
        }
        let Offer::Send { file, info } = &session.offer else { return Ok(Vec::new()) };
        if !self.route.supports_size(info.size) {
            bail!("The relay file limit is 128 MiB.");
        }
        let chunk = self.route.chunk();
        let first = (index - 1) as usize;
        let end = first.saturating_add(count).min(info.size.div_ceil(chunk as u64) as usize);
        if first >= end {
            return Ok(Vec::new());
        }
        let offset = first as u64 * chunk as u64;
        let length = ((end - first) * chunk).min((info.size - offset) as usize);
        let file = file.clone();
        let cipher = self.cipher.clone();
        let replies = tokio::task::spawn_blocking(move || -> Result<Vec<Bytes>> {
            let mut bytes = vec![0; length];
            read_at(&file, &mut bytes, offset).context("Could not read the selected file.")?;
            bytes.chunks(chunk).enumerate().map(|(i, data)| seal(&cipher, index + i as u32, data)).collect()
        })
        .await??;
        for i in first..end {
            self.sent.record(i, chunk_len(info.size, i, chunk), info.size, session.progress());
        }
        Ok(replies)
    }

    async fn write(
        &mut self,
        directory: &Path,
        index: u32,
        plaintext: &[u8],
        session: &Session,
    ) -> Result<()> {
        let chunk = self.route.chunk();
        if index == 0 {
            let info: FileInfo = serde_json::from_slice(plaintext).context("Invalid file information.")?;
            if !self.route.supports_size(info.size) {
                bail!("The relay file limit is 128 MiB.");
            }
            if !safe_filename(&info.name) {
                bail!("The file name is invalid.");
            }
            if let Some(incoming) = &self.incoming {
                if incoming.info != info {
                    bail!("The file information changed during the upload.");
                }
                return Ok(());
            }
            let (file, temp) = tempfile::NamedTempFile::new_in(directory)
                .context("Could not create a file in the destination folder.")?
                .into_parts();
            self.incoming = Some(Incoming {
                file: BufWriter::with_capacity(1024 * 1024, File::from_std(file)),
                position: 0,
                temp,
                chunks: Chunks::default(),
                info,
            });
        } else {
            let Some(file) = self.incoming.as_mut() else {
                bail!("File information must be sent before the upload.");
            };
            let i = (index - 1) as usize;
            if i as u64 >= file.info.size.div_ceil(chunk as u64)
                || plaintext.len() != chunk_len(file.info.size, i, chunk)
            {
                bail!("The upload contains an invalid file chunk.");
            }
            if !file.chunks.seen.contains(&i) {
                let offset = i as u64 * chunk as u64;
                if file.position != offset {
                    file.file.seek(SeekFrom::Start(offset)).await?;
                }
                file.file.write_all(plaintext).await.context("Could not write the received file.")?;
                file.position = offset + plaintext.len() as u64;
                file.chunks.record(i, plaintext.len(), file.info.size, session.progress());
            }
        }
        if self.incoming.as_ref().is_some_and(|file| file.chunks.bytes == file.info.size) {
            let incoming = self.incoming.take().expect("completed upload");
            self.saved = Some(save(incoming, directory, session.deadline).await?);
            self.finished = true;
        }
        Ok(())
    }
}

#[cfg(unix)]
fn read_at(file: &std::fs::File, bytes: &mut [u8], offset: u64) -> io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(file, bytes, offset)
}

#[cfg(windows)]
fn read_at(file: &std::fs::File, mut bytes: &mut [u8], mut offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;

    while !bytes.is_empty() {
        match file.seek_read(bytes, offset) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(length) => {
                offset += length as u64;
                bytes = &mut bytes[length..];
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn chunk_len(size: u64, index: usize, chunk: usize) -> usize {
    (size - index as u64 * chunk as u64).min(chunk as u64) as usize
}

fn safe_filename(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && !name.ends_with(['.', ' '])
        && !name.chars().any(|c| c.is_control() || "\\/:<>|\"?*".contains(c))
}

async fn save(mut incoming: Incoming, directory: &Path, deadline: Instant) -> Result<PathBuf> {
    incoming.file.flush().await?;
    drop(incoming.file);
    let path = Path::new(&incoming.info.name);
    let stem = path.file_stem().unwrap_or(path.as_os_str()).to_string_lossy();
    let ext = path.extension().map(|ext| format!(".{}", ext.to_string_lossy())).unwrap_or_default();
    let mut temp = incoming.temp;
    for attempt in 0..1000 {
        if Instant::now() >= deadline {
            bail!("The AnySend link has expired.");
        }
        let destination = if attempt == 0 {
            directory.join(path)
        } else {
            directory.join(format!("{stem} ({attempt}){ext}"))
        };
        match temp.persist_noclobber(&destination) {
            Ok(()) => return Ok(destination),
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => temp = error.path,
            Err(error) => return Err(error.error).context("Could not save the received file."),
        }
    }
    bail!("Could not choose a free name in the destination folder.")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ring::{agreement, rand::SystemRandom};
    use tokio::task::JoinSet;

    use super::*;

    fn claim(host: &SecretKey, ticket: &SecretKey, guest: PublicKey, route: Route) -> Vec<u8> {
        let key = agreement::EphemeralPrivateKey::generate(&agreement::X25519, &SystemRandom::new())
            .unwrap()
            .compute_public_key()
            .unwrap();
        let signature = ticket.sign(
            &[
                b"AnySend browser v2".as_slice(),
                host.public().as_bytes(),
                guest.as_bytes(),
                &[route.code()],
                key.as_ref(),
            ]
            .concat(),
        );
        [&[1], key.as_ref(), signature.to_bytes().as_slice()].concat()
    }

    #[tokio::test]
    async fn guest_limit_is_shared_by_both_routes() {
        let directory = tempfile::tempdir().unwrap();
        let host = SecretKey::generate();
        let ticket = SecretKey::generate();
        let session = Arc::new(
            Session::new(
                Selection::Receive(directory.path().to_owned()),
                host.clone(),
                ticket.clone(),
                Instant::now() + Duration::from_secs(10),
                super::super::MAX_GUESTS as usize,
                Arc::new(|_| {}),
            )
            .await
            .unwrap(),
        );

        // Dropped acknowledgments can be retried without spending another slot.
        let guest = SecretKey::generate().public();
        let request = claim(&host, &ticket, guest, Route::Lan);
        let ack = session.handle(Route::Lan, guest, &request).await;
        assert_eq!(ack[0].len(), 97);
        assert_eq!(session.handle(Route::Lan, guest, &request).await, ack);
        assert_eq!(session.handle(Route::Relay, guest, &request).await, [Bytes::from_static(&[0])]);
        let other_guest = SecretKey::generate().public();
        assert_eq!(session.handle(Route::Lan, other_guest, &request).await, [Bytes::from_static(&[0])]);

        let mut tasks = JoinSet::new();
        for index in 0..100 {
            let route = if index % 2 == 0 { Route::Lan } else { Route::Relay };
            let guest = SecretKey::generate().public();
            let request = claim(&host, &ticket, guest, route);
            let session = session.clone();
            tasks.spawn(async move { session.handle(route, guest, &request).await[0].len() == 97 });
        }
        let mut accepted = 1;
        while let Some(result) = tasks.join_next().await {
            accepted += usize::from(result.unwrap());
        }
        assert_eq!(accepted, 100);
    }

    #[tokio::test]
    async fn expired_session_neither_claims_nor_serves_packets() {
        let directory = tempfile::tempdir().unwrap();
        let host = SecretKey::generate();
        let ticket = SecretKey::generate();
        let session = Session::new(
            Selection::Receive(directory.path().to_owned()),
            host.clone(),
            ticket.clone(),
            Instant::now(),
            1,
            Arc::new(|_| {}),
        )
        .await
        .unwrap();

        let guest = SecretKey::generate().public();
        let request = claim(&host, &ticket, guest, Route::Lan);
        assert!(session.handle(Route::Lan, guest, &request).await.is_empty());
        assert!(session.handle(Route::Relay, guest, &[2; 37]).await.is_empty());
    }
}
