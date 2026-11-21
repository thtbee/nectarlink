// SPDX-License-Identifier: GPL-3.0-or-later
//! The NectarLink mark, the Waggle Dance (`assets/brand/icon.svg`): two loops,
//! the phone and the PC, around a drop of nectar, tilted like a bee's waggle
//! run, on an espresso tile with a warm glow. Drawn in code at any pixel
//! size: the tray, window and toast icons at runtime, and the executable's
//! icon at build time (`build.rs` includes this file), so every size is sharp
//! without bitmap assets. At 32 px and below the strokes are heavier and the
//! tile fills more of the square, as in the hand-tuned small sizes.

/// Espresso tile, its glow, the honey rim, and the two loops and the drop
/// (docs/design/tokens.json, Honey palette).
const TILE: [f32; 3] = rgb(0x2C, 0x16, 0x00);
const GLOW: [f32; 3] = rgb(0x69, 0x3C, 0x00);
const RIM: [f32; 3] = rgb(0xFF, 0xB9, 0x5F);
const PHONE: [f32; 3] = rgb(0xFF, 0xF8, 0xF3);
const PC: [f32; 3] = rgb(0xFF, 0xDD, 0xB8);
const NECTAR: [f32; 3] = rgb(0xFF, 0xB9, 0x5F);

/// The loops in a 100×100 design grid, before the tilt: each is two cubic
/// Béziers (start, control, control, end, control, control, end).
const LEFT_LOOP: [(f32, f32); 7] =
    [(41.5, 42.5), (26.0, 24.0), (13.5, 31.0), (13.5, 50.0), (13.5, 69.0), (26.0, 76.0), (41.5, 57.5)];
const RIGHT_LOOP: [(f32, f32); 7] =
    [(58.5, 42.5), (74.0, 24.0), (86.5, 31.0), (86.5, 50.0), (86.5, 69.0), (74.0, 76.0), (58.5, 57.5)];
/// The tilt, degrees (counterclockwise on screen).
const TILT: f32 = -25.0;

/// Supersampling grid per pixel (SS×SS samples) for smooth edges.
const SS: usize = 4;

const fn rgb(r: u8, g: u8, b: u8) -> [f32; 3] {
    [r as f32, g as f32, b as f32]
}

/// Sizes in the design grid: the small sizes are drawn bolder.
struct Geometry {
    inset: f32,
    corner: f32,
    stroke: f32,
    drop: f32,
}

impl Geometry {
    fn for_size(size: usize) -> Self {
        if size <= 32 {
            Geometry { inset: 2.0, corner: 20.0, stroke: 11.4, drop: 8.8 }
        } else {
            Geometry { inset: 4.0, corner: 22.0, stroke: 10.4, drop: 8.2 }
        }
    }
}

/// Renders the mark as straight-alpha RGBA, row by row, `size`×`size`.
pub fn render(size: usize) -> Vec<u8> {
    let g = Geometry::for_size(size);
    let unit = 100.0 / size as f32;
    let left = polyline(&LEFT_LOOP);
    let right = polyline(&RIGHT_LOOP);
    let (sin, cos) = (-TILT.to_radians()).sin_cos();
    // The rim is at least about a pixel wide, so the tile keeps an edge on dark taskbars.
    let rim = (1.5f32).max(unit * 0.6);

    let mut out = vec![0u8; size * size * 4];
    for py in 0..size {
        for px in 0..size {
            let mut color = [0f32; 3];
            let mut covered = 0usize;
            for sy in 0..SS {
                for sx in 0..SS {
                    // The sample in the design grid.
                    let x = (px as f32 + (sx as f32 + 0.5) / SS as f32) * unit;
                    let y = (py as f32 + (sy as f32 + 0.5) / SS as f32) * unit;
                    let edge = rounded_square_distance(x, y, g.inset, g.corner);
                    if edge > 0.0 {
                        continue;
                    }
                    covered += 1;
                    // Undo the tilt to test against the upright loops.
                    let (dx, dy) = (x - 50.0, y - 50.0);
                    let (ux, uy) = (50.0 + dx * cos - dy * sin, 50.0 + dx * sin + dy * cos);
                    let half = g.stroke / 2.0;
                    let paint = if (dx * dx + dy * dy).sqrt() <= g.drop {
                        NECTAR
                    } else if distance_to(&left, (ux, uy)) <= half {
                        PHONE
                    } else if distance_to(&right, (ux, uy)) <= half {
                        PC
                    } else if -edge <= rim {
                        mix(TILE, RIM, 0.34)
                    } else {
                        // The glow fades from the middle to the tile color at 40 units.
                        let t = ((dx * dx + dy * dy).sqrt() / 40.0).min(1.0);
                        mix(GLOW, TILE, t)
                    };
                    for c in 0..3 {
                        color[c] += paint[c];
                    }
                }
            }
            if covered == 0 {
                continue;
            }
            let i = (py * size + px) * 4;
            for c in 0..3 {
                out[i + c] = (color[c] / covered as f32).round() as u8;
            }
            out[i + 3] = ((covered as f32 / (SS * SS) as f32) * 255.0).round() as u8;
        }
    }
    out
}

/// The mark in one colour, for the tray (#479): the two loops and the drop
/// only, no tile, a little larger so it fills the square like other tray
/// icons. Straight-alpha RGBA, `size`×`size`.
// Only the Windows tray draws it; the build script and the other systems
// compile this file too.
#[allow(dead_code)]
pub fn render_glyph(size: usize, color: [u8; 3]) -> Vec<u8> {
    // Without the tile the loops span about 80 of the 100 units; scale them
    // up to about 96 and draw them bolder, so they read at 16 px.
    const SCALE: f32 = 1.18;
    let stroke = if size <= 32 { 13.0 } else { 11.6 };
    let drop = if size <= 32 { 9.6 } else { 9.0 };
    let unit = 100.0 / size as f32;
    let left = polyline(&LEFT_LOOP);
    let right = polyline(&RIGHT_LOOP);
    let (sin, cos) = (-TILT.to_radians()).sin_cos();
    let mut out = vec![0u8; size * size * 4];
    for py in 0..size {
        for px in 0..size {
            let mut inside = 0usize;
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = (px as f32 + (sx as f32 + 0.5) / SS as f32) * unit;
                    let y = (py as f32 + (sy as f32 + 0.5) / SS as f32) * unit;
                    // Back into the design grid, then undo the tilt.
                    let (dx, dy) = ((x - 50.0) / SCALE, (y - 50.0) / SCALE);
                    let (ux, uy) = (50.0 + dx * cos - dy * sin, 50.0 + dx * sin + dy * cos);
                    let half = stroke / 2.0;
                    if (dx * dx + dy * dy).sqrt() <= drop
                        || distance_to(&left, (ux, uy)) <= half
                        || distance_to(&right, (ux, uy)) <= half
                    {
                        inside += 1;
                    }
                }
            }
            if inside == 0 {
                continue;
            }
            let i = (py * size + px) * 4;
            out[i..i + 3].copy_from_slice(&color);
            out[i + 3] = ((inside as f32 / (SS * SS) as f32) * 255.0).round() as u8;
        }
    }
    out
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// How far (x, y) is outside the rounded square from `inset` to
/// `100 - inset` with corner radius `r` (negative inside).
fn rounded_square_distance(x: f32, y: f32, inset: f32, r: f32) -> f32 {
    let (lo, hi) = (inset + r, 100.0 - inset - r);
    let qx = (x.clamp(lo, hi) - x).abs();
    let qy = (y.clamp(lo, hi) - y).abs();
    if x > lo && x < hi || y > lo && y < hi {
        // Along a straight side.
        let to_x = (x - inset).min(100.0 - inset - x);
        let to_y = (y - inset).min(100.0 - inset - y);
        return -to_x.min(to_y);
    }
    (qx * qx + qy * qy).sqrt() - r
}

/// A loop's two Béziers as points close enough together to measure against.
fn polyline(loop_points: &[(f32, f32); 7]) -> Vec<(f32, f32)> {
    const STEPS: usize = 32;
    let mut points = Vec::with_capacity(STEPS * 2 + 1);
    for segment in [&loop_points[0..4], &loop_points[3..7]] {
        for step in 0..=STEPS {
            if step == 0 && !points.is_empty() {
                continue;
            }
            let t = step as f32 / STEPS as f32;
            let u = 1.0 - t;
            let (a, b, c, d) = (segment[0], segment[1], segment[2], segment[3]);
            let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
            points.push((
                w[0] * a.0 + w[1] * b.0 + w[2] * c.0 + w[3] * d.0,
                w[0] * a.1 + w[1] * b.1 + w[2] * c.1 + w[3] * d.1,
            ));
        }
    }
    points
}

fn distance_to(polyline: &[(f32, f32)], p: (f32, f32)) -> f32 {
    polyline.windows(2).map(|w| dist_to_segment(p, w[0], w[1])).fold(f32::INFINITY, f32::min)
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
    fn corners_are_transparent_and_the_drop_sits_in_the_middle() {
        let size = 64;
        let img = render(size);
        assert_eq!(img.len(), size * size * 4);
        assert_eq!(pixel(&img, size, 0, 0)[3], 0, "rounded corner");
        let middle = pixel(&img, size, 32, 32);
        assert_eq!(middle, [0xFF, 0xB9, 0x5F, 255], "the nectar drop");
        // Near the top edge, between the loops: the dark tile.
        let tile = pixel(&img, size, 32, 8);
        assert!(tile[0] < 0x80 && tile[3] == 255, "{tile:?}");
    }

    #[test]
    fn both_loops_are_drawn() {
        let size = 128;
        let img = render(size);
        let light = |p: [u8; 4]| p[0] > 0xF0 && p[3] == 255;
        // Somewhere left and right of the drop there's loop ink.
        let left = (8..56).any(|x| light(pixel(&img, size, x, 64)) || light(pixel(&img, size, x, 70)));
        let right = (72..120).any(|x| light(pixel(&img, size, x, 64)) || light(pixel(&img, size, x, 58)));
        assert!(left && right);
    }

    #[test]
    fn the_glyph_is_one_colour_on_nothing() {
        for size in [16, 20, 24, 32] {
            let img = render_glyph(size, [255, 255, 255]);
            assert_eq!(pixel(&img, size, 0, 0)[3], 0, "corner at {size}");
            let c = size / 2;
            assert_eq!(pixel(&img, size, c, c), [255, 255, 255, 255], "drop at {size}");
            assert!(img.chunks(4).all(|p| p[3] == 0 || p[..3] == [255, 255, 255]));
            // Both loops: something drawn in the left and the right third.
            let drawn =
                |x0: usize, x1: usize| (0..size).any(|y| (x0..x1).any(|x| pixel(&img, size, x, y)[3] > 128));
            assert!(drawn(0, size / 3) && drawn(size * 2 / 3, size), "loops at {size}");
        }
    }

    #[test]
    fn renders_every_tray_size() {
        for size in [16, 20, 24, 32, 48, 64, 256] {
            let img = render(size);
            assert!(img.chunks(4).any(|p| p[3] == 255), "{size}px has an opaque tile");
        }
    }
}
