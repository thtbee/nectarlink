// SPDX-License-Identifier: GPL-3.0-or-later
//! Image conversions for the clipboard, with WIC: what Windows apps copy
//! (PNG, or a device-independent bitmap) to PNG for sending, and a PNG or
//! JPEG from a phone to pixels for pasting.

use windows::{
    Win32::{
        Graphics::Imaging::{
            CLSID_WICImagingFactory, GUID_ContainerFormatPng, GUID_WICPixelFormat32bppBGRA,
            IWICImagingFactory, WICBitmapDitherTypeNone, WICBitmapEncoderNoCache, WICBitmapPaletteTypeCustom,
            WICDecodeMetadataCacheOnDemand,
        },
        System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, STATFLAG_NONAME, STREAM_SEEK_SET},
        UI::Shell::SHCreateMemStream,
    },
    core::{Error, HRESULT, Result},
};

use super::with_com;

/// Straight-alpha BGRA pixels, top row first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

fn invalid(message: &str) -> Error {
    // E_INVALIDARG
    Error::new(HRESULT(0x8007_0057_u32 as i32), message)
}

fn factory() -> Result<IWICImagingFactory> {
    // SAFETY: creating the WIC factory; COM is initialized by the caller.
    unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }
}

/// Decodes an image file in memory (PNG, JPEG, BMP and the other formats
/// WIC knows).
pub fn decode(bytes: &[u8]) -> Result<Bitmap> {
    with_com(|| {
        // SAFETY: WIC calls on live objects; the memory stream reads from
        // `bytes`, which outlives the decoder, and the pixel buffer is sized
        // for the converter's stride and height.
        unsafe {
            let factory = factory()?;
            let stream = factory.CreateStream()?;
            stream.InitializeFromMemory(bytes)?;
            let decoder =
                factory.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)?;
            let frame = decoder.GetFrame(0)?;
            let converter = factory.CreateFormatConverter()?;
            converter.Initialize(
                &frame,
                &GUID_WICPixelFormat32bppBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )?;
            let (mut width, mut height) = (0, 0);
            converter.GetSize(&mut width, &mut height)?;
            let len = (width as usize).checked_mul(height as usize).and_then(|n| n.checked_mul(4));
            let mut bgra = vec![0u8; len.ok_or_else(|| invalid("image too large"))?];
            converter.CopyPixels(std::ptr::null(), width * 4, &mut bgra)?;
            Ok(Bitmap { width, height, bgra })
        }
    })
}

/// Encodes pixels as a PNG file.
pub fn encode_png(bitmap: &Bitmap) -> Result<Vec<u8>> {
    with_com(|| {
        // SAFETY: as in `decode`; the bitmap is created from (a copy of)
        // the pixels, and the memory stream is read back within its size.
        unsafe {
            let factory = factory()?;
            let source = factory.CreateBitmapFromMemory(
                bitmap.width,
                bitmap.height,
                &GUID_WICPixelFormat32bppBGRA,
                bitmap.width * 4,
                &bitmap.bgra,
            )?;
            let stream = SHCreateMemStream(None).ok_or_else(|| invalid("no memory stream"))?;
            let encoder = factory.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
            encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
            let mut frame = None;
            encoder.CreateNewFrame(&mut frame, std::ptr::null_mut())?;
            let frame = frame.ok_or_else(|| invalid("no frame"))?;
            frame.Initialize(None)?;
            frame.WriteSource(&source, std::ptr::null())?;
            frame.Commit()?;
            encoder.Commit()?;

            let mut stat = Default::default();
            stream.Stat(&mut stat, STATFLAG_NONAME)?;
            let mut png = vec![0u8; usize::try_from(stat.cbSize).map_err(|_| invalid("too large"))?];
            stream.Seek(0, STREAM_SEEK_SET, None)?;
            let mut read = 0;
            stream
                .Read(png.as_mut_ptr().cast(), u32::try_from(png.len()).unwrap_or(u32::MAX), Some(&mut read))
                .ok()?;
            png.truncate(read as usize);
            Ok(png)
        }
    })
}

/// Scaled down (never up) so neither side is longer than `max`, averaging
/// the pixels each new one covers.
pub fn scale_to(bitmap: &Bitmap, max: u32) -> Bitmap {
    let longest = bitmap.width.max(bitmap.height);
    if longest <= max || longest == 0 {
        return bitmap.clone();
    }
    let scale = f64::from(max) / f64::from(longest);
    let width = ((f64::from(bitmap.width) * scale).round() as u32).max(1);
    let height = ((f64::from(bitmap.height) * scale).round() as u32).max(1);
    let mut bgra = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        let (y0, y1) = (
            y * bitmap.height / height,
            ((y + 1) * bitmap.height / height).max(y * bitmap.height / height + 1),
        );
        for x in 0..width {
            let (x0, x1) = (
                x * bitmap.width / width,
                ((x + 1) * bitmap.width / width).max(x * bitmap.width / width + 1),
            );
            let mut sum = [0u32; 4];
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let i = ((sy * bitmap.width + sx) * 4) as usize;
                    for (total, &value) in sum.iter_mut().zip(&bitmap.bgra[i..i + 4]) {
                        *total += u32::from(value);
                    }
                }
            }
            let n = (y1 - y0) * (x1 - x0);
            bgra.extend(sum.map(|s| (s / n) as u8));
        }
    }
    Bitmap { width, height, bgra }
}

/// A device-independent bitmap (`CF_DIB` / `CF_DIBV5`) as a BMP file, which
/// WIC can decode: the DIB with a file header in front.
pub fn bmp_from_dib(dib: &[u8]) -> Option<Vec<u8>> {
    let u32_at = |at: usize| dib.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let header = u32_at(0)?;
    let bit_count = dib.get(14..16).map(|b| u16::from_le_bytes([b[0], b[1]]))?;
    let compression = u32_at(16)?;
    let colors_used = u32_at(32)?;
    if !(40..=124).contains(&header) {
        return None;
    }
    // Color masks follow a plain BITMAPINFOHEADER (later headers hold them).
    let masks = match (header, compression) {
        (40, 3) => 12, // BI_BITFIELDS
        (40, 6) => 16, // BI_ALPHABITFIELDS
        _ => 0,
    };
    let palette = match colors_used {
        0 if bit_count <= 8 => 4u32 << bit_count,
        n => n.checked_mul(4)?,
    };
    let offset = 14 + header.checked_add(masks)?.checked_add(palette)?;
    let size = u32::try_from(14 + dib.len()).ok()?;
    if offset > size {
        return None;
    }
    let mut bmp = Vec::with_capacity(size as usize);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&size.to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&offset.to_le_bytes());
    bmp.extend_from_slice(dib);
    Some(bmp)
}

/// Pixels as a `CF_DIBV5` block: a BITMAPV5HEADER (32-bit, straight alpha,
/// sRGB) and the rows bottom-up, which is what apps read most reliably.
pub fn dibv5(bitmap: &Bitmap) -> Vec<u8> {
    let (width, height) = (bitmap.width, bitmap.height);
    let mut dib = Vec::with_capacity(124 + bitmap.bgra.len());
    let mut put = |bytes: &[u8]| dib.extend_from_slice(bytes);
    put(&124u32.to_le_bytes());
    put(&(width as i32).to_le_bytes());
    put(&(height as i32).to_le_bytes());
    put(&1u16.to_le_bytes());
    put(&32u16.to_le_bytes());
    put(&3u32.to_le_bytes()); // BI_BITFIELDS
    put(&(width * height * 4).to_le_bytes());
    put(&2835i32.to_le_bytes()); // 72 dpi
    put(&2835i32.to_le_bytes());
    put(&[0; 8]); // colors used, important
    for mask in [0x00FF_0000u32, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000] {
        put(&mask.to_le_bytes());
    }
    put(&0x7352_4742u32.to_le_bytes()); // LCS_sRGB
    put(&[0; 36 + 12]); // endpoints, gamma
    put(&4u32.to_le_bytes()); // LCS_GM_IMAGES
    put(&[0; 12]); // profile data, size, reserved
    let row = width as usize * 4;
    for line in bitmap.bgra.chunks_exact(row.max(1)).rev() {
        dib.extend_from_slice(line);
    }
    dib
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checker(width: u32, height: u32) -> Bitmap {
        let bgra = (0..width * height)
            .flat_map(|i| {
                let (x, y) = (i % width, i / width);
                if (x + y) % 2 == 0 { [255, 0, 0, 255] } else { [0, 0, 255, 128] }
            })
            .collect();
        Bitmap { width, height, bgra }
    }

    #[test]
    fn png_round_trips_with_alpha() {
        let bitmap = checker(5, 3);
        let png = encode_png(&bitmap).unwrap();
        assert_eq!(&png[1..4], b"PNG");
        assert_eq!(decode(&png).unwrap(), bitmap);
    }

    #[test]
    fn large_pictures_scale_down_evenly() {
        let big = Bitmap { width: 400, height: 200, bgra: [10, 20, 30, 255].repeat(400 * 200) };
        let small = scale_to(&big, 100);
        assert_eq!((small.width, small.height), (100, 50));
        assert_eq!(&small.bgra[..4], &[10, 20, 30, 255]);
        assert_eq!(scale_to(&small, 100), small, "never scaled up");
        // A checkerboard averages to grey.
        let checker = checker(4, 4);
        let half = scale_to(&checker, 2);
        assert_eq!(half.bgra.len(), 2 * 2 * 4);
    }

    #[test]
    fn copied_bitmaps_decode() {
        let bitmap = checker(7, 4);
        let dib = dibv5(&bitmap);
        assert_eq!(dib.len(), 124 + 7 * 4 * 4);
        let decoded = decode(&bmp_from_dib(&dib).unwrap()).unwrap();
        assert_eq!((decoded.width, decoded.height), (7, 4));
        // Top-left stays top-left (rows are stored bottom-up).
        assert_eq!(&decoded.bgra[..4], &[255, 0, 0, 255]);

        // A plain 24-bit BITMAPINFOHEADER DIB, as older apps copy.
        let mut old = Vec::new();
        for field in [40u32, 2, 2] {
            old.extend_from_slice(&field.to_le_bytes());
        }
        old.extend_from_slice(&1u16.to_le_bytes());
        old.extend_from_slice(&24u16.to_le_bytes());
        old.extend_from_slice(&[0; 24]);
        // Bottom row: green, green; top row: white, black (rows padded to 4).
        old.extend_from_slice(&[0, 255, 0, 0, 255, 0, 0, 0]);
        old.extend_from_slice(&[255, 255, 255, 0, 0, 0, 0, 0]);
        let decoded = decode(&bmp_from_dib(&old).unwrap()).unwrap();
        assert_eq!(&decoded.bgra[..4], &[255, 255, 255, 255]);
        assert_eq!(&decoded.bgra[8..12], &[0, 255, 0, 255]);

        assert_eq!(bmp_from_dib(&[1, 2, 3]), None);
    }
}
