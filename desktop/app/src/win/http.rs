// SPDX-License-Identifier: GPL-3.0-or-later
//! HTTPS GET with Windows' own HTTP stack (WinHTTP): the system's TLS,
//! proxy settings and certificates, and no extra dependencies. Used to
//! check for updates and download them.

use std::{
    io::Write,
    path::Path,
    time::{Duration, Instant},
};

use windows::{
    Win32::Networking::WinHttp::{
        WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_QUERY_CONTENT_LENGTH,
        WINHTTP_QUERY_CUSTOM, WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_FLAG_NUMBER64,
        WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest,
        WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts,
    },
    core::{HSTRING, PCWSTR, w},
};

/// Resolve, connect, send and receive limits (ms). GitHub's download host
/// can take well over ten seconds to answer on a slow or busy link.
const TIMEOUTS: (i32, i32, i32, i32) = (30_000, 30_000, 30_000, 60_000);

/// Tries in a row without progress before a download gives up.
const ATTEMPTS: u32 = 6;

/// Waits between tries; the last one repeats.
const BACKOFF: [Duration; 5] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(10),
    Duration::from_secs(20),
];

/// A handle closed when dropped.
struct Handle(*mut core::ffi::c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: a handle WinHTTP gave us, closed once.
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

fn last_error(what: &str) -> String {
    format!("{what}: {}", windows::core::Error::from_thread())
}

/// An open response: its status and, for reading, the handles behind it
/// (declared children first, so they close before their parents).
struct Response {
    request: Handle,
    _connection: Handle,
    _session: Handle,
    status: u32,
}

impl Response {
    /// The body's length when the server said, or the full size from a
    /// `Content-Range` (`bytes 100-199/200`).
    fn length(&self) -> Option<u64> {
        let mut value: u64 = 0;
        let mut size = std::mem::size_of::<u64>() as u32;
        // SAFETY: a valid request handle and an output buffer of the stated size.
        let ok = unsafe {
            WinHttpQueryHeaders(
                self.request.0,
                WINHTTP_QUERY_CONTENT_LENGTH | WINHTTP_QUERY_FLAG_NUMBER64,
                PCWSTR::null(),
                Some((&raw mut value).cast()),
                &mut size,
                std::ptr::null_mut(),
            )
        };
        ok.is_ok().then_some(value)
    }

    fn header(&self, name: &str) -> Option<String> {
        let name = HSTRING::from(name);
        let mut buffer = [0u16; 256];
        let mut size = std::mem::size_of_val(&buffer) as u32;
        // SAFETY: a valid request handle and an output buffer of the stated size.
        let ok = unsafe {
            WinHttpQueryHeaders(
                self.request.0,
                WINHTTP_QUERY_CUSTOM,
                &name,
                Some(buffer.as_mut_ptr().cast()),
                &mut size,
                std::ptr::null_mut(),
            )
        };
        ok.is_ok().then(|| String::from_utf16_lossy(&buffer[..size as usize / 2]))
    }

    /// The next chunk of the body; empty at the end.
    fn read<'a>(&self, chunk: &'a mut [u8]) -> Result<&'a [u8], String> {
        let mut read = 0u32;
        // SAFETY: a valid request handle and a buffer of the stated size.
        unsafe { WinHttpReadData(self.request.0, chunk.as_mut_ptr().cast(), chunk.len() as u32, &mut read) }
            .map_err(|e| format!("read: {e}"))?;
        Ok(&chunk[..read as usize])
    }
}

/// Sends a GET for an `https://` URL (following redirects), asking for the
/// bytes from `from` on when that's past 0, and only if it changed from
/// `etag` when there's one (a 304 answers otherwise).
fn open(url: &str, from: u64, etag: Option<&str>) -> Result<Response, String> {
    let rest = url.strip_prefix("https://").ok_or("only https")?;
    let (host, path) = rest.split_once('/').map_or((rest, "/".to_owned()), |(h, p)| (h, format!("/{p}")));
    let agent = HSTRING::from(format!("Nectarlink/{} (Windows)", env!("CARGO_PKG_VERSION")));
    // SAFETY: WinHTTP calls with valid strings; every handle is closed by
    // `Handle`.
    unsafe {
        let session = Handle(WinHttpOpen(
            &agent,
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ));
        if session.0.is_null() {
            return Err(last_error("WinHttpOpen"));
        }
        let (resolve, connect, send, receive) = TIMEOUTS;
        let _ = WinHttpSetTimeouts(session.0, resolve, connect, send, receive);
        let connection = Handle(WinHttpConnect(session.0, &HSTRING::from(host), 443, 0));
        if connection.0.is_null() {
            return Err(last_error("WinHttpConnect"));
        }
        let request = Handle(WinHttpOpenRequest(
            connection.0,
            w!("GET"),
            &HSTRING::from(path),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        ));
        if request.0.is_null() {
            return Err(last_error("WinHttpOpenRequest"));
        }
        // GitHub's API wants the Accept header; it's harmless elsewhere.
        let mut headers = "Accept: application/vnd.github+json, application/octet-stream\r\n".to_owned();
        if from > 0 {
            headers.push_str(&format!("Range: bytes={from}-\r\n"));
        }
        // An ETag is a short quoted token; anything else isn't sent.
        if let Some(etag) = etag.filter(|e| e.len() <= 200 && !e.contains(['\r', '\n'])) {
            headers.push_str(&format!("If-None-Match: {etag}\r\n"));
        }
        let headers: Vec<u16> = headers.encode_utf16().collect();
        WinHttpSendRequest(request.0, Some(&headers), None, 0, 0, 0).map_err(|e| format!("send: {e}"))?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut()).map_err(|e| format!("receive: {e}"))?;
        let mut status: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&raw mut status).cast()),
            &mut size,
            std::ptr::null_mut(),
        )
        .map_err(|e| format!("status: {e}"))?;
        Ok(Response { request, _connection: connection, _session: session, status })
    }
}

/// Fetches an `https://` URL (following redirects), up to `limit` bytes.
pub fn get(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    let response = open(url, 0, None)?;
    if response.status != 200 {
        return Err(format!("HTTP {}", response.status));
    }
    read_all(&response, limit)
}

/// What a conditional GET found.
#[derive(Debug, PartialEq, Eq)]
pub enum Fresh {
    /// The same as the copy with that ETag.
    NotModified,
    /// New content, and its ETag when the server gave one.
    Changed { body: Vec<u8>, etag: Option<String> },
}

/// Fetches an `https://` URL unless it's unchanged since `etag`: the server
/// then answers 304, a few hundred bytes with no body.
pub fn get_fresh(url: &str, limit: usize, etag: Option<&str>) -> Result<Fresh, String> {
    let response = open(url, 0, etag)?;
    match response.status {
        304 if etag.is_some() => Ok(Fresh::NotModified),
        200 => {
            let etag = response.header("ETag");
            Ok(Fresh::Changed { body: read_all(&response, limit)?, etag })
        }
        status => Err(format!("HTTP {status}")),
    }
}

/// The whole body, up to `limit` bytes.
fn read_all(response: &Response, limit: usize) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let data = response.read(&mut chunk)?;
        if data.is_empty() {
            break;
        }
        body.extend_from_slice(data);
        if body.len() > limit {
            return Err("too large".into());
        }
    }
    Ok(body)
}

/// Like [`get`], retrying network errors a few times.
pub fn get_retrying(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    let mut attempt = 0;
    loop {
        match get(url, limit) {
            Ok(body) => return Ok(body),
            Err(e) if attempt + 1 < ATTEMPTS && retryable(&e) => {
                tracing::info!(error = e, attempt, "retrying");
                std::thread::sleep(BACKOFF[(attempt as usize).min(BACKOFF.len() - 1)]);
                attempt += 1;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Network trouble or a server hiccup, worth another try. Not missing
/// files, refusals or a body that's too large.
fn retryable(error: &str) -> bool {
    match error.strip_prefix("HTTP ").and_then(|code| code.parse::<u32>().ok()) {
        Some(code) => code >= 500 || code == 408 || code == 429,
        None => !matches!(error, "too large" | "only https"),
    }
}

/// Downloads `url` into `file`, continuing from what `file` already holds
/// (a download cut short earlier, even by a restart). Retries network
/// errors, giving up after [`ATTEMPTS`] tries in a row that got nowhere.
/// `progress(done, total)` reports as it goes (`total` is 0 when unknown).
pub fn download(
    url: &str,
    file: &Path,
    limit: u64,
    mut progress: impl FnMut(u64, u64),
) -> Result<(), String> {
    let mut failures = 0;
    loop {
        let before = file.metadata().map(|m| m.len()).unwrap_or(0);
        match download_once(url, file, limit, &mut progress) {
            Ok(()) => return Ok(()),
            Err(e) if retryable(&e) => {
                let after = file.metadata().map(|m| m.len()).unwrap_or(0);
                failures = if after > before { 1 } else { failures + 1 };
                if failures >= ATTEMPTS {
                    return Err(e);
                }
                tracing::info!(error = e, done = after, "download interrupted; resuming");
                std::thread::sleep(BACKOFF[(failures as usize - 1).min(BACKOFF.len() - 1)]);
            }
            Err(e) => return Err(e),
        }
    }
}

fn download_once(
    url: &str,
    file: &Path,
    limit: u64,
    progress: &mut impl FnMut(u64, u64),
) -> Result<(), String> {
    let have = file.metadata().map(|m| m.len()).unwrap_or(0);
    let response = open(url, have, None)?;
    let (append, total) = match response.status {
        206 => {
            let total = response.header("Content-Range").and_then(|r| range_total(&r));
            (true, total.or_else(|| response.length().map(|l| have + l)).unwrap_or(0))
        }
        200 => (false, response.length().unwrap_or(0)),
        // Asked for bytes past the end: it's all here already.
        416 if have > 0 => return Ok(()),
        status => return Err(format!("HTTP {status}")),
    };
    if total > limit {
        return Err("too large".into());
    }
    let mut output = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(file)
        .map_err(|e| format!("write: {e}"))?;
    let mut done = if append { have } else { 0 };
    progress(done, total);
    let mut chunk = vec![0u8; 256 * 1024];
    let mut reported = Instant::now();
    loop {
        let data = response.read(&mut chunk)?;
        if data.is_empty() {
            break;
        }
        done += data.len() as u64;
        if done > limit {
            return Err("too large".into());
        }
        output.write_all(data).map_err(|e| format!("write: {e}"))?;
        if reported.elapsed() >= Duration::from_millis(200) {
            reported = Instant::now();
            progress(done, total);
        }
    }
    output.sync_all().map_err(|e| format!("write: {e}"))?;
    progress(done, total);
    if total > 0 && done < total {
        return Err("the download ended early".into());
    }
    Ok(())
}

/// The full size from a `Content-Range` header (`bytes 100-199/200`).
fn range_total(header: &str) -> Option<u64> {
    header.rsplit_once('/')?.1.trim().parse().ok().filter(|&n| n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_full_size_from_a_content_range() {
        assert_eq!(range_total("bytes 1048576-38473163/38473164"), Some(38_473_164));
        assert_eq!(range_total("bytes 0-99/*"), None);
        assert_eq!(range_total("garbage"), None);
    }

    #[test]
    fn retries_network_trouble_but_not_refusals() {
        assert!(retryable("receive: The operation timed out (0x80072EE2)"));
        assert!(retryable("HTTP 503"));
        assert!(retryable("HTTP 429"));
        assert!(!retryable("HTTP 404"));
        assert!(!retryable("too large"));
        assert!(!retryable("only https"));
    }

    /// Needs the internet: `cargo test -- --ignored`.
    #[test]
    #[ignore = "needs the internet"]
    fn fetches_over_https() {
        let body = get("https://api.github.com/repos/rust-lang/rust", 1024 * 1024).unwrap();
        assert!(String::from_utf8_lossy(&body).contains("\"full_name\""));
        assert_eq!(get("https://api.github.com/does-not-exist-nectarlink", 1024).unwrap_err(), "HTTP 404");
        assert_eq!(get("http://example.com", 1024).unwrap_err(), "only https");
    }

    /// Needs the internet: resumes a download from the middle.
    #[test]
    #[ignore = "needs the internet"]
    fn resumes_a_download() {
        let url = "https://github.com/thtbee/nectarlink/releases/download/v0.1.0/SHA256SUMS.txt";
        let dir = tempfile::tempdir().unwrap();
        let whole = dir.path().join("whole");
        download(url, &whole, 1 << 20, |_, _| {}).unwrap();
        let full = std::fs::read(&whole).unwrap();
        let part = dir.path().join("part");
        std::fs::write(&part, &full[..50]).unwrap();
        download(url, &part, 1 << 20, |_, _| {}).unwrap();
        assert_eq!(std::fs::read(&part).unwrap(), full);
    }
}

#[cfg(test)]
mod release_download {
    /// Needs the internet: the real installer, as an update downloads it.
    #[test]
    #[ignore = "needs the internet"]
    fn downloads_a_release_installer() {
        let url =
            "https://github.com/thtbee/nectarlink/releases/download/v0.1.2/Nectarlink-0.1.2-x64-setup.exe";
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("setup.exe.part");
        let started = std::time::Instant::now();
        let mut reports = 0;
        super::download(url, &file, 400 << 20, |_, _| reports += 1).unwrap();
        let len = std::fs::metadata(&file).unwrap().len();
        println!("{len} bytes in {:?}, {reports} progress reports", started.elapsed());
        assert_eq!(len, 38_473_164);
        assert!(reports > 5);
    }
}
