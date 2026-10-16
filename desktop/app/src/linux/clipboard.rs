use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;

use super::image::{self, Bitmap};

static LAST_CLIP: Mutex<Clip> = Mutex::new(Clip::Empty);
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
                let bmp =
                    image::bmp_from_dib(dib).ok_or_else(|| "invalid DIB header".to_string())?;
                let decoded = image::decode(&bmp).map_err(|e| e.to_string())?;
                image::encode_png(&decoded).map_err(|e| e.to_string())
            }
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
    Image(Image),
    Private,
    Empty,
}

pub fn start<F>(_on_copy: F)
where
    F: Fn(Clip) + Send + Sync + 'static,
{
}

pub fn read() -> Clip {
    if let Ok(guard) = LAST_CLIP.lock()
        && !matches!(*guard, Clip::Empty)
    {
        return guard.clone();
    }
    if let Ok(out) = Command::new("wl-paste").arg("--no-newline").output()
        && out.status.success()
    {
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        if !text.is_empty() {
            return Clip::Text {
                text,
                html: None,
                rtf: None,
            };
        }
    }
    if let Ok(out) = Command::new("xclip")
        .args(["-selection", "clipboard", "-o"])
        .output()
        && out.status.success()
    {
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        if !text.is_empty() {
            return Clip::Text {
                text,
                html: None,
                rtf: None,
            };
        }
    }
    Clip::Empty
}

pub fn read_for_paste() -> Clip {
    read()
}

pub fn read_image_png() -> Option<Vec<u8>> {
    match read() {
        Clip::Image(img) => img.to_png().ok(),
        _ => None,
    }
}

pub fn write(text: &str) -> Result<(), String> {
    write_rich(text, None, None)
}

pub fn write_rich(text: &str, html: Option<&str>, rtf: Option<&str>) -> Result<(), String> {
    if let Ok(mut guard) = LAST_CLIP.lock() {
        *guard = Clip::Text {
            text: text.to_string(),
            html: html.map(str::to_string),
            rtf: rtf.map(str::to_string),
        };
    }
    let _ = pipe_to_cmd("wl-copy", &[], text.as_bytes()).or_else(|_| {
        pipe_to_cmd("xclip", &["-selection", "clipboard"], text.as_bytes())
    });
    Ok(())
}

pub fn write_private(text: &str) -> Result<(), String> {
    write(text)?;
    if let Ok(mut guard) = LAST_CLIP.lock() {
        *guard = Clip::Private;
    }
    Ok(())
}

pub fn write_image(_mime: &str, bytes: &[u8]) -> Result<(), String> {
    let bitmap: Bitmap = image::decode(bytes).map_err(|e| e.to_string())?;
    let png = image::encode_png(&bitmap).map_err(|e| e.to_string())?;
    if let Ok(mut guard) = LAST_CLIP.lock() {
        *guard = Clip::Image(Image::Png(png.clone()));
    }
    let _ = pipe_to_cmd("wl-copy", &["--type", "image/png"], &png).or_else(|_| {
        pipe_to_cmd(
            "xclip",
            &["-selection", "clipboard", "-t", "image/png"],
            &png,
        )
    });
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
    let _ = child.wait()?;
    Ok(())
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
    LAST_FILES.lock().ok()?.clone()
}

pub fn write_files(paths: &[PathBuf]) -> bool {
    if paths.is_empty() {
        return false;
    }
    if let Ok(mut guard) = LAST_FILES.lock() {
        *guard = Some(paths.to_vec());
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
    if s.starts_with("{\\rtf") {
        Some(s.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
