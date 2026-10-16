// SPDX-License-Identifier: GPL-3.0-or-later
//! What plays on this PC, for paired phones (`docs/protocol/media.md`): active
//! MPRIS2 media sessions on Linux (via `playerctl` with a direct `gdbus`
//! D-Bus fallback), artwork resolution/caching, and remote transport commands.
//!
//! NectarLink's own MPRIS session (`org.mpris.MediaPlayer2.nectarlink`) is
//! excluded so a phone never gets its own music echoed back.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use nectarlink_core::{MediaAction, MediaError, MediaPlayer};

/// Maximum inline artwork payload (256 KiB); larger covers are scaled and JPEG-encoded.
const ART_BYTES: u64 = 256 * 1024;
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const CALL_PAUSE_TTL: Duration = Duration::from_secs(60 * 30);

const PLAYERCTL_FORMAT: &str = "{{playerName}}\t{{status}}\t{{mpris:trackid}}\t{{xesam:title}}\t{{xesam:artist}}\t{{xesam:album}}\t{{mpris:length}}\t{{position}}\t{{mpris:artUrl}}";

type OnChange = Box<dyn Fn(Vec<MediaPlayer>) + Send + Sync>;

static STARTED: OnceLock<()> = OnceLock::new();
static PAUSED_FOR_CALL: Mutex<Option<PausedForCall>> = Mutex::new(None);
static CALL_MEDIA: Mutex<()> = Mutex::new(());

#[derive(Debug)]
struct PausedForCall {
    players: Vec<String>,
    paused_at: Instant,
}

impl PausedForCall {
    fn take_valid(&mut self, now: Instant) -> Vec<String> {
        let taken = std::mem::take(&mut self.players);
        if now.saturating_duration_since(self.paused_at) <= CALL_PAUSE_TTL {
            taken
        } else {
            Vec::new()
        }
    }

    fn observe_playing(&mut self, id: &str, now: Instant) {
        if now.saturating_duration_since(self.paused_at) >= Duration::from_millis(600) {
            self.players.retain(|p| p != id);
        }
    }
}

/// Raw MPRIS player record before artwork bytes are loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawMprisPlayer {
    pub id: String,
    pub app: String,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub playing: bool,
    pub duration: Option<u64>,
    pub position: Option<u64>,
    pub actions: Vec<String>,
    pub art_url: Option<String>,
}

/// Caches resolved artwork bytes by URL so repeated polls do not re-read disk.
#[derive(Default)]
struct ArtCache {
    by_url: HashMap<String, Option<(String, Vec<u8>)>>,
}

impl ArtCache {
    fn resolve(&mut self, art_url: Option<&str>) -> Option<(String, Vec<u8>)> {
        let url = art_url?.trim();
        if url.is_empty() {
            return None;
        }
        if let Some(cached) = self.by_url.get(url) {
            return cached.clone();
        }
        if self.by_url.len() > 64 {
            self.by_url.clear();
        }
        let loaded = load_art_from_url(url);
        self.by_url.insert(url.to_owned(), loaded.clone());
        loaded
    }
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (
                (bytes[i + 1] as char).to_digit(16),
                (bytes[i + 2] as char).to_digit(16),
            )
        {
            out.push(((hi << 4) | lo) as u8);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Resolves a `file://` MPRIS `artUrl` (with percent-encoding) to a local filesystem path.
pub(crate) fn resolve_art_file_url(art_url: &str) -> Option<PathBuf> {
    let trimmed = art_url.trim().trim_matches('\'').trim_matches('"').trim();
    let rest = trimmed.strip_prefix("file://")?;
    let decoded = percent_decode(rest);
    if decoded.trim().is_empty() {
        None
    } else {
        Some(PathBuf::from(decoded))
    }
}

fn load_art_from_url(art_url: &str) -> Option<(String, Vec<u8>)> {
    let path = resolve_art_file_url(art_url)?;
    let bytes = fs::read(path).ok().filter(|b| !b.is_empty())?;
    let is_png_or_jpeg = bytes.starts_with(b"\x89PNG") || bytes.starts_with(b"\xFF\xD8\xFF");
    if is_png_or_jpeg && (bytes.len() as u64) <= ART_BYTES {
        let key = format!("{:016x}", fnv(&bytes));
        return Some((key, bytes));
    }
    let bitmap = super::image::decode(&bytes).ok()?;
    for max_side in [360, 280, 220, 160] {
        let scaled = super::image::scale_to(&bitmap, max_side);
        if let Ok(jpeg) = super::image::encode_jpeg(&scaled, 85)
            && (jpeg.len() as u64) <= ART_BYTES
        {
            let key = format!("{:016x}", fnv(&jpeg));
            return Some((key, jpeg));
        }
    }
    None
}

/// Watches this PC's active MPRIS2 media sessions on a background thread;
/// `on_change` is invoked whenever the player list or playback state meaningfully changes.
pub fn start<F>(on_change: F)
where
    F: Fn(Vec<MediaPlayer>) + Send + Sync + 'static,
{
    if STARTED.set(()).is_err() {
        return;
    }
    let on_change: OnChange = Box::new(on_change);
    let spawned = std::thread::Builder::new()
        .name("media-sessions".into())
        .spawn(move || run(&on_change));
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "can't spawn Linux media-sessions watcher");
    }
}

fn run(on_change: &OnChange) {
    let mut art_cache = ArtCache::default();
    let mut last: Option<(Vec<MediaPlayer>, Instant)> = None;
    loop {
        let players = snapshot(&mut art_cache);
        let should_emit = match &last {
            None => true,
            Some((prev, prev_at)) => {
                players_meaningfully_changed(prev, &players, prev_at.elapsed())
            }
        };
        if should_emit {
            on_change(players.clone());
            last = Some((players, Instant::now()));
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Compares two snapshots of [`MediaPlayer`]s while ignoring normal linear playback
/// position progression so periodic polling does not trigger spurious UI/network updates.
pub(crate) fn players_meaningfully_changed(
    old: &[MediaPlayer],
    new: &[MediaPlayer],
    elapsed: Duration,
) -> bool {
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
                    old_pos
                        .saturating_add(elapsed_ms)
                        .min(a.duration.unwrap_or(u64::MAX))
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

fn snapshot(art_cache: &mut ArtCache) -> Vec<MediaPlayer> {
    let raw_players = query_raw_players();
    let now = Instant::now();
    let mut players = Vec::with_capacity(raw_players.len());
    for raw in raw_players {
        if raw.playing
            && let Ok(mut guard) = PAUSED_FOR_CALL.lock()
            && let Some(state) = guard.as_mut()
        {
            state.observe_playing(&raw.id, now);
        }
        let (art_key, art) = art_cache.resolve(raw.art_url.as_deref()).unzip();
        players.push(MediaPlayer {
            app: raw.app,
            id: raw.id,
            title: Some(raw.title),
            artist: raw.artist,
            album: raw.album,
            playing: raw.playing,
            duration: raw.duration,
            position: raw.position,
            actions: raw.actions,
            art_key,
            art,
        });
    }
    players.sort_by_key(|p| !p.playing);
    players
}

fn query_raw_players() -> Vec<RawMprisPlayer> {
    if let Some(players) = query_via_playerctl() {
        return players;
    }
    query_via_gdbus().unwrap_or_default()
}

fn query_via_playerctl() -> Option<Vec<RawMprisPlayer>> {
    let output = Command::new("playerctl")
        .args(["--all-players", "metadata", "--format", PLAYERCTL_FORMAT])
        .output()
        .ok()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("No players found") || output.stdout.is_empty() {
            return Some(Vec::new());
        }
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Some(stdout.lines().filter_map(parse_playerctl_line).collect())
}

fn is_excluded_player(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    lower.is_empty()
        || lower == "nectarlink"
        || lower == "playerctld"
        || lower.starts_with("nectarlink.")
        || lower.ends_with(".nectarlink")
        || lower.ends_with(".playerctld")
}

fn readable_player_app(id: &str) -> String {
    let stripped = id.strip_prefix("org.mpris.MediaPlayer2.").unwrap_or(id);
    let base = stripped.split('.').next().unwrap_or(stripped).trim();
    match base.to_ascii_lowercase().as_str() {
        "" => "Media".to_owned(),
        "spotify" | "spotifymusic" => "Spotify".to_owned(),
        "vlc" => "VLC".to_owned(),
        "firefox" => "Firefox".to_owned(),
        "chromium" => "Chromium".to_owned(),
        "chrome" | "google-chrome" => "Chrome".to_owned(),
        "brave" => "Brave".to_owned(),
        "msedge" | "edge" => "Edge".to_owned(),
        "mpv" => "mpv".to_owned(),
        "rhythmbox" => "Rhythmbox".to_owned(),
        "amberol" => "Amberol".to_owned(),
        "elisa" => "Elisa".to_owned(),
        "audacious" => "Audacious".to_owned(),
        "clementine" => "Clementine".to_owned(),
        "strawberry" => "Strawberry".to_owned(),
        other => {
            let mut chars = other.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => "Media".to_owned(),
            }
        }
    }
}

/// Parses one line of `playerctl --all-players metadata --format ...` output.
pub(crate) fn parse_playerctl_line(line: &str) -> Option<RawMprisPlayer> {
    if line.trim().is_empty() {
        return None;
    }
    let fields: Vec<&str> = line.split('\t').collect();
    if fields.len() < 4 {
        return None;
    }
    let player_name = fields.first().copied().unwrap_or("").trim();
    if is_excluded_player(player_name) {
        return None;
    }
    let status = fields.get(1).copied().unwrap_or("").trim();
    let _track_id = fields.get(2).copied().unwrap_or("").trim();
    let raw_title = fields.get(3).copied().unwrap_or("").trim();
    if raw_title.is_empty() {
        return None;
    }
    let artist = fields
        .get(4)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let album = fields
        .get(5)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let duration_us = fields
        .get(6)
        .and_then(|s| s.trim().parse::<i64>().ok())
        .filter(|&v| v > 0);
    let position_us = fields
        .get(7)
        .and_then(|s| s.trim().parse::<i64>().ok())
        .filter(|&v| v >= 0);
    let art_url = fields
        .get(8)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(str::to_owned);

    let duration = duration_us.map(|us| (us as u64) / 1000).filter(|&ms| ms > 0);
    let position = position_us.map(|us| {
        let ms = (us as u64) / 1000;
        duration.map_or(ms, |d| ms.min(d))
    });

    let mut app = readable_player_app(player_name);
    let mut title = raw_title.to_owned();
    if matches!(
        app.as_str(),
        "Firefox" | "Chrome" | "Chromium" | "Brave" | "Edge"
    ) && let Some((clean, site)) = title.rsplit_once(" on ")
    {
        let site = site.trim();
        let clean = clean.trim();
        if !site.is_empty() && !clean.is_empty() && site.len() <= 28 {
            app = site.to_owned();
            title = clean.to_owned();
        }
    }

    let playing = status.eq_ignore_ascii_case("Playing");
    let mut actions = vec![
        "play".to_owned(),
        "pause".to_owned(),
        "next".to_owned(),
        "previous".to_owned(),
    ];
    if duration.is_some() {
        actions.push("seek".to_owned());
    }

    Some(RawMprisPlayer {
        id: player_name.to_owned(),
        app,
        title,
        artist,
        album,
        playing,
        duration,
        position,
        actions,
        art_url,
    })
}

/// Extracts `org.mpris.MediaPlayer2.*` bus names from `gdbus ListNames` output,
/// excluding `org.mpris.MediaPlayer2.nectarlink` and `playerctld`.
pub(crate) fn parse_mpris_bus_names(gdbus_output: &str) -> Vec<String> {
    let mut names = Vec::new();
    for token in gdbus_output.split(|c: char| {
        c == '\'' || c == '"' || c == ',' || c == '[' || c == ']' || c == '(' || c == ')' || c.is_whitespace()
    }) {
        let trimmed = token.trim();
        if let Some(suffix) = trimmed.strip_prefix("org.mpris.MediaPlayer2.")
            && !is_excluded_player(suffix)
            && !names.iter().any(|existing| existing == trimmed)
        {
            names.push(trimmed.to_owned());
        }
    }
    names
}

fn extract_gvariant_string(text: &str, key: &str) -> Option<String> {
    let pattern = format!("'{key}':");
    let start = text.find(&pattern)? + pattern.len();
    let rest = text[start..].trim_start();
    // Look for `<'value'>` or `<['value']>`
    let open_angle = rest.find('<')?;
    let inside = rest[open_angle + 1..].trim_start();
    let inside = inside.strip_prefix('[').map_or(inside, str::trim_start);
    let quote = inside.chars().next()?;
    if quote != '\'' && quote != '"' {
        return None;
    }
    let after_quote = &inside[quote.len_utf8()..];
    let end_quote = after_quote.find(quote)?;
    let val = after_quote[..end_quote].trim();
    if val.is_empty() {
        None
    } else {
        Some(val.to_owned())
    }
}

fn extract_gvariant_i64(text: &str, key: &str) -> Option<i64> {
    let pattern = format!("'{key}':");
    let start = text.find(&pattern)? + pattern.len();
    let rest = text[start..].trim_start();
    let open = rest.find('<')?;
    let close = rest[open + 1..].find('>')?;
    let inner = rest[open + 1..open + 1 + close].trim();
    let num_str = inner.split_whitespace().next_back()?;
    num_str.parse::<i64>().ok()
}

fn extract_gvariant_bool(text: &str, key: &str) -> Option<bool> {
    let pattern = format!("'{key}':");
    let start = text.find(&pattern)? + pattern.len();
    let rest = text[start..].trim_start();
    let open = rest.find('<')?;
    let close = rest[open + 1..].find('>')?;
    match rest[open + 1..open + 1 + close].trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

pub(crate) fn parse_gdbus_player_properties(bus_name: &str, gdbus_output: &str) -> Option<RawMprisPlayer> {
    let short_id = bus_name
        .strip_prefix("org.mpris.MediaPlayer2.")
        .unwrap_or(bus_name);
    if is_excluded_player(short_id) {
        return None;
    }
    let title = extract_gvariant_string(gdbus_output, "xesam:title")?;
    let status = extract_gvariant_string(gdbus_output, "PlaybackStatus").unwrap_or_default();
    let artist = extract_gvariant_string(gdbus_output, "xesam:artist");
    let album = extract_gvariant_string(gdbus_output, "xesam:album");
    let art_url = extract_gvariant_string(gdbus_output, "mpris:artUrl");
    let duration_us = extract_gvariant_i64(gdbus_output, "mpris:length").filter(|&v| v > 0);
    let position_us = extract_gvariant_i64(gdbus_output, "Position").filter(|&v| v >= 0);

    let duration = duration_us.map(|us| (us as u64) / 1000).filter(|&ms| ms > 0);
    let position = position_us.map(|us| {
        let ms = (us as u64) / 1000;
        duration.map_or(ms, |d| ms.min(d))
    });

    let mut actions = Vec::new();
    if extract_gvariant_bool(gdbus_output, "CanPlay").unwrap_or(true) {
        actions.push("play".to_owned());
    }
    if extract_gvariant_bool(gdbus_output, "CanPause").unwrap_or(true) {
        actions.push("pause".to_owned());
    }
    if extract_gvariant_bool(gdbus_output, "CanGoNext").unwrap_or(true) {
        actions.push("next".to_owned());
    }
    if extract_gvariant_bool(gdbus_output, "CanGoPrevious").unwrap_or(true) {
        actions.push("previous".to_owned());
    }
    if extract_gvariant_bool(gdbus_output, "CanSeek").unwrap_or(duration.is_some()) {
        actions.push("seek".to_owned());
    }

    Some(RawMprisPlayer {
        id: short_id.to_owned(),
        app: readable_player_app(short_id),
        title,
        artist,
        album,
        playing: status.eq_ignore_ascii_case("Playing"),
        duration,
        position,
        actions,
        art_url,
    })
}

fn list_gdbus_mpris_buses() -> Option<Vec<String>> {
    let out = Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.DBus",
            "--object-path",
            "/org/freedesktop/DBus",
            "--method",
            "org.freedesktop.DBus.ListNames",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(parse_mpris_bus_names(&String::from_utf8_lossy(&out.stdout)))
}

fn query_via_gdbus() -> Option<Vec<RawMprisPlayer>> {
    let buses = list_gdbus_mpris_buses()?;
    let mut players = Vec::new();
    for bus in buses {
        if let Ok(out) = Command::new("gdbus")
            .args([
                "call",
                "--session",
                "--dest",
                &bus,
                "--object-path",
                "/org/mpris/MediaPlayer2",
                "--method",
                "org.freedesktop.DBus.Properties.GetAll",
                "org.mpris.MediaPlayer2.Player",
            ])
            .output()
            && out.status.success()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            if let Some(player) = parse_gdbus_player_properties(&bus, &text) {
                players.push(player);
            }
        }
    }
    Some(players)
}

fn mpris_bus_name(player: &str) -> String {
    let trimmed = player.trim();
    if trimmed.starts_with("org.mpris.MediaPlayer2.") {
        trimmed.to_owned()
    } else {
        format!("org.mpris.MediaPlayer2.{trimmed}")
    }
}

fn gdbus_player_call(bus_name: &str, method: &str, extra_args: &[&str]) -> bool {
    let method_full = format!("org.mpris.MediaPlayer2.Player.{method}");
    let mut cmd = Command::new("gdbus");
    cmd.args([
        "call",
        "--session",
        "--dest",
        bus_name,
        "--object-path",
        "/org/mpris/MediaPlayer2",
        "--method",
        &method_full,
    ]);
    cmd.args(extra_args);
    cmd.output().is_ok_and(|o| o.status.success())
}

pub fn command(player: &str, action: MediaAction, position: Option<u64>) -> Result<(), MediaError> {
    let mut cmd = Command::new("playerctl");
    if !player.is_empty() {
        cmd.args(["--player", player]);
    }
    match action {
        MediaAction::Play => {
            cmd.arg("play");
        }
        MediaAction::Pause => {
            cmd.arg("pause");
        }
        MediaAction::Next => {
            cmd.arg("next");
        }
        MediaAction::Previous => {
            cmd.arg("previous");
        }
        MediaAction::Seek => {
            let secs = (position.unwrap_or(0) as f64) / 1000.0;
            cmd.args(["position", &format!("{secs:.2}")]);
        }
    }
    if cmd.status().is_ok_and(|s| s.success()) {
        return Ok(());
    }

    // Fallback to direct `gdbus` MPRIS2 call when `playerctl` is not installed or fails.
    let target_bus = if player.trim().is_empty() {
        list_gdbus_mpris_buses()
            .and_then(|mut b| b.drain(..).next())
            .ok_or(MediaError::NotFound)?
    } else {
        mpris_bus_name(player)
    };

    let ok = match action {
        MediaAction::Play => gdbus_player_call(&target_bus, "Play", &[]),
        MediaAction::Pause => gdbus_player_call(&target_bus, "Pause", &[]),
        MediaAction::Next => gdbus_player_call(&target_bus, "Next", &[]),
        MediaAction::Previous => gdbus_player_call(&target_bus, "Previous", &[]),
        MediaAction::Seek => {
            let target_us = i64::try_from(position.unwrap_or(0))
                .unwrap_or(0)
                .saturating_mul(1000);
            let offset_str = target_us.to_string();
            gdbus_player_call(&target_bus, "Seek", &[&offset_str])
        }
    };
    if ok {
        Ok(())
    } else {
        Err(MediaError::Failed(format!(
            "MPRIS command {action:?} failed for {target_bus}"
        )))
    }
}

pub fn pause_for_call() {
    let _serial = CALL_MEDIA.lock().unwrap_or_else(|e| e.into_inner());
    let mut paused = Vec::new();

    if let Ok(out) = Command::new("playerctl")
        .args([
            "--all-players",
            "status",
            "--format",
            "{{playerName}}\t{{status}}",
        ])
        .output()
        && out.status.success()
    {
        let stdout = String::from_utf8_lossy(&out.stdout);
        for line in stdout.lines() {
            if let Some((name, status)) = line.split_once('\t') {
                let name = name.trim();
                if !is_excluded_player(name) && status.trim().eq_ignore_ascii_case("Playing") {
                    let _ = Command::new("playerctl")
                        .args(["--player", name, "pause"])
                        .status();
                    paused.push(name.to_owned());
                }
            }
        }
    } else if let Some(players) = query_via_gdbus() {
        for p in players {
            if p.playing {
                let bus = mpris_bus_name(&p.id);
                if gdbus_player_call(&bus, "Pause", &[]) {
                    paused.push(p.id);
                }
            }
        }
    }

    if !paused.is_empty()
        && let Ok(mut guard) = PAUSED_FOR_CALL.lock()
    {
        *guard = Some(PausedForCall {
            players: paused,
            paused_at: Instant::now(),
        });
    }
}

pub fn resume_after_call() {
    let _serial = CALL_MEDIA.lock().unwrap_or_else(|e| e.into_inner());
    let players = PAUSED_FOR_CALL
        .lock()
        .ok()
        .and_then(|mut g| g.as_mut().map(|p| p.take_valid(Instant::now())))
        .unwrap_or_default();
    for player in players {
        let ok = Command::new("playerctl")
            .args(["--player", &player, "play"])
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            let bus = mpris_bus_name(&player);
            let _ = gdbus_player_call(&bus, "Play", &[]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_player(id: &str, playing: bool, position: Option<u64>) -> MediaPlayer {
        MediaPlayer {
            id: id.to_owned(),
            app: "Spotify".to_owned(),
            title: Some("Track 1".to_owned()),
            artist: Some("Artist".to_owned()),
            album: Some("Album".to_owned()),
            playing,
            duration: Some(200_000),
            position,
            actions: vec!["play".to_owned(), "pause".to_owned(), "seek".to_owned()],
            art_key: None,
            art: None,
        }
    }

    #[test]
    fn parses_playerctl_line_and_filters_self_player() {
        let line = "spotify\tPlaying\t/com/spotify/track/123\tMidnight City\tM83\tHurry Up, We're Dreaming\t243000000\t61500000\tfile:///tmp/cover%20art.jpg";
        let parsed = parse_playerctl_line(line).expect("valid playerctl line");
        assert_eq!(parsed.id, "spotify");
        assert_eq!(parsed.app, "Spotify");
        assert_eq!(parsed.title, "Midnight City");
        assert_eq!(parsed.artist.as_deref(), Some("M83"));
        assert_eq!(parsed.album.as_deref(), Some("Hurry Up, We're Dreaming"));
        assert!(parsed.playing);
        assert_eq!(parsed.duration, Some(243_000));
        assert_eq!(parsed.position, Some(61_500));
        assert_eq!(parsed.art_url.as_deref(), Some("file:///tmp/cover%20art.jpg"));

        // Self-player and playerctld must be ignored
        let self_line = "nectarlink\tPlaying\t/1\tPhone Song\tArtist\tAlbum\t1000000\t0\t";
        assert!(parse_playerctl_line(self_line).is_none());
        let daemon_line = "playerctld\tPlaying\t/1\tPhone Song\tArtist\tAlbum\t1000000\t0\t";
        assert!(parse_playerctl_line(daemon_line).is_none());
    }

    #[test]
    fn parses_mpris_bus_names_and_excludes_nectarlink() {
        let gdbus_out = "([':1.0', 'org.freedesktop.DBus', 'org.mpris.MediaPlayer2.spotify', 'org.mpris.MediaPlayer2.nectarlink', 'org.mpris.MediaPlayer2.playerctld', 'org.mpris.MediaPlayer2.vlc.instance42'],)";
        let buses = parse_mpris_bus_names(gdbus_out);
        assert_eq!(
            buses,
            vec![
                "org.mpris.MediaPlayer2.spotify".to_owned(),
                "org.mpris.MediaPlayer2.vlc.instance42".to_owned(),
            ]
        );
    }

    #[test]
    fn parses_gdbus_getall_player_properties() {
        let gdbus_props = "({'PlaybackStatus': <'Playing'>, 'Position': <int64 30000000>, 'Metadata': <{'mpris:length': <int64 180000000>, 'mpris:artUrl': <'file:///home/user/Album%20Cover.png'>, 'xesam:title': <'Starlight'>, 'xesam:artist': <['Muse']>, 'xesam:album': <'Black Holes and Revelations'>}>, 'CanPlay': <true>, 'CanPause': <true>, 'CanSeek': <true>},)";
        let player = parse_gdbus_player_properties("org.mpris.MediaPlayer2.vlc", gdbus_props)
            .expect("parse gdbus properties");
        assert_eq!(player.id, "vlc");
        assert_eq!(player.app, "VLC");
        assert_eq!(player.title, "Starlight");
        assert_eq!(player.artist.as_deref(), Some("Muse"));
        assert_eq!(player.album.as_deref(), Some("Black Holes and Revelations"));
        assert!(player.playing);
        assert_eq!(player.duration, Some(180_000));
        assert_eq!(player.position, Some(30_000));
    }

    #[test]
    fn players_meaningfully_changed_ignores_linear_progress_and_detects_seeks() {
        let old = vec![sample_player("spotify", true, Some(10_000))];
        // 2 seconds later, position is 12_100 ms (within 1500 ms of expected 12_000 ms)
        let normal_tick = vec![sample_player("spotify", true, Some(12_100))];
        assert!(!players_meaningfully_changed(
            &old,
            &normal_tick,
            Duration::from_secs(2)
        ));

        // User seeked forward to 45_000 ms
        let seeked = vec![sample_player("spotify", true, Some(45_000))];
        assert!(players_meaningfully_changed(
            &old,
            &seeked,
            Duration::from_secs(2)
        ));

        // Playback paused
        let paused = vec![sample_player("spotify", false, Some(12_000))];
        assert!(players_meaningfully_changed(
            &old,
            &paused,
            Duration::from_secs(2)
        ));
    }

    #[test]
    fn resolves_art_file_url_with_percent_encoding() {
        assert_eq!(
            resolve_art_file_url("file:///path/with%20spaces/cover%231.png"),
            Some(PathBuf::from("/path/with spaces/cover#1.png"))
        );
        assert!(resolve_art_file_url("https://example.com/cover.jpg").is_none());
        assert!(resolve_art_file_url("").is_none());
    }

    #[test]
    fn paused_for_call_expires_after_ttl() {
        let start = Instant::now();
        let mut state = PausedForCall {
            players: vec!["spotify".to_string()],
            paused_at: start,
        };
        let expired = state.take_valid(start + CALL_PAUSE_TTL + Duration::from_secs(1));
        assert!(expired.is_empty());
    }

    #[test]
    fn paused_for_call_returns_players_within_ttl() {
        let start = Instant::now();
        let mut state = PausedForCall {
            players: vec!["spotify".to_string(), "vlc".to_string()],
            paused_at: start,
        };
        let valid = state.take_valid(start + Duration::from_secs(10));
        assert_eq!(valid, vec!["spotify".to_string(), "vlc".to_string()]);
        assert!(state.take_valid(start + Duration::from_secs(11)).is_empty());
    }
}
