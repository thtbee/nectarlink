// SPDX-License-Identifier: GPL-3.0-or-later
//! Deck tile pictures on this PC (#490): a picture the user chose, cut to a
//! square, made small and kept as `deck-images/<id>.png` next to deck.json.
//! The ID is [`image_id`] of the PNG, so the same picture is one file.

use std::{
    collections::{BTreeSet, HashSet},
    io,
    path::{Path, PathBuf},
    sync::Mutex,
};

use nectarlink_core::deck::{deck_image, image_id};

use crate::{i18n::tr, win::image::Bitmap};

/// The folder next to deck.json the pictures are kept in.
pub const DIR: &str = "deck-images";
/// A picture file the user chooses, at most.
pub const MAX_SOURCE_BYTES: u64 = 20 * 1024 * 1024;
/// The sides tried in turn until the PNG fits in [`deck_image::MAX_BYTES`].
const SIDES: [u32; 3] = [192, 128, 96];

/// Pictures chosen in the tile editor that no saved tile names yet; they
/// stay on disk until the next tile is saved.
static PENDING: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

fn pending() -> std::sync::MutexGuard<'static, BTreeSet<String>> {
    PENDING.lock().unwrap_or_else(|e| e.into_inner())
}

/// Forgets the pictures chosen but not saved, so the next cleanup removes them.
pub fn clear_pending() {
    pending().clear();
}

pub fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join(DIR)
}

/// Where the picture `id` is kept, for a valid ID only.
pub fn path(data_dir: &Path, id: &str) -> Option<PathBuf> {
    deck_image::is_valid_id(id).then(|| dir(data_dir).join(format!("{id}.png")))
}

/// Whether the picture `id` is on disk.
pub fn exists(data_dir: &Path, id: &str) -> bool {
    path(data_dir, id).is_some_and(|p| p.is_file())
}

/// The picture file at `file` as a tile picture: read, decoded, cut to its
/// middle square and made small enough. Slow for big files; call it off
/// the UI thread.
pub fn prepare_file(file: &Path) -> Result<Vec<u8>, String> {
    let size = std::fs::metadata(file).map_err(|_| tr!("Deck", "Can't open that picture."))?.len();
    if size > MAX_SOURCE_BYTES {
        return Err(tr!("Deck", "That picture is over 20 MB. Choose a smaller one."));
    }
    let bytes = std::fs::read(file).map_err(|_| tr!("Deck", "Can't open that picture."))?;
    prepare(&bytes)
}

/// An image file's bytes as a tile picture (see [`prepare_file`]).
pub fn prepare(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let bitmap = crate::win::image::decode(bytes)
        .ok()
        .filter(|b| b.width > 0 && b.height > 0 && b.bgra.len() >= (b.width * b.height * 4) as usize)
        .ok_or_else(|| tr!("Deck", "Can't read that picture. Try a PNG or JPEG."))?;
    fit(&crop_square(&bitmap))
}

/// The middle square of `bitmap`.
pub fn crop_square(bitmap: &Bitmap) -> Bitmap {
    let side = bitmap.width.min(bitmap.height);
    if bitmap.width == bitmap.height {
        return bitmap.clone();
    }
    let (left, top) = ((bitmap.width - side) / 2, (bitmap.height - side) / 2);
    let mut bgra = Vec::with_capacity((side * side * 4) as usize);
    for y in top..top + side {
        let start = ((y * bitmap.width + left) * 4) as usize;
        bgra.extend_from_slice(&bitmap.bgra[start..start + (side * 4) as usize]);
    }
    Bitmap { width: side, height: side, bgra }
}

/// A square picture as a PNG a tile may show: 192 pixels a side, or 128,
/// or 96 when it doesn't fit in [`deck_image::MAX_BYTES`].
fn fit(square: &Bitmap) -> Result<Vec<u8>, String> {
    for side in SIDES {
        let png = encode_png(&crate::win::image::scale_to(square, side));
        if deck_image::is_valid_png(&png) {
            return Ok(png);
        }
    }
    Err(tr!("Deck", "That picture has too much detail to send to a phone. Try a simpler one."))
}

/// Keeps `png` as `deck-images/<id>.png` and returns its ID. Until a tile
/// that names it is saved, it's kept through cleanups.
pub fn store(data_dir: &Path, png: &[u8]) -> io::Result<String> {
    let id = image_id(png);
    let folder = dir(data_dir);
    std::fs::create_dir_all(&folder)?;
    let file = folder.join(format!("{id}.png"));
    if !file.is_file() {
        let part = folder.join(format!("{id}.part"));
        std::fs::write(&part, png)?;
        std::fs::rename(&part, &file)?;
    }
    pending().insert(id.clone());
    Ok(id)
}

/// The PNGs of the pictures `ids` names that are on disk.
pub fn load<'a>(data_dir: &Path, ids: impl IntoIterator<Item = &'a str>) -> Vec<Vec<u8>> {
    ids.into_iter().filter_map(|id| std::fs::read(path(data_dir, id)?).ok()).collect()
}

/// Removes the pictures neither `keep` nor the editor's unsaved choice names.
pub fn prune(data_dir: &Path, keep: &HashSet<String>) {
    let Ok(entries) = std::fs::read_dir(dir(data_dir)) else { return };
    let pending = pending();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(id) = name.strip_suffix(".png") else { continue };
        if !keep.contains(id)
            && !pending.contains(id)
            && let Err(error) = std::fs::remove_file(entry.path())
        {
            tracing::debug!(%error, "a deck picture no tile names wasn't removed");
        }
    }
}

/// Straight-alpha BGRA pixels as an RGBA PNG, each row filtered the way
/// that packs best, then deflated. The platforms' own encoders don't all
/// compress, and a tile picture has to be small.
pub fn encode_png(bitmap: &Bitmap) -> Vec<u8> {
    let stride = bitmap.width as usize * 4;
    let rgba: Vec<u8> =
        bitmap.bgra.as_chunks::<4>().0.iter().flat_map(|p| [p[2], p[1], p[0], p[3]]).collect();
    let mut raw = Vec::with_capacity((stride + 1) * bitmap.height as usize);
    let zero = vec![0u8; stride];
    let mut line = vec![0u8; stride];
    for (y, row) in rgba.chunks_exact(stride).enumerate() {
        let up = if y == 0 { &zero[..] } else { &rgba[(y - 1) * stride..y * stride] };
        let mut best: Option<(u64, u8, Vec<u8>)> = None;
        for filter in 0..5u8 {
            for i in 0..stride {
                let left = if i >= 4 { row[i - 4] } else { 0 };
                let corner = if i >= 4 { up[i - 4] } else { 0 };
                line[i] = row[i].wrapping_sub(match filter {
                    0 => 0,
                    1 => left,
                    2 => up[i],
                    3 => ((u16::from(left) + u16::from(up[i])) / 2) as u8,
                    _ => paeth(left, up[i], corner),
                });
            }
            let cost = line.iter().map(|&b| u64::from((b as i8).unsigned_abs())).sum();
            if best.as_ref().is_none_or(|(c, ..)| cost < *c) {
                best = Some((cost, filter, line.clone()));
            }
        }
        if let Some((_, filter, bytes)) = best {
            raw.push(filter);
            raw.extend_from_slice(&bytes);
        }
    }
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&bitmap.width.to_be_bytes());
    ihdr.extend_from_slice(&bitmap.height.to_be_bytes());
    // 8 bits a channel, RGBA, deflate, standard filters, not interlaced.
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &miniz_oxide::deflate::compress_to_vec_zlib(&raw, 9));
    chunk(&mut out, b"IEND", &[]);
    out
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = i16::from(a) + i16::from(b) - i16::from(c);
    let (pa, pb, pc) = ((p - i16::from(a)).abs(), (p - i16::from(b)).abs(), (p - i16::from(c)).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pixels that don't compress: each one from a simple random sequence.
    fn noise(width: u32, height: u32) -> Bitmap {
        let mut state = 0x2545_f491_u32;
        let bgra = (0..width * height * 4)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (state >> 24) as u8
            })
            .collect();
        Bitmap { width, height, bgra }
    }

    fn gradient(width: u32, height: u32) -> Bitmap {
        let bgra = (0..width * height)
            .flat_map(|i| {
                let (x, y) = (i % width, i / width);
                [(x * 255 / width) as u8, (y * 255 / height) as u8, 128, 255]
            })
            .collect();
        Bitmap { width, height, bgra }
    }

    fn side(png: &[u8]) -> (u32, u32) {
        let at = |i: usize| u32::from_be_bytes(png[i..i + 4].try_into().unwrap());
        (at(16), at(20))
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nectarlink-deck-images-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn crops_the_middle_square() {
        // 6 by 2: columns 2 and 3 are the middle.
        let wide = Bitmap { width: 6, height: 2, bgra: (0..12u8).flat_map(|i| [i, 0, 0, 255]).collect() };
        let square = crop_square(&wide);
        assert_eq!((square.width, square.height), (2, 2));
        let firsts: Vec<u8> = square.bgra.chunks(4).map(|p| p[0]).collect();
        assert_eq!(firsts, vec![2, 3, 8, 9]);

        let tall = gradient(3, 9);
        let square = crop_square(&tall);
        assert_eq!((square.width, square.height), (3, 3));
        assert_eq!(square.bgra[..], tall.bgra[3 * 3 * 4..6 * 3 * 4]);
        assert_eq!(crop_square(&square), square);
    }

    #[test]
    fn pictures_are_192_pixel_squares_within_the_byte_limit() {
        let png = fit(&crop_square(&gradient(800, 500))).unwrap();
        assert_eq!(side(&png), (192, 192));
        assert!(deck_image::is_valid_png(&png));
        assert!(png.len() <= deck_image::MAX_BYTES);
        // It's a real PNG: the platform decodes it back to the same size.
        let back = crate::win::image::decode(&png).unwrap();
        assert_eq!((back.width, back.height), (192, 192));
    }

    #[test]
    fn detailed_pictures_get_smaller_until_they_fit() {
        // Noise doesn't compress: at 192 pixels it's too many bytes.
        let png = fit(&noise(192, 192)).unwrap();
        assert!(side(&png).0 < 192);
        assert!(png.len() <= deck_image::MAX_BYTES);
        // And at 128 too, so it's 96.
        let png = fit(&noise(128, 128)).unwrap();
        assert_eq!(side(&png), (96, 96));
        assert!(deck_image::is_valid_png(&png));
        // Small pictures aren't made bigger.
        assert_eq!(side(&fit(&gradient(40, 40)).unwrap()), (40, 40));
    }

    #[test]
    fn files_too_big_or_not_pictures_are_refused() {
        let dir = scratch("refused");
        let junk = dir.join("junk.png");
        std::fs::write(&junk, b"not a picture at all").unwrap();
        assert!(prepare_file(&junk).is_err());
        let huge = dir.join("huge.jpg");
        let file = std::fs::File::create(&huge).unwrap();
        file.set_len(MAX_SOURCE_BYTES + 1).unwrap();
        drop(file);
        assert!(prepare_file(&huge).is_err());
        assert!(prepare_file(&dir.join("missing.png")).is_err());

        let real = dir.join("real.png");
        std::fs::write(&real, encode_png(&gradient(300, 200))).unwrap();
        assert_eq!(side(&prepare_file(&real).unwrap()), (192, 192));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn stores_by_id_and_removes_what_no_tile_names() {
        let dir = scratch("store");
        let a = encode_png(&gradient(20, 20));
        let b = encode_png(&gradient(30, 30));
        let id_a = store(&dir, &a).unwrap();
        let id_b = store(&dir, &b).unwrap();
        assert_eq!(id_a, image_id(&a));
        assert!(deck_image::is_valid_id(&id_a));
        assert_eq!(path(&dir, &id_a).unwrap(), dir.join(DIR).join(format!("{id_a}.png")));
        assert_eq!(path(&dir, "../deck"), None);
        assert_eq!(store(&dir, &a).unwrap(), id_a, "the same picture is one file");

        // Chosen but not saved: kept through a cleanup until the next save.
        std::fs::write(dir.join(DIR).join("notes.txt"), b"x").unwrap();
        prune(&dir, &HashSet::new());
        assert!(exists(&dir, &id_a) && exists(&dir, &id_b));
        assert!(dir.join(DIR).join("notes.txt").exists(), "only pictures are removed");

        pending().remove(&id_a);
        pending().remove(&id_b);
        prune(&dir, &HashSet::from([id_b.clone()]));
        assert!(!exists(&dir, &id_a));
        assert!(exists(&dir, &id_b));
        assert_eq!(load(&dir, [id_a.as_str(), id_b.as_str()]), vec![b]);
        let _ = std::fs::remove_dir_all(dir);
    }
}
