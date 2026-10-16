use std::io::{Error, ErrorKind, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

const BITMAPFILEHEADER_SIZE: usize = 14;
const BITMAPV5HEADER_SIZE: u32 = 124;
const BI_RGB: u32 = 0;
const BI_BITFIELDS: u32 = 3;
const LCS_SRGB: u32 = 0x7352_4742;

const PNG_SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for &byte in data {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

fn write_png_chunk(out: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(chunk_type);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

pub fn encode_png(bitmap: &Bitmap) -> Result<Vec<u8>> {
    if bitmap.width == 0 || bitmap.height == 0 {
        return Err(Error::new(ErrorKind::InvalidInput, "empty bitmap"));
    }
    let expected = (bitmap.width as usize)
        .checked_mul(bitmap.height as usize)
        .and_then(|px| px.checked_mul(4))
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "bitmap dimensions overflow"))?;
    if bitmap.bgra.len() < expected {
        return Err(Error::new(ErrorKind::InvalidInput, "bitmap buffer too short"));
    }

    let stride = bitmap.width as usize * 4;
    let mut raw = Vec::with_capacity((stride + 1) * bitmap.height as usize);
    for row in 0..bitmap.height as usize {
        raw.push(0u8); // Filter type 0 (None)
        let row_slice = &bitmap.bgra[row * stride..(row + 1) * stride];
        for px in row_slice.chunks_exact(4) {
            raw.push(px[2]); // R
            raw.push(px[1]); // G
            raw.push(px[0]); // B
            raw.push(px[3]); // A
        }
    }

    // Wrap raw scanlines in a valid zlib stream using stored (uncompressed) DEFLATE blocks.
    let mut zlib = Vec::with_capacity(raw.len() + (raw.len() / 65_535 + 1) * 5 + 6);
    zlib.push(0x78);
    zlib.push(0x01);
    let chunks: Vec<&[u8]> = if raw.is_empty() {
        vec![&[][..]]
    } else {
        raw.chunks(65_535).collect()
    };
    for (i, chunk) in chunks.iter().enumerate() {
        let is_last = i + 1 == chunks.len();
        zlib.push(u8::from(is_last));
        let len = chunk.len() as u16;
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&(!len).to_le_bytes());
        zlib.extend_from_slice(chunk);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut out = Vec::with_capacity(zlib.len() + 64);
    out.extend_from_slice(&PNG_SIG);

    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&bitmap.width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&bitmap.height.to_be_bytes());
    ihdr[8] = 8; // bit depth
    ihdr[9] = 6; // color type: RGBA
    ihdr[10] = 0; // compression
    ihdr[11] = 0; // filter
    ihdr[12] = 0; // interlace
    write_png_chunk(&mut out, b"IHDR", &ihdr);
    write_png_chunk(&mut out, b"IDAT", &zlib);
    write_png_chunk(&mut out, b"IEND", &[]);

    Ok(out)
}

fn decode_stored_zlib(zlib: &[u8]) -> Option<Vec<u8>> {
    if zlib.len() < 6 {
        return None;
    }
    let mut pos = 2; // skip 2-byte zlib header
    let end = zlib.len() - 4; // exclude 4-byte adler32
    let mut out = Vec::new();
    loop {
        if pos >= end {
            return None;
        }
        let header = zlib[pos];
        pos += 1;
        let bfinal = (header & 1) != 0;
        let btype = (header >> 1) & 0b11;
        if btype != 0 {
            return None; // compressed block; fall back to external decoder
        }
        if pos + 4 > end {
            return None;
        }
        let len = u16::from_le_bytes([zlib[pos], zlib[pos + 1]]) as usize;
        let nlen = u16::from_le_bytes([zlib[pos + 2], zlib[pos + 3]]);
        if (len as u16) != !nlen {
            return None;
        }
        pos += 4;
        if pos + len > end {
            return None;
        }
        out.extend_from_slice(&zlib[pos..pos + len]);
        pos += len;
        if bfinal {
            break;
        }
    }
    Some(out)
}

fn decode_stored_png(bytes: &[u8]) -> Option<Bitmap> {
    if bytes.len() < 8 || bytes[..8] != PNG_SIG {
        return None;
    }
    let mut pos = 8;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut color_type = 0u8;
    let mut idat = Vec::new();
    while pos + 8 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[pos..pos + 4].try_into().ok()?) as usize;
        let ctype = &bytes[pos + 4..pos + 8];
        pos += 8;
        if pos + len + 4 > bytes.len() {
            return None;
        }
        let data = &bytes[pos..pos + len];
        pos += len + 4; // skip CRC
        match ctype {
            b"IHDR" if data.len() >= 13 => {
                width = u32::from_be_bytes(data[0..4].try_into().ok()?);
                height = u32::from_be_bytes(data[4..8].try_into().ok()?);
                let bit_depth = data[8];
                color_type = data[9];
                if bit_depth != 8 || (color_type != 6 && color_type != 2) {
                    return None;
                }
            }
            b"IDAT" => idat.extend_from_slice(data),
            b"IEND" => break,
            _ => {}
        }
    }
    if width == 0 || height == 0 || idat.is_empty() {
        return None;
    }
    let raw = decode_stored_zlib(&idat)?;
    let bpp = if color_type == 6 { 4 } else { 3 };
    let stride = width as usize * bpp;
    if raw.len() < (stride + 1) * height as usize {
        return None;
    }
    let mut bgra = Vec::with_capacity(width as usize * height as usize * 4);
    for row in 0..height as usize {
        let row_start = row * (stride + 1);
        if raw[row_start] != 0 {
            return None;
        }
        let row_bytes = &raw[row_start + 1..row_start + 1 + stride];
        for px in row_bytes.chunks_exact(bpp) {
            bgra.push(px[2]); // B
            bgra.push(px[1]); // G
            bgra.push(px[0]); // R
            bgra.push(if bpp == 4 { px[3] } else { 255 });
        }
    }
    Some(Bitmap {
        width,
        height,
        bgra,
    })
}

fn decode_bmp(bytes: &[u8]) -> Option<Bitmap> {
    if bytes.len() < 54 || &bytes[0..2] != b"BM" {
        return None;
    }
    let pixel_offset = u32::from_le_bytes(bytes[10..14].try_into().ok()?) as usize;
    let header_size = u32::from_le_bytes(bytes[14..18].try_into().ok()?) as usize;
    if header_size < 40 || bytes.len() < 14 + header_size {
        return None;
    }
    let width = i32::from_le_bytes(bytes[18..22].try_into().ok()?);
    let height_raw = i32::from_le_bytes(bytes[22..26].try_into().ok()?);
    let bpp = u16::from_le_bytes(bytes[28..30].try_into().ok()?);
    if width <= 0 || height_raw == 0 || (bpp != 24 && bpp != 32) {
        return None;
    }
    let top_down = height_raw < 0;
    let width_u = width as u32;
    let height_u = height_raw.unsigned_abs();
    let bytes_per_px = (bpp / 8) as usize;
    let row_stride = (width_u as usize * bytes_per_px).div_ceil(4) * 4;
    if pixel_offset + row_stride * height_u as usize > bytes.len() {
        return None;
    }
    let mut bgra = vec![0u8; width_u as usize * height_u as usize * 4];
    for y in 0..height_u as usize {
        let src_row = if top_down {
            y
        } else {
            height_u as usize - 1 - y
        };
        let src = &bytes[pixel_offset + src_row * row_stride..];
        let dst = &mut bgra[y * width_u as usize * 4..(y + 1) * width_u as usize * 4];
        for x in 0..width_u as usize {
            let s = &src[x * bytes_per_px..];
            let d = &mut dst[x * 4..x * 4 + 4];
            d[0] = s[0];
            d[1] = s[1];
            d[2] = s[2];
            d[3] = if bytes_per_px == 4 { s[3] } else { 255 };
        }
    }
    Some(Bitmap {
        width: width_u,
        height: height_u,
        bgra,
    })
}

fn decode_via_python_or_ffmpeg(bytes: &[u8]) -> Option<Bitmap> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            "pipe:0",
            "-frames:v",
            "1",
            "-f",
            "bmp",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(bytes);
    }
    let output = child.wait_with_output().ok()?;
    if output.status.success() {
        decode_bmp(&output.stdout)
    } else {
        None
    }
}

pub fn decode(bytes: &[u8]) -> Result<Bitmap> {
    if bytes.is_empty() {
        return Err(Error::new(ErrorKind::InvalidInput, "empty image"));
    }
    if let Some(bmp) = decode_stored_png(bytes) {
        return Ok(bmp);
    }
    if let Some(bmp) = decode_bmp(bytes) {
        return Ok(bmp);
    }
    if let Some(dib_bmp) = bmp_from_dib(bytes).and_then(|b| decode_bmp(&b)) {
        return Ok(dib_bmp);
    }
    if let Some(bmp) = decode_via_python_or_ffmpeg(bytes) {
        return Ok(bmp);
    }
    Err(Error::new(
        ErrorKind::InvalidData,
        "unsupported or corrupt image data",
    ))
}

pub fn scale_to(bitmap: &Bitmap, max: u32) -> Bitmap {
    let longest = bitmap.width.max(bitmap.height);
    if longest <= max || max == 0 || bitmap.width == 0 || bitmap.height == 0 {
        return bitmap.clone();
    }
    let width = ((u64::from(bitmap.width) * u64::from(max)) / u64::from(longest)).max(1) as u32;
    let height = ((u64::from(bitmap.height) * u64::from(max)) / u64::from(longest)).max(1) as u32;
    let mut bgra = vec![0u8; width as usize * height as usize * 4];
    for y in 0..height {
        let src_y = ((u64::from(y) * u64::from(bitmap.height)) / u64::from(height))
            .min(u64::from(bitmap.height - 1)) as usize;
        for x in 0..width {
            let src_x = ((u64::from(x) * u64::from(bitmap.width)) / u64::from(width))
                .min(u64::from(bitmap.width - 1)) as usize;
            let src_idx = (src_y * bitmap.width as usize + src_x) * 4;
            let dst_idx = (y as usize * width as usize + x as usize) * 4;
            bgra[dst_idx..dst_idx + 4].copy_from_slice(&bitmap.bgra[src_idx..src_idx + 4]);
        }
    }
    Bitmap {
        width,
        height,
        bgra,
    }
}

pub fn bmp_from_dib(dib: &[u8]) -> Option<Vec<u8>> {
    if dib.len() < 40 {
        return None;
    }
    let header_size = u32::from_le_bytes(dib[0..4].try_into().ok()?);
    if header_size < 40 || (header_size as usize) > dib.len() {
        return None;
    }
    let bit_count = u16::from_le_bytes(dib[14..16].try_into().ok()?);
    let compression = u32::from_le_bytes(dib[16..20].try_into().ok()?);
    let colors_used = u32::from_le_bytes(dib[32..36].try_into().ok()?);

    let palette_entries = if colors_used != 0 {
        colors_used
    } else if bit_count <= 8 {
        1u32 << bit_count
    } else {
        0
    };
    let masks_bytes: u32 = if compression == BI_BITFIELDS && header_size == 40 {
        if bit_count == 32 { 16 } else { 12 }
    } else {
        0
    };
    let pixel_offset = (BITMAPFILEHEADER_SIZE as u32)
        .checked_add(header_size)?
        .checked_add(palette_entries.checked_mul(4)?)?
        .checked_add(masks_bytes)?;
    let total_size = (BITMAPFILEHEADER_SIZE as u32).checked_add(u32::try_from(dib.len()).ok()?)?;

    let mut bmp = Vec::with_capacity(BITMAPFILEHEADER_SIZE + dib.len());
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&total_size.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&pixel_offset.to_le_bytes());
    bmp.extend_from_slice(dib);
    Some(bmp)
}

pub fn dibv5(bitmap: &Bitmap) -> Vec<u8> {
    let row_bytes = (bitmap.width as usize) * 4;
    let image_bytes = row_bytes * (bitmap.height as usize);
    let mut out = Vec::with_capacity((BITMAPV5HEADER_SIZE as usize) + image_bytes);

    let neg_height = -i32::try_from(bitmap.height).unwrap_or(i32::MAX);
    out.extend_from_slice(&BITMAPV5HEADER_SIZE.to_le_bytes());
    out.extend_from_slice(&(bitmap.width as i32).to_le_bytes());
    out.extend_from_slice(&neg_height.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&BI_BITFIELDS.to_le_bytes());
    out.extend_from_slice(&(image_bytes as u32).to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0x00FF_0000u32.to_le_bytes());
    out.extend_from_slice(&0x0000_FF00u32.to_le_bytes());
    out.extend_from_slice(&0x0000_00FFu32.to_le_bytes());
    out.extend_from_slice(&0xFF00_0000u32.to_le_bytes());
    out.extend_from_slice(&LCS_SRGB.to_le_bytes());
    out.extend_from_slice(&[0u8; 36]);
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&4u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    debug_assert_eq!(out.len(), BITMAPV5HEADER_SIZE as usize);

    let _ = BI_RGB;
    out.extend_from_slice(&bitmap.bgra[..image_bytes]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_bitmap(width: u32, height: u32) -> Bitmap {
        let mut bgra = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                bgra.push((x * 17) as u8);
                bgra.push((y * 31) as u8);
                bgra.push(((x + y) * 13) as u8);
                bgra.push(0xFF);
            }
        }
        Bitmap {
            width,
            height,
            bgra,
        }
    }

    #[test]
    fn png_round_trip_preserves_dimensions_and_pixels() {
        let original = sample_bitmap(16, 12);
        let png = encode_png(&original).expect("encode_png");
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));

        let decoded = decode(&png).expect("decode png");
        assert_eq!(decoded, original);
    }

    #[test]
    fn dibv5_through_bmp_round_trips_pixels() {
        let original = sample_bitmap(8, 6);
        let dib = dibv5(&original);
        let bmp = bmp_from_dib(&dib).expect("bmp_from_dib");
        let decoded = decode(&bmp).expect("decode bmp");
        assert_eq!(decoded, original);
    }

    #[test]
    fn scale_to_shrinks_oversized_and_keeps_small() {
        let small = sample_bitmap(32, 24);
        assert_eq!(scale_to(&small, 64), small);

        let large = sample_bitmap(200, 100);
        let scaled = scale_to(&large, 50);
        assert_eq!(scaled.width, 50);
        assert_eq!(scaled.height, 25);
    }

    #[test]
    fn decode_rejects_empty_and_garbage_input() {
        assert!(decode(&[]).is_err());
        assert!(decode(b"not an image").is_err());
        assert!(bmp_from_dib(&[0u8; 12]).is_none());
    }
}
