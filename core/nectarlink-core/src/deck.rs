// SPDX-License-Identifier: MPL-2.0
//! Phone as a macro pad for a PC (`docs/protocol/deck.md`).
//!
//! A PC that offers `deck.actions` shares its Deck layout (`deck.layout`) and
//! live tile state (`deck.state`) when a session starts and after every edit
//! or state change. A phone sends `deck.press { tile }` by tile ID only; the
//! PC checks the `remote_input` toggle (and the `commands` toggle for
//! `run_command` tiles) before running the tile's action.

use std::{
    collections::{BTreeSet, HashMap},
    path::Path,
    sync::{Arc, Mutex},
};

pub use nectarlink_protocol::messages::{
    AudioOutputDevice, DeckLayout, DeckPage, DeckPress, DeckState, DeckTile, KeyMod, PcAudioSet,
    deck::{
        ACTIONS as DECK_ACTIONS, MAX_ID_BYTES as MAX_DECK_ID_BYTES, MAX_LABEL_BYTES as MAX_DECK_LABEL_BYTES,
        MAX_PAGES as MAX_DECK_PAGES, MAX_TILES_PER_PAGE as MAX_DECK_TILES_PER_PAGE, PC_AUDIO,
    },
    deck_colors, deck_icons, deck_kinds, is_valid_deck_id, remote_keys,
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
        }
    }

    /// Whether this action requires the per-device `commands` toggle.
    pub fn requires_commands_toggle(&self) -> bool {
        matches!(self, Self::RunCommand { .. })
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
            | Self::Screenshot => true,
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

/// Validates that `path` is a non-empty `.exe` or `.lnk` file path on the PC.
pub fn is_valid_app_path(path: &str) -> bool {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_PATH_BYTES || trimmed.chars().any(char::is_control) {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    (lower.ends_with(".exe") || lower.ends_with(".lnk")) && trimmed.len() > 4
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
        }
    }
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
        (!pages.is_empty()).then_some(DeckConfig { pages })
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
        }
    }

    /// Loads `DeckConfig` from `path`, falling back to [`Self::default_deck`]
    /// when the file does not exist or cannot be parsed/sanitized.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str::<DeckConfig>(&s).ok())
            .and_then(DeckConfig::sanitized)
            .unwrap_or_else(Self::default_deck)
    }

    /// Writes `self` as pretty-printed JSON to `path`.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, json)
    }
}

impl Default for DeckConfig {
    fn default() -> Self {
        Self::default_deck()
    }
}

/// This PC's Deck layout and live state (sent to newly connected phones) and
/// the latest layout and state received from each connected PC.
pub(crate) struct Current {
    local_layout: Mutex<DeckLayout>,
    local_state: Mutex<DeckState>,
    peer_layouts: Mutex<HashMap<DeviceId, DeckLayout>>,
    peer_states: Mutex<HashMap<DeviceId, DeckState>>,
}

impl Default for Current {
    fn default() -> Self {
        Self {
            local_layout: Mutex::new(DeckLayout::default_layout()),
            local_state: Mutex::new(DeckState::default()),
            peer_layouts: Mutex::new(HashMap::new()),
            peer_states: Mutex::new(HashMap::new()),
        }
    }
}

impl Current {
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
        self.local_capabilities().iter().any(|c| c == DECK_ACTIONS)
    }

    fn offers_pc_audio(&self) -> bool {
        self.local_capabilities().iter().any(|c| c == PC_AUDIO)
    }

    /// Sends this PC's Deck layout and live state to `session` if this device
    /// offers `deck.actions` or `pc.audio`.
    pub(crate) async fn send_deck(&self, session: &Arc<Session>) {
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
                let _ = session.send(env.clone()).await;
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
                let _ = session.send(env.clone()).await;
            }
        }
        Ok(())
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
    let req = PcAudioSet { volume, muted };
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

/// Handles `deck.layout`, `deck.state`, `deck.press`, and `pc.audio.set`.
pub(crate) async fn handle(shared: &Arc<Shared>, session: &Arc<Session>, env: &Envelope) -> Result<bool> {
    let peer = session.peer;
    match env.t.as_str() {
        types::DECK_LAYOUT => {
            let layout: DeckLayout = env.body()?;
            if let Some(clean) = layout.sanitized() {
                shared.deck.set_peer_layout(peer, clean.clone());
                shared.emit(NodeEvent::DeckLayout { device: peer, layout: clean });
            }
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
        _ => return Ok(false),
    }
    Ok(true)
}

async fn run_pc_audio_set(shared: &Arc<Shared>, peer: &DeviceId, req: PcAudioSet) -> Envelope {
    if !req.is_valid() {
        return Envelope::error(ErrorCode::BadMessage, "invalid volume or mute");
    }
    if !shared.offers_pc_audio() && !shared.offers_deck() {
        return Envelope::error(ErrorCode::Unsupported, "PC audio control is not supported here");
    }
    if !shared.toggle_on(peer, crate::actions::POWER_TOGGLE) && !shared.toggle_on(peer, INPUT_TOGGLE) {
        return Envelope::error(ErrorCode::Denied, "PC actions are turned off for this device");
    }
    let platform = shared.platform.clone();
    let from = *peer;
    let vol = req.volume;
    let muted = req.muted;
    let ran = tokio::task::spawn_blocking(move || platform.set_pc_audio(&from, vol, muted))
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
    match ran {
        Ok(()) => {
            let mut state = shared.deck.local_state();
            if let Some(v) = vol {
                state.volume = v.min(100);
            }
            if let Some(m) = muted {
                state.muted = m;
            }
            let _ = shared.set_deck_state(state).await;
            Envelope::empty(types::OK)
        }
        Err(reason) => {
            tracing::warn!(reason, "pc.audio.set failed");
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
    if tile.kind == deck_kinds::RUN_COMMAND && !shared.toggle_on(peer, COMMANDS_TOGGLE) {
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
        assert!(!DeckAction::LaunchApp { path: ".exe".into() }.is_valid());

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
            id: "secret_cmd".into(),
            label: "Deploy".into(),
            icon: deck_icons::TERMINAL.into(),
            color: deck_colors::CORAL.into(),
            action: DeckAction::RunCommand { command: "echo super-secret-token".into() },
        });
        cfg.pages[0].tiles.push(DeckTileConfig {
            id: "secret_app".into(),
            label: "Calculator".into(),
            icon: deck_icons::APP.into(),
            color: deck_colors::TEAL.into(),
            action: DeckAction::LaunchApp { path: r"C:\Private\secret_app.exe".into() },
        });
        cfg.pages[0].tiles.push(DeckTileConfig {
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
                id: "site".into(),
                label: "Docs".into(),
                icon: deck_icons::GLOBE.into(),
                color: deck_colors::BLUE.into(),
                action: DeckAction::OpenUrl { url: "https://example.com".into() },
            }],
        });
        custom.save(&path).unwrap();
        assert_eq!(DeckConfig::load(&path), custom);

        // Corrupt JSON falls back to default_deck().
        std::fs::write(&path, b"{not valid json").unwrap();
        assert_eq!(DeckConfig::load(&path), DeckConfig::default_deck());
    }
}
