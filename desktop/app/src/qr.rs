// SPDX-License-Identifier: GPL-3.0-or-later
//! QR codes as one SVG path, so QML draws them crisp at any size and scale.

use qrcode::{Color, EcLevel, QrCode};

/// A QR code ready to draw: `path` covers the dark modules of a
/// `size`×`size` grid (one unit per module, no quiet zone).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrPath {
    pub size: usize,
    pub path: String,
}

/// Encodes `text` with medium error correction (pairing links are short and
/// get scanned off screens, where M is the usual choice).
pub fn encode(text: &str) -> Result<QrPath, qrcode::types::QrError> {
    let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M)?;
    let size = code.width();
    let colors = code.to_colors();
    let mut path = String::with_capacity(size * size);
    for (y, row) in colors.chunks(size).enumerate() {
        let mut x = 0;
        while x < size {
            if row[x] != Color::Dark {
                x += 1;
                continue;
            }
            // Merge horizontal runs into one rectangle each.
            let start = x;
            while x < size && row[x] == Color::Dark {
                x += 1;
            }
            let run = x - start;
            path.push_str(&format!("M{start} {y}h{run}v1h-{run}z"));
        }
    }
    Ok(QrPath { size, path })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Turns the path back into a grid to check it matches the code.
    fn rasterize(qr: &QrPath) -> Vec<bool> {
        let mut grid = vec![false; qr.size * qr.size];
        for rect in qr.path.split('z').filter(|r| !r.is_empty()) {
            let nums: Vec<usize> = rect
                .split(|c: char| !c.is_ascii_digit())
                .filter(|s| !s.is_empty())
                .map(|s| s.parse().unwrap())
                .collect();
            let [x, y, run, ..] = nums[..] else { panic!("bad rect {rect}") };
            for i in x..x + run {
                grid[y * qr.size + i] = true;
            }
        }
        grid
    }

    #[test]
    fn path_matches_the_code() {
        let text = "nectarlink://pair?v=0&id=ybndrfg8ejkmcpqxot1uwisza345h769ybndrfg8ejkmcpqx&s=abcdefghijklmnopqrstuvwxyz";
        let qr = encode(text).unwrap();
        let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M).unwrap();
        let expected: Vec<bool> = code.to_colors().into_iter().map(|c| c == Color::Dark).collect();
        assert_eq!(qr.size, code.width());
        assert_eq!(rasterize(&qr), expected);
    }

    #[test]
    fn longer_text_needs_a_bigger_code() {
        assert!(encode(&"x".repeat(200)).unwrap().size > encode("x").unwrap().size);
    }
}
