// SPDX-License-Identifier: GPL-3.0-or-later
//! What plays on this PC, for paired phones (docs/protocol/media.md): the
//! sessions apps report to Windows (Global System Media Transport
//! Controls), and commands from a phone sent to them.
//!
//! NectarLink's own session (a phone's media in the flyout) is left out, so
//! a phone never gets its own music back.

use std::{
    collections::{HashMap, HashSet},
    sync::{Mutex, OnceLock, mpsc},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use nectarlink_core::{MediaAction, MediaError, MediaPlayer};
use windows::{
    ApplicationModel::AppInfo,
    Foundation::TypedEventHandler,
    Media::Control::{
        GlobalSystemMediaTransportControlsSession, GlobalSystemMediaTransportControlsSessionManager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus,
    },
    Media::MediaPlaybackType,
    Storage::Streams::DataReader,
    Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx},
    core::HSTRING,
};

use super::toast::AUMID;

/// The largest artwork sent as is; larger pictures are scaled down.
const ART_BYTES: u64 = 256 * 1024;
/// Short settle window so burst updates (title + artwork + timeline) coalesce without noticeable lag.
const SETTLE: Duration = Duration::from_millis(50);
const MAX_SETTLE: Duration = Duration::from_millis(150);
/// Windows' clock starts in 1601; Unix time in 1970.
const WINDOWS_TO_UNIX_SECS: i64 = 11_644_473_600;

type OnChange = Box<dyn Fn(Vec<MediaPlayer>) + Send + Sync>;

static MANAGER: OnceLock<GlobalSystemMediaTransportControlsSessionManager> = OnceLock::new();
/// Players that say they play a video (most browsers say "music" for
/// everything, so this is a hint, not the whole answer).
static VIDEO: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// Whether this PC's player `id` says it plays a video.
pub fn says_video(id: &str) -> bool {
    VIDEO.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|v| v.contains(id))
}

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
    let signal_current = changed.clone();
    let _ = manager.CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
        let _ = signal_current.send(());
        Ok(())
    }));
    let _ = MANAGER.set(manager.clone());

    let mut watched: HashMap<String, GlobalSystemMediaTransportControlsSession> = HashMap::new();
    let mut art = ArtCache::default();
    let mut timelines = SteadyTimelines::default();
    let mut last: Option<(Vec<MediaPlayer>, Instant)> = None;
    loop {
        // Hook up sessions that are new since last time, keeping their COM wrappers alive.
        let mut present = HashSet::new();
        for session in manager.GetSessions()? {
            let id = session.SourceAppUserModelId()?.to_string();
            if watched.get(&id) != Some(&session) {
                watch(&session, &changed)?;
                watched.insert(id.clone(), session);
            }
            present.insert(id);
        }
        // A session that ends and comes back is hooked up again.
        watched.retain(|id, _| present.contains(id));
        let mut players = snapshot(&manager, &mut art);
        timelines.steady(&mut players, Instant::now());
        let retry_soon = art.needs_retry(&players);
        let should_emit = match &last {
            None => true,
            Some((prev, prev_at)) => players_meaningfully_changed(prev, &players, prev_at.elapsed()),
        };
        if should_emit {
            on_change(players.clone());
            last = Some((players, Instant::now()));
        }
        // Wait for a change (or 400ms retry when a track's artwork stream is still loading, or 4s periodic rescan).
        let wait = if retry_soon { Duration::from_millis(400) } else { Duration::from_secs(4) };
        match changes.recv_timeout(wait) {
            Ok(()) => {
                let deadline = Instant::now() + MAX_SETTLE;
                while let Some(rem) = deadline.checked_duration_since(Instant::now()) {
                    if changes.recv_timeout(rem.min(SETTLE)).is_err() {
                        break;
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

/// How long seeking is kept through a moment without it, and how far past
/// its end a track's held timeline may run.
const TIMELINE_HOLD: Duration = Duration::from_secs(10);

/// Players' timelines as last reported. Apps (browsers above all) report an
/// empty timeline, or stop offering seeking, around play and pause or
/// between updates, sometimes for longer stretches; each gap reached the
/// phone, and its media controls dropped the seek bar until play or pause
/// brought the timeline back (#392). For the same track, a missing timeline
/// is carried on from the last one for as long as that track would still be
/// playing, and seeking that drops out alone is kept for [`TIMELINE_HOLD`].
#[derive(Default)]
struct SteadyTimelines {
    seen: HashMap<String, SeenTimeline>,
}

struct SeenTimeline {
    title: Option<String>,
    duration: u64,
    position: u64,
    playing: bool,
    /// When the app last reported this timeline itself.
    at: Instant,
    /// When the app last offered seeking.
    seek_at: Option<Instant>,
}

impl SeenTimeline {
    /// Whether this timeline still stands in for one the app stopped
    /// reporting: while paused, or until the track would have ended.
    fn lasts(&self, now: Instant) -> bool {
        let since = now.saturating_duration_since(self.at);
        !self.playing
            || since < TIMELINE_HOLD
            || self.position.saturating_add(since.as_millis() as u64) < self.duration
    }

    /// Whether the app offered seeking while it still reported this timeline.
    fn could_seek(&self) -> bool {
        self.seek_at.is_some_and(|at| self.at.saturating_duration_since(at) < TIMELINE_HOLD)
    }
}

impl SteadyTimelines {
    fn steady(&mut self, players: &mut [MediaPlayer], now: Instant) {
        self.seen.retain(|id, _| players.iter().any(|p| &p.id == id));
        for p in players.iter_mut() {
            let offers_seek = p.actions.iter().any(|a| a == "seek");
            let same_track = self.seen.get(&p.id).filter(|s| s.title == p.title);
            let held = |at: Instant| now.saturating_duration_since(at) < TIMELINE_HOLD;
            if let (Some(duration), Some(position)) = (p.duration, p.position) {
                let seek_at = if offers_seek { Some(now) } else { same_track.and_then(|s| s.seek_at) };
                if !offers_seek && seek_at.is_some_and(held) {
                    p.actions.push("seek".to_owned());
                }
                self.seen.insert(
                    p.id.clone(),
                    SeenTimeline {
                        title: p.title.clone(),
                        duration,
                        position,
                        playing: p.playing,
                        at: now,
                        seek_at,
                    },
                );
            } else if let Some(s) = same_track.filter(|s| s.lasts(now)) {
                let moved =
                    if s.playing { now.saturating_duration_since(s.at).as_millis() as u64 } else { 0 };
                p.duration = Some(s.duration);
                p.position = Some(s.position.saturating_add(moved).min(s.duration));
                if !offers_seek && s.could_seek() {
                    p.actions.push("seek".to_owned());
                }
            } else {
                self.seen.remove(&p.id);
            }
        }
    }
}

/// Compares two snapshots of [`MediaPlayer`]s while ignoring normal linear playback
/// position progression so the 4-second poll does not trigger spurious UI/network updates.
fn players_meaningfully_changed(old: &[MediaPlayer], new: &[MediaPlayer], elapsed: Duration) -> bool {
    if old.len() != new.len() {
        return true;
    }
    let elapsed_ms = elapsed.as_millis() as u64;
    for (a, b) in old.iter().zip(new.iter()) {
        if a.id != b.id
            || a.app != b.app
            || a.title != b.title
            || a.artist != b.artist
            || a.album != b.album
            || a.playing != b.playing
            || a.duration != b.duration
            || a.actions != b.actions
            || a.art_key != b.art_key
            || a.art.is_some() != b.art.is_some()
        {
            return true;
        }
        match (a.position, b.position) {
            (None, None) => {}
            (Some(_), None) | (None, Some(_)) => return true,
            (Some(old_pos), Some(new_pos)) => {
                let expected = if a.playing {
                    old_pos.saturating_add(elapsed_ms).min(a.duration.unwrap_or(u64::MAX))
                } else {
                    old_pos
                };
                if new_pos.abs_diff(expected) > 1500 {
                    return true;
                }
            }
        }
    }
    false
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

/// Every session but NectarLink's own, the one Windows considers current
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
    let now = Instant::now();
    for p in &players {
        if p.playing {
            paused_for_call(|s| s.observe_playing(&p.id, now));
        }
    }
    players.sort_by_key(|p| (Some(&p.id) != current.as_ref(), !p.playing));
    players
}

/// Whether a Windows media session belongs to NectarLink itself. The
/// explicit app ID isn't always what the system reports for the SMTC session
/// this app puts a phone's player into; an unpackaged app can get its
/// executable path or a generated ID instead. Matching the name as well
/// keeps that mirror from being read back as a PC player and echoed to the
/// phone as a second, identical player.
fn is_our_session(id: &str) -> bool {
    let lower = id.to_ascii_lowercase();
    lower == AUMID.to_ascii_lowercase() || lower.contains("nectarlink") || lower.contains("nectar-link")
}

fn player(
    session: &GlobalSystemMediaTransportControlsSession,
    art: &mut ArtCache,
) -> windows::core::Result<Option<MediaPlayer>> {
    let id = session.SourceAppUserModelId()?.to_string();
    if is_our_session(&id) {
        return Ok(None);
    }
    let properties = session.TryGetMediaPropertiesAsync()?.join()?;
    // A phone's media this PC shows in the flyout, under whatever ID Windows
    // gives it: sending it on would play a phone's music back to it (#653).
    let raw = |s: windows::core::Result<HSTRING>| s.map(|s| s.to_string()).unwrap_or_default();
    if super::smtc::is_published(&raw(properties.Title()), &raw(properties.Artist())) {
        return Ok(None);
    }
    let text =
        |s: windows::core::Result<HSTRING>| s.ok().map(|s| s.to_string()).filter(|s| !s.trim().is_empty());
    let mut title = text(properties.Title());
    let artist = text(properties.Artist()).filter(|a| !is_opaque_id(a));
    let album = text(properties.AlbumTitle()).filter(|a| !is_opaque_id(a));
    if title.is_none() {
        return Ok(None);
    }
    let video =
        properties.PlaybackType().and_then(|t| t.Value()).is_ok_and(|t| t == MediaPlaybackType::Video);
    {
        let mut guard = VIDEO.lock().unwrap_or_else(|e| e.into_inner());
        let set = guard.get_or_insert_default();
        if video {
            set.insert(id.clone());
        } else {
            set.remove(&id);
        }
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

    let mut resolved_app = app_name(&id);
    let is_browser_app = matches!(
        resolved_app.as_str(),
        "Browser" | "Media" | "Chrome" | "Edge" | "Brave" | "Firefox" | "Vivaldi" | "Opera" | "Zen"
    );
    if is_browser_app
        && let Some(t) = title.clone()
        && let Some((clean_title, site)) = t.rsplit_once(" on ")
    {
        let site = site.trim();
        let clean_title = clean_title.trim();
        if !site.is_empty() && !clean_title.is_empty() && site.len() <= 28 && !is_opaque_id(site) {
            resolved_app = site.to_owned();
            title = Some(clean_title.to_owned());
        }
    }
    let has_smtc_thumb = properties.Thumbnail().is_ok();
    let (art_key, art_bytes) =
        art.get(&id, &title, &artist, &album, has_smtc_thumb, || thumbnail(&properties)).unzip();
    Ok(Some(MediaPlayer {
        app: resolved_app,
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

const MAX_ART_ATTEMPTS: u8 = 8;

type CachedArtEntry = (u8, Option<(String, Vec<u8>)>);

/// Artwork per track, read once: reading it takes a moment.
/// If a player updates its title before its thumbnail stream is ready, we retry
/// on subsequent snapshots instead of permanently locking out `None`, and also
/// persist resolved covers on disk by track hash and album hash.
#[derive(Default)]
struct ArtCache {
    by_track: HashMap<String, CachedArtEntry>,
    by_album: HashMap<String, (String, Vec<u8>)>,
}

impl ArtCache {
    fn track_key(
        id: &str,
        title: &Option<String>,
        artist: &Option<String>,
        album: &Option<String>,
    ) -> String {
        format!("{id}\u{1}{title:?}\u{1}{artist:?}\u{1}{album:?}")
    }

    fn album_key(id: &str, artist: &Option<String>, album: &Option<String>) -> Option<String> {
        let album_str = album.as_deref().map(str::trim).filter(|s| !s.is_empty())?;
        let artist_str = artist.as_deref().map(str::trim).unwrap_or("");
        Some(format!("{id}\u{1}{artist_str}\u{1}{album_str}"))
    }

    fn track_disk_path(track: &str) -> Option<std::path::PathBuf> {
        Some(
            crate::core_host::try_data_dir()?
                .join("cache")
                .join("art")
                .join(format!("pc-track-{:016x}.img", fnv(track.as_bytes()))),
        )
    }

    fn album_disk_path(album: &str) -> Option<std::path::PathBuf> {
        Some(
            crate::core_host::try_data_dir()?
                .join("cache")
                .join("art")
                .join(format!("pc-album-{:016x}.img", fnv(album.as_bytes()))),
        )
    }

    fn get(
        &mut self,
        id: &str,
        title: &Option<String>,
        artist: &Option<String>,
        album: &Option<String>,
        has_smtc_thumb: bool,
        read: impl FnOnce() -> Option<Vec<u8>>,
    ) -> Option<(String, Vec<u8>)> {
        let track = Self::track_key(id, title, artist, album);
        if let Some((_, Some(cached))) = self.by_track.get(&track) {
            return Some(cached.clone());
        }
        let alb_key = Self::album_key(id, artist, album);
        let disk_path = Self::track_disk_path(&track);
        // Bounded on every path in: covers read back from disk used to skip this.
        if self.by_track.len() > 32 {
            self.by_track.clear();
        }
        if self.by_album.len() > 32 {
            self.by_album.clear();
        }
        if !self.by_track.contains_key(&track)
            && let Some(path) = disk_path.as_ref()
            && let Ok(bytes) = std::fs::read(path)
            && !bytes.is_empty()
        {
            let entry = (format!("{:016x}", fnv(&bytes)), bytes);
            if let Some(ak) = alb_key.as_ref() {
                self.by_album.insert(ak.clone(), entry.clone());
            }
            self.by_track.insert(track, (1, Some(entry.clone())));
            return Some(entry);
        }

        let prev_attempts = self.by_track.get(&track).map_or(0, |(a, _)| *a);
        if prev_attempts >= MAX_ART_ATTEMPTS && !has_smtc_thumb {
            return None;
        }
        let attempts = prev_attempts.saturating_add(1);
        let resolved = if has_smtc_thumb { read() } else { None };
        if let Some(bytes) = resolved {
            if let Some(path) = disk_path.as_ref() {
                let _ = std::fs::create_dir_all(path.parent().unwrap_or(std::path::Path::new(".")))
                    .and_then(|()| std::fs::write(path, &bytes));
            }
            if let Some(ak) = alb_key.as_ref()
                && let Some(album_path) = Self::album_disk_path(ak)
            {
                let _ = std::fs::create_dir_all(album_path.parent().unwrap_or(std::path::Path::new(".")))
                    .and_then(|()| std::fs::write(album_path, &bytes));
            }
            let entry = (format!("{:016x}", fnv(&bytes)), bytes);
            if let Some(ak) = alb_key {
                self.by_album.insert(ak, entry.clone());
            }
            self.by_track.insert(track, (attempts, Some(entry.clone())));
            return Some(entry);
        }

        // Fall back to cached album artwork if GSMTC reported no thumbnail or after initial read attempt.
        if (!has_smtc_thumb || attempts >= 2)
            && let Some(ak) = alb_key.as_ref()
        {
            if let Some(cached_album) = self.by_album.get(ak).cloned() {
                self.by_track.insert(track, (attempts, Some(cached_album.clone())));
                return Some(cached_album);
            }
            if let Some(album_path) = Self::album_disk_path(ak)
                && let Ok(bytes) = std::fs::read(&album_path)
                && !bytes.is_empty()
            {
                let entry = (format!("{:016x}", fnv(&bytes)), bytes);
                self.by_album.insert(ak.clone(), entry.clone());
                self.by_track.insert(track, (attempts, Some(entry.clone())));
                return Some(entry);
            }
        }

        self.by_track.insert(track, (attempts, None));
        None
    }

    fn needs_retry(&self, players: &[MediaPlayer]) -> bool {
        players.iter().any(|p| {
            p.art.is_none()
                && self
                    .by_track
                    .get(&Self::track_key(&p.id, &p.title, &p.artist, &p.album))
                    .is_some_and(|(attempts, cached)| cached.is_none() && *attempts < MAX_ART_ATTEMPTS)
        })
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
        let to_load = u32::try_from(size).ok().filter(|&n| n > 0).unwrap_or(512 * 1024);
        let reader = DataReader::CreateDataReader(&stream)?;
        let loaded = reader.LoadAsync(to_load)?.join()?;
        let mut bytes = vec![0u8; loaded as usize];
        reader.ReadBytes(&mut bytes)?;
        Ok(bytes)
    };
    let bytes = read().ok().filter(|b| !b.is_empty())?;
    let is_png_or_jpeg = bytes.starts_with(b"\x89PNG") || bytes.starts_with(b"\xFF\xD8\xFF");
    if is_png_or_jpeg && bytes.len() as u64 <= ART_BYTES {
        return Some(bytes);
    }
    // Either non-PNG/JPEG (e.g. WebP/BMP from browser SMTC) or larger than ART_BYTES:
    // transcode/scale to PNG so Qt and Android always receive standard PNG/JPEG bytes.
    let bitmap = super::image::decode(&bytes).ok()?;
    if (bytes.len() as u64) <= ART_BYTES
        && let Ok(png) = super::image::encode_png(&bitmap)
        && (png.len() as u64) <= ART_BYTES
    {
        return Some(png);
    }
    for max_side in [360, 280, 220, 160] {
        if let Ok(small) = super::image::encode_png(&super::image::scale_to(&bitmap, max_side))
            && (small.len() as u64) <= ART_BYTES
        {
            return Some(small);
        }
    }
    None
}

/// Returns true if `s` looks like an opaque hex hash (e.g. `F0DC299D809B9700`
/// or `308046B0AF4A39CB` used by Chromium/Gecko AUMIDs) or a raw GUID/CRX id.
fn is_opaque_id(s: &str) -> bool {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return true;
    }
    if trimmed.len() >= 8
        && trimmed.chars().any(|c| c.is_ascii_digit())
        && trimmed.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
    {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    lower.starts_with("_crx_")
        || lower.starts_with("crx_")
        || lower.starts_with('{')
        || lower.contains("._crx_")
}

/// Looks up `Software\Classes\AppUserModelId\<id>` (`DisplayName`) in the Windows Registry,
/// where unpackaged Win32 browsers (Chrome, Brave, Edge, Firefox, Zen) register their hashed AUMIDs.
fn registry_aumid_display_name(id: &str) -> Option<String> {
    use windows::Win32::System::Registry::{
        HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW,
    };
    let subkey = HSTRING::from(format!(r"Software\Classes\AppUserModelId\{id}"));
    let value_name = HSTRING::from("DisplayName");
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let mut buf = [0u16; 256];
        let mut byte_len = (buf.len() * 2) as u32;
        let status = unsafe {
            RegGetValueW(
                root,
                &subkey,
                &value_name,
                RRF_RT_REG_SZ,
                None,
                Some(buf.as_mut_ptr().cast()),
                Some(&mut byte_len),
            )
        };
        if status.is_ok() && byte_len >= 2 {
            let len = (byte_len as usize / 2).saturating_sub(1);
            let name = String::from_utf16_lossy(&buf[..len]).trim().to_owned();
            if !name.is_empty() && !name.starts_with('@') && !is_opaque_id(&name) {
                return Some(name);
            }
        }
    }
    None
}

static APP_NAME_CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

/// The app's name as Windows shows it, or a readable form of its ID (memoized per AUMID).
fn app_name(id: &str) -> String {
    let cache = APP_NAME_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(cached) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(id).cloned() {
        return cached;
    }
    let resolved = (|| {
        let display = AppInfo::GetFromAppUserModelId(&HSTRING::from(id))
            .and_then(|info| info.DisplayInfo()?.DisplayName());
        if let Ok(name) = display {
            let name = name.to_string();
            if !name.trim().is_empty() && !is_opaque_id(&name) {
                return name;
            }
        }
        if let Some(reg_name) = registry_aumid_display_name(id) {
            return reg_name;
        }
        readable_name(id)
    })();
    cache.lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_owned(), resolved.clone());
    resolved
}

/// "Spotify.exe" → "Spotify", `C:\…\vlc.exe` → "vlc", "Microsoft.ZuneMusic_8wekyb3d8bbwe!App" → "Media Player",
/// "F0DC299D809B9700" → "Browser".
fn readable_name(id: &str) -> String {
    let id = id.rsplit(['\\', '/']).next().unwrap_or(id);
    let id = id.split('!').next().unwrap_or(id);
    let id = id.split("._crx_").next().unwrap_or(id);
    let base = id.split('_').next().unwrap_or(id);
    let base = base.strip_suffix(".exe").or_else(|| base.strip_suffix(".EXE")).unwrap_or(base);
    let base = base.split('.').next_back().unwrap_or(base).trim();
    let base = match base.rsplit_once('-') {
        Some((prefix, suffix))
            if !prefix.is_empty() && suffix.len() >= 8 && suffix.chars().all(|c| c.is_ascii_hexdigit()) =>
        {
            prefix
        }
        _ => base,
    };
    if base.is_empty() {
        return "Media".into();
    }
    if is_opaque_id(base) {
        return "Browser".into();
    }
    match base.to_ascii_lowercase().as_str() {
        "msedge" => "Edge".into(),
        "chrome" => "Chrome".into(),
        "brave" => "Brave".into(),
        "firefox" => "Firefox".into(),
        "zunemusic" => "Media Player".into(),
        "zunevideo" => "Movies & TV".into(),
        "spotifymusic" | "spotify" => "Spotify".into(),
        "applemusicwin" => "Apple Music".into(),
        _ => base.to_owned(),
    }
}

/// Runs a phone's command on one of this PC's sessions.
pub fn command(player: &str, action: MediaAction, position: Option<u64>) -> Result<(), MediaError> {
    // SAFETY: ensures the caller thread is in the multithreaded COM apartment before WinRT async calls.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
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

/// Tracks which PC media sessions were actively playing when a phone call
/// started and were paused by NectarLink, so only those sessions are resumed
/// when the call ends.
#[derive(Debug, Default)]
pub(crate) struct PausedForCall {
    sessions: HashSet<String>,
    paused_at: Option<Instant>,
}

impl PausedForCall {
    pub fn record_paused(&mut self, id: String, now: Instant) {
        self.sessions.insert(id);
        self.paused_at = Some(now);
    }

    /// If a session we paused was manually resumed by the user during the call
    /// (after the initial pause settled), drop it so a subsequent manual pause
    /// during the call won't be overridden when the call ends.
    pub fn observe_playing(&mut self, id: &str, now: Instant) {
        if self.paused_at.is_some_and(|t| now.duration_since(t) >= Duration::from_millis(600)) {
            self.sessions.remove(id);
        }
    }

    pub fn take_to_resume(&mut self) -> HashSet<String> {
        self.paused_at = None;
        std::mem::take(&mut self.sessions)
    }
}

static PAUSED_FOR_CALL: Mutex<Option<PausedForCall>> = Mutex::new(None);

/// Held while pausing or resuming for a call, so the two never interleave
/// (a call that rings only briefly resumes what its pause recorded).
static CALL_MEDIA: Mutex<()> = Mutex::new(());

fn paused_for_call<T>(f: impl FnOnce(&mut PausedForCall) -> T) -> T {
    f(PAUSED_FOR_CALL.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_default())
}

/// Pauses any PC media sessions that are currently playing (excluding
/// NectarLink's own SMTC session) and remembers them so [`resume_after_call`]
/// can resume only what NectarLink paused.
pub fn pause_for_call() {
    let Some(manager) = MANAGER.get().cloned() else { return };
    std::thread::spawn(move || {
        // SAFETY: initialize COM for this worker thread.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let _serial = CALL_MEDIA.lock().unwrap_or_else(|e| e.into_inner());
        let Ok(sessions) = manager.GetSessions() else { return };
        let now = Instant::now();
        for session in sessions {
            let Ok(id_h) = session.SourceAppUserModelId() else { continue };
            let id = id_h.to_string();
            if is_our_session(&id) {
                continue;
            }
            let is_playing = session
                .GetPlaybackInfo()
                .and_then(|info| info.PlaybackStatus())
                .is_ok_and(|s| s == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing);
            if !is_playing {
                continue;
            }
            if session.TryPauseAsync().and_then(|op| op.join()).unwrap_or(false) {
                tracing::debug!(session = %id, "paused PC media for phone call");
                paused_for_call(|p| p.record_paused(id, now));
            }
        }
    });
}

/// Resumes only the PC media sessions that [`pause_for_call`] paused and that
/// are still paused.
pub fn resume_after_call() {
    let Some(manager) = MANAGER.get().cloned() else { return };
    std::thread::spawn(move || {
        // SAFETY: initialize COM for this worker thread.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let _serial = CALL_MEDIA.lock().unwrap_or_else(|e| e.into_inner());
        let ids = paused_for_call(PausedForCall::take_to_resume);
        if ids.is_empty() {
            return;
        }
        let Ok(sessions) = manager.GetSessions() else { return };
        for session in sessions {
            let Ok(id_h) = session.SourceAppUserModelId() else { continue };
            let id = id_h.to_string();
            if !ids.contains(&id) {
                continue;
            }
            let is_paused = session
                .GetPlaybackInfo()
                .and_then(|info| info.PlaybackStatus())
                .is_ok_and(|s| s == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Paused);
            if is_paused {
                let _ = session.TryPlayAsync().and_then(|op| op.join());
                tracing::debug!(session = %id, "resumed PC media after phone call");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_media_session_is_never_a_pc_player() {
        assert!(is_our_session(AUMID));
        assert!(is_our_session("Nectarlink.Desktop"));
        assert!(is_our_session(r"C:\Program Files\Nectarlink\nectarlink-desktop.exe"));
        assert!(is_our_session("nectarlink-desktop"));
        assert!(!is_our_session("Spotify.exe"));
        assert!(!is_our_session("chrome.exe"));
    }

    fn playing(title: &str, timeline: Option<(u64, u64)>, seek: bool) -> MediaPlayer {
        MediaPlayer {
            id: "Chrome".into(),
            app: "Chrome".into(),
            title: Some(title.into()),
            artist: None,
            album: None,
            playing: true,
            duration: timeline.map(|t| t.0),
            position: timeline.map(|t| t.1),
            actions: [Some("pause"), seek.then_some("seek")]
                .into_iter()
                .flatten()
                .map(str::to_owned)
                .collect(),
            art_key: None,
            art: None,
        }
    }

    #[test]
    fn a_moment_without_a_timeline_keeps_the_seek_bar() {
        let mut steady = SteadyTimelines::default();
        let start = Instant::now();
        let mut first = [playing("Clip", Some((600_000, 10_000)), true)];
        steady.steady(&mut first, start);

        // The app drops its timeline and seeking for a moment.
        let mut blip = [playing("Clip", None, false)];
        steady.steady(&mut blip, start + Duration::from_secs(2));
        assert_eq!(blip[0].duration, Some(600_000));
        assert_eq!(blip[0].position, Some(12_000));
        assert!(blip[0].actions.iter().any(|a| a == "seek"));

        // Seeking alone dropping out is held too.
        let mut no_seek = [playing("Clip", Some((600_000, 13_000)), false)];
        steady.steady(&mut no_seek, start + Duration::from_secs(3));
        assert!(no_seek[0].actions.iter().any(|a| a == "seek"));
    }

    #[test]
    fn a_long_stretch_without_a_timeline_keeps_the_seek_bar_while_the_track_plays() {
        let mut steady = SteadyTimelines::default();
        let start = Instant::now();
        steady.steady(&mut [playing("Clip", Some((600_000, 10_000)), true)], start);

        // The app reports no timeline and no seeking for two minutes.
        let mut gap = [playing("Clip", None, false)];
        steady.steady(&mut gap, start + Duration::from_secs(120));
        assert_eq!(gap[0].duration, Some(600_000));
        assert_eq!(gap[0].position, Some(130_000));
        assert!(gap[0].actions.iter().any(|a| a == "seek"));

        // Seeking it had stopped offering before then isn't brought back.
        let mut steady = SteadyTimelines::default();
        steady.steady(&mut [playing("Clip", Some((600_000, 10_000)), true)], start);
        let at = start + TIMELINE_HOLD * 2;
        steady.steady(&mut [playing("Clip", Some((600_000, 30_000)), false)], at);
        let mut gap = [playing("Clip", None, false)];
        steady.steady(&mut gap, at + Duration::from_secs(1));
        assert_eq!(gap[0].duration, Some(600_000));
        assert!(!gap[0].actions.iter().any(|a| a == "seek"));
    }

    #[test]
    fn a_lasting_or_new_track_without_a_timeline_shows_none() {
        let mut steady = SteadyTimelines::default();
        let start = Instant::now();
        steady.steady(&mut [playing("Clip", Some((600_000, 10_000)), true)], start);

        let mut next = [playing("Another", None, false)];
        steady.steady(&mut next, start + Duration::from_secs(1));
        assert_eq!(next[0].duration, None);
        assert!(!next[0].actions.iter().any(|a| a == "seek"));

        // Carried on past where the track would have ended, it stops.
        steady.steady(&mut [playing("Clip", Some((15_000, 10_000)), true)], start);
        let mut ended = [playing("Clip", None, false)];
        steady.steady(&mut ended, start + Duration::from_secs(5) + TIMELINE_HOLD);
        assert_eq!(ended[0].duration, None);

        let mut seekless = [playing("Clip", Some((600_000, 10_000)), false)];
        steady.steady(&mut seekless, start + TIMELINE_HOLD * 3);
        assert!(!seekless[0].actions.iter().any(|a| a == "seek"));
    }

    #[test]
    fn app_ids_read_as_names() {
        assert_eq!(readable_name("Spotify.exe"), "Spotify");
        assert_eq!(readable_name(r"C:\Program Files\VideoLAN\vlc.exe"), "vlc");
        assert_eq!(readable_name("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"), "Media Player");
        assert_eq!(readable_name("F0DC299D809B9700"), "Browser");
        assert_eq!(readable_name("Chrome"), "Chrome");
        assert_eq!(readable_name(""), "Media");
    }

    #[test]
    fn artwork_is_read_once_per_track() {
        let mut cache = ArtCache::default();
        let mut reads = 0;
        let title = Some("Song".to_owned());
        let album = Some("Album".to_owned());
        for _ in 0..3 {
            let art = cache.get("app", &title, &None, &album, true, || {
                reads += 1;
                Some(vec![1, 2, 3])
            });
            assert_eq!(art.map(|(_, b)| b), Some(vec![1, 2, 3]));
        }
        assert_eq!(reads, 1);
        assert!(cache.get("app", &Some("Other".into()), &None, &None, false, || None).is_none());
        let album_fallback = cache.get("app", &Some("Track 2".into()), &None, &album, false, || None);
        assert_eq!(album_fallback.map(|(_, b)| b), Some(vec![1, 2, 3]));
    }

    #[test]
    fn paused_for_call_resumes_only_what_we_paused_and_drops_user_resumed() {
        let mut state = PausedForCall::default();
        let t0 = Instant::now();
        state.record_paused("Spotify.exe".into(), t0);
        state.record_paused("vlc.exe".into(), t0);

        // Immediate settle notification right after pause does not drop the session.
        state.observe_playing("Spotify.exe", t0 + Duration::from_millis(100));
        assert!(state.sessions.contains("Spotify.exe"));

        // User manually resumes Spotify during the call (after settle window).
        state.observe_playing("Spotify.exe", t0 + Duration::from_secs(2));
        assert!(!state.sessions.contains("Spotify.exe"));

        // When the call ends, only VLC is resumed, and only once.
        let resumed = state.take_to_resume();
        assert_eq!(resumed, HashSet::from(["vlc.exe".to_owned()]));
        assert!(state.take_to_resume().is_empty());
    }
}
