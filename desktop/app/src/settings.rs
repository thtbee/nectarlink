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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RecordingFormat {
    #[default]
    M4a,
    Mp3,
    Wav,
    Flac,
}

impl RecordingFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            RecordingFormat::M4a => "m4a",
            RecordingFormat::Mp3 => "mp3",
            RecordingFormat::Wav => "wav",
            RecordingFormat::Flac => "flac",
        }
    }

    pub fn from_str_lossy(s: &str) -> RecordingFormat {
        match s.trim().to_ascii_lowercase().as_str() {
            "mp3" => RecordingFormat::Mp3,
            "wav" => RecordingFormat::Wav,
            "flac" => RecordingFormat::Flac,
            _ => RecordingFormat::M4a,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub theme: Theme,
    /// Bloom colors: "wallpaper" (from the desktop wallpaper) or a preset,
    /// e.g. "honey" (see docs/design/tokens.json).
    pub seed: String,
    pub color_mode: ColorMode,
    /// Use the Windows 11 Mica backdrop.
    pub backdrop: bool,
    /// Closing the window keeps Nectarlink running in the tray.
    pub close_to_tray: bool,
    /// Send what's copied on this PC to connected phones.
    pub auto_clipboard: bool,
    /// Keep the last 50 synced clips in encrypted local clipboard history.
    pub clipboard_history: bool,
    /// Suggest context chips (Open, Open in Maps, Call, Track, Email) for copied text.
    pub suggest_clipboard_actions: bool,
    /// Copy one-time codes from phone notifications and SMS automatically.
    pub auto_copy_otp: bool,
    /// Pause PC media playback while a phone call is ringing or active.
    pub pause_media_on_call: bool,
    /// Quiet phone notification pop-ups on the PC while the phone is in Do Not Disturb.
    pub sync_dnd: bool,
    /// Paired phones in Explorer's "Send to" menu.
    pub send_to_menu: bool,
    /// Check for updates on its own (installed copies).
    pub auto_update: bool,
    /// Tell when a phone's battery is low, or full.
    pub battery_alerts: bool,
    /// Start when the user signs in. Unset until the user chooses: then an
    /// installed copy starts with Windows and a build run from its folder
    /// doesn't.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_with_windows: Option<bool>,
    /// Where voice recordings from phones are saved (`None` = `Documents\Nectarlink Recordings`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recordings_folder: Option<PathBuf>,
    /// Output format for saved voice recordings (`m4a`, `mp3`, `wav`, or `flac`).
    pub recordings_format: RecordingFormat,
    /// Preferred phone for the webcam feature (`None` = first paired phone).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webcam_phone: Option<String>,
    /// Webcam output resolution height (`720` or `1080`).
    pub webcam_height: u32,
    /// Horizontally mirror the webcam video.
    pub webcam_mirror: bool,
    /// Days to keep timeline entries before auto-purging (`0` = keep up to max entries).
    pub timeline_retention_days: u32,
    /// Global hotkey to trigger "Take photo with phone" (e.g. `"Ctrl+Alt+C"`, or `""` to disable).
    pub continuity_photo_hotkey: String,
    /// Global hotkey to trigger "Scan document with phone" (e.g. `"Ctrl+Alt+D"`, or `""` to disable).
    pub continuity_scan_hotkey: String,
    /// Global hotkey to open the Command Palette (e.g. `"Ctrl+Alt+Space"`, or `""` to disable).
    pub command_palette_hotkey: String,
    /// Global hotkey to slide out the Shelf panel at the screen edge (e.g. `"Ctrl+Alt+S"`, or `""` to disable).
    pub shelf_hotkey: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: Theme::default(),
            seed: "wallpaper".into(),
            color_mode: ColorMode::default(),
            backdrop: true,
            close_to_tray: true,
            auto_clipboard: true,
            clipboard_history: true,
            suggest_clipboard_actions: true,
            auto_copy_otp: false,
            pause_media_on_call: true,
            sync_dnd: false,
            send_to_menu: true,
            auto_update: true,
            battery_alerts: true,
            start_with_windows: None,
            recordings_folder: None,
            recordings_format: RecordingFormat::default(),
            webcam_phone: None,
            webcam_height: 720,
            webcam_mirror: false,
            timeline_retention_days: 90,
            continuity_photo_hotkey: "Ctrl+Alt+C".into(),
            continuity_scan_hotkey: "Ctrl+Alt+D".into(),
            command_palette_hotkey: "Ctrl+Alt+Space".into(),
            shelf_hotkey: "Ctrl+Alt+S".into(),
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
        let def = Settings::load(dir.path());
        assert_eq!(def, Settings::default());
        assert_eq!(def.command_palette_hotkey, "Ctrl+Alt+Space");
        assert_eq!(def.shelf_hotkey, "Ctrl+Alt+S");

        let settings = Settings {
            theme: Theme::Graphite,
            seed: "ocean".into(),
            color_mode: ColorMode::Dark,
            backdrop: false,
            close_to_tray: false,
            auto_clipboard: false,
            clipboard_history: false,
            suggest_clipboard_actions: false,
            auto_copy_otp: true,
            pause_media_on_call: false,
            sync_dnd: true,
            send_to_menu: false,
            auto_update: false,
            battery_alerts: false,
            start_with_windows: Some(true),
            recordings_folder: Some(PathBuf::from(r"C:\Recordings")),
            recordings_format: RecordingFormat::Flac,
            webcam_phone: Some("0101".repeat(16)),
            webcam_height: 1080,
            webcam_mirror: true,
            timeline_retention_days: 30,
            continuity_photo_hotkey: "Ctrl+Alt+P".into(),
            continuity_scan_hotkey: "Ctrl+Alt+S".into(),
            command_palette_hotkey: "Ctrl+Shift+Space".into(),
            shelf_hotkey: "Ctrl+Alt+E".into(),
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
        assert_eq!(loaded.seed, "wallpaper");

        fs::write(dir.path().join(FILE_NAME), "{ not json").unwrap();
        assert_eq!(Settings::load(dir.path()), Settings::default());
        assert!(dir.path().join("desktop-settings.json.corrupt").exists());
    }
}
