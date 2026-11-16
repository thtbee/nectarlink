// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS on-demand phone storage mount (`~/Nectarlink/<Phone name>`) via a
//! local loopback WebDAV server mounted into Finder with `/sbin/mount_webdav`.
//!
//! Zero third-party kernel extensions (no macFUSE required): every Mac with
//! built-in `mount_webdav` can browse, read, write, rename, and delete files on
//! paired phones directly in Finder.

use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex, MutexGuard, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use nectarlink_core::{DeviceId, LinkState, NodeEvent, StorageEntry};

use crate::{
    core_host,
    state::{Changes, CoreStatus},
    storage::{base_sync_dir, folder_names, sanitize_folder_name},
};

const DIR_CACHE_TTL: Duration = Duration::from_secs(3);
const MOUNT_RETRY_AFTER: Duration = Duration::from_secs(30);
const STREAM_READ_CHUNK_BYTES: usize = 64 * 1024;
const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_CONTROL_BODY_BYTES: usize = 1024 * 1024;
const CONN_IO_TIMEOUT: Duration = Duration::from_secs(30);

static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);
static MAC_RECONCILING: Mutex<()> = Mutex::new(());
static MOUNTS: OnceLock<Mutex<HashMap<DeviceId, Arc<MountedDevice>>>> = OnceLock::new();
static LINK_ONLINE: Mutex<Option<HashMap<DeviceId, bool>>> = Mutex::new(None);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn mounts() -> MutexGuard<'static, HashMap<DeviceId, Arc<MountedDevice>>> {
    lock(MOUNTS.get_or_init(|| Mutex::new(HashMap::new())))
}

fn get_mounted_device(device: &DeviceId) -> Option<Arc<MountedDevice>> {
    mounts().get(device).cloned()
}

pub fn mounted_path_for(device: &DeviceId) -> Option<PathBuf> {
    get_mounted_device(device).map(|m| m.mount_path.clone())
}

fn online_changed(device: DeviceId, online: bool) -> bool {
    let mut links = lock(&LINK_ONLINE);
    links.get_or_insert_with(HashMap::new).insert(device, online) != Some(online)
}

#[derive(Default)]
struct DirCache {
    dirs: HashMap<String, (Instant, Vec<StorageEntry>)>,
}

impl DirCache {
    fn get_dir(&mut self, dir_rel: &str) -> Option<Vec<StorageEntry>> {
        if let Some((fetched_at, entries)) = self.dirs.get(dir_rel)
            && fetched_at.elapsed() < DIR_CACHE_TTL
        {
            return Some(entries.clone());
        }
        self.dirs.remove(dir_rel);
        None
    }

    fn insert_dir(&mut self, dir_rel: String, entries: Vec<StorageEntry>) {
        self.dirs.insert(dir_rel, (Instant::now(), entries));
    }

    fn invalidate(&mut self, rel_path: &str) {
        let trimmed = rel_path.trim_matches('/');
        if trimmed.is_empty() {
            self.dirs.clear();
            return;
        }
        self.dirs.remove(trimmed);
        let prefix = format!("{trimmed}/");
        self.dirs.retain(|k, _| !k.starts_with(&prefix));
        if let Some((parent, _)) = trimmed.rsplit_once('/') {
            self.dirs.remove(parent);
        } else {
            self.dirs.remove("");
        }
    }

    fn clear(&mut self) {
        self.dirs.clear();
    }
}

struct MountedDevice {
    device: DeviceId,
    device_name: String,
    mount_path: PathBuf,
    port: u16,
    stop: Arc<AtomicBool>,
    dir_cache: Arc<Mutex<DirCache>>,
    worker: Mutex<Option<JoinHandle<()>>>,
    died_at: Mutex<Option<Instant>>,
}

impl MountedDevice {
    fn retry_due(&self) -> bool {
        lock(&self.died_at).is_some_and(|t| t.elapsed() >= MOUNT_RETRY_AFTER)
    }

    fn invalidate_cache(&self, path: &str) {
        lock(&self.dir_cache).invalidate(path);
    }

    fn clear_cache(&self) {
        lock(&self.dir_cache).clear();
    }

    fn stop_and_join(&self) {
        unmount_path(&self.mount_path);
        self.stop.store(true, Ordering::Release);
        wake_listener(self.port);
        if let Some(handle) = lock(&self.worker).take() {
            let _ = handle.join();
        }
        let _ = fs::remove_dir(&self.mount_path);
    }
}

struct DesiredMount {
    device: DeviceId,
    name: String,
    folder: String,
    online: bool,
}

pub fn start() {
    SHUTTING_DOWN.store(false, Ordering::Release);
    core_host::host().hub.subscribe(
        Changes::DEVICES | Changes::STATUS | Changes::CAPABILITIES | Changes::TOGGLES,
        || {
            sync();
            true
        },
    );
    sync();
}

pub fn sync() {
    if SHUTTING_DOWN.load(Ordering::Acquire) {
        return;
    }
    let host = core_host::host();
    let Some(node) = core_host::node() else { return };
    let ready = host.hub.read(|s| matches!(s.status, Some(CoreStatus::Ready { .. })));
    if !ready {
        return;
    }
    let devices = host.hub.read(|s| {
        s.devices
            .iter()
            .map(|d| (d.id, d.info.name.clone(), matches!(d.link, LinkState::Online { .. })))
            .collect::<Vec<_>>()
    });
    let mut folders = folder_names(devices.iter().map(|(id, name, _)| (*id, name.as_str())));
    let mut desired: Vec<DesiredMount> = Vec::new();
    let mut disabled: Vec<(DeviceId, String)> = Vec::new();
    for (id, name, online) in devices {
        // Not being able to read the setting isn't the setting being off: leave
        // the mount and its open files alone.
        let Ok(toggles) = node.device_toggles(id) else {
            tracing::debug!(device = %id.short(), "can't read storage permission; skipping this sync");
            return;
        };
        let enabled = toggles.into_iter().find(|(k, _)| *k == "storage").is_some_and(|(_, on)| on);
        let folder = folders.remove(&id).unwrap_or_else(|| sanitize_folder_name(&name));
        if enabled {
            desired.push(DesiredMount { device: id, name, folder, online });
        } else {
            disabled.push((id, folder));
        }
    }

    let _ = std::thread::Builder::new().name("storage-mac-reconcile".into()).spawn(move || {
        let _guard = lock(&MAC_RECONCILING);
        reconcile(&desired, &disabled);
    });
}

fn reconcile(desired: &[DesiredMount], disabled: &[(DeviceId, String)]) {
    if SHUTTING_DOWN.load(Ordering::Acquire) {
        return;
    }
    let Some(base) = base_sync_dir() else { return };

    for (id, folder) in disabled {
        unmount_device(*id, Some(folder));
    }

    let desired_ids: HashSet<DeviceId> = desired.iter().map(|d| d.device).collect();
    let stale: Vec<DeviceId> = mounts().keys().filter(|id| !desired_ids.contains(id)).copied().collect();
    for id in stale {
        unmount_device(id, None);
    }

    for DesiredMount { device, name, folder, online } in desired {
        let mount_path = base.join(folder);
        let existing = get_mounted_device(device);
        if let Some(mounted) = existing {
            let unchanged = mounted.mount_path == mount_path && mounted.device_name == *name;
            if unchanged && !mounted.retry_due() {
                if *online {
                    mounted.clear_cache();
                }
                continue;
            }
            unmount_device(*device, None);
        } else {
            clear_stale_mount(&mount_path);
        }

        if let Err(e) = fs::create_dir_all(&mount_path) {
            tracing::warn!(error = %e, "can't create the phone storage folder");
            continue;
        }

        let listener = match TcpListener::bind(("127.0.0.1", 0)) {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!(error = %e, "failed to bind local WebDAV listener");
                continue;
            }
        };
        let Ok(addr) = listener.local_addr() else { continue };
        let port = addr.port();
        let stop = Arc::new(AtomicBool::new(false));
        let dir_cache = Arc::new(Mutex::new(DirCache::default()));
        let mounted = Arc::new(MountedDevice {
            device: *device,
            device_name: name.clone(),
            mount_path: mount_path.clone(),
            port,
            stop,
            dir_cache,
            worker: Mutex::new(None),
            died_at: Mutex::new(None),
        });

        let worker_mount = Arc::clone(&mounted);
        let thread_name = format!("storage-webdav-{}", device.short());
        if let Ok(handle) = std::thread::Builder::new().name(thread_name).spawn(move || {
            serve_and_mount(listener, &worker_mount);
        }) {
            *lock(&mounted.worker) = Some(handle);
            mounts().insert(*device, mounted);
        }
    }
}

fn unmount_device(device: DeviceId, fallback_folder: Option<&str>) {
    let removed = mounts().remove(&device);
    if let Some(mounted) = removed {
        mounted.stop_and_join();
    } else if let Some(folder) = fallback_folder
        && let Some(base) = base_sync_dir()
    {
        let mount_path = base.join(folder);
        clear_stale_mount(&mount_path);
        let _ = fs::remove_dir(&mount_path);
    }
}

pub fn on_event(event: &NodeEvent) {
    match event {
        NodeEvent::DeviceRemoved(device) => {
            let device = *device;
            let _ = std::thread::Builder::new().name("storage-mac-unpair".into()).spawn(move || {
                let _guard = lock(&MAC_RECONCILING);
                unmount_device(device, None);
            });
        }
        NodeEvent::LinkChanged { device, link } => {
            if online_changed(*device, matches!(link, LinkState::Online { .. })) {
                sync();
            }
        }
        NodeEvent::DeviceAdded(_) | NodeEvent::PeerInfoChanged { .. } | NodeEvent::Capabilities(_) => {
            sync();
        }
        NodeEvent::StorageChanged { device, path } => {
            if let Some(mounted) = get_mounted_device(device) {
                mounted.invalidate_cache(path);
            }
        }
        _ => {}
    }
}

pub fn shutdown() {
    SHUTTING_DOWN.store(true, Ordering::Release);
    let _guard = lock(&MAC_RECONCILING);
    let active: Vec<Arc<MountedDevice>> = mounts().drain().map(|(_, v)| v).collect();
    for mounted in active {
        mounted.stop_and_join();
    }
}

pub fn unregister_all() {
    shutdown();
    if let Some(base) = base_sync_dir() {
        let _ = fs::remove_dir(&base);
    }
}

fn mount_webdav_args(port: u16, device_name: &str, mount_path: &Path) -> Vec<String> {
    vec![
        "-S".into(),
        "-v".into(),
        device_name.into(),
        format!("http://127.0.0.1:{port}/"),
        mount_path.to_string_lossy().into_owned(),
    ]
}

fn is_mounted(mount_path: &Path) -> bool {
    let Ok(output) = Command::new("/sbin/mount").output() else {
        return false;
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let target = format!(" on {} ", mount_path.to_string_lossy());
    text.lines().any(|line| line.contains(&target))
}

fn clear_stale_mount(mount_path: &Path) {
    if is_mounted(mount_path) {
        unmount_path(mount_path);
    }
}

fn unmount_path(mount_path: &Path) {
    let path_str = mount_path.to_string_lossy();
    let ok = Command::new("/sbin/umount")
        .arg(path_str.as_ref())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        let _ = Command::new("/sbin/umount")
            .args(["-f", path_str.as_ref()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn wake_listener(port: u16) {
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let _ = TcpStream::connect_timeout(&addr, Duration::from_millis(200));
}

fn serve_and_mount(listener: TcpListener, mounted: &Arc<MountedDevice>) {
    if mounted.stop.load(Ordering::Acquire) {
        return;
    }
    let mount_args = mount_webdav_args(mounted.port, &mounted.device_name, &mounted.mount_path);
    let m_device = mounted.device;
    let m_stop = Arc::clone(&mounted.stop);
    let m_died = Arc::clone(mounted);
    let _ = std::thread::Builder::new().name(format!("storage-mount-{}", m_device.short())).spawn(move || {
        // Give the accept loop a moment to enter accept().
        std::thread::sleep(Duration::from_millis(25));
        if m_stop.load(Ordering::Acquire) {
            return;
        }
        let res = Command::new("/sbin/mount_webdav")
            .args(&mount_args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output();
        match res {
            Ok(out) if out.status.success() => {
                tracing::info!(device = %m_device.short(), port = m_died.port, "mounted phone storage in Finder via WebDAV");
            }
            Ok(out) => {
                let err = String::from_utf8_lossy(&out.stderr);
                tracing::warn!(device = %m_device.short(), stderr = %err.trim(), "mount_webdav failed; will retry later");
                *lock(&m_died.died_at) = Some(Instant::now());
            }
            Err(e) => {
                tracing::warn!(device = %m_device.short(), error = %e, "failed to launch mount_webdav");
                *lock(&m_died.died_at) = Some(Instant::now());
            }
        }
    });

    for incoming in listener.incoming() {
        if mounted.stop.load(Ordering::Acquire) {
            break;
        }
        let Ok(stream) = incoming else { continue };
        if mounted.stop.load(Ordering::Acquire) {
            break;
        }
        let dev = mounted.device;
        let cache = Arc::clone(&mounted.dir_cache);
        let stop = Arc::clone(&mounted.stop);
        let _ = std::thread::Builder::new()
            .name(format!("webdav-conn-{}", dev.short()))
            .spawn(move || handle_connection(stream, dev, &cache, &stop));
    }
}

fn handle_connection(mut stream: TcpStream, device: DeviceId, cache: &Mutex<DirCache>, stop: &AtomicBool) {
    let _ = stream.set_read_timeout(Some(CONN_IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(CONN_IO_TIMEOUT));

    let Ok(read_stream) = stream.try_clone() else { return };
    let mut reader = BufReader::new(read_stream);

    while !stop.load(Ordering::Acquire) {
        let Some(req) = read_request(&mut reader, &mut stream) else {
            break;
        };
        let keep_alive = req.keep_alive;
        if !dispatch_request(&req, &mut stream, device, cache) || !keep_alive {
            break;
        }
    }
}

struct HttpRequest {
    method: String,
    rel_path: String,
    headers: HashMap<String, String>,
    body: RequestBody,
    keep_alive: bool,
}

enum RequestBody {
    Empty,
    Bytes(Vec<u8>),
    File(StageFile),
}

struct StageFile {
    path: PathBuf,
    file: fs::File,
}

impl StageFile {
    fn create() -> std::io::Result<Self> {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let dir =
            core_host::try_data_dir().map(|d| d.join("webdav-stage")).unwrap_or_else(std::env::temp_dir);
        fs::create_dir_all(&dir)?;
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!("put-{}-{id}.tmp", std::process::id()));
        let file = fs::OpenOptions::new().create_new(true).read(true).write(true).open(&path)?;
        Ok(Self { path, file })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for StageFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn read_request(reader: &mut BufReader<TcpStream>, writer: &mut TcpStream) -> Option<HttpRequest> {
    let mut request_line = String::new();
    let n = reader.read_line(&mut request_line).ok()?;
    if n == 0 {
        return None;
    }
    let request_line = request_line.trim_end_matches(['\r', '\n']);
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_ascii_uppercase();
    let raw_target = parts.next()?;
    let version = parts.next().unwrap_or("HTTP/1.1");

    let mut headers = HashMap::new();
    let mut header_bytes = 0usize;
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).ok()?;
        if read == 0 {
            return None;
        }
        header_bytes += read;
        if header_bytes > MAX_HEADER_BYTES {
            return None;
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, val)) = trimmed.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), val.trim().to_owned());
        }
    }

    if headers.get("expect").is_some_and(|v| v.eq_ignore_ascii_case("100-continue")) {
        let _ = writer.write_all(b"HTTP/1.1 100 Continue\r\n\r\n");
        let _ = writer.flush();
    }

    let is_chunked =
        headers.get("transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
    let content_length = headers.get("content-length").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);

    let body = if method == "PUT" {
        if is_chunked {
            read_chunked_to_tempfile(reader).ok()?
        } else if content_length > 0 {
            read_exact_to_tempfile(reader, content_length).ok()?
        } else {
            RequestBody::Empty
        }
    } else if is_chunked {
        let bytes = read_chunked_bytes(reader, MAX_CONTROL_BODY_BYTES).ok()?;
        RequestBody::Bytes(bytes)
    } else if content_length > 0 {
        let len = usize::try_from(content_length).ok()?.min(MAX_CONTROL_BODY_BYTES);
        let mut buf = vec![0u8; len];
        reader.read_exact(&mut buf).ok()?;
        RequestBody::Bytes(buf)
    } else {
        RequestBody::Empty
    };

    let conn_hdr = headers.get("connection").map(|s| s.to_ascii_lowercase());
    let keep_alive = if version == "HTTP/1.0" {
        conn_hdr.as_deref() == Some("keep-alive")
    } else {
        conn_hdr.as_deref() != Some("close")
    };

    let rel_path = normalize_uri_path(raw_target)?;
    Some(HttpRequest { method, rel_path, headers, body, keep_alive })
}

fn read_exact_to_tempfile(reader: &mut BufReader<TcpStream>, len: u64) -> std::io::Result<RequestBody> {
    let mut tmp = StageFile::create()?;
    let mut remaining = len;
    let mut buf = [0u8; 64 * 1024];
    while remaining > 0 {
        let want = (remaining as usize).min(buf.len());
        let n = reader.read(&mut buf[..want])?;
        if n == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "truncated PUT body"));
        }
        tmp.file.write_all(&buf[..n])?;
        remaining -= n as u64;
    }
    tmp.file.flush()?;
    Ok(RequestBody::File(tmp))
}

fn read_chunked_bytes(reader: &mut BufReader<TcpStream>, max_len: usize) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let mut size_line = String::new();
        if reader.read_line(&mut size_line)? == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "eof in chunk size"));
        }
        let hex = size_line.trim().split(';').next().unwrap_or("").trim();
        let chunk_len = usize::from_str_radix(hex, 16)
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad chunk size"))?;
        if chunk_len == 0 {
            // Consume trailing headers/CRLF.
            loop {
                let mut trailer = String::new();
                if reader.read_line(&mut trailer)? == 0 || trailer.trim().is_empty() {
                    break;
                }
            }
            break;
        }
        if out.len() + chunk_len > max_len {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "chunked body too large"));
        }
        let start = out.len();
        out.resize(start + chunk_len, 0);
        reader.read_exact(&mut out[start..])?;
        let mut crlf = [0u8; 2];
        reader.read_exact(&mut crlf)?;
    }
    Ok(out)
}

fn read_chunked_to_tempfile(reader: &mut BufReader<TcpStream>) -> std::io::Result<RequestBody> {
    let mut tmp = StageFile::create()?;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let mut size_line = String::new();
        if reader.read_line(&mut size_line)? == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "eof in chunk size"));
        }
        let hex = size_line.trim().split(';').next().unwrap_or("").trim();
        let mut remaining = usize::from_str_radix(hex, 16)
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad chunk size"))?;
        if remaining == 0 {
            loop {
                let mut trailer = String::new();
                if reader.read_line(&mut trailer)? == 0 || trailer.trim().is_empty() {
                    break;
                }
            }
            break;
        }
        while remaining > 0 {
            let want = remaining.min(buf.len());
            let n = reader.read(&mut buf[..want])?;
            if n == 0 {
                return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "truncated chunk"));
            }
            tmp.file.write_all(&buf[..n])?;
            remaining -= n;
        }
        let mut crlf = [0u8; 2];
        reader.read_exact(&mut crlf)?;
    }
    tmp.file.flush()?;
    Ok(RequestBody::File(tmp))
}

fn dispatch_request(
    req: &HttpRequest,
    stream: &mut TcpStream,
    device: DeviceId,
    cache: &Mutex<DirCache>,
) -> bool {
    let rel = req.rel_path.as_str();
    match req.method.as_str() {
        "OPTIONS" => send_options(stream),
        "PROPFIND" => handle_propfind(req, stream, device, cache, rel),
        "GET" | "HEAD" => handle_get_or_head(req, stream, device, cache, rel, req.method == "HEAD"),
        "PUT" => handle_put(req, stream, device, cache, rel),
        "MKCOL" => handle_mkcol(stream, device, cache, rel),
        "DELETE" => handle_delete(stream, device, cache, rel),
        "MOVE" => handle_move(req, stream, device, cache, rel),
        "PROPPATCH" => handle_proppatch(stream, rel),
        "LOCK" => handle_lock(stream, rel),
        "UNLOCK" => send_empty_status(stream, "204 No Content"),
        _ => send_empty_status(stream, "405 Method Not Allowed"),
    }
}

fn send_options(stream: &mut TcpStream) -> bool {
    let resp = "HTTP/1.1 200 OK\r\n\
                DAV: 1, 2\r\n\
                MS-Author-Via: DAV\r\n\
                Allow: OPTIONS, GET, HEAD, PUT, DELETE, MKCOL, MOVE, PROPFIND, PROPPATCH, LOCK, UNLOCK\r\n\
                Content-Length: 0\r\n\r\n";
    stream.write_all(resp.as_bytes()).is_ok()
}

fn send_empty_status(stream: &mut TcpStream, status: &str) -> bool {
    let resp = format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\n\r\n");
    stream.write_all(resp.as_bytes()).is_ok()
}

fn send_xml_status(stream: &mut TcpStream, status: &str, body: &str, extra_headers: &[(&str, &str)]) -> bool {
    let mut head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/xml; charset=\"utf-8\"\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (k, v) in extra_headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).is_ok() && stream.write_all(body.as_bytes()).is_ok()
}

fn handle_propfind(
    req: &HttpRequest,
    stream: &mut TcpStream,
    device: DeviceId,
    cache: &Mutex<DirCache>,
    rel: &str,
) -> bool {
    if is_finder_metadata(rel) {
        return send_empty_status(stream, "404 Not Found");
    }
    let depth = req.headers.get("depth").map(|s| s.trim()).unwrap_or("1");
    let target_entry = if rel.is_empty() {
        StorageEntry { name: String::new(), size: 0, modified: 0, is_dir: true }
    } else {
        match fetch_entry(device, cache, rel) {
            Ok(Some(e)) => e,
            Ok(None) => return send_empty_status(stream, "404 Not Found"),
            Err(status) => return send_empty_status(stream, status),
        }
    };

    let mut xml =
        String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<D:multistatus xmlns:D=\"DAV:\">\n");
    append_propfind_response(&mut xml, rel, &target_entry);

    if depth != "0" && target_entry.is_dir {
        match fetch_dir(device, cache, rel) {
            Ok(children) => {
                for child in children {
                    let name = child.name.trim_matches('/');
                    if name.is_empty() || name == "." || name == ".." || name.contains('/') {
                        continue;
                    }
                    let child_rel = join_rel(rel, name);
                    append_propfind_response(&mut xml, &child_rel, &child);
                }
            }
            Err(status) => return send_empty_status(stream, status),
        }
    }

    xml.push_str("</D:multistatus>\n");
    send_xml_status(stream, "207 Multi-Status", &xml, &[])
}

fn append_propfind_response(xml: &mut String, rel_path: &str, entry: &StorageEntry) {
    let href = encode_href(rel_path, entry.is_dir);
    let display_name = if rel_path.is_empty() {
        "/".to_owned()
    } else {
        rel_path.rsplit('/').next().unwrap_or(rel_path).to_owned()
    };
    let last_mod = format_rfc1123(entry.modified);
    let created = format_iso8601(entry.modified);
    xml.push_str("  <D:response>\n");
    xml.push_str(&format!("    <D:href>{}</D:href>\n", xml_escape(&href)));
    xml.push_str("    <D:propstat>\n      <D:prop>\n");
    xml.push_str(&format!("        <D:displayname>{}</D:displayname>\n", xml_escape(&display_name)));
    xml.push_str(&format!("        <D:getlastmodified>{last_mod}</D:getlastmodified>\n"));
    xml.push_str(&format!("        <D:creationdate>{created}</D:creationdate>\n"));
    if entry.is_dir {
        xml.push_str("        <D:resourcetype><D:collection/></D:resourcetype>\n");
    } else {
        xml.push_str("        <D:resourcetype/>\n");
        xml.push_str(&format!("        <D:getcontentlength>{}</D:getcontentlength>\n", entry.size));
        xml.push_str(&format!("        <D:getetag>\"{}-{}\"</D:getetag>\n", entry.size, entry.modified));
    }
    xml.push_str("      </D:prop>\n      <D:status>HTTP/1.1 200 OK</D:status>\n    </D:propstat>\n");
    xml.push_str("  </D:response>\n");
}

fn handle_get_or_head(
    req: &HttpRequest,
    stream: &mut TcpStream,
    device: DeviceId,
    cache: &Mutex<DirCache>,
    rel: &str,
    head_only: bool,
) -> bool {
    if rel.is_empty() || is_finder_metadata(rel) {
        return send_empty_status(stream, "404 Not Found");
    }
    let entry = match fetch_entry(device, cache, rel) {
        Ok(Some(e)) => e,
        Ok(None) => return send_empty_status(stream, "404 Not Found"),
        Err(status) => return send_empty_status(stream, status),
    };
    if entry.is_dir {
        return send_empty_status(stream, "405 Method Not Allowed");
    }

    let total = entry.size;
    let last_mod = format_rfc1123(entry.modified);
    let etag = format!("\"{}-{}\"", entry.size, entry.modified);

    let (status_line, offset, length, content_range) = if let Some(range_hdr) = req.headers.get("range") {
        match parse_range_header(range_hdr, total) {
            Some((start, end)) => {
                let len = end - start + 1;
                ("206 Partial Content", start, len, Some(format!("bytes {start}-{end}/{total}")))
            }
            None => return send_empty_status(stream, "416 Range Not Satisfiable"),
        }
    } else {
        ("200 OK", 0, total, None)
    };

    let mut resp = format!(
        "HTTP/1.1 {status_line}\r\n\
         Accept-Ranges: bytes\r\n\
         Content-Type: application/octet-stream\r\n\
         Content-Length: {length}\r\n\
         Last-Modified: {last_mod}\r\n\
         ETag: {etag}\r\n"
    );
    if let Some(cr) = content_range {
        resp.push_str(&format!("Content-Range: {cr}\r\n"));
    }
    resp.push_str("\r\n");

    if stream.write_all(resp.as_bytes()).is_err() {
        return false;
    }
    if head_only || length == 0 {
        return true;
    }

    let Some(node) = core_host::node() else { return false };
    let rel_owned = rel.to_owned();
    let res = core_host::host().runtime.block_on(async move {
        let (_, mut recv) = node
            .storage_read_stream(device, &rel_owned, offset, Some(length))
            .await
            .map_err(|e| core_err_to_http(&e))?;
        let mut remaining = length;
        while remaining > 0 {
            let want = (remaining as usize).min(STREAM_READ_CHUNK_BYTES);
            match recv.read_chunk(want).await {
                Ok(Some(chunk)) if !chunk.is_empty() => {
                    let take = chunk.len().min(remaining as usize);
                    stream.write_all(&chunk[..take]).map_err(|_| "500 Internal Server Error")?;
                    remaining -= take as u64;
                }
                Ok(_) => break,
                Err(_) => return Err("502 Bad Gateway"),
            }
        }
        Ok::<(), &'static str>(())
    });
    res.is_ok()
}

fn handle_put(
    req: &HttpRequest,
    stream: &mut TcpStream,
    device: DeviceId,
    cache: &Mutex<DirCache>,
    rel: &str,
) -> bool {
    if rel.is_empty() {
        return send_empty_status(stream, "405 Method Not Allowed");
    }
    if is_finder_metadata(rel) {
        return send_empty_status(stream, "201 Created");
    }
    let Some(node) = core_host::node() else {
        return send_empty_status(stream, "503 Service Unavailable");
    };

    let tmp_holder;
    let source_path: &Path = match &req.body {
        RequestBody::File(f) => f.path(),
        RequestBody::Empty => {
            let Ok(f) = StageFile::create() else {
                return send_empty_status(stream, "500 Internal Server Error");
            };
            tmp_holder = f;
            tmp_holder.path()
        }
        RequestBody::Bytes(b) => {
            let Ok(mut f) = StageFile::create() else {
                return send_empty_status(stream, "500 Internal Server Error");
            };
            if f.file.write_all(b).is_err() || f.file.flush().is_err() {
                return send_empty_status(stream, "500 Internal Server Error");
            }
            tmp_holder = f;
            tmp_holder.path()
        }
    };

    let rel_owned = rel.to_owned();
    let src = source_path.to_owned();
    let res =
        core_host::host().runtime.block_on(async move { node.storage_write(device, &rel_owned, &src).await });
    match res {
        Ok(_) => {
            lock(cache).invalidate(rel);
            send_empty_status(stream, "201 Created")
        }
        Err(e) => send_empty_status(stream, core_err_to_http(&e)),
    }
}

fn handle_mkcol(stream: &mut TcpStream, device: DeviceId, cache: &Mutex<DirCache>, rel: &str) -> bool {
    if rel.is_empty() {
        return send_empty_status(stream, "405 Method Not Allowed");
    }
    if is_finder_metadata(rel) {
        return send_empty_status(stream, "201 Created");
    }
    let Some(node) = core_host::node() else {
        return send_empty_status(stream, "503 Service Unavailable");
    };
    let rel_owned = rel.to_owned();
    let res = core_host::host().runtime.block_on(async move { node.storage_mkdir(device, &rel_owned).await });
    match res {
        Ok(()) => {
            lock(cache).invalidate(rel);
            send_empty_status(stream, "201 Created")
        }
        Err(e) => send_empty_status(stream, core_err_to_http(&e)),
    }
}

fn handle_delete(stream: &mut TcpStream, device: DeviceId, cache: &Mutex<DirCache>, rel: &str) -> bool {
    if rel.is_empty() {
        return send_empty_status(stream, "405 Method Not Allowed");
    }
    if is_finder_metadata(rel) {
        return send_empty_status(stream, "204 No Content");
    }
    let Some(node) = core_host::node() else {
        return send_empty_status(stream, "503 Service Unavailable");
    };
    let rel_owned = rel.to_owned();
    let res = core_host::host()
        .runtime
        .block_on(async move { node.storage_delete(device, &rel_owned, true).await });
    match res {
        Ok(()) => {
            lock(cache).invalidate(rel);
            send_empty_status(stream, "204 No Content")
        }
        Err(e) => send_empty_status(stream, core_err_to_http(&e)),
    }
}

fn handle_move(
    req: &HttpRequest,
    stream: &mut TcpStream,
    device: DeviceId,
    cache: &Mutex<DirCache>,
    rel: &str,
) -> bool {
    if rel.is_empty() {
        return send_empty_status(stream, "405 Method Not Allowed");
    }
    let Some(dest_hdr) = req.headers.get("destination") else {
        return send_empty_status(stream, "400 Bad Request");
    };
    let Some(dest_rel) = normalize_uri_path(dest_hdr) else {
        return send_empty_status(stream, "400 Bad Request");
    };
    if dest_rel.is_empty() {
        return send_empty_status(stream, "403 Forbidden");
    }
    if is_finder_metadata(rel) || is_finder_metadata(&dest_rel) {
        return send_empty_status(stream, "201 Created");
    }
    let Some(node) = core_host::node() else {
        return send_empty_status(stream, "503 Service Unavailable");
    };
    let old_owned = rel.to_owned();
    let new_owned = dest_rel.clone();
    let res = core_host::host()
        .runtime
        .block_on(async move { node.storage_rename(device, &old_owned, &new_owned).await });
    match res {
        Ok(()) => {
            let mut c = lock(cache);
            c.invalidate(rel);
            c.invalidate(&dest_rel);
            send_empty_status(stream, "201 Created")
        }
        Err(e) => send_empty_status(stream, core_err_to_http(&e)),
    }
}

fn handle_proppatch(stream: &mut TcpStream, rel: &str) -> bool {
    let href = encode_href(rel, false);
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <D:multistatus xmlns:D=\"DAV:\">\n\
           <D:response>\n\
             <D:href>{}</D:href>\n\
             <D:propstat>\n\
               <D:prop/>\n\
               <D:status>HTTP/1.1 200 OK</D:status>\n\
             </D:propstat>\n\
           </D:response>\n\
         </D:multistatus>\n",
        xml_escape(&href)
    );
    send_xml_status(stream, "207 Multi-Status", &xml, &[])
}

fn handle_lock(stream: &mut TcpStream, _rel: &str) -> bool {
    let token = "opaquelocktoken:nectarlink-webdav-1";
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <D:prop xmlns:D=\"DAV:\">\n\
           <D:lockdiscovery>\n\
             <D:activelock>\n\
               <D:locktype><D:write/></D:locktype>\n\
               <D:lockscope><D:exclusive/></D:lockscope>\n\
               <D:depth>0</D:depth>\n\
               <D:timeout>Second-3600</D:timeout>\n\
               <D:locktoken><D:href>{token}</D:href></D:locktoken>\n\
             </D:activelock>\n\
           </D:lockdiscovery>\n\
         </D:prop>\n"
    );
    let hdr_val = format!("<{token}>");
    send_xml_status(stream, "200 OK", &xml, &[("Lock-Token", &hdr_val)])
}

fn fetch_dir(
    device: DeviceId,
    cache: &Mutex<DirCache>,
    dir_rel: &str,
) -> Result<Vec<StorageEntry>, &'static str> {
    if let Some(cached) = lock(cache).get_dir(dir_rel) {
        return Ok(cached);
    }
    let Some(node) = core_host::node() else {
        return Err("503 Service Unavailable");
    };
    let rel_owned = dir_rel.to_owned();
    let entries = core_host::host()
        .runtime
        .block_on(async move { node.storage_list(device, &rel_owned).await })
        .map_err(|e| core_err_to_http(&e))?;
    lock(cache).insert_dir(dir_rel.to_owned(), entries.clone());
    Ok(entries)
}

fn fetch_entry(
    device: DeviceId,
    cache: &Mutex<DirCache>,
    rel_path: &str,
) -> Result<Option<StorageEntry>, &'static str> {
    let (parent, name) = match rel_path.rsplit_once('/') {
        Some((p, n)) => (p, n),
        None => ("", rel_path),
    };
    let entries = fetch_dir(device, cache, parent)?;
    Ok(entries.into_iter().find(|e| e.name.trim_matches('/') == name))
}

fn core_err_to_http(error: &nectarlink_core::Error) -> &'static str {
    use nectarlink_core::Error;
    match error {
        Error::Offline | Error::NotPaired => "503 Service Unavailable",
        Error::Denied | Error::Declined => "403 Forbidden",
        Error::NotFound => "404 Not Found",
        Error::Timeout => "504 Gateway Timeout",
        Error::Unsupported => "501 Not Implemented",
        Error::TooLarge => "413 Payload Too Large",
        _ => "500 Internal Server Error",
    }
}

fn join_rel(parent: &str, name: &str) -> String {
    if parent.is_empty() { name.to_owned() } else { format!("{parent}/{name}") }
}

fn is_finder_metadata(rel_path: &str) -> bool {
    let name = rel_path.rsplit('/').next().unwrap_or(rel_path);
    name.starts_with("._")
        || matches!(
            name,
            ".DS_Store"
                | ".localized"
                | ".VolumeIcon.icns"
                | ".Spotlight-V100"
                | ".Trashes"
                | ".fseventsd"
                | ".TemporaryItems"
        )
}

fn normalize_uri_path(raw: &str) -> Option<String> {
    let without_scheme = if let Some(rest) = raw.strip_prefix("http://") {
        rest.find('/').map(|idx| &rest[idx..]).unwrap_or("/")
    } else if let Some(rest) = raw.strip_prefix("https://") {
        rest.find('/').map(|idx| &rest[idx..]).unwrap_or("/")
    } else {
        raw
    };
    let path_only = without_scheme.split('?').next().unwrap_or("");
    let decoded = percent_decode(path_only)?;
    let mut segments = Vec::new();
    for seg in decoded.split('/') {
        match seg {
            "" | "." => continue,
            ".." => return None,
            s => segments.push(s),
        }
    }
    Some(segments.join("/"))
}

fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let hi = hex_val(bytes[i + 1])?;
            let lo = hex_val(bytes[i + 2])?;
            out.push((hi << 4) | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn encode_href(rel_path: &str, is_dir: bool) -> String {
    if rel_path.is_empty() {
        return "/".to_owned();
    }
    let mut out = String::with_capacity(rel_path.len() + 2);
    for seg in rel_path.split('/') {
        out.push('/');
        for b in seg.bytes() {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                out.push(b as char);
            } else {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    if is_dir && !out.ends_with('/') {
        out.push('/');
    }
    out
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

fn parse_range_header(header: &str, total: u64) -> Option<(u64, u64)> {
    if total == 0 {
        return None;
    }
    let spec = header.trim().strip_prefix("bytes=")?;
    let (start_s, end_s) = spec.split_once('-')?;
    if start_s.is_empty() {
        let suffix = end_s.trim().parse::<u64>().ok()?;
        if suffix == 0 {
            return None;
        }
        let start = total.saturating_sub(suffix);
        return Some((start, total - 1));
    }
    let start = start_s.trim().parse::<u64>().ok()?;
    if start >= total {
        return None;
    }
    let end =
        if end_s.trim().is_empty() { total - 1 } else { end_s.trim().parse::<u64>().ok()?.min(total - 1) };
    if start > end {
        return None;
    }
    Some((start, end))
}

fn unix_ms_to_utc_parts(unix_ms: i64) -> (i64, u32, u32, u32, u32, u32, u32) {
    let secs = unix_ms.max(0) / 1000;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400) as u32;
    let hour = rem / 3600;
    let min = (rem % 3600) / 60;
    let sec = rem % 60;
    // 1970-01-01 was a Thursday (index 4 where 0 = Sun).
    let wday = ((days + 4).rem_euclid(7)) as u32;

    // Howard Hinnant's civil-from-days algorithm.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + if month <= 2 { 1 } else { 0 };
    (year, month, day, hour, min, sec, wday)
}

fn format_rfc1123(unix_ms: i64) -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] =
        ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let (year, month, day, hour, min, sec, wday) = unix_ms_to_utc_parts(unix_ms);
    let d_name = DAYS[wday as usize];
    let m_name = MONTHS[(month - 1) as usize];
    format!("{d_name}, {day:02} {m_name} {year:04} {hour:02}:{min:02}:{sec:02} GMT")
}

fn format_iso8601(unix_ms: i64) -> String {
    let (year, month, day, hour, min, sec, _) = unix_ms_to_utc_parts(unix_ms);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_and_decodes_uri_paths() {
        assert_eq!(normalize_uri_path("/"), Some(String::new()));
        assert_eq!(normalize_uri_path("/DCIM/Camera/photo%201.jpg"), Some("DCIM/Camera/photo 1.jpg".into()));
        assert_eq!(
            normalize_uri_path("http://127.0.0.1:45678/Download/caf%C3%A9.txt?foo=1"),
            Some("Download/café.txt".into())
        );
        assert_eq!(normalize_uri_path("/DCIM/../secret"), None);
        assert_eq!(normalize_uri_path("/bad%ZZ"), None);
    }

    #[test]
    fn encodes_hrefs_and_escapes_xml() {
        assert_eq!(encode_href("", true), "/");
        assert_eq!(encode_href("DCIM/My Photos", true), "/DCIM/My%20Photos/");
        assert_eq!(encode_href("DCIM/a&b.jpg", false), "/DCIM/a%26b.jpg");
        assert_eq!(xml_escape("A & B <1>"), "A &amp; B &lt;1&gt;");
    }

    #[test]
    fn formats_http_and_iso_dates() {
        assert_eq!(format_rfc1123(0), "Thu, 01 Jan 1970 00:00:00 GMT");
        assert_eq!(format_iso8601(0), "1970-01-01T00:00:00Z");
        // 2024-03-01T12:34:56Z = 1709296496000 ms
        assert_eq!(format_rfc1123(1_709_296_496_000), "Fri, 01 Mar 2024 12:34:56 GMT");
        assert_eq!(format_iso8601(1_709_296_496_000), "2024-03-01T12:34:56Z");
    }

    #[test]
    fn parses_byte_ranges() {
        assert_eq!(parse_range_header("bytes=0-99", 1000), Some((0, 99)));
        assert_eq!(parse_range_header("bytes=500-", 1000), Some((500, 999)));
        assert_eq!(parse_range_header("bytes=500-5000", 1000), Some((500, 999)));
        assert_eq!(parse_range_header("bytes=-100", 1000), Some((900, 999)));
        assert_eq!(parse_range_header("bytes=1000-", 1000), None);
        assert_eq!(parse_range_header("bytes=500-400", 1000), None);
        assert_eq!(parse_range_header("bytes=0-0", 0), None);
    }

    #[test]
    fn filters_finder_metadata_files() {
        assert!(is_finder_metadata(".DS_Store"));
        assert!(is_finder_metadata("DCIM/.DS_Store"));
        assert!(is_finder_metadata("DCIM/._IMG_001.jpg"));
        assert!(is_finder_metadata(".Spotlight-V100"));
        assert!(!is_finder_metadata("DCIM/IMG_001.jpg"));
    }

    #[test]
    fn builds_mount_webdav_args() {
        let args = mount_webdav_args(51234, "Pixel 9", Path::new("/Users/alice/Nectarlink/Pixel 9"));
        assert_eq!(
            args,
            vec!["-S", "-v", "Pixel 9", "http://127.0.0.1:51234/", "/Users/alice/Nectarlink/Pixel 9"]
        );
    }

    #[test]
    fn decodes_chunked_transfer_encoding() {
        let raw = b"5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let writer = std::thread::spawn(move || {
            let mut s = TcpStream::connect(addr).unwrap();
            s.write_all(raw).unwrap();
        });
        let (sock, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(sock);
        let decoded = read_chunked_bytes(&mut reader, 1024).unwrap();
        writer.join().unwrap();
        assert_eq!(decoded, b"hello world");
    }
}
