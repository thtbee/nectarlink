// SPDX-License-Identifier: GPL-3.0-or-later
//! Write-ahead recovery of desktop audio settings and selective media resume.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use nectarlink_core::{
    AudioLaneLevel, AudioLatencyPreset, AudioPlaybackPolicy, AudioRouteLevels, AudioRoutePcm, AudioRouteRole,
    AudioRouteSet, AudioRouteStatus, DeviceId, LinkState, NodeEvent,
    audio_mix::{AudioFormat, LatencyPreset, LiveAudioMixer, PlaybackPolicy, SinkMode},
};
use serde::{Deserialize, Serialize};

const JOURNAL_NAME: &str = "audio-state-journal.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioRole {
    Default,
    Console,
    Multimedia,
    Communications,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeakerMuteRecord {
    pub endpoint_id: String,
    pub was_muted: bool,
    pub owned_mute: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefaultOutputRecord {
    pub role: AudioRole,
    pub previous_endpoint_id: String,
    pub owned_endpoint_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefaultInputRecord {
    pub role: AudioRole,
    pub previous_endpoint_id: String,
    pub owned_endpoint_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AudioStateRecord {
    SpeakerMute(SpeakerMuteRecord),
    DefaultOutput(DefaultOutputRecord),
    DefaultInput(DefaultInputRecord),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum RecordKey<'a> {
    SpeakerMute(&'a str),
    DefaultOutput(AudioRole),
    DefaultInput(AudioRole),
}

impl AudioStateRecord {
    fn key(&self) -> RecordKey<'_> {
        match self {
            Self::SpeakerMute(record) => RecordKey::SpeakerMute(&record.endpoint_id),
            Self::DefaultOutput(record) => RecordKey::DefaultOutput(record.role),
            Self::DefaultInput(record) => RecordKey::DefaultInput(record.role),
        }
    }

    fn preserve_original_from(&mut self, prior: &Self) {
        match (self, prior) {
            (Self::SpeakerMute(next), Self::SpeakerMute(prior)) if next.was_muted == prior.owned_mute => {
                next.was_muted = prior.was_muted;
            }
            (Self::DefaultOutput(next), Self::DefaultOutput(prior))
                if next.previous_endpoint_id == prior.owned_endpoint_id =>
            {
                next.previous_endpoint_id.clone_from(&prior.previous_endpoint_id);
            }
            (Self::DefaultInput(next), Self::DefaultInput(prior))
                if next.previous_endpoint_id == prior.owned_endpoint_id =>
            {
                next.previous_endpoint_id.clone_from(&prior.previous_endpoint_id);
            }
            _ => {}
        }
    }

    fn validate(&self) -> io::Result<()> {
        if matches!(self, Self::SpeakerMute(record) if !record.owned_mute) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the journal can only own a speaker mute",
            ));
        }
        Ok(())
    }

    fn restore(&self, backend: &mut impl AudioStateBackend) -> io::Result<bool> {
        match self {
            Self::SpeakerMute(record) => {
                if backend.speaker_muted(&record.endpoint_id)? != record.owned_mute {
                    return Ok(false);
                }
                backend.set_speaker_muted(&record.endpoint_id, record.was_muted)?;
            }
            Self::DefaultOutput(record) => {
                if backend.default_output(record.role)?.as_deref() != Some(&record.owned_endpoint_id) {
                    return Ok(false);
                }
                backend.set_default_output(record.role, &record.previous_endpoint_id)?;
            }
            Self::DefaultInput(record) => {
                if backend.default_input(record.role)?.as_deref() != Some(&record.owned_endpoint_id) {
                    return Ok(false);
                }
                backend.set_default_input(record.role, &record.previous_endpoint_id)?;
            }
        }
        Ok(true)
    }
}

/// Platform reads must report a system error as `Err`. A default endpoint of
/// `None` means the system currently has no default for that role.
pub trait AudioStateBackend {
    fn speaker_muted(&mut self, endpoint_id: &str) -> io::Result<bool>;
    fn set_speaker_muted(&mut self, endpoint_id: &str, muted: bool) -> io::Result<()>;
    fn default_output(&mut self, role: AudioRole) -> io::Result<Option<String>>;
    fn set_default_output(&mut self, role: AudioRole, endpoint_id: &str) -> io::Result<()>;
    fn default_input(&mut self, role: AudioRole) -> io::Result<Option<String>>;
    fn set_default_input(&mut self, role: AudioRole, endpoint_id: &str) -> io::Result<()>;
}

#[derive(Debug, Default)]
pub struct RecoveryReport {
    pub restored: usize,
    pub superseded: usize,
    pub pending: usize,
    pub failures: Vec<io::Error>,
}

/// Keep one journal per app data directory and serialize desktop audio changes
/// through its owner. Persist each record before applying its owned value.
#[derive(Debug)]
pub struct AudioStateJournal {
    path: PathBuf,
    records: Vec<AudioStateRecord>,
}

impl AudioStateJournal {
    pub fn open(app_data_dir: &Path) -> io::Result<Self> {
        let path = app_data_dir.join(JOURNAL_NAME);
        let records: Vec<AudioStateRecord> = match File::open(&path) {
            Ok(file) => serde_json::from_reader(file)?,
            Err(err) if err.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(err) => return Err(err),
        };
        for record in &records {
            record.validate()?;
        }
        Ok(Self { path, records })
    }

    pub fn records(&self) -> &[AudioStateRecord] {
        &self.records
    }

    /// Persists a mutation before the caller changes the system. For a later
    /// mutation of the same setting, read its current value again as the previous value.
    /// The journal keeps the original value if we still own the current one.
    pub fn record(&mut self, mut record: AudioStateRecord) -> io::Result<()> {
        record.validate()?;
        let mut records = self.records.clone();
        if let Some(prior) = records.iter_mut().find(|prior| prior.key() == record.key()) {
            record.preserve_original_from(prior);
            if *prior == record {
                return Ok(());
            }
            *prior = record;
        } else {
            records.push(record);
        }
        self.persist(records)
    }

    /// Compare each system value with the value we own. Persist failures for retry.
    pub fn recover(&mut self, backend: &mut impl AudioStateBackend) -> io::Result<RecoveryReport> {
        let mut report = RecoveryReport::default();
        let mut pending = Vec::new();
        for record in &self.records {
            match record.restore(backend) {
                Ok(true) => report.restored += 1,
                Ok(false) => report.superseded += 1,
                Err(err) => {
                    pending.push(record.clone());
                    report.failures.push(err);
                }
            }
        }
        report.pending = pending.len();
        if pending.len() != self.records.len() {
            self.persist(pending)?;
        }
        Ok(report)
    }

    fn persist(&mut self, records: Vec<AudioStateRecord>) -> io::Result<()> {
        let bytes = serde_json::to_vec(&records)?;
        let directory = self.path.parent().expect("journal path has a parent");
        fs::create_dir_all(directory)?;
        let temporary = directory.join(format!("{JOURNAL_NAME}.{:016x}.tmp", rand::random::<u64>()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        let written = file.write_all(&bytes).and_then(|()| file.sync_all());
        drop(file);
        if let Err(err) = written.and_then(|()| fs::rename(&temporary, &self.path)) {
            let _ = fs::remove_file(&temporary);
            return Err(err);
        }
        #[cfg(unix)]
        File::open(directory)?.sync_all()?;
        self.records = records;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PauseOwner {
    PhoneCall,
    AudioRoute { generation: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseRelease {
    Normal { selective_resume: bool },
    Abnormal,
}

/// The ID is the platform session ID (AUMID or MPRIS instance). The backend
/// changes the incarnation when a different session reuses the same ID.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct MediaSessionKey {
    pub id: String,
    pub incarnation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Playing,
    Paused,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaSession {
    pub key: MediaSessionKey,
    pub state: PlaybackState,
}

/// Commands address an exact session incarnation. Return true only if this
/// command changed a playing session to paused or a paused session to playing.
pub trait MediaPauseBackend {
    fn sessions(&mut self) -> io::Result<Vec<MediaSession>>;
    fn pause_if_playing(&mut self, session: &MediaSessionKey) -> io::Result<bool>;
    fn play_if_paused(&mut self, session: &MediaSessionKey) -> io::Result<bool>;
}

/// Hold this coordinator on one worker, or under one lock shared by both
/// pause sources. Each pause ticket belongs to every current pause owner.
#[derive(Debug, Default)]
pub struct MediaPauseCoordinator {
    owners: BTreeSet<PauseOwner>,
    tickets: BTreeMap<MediaSessionKey, BTreeSet<PauseOwner>>,
    suppressed: BTreeSet<MediaSessionKey>,
}

impl MediaPauseCoordinator {
    /// Apply the current pause policy even while playback is already in progress.
    pub fn set_pause(
        &mut self,
        owner: PauseOwner,
        pause: bool,
        selective_resume: bool,
        backend: &mut impl MediaPauseBackend,
    ) -> io::Result<()> {
        if pause {
            if self.owners.insert(owner) {
                for owners in self.tickets.values_mut() {
                    owners.insert(owner);
                }
            }
            self.reconcile(backend)
        } else {
            self.release(owner, PauseRelease::Normal { selective_resume }, backend)
        }
    }

    /// Recheck while a pause policy is active, for example on a playback change event.
    pub fn reconcile(&mut self, backend: &mut impl MediaPauseBackend) -> io::Result<()> {
        if self.owners.is_empty() {
            return Ok(());
        }

        let sessions: BTreeMap<_, _> =
            backend.sessions()?.into_iter().map(|session| (session.key, session.state)).collect();
        self.tickets.retain(|session, _| {
            if sessions.get(session) == Some(&PlaybackState::Paused) {
                true
            } else {
                // The user changed playback or the original session disappeared.
                self.suppressed.insert(session.clone());
                false
            }
        });

        let mut first_error = None;
        for (session, state) in sessions {
            if state == PlaybackState::Playing && !self.suppressed.contains(&session) {
                match backend.pause_if_playing(&session) {
                    Ok(true) => {
                        self.tickets.insert(session, self.owners.clone());
                    }
                    Ok(false) => {}
                    Err(err) => {
                        first_error.get_or_insert(err);
                    }
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Revoke a ticket on an observed manual playback or session replacement
    /// event, even if playback has changed again by the next snapshot.
    pub fn invalidate_session(&mut self, session: &MediaSessionKey) {
        if self.tickets.remove(session).is_some() {
            self.suppressed.insert(session.clone());
        }
    }

    #[cfg(test)]
    pub fn paused_sessions(&self, owner: PauseOwner) -> impl Iterator<Item = &MediaSessionKey> {
        self.tickets.iter().filter_map(move |(session, owners)| owners.contains(&owner).then_some(session))
    }

    /// Release ownership without resuming while another owner still requires
    /// pause. Expiry revokes affected tickets even when another owner remains.
    pub fn release(
        &mut self,
        owner: PauseOwner,
        release: PauseRelease,
        backend: &mut impl MediaPauseBackend,
    ) -> io::Result<()> {
        if !self.owners.remove(&owner) {
            return Ok(());
        }
        if release == PauseRelease::Abnormal {
            for (session, owners) in &self.tickets {
                if owners.contains(&owner) {
                    self.suppressed.insert(session.clone());
                }
            }
            self.tickets.retain(|_, owners| !owners.contains(&owner));
        }
        if !self.owners.is_empty() {
            for owners in self.tickets.values_mut() {
                owners.remove(&owner);
            }
            return self.reconcile(backend);
        }

        self.suppressed.clear();
        let tickets = std::mem::take(&mut self.tickets);
        if release != (PauseRelease::Normal { selective_resume: true }) || tickets.is_empty() {
            return Ok(());
        }

        let mut first_error = None;
        for session in backend.sessions()? {
            if session.state == PlaybackState::Paused
                && tickets.contains_key(&session.key)
                && let Err(err) = backend.play_if_paused(&session.key)
            {
                first_error.get_or_insert(err);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

// ---- Live desktop AudioLink runtime ----

const DEFAULT_ENDPOINT_ID: &str = "@default";
const ROUTE_EXPIRY: Duration = Duration::from_secs(4);
const MIXER_SAMPLE_RATE: u32 = 48_000;
const MIXER_TICK_MS: u64 = 10;
const MIXER_TICK_FRAMES: usize = (MIXER_SAMPLE_RATE as usize) / 100; // 480 frames = 10 ms
const LEVELS_EVERY_TICKS: u32 = 10; // ~100 ms
const MANUAL_PLAY_SETTLE: Duration = Duration::from_millis(600);

#[derive(Debug, Default)]
struct PlatformAudioBackend;

impl AudioStateBackend for PlatformAudioBackend {
    fn speaker_muted(&mut self, _endpoint_id: &str) -> io::Result<bool> {
        crate::win::audio::speaker_state()
            .map(|(_, muted)| muted)
            .ok_or_else(|| io::Error::other("speaker mute state unavailable"))
    }

    fn set_speaker_muted(&mut self, _endpoint_id: &str, muted: bool) -> io::Result<()> {
        crate::win::audio::set_speaker_mute(muted).map_err(io::Error::other)
    }

    fn default_output(&mut self, _role: AudioRole) -> io::Result<Option<String>> {
        Ok(crate::win::audio::output_devices().into_iter().find(|d| d.is_default).map(|d| d.id))
    }

    fn set_default_output(&mut self, _role: AudioRole, endpoint_id: &str) -> io::Result<()> {
        crate::win::audio::set_default_output_device(endpoint_id).map_err(io::Error::other)
    }

    fn default_input(&mut self, _role: AudioRole) -> io::Result<Option<String>> {
        #[cfg(windows)]
        {
            Ok(crate::win::audio::input_devices().into_iter().find(|d| d.is_default).map(|d| d.id))
        }
        #[cfg(not(windows))]
        {
            Ok(None)
        }
    }

    fn set_default_input(&mut self, _role: AudioRole, _endpoint_id: &str) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Debug, Default)]
struct PlatformMediaBackend {
    incarnations: BTreeMap<String, u64>,
    present: BTreeSet<String>,
}

impl PlatformMediaBackend {
    fn key_for(&mut self, id: &str) -> MediaSessionKey {
        let incarnation = *self.incarnations.entry(id.to_owned()).or_insert(1);
        MediaSessionKey { id: id.to_owned(), incarnation }
    }

    fn current_key(&self, id: &str) -> Option<MediaSessionKey> {
        self.incarnations.get(id).map(|&incarnation| MediaSessionKey { id: id.to_owned(), incarnation })
    }
}

impl MediaPauseBackend for PlatformMediaBackend {
    fn sessions(&mut self) -> io::Result<Vec<MediaSession>> {
        let live = crate::win::media_sessions::list_sessions();
        let live_ids: BTreeSet<String> = live.iter().map(|(id, _, _)| id.clone()).collect();
        for gone in self.present.difference(&live_ids) {
            if let Some(inc) = self.incarnations.get_mut(gone) {
                *inc = inc.saturating_add(1);
            }
        }
        self.present = live_ids;
        let mut out = Vec::with_capacity(live.len());
        for (id, playing, paused) in live {
            let key = self.key_for(&id);
            let state = if playing {
                PlaybackState::Playing
            } else if paused {
                PlaybackState::Paused
            } else {
                PlaybackState::Other
            };
            out.push(MediaSession { key, state });
        }
        Ok(out)
    }

    fn pause_if_playing(&mut self, session: &MediaSessionKey) -> io::Result<bool> {
        if self.current_key(&session.id).as_ref() != Some(session) {
            return Ok(false);
        }
        Ok(crate::win::media_sessions::pause_session_if_playing(&session.id))
    }

    fn play_if_paused(&mut self, session: &MediaSessionKey) -> io::Result<bool> {
        if self.current_key(&session.id).as_ref() != Some(session) {
            return Ok(false);
        }
        Ok(crate::win::media_sessions::play_session_if_paused(&session.id))
    }
}

#[derive(Debug, Default)]
struct MediaPauseState {
    coordinator: MediaPauseCoordinator,
    backend: PlatformMediaBackend,
    last_pause_at: Option<Instant>,
    route_owner: Option<PauseOwner>,
}

static MEDIA_PAUSE: OnceLock<Mutex<MediaPauseState>> = OnceLock::new();

fn media_pause() -> &'static Mutex<MediaPauseState> {
    MEDIA_PAUSE.get_or_init(|| Mutex::new(MediaPauseState::default()))
}

/// Pauses or selectively resumes desktop media for an ongoing phone call,
/// coordinated with any active AudioLink pause policy.
pub fn set_call_media_paused(paused: bool) {
    let name = if paused { "call-media-pause" } else { "call-media-resume" };
    let _ = thread::Builder::new().name(name.into()).spawn(move || {
        let mut state = media_pause().lock().unwrap_or_else(|e| e.into_inner());
        let MediaPauseState { coordinator, backend, last_pause_at, .. } = &mut *state;
        if paused {
            *last_pause_at = Some(Instant::now());
        }
        if let Err(err) = coordinator.set_pause(PauseOwner::PhoneCall, paused, true, backend) {
            tracing::debug!(error = %err, paused, "call media pause update failed");
        }
    });
}

/// Records that the user manually resumed a desktop media session so neither
/// calls nor AudioLink will auto-resume it later.
pub fn note_media_session_playing(id: &str) {
    let Ok(mut state) = media_pause().try_lock() else { return };
    if state.last_pause_at.is_some_and(|at| at.elapsed() < MANUAL_PLAY_SETTLE) {
        return;
    }
    if let Some(key) = state.backend.current_key(id) {
        state.coordinator.invalidate_session(&key);
    }
}

fn reconcile_route_media_pause(generation: u64, pause: bool, selective_resume: bool) {
    let _ = thread::Builder::new().name("route-media-pause".into()).spawn(move || {
        let mut state = media_pause().lock().unwrap_or_else(|e| e.into_inner());
        let owner = PauseOwner::AudioRoute { generation };
        let MediaPauseState { coordinator, backend, last_pause_at, route_owner } = &mut *state;
        if let Some(prev) = *route_owner
            && prev != owner
        {
            let _ = coordinator.release(prev, PauseRelease::Normal { selective_resume: false }, backend);
        }
        if pause {
            *last_pause_at = Some(Instant::now());
            *route_owner = Some(owner);
        } else if *route_owner == Some(owner) {
            *route_owner = None;
        }
        if let Err(err) = coordinator.set_pause(owner, pause, selective_resume, backend) {
            tracing::debug!(error = %err, generation, pause, "audio route media pause update failed");
        }
    });
}

fn release_route_media_pause(release: PauseRelease) {
    let _ = thread::Builder::new().name("route-media-release".into()).spawn(move || {
        let mut state = media_pause().lock().unwrap_or_else(|e| e.into_inner());
        let MediaPauseState { coordinator, backend, route_owner, .. } = &mut *state;
        if let Some(owner) = route_owner.take()
            && let Err(err) = coordinator.release(owner, release, backend)
        {
            tracing::debug!(error = %err, "audio route media release failed");
        }
    });
}

fn map_latency(preset: AudioLatencyPreset) -> LatencyPreset {
    match preset {
        AudioLatencyPreset::Lowest => LatencyPreset::Lowest,
        AudioLatencyPreset::Balanced => LatencyPreset::Balanced,
        AudioLatencyPreset::Stable => LatencyPreset::Stable,
    }
}

fn map_policy(policy: AudioPlaybackPolicy, phone_playback_active: bool) -> PlaybackPolicy {
    if !phone_playback_active {
        return PlaybackPolicy::Off;
    }
    match policy {
        AudioPlaybackPolicy::Pause => PlaybackPolicy::Pause,
        AudioPlaybackPolicy::Mute => PlaybackPolicy::Mute,
        AudioPlaybackPolicy::Lower => PlaybackPolicy::Lower,
        AudioPlaybackPolicy::Off => PlaybackPolicy::Off,
    }
}

#[derive(Debug)]
struct SinkWorker {
    mixer: Arc<Mutex<LiveAudioMixer<String>>>,
    lane_formats: Arc<Mutex<BTreeMap<String, AudioFormat>>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    output_id: Option<String>,
    generation: u64,
    controller: DeviceId,
}

impl SinkWorker {
    fn start(controller: DeviceId, set: &AudioRouteSet) -> Result<Self, String> {
        let mut mixer = LiveAudioMixer::new(map_latency(set.latency_preset), SinkMode::Program);
        let mut formats = BTreeMap::new();
        apply_set_to_mixer(&mut mixer, &mut formats, set);

        let mixer = Arc::new(Mutex::new(mixer));
        let lane_formats = Arc::new(Mutex::new(formats));
        let stop = Arc::new(AtomicBool::new(false));
        let output_id = set.output_id.clone();
        let generation = set.generation;

        let thread_mixer = Arc::clone(&mixer);
        let thread_formats = Arc::clone(&lane_formats);
        let thread_stop = Arc::clone(&stop);
        let thread_output = output_id.clone();

        let handle = thread::Builder::new()
            .name("audio-route-sink".into())
            .spawn(move || {
                let mut player: Option<crate::win::audio_out::Player> = None;
                let mut frames = vec![[0i16; 2]; MIXER_TICK_FRAMES];
                let tick = Duration::from_millis(MIXER_TICK_MS);
                let mut next_tick = Instant::now() + tick;
                let mut level_ticks: u32 = 0;

                while !thread_stop.load(Ordering::Acquire) {
                    let now = Instant::now();
                    let rendered = {
                        let mut m = thread_mixer.lock().unwrap_or_else(|e| e.into_inner());
                        m.render(MIXER_SAMPLE_RATE, &mut frames, now)
                    };
                    if rendered {
                        if player.is_none() {
                            match crate::win::audio_out::Player::start_on(
                                thread_output.clone(),
                                MIXER_SAMPLE_RATE,
                                2,
                            ) {
                                Ok(p) => player = Some(p),
                                Err(err) => {
                                    tracing::warn!(error = %err, "can't start audio route player");
                                }
                            }
                        }
                        if let Some(p) = player.as_ref() {
                            let mut pcm = Vec::with_capacity(MIXER_TICK_FRAMES * 4);
                            for [l, r] in &frames {
                                pcm.extend_from_slice(&l.to_le_bytes());
                                pcm.extend_from_slice(&r.to_le_bytes());
                            }
                            p.push(pcm);
                        }
                    }

                    level_ticks += 1;
                    if level_ticks >= LEVELS_EVERY_TICKS {
                        level_ticks = 0;
                        if let Some(node) = crate::core_host::node() {
                            let ids: Vec<String> = thread_formats
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .keys()
                                .cloned()
                                .collect();
                            let levels: Vec<AudioLaneLevel> = {
                                let m = thread_mixer.lock().unwrap_or_else(|e| e.into_inner());
                                ids.into_iter()
                                    .filter_map(|source_id| {
                                        let st = m.lane_status(&source_id)?;
                                        Some(AudioLaneLevel {
                                            source_id,
                                            peak: st.peak.clamp(0.0, 1.0),
                                            active: st.playing,
                                        })
                                    })
                                    .collect()
                            };
                            if !levels.is_empty() {
                                let _ = node.send_audio_route_levels(
                                    controller,
                                    AudioRouteLevels { generation, levels },
                                );
                            }
                        }
                    }

                    let after = Instant::now();
                    if next_tick > after {
                        thread::sleep(next_tick - after);
                    } else if after.duration_since(next_tick) > Duration::from_millis(50) {
                        next_tick = after;
                    }
                    next_tick += tick;
                }
            })
            .map_err(|e| format!("failed to spawn audio route sink worker: {e}"))?;

        Ok(Self { mixer, lane_formats, stop, handle: Some(handle), output_id, generation, controller })
    }

    fn update(&self, set: &AudioRouteSet) {
        let mut mixer = self.mixer.lock().unwrap_or_else(|e| e.into_inner());
        let mut formats = self.lane_formats.lock().unwrap_or_else(|e| e.into_inner());
        apply_set_to_mixer(&mut mixer, &mut formats, set);
    }

    fn push_pcm(&self, pcm: &AudioRoutePcm) {
        if pcm.generation != self.generation {
            return;
        }
        let incoming_fmt = AudioFormat { sample_rate: pcm.sample_rate, channels: pcm.channels };
        let mut formats = self.lane_formats.lock().unwrap_or_else(|e| e.into_inner());
        let Some(current_fmt) = formats.get_mut(&pcm.source_id) else {
            return;
        };
        let mut mixer = self.mixer.lock().unwrap_or_else(|e| e.into_inner());
        if *current_fmt != incoming_fmt {
            if mixer.authorize_lane(pcm.source_id.clone(), pcm.generation, incoming_fmt) {
                *current_fmt = incoming_fmt;
            } else {
                return;
            }
        }
        let _ = mixer.push_pcm_le(&pcm.source_id, pcm.generation, Some(pcm.seq), &pcm.pcm, Instant::now());
    }
}

impl Drop for SinkWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn apply_set_to_mixer(
    mixer: &mut LiveAudioMixer<String>,
    formats: &mut BTreeMap<String, AudioFormat>,
    set: &AudioRouteSet,
) {
    mixer.set_latency(map_latency(set.latency_preset));
    mixer.set_master_gain(set.master_gain);
    mixer.set_master_muted(set.master_muted);
    mixer.set_policy(map_policy(set.playback_policy, set.phone_playback_active));

    let wanted: BTreeSet<&str> = set.lanes.iter().map(|l| l.source_id.as_str()).collect();
    let stale: Vec<String> = formats.keys().filter(|id| !wanted.contains(id.as_str())).cloned().collect();
    for id in stale {
        formats.remove(&id);
        mixer.remove_lane(&id);
    }
    for lane in &set.lanes {
        let fmt = formats
            .entry(lane.source_id.clone())
            .or_insert(AudioFormat { sample_rate: MIXER_SAMPLE_RATE, channels: 2 });
        mixer.authorize_lane(lane.source_id.clone(), set.generation, *fmt);
        mixer.set_lane_gain(&lane.source_id, lane.gain);
        mixer.set_lane_muted(&lane.source_id, lane.muted);
        mixer.set_lane_active(&lane.source_id, true);
    }
}

#[derive(Debug, Default)]
struct AudioRouteRuntime {
    journal: Option<AudioStateJournal>,
    controller: Option<DeviceId>,
    generation: u64,
    role: Option<AudioRouteRole>,
    last_renewed: Option<Instant>,
    local_paused: bool,
    speaker_muted_by_route: bool,
    output_id: Option<String>,
    active_set: Option<AudioRouteSet>,
    sink: Option<SinkWorker>,
    watchdog_running: bool,
}

static RUNTIME: OnceLock<Mutex<AudioRouteRuntime>> = OnceLock::new();

fn runtime() -> &'static Mutex<AudioRouteRuntime> {
    RUNTIME.get_or_init(|| Mutex::new(AudioRouteRuntime::default()))
}

/// Opens the audio state journal and recovers any speaker mute or default
/// output changes left behind by a previous crash.
pub fn init_and_recover(data_dir: &Path) {
    let mut rt = runtime().lock().unwrap_or_else(|e| e.into_inner());
    match AudioStateJournal::open(data_dir) {
        Ok(mut journal) => {
            if !journal.records().is_empty() {
                let mut backend = PlatformAudioBackend;
                match journal.recover(&mut backend) {
                    Ok(report) => {
                        tracing::info!(
                            restored = report.restored,
                            superseded = report.superseded,
                            pending = report.pending,
                            "recovered audio state journal on startup"
                        );
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "failed to recover audio state journal on startup");
                    }
                }
            }
            rt.journal = Some(journal);
        }
        Err(err) => {
            tracing::warn!(error = %err, "can't open audio state journal");
        }
    }
}

/// Stops any active managed audio route, releases media pause ownership, and
/// restores owned speaker mute / default output state.
pub fn shutdown() {
    crate::pc_audio_stream::set_managed_target(None);
    let mut rt = runtime().lock().unwrap_or_else(|e| e.into_inner());
    rt.sink = None;
    rt.role = Some(AudioRouteRole::Idle);
    rt.controller = None;
    rt.last_renewed = None;
    rt.active_set = None;
    rt.local_paused = false;
    rt.speaker_muted_by_route = false;
    rt.output_id = None;
    if let Some(node) = crate::core_host::node() {
        let _ = node.set_local_audio_route(None);
    }
    if let Some(journal) = rt.journal.as_mut() {
        let mut backend = PlatformAudioBackend;
        if let Err(err) = journal.recover(&mut backend) {
            tracing::warn!(error = %err, "failed to recover audio state journal on shutdown");
        }
    }
    drop(rt);
    let mut mp = media_pause().lock().unwrap_or_else(|e| e.into_inner());
    let MediaPauseState { coordinator, backend, route_owner, .. } = &mut *mp;
    if let Some(owner) = route_owner.take() {
        let _ = coordinator.release(owner, PauseRelease::Abnormal, backend);
    }
    let _ =
        coordinator.release(PauseOwner::PhoneCall, PauseRelease::Normal { selective_resume: false }, backend);
}

fn ensure_watchdog(rt: &mut AudioRouteRuntime) {
    if rt.watchdog_running {
        return;
    }
    rt.watchdog_running = true;
    crate::core_host::spawn(async {
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let expired_gen = {
                let mut rt = runtime().lock().unwrap_or_else(|e| e.into_inner());
                let active = !matches!(rt.role, None | Some(AudioRouteRole::Idle));
                if !active {
                    rt.watchdog_running = false;
                    return;
                }
                if rt.last_renewed.is_some_and(|at| at.elapsed() > ROUTE_EXPIRY) {
                    Some(rt.generation)
                } else {
                    None
                }
            };
            if let Some(expired) = expired_gen {
                tracing::info!(generation = expired, "audio route lease expired; returning to idle");
                stop_route_internal(PauseRelease::Abnormal);
                return;
            }
        }
    });
}

fn stop_route_internal(release: PauseRelease) {
    crate::pc_audio_stream::set_managed_target(None);
    let mut rt = runtime().lock().unwrap_or_else(|e| e.into_inner());
    rt.sink = None;
    rt.role = Some(AudioRouteRole::Idle);
    rt.controller = None;
    rt.last_renewed = None;
    rt.active_set = None;
    rt.local_paused = false;
    rt.speaker_muted_by_route = false;
    rt.output_id = None;
    rt.watchdog_running = false;
    if let Some(node) = crate::core_host::node() {
        let _ = node.set_local_audio_route(None);
    }
    if let Some(journal) = rt.journal.as_mut() {
        let mut backend = PlatformAudioBackend;
        if let Err(err) = journal.recover(&mut backend) {
            tracing::warn!(error = %err, "failed to restore audio state after route stop");
        }
    }
    drop(rt);
    release_route_media_pause(release);
}

fn apply_speaker_silence(rt: &mut AudioRouteRuntime, silence: bool) {
    let mut backend = PlatformAudioBackend;
    if silence {
        if rt.speaker_muted_by_route {
            return;
        }
        match backend.speaker_muted(DEFAULT_ENDPOINT_ID) {
            Ok(false) => {
                let record = AudioStateRecord::SpeakerMute(SpeakerMuteRecord {
                    endpoint_id: DEFAULT_ENDPOINT_ID.to_owned(),
                    was_muted: false,
                    owned_mute: true,
                });
                let recorded = rt.journal.as_mut().map_or(Ok(()), |j| j.record(record));
                if recorded.is_ok() && backend.set_speaker_muted(DEFAULT_ENDPOINT_ID, true).is_ok() {
                    rt.speaker_muted_by_route = true;
                }
            }
            Ok(true) => {
                // Already muted by the user: leave it alone and do not claim ownership.
                rt.speaker_muted_by_route = false;
            }
            Err(err) => {
                tracing::debug!(error = %err, "could not read speaker mute before silencing");
            }
        }
    } else if rt.speaker_muted_by_route {
        if let Some(journal) = rt.journal.as_mut() {
            let _ = journal.recover(&mut backend);
        } else {
            let _ = backend.set_speaker_muted(DEFAULT_ENDPOINT_ID, false);
        }
        rt.speaker_muted_by_route = false;
    }
}

fn apply_default_output(rt: &mut AudioRouteRuntime, requested: Option<&str>) {
    let Some(target) = requested.map(str::trim).filter(|s| !s.is_empty()) else {
        return;
    };
    let mut backend = PlatformAudioBackend;
    let current = backend.default_output(AudioRole::Default).ok().flatten();
    if current.as_deref() == Some(target) {
        return;
    }
    if let Some(prev) = current {
        let record = AudioStateRecord::DefaultOutput(DefaultOutputRecord {
            role: AudioRole::Default,
            previous_endpoint_id: prev,
            owned_endpoint_id: target.to_owned(),
        });
        if let Some(journal) = rt.journal.as_mut()
            && let Err(err) = journal.record(record)
        {
            tracing::debug!(error = %err, "could not record default output switch in journal");
            return;
        }
    }
    if let Err(err) = backend.set_default_output(AudioRole::Default, target) {
        tracing::debug!(error = %err, "could not switch default output for audio route");
    }
}

/// Applies an `audio.route.set` request from `peer` and returns the resulting status.
pub fn handle_set(peer: DeviceId, set: AudioRouteSet) -> Result<AudioRouteStatus, String> {
    if !set.is_valid() {
        return Err("invalid audio route configuration".to_owned());
    }
    let node = crate::core_host::node().ok_or_else(|| "core node not running".to_owned())?;

    let mut rt = runtime().lock().unwrap_or_else(|e| e.into_inner());
    if set.generation < rt.generation {
        return Err("stale audio route generation".to_owned());
    }
    let gen_changed = set.generation > rt.generation;
    rt.generation = set.generation;
    if gen_changed {
        rt.local_paused = false;
    }

    match set.role {
        AudioRouteRole::Idle => {
            let selective_resume = set.selective_resume;
            drop(rt);
            stop_route_internal(PauseRelease::Normal { selective_resume });
            Ok(AudioRouteStatus {
                generation: set.generation,
                role: AudioRouteRole::Idle,
                active: false,
                local_paused: false,
                speaker_muted_by_route: false,
                output_id: None,
                reason: None,
            })
        }
        AudioRouteRole::Source => {
            rt.sink = None;
            let _ = node.set_local_audio_route(None);
            release_route_media_pause(PauseRelease::Normal { selective_resume: set.selective_resume });

            let target_peer =
                set.target_device_id.as_deref().and_then(|s| s.parse::<DeviceId>().ok()).unwrap_or(peer);
            let source_id = node.device_id().to_string();
            rt.controller = Some(peer);
            rt.role = Some(AudioRouteRole::Source);
            rt.last_renewed = Some(Instant::now());
            rt.output_id = None;
            rt.active_set = Some(set.clone());

            if rt.local_paused {
                crate::pc_audio_stream::set_managed_target(None);
                apply_speaker_silence(&mut rt, false);
            } else {
                crate::pc_audio_stream::set_managed_target(Some(
                    crate::pc_audio_stream::ManagedRouteTarget {
                        peer: target_peer,
                        generation: set.generation,
                        source_id,
                    },
                ));
                apply_speaker_silence(&mut rt, set.silence_local_speaker);
            }
            ensure_watchdog(&mut rt);

            Ok(AudioRouteStatus {
                generation: set.generation,
                role: AudioRouteRole::Source,
                active: !rt.local_paused,
                local_paused: rt.local_paused,
                speaker_muted_by_route: rt.speaker_muted_by_route,
                output_id: None,
                reason: None,
            })
        }
        AudioRouteRole::Sink | AudioRouteRole::PhonePlayback => {
            crate::pc_audio_stream::set_managed_target(None);
            apply_speaker_silence(&mut rt, false);
            apply_default_output(&mut rt, set.output_id.as_deref());
            node.set_local_audio_route(Some(set.clone())).map_err(|e| e.to_string())?;

            let needs_new_sink = rt.sink.as_ref().is_none_or(|s| {
                s.generation != set.generation || s.output_id != set.output_id || s.controller != peer
            });
            if needs_new_sink {
                rt.sink = None;
                rt.sink = Some(SinkWorker::start(peer, &set)?);
            } else if let Some(sink) = rt.sink.as_ref() {
                sink.update(&set);
            }

            rt.controller = Some(peer);
            rt.role = Some(set.role);
            rt.last_renewed = Some(Instant::now());
            rt.output_id = set.output_id.clone();
            rt.active_set = Some(set.clone());
            ensure_watchdog(&mut rt);

            let should_pause = set.phone_playback_active && set.playback_policy == AudioPlaybackPolicy::Pause;
            reconcile_route_media_pause(set.generation, should_pause, set.selective_resume);

            Ok(AudioRouteStatus {
                generation: set.generation,
                role: set.role,
                active: true,
                local_paused: false,
                speaker_muted_by_route: false,
                output_id: set.output_id,
                reason: None,
            })
        }
    }
}

/// Feeds an incoming managed PCM frame into the active sink mixer.
pub fn handle_pcm(_peer: DeviceId, pcm: AudioRoutePcm) {
    let rt = runtime().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(sink) = rt.sink.as_ref() {
        sink.push_pcm(&pcm);
    }
}

/// Handles incoming `audio.route.levels` notifications (no-op on desktop peers).
pub fn handle_levels(_peer: DeviceId, _levels: AudioRouteLevels) {}

/// Locally pauses or resumes an active desktop audio source and notifies the controller.
#[allow(dead_code)]
pub fn set_local_paused(paused: bool) {
    let notify = {
        let mut rt = runtime().lock().unwrap_or_else(|e| e.into_inner());
        if rt.role != Some(AudioRouteRole::Source) {
            return;
        }
        rt.local_paused = paused;
        if paused {
            crate::pc_audio_stream::set_managed_target(None);
            apply_speaker_silence(&mut rt, false);
        } else if let (Some(set), Some(controller), Some(node)) =
            (rt.active_set.clone(), rt.controller, crate::core_host::node())
        {
            let target_peer = set
                .target_device_id
                .as_deref()
                .and_then(|s| s.parse::<DeviceId>().ok())
                .unwrap_or(controller);
            crate::pc_audio_stream::set_managed_target(Some(crate::pc_audio_stream::ManagedRouteTarget {
                peer: target_peer,
                generation: set.generation,
                source_id: node.device_id().to_string(),
            }));
            apply_speaker_silence(&mut rt, set.silence_local_speaker);
        }
        rt.controller.map(|controller| {
            (
                controller,
                AudioRouteStatus {
                    generation: rt.generation,
                    role: AudioRouteRole::Source,
                    active: !rt.local_paused,
                    local_paused: rt.local_paused,
                    speaker_muted_by_route: rt.speaker_muted_by_route,
                    output_id: None,
                    reason: None,
                },
            )
        })
    };
    if let Some((controller, status)) = notify
        && let Some(node) = crate::core_host::node()
    {
        crate::core_host::spawn(async move {
            let _ = node.send_audio_route_status(controller, status).await;
        });
    }
}

/// Cleans up an active managed audio route if its controller goes offline or is removed.
pub fn on_event(event: &NodeEvent) {
    let affected = match event {
        NodeEvent::LinkChanged { device, link: LinkState::Offline { .. } }
        | NodeEvent::DeviceRemoved(device) => Some(*device),
        _ => None,
    };
    let Some(device) = affected else { return };
    let is_controller = {
        let rt = runtime().lock().unwrap_or_else(|e| e.into_inner());
        rt.controller == Some(device) && !matches!(rt.role, None | Some(AudioRouteRole::Idle))
    };
    if is_controller {
        tracing::info!(peer = %device.short(), "audio route controller went offline; stopping route");
        stop_route_internal(PauseRelease::Abnormal);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROLE: AudioRole = AudioRole::Communications;
    const ROUTE: PauseOwner = PauseOwner::AudioRoute { generation: 7 };
    const NORMAL: PauseRelease = PauseRelease::Normal { selective_resume: true };

    #[derive(Debug, Default)]
    struct Audio {
        muted: bool,
        output: Option<String>,
        input: Option<String>,
        input_restore_fails: bool,
    }

    impl AudioStateBackend for Audio {
        fn speaker_muted(&mut self, endpoint_id: &str) -> io::Result<bool> {
            assert_eq!(endpoint_id, "speakers");
            Ok(self.muted)
        }

        fn set_speaker_muted(&mut self, endpoint_id: &str, muted: bool) -> io::Result<()> {
            assert_eq!(endpoint_id, "speakers");
            self.muted = muted;
            Ok(())
        }

        fn default_output(&mut self, role: AudioRole) -> io::Result<Option<String>> {
            assert_eq!(role, ROLE);
            Ok(self.output.clone())
        }

        fn set_default_output(&mut self, role: AudioRole, endpoint_id: &str) -> io::Result<()> {
            assert_eq!(role, ROLE);
            self.output = Some(endpoint_id.into());
            Ok(())
        }

        fn default_input(&mut self, role: AudioRole) -> io::Result<Option<String>> {
            assert_eq!(role, ROLE);
            Ok(self.input.clone())
        }

        fn set_default_input(&mut self, role: AudioRole, endpoint_id: &str) -> io::Result<()> {
            assert_eq!(role, ROLE);
            if self.input_restore_fails {
                return Err(io::Error::other("audio service unavailable"));
            }
            self.input = Some(endpoint_id.into());
            Ok(())
        }
    }

    fn output_change(previous: &str, owned: &str) -> AudioStateRecord {
        AudioStateRecord::DefaultOutput(DefaultOutputRecord {
            role: ROLE,
            previous_endpoint_id: previous.into(),
            owned_endpoint_id: owned.into(),
        })
    }

    fn mute() -> AudioStateRecord {
        AudioStateRecord::SpeakerMute(SpeakerMuteRecord {
            endpoint_id: "speakers".into(),
            was_muted: false,
            owned_mute: true,
        })
    }

    #[test]
    fn recovers_after_restart_and_persists_only_failed_restores_for_retry() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = AudioStateJournal::open(dir.path()).unwrap();
        let mut audio =
            Audio { output: Some("speakers".into()), input: Some("desk mic".into()), ..Audio::default() };
        journal.record(mute()).unwrap();
        audio.set_speaker_muted("speakers", true).unwrap();
        journal.record(output_change("speakers", "virtual speaker")).unwrap();
        audio.set_default_output(ROLE, "virtual speaker").unwrap();

        let input = AudioStateRecord::DefaultInput(DefaultInputRecord {
            role: ROLE,
            previous_endpoint_id: "desk mic".into(),
            owned_endpoint_id: "virtual mic".into(),
        });
        journal.record(input.clone()).unwrap();
        audio.set_default_input(ROLE, "virtual mic").unwrap();
        audio.input_restore_fails = true;
        drop(journal);

        let mut journal = AudioStateJournal::open(dir.path()).unwrap();
        assert_eq!(journal.records().len(), 3);
        let report = journal.recover(&mut audio).unwrap();
        assert_eq!((report.restored, report.superseded, report.pending, report.failures.len()), (2, 0, 1, 1));
        assert!(!audio.muted);
        assert_eq!(audio.output.as_deref(), Some("speakers"));
        assert_eq!(audio.input.as_deref(), Some("virtual mic"));
        assert_eq!(journal.records(), std::slice::from_ref(&input));

        let mut reopened = AudioStateJournal::open(dir.path()).unwrap();
        assert_eq!(reopened.records(), &[input]);
        audio.input_restore_fails = false;
        assert_eq!(reopened.recover(&mut audio).unwrap().restored, 1);
        assert_eq!(audio.input.as_deref(), Some("desk mic"));
        assert!(AudioStateJournal::open(dir.path()).unwrap().records().is_empty());
    }

    #[test]
    fn user_override_supersedes_even_an_older_owned_value() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = AudioStateJournal::open(dir.path()).unwrap();
        journal.record(mute()).unwrap();
        journal.record(output_change("speakers", "first route")).unwrap();
        journal.record(output_change("first route", "second route")).unwrap();

        let mut audio = Audio { output: Some("first route".into()), ..Audio::default() };
        let report = journal.recover(&mut audio).unwrap();
        assert_eq!((report.restored, report.superseded, report.pending), (0, 2, 0));
        assert_eq!(audio.output.as_deref(), Some("first route"));
        assert!(!audio.muted);
        assert!(AudioStateJournal::open(dir.path()).unwrap().records().is_empty());
    }

    #[test]
    fn unwinds_successive_output_changes_and_ignores_unapplied_mute() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = AudioStateJournal::open(dir.path()).unwrap();
        journal.record(output_change("speakers", "first route")).unwrap();
        journal.record(output_change("first route", "second route")).unwrap();
        journal.record(mute()).unwrap();
        let mut audio = Audio { output: Some("second route".into()), ..Audio::default() };

        let report = journal.recover(&mut audio).unwrap();
        assert_eq!((report.restored, report.superseded, report.pending), (1, 1, 0));
        assert_eq!(audio.output.as_deref(), Some("speakers"));
    }

    #[test]
    fn new_route_restores_the_users_selection_if_it_replaced_our_previous_route() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = AudioStateJournal::open(dir.path()).unwrap();
        journal.record(output_change("speakers", "first route")).unwrap();
        journal.record(output_change("headphones", "second route")).unwrap();
        let mut audio = Audio { output: Some("second route".into()), ..Audio::default() };

        assert_eq!(journal.recover(&mut audio).unwrap().restored, 1);
        assert_eq!(audio.output.as_deref(), Some("headphones"));
    }

    #[test]
    fn failed_journal_write_leaves_in_memory_records_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let blocked = dir.path().join("app-data");
        let mut journal = AudioStateJournal::open(&blocked).unwrap();
        fs::write(&blocked, []).unwrap();
        assert!(journal.record(mute()).is_err());
        assert!(journal.records().is_empty());
    }

    #[derive(Debug, Default)]
    struct Media {
        states: BTreeMap<MediaSessionKey, PlaybackState>,
        cannot_pause: BTreeSet<MediaSessionKey>,
        list_fails: bool,
        resumes: Vec<MediaSessionKey>,
    }

    fn key(id: &str, incarnation: u64) -> MediaSessionKey {
        MediaSessionKey { id: id.into(), incarnation }
    }

    impl MediaPauseBackend for Media {
        fn sessions(&mut self) -> io::Result<Vec<MediaSession>> {
            if self.list_fails {
                return Err(io::Error::other("session listing unavailable"));
            }
            Ok(self
                .states
                .iter()
                .map(|(key, state)| MediaSession { key: key.clone(), state: *state })
                .collect())
        }

        fn pause_if_playing(&mut self, session: &MediaSessionKey) -> io::Result<bool> {
            if self.states.get(session) == Some(&PlaybackState::Playing)
                && !self.cannot_pause.contains(session)
            {
                self.states.insert(session.clone(), PlaybackState::Paused);
                return Ok(true);
            }
            Ok(false)
        }

        fn play_if_paused(&mut self, session: &MediaSessionKey) -> io::Result<bool> {
            if self.states.get(session) == Some(&PlaybackState::Paused) {
                self.states.insert(session.clone(), PlaybackState::Playing);
                self.resumes.push(session.clone());
                return Ok(true);
            }
            Ok(false)
        }
    }

    #[test]
    fn last_of_overlapping_owners_resumes_only_sessions_we_actually_paused() {
        let player = key("player", 1);
        let already_paused = key("podcast", 1);
        let failed = key("browser", 1);
        for (first, last) in [(PauseOwner::PhoneCall, ROUTE), (ROUTE, PauseOwner::PhoneCall)] {
            let mut coordinator = MediaPauseCoordinator::default();
            let mut media = Media {
                states: BTreeMap::from([
                    (player.clone(), PlaybackState::Playing),
                    (already_paused.clone(), PlaybackState::Paused),
                    (failed.clone(), PlaybackState::Playing),
                ]),
                cannot_pause: BTreeSet::from([failed.clone()]),
                ..Media::default()
            };
            coordinator.set_pause(first, true, true, &mut media).unwrap();
            coordinator.set_pause(last, true, true, &mut media).unwrap();
            assert_eq!(coordinator.paused_sessions(first).collect::<Vec<_>>(), [&player]);
            assert_eq!(coordinator.paused_sessions(last).collect::<Vec<_>>(), [&player]);
            coordinator.release(first, NORMAL, &mut media).unwrap();
            assert_eq!(coordinator.paused_sessions(first).count(), 0);
            assert_eq!(media.states[&player], PlaybackState::Paused);
            assert!(media.resumes.is_empty());
            coordinator.release(last, NORMAL, &mut media).unwrap();
            assert_eq!(media.resumes, std::slice::from_ref(&player));
            assert_eq!(media.states[&already_paused], PlaybackState::Paused);
            assert_eq!(media.states[&failed], PlaybackState::Playing);
        }
    }

    #[test]
    fn policy_changes_and_new_playback_reconcile_without_route_edge() {
        let player = key("player", 1);
        let newcomer = key("new", 1);
        let mut media =
            Media { states: BTreeMap::from([(player.clone(), PlaybackState::Playing)]), ..Media::default() };
        let mut coordinator = MediaPauseCoordinator::default();
        coordinator.set_pause(ROUTE, false, true, &mut media).unwrap();
        assert_eq!(media.states[&player], PlaybackState::Playing);
        coordinator.set_pause(ROUTE, true, true, &mut media).unwrap();
        media.states.insert(newcomer.clone(), PlaybackState::Playing);
        coordinator.reconcile(&mut media).unwrap();
        assert_eq!(media.states[&player], PlaybackState::Paused);
        assert_eq!(media.states[&newcomer], PlaybackState::Paused);
        coordinator.set_pause(ROUTE, false, false, &mut media).unwrap();
        assert!(media.resumes.is_empty());
        assert_eq!(coordinator.paused_sessions(ROUTE).count(), 0);
    }

    #[test]
    fn user_playback_and_session_replacement_revoke_resume_tickets() {
        let manual = key("player", 1);
        let original = key("browser", 1);
        let replacement = key("browser", 2);
        let mut media = Media {
            states: BTreeMap::from([
                (manual.clone(), PlaybackState::Playing),
                (original.clone(), PlaybackState::Playing),
            ]),
            ..Media::default()
        };
        let mut coordinator = MediaPauseCoordinator::default();
        coordinator.set_pause(ROUTE, true, true, &mut media).unwrap();
        media.states.insert(manual.clone(), PlaybackState::Playing);
        media.states.remove(&original);
        media.states.insert(replacement.clone(), PlaybackState::Paused);
        coordinator.reconcile(&mut media).unwrap();
        assert_eq!(media.states[&manual], PlaybackState::Playing);
        assert_eq!(coordinator.paused_sessions(ROUTE).count(), 0);

        media.states.insert(manual.clone(), PlaybackState::Paused);
        coordinator.release(ROUTE, NORMAL, &mut media).unwrap();
        assert!(media.resumes.is_empty());
        assert_eq!(media.states[&manual], PlaybackState::Paused);
        assert_eq!(media.states[&replacement], PlaybackState::Paused);
    }

    #[test]
    fn observed_manual_playback_revokes_a_ticket_between_snapshots() {
        let player = key("player", 1);
        let mut media =
            Media { states: BTreeMap::from([(player.clone(), PlaybackState::Playing)]), ..Media::default() };
        let mut coordinator = MediaPauseCoordinator::default();
        coordinator.set_pause(ROUTE, true, true, &mut media).unwrap();
        coordinator.invalidate_session(&player);
        coordinator.release(ROUTE, NORMAL, &mut media).unwrap();
        assert!(media.resumes.is_empty());
        assert_eq!(media.states[&player], PlaybackState::Paused);
    }

    #[test]
    fn abnormal_expiry_discards_tickets_even_if_call_still_owns_pause() {
        let player = key("player", 1);
        let mut media =
            Media { states: BTreeMap::from([(player.clone(), PlaybackState::Playing)]), ..Media::default() };
        let mut coordinator = MediaPauseCoordinator::default();
        coordinator.set_pause(PauseOwner::PhoneCall, true, true, &mut media).unwrap();
        media.list_fails = true;
        assert!(coordinator.set_pause(ROUTE, true, true, &mut media).is_err());
        media.list_fails = false;
        coordinator.release(ROUTE, PauseRelease::Abnormal, &mut media).unwrap();
        assert_eq!(coordinator.paused_sessions(PauseOwner::PhoneCall).count(), 0);
        coordinator.release(PauseOwner::PhoneCall, NORMAL, &mut media).unwrap();
        assert!(media.resumes.is_empty());
        assert_eq!(media.states[&player], PlaybackState::Paused);
    }

    #[test]
    fn abnormal_expiry_as_sole_owner_does_not_transfer_tickets_to_next_generation() {
        let player = key("player", 1);
        let mut media =
            Media { states: BTreeMap::from([(player.clone(), PlaybackState::Playing)]), ..Media::default() };
        let mut coordinator = MediaPauseCoordinator::default();
        coordinator.set_pause(ROUTE, true, true, &mut media).unwrap();
        coordinator.release(ROUTE, PauseRelease::Abnormal, &mut media).unwrap();
        let next = PauseOwner::AudioRoute { generation: 8 };
        coordinator.set_pause(next, true, true, &mut media).unwrap();
        coordinator.release(next, NORMAL, &mut media).unwrap();
        assert!(media.resumes.is_empty());
        assert_eq!(media.states[&player], PlaybackState::Paused);
    }
}
