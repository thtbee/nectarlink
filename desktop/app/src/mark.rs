// SPDX-License-Identifier: GPL-3.0-or-later
//! The Nectarlink mark (a hexagon on a rounded square, as in the app's nav
//! rail), drawn in code at any pixel size: the tray, window and toast icons
//! at runtime, and the executable's icon at build time (`build.rs` includes
//! this file), so every size is sharp without bitmap assets.

/// Honey primary (docs/design/tokens.json) and the mark's ink.
const BACKGROUND: [u8; 3] = [0x8A, 0x51, 0x00];
const INK: [u8; 3] = [0xFF, 0xFF, 0xFF];

/// Hexagon and bar of the mark, in a 24×24 design grid.
const HEXAGON: [(f32, f32); 6] =
    [(12.0, 2.5), (20.2, 7.25), (20.2, 16.75), (12.0, 21.5), (3.8, 16.75), (3.8, 7.25)];
const BAR: ((f32, f32), (f32, f32)) = ((9.0, 12.0), (15.0, 12.0));

/// Supersampling grid per pixel (SS×SS samples) for smooth edges.
const SS: usize = 4;

/// Renders the mark as straight-alpha RGBA, row by row, `size`×`size`.
pub fn render(size: usize) -> Vec<u8> {
    let s = size as f32;
    let radius = s * 0.28;
    // The glyph fills 62% of the tile, centered; stroke scales with it but
    // never gets thinner than ~1.4 px so small sizes stay legible.
    let glyph = s * 0.62;
    let scale = glyph / 24.0;
    let offset = (s - glyph) / 2.0;
    let stroke = (1.8 * scale).max(1.4);
    let hex: Vec<(f32, f32)> =
        HEXAGON.iter().map(|&(x, y)| (offset + x * scale, offset + y * scale)).collect();
    let bar = (
        (offset + BAR.0.0 * scale, offset + BAR.0.1 * scale),
        (offset + BAR.1.0 * scale, offset + BAR.1.1 * scale),
    );

    let mut out = vec![0u8; size * size * 4];
    for py in 0..size {
        for px in 0..size {
            let (mut tile, mut ink) = (0usize, 0usize);
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = px as f32 + (sx as f32 + 0.5) / SS as f32;
                    let y = py as f32 + (sy as f32 + 0.5) / SS as f32;
                    if !in_rounded_square(x, y, s, radius) {
                        continue;
                    }
                    tile += 1;
                    let on_hex = (0..hex.len())
                        .any(|i| dist_to_segment((x, y), hex[i], hex[(i + 1) % hex.len()]) <= stroke / 2.0);
                    if on_hex || dist_to_segment((x, y), bar.0, bar.1) <= stroke / 2.0 {
                        ink += 1;
                    }
                }
            }
            let i = (py * size + px) * 4;
            if tile == 0 {
                continue;
            }
            // Ink over background, then the tile's coverage as alpha.
            let t = ink as f32 / tile as f32;
            for c in 0..3 {
                out[i + c] = (BACKGROUND[c] as f32 * (1.0 - t) + INK[c] as f32 * t).round() as u8;
            }
            out[i + 3] = ((tile as f32 / (SS * SS) as f32) * 255.0).round() as u8;
        }
    }
    out
}

fn in_rounded_square(x: f32, y: f32, size: f32, r: f32) -> bool {
    let cx = x.clamp(r, size - r);
    let cy = y.clamp(r, size - r);
    (x - cx).powi(2) + (y - cy).powi(2) <= r * r
}

fn dist_to_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 { 0.0 } else { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) };
    let (qx, qy) = (a.0 + t * dx, a.1 + t * dy);
    ((p.0 - qx).powi(2) + (p.1 - qy).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(img: &[u8], size: usize, x: usize, y: usize) -> [u8; 4] {
        let i = (y * size + x) * 4;
        [img[i], img[i + 1], img[i + 2], img[i + 3]]
    }

    #[test]
    fn corners_are_transparent_and_the_tile_is_opaque() {
        let size = 32;
        let img = render(size);
        assert_eq!(img.len(), size * size * 4);
        assert_eq!(pixel(&img, size, 0, 0)[3], 0, "rounded corner");
        // Between the hexagon and the tile edge: solid background.
        assert_eq!(pixel(&img, size, 3, 16), [BACKGROUND[0], BACKGROUND[1], BACKGROUND[2], 255]);
        // The bar through the middle is (anti-aliased) ink on an opaque tile.
        let mid = pixel(&img, size, 16, 16);
        assert!(mid[0] > 200 && mid[3] == 255, "{mid:?}");
    }

    #[test]
    fn renders_every_tray_size() {
        for size in [16, 20, 24, 32, 48, 64, 256] {
            let img = render(size);
            assert!(img.chunks(4).any(|p| p[3] == 255), "{size}px has an opaque tile");
        }
    }
}
