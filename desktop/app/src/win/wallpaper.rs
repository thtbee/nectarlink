// SPDX-License-Identifier: GPL-3.0-or-later
//! The desktop wallpaper's seed color, for Bloom's wallpaper colors: the
//! image is decoded and shrunk with WIC, and its colors are scored like
//! Android scores a phone's wallpaper. A solid-color background is its own
//! seed. When neither gives a usable color (a grey photo, an image Windows
//! can't decode), the Windows accent color is used.

use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::SystemTime,
};

use material_colors::color::Rgb;
use windows::{
    Win32::{
        Foundation::GENERIC_READ,
        Graphics::{
            Gdi::{COLOR_DESKTOP, GetSysColor},
            Imaging::{
                CLSID_WICImagingFactory, GUID_WICPixelFormat32bppBGR, IWICImagingFactory,
                WICBitmapDitherTypeNone, WICBitmapInterpolationModeFant, WICBitmapPaletteTypeCustom,
                WICDecodeMetadataCacheOnDemand,
            },
        },
        System::{
            Com::{
                CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
            },
            Registry::HKEY_CURRENT_USER,
        },
        UI::WindowsAndMessaging::{
            SPI_GETDESKWALLPAPER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
        },
    },
    core::HSTRING,
};

use crate::palette;

/// Wallpapers are shrunk to fit this square before their colors are
/// counted: plenty for picking a seed, and fast.
const SAMPLE_SIZE: u32 = 128;

/// Where the wallpaper came from, so an unchanged one isn't decoded again
/// (Windows announces setting changes far more often than wallpapers
/// change).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Source {
    Image { path: PathBuf, modified: Option<SystemTime>, len: u64 },
    Solid(u32),
}

static LAST: Mutex<Option<(Source, Option<Rgb>)>> = Mutex::new(None);

/// The seed for the current wallpaper. Decoding takes up to a few hundred
/// milliseconds for a large photo, so call this off the UI thread.
pub fn seed() -> Option<Rgb> {
    let source = current_source();
    let wallpaper = match &source {
        Some(source) => {
            let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
            match last.as_ref() {
                Some((previous, seed)) if previous == source => *seed,
                _ => {
                    let seed = seed_of(source);
                    *last = Some((source.clone(), seed));
                    seed
                }
            }
        }
        None => None,
    };
    wallpaper.or_else(accent_color)
}

fn current_source() -> Option<Source> {
    let path = wallpaper_path();
    if path.as_os_str().is_empty() {
        // No picture: a solid color. COLORREF is 0x00BBGGRR.
        // SAFETY: GetSysColor has no preconditions.
        return Some(Source::Solid(unsafe { GetSysColor(COLOR_DESKTOP) }));
    }
    // Windows keeps its own copy of the picture it shows, which outlives the
    // original being moved or deleted.
    let transcoded = dirs::config_dir().map(|d| d.join(r"Microsoft\Windows\Themes\TranscodedWallpaper"));
    [Some(path), transcoded].into_iter().flatten().find_map(|path| {
        let meta = std::fs::metadata(&path).ok().filter(std::fs::Metadata::is_file)?;
        Some(Source::Image { path, modified: meta.modified().ok(), len: meta.len() })
    })
}

fn seed_of(source: &Source) -> Option<Rgb> {
    match source {
        Source::Solid(colorref) => {
            let [r, g, b, _] = colorref.to_le_bytes();
            palette::seed_from_pixels(&[Rgb::new(r, g, b)])
        }
        Source::Image { path, .. } => match sample(path) {
            Ok(pixels) => palette::seed_from_pixels(&pixels),
            Err(e) => {
                tracing::debug!(error = %e, path = %path.display(), "can't read the wallpaper");
                None
            }
        },
    }
}

fn wallpaper_path() -> PathBuf {
    let mut buf = [0u16; 1024];
    // SAFETY: SPI_GETDESKWALLPAPER writes at most `uiParam` UTF-16 units,
    // including the terminator, to the buffer.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETDESKWALLPAPER,
            buf.len() as u32,
            Some(buf.as_mut_ptr().cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .is_ok();
    if !ok {
        return PathBuf::new();
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    PathBuf::from(String::from_utf16_lossy(&buf[..len]))
}

/// The Windows accent color (Settings > Personalization > Colors), which
/// Windows itself can pick from the wallpaper.
fn accent_color() -> Option<Rgb> {
    // 0xAABBGGRR.
    let value = super::reg_dword(HKEY_CURRENT_USER, r"Software\Microsoft\Windows\DWM", "AccentColor")?;
    let [r, g, b, _] = value.to_le_bytes();
    Some(Rgb::new(r, g, b))
}

/// Initializes COM on this thread for as long as it lives.
struct Com;

impl Com {
    fn init() -> windows::core::Result<Com> {
        // SAFETY: balanced by CoUninitialize in Drop, on the same thread.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok()?;
        Ok(Com)
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        // SAFETY: paired with the successful CoInitializeEx in `init`.
        unsafe { CoUninitialize() };
    }
}

/// Decodes the image and shrinks it to fit `SAMPLE_SIZE`, keeping its
/// aspect ratio.
fn sample(path: &Path) -> windows::core::Result<Vec<Rgb>> {
    let _com = Com::init()?;
    // SAFETY: plain WIC calls on live interfaces; the pixel buffer is sized
    // for the width, height and stride passed to CopyPixels.
    unsafe {
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let decoder = factory.CreateDecoderFromFilename(
            &HSTRING::from(path.as_os_str()),
            None,
            GENERIC_READ,
            WICDecodeMetadataCacheOnDemand,
        )?;
        let frame = decoder.GetFrame(0)?;
        let (mut width, mut height) = (0, 0);
        frame.GetSize(&mut width, &mut height)?;
        if width == 0 || height == 0 {
            return Ok(Vec::new());
        }
        let scale = (f64::from(SAMPLE_SIZE) / f64::from(width.max(height))).min(1.0);
        let (w, h) = (
            ((f64::from(width) * scale).round() as u32).max(1),
            ((f64::from(height) * scale).round() as u32).max(1),
        );

        let scaler = factory.CreateBitmapScaler()?;
        scaler.Initialize(&frame, w, h, WICBitmapInterpolationModeFant)?;
        let converter = factory.CreateFormatConverter()?;
        converter.Initialize(
            &scaler,
            &GUID_WICPixelFormat32bppBGR,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )?;
        let stride = w * 4;
        let mut bgrx = vec![0u8; (stride * h) as usize];
        converter.CopyPixels(std::ptr::null(), stride, &mut bgrx)?;
        Ok(bgrx.as_chunks::<4>().0.iter().map(|&[b, g, r, _]| Rgb::new(r, g, b)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes a 64x32 24-bit BMP: left half `left`, right half `right`.
    fn write_bmp(path: &Path, left: [u8; 3], right: [u8; 3]) {
        let (w, h) = (64u32, 32u32);
        let row = w * 3; // already a multiple of 4
        let mut data = Vec::new();
        data.extend_from_slice(b"BM");
        data.extend_from_slice(&(54 + row * h).to_le_bytes());
        data.extend_from_slice(&[0; 4]);
        data.extend_from_slice(&54u32.to_le_bytes());
        data.extend_from_slice(&40u32.to_le_bytes());
        data.extend_from_slice(&w.to_le_bytes());
        data.extend_from_slice(&h.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&24u16.to_le_bytes());
        data.extend_from_slice(&[0; 24]);
        for _ in 0..h {
            for x in 0..w {
                let [r, g, b] = if x < w / 2 { left } else { right };
                data.extend_from_slice(&[b, g, r]);
            }
        }
        std::fs::write(path, data).unwrap();
    }

    #[test]
    fn samples_an_image_with_wic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wall.bmp");
        write_bmp(&path, [0xC0, 0x30, 0x30], [0x20, 0x40, 0xC0]);
        let pixels = sample(&path).unwrap();
        assert_eq!(pixels.len(), 64 * 32, "small images aren't scaled up");
        assert_eq!(pixels[0], Rgb::new(0xC0, 0x30, 0x30), "channels come out in RGB order");
        assert_eq!(pixels[63], Rgb::new(0x20, 0x40, 0xC0));
        assert!(sample(&dir.path().join("missing.png")).is_err());
    }

    #[test]
    fn a_solid_background_is_its_own_seed() {
        // Windows' default "solid color" blue.
        let colorref = u32::from_le_bytes([0x00, 0x78, 0xD4, 0]);
        let seed = seed_of(&Source::Solid(colorref)).unwrap();
        assert_eq!(seed, Rgb::new(0x00, 0x78, 0xD4));
        assert_eq!(seed_of(&Source::Solid(0x0080_8080)), None, "grey has no seed");
    }
}
