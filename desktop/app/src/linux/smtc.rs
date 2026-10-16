// SPDX-License-Identifier: GPL-3.0-or-later
//! What plays on a phone, exported to Linux desktop media controls (MPRIS2
//! provider `org.mpris.MediaPlayer2.nectarlink` and atomic JSON state snapshot).
//!
//! Mirrors the public API of `win::smtc`: [`start`] registers the command
//! callback and spawns the `"media-controls"` worker thread; [`show`] updates
//! or clears the currently displayed phone media session.

use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock, mpsc};
use std::time::{SystemTime, UNIX_EPOCH};

use nectarlink_core::{DeviceId, MediaAction};
use serde::{Deserialize, Serialize};

use crate::state::PlayerView;

/// What the controls show: one player on one device.
#[derive(Debug, Clone)]
pub struct Shown {
    pub device: DeviceId,
    pub device_name: String,
    pub view: PlayerView,
}

type OnCommand = Box<dyn Fn(DeviceId, String, MediaAction, Option<u64>) + Send + Sync>;

static ON_COMMAND: OnceLock<Mutex<Option<OnCommand>>> = OnceLock::new();
static QUEUE: OnceLock<Mutex<mpsc::Sender<Option<Shown>>>> = OnceLock::new();
static CURRENT: Mutex<Option<(DeviceId, String)>> = Mutex::new(None);
static LAST_SHOWN: Mutex<Option<Shown>> = Mutex::new(None);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MprisExportState {
    pub bus_name: String,
    pub enabled: bool,
    pub device_id: Option<String>,
    pub device_name: Option<String>,
    pub player_id: Option<String>,
    pub playback_status: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub art_url: Option<String>,
    pub length_us: Option<u64>,
    pub position_us: Option<u64>,
    pub can_play: bool,
    pub can_pause: bool,
    pub can_go_next: bool,
    pub can_go_previous: bool,
    pub can_seek: bool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn on_command_slot() -> &'static Mutex<Option<OnCommand>> {
    ON_COMMAND.get_or_init(|| Mutex::new(None))
}

/// Formats the second line of the media flyout with source attribution matching `win::smtc`.
pub(crate) fn format_artist_line(artist: Option<&str>, app: &str, device_name: &str) -> String {
    let source = format!("{app} on {device_name}");
    match artist.map(str::trim).filter(|s| !s.is_empty()) {
        Some(artist) => format!("{artist} · {source}"),
        None => source,
    }
}

/// Starts the Linux media controls provider and spawns the `"media-controls"` worker thread.
pub fn start<F>(on_command: F)
where
    F: Fn(DeviceId, String, MediaAction, Option<u64>) + Send + Sync + 'static,
{
    *lock(on_command_slot()) = Some(Box::new(on_command));
    if QUEUE.get().is_some() {
        return;
    }
    let (sender, receiver) = mpsc::channel::<Option<Shown>>();
    if QUEUE.set(Mutex::new(sender)).is_ok() {
        let spawned =
            std::thread::Builder::new().name("media-controls".into()).spawn(move || run_worker(receiver));
        if let Err(e) = spawned {
            tracing::warn!(error = %e, "Linux media-controls worker unavailable");
        }
    }
}

fn run_worker(receiver: mpsc::Receiver<Option<Shown>>) {
    while let Ok(first) = receiver.recv() {
        let mut latest = first;
        while let Ok(next) = receiver.try_recv() {
            latest = next;
        }
        let export = build_export_state(latest.as_ref());
        write_mpris_state_file(&export);
    }
}

/// Shows a phone player in the desktop media controls, or hides them when `None`.
pub fn show(current: Option<Shown>) {
    match &current {
        Some(shown) => {
            *lock(&CURRENT) = Some((shown.device, shown.view.player.id.clone()));
        }
        None => {
            *lock(&CURRENT) = None;
        }
    }
    *lock(&LAST_SHOWN) = current.clone();
    if let Some(queue) = QUEUE.get() {
        let _ = lock(queue).send(current);
    } else {
        let export = build_export_state(current.as_ref());
        write_mpris_state_file(&export);
    }
}

pub(crate) fn build_export_state(shown: Option<&Shown>) -> MprisExportState {
    let Some(shown) = shown else {
        return MprisExportState {
            bus_name: "org.mpris.MediaPlayer2.nectarlink".to_owned(),
            enabled: false,
            device_id: None,
            device_name: None,
            player_id: None,
            playback_status: "Stopped".to_owned(),
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            art_url: None,
            length_us: None,
            position_us: None,
            can_play: false,
            can_pause: false,
            can_go_next: false,
            can_go_previous: false,
            can_seek: false,
        };
    };

    let player = &shown.view.player;
    let can = |action: &str| player.actions.iter().any(|a| a == action);
    let title = player.title.as_deref().unwrap_or(&player.app).to_owned();
    let artist = format_artist_line(player.artist.as_deref(), &player.app, &shown.device_name);
    let album = player.album.clone().unwrap_or_default();
    let art_url = shown.view.art.as_ref().map(|p| format!("file://{}", p.display()));

    let elapsed_ms = if player.playing {
        SystemTime::now().duration_since(shown.view.at).map_or(0, |d| d.as_millis() as u64)
    } else {
        0
    };
    let duration_ms = player.duration;
    let position_ms = player.position.map(|pos| {
        let advanced = pos.saturating_add(elapsed_ms);
        duration_ms.map_or(advanced, |d| advanced.min(d))
    });

    MprisExportState {
        bus_name: "org.mpris.MediaPlayer2.nectarlink".to_owned(),
        enabled: true,
        device_id: Some(shown.device.to_string()),
        device_name: Some(shown.device_name.clone()),
        player_id: Some(player.id.clone()),
        playback_status: if player.playing { "Playing".to_owned() } else { "Paused".to_owned() },
        title,
        artist,
        album,
        art_url,
        length_us: duration_ms.map(|ms| ms.saturating_mul(1000)),
        position_us: position_ms.map(|ms| ms.saturating_mul(1000)),
        can_play: can("play"),
        can_pause: can("pause"),
        can_go_next: can("next"),
        can_go_previous: can("previous"),
        can_seek: can("seek"),
    }
}

fn mpris_state_path() -> Option<PathBuf> {
    let base = dirs::runtime_dir().or_else(dirs::cache_dir).unwrap_or_else(std::env::temp_dir);
    Some(base.join("nectarlink").join("mpris-state.json"))
}

fn write_mpris_state_file(state: &MprisExportState) {
    let Some(path) = mpris_state_path() else {
        return;
    };
    let Some(parent) = path.parent() else {
        return;
    };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    let Ok(json) = serde_json::to_vec_pretty(state) else {
        return;
    };
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    let tmp_path = parent.join(format!(".mpris-state.{}.{}.tmp", std::process::id(), nanos));
    if fs::write(&tmp_path, &json).is_ok() {
        let _ = fs::rename(&tmp_path, &path);
    }
}

/// Dispatches a [`MediaAction`] for the currently shown phone media session.
pub fn dispatch_command(action: MediaAction, position_ms: Option<u64>) -> bool {
    let Some((device, player_id)) = lock(&CURRENT).clone() else {
        return false;
    };
    let guard = lock(on_command_slot());
    let Some(on_command) = guard.as_ref() else {
        return false;
    };
    on_command(device, player_id, action, position_ms);
    true
}

/// Handles an incoming MPRIS2 `org.mpris.MediaPlayer2.Player` method call.
pub fn handle_mpris_call(method: &str, position_us: Option<i64>) -> bool {
    match method.trim() {
        "Play" => dispatch_command(MediaAction::Play, None),
        "Pause" => dispatch_command(MediaAction::Pause, None),
        "PlayPause" => {
            let currently_playing = lock(&LAST_SHOWN).as_ref().is_some_and(|s| s.view.player.playing);
            let action = if currently_playing { MediaAction::Pause } else { MediaAction::Play };
            dispatch_command(action, None)
        }
        "Next" => dispatch_command(MediaAction::Next, None),
        "Previous" => dispatch_command(MediaAction::Previous, None),
        "Seek" | "SetPosition" => {
            let pos_ms = position_us.map(|us| u64::try_from((us / 1000).max(0)).unwrap_or(0));
            dispatch_command(MediaAction::Seek, pos_ms)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nectarlink_core::MediaPlayer;
    use std::sync::Arc;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn sample_shown(playing: bool) -> Shown {
        let device = DeviceId([7u8; 32]);
        Shown {
            device,
            device_name: "Pixel 9".to_owned(),
            view: PlayerView {
                device,
                player: MediaPlayer {
                    id: "com.spotify.music".to_owned(),
                    app: "Spotify".to_owned(),
                    title: Some("Veridis Quo".to_owned()),
                    artist: Some("Daft Punk".to_owned()),
                    album: Some("Discovery".to_owned()),
                    playing,
                    duration: Some(345_000),
                    position: Some(12_000),
                    actions: vec![
                        "play".to_owned(),
                        "pause".to_owned(),
                        "next".to_owned(),
                        "previous".to_owned(),
                        "seek".to_owned(),
                    ],
                    art_key: None,
                    art: None,
                },
                art: Some(PathBuf::from("/tmp/discovery.jpg")),
                at: SystemTime::now(),
            },
        }
    }

    #[test]
    fn formats_artist_line_with_source_attribution() {
        assert_eq!(
            format_artist_line(Some("Daft Punk"), "Spotify", "Pixel 9"),
            "Daft Punk · Spotify on Pixel 9"
        );
        assert_eq!(format_artist_line(None, "YouTube Music", "Galaxy S24"), "YouTube Music on Galaxy S24");
    }

    #[test]
    fn show_and_mpris_calls_dispatch_commands_and_clear_on_none() {
        type ReceivedMediaCmd = (DeviceId, String, MediaAction, Option<u64>);
        let _serial = lock(&TEST_LOCK);
        let received: Arc<Mutex<Vec<ReceivedMediaCmd>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&received);
        start(move |dev, player, action, pos| {
            lock(&sink).push((dev, player, action, pos));
        });

        let shown_playing = sample_shown(true);
        let expected_dev = shown_playing.device;
        show(Some(shown_playing));

        assert!(handle_mpris_call("Play", None));
        assert!(handle_mpris_call("Pause", None));
        // PlayPause while playing -> Pause
        assert!(handle_mpris_call("PlayPause", None));
        assert!(handle_mpris_call("Next", None));
        assert!(handle_mpris_call("Previous", None));
        assert!(handle_mpris_call("Seek", Some(42_500_000)));

        // Switch state to paused and test PlayPause -> Play
        show(Some(sample_shown(false)));
        assert!(handle_mpris_call("PlayPause", None));

        let events = lock(&received).clone();
        assert_eq!(events.len(), 7);
        assert_eq!(events[0], (expected_dev, "com.spotify.music".to_owned(), MediaAction::Play, None));
        assert_eq!(events[1], (expected_dev, "com.spotify.music".to_owned(), MediaAction::Pause, None));
        assert_eq!(events[2], (expected_dev, "com.spotify.music".to_owned(), MediaAction::Pause, None));
        assert_eq!(events[3], (expected_dev, "com.spotify.music".to_owned(), MediaAction::Next, None));
        assert_eq!(events[4], (expected_dev, "com.spotify.music".to_owned(), MediaAction::Previous, None));
        assert_eq!(
            events[5],
            (expected_dev, "com.spotify.music".to_owned(), MediaAction::Seek, Some(42_500))
        );
        assert_eq!(events[6], (expected_dev, "com.spotify.music".to_owned(), MediaAction::Play, None));

        // Clearing with show(None) stops dispatching commands
        show(None);
        assert!(!handle_mpris_call("Play", None));
        let cleared_state = build_export_state(None);
        assert!(!cleared_state.enabled);
        assert_eq!(cleared_state.playback_status, "Stopped");
    }
}
