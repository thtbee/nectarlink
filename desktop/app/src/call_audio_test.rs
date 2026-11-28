// SPDX-License-Identifier: GPL-3.0-or-later
//! "Test call audio" (Settings › Calls through this PC): runs the sound path
//! of a call on this PC with no call, and shows what's going on.
//!
//! With a phone that offers `call.audio.test` (docs/protocol/calls.md §2.6) it
//! is a loopback: the PC's microphone goes to the phone, which plays it on its
//! speaker and sends it back, and the PC plays what returns on its call
//! output. Without one (an older app, or the phone's offline) it records 5 s
//! and plays that back, and says so. Either way the microphone is recorded
//! by the code a real call uses ([`crate::call_audio::recorder_with`]).
//!
//! The levels, the counters and the stages are platform neutral; only the
//! recorder and the player differ per system, and they already exist. Windows
//! and macOS; Linux carries calls over Bluetooth, so there's nothing to test.

use std::{
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use nectarlink_core::DeviceId;
use serde_json::json;

use crate::{core_host, i18n::tr};

/// How much a local test records.
const LOCAL_RECORD: Duration = Duration::from_secs(5);
/// A loopback test ends by itself after this long.
const LOOPBACK_MAX: Duration = Duration::from_secs(60);
/// Sound that comes back this long after the one before is late.
const LATE_GAP: Duration = Duration::from_millis(100);
/// Below this, with sound flowing, the microphone is as good as silent.
const QUIET_DB: f32 = -50.0;
/// The least a level shows.
pub const FLOOR_DB: f32 = -90.0;
/// 16-bit mono: bytes in a second at the call's rate.
const BYTES_PER_SECOND: usize = nectarlink_core::CALLS_AUDIO_RATE as usize * 2;

/// Whether this system has a call's sound to test.
pub fn supported() -> bool {
    cfg!(any(windows, target_os = "macos"))
}

// ---- Levels ----

/// A level in dBFS (0 is full scale) from a sample's size, 0 to 32768.
pub fn dbfs(amplitude: f64) -> f32 {
    if amplitude <= 0.0 {
        return FLOOR_DB;
    }
    ((20.0 * (amplitude / 32768.0).log10()) as f32).clamp(FLOOR_DB, 0.0)
}

/// What the sound showed over a window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Level {
    pub rms_db: f32,
    pub peak_db: f32,
}

/// Measures 16-bit little-endian PCM: the level since the last
/// [`Meter::window`], and what the whole run showed.
#[derive(Debug, Clone, Default)]
pub struct Meter {
    sum_squares: f64,
    samples: u64,
    peak: u32,
    /// Samples measured in all.
    pub total_samples: u64,
    /// The biggest sample of the run, 0 to 32768.
    pub loudest: u32,
    /// Samples at full scale: the sound was cut off there.
    pub clipped: u64,
}

impl Meter {
    pub fn push(&mut self, pcm: &[u8]) {
        for pair in pcm.as_chunks::<2>().0 {
            let sample = i32::from(i16::from_le_bytes(*pair));
            let size = sample.unsigned_abs();
            self.sum_squares += f64::from(size) * f64::from(size);
            self.samples += 1;
            self.peak = self.peak.max(size);
            self.loudest = self.loudest.max(size);
            if sample == i32::from(i16::MAX) || sample == i32::from(i16::MIN) {
                self.clipped += 1;
            }
        }
        self.total_samples += (pcm.len() / 2) as u64;
    }

    /// The level of what came since the last call; `None` when nothing did.
    pub fn window(&mut self) -> Option<Level> {
        if self.samples == 0 {
            return None;
        }
        let level = Level {
            rms_db: dbfs((self.sum_squares / self.samples as f64).sqrt()),
            peak_db: dbfs(f64::from(self.peak)),
        };
        self.sum_squares = 0.0;
        self.samples = 0;
        self.peak = 0;
        Some(level)
    }
}

/// How long `bytes` of the call's sound last, in microseconds.
fn duration_us(bytes: usize) -> u64 {
    (bytes as u64 / 2) * 1_000_000 / u64::from(nectarlink_core::CALLS_AUDIO_RATE)
}

// ---- Stages ----

/// Where a test is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Stage {
    #[default]
    Idle,
    /// Looking at the microphone and asking the phone.
    Starting,
    /// A loopback: the microphone goes to the phone and comes back.
    Running,
    /// A local test is recording.
    Recording,
    /// A local test plays what it recorded.
    Playing,
    Done,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Begin,
    /// The microphone records, for a loopback or a local test.
    Live(bool),
    Recorded,
    Played,
    Fail,
    Stop,
}

impl Stage {
    pub fn is_active(self) -> bool {
        !matches!(self, Stage::Idle | Stage::Done | Stage::Failed)
    }

    pub fn on(self, event: Event) -> Stage {
        match (self, event) {
            (Stage::Idle | Stage::Done | Stage::Failed, Event::Begin) => Stage::Starting,
            (Stage::Starting, Event::Live(true)) => Stage::Running,
            (Stage::Starting, Event::Live(false)) => Stage::Recording,
            (Stage::Recording, Event::Recorded) => Stage::Playing,
            (Stage::Playing, Event::Played) => Stage::Done,
            (s, Event::Fail) if s.is_active() => Stage::Failed,
            (Stage::Starting, Event::Stop) => Stage::Idle,
            (s, Event::Stop) if s.is_active() => Stage::Done,
            (s, _) => s,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Stage::Idle => "idle",
            Stage::Starting => "starting",
            Stage::Running => "running",
            Stage::Recording => "recording",
            Stage::Playing => "playing",
            Stage::Done => "done",
            Stage::Failed => "failed",
        }
    }
}

/// What a test runs through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Through the phone: the PC's microphone to its speaker and back.
    Loopback,
    /// On this PC only, and why not through the phone.
    Local(LocalReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalReason {
    /// No paired phone is connected.
    Offline,
    /// The connected phone doesn't offer the test (an older NectarLink, or its
    /// calls or call sound are off for this PC).
    NoTest,
    /// The phone offered it and then said no.
    Refused,
}

/// What this PC knows of its phones, for choosing a mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phones {
    None,
    OnlineWithoutTest,
    OnlineWithTest(DeviceId),
}

pub fn mode_for(phones: Phones) -> Mode {
    match phones {
        Phones::OnlineWithTest(_) => Mode::Loopback,
        Phones::OnlineWithoutTest => Mode::Local(LocalReason::NoTest),
        Phones::None => Mode::Local(LocalReason::Offline),
    }
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Mode::Loopback => "loopback",
            Mode::Local(_) => "local",
        }
    }

    fn reason(self) -> &'static str {
        match self {
            Mode::Loopback => "",
            Mode::Local(LocalReason::Offline) => "offline",
            Mode::Local(LocalReason::NoTest) => "no_test",
            Mode::Local(LocalReason::Refused) => "refused",
        }
    }
}

// ---- The run ----

/// What the PC records from, as the system describes it.
#[derive(Debug, Clone, Default, PartialEq)]
struct Source {
    /// The microphone's name; empty when the system won't say.
    name: String,
    native_rate: Option<u32>,
    native_channels: Option<u32>,
    bluetooth: bool,
    /// The system's input volume, where the system has one NectarLink can read.
    system_volume: Option<u8>,
}

#[derive(Default)]
struct Run {
    id: u64,
    stage: Stage,
    mode: Option<Mode>,
    /// The phone a loopback goes through.
    phone: Option<DeviceId>,
    started: Option<Instant>,
    source: Source,
    output: String,
    error: String,
    /// The microphone, as recorded.
    input: Meter,
    /// What came back (a loopback) or plays (a local test).
    returned: Meter,
    /// When the first sound was recorded, which its times count from.
    capture_start: Option<Instant>,
    sent_chunks: u64,
    sent_bytes: u64,
    /// Chunks the stream refused to keep the delay low.
    dropped: u64,
    stream_closed: bool,
    returned_chunks: u64,
    returned_bytes: u64,
    /// Sound that returned a while after the one before.
    late: u64,
    /// Sound that never returned, in microseconds, from gaps in its times.
    lost_us: u64,
    next_time_us: Option<u64>,
    last_return: Option<Instant>,
    latency: Latency,
    /// What a local test recorded.
    local_pcm: Vec<u8>,
    recording_since: Option<Instant>,
}

/// Round trips, from the sound recorded to its copy back on the PC (before
/// the PC's own playback buffer).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Latency {
    last: Option<Duration>,
    max: Duration,
    sum: Duration,
    count: u32,
}

impl Latency {
    fn push(&mut self, trip: Duration) {
        self.last = Some(trip);
        self.max = self.max.max(trip);
        self.sum += trip;
        self.count += 1;
    }

    fn average(&self) -> Option<Duration> {
        (self.count > 0).then(|| self.sum / self.count)
    }
}

impl Run {
    fn on(&mut self, event: Event) {
        self.stage = self.stage.on(event);
    }

    /// Sound the microphone just recorded.
    fn recorded(&mut self, now: Instant, pcm: &[u8]) {
        let first = self.capture_start.is_none();
        if first {
            self.capture_start =
                Some(now.checked_sub(Duration::from_micros(duration_us(pcm.len()))).unwrap_or(now));
        }
        self.input.push(pcm);
        self.sent_chunks += 1;
        self.sent_bytes += pcm.len() as u64;
    }

    /// Sound that came back from the phone, recorded at `time_us`.
    fn came_back(&mut self, now: Instant, time_us: u64, pcm: &[u8]) {
        self.returned.push(pcm);
        self.returned_chunks += 1;
        self.returned_bytes += pcm.len() as u64;
        if self.last_return.is_some_and(|last| now.duration_since(last) > LATE_GAP) {
            self.late += 1;
        }
        self.last_return = Some(now);
        let len_us = duration_us(pcm.len());
        // Contiguous sound has contiguous times: a jump forward is sound that never came.
        if let Some(next) = self.next_time_us
            && time_us > next + 1_000
        {
            self.lost_us += time_us - next;
        }
        self.next_time_us = Some(time_us + len_us);
        if let Some(start) = self.capture_start {
            let recorded_at = start + Duration::from_micros(time_us + len_us);
            self.latency.push(now.saturating_duration_since(recorded_at));
        }
    }

    fn elapsed(&self, now: Instant) -> Duration {
        self.started.map_or(Duration::ZERO, |s| now.saturating_duration_since(s))
    }

    /// What the view shows now. Takes the meters' windows, so call it once
    /// per refresh.
    fn view(&mut self, now: Instant) -> serde_json::Value {
        let (input, returned) = (self.input.window(), self.returned.window());
        // Nothing recorded since the last refresh: the meters fall to the floor.
        let stalled = |level: Option<Level>, live: bool| match (level, live) {
            (Some(l), true) => json!({ "rms": l.rms_db, "peak": l.peak_db }),
            _ => json!({ "rms": FLOOR_DB, "peak": FLOOR_DB }),
        };
        let recording = matches!(self.stage, Stage::Running | Stage::Recording);
        let playing = matches!(self.stage, Stage::Running | Stage::Playing);
        let ms = |d: Duration| d.as_millis() as u64;
        let remaining = match (self.stage, self.recording_since) {
            (Stage::Recording, Some(since)) => {
                LOCAL_RECORD.saturating_sub(now.saturating_duration_since(since))
            }
            (Stage::Running, _) => LOOPBACK_MAX.saturating_sub(self.elapsed(now)),
            _ => Duration::ZERO,
        };
        json!({
            "stage": self.stage.name(),
            "mode": self.mode.map_or("", Mode::name),
            "reason": self.mode.map_or("", Mode::reason),
            "elapsed_ms": ms(self.elapsed(now)),
            "remaining_ms": ms(remaining),
            "source": {
                "name": self.source.name,
                "native_rate": self.source.native_rate,
                "native_channels": self.source.native_channels,
                "bluetooth": self.source.bluetooth,
                "system_volume": self.source.system_volume,
                "rate": nectarlink_core::CALLS_AUDIO_RATE,
                "channels": 1,
            },
            "output": self.output,
            "input": stalled(input, recording),
            "loudest_db": dbfs(f64::from(self.input.loudest)),
            "clipped": self.input.clipped,
            "returned": stalled(returned, playing),
            "sent_chunks": self.sent_chunks,
            "sent_bytes": self.sent_bytes,
            "dropped": self.dropped,
            "returned_chunks": self.returned_chunks,
            "returned_bytes": self.returned_bytes,
            "late": self.late,
            "lost_ms": self.lost_us / 1000,
            "latency_ms": self.latency.average().map(|average| json!({
                "last": self.latency.last.map(ms),
                "average": ms(average),
                "max": ms(self.latency.max),
            })),
            "hints": self.hints(),
            "error": self.error,
        })
    }

    /// What the numbers say is wrong, as codes the view words.
    fn hints(&self) -> Vec<&'static str> {
        let mut hints = Vec::new();
        let heard = self.input.total_samples >= u64::from(nectarlink_core::CALLS_AUDIO_RATE) * 2;
        if heard && dbfs(f64::from(self.input.loudest)) < QUIET_DB {
            hints.push("quiet");
        }
        if self.input.clipped > 0 {
            hints.push("clipping");
        }
        if self.source.bluetooth {
            hints.push("bluetooth");
        }
        if self.source.system_volume.is_some_and(|v| v < 25) {
            hints.push("low_volume");
        }
        if self.dropped > 0 || self.stream_closed {
            hints.push("dropped");
        }
        if self.mode == Some(Mode::Loopback) && (self.late > 0 || self.lost_us > 0) {
            hints.push("link");
        }
        hints
    }
}

static RUN: Mutex<Option<Run>> = Mutex::new(None);
static NEXT_RUN: AtomicU64 = AtomicU64::new(1);

fn with_run<T>(id: u64, f: impl FnOnce(&mut Run) -> T) -> Option<T> {
    let mut guard = RUN.lock().unwrap_or_else(|e| e.into_inner());
    guard.as_mut().filter(|r| r.id == id).map(f)
}

/// Whether run `id` is still the one going.
fn active(id: u64) -> bool {
    with_run(id, |r| r.stage.is_active()).unwrap_or(false)
}

fn fail(id: u64, error: String) {
    with_run(id, |r| {
        r.error = error;
        r.on(Event::Fail);
    });
}

/// What the view shows; `"{}"` before any test.
pub fn view() -> String {
    let mut guard = RUN.lock().unwrap_or_else(|e| e.into_inner());
    guard.as_mut().map_or_else(|| "{}".to_owned(), |run| run.view(Instant::now()).to_string())
}

/// Ends the test going on, if any.
pub fn stop() {
    if let Some(run) = RUN.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        run.on(Event::Stop);
    }
}

/// Begins a test, unless one is going or a call's sound is on this PC.
pub fn start() {
    if !supported() || crate::call_audio::phase() != crate::call_audio::Phase::Off {
        return;
    }
    let id = NEXT_RUN.fetch_add(1, Ordering::Relaxed);
    {
        let mut guard = RUN.lock().unwrap_or_else(|e| e.into_inner());
        if guard.as_ref().is_some_and(|r| r.stage.is_active()) {
            return;
        }
        let mut run = Run { id, started: Some(Instant::now()), ..Run::default() };
        run.on(Event::Begin);
        *guard = Some(run);
    }
    core_host::spawn(run_test(id));
}

/// Where a loopback's sound comes back to: only while this PC asked `peer` for the test.
pub fn sink(peer: &DeviceId) -> Option<std::sync::Arc<dyn nectarlink_core::MicSink>> {
    let guard = RUN.lock().unwrap_or_else(|e| e.into_inner());
    let run = guard
        .as_ref()
        .filter(|r| matches!(r.stage, Stage::Starting | Stage::Running) && r.phone.as_ref() == Some(peer))?;
    Some(std::sync::Arc::new(Sink::new(run.id)))
}

/// A connected phone that offers the test, else any connected phone.
fn phones() -> Phones {
    core_host::host().hub.read(|s| {
        let online: Vec<_> = s
            .devices
            .iter()
            .filter(|d| d.info.kind == nectarlink_core::DeviceKind::Phone)
            .filter(|d| matches!(d.link, nectarlink_core::LinkState::Online { .. }))
            .collect();
        online
            .iter()
            .find(|d| {
                s.matrices.get(&d.id).and_then(|m| m.state("calls.audio_test"))
                    == Some(nectarlink_core::FeatureState::Available)
            })
            .map(|d| Phones::OnlineWithTest(d.id))
            .unwrap_or(if online.is_empty() { Phones::None } else { Phones::OnlineWithoutTest })
    })
}

async fn run_test(id: u64) {
    let chosen = crate::call_audio::chosen_mic();
    let (source, output) =
        tokio::task::spawn_blocking(move || (source_info(chosen), output_name())).await.unwrap_or_default();
    let phones = phones();
    with_run(id, |r| {
        r.source = source;
        r.output = output;
        r.phone = match phones {
            Phones::OnlineWithTest(d) => Some(d),
            _ => None,
        };
    });
    let mut mode = mode_for(phones);
    let mut loopback = None;
    if let (Mode::Loopback, Phones::OnlineWithTest(phone), Some(node)) = (mode, phones, core_host::node()) {
        // The phone's side first: it must agree before any sound goes.
        match node.call_audio_test_start(phone).await {
            Ok(()) => loopback = Some((node, phone)),
            Err(e) => {
                tracing::info!(error = %e, "the phone didn't take the call audio test");
                mode = Mode::Local(LocalReason::Refused);
            }
        }
    } else if mode == Mode::Loopback {
        mode = Mode::Local(LocalReason::Offline);
    }
    if with_run(id, |r| r.mode = Some(mode)).is_none() || !active(id) {
        if let Some((node, phone)) = loopback {
            node.call_audio_test_stop(phone).await;
        }
        return;
    }
    match loopback {
        Some((node, phone)) => run_loopback(id, node, phone).await,
        None => {
            let ran = tokio::task::spawn_blocking(move || run_local(id)).await;
            if let Err(e) = ran {
                fail(id, e.to_string());
            }
        }
    }
}

/// The PC's microphone through the phone and back, until stopped.
async fn run_loopback(id: u64, node: nectarlink_core::Node, phone: DeviceId) {
    let result = async {
        let uplink = std::sync::Arc::new(
            node.call_audio_test_open(phone).await.map_err(|e| crate::bridge::app::describe(&e))?,
        );
        let sending = uplink.clone();
        // Sending a config packet and starting the recorder block: off the runtime's threads.
        let recorder = tokio::task::spawn_blocking(move || {
            let config = nectarlink_core::MirrorAudioConfig {
                codec: nectarlink_core::MIRROR_PCM.into(),
                rate: nectarlink_core::CALLS_AUDIO_RATE,
                channels: 1,
            };
            sending.send(nectarlink_core::PacketKind::Config, 0, config.to_cbor());
            crate::call_audio::recorder_with(move |time_us, pcm| {
                let now = Instant::now();
                with_run(id, |r| {
                    r.recorded(now, &pcm);
                    match sending.send(nectarlink_core::PacketKind::Frame, time_us, pcm) {
                        nectarlink_core::MirrorSend::Queued => {}
                        nectarlink_core::MirrorSend::Closed => r.stream_closed = true,
                        _ => r.dropped += 1,
                    }
                });
            })
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| crate::call_audio::mic_message(&e))?;
        Ok::<_, String>((uplink, recorder))
    }
    .await;
    match result {
        Ok((uplink, recorder)) => {
            with_run(id, |r| r.on(Event::Live(true)));
            while active(id)
                && with_run(id, |r| r.elapsed(Instant::now()) < LOOPBACK_MAX && !r.stream_closed)
                    .unwrap_or(false)
            {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            // The recorder stops when it goes; its thread may be waiting on the run's lock.
            let _ = tokio::task::spawn_blocking(move || drop(recorder)).await;
            uplink.close();
        }
        Err(e) => fail(id, e),
    }
    node.call_audio_test_stop(phone).await;
    with_run(id, |r| r.on(Event::Stop));
}

/// Records 5 s from the microphone and plays them back on the call output.
fn run_local(id: u64) {
    let recorder = match crate::call_audio::recorder_with(move |_, pcm| {
        let now = Instant::now();
        with_run(id, |r| {
            r.recorded(now, &pcm);
            if r.local_pcm.len() < LOCAL_RECORD.as_secs() as usize * BYTES_PER_SECOND {
                r.local_pcm.extend_from_slice(&pcm);
            }
        });
    }) {
        Ok(recorder) => recorder,
        Err(e) => return fail(id, crate::call_audio::mic_message(&e)),
    };
    with_run(id, |r| {
        r.recording_since = Some(Instant::now());
        r.on(Event::Live(false));
    });
    let need = LOCAL_RECORD.as_secs() as usize * BYTES_PER_SECOND;
    while active(id) && with_run(id, |r| r.local_pcm.len() < need).unwrap_or(false) {
        std::thread::sleep(Duration::from_millis(50));
    }
    drop(recorder);
    let Some(pcm) = with_run(id, |r| {
        r.on(Event::Recorded);
        std::mem::take(&mut r.local_pcm)
    }) else {
        return;
    };
    if !active(id) {
        return;
    }
    play_back(id, &pcm);
}

#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
fn play_back(id: u64, pcm: &[u8]) {
    let player = match crate::win::audio_out::Player::start(nectarlink_core::CALLS_AUDIO_RATE, 1) {
        Ok(player) => player,
        Err(e) => {
            tracing::warn!(error = %e, "can't play the call audio test");
            return fail(id, tr!("Calls", "The PC's speakers couldn't be started."));
        }
    };
    // In real time, as the phone's sound would arrive: the player's buffer is a
    // few tenths of a second, and it drops what it can't hold.
    let chunk = BYTES_PER_SECOND / 50;
    let mut next = Instant::now();
    for piece in pcm.chunks(chunk) {
        if !active(id) {
            return;
        }
        with_run(id, |r| r.returned.push(piece));
        player.push(piece.to_vec());
        next += Duration::from_millis(20);
        std::thread::sleep(next.saturating_duration_since(Instant::now()));
    }
    // What's still queued in the player.
    std::thread::sleep(Duration::from_millis(600));
    with_run(id, |r| r.on(Event::Played));
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn play_back(id: u64, _pcm: &[u8]) {
    fail(id, "not on this system".into());
}

/// Plays what the phone sent back, and counts it.
struct Sink {
    run: u64,
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    player: Mutex<Option<crate::win::audio_out::Player>>,
}

impl Sink {
    fn new(run: u64) -> Self {
        Sink {
            run,
            #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
            player: Mutex::new(None),
        }
    }
}

impl nectarlink_core::MicSink for Sink {
    fn config(&self, config: nectarlink_core::MirrorAudioConfig) {
        #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
        match crate::win::audio_out::Player::start(config.rate, u16::from(config.channels)) {
            Ok(player) => *self.player.lock().unwrap_or_else(|e| e.into_inner()) = Some(player),
            Err(e) => {
                tracing::warn!(error = %e, "can't play the call audio test");
                fail(self.run, tr!("Calls", "The PC's speakers couldn't be started."));
            }
        }
        #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
        let _ = config;
    }

    fn audio(&self, time_us: u64, data: Vec<u8>) {
        let now = Instant::now();
        with_run(self.run, |r| r.came_back(now, time_us, &data));
        #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
        if let Some(player) = self.player.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            player.push(data);
        }
    }

    fn ended(&self) {
        #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
        self.player.lock().unwrap_or_else(|e| e.into_inner()).take();
    }
}

// ---- What the system says ----

#[cfg(target_os = "macos")]
fn source_info(chosen: Option<String>) -> Source {
    let format = crate::win::audio::input_format(chosen.as_deref());
    Source {
        name: format.as_ref().map(|f| f.name.clone()).unwrap_or_default(),
        native_rate: format.as_ref().map(|f| f.rate).filter(|r| *r > 0),
        native_channels: format.as_ref().map(|f| f.channels).filter(|c| *c > 0),
        bluetooth: format.as_ref().is_some_and(|f| f.bluetooth),
        system_volume: crate::win::audio::input_volume(),
    }
}

/// Windows tells only the microphone's name here: its own format is up to
/// WASAPI, which converts it for the recorder.
#[cfg(windows)]
fn source_info(chosen: Option<String>) -> Source {
    let devices = crate::win::audio::input_devices();
    let device = devices
        .iter()
        .find(|d| chosen.as_deref() == Some(d.id.as_str()))
        .or_else(|| devices.iter().find(|d| d.is_default));
    Source { name: device.map(|d| d.name.clone()).unwrap_or_default(), ..Source::default() }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn source_info(_chosen: Option<String>) -> Source {
    Source::default()
}

#[cfg(target_os = "macos")]
fn output_name() -> String {
    crate::win::audio::default_output_name().unwrap_or_default()
}

#[cfg(windows)]
fn output_name() -> String {
    crate::win::audio::output_devices().into_iter().find(|d| d.is_default).map(|d| d.name).unwrap_or_default()
}

#[cfg(not(any(windows, target_os = "macos")))]
fn output_name() -> String {
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pcm(samples: &[i16]) -> Vec<u8> {
        samples.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    #[test]
    fn dbfs_is_zero_at_full_scale_and_6_db_down_at_half() {
        assert_eq!(dbfs(32768.0), 0.0);
        assert!((dbfs(16384.0) + 6.02).abs() < 0.01);
        assert_eq!(dbfs(0.0), FLOOR_DB);
        assert_eq!(dbfs(0.0001), FLOOR_DB);
        // Never above full scale, whatever the input.
        assert_eq!(dbfs(100_000.0), 0.0);
    }

    #[test]
    fn a_meter_reads_rms_and_peak_of_a_window() {
        let mut meter = Meter::default();
        assert_eq!(meter.window(), None);
        // A square wave at half scale: rms and peak are the same.
        meter.push(&pcm(&[16384, -16384, 16384, -16384]));
        let level = meter.window().unwrap();
        assert!((level.rms_db + 6.02).abs() < 0.01, "{level:?}");
        assert!((level.peak_db + 6.02).abs() < 0.01, "{level:?}");
        // The window starts over; the run's loudest stays.
        assert_eq!(meter.window(), None);
        meter.push(&pcm(&[0, 0, 100, 0]));
        let level = meter.window().unwrap();
        assert!(level.peak_db > level.rms_db);
        assert_eq!(meter.loudest, 16384);
        assert_eq!(meter.total_samples, 8);
    }

    #[test]
    fn a_meter_counts_samples_at_full_scale_as_clipped() {
        let mut meter = Meter::default();
        meter.push(&pcm(&[i16::MAX, 5, i16::MIN, i16::MAX - 1]));
        assert_eq!(meter.clipped, 2);
        assert_eq!(meter.loudest, 32768);
        // A half sample at the end is left out.
        let mut odd = pcm(&[1, 2]);
        odd.push(7);
        let mut meter = Meter::default();
        meter.push(&odd);
        assert_eq!(meter.total_samples, 2);
    }

    #[test]
    fn a_silent_microphone_has_the_floor_level() {
        let mut meter = Meter::default();
        meter.push(&pcm(&[0; 320]));
        assert_eq!(meter.window(), Some(Level { rms_db: FLOOR_DB, peak_db: FLOOR_DB }));
    }

    #[test]
    fn a_loopback_runs_start_to_done_and_a_local_test_records_then_plays() {
        let loopback = [Event::Begin, Event::Live(true), Event::Stop];
        let stages: Vec<_> = loopback
            .iter()
            .scan(Stage::Idle, |s, e| {
                *s = s.on(*e);
                Some(*s)
            })
            .collect();
        assert_eq!(stages, [Stage::Starting, Stage::Running, Stage::Done]);

        let local = [Event::Begin, Event::Live(false), Event::Recorded, Event::Played];
        let stages: Vec<_> = local
            .iter()
            .scan(Stage::Idle, |s, e| {
                *s = s.on(*e);
                Some(*s)
            })
            .collect();
        assert_eq!(stages, [Stage::Starting, Stage::Recording, Stage::Playing, Stage::Done]);
    }

    #[test]
    fn stages_ignore_what_doesnt_fit_and_a_finished_test_can_begin_again() {
        // Nothing happens out of turn.
        assert_eq!(Stage::Idle.on(Event::Recorded), Stage::Idle);
        assert_eq!(Stage::Running.on(Event::Begin), Stage::Running);
        assert_eq!(Stage::Recording.on(Event::Played), Stage::Recording);
        assert_eq!(Stage::Done.on(Event::Fail), Stage::Done);
        assert_eq!(Stage::Idle.on(Event::Stop), Stage::Idle);
        // Stopping before anything records is no result; during one it ends it.
        assert_eq!(Stage::Starting.on(Event::Stop), Stage::Idle);
        assert_eq!(Stage::Playing.on(Event::Stop), Stage::Done);
        // A failure ends any active stage, and the test can run again.
        for s in [Stage::Starting, Stage::Running, Stage::Recording, Stage::Playing] {
            assert!(s.is_active());
            assert_eq!(s.on(Event::Fail), Stage::Failed);
        }
        assert_eq!(Stage::Failed.on(Event::Begin), Stage::Starting);
        assert_eq!(Stage::Done.on(Event::Begin), Stage::Starting);
    }

    #[test]
    fn the_mode_follows_what_the_phone_offers() {
        let phone = DeviceId([7; 32]);
        assert_eq!(mode_for(Phones::OnlineWithTest(phone)), Mode::Loopback);
        assert_eq!(mode_for(Phones::OnlineWithoutTest), Mode::Local(LocalReason::NoTest));
        assert_eq!(mode_for(Phones::None), Mode::Local(LocalReason::Offline));
        assert_eq!(Mode::Local(LocalReason::Offline).reason(), "offline");
        assert_eq!(Mode::Loopback.reason(), "");
    }

    #[test]
    fn a_round_trip_is_measured_from_when_the_sound_was_recorded() {
        let now = Instant::now();
        let mut run = Run { mode: Some(Mode::Loopback), ..Run::default() };
        let chunk = pcm(&[1000; 320]);
        // 20 ms of sound, recorded when `now` came.
        run.recorded(now, &chunk);
        let start = run.capture_start.unwrap();
        assert_eq!(now.saturating_duration_since(start), Duration::from_millis(20));
        // It's back 120 ms after its end: a round trip of 120 ms.
        run.came_back(start + Duration::from_millis(140), 0, &chunk);
        assert_eq!(run.latency.last, Some(Duration::from_millis(120)));
        // The next one is on time and contiguous: nothing lost, nothing late.
        run.came_back(start + Duration::from_millis(160), 20_000, &chunk);
        assert_eq!((run.late, run.lost_us), (0, 0));
        assert_eq!(run.latency.average(), Some(Duration::from_millis(120)));
    }

    #[test]
    fn missing_and_late_sound_are_counted() {
        let start = Instant::now();
        let mut run = Run { mode: Some(Mode::Loopback), capture_start: Some(start), ..Run::default() };
        let chunk = pcm(&[1; 320]);
        run.came_back(start + Duration::from_millis(100), 0, &chunk);
        // 60 ms of sound never came (the next one's time jumped), and it's 150 ms after the last.
        run.came_back(start + Duration::from_millis(250), 80_000, &chunk);
        assert_eq!(run.lost_us, 60_000);
        assert_eq!(run.late, 1);
        assert!(run.hints().contains(&"link"));
    }

    #[test]
    fn hints_say_what_the_numbers_mean() {
        let mut run = Run::default();
        assert!(run.hints().is_empty());
        // Two seconds of near silence.
        run.input.push(&pcm(&vec![3; 32_000]));
        assert!(run.hints().contains(&"quiet"));
        // A short silence isn't judged yet.
        let mut short = Run::default();
        short.input.push(&pcm(&[0; 160]));
        assert!(!short.hints().contains(&"quiet"));
        run.input.push(&pcm(&[i16::MAX]));
        run.source = Source { bluetooth: true, system_volume: Some(10), ..Source::default() };
        run.dropped = 2;
        let hints = run.hints();
        for hint in ["clipping", "bluetooth", "low_volume", "dropped"] {
            assert!(hints.contains(&hint), "{hint} in {hints:?}");
        }
        assert!(!hints.contains(&"quiet"), "{hints:?}");
    }

    #[test]
    fn the_view_carries_the_source_the_levels_and_the_counters() {
        let now = Instant::now();
        let mut run = Run {
            stage: Stage::Running,
            mode: Some(Mode::Loopback),
            started: Some(now),
            source: Source {
                name: "Mic".into(),
                native_rate: Some(48_000),
                native_channels: Some(2),
                ..Source::default()
            },
            output: "Speakers".into(),
            ..Run::default()
        };
        run.recorded(now, &pcm(&[16384; 320]));
        let view = run.view(now + Duration::from_millis(500));
        assert_eq!(view["stage"], "running");
        assert_eq!(view["mode"], "loopback");
        assert_eq!(view["source"]["name"], "Mic");
        assert_eq!(view["source"]["native_rate"], 48_000);
        assert_eq!(view["source"]["rate"], 16_000);
        assert_eq!(view["output"], "Speakers");
        assert_eq!(view["sent_chunks"], 1);
        assert_eq!(view["sent_bytes"], 640);
        assert!((view["input"]["rms"].as_f64().unwrap() + 6.02).abs() < 0.01);
        assert_eq!(view["latency_ms"], serde_json::Value::Null);
        // Nothing recorded since: the meter falls to the floor, but the run's loudest stays.
        let again = run.view(now + Duration::from_millis(600));
        assert_eq!(again["input"]["rms"], f64::from(FLOOR_DB));
        assert!((again["loudest_db"].as_f64().unwrap() + 6.02).abs() < 0.01);
    }

    /// Needs a microphone and speakers: records 5 s and plays them back
    /// (`cargo test -p nectarlink-desktop -- --ignored a_local_test_runs`).
    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    #[ignore = "records the microphone and plays sound"]
    fn a_local_test_runs_from_recording_to_playback() {
        let id = NEXT_RUN.fetch_add(1, Ordering::Relaxed);
        let mut run = Run {
            id,
            started: Some(Instant::now()),
            mode: Some(Mode::Local(LocalReason::Offline)),
            ..Run::default()
        };
        run.on(Event::Begin);
        *RUN.lock().unwrap() = Some(run);
        run_local(id);
        let stage = with_run(id, |r| r.stage).unwrap();
        let (sent, bytes, returned) =
            with_run(id, |r| (r.sent_chunks, r.sent_bytes, r.returned.total_samples)).unwrap();
        eprintln!("stage {stage:?}, {sent} chunks, {bytes} bytes, {returned} samples played");
        assert_eq!(stage, Stage::Done, "{:?}", with_run(id, |r| r.error.clone()));
        assert!(sent > 100);
        assert_eq!(returned, 5 * u64::from(nectarlink_core::CALLS_AUDIO_RATE));
    }
}
