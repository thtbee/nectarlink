// SPDX-License-Identifier: GPL-3.0-or-later
use std::fs;
use std::io::Result;
use std::path::Path;

use crate::mark;

use super::image::{Bitmap, encode_png};

pub fn write_png(path: &Path, size: u32) -> Result<()> {
    let rgba = mark::render(size as usize);
    let mut bgra = Vec::with_capacity(rgba.len());
    for px in rgba.as_chunks::<4>().0 {
        bgra.push(px[2]);
        bgra.push(px[1]);
        bgra.push(px[0]);
        bgra.push(px[3]);
    }
    let bitmap = Bitmap { width: size, height: size, bgra };
    let png = encode_png(&bitmap)?;
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(path, png)
}
