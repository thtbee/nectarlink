// SPDX-License-Identifier: GPL-3.0-or-later
//! HTTPS client for Linux (`win::http` equivalent), used by the auto-updater
//! and metadata checks. Uses `curl` (with `wget` fallback) so system CA
//! certificates and proxy settings are respected without pulling in an extra
//! TLS stack.

use std::{
    fs::{self, File},
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

/// Retry delays for transient failures in [`get_retrying`].
const RETRY_DELAYS: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(3)];

fn user_agent() -> String {
    format!("NectarLink/{}", env!("CARGO_PKG_VERSION"))
}

fn validate_https(url: &str) -> Result<(), String> {
    if !url.get(..8).is_some_and(|p| p.eq_ignore_ascii_case("https://")) {
        return Err("only https:// URLs are fetched".into());
    }
    Ok(())
}

/// Fetches `url` over HTTPS, returning at most `limit` bytes.
pub fn get(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    validate_https(url)?;
    let ua = user_agent();
    let max_size = (limit.saturating_add(1)).to_string();

    if let Ok(output) = Command::new("curl")
        .args(["-fsSL", "--max-time", "60", "--max-filesize", &max_size, "-A", &ua, url])
        .stdin(Stdio::null())
        .output()
    {
        if output.status.success() {
            if output.stdout.len() > limit {
                return Err(format!("response exceeds {limit} bytes"));
            }
            return Ok(output.stdout);
        }
        // Exit code 63 from curl means `--max-filesize` was exceeded.
        if output.status.code() == Some(63) {
            return Err(format!("response exceeds {limit} bytes"));
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if stderr.is_empty() {
            format!("curl failed with status {}", output.status)
        } else {
            stderr
        });
    }

    // Fallback to `wget`.
    let output = Command::new("wget")
        .args(["-qO-", "--timeout=60", &format!("--user-agent={ua}"), url])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("neither curl nor wget is available: {e}"))?;
    if !output.status.success() {
        return Err(format!("wget failed with status {}", output.status));
    }
    if output.stdout.len() > limit {
        return Err(format!("response exceeds {limit} bytes"));
    }
    Ok(output.stdout)
}

/// Fetches `url` with up to 3 attempts on transient errors.
pub fn get_retrying(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    validate_https(url)?;
    let mut delays = RETRY_DELAYS.iter();
    loop {
        match get(url, limit) {
            Ok(bytes) => return Ok(bytes),
            Err(err) => {
                if err.contains("exceeds") {
                    return Err(err);
                }
                match delays.next() {
                    Some(&delay) => {
                        tracing::debug!(url, error = %err, ?delay, "retrying HTTPS request");
                        std::thread::sleep(delay);
                    }
                    None => return Err(err),
                }
            }
        }
    }
}

/// Downloads `url` over HTTPS to `file`, failing if it exceeds `limit` bytes,
/// and reporting `(downloaded_bytes, total_bytes)` to `progress`.
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
    let ua = user_agent();
    let mut child = Command::new("curl")
        .args(["-fsSL", "--max-time", "300", "-A", &ua, url])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("curl: {e}"))?;

    let mut stdout = child.stdout.take().ok_or_else(|| "missing curl stdout".to_owned())?;
    let mut out = File::create(file).map_err(|e| e.to_string())?;
    let mut received = 0u64;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = stdout.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        received += n as u64;
        if received > limit {
            let _ = child.kill();
            let _ = fs::remove_file(file);
            return Err(format!("download exceeds {limit} bytes"));
        }
        out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        progress(received, 0);
    }
    out.flush().map_err(|e| e.to_string())?;
    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        let _ = fs::remove_file(file);
        return Err(format!("download failed with status {status}"));
    }
    progress(received, received);
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
        assert!(get("http://example.com/releases", 1024).is_err());
        assert!(get_retrying("http://example.com/releases", 1024).is_err());
        assert!(download("http://example.com/setup", Path::new("out.bin"), 1024, |_, _| {}).is_err());
    }
}
