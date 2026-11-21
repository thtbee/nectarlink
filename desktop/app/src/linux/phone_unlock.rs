// SPDX-License-Identifier: GPL-3.0-or-later
//! Local transport for the dedicated Linux screen-lock PAM module.

use std::{
    ffi::CStr,
    fs::{self, OpenOptions},
    future::Future,
    io,
    os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use nectarlink_core::{DeviceId, Node};
use nectarlink_protocol::LinuxUnlockChallenge;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    sync::Semaphore,
    time::timeout,
};

const SOCKET: &str = "nectarlink-phone-unlock.sock";
const PAM_CONFIG: &str = "/etc/pam.d/nectarlink-phone-lock";
const MAX_REQUEST: usize = 2048;
const MAX_RESPONSE: usize = 16 * 1024;
const MAX_CLIENTS: usize = 8;
const READ_TIMEOUT: Duration = Duration::from_secs(2);
const IPC_VERSION: u8 = 2;
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
    if !meta.is_dir() || meta.uid() != uid() || meta.mode() & 0o7777 != 0o700 || uid() == 0 {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "invalid runtime directory"));
    }
    Ok(directory.join(SOCKET))
}

async fn read_frame(mut reader: impl AsyncRead + Unpin, limit: usize, wait: Duration) -> io::Result<Vec<u8>> {
    timeout(wait, async {
        let len = reader.read_u32().await? as usize;
        if len == 0 || len > limit {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid phone unlock frame length"));
        }
        let mut bytes = vec![0; len];
        reader.read_exact(&mut bytes).await?;
        Ok(bytes)
    })
    .await?
}

async fn write_request(stream: &mut UnixStream, request: Request) -> io::Result<()> {
    let body = serde_json::to_vec(&Message { version: IPC_VERSION, request })?;
    timeout(READ_TIMEOUT, async {
        stream.write_u32(u32::try_from(body.len()).map_err(io::Error::other)?).await?;
        stream.write_all(&body).await
    })
    .await?
}

async fn read_reply(reader: impl AsyncRead + Unpin, limit: usize, wait: Duration) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    timeout(wait, reader.take(limit as u64 + 1).read_to_end(&mut bytes)).await??;
    if bytes.is_empty() || bytes.len() > limit {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid phone unlock frame length"));
    }
    Ok(bytes)
}

fn bind_private(path: &Path) -> io::Result<std::os::unix::net::UnixListener> {
    let directory = path.parent().ok_or_else(|| io::Error::other("phone unlock socket needs a parent"))?;
    // The caller has verified the runtime directory is private. Publish only after chmod.
    let staged_socket = directory.join(format!(".nl-{:016x}", rand::random::<u64>()));
    let listener = std::os::unix::net::UnixListener::bind(&staged_socket)?;
    let publish = (|| {
        fs::set_permissions(&staged_socket, fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        fs::rename(&staged_socket, path)
    })();
    if publish.is_err() {
        let _ = fs::remove_file(staged_socket);
    }
    publish?;
    Ok(listener)
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
    if !lock_meta.is_file() || lock_meta.uid() != uid() || lock_meta.mode() & 0o7777 != 0o600 {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "invalid phone unlock socket lock"));
    }
    // SAFETY: the fd stays open until the listener task exits.
    if unsafe { libc::flock(std::os::fd::AsRawFd::as_raw_fd(&lock), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(io::Error::last_os_error());
    }
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_socket() && meta.uid() == uid() => (),
        Ok(_) => return Err(io::Error::new(io::ErrorKind::PermissionDenied, "invalid phone unlock socket")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let listener = bind_private(&path)?;
    let listener = {
        let _enter = crate::core_host::host().runtime.enter();
        UnixListener::from_std(listener)?
    };
    AVAILABLE.store(true, Ordering::Release);
    crate::core_host::spawn(async move {
        let _lock = lock;
        let clients = Arc::new(Semaphore::new(MAX_CLIENTS));
        let signing = Arc::new(Semaphore::new(1));
        let enrolling = Arc::new(Semaphore::new(1));
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
            let Ok(client) = clients.clone().try_acquire_owned() else { continue };
            let (signing, enrolling) = (signing.clone(), enrolling.clone());
            tokio::spawn(async move {
                let _client = client;
                let _ = serve(stream, &signing, &enrolling, |request| async move {
                    let node = crate::core_host::node().ok_or("NectarLink is still starting")?;
                    process(&node, request).await
                })
                .await;
            });
        }
        AVAILABLE.store(false, Ordering::Release);
    });
    Ok(())
}

async fn serve<F: Future<Output = Result<Value, String>>>(
    mut stream: UnixStream,
    signing: &Semaphore,
    enrolling: &Semaphore,
    run: impl FnOnce(Request) -> F,
) -> io::Result<()> {
    let bytes = read_frame(&mut stream, MAX_REQUEST, READ_TIMEOUT).await?;
    let request = parse_request(&bytes)?;
    let permit = match &request {
        Request::Devices {} => None,
        Request::Sign { .. } => Some(signing.try_acquire()),
        Request::Enroll { .. } => Some(enrolling.try_acquire()),
    };
    let mut reply = if matches!(permit, Some(Err(_))) {
        json!({ "error": "Another phone unlock request of this type is in progress" })
    } else {
        // A version 2 client keeps its write side open until it receives the reply.
        tokio::select! {
            biased;
            extra = stream.read_u8() => {
                extra?;
                return Err(io::Error::new(io::ErrorKind::InvalidData, "expected one phone unlock request"));
            }
            reply = run(request) => reply.unwrap_or_else(|error| json!({ "error": error })),
        }
    };
    drop(permit);
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
    // SAFETY: the non-null result points to the passwd record returned by getpwuid_r.
    let name = unsafe { (*name).pw_name };
    if name.is_null() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Linux account name missing"));
    }
    // SAFETY: getpwuid_r returned a non-null, NUL-terminated name in the live buffer.
    Ok(unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned())
}

pub fn run_cli(request: Request) -> io::Result<Value> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    runtime.block_on(async {
        let mut stream = UnixStream::connect(socket_path()?).await?;
        if stream.peer_cred()?.uid() != uid() {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "unexpected socket owner"));
        }
        write_request(&mut stream, request).await?;
        let bytes = read_reply(stream, MAX_RESPONSE, Duration::from_secs(125)).await?;
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
    async fn reads_one_request_while_client_remains_open() {
        let (mut client, mut relay) = UnixStream::pair().unwrap();
        let body = br#"{"version":2,"operation":"devices"}"#;
        client.write_u32(u32::try_from(body.len()).unwrap()).await.unwrap();
        client.write_all(body).await.unwrap();
        assert_eq!(read_frame(&mut relay, MAX_REQUEST, Duration::from_millis(100)).await.unwrap(), body);
    }

    #[tokio::test]
    async fn truncated_and_oversized_frames_fail_closed() {
        assert!(read_frame(&b""[..], 1, READ_TIMEOUT).await.is_err());
        assert!(read_frame(&b"\0\0\0\0"[..], 2, READ_TIMEOUT).await.is_err());
        assert!(read_frame(&b"\0\0\0\x02a"[..], 2, READ_TIMEOUT).await.is_err());
        assert!(read_frame(&b"\0\0\0\x02ab"[..], 1, READ_TIMEOUT).await.is_err());
        assert_eq!(read_frame(&b"\0\0\0\x02ab"[..], 2, READ_TIMEOUT).await.unwrap(), b"ab");
        assert!(serde_json::from_slice::<Request>(b"{\"operation\":\"sign\",\"approved\":true}").is_err());
    }

    #[test]
    fn requires_supported_message_version() {
        assert!(parse_request(br#"{"version":2,"operation":"devices"}"#).is_ok());
        assert!(parse_request(br#"{"version":1,"operation":"devices"}"#).is_err());
        assert!(parse_request(br#"{"operation":"devices"}"#).is_err());
        assert!(parse_request(br#"{"version":2,"operation":"devices","approved":true}"#).is_err());
    }

    #[tokio::test]
    async fn disconnect_releases_signing_permit_and_enrollment_has_its_own() {
        let signing = Semaphore::new(1);
        let enrolling = Semaphore::new(1);
        let challenge = LinuxUnlockChallenge {
            pc: DeviceId([1; 32]),
            phone: DeviceId([2; 32]),
            uid: 1000,
            nonce: [3; 32],
        };
        let (mut first, relay) = UnixStream::pair().unwrap();
        write_request(&mut first, Request::Sign { challenge }).await.unwrap();
        let (started, waiting) = tokio::sync::oneshot::channel();
        let request = serve(relay, &signing, &enrolling, |request| async move {
            assert!(matches!(request, Request::Sign { .. }));
            started.send(()).unwrap();
            std::future::pending().await
        });
        let peer = async {
            waiting.await.unwrap();
            assert_eq!(signing.available_permits(), 0);
            let (mut second, relay) = UnixStream::pair().unwrap();
            write_request(&mut second, Request::Enroll { phone: challenge.phone }).await.unwrap();
            let enrollment = serve(relay, &signing, &enrolling, |request| async move {
                assert!(matches!(request, Request::Enroll { .. }));
                Ok(json!({ "enrolled": true }))
            });
            let (result, reply) = tokio::join!(enrollment, read_reply(second, MAX_RESPONSE, READ_TIMEOUT));
            result.unwrap();
            assert_eq!(serde_json::from_slice::<Value>(&reply.unwrap()).unwrap()["enrolled"], true);
            drop(first);
        };
        let (result, ()) = timeout(READ_TIMEOUT, async { tokio::join!(request, peer) }).await.unwrap();
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
        assert_eq!(signing.available_permits(), 1);

        let (mut retry, relay) = UnixStream::pair().unwrap();
        write_request(&mut retry, Request::Sign { challenge }).await.unwrap();
        let request = serve(relay, &signing, &enrolling, |_| async { Ok(json!({ "signature": [1] })) });
        let (result, reply) = tokio::join!(request, read_reply(retry, MAX_RESPONSE, READ_TIMEOUT));
        result.unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&reply.unwrap()).unwrap()["signature"], json!([1]));
    }

    #[test]
    fn publishes_a_private_socket() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(SOCKET);
        let _listener = bind_private(&path).unwrap();
        assert_eq!(fs::symlink_metadata(&path).unwrap().mode() & 0o7777, 0o600);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
        assert!(std::os::unix::net::UnixStream::connect(path).is_ok());
    }
}
