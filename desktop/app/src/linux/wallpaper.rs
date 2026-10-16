// SPDX-License-Identifier: GPL-3.0-or-later
//! Desktop wallpaper seed color and Material You / Bloom scheme extraction on Linux.
//!
//! Discovers the current wallpaper across GNOME, Cinnamon, MATE, Budgie,
//! KDE Plasma, Hyprland (`hyprpaper`), Nitrogen, and `feh` (`~/.fehbg`),
//! downsamples it to [`palette::MAX_SAMPLE_SIDE`], and extracts perceptual
//! Material 3 tonal seeds and full artwork schemes.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::SystemTime;

use material_colors::color::Rgb;

use crate::palette;

use super::image;

const SAMPLE_MAX_DIM: u32 = palette::MAX_SAMPLE_SIDE;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Source {
    Image { path: PathBuf, modified: Option<SystemTime>, len: u64 },
    Solid(Rgb),
}

static LAST: Mutex<Option<(Source, Option<Rgb>)>> = Mutex::new(None);

/// Returns the Material You seed color for the current desktop wallpaper,
/// caching the result until the wallpaper file changes.
pub fn seed() -> Option<Rgb> {
    let source = current_source()?;
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((prev, cached)) = last.as_ref()
        && prev == &source
    {
        return *cached;
    }
    let computed = match &source {
        Source::Image { path, .. } => seed_from_path(path),
        Source::Solid(rgb) => palette::seed_from_pixels(&[*rgb]),
    };
    *last = Some((source, computed));
    computed
}

fn current_source() -> Option<Source> {
    if let Some(path) = current_wallpaper_path()
        && let Ok(meta) = fs::metadata(&path)
        && meta.is_file()
    {
        return Some(Source::Image { path, modified: meta.modified().ok(), len: meta.len() });
    }
    gnome_solid_color().map(Source::Solid)
}

fn sample_grid(path: &Path) -> Option<(u32, u32, Vec<Rgb>)> {
    let bytes = fs::read(path).ok()?;
    let bitmap = image::decode(&bytes).ok()?;
    let scaled = image::scale_to(&bitmap, SAMPLE_MAX_DIM);
    let mut pixels = Vec::with_capacity((scaled.width * scaled.height) as usize);
    for chunk in scaled.bgra.as_chunks::<4>().0 {
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

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) =
                ((bytes[i + 1] as char).to_digit(16), (bytes[i + 2] as char).to_digit(16))
        {
            out.push(((hi << 4) | lo) as u8);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn expand_tilde(path_str: &str) -> PathBuf {
    if let Some(rest) = path_str.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    if let Some(rest) = path_str.strip_prefix("$HOME/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    PathBuf::from(path_str)
}

/// Parses a `gsettings get` output value (such as `'file:///usr/share/backgrounds/my%20wall.png'`)
/// into a decoded local [`PathBuf`].
pub fn parse_gsettings_wallpaper_uri(raw: &str) -> Option<PathBuf> {
    let trimmed = raw
        .trim()
        .strip_prefix("@as ")
        .unwrap_or(raw.trim())
        .trim()
        .trim_matches('\'')
        .trim_matches('"')
        .trim();
    if trimmed.is_empty() || trimmed == "none" {
        return None;
    }
    let without_scheme = trimmed.strip_prefix("file://").unwrap_or(trimmed);
    let decoded = percent_decode(without_scheme);
    let cleaned = decoded.trim();
    if cleaned.is_empty() { None } else { Some(expand_tilde(cleaned)) }
}

/// Parses KDE Plasma's `plasma-org.kde.plasma.desktop-appletsrc` contents for `Image=` entries.
pub fn parse_kde_wallpaper_config(content: &str) -> Option<PathBuf> {
    let mut found: Option<PathBuf> = None;
    for line in content.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Image=")
            && let Some(path) = parse_gsettings_wallpaper_uri(rest)
            && path.extension().is_some()
        {
            found = Some(path);
        }
    }
    found
}

/// Parses `~/.fehbg` shell script contents to extract the wallpaper file path.
pub fn parse_fehbg_wallpaper(content: &str) -> Option<PathBuf> {
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if !line.contains("feh") {
            continue;
        }
        // Extract the last single- or double-quoted token, or the last whitespace token.
        if let Some(end_quote) = line.rfind('\'')
            && let Some(start_quote) = line[..end_quote].rfind('\'')
        {
            let token = line[start_quote + 1..end_quote].trim();
            if !token.is_empty() && !token.starts_with('-') {
                return Some(expand_tilde(token));
            }
        }
        if let Some(end_quote) = line.rfind('"')
            && let Some(start_quote) = line[..end_quote].rfind('"')
        {
            let token = line[start_quote + 1..end_quote].trim();
            if !token.is_empty() && !token.starts_with('-') {
                return Some(expand_tilde(token));
            }
        }
        if let Some(last) = line.split_whitespace().next_back() {
            let token = last.trim_matches('\'').trim_matches('"');
            if !token.is_empty() && !token.starts_with('-') && token != "feh" {
                return Some(expand_tilde(token));
            }
        }
    }
    None
}

/// Parses `~/.config/nitrogen/bg-saved.cfg` contents to extract the `file=` wallpaper path.
pub fn parse_nitrogen_wallpaper(content: &str) -> Option<PathBuf> {
    for line in content.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("file=") {
            let trimmed = rest.trim();
            if !trimmed.is_empty() {
                return Some(expand_tilde(trimmed));
            }
        }
    }
    None
}

/// Parses `~/.config/hypr/hyprpaper.conf` contents to extract the `wallpaper = monitor,path` entry.
pub fn parse_hyprpaper_wallpaper(content: &str) -> Option<PathBuf> {
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("wallpaper") {
            let rest = rest.trim_start();
            if let Some(rhs) = rest.strip_prefix('=') {
                let rhs = rhs.trim();
                let path_part = match rhs.split_once(',') {
                    Some((_, path)) => path.trim(),
                    None => rhs,
                };
                let unquoted = path_part.trim_matches('\'').trim_matches('"').trim();
                if !unquoted.is_empty() {
                    return Some(expand_tilde(unquoted));
                }
            }
        }
    }
    None
}

fn current_wallpaper_path() -> Option<PathBuf> {
    let dark = super::system_dark();
    let uri_keys =
        if dark { ["picture-uri-dark", "picture-uri"] } else { ["picture-uri", "picture-uri-dark"] };

    for schema in ["org.gnome.desktop.background", "org.cinnamon.desktop.background"] {
        for key in uri_keys {
            if let Ok(output) = Command::new("gsettings").args(["get", schema, key]).output()
                && output.status.success()
                && let Some(candidate) =
                    parse_gsettings_wallpaper_uri(&String::from_utf8_lossy(&output.stdout))
                && candidate.is_file()
            {
                return Some(candidate);
            }
        }
    }

    for key in ["picture-filename", "picture-uri"] {
        if let Ok(output) = Command::new("gsettings").args(["get", "org.mate.background", key]).output()
            && output.status.success()
            && let Some(candidate) = parse_gsettings_wallpaper_uri(&String::from_utf8_lossy(&output.stdout))
            && candidate.is_file()
        {
            return Some(candidate);
        }
    }

    if let Some(config) = dirs::config_dir() {
        let kde_rc = config.join("plasma-org.kde.plasma.desktop-appletsrc");
        if let Ok(content) = fs::read_to_string(kde_rc)
            && let Some(candidate) = parse_kde_wallpaper_config(&content)
            && candidate.is_file()
        {
            return Some(candidate);
        }

        let hypr_conf = config.join("hypr").join("hyprpaper.conf");
        if let Ok(content) = fs::read_to_string(hypr_conf)
            && let Some(candidate) = parse_hyprpaper_wallpaper(&content)
            && candidate.is_file()
        {
            return Some(candidate);
        }

        let nitrogen_cfg = config.join("nitrogen").join("bg-saved.cfg");
        if let Ok(content) = fs::read_to_string(nitrogen_cfg)
            && let Some(candidate) = parse_nitrogen_wallpaper(&content)
            && candidate.is_file()
        {
            return Some(candidate);
        }
    }

    if let Some(home) = dirs::home_dir() {
        let fehbg = home.join(".fehbg");
        if let Ok(content) = fs::read_to_string(fehbg)
            && let Some(candidate) = parse_fehbg_wallpaper(&content)
            && candidate.is_file()
        {
            return Some(candidate);
        }
    }

    None
}

fn gnome_solid_color() -> Option<Rgb> {
    let output = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.background", "primary-color"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_hex_rgb(&String::from_utf8_lossy(&output.stdout))
}

fn parse_hex_rgb(raw: &str) -> Option<Rgb> {
    let trimmed = raw.trim().trim_matches('\'').trim_matches('"').trim();
    let hex = trimmed.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Rgb::new(r, g, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gsettings_wallpaper_uri_with_quotes_and_percent_encoding() {
        assert_eq!(
            parse_gsettings_wallpaper_uri("'file:///usr/share/backgrounds/warty%20final%20ubuntu.png'"),
            Some(PathBuf::from("/usr/share/backgrounds/warty final ubuntu.png"))
        );
        assert_eq!(
            parse_gsettings_wallpaper_uri("\"/home/user/Pictures/wall.jpg\""),
            Some(PathBuf::from("/home/user/Pictures/wall.jpg"))
        );
        assert_eq!(parse_gsettings_wallpaper_uri("''"), None);
        assert_eq!(parse_gsettings_wallpaper_uri("'none'"), None);
    }

    #[test]
    fn parses_kde_plasma_wallpaper_config() {
        let sample = "\
[Containments][1][Wallpaper][org.kde.image][General]
Image=file:///usr/share/wallpapers/Next/contents/images/5120x2880.png
SlidePaths=/usr/share/wallpapers
";
        assert_eq!(
            parse_kde_wallpaper_config(sample),
            Some(PathBuf::from("/usr/share/wallpapers/Next/contents/images/5120x2880.png"))
        );
    }

    #[test]
    fn parses_fehbg_nitrogen_and_hyprpaper_configs() {
        let fehbg = "#!/bin/sh\nfeh --no-fehbg --bg-fill '/home/alice/wallpapers/forest lake.png' \n";
        assert_eq!(
            parse_fehbg_wallpaper(fehbg),
            Some(PathBuf::from("/home/alice/wallpapers/forest lake.png"))
        );

        let nitrogen = "[xin_-1]\nfile=/home/alice/Pictures/sunset.jpg\nmode=4\nbgcolor=#000000\n";
        assert_eq!(
            parse_nitrogen_wallpaper(nitrogen),
            Some(PathBuf::from("/home/alice/Pictures/sunset.jpg"))
        );

        let hyprpaper = "\
preload = /home/alice/wall.png
# monitor wallpaper
wallpaper = DP-1, /home/alice/wall.png
";
        assert_eq!(parse_hyprpaper_wallpaper(hyprpaper), Some(PathBuf::from("/home/alice/wall.png")));
    }

    #[test]
    fn extracts_seed_and_artwork_scheme_from_png_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("vibrant.png");
        let width = 32u32;
        let height = 32u32;
        let mut bgra = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..(width * height) {
            // Vibrant cobalt blue in BGRA order
            bgra.extend_from_slice(&[220, 90, 30, 255]);
        }
        let png = image::encode_png(&image::Bitmap { width, height, bgra }).expect("encode_png");
        fs::write(&path, png).expect("write png");

        let seed = seed_from_path(&path).expect("seed_from_path");
        assert!(seed.blue > seed.red);

        let scheme_json = artwork_scheme_from_path(&path).expect("artwork_scheme_from_path");
        assert!(scheme_json.contains("primary"));
    }
}
