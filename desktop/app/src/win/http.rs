// SPDX-License-Identifier: GPL-3.0-or-later
//! HTTPS GET with Windows' own HTTP stack (WinHTTP): the system's TLS,
//! proxy settings and certificates, and no extra dependencies. Used to
//! check for updates and download them.

use windows::{
    Win32::Networking::WinHttp::{
        WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER,
        WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest,
        WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts,
    },
    core::{HSTRING, PCWSTR, w},
};

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

/// Fetches an `https://` URL (following redirects), up to `limit` bytes.
pub fn get(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    let rest = url.strip_prefix("https://").ok_or("only https")?;
    let (host, path) = rest.split_once('/').map_or((rest, "/".to_owned()), |(h, p)| (h, format!("/{p}")));
    let agent = HSTRING::from(format!("Nectarlink/{} (Windows)", env!("CARGO_PKG_VERSION")));
    // SAFETY: WinHTTP calls with valid strings; every handle is closed by
    // `Handle`, children before parents (declaration order is reversed on
    // drop).
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
        // Resolve, connect, send, receive (ms).
        let _ = WinHttpSetTimeouts(session.0, 10_000, 10_000, 30_000, 60_000);
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
        // GitHub's API wants this; it's harmless elsewhere.
        let headers: Vec<u16> =
            "Accept: application/vnd.github+json, application/octet-stream\r\n".encode_utf16().collect();
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
        if status != 200 {
            return Err(format!("HTTP {status}"));
        }
        let mut body = Vec::new();
        let mut chunk = vec![0u8; 64 * 1024];
        loop {
            let mut read = 0u32;
            WinHttpReadData(request.0, chunk.as_mut_ptr().cast(), chunk.len() as u32, &mut read)
                .map_err(|e| format!("read: {e}"))?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..read as usize]);
            if body.len() > limit {
                return Err("too large".into());
            }
        }
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs the internet: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn fetches_over_https() {
        let body = get("https://api.github.com/repos/rust-lang/rust", 1024 * 1024).unwrap();
        assert!(String::from_utf8_lossy(&body).contains("\"full_name\""));
        assert_eq!(get("https://api.github.com/does-not-exist-nectarlink", 1024).unwrap_err(), "HTTP 404");
        assert_eq!(get("http://example.com", 1024).unwrap_err(), "only https");
    }
}
