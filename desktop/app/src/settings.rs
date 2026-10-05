// SPDX-License-Identifier: GPL-3.0-or-later
//! The desktop app's own preferences (look and behavior), stored as JSON next
//! to the core's data. Device settings live in the core.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

const FILE_NAME: &str = "desktop-settings.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Bloom,
    Graphite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ColorMode {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub theme: Theme,
    /// Bloom seed preset, e.g. "honey" (see docs/design/tokens.json).
    pub seed: String,
    pub color_mode: ColorMode,
    /// Use the Windows 11 Mica backdrop.
    pub backdrop: bool,
    /// Closing the window keeps Nectarlink running in the tray.
    pub close_to_tray: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: Theme::default(),
            seed: "honey".into(),
            color_mode: ColorMode::default(),
            backdrop: true,
            close_to_tray: true,
        }
    }
}

impl Settings {
    fn path(dir: &Path) -> PathBuf {
        dir.join(FILE_NAME)
    }

    /// Loads the settings, falling back to defaults if the file is missing.
    /// A corrupt file is set aside (not deleted) so it can be inspected.
    pub fn load(dir: &Path) -> Settings {
        let path = Self::path(dir);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Settings::default(),
            Err(e) => {
                tracing::warn!(error = %e, "can't read settings; using defaults");
                return Settings::default();
            }
        };
        match serde_json::from_str(&text) {
            Ok(settings) => settings,
            Err(e) => {
                tracing::warn!(error = %e, "settings file is corrupt; using defaults");
                let _ = fs::rename(&path, path.with_extension("json.corrupt"));
                Settings::default()
            }
        }
    }

    /// Saves atomically: a crash mid-write never leaves a half-written file.
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        let path = Self::path(dir);
        let tmp = path.with_extension("json.tmp");
        {
            let mut file = fs::File::create(&tmp)?;
            file.write_all(serde_json::to_string_pretty(self)?.as_bytes())?;
            file.sync_all()?;
        }
        fs::rename(&tmp, &path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_defaults() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Settings::load(dir.path()), Settings::default());

        let settings = Settings {
            theme: Theme::Graphite,
            seed: "ocean".into(),
            color_mode: ColorMode::Dark,
            backdrop: false,
            close_to_tray: false,
        };
        settings.save(dir.path()).unwrap();
        assert_eq!(Settings::load(dir.path()), settings);
    }

    #[test]
    fn missing_fields_take_defaults_and_corrupt_files_are_set_aside() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(FILE_NAME), r#"{ "theme": "graphite" }"#).unwrap();
        let loaded = Settings::load(dir.path());
        assert_eq!(loaded.theme, Theme::Graphite);
        assert_eq!(loaded.seed, "honey");

        fs::write(dir.path().join(FILE_NAME), "{ not json").unwrap();
        assert_eq!(Settings::load(dir.path()), Settings::default());
        assert!(dir.path().join("desktop-settings.json.corrupt").exists());
    }
}
