// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS clipboard implementation: pbcopy / pbpaste / file handling.

use std::{
    path::PathBuf,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

static CLIP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[allow(dead_code)]
#[derive(Clone, PartialEq, Eq)]
pub enum Image {
    Png(Vec<u8>),
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
    pub fn to_png(&self) -> Result<Vec<u8>, String> {
        match self {
            Image::Png(png) => Ok(png.clone()),
            Image::Bitmap(b) => Ok(b.clone()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Clip {
    Text {
        text: String,
        html: Option<String>,
        rtf: Option<String>,
    },
    #[allow(dead_code)]
    Image(Image),
    Private,
    Empty,
}

type OnCopy = Box<dyn Fn(Clip) + Send + Sync>;
static ON_COPY: OnceLock<OnCopy> = OnceLock::new();
static LAST_TEXT: Mutex<Option<String>> = Mutex::new(None);
static LAST_COUNT: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(-1);

pub fn start(on_copy: impl Fn(Clip) + Send + Sync + 'static) {
    let _ = ON_COPY.set(Box::new(on_copy));
    // Record initial changeCount at startup so we never broadcast existing clipboard contents
    let initial_count = crate::bridge::native::ffi::mac_clipboard_change_count();
    LAST_COUNT.store(initial_count, Ordering::Relaxed);

    std::thread::Builder::new()
        .name("mac-clipboard-watcher".into())
        .spawn(|| {
            loop {
                std::thread::sleep(std::time::Duration::from_millis(500));
                let count = crate::bridge::native::ffi::mac_clipboard_change_count();
                let last = LAST_COUNT.load(Ordering::Relaxed);
                if count != last {
                    LAST_COUNT.store(count, Ordering::Relaxed);
                    let clip = read();
                    match &clip {
                        Clip::Text { text, .. } => {
                            let mut last_text = LAST_TEXT.lock().unwrap_or_else(|e| e.into_inner());
                            if last_text.as_deref() != Some(text) {
                                *last_text = Some(text.clone());
                                drop(last_text);
                                if let Some(cb) = ON_COPY.get() {
                                    cb(clip);
                                }
                            }
                        }
                        Clip::Private => {
                            if let Some(cb) = ON_COPY.get() {
                                cb(clip);
                            }
                        }
                        _ => {}
                    }
                }
            }
        })
        .ok();
}

/// macOS gives the text only when it isn't marked private, so a password stays
/// out of reach here.
pub fn read_private_text() -> Option<String> {
    None
}

pub fn read() -> Clip {
    let mut is_private = false;
    let text = crate::bridge::native::ffi::mac_clipboard_read_text(&mut is_private);
    if is_private {
        return Clip::Private;
    }
    if !text.is_empty() {
        return Clip::Text { text, html: None, rtf: None };
    }
    Clip::Empty
}

pub fn read_for_paste() -> Clip {
    read()
}

pub fn read_image_png() -> Option<Vec<u8>> {
    None
}

pub fn write(text: &str) -> Result<(), String> {
    if crate::bridge::native::ffi::mac_clipboard_write_text(text, false) {
        *LAST_TEXT.lock().unwrap_or_else(|e| e.into_inner()) = Some(text.to_string());
        LAST_COUNT.store(crate::bridge::native::ffi::mac_clipboard_change_count(), Ordering::Relaxed);
        Ok(())
    } else {
        Err("mac_clipboard_write_text failed".into())
    }
}

pub fn write_rich(text: &str, _html: Option<&str>, _rtf: Option<&str>) -> Result<(), String> {
    write(text)
}

pub fn write_private(text: &str) -> Result<(), String> {
    if crate::bridge::native::ffi::mac_clipboard_write_text(text, true) {
        *LAST_TEXT.lock().unwrap_or_else(|e| e.into_inner()) = Some(text.to_string());
        LAST_COUNT.store(crate::bridge::native::ffi::mac_clipboard_change_count(), Ordering::Relaxed);
        Ok(())
    } else {
        Err("mac_clipboard_write_text failed".into())
    }
}

pub fn write_image(mime: &str, bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() {
        return Err("empty image data".into());
    }
    let png_bytes = if mime == "image/png" && bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        bytes.to_vec()
    } else {
        let bitmap = super::image::decode(bytes)?;
        super::image::encode_png(&bitmap)?
    };

    let seq = CLIP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let tmp = std::env::temp_dir().join(format!("nectar_clip_{}_{}_{}.png", std::process::id(), now, seq));

    std::fs::write(&tmp, &png_bytes).map_err(|e| e.to_string())?;
    let script =
        format!(r#"set the clipboard to (read (POSIX file "{}") as «class PNGf»)"#, tmp.to_string_lossy());
    let res = std::process::Command::new("osascript").args(["-e", &script]).output();
    let _ = std::fs::remove_file(&tmp);
    match res {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => Err(String::from_utf8_lossy(&out.stderr).into_owned()),
        Err(e) => Err(e.to_string()),
    }
}

#[allow(dead_code)]
pub fn build_dropfiles_payload(_paths: &[PathBuf]) -> Vec<u8> {
    Vec::new()
}

#[allow(dead_code)]
pub fn parse_dropfiles_payload(_bytes: &[u8]) -> Option<Vec<PathBuf>> {
    None
}

#[allow(dead_code)]
pub fn read_files() -> Option<Vec<PathBuf>> {
    None
}

#[allow(dead_code)]
pub fn write_files(_paths: &[PathBuf]) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_image_rejects_empty_data() {
        assert!(write_image("image/png", &[]).is_err());
    }

    #[test]
    fn write_image_rejects_corrupted_jpeg() {
        assert!(write_image("image/jpeg", &[0xff, 0xd8, 0x00, 0x00]).is_err());
    }

    #[test]
    fn write_image_converts_valid_jpeg() {
        #[rustfmt::skip]
        const TINY_JPEG: &[u8] = &[
            255, 216, 255, 224, 0, 16, 74, 70, 73, 70, 0, 1, 1, 0, 0, 1, 0, 1, 0, 0, 255, 219, 0, 67, 0, 8,
            6, 6, 7, 6, 5, 8, 7, 7, 7, 9, 9, 8, 10, 12, 20, 13, 12, 11, 11, 12, 25, 18, 19, 15, 20, 29, 26,
            31, 30, 29, 26, 28, 28, 32, 36, 46, 39, 32, 34, 44, 35, 28, 28, 40, 55, 41, 44, 48, 49, 52, 52,
            52, 31, 39, 57, 61, 56, 50, 60, 46, 51, 52, 50, 255, 219, 0, 67, 1, 9, 9, 9, 12, 11, 12, 24,
            13, 13, 24, 50, 33, 28, 33, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50,
            50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50,
            50, 50, 50, 50, 50, 50, 50, 50, 50, 255, 192, 0, 17, 8, 0, 1, 0, 1, 3, 1, 34, 0, 2, 17, 1, 3,
            17, 1, 255, 196, 0, 31, 0, 0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6,
            7, 8, 9, 10, 11, 255, 196, 0, 181, 16, 0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 125, 1, 2,
            3, 0, 4, 17, 5, 18, 33, 49, 65, 6, 19, 81, 97, 7, 34, 113, 20, 50, 129, 145, 161, 8, 35, 66,
            177, 193, 21, 82, 209, 240, 36, 51, 98, 114, 130, 9, 10, 22, 23, 24, 25, 26, 37, 38, 39, 40,
            41, 42, 52, 53, 54, 55, 56, 57, 58, 67, 68, 69, 70, 71, 72, 73, 74, 83, 84, 85, 86, 87, 88, 89,
            90, 99, 100, 101, 102, 103, 104, 105, 106, 115, 116, 117, 118, 119, 120, 121, 122, 131, 132,
            133, 134, 135, 136, 137, 138, 146, 147, 148, 149, 150, 151, 152, 153, 154, 162, 163, 164, 165,
            166, 167, 168, 169, 170, 178, 179, 180, 181, 182, 183, 184, 185, 186, 194, 195, 196, 197, 198,
            199, 200, 201, 202, 210, 211, 212, 213, 214, 215, 216, 217, 218, 225, 226, 227, 228, 229, 230,
            231, 232, 233, 234, 241, 242, 243, 244, 245, 246, 247, 248, 249, 250, 255, 196, 0, 31, 1, 0,
            3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 255, 196, 0,
            181, 17, 0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 119, 0, 1, 2, 3, 17, 4, 5, 33, 49, 6, 18,
            65, 81, 7, 97, 113, 19, 34, 50, 129, 8, 20, 66, 145, 161, 177, 193, 9, 35, 51, 82, 240, 21, 98,
            114, 209, 10, 22, 36, 52, 225, 37, 241, 23, 24, 25, 26, 38, 39, 40, 41, 42, 53, 54, 55, 56, 57,
            58, 67, 68, 69, 70, 71, 72, 73, 74, 83, 84, 85, 86, 87, 88, 89, 90, 99, 100, 101, 102, 103,
            104, 105, 106, 115, 116, 117, 118, 119, 120, 121, 122, 130, 131, 132, 133, 134, 135, 136, 137,
            138, 146, 147, 148, 149, 150, 151, 152, 153, 154, 162, 163, 164, 165, 166, 167, 168, 169, 170,
            178, 179, 180, 181, 182, 183, 184, 185, 186, 194, 195, 196, 197, 198, 199, 200, 201, 202, 210,
            211, 212, 213, 214, 215, 216, 217, 218, 226, 227, 228, 229, 230, 231, 232, 233, 234, 242, 243,
            244, 245, 246, 247, 248, 249, 250, 255, 218, 0, 12, 3, 1, 0, 2, 17, 3, 17, 0, 63, 0, 226, 232,
            162, 138, 249, 147, 247, 19, 255, 217,
        ];
        assert!(write_image("image/jpeg", TINY_JPEG).is_ok());
    }
}
