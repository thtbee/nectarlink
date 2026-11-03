// SPDX-License-Identifier: GPL-3.0-or-later
//! HTTPS client for Linux (`win::http` equivalent), used by the auto-updater
//! and metadata checks. Uses `curl` (with `wget` fallback) so system CA
//! certificates and proxy settings are respected without pulling in an extra
//! TLS stack.

use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

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

fn user_agent() -> String {
    format!("Nectarlink/{} (Linux)", env!("CARGO_PKG_VERSION"))
}

fn validate_https(url: &str) -> Result<(), String> {
    if !url.get(..8).is_some_and(|p| p.eq_ignore_ascii_case("https://")) {
        return Err("only https".into());
    }
    Ok(())
}

/// Builds the `curl` argument list for fetching `url` with HTTPS-only redirect enforcement.
fn curl_get_args(max_size: &str, ua: &str, url: &str) -> Vec<String> {
    vec![
        "-sSL".into(),
        "-D".into(),
        "/dev/stderr".into(),
        "--proto".into(),
        "=https".into(),
        "--proto-redir".into(),
        "=https".into(),
        "--connect-timeout".into(),
        "30".into(),
        "--max-time".into(),
        "60".into(),
        "--max-filesize".into(),
        max_size.into(),
        "-H".into(),
        "Accept: application/vnd.github+json, application/octet-stream".into(),
        "-A".into(),
        ua.into(),
        "--".into(),
        url.into(),
    ]
}

/// Builds the `wget` argument list for fetching `url` with HTTPS-only redirect enforcement.
fn wget_get_args(ua: &str, url: &str) -> Vec<String> {
    vec![
        "-qSO-".into(),
        "--https-only".into(),
        "--timeout=60".into(),
        "--header=Accept: application/vnd.github+json, application/octet-stream".into(),
        format!("--user-agent={ua}"),
        "--".into(),
        url.into(),
    ]
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct ResponseHeaders {
    status: u32,
    content_length: Option<u64>,
    range_total: Option<u64>,
}

/// Parses an HTTP status line (`HTTP/1.1 200 OK` or `HTTP/2 404`).
fn parse_status_line(line: &str) -> Option<u32> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix("HTTP/")?;
    let (_, after_ver) = rest.split_once(char::is_whitespace)?;
    let code_str = after_ver.split_whitespace().next()?;
    code_str.parse::<u32>().ok()
}

/// The full size from a `Content-Range` header (`bytes 100-199/200`).
fn range_total(header: &str) -> Option<u64> {
    header.rsplit_once('/')?.1.trim().parse().ok().filter(|&n| n > 0)
}

/// Extracts the final HTTP status code from raw header output (`curl -D` or `wget -S`).
fn parse_last_http_status(stderr_text: &str) -> Option<u32> {
    stderr_text.lines().filter_map(parse_status_line).next_back()
}

/// Reads response header blocks from `stderr` until the final non-1xx, non-3xx
/// response header block completes, then returns its parsed headers and drains
/// the remainder of `stderr`.
fn spawn_header_reader(stderr: std::process::ChildStderr) -> mpsc::Receiver<ResponseHeaders> {
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut line = String::new();
        let mut current: Option<ResponseHeaders> = None;
        let mut sent = false;

        loop {
            line.clear();
            let Ok(n) = reader.read_line(&mut line) else { break };
            if n == 0 {
                break;
            }
            if sent {
                continue;
            }
            let trimmed = line.trim();
            if let Some(code) = parse_status_line(trimmed) {
                current = Some(ResponseHeaders { status: code, content_length: None, range_total: None });
                continue;
            }
            if let Some(ref mut hdrs) = current {
                if trimmed.is_empty() {
                    let code = hdrs.status;
                    if !(100..200).contains(&code) && !(300..400).contains(&code) {
                        let _ = tx.send(hdrs.clone());
                        sent = true;
                    }
                    current = None;
                } else if let Some((name, val)) = trimmed.split_once(':') {
                    let name = name.trim();
                    let val = val.trim();
                    if name.eq_ignore_ascii_case("content-length") {
                        hdrs.content_length = val.parse::<u64>().ok();
                    } else if name.eq_ignore_ascii_case("content-range") {
                        hdrs.range_total = range_total(val);
                    }
                }
            }
        }
        if !sent && let Some(hdrs) = current {
            let _ = tx.send(hdrs);
        }
    });
    rx
}

/// Fetches `url` over HTTPS, returning at most `limit` bytes.
pub fn get(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    validate_https(url)?;
    let ua = user_agent();
    let max_size = (limit.saturating_add(1)).to_string();

    if let Ok(mut child) = Command::new("curl")
        .args(curl_get_args(&max_size, &ua, url))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        let mut stdout = child.stdout.take().ok_or_else(|| "missing curl stdout".to_owned())?;
        let mut stderr = child.stderr.take().ok_or_else(|| "missing curl stderr".to_owned())?;
        let stderr_handle = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = stderr.read_to_string(&mut s);
            s
        });
        let mut body = Vec::new();
        let read_res = (&mut stdout).take(limit as u64 + 1).read_to_end(&mut body);
        if body.len() > limit {
            let _ = child.kill();
            let _ = child.wait();
            return Err("too large".into());
        }
        let status = child.wait().map_err(|e| e.to_string())?;
        let stderr_text = stderr_handle.join().unwrap_or_default();
        read_res.map_err(|e| e.to_string())?;

        if status.code() == Some(63) {
            return Err("too large".into());
        }
        if let Some(http_code) = parse_last_http_status(&stderr_text)
            && http_code != 200
        {
            return Err(format!("HTTP {http_code}"));
        }
        if !status.success() {
            let msg = stderr_text.lines().last().unwrap_or("").trim();
            return Err(if msg.is_empty() {
                format!("curl failed with status {status}")
            } else {
                msg.to_owned()
            });
        }
        return Ok(body);
    }

    // Fallback to `wget` with bounded stdout read.
    let mut child = Command::new("wget")
        .args(wget_get_args(&ua, url))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("neither curl nor wget is available: {e}"))?;

    let mut stdout = child.stdout.take().ok_or_else(|| "missing wget stdout".to_owned())?;
    let mut stderr = child.stderr.take().ok_or_else(|| "missing wget stderr".to_owned())?;
    let stderr_handle = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    let mut body = Vec::new();
    let read_res = (&mut stdout).take(limit as u64 + 1).read_to_end(&mut body);
    if body.len() > limit {
        let _ = child.kill();
        let _ = child.wait();
        return Err("too large".into());
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let stderr_text = stderr_handle.join().unwrap_or_default();
    read_res.map_err(|e| e.to_string())?;

    if let Some(http_code) = parse_last_http_status(&stderr_text)
        && http_code != 200
    {
        return Err(format!("HTTP {http_code}"));
    }
    if !status.success() {
        return Err(format!("wget failed with status {status}"));
    }
    Ok(body)
}

/// What a conditional GET found.
#[derive(Debug, PartialEq, Eq)]
pub enum Fresh {
    /// The same as the copy with that ETag.
    NotModified,
    /// New content, and its ETag when the server gave one.
    Changed { body: Vec<u8>, etag: Option<String> },
}

/// The `ETag` of the last response in `curl -D` output.
fn last_etag(headers: &str) -> Option<String> {
    let mut etag = None;
    for line in headers.lines() {
        if parse_status_line(line).is_some() {
            etag = None;
        } else if let Some((name, value)) = line.split_once(':')
            && name.trim().eq_ignore_ascii_case("etag")
        {
            etag = Some(value.trim().to_owned()).filter(|v| !v.is_empty());
        }
    }
    etag
}

/// Fetches `url` unless it's unchanged since `etag`: the server then answers
/// 304, a few hundred bytes with no body. Without `curl` it's a plain fetch.
pub fn get_fresh(url: &str, limit: usize, etag: Option<&str>) -> Result<Fresh, String> {
    validate_https(url)?;
    // An ETag is a short quoted token; anything else isn't sent.
    let etag = etag.filter(|e| e.len() <= 200 && !e.contains(['\r', '\n']));
    let mut args = curl_get_args(&(limit.saturating_add(1)).to_string(), &user_agent(), url);
    if let Some(etag) = etag {
        let at = args.len() - 2;
        args.insert(at, "-H".into());
        args.insert(at + 1, format!("If-None-Match: {etag}"));
    }
    let Ok(output) = Command::new("curl").args(&args).stdin(Stdio::null()).output() else {
        return get(url, limit).map(|body| Fresh::Changed { body, etag: None });
    };
    if output.status.code() == Some(63) || output.stdout.len() > limit {
        return Err("too large".into());
    }
    let headers = String::from_utf8_lossy(&output.stderr);
    match parse_last_http_status(&headers) {
        Some(304) if etag.is_some() => Ok(Fresh::NotModified),
        Some(200) if output.status.success() => {
            Ok(Fresh::Changed { body: output.stdout, etag: last_etag(&headers) })
        }
        Some(code) => Err(format!("HTTP {code}")),
        None => Err(headers.lines().last().unwrap_or("curl failed").trim().to_owned()),
    }
}

/// Network trouble or a server hiccup, worth another try. Not missing
/// files, refusals or a body that's too large.
fn retryable(error: &str) -> bool {
    match error.strip_prefix("HTTP ").and_then(|code| code.parse::<u32>().ok()) {
        Some(code) => code >= 500 || code == 408 || code == 429,
        None => !matches!(error, "too large" | "only https") && !error.contains("exceeds"),
    }
}

/// Fetches `url` retrying transient network errors a few times.
pub fn get_retrying(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    validate_https(url)?;
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
    validate_https(url)?;
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
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
    let ua = user_agent();

    let mut curl_args = vec![
        "-sSL".to_owned(),
        "-D".to_owned(),
        "/dev/stderr".to_owned(),
        "--proto".to_owned(),
        "=https".to_owned(),
        "--proto-redir".to_owned(),
        "=https".to_owned(),
        "--connect-timeout".to_owned(),
        "30".to_owned(),
        "--speed-time".to_owned(),
        "60".to_owned(),
        "--speed-limit".to_owned(),
        "1".to_owned(),
        "-H".to_owned(),
        "Accept: application/vnd.github+json, application/octet-stream".to_owned(),
        "-A".to_owned(),
        ua.clone(),
    ];
    if have > 0 {
        curl_args.push("-H".to_owned());
        curl_args.push(format!("Range: bytes={have}-"));
    }
    curl_args.push("--".to_owned());
    curl_args.push(url.to_owned());

    let mut child = Command::new("curl")
        .args(&curl_args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .or_else(|_| {
            let mut wget_args = vec![
                "-qSO-".to_owned(),
                "--https-only".to_owned(),
                "--timeout=60".to_owned(),
                "--header=Accept: application/vnd.github+json, application/octet-stream".to_owned(),
                format!("--user-agent={ua}"),
            ];
            if have > 0 {
                wget_args.push(format!("--header=Range: bytes={have}-"));
            }
            wget_args.push("--".to_owned());
            wget_args.push(url.to_owned());
            Command::new("wget")
                .args(&wget_args)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
        })
        .map_err(|e| format!("neither curl nor wget is available: {e}"))?;

    let mut stdout = child.stdout.take().ok_or_else(|| "missing downloader stdout".to_owned())?;
    let stderr = child.stderr.take().ok_or_else(|| "missing downloader stderr".to_owned())?;
    let hdr_rx = spawn_header_reader(stderr);

    let hdrs = match hdr_rx.recv_timeout(Duration::from_secs(60)) {
        Ok(h) => h,
        Err(_) => {
            let _ = child.kill();
            let status = child.wait().map_err(|e| e.to_string())?;
            return Err(format!("download failed before response headers ({status})"));
        }
    };

    let (append, total) = match hdrs.status {
        206 => (true, hdrs.range_total.or_else(|| hdrs.content_length.map(|l| have + l)).unwrap_or(0)),
        200 => (false, hdrs.content_length.unwrap_or(0)),
        416 if have > 0 => {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(());
        }
        status => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("HTTP {status}"));
        }
    };

    if total > limit {
        let _ = child.kill();
        let _ = child.wait();
        return Err("too large".into());
    }

    let mut out = {
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .custom_flags(libc::O_NOFOLLOW)
            .mode(0o600)
            .open(file)
            .map_err(|e| format!("write: {e}"))?
    };

    let mut done = if append { have } else { 0 };
    progress(done, total);
    let mut buf = vec![0u8; 256 * 1024];
    let mut reported = Instant::now();
    loop {
        let n = stdout.read(&mut buf).map_err(|e| format!("read: {e}"))?;
        if n == 0 {
            break;
        }
        done += n as u64;
        if done > limit {
            let _ = child.kill();
            let _ = child.wait();
            let _ = fs::remove_file(file);
            return Err("too large".into());
        }
        out.write_all(&buf[..n]).map_err(|e| format!("write: {e}"))?;
        if reported.elapsed() >= Duration::from_millis(200) {
            reported = Instant::now();
            progress(done, total);
        }
    }
    out.sync_all().map_err(|e| format!("write: {e}"))?;
    let status = child.wait().map_err(|e| e.to_string())?;
    progress(done, total);
    if total > 0 && done < total {
        return Err("the download ended early".into());
    }
    if !status.success() {
        return Err(format!("download failed with status {status}"));
    }
    Ok(())
}

/// Fetches a JSON document as a UTF-8 `String` (up to 2 MiB).
pub fn get_json(url: &str) -> Result<String, String> {
    let bytes = get_retrying(url, 2 * 1024 * 1024)?;
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

/// Downloads `url` to `dest` (up to 512 MiB).
pub fn download_to_file(url: &str, dest: &Path, on_progress: impl FnMut(u64, u64)) -> Result<(), String> {
    download(url, dest, 512 * 1024 * 1024, on_progress)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_plain_http_urls() {
        assert_eq!(get("http://example.com/releases", 1024).unwrap_err(), "only https");
        assert_eq!(get_retrying("http://example.com/releases", 1024).unwrap_err(), "only https");
        assert_eq!(
            download("http://example.com/setup", Path::new("out.bin"), 1024, |_, _| {}).unwrap_err(),
            "only https"
        );
    }

    #[test]
    fn reads_the_etag_of_the_final_response() {
        let headers = "HTTP/2 302\r\netag: \"redirect\"\r\n\r\nHTTP/2 200\r\nETag: W/\"abc\"\r\n\r\n";
        assert_eq!(last_etag(headers), Some("W/\"abc\"".into()));
        assert_eq!(last_etag("HTTP/2 302\r\netag: \"x\"\r\n\r\nHTTP/2 200\r\n\r\n"), None);
        assert_eq!(get_fresh("http://example.com/feed", 1024, None).unwrap_err(), "only https");
    }

    #[test]
    fn reads_the_full_size_from_a_content_range() {
        assert_eq!(range_total("bytes 1048576-38473163/38473164"), Some(38_473_164));
        assert_eq!(range_total("bytes 0-99/*"), None);
        assert_eq!(range_total("garbage"), None);
    }

    #[test]
    fn parses_redirected_http_status_lines() {
        let raw =
            "HTTP/2 302 \r\nlocation: https://objects.githubusercontent.com/x\r\n\r\nHTTP/2 404 \r\n\r\n";
        assert_eq!(parse_last_http_status(raw), Some(404));
        assert_eq!(parse_status_line("HTTP/1.1 206 Partial Content"), Some(206));
    }

    #[test]
    fn retries_network_trouble_but_not_refusals() {
        assert!(retryable("curl failed with status exit status: 28"));
        assert!(retryable("HTTP 503"));
        assert!(retryable("HTTP 429"));
        assert!(!retryable("HTTP 404"));
        assert!(!retryable("too large"));
        assert!(!retryable("only https"));
    }

    #[test]
    fn curl_and_wget_args_enforce_https_redirects_and_end_of_options() {
        let curl_args = curl_get_args("1025", "NectarLink/0.1", "https://example.com/feed.json");
        assert!(curl_args.windows(2).any(|w| w[0] == "--proto" && w[1] == "=https"));
        assert!(curl_args.windows(2).any(|w| w[0] == "--proto-redir" && w[1] == "=https"));
        assert_eq!(&curl_args[curl_args.len() - 2..], ["--", "https://example.com/feed.json"]);

        let wget_args = wget_get_args("NectarLink/0.1", "https://example.com/feed.json");
        assert!(wget_args.iter().any(|a| a == "--https-only"));
        assert_eq!(&wget_args[wget_args.len() - 2..], ["--", "https://example.com/feed.json"]);
    }
}
