// SPDX-License-Identifier: GPL-3.0-or-later
//! What plays on this PC, for paired phones (docs/protocol/media.md): the
//! sessions apps report to Windows (Global System Media Transport
//! Controls), and commands from a phone sent to them.
//!
//! Nectarlink's own session (a phone's media in the flyout) is left out, so
//! a phone never gets its own music back.

use std::{
    collections::HashMap,
    sync::{OnceLock, mpsc},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use nectarlink_core::{MediaAction, MediaError, MediaPlayer};
use windows::{
    ApplicationModel::AppInfo,
    Foundation::TypedEventHandler,
    Media::Control::{
        GlobalSystemMediaTransportControlsSession, GlobalSystemMediaTransportControlsSessionManager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus,
    },
    Storage::Streams::DataReader,
    Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx},
    core::HSTRING,
};

use super::toast::AUMID;

/// The largest artwork sent as is; larger pictures are scaled down.
const ART_BYTES: u64 = 256 * 1024;
/// Changes come in bursts (title, then artwork, then position).
const SETTLE: Duration = Duration::from_millis(300);
/// Windows' clock starts in 1601; Unix time in 1970.
const WINDOWS_TO_UNIX_SECS: i64 = 11_644_473_600;

type OnChange = Box<dyn Fn(Vec<MediaPlayer>) + Send + Sync>;

static MANAGER: OnceLock<GlobalSystemMediaTransportControlsSessionManager> = OnceLock::new();

/// Watches this PC's media; `on_change` gets every player after each
/// change (on the watcher's thread).
pub fn start(on_change: impl Fn(Vec<MediaPlayer>) + Send + Sync + 'static) {
    let on_change: OnChange = Box::new(on_change);
    let spawned = std::thread::Builder::new().name("media-sessions".into()).spawn(move || {
        if let Err(e) = run(&on_change) {
            tracing::warn!(error = %e, "can't watch this PC's media");
        }
    });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "can't watch this PC's media");
    }
}

fn run(on_change: &OnChange) -> windows::core::Result<()> {
    // SAFETY: COM for this thread, which lives as long as the app.
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok()?;
    let manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?.join()?;
    let (changed, changes) = mpsc::channel::<()>();
    let signal = changed.clone();
    manager.SessionsChanged(&TypedEventHandler::new(move |_, _| {
        let _ = signal.send(());
        Ok(())
    }))?;
    let _ = MANAGER.set(manager.clone());

    let mut watched: Vec<String> = Vec::new();
    let mut art = ArtCache::default();
    let mut last: Option<Vec<MediaPlayer>> = None;
    loop {
        // Hook up sessions that are new since last time.
        let mut present = Vec::new();
        for session in manager.GetSessions()? {
            let id = session.SourceAppUserModelId()?.to_string();
            if !watched.contains(&id) {
                watch(&session, &changed)?;
                watched.push(id.clone());
            }
            present.push(id);
        }
        // A session that ends and comes back is hooked up again.
        watched.retain(|id| present.contains(id));
        let players = snapshot(&manager, &mut art);
        if last.as_ref() != Some(&players) {
            on_change(players.clone());
            last = Some(players);
        }
        // Wait for a change, then let the burst settle.
        if changes.recv().is_err() {
            return Ok(());
        }
        while changes.recv_timeout(SETTLE).is_ok() {}
    }
}

fn watch(
    session: &GlobalSystemMediaTransportControlsSession,
    changed: &mpsc::Sender<()>,
) -> windows::core::Result<()> {
    session.MediaPropertiesChanged(&TypedEventHandler::new({
        let changed = changed.clone();
        move |_, _| {
            let _ = changed.send(());
            Ok(())
        }
    }))?;
    session.PlaybackInfoChanged(&TypedEventHandler::new({
        let changed = changed.clone();
        move |_, _| {
            let _ = changed.send(());
            Ok(())
        }
    }))?;
    session.TimelinePropertiesChanged(&TypedEventHandler::new({
        let changed = changed.clone();
        move |_, _| {
            let _ = changed.send(());
            Ok(())
        }
    }))?;
    Ok(())
}

/// Every session but Nectarlink's own, the one Windows considers current
/// first.
fn snapshot(
    manager: &GlobalSystemMediaTransportControlsSessionManager,
    art: &mut ArtCache,
) -> Vec<MediaPlayer> {
    let current =
        manager.GetCurrentSession().ok().and_then(|s| s.SourceAppUserModelId().ok()).map(|id| id.to_string());
    let Ok(sessions) = manager.GetSessions() else { return Vec::new() };
    let mut players: Vec<MediaPlayer> = sessions
        .into_iter()
        .filter_map(|session| match player(&session, art) {
            Ok(player) => player,
            Err(e) => {
                tracing::debug!(error = %e, "can't read a media session");
                None
            }
        })
        .collect();
    players.sort_by_key(|p| (Some(&p.id) != current.as_ref(), !p.playing));
    players
}

fn player(
    session: &GlobalSystemMediaTransportControlsSession,
    art: &mut ArtCache,
) -> windows::core::Result<Option<MediaPlayer>> {
    let id = session.SourceAppUserModelId()?.to_string();
    if id == AUMID {
        return Ok(None);
    }
    let properties = session.TryGetMediaPropertiesAsync()?.join()?;
    let text =
        |s: windows::core::Result<HSTRING>| s.ok().map(|s| s.to_string()).filter(|s| !s.trim().is_empty());
    let title = text(properties.Title());
    let artist = text(properties.Artist());
    let album = text(properties.AlbumTitle());
    if title.is_none() {
        return Ok(None);
    }
    let info = session.GetPlaybackInfo()?;
    let playing = info.PlaybackStatus()? == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing;
    let controls = info.Controls()?;
    let mut actions = Vec::new();
    for (on, action) in [
        (controls.IsPlayEnabled()?, "play"),
        (controls.IsPauseEnabled()?, "pause"),
        (controls.IsNextEnabled()?, "next"),
        (controls.IsPreviousEnabled()?, "previous"),
        (controls.IsPlaybackPositionEnabled()?, "seek"),
    ] {
        if on {
            actions.push(action.to_owned());
        }
    }

    let timeline = session.GetTimelineProperties()?;
    let ms = |ticks: i64| u64::try_from(ticks / 10_000).unwrap_or(0);
    let duration = ms(timeline.EndTime()?.Duration - timeline.StartTime()?.Duration);
    let (duration, position) = if duration > 0 {
        let mut position = ms(timeline.Position()?.Duration);
        // The position is as of the last update; it has moved on since.
        if playing {
            let updated = timeline.LastUpdatedTime()?.UniversalTime;
            let updated_ms = updated / 10_000 - WINDOWS_TO_UNIX_SECS * 1000;
            let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
            position = position.saturating_add(u64::try_from(now_ms - updated_ms).unwrap_or(0));
        }
        (Some(duration), Some(position.min(duration)))
    } else {
        (None, None)
    };

    let (art_key, art_bytes) = art.get(&id, &title, &artist, &album, || thumbnail(&properties)).unzip();
    Ok(Some(MediaPlayer {
        app: app_name(&id),
        id,
        title,
        artist,
        album,
        playing,
        duration,
        position,
        actions,
        art_key,
        art: art_bytes,
    }))
}

/// Artwork per track, read once: reading it takes a moment.
#[derive(Default)]
struct ArtCache {
    by_track: HashMap<String, Option<(String, Vec<u8>)>>,
}

impl ArtCache {
    fn get(
        &mut self,
        id: &str,
        title: &Option<String>,
        artist: &Option<String>,
        album: &Option<String>,
        read: impl FnOnce() -> Option<Vec<u8>>,
    ) -> Option<(String, Vec<u8>)> {
        let track = format!("{id}\u{1}{title:?}\u{1}{artist:?}\u{1}{album:?}");
        if !self.by_track.contains_key(&track) {
            if self.by_track.len() > 32 {
                self.by_track.clear();
            }
            let art = read().map(|bytes| (format!("{:016x}", fnv(&bytes)), bytes));
            self.by_track.insert(track.clone(), art);
        }
        self.by_track.get(&track).cloned().flatten()
    }
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
}

fn thumbnail(
    properties: &windows::Media::Control::GlobalSystemMediaTransportControlsSessionMediaProperties,
) -> Option<Vec<u8>> {
    let read = || -> windows::core::Result<Vec<u8>> {
        let stream = properties.Thumbnail()?.OpenReadAsync()?.join()?;
        let size = stream.Size()?;
        let reader = DataReader::CreateDataReader(&stream)?;
        let loaded = reader.LoadAsync(u32::try_from(size).unwrap_or(u32::MAX))?.join()?;
        let mut bytes = vec![0u8; loaded as usize];
        reader.ReadBytes(&mut bytes)?;
        Ok(bytes)
    };
    let bytes = read().ok().filter(|b| !b.is_empty())?;
    if bytes.len() as u64 <= ART_BYTES {
        return Some(bytes);
    }
    // Too large to send: a smaller PNG.
    let bitmap = super::image::decode(&bytes).ok()?;
    let small = super::image::encode_png(&super::image::scale_to(&bitmap, 360)).ok()?;
    (small.len() as u64 <= ART_BYTES).then_some(small)
}

/// The app's name as Windows shows it, or a readable form of its ID.
fn app_name(id: &str) -> String {
    let display =
        AppInfo::GetFromAppUserModelId(&HSTRING::from(id)).and_then(|info| info.DisplayInfo()?.DisplayName());
    if let Ok(name) = display {
        let name = name.to_string();
        if !name.trim().is_empty() {
            return name;
        }
    }
    readable_name(id)
}

/// "Spotify.exe" → "Spotify", `C:\…\vlc.exe` → "vlc", "Microsoft.ZuneMusic_8wekyb3d8bbwe!App" → "Microsoft.ZuneMusic".
fn readable_name(id: &str) -> String {
    let id = id.rsplit(['\\', '/']).next().unwrap_or(id);
    let id = id.split('!').next().unwrap_or(id);
    let id = id.split('_').next().unwrap_or(id);
    let id = id.strip_suffix(".exe").or_else(|| id.strip_suffix(".EXE")).unwrap_or(id);
    if id.is_empty() { "Media".into() } else { id.to_owned() }
}

/// Runs a phone's command on one of this PC's sessions.
pub fn command(player: &str, action: MediaAction, position: Option<u64>) -> Result<(), MediaError> {
    let manager = MANAGER.get().ok_or(MediaError::Unsupported)?;
    let failed = |e: windows::core::Error| MediaError::Failed(e.to_string());
    let session = manager
        .GetSessions()
        .map_err(failed)?
        .into_iter()
        .find(|s| s.SourceAppUserModelId().is_ok_and(|id| id == player))
        .ok_or(MediaError::NotFound)?;
    let done = match action {
        MediaAction::Play => session.TryPlayAsync(),
        MediaAction::Pause => session.TryPauseAsync(),
        MediaAction::Next => session.TrySkipNextAsync(),
        MediaAction::Previous => session.TrySkipPreviousAsync(),
        MediaAction::Seek => {
            let ticks = i64::try_from(position.unwrap_or(0)).unwrap_or(0).saturating_mul(10_000);
            session.TryChangePlaybackPositionAsync(ticks)
        }
    }
    .and_then(|op| op.join())
    .map_err(failed)?;
    if done { Ok(()) } else { Err(MediaError::Unsupported) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_ids_read_as_names() {
        assert_eq!(readable_name("Spotify.exe"), "Spotify");
        assert_eq!(readable_name(r"C:\Program Files\VideoLAN\vlc.exe"), "vlc");
        assert_eq!(
            readable_name("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"),
            "Microsoft.ZuneMusic"
        );
        assert_eq!(readable_name("Chrome"), "Chrome");
        assert_eq!(readable_name(""), "Media");
    }

    #[test]
    fn artwork_is_read_once_per_track() {
        let mut cache = ArtCache::default();
        let mut reads = 0;
        let title = Some("Song".to_owned());
        for _ in 0..3 {
            let art = cache.get("app", &title, &None, &None, || {
                reads += 1;
                Some(vec![1, 2, 3])
            });
            assert_eq!(art.map(|(_, b)| b), Some(vec![1, 2, 3]));
        }
        assert_eq!(reads, 1);
        assert!(cache.get("app", &Some("Other".into()), &None, &None, || None).is_none());
    }
}
