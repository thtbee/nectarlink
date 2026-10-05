// SPDX-License-Identifier: GPL-3.0-or-later
//! App icons that phones send with their notifications, kept as PNG files
//! (`<data>\cache\icons`) so QML and Windows toasts can show them.

use std::{
    fs,
    path::{Path, PathBuf},
};

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

pub fn icons_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("cache").join("icons")
}

/// A file name for a package: package names are already safe
/// (`[A-Za-z0-9._]`); anything else is replaced.
fn file_name(app: &str) -> String {
    let safe: String = app
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '_' { c } else { '_' })
        .collect();
    let safe = safe.trim_matches('.');
    format!("{}.png", if safe.is_empty() { "_" } else { safe })
}

/// Stores an app's icon and returns its path. Only PNG data is accepted.
pub fn save(data_dir: &Path, app: &str, png: &[u8]) -> std::io::Result<PathBuf> {
    if !png.starts_with(PNG_SIGNATURE) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "not a PNG"));
    }
    let dir = icons_dir(data_dir);
    fs::create_dir_all(&dir)?;
    let path = dir.join(file_name(app));
    if fs::read(&path).is_ok_and(|existing| existing == png) {
        return Ok(path);
    }
    let tmp = path.with_extension("png.tmp");
    fs::write(&tmp, png)?;
    fs::rename(&tmp, &path)?;
    Ok(path)
}

/// A `file:///` URL for QML and toasts.
pub fn file_url(path: &Path) -> String {
    format!("file:///{}", path.to_string_lossy().replace('\\', "/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_png_icons_under_safe_names() {
        let dir = tempfile::tempdir().unwrap();
        let png = [PNG_SIGNATURE, b"rest"].concat();
        let path = save(dir.path(), "com.whatsapp", &png).unwrap();
        assert_eq!(path.file_name().unwrap(), "com.whatsapp.png");
        assert_eq!(fs::read(&path).unwrap(), png);
        // Saving the same icon again is a no-op; a new one replaces it.
        assert_eq!(save(dir.path(), "com.whatsapp", &png).unwrap(), path);

        assert_eq!(file_name("../../evil\\x"), "_.._evil_x.png");
        assert!(save(dir.path(), "com.bad", b"GIF89a").is_err(), "only PNG");
        assert!(file_url(Path::new(r"C:\a\b.png")).starts_with("file:///C:/a/b.png"));
    }
}
