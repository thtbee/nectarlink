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

/// What a conditional GET found.
#[derive(Debug, PartialEq, Eq)]
pub enum Fresh {
    /// The same as the copy with that ETag.
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
