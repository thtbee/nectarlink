// SPDX-License-Identifier: GPL-3.0-or-later
//! Media between this PC and paired phones (docs/protocol/media.md): what
//! plays on a phone shows in the app and in Windows' media flyout, where
//! it can be controlled; what plays on this PC goes to the phones (see
//! `win::media_sessions`).

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Mutex, MutexGuard},
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
/// Artwork files kept per device (older ones are deleted).
const KEPT_ART: usize = 12;

fn art_dir() -> PathBuf {
    core_host::host().data_dir.join("cache").join("art")
}

/// Saves the artwork in a media event; returns every player's artwork file
/// by player ID.
pub fn save_art(event: &NodeEvent) -> HashMap<String, PathBuf> {
    let NodeEvent::MediaChanged { device, players } = event else { return HashMap::new() };
    let mut art = lock(&ART);
    let art = art.get_or_insert_default();
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

/// Runs a command on a phone's player; says so if it didn't work.
pub fn command(device: DeviceId, player: String, action: MediaAction, position: Option<u64>) {
    send(device, player, action, position, true);
}

fn send(device: DeviceId, player: String, action: MediaAction, position: Option<u64>, tell: bool) {
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        let result = node.media_command(device, player, action, position).await;
        let message = match result {
            Ok(()) => return,
            Err(Error::NotFound) => "That player isn't playing anymore.".to_owned(),
            Err(Error::Unsupported) => "That player can't do that.".to_owned(),
            Err(Error::Denied) => "Media is turned off for that device.".to_owned(),
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

/// What plays on this PC changed: tell the phones.
pub fn local_changed(players: Vec<nectarlink_core::MediaPlayer>) {
    crate::deck::on_local_media_changed(&players);
    core_host::spawn(async move {
        if let Some(node) = core_host::wait_for_node().await {
            node.media_changed(players).await;
        }
    });
}
