// SPDX-License-Identifier: MPL-2.0
//! Phone as a macro pad for a PC (`docs/protocol/deck.md`).
//!
//! A PC that offers `deck.actions` shares its Deck layout (`deck.layout`) and
//! live tile state (`deck.state`) when a session starts and after every edit
//! or state change. A phone sends `deck.press { tile }` by tile ID only; the
//! PC checks the `remote_input` toggle (and the `commands` toggle for
//! `run_command` tiles) before running the tile's action.
//!
//! A tile can show a picture the user gave it instead of its icon (#490):
//! the layout names it by ID, and a phone asks for each one it doesn't have
//! (`deck.image`), checks it against its ID, and keeps it while the layout
//! names it.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::Path,
    sync::{Arc, Mutex},
};

pub use nectarlink_protocol::messages::{
    AudioOutputDevice, DeckImage, DeckImageGet, DeckLayout, DeckPage, DeckPress, DeckState, DeckTile, KeyMod,
    PcAudioSet, PcRadioSet,
    deck::{
        ACTIONS as DECK_ACTIONS, MAX_ID_BYTES as MAX_DECK_ID_BYTES, MAX_LABEL_BYTES as MAX_DECK_LABEL_BYTES,
        MAX_PAGES as MAX_DECK_PAGES, MAX_TILES_PER_PAGE as MAX_DECK_TILES_PER_PAGE, PC_AUDIO, PC_RADIOS,
    },
    deck_colors, deck_icons, deck_image, deck_kinds, is_valid_deck_id, remote_keys,
};
use nectarlink_protocol::{
    DeviceId, Envelope, ErrorCode,
    messages::{remote, types},
};
use serde::{Deserialize, Serialize};

use crate::{
    Error, Result,
    events::NodeEvent,
    node::Shared,
    session::{REQUEST_TIMEOUT, Session},
};

/// Per-device toggle required for all `deck.press` actions (off by default).
pub const INPUT_TOGGLE: &str = "remote_input";

/// Additional per-device toggle required for `run_command` tiles (off by default).
pub const COMMANDS_TOGGLE: &str = "commands";

const MAX_URL_BYTES: usize = 2048;
const MAX_PATH_BYTES: usize = 1024;
const MAX_COMMAND_BYTES: usize = 1024;
/// Most steps in a macro tile (#490).
pub const MAX_MACRO_STEPS: usize = 16;
/// Longest pause after one macro step, and all of a macro's pauses together.
pub const MAX_MACRO_DELAY_MS: u32 = 10_000;
pub const MAX_MACRO_TOTAL_DELAY_MS: u32 = 30_000;

/// One step of a macro tile: an action, then a pause before the next one.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacroStep {
    pub action: DeckAction,
    #[serde(default)]
    pub delay_ms: u32,
}

impl std::fmt::Debug for MacroStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MacroStep").field("action", &self.action).field("delay_ms", &self.delay_ms).finish()
    }
}

/// A PC-owned action bound to a Deck tile. Parameters stay on the PC and are
/// never sent over the wire.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeckAction {
    MediaPlayPause,
    MediaNext,
    MediaPrevious,
    VolumeUp,
    VolumeDown,
    VolumeMute,
    MicMute,
    LockPc,
    ShowDesktop,
    SwitchWindow,
    Screenshot,
    Shortcut {
        key: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        mods: Vec<KeyMod>,
    },
    OpenUrl {
        url: String,
    },
    TypeText {
        text: String,
    },
    LaunchApp {
        path: String,
    },
    RunCommand {
        command: String,
    },
    RestartPc,
    ShutDownPc,
    /// Its steps in order (#490). Not nested, and no restart or shut down
    /// inside: those are asked about on the phone, a macro isn't.
    Macro {
        steps: Vec<MacroStep>,
    },
    /// On and off in turn (#640): `on` runs when it's off, `off` when it's
    /// on. Each is an action that fits in a macro. Whether it's on is kept
    /// in [`DeckConfig::toggles_on`].
    Toggle {
        on: Box<DeckAction>,
        off: Box<DeckAction>,
    },
}

/// Never prints paths, URLs, text snippets, or commands (protocol v0 §11).
impl std::fmt::Debug for DeckAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeckAction").field("kind", &self.kind()).finish_non_exhaustive()
    }
}

impl DeckAction {
    /// The wire action kind (`docs/protocol/deck.md` §2.4).
    pub fn kind(&self) -> &'static str {
        match self {
            Self::MediaPlayPause => deck_kinds::MEDIA_PLAY_PAUSE,
            Self::MediaNext => deck_kinds::MEDIA_NEXT,
            Self::MediaPrevious => deck_kinds::MEDIA_PREVIOUS,
            Self::VolumeUp => deck_kinds::VOLUME_UP,
            Self::VolumeDown => deck_kinds::VOLUME_DOWN,
            Self::VolumeMute => deck_kinds::VOLUME_MUTE,
            Self::MicMute => deck_kinds::MIC_MUTE,
            Self::LockPc => deck_kinds::LOCK_PC,
            Self::ShowDesktop => deck_kinds::SHOW_DESKTOP,
            Self::SwitchWindow => deck_kinds::SWITCH_WINDOW,
            Self::Screenshot => deck_kinds::SCREENSHOT,
            Self::Shortcut { .. } => deck_kinds::SHORTCUT,
            Self::OpenUrl { .. } => deck_kinds::OPEN_URL,
            Self::TypeText { .. } => deck_kinds::TYPE_TEXT,
            Self::LaunchApp { .. } => deck_kinds::LAUNCH_APP,
            Self::RunCommand { .. } => deck_kinds::RUN_COMMAND,
            Self::RestartPc => deck_kinds::RESTART_PC,
            Self::ShutDownPc => deck_kinds::SHUT_DOWN_PC,
            Self::Macro { .. } => deck_kinds::MACRO,
            Self::Toggle { .. } => deck_kinds::TOGGLE,
        }
    }

    /// Whether this action requires the per-device `commands` toggle.
    pub fn requires_commands_toggle(&self) -> bool {
        match self {
            Self::RunCommand { .. } => true,
            Self::Macro { steps } => steps.iter().any(|s| s.action.requires_commands_toggle()),
            Self::Toggle { on, off } => on.requires_commands_toggle() || off.requires_commands_toggle(),
            _ => false,
        }
    }

    /// Whether it can be one step of a macro, or one side of a toggle.
    pub fn fits_in_macro(&self) -> bool {
        !matches!(self, Self::Macro { .. } | Self::Toggle { .. } | Self::RestartPc | Self::ShutDownPc)
    }

    /// Validates action parameters.
    pub fn is_valid(&self) -> bool {
        match self {
            Self::MediaPlayPause
            | Self::MediaNext
            | Self::MediaPrevious
            | Self::VolumeUp
            | Self::VolumeDown
            | Self::VolumeMute
            | Self::MicMute
            | Self::LockPc
            | Self::ShowDesktop
            | Self::SwitchWindow
            | Self::Screenshot
            | Self::RestartPc
            | Self::ShutDownPc => true,
            Self::Macro { steps } => {
                (1..=MAX_MACRO_STEPS).contains(&steps.len())
                    && steps.iter().all(|s| {
                        s.action.fits_in_macro() && s.action.is_valid() && s.delay_ms <= MAX_MACRO_DELAY_MS
                    })
                    && steps.iter().map(|s| s.delay_ms).sum::<u32>() <= MAX_MACRO_TOTAL_DELAY_MS
            }
            Self::Toggle { on, off } => {
                on.fits_in_macro() && on.is_valid() && off.fits_in_macro() && off.is_valid()
            }
            Self::Shortcut { key, mods } => is_valid_shortcut(key, mods),
            Self::OpenUrl { url } => is_valid_http_url(url),
            Self::TypeText { text } => is_valid_snippet(text),
            Self::LaunchApp { path } => is_valid_app_path(path),
            Self::RunCommand { command } => is_valid_command(command),
        }
    }

    /// Short human-readable summary for the PC's Deck editor.
    pub fn summary(&self) -> String {
        match self {
            Self::MediaPlayPause => "Play or pause media".into(),
            Self::MediaNext => "Next track".into(),
            Self::MediaPrevious => "Previous track".into(),
            Self::VolumeUp => "Volume up".into(),
            Self::VolumeDown => "Volume down".into(),
            Self::VolumeMute => "Mute or unmute speakers".into(),
            Self::MicMute => "Mute or unmute microphone".into(),
            Self::LockPc => "Lock this PC".into(),
            Self::ShowDesktop => "Show desktop (Win+D)".into(),
            Self::SwitchWindow => "Switch window (Alt+Tab)".into(),
            Self::Screenshot => "Take screenshot (Win+Shift+S)".into(),
            Self::RestartPc => "Restart this PC".into(),
            Self::ShutDownPc => "Shut down this PC".into(),
            Self::Macro { steps } => {
                steps.iter().map(|s| s.action.summary()).collect::<Vec<_>>().join(", then ")
            }
            Self::Toggle { on, off } => format!("On: {}. Off: {}", on.summary(), off.summary()),
            Self::Shortcut { key, mods } => format_shortcut(key, mods),
            Self::OpenUrl { url } => url.clone(),
            Self::TypeText { text } => {
                let single: String =
                    text.chars().map(|c| if c == '\n' || c == '\t' { ' ' } else { c }).collect();
                let mut chars = single.chars();
                let head: String = chars.by_ref().take(32).collect();
                if chars.next().is_some() { format!("{head}…") } else { head }
            }
            Self::LaunchApp { path } => Path::new(path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| path.clone()),
            Self::RunCommand { command } => {
                let mut chars = command.trim().chars();
                let head: String = chars.by_ref().take(32).collect();
                if chars.next().is_some() { format!("{head}…") } else { head }
            }
        }
    }
}

/// Validates a shortcut key and modifier list.
pub fn is_valid_shortcut(key: &str, mods: &[KeyMod]) -> bool {
    if mods.len() > 4 {
        return false;
    }
    for (i, m) in mods.iter().enumerate() {
        if mods[..i].contains(m) {
            return false;
        }
    }
    if remote_keys::SHORTCUTS.contains(&key) {
        return mods.is_empty();
    }
    if remote_keys::NAMED.contains(&key) {
        return true;
    }
    key.len() == 1 && key.as_bytes()[0].is_ascii_lowercase()
        || (key.len() == 1 && key.as_bytes()[0].is_ascii_digit())
}

/// Formats a shortcut for display (e.g. `"Ctrl + Shift + M"`).
pub fn format_shortcut(key: &str, mods: &[KeyMod]) -> String {
    let mut parts: Vec<String> = mods
        .iter()
        .map(|m| {
            match m {
                KeyMod::Ctrl => "Ctrl",
                KeyMod::Alt => "Alt",
                KeyMod::Shift => "Shift",
                KeyMod::Win => "Win",
            }
            .into()
        })
        .collect();
    let key_label = if key.len() == 1 { key.to_ascii_uppercase() } else { key.replace('_', " ") };
    parts.push(key_label);
    parts.join(" + ")
}

/// Validates that `url` is a well-formed `http` or `https` URL.
pub fn is_valid_http_url(url: &str) -> bool {
    let trimmed = url.trim();
    trimmed.len() <= MAX_URL_BYTES && crate::actions::valid_link(trimmed)
}

/// Validates a text snippet for `TypeText`.
pub fn is_valid_snippet(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= remote::MAX_TEXT_BYTES
        && !text.chars().any(|c| c.is_control() && c != '\n' && c != '\t')
}

/// Validates that `path` is a non-empty `.exe`, `.lnk`, `.desktop`, or `.appimage`
/// file path, or an absolute (or `~/`) Unix executable path excluding shells,
/// `.sh` scripts, and non-application system/temp directories.
pub fn is_valid_app_path(path: &str) -> bool {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_PATH_BYTES || trimmed.chars().any(char::is_control) {
        return false;
    }
    if trimmed.split(['/', '\\']).any(|seg| seg == "..") {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    for ext in [".exe", ".lnk", ".desktop", ".appimage"] {
        if let Some(stem) = lower.strip_suffix(ext) {
            let file_stem = stem.rsplit(['/', '\\']).next().unwrap_or(stem);
            return !file_stem.is_empty();
        }
    }
    if !trimmed.starts_with('/') && !trimmed.starts_with("~/") {
        return false;
    }
    let Some((parent, name)) = trimmed.rsplit_once('/') else {
        return false;
    };
    let lower_name = name.to_ascii_lowercase();
    let is_shell =
        matches!(lower_name.as_str(), "sh" | "bash" | "dash" | "zsh" | "fish" | "csh" | "tcsh" | "ksh");
    if parent.is_empty()
        || parent == "~"
        || name.is_empty()
        || name == "."
        || name.contains('\\')
        || lower_name.ends_with(".sh")
        || is_shell
    {
        return false;
    }
    let parent_segments = parent.strip_prefix("~/").unwrap_or_else(|| &parent[1..]);
    if parent_segments.split('/').any(|seg| seg.is_empty() || seg == ".") {
        return false;
    }
    !matches!(parent, "/etc" | "/tmp" | "/var/tmp" | "/dev" | "/proc" | "/sys")
        && !parent.starts_with("/etc/")
        && !parent.starts_with("/tmp/")
        && !parent.starts_with("/var/tmp/")
        && !parent.starts_with("/dev/")
        && !parent.starts_with("/proc/")
        && !parent.starts_with("/sys/")
}

/// Validates a command string for `RunCommand`.
pub fn is_valid_command(command: &str) -> bool {
    let trimmed = command.trim();
    !trimmed.is_empty() && trimmed.len() <= MAX_COMMAND_BYTES && !trimmed.chars().any(char::is_control)
}

/// One tile in the PC's stored [`DeckConfig`].
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeckTileConfig {
    pub id: String,
    pub label: String,
    pub icon: String,
    pub color: String,
    pub action: DeckAction,
    /// The ID of the picture shown instead of the icon ([`image_id`]), #490.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
}

impl std::fmt::Debug for DeckTileConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeckTileConfig")
            .field("id", &self.id)
            .field("icon", &self.icon)
            .field("color", &self.color)
            .field("action", &self.action)
            .finish_non_exhaustive()
    }
}

impl DeckTileConfig {
    pub fn is_valid(&self) -> bool {
        self.action.is_valid() && self.to_wire().is_valid()
    }

    pub fn sanitized(self) -> Option<Self> {
        if !self.action.is_valid() {
            return None;
        }
        let wire = self.to_wire().sanitized()?;
        Some(DeckTileConfig {
            id: wire.id,
            label: wire.label,
            icon: wire.icon,
            color: wire.color,
            action: self.action,
            image: wire.image,
        })
    }

    /// Converts to the wire [`DeckTile`], stripping any local path, URL,
    /// snippet, or command.
    pub fn to_wire(&self) -> DeckTile {
        DeckTile {
            id: self.id.clone(),
            label: self.label.clone(),
            icon: self.icon.clone(),
            color: self.color.clone(),
            kind: self.action.kind().into(),
            needs_commands: self.action.kind() != deck_kinds::RUN_COMMAND
                && self.action.requires_commands_toggle(),
            image: self.image.clone(),
        }
    }
}

/// A tile picture's ID: the first 16 bytes of its PNG's SHA-256, in hex.
pub fn image_id(png: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(png)[..16].iter().map(|b| format!("{b:02x}")).collect()
}

/// The picture IDs `layout`'s tiles name.
fn image_ids(layout: &DeckLayout) -> HashSet<String> {
    layout.pages.iter().flat_map(|p| &p.tiles).filter_map(|t| t.image.clone()).collect()
}

/// One page of tiles in the PC's stored [`DeckConfig`].
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeckPageConfig {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub tiles: Vec<DeckTileConfig>,
}

impl std::fmt::Debug for DeckPageConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeckPageConfig")
            .field("id", &self.id)
            .field("tiles", &self.tiles.len())
            .finish_non_exhaustive()
    }
}

impl DeckPageConfig {
    pub fn to_wire(&self) -> DeckPage {
        DeckPage {
            id: self.id.clone(),
            name: self.name.clone(),
            tiles: self.tiles.iter().map(DeckTileConfig::to_wire).collect(),
        }
    }
}

/// The PC-owned Deck configuration (saved as `deck.json` in the PC's data directory).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeckConfig {
    pub pages: Vec<DeckPageConfig>,
    /// The `toggle` tiles that are on (#640), kept across restarts.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub toggles_on: BTreeSet<String>,
}

impl DeckConfig {
    pub fn is_valid(&self) -> bool {
        self.pages.iter().flat_map(|p| p.tiles.iter()).all(DeckTileConfig::is_valid)
            && self.to_wire_layout().is_valid()
    }

    pub fn sanitized(self) -> Option<Self> {
        let mut page_ids = BTreeSet::new();
        let mut tile_ids = BTreeSet::new();
        let mut pages = Vec::new();
        for page in self.pages {
            if pages.len() >= MAX_DECK_PAGES {
                break;
            }
            if !is_valid_deck_id(&page.id) || !page_ids.insert(page.id.clone()) {
                continue;
            }
            let name: String = page.name.chars().filter(|c| !c.is_control()).collect();
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            let mut end = name.len().min(MAX_DECK_LABEL_BYTES);
            while end > 0 && !name.is_char_boundary(end) {
                end -= 1;
            }
            let name = name[..end].trim().to_owned();
            if name.is_empty() {
                continue;
            }
            let mut tiles = Vec::new();
            for tile in page.tiles {
                if tiles.len() >= MAX_DECK_TILES_PER_PAGE {
                    break;
                }
                if let Some(clean) = tile.sanitized()
                    && tile_ids.insert(clean.id.clone())
                {
                    tiles.push(clean);
                }
            }
            pages.push(DeckPageConfig { id: page.id, name, tiles });
        }
        // Only toggles that are still there.
        let toggles_on = self
            .toggles_on
            .into_iter()
            .filter(|id| {
                pages
                    .iter()
                    .flat_map(|p| p.tiles.iter())
                    .any(|t| t.id == *id && matches!(t.action, DeckAction::Toggle { .. }))
            })
            .collect();
        (!pages.is_empty()).then_some(DeckConfig { pages, toggles_on })
    }

    /// Presses the `toggle` tile `id`: the action to run now (`off` when
    /// it's on, else `on`), and whether it's on after. `None` for any other tile.
    pub fn toggle_press(&self, id: &str) -> Option<(DeckAction, bool)> {
        let DeckAction::Toggle { on, off } = &self.tile(id)?.action else { return None };
        Some(if self.toggles_on.contains(id) { ((**off).clone(), false) } else { ((**on).clone(), true) })
    }

    /// Notes whether the `toggle` tile `id` is on now.
    pub fn set_toggle(&mut self, id: &str, on: bool) {
        if on {
            self.toggles_on.insert(id.to_owned());
        } else {
            self.toggles_on.remove(id);
        }
    }

    /// The `toggle` tiles that are on, for [`DeckState::toggles_on`].
    pub fn toggles_on_list(&self) -> Vec<String> {
        self.toggles_on.iter().cloned().collect()
    }

    /// Converts to the wire [`DeckLayout`] sent to paired phones.
    pub fn to_wire_layout(&self) -> DeckLayout {
        DeckLayout { pages: self.pages.iter().map(DeckPageConfig::to_wire).collect() }
    }

    /// Finds a tile by ID across all pages.
    pub fn tile(&self, id: &str) -> Option<&DeckTileConfig> {
        self.pages.iter().flat_map(|p| p.tiles.iter()).find(|t| t.id == id)
    }

    /// The default single-page Deck on first use (`docs/protocol/deck.md`).
    pub fn default_deck() -> Self {
        let tile = |id: &str, action: DeckAction| {
            let kind = action.kind();
            DeckTileConfig {
                image: None,
                id: id.into(),
                label: deck_kinds::default_label(kind).into(),
                icon: deck_kinds::default_icon(kind).into(),
                color: deck_kinds::default_color(kind).into(),
                action,
            }
        };
        DeckConfig {
            pages: vec![DeckPageConfig {
                id: "main".into(),
                name: "Main".into(),
                tiles: vec![
                    tile("play_pause", DeckAction::MediaPlayPause),
                    tile("prev_track", DeckAction::MediaPrevious),
                    tile("next_track", DeckAction::MediaNext),
                    tile("vol_down", DeckAction::VolumeDown),
                    tile("vol_up", DeckAction::VolumeUp),
                    tile("vol_mute", DeckAction::VolumeMute),
                    tile("mic_mute", DeckAction::MicMute),
                    tile("show_desktop", DeckAction::ShowDesktop),
                    tile("switch_window", DeckAction::SwitchWindow),
                    tile("screenshot", DeckAction::Screenshot),
                    tile("lock_pc", DeckAction::LockPc),
                ],
            }],
            toggles_on: BTreeSet::new(),
        }
    }

    /// Loads `DeckConfig` from `path`, falling back to [`Self::default_deck`]
    /// when the file does not exist or cannot be parsed/sanitized. An
    /// unreadable file is kept beside it as `deck.json.bad`, since the next
    /// save would otherwise replace the user's layout with the default.
    pub fn load(path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else { return Self::default_deck() };
        let loaded = serde_json::from_str::<DeckConfig>(&text).ok().and_then(DeckConfig::sanitized);
        loaded.unwrap_or_else(|| {
            tracing::warn!("the Deck layout can't be read; starting from the default");
            let _ = std::fs::rename(path, path.with_extension("json.bad"));
            Self::default_deck()
        })
    }

    /// Writes `self` as pretty-printed JSON to `path`, all or nothing.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, json)?;
        std::fs::rename(&temp, path).inspect_err(|_| {
            let _ = std::fs::remove_file(&temp);
        })
    }
}

impl Default for DeckConfig {
    fn default() -> Self {
        Self::default_deck()
    }
}

/// Tile pictures by ID.
type Images = HashMap<String, Arc<Vec<u8>>>;

/// This PC's Deck layout and live state (sent to newly connected phones) and
/// the latest layout and state received from each connected PC.
pub(crate) struct Current {
    local_layout: Mutex<DeckLayout>,
    local_state: Mutex<DeckState>,
    peer_layouts: Mutex<HashMap<DeviceId, DeckLayout>>,
    peer_states: Mutex<HashMap<DeviceId, DeckState>>,
    /// This PC's tile pictures, by ID, for phones that ask.
    local_images: Mutex<Images>,
    /// Each connected PC's tile pictures this phone has, by ID.
    peer_images: Mutex<HashMap<DeviceId, Images>>,
}

impl Default for Current {
    fn default() -> Self {
        Self {
            local_layout: Mutex::new(DeckLayout::default_layout()),
            local_state: Mutex::new(DeckState::default()),
            peer_layouts: Mutex::new(HashMap::new()),
            peer_states: Mutex::new(HashMap::new()),
            local_images: Mutex::new(HashMap::new()),
            peer_images: Mutex::new(HashMap::new()),
        }
    }
}

impl Current {
    /// Replaces this PC's tile pictures; those that aren't small PNGs are
    /// left out. Their IDs are worked out here.
    pub fn set_local_images(&self, images: Vec<Vec<u8>>) {
        let images = images
            .into_iter()
            .filter(|png| deck_image::is_valid_png(png))
            .map(|png| (image_id(&png), Arc::new(png)))
            .collect();
        *self.local_images.lock().unwrap_or_else(|e| e.into_inner()) = images;
    }

    pub fn local_image(&self, id: &str) -> Option<Arc<Vec<u8>>> {
        self.local_images.lock().unwrap_or_else(|e| e.into_inner()).get(id).cloned()
    }

    pub fn peer_image(&self, peer: &DeviceId, id: &str) -> Option<Arc<Vec<u8>>> {
        self.peer_images.lock().unwrap_or_else(|e| e.into_inner()).get(peer)?.get(id).cloned()
    }

    fn set_peer_image(&self, peer: DeviceId, id: String, png: Vec<u8>) {
        self.peer_images
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(peer)
            .or_default()
            .insert(id, Arc::new(png));
    }

    /// Keeps only the pictures `layout` names; the IDs it names that this
    /// phone doesn't have yet.
    fn missing_peer_images(&self, peer: DeviceId, layout: &DeckLayout) -> Vec<String> {
        let wanted = image_ids(layout);
        let mut all = self.peer_images.lock().unwrap_or_else(|e| e.into_inner());
        let have = all.entry(peer).or_default();
        have.retain(|id, _| wanted.contains(id));
        let mut missing: Vec<String> = wanted.into_iter().filter(|id| !have.contains_key(id)).collect();
        missing.sort();
        missing
    }
    pub fn local_layout(&self) -> DeckLayout {
        self.local_layout.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set_local_layout(&self, layout: DeckLayout) -> bool {
        let mut guard = self.local_layout.lock().unwrap_or_else(|e| e.into_inner());
        if *guard == layout {
            false
        } else {
            *guard = layout;
            true
        }
    }

    pub fn local_state(&self) -> DeckState {
        self.local_state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set_local_state(&self, state: DeckState) -> bool {
        let mut guard = self.local_state.lock().unwrap_or_else(|e| e.into_inner());
        if *guard == state {
            false
        } else {
            *guard = state;
            true
        }
    }

    pub fn peer_layout(&self, peer: &DeviceId) -> Option<DeckLayout> {
        self.peer_layouts.lock().unwrap_or_else(|e| e.into_inner()).get(peer).cloned()
    }

    pub fn set_peer_layout(&self, peer: DeviceId, layout: DeckLayout) {
        self.peer_layouts.lock().unwrap_or_else(|e| e.into_inner()).insert(peer, layout);
    }

    pub fn peer_state(&self, peer: &DeviceId) -> Option<DeckState> {
        self.peer_states.lock().unwrap_or_else(|e| e.into_inner()).get(peer).cloned()
    }

    pub fn set_peer_state(&self, peer: DeviceId, state: DeckState) {
        self.peer_states.lock().unwrap_or_else(|e| e.into_inner()).insert(peer, state);
    }

    pub fn remove_peer(&self, peer: &DeviceId) {
        self.peer_layouts.lock().unwrap_or_else(|e| e.into_inner()).remove(peer);
        self.peer_states.lock().unwrap_or_else(|e| e.into_inner()).remove(peer);
    }
}

impl Shared {
    fn offers_deck(&self) -> bool {
        self.we_offer(DECK_ACTIONS)
    }

    fn offers_pc_audio(&self) -> bool {
        self.we_offer(PC_AUDIO)
    }

    fn offers_pc_radios(&self) -> bool {
        self.we_offer(PC_RADIOS)
    }

    fn deck_allowed(&self, peer: &DeviceId) -> bool {
        self.toggle_on(peer, INPUT_TOGGLE) || self.toggle_on(peer, crate::actions::POWER_TOGGLE)
    }

    /// Sends this PC's Deck layout and live state to `session` if this device
    /// offers `deck.actions` or `pc.audio`.
    pub(crate) async fn send_deck(&self, session: &Arc<Session>) {
        if !self.deck_allowed(&session.peer) {
            return;
        }
        if self.offers_deck() {
            let layout = self.deck.local_layout();
            if let Ok(env) = Envelope::new(types::DECK_LAYOUT, &layout) {
                let _ = session.send(env).await;
            }
        }
        if self.offers_deck() || self.offers_pc_audio() {
            let state = self.deck.local_state();
            if let Ok(env) = Envelope::new(types::DECK_STATE, &state) {
                let _ = session.send(env).await;
            }
        }
    }

    /// Updates this PC's Deck layout and sends it to connected peers.
    pub(crate) async fn set_deck_layout(&self, layout: DeckLayout) -> Result<()> {
        let clean = layout.sanitized().ok_or_else(|| Error::Protocol("invalid deck layout".into()))?;
        self.deck.set_local_layout(clean.clone());
        if self.offers_deck()
            && let Ok(env) = Envelope::new(types::DECK_LAYOUT, &clean)
        {
            for session in self.live_sessions() {
                if self.deck_allowed(&session.peer) {
                    let _ = session.send(env.clone()).await;
                }
            }
        }
        Ok(())
    }

    /// Updates this PC's live Deck state and sends it to connected peers when
    /// it changed.
    pub(crate) async fn set_deck_state(&self, state: DeckState) -> Result<()> {
        let clean = state.sanitized().ok_or_else(|| Error::Protocol("invalid deck state".into()))?;
        let changed = self.deck.set_local_state(clean.clone());
        if changed
            && (self.offers_deck() || self.offers_pc_audio())
            && let Ok(env) = Envelope::new(types::DECK_STATE, &clean)
        {
            for session in self.live_sessions() {
                if self.deck_allowed(&session.peer) {
                    let _ = session.send(env.clone()).await;
                }
            }
        }
        Ok(())
    }
}

/// Answers a phone's `deck.image`: the picture, to a phone the layout goes to.
fn serve_image(shared: &Shared, peer: &DeviceId, id: &str) -> Envelope {
    if !shared.offers_deck() {
        return Envelope::error(ErrorCode::Unsupported, "no Deck here");
    }
    if !shared.deck_allowed(peer) {
        return Envelope::error(ErrorCode::Denied, "the Deck is turned off for this device");
    }
    match shared.deck.local_image(id) {
        Some(png) => {
            Envelope::new(types::DECK_IMAGE_DATA, &DeckImage { id: id.to_owned(), png: png.to_vec() })
                .unwrap_or_else(|_| Envelope::error(ErrorCode::Internal, "can't send the picture"))
        }
        None => Envelope::error(ErrorCode::NotFound, "no such picture"),
    }
}

/// Phone side: gets the pictures a PC's new layout names that this phone
/// doesn't have, then tells the app the layout again so it shows them. A
/// picture that doesn't match its ID, or isn't a small PNG, is dropped: the
/// tile keeps its icon.
async fn fetch_images(shared: Arc<Shared>, session: Arc<Session>, ids: Vec<String>) {
    let peer = session.peer;
    let mut got = false;
    for id in ids {
        let Ok(env) = Envelope::new(types::DECK_IMAGE, &DeckImageGet { id: id.clone() }) else { continue };
        let reply = match session.request(env, REQUEST_TIMEOUT).await {
            Ok(reply) => reply,
            Err(error) => {
                tracing::debug!(%error, "deck picture not fetched");
                if !session.is_alive() {
                    return;
                }
                continue;
            }
        };
        let Ok(image) = reply.expect_body::<DeckImage>(types::DECK_IMAGE_DATA) else { continue };
        if image.id == id && deck_image::is_valid_png(&image.png) && image_id(&image.png) == id {
            shared.deck.set_peer_image(peer, id, image.png);
            got = true;
        } else {
            tracing::debug!("a deck picture didn't match its ID; the tile keeps its icon");
        }
    }
    if got && let Some(layout) = shared.deck.peer_layout(&peer) {
        shared.emit(NodeEvent::DeckLayout { device: peer, layout });
    }
}

/// Asks a paired PC to run the action bound to `tile` (`deck.press`).
pub(crate) async fn press(shared: &Arc<Shared>, session: &Arc<Session>, tile: String) -> Result<()> {
    let req = DeckPress { tile };
    if !req.is_valid() {
        return Err(Error::Protocol("invalid deck tile ID".into()));
    }
    let peer_caps = shared.store.get_peer(&session.peer)?.map(|p| p.caps).unwrap_or_default();
    if !peer_caps.contains(DECK_ACTIONS) {
        return Err(Error::Unsupported);
    }
    let env = Envelope::new(types::DECK_PRESS, &req)?;
    session.request(env, REQUEST_TIMEOUT).await?.expect(types::OK)?;
    Ok(())
}

/// Asks a paired PC to change its master speaker volume (`0..=100`) and/or mute
/// state (`pc.audio.set`).
pub(crate) async fn set_pc_audio(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    volume: Option<u8>,
    muted: Option<bool>,
) -> Result<()> {
    send_pc_audio(shared, session, PcAudioSet { volume, muted, output: None }).await
}

/// Asks a paired PC to make one of its `output_devices` its default audio
/// output (`pc.audio.set { output }`).
pub(crate) async fn set_pc_audio_output(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    output: String,
) -> Result<()> {
    send_pc_audio(shared, session, PcAudioSet { volume: None, muted: None, output: Some(output) }).await
}

async fn send_pc_audio(shared: &Arc<Shared>, session: &Arc<Session>, req: PcAudioSet) -> Result<()> {
    if !req.is_valid() {
        return Err(Error::Protocol("invalid PC audio request".into()));
    }
    let peer_caps = shared.store.get_peer(&session.peer)?.map(|p| p.caps).unwrap_or_default();
    if !peer_caps.contains(PC_AUDIO) && !peer_caps.contains(DECK_ACTIONS) {
        return Err(Error::Unsupported);
    }
    let env = Envelope::new(types::PC_AUDIO_SET, &req)?;
    session.request(env, REQUEST_TIMEOUT).await?.expect(types::OK)?;
    Ok(())
}

/// Asks a paired PC to turn its Wi-Fi and/or Bluetooth on or off (`pc.radio.set`, #636).
pub(crate) async fn set_pc_radio(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    wifi: Option<bool>,
    bluetooth: Option<bool>,
) -> Result<()> {
    let req = PcRadioSet { wifi, bluetooth };
    if !req.is_valid() {
        return Err(Error::Protocol("invalid PC radio request".into()));
    }
    let peer_caps = shared.store.get_peer(&session.peer)?.map(|p| p.caps).unwrap_or_default();
    if !peer_caps.contains(PC_RADIOS) {
        return Err(Error::Unsupported);
    }
    let env = Envelope::new(types::PC_RADIO_SET, &req)?;
    session.request(env, REQUEST_TIMEOUT).await?.expect(types::OK)?;
    Ok(())
}

/// Handles `deck.layout`, `deck.state`, `deck.press`, `pc.audio.set` and `pc.radio.set`.
pub(crate) async fn handle(shared: &Arc<Shared>, session: &Arc<Session>, env: &Envelope) -> Result<bool> {
    let peer = session.peer;
    match env.t.as_str() {
        types::DECK_LAYOUT => {
            let layout: DeckLayout = env.body()?;
            if let Some(clean) = layout.sanitized() {
                shared.deck.set_peer_layout(peer, clean.clone());
                let missing = shared.deck.missing_peer_images(peer, &clean);
                shared.emit(NodeEvent::DeckLayout { device: peer, layout: clean });
                if !missing.is_empty() {
                    tokio::spawn(fetch_images(shared.clone(), session.clone(), missing));
                }
            }
        }
        types::DECK_IMAGE => {
            let reply = match env.body::<DeckImageGet>() {
                Ok(req) if req.is_valid() => serve_image(shared, &peer, &req.id),
                _ => Envelope::error(ErrorCode::BadMessage, "invalid deck.image body"),
            };
            session.send(reply.reply_to(env.id)).await?;
        }
        types::DECK_STATE => {
            let state: DeckState = env.body()?;
            if let Some(clean) = state.sanitized() {
                shared.deck.set_peer_state(peer, clean.clone());
                shared.emit(NodeEvent::DeckState { device: peer, state: clean });
            }
        }
        types::DECK_PRESS => {
            let req: DeckPress = env.body()?;
            let reply = run_press(shared, &peer, req).await;
            session.send(reply.reply_to(env.id)).await?;
        }
        types::PC_AUDIO_SET => {
            let reply = match env.body::<PcAudioSet>() {
                Ok(req) => run_pc_audio_set(shared, &peer, req).await,
                Err(_) => Envelope::error(ErrorCode::BadMessage, "invalid pc.audio.set body"),
            };
            session.send(reply.reply_to(env.id)).await?;
        }
        types::PC_RADIO_SET => {
            let reply = match env.body::<PcRadioSet>() {
                Ok(req) => run_pc_radio_set(shared, &peer, req).await,
                Err(_) => Envelope::error(ErrorCode::BadMessage, "invalid pc.radio.set body"),
            };
            session.send(reply.reply_to(env.id)).await?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// `pc.audio.set` volume values that mean "start" and "stop playing this PC's audio on the
/// phone" (Use phone as speaker), outside the 0..=100 volume range.
pub const PHONE_SPEAKER_ON: u8 = 201;
pub const PHONE_SPEAKER_OFF: u8 = 200;

async fn run_pc_audio_set(shared: &Arc<Shared>, peer: &DeviceId, req: PcAudioSet) -> Envelope {
    if !req.is_valid() {
        return Envelope::error(ErrorCode::BadMessage, "invalid volume or mute");
    }
    if !shared.offers_pc_audio() && !shared.offers_deck() {
        return Envelope::error(ErrorCode::Unsupported, "PC audio control is not supported here");
    }
    let is_phone_speaker_toggle = req.volume.is_some_and(|v| v == PHONE_SPEAKER_OFF || v == PHONE_SPEAKER_ON)
        && req.muted.is_none()
        && req.output.is_none();
    // Playing this PC's sound on a phone shares what the PC plays, so it
    // needs that phone's media permission. Stopping is always allowed.
    if is_phone_speaker_toggle
        && req.volume == Some(PHONE_SPEAKER_ON)
        && !shared.toggle_on(peer, crate::media::TOGGLE)
    {
        return Envelope::error(ErrorCode::Denied, "media is turned off for this device");
    }
    if !is_phone_speaker_toggle
        && !shared.toggle_on(peer, crate::actions::POWER_TOGGLE)
        && !shared.toggle_on(peer, INPUT_TOGGLE)
    {
        return Envelope::error(ErrorCode::Denied, "PC actions are turned off for this device");
    }
    let platform = shared.platform.clone();
    let from = *peer;
    let vol = req.volume;
    let muted = req.muted;
    let output = req.output.clone();
    let ran = tokio::task::spawn_blocking(move || {
        if vol.is_some() || muted.is_some() {
            platform.set_pc_audio(&from, vol, muted)?;
        }
        match &output {
            Some(id) => platform.set_pc_audio_output(&from, id),
            None => Ok(()),
        }
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));
    match ran {
        Ok(()) => {
            if !is_phone_speaker_toggle {
                let mut state = shared.deck.local_state();
                if let Some(id) = &req.output {
                    for device in &mut state.output_devices {
                        device.is_default = device.id == *id;
                    }
                }
                if let Some(v) = vol
                    && v <= 100
                {
                    state.volume = v;
                }
                if let Some(m) = muted {
                    state.muted = m;
                }
                let _ = shared.set_deck_state(state).await;
            }
            Envelope::empty(types::OK)
        }
        Err(reason) => {
            tracing::warn!(reason, "pc.audio.set failed");
            Envelope::error(ErrorCode::Internal, "failed")
        }
    }
}

/// Turns this PC's radios on or off for a phone, with the same permission
/// as its volume: PC actions or remote input on for that phone.
async fn run_pc_radio_set(shared: &Arc<Shared>, peer: &DeviceId, req: PcRadioSet) -> Envelope {
    if !req.is_valid() {
        return Envelope::error(ErrorCode::BadMessage, "nothing to turn on or off");
    }
    if !shared.offers_pc_radios() {
        return Envelope::error(ErrorCode::Unsupported, "this PC can't switch its radios");
    }
    if !shared.toggle_on(peer, crate::actions::POWER_TOGGLE) && !shared.toggle_on(peer, INPUT_TOGGLE) {
        return Envelope::error(ErrorCode::Denied, "PC actions are turned off for this device");
    }
    let platform = shared.platform.clone();
    let from = *peer;
    let (wifi, bluetooth) = (req.wifi, req.bluetooth);
    let ran = tokio::task::spawn_blocking(move || platform.set_pc_radio(&from, wifi, bluetooth))
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
    match ran {
        Ok(()) => {
            let mut state = shared.deck.local_state();
            state.wifi = wifi.or(state.wifi);
            state.bluetooth = bluetooth.or(state.bluetooth);
            let _ = shared.set_deck_state(state).await;
            Envelope::empty(types::OK)
        }
        Err(reason) => {
            tracing::warn!(reason, "pc.radio.set failed");
            Envelope::error(ErrorCode::Internal, "failed")
        }
    }
}

async fn run_press(shared: &Arc<Shared>, peer: &DeviceId, req: DeckPress) -> Envelope {
    if !req.is_valid() {
        return Envelope::error(ErrorCode::BadMessage, "invalid tile ID");
    }
    if !shared.offers_deck() {
        return Envelope::error(ErrorCode::Unsupported, "deck actions are not supported here");
    }
    if !shared.toggle_on(peer, INPUT_TOGGLE) {
        let first_time = shared.remote.lock().unwrap_or_else(|e| e.into_inner()).mark_prompted(*peer);
        if first_time {
            shared.emit(NodeEvent::RemoteInputRequested { device: *peer });
        }
        return Envelope::error(ErrorCode::Denied, "remote input is turned off for this device");
    }
    let layout = shared.deck.local_layout();
    let Some(tile) = layout.tile(&req.tile).cloned() else {
        return Envelope::error(ErrorCode::NotFound, "unknown tile");
    };
    if (tile.kind == deck_kinds::RUN_COMMAND || tile.needs_commands)
        && !shared.toggle_on(peer, COMMANDS_TOGGLE)
    {
        return Envelope::error(ErrorCode::Denied, "commands are turned off for this device");
    }
    let platform = shared.platform.clone();
    let from = *peer;
    let id = tile.id;
    let ran = tokio::task::spawn_blocking(move || platform.deck_press(&from, &id))
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
    match ran {
        Ok(()) => Envelope::empty(types::OK),
        Err(reason) => {
            tracing::warn!(reason, "deck action failed");
            Envelope::error(ErrorCode::Internal, "failed")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(action: DeckAction, delay_ms: u32) -> MacroStep {
        MacroStep { action, delay_ms }
    }

    #[test]
    fn a_macro_is_some_ordinary_actions_with_short_pauses() {
        let ok = DeckAction::Macro {
            steps: vec![step(DeckAction::MediaPlayPause, 500), step(DeckAction::LockPc, 0)],
        };
        assert!(ok.is_valid());
        assert_eq!(ok.kind(), deck_kinds::MACRO);
        assert!(!ok.requires_commands_toggle());
        assert_eq!(ok.summary(), "Play or pause media, then Lock this PC");

        assert!(!DeckAction::Macro { steps: Vec::new() }.is_valid());
        let too_many = vec![step(DeckAction::VolumeUp, 0); MAX_MACRO_STEPS + 1];
        assert!(!DeckAction::Macro { steps: too_many }.is_valid());
        let long_pause = vec![step(DeckAction::VolumeUp, MAX_MACRO_DELAY_MS + 1)];
        assert!(!DeckAction::Macro { steps: long_pause }.is_valid());
        let long_in_all = vec![step(DeckAction::VolumeUp, MAX_MACRO_DELAY_MS); 4];
        assert!(!DeckAction::Macro { steps: long_in_all }.is_valid());
        // Nothing nested, and no unconfirmed restart or shut down.
        for inner in [ok.clone(), DeckAction::RestartPc, DeckAction::ShutDownPc] {
            assert!(!DeckAction::Macro { steps: vec![step(inner, 0)] }.is_valid());
        }
        let bad_step = DeckAction::OpenUrl { url: "javascript:alert(1)".into() };
        assert!(!DeckAction::Macro { steps: vec![step(bad_step, 0)] }.is_valid());
    }

    #[test]
    fn a_macro_with_a_command_needs_the_commands_toggle_on_the_wire() {
        let action = DeckAction::Macro {
            steps: vec![
                step(DeckAction::RunCommand { command: "echo hi".into() }, 100),
                step(DeckAction::VolumeMute, 0),
            ],
        };
        assert!(action.requires_commands_toggle());
        let tile = DeckTileConfig {
            image: None,
            id: "m1".into(),
            label: "Wind down".into(),
            icon: deck_icons::BOLT.into(),
            color: deck_colors::VIOLET.into(),
            action,
        };
        assert!(tile.is_valid());
        let wire = tile.to_wire();
        assert_eq!(wire.kind, deck_kinds::MACRO);
        assert!(wire.needs_commands);
        // The command itself never goes to the phone.
        let json = serde_json::to_string(&wire).unwrap();
        assert!(!json.contains("echo"));
        // A run_command tile says so by its kind, and the flag stays off the wire.
        let plain =
            DeckTileConfig { action: DeckAction::RunCommand { command: "echo hi".into() }, ..tile }.to_wire();
        assert!(!plain.needs_commands);
        assert!(!serde_json::to_string(&plain).unwrap().contains("needs_commands"));
    }

    #[test]
    fn power_tiles_and_macros_keep_their_steps_on_the_pc() {
        let config = DeckConfig {
            pages: vec![DeckPageConfig {
                id: "p".into(),
                name: "Main".into(),
                tiles: vec![
                    DeckTileConfig {
                        image: None,
                        id: "off".into(),
                        label: "Shut down".into(),
                        icon: deck_icons::POWER.into(),
                        color: deck_colors::RED.into(),
                        action: DeckAction::ShutDownPc,
                    },
                    DeckTileConfig {
                        image: None,
                        id: "m".into(),
                        label: "Pause and lock".into(),
                        icon: deck_icons::BOLT.into(),
                        color: deck_colors::VIOLET.into(),
                        action: DeckAction::Macro {
                            steps: vec![step(DeckAction::MediaPlayPause, 1000), step(DeckAction::LockPc, 0)],
                        },
                    },
                ],
            }],
            toggles_on: BTreeSet::new(),
        };
        let saved = serde_json::to_string(&config).unwrap();
        let back: DeckConfig = serde_json::from_str(&saved).unwrap();
        assert_eq!(back, config);
        assert!(back.sanitized().is_some_and(|c| c.pages[0].tiles.len() == 2));
        let layout = config.to_wire_layout();
        assert!(layout.is_valid());
        assert!(deck_kinds::needs_confirmation(&layout.pages[0].tiles[0].kind));
        assert!(!deck_kinds::needs_confirmation(&layout.pages[0].tiles[1].kind));
    }

    fn toggle_tile(id: &str, on: DeckAction, off: DeckAction) -> DeckTileConfig {
        DeckTileConfig {
            image: None,
            id: id.into(),
            label: "Monitors".into(),
            icon: deck_icons::TOGGLE.into(),
            color: deck_colors::GREEN.into(),
            action: DeckAction::Toggle { on: Box::new(on), off: Box::new(off) },
        }
    }

    fn deck_with(tiles: Vec<DeckTileConfig>) -> DeckConfig {
        DeckConfig {
            pages: vec![DeckPageConfig { id: "p".into(), name: "Main".into(), tiles }],
            toggles_on: BTreeSet::new(),
        }
    }

    #[test]
    fn a_toggle_runs_on_then_off_in_turn_and_keeps_where_it_is() {
        let on = DeckAction::RunCommand { command: "niri msg action power-on-monitors".into() };
        let off = DeckAction::RunCommand { command: "niri msg action power-off-monitors".into() };
        let mut deck = deck_with(vec![toggle_tile("mon", on.clone(), off.clone())]);
        assert!(deck.is_valid());

        let (run, after) = deck.toggle_press("mon").unwrap();
        assert_eq!((run, after), (on.clone(), true));
        deck.set_toggle("mon", after);
        assert_eq!(deck.toggles_on_list(), vec!["mon".to_owned()]);
        let (run, after) = deck.toggle_press("mon").unwrap();
        assert_eq!((run, after), (off, false));
        deck.set_toggle("mon", after);
        assert!(deck.toggles_on_list().is_empty());

        // Kept in deck.json, and only for toggles that are still there.
        deck.set_toggle("mon", true);
        deck.set_toggle("gone", true);
        let back: DeckConfig = serde_json::from_str(&serde_json::to_string(&deck).unwrap()).unwrap();
        assert_eq!(back.sanitized().unwrap().toggles_on_list(), vec!["mon".to_owned()]);
        // Any other tile isn't a toggle.
        assert!(DeckConfig::default_deck().toggle_press("play_pause").is_none());
    }

    #[test]
    fn a_toggle_sides_are_plain_actions_and_commands_need_the_toggle() {
        let command = DeckAction::RunCommand { command: "echo on".into() };
        let toggle = |on, off| DeckAction::Toggle { on: Box::new(on), off: Box::new(off) };
        assert!(toggle(DeckAction::MicMute, DeckAction::MicMute).is_valid());
        assert!(!toggle(DeckAction::ShutDownPc, DeckAction::MicMute).is_valid());
        assert!(!toggle(DeckAction::MicMute, toggle(DeckAction::MicMute, DeckAction::MicMute)).is_valid());
        let with_command = toggle(command, DeckAction::MicMute);
        assert!(with_command.requires_commands_toggle());
        assert!(!with_command.fits_in_macro());
        let wire =
            toggle_tile("t", DeckAction::MicMute, DeckAction::RunCommand { command: "x".into() }).to_wire();
        assert_eq!(wire.kind, deck_kinds::TOGGLE);
        assert!(wire.needs_commands && wire.is_valid());
    }

    #[test]
    fn a_phone_reads_which_toggles_are_on() {
        let state = DeckState { toggles_on: vec!["mon".into()], ..DeckState::default() };
        let back: DeckState = serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        assert!(back.is_valid() && back.is_on("mon") && !back.is_on("other"));
        // None on: left out, so older phones see what they always did.
        assert!(!serde_json::to_string(&DeckState::default()).unwrap().contains("toggles_on"));
    }

    #[test]
    fn an_older_phone_drops_tiles_it_does_not_know_and_keeps_the_rest() {
        // What a phone from before these kinds makes of a layout with them:
        // the tile goes, the page stays.
        let json = r#"{"pages":[{"id":"p","name":"Main","tiles":[
            {"id":"a","label":"Mute","icon":"volume_off","color":"teal","kind":"volume_mute"},
            {"id":"b","label":"Later","icon":"bolt","color":"violet","kind":"some_future_kind","needs_commands":true}
        ]}]}"#;
        let layout: DeckLayout = serde_json::from_str(json).unwrap();
        let clean = layout.sanitized().unwrap();
        assert_eq!(clean.pages[0].tiles.len(), 1);
        assert_eq!(clean.pages[0].tiles[0].id, "a");
    }

    #[test]
    fn default_deck_matches_wire_layout_and_is_valid() {
        let cfg = DeckConfig::default_deck();
        assert!(cfg.is_valid());
        let wire = cfg.to_wire_layout();
        assert_eq!(wire, DeckLayout::default_layout());
        assert!(wire.is_valid());
    }

    #[test]
    fn action_validation_enforces_http_exe_lnk_and_shortcuts() {
        assert!(DeckAction::OpenUrl { url: "https://example.com/path?q=1".into() }.is_valid());
        assert!(DeckAction::OpenUrl { url: "http://localhost:8080".into() }.is_valid());
        assert!(!DeckAction::OpenUrl { url: "javascript:alert(1)".into() }.is_valid());
        assert!(!DeckAction::OpenUrl { url: "file:///C:/Windows/System32/cmd.exe".into() }.is_valid());
        assert!(!DeckAction::OpenUrl { url: "ftp://example.com".into() }.is_valid());

        assert!(DeckAction::LaunchApp { path: r"C:\Windows\System32\calc.exe".into() }.is_valid());
        assert!(DeckAction::LaunchApp { path: r"C:\Users\me\Desktop\OBS Studio.LNK".into() }.is_valid());
        assert!(!DeckAction::LaunchApp { path: r"C:\scripts\evil.bat".into() }.is_valid());
        assert!(!DeckAction::LaunchApp { path: r"C:\scripts\evil.ps1".into() }.is_valid());

        assert!(DeckAction::LaunchApp { path: "/usr/bin/firefox".into() }.is_valid());
        assert!(DeckAction::LaunchApp { path: "org.mozilla.firefox.desktop".into() }.is_valid());
        assert!(DeckAction::LaunchApp { path: "/opt/app/MyApp.AppImage".into() }.is_valid());
        assert!(DeckAction::LaunchApp { path: "/home/alice/.local/bin/obsidian".into() }.is_valid());
        assert!(DeckAction::LaunchApp { path: "~/.local/bin/firefox".into() }.is_valid());
        assert!(DeckAction::LaunchApp { path: "/run/current-system/sw/bin/alacritty".into() }.is_valid());
        assert!(DeckAction::LaunchApp { path: "/usr/games/steam".into() }.is_valid());
        assert!(!DeckAction::LaunchApp { path: "/usr/local/bin/start.sh".into() }.is_valid());
        assert!(!DeckAction::LaunchApp { path: "/usr/bin/sh".into() }.is_valid());
        assert!(!DeckAction::LaunchApp { path: "/usr/bin/bash".into() }.is_valid());
        assert!(!DeckAction::LaunchApp { path: "/home/alice/.local/bin/zsh".into() }.is_valid());
        assert!(!DeckAction::LaunchApp { path: "~/.local/bin/zsh".into() }.is_valid());
        assert!(!DeckAction::LaunchApp { path: "/home/alice/.local/bin/deploy.sh".into() }.is_valid());
        assert!(!DeckAction::LaunchApp { path: "~/.local/bin/deploy.sh".into() }.is_valid());
        assert!(!DeckAction::LaunchApp { path: "/etc/passwd".into() }.is_valid());
        assert!(!DeckAction::LaunchApp { path: "/tmp/payload".into() }.is_valid());
        assert!(!DeckAction::LaunchApp { path: "/var/tmp/sub/payload".into() }.is_valid());
        assert!(!DeckAction::LaunchApp { path: "/dev/shm/payload".into() }.is_valid());
        assert!(!DeckAction::LaunchApp { path: "./payload".into() }.is_valid());
        assert!(!DeckAction::LaunchApp { path: "/payload".into() }.is_valid());
        assert!(!DeckAction::LaunchApp { path: "~/payload".into() }.is_valid());
        assert!(!DeckAction::LaunchApp { path: "~/".into() }.is_valid());
        assert!(!DeckAction::LaunchApp { path: ".exe".into() }.is_valid());
        assert!(!DeckAction::LaunchApp { path: ".desktop".into() }.is_valid());
        assert!(!DeckAction::LaunchApp { path: "../bin/sh".into() }.is_valid());
        assert!(!DeckAction::LaunchApp { path: "/usr/../etc/passwd".into() }.is_valid());
        assert!(!DeckAction::LaunchApp { path: "~/.local/../bin/firefox".into() }.is_valid());
        assert!(!DeckAction::LaunchApp { path: "relative/bin/firefox".into() }.is_valid());
        assert!(!DeckAction::LaunchApp { path: "/".into() }.is_valid());

        assert!(DeckAction::Shortcut { key: "m".into(), mods: vec![KeyMod::Ctrl, KeyMod::Shift] }.is_valid());
        assert!(DeckAction::Shortcut { key: remote_keys::F5.into(), mods: vec![] }.is_valid());
        assert!(!DeckAction::Shortcut { key: "m".into(), mods: vec![KeyMod::Ctrl, KeyMod::Ctrl] }.is_valid());
        assert!(!DeckAction::Shortcut { key: "unknown".into(), mods: vec![] }.is_valid());

        assert!(DeckAction::TypeText { text: "Hello\nworld".into() }.is_valid());
        assert!(!DeckAction::TypeText { text: String::new() }.is_valid());
        assert!(!DeckAction::TypeText { text: "bad\u{0}char".into() }.is_valid());

        assert!(DeckAction::RunCommand { command: "cargo test --workspace".into() }.is_valid());
        assert!(!DeckAction::RunCommand { command: "   ".into() }.is_valid());
        assert!(!DeckAction::RunCommand { command: "line1\nline2".into() }.is_valid());
    }

    #[test]
    fn wire_layout_and_debug_never_expose_paths_urls_or_commands() {
        let mut cfg = DeckConfig::default_deck();
        cfg.pages[0].tiles.push(DeckTileConfig {
            image: None,
            id: "secret_cmd".into(),
            label: "Deploy".into(),
            icon: deck_icons::TERMINAL.into(),
            color: deck_colors::CORAL.into(),
            action: DeckAction::RunCommand { command: "echo super-secret-token".into() },
        });
        cfg.pages[0].tiles.push(DeckTileConfig {
            image: None,
            id: "secret_app".into(),
            label: "Calculator".into(),
            icon: deck_icons::APP.into(),
            color: deck_colors::TEAL.into(),
            action: DeckAction::LaunchApp { path: r"C:\Private\secret_app.exe".into() },
        });
        cfg.pages[0].tiles.push(DeckTileConfig {
            image: None,
            id: "secret_url".into(),
            label: "Dashboard".into(),
            icon: deck_icons::GLOBE.into(),
            color: deck_colors::BLUE.into(),
            action: DeckAction::OpenUrl { url: "https://internal.example.com/secret".into() },
        });

        let dbg = format!("{cfg:?}");
        assert!(!dbg.contains("super-secret-token") && !dbg.contains("secret_app.exe"), "{dbg}");

        let wire = cfg.to_wire_layout();
        let cbor = Envelope::new(types::DECK_LAYOUT, &wire).unwrap().to_cbor();
        let cbor_text = String::from_utf8_lossy(&cbor);
        assert!(!cbor_text.contains("super-secret-token"));
        assert!(!cbor_text.contains("secret_app.exe"));
        assert!(!cbor_text.contains("internal.example.com"));
        assert_eq!(wire.tile("secret_cmd").unwrap().kind, deck_kinds::RUN_COMMAND);
        assert_eq!(wire.tile("secret_app").unwrap().kind, deck_kinds::LAUNCH_APP);
        assert_eq!(wire.tile("secret_url").unwrap().kind, deck_kinds::OPEN_URL);
    }

    #[test]
    fn deck_config_persists_to_disk_and_falls_back_on_corrupt_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deck.json");

        // Missing file returns the default deck.
        assert_eq!(DeckConfig::load(&path), DeckConfig::default_deck());

        let mut custom = DeckConfig::default_deck();
        custom.pages.push(DeckPageConfig {
            id: "custom".into(),
            name: "Work".into(),
            tiles: vec![DeckTileConfig {
                image: None,
                id: "site".into(),
                label: "Docs".into(),
                icon: deck_icons::GLOBE.into(),
                color: deck_colors::BLUE.into(),
                action: DeckAction::OpenUrl { url: "https://example.com".into() },
            }],
        });
        custom.save(&path).unwrap();
        assert_eq!(DeckConfig::load(&path), custom);

        // Corrupt JSON falls back to default_deck(), keeping the file aside.
        std::fs::write(&path, b"{not valid json").unwrap();
        assert_eq!(DeckConfig::load(&path), DeckConfig::default_deck());
        assert_eq!(std::fs::read(path.with_extension("json.bad")).unwrap(), b"{not valid json");
        assert!(!dir.path().join("deck.json.tmp").exists());
    }
}
