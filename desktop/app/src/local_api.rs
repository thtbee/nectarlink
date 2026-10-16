// SPDX-License-Identifier: GPL-3.0-or-later
//! Local authenticated HTTP/JSON automation API (`127.0.0.1:<port>`, default
//! `53320`, off by default): lets local tools (such as the NectarLink PowerToys
//! Command Palette / Run extension, Raycast scripts, AutoHotkey, or CLI scripts)
//! list paired phones, send files/links/clipboard text, trigger Continuity
//! Camera, ring a phone, or read recent timeline and clipboard items.
//!
//! Security model:
//! - Strictly bound to loopback (`127.0.0.1`).
//! - Every request must carry `Authorization: Bearer <token>` matching the
//!   256-bit random hex secret in `<data_dir>/api-token.json` (verified in
//!   constant time).
//! - Any request carrying an `Origin` header is rejected with `403 Forbidden`
//!   so web pages opened in a browser cannot invoke the loopback API.

#![allow(dead_code)]

use std::{
    collections::HashMap,
    fs,
    io::Write,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use data_encoding::HEXLOWER;
use nectarlink_core::{DeviceId, LinkState, TaskNotify};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
};

use crate::settings::{DEFAULT_LOCAL_API_PORT, Settings};

/// Default loopback TCP port (`127.0.0.1:53320`).
pub const DEFAULT_PORT: u16 = DEFAULT_LOCAL_API_PORT;

/// File name inside `<data_dir>` storing the active port and 256-bit bearer token.
pub const TOKEN_FILE_NAME: &str = "api-token.json";

const MAX_REQUEST_BYTES: usize = 256 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// Contents of `<data_dir>/api-token.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiTokenFile {
    pub port: u16,
    pub token: String,
}

/// Handle to a running loopback API server instance.
#[derive(Debug)]
pub struct ServerHandle {
    addr: SocketAddr,
    port: u16,
    shutdown: Option<oneshot::Sender<()>>,
}

impl ServerHandle {
    /// Returns the bound loopback socket address (`127.0.0.1:<port>`).
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Stops the server loop.
    pub fn stop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

static RUNNING: OnceLock<Mutex<Option<ServerHandle>>> = OnceLock::new();

fn running_slot() -> &'static Mutex<Option<ServerHandle>> {
    RUNNING.get_or_init(|| Mutex::new(None))
}

/// Returns the path to `<data_dir>/api-token.json` for a specific `data_dir`.
pub fn token_file_path_in(data_dir: &Path) -> PathBuf {
    data_dir.join(TOKEN_FILE_NAME)
}

/// Returns the path to `<data_dir>/api-token.json` for the running desktop host.
pub fn token_file_path() -> PathBuf {
    token_file_path_in(&crate::core_host::host().data_dir)
}

/// Generates a 256-bit (32-byte) cryptographically random token formatted as
/// 64 lowercase hexadecimal characters, from the system's secure random generator.
/// `None` if no secure source answered: the API then stays off rather than run with a
/// guessable token.
pub fn generate_token() -> Option<String> {
    let mut bytes = [0u8; 32];
    crate::win::secure_random(&mut bytes).then(|| HEXLOWER.encode(&bytes))
}

fn is_valid_token_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Reads and validates `<data_dir>/api-token.json` if present.
pub fn read_token_file(data_dir: &Path) -> Option<ApiTokenFile> {
    let text = fs::read_to_string(token_file_path_in(data_dir)).ok()?;
    let parsed: ApiTokenFile = serde_json::from_str(&text).ok()?;
    if parsed.port > 0 && is_valid_token_hex(&parsed.token) { Some(parsed) } else { None }
}

/// Atomically writes `<data_dir>/api-token.json` with `port` and `token`.
pub fn write_token_file(data_dir: &Path, port: u16, token: &str) -> std::io::Result<PathBuf> {
    fs::create_dir_all(data_dir)?;
    let path = token_file_path_in(data_dir);
    let tmp = path.with_extension("json.tmp");
    let data = ApiTokenFile { port, token: token.to_owned() };
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(serde_json::to_string_pretty(&data)?.as_bytes())?;
        file.sync_all()?;
    }
    fs::rename(&tmp, &path)?;
    Ok(path)
}

/// Ensures `<data_dir>/api-token.json` exists with a 64-hex-char 256-bit token
/// and the given `port`.
pub fn ensure_token_file(data_dir: &Path, port: u16) -> std::io::Result<ApiTokenFile> {
    let token = match read_token_file(data_dir) {
        Some(existing) => existing.token,
        None => generate_token()
            .ok_or_else(|| std::io::Error::other("no secure random source for the API token"))?,
    };
    write_token_file(data_dir, port, &token)?;
    Ok(ApiTokenFile { port, token })
}

/// Removes `<data_dir>/api-token.json` when the local API is disabled.
pub fn remove_token_file(data_dir: &Path) {
    let path = token_file_path_in(data_dir);
    if path.exists() {
        let _ = fs::remove_file(path);
    }
}

/// Constant-time string comparison for bearer tokens.
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    let max_len = ab.len().max(bb.len());
    let mut diff = ab.len() ^ bb.len();
    for i in 0..max_len {
        let x = ab.get(i).copied().unwrap_or(0);
        let y = bb.get(i).copied().unwrap_or(0);
        diff |= usize::from(x ^ y);
    }
    diff == 0
}

/// Synchronizes the running localhost API server with `desktop-settings.json`.
pub fn sync_from_settings() {
    let data_dir = crate::core_host::host().data_dir.clone();
    let settings = Settings::load(&data_dir);
    apply_settings(&data_dir, &settings);
}

/// Initializes the localhost API server from `settings` on startup.
pub fn init(settings: &Settings) {
    let data_dir = crate::core_host::host().data_dir.clone();
    apply_settings(&data_dir, settings);
}

/// Stops the running localhost API server, if any.
pub fn stop() {
    if let Ok(mut guard) = running_slot().lock() {
        *guard = None;
    }
}

fn apply_settings(data_dir: &Path, settings: &Settings) {
    let port = if settings.local_api_port == 0 { DEFAULT_PORT } else { settings.local_api_port };
    if !settings.local_api_enabled {
        stop();
        remove_token_file(data_dir);
        return;
    }
    if let Ok(guard) = running_slot().lock()
        && let Some(active) = guard.as_ref()
        && active.port == port
        && read_token_file(data_dir).is_some()
    {
        return;
    }
    let token_file = match ensure_token_file(data_dir, port) {
        Ok(tf) => tf,
        Err(e) => {
            tracing::warn!(error = %e, "can't write api-token.json");
            return;
        }
    };
    let dir = data_dir.to_path_buf();
    crate::core_host::spawn(async move {
        match start_on_port(port, token_file.token.clone()).await {
            Ok((addr, handle)) => {
                let _ = write_token_file(&dir, addr.port(), &token_file.token);
                if let Ok(mut guard) = running_slot().lock() {
                    *guard = Some(handle);
                }
                tracing::info!(%addr, "started local automation API");
            }
            Err(e) => {
                tracing::warn!(port, error = %e, "can't bind local automation API");
            }
        }
    });
}

/// Starts the localhost HTTP/JSON server on `127.0.0.1:<port>` (pass `0` in
/// tests for an OS-assigned ephemeral port).
pub async fn start_on_port(port: u16, token: String) -> std::io::Result<(SocketAddr, ServerHandle)> {
    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    let addr = listener.local_addr()?;
    let actual_port = addr.port();
    let (tx, mut rx) = oneshot::channel::<()>();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut rx => break,
                accepted = listener.accept() => {
                    let Ok((stream, _peer)) = accepted else { continue };
                    let expected_token = token.clone();
                    tokio::spawn(async move {
                        let _ = tokio::time::timeout(
                            IO_TIMEOUT,
                            handle_connection(stream, &expected_token),
                        )
                        .await;
                    });
                }
            }
        }
    });

    Ok((addr, ServerHandle { addr, port: actual_port, shutdown: Some(tx) }))
}

#[derive(Debug)]
struct HttpRequest {
    method: String,
    path: String,
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

async fn read_http_request(stream: &mut TcpStream) -> std::io::Result<Option<HttpRequest>> {
    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0u8; 2048];
    let header_end = loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos;
        }
        if buf.len() >= MAX_REQUEST_BYTES {
            return Ok(None);
        }
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(None);
        }
        buf.extend_from_slice(&chunk[..n]);
    };

    let header_bytes = &buf[..header_end];
    let header_text = String::from_utf8_lossy(header_bytes);
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_ascii_uppercase();
    let raw_target = parts.next().unwrap_or("/");
    if method.is_empty() {
        return Ok(None);
    }

    let (path, query) = parse_target(raw_target);
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
        }
    }

    let content_len = headers
        .get("content-length")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0)
        .min(MAX_REQUEST_BYTES);

    let body_start = header_end + 4;
    let mut body = Vec::with_capacity(content_len);
    if buf.len() > body_start {
        let available = (buf.len() - body_start).min(content_len);
        body.extend_from_slice(&buf[body_start..body_start + available]);
    }
    while body.len() < content_len {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        let take = n.min(content_len - body.len());
        body.extend_from_slice(&chunk[..take]);
    }

    Ok(Some(HttpRequest { method, path, query, headers, body }))
}

fn parse_target(target: &str) -> (String, HashMap<String, String>) {
    let mut query = HashMap::new();
    let (path, qs) = match target.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (target, None),
    };
    if let Some(qs) = qs {
        for pair in qs.split('&') {
            if pair.is_empty() {
                continue;
            }
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            query.insert(percent_decode(k), percent_decode(v));
        }
    }
    (path.to_owned(), query)
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (from_hex(bytes[i + 1]), from_hex(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        } else if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

async fn write_json_response(
    stream: &mut TcpStream,
    status_code: u16,
    status_text: &str,
    body: &Value,
) -> std::io::Result<()> {
    let payload = serde_json::to_vec(body).unwrap_or_else(|_| b"{}".to_vec());
    let header = format!(
        "HTTP/1.1 {status_code} {status_text}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(&payload).await?;
    stream.flush().await
}

async fn handle_connection(mut stream: TcpStream, expected_token: &str) -> std::io::Result<()> {
    let Some(req) = read_http_request(&mut stream).await? else {
        return Ok(());
    };

    // 1. Reject browser cross-origin requests carrying an `Origin` header.
    if req.headers.contains_key("origin") {
        return write_json_response(
            &mut stream,
            403,
            "Forbidden",
            &json!({ "ok": false, "error": "forbidden" }),
        )
        .await;
    }

    // 2. Verify `Authorization: Bearer <token>` in constant time.
    let provided = req
        .headers
        .get("authorization")
        .and_then(|v| {
            let trimmed = v.trim();
            if trimmed.len() >= 7 && trimmed[..7].eq_ignore_ascii_case("bearer ") {
                Some(trimmed[7..].trim())
            } else {
                None
            }
        })
        .unwrap_or("");
    if provided.is_empty() || !constant_time_eq(provided, expected_token) {
        return write_json_response(
            &mut stream,
            401,
            "Unauthorized",
            &json!({ "ok": false, "error": "unauthorized" }),
        )
        .await;
    }

    let (status, reason, payload) = route_request(req).await;
    write_json_response(&mut stream, status, reason, &payload).await
}

#[cfg(not(test))]
fn with_state<T>(f: impl FnOnce(&crate::state::AppState) -> T) -> Option<T> {
    Some(crate::core_host::host().hub.read(f))
}

#[cfg(test)]
fn with_state<T>(_f: impl FnOnce(&crate::state::AppState) -> T) -> Option<T> {
    None
}

#[cfg(not(test))]
fn try_node() -> Option<nectarlink_core::Node> {
    crate::core_host::node()
}

#[cfg(test)]
fn try_node() -> Option<nectarlink_core::Node> {
    None
}

#[cfg(test)]
static TEST_CLIPBOARD: Mutex<String> = Mutex::new(String::new());

fn read_pc_clipboard() -> String {
    #[cfg(test)]
    {
        return TEST_CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner()).clone();
    }
    #[cfg(not(test))]
    {
        match crate::win::clipboard::read() {
            crate::win::clipboard::Clip::Text { text, .. } => text,
            _ => String::new(),
        }
    }
}

fn write_pc_clipboard(text: &str) -> Result<(), String> {
    #[cfg(test)]
    {
        *TEST_CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner()) = text.to_owned();
        Ok(())
    }
    #[cfg(not(test))]
    {
        crate::win::clipboard::write(text)
    }
}

fn resolve_target_device(requested: Option<&str>) -> Option<DeviceId> {
    let query = requested.map(str::trim).filter(|s| !s.is_empty());
    if let Some(q) = query
        && let Ok(id) = q.parse::<DeviceId>()
    {
        return Some(id);
    }
    with_state(|s| {
        if let Some(q) = query {
            let lower = q.to_ascii_lowercase();
            if let Some(found) = s.devices.iter().find(|d| {
                d.id.to_string().starts_with(&lower) || d.info.name.to_ascii_lowercase().contains(&lower)
            }) {
                return Some(found.id);
            }
        }
        s.devices
            .iter()
            .find(|d| matches!(d.link, LinkState::Online { .. }))
            .or_else(|| s.devices.first())
            .map(|d| d.id)
    })
    .flatten()
}

async fn route_request(req: HttpRequest) -> (u16, &'static str, Value) {
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/v1/devices") => handle_get_devices(),
        ("POST", "/v1/send") => handle_post_send(&req.body).await,
        ("GET", "/v1/clipboard") => handle_get_clipboard(),
        ("POST", "/v1/clipboard") => handle_post_clipboard(&req.body).await,
        ("POST", "/v1/notify") => handle_post_notify(&req.body).await,
        ("POST", "/v1/ring") => handle_post_ring(&req.body).await,
        ("POST", "/v1/camera") => handle_post_camera(&req.body),
        ("GET", "/v1/timeline") => handle_get_timeline(&req.query),
        _ => (404, "Not Found", json!({ "ok": false, "error": "not_found" })),
    }
}

fn handle_get_devices() -> (u16, &'static str, Value) {
    let devices = with_state(|s| {
        s.devices
            .iter()
            .map(|d| {
                let (online, link_kind) = match &d.link {
                    LinkState::Online { path, .. } => (
                        true,
                        match path {
                            nectarlink_core::ConnectionPath::Lan => "lan",
                            nectarlink_core::ConnectionPath::Usb => "usb",
                            nectarlink_core::ConnectionPath::Relay => "relay",
                        },
                    ),
                    LinkState::Connecting => (false, "connecting"),
                    LinkState::Offline { .. } => (false, "offline"),
                };
                let battery = d.battery.as_ref().map(|b| {
                    json!({
                        "level": b.level,
                        "charging": b.charging,
                    })
                });
                json!({
                    "id": d.id.to_string(),
                    "name": d.info.name,
                    "platform": d.info.os,
                    "online": online,
                    "link": link_kind,
                    "battery": battery,
                })
            })
            .collect::<Vec<_>>()
    })
    .unwrap_or_default();

    (200, "OK", json!({ "ok": true, "devices": devices }))
}

#[derive(Debug, Default, Deserialize)]
struct SendPayload {
    #[serde(default)]
    device: Option<String>,
    #[serde(default)]
    paths: Vec<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

async fn handle_post_send(body: &[u8]) -> (u16, &'static str, Value) {
    let payload: SendPayload = match serde_json::from_slice(body) {
        Ok(p) => p,
        Err(_) => return (400, "Bad Request", json!({ "ok": false, "error": "invalid_json" })),
    };
    if payload.paths.is_empty()
        && payload.text.as_deref().unwrap_or("").trim().is_empty()
        && payload.url.as_deref().unwrap_or("").trim().is_empty()
    {
        return (400, "Bad Request", json!({ "ok": false, "error": "empty_send_payload" }));
    }

    let target = resolve_target_device(payload.device.as_deref());
    if let (Some(device), Some(node)) = (target, try_node()) {
        if !payload.paths.is_empty() {
            let paths: Vec<PathBuf> = payload.paths.iter().map(PathBuf::from).collect();
            if let Ok(Ok(outgoing)) =
                tokio::task::spawn_blocking(move || nectarlink_core::outgoing_paths(&paths)).await
                && !outgoing.is_empty()
            {
                let _ = node.send_files(device, outgoing).await;
            }
        }
        if let Some(url) = payload.url.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            crate::links::send_to(device, url.to_owned());
        }
        if let Some(text) = payload.text.as_deref().filter(|s| !s.is_empty()) {
            let _ = node.send_clipboard(device, text.to_owned()).await;
        }
    }

    (200, "OK", json!({ "ok": true }))
}

fn handle_get_clipboard() -> (u16, &'static str, Value) {
    let text = read_pc_clipboard();
    let history = try_node()
        .map(|node| {
            node.clipboard_history(None)
                .into_iter()
                .take(20)
                .map(|item| {
                    let kind = match item.kind {
                        nectarlink_core::ClipboardItemKind::Text => "text",
                        nectarlink_core::ClipboardItemKind::Image => "image",
                    };
                    json!({
                        "id": item.id,
                        "kind": kind,
                        "preview": item.text,
                        "deviceName": item.device_name,
                        "incoming": item.incoming,
                        "timestamp": item.timestamp,
                        "pinned": item.pinned,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    (200, "OK", json!({ "ok": true, "text": text, "history": history }))
}

#[derive(Debug, Default, Deserialize)]
struct ClipboardPayload {
    text: String,
    #[serde(default)]
    device: Option<String>,
}

async fn handle_post_clipboard(body: &[u8]) -> (u16, &'static str, Value) {
    let payload: ClipboardPayload = match serde_json::from_slice(body) {
        Ok(p) => p,
        Err(_) => return (400, "Bad Request", json!({ "ok": false, "error": "invalid_json" })),
    };
    if let Err(e) = write_pc_clipboard(&payload.text) {
        return (500, "Internal Server Error", json!({ "ok": false, "error": e }));
    }
    if let (Some(device), Some(node)) = (resolve_target_device(payload.device.as_deref()), try_node()) {
        let _ = node.send_clipboard(device, payload.text).await;
    }
    (200, "OK", json!({ "ok": true }))
}

#[derive(Debug, Default, Deserialize)]
struct NotifyPayload {
    title: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    device: Option<String>,
}

async fn handle_post_notify(body: &[u8]) -> (u16, &'static str, Value) {
    let payload: NotifyPayload = match serde_json::from_slice(body) {
        Ok(p) => p,
        Err(_) => return (400, "Bad Request", json!({ "ok": false, "error": "invalid_json" })),
    };
    if payload.title.trim().is_empty() {
        return (400, "Bad Request", json!({ "ok": false, "error": "missing_title" }));
    }
    let full_title = match payload.body.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(b) => format!("{}: {b}", payload.title.trim()),
        None => payload.title.trim().to_owned(),
    };
    let task = TaskNotify {
        id: format!("api-{}", SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis())),
        title: full_title,
        active: false,
        elapsed_ms: 0,
        exit_code: Some(0),
    };
    if let Some(node) = try_node() {
        let target = resolve_target_device(payload.device.as_deref());
        let _ = node.task_notify(target, task).await;
    }
    (200, "OK", json!({ "ok": true }))
}

#[derive(Debug, Default, Deserialize)]
struct RingPayload {
    #[serde(default)]
    device: Option<String>,
}

async fn handle_post_ring(body: &[u8]) -> (u16, &'static str, Value) {
    let payload: RingPayload = if body.is_empty() {
        RingPayload::default()
    } else {
        match serde_json::from_slice(body) {
            Ok(p) => p,
            Err(_) => return (400, "Bad Request", json!({ "ok": false, "error": "invalid_json" })),
        }
    };
    if let (Some(device), Some(node)) = (resolve_target_device(payload.device.as_deref()), try_node()) {
        let _ = node.ring(device, true).await;
    }
    (200, "OK", json!({ "ok": true }))
}

#[derive(Debug, Default, Deserialize)]
struct CameraPayload {
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    device: Option<String>,
}

fn handle_post_camera(body: &[u8]) -> (u16, &'static str, Value) {
    let payload: CameraPayload = if body.is_empty() {
        CameraPayload::default()
    } else {
        match serde_json::from_slice(body) {
            Ok(p) => p,
            Err(_) => return (400, "Bad Request", json!({ "ok": false, "error": "invalid_json" })),
        }
    };
    let mode = match payload.mode.as_deref().map(str::trim) {
        Some("scan") => "scan",
        Some("photo") | None | Some("") => "photo",
        Some(_) => return (400, "Bad Request", json!({ "ok": false, "error": "invalid_mode" })),
    };
    #[cfg(not(test))]
    {
        let target = resolve_target_device(payload.device.as_deref());
        let index = target
            .and_then(|id| with_state(|s| s.devices.iter().position(|d| d.id == id)).flatten())
            .unwrap_or(0);
        crate::continuity_camera::start_for_index(index, mode);
    }
    #[cfg(test)]
    let _ = &payload.device;
    (200, "OK", json!({ "ok": true, "mode": mode }))
}

fn handle_get_timeline(query: &HashMap<String, String>) -> (u16, &'static str, Value) {
    let limit = query
        .get("limit")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(20)
        .clamp(1, nectarlink_core::MAX_TIMELINE_PAGE_LIMIT);
    let search = query.get("q").map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());

    let items = try_node()
        .and_then(|node| {
            node.timeline_page(&nectarlink_core::TimelineQuery {
                kind: None,
                device: None,
                search,
                offset: 0,
                limit,
            })
            .ok()
        })
        .map(|page| {
            page.entries
                .into_iter()
                .map(|entry| {
                    json!({
                        "id": entry.id,
                        "kind": entry.kind.as_str(),
                        "direction": if entry.incoming { "incoming" } else { "outgoing" },
                        "deviceId": entry.device_id.to_string(),
                        "timestamp": entry.timestamp,
                        "title": entry.title,
                        "detail": entry.detail,
                        "target": entry.target,
                        "sizeBytes": entry.size_bytes,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    (200, "OK", json!({ "ok": true, "items": items }))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn raw_http_call(addr: SocketAddr, raw_request: &str) -> (u16, Value) {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream.write_all(raw_request.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut resp = String::new();
        stream.read_to_string(&mut resp).await.unwrap();
        let (head, body) = resp.split_once("\r\n\r\n").unwrap();
        let status_line = head.lines().next().unwrap();
        let code: u16 = status_line.split_whitespace().nth(1).unwrap().parse().unwrap();
        let json: Value = serde_json::from_str(body).unwrap();
        (code, json)
    }

    #[test]
    fn generates_64_hex_char_token_and_persists_api_token_json() {
        let dir = tempfile::tempdir().unwrap();
        let tok = generate_token().expect("secure random");
        assert_eq!(tok.len(), 64);
        assert!(tok.chars().all(|c| c.is_ascii_hexdigit()));

        let created = ensure_token_file(dir.path(), 53320).unwrap();
        assert_eq!(created.port, 53320);
        assert_eq!(created.token.len(), 64);

        let loaded = read_token_file(dir.path()).unwrap();
        assert_eq!(loaded, created);

        // Calling ensure_token_file again preserves the existing token.
        let again = ensure_token_file(dir.path(), 54000).unwrap();
        assert_eq!(again.token, created.token);
        assert_eq!(again.port, 54000);

        remove_token_file(dir.path());
        assert!(read_token_file(dir.path()).is_none());
    }

    #[tokio::test]
    async fn enforces_bearer_auth_blocks_origin_header_and_serves_v1_routes() {
        let token = generate_token().expect("secure random");
        let (addr, mut handle) = start_on_port(0, token.clone()).await.unwrap();

        // 1. Missing Authorization header -> 401 Unauthorized.
        let (code, body) = raw_http_call(addr, "GET /v1/devices HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").await;
        assert_eq!(code, 401);
        assert_eq!(body["ok"], false);
        assert_eq!(body["error"], "unauthorized");

        // 2. Wrong Bearer token -> 401 Unauthorized.
        let (code, body) = raw_http_call(
            addr,
            "GET /v1/devices HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer deadbeef\r\n\r\n",
        )
        .await;
        assert_eq!(code, 401);
        assert_eq!(body["error"], "unauthorized");

        // 3. Request with Origin header (even with valid token) -> 403 Forbidden.
        let (code, body) = raw_http_call(
            addr,
            &format!(
                "GET /v1/devices HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: https://evil.example\r\nAuthorization: Bearer {token}\r\n\r\n"
            ),
        )
        .await;
        assert_eq!(code, 403);
        assert_eq!(body["ok"], false);
        assert_eq!(body["error"], "forbidden");

        // 4. Valid GET /v1/devices -> 200 OK.
        let (code, body) = raw_http_call(
            addr,
            &format!("GET /v1/devices HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\n\r\n"),
        )
        .await;
        assert_eq!(code, 200);
        assert_eq!(body["ok"], true);
        assert!(body["devices"].is_array());

        // 5. Valid POST /v1/clipboard and GET /v1/clipboard -> 200 OK.
        let clip_body = r#"{"text":"hello from local api"}"#;
        let (code, body) = raw_http_call(
            addr,
            &format!(
                "POST /v1/clipboard HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\n\r\n{clip_body}",
                clip_body.len()
            ),
        )
        .await;
        assert_eq!(code, 200);
        assert_eq!(body["ok"], true);

        let (code, body) = raw_http_call(
            addr,
            &format!(
                "GET /v1/clipboard HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\n\r\n"
            ),
        )
        .await;
        assert_eq!(code, 200);
        assert_eq!(body["ok"], true);
        assert_eq!(body["text"], "hello from local api");

        // 6. Valid POST /v1/ring, POST /v1/camera, POST /v1/notify, POST /v1/send, GET /v1/timeline.
        let (code, _) = raw_http_call(
            addr,
            &format!(
                "POST /v1/ring HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nContent-Length: 2\r\n\r\n{{}}"
            ),
        )
        .await;
        assert_eq!(code, 200);

        let cam_body = r#"{"mode":"scan"}"#;
        let (code, body) = raw_http_call(
            addr,
            &format!(
                "POST /v1/camera HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\n\r\n{cam_body}",
                cam_body.len()
            ),
        )
        .await;
        assert_eq!(code, 200);
        assert_eq!(body["mode"], "scan");

        let notify_body = r#"{"title":"Build finished","body":"0 errors"}"#;
        let (code, _) = raw_http_call(
            addr,
            &format!(
                "POST /v1/notify HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\n\r\n{notify_body}",
                notify_body.len()
            ),
        )
        .await;
        assert_eq!(code, 200);

        let send_body = r#"{"url":"https://nectarlink.dev"}"#;
        let (code, _) = raw_http_call(
            addr,
            &format!(
                "POST /v1/send HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\n\r\n{send_body}",
                send_body.len()
            ),
        )
        .await;
        assert_eq!(code, 200);

        let (code, body) = raw_http_call(
            addr,
            &format!(
                "GET /v1/timeline?limit=10&q=test HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\n\r\n"
            ),
        )
        .await;
        assert_eq!(code, 200);
        assert_eq!(body["ok"], true);
        assert!(body["items"].is_array());

        handle.stop();
    }
}
