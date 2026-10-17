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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Bloom,
    /// Honey on charcoal or wax, with the living comb.
    Hive,
    Glass,
    Graphite,
}

impl<'de> Deserialize<'de> for Theme {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer).unwrap_or_default();
        Ok(match s.trim().to_ascii_lowercase().as_str() {
            "hive" => Theme::Hive,
            "graphite" => Theme::Graphite,
            "glass" | "blurry" => Theme::Glass,
            _ => Theme::Bloom,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ColorMode {
    #[default]
    System,
    Light,
    Dark,
}

impl<'de> Deserialize<'de> for ColorMode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer).unwrap_or_default();
        Ok(match s.trim().to_ascii_lowercase().as_str() {
            "light" => ColorMode::Light,
            "dark" => ColorMode::Dark,
            _ => ColorMode::System,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RecordingFormat {
    #[default]
    M4a,
    Mp3,
    Wav,
    Flac,
}

impl<'de> Deserialize<'de> for RecordingFormat {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer).unwrap_or_default();
        Ok(RecordingFormat::from_str_lossy(&s))
    }
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

pub const DEFAULT_LOCAL_API_PORT: u16 = 53320;
pub const DEFAULT_WINDOW_WIDTH: u32 = 1120;
pub const DEFAULT_WINDOW_HEIGHT: u32 = 740;
pub const MIN_WINDOW_WIDTH: u32 = 900;
pub const MIN_WINDOW_HEIGHT: u32 = 620;

fn default_local_api_port() -> u16 {
    DEFAULT_LOCAL_API_PORT
}

fn default_window_width() -> u32 {
    DEFAULT_WINDOW_WIDTH
}

fn default_window_height() -> u32 {
    DEFAULT_WINDOW_HEIGHT
}

fn default_true() -> bool {
    true
}

fn default_webcam_camera() -> String {
    "back".to_owned()
}

fn normalize_webcam_camera(cam: &str) -> String {
    if cam.trim().eq_ignore_ascii_case("front") { "front".to_owned() } else { "back".to_owned() }
}

static CLIPBOARD_AUTO_COPY_FILES: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
static NECTAR_ISLAND: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static STREAM_PC_AUDIO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static SHARE_PC_STORAGE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static FORWARD_PC_NOTIFICATIONS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static AWAY_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static FLOW_FOLLOW_CLIPBOARD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
static OPEN_SHARED_FILES: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

static FOLDER_SYNC_RULES: std::sync::OnceLock<std::sync::Mutex<Vec<crate::folder_sync::FolderSyncRule>>> =
    std::sync::OnceLock::new();
static AUTOMATION_RULES: std::sync::OnceLock<std::sync::Mutex<Vec<crate::automations::AutomationRule>>> =
    std::sync::OnceLock::new();
type WindowGeometryState = (Option<i32>, Option<i32>, u32, u32, bool);
static WINDOW_GEOMETRY: std::sync::Mutex<WindowGeometryState> =
    std::sync::Mutex::new((None, None, DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT, false));

pub fn clipboard_auto_copy_received_files() -> bool {
    CLIPBOARD_AUTO_COPY_FILES.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_clipboard_auto_copy_received_files(on: bool) {
    CLIPBOARD_AUTO_COPY_FILES.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Whether a single file shared from the phone opens in its default app on arrival.
pub fn open_shared_files() -> bool {
    OPEN_SHARED_FILES.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_open_shared_files(on: bool) {
    OPEN_SHARED_FILES.store(on, std::sync::atomic::Ordering::Relaxed);
}

#[allow(dead_code)]
pub fn nectar_island() -> bool {
    NECTAR_ISLAND.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_nectar_island(on: bool) {
    NECTAR_ISLAND.store(on, std::sync::atomic::Ordering::Relaxed);
}

#[allow(dead_code)]
pub fn stream_pc_audio_to_phone() -> bool {
    STREAM_PC_AUDIO.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_stream_pc_audio_to_phone(on: bool) {
    STREAM_PC_AUDIO.store(on, std::sync::atomic::Ordering::Relaxed);
    crate::pc_audio_stream::set_enabled(on);
    crate::deck::broadcast_live_state();
}

pub fn share_pc_storage() -> bool {
    SHARE_PC_STORAGE.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_share_pc_storage(on: bool) {
    SHARE_PC_STORAGE.store(on, std::sync::atomic::Ordering::Relaxed);
    if on
        && let Some(node) = crate::core_host::node()
        && let Ok(devices) = node.paired_devices()
    {
        for d in devices {
            let _ = node.set_device_toggle(d.id, "storage", true);
        }
    }
}

pub fn forward_pc_notifications_to_phone() -> bool {
    FORWARD_PC_NOTIFICATIONS.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_forward_pc_notifications_to_phone(on: bool) {
    FORWARD_PC_NOTIFICATIONS.store(on, std::sync::atomic::Ordering::Relaxed);
    #[cfg(target_os = "linux")]
    if on {
        crate::pc_notifications::start_listener();
    }
}

/// Whether the PC's clipboard goes to the phone when control moves to it (`input_share`).
pub fn flow_follow_clipboard() -> bool {
    FLOW_FOLLOW_CLIPBOARD.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_flow_follow_clipboard(on: bool) {
    FLOW_FOLLOW_CLIPBOARD.store(on, std::sync::atomic::Ordering::Relaxed);
}

#[allow(dead_code)]
pub fn away_mode() -> bool {
    AWAY_MODE.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_away_mode(on: bool) {
    AWAY_MODE.store(on, std::sync::atomic::Ordering::Relaxed);
    if let Some(node) = crate::core_host::node() {
        node.set_away_mode(on);
    }
}

static OBS: std::sync::Mutex<(u16, Option<String>)> = std::sync::Mutex::new((crate::obs::DEFAULT_PORT, None));

/// OBS port and encrypted password, as stored.
pub fn obs_settings() -> (u16, Option<String>) {
    OBS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

pub fn set_obs_settings(port: u16, protected_password: Option<String>) {
    *OBS.lock().unwrap_or_else(|e| e.into_inner()) = (port, protected_password);
}

/// Encrypts an OBS password for storage (DPAPI on Windows, base64 for the file).
pub fn protect_obs_password(password: &str) -> Option<String> {
    let sealed = nectarlink_core::default_protector().protect(password.as_bytes()).ok()?;
    Some(data_encoding::BASE64.encode(&sealed))
}

/// The OBS password in clear, for one connection; `None` when unset or unreadable.
pub fn obs_password() -> Option<String> {
    let stored = obs_settings().1?;
    let sealed = data_encoding::BASE64.decode(stored.as_bytes()).ok()?;
    let plain = nectarlink_core::default_protector().unprotect(&sealed).ok()?;
    String::from_utf8(plain).ok()
}

pub fn folder_sync_rules() -> Vec<crate::folder_sync::FolderSyncRule> {
    FOLDER_SYNC_RULES
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

pub fn set_folder_sync_rules(rules: Vec<crate::folder_sync::FolderSyncRule>) {
    *FOLDER_SYNC_RULES
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = rules;
}

pub fn automation_rules() -> Vec<crate::automations::AutomationRule> {
    AUTOMATION_RULES
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

pub fn set_automation_rules(rules: Vec<crate::automations::AutomationRule>) {
    *AUTOMATION_RULES
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = rules;
}

pub fn window_geometry() -> (Option<i32>, Option<i32>, u32, u32, bool) {
    *WINDOW_GEOMETRY.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn set_window_geometry(x: Option<i32>, y: Option<i32>, width: u32, height: u32, maximized: bool) {
    *WINDOW_GEOMETRY.lock().unwrap_or_else(|e| e.into_inner()) =
        (x, y, width.max(MIN_WINDOW_WIDTH), height.max(MIN_WINDOW_HEIGHT), maximized);
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub theme: Theme,
    /// Bloom colors: "wallpaper" (from the desktop wallpaper) or a preset,
    /// e.g. "honey" (see docs/design/tokens.json).
    #[serde(alias = "accent")]
    pub seed: String,
    #[serde(alias = "color_mode")]
    pub color_mode: ColorMode,
    /// Pure #000000 OLED black surfaces when in dark mode.
    #[serde(default)]
    pub amoled: bool,
    /// Use the Windows 11 Mica backdrop.
    pub backdrop: bool,
    /// Hive's comb breathes and pulses (it holds still when off, and with
    /// reduced motion or battery saver whatever this says).
    #[serde(default = "default_true", alias = "comb_motion")]
    pub comb_motion: bool,
    /// Closing the window keeps NectarLink running in the tray.
    #[serde(alias = "close_to_tray")]
    pub close_to_tray: bool,
    /// Send what's copied on this PC to connected phones.
    #[serde(alias = "auto_clipboard", alias = "clipboard_sync")]
    pub auto_clipboard: bool,
    /// Keep the last 50 synced clips in encrypted local clipboard history.
    #[serde(alias = "clipboard_history")]
    pub clipboard_history: bool,
    /// Suggest context chips (Open, Open in Maps, Call, Track, Email) for copied text.
    #[serde(alias = "suggest_clipboard_actions")]
    pub suggest_clipboard_actions: bool,
    /// Copy one-time codes from phone notifications and SMS automatically.
    #[serde(alias = "auto_copy_otp", alias = "smart_otp_auto_copy")]
    pub auto_copy_otp: bool,
    /// Pause PC media playback while a phone call is ringing or active.
    #[serde(alias = "pause_media_on_call", alias = "auto_pause_media")]
    pub pause_media_on_call: bool,
    /// Quiet phone notification pop-ups on the PC while the phone is in Do Not Disturb.
    #[serde(alias = "sync_dnd")]
    pub sync_dnd: bool,
    /// Paired phones in Explorer's "Send to" menu.
    #[serde(alias = "send_to_menu")]
    pub send_to_menu: bool,
    /// Check for updates on its own (installed copies).
    #[serde(alias = "auto_update")]
    pub auto_update: bool,
    /// Install a newer release by itself at a quiet moment (with `auto_update`).
    #[serde(alias = "install_updates")]
    pub install_updates: bool,
    /// Tell when a phone's battery is low, or full.
    #[serde(alias = "battery_alerts")]
    pub battery_alerts: bool,
    /// Start when the user signs in. Unset until the user chooses: then an
    /// installed copy starts with Windows and a build run from its folder
    /// doesn't.
    #[serde(skip_serializing_if = "Option::is_none", alias = "start_with_windows")]
    pub start_with_windows: Option<bool>,
    /// Where voice recordings from phones are saved (`None` = `Documents\Nectarlink Recordings`).
    #[serde(skip_serializing_if = "Option::is_none", alias = "recordings_folder")]
    pub recordings_folder: Option<PathBuf>,
    /// Output format for saved voice recordings (`m4a`, `mp3`, `wav`, or `flac`).
    #[serde(alias = "recordings_format")]
    pub recordings_format: RecordingFormat,
    /// Preferred phone for the webcam feature (`None` = first paired phone).
    #[serde(skip_serializing_if = "Option::is_none", alias = "webcam_phone")]
    pub webcam_phone: Option<String>,
    /// Webcam output resolution height (`720`, `1080`, or `2160`).
    #[serde(alias = "webcam_height")]
    pub webcam_height: u32,
    /// Horizontally mirror the webcam video.
    #[serde(alias = "webcam_mirror")]
    pub webcam_mirror: bool,
    /// Which phone camera lens to stream (`"back"` or `"front"`).
    #[serde(default = "default_webcam_camera", alias = "webcam_camera")]
    pub webcam_camera: String,
    /// Days to keep timeline entries before auto-purging (`0` = keep up to max entries).
    #[serde(alias = "timeline_retention_days")]
    pub timeline_retention_days: u32,
    /// Global hotkey to trigger "Take photo with phone" (e.g. `"Ctrl+Alt+C"`, or `""` to disable).
    #[serde(alias = "continuity_photo_hotkey")]
    pub continuity_photo_hotkey: String,
    /// Global hotkey to trigger "Scan document with phone" (e.g. `"Ctrl+Alt+D"`, or `""` to disable).
    #[serde(alias = "continuity_scan_hotkey")]
    pub continuity_scan_hotkey: String,
    /// Global hotkey to open the Command Palette (e.g. `"Ctrl+Alt+Space"`, or `""` to disable).
    #[serde(alias = "command_palette_hotkey")]
    pub command_palette_hotkey: String,
    /// Global hotkey to slide out the Shelf panel at the screen edge (e.g. `"Ctrl+Alt+S"`, or `""` to disable).
    #[serde(alias = "shelf_hotkey")]
    pub shelf_hotkey: String,
    /// Whether the localhost HTTP/JSON automation API (`127.0.0.1:<local_api_port>`) is enabled (off by default).
    #[serde(default, alias = "local_api_enabled")]
    pub local_api_enabled: bool,
    /// TCP port on `127.0.0.1` for the localhost automation API.
    #[serde(default = "default_local_api_port", alias = "local_api_port")]
    pub local_api_port: u16,
    /// Automatically place small incoming received files onto the Windows clipboard (`CF_HDROP`).
    #[serde(default = "default_true", alias = "clipboard_auto_copy_received_files")]
    pub clipboard_auto_copy_received_files: bool,
    /// Open a single photo or document shared from the phone as soon as it
    /// arrives. Off by default: it's saved in Downloads\Nectarlink either way.
    #[serde(default, alias = "open_shared_files")]
    pub open_shared_files: bool,
    /// Send the PC's clipboard to the phone when control moves to it: the mouse
    /// enters the phone's window, or a remote control session starts.
    #[serde(default = "default_true", alias = "flow_follow_clipboard")]
    pub flow_follow_clipboard: bool,
    /// Configured one-way / two-way folder synchronization rules.
    #[serde(default, skip_serializing_if = "Vec::is_empty", alias = "folder_sync_rules")]
    pub folder_sync_rules: Vec<crate::folder_sync::FolderSyncRule>,
    /// Show the optional top-of-screen Nectar Island status pill when phone activity is ongoing.
    #[serde(default, alias = "nectar_island")]
    pub nectar_island: bool,
    /// Trigger Nectar Island for incoming and active phone calls.
    #[serde(default = "default_true", alias = "nectar_island_calls")]
    pub nectar_island_calls: bool,
    /// Trigger Nectar Island for active file transfers.
    #[serde(default = "default_true", alias = "nectar_island_transfers")]
    pub nectar_island_transfers: bool,
    /// Trigger Nectar Island for active media playback.
    #[serde(default = "default_true", alias = "nectar_island_media")]
    pub nectar_island_media: bool,
    /// Trigger Nectar Island for phone notifications.
    #[serde(default, alias = "nectar_island_notifications")]
    pub nectar_island_notifications: bool,
    /// Configured "When X, Do Y" automation rules.
    #[serde(default, skip_serializing_if = "Vec::is_empty", alias = "automation_rules")]
    pub automation_rules: Vec<crate::automations::AutomationRule>,
    /// Stream PC system audio to the connected phone (Phone as Speaker / Headphones).
    #[serde(default, alias = "stream_pc_audio_to_phone")]
    pub stream_pc_audio_to_phone: bool,
    /// Expose PC user folders (`Downloads`, `Documents`, `Desktop`, `Pictures`) to paired phones.
    #[serde(default, alias = "share_pc_storage")]
    pub share_pc_storage: bool,
    /// Forward PC notifications to connected phones (privacy-first: off by default).
    #[serde(default, alias = "forward_pc_notifications_to_phone")]
    pub forward_pc_notifications_to_phone: bool,
    /// Allow staying connected over mobile data (LTE/5G) / relay when away from Wi-Fi.
    #[serde(default, alias = "away_mode")]
    pub away_mode: bool,
    /// Whether the first-run setup walkthrough has been completed or skipped.
    #[serde(default, alias = "walkthrough_completed")]
    pub walkthrough_completed: bool,
    /// Current step index (0..=6) in the first-run setup walkthrough for resuming after a restart.
    #[serde(default, alias = "walkthrough_step")]
    pub walkthrough_step: u8,
    /// Whether the gentle "What's new" card on Home has been dismissed.
    #[serde(default, alias = "whats_new_dismissed")]
    pub whats_new_dismissed: bool,
    /// Whether the card on Home inviting to try Hive has been answered.
    #[serde(default, alias = "hive_invite_dismissed")]
    pub hive_invite_dismissed: bool,
    /// OBS Studio's WebSocket port, for `obs:` Deck commands.
    pub obs_port: u16,
    /// OBS's WebSocket password, encrypted with Windows DPAPI (base64). Never stored in clear.
    pub obs_password: Option<String>,
    /// Last saved main window X coordinate in screen pixels (`None` = OS/centered default).
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "window_x")]
    pub window_x: Option<i32>,
    /// Last saved main window Y coordinate in screen pixels (`None` = OS/centered default).
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "window_y")]
    pub window_y: Option<i32>,
    /// Last saved main window normal width in logical pixels.
    #[serde(default = "default_window_width", alias = "window_width")]
    pub window_width: u32,
    /// Last saved main window normal height in logical pixels.
    #[serde(default = "default_window_height", alias = "window_height")]
    pub window_height: u32,
    /// Whether the main window was maximized when last open.
    #[serde(default, alias = "window_maximized")]
    pub window_maximized: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: Theme::default(),
            seed: "wallpaper".into(),
            color_mode: ColorMode::default(),
            amoled: false,
            backdrop: true,
            comb_motion: true,
            close_to_tray: true,
            auto_clipboard: true,
            clipboard_history: true,
            suggest_clipboard_actions: true,
            auto_copy_otp: false,
            pause_media_on_call: true,
            sync_dnd: false,
            send_to_menu: true,
            auto_update: true,
            install_updates: true,
            battery_alerts: true,
            start_with_windows: None,
            recordings_folder: None,
            recordings_format: RecordingFormat::default(),
            webcam_phone: None,
            webcam_height: 720,
            webcam_mirror: false,
            webcam_camera: default_webcam_camera(),
            timeline_retention_days: 90,
            continuity_photo_hotkey: "Ctrl+Alt+C".into(),
            continuity_scan_hotkey: "Ctrl+Alt+D".into(),
            command_palette_hotkey: "Ctrl+Alt+Space".into(),
            shelf_hotkey: "Ctrl+Alt+S".into(),
            local_api_enabled: false,
            local_api_port: default_local_api_port(),
            clipboard_auto_copy_received_files: true,
            open_shared_files: false,
            flow_follow_clipboard: true,
            folder_sync_rules: Vec::new(),
            nectar_island: false,
            nectar_island_calls: true,
            nectar_island_transfers: true,
            nectar_island_media: true,
            nectar_island_notifications: false,
            automation_rules: Vec::new(),
            stream_pc_audio_to_phone: false,
            share_pc_storage: false,
            forward_pc_notifications_to_phone: false,
            away_mode: false,
            walkthrough_completed: false,
            walkthrough_step: 0,
            whats_new_dismissed: false,
            hive_invite_dismissed: false,
            obs_port: crate::obs::DEFAULT_PORT,
            obs_password: None,
            window_x: None,
            window_y: None,
            window_width: DEFAULT_WINDOW_WIDTH,
            window_height: DEFAULT_WINDOW_HEIGHT,
            window_maximized: false,
        }
    }
}

impl Settings {
    fn path(dir: &Path) -> PathBuf {
        dir.join(FILE_NAME)
    }

    pub fn sanitized(mut self) -> Self {
        self.webcam_camera = normalize_webcam_camera(&self.webcam_camera);
        self.walkthrough_step = self.walkthrough_step.min(6);
        self.window_width = self.window_width.clamp(MIN_WINDOW_WIDTH, 7680);
        self.window_height = self.window_height.clamp(MIN_WINDOW_HEIGHT, 4320);
        if self.window_x.is_none() || self.window_y.is_none() {
            self.window_x = None;
            self.window_y = None;
        }
        self
    }

    /// Auto-completes the first-run walkthrough when upgrading from an earlier build
    /// that already has at least one paired phone (`paired_count > 0`) and has not
    /// begun stepping through the walkthrough (`walkthrough_step == 0`).
    pub fn reconcile_walkthrough_for_paired_devices(&mut self, paired_count: usize) -> bool {
        if !self.walkthrough_completed && self.walkthrough_step == 0 && paired_count > 0 {
            self.walkthrough_completed = true;
            true
        } else {
            false
        }
    }

    pub fn apply_runtime(&self) {
        set_clipboard_auto_copy_received_files(self.clipboard_auto_copy_received_files);
        set_open_shared_files(self.open_shared_files);
        set_flow_follow_clipboard(self.flow_follow_clipboard);
        set_folder_sync_rules(self.folder_sync_rules.clone());
        set_obs_settings(
            if self.obs_port == 0 { crate::obs::DEFAULT_PORT } else { self.obs_port },
            self.obs_password.clone(),
        );
        set_nectar_island(self.nectar_island);
        set_automation_rules(self.automation_rules.clone());
        set_stream_pc_audio_to_phone(self.stream_pc_audio_to_phone);
        set_share_pc_storage(self.share_pc_storage);
        set_forward_pc_notifications_to_phone(self.forward_pc_notifications_to_phone);
        set_away_mode(self.away_mode);
        set_window_geometry(
            self.window_x,
            self.window_y,
            self.window_width,
            self.window_height,
            self.window_maximized,
        );
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
        match serde_json::from_str::<Settings>(&text) {
            Ok(settings) => settings.sanitized(),
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
        assert!(!def.local_api_enabled);
        assert_eq!(def.local_api_port, 53320);
        def.apply_runtime();
        assert!(flow_follow_clipboard());
        assert!(!nectar_island());
        assert!(!stream_pc_audio_to_phone());

        let settings = Settings {
            theme: Theme::Graphite,
            seed: "ocean".into(),
            color_mode: ColorMode::Dark,
            amoled: false,
            backdrop: false,
            comb_motion: false,
            close_to_tray: false,
            auto_clipboard: false,
            clipboard_history: false,
            suggest_clipboard_actions: false,
            auto_copy_otp: true,
            pause_media_on_call: false,
            sync_dnd: true,
            send_to_menu: false,
            auto_update: false,
            install_updates: false,
            battery_alerts: false,
            start_with_windows: Some(true),
            recordings_folder: Some(PathBuf::from(r"C:\Recordings")),
            recordings_format: RecordingFormat::Flac,
            webcam_phone: Some("0101".repeat(16)),
            webcam_height: 2160,
            webcam_mirror: true,
            webcam_camera: "front".into(),
            timeline_retention_days: 30,
            continuity_photo_hotkey: "Ctrl+Alt+P".into(),
            continuity_scan_hotkey: "Ctrl+Alt+S".into(),
            command_palette_hotkey: "Ctrl+Shift+Space".into(),
            shelf_hotkey: "Ctrl+Alt+E".into(),
            local_api_enabled: true,
            local_api_port: 54321,
            clipboard_auto_copy_received_files: false,
            open_shared_files: true,
            flow_follow_clipboard: false,
            folder_sync_rules: Vec::new(),
            nectar_island: true,
            nectar_island_calls: true,
            nectar_island_transfers: true,
            nectar_island_media: true,
            nectar_island_notifications: false,
            automation_rules: Vec::new(),
            stream_pc_audio_to_phone: true,
            share_pc_storage: true,
            forward_pc_notifications_to_phone: true,
            away_mode: true,
            walkthrough_completed: true,
            walkthrough_step: 4,
            whats_new_dismissed: true,
            hive_invite_dismissed: true,
            obs_port: crate::obs::DEFAULT_PORT,
            obs_password: None,
            window_x: Some(180),
            window_y: Some(120),
            window_width: 1280,
            window_height: 820,
            window_maximized: true,
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

    #[test]
    fn loads_every_historical_settings_json_from_git_history() {
        let dir = tempfile::tempdir().unwrap();

        // 1. Commit d16a106 (initial desktop app: theme, seed, colorMode, backdrop, closeToTray)
        let snap_d16a106 = r#"{
            "theme": "bloom",
            "seed": "honey",
            "colorMode": "dark",
            "backdrop": false,
            "closeToTray": false
        }"#;
        fs::write(dir.path().join(FILE_NAME), snap_d16a106).unwrap();
        let s = Settings::load(dir.path());
        assert_eq!(s.theme, Theme::Bloom);
        assert_eq!(s.seed, "honey");
        assert_eq!(s.color_mode, ColorMode::Dark);
        assert!(!s.backdrop);
        assert!(!s.close_to_tray);
        assert!(s.auto_clipboard);
        assert!(!s.local_api_enabled);
        assert_eq!(s.local_api_port, 53320);

        // 2. Commit 351fa06 (+ autoClipboard) & c1a04dd (+ sendToMenu) & 4659bd9 (+ startWithWindows)
        let snap_4659bd9 = r#"{
            "theme": "graphite",
            "seed": "wallpaper",
            "colorMode": "light",
            "backdrop": true,
            "closeToTray": true,
            "autoClipboard": false,
            "sendToMenu": false,
            "startWithWindows": true
        }"#;
        fs::write(dir.path().join(FILE_NAME), snap_4659bd9).unwrap();
        let s = Settings::load(dir.path());
        assert_eq!(s.theme, Theme::Graphite);
        assert_eq!(s.color_mode, ColorMode::Light);
        assert!(!s.auto_clipboard);
        assert!(!s.send_to_menu);
        assert_eq!(s.start_with_windows, Some(true));

        // 3. Commit 636da6f (+ autoUpdate) & 7a26adf (+ batteryAlerts) & f287fb8 (+ recordingsFolder, recordingsFormat)
        let snap_f287fb8 = r#"{
            "theme": "bloom",
            "seed": "sage",
            "colorMode": "system",
            "backdrop": true,
            "closeToTray": true,
            "autoClipboard": true,
            "sendToMenu": true,
            "autoUpdate": false,
            "batteryAlerts": false,
            "recordingsFolder": "C:\\Users\\Test\\Recordings",
            "recordingsFormat": "flac"
        }"#;
        fs::write(dir.path().join(FILE_NAME), snap_f287fb8).unwrap();
        let s = Settings::load(dir.path());
        assert!(!s.auto_update);
        assert!(!s.battery_alerts);
        assert_eq!(s.recordings_folder, Some(PathBuf::from(r"C:\Users\Test\Recordings")));
        assert_eq!(s.recordings_format, RecordingFormat::Flac);

        // 4. Commit 45b5681 (+ webcamPhone, webcamHeight, webcamMirror)
        let snap_45b5681 = r#"{
            "theme": "bloom",
            "seed": "wallpaper",
            "colorMode": "system",
            "backdrop": true,
            "closeToTray": true,
            "autoClipboard": true,
            "sendToMenu": true,
            "autoUpdate": true,
            "batteryAlerts": true,
            "recordingsFormat": "mp3",
            "webcamPhone": "abababababababababababababababababababababababababababababababab",
            "webcamHeight": 1080,
            "webcamMirror": true
        }"#;
        fs::write(dir.path().join(FILE_NAME), snap_45b5681).unwrap();
        let s = Settings::load(dir.path());
        assert_eq!(s.webcam_height, 1080);
        assert!(s.webcam_mirror);
        assert_eq!(s.recordings_format, RecordingFormat::Mp3);

        // 5. Commit e9176d5 (+ clipboardHistory, autoCopyOtp, pauseMediaOnCall, syncDnd) & 9520c59 (+ suggestClipboardActions)
        let snap_9520c59 = r#"{
            "theme": "bloom",
            "seed": "lavender",
            "colorMode": "dark",
            "backdrop": true,
            "closeToTray": true,
            "autoClipboard": true,
            "clipboardHistory": false,
            "suggestClipboardActions": false,
            "autoCopyOtp": true,
            "pauseMediaOnCall": false,
            "syncDnd": true,
            "sendToMenu": true,
            "autoUpdate": true,
            "batteryAlerts": true,
            "recordingsFormat": "wav",
            "webcamHeight": 720,
            "webcamMirror": false
        }"#;
        fs::write(dir.path().join(FILE_NAME), snap_9520c59).unwrap();
        let s = Settings::load(dir.path());
        assert!(!s.clipboard_history);
        assert!(!s.suggest_clipboard_actions);
        assert!(s.auto_copy_otp);
        assert!(!s.pause_media_on_call);
        assert!(s.sync_dnd);

        // 6. Commit 9e2154b (+ timelineRetentionDays) & 70da2e8 (+ continuity/palette hotkeys) & ebe0012 (+ shelfHotkey)
        let snap_ebe0012 = r#"{
            "theme": "graphite",
            "seed": "phone",
            "colorMode": "dark",
            "backdrop": false,
            "closeToTray": true,
            "autoClipboard": true,
            "clipboardHistory": true,
            "suggestClipboardActions": true,
            "autoCopyOtp": false,
            "pauseMediaOnCall": true,
            "syncDnd": false,
            "sendToMenu": true,
            "autoUpdate": true,
            "batteryAlerts": true,
            "recordingsFormat": "m4a",
            "webcamHeight": 1080,
            "webcamMirror": false,
            "timelineRetentionDays": 365,
            "continuityPhotoHotkey": "Ctrl+Shift+C",
            "continuityScanHotkey": "Ctrl+Shift+D",
            "commandPaletteHotkey": "Ctrl+Alt+Space",
            "shelfHotkey": "Ctrl+Alt+S"
        }"#;
        fs::write(dir.path().join(FILE_NAME), snap_ebe0012).unwrap();
        let s = Settings::load(dir.path());
        assert_eq!(s.theme, Theme::Graphite);
        assert_eq!(s.seed, "phone");
        assert_eq!(s.timeline_retention_days, 365);
        assert_eq!(s.continuity_photo_hotkey, "Ctrl+Shift+C");
        assert_eq!(s.continuity_scan_hotkey, "Ctrl+Shift+D");
        assert_eq!(s.command_palette_hotkey, "Ctrl+Alt+Space");
        assert_eq!(s.shelf_hotkey, "Ctrl+Alt+S");
        assert!(!s.local_api_enabled);
        assert_eq!(s.local_api_port, 53320);

        // 7. Legacy snake_case & unknown extra fields from early prototypes (f3bf03d, 7f276f2, 8d46bb6, f26b368, 0f5b58a, ef738f1, fa9ddca)
        let legacy_snake = r#"{
            "theme": "graphite",
            "accent": "ocean",
            "download_dir": "C:\\Downloads",
            "start_with_windows": true,
            "start_minimized": false,
            "close_to_tray": false,
            "clipboard_sync": false,
            "clipboard_images": true,
            "notifications": true,
            "quiet_hours_enabled": false,
            "blocked_apps": ["com.example.spam"],
            "smart_otp": true,
            "smart_otp_auto_copy": true,
            "auto_pause_media": false,
            "auto_mute_on_call": false,
            "webcam_resolution": "1080p",
            "webcam_fps": 30,
            "away_mode": true,
            "relay_url": null,
            "localsend": false,
            "shelf_enabled": true,
            "handoff_links": true,
            "universal_input_edge": "right",
            "onboarding_completed": true,
            "dismissed_banners": [],
            "collapsed_notif_apps": []
        }"#;
        fs::write(dir.path().join(FILE_NAME), legacy_snake).unwrap();
        let s = Settings::load(dir.path());
        assert_eq!(s.theme, Theme::Graphite);
        assert_eq!(s.seed, "ocean");
        assert_eq!(s.start_with_windows, Some(true));
        assert!(!s.close_to_tray);
        assert!(!s.auto_clipboard);
        assert!(s.auto_copy_otp);
        assert!(!s.pause_media_on_call);

        // 8. Empty `{}` object and unknown legacy enum values (`"theme": "sepia"`, `"accent": "ultraviolet"`)
        fs::write(dir.path().join(FILE_NAME), "{}").unwrap();
        assert_eq!(Settings::load(dir.path()), Settings::default());

        let unknown_enums = r#"{
            "theme": "sepia",
            "accent": "ultraviolet",
            "colorMode": "solarized",
            "recordingsFormat": "ogg"
        }"#;
        fs::write(dir.path().join(FILE_NAME), unknown_enums).unwrap();
        let s = Settings::load(dir.path());
        assert_eq!(s.theme, Theme::Bloom);
        assert_eq!(s.seed, "ultraviolet");
        assert_eq!(s.color_mode, ColorMode::System);
        assert_eq!(s.recordings_format, RecordingFormat::M4a);
    }

    #[test]
    fn walkthrough_first_run_resume_skip_and_upgrade_with_paired_device() {
        let dir = tempfile::tempdir().unwrap();

        // 1. First run: no paired devices, walkthrough_completed == false, step == 0.
        let mut first_run = Settings::load(dir.path());
        assert!(!first_run.walkthrough_completed);
        assert_eq!(first_run.walkthrough_step, 0);
        assert!(!first_run.reconcile_walkthrough_for_paired_devices(0));
        assert!(!first_run.walkthrough_completed);
        assert_eq!(first_run.walkthrough_step, 0);

        // 2. Resume: user reaches step 3 (pair step) and pairs a phone mid-walkthrough, then restarts.
        first_run.walkthrough_step = 3;
        first_run.save(dir.path()).unwrap();
        let mut resumed = Settings::load(dir.path());
        assert!(!resumed.walkthrough_completed);
        assert_eq!(resumed.walkthrough_step, 3);
        assert!(
            !resumed.reconcile_walkthrough_for_paired_devices(1),
            "mid-walkthrough user with step > 0 must resume at step 3 even after pairing"
        );
        assert!(!resumed.walkthrough_completed);
        assert_eq!(resumed.walkthrough_step, 3);

        // Out-of-bounds step in JSON is clamped to 6.
        fs::write(dir.path().join(FILE_NAME), r#"{ "walkthroughStep": 42 }"#).unwrap();
        let clamped = Settings::load(dir.path());
        assert_eq!(clamped.walkthrough_step, 6);

        // 3. Skip: user skips the walkthrough, stays completed across restarts.
        let skipped = Settings { walkthrough_completed: true, walkthrough_step: 2, ..Default::default() };
        skipped.save(dir.path()).unwrap();
        let mut reloaded_skipped = Settings::load(dir.path());
        assert!(reloaded_skipped.walkthrough_completed);
        assert!(!reloaded_skipped.reconcile_walkthrough_for_paired_devices(0));
        assert!(reloaded_skipped.walkthrough_completed);

        // 4. Upgrade with existing paired device: legacy settings JSON has no walkthrough fields,
        // so walkthrough_completed == false and walkthrough_step == 0, while paired_count > 0.
        fs::write(dir.path().join(FILE_NAME), r#"{ "theme": "bloom" }"#).unwrap();
        let mut upgraded = Settings::load(dir.path());
        assert!(!upgraded.walkthrough_completed);
        assert_eq!(upgraded.walkthrough_step, 0);
        assert!(upgraded.reconcile_walkthrough_for_paired_devices(2));
        assert!(upgraded.walkthrough_completed);
    }

    #[test]
    fn theme_switching_persists_all_themes_color_modes_seeds_and_modifiers() {
        let dir = tempfile::tempdir().unwrap();

        let themes = [
            ("bloom", Theme::Bloom),
            ("hive", Theme::Hive),
            ("glass", Theme::Glass),
            ("blurry", Theme::Glass),
            ("graphite", Theme::Graphite),
        ];
        let modes = [("system", ColorMode::System), ("light", ColorMode::Light), ("dark", ColorMode::Dark)];
        let seeds = [
            "wallpaper",
            "phone",
            "artwork",
            "black",
            "honey",
            "lavender",
            "ocean",
            "rose",
            "sage",
            "terracotta",
        ];

        for &(theme_str, expected_theme) in &themes {
            for &(mode_str, expected_mode) in &modes {
                for &seed in &seeds {
                    for &amoled in &[false, true] {
                        for &comb_motion in &[false, true] {
                            let raw = serde_json::json!({
                                "theme": theme_str,
                                "colorMode": mode_str,
                                "seed": seed,
                                "amoled": amoled,
                                "combMotion": comb_motion,
                            });
                            fs::write(dir.path().join(FILE_NAME), raw.to_string()).unwrap();
                            let loaded = Settings::load(dir.path());
                            assert_eq!(loaded.theme, expected_theme);
                            assert_eq!(loaded.color_mode, expected_mode);
                            assert_eq!(loaded.seed, seed);
                            assert_eq!(loaded.amoled, amoled);
                            assert_eq!(loaded.comb_motion, comb_motion);

                            loaded.save(dir.path()).unwrap();
                            let round_tripped = Settings::load(dir.path());
                            assert_eq!(round_tripped.theme, expected_theme);
                            assert_eq!(round_tripped.color_mode, expected_mode);
                            assert_eq!(round_tripped.seed, seed);
                            assert_eq!(round_tripped.amoled, amoled);
                            assert_eq!(round_tripped.comb_motion, comb_motion);
                        }
                    }
                }
            }
        }
    }

    fn extract_qt_placeholders(s: &str) -> Vec<String> {
        let mut out = Vec::new();
        let bytes = s.as_bytes();
        let mut i = 0;
        while i + 1 < bytes.len() {
            if bytes[i] == b'%' {
                if bytes[i + 1].is_ascii_digit() && bytes[i + 1] != b'0' {
                    out.push(format!("%{}", bytes[i + 1] as char));
                    i += 2;
                    continue;
                } else if bytes[i + 1] == b'n' {
                    let prev_ok = i == 0 || !bytes[i - 1].is_ascii_alphabetic();
                    let next_ok = i + 2 >= bytes.len() || !bytes[i + 2].is_ascii_alphabetic();
                    if prev_ok && next_ok {
                        out.push("%n".to_owned());
                        i += 2;
                        continue;
                    }
                }
            }
            i += 1;
        }
        out.sort();
        out.dedup();
        out
    }

    #[test]
    fn desktop_i18n_catalog_is_complete_preserves_placeholders_and_compiles_to_qm() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let ts_path = manifest_dir.join("i18n/nectarlink_zh_CN.ts");
        let xml = fs::read_to_string(&ts_path).expect("read desktop/app/i18n/nectarlink_zh_CN.ts");
        assert!(xml.contains(r#"language="zh_CN""#));
        assert!(
            !xml.contains(r#"type="unfinished""#),
            "nectarlink_zh_CN.ts must not contain unfinished translations"
        );

        let mut msg_count = 0usize;
        for msg_chunk in xml.split("<message").skip(1) {
            let Some(msg_body) = msg_chunk.split("</message>").next() else {
                continue;
            };
            let Some(src_start) = msg_body.find("<source>") else {
                continue;
            };
            let Some(src_end) = msg_body.find("</source>") else {
                continue;
            };
            let source = &msg_body[src_start + "<source>".len()..src_end];

            let Some(tr_tag_start) = msg_body.find("<translation") else {
                panic!("message missing <translation> for source: {source}");
            };
            let tr_after_open = &msg_body[tr_tag_start..];
            let Some(tr_gt) = tr_after_open.find('>') else {
                panic!("malformed <translation> tag for source: {source}");
            };
            let Some(tr_end) = tr_after_open.find("</translation>") else {
                panic!("missing </translation> for source: {source}");
            };
            let tr_inner = &tr_after_open[tr_gt + 1..tr_end];
            assert!(
                !tr_inner.trim().is_empty(),
                "empty translation in nectarlink_zh_CN.ts for source: {source}"
            );

            let src_ph = extract_qt_placeholders(source);
            let mut tr_texts = Vec::new();
            if tr_inner.contains("<numerusform>") {
                for nf in tr_inner.split("<numerusform>").skip(1) {
                    if let Some(nf_body) = nf.split("</numerusform>").next() {
                        assert!(!nf_body.trim().is_empty(), "empty numerusform for source: {source}");
                        tr_texts.push(nf_body);
                    }
                }
            } else {
                tr_texts.push(tr_inner);
            }

            for tr_text in tr_texts {
                for ph in &src_ph {
                    if ph == "%n" {
                        continue;
                    }
                    assert!(
                        tr_text.contains(ph),
                        "placeholder {ph} missing in zh_CN translation '{tr_text}' for source '{source}'"
                    );
                }
            }
            msg_count += 1;
        }
        assert!(msg_count >= 900, "expected >= 900 translated messages, found {msg_count}");

        // Verify build.rs compiled nectarlink_zh_CN.ts into a valid binary .qm file in OUT_DIR.
        let out_qm = Path::new(env!("OUT_DIR")).join("nectarlink_zh_CN.qm");
        let qm_bytes = fs::read(&out_qm).expect("read compiled nectarlink_zh_CN.qm from OUT_DIR");
        assert!(qm_bytes.len() > 10_000, "compiled .qm unexpectedly small: {} bytes", qm_bytes.len());
        assert_eq!(
            &qm_bytes[..16],
            &[0x3C, 0xB8, 0x64, 0x18, 0xCA, 0xEF, 0x9C, 0x95, 0xCD, 0x21, 0x1C, 0xBF, 0x60, 0xA1, 0xBD, 0xDD,],
            "compiled .qm file must start with 16-byte Qt magic header"
        );
    }

    fn extract_android_placeholders(s: &str) -> Vec<String> {
        let mut out = Vec::new();
        let bytes = s.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' {
                if i + 1 < bytes.len() && bytes[i + 1] == b'%' {
                    i += 2;
                    continue;
                }
                let start = i;
                i += 1;
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'$' || bytes[i] == b'.') {
                    i += 1;
                }
                if i < bytes.len() && matches!(bytes[i], b's' | b'd' | b'f') {
                    i += 1;
                    out.push(s[start..i].to_owned());
                    continue;
                }
            }
            i += 1;
        }
        out.sort();
        out.dedup();
        out
    }

    fn parse_android_strings_xml(xml: &str) -> std::collections::BTreeMap<String, String> {
        let mut map = std::collections::BTreeMap::new();
        for chunk in xml.split("<string ").skip(1) {
            let Some(tag_end) = chunk.find('>') else {
                continue;
            };
            let attrs = &chunk[..tag_end];
            if attrs.contains(r#"translatable="false""#) {
                continue;
            }
            let Some(name_idx) = attrs.find(r#"name=""#) else {
                continue;
            };
            let after_name = &attrs[name_idx + r#"name=""#.len()..];
            let Some(quote_end) = after_name.find('"') else {
                continue;
            };
            let name = &after_name[..quote_end];
            let rest = &chunk[tag_end + 1..];
            let Some(val_end) = rest.find("</string>") else {
                continue;
            };
            map.insert(name.to_owned(), rest[..val_end].to_owned());
        }
        map
    }

    #[test]
    fn android_i18n_catalog_matches_all_translatable_keys_and_placeholders() {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let en_xml = fs::read_to_string(repo_root.join("android/app/src/main/res/values/strings.xml"))
            .expect("read values/strings.xml");
        let zh_xml = fs::read_to_string(repo_root.join("android/app/src/main/res/values-zh-rCN/strings.xml"))
            .expect("read values-zh-rCN/strings.xml");

        let en_strings = parse_android_strings_xml(&en_xml);
        let zh_strings = parse_android_strings_xml(&zh_xml);
        assert!(en_strings.len() >= 1000, "expected >= 1000 Android strings, got {}", en_strings.len());

        for (key, en_val) in &en_strings {
            let zh_val = zh_strings
                .get(key)
                .unwrap_or_else(|| panic!("missing zh-rCN translation for Android string key '{key}'"));
            assert!(!zh_val.trim().is_empty(), "empty zh-rCN translation for Android string key '{key}'");
            let en_ph = extract_android_placeholders(en_val);
            let zh_ph = extract_android_placeholders(zh_val);
            assert_eq!(
                en_ph, zh_ph,
                "placeholder mismatch for Android string key '{key}': en={en_ph:?} vs zh={zh_ph:?}"
            );
        }
    }
}
