// SPDX-License-Identifier: GPL-3.0-or-later
//! The Windows clipboard: watching for copied text and images, reading them
//! and writing what a phone sent (docs/protocol/clipboard.md).
//!
//! A hidden message-only window on its own thread listens for clipboard
//! changes and owns what NectarLink writes, so text that came from a phone
//! is never sent back. Content that password managers mark as private is
//! never read for sending. Images are read as PNG when the app offers it,
//! or else as a bitmap (converted to PNG off the clipboard thread), and
//! written as a bitmap with alpha, plus PNG when that's what arrived.

use std::{
    ffi::OsString,
    hash::{DefaultHasher, Hash, Hasher},
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::PathBuf,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicIsize, Ordering},
    },
    time::{Duration, Instant},
};

use windows::{
    Win32::{
        Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND, LPARAM, LRESULT, WPARAM},
        System::{
            DataExchange::{
                AddClipboardFormatListener, CloseClipboard, EmptyClipboard, GetClipboardData,
                GetClipboardOwner, IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW,
                SetClipboardData,
            },
            LibraryLoader::GetModuleHandleW,
            Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock},
        },
        UI::{
            Shell::{DROPFILES, DragQueryFileW, HDROP},
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, HWND_MESSAGE, MSG,
                RegisterClassExW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLIPBOARDUPDATE, WNDCLASSEXW,
            },
        },
    },
    core::{HSTRING, w},
};

/// Standard clipboard formats.
const CF_DIB: u32 = 8;
const CF_UNICODETEXT: u32 = 13;
const CF_HDROP: u32 = 15;
const CF_DIBV5: u32 = 17;
/// Copied bitmaps larger than this aren't read (an 8K screenshot is
/// about 130 MB raw).
const MAX_BITMAP_BYTES: usize = 160 * 1024 * 1024;

/// A copied image, as the app offered it.
#[derive(Clone, PartialEq, Eq)]
pub enum Image {
    Png(Vec<u8>),
    /// A device-independent bitmap (`CF_DIBV5` or `CF_DIB`).
    Bitmap(Vec<u8>),
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (kind, len) = match self {
            Image::Png(b) => ("Png", b.len()),
            Image::Bitmap(b) => ("Bitmap", b.len()),
        };
        f.debug_struct(kind).field("bytes", &len).finish()
    }
}

impl Image {
    /// As a PNG file (bitmaps are converted; this can take a moment).
    pub fn to_png(&self) -> Result<Vec<u8>, String> {
        match self {
            Image::Png(png) => Ok(png.clone()),
            Image::Bitmap(dib) => {
                let bmp = super::image::bmp_from_dib(dib).ok_or("not a bitmap Windows knows")?;
                let bitmap = super::image::decode(&bmp).map_err(|e| e.to_string())?;
                super::image::encode_png(&bitmap).map_err(|e| e.to_string())
            }
        }
    }
}

/// What's on the clipboard, for sending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Clip {
    Text {
        text: String,
        html: Option<String>,
        rtf: Option<String>,
    },
    Image(Image),
    /// Marked private (a password manager): never sent.
    Private,
    /// Nothing that's text or an image.
    Empty,
}

type OnCopy = Box<dyn Fn(Clip) + Send + Sync>;

type WrittenRich = (Instant, String, Option<String>, Option<String>);

/// The listener window, once started (as an isize: HWND isn't Send).
static WINDOW: AtomicIsize = AtomicIsize::new(0);
static ON_COPY: OnceLock<OnCopy> = OnceLock::new();
/// A fingerprint of the last content reported, so one copy reported several
/// times (apps often write the clipboard in steps) is sent once.
static LAST_REPORTED: Mutex<Option<u64>> = Mutex::new(None);
/// The last rich-text clip written by NectarLink, so if a host-clipboard bridge
/// (such as the Android emulator's QEMU clipboard pipe) immediately echoes the
/// same plain text without rich formats, we preserve the rich formats.
static LAST_WRITTEN_RICH: Mutex<Option<WrittenRich>> = Mutex::new(None);

fn fingerprint(clip: &Clip) -> Option<u64> {
    let mut hasher = DefaultHasher::new();
    match clip {
        Clip::Text { text, .. } => text.replace("\r\n", "\n").hash(&mut hasher),
        Clip::Image(Image::Png(bytes) | Image::Bitmap(bytes)) => bytes.hash(&mut hasher),
        Clip::Private | Clip::Empty => return None,
    }
    Some(hasher.finish())
}

/// Starts watching the clipboard; `on_copy` gets each newly copied text or
/// image that isn't private and didn't come from NectarLink, on the watcher
/// thread.
pub fn start(on_copy: impl Fn(Clip) + Send + Sync + 'static) {
    if ON_COPY.set(Box::new(on_copy)).is_err() {
        return; // already started
    }
    let spawned = std::thread::Builder::new().name("clipboard".into()).spawn(|| {
        if let Err(e) = run() {
            tracing::warn!(error = %e, "can't watch the clipboard");
        }
    });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "can't watch the clipboard");
    }
}

fn run() -> windows::core::Result<()> {
    // SAFETY: standard class registration and message-only window creation;
    // the window procedure only reads statics.
    let hwnd = unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: w!("Nectarlink.Clipboard"),
            ..Default::default()
        };
        RegisterClassExW(&class);
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("Nectarlink.Clipboard"),
            w!("NectarLink clipboard"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instance.into()),
            None,
        )?
    };
    WINDOW.store(hwnd.0 as isize, Ordering::Release);
    // SAFETY: registering our own window for clipboard change messages.
    unsafe { AddClipboardFormatListener(hwnd)? };
    let mut msg = MSG::default();
    // SAFETY: a plain message loop for this thread's window.
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_CLIPBOARDUPDATE {
        on_clipboard_changed(hwnd);
        return LRESULT(0);
    }
    // SAFETY: default handling for everything else.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn on_clipboard_changed(hwnd: HWND) {
    // What NectarLink wrote (a phone's text) isn't sent back.
    // SAFETY: GetClipboardOwner has no preconditions.
    if unsafe { GetClipboardOwner() }.is_ok_and(|owner| owner == hwnd) {
        return;
    }
    let clip = read_with(Some(hwnd), true);
    // A password manager's: the listener decides, by the user's choice.
    if matches!(clip, Clip::Private) {
        if let Some(on_copy) = ON_COPY.get() {
            on_copy(clip);
        }
        return;
    }
    if let Clip::Text { ref text, html: None, rtf: None } = clip {
        let restore = LAST_WRITTEN_RICH
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .filter(|(at, prev_text, h, r)| {
                at.elapsed() < Duration::from_secs(2) && prev_text == text && (h.is_some() || r.is_some())
            })
            .map(|(_, _, h, r)| (h.clone(), r.clone()));
        if let Some((saved_html, saved_rtf)) = restore {
            let _ = write_text(text, saved_html.as_deref(), saved_rtf.as_deref(), false);
            return;
        }
    }
    let Some(print) = fingerprint(&clip) else { return };
    {
        let mut last = LAST_REPORTED.lock().unwrap_or_else(|e| e.into_inner());
        if *last == Some(print) {
            return;
        }
        *last = Some(print);
    }
    if let Some(on_copy) = ON_COPY.get() {
        on_copy(clip);
    }
}

fn window() -> Option<HWND> {
    let raw = WINDOW.load(Ordering::Acquire);
    (raw != 0).then_some(HWND(raw as *mut _))
}

/// What's on the clipboard now, for watching it: content marked private
/// (password managers, or kept out of clipboard history) reads as
/// [`Clip::Private`].
pub fn read() -> Clip {
    read_with(window(), true)
}

/// What's on the clipboard now, for a paste the user asked for (Ctrl+V,
/// or an item picked in clipboard history, which Windows marks to keep
/// monitors away). Pasting it is what any app does; only watchers skip it.
pub fn read_for_paste() -> Clip {
    read_with(window(), false)
}

/// Empties the clipboard if `text` is still what's on it.
pub fn clear_private(text: &str) {
    let Clip::Text { text: now, .. } = read_for_paste() else { return };
    if now.replace("\r\n", "\n") != text.replace("\r\n", "\n") {
        return;
    }
    let Some(_open) = Opened::open(window()) else { return };
    // SAFETY: the clipboard is open on this thread until `_open` drops.
    if let Err(error) = unsafe { EmptyClipboard() } {
        tracing::warn!(%error, "can't clear a password from the clipboard");
    }
}

/// The text a password manager marked private, for sending it when the user
/// allows that.
pub fn read_private_text() -> Option<String> {
    match read_for_paste() {
        Clip::Text { text, .. } => Some(text),
        _ => None,
    }
}

/// Reads an image from the clipboard as PNG bytes, if an image (`PNG`,
/// `CF_DIBV5`, or `CF_DIB`) is on the clipboard and isn't marked private.
pub fn read_image_png() -> Option<Vec<u8>> {
    // SAFETY: format availability checks need no open clipboard.
    let has = |format: u32| format != 0 && unsafe { IsClipboardFormatAvailable(format) }.is_ok();
    let png = format("PNG");
    if !has(png) && !has(CF_DIBV5) && !has(CF_DIB) {
        return None;
    }
    if has(format("ExcludeClipboardContentFromMonitorProcessing")) || has(format("Clipboard Viewer Ignore")) {
        return None;
    }
    let _open = Opened::open(window())?;
    if let Some(0) = read_dword(format("CanIncludeInClipboardHistory")) {
        return None;
    }
    let image = [(png, true), (CF_DIBV5, false), (CF_DIB, false)]
        .into_iter()
        .filter(|&(format, _)| has(format))
        .find_map(|(format, is_png)| {
            let bytes = read_block(format)?;
            Some(if is_png { Image::Png(bytes) } else { Image::Bitmap(bytes) })
        })?;
    drop(_open);
    image.to_png().ok()
}

/// Puts text on the clipboard (owned by the watcher window, so it isn't
/// reported as a new copy).
pub fn write(text: &str) -> Result<(), String> {
    write_text(text, None, None, false)
}

/// Puts text (and optional HTML and RTF rich text) on the clipboard.
pub fn write_rich(text: &str, html: Option<&str>, rtf: Option<&str>) -> Result<(), String> {
    write_text(text, html, rtf, false)
}

/// Puts a secret (a one-time code) on the clipboard, marked the way password
/// managers mark theirs, so Windows' clipboard history, cloud clipboard and
/// other clipboard tools leave it alone.
pub fn write_private(text: &str) -> Result<(), String> {
    write_text(text, None, None, true)
}

fn write_text(text: &str, html: Option<&str>, rtf: Option<&str>, private: bool) -> Result<(), String> {
    let owner = window().ok_or("the clipboard watcher isn't running")?;
    let mut html_owned = html.filter(|s| !s.trim().is_empty()).map(str::to_owned);
    let mut rtf_owned = rtf.filter(|s| !s.trim().is_empty()).map(str::to_owned);
    if !private
        && html_owned.is_none()
        && rtf_owned.is_none()
        && let Some((at, prev_text, prev_h, prev_r)) =
            LAST_WRITTEN_RICH.lock().unwrap_or_else(|e| e.into_inner()).as_ref()
        && at.elapsed() < Duration::from_secs(2)
        && prev_text == text
    {
        html_owned = prev_h.clone();
        rtf_owned = prev_r.clone();
    }
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let _open = Opened::open(Some(owner)).ok_or("the clipboard is busy")?;
    // SAFETY: the clipboard is open; the global block is sized for `wide`,
    // and ownership passes to the clipboard only when SetClipboardData
    // succeeds (it's freed otherwise).
    unsafe {
        EmptyClipboard().map_err(|e| e.to_string())?;
        let block = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2).map_err(|e| e.to_string())?;
        let target = GlobalLock(block).cast::<u16>();
        if target.is_null() {
            let _ = GlobalFree(Some(block));
            return Err("can't lock memory".into());
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), target, wide.len());
        let _ = GlobalUnlock(block);
        if let Err(e) = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(block.0))) {
            let _ = GlobalFree(Some(block));
            return Err(e.to_string());
        }
    }
    if private {
        set_block(format("ExcludeClipboardContentFromMonitorProcessing"), &[0])?;
        set_block(format("CanIncludeInClipboardHistory"), &0u32.to_le_bytes())?;
        set_block(format("CanUploadToCloudClipboard"), &0u32.to_le_bytes())?;
        *LAST_WRITTEN_RICH.lock().unwrap_or_else(|e| e.into_inner()) = None;
    } else {
        if let Some(ref h) = html_owned {
            let _ = set_block(format("HTML Format"), &encode_cf_html(h));
        }
        if let Some(ref r) = rtf_owned {
            let _ = set_block(format("Rich Text Format"), &encode_cf_rtf(r));
        }
        *LAST_WRITTEN_RICH.lock().unwrap_or_else(|e| e.into_inner()) =
            if html_owned.is_some() || rtf_owned.is_some() {
                Some((Instant::now(), text.to_owned(), html_owned, rtf_owned))
            } else {
                None
            };
    }
    *LAST_REPORTED.lock().unwrap_or_else(|e| e.into_inner()) =
        fingerprint(&Clip::Text { text: text.to_owned(), html: None, rtf: None });
    crate::mirror::note_remote_clipboard(text);
    Ok(())
}

/// Puts an image a phone sent (`image/png` or `image/jpeg`) on the
/// clipboard: as a bitmap with alpha, which every app pastes, and also as
/// PNG when it is one, which keeps it exact for apps that read PNG.
pub fn write_image(mime: &str, bytes: &[u8]) -> Result<(), String> {
    let owner = window().ok_or("the clipboard watcher isn't running")?;
    let bitmap = super::image::decode(bytes).map_err(|e| e.to_string())?;
    let dib = super::image::dibv5(&bitmap);
    let reported_clip = if mime == "image/png" {
        Clip::Image(Image::Png(bytes.to_vec()))
    } else {
        Clip::Image(Image::Bitmap(dib.clone()))
    };
    let mut blocks = vec![(CF_DIBV5, dib)];
    if mime == "image/png" {
        blocks.push((format("PNG"), bytes.to_vec()));
    }
    let _open = Opened::open(Some(owner)).ok_or("the clipboard is busy")?;
    // SAFETY: the clipboard is open.
    unsafe { EmptyClipboard() }.map_err(|e| e.to_string())?;
    for (format, data) in blocks {
        set_block(format, &data)?;
    }
    *LAST_REPORTED.lock().unwrap_or_else(|e| e.into_inner()) = fingerprint(&reported_clip);
    Ok(())
}

/// Copies `data` into a global block and hands it to the clipboard, which
/// must be open.
fn set_block(format: u32, data: &[u8]) -> Result<(), String> {
    // SAFETY: the block is sized for `data`; ownership passes to the
    // clipboard only when SetClipboardData succeeds (it's freed otherwise).
    unsafe {
        let block = GlobalAlloc(GMEM_MOVEABLE, data.len()).map_err(|e| e.to_string())?;
        let target = GlobalLock(block).cast::<u8>();
        if target.is_null() {
            let _ = GlobalFree(Some(block));
            return Err("can't lock memory".into());
        }
        std::ptr::copy_nonoverlapping(data.as_ptr(), target, data.len());
        let _ = GlobalUnlock(block);
        if let Err(e) = SetClipboardData(format, Some(HANDLE(block.0))) {
            let _ = GlobalFree(Some(block));
            return Err(e.to_string());
        }
    }
    Ok(())
}

/// The clipboard, opened; closed when dropped. Other apps hold it briefly
/// while they write, so opening retries for a moment.
struct Opened;

impl Opened {
    fn open(owner: Option<HWND>) -> Option<Opened> {
        for _ in 0..20 {
            // SAFETY: opening the clipboard for this thread; closed in Drop.
            if unsafe { OpenClipboard(owner) }.is_ok() {
                return Some(Opened);
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        None
    }
}

impl Drop for Opened {
    fn drop(&mut self) {
        // SAFETY: we opened it.
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

fn format(name: &str) -> u32 {
    // SAFETY: registering (or looking up) a named format.
    unsafe { RegisterClipboardFormatW(&HSTRING::from(name)) }
}

fn read_with(owner: Option<HWND>, skip_private: bool) -> Clip {
    // SAFETY: format availability checks need no open clipboard.
    let has = |format: u32| format != 0 && unsafe { IsClipboardFormatAvailable(format) }.is_ok();
    let png = format("PNG");
    let text = has(CF_UNICODETEXT);
    if !text && !has(png) && !has(CF_DIBV5) && !has(CF_DIB) {
        return Clip::Empty;
    }
    // Password managers and other apps mark private content this way.
    if skip_private
        && (has(format("ExcludeClipboardContentFromMonitorProcessing"))
            || has(format("Clipboard Viewer Ignore")))
    {
        return Clip::Private;
    }
    let Some(_open) = Opened::open(owner) else { return Clip::Empty };
    if skip_private && let Some(0) = read_dword(format("CanIncludeInClipboardHistory")) {
        return Clip::Private;
    }
    // Apps that copy both (a table from a spreadsheet) mean the text.
    if !text {
        return [(png, true), (CF_DIBV5, false), (CF_DIB, false)]
            .into_iter()
            .filter(|&(format, _)| has(format))
            .find_map(|(format, is_png)| {
                let bytes = read_block(format)?;
                Some(Clip::Image(if is_png { Image::Png(bytes) } else { Image::Bitmap(bytes) }))
            })
            .unwrap_or(Clip::Empty);
    }
    // SAFETY: the clipboard is open; the data handle is valid until it's
    // closed, and the text is read within the block's size.
    let text = unsafe {
        let Ok(handle) = GetClipboardData(CF_UNICODETEXT) else { return Clip::Empty };
        let block = HGLOBAL(handle.0);
        let units = GlobalSize(block) / 2;
        let data = GlobalLock(block).cast::<u16>();
        if data.is_null() {
            return Clip::Empty;
        }
        let slice = std::slice::from_raw_parts(data, units);
        let len = slice.iter().position(|&c| c == 0).unwrap_or(units);
        let text = String::from_utf16_lossy(&slice[..len]);
        let _ = GlobalUnlock(block);
        text
    };
    if text.trim().is_empty() {
        return Clip::Empty;
    }
    let html_fmt = format("HTML Format");
    let html = if has(html_fmt) { read_block(html_fmt).and_then(|b| decode_cf_html(&b)) } else { None };
    let rtf_fmt = format("Rich Text Format");
    let rtf = if has(rtf_fmt) { read_block(rtf_fmt).and_then(|b| decode_cf_rtf(&b)) } else { None };
    Clip::Text { text, html, rtf }
}

/// Wraps an HTML fragment in Windows' `"HTML Format"` (`CF_HTML`) header with
/// 10-digit zero-padded byte offsets and a trailing NUL byte.
pub(crate) fn encode_cf_html(fragment: &str) -> Vec<u8> {
    const HEADER_TEMPLATE: &str = "Version:0.9\r\nStartHTML:0000000000\r\nEndHTML:0000000000\r\nStartFragment:0000000000\r\nEndFragment:0000000000\r\n";
    const PREFIX: &str = "<html>\r\n<body>\r\n<!--StartFragment-->";
    const SUFFIX: &str = "<!--EndFragment-->\r\n</body>\r\n</html>";
    let start_html = HEADER_TEMPLATE.len();
    let start_fragment = start_html + PREFIX.len();
    let end_fragment = start_fragment + fragment.len();
    let end_html = end_fragment + SUFFIX.len();
    let header = format!(
        "Version:0.9\r\nStartHTML:{start_html:010}\r\nEndHTML:{end_html:010}\r\nStartFragment:{start_fragment:010}\r\nEndFragment:{end_fragment:010}\r\n"
    );
    debug_assert_eq!(header.len(), HEADER_TEMPLATE.len());
    let mut out = Vec::with_capacity(end_html + 1);
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(PREFIX.as_bytes());
    out.extend_from_slice(fragment.as_bytes());
    out.extend_from_slice(SUFFIX.as_bytes());
    out.push(0);
    out
}

/// Extracts the HTML fragment from a Windows `"HTML Format"` (`CF_HTML`) block.
pub(crate) fn decode_cf_html(bytes: &[u8]) -> Option<String> {
    let end = bytes.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
    let raw = &bytes[..end];
    if raw.is_empty() || raw.len() > nectarlink_core::CLIP_MAX_BYTES {
        return None;
    }
    let text = std::str::from_utf8(raw).ok()?;
    let parse_offset = |key: &str| -> Option<usize> {
        text.lines()
            .take(12)
            .find_map(|line| line.strip_prefix(key))
            .and_then(|val| val.trim().parse::<usize>().ok())
    };
    if let (Some(sf), Some(ef)) = (parse_offset("StartFragment:"), parse_offset("EndFragment:"))
        && sf > 0
        && sf < ef
        && ef <= raw.len()
        && let Ok(frag) = std::str::from_utf8(&raw[sf..ef])
    {
        let trimmed = frag.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_owned());
        }
    }
    const START_MARKER: &str = "<!--StartFragment-->";
    const END_MARKER: &str = "<!--EndFragment-->";
    if let Some(s_idx) = text.find(START_MARKER) {
        let content_start = s_idx + START_MARKER.len();
        if let Some(rel_end) = text[content_start..].find(END_MARKER) {
            let trimmed = text[content_start..content_start + rel_end].trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_owned());
            }
        }
    }
    if let (Some(sh), Some(eh)) = (parse_offset("StartHTML:"), parse_offset("EndHTML:"))
        && sh > 0
        && sh < eh
        && eh <= raw.len()
        && let Ok(html) = std::str::from_utf8(&raw[sh..eh])
    {
        let trimmed = html.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_owned());
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
    let end = bytes.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
    let raw = &bytes[..end];
    if raw.is_empty() || raw.len() > nectarlink_core::CLIP_MAX_BYTES {
        return None;
    }
    let s = std::str::from_utf8(raw).ok()?.trim();
    s.starts_with("{\\rtf").then(|| s.to_owned())
}

/// A format's raw bytes, if present and not too large. The clipboard must
/// be open.
fn read_block(format: u32) -> Option<Vec<u8>> {
    // SAFETY: the caller opened the clipboard; the block is read within its
    // size while locked.
    unsafe {
        let handle = GetClipboardData(format).ok()?;
        let block = HGLOBAL(handle.0);
        let size = GlobalSize(block);
        if size == 0 || size > MAX_BITMAP_BYTES {
            return None;
        }
        let data = GlobalLock(block).cast::<u8>();
        if data.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(data, size).to_vec();
        let _ = GlobalUnlock(block);
        Some(bytes)
    }
}

/// A DWORD-valued clipboard format, if present. The clipboard must be open.
fn read_dword(format: u32) -> Option<u32> {
    if format == 0 {
        return None;
    }
    // SAFETY: the caller opened the clipboard; the block is read only if it
    // holds at least four bytes.
    unsafe {
        let handle = GetClipboardData(format).ok()?;
        let block = HGLOBAL(handle.0);
        if GlobalSize(block) < 4 {
            return None;
        }
        let data = GlobalLock(block).cast::<u32>();
        if data.is_null() {
            return None;
        }
        let value = data.read_unaligned();
        let _ = GlobalUnlock(block);
        Some(value)
    }
}

/// Builds a `CF_HDROP` (`DROPFILES`) binary payload for `paths` (UTF-16LE,
/// double-null-terminated).
pub fn build_dropfiles_payload(paths: &[PathBuf]) -> Vec<u8> {
    let header_size = std::mem::size_of::<DROPFILES>();
    debug_assert_eq!(header_size, 20);
    let mut wide_units: Vec<u16> = Vec::new();
    for path in paths {
        wide_units.extend(path.as_os_str().encode_wide());
        wide_units.push(0);
    }
    wide_units.push(0);

    let mut out = Vec::with_capacity(header_size + wide_units.len() * 2);
    out.extend_from_slice(&(header_size as u32).to_le_bytes());
    out.extend_from_slice(&0i32.to_le_bytes());
    out.extend_from_slice(&0i32.to_le_bytes());
    out.extend_from_slice(&0i32.to_le_bytes());
    out.extend_from_slice(&1i32.to_le_bytes());
    for unit in wide_units {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    out
}

/// Parses a `CF_HDROP` (`DROPFILES`) binary payload into file paths.
#[cfg(test)]
pub fn parse_dropfiles_payload(bytes: &[u8]) -> Option<Vec<PathBuf>> {
    let header_size = std::mem::size_of::<DROPFILES>();
    if bytes.len() < header_size {
        return None;
    }
    let p_files = u32::from_le_bytes(bytes[0..4].try_into().ok()?) as usize;
    let f_wide = i32::from_le_bytes(bytes[16..20].try_into().ok()?) != 0;
    if !f_wide || p_files < header_size || p_files > bytes.len() {
        return None;
    }
    let tail = &bytes[p_files..];
    if tail.len() < 2 {
        return None;
    }
    let units: Vec<u16> = tail.as_chunks::<2>().0.iter().map(|&c| u16::from_le_bytes(c)).collect();
    let mut paths = Vec::new();
    let mut start = 0usize;
    for (idx, &u) in units.iter().enumerate() {
        if u == 0 {
            if idx == start {
                break;
            }
            paths.push(PathBuf::from(OsString::from_wide(&units[start..idx])));
            start = idx + 1;
        }
    }
    Some(paths)
}

/// Reads copied file paths (`CF_HDROP`) from the Windows clipboard, returning
/// only paths that exist on disk.
pub fn read_files() -> Option<Vec<PathBuf>> {
    // SAFETY: format availability check needs no open clipboard.
    if unsafe { IsClipboardFormatAvailable(CF_HDROP) }.is_err() {
        return None;
    }
    let _open = Opened::open(None)?;
    // SAFETY: the clipboard is open; `GetClipboardData(CF_HDROP)` returns an HDROP
    // backed by global memory that stays valid until `CloseClipboard`.
    unsafe {
        let handle = GetClipboardData(CF_HDROP).ok()?;
        let block = HGLOBAL(handle.0);
        let ptr = GlobalLock(block);
        if ptr.is_null() {
            return None;
        }
        let hdrop = HDROP(ptr);
        let count = DragQueryFileW(hdrop, 0xFFFF_FFFF, None);
        let mut paths = Vec::with_capacity(count as usize);
        for i in 0..count {
            let len = DragQueryFileW(hdrop, i, None);
            if len == 0 {
                continue;
            }
            let mut buf = vec![0u16; (len as usize) + 1];
            let copied = DragQueryFileW(hdrop, i, Some(&mut buf));
            if copied > 0 {
                let p = PathBuf::from(OsString::from_wide(&buf[..copied as usize]));
                if p.exists() {
                    paths.push(p);
                }
            }
        }
        let _ = GlobalUnlock(block);
        (!paths.is_empty()).then_some(paths)
    }
}

/// Places `paths` onto the Windows clipboard as `CF_HDROP` so the user can
/// immediately press `Ctrl+V` in File Explorer or any desktop app.
pub fn write_files(paths: &[PathBuf]) -> bool {
    if paths.is_empty() {
        return false;
    }
    let payload = build_dropfiles_payload(paths);
    let Some(_open) = Opened::open(window()) else {
        return false;
    };
    // SAFETY: the clipboard is open.
    if unsafe { EmptyClipboard() }.is_err() {
        return false;
    }
    set_block(CF_HDROP, &payload).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cf_html_and_rtf_round_trip() {
        let frag = "<p><b>Hello</b> <i>NectarLink</i> 🐝</p>";
        let encoded = encode_cf_html(frag);
        assert_eq!(decode_cf_html(&encoded).as_deref(), Some(frag));

        let marker_only = b"Version:0.9\r\n<html><body><!--StartFragment--><span>Fallback</span><!--EndFragment--></body></html>\0";
        assert_eq!(decode_cf_html(marker_only).as_deref(), Some("<span>Fallback</span>"));

        let rtf = r"{\rtf1\ansi\deff0 {\b Hello} world}";
        let rtf_bytes = encode_cf_rtf(rtf);
        assert_eq!(decode_cf_rtf(&rtf_bytes).as_deref(), Some(rtf));
        assert_eq!(decode_cf_rtf(b"not rtf\0"), None);
    }

    #[test]
    fn dropfiles_payload_round_trips_multiple_paths() {
        let paths = vec![
            PathBuf::from(r"C:\Users\Alice\Documents\report.pdf"),
            PathBuf::from(r"D:\Photos\Summer 2026\beach_🐝.jpg"),
        ];
        let payload = build_dropfiles_payload(&paths);
        assert!(payload.len() > 20);
        let parsed = parse_dropfiles_payload(&payload).expect("valid DROPFILES payload");
        assert_eq!(parsed, paths);
        assert!(parse_dropfiles_payload(&[0u8; 8]).is_none());
    }
}
