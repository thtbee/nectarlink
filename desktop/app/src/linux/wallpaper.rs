use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use material_colors::color::Rgb;

use crate::palette;

use super::image;

const SAMPLE_MAX_DIM: u32 = palette::MAX_SAMPLE_SIDE;

pub fn seed() -> Option<Rgb> {
    let path = current_wallpaper_path()?;
    seed_from_path(&path)
}

fn sample_grid(path: &Path) -> Option<(u32, u32, Vec<Rgb>)> {
    let bytes = fs::read(path).ok()?;
    let bitmap = image::decode(&bytes).ok()?;
    let scaled = image::scale_to(&bitmap, SAMPLE_MAX_DIM);
    let mut pixels = Vec::with_capacity((scaled.width * scaled.height) as usize);
    for chunk in scaled.bgra.chunks_exact(4) {
        pixels.push(Rgb::new(chunk[2], chunk[1], chunk[0]));
    }
    Some((scaled.width, scaled.height, pixels))
}

pub fn seed_from_path(path: &Path) -> Option<Rgb> {
    let (w, h, pixels) = sample_grid(path)?;
    palette::seed_from_grid(&pixels, w, h)
}

pub fn artwork_scheme_from_path(path: &Path) -> Option<String> {
    let (w, h, pixels) = sample_grid(path)?;
    palette::extract_artwork_scheme_rgb(&pixels, w, h, Some(&path.to_string_lossy()))
        .map(|s| s.to_json().to_string())
}

fn current_wallpaper_path() -> Option<PathBuf> {
    for key in ["picture-uri", "picture-uri-dark"] {
        if let Ok(output) = Command::new("gsettings")
            .args(["get", "org.gnome.desktop.background", key])
            .output()
            && output.status.success()
        {
            let raw = String::from_utf8_lossy(&output.stdout);
            let trimmed = raw.trim().trim_matches('\'').trim_matches('"');
            let path_str = trimmed.strip_prefix("file://").unwrap_or(trimmed);
            if !path_str.is_empty() {
                let candidate = PathBuf::from(path_str);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}
