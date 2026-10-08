// SPDX-License-Identifier: GPL-3.0-or-later
//! PC-owned Deck configuration, live state syncing, and action execution
//! (`docs/protocol/deck.md`).

use std::{
    path::{Path, PathBuf},
    sync::{
        Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime},
};

use nectarlink_core::{
    DeckAction, DeckConfig, DeckPageConfig, DeckState, DeckTileConfig, DeviceId, KeyMod, deck_colors,
    deck_icons, deck_kinds,
};

use crate::{core_host, links, state::Changes, win};

const FILE_NAME: &str = "deck.json";

struct RuntimeState {
    config: DeckConfig,
    state: DeckState,
    path: Option<PathBuf>,
    last_mtime: Option<SystemTime>,
}

static DECK: Mutex<Option<RuntimeState>> = Mutex::new(None);
static MEDIA_PLAYING: AtomicBool = AtomicBool::new(false);
static HAS_MEDIA_SESSION: AtomicBool = AtomicBool::new(false);
static PAGE_ACTIVE: AtomicBool = AtomicBool::new(false);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn file_mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn sample_live_state() -> DeckState {
    let (volume, muted) = win::audio::speaker_state().unwrap_or((50, false));
    let mic_muted = win::audio::mic_muted();
    let playing = MEDIA_PLAYING.load(Ordering::Relaxed);
    let output_devices = win::audio::output_devices();
    DeckState { volume, muted, mic_muted, playing, output_devices }
}

/// Marks whether the desktop Deck page is currently visible so idle background
/// polling can pause when no paired device is online.
pub fn set_page_active(active: bool) {
    let prev = PAGE_ACTIVE.swap(active, Ordering::Relaxed);
    if active && !prev {
        core_host::spawn(async {
            reload_from_disk_if_modified();
            refresh_live_state().await;
        });
    }
}

/// Loads `deck.json` from `data_dir` and starts the background poller that
/// keeps speaker volume, microphone mute, output devices, and external
/// `deck.json` edits in sync.
pub fn init(data_dir: &Path) {
    let path = data_dir.join(FILE_NAME);
    if !path.exists() {
        let default = DeckConfig::default_deck();
        let _ = default.save(&path);
    }
    let config = DeckConfig::load(&path);
    let mtime = file_mtime(&path);
    let initial_state = DeckState {
        volume: 50,
        muted: false,
        mic_muted: Some(false),
        playing: false,
        output_devices: Vec::new(),
    };
    let wire_layout = config.to_wire_layout();
    {
        let mut guard = lock(&DECK);
        *guard = Some(RuntimeState { config, state: initial_state, path: Some(path), last_mtime: mtime });
    }
    core_host::host().hub.update(|_| Changes::DECK);

    core_host::spawn(async move {
        let sampled = tokio::task::spawn_blocking(sample_live_state).await.unwrap_or(DeckState {
            volume: 50,
            muted: false,
            mic_muted: Some(false),
            playing: false,
            output_devices: Vec::new(),
        });
        {
            let mut guard = lock(&DECK);
            if let Some(rt) = guard.as_mut() {
                rt.state = sampled.clone();
            }
        }
        core_host::host().hub.update(|_| Changes::DECK);
        if let Some(node) = core_host::wait_for_node().await {
            let _ = node.set_deck_layout(wire_layout).await;
            let _ = node.set_deck_state(sampled).await;
        }
        // Windows reports audio changes as they happen; the poll only
        // picks up edits to deck.json while the Deck page is open.
        win::audio::watch(|| core_host::spawn(refresh_live_state()));
        let mut interval = tokio::time::interval(Duration::from_millis(1500));
        loop {
            interval.tick().await;
            if !PAGE_ACTIVE.load(Ordering::Relaxed) {
                continue;
            }
            reload_from_disk_if_modified();
            refresh_live_state().await;
        }
    });
}

fn reload_from_disk_if_modified() {
    let updated_wire = {
        let mut guard = lock(&DECK);
        let Some(rt) = guard.as_mut() else { return };
        let Some(path) = rt.path.as_ref() else { return };
        let mtime = file_mtime(path);
        if mtime.is_none() || mtime == rt.last_mtime {
            return;
        }
        rt.last_mtime = mtime;
        let fresh = DeckConfig::load(path);
        if fresh == rt.config {
            return;
        }
        rt.config = fresh;
        Some(rt.config.to_wire_layout())
    };
    if let Some(wire) = updated_wire {
        core_host::host().hub.update(|_| Changes::DECK);
        core_host::spawn(async move {
            if let Some(node) = core_host::node() {
                let _ = node.set_deck_layout(wire).await;
            }
        });
    }
}

/// Re-reads the PC's speaker and mic states and broadcasts `deck.state` if changed.
pub async fn refresh_live_state() {
    let Ok(next) = tokio::task::spawn_blocking(sample_live_state).await else {
        return;
    };
    let changed = {
        let mut guard = lock(&DECK);
        match guard.as_mut() {
            Some(rt) if rt.state != next => {
                rt.state = next.clone();
                true
            }
            _ => false,
        }
    };
    if changed {
        core_host::host().hub.update(|_| Changes::DECK);
        if let Some(node) = core_host::node() {
            let _ = node.set_deck_state(next).await;
        }
    }
}

/// Called by `crate::media::local_changed` when the PC's SMTC media sessions change.
pub fn on_local_media_changed(players: &[nectarlink_core::MediaPlayer]) {
    HAS_MEDIA_SESSION.store(!players.is_empty(), Ordering::Relaxed);
    let playing = players.iter().any(|p| p.playing);
    MEDIA_PLAYING.store(playing, Ordering::Relaxed);
    core_host::spawn(async {
        refresh_live_state().await;
    });
}

/// Returns the current [`DeckConfig`].
pub fn config() -> DeckConfig {
    lock(&DECK).as_ref().map(|rt| rt.config.clone()).unwrap_or_else(DeckConfig::default_deck)
}

/// Returns the current [`DeckState`].
pub fn state() -> DeckState {
    lock(&DECK).as_ref().map(|rt| rt.state.clone()).unwrap_or_else(sample_live_state)
}

fn commit_config(mutate: impl FnOnce(&mut DeckConfig)) {
    let wire = {
        let mut guard = lock(&DECK);
        let rt = guard.get_or_insert_with(|| RuntimeState {
            config: DeckConfig::default_deck(),
            state: sample_live_state(),
            path: Some(core_host::host().data_dir.join(FILE_NAME)),
            last_mtime: None,
        });
        mutate(&mut rt.config);
        if let Some(clean) = rt.config.clone().sanitized() {
            rt.config = clean;
        }
        if let Some(path) = &rt.path {
            if let Err(e) = rt.config.save(path) {
                tracing::warn!(error = %e, "failed to save deck.json");
            }
            rt.last_mtime = file_mtime(path);
        }
        rt.config.to_wire_layout()
    };
    core_host::host().hub.update(|_| Changes::DECK);
    core_host::spawn(async move {
        if let Some(node) = core_host::node() {
            let _ = node.set_deck_layout(wire).await;
        }
    });
}

/// Called when a paired phone presses a Deck tile (`Platform::deck_press`).
pub fn handle_press(_peer: &DeviceId, tile_id: &str) -> Result<(), String> {
    execute_tile(tile_id)
}

/// Called when a paired phone changes the PC's speaker volume and/or mute state
/// (`Platform::set_pc_audio`).
pub fn handle_set_audio(_peer: &DeviceId, volume: Option<u8>, muted: Option<bool>) -> Result<(), String> {
    if let Some(level) = volume {
        win::audio::set_speaker_volume(level)?;
    }
    if let Some(m) = muted {
        win::audio::set_speaker_mute(m)?;
    }
    core_host::spawn(async {
        refresh_live_state().await;
    });
    Ok(())
}

/// Executes a tile's action by ID (used both for remote `deck.press` and the
/// desktop Deck editor's "Test" button).
pub fn execute_tile(tile_id: &str) -> Result<(), String> {
    reload_from_disk_if_modified();
    let action = {
        let guard = lock(&DECK);
        let rt = guard.as_ref().ok_or_else(|| "deck not initialized".to_owned())?;
        rt.config
            .tile(tile_id)
            .map(|t| t.action.clone())
            .ok_or_else(|| format!("unknown tile {tile_id:?}"))?
    };
    let res = execute_action(&action);
    core_host::spawn(async {
        refresh_live_state().await;
    });
    res
}

fn execute_action(action: &DeckAction) -> Result<(), String> {
    match action {
        DeckAction::MediaPlayPause => {
            win::input::press_key("play_pause", &[]);
            if !HAS_MEDIA_SESSION.load(Ordering::Relaxed) {
                let prev = MEDIA_PLAYING.load(Ordering::Relaxed);
                MEDIA_PLAYING.store(!prev, Ordering::Relaxed);
            }
            Ok(())
        }
        DeckAction::MediaNext => {
            win::input::press_key("next_track", &[]);
            Ok(())
        }
        DeckAction::MediaPrevious => {
            win::input::press_key("prev_track", &[]);
            Ok(())
        }
        DeckAction::VolumeUp => win::audio::speaker_volume_up().map(|_| ()),
        DeckAction::VolumeDown => win::audio::speaker_volume_down().map(|_| ()),
        DeckAction::VolumeMute => win::audio::toggle_speaker_mute().map(|_| ()),
        DeckAction::MicMute => win::audio::toggle_mic_mute().map(|_| ()),
        DeckAction::LockPc => win::shell::lock(),
        DeckAction::ShowDesktop => {
            win::input::press_key("d", &[KeyMod::Win]);
            Ok(())
        }
        DeckAction::SwitchWindow => {
            win::input::press_key("tab", &[KeyMod::Alt]);
            Ok(())
        }
        DeckAction::Screenshot => {
            win::input::press_key("s", &[KeyMod::Win, KeyMod::Shift]);
            Ok(())
        }
        DeckAction::Shortcut { key, mods } => {
            win::input::press_key(key, mods);
            Ok(())
        }
        DeckAction::OpenUrl { url } => links::open_here(url),
        DeckAction::TypeText { text } => {
            win::input::type_text(text);
            Ok(())
        }
        DeckAction::LaunchApp { path } => win::shell::launch_app(Path::new(path)),
        DeckAction::RunCommand { command } => win::shell::run_command(command),
    }
}

fn next_unique_id(prefix: &str, existing: &[String]) -> String {
    for n in 1..=999 {
        let candidate = format!("{prefix}_{n}");
        if !existing.contains(&candidate) {
            return candidate;
        }
    }
    format!("{prefix}_x")
}

/// Adds a new page and returns its ID.
pub fn add_page(name: &str) -> String {
    let trimmed = name.trim();
    let page_name = if trimmed.is_empty() { "New Page" } else { trimmed };
    let mut created_id = String::new();
    commit_config(|cfg| {
        if cfg.pages.len() >= nectarlink_core::MAX_DECK_PAGES {
            return;
        }
        let ids: Vec<String> = cfg.pages.iter().map(|p| p.id.clone()).collect();
        let id = next_unique_id("page", &ids);
        created_id = id.clone();
        cfg.pages.push(DeckPageConfig { id, name: page_name.to_owned(), tiles: Vec::new() });
    });
    created_id
}

/// Renames an existing page.
pub fn rename_page(page_id: &str, name: &str) {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return;
    }
    commit_config(|cfg| {
        if let Some(page) = cfg.pages.iter_mut().find(|p| p.id == page_id) {
            page.name = trimmed.to_owned();
        }
    });
}

/// Removes a page (unless it is the last remaining page).
pub fn remove_page(page_id: &str) {
    commit_config(|cfg| {
        if cfg.pages.len() <= 1 {
            return;
        }
        cfg.pages.retain(|p| p.id != page_id);
    });
}

/// Resets the Deck to the default layout.
pub fn reset_default() {
    commit_config(|cfg| {
        *cfg = DeckConfig::default_deck();
    });
}

/// Moves a tile within `page_id` from `from_index` to `to_index`.
pub fn move_tile(page_id: &str, from_index: usize, to_index: usize) {
    commit_config(|cfg| {
        let Some(page) = cfg.pages.iter_mut().find(|p| p.id == page_id) else { return };
        if from_index >= page.tiles.len() || to_index >= page.tiles.len() || from_index == to_index {
            return;
        }
        let tile = page.tiles.remove(from_index);
        page.tiles.insert(to_index, tile);
    });
}

/// Removes a tile by ID from `page_id`.
pub fn remove_tile(page_id: &str, tile_id: &str) {
    commit_config(|cfg| {
        if let Some(page) = cfg.pages.iter_mut().find(|p| p.id == page_id) {
            page.tiles.retain(|t| t.id != tile_id);
        }
    });
}

/// Builds a [`DeckAction`] from editor fields.
pub fn build_action(
    kind: &str,
    param: &str,
    ctrl: bool,
    alt: bool,
    shift: bool,
    win_mod: bool,
) -> Result<DeckAction, String> {
    let p = param.trim();
    let action = match kind {
        deck_kinds::MEDIA_PLAY_PAUSE => DeckAction::MediaPlayPause,
        deck_kinds::MEDIA_NEXT => DeckAction::MediaNext,
        deck_kinds::MEDIA_PREVIOUS => DeckAction::MediaPrevious,
        deck_kinds::VOLUME_UP => DeckAction::VolumeUp,
        deck_kinds::VOLUME_DOWN => DeckAction::VolumeDown,
        deck_kinds::VOLUME_MUTE => DeckAction::VolumeMute,
        deck_kinds::MIC_MUTE => DeckAction::MicMute,
        deck_kinds::LOCK_PC => DeckAction::LockPc,
        deck_kinds::SHOW_DESKTOP => DeckAction::ShowDesktop,
        deck_kinds::SWITCH_WINDOW => DeckAction::SwitchWindow,
        deck_kinds::SCREENSHOT => DeckAction::Screenshot,
        deck_kinds::SHORTCUT => {
            let key = p.to_ascii_lowercase();
            if key.is_empty() {
                return Err("Choose a key for the shortcut.".into());
            }
            let mut mods = Vec::new();
            if ctrl {
                mods.push(KeyMod::Ctrl);
            }
            if alt {
                mods.push(KeyMod::Alt);
            }
            if shift {
                mods.push(KeyMod::Shift);
            }
            if win_mod {
                mods.push(KeyMod::Win);
            }
            DeckAction::Shortcut { key, mods }
        }
        deck_kinds::OPEN_URL => {
            if p.is_empty() {
                return Err("Enter an http:// or https:// web address.".into());
            }
            let url = if p.starts_with("http://") || p.starts_with("https://") {
                p.to_owned()
            } else {
                format!("https://{p}")
            };
            DeckAction::OpenUrl { url }
        }
        deck_kinds::TYPE_TEXT => {
            if param.is_empty() {
                return Err("Enter the text snippet to type.".into());
            }
            DeckAction::TypeText { text: param.to_owned() }
        }
        deck_kinds::LAUNCH_APP => {
            let unquoted = p.trim_matches('"');
            if unquoted.is_empty() {
                return Err("Choose an application (.exe or .lnk) on this PC.".into());
            }
            DeckAction::LaunchApp { path: unquoted.to_owned() }
        }
        deck_kinds::RUN_COMMAND => {
            if p.is_empty() {
                return Err("Enter a command to run on this PC.".into());
            }
            DeckAction::RunCommand { command: p.to_owned() }
        }
        _ => return Err(format!("Unknown action type {kind:?}.")),
    };
    if !action.is_valid() {
        return Err(match kind {
            deck_kinds::LAUNCH_APP => "Application path must end with .exe or .lnk.".into(),
            deck_kinds::OPEN_URL => "Web address must start with http:// or https://.".into(),
            deck_kinds::SHORTCUT => {
                "Use a single key (a-z, 0-9, f1-f12, enter, tab, space, escape, arrows, etc.).".into()
            }
            _ => "Invalid action configuration.".into(),
        });
    }
    Ok(action)
}

/// Adds or updates a tile on `page_id`. If `tile_id` is empty, creates a new tile.
pub fn save_tile(
    page_id: &str,
    tile_id: &str,
    label: &str,
    icon: &str,
    color: &str,
    action: DeckAction,
) -> Result<String, String> {
    let kind = action.kind();
    let final_label = if label.trim().is_empty() {
        deck_kinds::default_label(kind).to_owned()
    } else {
        label.trim().to_owned()
    };
    let final_icon = if deck_icons::ALL.contains(&icon) {
        icon.to_owned()
    } else {
        deck_kinds::default_icon(kind).to_owned()
    };
    let final_color = if deck_colors::ALL.contains(&color) {
        color.to_owned()
    } else {
        deck_kinds::default_color(kind).to_owned()
    };

    let mut saved_id = String::new();
    let mut err: Option<String> = None;
    commit_config(|cfg| {
        let all_ids: Vec<String> =
            cfg.pages.iter().flat_map(|p| p.tiles.iter().map(|t| t.id.clone())).collect();
        let Some(page) = cfg.pages.iter_mut().find(|p| p.id == page_id) else {
            err = Some("Page not found.".into());
            return;
        };
        if tile_id.is_empty() {
            if page.tiles.len() >= nectarlink_core::MAX_DECK_TILES_PER_PAGE {
                err = Some("This page already has the maximum number of tiles (24).".into());
                return;
            }
            let new_id = next_unique_id("tile", &all_ids);
            let tile = DeckTileConfig {
                id: new_id.clone(),
                label: final_label.clone(),
                icon: final_icon.clone(),
                color: final_color.clone(),
                action: action.clone(),
            };
            if let Some(clean) = tile.sanitized() {
                saved_id = clean.id.clone();
                page.tiles.push(clean);
            } else {
                err = Some("Invalid tile properties.".into());
            }
        } else if let Some(existing) = page.tiles.iter_mut().find(|t| t.id == tile_id) {
            let updated = DeckTileConfig {
                id: existing.id.clone(),
                label: final_label.clone(),
                icon: final_icon.clone(),
                color: final_color.clone(),
                action: action.clone(),
            };
            if let Some(clean) = updated.sanitized() {
                saved_id = clean.id.clone();
                *existing = clean;
            } else {
                err = Some("Invalid tile properties.".into());
            }
        } else {
            err = Some("Tile not found.".into());
        }
    });
    if let Some(e) = err { Err(e) } else { Ok(saved_id) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_and_validates_editor_actions() {
        let sc = build_action("shortcut", "K", true, false, true, false).unwrap();
        assert_eq!(sc, DeckAction::Shortcut { key: "k".into(), mods: vec![KeyMod::Ctrl, KeyMod::Shift] });
        assert!(build_action("shortcut", "not_a_key", true, false, false, false).is_err());

        let url = build_action("open_url", "example.com/docs", false, false, false, false).unwrap();
        assert_eq!(url, DeckAction::OpenUrl { url: "https://example.com/docs".into() });

        let app =
            build_action("launch_app", "\"C:\\Windows\\System32\\calc.exe\"", false, false, false, false)
                .unwrap();
        assert_eq!(app, DeckAction::LaunchApp { path: "C:\\Windows\\System32\\calc.exe".into() });
        assert!(build_action("launch_app", "C:\\script.bat", false, false, false, false).is_err());

        let cmd = build_action("run_command", "  echo hello  ", false, false, false, false).unwrap();
        assert_eq!(cmd, DeckAction::RunCommand { command: "echo hello".into() });
    }

    #[test]
    fn unique_ids_skip_existing_ones() {
        let existing = vec!["tile_1".to_owned(), "tile_2".to_owned()];
        assert_eq!(next_unique_id("tile", &existing), "tile_3");
    }
}
