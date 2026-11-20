// SPDX-License-Identifier: GPL-3.0-or-later
//! HTTPS GET and download using system curl on macOS.

use std::path::Path;

pub fn get(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    let output = std::process::Command::new("curl")
        .args(["-s", "-L", "-f", "--max-time", "30", url])
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!("curl error: {}", String::from_utf8_lossy(&output.stderr)));
    }
    if output.stdout.len() > limit {
        return Err("too large".into());
    }
    Ok(output.stdout)
}

/// Sends `body` as JSON to an `https://` URL with `headers` (name, value)
/// and returns the status and up to `limit` bytes of the answer.
pub fn post_json(
    url: &str,
    headers: &[(&str, &str)],
    body: &[u8],
    limit: usize,
) -> Result<(u16, Vec<u8>), String> {
    let mut args: Vec<String> = vec!["-X".into(), "POST".into(), "--data-binary".into(), "@-".into()];
    for (name, value) in [("Content-Type", "application/json")].iter().chain(headers) {
        if name.contains([':', '\r', '\n']) || value.contains(['\r', '\n']) {
            return Err("bad header".into());
        }
        args.push("-H".into());
        args.push(format!("{name}: {value}"));
    }
    curl_status(url, &args, body, limit)
}

/// A GET that returns any status with its body, for small JSON answers.
pub fn get_status(url: &str, limit: usize) -> Result<(u16, Vec<u8>), String> {
    curl_status(url, &[], &[], limit)
}

/// Runs curl for `url` with `extra` arguments, `stdin` as its input, and
/// returns the final status and the answer.
fn curl_status(url: &str, extra: &[String], stdin: &[u8], limit: usize) -> Result<(u16, Vec<u8>), String> {
    use std::{
        io::Write as _,
        process::{Command, Stdio},
    };
    if !url.get(..8).is_some_and(|p| p.eq_ignore_ascii_case("https://")) {
        return Err("only https".into());
    }
    let mut child = Command::new("curl")
        .args(["-sS", "--proto", "=https", "--connect-timeout", "30", "--max-time", "60"])
        .args(["-H", "Accept: application/json", "-w", "\n%{http_code}"])
        .args(extra)
        .args(["--", url])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("curl isn't available: {e}"))?;
    if let Some(mut input) = child.stdin.take() {
        input.write_all(stdin).map_err(|e| e.to_string())?;
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!("curl failed with status {}", output.status));
    }
    let out = output.stdout;
    let split = out.iter().rposition(|&b| b == b'\n').ok_or("no status")?;
    let status = std::str::from_utf8(&out[split + 1..])
        .ok()
        .and_then(|s| s.trim().parse::<u16>().ok())
        .ok_or("no status")?;
    let body = out[..split].to_vec();
    if body.len() > limit {
        return Err("too large".into());
    }
    Ok((status, body))
}

/// What a conditional GET found.
#[derive(Debug, PartialEq, Eq)]
pub enum Fresh {
    /// The same as the copy with that ETag. Never on the Mac yet: its
    /// `get_fresh` is a plain fetch while the preview doesn't update itself,
    /// but callers match on the same variants as on the other platforms.
    #[allow(dead_code)]
    NotModified,
    /// New content, and its ETag when the server gave one.
    Changed { body: Vec<u8>, etag: Option<String> },
}

/// A plain fetch for now: the Mac preview doesn't update itself yet.
pub fn get_fresh(url: &str, limit: usize, _etag: Option<&str>) -> Result<Fresh, String> {
    get(url, limit).map(|body| Fresh::Changed { body, etag: None })
}

pub fn get_retrying(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    for _ in 0..3 {
        if let Ok(data) = get(url, limit) {
            return Ok(data);
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    get(url, limit)
}

pub fn download(
    url: &str,
    file: &Path,
    limit: u64,
    mut progress: impl FnMut(u64, u64),
) -> Result<(), String> {
    let have = file.metadata().map(|m| m.len()).unwrap_or(0);
    if have > limit {
        return Err("too large".into());
    }
    let limit_str = limit.to_string();
    let status = std::process::Command::new("curl")
        .args(["-s", "-L", "-f", "-C", "-", "--max-filesize", &limit_str, "-o", &file.to_string_lossy(), url])
        .status()
        .map_err(|e| e.to_string())?;
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    if len > limit {
        let _ = std::fs::remove_file(file);
        return Err("too large".into());
    }
    if !status.success() {
        if status.code() == Some(63) {
            let _ = std::fs::remove_file(file);
            return Err("too large".into());
        }
        return Err("download failed".into());
    }
    progress(len, len);
    Ok(())
}
