// SPDX-License-Identifier: GPL-3.0-or-later
//! The Windows clipboard: watching for copied text and images, reading them
//! and writing what a phone sent (docs/protocol/clipboard.md).
//!
//! A hidden message-only window on its own thread listens for clipboard
//! changes and owns what Nectarlink writes, so text that came from a phone
//! is never sent back. Content that password managers mark as private is
//! never read for sending. Images are read as PNG when the app offers it,
//! or else as a bitmap (converted to PNG off the clipboard thread), and
//! written as a bitmap with alpha, plus PNG when that's what arrived.

use std::{
    hash::{DefaultHasher, Hash, Hasher},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicIsize, Ordering},
    },
    time::Duration,
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
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, HWND_MESSAGE, MSG,
            RegisterClassExW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLIPBOARDUPDATE, WNDCLASSEXW,
        },
    },
    core::{HSTRING, w},
};

/// Standard clipboard formats.
const CF_DIB: u32 = 8;
const CF_UNICODETEXT: u32 = 13;
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
    Text(String),
    Image(Image),
    /// Marked private (a password manager): never sent.
    Private,
    /// Nothing that's text or an image.
    Empty,
}

type OnCopy = Box<dyn Fn(Clip) + Send + Sync>;

/// The listener window, once started (as an isize: HWND isn't Send).
static WINDOW: AtomicIsize = AtomicIsize::new(0);
static ON_COPY: OnceLock<OnCopy> = OnceLock::new();
/// A fingerprint of the last content reported, so one copy reported several
/// times (apps often write the clipboard in steps) is sent once.
static LAST_REPORTED: Mutex<Option<u64>> = Mutex::new(None);

fn fingerprint(clip: &Clip) -> Option<u64> {
    let mut hasher = DefaultHasher::new();
    match clip {
        Clip::Text(text) => text.hash(&mut hasher),
        Clip::Image(Image::Png(bytes) | Image::Bitmap(bytes)) => bytes.hash(&mut hasher),
        Clip::Private | Clip::Empty => return None,
    }
    Some(hasher.finish())
}

/// Starts watching the clipboard; `on_copy` gets each newly copied text or
/// image that isn't private and didn't come from Nectarlink, on the watcher
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
            w!("Nectarlink clipboard"),
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
    // What Nectarlink wrote (a phone's text) isn't sent back.
    // SAFETY: GetClipboardOwner has no preconditions.
    if unsafe { GetClipboardOwner() }.is_ok_and(|owner| owner == hwnd) {
        return;
    }
    let clip = read_with(Some(hwnd));
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

/// What's on the clipboard now.
pub fn read() -> Clip {
    read_with(window())
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
    write_text(text, false)
}

/// Puts a secret (a one-time code) on the clipboard, marked the way password
/// managers mark theirs, so Windows' clipboard history, cloud clipboard and
/// other clipboard tools leave it alone.
pub fn write_private(text: &str) -> Result<(), String> {
    write_text(text, true)
}

fn write_text(text: &str, private: bool) -> Result<(), String> {
    let owner = window().ok_or("the clipboard watcher isn't running")?;
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
    }
    *LAST_REPORTED.lock().unwrap_or_else(|e| e.into_inner()) = fingerprint(&Clip::Text(text.to_owned()));
    Ok(())
}

/// Puts an image a phone sent (`image/png` or `image/jpeg`) on the
/// clipboard: as a bitmap with alpha, which every app pastes, and also as
/// PNG when it is one, which keeps it exact for apps that read PNG.
pub fn write_image(mime: &str, bytes: &[u8]) -> Result<(), String> {
    let owner = window().ok_or("the clipboard watcher isn't running")?;
    let bitmap = super::image::decode(bytes).map_err(|e| e.to_string())?;
    let mut blocks = vec![(CF_DIBV5, super::image::dibv5(&bitmap))];
    if mime == "image/png" {
        blocks.push((format("PNG"), bytes.to_vec()));
    }
    let _open = Opened::open(Some(owner)).ok_or("the clipboard is busy")?;
    // SAFETY: the clipboard is open.
    unsafe { EmptyClipboard() }.map_err(|e| e.to_string())?;
    for (format, data) in blocks {
        set_block(format, &data)?;
    }
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

fn read_with(owner: Option<HWND>) -> Clip {
    // SAFETY: format availability checks need no open clipboard.
    let has = |format: u32| format != 0 && unsafe { IsClipboardFormatAvailable(format) }.is_ok();
    let png = format("PNG");
    let text = has(CF_UNICODETEXT);
    if !text && !has(png) && !has(CF_DIBV5) && !has(CF_DIB) {
        return Clip::Empty;
    }
    // Password managers and other apps mark private content this way.
    if has(format("ExcludeClipboardContentFromMonitorProcessing")) || has(format("Clipboard Viewer Ignore")) {
        return Clip::Private;
    }
    let Some(_open) = Opened::open(owner) else { return Clip::Empty };
    if let Some(0) = read_dword(format("CanIncludeInClipboardHistory")) {
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
    if text.trim().is_empty() { Clip::Empty } else { Clip::Text(text) }
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
