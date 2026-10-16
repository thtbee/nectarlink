// SPDX-License-Identifier: GPL-3.0-or-later
//! The Linux clipboard (`wl-copy`/`wl-paste` on Wayland, `xclip`/`xsel` on X11,
//! with an in-memory fallback for headless test environments): watching for
//! copied text and images, reading them, and writing what a phone sent
//! (docs/protocol/clipboard.md).
//!
//! Content that password managers mark as private (`x-kde-passwordManagerHint`
//! set to `"secret"`, or `x-gnome-privacy-hint`) is never read for sending.
//! Writes from NectarLink record their [`Fingerprint`] in [`LAST_REPORTED`]
//! before updating the system clipboard so the watcher thread never echoes
//! phone clipboard writes back to the phone.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use super::image::{self, Bitmap};

/// Poll interval for the background Linux clipboard watcher thread.
const POLL_INTERVAL: Duration = Duration::from_millis(450);

/// Content fingerprint used to deduplicate clipboard updates and suppress
/// echoing NectarLink's own writes back to connected phones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fingerprint {
    Text(u64),
    Image(u64),
}

type OnCopy = Box<dyn Fn(Clip) + Send + Sync>;
type WrittenRich = (String, Option<String>, Option<String>);

static ON_COPY: OnceLock<OnCopy> = OnceLock::new();

/// Fingerprint of the most recently reported or locally-written clipboard
/// item so the watcher thread never echoes our own writes back to the phone.
static LAST_REPORTED: Mutex<Option<Fingerprint>> = Mutex::new(None);

/// Most recent rich-text payload (`(plain_text, html, rtf)`) written by
/// NectarLink so rich formats survive round-trips through plain-text bridges.
static LAST_WRITTEN_RICH: Mutex<Option<WrittenRich>> = Mutex::new(None);

/// In-memory clipboard fallback for headless environments without a live
/// Wayland/X11 clipboard server.
static LAST_CLIP: Mutex<Clip> = Mutex::new(Clip::Empty);

/// In-memory file-list fallback for headless environments.
static LAST_FILES: Mutex<Option<Vec<PathBuf>>> = Mutex::new(None);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Image {
    Png(Vec<u8>),
    Bitmap(Vec<u8>),
}

impl Image {
    pub fn to_png(&self) -> Result<Vec<u8>, String> {
        match self {
            Image::Png(bytes) => Ok(bytes.clone()),
            Image::Bitmap(dib) => {
                let bmp = image::bmp_from_dib(dib).ok_or_else(|| "invalid DIB header".to_string())?;
                let decoded = image::decode(&bmp).map_err(|e| e.to_string())?;
                image::encode_png(&decoded).map_err(|e| e.to_string())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Clip {
    Text { text: String, html: Option<String>, rtf: Option<String> },
    Image(Image),
    Private,
    Empty,
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

pub(crate) fn fingerprint(clip: &Clip) -> Option<Fingerprint> {
    match clip {
        Clip::Text { text, .. } => {
            let normalized = text.replace("\r\n", "\n");
            let stripped = normalized.trim_end_matches('\n');
            if stripped.trim().is_empty() { None } else { Some(Fingerprint::Text(fnv(stripped.as_bytes()))) }
        }
        Clip::Image(Image::Png(bytes) | Image::Bitmap(bytes)) => {
            if let Ok(bitmap) = image::decode(bytes) {
                Some(Fingerprint::Image(fnv(&bitmap.bgra)))
            } else if !bytes.is_empty() {
                Some(Fingerprint::Image(fnv(bytes)))
            } else {
                None
            }
        }
        Clip::Private | Clip::Empty => None,
    }
}

fn has_display_server() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some()
}

fn is_wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// Checks whether a newline-separated list of clipboard MIME targets contains
/// a password-manager / privacy exclusion hint (`x-kde-passwordManagerHint` or
/// `x-gnome-privacy-hint`).
pub(crate) fn is_secret_mime_list(targets: &str) -> bool {
    targets.lines().any(|line| {
        let t = line.trim();
        t.eq_ignore_ascii_case("x-gnome-privacy-hint")
            || t.eq_ignore_ascii_case("x-kde-passwordManagerHint")
            || t.eq_ignore_ascii_case("ExcludeClipboardContentFromMonitorProcessing")
            || t.eq_ignore_ascii_case("Clipboard Viewer Ignore")
    })
}

fn has_gnome_privacy_hint(targets: &str) -> bool {
    targets.lines().any(|line| {
        let t = line.trim();
        t.eq_ignore_ascii_case("x-gnome-privacy-hint")
            || t.eq_ignore_ascii_case("ExcludeClipboardContentFromMonitorProcessing")
            || t.eq_ignore_ascii_case("Clipboard Viewer Ignore")
    })
}

fn has_kde_password_hint(targets: &str) -> bool {
    targets.lines().any(|line| line.trim().eq_ignore_ascii_case("x-kde-passwordManagerHint"))
}

fn has_mime_target(targets: &str, wanted: &str) -> bool {
    targets.lines().any(|line| {
        let t = line.trim();
        t.eq_ignore_ascii_case(wanted)
            || t.split(';').next().is_some_and(|base| base.trim().eq_ignore_ascii_case(wanted))
    })
}

pub(crate) fn is_kde_secret_value(bytes: &[u8]) -> bool {
    let trimmed = match bytes.iter().position(|&b| b == 0) {
        Some(end) => &bytes[..end],
        None => bytes,
    };
    std::str::from_utf8(trimmed).ok().is_some_and(|s| s.trim().eq_ignore_ascii_case("secret"))
}

/// Starts watching the Linux clipboard on a background thread (`"clipboard"`).
/// Newly copied text or images that are not marked private and were not
/// written by NectarLink are passed to `on_copy`.
pub fn start<F>(on_copy: F)
where
    F: Fn(Clip) + Send + Sync + 'static,
{
    if ON_COPY.set(Box::new(on_copy)).is_err() {
        return;
    }
    if !has_display_server() {
        return;
    }
    let spawned = std::thread::Builder::new().name("clipboard".into()).spawn(watcher_loop);
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "can't start Linux clipboard watcher");
    }
}

fn watcher_loop() {
    loop {
        std::thread::sleep(POLL_INTERVAL);
        let clip = read();
        if let Some(on_copy) = ON_COPY.get() {
            let _ = report_if_changed(clip, on_copy);
        }
    }
}

/// Evaluates `clip` against [`LAST_REPORTED`] and [`LAST_WRITTEN_RICH`],
/// invoking ` callback ` only when `clip` represents new non-private content.
pub(crate) fn report_if_changed<F>(mut clip: Clip, callback: F) -> bool
where
    F: FnOnce(Clip),
{
    if matches!(clip, Clip::Private | Clip::Empty) {
        return false;
    }
    if let Clip::Text { ref text, ref mut html, ref mut rtf } = clip
        && html.is_none()
        && rtf.is_none()
        && let Some((prev_text, saved_html, saved_rtf)) =
            LAST_WRITTEN_RICH.lock().unwrap_or_else(|e| e.into_inner()).clone()
        && prev_text == *text
    {
        *html = saved_html;
        *rtf = saved_rtf;
    }
    let Some(fp) = fingerprint(&clip) else {
        return false;
    };
    {
        let mut last = LAST_REPORTED.lock().unwrap_or_else(|e| e.into_inner());
        if *last == Some(fp) {
            return false;
        }
        *last = Some(fp);
    }
    callback(clip);
    true
}

fn query_os_targets() -> Option<(bool, String)> {
    if is_wayland()
        && let Ok(out) = Command::new("wl-paste").arg("--list-types").output()
        && out.status.success()
    {
        return Some((true, String::from_utf8_lossy(&out.stdout).into_owned()));
    }
    if let Ok(out) = Command::new("xclip").args(["-selection", "clipboard", "-t", "TARGETS", "-o"]).output()
        && out.status.success()
    {
        return Some((false, String::from_utf8_lossy(&out.stdout).into_owned()));
    }
    None
}

fn read_os_mime(wayland: bool, mime: &str) -> Option<Vec<u8>> {
    if wayland {
        let out = Command::new("wl-paste").args(["--no-newline", "--type", mime]).output().ok()?;
        out.status.success().then_some(out.stdout)
    } else {
        let out = Command::new("xclip").args(["-selection", "clipboard", "-t", mime, "-o"]).output().ok()?;
        out.status.success().then_some(out.stdout)
    }
}

fn clean_valid_utf8_text(bytes: Vec<u8>) -> Option<String> {
    if bytes.is_empty()
        || bytes.starts_with(b"\xFF\xD8\xFF")
        || bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes.starts_with(b"GIF87a")
        || bytes.starts_with(b"GIF89a")
        || bytes.starts_with(b"RIFF")
        || bytes.starts_with(b"BM")
    {
        return None;
    }
    let text = String::from_utf8(bytes).ok()?;
    let trimmed = text.trim_matches('\0');
    if trimmed.trim().is_empty() || nectarlink_core::is_likely_binary_or_corrupted_image_text(trimmed) {
        return None;
    }
    Some(trimmed.to_owned())
}

fn read_os_plain_text(wayland: bool) -> Option<String> {
    if wayland {
        for mime in ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"] {
            if let Ok(out) = Command::new("wl-paste").args(["--no-newline", "--type", mime]).output()
                && out.status.success()
                && let Some(text) = clean_valid_utf8_text(out.stdout)
            {
                return Some(text);
            }
        }
    } else {
        for mime in ["UTF8_STRING", "text/plain;charset=utf-8", "text/plain", "STRING"] {
            if let Ok(out) =
                Command::new("xclip").args(["-selection", "clipboard", "-t", mime, "-o"]).output()
                && out.status.success()
                && let Some(text) = clean_valid_utf8_text(out.stdout)
            {
                return Some(text);
            }
        }
        if let Ok(out) = Command::new("xsel").args(["--clipboard", "--output"]).output()
            && out.status.success()
            && let Some(text) = clean_valid_utf8_text(out.stdout)
        {
            return Some(text);
        }
    }
    None
}

fn enrich_with_last_written_rich(text: String, mut html: Option<String>, mut rtf: Option<String>) -> Clip {
    if html.is_none()
        && rtf.is_none()
        && let Some((prev_text, saved_html, saved_rtf)) =
            LAST_WRITTEN_RICH.lock().unwrap_or_else(|e| e.into_inner()).clone()
        && prev_text == text
    {
        html = saved_html;
        rtf = saved_rtf;
    }
    Clip::Text { text, html, rtf }
}

pub(crate) fn has_copied_files_mime(targets: &str) -> bool {
    has_mime_target(targets, "text/uri-list")
        || has_mime_target(targets, "x-special/gnome-copied-files")
        || has_mime_target(targets, "x-special/mate-copied-files")
        || has_mime_target(targets, "application/x-kde-cutselection")
}

pub(crate) fn is_copied_file_uri_text(text: &str) -> bool {
    let mut found_file_uri = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || matches!(trimmed, "copy" | "cut") {
            continue;
        }
        if from_file_uri(trimmed).is_some() {
            found_file_uri = true;
        } else {
            return false;
        }
    }
    found_file_uri
}

fn read_from_os(check_privacy: bool) -> Option<Clip> {
    if cfg!(test) || !has_display_server() {
        return None;
    }
    if let Some((wayland, targets)) = query_os_targets() {
        if check_privacy {
            if has_gnome_privacy_hint(&targets) {
                return Some(Clip::Private);
            }
            if has_kde_password_hint(&targets) {
                match read_os_mime(wayland, "x-kde-passwordManagerHint") {
                    Some(val) if is_kde_secret_value(&val) => return Some(Clip::Private),
                    None => return Some(Clip::Private),
                    _ => {}
                }
            }
        }

        if has_mime_target(&targets, "image/png")
            && let Some(bytes) = read_os_mime(wayland, "image/png")
            && !bytes.is_empty()
        {
            return Some(Clip::Image(Image::Png(bytes)));
        }
        for img_mime in
            ["image/jpeg", "image/jpg", "image/webp", "image/bmp", "image/x-bmp", "image/x-MS-bmp"]
        {
            if has_mime_target(&targets, img_mime)
                && let Some(bytes) = read_os_mime(wayland, img_mime)
                && let Ok(bitmap) = image::decode(&bytes)
                && let Ok(png) = image::encode_png(&bitmap)
            {
                return Some(Clip::Image(Image::Png(png)));
            }
        }

        if has_copied_files_mime(&targets) {
            return Some(Clip::Empty);
        }

        if let Some(text) = read_os_plain_text(wayland) {
            if is_copied_file_uri_text(&text) {
                return Some(Clip::Empty);
            }
            let html = if has_mime_target(&targets, "text/html") {
                read_os_mime(wayland, "text/html").and_then(|b| {
                    decode_cf_html(&b).or_else(|| {
                        let s = String::from_utf8(b).ok()?;
                        let trimmed = s.trim_matches('\0').trim();
                        (!trimmed.is_empty()).then(|| trimmed.to_owned())
                    })
                })
            } else {
                None
            };
            let rtf = if has_mime_target(&targets, "text/rtf") {
                read_os_mime(wayland, "text/rtf").and_then(|b| decode_cf_rtf(&b))
            } else if has_mime_target(&targets, "application/rtf") {
                read_os_mime(wayland, "application/rtf").and_then(|b| decode_cf_rtf(&b))
            } else {
                None
            };
            return Some(enrich_with_last_written_rich(text, html, rtf));
        }
    }

    if let Some(text) = read_os_plain_text(is_wayland()) {
        if is_copied_file_uri_text(&text) {
            return Some(Clip::Empty);
        }
        return Some(enrich_with_last_written_rich(text, None, None));
    }
    None
}

/// Reads the current clipboard contents.
pub fn read() -> Clip {
    if let Some(os_clip) = read_from_os(true) {
        // If we wrote a private clip whose fingerprint matches what's still on
        // the OS clipboard, keep treating it as `Clip::Private`.
        if let Some(fp) = fingerprint(&os_clip)
            && *LAST_REPORTED.lock().unwrap_or_else(|e| e.into_inner()) == Some(fp)
            && matches!(*LAST_CLIP.lock().unwrap_or_else(|e| e.into_inner()), Clip::Private)
        {
            return Clip::Private;
        }
        return os_clip;
    }

    let fallback = LAST_CLIP.lock().unwrap_or_else(|e| e.into_inner()).clone();
    match fallback {
        Clip::Text { text, html, rtf } => enrich_with_last_written_rich(text, html, rtf),
        other => other,
    }
}

/// Like [`read`], but for an explicit paste (Ctrl+V in a mirror window),
/// where the user asked to send whatever is on the clipboard right now,
/// including items marked excluded from automatic clipboard monitoring.
pub fn read_for_paste() -> Clip {
    if let Some(os_clip) = read_from_os(false) {
        return os_clip;
    }
    read()
}

pub fn read_image_png() -> Option<Vec<u8>> {
    match read() {
        Clip::Image(img) => img.to_png().ok(),
        _ => None,
    }
}

pub fn write(text: &str) -> Result<(), String> {
    write_text(text, None, None, false)
}

pub fn write_rich(text: &str, html: Option<&str>, rtf: Option<&str>) -> Result<(), String> {
    write_text(text, html, rtf, false)
}

pub fn write_private(text: &str) -> Result<(), String> {
    write_text(text, None, None, true)
}

fn write_text(text: &str, html: Option<&str>, rtf: Option<&str>, private: bool) -> Result<(), String> {
    let mut html_owned = html.filter(|s| !s.trim().is_empty()).map(str::to_owned);
    let mut rtf_owned = rtf.filter(|s| !s.trim().is_empty()).map(str::to_owned);

    if !private
        && html_owned.is_none()
        && rtf_owned.is_none()
        && let Some((prev_text, prev_h, prev_r)) =
            LAST_WRITTEN_RICH.lock().unwrap_or_else(|e| e.into_inner()).as_ref()
        && prev_text == text
    {
        html_owned = prev_h.clone();
        rtf_owned = prev_r.clone();
    }

    let text_clip = Clip::Text { text: text.to_owned(), html: html_owned.clone(), rtf: rtf_owned.clone() };

    // Update LAST_REPORTED and LAST_WRITTEN_RICH BEFORE touching the OS
    // clipboard so the background watcher never echoes our own write back.
    *LAST_REPORTED.lock().unwrap_or_else(|e| e.into_inner()) = fingerprint(&text_clip);

    if private {
        *LAST_WRITTEN_RICH.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *LAST_CLIP.lock().unwrap_or_else(|e| e.into_inner()) = Clip::Private;
    } else {
        *LAST_WRITTEN_RICH.lock().unwrap_or_else(|e| e.into_inner()) =
            if html_owned.is_some() || rtf_owned.is_some() {
                Some((text.to_owned(), html_owned.clone(), rtf_owned.clone()))
            } else {
                None
            };
        *LAST_CLIP.lock().unwrap_or_else(|e| e.into_inner()) = text_clip;
    }

    if !cfg!(test) {
        crate::mirror::note_remote_clipboard(text);
    }

    if !cfg!(test) && has_display_server() {
        if private {
            let _ = pipe_to_cmd("wl-copy", &["--sensitive"], text.as_bytes())
                .or_else(|_| pipe_to_cmd("wl-copy", &[], text.as_bytes()))
                .or_else(|_| pipe_to_cmd("xclip", &["-selection", "clipboard"], text.as_bytes()))
                .or_else(|_| pipe_to_cmd("xsel", &["--clipboard", "--input"], text.as_bytes()));
        } else {
            let _ = pipe_to_cmd("wl-copy", &[], text.as_bytes())
                .or_else(|_| pipe_to_cmd("xclip", &["-selection", "clipboard"], text.as_bytes()))
                .or_else(|_| pipe_to_cmd("xsel", &["--clipboard", "--input"], text.as_bytes()));
        }
    }
    Ok(())
}

pub fn write_image(mime: &str, bytes: &[u8]) -> Result<(), String> {
    let bitmap: Bitmap = image::decode(bytes).map_err(|e| e.to_string())?;
    let png = if mime == "image/png" && bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        bytes.to_vec()
    } else {
        image::encode_png(&bitmap).map_err(|e| e.to_string())?
    };

    // Record fingerprint BEFORE updating the system clipboard.
    *LAST_REPORTED.lock().unwrap_or_else(|e| e.into_inner()) = Some(Fingerprint::Image(fnv(&bitmap.bgra)));
    *LAST_WRITTEN_RICH.lock().unwrap_or_else(|e| e.into_inner()) = None;
    *LAST_CLIP.lock().unwrap_or_else(|e| e.into_inner()) = Clip::Image(Image::Png(png.clone()));

    if !cfg!(test) && has_display_server() {
        let _ = pipe_to_cmd("wl-copy", &["--type", "image/png"], &png)
            .or_else(|_| pipe_to_cmd("xclip", &["-selection", "clipboard", "-t", "image/png"], &png));
    }
    Ok(())
}

fn pipe_to_cmd(program: &str, args: &[&str], bytes: &[u8]) -> std::io::Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(bytes)?;
    }
    let status = child.wait()?;
    if status.success() { Ok(()) } else { Err(std::io::Error::other("clipboard helper exited with error")) }
}

/// Converts a filesystem path into a percent-encoded `file:///...` URI suitable
/// for `text/uri-list`.
pub(crate) fn to_file_uri(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let normalized = raw.replace('\\', "/");
    let mut encoded = String::with_capacity(normalized.len() + 16);
    if !normalized.starts_with('/') {
        encoded.push('/');
    }
    for &b in normalized.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'/' | b':') {
            encoded.push(b as char);
        } else {
            encoded.push_str(&format!("%{b:02X}"));
        }
    }
    format!("file://{encoded}")
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Parses a `file://` URI from a `text/uri-list` line and percent-decodes it
/// into a [`PathBuf`].
pub(crate) fn from_file_uri(uri: &str) -> Option<PathBuf> {
    let trimmed = uri.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let rest = if let Some(after) = trimmed.strip_prefix("file://localhost/") {
        format!("/{after}")
    } else {
        let after = trimmed.strip_prefix("file://")?;
        if after.is_empty() {
            return None;
        }
        after.to_owned()
    };

    let bytes = rest.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2]))
        {
            decoded.push((hi << 4) | lo);
            i += 3;
            continue;
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    let path_str = String::from_utf8(decoded).ok()?;
    (!path_str.is_empty()).then(|| PathBuf::from(path_str))
}

pub(crate) fn format_uri_list(paths: &[PathBuf]) -> String {
    let mut out = String::new();
    for path in paths {
        out.push_str(&to_file_uri(path));
        out.push_str("\r\n");
    }
    out
}

pub(crate) fn parse_uri_list(uri_list: &str) -> Vec<PathBuf> {
    uri_list.lines().filter_map(from_file_uri).collect()
}

pub fn build_dropfiles_payload(paths: &[PathBuf]) -> Vec<u8> {
    const DROPFILES_HEADER_SIZE: u32 = 20;
    let mut utf16_units: Vec<u16> = Vec::new();
    for path in paths {
        let s = path.to_string_lossy();
        utf16_units.extend(s.encode_utf16());
        utf16_units.push(0);
    }
    utf16_units.push(0);

    let mut out = Vec::with_capacity(DROPFILES_HEADER_SIZE as usize + utf16_units.len() * 2);
    out.extend_from_slice(&DROPFILES_HEADER_SIZE.to_le_bytes());
    out.extend_from_slice(&0i32.to_le_bytes());
    out.extend_from_slice(&0i32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    for unit in utf16_units {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    out
}

pub fn read_files() -> Option<Vec<PathBuf>> {
    if !cfg!(test) && has_display_server() {
        let wayland = is_wayland();
        for mime in ["text/uri-list", "x-special/gnome-copied-files", "x-special/mate-copied-files"] {
            if let Some(bytes) = read_os_mime(wayland, mime)
                && let Ok(text) = String::from_utf8(bytes)
            {
                let parsed: Vec<PathBuf> = parse_uri_list(&text).into_iter().filter(|p| p.exists()).collect();
                if !parsed.is_empty() {
                    return Some(parsed);
                }
            }
        }
        if let Some(text) = read_os_plain_text(wayland)
            && is_copied_file_uri_text(&text)
        {
            let parsed: Vec<PathBuf> = parse_uri_list(&text).into_iter().filter(|p| p.exists()).collect();
            if !parsed.is_empty() {
                return Some(parsed);
            }
        }
    }
    LAST_FILES.lock().unwrap_or_else(|e| e.into_inner()).clone().filter(|v| !v.is_empty())
}

pub fn write_files(paths: &[PathBuf]) -> bool {
    if paths.is_empty() {
        return false;
    }
    let uri_list = format_uri_list(paths);
    let trimmed = uri_list.trim_end_matches(&['\r', '\n'][..]);
    *LAST_REPORTED.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(Fingerprint::Text(fnv(trimmed.as_bytes())));
    *LAST_WRITTEN_RICH.lock().unwrap_or_else(|e| e.into_inner()) = None;
    *LAST_CLIP.lock().unwrap_or_else(|e| e.into_inner()) = Clip::Empty;
    *LAST_FILES.lock().unwrap_or_else(|e| e.into_inner()) = Some(paths.to_vec());
    if !cfg!(test) && has_display_server() {
        let _ = pipe_to_cmd("wl-copy", &["--type", "text/uri-list"], uri_list.as_bytes()).or_else(|_| {
            pipe_to_cmd("xclip", &["-selection", "clipboard", "-t", "text/uri-list"], uri_list.as_bytes())
        });
    }
    true
}

pub(crate) fn encode_cf_html(fragment: &str) -> Vec<u8> {
    const PREFIX: &str = "<!DOCTYPE html><html><body><!--StartFragment-->";
    const SUFFIX: &str = "<!--EndFragment--></body></html>";
    const HEADER_TEMPLATE: &str = concat!(
        "Version:0.9\r\n",
        "StartHTML:0000000000\r\n",
        "EndHTML:0000000000\r\n",
        "StartFragment:0000000000\r\n",
        "EndFragment:0000000000\r\n",
    );

    let start_html = HEADER_TEMPLATE.len();
    let start_fragment = start_html + PREFIX.len();
    let end_fragment = start_fragment + fragment.len();
    let end_html = end_fragment + SUFFIX.len();

    let header = format!(
        "Version:0.9\r\nStartHTML:{start_html:010}\r\nEndHTML:{end_html:010}\r\nStartFragment:{start_fragment:010}\r\nEndFragment:{end_fragment:010}\r\n"
    );
    debug_assert_eq!(header.len(), start_html);

    let mut out = Vec::with_capacity(end_html + 1);
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(PREFIX.as_bytes());
    out.extend_from_slice(fragment.as_bytes());
    out.extend_from_slice(SUFFIX.as_bytes());
    out.push(0);
    out
}

pub(crate) fn decode_cf_html(bytes: &[u8]) -> Option<String> {
    let trimmed = match bytes.iter().position(|&b| b == 0) {
        Some(end) => &bytes[..end],
        None => bytes,
    };
    if trimmed.is_empty() {
        return None;
    }
    let text = std::str::from_utf8(trimmed).ok()?;

    let start_frag = parse_cf_html_offset(text, "StartFragment:");
    let end_frag = parse_cf_html_offset(text, "EndFragment:");
    if let (Some(s), Some(e)) = (start_frag, end_frag)
        && s <= e
        && e <= trimmed.len()
        && let Ok(slice) = std::str::from_utf8(&trimmed[s..e])
    {
        let cleaned = slice.trim();
        if !cleaned.is_empty() {
            return Some(cleaned.to_string());
        }
    }

    const START_MARK: &str = "<!--StartFragment-->";
    const END_MARK: &str = "<!--EndFragment-->";
    if let Some(s_idx) = text.find(START_MARK) {
        let after = &text[s_idx + START_MARK.len()..];
        if let Some(e_idx) = after.find(END_MARK) {
            let cleaned = after[..e_idx].trim();
            if !cleaned.is_empty() {
                return Some(cleaned.to_string());
            }
        }
    }
    None
}

fn parse_cf_html_offset(text: &str, key: &str) -> Option<usize> {
    for line in text.lines().take(12) {
        if let Some(rest) = line.trim().strip_prefix(key) {
            return rest.trim().parse::<usize>().ok();
        }
    }
    None
}

pub(crate) fn encode_cf_rtf(rtf: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(rtf.len() + 1);
    out.extend_from_slice(rtf.as_bytes());
    out.push(0);
    out
}

pub(crate) fn decode_cf_rtf(bytes: &[u8]) -> Option<String> {
    let trimmed = match bytes.iter().position(|&b| b == 0) {
        Some(end) => &bytes[..end],
        None => bytes,
    };
    let s = std::str::from_utf8(trimmed).ok()?.trim();
    if s.starts_with("{\\rtf") { Some(s.to_string()) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn cf_html_round_trips_utf8_fragment() {
        let fragment = "<p>Hello <b>world</b> — 🐝</p>";
        let encoded = encode_cf_html(fragment);
        assert_eq!(encoded.last(), Some(&0));
        let decoded = decode_cf_html(&encoded).expect("decoded fragment");
        assert_eq!(decoded, fragment);
    }

    #[test]
    fn cf_html_falls_back_to_markers_when_offsets_invalid() {
        let raw = b"Version:0.9\r\nStartFragment:99999\r\nEndFragment:99999\r\n<html><body><!--StartFragment--><em>fallback</em><!--EndFragment--></body></html>\0";
        assert_eq!(decode_cf_html(raw).as_deref(), Some("<em>fallback</em>"));
    }

    #[test]
    fn cf_rtf_round_trips_and_validates_prefix() {
        let rtf = "{\\rtf1\\ansi\\b Hello\\b0}";
        let encoded = encode_cf_rtf(rtf);
        assert_eq!(decode_cf_rtf(&encoded).as_deref(), Some(rtf));
        assert!(decode_cf_rtf(b"not rtf\0").is_none());
    }

    #[test]
    fn dropfiles_payload_has_valid_header_and_double_null_terminator() {
        let paths = vec![PathBuf::from(r"C:\Temp\a.txt")];
        let bytes = build_dropfiles_payload(&paths);
        assert_eq!(u32::from_le_bytes(bytes[0..4].try_into().unwrap()), 20);
        assert_eq!(u32::from_le_bytes(bytes[16..20].try_into().unwrap()), 1);
        assert_eq!(&bytes[bytes.len() - 4..], &[0, 0, 0, 0]);
    }

    #[test]
    fn text_and_rich_text_round_trip_and_preserve_last_written_rich() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        write("plain clip").expect("write plain");
        assert_eq!(read(), Clip::Text { text: "plain clip".to_string(), html: None, rtf: None });

        let html = "<p><b>Rich</b> clip</p>";
        let rtf = "{\\rtf1\\ansi{\\b Rich} clip}";
        write_rich("Rich clip", Some(html), Some(rtf)).expect("write rich");
        assert_eq!(
            read(),
            Clip::Text {
                text: "Rich clip".to_string(),
                html: Some(html.to_string()),
                rtf: Some(rtf.to_string()),
            }
        );

        // If a clipboard bridge echoes the same plain text without rich MIME
        // formats, LAST_WRITTEN_RICH preserves the HTML and RTF formats.
        write("Rich clip").expect("echo plain text");
        assert_eq!(
            read(),
            Clip::Text {
                text: "Rich clip".to_string(),
                html: Some(html.to_string()),
                rtf: Some(rtf.to_string()),
            }
        );

        // Writing private text marks the clipboard as Private and clears rich state.
        write_private("948271").expect("write private");
        assert_eq!(read(), Clip::Private);
        assert!(LAST_WRITTEN_RICH.lock().unwrap_or_else(|e| e.into_inner()).is_none());
    }

    #[test]
    fn image_png_and_bitmap_round_trip_and_update_fingerprint() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let bmp = Bitmap { width: 4, height: 4, bgra: [0x42, 0x84, 0xC6, 0xFF].repeat(16) };
        let png = image::encode_png(&bmp).expect("encode png");
        write_image("image/png", &png).expect("write image");
        let read_png = read_image_png().expect("read_image_png");
        let decoded = image::decode(&read_png).expect("decode round-trip png");
        assert_eq!(decoded, bmp);

        let dib = image::dibv5(&bmp);
        let img_from_dib = Image::Bitmap(dib);
        let converted = img_from_dib.to_png().expect("Bitmap::to_png");
        assert_eq!(image::decode(&converted).expect("decode"), bmp);
    }

    #[test]
    fn file_uri_encoding_and_write_read_files_round_trip() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let path = PathBuf::from("/path/with spaces/doc #1 (final).pdf");
        let uri = to_file_uri(&path);
        assert_eq!(uri, "file:///path/with%20spaces/doc%20%231%20%28final%29.pdf");
        assert_eq!(from_file_uri(&uri), Some(path.clone()));
        assert_eq!(
            from_file_uri("file://localhost/home/alice/notes%202.txt"),
            Some(PathBuf::from("/home/alice/notes 2.txt"))
        );
        assert_eq!(from_file_uri("# comment line"), None);

        let paths =
            vec![PathBuf::from("/tmp/nectarlink_a.txt"), PathBuf::from("/tmp/nectarlink dir/b 🐝.png")];
        let list = format_uri_list(&paths);
        assert_eq!(parse_uri_list(&list), paths);

        assert!(!write_files(&[]));
        assert!(write_files(&paths));
        assert_eq!(read_files(), Some(paths));
    }

    #[test]
    fn loop_prevention_fingerprint_deduplicates_writes_and_copies() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        write("from phone").expect("write from phone");

        // Because write() updated LAST_REPORTED, report_if_changed() must not
        // echo "from phone" back to the phone (even with CRLF differences).
        let mut reported = Vec::new();
        assert!(!report_if_changed(
            Clip::Text { text: "from phone\r\n".into(), html: None, rtf: None },
            |c| reported.push(c)
        ));
        let _ = write("from phone");
        assert!(!report_if_changed(Clip::Text { text: "from phone".into(), html: None, rtf: None }, |c| {
            reported.push(c)
        }));
        assert!(reported.is_empty());

        // When new user content appears on the PC, it is reported exactly once.
        let user_clip = Clip::Text { text: "copied on PC".into(), html: None, rtf: None };
        assert!(report_if_changed(user_clip.clone(), |c| reported.push(c)));
        assert!(!report_if_changed(user_clip, |c| reported.push(c)));
        assert_eq!(reported.len(), 1);
    }

    #[test]
    fn password_manager_mime_hint_detection() {
        let normal_targets = "TIMESTAMP\nTARGETS\ntext/plain\ntext/plain;charset=utf-8\n";
        assert!(!is_secret_mime_list(normal_targets));

        let kde_targets = "TARGETS\nx-kde-passwordManagerHint\ntext/plain\n";
        assert!(is_secret_mime_list(kde_targets));
        assert!(is_kde_secret_value(b"secret\n"));
        assert!(is_kde_secret_value(b"secret\0"));
        assert!(!is_kde_secret_value(b"public"));

        let gnome_targets = "text/plain\nx-gnome-privacy-hint\n";
        assert!(is_secret_mime_list(gnome_targets));
    }

    #[test]
    fn copied_files_uri_list_is_not_treated_as_text_clip() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let targets = "text/plain\ntext/uri-list\n";
        assert!(has_copied_files_mime(targets));
        let gnome_targets = "x-special/gnome-copied-files\nUTF8_STRING\n";
        assert!(has_copied_files_mime(gnome_targets));

        assert!(is_copied_file_uri_text(
            "file:///home/jochen/Downloads/Nectarlink/PXL_20261016_130349832.MP.jpg\r\n"
        ));
        assert!(is_copied_file_uri_text(
            "copy\nfile:///home/jochen/Downloads/Nectarlink/PXL_20261016_130349832.MP.jpg"
        ));
        assert!(!is_copied_file_uri_text("Check out file:///tmp/a.txt and https://example.com"));

        // After write_files, read() must return Clip::Empty (so send_now falls through to
        // read_files() and the background watcher never echoes file:///... as text).
        let paths = vec![PathBuf::from("/home/jochen/Downloads/Nectarlink/PXL_20261016_130349832.MP.jpg")];
        assert!(write_files(&paths));
        assert_eq!(read(), Clip::Empty);
        let mut reported = Vec::new();
        assert!(!report_if_changed(
            Clip::Text {
                text: "file:///home/jochen/Downloads/Nectarlink/PXL_20261016_130349832.MP.jpg".into(),
                html: None,
                rtf: None,
            },
            |c| reported.push(c)
        ));
        assert!(reported.is_empty());
    }
}
