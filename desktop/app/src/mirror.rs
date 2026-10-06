// SPDX-License-Identifier: GPL-3.0-or-later
//! A phone's screen on this PC (docs/protocol/mirror.md): asks the phone,
//! decodes its video on a thread of its own and hands each picture to the
//! mirror window's `VideoView`. Only the newest picture is shown; when the
//! decoder falls behind or loses its place, the phone is asked for a
//! keyframe. The phone's sound, when it sends it, plays on the PC's
//! speakers.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
        mpsc::{Receiver, SyncSender, TrySendError, sync_channel},
    },
    time::{Duration, Instant},
};

use nectarlink_core::{
    DeviceId, Error, MIRROR_SCREEN, MirrorAudioConfig, MirrorConfig, MirrorSink, MirrorStart, NodeEvent,
};

use crate::{
    bridge::{app::describe, native::ffi},
    core_host,
    state::Changes,
    win::{
        audio_out::Player,
        h264::{Decoder, to_bgrx},
    },
};

/// What the PC asks for: the phone scales its screen to fit.
const OPTIONS: MirrorStart = MirrorStart {
    max_size: 1920,
    fps: 60,
    bitrate: 8_000_000,
    audio: true,
    session: MIRROR_SCREEN,
    app: None,
};
/// Packets waiting for the decoder, at most (about two seconds).
const BACKLOG: usize = 120;
/// How often decode times are logged.
const STATS_EVERY: Duration = Duration::from_secs(10);

/// A mirroring on this PC: a phone and the session, 0 for its screen,
/// others for its apps in windows of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Window {
    pub device: DeviceId,
    pub session: u32,
}

impl Window {
    pub fn screen(device: DeviceId) -> Window {
        Window { device, session: MIRROR_SCREEN }
    }

    /// Its name for QML and for `VideoView`'s stream: `<device>/<session>`.
    pub fn key(&self) -> String {
        format!("{}/{}", self.device, self.session)
    }

    pub fn parse(key: &str) -> Option<Window> {
        let (device, session) = key.split_once('/')?;
        Some(Window { device: device.parse().ok()?, session: session.parse().ok()? })
    }
}

/// Where a mirroring stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// Waiting for the phone (its user agreeing to share the screen, or the
    /// app starting).
    Asking,
    Showing,
    /// It ended; the reason for the window, if any.
    Ended(Option<String>),
}

/// A mirroring the PC asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    pub phase: Phase,
    /// For an app window: the app's name, and its package.
    pub app: Option<String>,
    pub pkg: Option<String>,
}

/// A phone's apps that open in windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Apps {
    Loading,
    Ready(Vec<App>),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    pub pkg: String,
    pub label: String,
    /// The icon, saved for QML.
    pub icon: Option<PathBuf>,
}

#[derive(Debug, Default)]
struct State {
    windows: HashMap<Window, Shown>,
    /// Phones whose sound is coming in.
    sound: HashSet<DeviceId>,
    apps: HashMap<DeviceId, Apps>,
}

/// The user turned the phones' sound off on this PC (until turned on).
static MUTED: AtomicBool = AtomicBool::new(false);
/// The next app window's session.
static NEXT_SESSION: AtomicU32 = AtomicU32::new(1);

pub fn muted() -> bool {
    MUTED.load(Ordering::Relaxed)
}

pub fn set_muted(muted: bool) {
    MUTED.store(muted, Ordering::Relaxed);
    core_host::host().hub.changed(Changes::MIRROR);
}

/// Whether `device`'s sound is coming in.
pub fn has_sound(device: &DeviceId) -> bool {
    state(|s| s.sound.contains(device))
}

fn set_sound(device: DeviceId, on: bool) {
    state(|s| if on { s.sound.insert(device) } else { s.sound.remove(&device) });
    core_host::host().hub.changed(Changes::MIRROR);
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn state<T>(f: impl FnOnce(&mut State) -> T) -> T {
    f(STATE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_default())
}

fn set_phase(window: Window, phase: Phase) {
    state(|s| {
        if let Some(shown) = s.windows.get_mut(&window) {
            shown.phase = phase;
        }
    });
    core_host::host().hub.changed(Changes::MIRROR);
}

/// Where a mirroring stands (`None`: not asked, or closed).
pub fn phase(window: &Window) -> Option<Phase> {
    state(|s| s.windows.get(window).map(|w| w.phase.clone()))
}

/// Every mirroring the PC asked for, oldest first.
pub fn windows() -> Vec<(Window, Shown)> {
    let mut all: Vec<_> = state(|s| s.windows.iter().map(|(w, s)| (*w, s.clone())).collect());
    all.sort_by_key(|(w, _)| (w.device.to_string(), w.session));
    all
}

/// Asks the phone for its screen.
pub fn start(device: DeviceId) {
    let window = Window::screen(device);
    state(|s| s.windows.insert(window, Shown { phase: Phase::Asking, app: None, pkg: None }));
    request(window, OPTIONS);
}

/// Opens one of the phone's apps in a window of its own (or does nothing
/// when it's already open).
pub fn start_app(device: DeviceId, pkg: String, label: String) {
    let open = state(|s| {
        s.windows.iter().any(|(w, shown)| {
            w.device == device
                && shown.pkg.as_deref() == Some(pkg.as_str())
                && matches!(shown.phase, Phase::Asking | Phase::Showing)
        })
    });
    if open {
        return;
    }
    let session = NEXT_SESSION.fetch_add(1, Ordering::Relaxed);
    let window = Window { device, session };
    state(|s| {
        s.windows.insert(window, Shown { phase: Phase::Asking, app: Some(label), pkg: Some(pkg.clone()) })
    });
    request(window, MirrorStart { audio: false, session, app: Some(pkg), ..OPTIONS });
}

fn request(window: Window, options: MirrorStart) {
    core_host::host().hub.changed(Changes::MIRROR);
    let Some(node) = core_host::node() else { return };
    let app = options.app.is_some();
    core_host::spawn(async move {
        if let Err(e) = node.mirror_start(window.device, options).await {
            let reason = match e {
                Error::Denied => "Mirroring is turned off for this phone.".to_owned(),
                Error::Unsupported if app => {
                    "Set up Wireless debugging in Nectarlink on the phone to open its apps here.".into()
                }
                Error::Unsupported => {
                    "This phone's app doesn't share its screen yet. Update Nectarlink on the phone.".into()
                }
                Error::Offline | Error::NotPaired => "The phone isn't connected.".into(),
                e => describe(&e),
            };
            set_phase(window, Phase::Ended(Some(reason)));
        }
    });
}

/// Stops a mirroring (its window closed).
pub fn stop(window: Window) {
    state(|s| s.windows.remove(&window));
    ffi::video_clear(&window.key());
    core_host::host().hub.changed(Changes::MIRROR);
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move { node.mirror_stop(window.device, window.session).await });
}

pub fn on_event(event: &NodeEvent) {
    match event {
        NodeEvent::Mirroring { device, session, on: true } => {
            set_phase(Window { device: *device, session: *session }, Phase::Showing);
        }
        // Closed here: nothing to say. Stopped on the phone: say so.
        NodeEvent::Mirroring { device, session, on: false } => {
            let window = Window { device: *device, session: *session };
            if phase(&window).is_some() {
                set_phase(window, Phase::Ended(None));
            }
        }
        // A phone that went away takes its app list with it.
        NodeEvent::LinkChanged { device, link: nectarlink_core::LinkState::Offline { .. } } => {
            state(|s| s.apps.remove(device));
        }
        _ => {}
    }
}

/// A phone's apps that open in windows, as last loaded.
pub fn apps(device: &DeviceId) -> Option<Apps> {
    state(|s| s.apps.get(device).cloned())
}

/// Loads (or reloads) a phone's apps, with their icons.
pub fn load_apps(device: DeviceId) {
    state(|s| s.apps.insert(device, Apps::Loading));
    core_host::host().hub.changed(Changes::MIRROR);
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        let loaded = match node.mirror_apps(device).await {
            Ok(apps) => {
                let data_dir = core_host::host().data_dir.clone();
                let apps = tokio::task::spawn_blocking(move || {
                    apps.into_iter()
                        .map(|a| App {
                            icon: a.icon.and_then(|png| crate::icons::save(&data_dir, &a.pkg, &png).ok()),
                            pkg: a.pkg,
                            label: a.label,
                        })
                        .collect()
                })
                .await
                .unwrap_or_default();
                Apps::Ready(apps)
            }
            Err(Error::Unsupported) => Apps::Failed(
                "Set up Wireless debugging in Nectarlink on the phone to open its apps here.".into(),
            ),
            Err(e) => Apps::Failed(describe(&e)),
        };
        state(|s| s.apps.insert(device, loaded));
        core_host::host().hub.changed(Changes::MIRROR);
    });
}

/// The PC's mouse and keyboard on a mirrored screen or app window, sent in
/// order.
pub fn input(window: Window, input: nectarlink_core::MirrorInput) {
    static QUEUE: std::sync::OnceLock<
        tokio::sync::mpsc::UnboundedSender<(Window, nectarlink_core::MirrorInput)>,
    > = std::sync::OnceLock::new();
    let queue = QUEUE.get_or_init(|| {
        let (queue, mut inputs) =
            tokio::sync::mpsc::unbounded_channel::<(Window, nectarlink_core::MirrorInput)>();
        core_host::spawn(async move {
            while let Some((window, input)) = inputs.recv().await {
                if let Some(node) = core_host::node()
                    && let Err(e) = node.mirror_input(window.device, window.session, input).await
                {
                    tracing::debug!(error = %e, "input didn't reach the phone");
                }
            }
        });
        queue
    });
    let _ = queue.send((window, input));
}

/// Where the core puts a phone's video (or the screen's sound, from a
/// stream of its own): only for mirrorings this PC asked for.
pub fn sink(device: &DeviceId, session: u32) -> Option<Arc<dyn MirrorSink>> {
    let window = Window { device: *device, session };
    matches!(phase(&window), Some(Phase::Asking | Phase::Showing))
        .then(|| Arc::new(Sink::new(window)) as Arc<dyn MirrorSink>)
}

enum Item {
    Config(MirrorConfig),
    Packet { keyframe: bool, time_us: u64, data: Vec<u8> },
    Ended,
}

/// Hands the core's packets to the decoder thread, or its sound to the
/// player; each starts with its stream's first packet (the format).
struct Sink {
    window: Window,
    video: OnceLock<Video>,
    sound: Mutex<Option<Player>>,
}

/// The decoder thread's end of a sink.
struct Video {
    queue: SyncSender<Item>,
    /// Packets waiting for the decoder.
    waiting: Arc<AtomicUsize>,
}

impl Sink {
    fn new(window: Window) -> Sink {
        Sink { window, video: OnceLock::new(), sound: Mutex::new(None) }
    }
}

impl Video {
    fn start(window: Window) -> Video {
        let (queue, items) = sync_channel(BACKLOG);
        let waiting = Arc::new(AtomicUsize::new(0));
        let counter = waiting.clone();
        let started = std::thread::Builder::new()
            .name("mirror-decoder".into())
            .spawn(move || decode(window, &items, &counter));
        if let Err(e) = started {
            tracing::error!(error = %e, "can't start the video decoder");
        }
        Video { queue, waiting }
    }
}

impl Sink {
    fn keyframe_please(&self) {
        let window = self.window;
        if let Some(node) = core_host::node() {
            core_host::spawn(async move { node.mirror_keyframe(window.device, window.session).await });
        }
    }
}

impl MirrorSink for Sink {
    fn config(&self, config: MirrorConfig) {
        let video = self.video.get_or_init(|| Video::start(self.window));
        // The format matters: wait for room rather than drop it.
        let _ = video.queue.send(Item::Config(config));
    }

    fn packet(&self, keyframe: bool, time_us: u64, data: Vec<u8>) {
        let Some(video) = self.video.get() else { return };
        match video.queue.try_send(Item::Packet { keyframe, time_us, data }) {
            Ok(()) => {
                video.waiting.fetch_add(1, Ordering::AcqRel);
            }
            Err(TrySendError::Disconnected(_)) => {}
            // The decoder is behind; it starts over at the next keyframe.
            Err(TrySendError::Full(_)) => self.keyframe_please(),
        }
    }

    fn ended(&self) {
        if let Some(video) = self.video.get() {
            let _ = video.queue.send(Item::Ended);
        }
    }

    fn audio_config(&self, config: MirrorAudioConfig) {
        let mut sound = self.sound.lock().unwrap_or_else(|e| e.into_inner());
        // A new format: a new player (the old one ends when dropped).
        *sound = match Player::start(config.rate, u16::from(config.channels)) {
            Ok(player) => Some(player),
            Err(e) => {
                tracing::warn!(error = %e, "can't play the phone's sound");
                None
            }
        };
        set_sound(self.window.device, sound.is_some());
    }

    fn audio(&self, _time_us: u64, data: Vec<u8>) {
        if muted() {
            return;
        }
        if let Some(player) = self.sound.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            player.push(data);
        }
    }

    fn audio_ended(&self) {
        self.sound.lock().unwrap_or_else(|e| e.into_inner()).take();
        set_sound(self.window.device, false);
    }
}

/// The decoder thread: decodes everything (later pictures depend on
/// earlier ones), shows the newest picture of each batch.
fn decode(window: Window, items: &Receiver<Item>, waiting: &AtomicUsize) {
    let stream = window.key();
    let mut decoder = match Decoder::new() {
        Ok(decoder) => decoder,
        Err(e) => {
            tracing::error!(error = %e, "no H.264 decoder");
            set_phase(window, Phase::Ended(Some("This PC can't decode the phone's video.".into())));
            return;
        }
    };
    let keyframe_please = || {
        if let Some(node) = core_host::node() {
            core_host::spawn(async move { node.mirror_keyframe(window.device, window.session).await });
        }
    };
    let mut size = (0, 0);
    // After a gap, pictures are garbage until the next keyframe.
    let mut need_keyframe = true;
    let mut stats = Stats::new();
    while let Ok(item) = items.recv() {
        match item {
            Item::Config(config) => {
                size = (config.width, config.height);
                need_keyframe = true;
            }
            Item::Packet { keyframe, time_us, data } => {
                let behind = waiting.fetch_sub(1, Ordering::AcqRel) > 1;
                if need_keyframe && !keyframe {
                    continue;
                }
                need_keyframe = false;
                let started = Instant::now();
                let pictures = match decoder.decode(&data, time_us) {
                    Ok(pictures) => pictures,
                    Err(e) => {
                        tracing::debug!(error = %e, "a video packet didn't decode");
                        decoder.flush();
                        need_keyframe = true;
                        keyframe_please();
                        continue;
                    }
                };
                let decoded = started.elapsed();
                // Behind: skip showing this one, the next is already here.
                if let (Some(picture), false) = (pictures.last(), behind) {
                    let converted = Instant::now();
                    let (w, h) = if size.0 > 0 { size } else { (picture.width, picture.height) };
                    let bgrx = to_bgrx(picture, w, h);
                    ffi::video_frame(&stream, w.min(picture.width), h.min(picture.height), &bgrx);
                    stats.shown(decoded, converted.elapsed());
                } else {
                    stats.skipped();
                }
            }
            Item::Ended => break,
        }
    }
    ffi::video_clear(&stream);
}

/// Decode and conversion times, logged now and then.
struct Stats {
    since: Instant,
    shown: u32,
    skipped: u32,
    decode: Duration,
    convert: Duration,
}

impl Stats {
    fn new() -> Stats {
        Stats { since: Instant::now(), shown: 0, skipped: 0, decode: Duration::ZERO, convert: Duration::ZERO }
    }

    fn shown(&mut self, decode: Duration, convert: Duration) {
        self.shown += 1;
        self.decode += decode;
        self.convert += convert;
        self.maybe_log();
    }

    fn skipped(&mut self) {
        self.skipped += 1;
        self.maybe_log();
    }

    fn maybe_log(&mut self) {
        let elapsed = self.since.elapsed();
        if elapsed < STATS_EVERY || self.shown == 0 {
            return;
        }
        tracing::info!(
            fps = format!("{:.1}", f64::from(self.shown) / elapsed.as_secs_f64()),
            skipped = self.skipped,
            decode_ms = format!("{:.1}", self.decode.as_secs_f64() * 1000.0 / f64::from(self.shown)),
            convert_ms = format!("{:.1}", self.convert.as_secs_f64() * 1000.0 / f64::from(self.shown)),
            "mirroring"
        );
        *self = Stats::new();
    }
}
