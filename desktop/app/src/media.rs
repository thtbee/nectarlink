// SPDX-License-Identifier: GPL-3.0-or-later
//! Media between this PC and paired phones (docs/protocol/media.md): what
//! plays on a phone shows in the app and in Windows' media flyout, where
//! it can be controlled; what plays on this PC goes to the phones (see
//! `win::media_sessions`).

use crate::i18n::tr;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Mutex, MutexGuard, OnceLock},
};

use nectarlink_core::{DeviceId, Error, MediaAction, NodeEvent};

use crate::{
    bridge::app::{describe, show_message},
    core_host,
    state::PlayerView,
    win::smtc,
};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Artwork files by (device, art key), for players that only send a key.
static ART: Mutex<Option<HashMap<(DeviceId, String), PathBuf>>> = Mutex::new(None);
/// Extracted Material 3 color scheme JSON by artwork file path.
static ART_SCHEMES: Mutex<Option<HashMap<PathBuf, String>>> = Mutex::new(None);
/// Artwork file for the currently active local PC media player (if any).
static LOCAL_ART: Mutex<Option<PathBuf>> = Mutex::new(None);
/// Artwork files kept per device (older ones are deleted).
const KEPT_ART: usize = 12;
/// This PC's own artwork kept on disk (the `pc-` files: what its players show,
/// by player, track and album). Every new track adds up to three, and nothing
/// else ever removed them.
const KEPT_LOCAL_ART: usize = 90;
/// The folder is tidied on the first change and then every this many.
const LOCAL_ART_TIDY_EVERY: usize = 24;

fn art_dir() -> PathBuf {
    core_host::host().data_dir.join("cache").join("art")
}

/// Computes or returns the cached Material 3 scheme JSON (`{"seed":...,"light":...,"dark":...}`)
/// for an artwork image file on disk.
pub fn scheme_for_path(path: &std::path::Path) -> String {
    if let Some(existing) = lock(&ART_SCHEMES).as_ref().and_then(|m| m.get(path)).cloned() {
        return existing;
    }
    let json = crate::win::wallpaper::artwork_scheme_from_path(path).unwrap_or_default();
    let mut cache = lock(&ART_SCHEMES);
    let map = cache.get_or_insert_default();
    if map.len() > 64 {
        map.clear();
    }
    map.insert(path.to_path_buf(), json.clone());
    json
}

fn cached_or_async_scheme(path: &std::path::Path) -> String {
    /// Artwork whose scheme is being worked out: each media refresh asks again until it's
    /// ready, and one worker per file is enough.
    static PENDING: Mutex<Option<std::collections::HashSet<PathBuf>>> = Mutex::new(None);
    if let Some(existing) = lock(&ART_SCHEMES).as_ref().and_then(|m| m.get(path)).cloned() {
        return existing;
    }
    if !lock(&PENDING).get_or_insert_default().insert(path.to_path_buf()) {
        return String::new();
    }
    let path_owned = path.to_path_buf();
    std::thread::spawn(move || {
        let _ = scheme_for_path(&path_owned);
        if let Some(pending) = lock(&PENDING).as_mut() {
            pending.remove(&path_owned);
        }
        core_host::host().hub.changed(crate::state::Changes::MEDIA);
    });
    String::new()
}

/// Returns the active media artwork path and its Material 3 color scheme JSON,
/// prioritizing:
/// 1. Any currently playing phone player with artwork
/// 2. Any currently playing local PC player with artwork
/// 3. Any paused phone player with artwork
/// 4. Any paused local PC player with artwork
pub fn active_artwork() -> Option<(PathBuf, String)> {
    let phone_playing = core_host::host()
        .hub
        .read(|s| s.media.iter().find(|p| p.player.playing && p.art.is_some()).and_then(|p| p.art.clone()));
    if let Some(path) = phone_playing {
        let colors = cached_or_async_scheme(&path);
        return Some((path, colors));
    }
    let local_playing = lock(&LOCAL_PLAYERS).iter().any(|(p, ..)| p.playing);
    if local_playing && let Some(path) = lock(&LOCAL_ART).clone() {
        let colors = cached_or_async_scheme(&path);
        return Some((path, colors));
    }
    let phone_any = core_host::host().hub.read(|s| s.media.iter().find_map(|p| p.art.clone()));
    if let Some(path) = phone_any {
        let colors = cached_or_async_scheme(&path);
        return Some((path, colors));
    }
    if let Some(path) = lock(&LOCAL_ART).clone() {
        let colors = cached_or_async_scheme(&path);
        return Some((path, colors));
    }
    None
}

/// Saves the artwork in a media event; returns every player's artwork file
/// by player ID.
pub fn save_art(event: &NodeEvent) -> HashMap<String, PathBuf> {
    let NodeEvent::MediaChanged { device, players } = event else { return HashMap::new() };
    let mut guard = lock(&ART);
    let art = guard.get_or_insert_default();
    let mut found = HashMap::new();
    for player in players {
        let Some(key) = &player.art_key else { continue };
        let slot = (*device, key.clone());
        if let Some(bytes) = &player.art {
            let ext = if bytes.starts_with(b"\x89PNG") { "png" } else { "jpg" };
            // Keys are the sender's; the file name is ours.
            let name = format!("{}-{:016x}.{ext}", device.short(), fnv(key));
            let path = art_dir().join(name);
            let written = std::fs::create_dir_all(art_dir()).and_then(|()| std::fs::write(&path, bytes));
            match written {
                Ok(()) => {
                    art.insert(slot.clone(), path);
                }
                Err(e) => tracing::warn!(error = %e, "can't save artwork"),
            }
        }
        if let Some(path) = art.get(&slot) {
            found.insert(player.id.clone(), path.clone());
        }
    }
    prune(art, *device, players.iter().filter_map(|p| p.art_key.clone()).collect());
    drop(guard);
    for path in found.values() {
        let _ = scheme_for_path(path);
    }
    found
}

/// Deletes a device's artwork beyond the newest few, never what's in use.
fn prune(art: &mut HashMap<(DeviceId, String), PathBuf>, device: DeviceId, in_use: Vec<String>) {
    let mut theirs: Vec<(String, PathBuf)> =
        art.iter().filter(|((d, _), _)| *d == device).map(|((_, k), p)| (k.clone(), p.clone())).collect();
    if theirs.len() <= KEPT_ART {
        return;
    }
    theirs.sort_by_key(|(_, path)| std::fs::metadata(path).and_then(|m| m.modified()).ok());
    let excess = theirs.len() - KEPT_ART;
    for (key, path) in theirs.into_iter().filter(|(k, _)| !in_use.contains(k)).take(excess) {
        let _ = std::fs::remove_file(&path);
        art.remove(&(device, key));
    }
}

/// Deletes this PC's own artwork (`pc-` files) in `dir` beyond the newest `keep`,
/// never what's `in_use`. Returns how many went. A file that's wanted again is
/// simply read from the player and written again.
fn prune_local_art(dir: &std::path::Path, keep: usize, in_use: &[PathBuf]) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    let mut ours: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("pc-"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    if ours.len() <= keep {
        return 0;
    }
    ours.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    ours.into_iter()
        .skip(keep)
        .filter(|(_, path)| !in_use.contains(path))
        .filter(|(_, path)| std::fs::remove_file(path).is_ok())
        .count()
}

fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// Keeps Windows' media flyout showing what plays on the phones.
pub fn update_flyout() {
    let current = core_host::host().hub.read(|s| {
        // A player that's playing wins; then the first one shown.
        let pick = s.media.iter().find(|p| p.player.playing).or_else(|| s.media.first())?;
        let device = s.name_of(&pick.device).unwrap_or_default();
        Some(smtc::Shown { device: pick.device, device_name: device, view: pick.clone() })
    });
    smtc::show(current);
}

/// What the flyout or the keyboard's media keys asked for.
pub fn on_flyout(device: DeviceId, player: String, action: MediaAction, position: Option<u64>) {
    send(device, player, action, position, false);
}

/// Sentinel DeviceId representing this PC's own local media players in QML `MediaList`.
pub const LOCAL_DEVICE: DeviceId = DeviceId([0; 32]);

/// Runs a command on a phone's player (or this PC's local player if `device == LOCAL_DEVICE`); says so if it didn't work.
pub fn command(device: DeviceId, player: String, action: MediaAction, position: Option<u64>) {
    if device == LOCAL_DEVICE {
        std::thread::spawn(move || {
            let _ = crate::win::media_sessions::command(&player, action, position);
        });
        return;
    }
    send(device, player, action, position, true);
}

fn send(device: DeviceId, player: String, action: MediaAction, position: Option<u64>, tell: bool) {
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        let result = node.media_command(device, player, action, position).await;
        let message = match result {
            Ok(()) => return,
            Err(Error::NotFound) => tr!("Media", "That player isn't playing anymore."),
            Err(Error::Unsupported) => tr!("Media", "That player can't do that."),
            Err(Error::Denied) => tr!("Media", "Media is turned off for that device."),
            Err(e) => describe(&e),
        };
        tracing::debug!(message, "media command failed");
        if tell {
            show_message(message);
        }
    });
}

/// The hub's view of a player, with when its position was measured.
pub fn view(device: DeviceId, mut player: nectarlink_core::MediaPlayer, art: Option<PathBuf>) -> PlayerView {
    player.art = None;
    PlayerView { device, player, art, at: std::time::SystemTime::now() }
}

/// Folds a media event into the hub (artwork saved first) and updates the
/// flyout.
pub fn on_event(event: &NodeEvent) {
    let changed = match event {
        NodeEvent::MediaChanged { device, players } => {
            let art = save_art(event);
            let views = players.iter().map(|p| view(*device, p.clone(), art.get(&p.id).cloned())).collect();
            core_host::host().hub.update(|s| s.set_media(*device, views));
            true
        }
        // Offline or unpaired devices' players are cleared by the hub.
        NodeEvent::LinkChanged { .. } | NodeEvent::DeviceRemoved(_) => true,
        _ => false,
    };
    if changed {
        update_flyout();
    }
}

/// Latest local media players on this PC and when they were sampled.
static LOCAL_PLAYERS: Mutex<Vec<(nectarlink_core::MediaPlayer, std::time::SystemTime, std::time::Instant)>> =
    Mutex::new(Vec::new());
/// Saved artwork paths by player ID for local PC media sessions.
static LOCAL_PLAYER_ART: Mutex<Option<HashMap<String, PathBuf>>> = Mutex::new(None);

/// Returns `PlayerView`s for all active local media sessions on this PC so
/// the Desktop UI can display and control PC media alongside phone media.
pub fn local_views() -> Vec<PlayerView> {
    let guard = lock(&LOCAL_PLAYERS);
    let art_guard = lock(&LOCAL_PLAYER_ART);
    guard
        .iter()
        .map(|(p, sampled_sys, _)| {
            let mut player = p.clone();
            let art = art_guard.as_ref().and_then(|m| m.get(&player.id)).cloned();
            player.art = None;
            PlayerView { device: LOCAL_DEVICE, player, art, at: *sampled_sys }
        })
        .collect()
}

/// Returns the current playback position in seconds of the active local media
/// player on this PC (if any player has a non-zero position).
pub fn local_playback_position_secs() -> Option<u32> {
    let guard = lock(&LOCAL_PLAYERS);
    let (player, _, sampled_at) = guard
        .iter()
        .find(|(p, _, _)| p.playing && p.position.is_some_and(|pos| pos > 0))
        .or_else(|| guard.iter().find(|(p, _, _)| p.position.is_some_and(|pos| pos > 0)))?;
    let base_ms = player.position?;
    let extra_ms = if player.playing { sampled_at.elapsed().as_millis() as u64 } else { 0 };
    let total_ms = match player.duration {
        Some(dur) if dur > 0 => base_ms.saturating_add(extra_ms).min(dur),
        _ => base_ms.saturating_add(extra_ms),
    };
    let secs = (total_ms / 1000) as u32;
    (secs > 0).then_some(secs)
}

/// What plays on this PC changed: tell the phones and refresh Desktop UI.
pub fn local_changed(players: Vec<nectarlink_core::MediaPlayer>) {
    let now_sys = std::time::SystemTime::now();
    let now = std::time::Instant::now();
    *lock(&LOCAL_PLAYERS) = players.iter().cloned().map(|p| (p, now_sys, now)).collect();
    let mut per_player_art = HashMap::new();
    for p in &players {
        if let Some(bytes) = p.art.as_ref() {
            let key = p.art_key.as_deref().unwrap_or(&p.id);
            let ext = if bytes.starts_with(b"\x89PNG") { "png" } else { "jpg" };
            let path = art_dir().join(format!("pc-{:016x}.{ext}", fnv(key)));
            if !path.exists() {
                let _ = std::fs::create_dir_all(art_dir()).and_then(|()| std::fs::write(&path, bytes));
            }
            let _ = scheme_for_path(&path);
            per_player_art.insert(p.id.clone(), path);
        }
    }
    let chosen_art = players
        .iter()
        .find(|p| p.playing && per_player_art.contains_key(&p.id))
        .or_else(|| players.iter().find(|p| per_player_art.contains_key(&p.id)))
        .and_then(|p| per_player_art.get(&p.id).cloned());
    static CHANGES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    if CHANGES.fetch_add(1, std::sync::atomic::Ordering::Relaxed).is_multiple_of(LOCAL_ART_TIDY_EVERY) {
        let in_use: Vec<PathBuf> = per_player_art.values().cloned().collect();
        let dir = art_dir();
        std::thread::spawn(move || {
            let removed = prune_local_art(&dir, KEPT_LOCAL_ART, &in_use);
            if removed > 0 {
                tracing::debug!(removed, "old artwork of this PC's players removed");
            }
        });
    }
    *lock(&LOCAL_PLAYER_ART) = Some(per_player_art);
    *lock(&LOCAL_ART) = chosen_art;
    core_host::host().hub.changed(crate::state::Changes::MEDIA);
    crate::deck::on_local_media_changed(&players);
    static UPDATES: OnceLock<tokio::sync::watch::Sender<Vec<nectarlink_core::MediaPlayer>>> = OnceLock::new();
    let updates = UPDATES.get_or_init(|| {
        let (tx, mut rx) = tokio::sync::watch::channel(Vec::new());
        core_host::spawn(async move {
            let Some(node) = core_host::wait_for_node().await else { return };
            while rx.changed().await.is_ok() {
                let players = rx.borrow_and_update().clone();
                node.media_changed(players).await;
            }
        });
        tx
    });
    updates.send_replace(players);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_pcs_old_artwork_goes_and_the_rest_stays() {
        let dir = tempfile::tempdir().unwrap();
        let file = |name: &str, age_secs: u64| {
            let path = dir.path().join(name);
            let f = std::fs::File::create(&path).unwrap();
            f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(age_secs)).unwrap();
            path
        };
        let oldest = file("pc-track-01.img", 500);
        let in_use = file("pc-02.png", 400);
        let newer = file("pc-album-03.img", 300);
        let newest = file("pc-track-04.img", 200);
        let phones = file("386j39qo-05.jpg", 900);

        let removed = prune_local_art(dir.path(), 2, std::slice::from_ref(&in_use));
        assert_eq!(removed, 1);
        assert!(!oldest.exists(), "the oldest of this PC's goes");
        assert!(in_use.exists(), "what a player is showing stays");
        assert!(newer.exists() && newest.exists());
        assert!(phones.exists(), "a phone's artwork isn't this one's to remove");
        assert_eq!(prune_local_art(dir.path(), 10, &[]), 0);
    }
}
