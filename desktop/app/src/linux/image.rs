// SPDX-License-Identifier: GPL-3.0-or-later
//! Image decoding, scaling, square cropping, and PNG/JPEG/DIB encoding on Linux.

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

const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7,
    14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39,
    46, 53, 60, 61, 54, 47, 55, 62, 63,
];

const STD_LUMA_QUANT: [u8; 64] = [
    16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69, 56, 14, 17,
    22, 29, 51, 87, 80, 62, 18, 22, 37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81, 104, 113, 92, 49, 64, 78,
    87, 103, 121, 120, 101, 72, 92, 95, 98, 112, 100, 103, 99,
];

const STD_CHROMA_QUANT: [u8; 64] = [
    17, 18, 24, 47, 99, 99, 99, 99, 18, 21, 26, 66, 99, 99, 99, 99, 24, 26, 56, 99, 99, 99, 99, 99, 47, 66,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
];

const BITS_DC_LUMA: [u8; 16] = [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0];
const VAL_DC_LUMA: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];

const BITS_DC_CHROMA: [u8; 16] = [0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0];
const VAL_DC_CHROMA: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];

const BITS_AC_LUMA: [u8; 16] = [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7d];
const VAL_AC_LUMA: [u8; 162] = [
    0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07, 0x22,
    0x71, 0x14, 0x32, 0x81, 0x91, 0xa1, 0x08, 0x23, 0x42, 0xb1, 0xc1, 0x15, 0x52, 0xd1, 0xf0, 0x24, 0x33,
    0x62, 0x72, 0x82, 0x09, 0x0a, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x34,
    0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x53, 0x54, 0x55,
    0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x73, 0x74, 0x75, 0x76,
    0x77, 0x78, 0x79, 0x7a, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x92, 0x93, 0x94, 0x95, 0x96,
    0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5,
    0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4,
    0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe1, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf1,
    0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa,
];

const BITS_AC_CHROMA: [u8; 16] = [0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 0x77];
const VAL_AC_CHROMA: [u8; 162] = [
    0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71, 0x13,
    0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xa1, 0xb1, 0xc1, 0x09, 0x23, 0x33, 0x52, 0xf0, 0x15, 0x62,
    0x72, 0xd1, 0x0a, 0x16, 0x24, 0x34, 0xe1, 0x25, 0xf1, 0x17, 0x18, 0x19, 0x1a, 0x26, 0x27, 0x28, 0x29,
    0x2a, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x53, 0x54,
    0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x73, 0x74, 0x75,
    0x76, 0x77, 0x78, 0x79, 0x7a, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x92, 0x93, 0x94,
    0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3,
    0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2,
    0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea,
    0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa,
];

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
        for px in row_slice.as_chunks::<4>().0 {
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
    let chunks: Vec<&[u8]> = if raw.is_empty() { vec![&[][..]] } else { raw.chunks(65_535).collect() };
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

fn scale_quant_table(base: &[u8; 64], quality: u8) -> [u8; 64] {
    let q = i32::from(quality.clamp(1, 100));
    let scale = if q < 50 { 5000 / q } else { 200 - q * 2 };
    let mut out = [0u8; 64];
    for (i, &b) in base.iter().enumerate() {
        let v = (i32::from(b) * scale + 50) / 100;
        out[ZIGZAG[i]] = v.clamp(1, 255) as u8;
    }
    out
}

fn build_huff_encode_table(bits: &[u8; 16], vals: &[u8]) -> [(u16, u8); 256] {
    let mut table = [(0u16, 0u8); 256];
    let mut code: u16 = 0;
    let mut idx = 0usize;
    for (len_minus_1, &count) in bits.iter().enumerate() {
        let bit_len = (len_minus_1 + 1) as u8;
        for _ in 0..count {
            let sym = vals[idx] as usize;
            table[sym] = (code, bit_len);
            code = code.wrapping_add(1);
            idx += 1;
        }
        code <<= 1;
    }
    table
}

struct JpegBitWriter {
    out: Vec<u8>,
    bit_buf: u32,
    bit_count: u8,
}

impl JpegBitWriter {
    fn new(capacity: usize) -> Self {
        Self { out: Vec::with_capacity(capacity), bit_buf: 0, bit_count: 0 }
    }

    fn write_bits(&mut self, bits: u16, len: u8) {
        if len == 0 {
            return;
        }
        let mask = (1u32 << len) - 1;
        self.bit_buf = (self.bit_buf << len) | (u32::from(bits) & mask);
        self.bit_count += len;
        while self.bit_count >= 8 {
            self.bit_count -= 8;
            let byte = ((self.bit_buf >> self.bit_count) & 0xFF) as u8;
            self.out.push(byte);
            if byte == 0xFF {
                self.out.push(0x00);
            }
        }
    }

    fn flush(&mut self) {
        if self.bit_count > 0 {
            let pad_len = 8 - self.bit_count;
            self.write_bits((1u16 << pad_len) - 1, pad_len);
        }
    }
}

fn magnitude_category(val: i32) -> (u8, u16) {
    let abs_v = val.unsigned_abs();
    if abs_v == 0 {
        return (0, 0);
    }
    let cat = (32 - abs_v.leading_zeros()) as u8;
    let mask = (1u32 << cat) - 1;
    let bits = if val > 0 { abs_v & mask } else { (abs_v ^ mask) & mask };
    (cat, bits as u16)
}

fn fdct_block(block: &[f32; 64], quant_zigzag: &[u8; 64]) -> [i32; 64] {
    let mut tmp = [0.0f32; 64];
    let mut cos_table = [[0.0f32; 8]; 8];
    for (u, row) in cos_table.iter_mut().enumerate() {
        for (x, cell) in row.iter_mut().enumerate() {
            *cell = (((2 * x + 1) * u) as f32 * std::f32::consts::PI / 16.0).cos();
        }
    }
    for y in 0..8 {
        for u in 0..8 {
            let cu = if u == 0 { std::f32::consts::FRAC_1_SQRT_2 } else { 1.0 };
            let mut sum = 0.0f32;
            for x in 0..8 {
                sum += block[y * 8 + x] * cos_table[u][x];
            }
            tmp[y * 8 + u] = 0.5 * cu * sum;
        }
    }
    let mut out_zz = [0i32; 64];
    for (v, cos_v) in cos_table.iter().enumerate() {
        let cv = if v == 0 { std::f32::consts::FRAC_1_SQRT_2 } else { 1.0 };
        for u in 0..8 {
            let mut sum = 0.0f32;
            for (y, &cos_vy) in cos_v.iter().enumerate() {
                sum += tmp[y * 8 + u] * cos_vy;
            }
            let coeff = 0.5 * cv * sum;
            let idx = v * 8 + u;
            let zz = ZIGZAG[idx];
            let q = f32::from(quant_zigzag[zz]);
            out_zz[zz] = (coeff / q).round() as i32;
        }
    }
    out_zz
}

fn encode_jpeg_block(
    bw: &mut JpegBitWriter,
    zz: &[i32; 64],
    prev_dc: i32,
    dc_table: &[(u16, u8); 256],
    ac_table: &[(u16, u8); 256],
) -> i32 {
    let dc = zz[0];
    let diff = dc - prev_dc;
    let (cat, bits) = magnitude_category(diff);
    let (hcode, hlen) = dc_table[cat as usize];
    bw.write_bits(hcode, hlen);
    bw.write_bits(bits, cat);

    let mut zero_run = 0u8;
    for &ac in &zz[1..] {
        if ac == 0 {
            zero_run += 1;
        } else {
            while zero_run >= 16 {
                let (zcode, zlen) = ac_table[0xF0];
                bw.write_bits(zcode, zlen);
                zero_run -= 16;
            }
            let (acat, abits) = magnitude_category(ac);
            let sym = ((zero_run << 4) | acat) as usize;
            let (acode, alen) = ac_table[sym];
            bw.write_bits(acode, alen);
            bw.write_bits(abits, acat);
            zero_run = 0;
        }
    }
    if zero_run > 0 {
        let (ecode, elen) = ac_table[0x00];
        bw.write_bits(ecode, elen);
    }
    dc
}

/// Encodes `bitmap` into a standard baseline JFIF JPEG stream (`quality` 1..=100).
pub fn encode_jpeg(bitmap: &Bitmap, quality: u8) -> Result<Vec<u8>> {
    if bitmap.width == 0 || bitmap.height == 0 {
        return Err(Error::new(ErrorKind::InvalidInput, "empty bitmap"));
    }
    if bitmap.width > 65_535 || bitmap.height > 65_535 {
        return Err(Error::new(ErrorKind::InvalidInput, "bitmap dimensions exceed JPEG maximum"));
    }
    let expected = (bitmap.width as usize)
        .checked_mul(bitmap.height as usize)
        .and_then(|px| px.checked_mul(4))
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "bitmap dimensions overflow"))?;
    if bitmap.bgra.len() < expected {
        return Err(Error::new(ErrorKind::InvalidInput, "bitmap buffer too short"));
    }

    let luma_q = scale_quant_table(&STD_LUMA_QUANT, quality);
    let chroma_q = scale_quant_table(&STD_CHROMA_QUANT, quality);
    let dc_luma = build_huff_encode_table(&BITS_DC_LUMA, &VAL_DC_LUMA);
    let dc_chroma = build_huff_encode_table(&BITS_DC_CHROMA, &VAL_DC_CHROMA);
    let ac_luma = build_huff_encode_table(&BITS_AC_LUMA, &VAL_AC_LUMA);
    let ac_chroma = build_huff_encode_table(&BITS_AC_CHROMA, &VAL_AC_CHROMA);

    let mut bw = JpegBitWriter::new(expected / 2 + 1024);

    // SOI
    bw.out.extend_from_slice(&[0xFF, 0xD8]);

    // APP0 (JFIF 1.01)
    bw.out.extend_from_slice(&[
        0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0x00, 0x01, 0x01, 0x01, 0x00, 0x48, 0x00, 0x48, 0x00,
        0x00,
    ]);

    // DQT (table 0 = luma, table 1 = chroma)
    bw.out.extend_from_slice(&[0xFF, 0xDB, 0x00, 67, 0x00]);
    bw.out.extend_from_slice(&luma_q);
    bw.out.extend_from_slice(&[0xFF, 0xDB, 0x00, 67, 0x01]);
    bw.out.extend_from_slice(&chroma_q);

    // SOF0 (Baseline DCT, 3 components, 4:4:4 sampling 1x1)
    let w_be = (bitmap.width as u16).to_be_bytes();
    let h_be = (bitmap.height as u16).to_be_bytes();
    bw.out.extend_from_slice(&[
        0xFF, 0xC0, 0x00, 17, 8, h_be[0], h_be[1], w_be[0], w_be[1], 3, 1, 0x11, 0, 2, 0x11, 1, 3, 0x11, 1,
    ]);

    // DHT (4 tables: DC0, AC0, DC1, AC1)
    let write_dht = |out: &mut Vec<u8>, tc_th: u8, bits: &[u8; 16], vals: &[u8]| {
        let len = (2 + 1 + 16 + vals.len()) as u16;
        out.extend_from_slice(&[0xFF, 0xC4]);
        out.extend_from_slice(&len.to_be_bytes());
        out.push(tc_th);
        out.extend_from_slice(bits);
        out.extend_from_slice(vals);
    };
    write_dht(&mut bw.out, 0x00, &BITS_DC_LUMA, &VAL_DC_LUMA);
    write_dht(&mut bw.out, 0x10, &BITS_AC_LUMA, &VAL_AC_LUMA);
    write_dht(&mut bw.out, 0x01, &BITS_DC_CHROMA, &VAL_DC_CHROMA);
    write_dht(&mut bw.out, 0x11, &BITS_AC_CHROMA, &VAL_AC_CHROMA);

    // SOS
    bw.out.extend_from_slice(&[0xFF, 0xDA, 0x00, 12, 3, 1, 0x00, 2, 0x11, 3, 0x11, 0x00, 0x3F, 0x00]);

    let w = bitmap.width as usize;
    let h = bitmap.height as usize;
    let mcu_cols = w.div_ceil(8);
    let mcu_rows = h.div_ceil(8);

    let mut prev_y = 0i32;
    let mut prev_cb = 0i32;
    let mut prev_cr = 0i32;

    for my in 0..mcu_rows {
        for mx in 0..mcu_cols {
            let mut y_blk = [0.0f32; 64];
            let mut cb_blk = [0.0f32; 64];
            let mut cr_blk = [0.0f32; 64];
            for by in 0..8 {
                let py = (my * 8 + by).min(h - 1);
                for bx in 0..8 {
                    let px = (mx * 8 + bx).min(w - 1);
                    let idx = (py * w + px) * 4;
                    let b = f32::from(bitmap.bgra[idx]);
                    let g = f32::from(bitmap.bgra[idx + 1]);
                    let r = f32::from(bitmap.bgra[idx + 2]);
                    let y_val = 0.299 * r + 0.587 * g + 0.114 * b - 128.0;
                    let cb_val = -0.168_736 * r - 0.331_264 * g + 0.5 * b;
                    let cr_val = 0.5 * r - 0.418_688 * g - 0.081_312 * b;
                    let bidx = by * 8 + bx;
                    y_blk[bidx] = y_val;
                    cb_blk[bidx] = cb_val;
                    cr_blk[bidx] = cr_val;
                }
            }
            let y_zz = fdct_block(&y_blk, &luma_q);
            let cb_zz = fdct_block(&cb_blk, &chroma_q);
            let cr_zz = fdct_block(&cr_blk, &chroma_q);
            prev_y = encode_jpeg_block(&mut bw, &y_zz, prev_y, &dc_luma, &ac_luma);
            prev_cb = encode_jpeg_block(&mut bw, &cb_zz, prev_cb, &dc_chroma, &ac_chroma);
            prev_cr = encode_jpeg_block(&mut bw, &cr_zz, prev_cr, &dc_chroma, &ac_chroma);
        }
    }

    bw.flush();
    // EOI
    bw.out.extend_from_slice(&[0xFF, 0xD9]);
    Ok(bw.out)
}

#[derive(Default, Clone)]
struct HuffDecodeTable {
    max_code: [i32; 17],
    val_offset: [i32; 17],
    vals: Vec<u8>,
}

impl HuffDecodeTable {
    fn from_bits_vals(bits: &[u8; 16], vals: &[u8]) -> Self {
        let mut max_code = [-1i32; 17];
        let mut val_offset = [0i32; 17];
        let mut code = 0i32;
        let mut idx = 0i32;
        for i in 1..=16 {
            let count = i32::from(bits[i - 1]);
            if count > 0 {
                val_offset[i] = idx - code;
                idx += count;
                code += count;
                max_code[i] = code - 1;
            } else {
                max_code[i] = -1;
            }
            code <<= 1;
        }
        Self { max_code, val_offset, vals: vals.to_vec() }
    }
}

struct JpegBitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bit_buf: u32,
    bit_count: u8,
}

impl<'a> JpegBitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0, bit_buf: 0, bit_count: 0 }
    }

    fn next_byte(&mut self) -> Option<u8> {
        if self.pos >= self.data.len() {
            return None;
        }
        let b = self.data[self.pos];
        self.pos += 1;
        if b == 0xFF {
            if self.pos < self.data.len() && self.data[self.pos] == 0x00 {
                self.pos += 1;
            } else {
                return None;
            }
        }
        Some(b)
    }

    fn read_bit(&mut self) -> Option<u8> {
        if self.bit_count == 0 {
            let b = self.next_byte()?;
            self.bit_buf = u32::from(b);
            self.bit_count = 8;
        }
        self.bit_count -= 1;
        Some(((self.bit_buf >> self.bit_count) & 1) as u8)
    }

    fn read_bits(&mut self, len: u8) -> Option<u16> {
        let mut v = 0u16;
        for _ in 0..len {
            v = (v << 1) | u16::from(self.read_bit()?);
        }
        Some(v)
    }

    fn decode_huff(&mut self, table: &HuffDecodeTable) -> Option<u8> {
        let mut code = 0i32;
        for len in 1..=16 {
            code = (code << 1) | i32::from(self.read_bit()?);
            if code <= table.max_code[len] {
                let idx = (code + table.val_offset[len]) as usize;
                return table.vals.get(idx).copied();
            }
        }
        None
    }
}

fn extend_magnitude(bits: u16, cat: u8) -> i32 {
    if cat == 0 {
        return 0;
    }
    let half = 1u16 << (cat - 1);
    if bits < half { i32::from(bits) - ((1i32 << cat) - 1) } else { i32::from(bits) }
}

fn idct_block(zz: &[i32; 64], quant_zigzag: &[u8; 64]) -> [f32; 64] {
    let mut dequant = [0.0f32; 64];
    for i in 0..64 {
        let z = ZIGZAG[i];
        dequant[i] = (zz[z] as f32) * f32::from(quant_zigzag[z]);
    }
    let mut cos_table = [[0.0f32; 8]; 8];
    for (u, row) in cos_table.iter_mut().enumerate() {
        for (x, cell) in row.iter_mut().enumerate() {
            *cell = (((2 * x + 1) * u) as f32 * std::f32::consts::PI / 16.0).cos();
        }
    }
    let mut tmp = [0.0f32; 64];
    for y in 0..8 {
        for x in 0..8 {
            let mut sum = 0.0f32;
            for u in 0..8 {
                let cu = if u == 0 { std::f32::consts::FRAC_1_SQRT_2 } else { 1.0 };
                sum += cu * dequant[y * 8 + u] * cos_table[u][x];
            }
            tmp[y * 8 + x] = 0.5 * sum;
        }
    }
    let mut out = [0.0f32; 64];
    for y in 0..8 {
        for x in 0..8 {
            let mut sum = 0.0f32;
            for v in 0..8 {
                let cv = if v == 0 { std::f32::consts::FRAC_1_SQRT_2 } else { 1.0 };
                sum += cv * tmp[v * 8 + x] * cos_table[v][y];
            }
            out[y * 8 + x] = 0.5 * sum;
        }
    }
    out
}

fn decode_baseline_jpeg(bytes: &[u8]) -> Option<Bitmap> {
    if bytes.len() < 4 || bytes[0..2] != [0xFF, 0xD8] {
        return None;
    }
    let mut pos = 2usize;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut quant_tables = [[1u8; 64]; 4];
    let mut dc_tables: [Option<HuffDecodeTable>; 4] = [None, None, None, None];
    let mut ac_tables: [Option<HuffDecodeTable>; 4] = [None, None, None, None];
    let mut comp_q = [0usize, 1, 1];
    let mut comp_dc = [0usize, 1, 1];
    let mut comp_ac = [0usize, 1, 1];

    while pos + 4 <= bytes.len() {
        if bytes[pos] != 0xFF {
            return None;
        }
        while pos < bytes.len() && bytes[pos] == 0xFF {
            pos += 1;
        }
        if pos >= bytes.len() {
            return None;
        }
        let marker = bytes[pos];
        pos += 1;
        if marker == 0xD9 {
            break;
        }
        if marker == 0xD8 || (0xD0..=0xD7).contains(&marker) {
            continue;
        }
        if pos + 2 > bytes.len() {
            return None;
        }
        let seg_len = u16::from_be_bytes([bytes[pos], bytes[pos + 1]]) as usize;
        if seg_len < 2 || pos + seg_len > bytes.len() {
            return None;
        }
        let seg = &bytes[pos + 2..pos + seg_len];
        pos += seg_len;

        match marker {
            0xDB => {
                let mut s = 0usize;
                while s + 65 <= seg.len() {
                    let pq_tq = seg[s];
                    if (pq_tq >> 4) != 0 {
                        return None;
                    }
                    let tq = (pq_tq & 0x0F) as usize;
                    if tq >= 4 {
                        return None;
                    }
                    quant_tables[tq].copy_from_slice(&seg[s + 1..s + 65]);
                    s += 65;
                }
            }
            0xC0 => {
                if seg.len() < 15 || seg[0] != 8 || seg[5] != 3 {
                    return None;
                }
                height = u32::from(u16::from_be_bytes([seg[1], seg[2]]));
                width = u32::from(u16::from_be_bytes([seg[3], seg[4]]));
                for c in 0..3 {
                    let sampling = seg[6 + c * 3 + 1];
                    if sampling != 0x11 {
                        return None; // non-4:4:4 handled by external fallback
                    }
                    comp_q[c] = (seg[6 + c * 3 + 2] & 0x03) as usize;
                }
            }
            0xC4 => {
                let mut s = 0usize;
                while s + 17 <= seg.len() {
                    let tc_th = seg[s];
                    let tc = (tc_th >> 4) as usize;
                    let th = (tc_th & 0x0F) as usize;
                    if tc > 1 || th >= 4 {
                        return None;
                    }
                    let bits: [u8; 16] = seg[s + 1..s + 17].try_into().ok()?;
                    let total_vals: usize = bits.iter().map(|&b| b as usize).sum();
                    s += 17;
                    if s + total_vals > seg.len() {
                        return None;
                    }
                    let vals = &seg[s..s + total_vals];
                    s += total_vals;
                    let table = HuffDecodeTable::from_bits_vals(&bits, vals);
                    if tc == 0 {
                        dc_tables[th] = Some(table);
                    } else {
                        ac_tables[th] = Some(table);
                    }
                }
            }
            0xDA => {
                if seg.len() < 10 || seg[0] != 3 || width == 0 || height == 0 {
                    return None;
                }
                for c in 0..3 {
                    let td_ta = seg[1 + c * 2 + 1];
                    comp_dc[c] = ((td_ta >> 4) & 0x03) as usize;
                    comp_ac[c] = (td_ta & 0x03) as usize;
                }
                let mut br = JpegBitReader::new(&bytes[pos..]);
                let w = width as usize;
                let h = height as usize;
                let mcu_cols = w.div_ceil(8);
                let mcu_rows = h.div_ceil(8);
                let mut bgra = vec![0u8; w * h * 4];
                let mut prev_dc = [0i32; 3];

                for my in 0..mcu_rows {
                    for mx in 0..mcu_cols {
                        let mut planes = [[0.0f32; 64]; 3];
                        for c in 0..3 {
                            let dc_tab = dc_tables[comp_dc[c]].as_ref()?;
                            let ac_tab = ac_tables[comp_ac[c]].as_ref()?;
                            let mut zz = [0i32; 64];
                            let cat = br.decode_huff(dc_tab)?;
                            let diff_bits = br.read_bits(cat)?;
                            let dc_val = prev_dc[c] + extend_magnitude(diff_bits, cat);
                            prev_dc[c] = dc_val;
                            zz[0] = dc_val;

                            let mut k = 1usize;
                            while k < 64 {
                                let rs = br.decode_huff(ac_tab)?;
                                if rs == 0x00 {
                                    break;
                                }
                                if rs == 0xF0 {
                                    k += 16;
                                    continue;
                                }
                                let run = (rs >> 4) as usize;
                                let acat = rs & 0x0F;
                                k += run;
                                if k >= 64 {
                                    return None;
                                }
                                let abits = br.read_bits(acat)?;
                                zz[k] = extend_magnitude(abits, acat);
                                k += 1;
                            }
                            planes[c] = idct_block(&zz, &quant_tables[comp_q[c]]);
                        }
                        for by in 0..8 {
                            let py = my * 8 + by;
                            if py >= h {
                                continue;
                            }
                            for bx in 0..8 {
                                let px = mx * 8 + bx;
                                if px >= w {
                                    continue;
                                }
                                let bidx = by * 8 + bx;
                                let y_val = planes[0][bidx] + 128.0;
                                let cb_val = planes[1][bidx];
                                let cr_val = planes[2][bidx];
                                let r = (y_val + 1.402 * cr_val).round().clamp(0.0, 255.0) as u8;
                                let g = (y_val - 0.344_136 * cb_val - 0.714_136 * cr_val)
                                    .round()
                                    .clamp(0.0, 255.0) as u8;
                                let b = (y_val + 1.772 * cb_val).round().clamp(0.0, 255.0) as u8;
                                let dst = (py * w + px) * 4;
                                bgra[dst] = b;
                                bgra[dst + 1] = g;
                                bgra[dst + 2] = r;
                                bgra[dst + 3] = 255;
                            }
                        }
                    }
                }
                return Some(Bitmap { width, height, bgra });
            }
            _ => {}
        }
    }
    None
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
    Some(Bitmap { width, height, bgra })
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
        let src_row = if top_down { y } else { height_u as usize - 1 - y };
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
    Some(Bitmap { width: width_u, height: height_u, bgra })
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
            "image2pipe",
            "-c:v",
            "bmp",
            "-pix_fmt",
            "bgra",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let input = bytes.to_vec();
        std::thread::spawn(move || {
            let _ = stdin.write_all(&input);
        });
    }
    let output = child.wait_with_output().ok()?;
    if output.status.success() { decode_bmp(&output.stdout) } else { None }
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
    if let Some(jpg) = decode_baseline_jpeg(bytes) {
        return Ok(jpg);
    }
    if let Some(bmp) = decode_via_python_or_ffmpeg(bytes) {
        return Ok(bmp);
    }
    Err(Error::new(ErrorKind::InvalidData, "unsupported or corrupt image data"))
}

/// Scales down (never up) so neither side exceeds `max`, averaging covered pixels.
pub fn scale_to(bitmap: &Bitmap, max: u32) -> Bitmap {
    let longest = bitmap.width.max(bitmap.height);
    if longest <= max || max == 0 || bitmap.width == 0 || bitmap.height == 0 {
        return bitmap.clone();
    }
    let width = ((u64::from(bitmap.width) * u64::from(max)) / u64::from(longest)).max(1) as u32;
    let height = ((u64::from(bitmap.height) * u64::from(max)) / u64::from(longest)).max(1) as u32;
    let mut bgra = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        let y0 = (u64::from(y) * u64::from(bitmap.height) / u64::from(height)) as u32;
        let y1 = ((u64::from(y + 1) * u64::from(bitmap.height) / u64::from(height)) as u32)
            .max(y0 + 1)
            .min(bitmap.height);
        for x in 0..width {
            let x0 = (u64::from(x) * u64::from(bitmap.width) / u64::from(width)) as u32;
            let x1 = ((u64::from(x + 1) * u64::from(bitmap.width) / u64::from(width)) as u32)
                .max(x0 + 1)
                .min(bitmap.width);
            let mut sum = [0u32; 4];
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let i = ((sy * bitmap.width + sx) * 4) as usize;
                    for (total, &val) in sum.iter_mut().zip(&bitmap.bgra[i..i + 4]) {
                        *total += u32::from(val);
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)).max(1);
            bgra.extend(sum.map(|s| (s / n) as u8));
        }
    }
    Bitmap { width, height, bgra }
}

/// Crops the center square of `bitmap`.
pub fn crop_square(bitmap: &Bitmap) -> Bitmap {
    let side = bitmap.width.min(bitmap.height);
    if side == 0 || (bitmap.width == bitmap.height) {
        return bitmap.clone();
    }
    let x_off = (bitmap.width - side) / 2;
    let y_off = (bitmap.height - side) / 2;
    let mut bgra = Vec::with_capacity((side * side * 4) as usize);
    for y in 0..side {
        let src_row = (y_off + y) as usize * bitmap.width as usize;
        let start = (src_row + x_off as usize) * 4;
        let end = start + (side as usize * 4);
        bgra.extend_from_slice(&bitmap.bgra[start..end]);
    }
    Bitmap { width: side, height: side, bgra }
}

/// Decodes an image, center-crops it to a square, scales to at most `max_side`,
/// and encodes it as a baseline JPEG.
pub fn crop_square_jpeg(bytes: &[u8], max_side: u32, quality: u8) -> Result<Vec<u8>> {
    let decoded = decode(bytes)?;
    let cropped = crop_square(&decoded);
    let scaled = if max_side > 0 { scale_to(&cropped, max_side) } else { cropped };
    encode_jpeg(&scaled, quality)
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
        Bitmap { width, height, bgra }
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
    fn jpeg_encode_and_decode_round_trips_solid_color() {
        let mut bgra = Vec::with_capacity(16 * 16 * 4);
        for _ in 0..(16 * 16) {
            bgra.extend_from_slice(&[40, 120, 220, 255]); // B=40, G=120, R=220
        }
        let original = Bitmap { width: 16, height: 16, bgra };
        let jpeg = encode_jpeg(&original, 90).expect("encode_jpeg");
        assert!(jpeg.starts_with(&[0xFF, 0xD8, 0xFF]));
        assert!(jpeg.ends_with(&[0xFF, 0xD9]));

        let decoded = decode(&jpeg).expect("decode jpeg");
        assert_eq!(decoded.width, 16);
        assert_eq!(decoded.height, 16);
        assert!(decoded.bgra[0].abs_diff(40) <= 8);
        assert!(decoded.bgra[1].abs_diff(120) <= 8);
        assert!(decoded.bgra[2].abs_diff(220) <= 8);
        assert_eq!(decoded.bgra[3], 255);
    }

    #[test]
    fn crop_square_jpeg_crops_and_scales_rectangular_image() {
        let rect = sample_bitmap(80, 40);
        let png = encode_png(&rect).expect("encode_png");
        let square_jpg = crop_square_jpeg(&png, 24, 85).expect("crop_square_jpeg");
        assert!(square_jpg.starts_with(&[0xFF, 0xD8, 0xFF]));

        let decoded = decode(&square_jpg).expect("decode cropped jpeg");
        assert_eq!(decoded.width, 24);
        assert_eq!(decoded.height, 24);
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

    #[test]
    fn decode_handles_420_subsampled_camera_jpeg() {
        let out = std::process::Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=0x2878dc:s=32x24:d=1",
                "-frames:v",
                "1",
                "-f",
                "image2pipe",
                "-c:v",
                "mjpeg",
                "-pix_fmt",
                "yuvj420p",
                "pipe:1",
            ])
            .output();
        let Ok(out) = out else { return };
        if !out.status.success() || out.stdout.is_empty() {
            return;
        }
        let decoded = decode(&out.stdout).expect("decode 4:2:0 camera jpeg");
        assert_eq!(decoded.width, 32);
        assert_eq!(decoded.height, 24);
        assert_eq!(decoded.bgra[3], 255);
    }
}
