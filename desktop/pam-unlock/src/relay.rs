// SPDX-License-Identifier: MPL-2.0
use std::{
    fs::File,
    io::{self, Read, Write},
    net::Shutdown,
    os::unix::{
        fs::{FileTypeExt, MetadataExt},
        net::UnixStream,
    },
    path::Path,
    time::{Duration, Instant},
};

use nectarlink_protocol::LinuxUnlockChallenge;
use ring::signature::{ECDSA_P256_SHA256_ASN1, UnparsedPublicKey};
use rustix::{
    fs::{Mode, OFlags, openat},
    net::{AddressFamily, SocketAddrUnix, SocketFlags, SocketType, connect, socket_with, sockopt},
};
use serde::{Deserialize, Serialize};

use crate::store::{descriptor_path, open_trusted_directory};

const SOCKET_NAME: &str = "nectarlink-phone-unlock.sock";
const VERSION: u32 = 1;
const MAX_REPLY: usize = 1024;

#[derive(Serialize)]
struct Request<'a> {
    version: u32,
    operation: &'static str,
    challenge: &'a LinuxUnlockChallenge,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    version: u32,
    signature: Vec<u8>,
}

pub(crate) fn request_signature(
    runtime_dir: &Path,
    challenge: &LinuxUnlockChallenge,
    deadline: Instant,
) -> io::Result<Vec<u8>> {
    let directory = open_trusted_directory(runtime_dir, challenge.uid, 0o7077)?;
    let socket_path = File::from(openat(
        &directory,
        SOCKET_NAME,
        OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?);
    let metadata = socket_path.metadata()?;
    if !metadata.file_type().is_socket()
        || metadata.uid() != challenge.uid
        || metadata.mode() & 0o7777 != 0o600
    {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "untrusted relay socket"));
    }
    let socket = socket_with(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::NONBLOCK | SocketFlags::CLOEXEC,
        None,
    )?;
    // Pin the checked inode through connect even if the user removes or replaces its path.
    connect(&socket, &SocketAddrUnix::new(descriptor_path(&socket_path))?)?;
    if sockopt::socket_peercred(&socket)?.uid.as_raw() != challenge.uid {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "wrong relay peer UID"));
    }
    let mut stream = UnixStream::from(socket);
    stream.set_nonblocking(false)?;
    stream.set_write_timeout(Some(remaining(deadline)?))?;
    let request = serde_json::to_vec(&Request { version: VERSION, operation: "sign", challenge })?;
    stream.write_all(&request)?;
    stream.shutdown(Shutdown::Write)?;
    read_signature(&mut stream, deadline)
}

pub(crate) fn read_signature(stream: &mut UnixStream, deadline: Instant) -> io::Result<Vec<u8>> {
    let mut bytes = [0; MAX_REPLY + 1];
    let mut length = 0;
    loop {
        stream.set_read_timeout(Some(remaining(deadline)?))?;
        match stream.read(&mut bytes[length..]) {
            Ok(0) => break,
            Ok(read) => length += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
        if length > MAX_REPLY {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "relay response too large"));
        }
    }
    remaining(deadline)?;
    let response: Response = serde_json::from_slice(&bytes[..length])?;
    if response.version != VERSION || response.signature.is_empty() || response.signature.len() > 72 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid relay response"));
    }
    Ok(response.signature)
}

pub(crate) fn verify_signature(
    public_key: &[u8],
    challenge: &LinuxUnlockChallenge,
    signature: &[u8],
) -> bool {
    UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, public_key)
        .verify(&challenge.signing_bytes(), signature)
        .is_ok()
}

fn remaining(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "phone unlock deadline elapsed"))
}
