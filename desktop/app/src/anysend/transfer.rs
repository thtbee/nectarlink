// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    io::SeekFrom,
    path::{Path, PathBuf},
    time::SystemTime,
};

use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use anyhow::{Context, Result, bail};
use bytes::Bytes;
use iroh::{PublicKey, SecretKey, Signature};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha512};
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
};

use super::{Event, Report, Route, Selection};

pub(super) const CHUNK: usize = 48 * 1024;
const WINDOW: usize = 16;
const MAX_SIZE: u64 = 128 * 1024 * 1024;
const REQUEST: &[u8] = b"AnySend file request v1";
const CLAIM: &[u8] = b"AnySend browser v1";
const ACK: &[u8] = b"AnySend sender v1";
const UPLOAD: &[u8] = b"AnySend file upload v1";
const ERROR_INDEX: u32 = u32::MAX;

#[derive(Serialize, Deserialize)]
struct FileInfo {
    name: String,
    size: u64,
}

enum Transfer {
    Send { file: File, info: FileInfo, chunks: Vec<bool>, sent: u64 },
    Receive { directory: PathBuf, incoming: Option<Incoming> },
}

struct Incoming {
    info: FileInfo,
    file: File,
    temp: tempfile::TempPath,
    chunks: Vec<bool>,
    received: u64,
}

pub(super) struct Session {
    guest: PublicKey,
    claim: Vec<u8>,
    ack: Bytes,
    cipher: Aes256Gcm,
    transfer: Transfer,
    expires: SystemTime,
    route: Option<Route>,
    finished: bool,
    failed: bool,
    report: Report,
}

impl Session {
    pub(super) async fn new(
        selection: Selection,
        host: &SecretKey,
        guest: &SecretKey,
        expires: SystemTime,
        report: Report,
    ) -> Result<Self> {
        let transfer = match selection {
            Selection::Send(path) => {
                let file = File::open(&path).await.context("Could not open the selected file.")?;
                let metadata = file.metadata().await?;
                let size = metadata.len();
                if !metadata.is_file() || size > MAX_SIZE {
                    bail!("Choose a file up to 128 MiB.");
                }
                let name = path.file_name().context("The file has no name.")?.to_string_lossy().into_owned();
                Transfer::Send {
                    file,
                    info: FileInfo { name, size },
                    chunks: vec![false; count(size)],
                    sent: 0,
                }
            }
            Selection::Receive(directory) => {
                if !tokio::fs::metadata(&directory).await.is_ok_and(|meta| meta.is_dir()) {
                    bail!("Choose an existing destination folder.");
                }
                Transfer::Receive { directory, incoming: None }
            }
        };
        let peer = host.public();
        let seed = guest.to_bytes();
        let guest = guest.public();
        let key = Sha512::new().chain_update(peer.as_bytes()).chain_update(seed).finalize();
        let cipher = Aes256Gcm::new_from_slice(&key[..32]).expect("32-byte AES key");
        let claim = [CLAIM, peer.as_bytes()].concat();
        let acknowledgment = host.sign(&[ACK, guest.as_bytes()].concat());
        let ack = Bytes::from([&[1], acknowledgment.to_bytes().as_slice()].concat());
        Ok(Self {
            guest,
            claim,
            ack,
            cipher,
            transfer,
            expires,
            route: None,
            finished: false,
            failed: false,
            report,
        })
    }

    pub(super) fn finished(&self) -> bool {
        self.finished || self.failed
    }

    pub(super) async fn handle(&mut self, route: Route, request: &[u8]) -> Vec<Bytes> {
        if let Ok(signature) = <&[u8; 64]>::try_from(request)
            && self.guest.verify(&self.claim, &Signature::from_bytes(signature)).is_ok()
        {
            if self.route.is_some() || SystemTime::now() >= self.expires {
                return vec![Bytes::from_static(&[0])];
            }
            self.route = Some(route);
            (self.report)(Event::Started(route));
            return vec![self.ack.clone()];
        }
        if self.route != Some(route) || self.failed || SystemTime::now() >= self.expires {
            return Vec::new();
        }
        match self.packet(request).await {
            Ok(replies) => replies,
            Err(error) => {
                self.failed = true;
                if let Transfer::Receive { incoming, .. } = &mut self.transfer {
                    *incoming = None;
                }
                let message = error.to_string();
                (self.report)(Event::Failed(message.clone()));
                seal(&self.cipher, ERROR_INDEX, message.as_bytes()).into_iter().collect()
            }
        }
    }

    async fn packet(&mut self, request: &[u8]) -> Result<Vec<Bytes>> {
        if request.len() == 32 {
            let Some(index) = open(&self.cipher, request, REQUEST).and_then(|bytes| read_index(&bytes))
            else {
                return Ok(Vec::new());
            };
            return self.read(index).await;
        }
        if !self.finished
            && request.first() == Some(&2)
            && let Some(index) = read_index(&request[1..])
            && let Some(plaintext) = open(&self.cipher, &request[5..], &[UPLOAD, &request[1..5]].concat())
        {
            return self.write(index, &plaintext).await;
        }
        Ok(Vec::new())
    }

    async fn read(&mut self, index: u32) -> Result<Vec<Bytes>> {
        if index == 0 {
            let metadata = match &self.transfer {
                Transfer::Send { info, .. } => serde_json::to_vec(info)?,
                Transfer::Receive { .. } => br#"{"direction":"upload","maxSize":134217728}"#.to_vec(),
            };
            if matches!(&self.transfer, Transfer::Send { info, .. } if info.size == 0) {
                self.finish(None);
            }
            return Ok(vec![seal(&self.cipher, 0, &metadata)?]);
        }
        let Transfer::Send { file, info, chunks, sent } = &mut self.transfer else {
            return Ok(Vec::new());
        };
        let first = (index - 1) as usize;
        let end = (first + WINDOW).min(chunks.len());
        let before = *sent;
        let mut replies = Vec::with_capacity(end.saturating_sub(first));
        if first < end {
            file.seek(SeekFrom::Start(first as u64 * CHUNK as u64)).await?;
        }
        for (i, served) in chunks.iter_mut().enumerate().take(end).skip(first) {
            let length = chunk_len(info.size, i);
            let mut buffer = vec![0; length];
            file.read_exact(&mut buffer).await.context("Could not read the selected file.")?;
            replies.push(seal(&self.cipher, i as u32 + 1, &buffer)?);
            if !*served {
                *served = true;
                *sent += length as u64;
            }
        }
        let total = info.size;
        let sent = *sent;
        progress(&self.report, before, sent, total);
        if sent == total {
            self.finish(None);
        }
        Ok(replies)
    }

    async fn write(&mut self, index: u32, plaintext: &[u8]) -> Result<Vec<Bytes>> {
        let Transfer::Receive { directory, incoming } = &mut self.transfer else {
            return Ok(Vec::new());
        };
        if index == 0 {
            if incoming.is_some() {
                return Ok(vec![seal(&self.cipher, 0, &[])?]);
            }
            let info: FileInfo = serde_json::from_slice(plaintext).context("Invalid file information.")?;
            if info.size > MAX_SIZE || !safe_filename(&info.name) {
                bail!("The file name is invalid or the file is larger than 128 MiB.");
            }
            let (file, temp) = tempfile::NamedTempFile::new_in(&*directory)
                .context("Could not create a file in the destination folder.")?
                .into_parts();
            *incoming = Some(Incoming {
                file: File::from_std(file),
                temp,
                chunks: vec![false; count(info.size)],
                info,
                received: 0,
            });
        } else {
            let Some(file) = incoming.as_mut() else {
                return Ok(Vec::new());
            };
            let i = (index - 1) as usize;
            if i >= file.chunks.len() || plaintext.len() != chunk_len(file.info.size, i) {
                bail!("The upload contains an invalid file chunk.");
            }
            if !file.chunks[i] {
                file.file.seek(SeekFrom::Start(i as u64 * CHUNK as u64)).await?;
                file.file.write_all(plaintext).await.context("Could not write the received file.")?;
                file.chunks[i] = true;
                let before = file.received;
                file.received += plaintext.len() as u64;
                progress(&self.report, before, file.received, file.info.size);
            }
        }
        if incoming.as_ref().is_some_and(|file| file.received == file.info.size) {
            let path = save(incoming.take().expect("completed upload"), directory).await?;
            self.finish(Some(path));
        }
        Ok(vec![seal(&self.cipher, index, &[])?])
    }

    fn finish(&mut self, file: Option<PathBuf>) {
        if !self.finished {
            self.finished = true;
            (self.report)(Event::Finished(file));
        }
    }
}

fn count(size: u64) -> usize {
    size.div_ceil(CHUNK as u64) as usize
}

fn chunk_len(size: u64, index: usize) -> usize {
    (size - index as u64 * CHUNK as u64).min(CHUNK as u64) as usize
}

fn progress(report: &Report, before: u64, after: u64, total: u64) {
    if before < after && (before == 0 || before * 100 / total != after * 100 / total) {
        report(Event::Progress(after, total));
    }
}

fn safe_filename(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && !name.ends_with(['.', ' '])
        && !name.chars().any(|c| c.is_control() || "\\/:<>|\"?*".contains(c))
}

async fn save(mut incoming: Incoming, directory: &Path) -> Result<PathBuf> {
    incoming.file.flush().await?;
    drop(incoming.file);
    let path = Path::new(&incoming.info.name);
    let stem = path.file_stem().unwrap_or(path.as_os_str()).to_string_lossy();
    let ext = path.extension().map(|ext| format!(".{}", ext.to_string_lossy())).unwrap_or_default();
    let mut temp = incoming.temp;
    for attempt in 0..1000 {
        let destination = if attempt == 0 {
            directory.join(path)
        } else {
            directory.join(format!("{stem} ({attempt}){ext}"))
        };
        match temp.persist_noclobber(&destination) {
            Ok(()) => return Ok(destination),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => temp = error.path,
            Err(error) => return Err(error.error).context("Could not save the received file."),
        }
    }
    bail!("Could not choose a free name in the destination folder.")
}

fn read_index(bytes: &[u8]) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(..4)?.try_into().ok()?))
}

fn open(cipher: &Aes256Gcm, frame: &[u8], aad: &[u8]) -> Option<Vec<u8>> {
    let nonce = Nonce::from_slice(frame.get(..12)?);
    cipher.decrypt(nonce, Payload { msg: &frame[12..], aad }).ok()
}

fn seal(cipher: &Aes256Gcm, index: u32, plaintext: &[u8]) -> Result<Bytes> {
    let header = index.to_be_bytes();
    let nonce = rand::random::<[u8; 12]>();
    let encrypted = cipher
        .encrypt(Nonce::from_slice(&nonce), Payload { msg: plaintext, aad: &header })
        .map_err(|_| anyhow::anyhow!("Could not encrypt the file payload."))?;
    Ok(Bytes::from([header.as_slice(), &nonce, &encrypted].concat()))
}
