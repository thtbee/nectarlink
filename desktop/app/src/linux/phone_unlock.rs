// SPDX-License-Identifier: GPL-3.0-or-later
//! Local transport for the dedicated Linux screen-lock PAM module.

use std::{
    ffi::CStr,
    fs::{self, OpenOptions},
    io,
    os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use nectarlink_core::{DeviceId, Node};
use nectarlink_protocol::LinuxUnlockChallenge;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    time::timeout,
};

const SOCKET: &str = "nectarlink-phone-unlock.sock";
const PAM_CONFIG: &str = "/etc/pam.d/nectarlink-phone-lock";
const MAX_REQUEST: usize = 2048;
const MAX_RESPONSE: usize = 16 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(2);
const IPC_VERSION: u8 = 1;
static AVAILABLE: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Sign { challenge: LinuxUnlockChallenge },
    Devices {},
    Enroll { phone: DeviceId },
}

#[derive(Deserialize, Serialize)]
struct Message {
    version: u8,
    #[serde(flatten)]
    request: Request,
}

fn parse_request(bytes: &[u8]) -> io::Result<Request> {
    let message: Message = serde_json::from_slice(bytes)?;
    if message.version != IPC_VERSION {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "unsupported phone unlock socket version"));
    }
    Ok(message.request)
}

pub fn available() -> bool {
    AVAILABLE.load(Ordering::Acquire)
}

pub fn configured() -> bool {
    fs::symlink_metadata(PAM_CONFIG)
        .is_ok_and(|meta| meta.is_file() && meta.uid() == 0 && meta.mode() & 0o022 == 0)
}

fn uid() -> u32 {
    // SAFETY: no arguments and no failure conditions.
    unsafe { libc::geteuid() }
}

fn socket_path() -> io::Result<PathBuf> {
    let directory = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| io::Error::other("XDG_RUNTIME_DIR must name a private runtime directory"))?;
    let meta = fs::symlink_metadata(&directory)?;
    if !meta.is_dir() || meta.uid() != uid() || meta.mode() & 0o777 != 0o700 || uid() == 0 {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "invalid runtime directory"));
    }
    Ok(directory.join(SOCKET))
}

async fn read_frame(reader: impl AsyncRead + Unpin, limit: usize, wait: Duration) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    timeout(wait, reader.take(limit as u64 + 1).read_to_end(&mut bytes)).await??;
    if bytes.is_empty() || bytes.len() > limit {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid phone unlock frame length"));
    }
    Ok(bytes)
}

pub fn start() -> io::Result<()> {
    let path = socket_path()?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path.with_extension("lock"))?;
    let lock_meta = lock.metadata()?;
    if !lock_meta.is_file() || lock_meta.uid() != uid() || lock_meta.mode() & 0o777 != 0o600 {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "invalid phone unlock socket lock"));
    }
    // SAFETY: the fd stays open until the listener task exits.
    if unsafe { libc::flock(std::os::fd::AsRawFd::as_raw_fd(&lock), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(io::Error::last_os_error());
    }
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_socket() && meta.uid() == uid() => fs::remove_file(&path)?,
        Ok(_) => return Err(io::Error::new(io::ErrorKind::PermissionDenied, "invalid phone unlock socket")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let listener = std::os::unix::net::UnixListener::bind(&path)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    let listener = {
        let _enter = crate::core_host::host().runtime.enter();
        UnixListener::from_std(listener)?
    };
    AVAILABLE.store(true, Ordering::Release);
    crate::core_host::spawn(async move {
        let _lock = lock;
        let requests = std::sync::Arc::new(tokio::sync::Semaphore::new(1));
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(accepted) => accepted,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::Interrupted | io::ErrorKind::ConnectionAborted
                    ) =>
                {
                    continue;
                }
                Err(error)
                    if matches!(
                        error.raw_os_error(),
                        Some(libc::EMFILE | libc::ENFILE | libc::ENOMEM | libc::ENOBUFS)
                    ) =>
                {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    continue;
                }
                Err(error) => {
                    tracing::warn!(%error, "phone unlock listener stopped");
                    break;
                }
            };
            if !stream.peer_cred().is_ok_and(|cred| cred.uid() == uid()) {
                continue;
            }
            let Ok(permit) = requests.clone().try_acquire_owned() else { continue };
            tokio::spawn(async move {
                let _permit = permit;
                let _ = serve(stream).await;
            });
        }
        AVAILABLE.store(false, Ordering::Release);
    });
    Ok(())
}

async fn serve(mut stream: UnixStream) -> io::Result<()> {
    let bytes = read_frame(&mut stream, MAX_REQUEST, READ_TIMEOUT).await?;
    let request = parse_request(&bytes)?;
    let mut reply = match crate::core_host::node() {
        Some(node) => process(&node, request).await,
        None => Err("NectarLink is still starting".into()),
    }
    .unwrap_or_else(|error| json!({ "error": error }));
    reply["version"] = json!(IPC_VERSION);
    let response = serde_json::to_vec(&reply)?;
    timeout(READ_TIMEOUT, stream.write_all(&response)).await??;
    stream.shutdown().await
}

async fn process(node: &Node, request: Request) -> Result<Value, String> {
    match request {
        Request::Sign { challenge } => {
            if challenge.uid != uid() || challenge.pc != node.device_id() {
                return Err("The request belongs to a different Linux account or PC".into());
            }
            let signature = timeout(Duration::from_secs(30), node.linux_unlock_sign(challenge))
                .await
                .map_err(|_| "The unlock request expired".to_owned())?
                .map_err(|e| e.to_string())?;
            Ok(json!({ "signature": signature }))
        }
        Request::Devices {} => {
            let phones: Vec<_> = node
                .paired_devices()
                .map_err(|e| e.to_string())?
                .into_iter()
                .filter(|device| device.info.kind == nectarlink_core::DeviceKind::Phone)
                .map(|device| json!({ "id": device.id.to_string(), "name": device.info.name }))
                .collect();
            Ok(json!({ "devices": phones }))
        }
        Request::Enroll { phone } => {
            let uid = uid();
            let name = account_name(uid).map_err(|e| e.to_string())?;
            let public_key = timeout(Duration::from_secs(120), node.linux_unlock_enroll(phone, uid, &name))
                .await
                .map_err(|_| "The enrollment request expired".to_owned())?
                .map_err(|e| e.to_string())?;
            Ok(json!({ "pc": node.device_id(), "phone": phone, "uid": uid, "public_key": public_key }))
        }
    }
}

fn account_name(uid: u32) -> io::Result<String> {
    let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut name = std::ptr::null_mut();
    let mut buffer = [0u8; 16384];
    // SAFETY: both output pointers and the buffer remain valid for this call.
    let result = unsafe {
        libc::getpwuid_r(uid, entry.as_mut_ptr(), buffer.as_mut_ptr().cast(), buffer.len(), &mut name)
    };
    if result != 0 {
        return Err(io::Error::from_raw_os_error(result));
    }
    if name.is_null() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "Linux account not found"));
    }
    // SAFETY: a non-null result from getpwuid_r points to a valid passwd record,
    // and pw_name is a NUL-terminated string within the buffer.
    Ok(unsafe { CStr::from_ptr((*name).pw_name) }.to_string_lossy().into_owned())
}

pub fn run_cli(request: Request) -> io::Result<Value> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    runtime.block_on(async {
        let mut stream = UnixStream::connect(socket_path()?).await?;
        if stream.peer_cred()?.uid() != uid() {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "unexpected socket owner"));
        }
        let message = Message { version: IPC_VERSION, request };
        stream.write_all(&serde_json::to_vec(&message)?).await?;
        stream.shutdown().await?;
        let bytes = read_frame(stream, MAX_RESPONSE, Duration::from_secs(125)).await?;
        let mut value: Value = serde_json::from_slice(&bytes)?;
        if value.as_object_mut().and_then(|object| object.remove("version")).and_then(|v| v.as_u64())
            != Some(IPC_VERSION.into())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unsupported phone unlock socket version",
            ));
        }
        if let Some(error) = value.get("error").and_then(Value::as_str) {
            return Err(io::Error::other(error.to_owned()));
        }
        Ok(value)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn truncated_and_oversized_frames_fail_closed() {
        assert!(read_frame(&b""[..], 1, READ_TIMEOUT).await.is_err());
        assert!(read_frame(&b"ab"[..], 1, READ_TIMEOUT).await.is_err());
        assert_eq!(read_frame(&b"ab"[..], 2, READ_TIMEOUT).await.unwrap(), b"ab");
        assert!(serde_json::from_slice::<Request>(b"{\"operation\":\"sign\",\"approved\":true}").is_err());
    }

    #[test]
    fn requires_supported_message_version() {
        assert!(parse_request(br#"{"version":1,"operation":"devices"}"#).is_ok());
        assert!(parse_request(br#"{"version":2,"operation":"devices"}"#).is_err());
        assert!(parse_request(br#"{"operation":"devices"}"#).is_err());
        assert!(parse_request(br#"{"version":1,"operation":"devices","approved":true}"#).is_err());
    }
}
