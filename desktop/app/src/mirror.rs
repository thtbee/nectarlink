// SPDX-License-Identifier: GPL-3.0-or-later
//! A phone's screen on this PC (docs/protocol/mirror.md): asks the phone,
//! decodes its video on a thread of its own and hands each picture to the
//! mirror window's `VideoView`. Only the newest picture is shown; when the
//! decoder falls behind or loses its place, the phone is asked for a
//! keyframe. The phone's sound, when it sends it, plays on the PC's
//! speakers.

use crate::i18n::tr;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
        mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel},
    },
    time::{Duration, Instant},
};

use nectarlink_core::{
    DeviceId, Error, MIRROR_SCREEN, MirrorAudioConfig, MirrorConfig, MirrorSink, MirrorStart, NodeEvent,
};

use crate::{
    bridge::{
        app::{describe, show_message},
        native::ffi,
    },
    core_host,
    state::Changes,
    transfers::{ACTION_OPEN, ACTION_SHOW, TOAST_GROUP},
    win::{
        audio_out::Player,
        h264::{Decoder, to_bgrx},
        image::{Bitmap, encode_png},
        mp4::{MirrorRecorder, local_timestamp_filename, merge_sps_pps},
        toast::{self, Toast},
    },
};

/// How sharp the screen and app windows are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    Balanced,
    Sharper,
    Smoother,
}

impl Quality {
    pub fn parse(name: &str) -> Quality {
        match name {
            "sharper" => Quality::Sharper,
            "smoother" => Quality::Smoother,
            _ => Quality::Balanced,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Quality::Balanced => "balanced",
            Quality::Sharper => "sharper",
            Quality::Smoother => "smoother",
        }
    }
}

/// The whole screen's size and bitrate. The frame rate stays, as recording relies
/// on it. Over the relay it's always Balanced, which may be the phone's mobile data.
fn screen_options(quality: Quality, on_relay: bool) -> MirrorStart {
    match (quality, on_relay) {
        (Quality::Sharper, false) => MirrorStart { max_size: 4096, bitrate: 20_000_000, ..OPTIONS },
        (Quality::Smoother, false) => MirrorStart { max_size: 1280, bitrate: 4_000_000, ..OPTIONS },
        _ => OPTIONS,
    }
}

fn app_options(quality: Quality, on_relay: bool) -> MirrorStart {
    let options = screen_options(quality, on_relay);
    // App displays are limited to 1920 pixels by Android's app-window path.
    MirrorStart { max_size: options.max_size.min(OPTIONS.max_size), ..options }
}

fn on_relay(device: &DeviceId) -> bool {
    core_host::host().hub.read(|s| {
        s.devices.iter().any(|d| {
            d.id == *device
                && matches!(
                    d.link,
                    nectarlink_core::LinkState::Online { path: nectarlink_core::ConnectionPath::Relay, .. }
                )
        })
    })
}

/// Whether a stream that ended should be asked for again at Balanced: Sharper or
/// Smoother was asked for, the phone started, and no picture ever came, which is
/// how a phone's encoder refusing that size looks from here.
fn retry_at_balanced(unproven: bool, showing: bool) -> bool {
    unproven && showing
}

/// Phones asked for a quality other than Balanced that haven't sent a picture yet.
static UNPROVEN_QUALITY: Mutex<Option<std::collections::HashSet<DeviceId>>> = Mutex::new(None);

fn set_unproven(device: DeviceId, unproven: bool) -> bool {
    let mut set = UNPROVEN_QUALITY.lock().unwrap_or_else(|e| e.into_inner());
    let set = set.get_or_insert_with(Default::default);
    if unproven { !set.insert(device) } else { set.remove(&device) }
}

/// What the PC asks for: the phone scales its screen to fit.
const OPTIONS: MirrorStart = MirrorStart {
    max_size: 1920,
    fps: 60,
    bitrate: 8_000_000,
    audio: true,
    session: MIRROR_SCREEN,
    app: None,
    stay_awake: false,
    screen_off: false,
    watch: None,
    sound_only: false,
};
/// A short burst, rather than seconds of old frames behind the user's input.
const BACKLOG: usize = 8;
/// How often decode times are logged.
const STATS_EVERY: Duration = Duration::from_secs(10);
/// Even while it's behind, the decoder shows a picture at least this often:
/// skipping every one froze the window on its last picture (#125).
const SHOW_AT_LEAST_EVERY: Duration = Duration::from_millis(200);
/// How long an in-window status notice stays visible.
const NOTICE_DURATION: Duration = Duration::from_secs(3);

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
    /// Stopped to start again (flexible display turned off): its stream's
    /// end starts it again instead of ending the window.
    pub restarting: bool,
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

/// Remembered window position and size on this PC for an app window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WindowGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct SavedWindows {
    /// Keyed by `<device>/<pkg>`.
    #[serde(default)]
    geometry: HashMap<String, WindowGeometry>,
    /// Keyed by `<device>`, most recently opened `pkg` first.
    #[serde(default)]
    recent: HashMap<String, Vec<String>>,
    /// `<device>/<pkg>` of app windows kept at the phone's resolution (flexible
    /// display off): their display doesn't follow the window's size.
    #[serde(default)]
    fixed: HashSet<String>,
    /// `<device>/<pkg>` to how big the app draws in its window, in percent
    /// (Ctrl + mouse wheel); missing is 100.
    #[serde(default)]
    zoom: HashMap<String, u32>,
}

impl SavedWindows {
    fn path() -> PathBuf {
        core_host::host().data_dir.join("app-windows.json")
    }

    fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn save(&self) {
        let path = Self::path();
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }
}

#[derive(Debug)]
struct State {
    windows: HashMap<Window, Shown>,
    /// Most recently focused or interacted-with mirror window.
    active_window: Option<Window>,
    /// Last clipboard text synced on window focus so unchanged text isn't resent.
    last_focus_clip: Option<String>,
    /// Phones whose sound is coming in.
    sound: HashSet<DeviceId>,
    /// Phones kept awake while mirroring (their screen or app windows).
    stay_awake: HashSet<DeviceId>,
    /// Phones whose physical screen is turned off while mirroring (their
    /// screen or app windows); cleared when the last window closes.
    screen_off: HashSet<DeviceId>,
    /// Phone currently receiving PC keyboard input without screen mirroring.
    keyboard_device: Option<DeviceId>,
    /// Short transient feedback notices per mirror window: `(text, shown_until, token)`.
    notices: HashMap<Window, (String, Instant, u64)>,
    apps: HashMap<DeviceId, Apps>,
    saved: SavedWindows,
}

impl State {
    /// Sets what `device`'s screen starts as, unless a window is already open.
    fn reset_screen_off(&mut self, device: &DeviceId) {
        if self.mirroring(device) {
            return;
        }
        if crate::settings::keep_screen_off_phones().contains(&device.to_string()) {
            self.screen_off.insert(*device);
        } else {
            self.screen_off.remove(device);
        }
    }

    /// Whether `device` has a window open or opening (the screen or an app).
    fn mirroring(&self, device: &DeviceId) -> bool {
        self.windows
            .iter()
            .any(|(w, shown)| w.device == *device && matches!(shown.phase, Phase::Asking | Phase::Showing))
    }

    /// Whether `device` has an app window open (asked for or showing), or
    /// its screen's window when `screen`.
    fn has_window(&self, device: &DeviceId, screen: bool) -> bool {
        self.windows.iter().any(|(w, shown)| {
            w.device == *device
                && (w.session == MIRROR_SCREEN) == screen
                && matches!(shown.phase, Phase::Asking | Phase::Showing)
        })
    }
}

/// The phone's sound comes with its app windows (#121): it starts with the
/// first one and stops with the last, unless the screen's window has it.
fn follow_app_windows(device: DeviceId) {
    let (apps, screen) = state(|s| (s.has_window(&device, false), s.has_window(&device, true)));
    crate::phone_sound::for_app_windows(device, apps && !screen);
}

impl Default for State {
    fn default() -> Self {
        Self {
            windows: HashMap::new(),
            active_window: None,
            last_focus_clip: None,
            sound: HashSet::new(),
            stay_awake: HashSet::new(),
            screen_off: HashSet::new(),
            keyboard_device: None,
            notices: HashMap::new(),
            apps: HashMap::new(),
            saved: SavedWindows::load(),
        }
    }
}

/// The user turned the phones' sound off on this PC (until turned on).
static MUTED: AtomicBool = AtomicBool::new(false);
/// The next app window's session.
static NEXT_SESSION: AtomicU32 = AtomicU32::new(1);
/// Monotonic token for auto-clearing window notices.
static NOTICE_SEQ: AtomicU64 = AtomicU64::new(1);

type FrameMap = HashMap<Window, (u32, u32, Arc<Vec<u8>>)>;

/// Latest displayed BGRX frame per window, for instant screenshots.
static FRAMES: Mutex<Option<FrameMap>> = Mutex::new(None);
/// Active MP4 recorders per window.
static RECORDERS: Mutex<Option<HashMap<Window, MirrorRecorder>>> = Mutex::new(None);
/// Fast atomic count of active recorders to skip locking RECORDERS on the 60fps packet/audio hot path.
static ACTIVE_RECORDERS: AtomicUsize = AtomicUsize::new(0);
/// Latest video stream dimensions per window.
static LAST_CONFIG: Mutex<Option<HashMap<Window, (u32, u32)>>> = Mutex::new(None);
/// Latest audio stream format `(rate, channels)` per device.
static LAST_AUDIO_CONFIG: Mutex<Option<HashMap<DeviceId, (u32, u8)>>> = Mutex::new(None);
/// Latest Annex-B SPS/PPS bytes per window so mid-stream recordings have parameter sets immediately.
static LAST_SPS_PPS: Mutex<Option<HashMap<Window, Vec<u8>>>> = Mutex::new(None);

pub fn muted() -> bool {
    MUTED.load(Ordering::Relaxed)
}

pub fn set_muted(muted: bool) {
    MUTED.store(muted, Ordering::Relaxed);
    core_host::host().hub.changed(Changes::MIRROR);
    // Muted here, it plays on the phone instead (#178).
    let Some(node) = core_host::node() else { return };
    let phones: HashSet<DeviceId> = windows()
        .into_iter()
        .filter(|(window, _)| window.session == MIRROR_SCREEN)
        .map(|(window, _)| window.device)
        .collect();
    core_host::spawn(async move {
        for phone in phones {
            if let Err(e) = node.mirror_sound(phone, !muted).await {
                tracing::debug!(device = %phone.short(), error = %e, "can't move the phone's sound");
            }
        }
    });
}

/// Whether `device`'s sound is coming in.
pub fn has_sound(device: &DeviceId) -> bool {
    state(|s| s.sound.contains(device))
}

fn set_sound(device: DeviceId, on: bool) {
    state(|s| if on { s.sound.insert(device) } else { s.sound.remove(&device) });
    core_host::host().hub.changed(Changes::MIRROR);
}

/// Whether `device` is set to stay awake while mirroring.
pub fn stay_awake(device: &DeviceId) -> bool {
    state(|s| s.stay_awake.contains(device))
}

/// Whether `device`'s physical screen is turned off while mirroring.
pub fn screen_off(device: &DeviceId) -> bool {
    state(|s| s.screen_off.contains(device))
}

/// Toggles keeping the phone awake while mirroring (its screen or app windows).
pub fn set_stay_awake(window: Window, on: bool) {
    let screen_off = state(|s| {
        if on {
            s.stay_awake.insert(window.device);
        } else {
            s.stay_awake.remove(&window.device);
        }
        s.screen_off.contains(&window.device)
    });
    set_notice(
        window,
        if on { "Keeping phone awake while mirroring" } else { "Phone sleep timeout restored" },
    );
    core_host::host().hub.changed(Changes::MIRROR);
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        if let Err(e) = node.mirror_power(window.device, on, screen_off).await {
            tracing::debug!(error = %e, "mirror.power didn't reach the phone");
        }
    });
}

/// Toggles turning the phone's physical screen off while mirroring (its
/// screen or app windows).
pub fn set_screen_off(window: Window, on: bool) {
    let stay_awake = state(|s| {
        if on {
            s.screen_off.insert(window.device);
        } else {
            s.screen_off.remove(&window.device);
        }
        s.stay_awake.contains(&window.device)
    });
    remember_screen_off(window.device, on);
    set_notice(window, if on { "Phone screen turned off" } else { "Phone screen turned back on" });
    core_host::host().hub.changed(Changes::MIRROR);
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        if let Err(e) = node.mirror_power(window.device, stay_awake, on).await {
            tracing::debug!(error = %e, "mirror.power didn't reach the phone");
        }
    });
}

/// Saves whether `device`'s screen stays off from the start of each mirroring
/// or app window. The toolbar button and Settings > Devices both set this.
pub fn remember_screen_off(device: DeviceId, on: bool) {
    let phones = crate::settings::with_keep_screen_off(
        crate::settings::keep_screen_off_phones(),
        &device.to_string(),
        on,
    );
    crate::settings::set_keep_screen_off_phones(phones.clone());
    let dir = core_host::host().data_dir.clone();
    let mut saved = crate::settings::Settings::load(&dir);
    saved.keep_screen_off = phones;
    if let Err(e) = saved.save(&dir) {
        tracing::warn!(error = %e, "can't save settings");
    }
    core_host::host().hub.changed(Changes::MIRROR);
}

/// The status banner for `window`, while it's up.
pub fn last_notice(window: &Window) -> String {
    state(|s| {
        s.notices
            .get(window)
            .filter(|(_, until, _)| Instant::now() < *until)
            .map(|(text, _, _)| text.clone())
            .unwrap_or_default()
    })
}

fn set_notice(window: Window, text: impl Into<String>) {
    set_notice_for(window, text, NOTICE_DURATION);
}

fn set_notice_for(window: Window, text: impl Into<String>, duration: Duration) {
    let token = NOTICE_SEQ.fetch_add(1, Ordering::Relaxed);
    state(|s| {
        s.notices.insert(window, (text.into(), Instant::now() + duration, token));
    });
    core_host::host().hub.changed(Changes::MIRROR);
    core_host::spawn(async move {
        tokio::time::sleep(duration).await;
        let cleared = state(|s| {
            if s.notices.get(&window).is_some_and(|(_, _, t)| *t == token) {
                s.notices.remove(&window);
                true
            } else {
                false
            }
        });
        if cleared {
            core_host::host().hub.changed(Changes::MIRROR);
        }
    });
}

fn save_latest_frame(window: Window, width: u32, height: u32, bgrx: Vec<u8>) {
    FRAMES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(window, (width, height, Arc::new(bgrx)));
}

fn clear_window_buffers(window: &Window) {
    if let Some(frames) = FRAMES.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        frames.remove(window);
    }
    if let Some(cfg) = LAST_CONFIG.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        cfg.remove(window);
    }
    if let Some(sps) = LAST_SPS_PPS.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        sps.remove(window);
    }
}

fn latest_bitmap(window: &Window) -> Option<Bitmap> {
    let (width, height, raw) = FRAMES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(window))
        .map(|(w, h, b)| (*w, *h, b.clone()))?;
    if width == 0 || height == 0 {
        return None;
    }
    let mut bgra = (*raw).clone();
    for px in bgra.as_chunks_mut::<4>().0 {
        px[3] = 0xFF;
    }
    Some(Bitmap { width, height, bgra })
}

/// Copies the current mirror frame for `window` to the PC clipboard as PNG.
pub fn screenshot_clipboard(window: Window) {
    let Some(bitmap) = latest_bitmap(&window) else {
        set_notice(window, "Wait for the screen to appear first");
        show_message(tr!("Mirror", "Wait for the screen to appear first."));
        return;
    };
    let copied = encode_png(&bitmap)
        .map_err(|e| e.to_string())
        .and_then(|png| crate::win::clipboard::write_image("image/png", &png));
    match copied {
        Ok(()) => {
            set_notice(window, "Screenshot copied to clipboard");
            show_message(tr!("Mirror", "Screenshot copied to clipboard."));
        }
        Err(e) => {
            tracing::warn!(error = %e, "can't copy mirror screenshot");
            set_notice(window, "Couldn't copy screenshot");
            show_message(tr!("Mirror", "The screenshot couldn't be copied to the clipboard."));
        }
    }
}

/// Saves the current mirror frame for `window` to `Downloads\Nectarlink` as a PNG file.
pub fn screenshot_file(window: Window) {
    let Some(bitmap) = latest_bitmap(&window) else {
        set_notice(window, "Wait for the screen to appear first");
        show_message(tr!("Mirror", "Wait for the screen to appear first."));
        return;
    };
    let png = match encode_png(&bitmap) {
        Ok(png) => png,
        Err(e) => {
            tracing::warn!(error = %e, "can't encode mirror screenshot");
            set_notice(window, "Couldn't save screenshot");
            show_message(tr!("Mirror", "The screenshot couldn't be saved."));
            return;
        }
    };
    let base_name = local_timestamp_filename("Screenshot", "png");
    let dir = core_host::downloads_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        let subdir = if cfg!(windows) { "Downloads\\Nectarlink" } else { "Downloads/Nectarlink" };
        tracing::warn!(error = %e, "can't create {subdir}");
        set_notice(window, "Couldn't save screenshot");
        show_message(tr!("Mirror", "The screenshot couldn't be saved."));
        return;
    }
    let path = crate::photos::free_path(&dir, &base_name);
    if let Err(e) = std::fs::write(&path, &png) {
        tracing::warn!(error = %e, "can't write mirror screenshot");
        set_notice(window, "Couldn't save screenshot");
        show_message(tr!("Mirror", "The screenshot couldn't be saved."));
        return;
    }
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or(&base_name).to_owned();
    if let Some(node) = core_host::node() {
        node.record_timeline(
            nectarlink_core::TimelineKind::Photo,
            window.device,
            true,
            file_name.clone(),
            tr!("Mirror", "Mirror screenshot"),
            path.to_string_lossy().into_owned(),
            png.len() as u64,
            0,
            None,
        );
    }
    let device_name =
        core_host::host().hub.read(|s| s.name_of(&window.device)).unwrap_or_else(|| "your phone".into());
    let toast_body = if cfg!(windows) {
        tr!("Mirror", "Screenshot from %1, saved in Downloads\\Nectarlink", device_name)
    } else {
        tr!("Mirror", "Screenshot from %1, saved in Downloads/Nectarlink", device_name)
    };
    toast::show(Toast {
        device: TOAST_GROUP.into(),
        key: path.to_string_lossy().into_owned(),
        title: file_name.clone(),
        body: toast_body,
        attribution: "NectarLink".into(),
        icon: None,
        image: Some(path),
        actions: vec![(ACTION_OPEN.into(), "Open".into()), (ACTION_SHOW.into(), "Show in folder".into())],
        reply: None,
        silent: false,
        progress: None,
        call: false,
    });
    set_notice(window, tr!("Mirror", "Saved %1", file_name));
    let saved_msg = if cfg!(windows) {
        tr!("Mirror", "Saved %1 to Downloads\\Nectarlink.", file_name)
    } else {
        tr!("Mirror", "Saved %1 to Downloads/Nectarlink.", file_name)
    };
    show_message(saved_msg);
}

/// Whether `window` is currently recording to an MP4 file.
pub fn is_recording(window: &Window) -> bool {
    ACTIVE_RECORDERS.load(Ordering::Relaxed) > 0
        && RECORDERS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|m| m.contains_key(window))
}

/// Unix epoch milliseconds when the active recording for `window` started (`0` if not recording).
pub fn recording_started_ms(window: &Window) -> i64 {
    if ACTIVE_RECORDERS.load(Ordering::Relaxed) == 0 {
        return 0;
    }
    RECORDERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(window))
        .map_or(0, MirrorRecorder::started_epoch_ms)
}

/// Starts or stops MP4 recording for `window`.
pub fn toggle_recording(window: Window) {
    if is_recording(&window) {
        stop_recording(window);
        return;
    }
    if !matches!(phase(&window), Some(Phase::Showing)) {
        set_notice(window, "Wait for the screen to appear first");
        show_message(tr!("Mirror", "Wait for the screen to appear first."));
        return;
    }
    let base_name = local_timestamp_filename("Mirror", "mp4");
    let dir = core_host::downloads_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        let subdir = if cfg!(windows) { "Downloads\\Nectarlink" } else { "Downloads/Nectarlink" };
        tracing::warn!(error = %e, "can't create {subdir}");
        set_notice(window, "Couldn't start recording");
        show_message(tr!("Mirror", "Couldn't start recording."));
        return;
    }
    let path = crate::photos::free_path(&dir, &base_name);
    let (width, height) = LAST_CONFIG
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(&window).copied())
        .or_else(|| {
            FRAMES
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .and_then(|m| m.get(&window).map(|(w, h, _)| (*w, *h)))
        })
        .unwrap_or((0, 0));

    let audio_cfg = if window.session == MIRROR_SCREEN && has_sound(&window.device) {
        LAST_AUDIO_CONFIG
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|m| m.get(&window.device).copied())
            .or(Some((48_000, 2)))
    } else {
        None
    };

    let mut recorder = MirrorRecorder::new(path, width, height, OPTIONS.fps, audio_cfg);
    if let Some(sps_pps) =
        LAST_SPS_PPS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&window))
    {
        recorder.set_sps_pps(sps_pps);
    }

    {
        let mut guard = RECORDERS.lock().unwrap_or_else(|e| e.into_inner());
        let map = guard.get_or_insert_with(HashMap::new);
        map.insert(window, recorder);
        ACTIVE_RECORDERS.store(map.len(), Ordering::Release);
    }

    set_notice(window, "Recording started");
    core_host::host().hub.changed(Changes::MIRROR);
    if let Some(node) = core_host::node() {
        core_host::spawn(async move { node.mirror_keyframe(window.device, window.session).await });
    }
}

/// Stops and finalizes any active MP4 recording for `window`.
pub fn stop_recording(window: Window) {
    let recorder = {
        let mut guard = RECORDERS.lock().unwrap_or_else(|e| e.into_inner());
        let rec = guard.as_mut().and_then(|m| m.remove(&window));
        ACTIVE_RECORDERS.store(guard.as_ref().map_or(0, HashMap::len), Ordering::Release);
        rec
    };
    let Some(recorder) = recorder else { return };
    core_host::host().hub.changed(Changes::MIRROR);
    let duration_secs = recorder.started_at().elapsed().as_secs().max(1);

    core_host::spawn(async move {
        let res = tokio::task::spawn_blocking(move || recorder.finish()).await;
        match res {
            Ok(Ok(path)) => {
                let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("Mirror.mp4").to_owned();
                let size_bytes = std::fs::metadata(&path).map_or(0, |m| m.len());
                if let Some(node) = core_host::node() {
                    node.record_timeline(
                        nectarlink_core::TimelineKind::Recording,
                        window.device,
                        true,
                        file_name.clone(),
                        tr!("Mirror", "Mirror recording"),
                        path.to_string_lossy().into_owned(),
                        size_bytes,
                        duration_secs,
                        None,
                    );
                }
                let device_name = core_host::host()
                    .hub
                    .read(|s| s.name_of(&window.device))
                    .unwrap_or_else(|| "your phone".into());
                let toast_body = if cfg!(windows) {
                    tr!("Mirror", "Recording from %1, saved in Downloads\\Nectarlink", device_name)
                } else {
                    tr!("Mirror", "Recording from %1, saved in Downloads/Nectarlink", device_name)
                };
                toast::show(Toast {
                    device: TOAST_GROUP.into(),
                    key: path.to_string_lossy().into_owned(),
                    title: file_name.clone(),
                    body: toast_body,
                    attribution: "NectarLink".into(),
                    icon: None,
                    image: None,
                    actions: vec![
                        (ACTION_OPEN.into(), "Open".into()),
                        (ACTION_SHOW.into(), tr!("Mirror", "Show in folder")),
                    ],
                    reply: None,
                    silent: false,
                    progress: None,
                    call: false,
                });
                set_notice(window, tr!("Mirror", "Saved %1", file_name));
                let saved_msg = if cfg!(windows) {
                    tr!("Mirror", "Saved %1 to Downloads\\Nectarlink.", file_name)
                } else {
                    tr!("Mirror", "Saved %1 to Downloads/Nectarlink.", file_name)
                };
                show_message(saved_msg);
            }
            Ok(Err(msg)) => {
                set_notice(window, msg.clone());
                show_message(msg);
            }
            Err(e) => {
                tracing::warn!(error = %e, "mirror recording finalize panicked");
            }
        }
    });
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn state<T>(f: impl FnOnce(&mut State) -> T) -> T {
    f(STATE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_default())
}

fn set_phase(window: Window, phase: Phase) {
    let showing = matches!(phase, Phase::Showing);
    state(|s| {
        if showing && s.active_window.is_none() {
            s.active_window = Some(window);
        } else if !showing && s.active_window == Some(window) {
            s.active_window = s
                .windows
                .iter()
                .find(|(w, shown)| **w != window && matches!(shown.phase, Phase::Showing))
                .map(|(w, _)| *w);
        }
        if let Some(shown) = s.windows.get_mut(&window) {
            shown.phase = phase;
        }
    });
    if showing {
        ensure_gamepad_loop();
    }
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

/// Where the phone's own screen keeps its window's place and size, beside its
/// apps': not a package name, so no app can have it.
pub const SCREEN_GEOMETRY: &str = "#screen";

/// Remembered window position and size for `(device, pkg)` (or the phone's
/// screen, [`SCREEN_GEOMETRY`]).
pub fn geometry(device: &DeviceId, pkg: &str) -> Option<WindowGeometry> {
    state(|s| s.saved.geometry.get(&format!("{device}/{pkg}")).copied())
}

/// Whether an app window's display follows the window's size (on unless
/// turned off for its app).
pub fn flexible(window: &Window) -> bool {
    window.session != MIRROR_SCREEN
        && state(|s| {
            let pkg = s.windows.get(window).and_then(|w| w.pkg.clone());
            pkg.is_none_or(|pkg| !s.saved.fixed.contains(&format!("{}/{pkg}", window.device)))
        })
}

/// Turns an app window's flexible display on or off, for its app. Off, the
/// app starts again on a display of the phone's resolution; on, the window's
/// next resize (which the window sends) gives the display its size.
pub fn set_flexible(window: Window, on: bool) {
    if window.session == MIRROR_SCREEN || flexible(&window) == on {
        return;
    }
    state(|s| {
        let Some(pkg) = s.windows.get(&window).and_then(|w| w.pkg.clone()) else { return };
        let key = format!("{}/{pkg}", window.device);
        if on {
            s.saved.fixed.remove(&key);
        } else {
            s.saved.fixed.insert(key);
        }
        s.saved.save();
    });
    core_host::host().hub.changed(Changes::MIRROR);
    if !on {
        restart(window);
    }
}

/// Starts an app window's app again on a new display. The phone takes the
/// same session for the new one, so the old one is stopped first and the
/// new asked for once it has ended (or after a while, if no end arrives):
/// otherwise the old stream's end ended the new window.
fn restart(window: Window) {
    let started = state(|s| {
        let shown = s.windows.get_mut(&window)?;
        shown.restarting = true;
        shown.phase = Phase::Asking;
        Some(())
    });
    if started.is_none() {
        return;
    }
    stop_recording(window);
    clear_window_buffers(&window);
    ffi::video_clear(&window.key());
    core_host::host().hub.changed(Changes::MIRROR);
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        node.mirror_stop(window.device, window.session).await;
        tokio::time::sleep(RESTART_FALLBACK).await;
        finish_restart(window);
    });
}

/// How long a restart waits for the old stream's end before asking anyway.
const RESTART_FALLBACK: std::time::Duration = std::time::Duration::from_secs(2);

/// Asks for a restarting window again, once (its old stream ended, or the
/// fallback ran out); false if it isn't restarting.
fn finish_restart(window: Window) -> bool {
    let was =
        state(|s| s.windows.get_mut(&window).is_some_and(|w| std::mem::replace(&mut w.restarting, false)));
    if was {
        reopen(window);
    }
    was
}

/// How big an app window's app draws, in percent (100 unless changed).
pub fn zoom(window: &Window) -> u32 {
    state(|s| {
        let pkg = s.windows.get(window).and_then(|w| w.pkg.as_deref())?;
        s.saved.zoom.get(&format!("{}/{pkg}", window.device)).copied()
    })
    .unwrap_or(100)
}

/// One step of zoom.
const ZOOM_STEP: u32 = 10;

/// `zoom` moved by `steps` of the mouse wheel, within what the phone takes.
fn zoomed(zoom: u32, steps: i32) -> u32 {
    let range = nectarlink_core::MirrorResize::ZOOM;
    zoom.saturating_add_signed(steps.saturating_mul(ZOOM_STEP as i32)).clamp(*range.start(), *range.end())
}

/// Makes an app window's app draw bigger or smaller, for its app, by `steps`
/// of the mouse wheel; the window then applies it (`resize`, or
/// `zoom_display` when it isn't flexible). False when nothing changed (it's
/// at its limit already).
pub fn change_zoom(window: Window, steps: i32) -> bool {
    let old = zoom(&window);
    let new = zoomed(old, steps);
    if new == old {
        return false;
    }
    let saved = state(|s| {
        let pkg = s.windows.get(&window).and_then(|w| w.pkg.clone())?;
        let key = format!("{}/{pkg}", window.device);
        if new == 100 {
            s.saved.zoom.remove(&key);
        } else {
            s.saved.zoom.insert(key, new);
        }
        s.saved.save();
        Some(())
    });
    if saved.is_some() {
        set_notice(window, format!("{new}%"));
    }
    saved.is_some()
}

/// Saves a window's position and size on this PC: an app's, or the phone
/// screen's (#358), which the next one opens at.
pub fn save_geometry(window: Window, x: i32, y: i32, width: u32, height: u32) {
    if width < 200 || height < 200 {
        return;
    }
    state(|s| {
        let pkg = if window.session == MIRROR_SCREEN {
            Some(SCREEN_GEOMETRY.to_owned())
        } else {
            s.windows.get(&window).and_then(|w| w.pkg.clone())
        };
        let Some(pkg) = pkg else { return };
        let key = format!("{}/{pkg}", window.device);
        let geom = WindowGeometry { x, y, width, height };
        if s.saved.geometry.insert(key, geom) != Some(geom) {
            s.saved.save();
        }
    });
}

/// Package names of recent apps on `device` (apps opened from this PC first,
/// followed by apps with active or recent notifications), most recent first.
pub fn recent_apps(device: &DeviceId) -> Vec<String> {
    let saved = state(|s| s.saved.recent.get(&device.to_string()).cloned().unwrap_or_default());
    let (notif_pkgs, hist_pkgs) = core_host::host().hub.read(|s| {
        let active: Vec<String> = s
            .notifications
            .iter()
            .filter(|n| &n.device == device && !n.notification.app.is_empty())
            .map(|n| n.notification.app.clone())
            .collect();
        let history: Vec<String> = s
            .history
            .iter()
            .filter(|h| &h.device == device && !h.notification.app.is_empty())
            .map(|h| h.notification.app.clone())
            .collect();
        (active, history)
    });
    merge_recent_apps(&saved, notif_pkgs, hist_pkgs)
}

pub(crate) fn merge_recent_apps(
    saved: &[String],
    active_notif_pkgs: impl IntoIterator<Item = String>,
    history_pkgs: impl IntoIterator<Item = String>,
) -> Vec<String> {
    let mut out = Vec::with_capacity(12);
    for pkg in saved.iter().cloned().chain(active_notif_pkgs).chain(history_pkgs) {
        if !pkg.is_empty() && !out.contains(&pkg) {
            out.push(pkg);
            if out.len() >= 12 {
                break;
            }
        }
    }
    out
}

fn record_recent(s: &mut State, device: &DeviceId, pkg: &str) {
    let list = s.saved.recent.entry(device.to_string()).or_default();
    list.retain(|p| p != pkg);
    list.insert(0, pkg.to_owned());
    list.truncate(12);
    s.saved.save();
}

/// What `device` is set to while mirroring: `(stay_awake, screen_off)`.
fn power(s: &State, device: &DeviceId) -> (bool, bool) {
    (s.stay_awake.contains(device), s.screen_off.contains(device))
}

/// The Android release in a phone's OS version (`"17.1"` → 17).
fn android_release(os_ver: &str) -> Option<u32> {
    os_ver.split('.').next()?.trim().parse().ok()
}

/// An app window keeps running while the phone sleeps from Android 14 on
/// (its display is in a power group of its own, the helper's `VirtualDevice`),
/// so keeping the phone awake or its screen off does nothing for it. Unknown
/// versions count as older.
fn runs_asleep(android: Option<u32>) -> bool {
    android.is_some_and(|v| v >= 14)
}

/// Whether the phone's power matters to `window`: its screen always, an app
/// window only where it sleeps with the phone. Where it doesn't, the window
/// has no power buttons and asks nothing of the phone's power.
pub fn power_matters(window: &Window) -> bool {
    window.session == MIRROR_SCREEN
        || !runs_asleep(
            core_host::host()
                .hub
                .read(|s| s.device(&window.device).and_then(|d| android_release(&d.info.os_ver))),
        )
}

/// [`power`] for an app window: nothing where the phone's power doesn't matter to it.
fn app_power(window: &Window) -> (bool, bool) {
    if power_matters(window) { state(|s| power(s, &window.device)) } else { (false, false) }
}

/// Asks the phone for its screen.
pub fn start(device: DeviceId) {
    if keyboard_device() == Some(device) {
        set_remote_keyboard(None);
    }
    let window = Window::screen(device);
    ffi::video_clear(&window.key());
    let (stay_awake, screen_off) = state(|s| {
        // The phone's screen starts as it's set to, unless app windows already turned it off.
        s.reset_screen_off(&device);
        s.windows.insert(window, Shown { phase: Phase::Asking, app: None, pkg: None, restarting: false });
        power(s, &device)
    });
    let options = screen_options(crate::settings::mirror_quality(), on_relay(&device));
    set_unproven(device, options.max_size != OPTIONS.max_size);
    request(window, MirrorStart { stay_awake, screen_off, ..options });
}

/// Phone currently receiving PC keyboard input without screen mirroring.
pub fn keyboard_device() -> Option<DeviceId> {
    state(|s| s.keyboard_device)
}

/// Starts or stops remote keyboard typing on `device` without screen mirroring.
pub fn set_remote_keyboard(device: Option<DeviceId>) {
    let prev = state(|s| {
        let prev = s.keyboard_device;
        s.keyboard_device = device;
        prev
    });
    if prev == device {
        return;
    }
    if let Some(old) = prev {
        input(Window::screen(old), nectarlink_core::MirrorInput::Key { key: "keyboard_off".into() });
    }
    if let Some(new) = device {
        input(Window::screen(new), nectarlink_core::MirrorInput::Key { key: "keyboard_on".into() });
    }
    core_host::host().hub.changed(Changes::MIRROR);
}

/// Toggles remote keyboard typing on `device` without screen mirroring.
pub fn toggle_remote_keyboard(device: DeviceId) {
    if keyboard_device() == Some(device) {
        set_remote_keyboard(None);
    } else {
        set_remote_keyboard(Some(device));
    }
}

/// Sends a special key (`"enter"`, `"backspace"`, `"delete"`, `"left"`, `"right"`, `"up"`, `"down"`, `"tab"`, `"back"`, `"home"`)
/// to `device` while remote keyboard mode is active.
pub fn keyboard_press(device: DeviceId, key_name: String) {
    if keyboard_device() == Some(device) {
        input(Window::screen(device), nectarlink_core::MirrorInput::Key { key: key_name });
    }
}

/// Sends typed text to `device` while remote keyboard mode is active, splitting into protocol-sized chunks.
pub fn keyboard_text(device: DeviceId, text: String) {
    if keyboard_device() != Some(device) || text.is_empty() {
        return;
    }
    let window = Window::screen(device);
    let mut piece = String::new();
    for c in text.chars() {
        if piece.len() + c.len_utf8() > nectarlink_core::MIRROR_MAX_TEXT_BYTES {
            input(window, nectarlink_core::MirrorInput::Text { text: std::mem::take(&mut piece) });
        }
        piece.push(c);
    }
    if !piece.is_empty() {
        input(window, nectarlink_core::MirrorInput::Text { text: piece });
    }
}

/// Pastes the PC's current clipboard text onto `device` while remote keyboard mode is active.
pub fn keyboard_paste(device: DeviceId) {
    if keyboard_device() != Some(device) {
        return;
    }
    if let crate::win::clipboard::Clip::Text { text, .. } = crate::win::clipboard::read_for_paste() {
        keyboard_text(device, text);
    }
}

/// Opens one of the phone's apps in a window of its own (or does nothing
/// when it's already open; reuses an ended window for that app).
pub fn start_app(device: DeviceId, pkg: String, label: String) {
    let window = state(|s| {
        if s.windows.iter().any(|(w, shown)| {
            w.device == device
                && shown.pkg.as_deref() == Some(pkg.as_str())
                && matches!(shown.phase, Phase::Asking | Phase::Showing)
        }) {
            return None;
        }
        record_recent(s, &device, &pkg);
        let ended = s
            .windows
            .iter()
            .find(|(w, shown)| w.device == device && shown.pkg.as_deref() == Some(pkg.as_str()))
            .map(|(w, _)| *w);
        let window =
            ended.unwrap_or_else(|| Window { device, session: NEXT_SESSION.fetch_add(1, Ordering::Relaxed) });
        s.reset_screen_off(&device);
        s.windows.insert(
            window,
            Shown { phase: Phase::Asking, app: Some(label), pkg: Some(pkg.clone()), restarting: false },
        );
        Some(window)
    });
    let Some(window) = window else { return };
    let (stay_awake, screen_off) = app_power(&window);
    let options = app_options(crate::settings::mirror_quality(), on_relay(&device));
    ffi::video_clear(&window.key());
    request(
        window,
        MirrorStart {
            audio: false,
            session: window.session,
            app: Some(pkg),
            stay_awake,
            screen_off,
            ..options
        },
    );
}

/// Reopens a mirroring window that ended (the screen or an app window).
pub fn reopen(window: Window) {
    if window.session == MIRROR_SCREEN {
        start(window.device);
        return;
    }
    let pkg = state(|s| {
        s.reset_screen_off(&window.device);
        let shown = s.windows.get_mut(&window)?;
        let pkg = shown.pkg.clone()?;
        shown.phase = Phase::Asking;
        record_recent(s, &window.device, &pkg);
        Some(pkg)
    });
    let Some(pkg) = pkg else { return };
    let (stay_awake, screen_off) = app_power(&window);
    let options = app_options(crate::settings::mirror_quality(), on_relay(&window.device));
    ffi::video_clear(&window.key());
    request(
        window,
        MirrorStart {
            audio: false,
            session: window.session,
            app: Some(pkg),
            stay_awake,
            screen_off,
            ..options
        },
    );
}

/// Resizes an app window's display on the phone (`width` and `height` are the
/// window's logical dimensions on the PC).
pub fn resize(window: Window, width: u32, height: u32) {
    if window.session == MIRROR_SCREEN || width < 120 || height < 120 || !flexible(&window) {
        return;
    }
    let (mut w, mut h) = (width.saturating_mul(2), height.saturating_mul(2));
    let longest = w.max(h);
    if longest > OPTIONS.max_size {
        w = (u64::from(w) * u64::from(OPTIONS.max_size) / u64::from(longest)) as u32;
        h = (u64::from(h) * u64::from(OPTIONS.max_size) / u64::from(longest)) as u32;
    }
    let w = (w & !1).clamp(240, 4096);
    let h = (h & !1).clamp(240, 4096);
    let zoom = zoom(&window);
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        if let Err(e) = node.mirror_resize(window.device, window.session, w, h, zoom).await {
            tracing::debug!(error = %e, "resize didn't reach the phone");
        }
    });
}

/// Applies an app window's zoom to a display that keeps its size (flexible
/// display off): `width` x `height` are the picture's own pixels, so the
/// phone changes nothing but how big the app draws.
pub fn zoom_display(window: Window, width: u32, height: u32) {
    if window.session == MIRROR_SCREEN || flexible(&window) {
        return;
    }
    let zoom = zoom(&window);
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        if let Err(e) = node.mirror_resize(window.device, window.session, width, height, zoom).await {
            tracing::debug!(error = %e, "zoom didn't reach the phone");
        }
    });
}

/// Each request's number per window, so an earlier request's timer can't
/// end a retry of the same window.
static ATTEMPTS: Mutex<Option<HashMap<Window, u64>>> = Mutex::new(None);

fn next_attempt(window: Window) -> u64 {
    let mut attempts = ATTEMPTS.lock().unwrap_or_else(|e| e.into_inner());
    let n = attempts.get_or_insert_with(HashMap::new).entry(window).or_default();
    *n += 1;
    *n
}

fn is_latest_attempt(window: Window, attempt: u64) -> bool {
    ATTEMPTS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|a| a.get(&window)) == Some(&attempt)
}

fn request(window: Window, mut options: MirrorStart) {
    // An app window's start can't carry these (`MirrorStart::is_valid`), so
    // asking with them failed every app window after one was turned on
    // (#124). The phone takes them with `mirror.power` once the window shows
    // (see `on_event`).
    if options.app.is_some() {
        options.stay_awake = false;
        options.screen_off = false;
    }
    if muted() {
        options.audio = false;
    }
    let attempt = next_attempt(window);
    if window.session == MIRROR_SCREEN {
        crate::phone_sound::stop_for_window(window.device);
    }
    core_host::host().hub.changed(Changes::MIRROR);
    let Some(node) = core_host::node() else { return };
    let app = options.app.is_some();
    core_host::spawn(async move {
        if let Err(e) = node.mirror_start(window.device, options).await {
            let reason = match e {
                Error::Denied => tr!("Mirror", "Mirroring is turned off for this phone."),
                Error::Unsupported if app => tr!(
                    "Mirror",
                    "Set up Wireless debugging in NectarLink on the phone to open its apps here."
                ),
                Error::Unsupported => {
                    tr!(
                        "Mirror",
                        "This phone's app doesn't share its screen yet. Update NectarLink on the phone."
                    )
                }
                Error::Offline | Error::NotPaired => tr!("Mirror", "The phone isn't connected."),
                e => describe(&e),
            };
            set_phase(window, Phase::Ended(Some(reason)));
            return;
        }
        // An app window has no prompt to wait for on the phone: its first frame comes within
        // seconds, or the phone couldn't start it (its stream then ends with no picture).
        if app {
            tokio::time::sleep(APP_START_TIMEOUT).await;
            let stuck = is_latest_attempt(window, attempt)
                && state(|s| s.windows.get(&window).is_some_and(|w| w.phase == Phase::Asking));
            if stuck {
                tracing::warn!(session = window.session, "an app window never started on the phone");
                node.mirror_stop(window.device, window.session).await;
                set_phase(
                    window,
                    Phase::Ended(Some(tr!(
                        "Mirror",
                        "The app didn't open on the phone. Try again, or open NectarLink on the phone to check Wireless debugging."
                    ))),
                );
            }
        }
    });
}

#[cfg(test)]
mod quality_tests {
    use super::*;

    #[test]
    fn app_windows_from_android_14_run_while_the_phone_sleeps() {
        assert!(runs_asleep(android_release("17.1")));
        assert!(runs_asleep(android_release("14")));
        assert!(!runs_asleep(android_release("13")));
        assert!(!runs_asleep(android_release("")));
    }

    #[test]
    fn zoom_moves_in_steps_and_stops_at_what_the_phone_takes() {
        assert_eq!(zoomed(100, 1), 110);
        assert_eq!(zoomed(100, -2), 80);
        assert_eq!(zoomed(190, 3), 200);
        assert_eq!(zoomed(60, -5), 50);
    }

    #[test]
    fn app_windows_apply_smoother_and_keep_android_size_limits() {
        let smoother = app_options(Quality::Smoother, false);
        assert_eq!((smoother.max_size, smoother.bitrate, smoother.fps), (1280, 4_000_000, 60));
        let sharper = app_options(Quality::Sharper, false);
        assert_eq!((sharper.max_size, sharper.bitrate), (1920, 20_000_000));
        assert_eq!(app_options(Quality::Smoother, true).max_size, OPTIONS.max_size);
    }

    #[test]
    fn balanced_is_exactly_what_mirroring_always_asked_for() {
        assert_eq!(OPTIONS.max_size, 1920);
        assert_eq!(OPTIONS.fps, 60);
        assert_eq!(OPTIONS.bitrate, 8_000_000);
        let balanced = screen_options(Quality::Balanced, false);
        assert_eq!((balanced.max_size, balanced.fps, balanced.bitrate), (1920, 60, 8_000_000));
    }

    #[test]
    fn a_settings_file_without_a_quality_or_with_an_unknown_one_is_balanced() {
        assert_eq!(Quality::parse(""), Quality::Balanced);
        assert_eq!(Quality::parse("ultra"), Quality::Balanced);
        for quality in [Quality::Balanced, Quality::Sharper, Quality::Smoother] {
            assert_eq!(Quality::parse(quality.name()), quality);
        }
    }

    #[test]
    fn the_relay_always_gets_balanced_and_every_option_keeps_the_frame_rate() {
        for quality in [Quality::Sharper, Quality::Smoother] {
            let relayed = screen_options(quality, true);
            assert_eq!((relayed.max_size, relayed.bitrate), (OPTIONS.max_size, OPTIONS.bitrate));
            assert_eq!(screen_options(quality, false).fps, OPTIONS.fps);
            assert!(screen_options(quality, false).is_valid());
        }
        assert!(screen_options(Quality::Sharper, false).max_size > OPTIONS.max_size);
        assert!(screen_options(Quality::Smoother, false).bitrate < OPTIONS.bitrate);
    }

    #[test]
    fn only_a_started_stream_with_no_picture_asks_again() {
        assert!(retry_at_balanced(true, true));
        assert!(!retry_at_balanced(false, true), "it showed a picture, or asked for Balanced");
        assert!(!retry_at_balanced(true, false), "declined on the phone, or closed here");
    }
}

/// How long an app window may take to show its first frame.
const APP_START_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// Stops a mirroring (its window closed).
pub fn stop(window: Window) {
    stop_recording(window);
    clear_window_buffers(&window);
    state(|s| {
        s.windows.remove(&window);
        s.notices.remove(&window);
        if !s.mirroring(&window.device) {
            s.screen_off.remove(&window.device);
        }
    });
    ffi::video_clear(&window.key());
    core_host::host().hub.changed(Changes::MIRROR);
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        node.mirror_stop(window.device, window.session).await;
        // After the stop, so a screen window's sound is gone before the app
        // windows ask for it again.
        follow_app_windows(window.device);
    });
}

pub fn on_event(event: &NodeEvent) {
    match event {
        NodeEvent::MirrorNotice { device, session, code } => {
            if code == nectarlink_core::MIRROR_NOTICE_INPUT_BLOCKED {
                let window = Window { device: *device, session: *session };
                set_notice_for(
                    window,
                    tr!(
                        "Mirror",
                        "The phone ignores taps from this PC. In its Developer options, turn on USB debugging (Security settings)."
                    ),
                    Duration::from_secs(12),
                );
            }
        }
        NodeEvent::Mirroring { device, session, on: true } => {
            let window = Window { device: *device, session: *session };
            set_phase(window, Phase::Showing);
            if *session != MIRROR_SCREEN {
                follow_app_windows(*device);
                // Keep awake / screen off, as set for this phone, apply to its app windows too.
                let (stay_awake, screen_off) = app_power(&window);
                if (stay_awake || screen_off)
                    && let Some(node) = core_host::node()
                {
                    let device = *device;
                    core_host::spawn(async move {
                        if let Err(e) = node.mirror_power(device, stay_awake, screen_off).await {
                            tracing::debug!(error = %e, "mirror.power didn't reach the phone");
                        }
                    });
                }
                let saved = state(|s| {
                    let pkg = s.windows.get(&window).and_then(|w| w.pkg.as_deref())?;
                    s.saved.geometry.get(&format!("{device}/{pkg}")).copied()
                });
                if let Some(geom) = saved {
                    resize(window, geom.width, geom.height);
                }
            }
        }
        // Closed here: nothing to say. Stopped on the phone: say so.
        NodeEvent::Mirroring { device, session, on: false } => {
            let window = Window { device: *device, session: *session };
            stop_recording(window);
            clear_window_buffers(&window);
            if *session == MIRROR_SCREEN {
                let cleared_kb = state(|s| {
                    if s.keyboard_device == Some(*device) {
                        s.keyboard_device = None;
                        true
                    } else {
                        false
                    }
                });
                if cleared_kb {
                    core_host::host().hub.changed(Changes::MIRROR);
                }
            }
            if finish_restart(window) {
                return;
            }
            let showing = matches!(phase(&window), Some(Phase::Showing));
            let unproven = *session == MIRROR_SCREEN && set_unproven(*device, false);
            if retry_at_balanced(unproven, showing) {
                tracing::warn!("the phone couldn't mirror at this quality; asking for Balanced");
                let (stay_awake, screen_off) = state(|s| power(s, device));
                set_phase(window, Phase::Asking);
                set_notice(window, "This phone can't mirror at that quality, so it's using Balanced");
                request(window, MirrorStart { stay_awake, screen_off, ..OPTIONS });
                return;
            }
            if phase(&window).is_some() {
                set_phase(window, Phase::Ended(None));
            }
            state(|s| {
                if !s.mirroring(device) {
                    s.screen_off.remove(device);
                }
            });
            // The phone's sound plays here alone with session 0 too: its own
            // ending must not ask for it again.
            if *session != MIRROR_SCREEN {
                follow_app_windows(*device);
            }
        }
        // A phone that went away takes its app list with it.
        NodeEvent::LinkChanged { device, link: nectarlink_core::LinkState::Offline { .. } } => {
            let cleared_kb = state(|s| {
                s.apps.remove(device);
                s.screen_off.remove(device);
                if s.keyboard_device == Some(*device) {
                    s.keyboard_device = None;
                    true
                } else {
                    false
                }
            });
            if cleared_kb {
                core_host::host().hub.changed(Changes::MIRROR);
            }
        }
        NodeEvent::DeviceRemoved(id) => {
            let cleared_kb = state(|s| {
                s.apps.remove(id);
                s.screen_off.remove(id);
                if s.keyboard_device == Some(*id) {
                    s.keyboard_device = None;
                    true
                } else {
                    false
                }
            });
            if cleared_kb {
                core_host::host().hub.changed(Changes::MIRROR);
            }
        }
        _ => {}
    }
}

/// A phone's apps that open in windows, as last loaded.
pub fn apps(device: &DeviceId) -> Option<Apps> {
    state(|s| s.apps.get(device).cloned())
}

/// Loads (or reloads) a phone's apps, with their icons. A list already
/// loaded (this run, or saved by an earlier one) shows at once and stays up
/// while the phone sends a fresh one: only the first load ever waits.
pub fn load_apps(device: DeviceId) {
    let data_dir = core_host::host().data_dir.clone();
    let shown = state(|s| match s.apps.get(&device) {
        Some(Apps::Ready(_)) => true,
        _ => match saved_apps(&data_dir, &device) {
            Some(saved) => {
                s.apps.insert(device, Apps::Ready(saved));
                true
            }
            None => {
                s.apps.insert(device, Apps::Loading);
                false
            }
        },
    });
    core_host::host().hub.changed(Changes::MIRROR);
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        let started = Instant::now();
        let loaded = match node.mirror_apps(device).await {
            Ok(apps) => {
                let apps = tokio::task::spawn_blocking(move || {
                    let apps: Vec<App> = apps
                        .into_iter()
                        .map(|a| App {
                            icon: a.icon.and_then(|png| crate::icons::save(&data_dir, &a.pkg, &png).ok()),
                            pkg: a.pkg,
                            label: a.label,
                        })
                        .collect();
                    save_apps(&data_dir, &device, &apps);
                    apps
                })
                .await
                .unwrap_or_default();
                tracing::info!(
                    apps = apps.len(),
                    ms = started.elapsed().as_millis() as u64,
                    "loaded a phone's apps"
                );
                Apps::Ready(apps)
            }
            // A list on screen stays: it is a moment old, not wrong.
            Err(e) if shown => {
                tracing::info!(error = %e, "can't refresh a phone's apps; keeping the last list");
                return;
            }
            Err(Error::Unsupported) => Apps::Failed(tr!(
                "Mirror",
                "Set up Wireless debugging in NectarLink on the phone to open its apps here."
            )),
            Err(e) => Apps::Failed(describe(&e)),
        };
        state(|s| s.apps.insert(device, loaded));
        core_host::host().hub.changed(Changes::MIRROR);
    });
}

fn saved_apps_path(data_dir: &Path, device: &DeviceId) -> PathBuf {
    data_dir.join("cache").join("apps").join(format!("{device}.json"))
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SavedApp {
    pkg: String,
    label: String,
}

/// The phone's apps as an earlier run saved them, with the icons it kept.
fn saved_apps(data_dir: &Path, device: &DeviceId) -> Option<Vec<App>> {
    let bytes = std::fs::read(saved_apps_path(data_dir, device)).ok()?;
    let saved: Vec<SavedApp> = serde_json::from_slice(&bytes).ok()?;
    let apps: Vec<App> = saved
        .into_iter()
        .filter(|a| nectarlink_core::is_package_name(&a.pkg))
        .map(|a| App { icon: crate::icons::existing(data_dir, &a.pkg), pkg: a.pkg, label: a.label })
        .collect();
    (!apps.is_empty()).then_some(apps)
}

fn save_apps(data_dir: &Path, device: &DeviceId, apps: &[App]) {
    let path = saved_apps_path(data_dir, device);
    let saved: Vec<SavedApp> =
        apps.iter().map(|a| SavedApp { pkg: a.pkg.clone(), label: a.label.clone() }).collect();
    let written = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| {
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(&saved).map_err(std::io::Error::other)?)?;
        std::fs::rename(&tmp, &path)
    });
    if let Err(e) = written {
        tracing::warn!(error = %e, "can't save a phone's apps");
    }
}

/// Returns cached `(pkg, label)` pairs for `device` synchronously (0 ms) for
/// the Command Palette, placing recently mirrored apps first and triggering a
/// background app list load if not yet cached.
pub fn palette_apps(device: DeviceId) -> Vec<(String, String)> {
    let recent = recent_apps(&device);
    let current = apps(&device);
    let (app_names, can_load) = core_host::host().hub.read(|s| {
        let online = s
            .devices
            .iter()
            .any(|d| d.id == device && matches!(d.link, nectarlink_core::LinkState::Online { .. }));
        let avail = s.matrices.get(&device).and_then(|m| m.state("mirroring.app_windows"))
            == Some(nectarlink_core::FeatureState::Available);
        (s.app_names.clone(), online && avail)
    });

    if current.is_none() && can_load {
        load_apps(device);
    }

    let ready_list = match current {
        Some(Apps::Ready(list)) => list,
        _ => Vec::new(),
    };

    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();

    for pkg in &recent {
        let pkg_trim = pkg.trim();
        if pkg_trim.is_empty() {
            continue;
        }
        let label = ready_list
            .iter()
            .find(|a| a.pkg == pkg_trim)
            .map(|a| a.label.clone())
            .or_else(|| app_names.get(pkg_trim).cloned())
            .unwrap_or_else(|| pkg_trim.rsplit('.').next().unwrap_or(pkg_trim).to_owned());
        if !label.trim().is_empty() && seen.insert(pkg_trim.to_owned()) {
            out.push((pkg_trim.to_owned(), label));
        }
    }

    for a in ready_list {
        let pkg_trim = a.pkg.trim();
        let label_trim = a.label.trim();
        if !pkg_trim.is_empty() && !label_trim.is_empty() && seen.insert(pkg_trim.to_owned()) {
            out.push((pkg_trim.to_owned(), label_trim.to_owned()));
        }
    }

    for (pkg, label) in app_names {
        let pkg_trim = pkg.trim();
        let label_trim = label.trim();
        if !pkg_trim.is_empty() && !label_trim.is_empty() && seen.insert(pkg_trim.to_owned()) {
            out.push((pkg_trim.to_owned(), label_trim.to_owned()));
        }
    }

    out
}

/// The PC's mouse, keyboard, and gamepad on a mirrored screen or app window,
/// sent in order.
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

/// Resolves the currently active or showing mirror window.
fn active_or_showing_window() -> Option<Window> {
    state(|s| {
        if let Some(active) = s.active_window
            && s.windows.get(&active).is_some_and(|w| matches!(w.phase, Phase::Showing))
        {
            return Some(active);
        }
        s.windows.iter().find(|(_, shown)| matches!(shown.phase, Phase::Showing)).map(|(w, _)| *w)
    })
}

/// Parses dropped `file://` URLs or local paths, optionally prefixed by a `<device>/<session>` window key.
pub(crate) fn parse_dropped_file_urls(raw_urls: &[String]) -> (Option<Window>, Vec<PathBuf>) {
    let mut explicit_window: Option<Window> = None;
    let mut paths: Vec<PathBuf> = Vec::new();
    for (idx, raw) in raw_urls.iter().enumerate() {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if idx == 0
            && !trimmed.starts_with("file:")
            && !Path::new(trimmed).exists()
            && let Some(win) = Window::parse(trimmed)
        {
            explicit_window = Some(win);
            continue;
        }
        let local = cxx_qt_lib::QUrl::from(&cxx_qt_lib::QString::from(trimmed))
            .to_local_file()
            .map(|p| String::from(&p))
            .filter(|p| !p.is_empty())
            .or_else(|| {
                if cfg!(windows) {
                    trimmed
                        .strip_prefix("file:///")
                        .or_else(|| trimmed.strip_prefix("file://"))
                        .map(|s| s.replace('/', "\\"))
                } else {
                    trimmed.strip_prefix("file://").map(str::to_owned)
                }
            });
        let pb = PathBuf::from(local.as_deref().unwrap_or(trimmed));
        if pb.exists() {
            paths.push(pb);
        }
    }
    (explicit_window, paths)
}

/// Sends files dropped onto a mirror window to the mirrored phone and shows an
/// in-window toast notice.
pub fn drop_files(raw_urls: &[String]) {
    let (explicit_window, paths) = parse_dropped_file_urls(raw_urls);
    if paths.is_empty() {
        return;
    }
    let Some(window) = explicit_window.or_else(active_or_showing_window) else {
        return;
    };
    let notice = if paths.len() == 1 {
        let name = paths[0].file_name().and_then(|n| n.to_str()).unwrap_or("file");
        tr!("Mirror", "Sending %1…", name)
    } else {
        tr!("Mirror", "Sending %1 files…", paths.len())
    };
    set_notice(window, notice);
    crate::transfers::send(window.device, paths);
}

/// Requests remote input control from the phone for `key` and shows a guidance banner.
pub fn request_control(key: &str) {
    let Some(window) = Window::parse(key) else { return };
    set_notice(window, "On your phone, tap Allow / turn on Control from your PC in NectarLink");
    input(
        window,
        nectarlink_core::MirrorInput::Touch { action: nectarlink_core::TouchAction::Up, x: 0.5, y: 0.5 },
    );
}

/// Notifies PC-to-phone cursor/keyboard Flow when the mouse enters `key`.
pub fn enter_window(key: &str) {
    if let Some(win) = Window::parse(key) {
        state(|s| {
            s.active_window = Some(win);
        });
        crate::input_share::enter_remote(win.device);
    }
}

/// Notifies PC-to-phone cursor/keyboard Flow when the mouse leaves a mirror window.
pub fn leave_window() {
    let _ = crate::input_share::leave_remote();
}

/// Records text that arrived from a remote device so focus/hover sync never echoes it back.
#[allow(dead_code)]
pub fn note_remote_clipboard(text: &str) {
    let normalized = text.replace("\r\n", "\n");
    state(|s| {
        s.last_focus_clip = Some(normalized);
    });
}

/// Synchronizes the clipboard from PC to `device` only if the clipboard contains new text.
pub fn sync_clipboard_for_device(device: DeviceId) {
    let Some(node) = core_host::node() else { return };
    let can_send = core_host::host().hub.read(|s| {
        s.matrices.get(&device).and_then(|m| m.state("clipboard.pc_to_phone"))
            == Some(nectarlink_core::FeatureState::Available)
    });
    if !can_send {
        return;
    }

    if let crate::win::clipboard::Clip::Text { text, html, rtf } = crate::win::clipboard::read()
        && !text.is_empty()
    {
        let normalized = text.replace("\r\n", "\n");
        let changed = state(|s| {
            if s.last_focus_clip.as_deref() == Some(normalized.as_str()) {
                false
            } else {
                s.last_focus_clip = Some(normalized);
                true
            }
        });
        if changed {
            core_host::spawn(async move {
                let _ = node.send_clipboard_rich(device, text, html, rtf).await;
            });
        }
    }
}

/// Sends the PC's clipboard to the phone when `window` gains focus, if
/// "Clipboard follows control" is on.
pub fn sync_clipboard_on_focus(window: Window) {
    state(|s| {
        s.active_window = Some(window);
    });
    if crate::settings::flow_follow_clipboard() {
        sync_clipboard_for_device(window.device);
    }
}

/// Standard XInput / Xbox left thumbstick deadzone.
#[allow(dead_code)]
pub const LEFT_THUMB_DEADZONE: i16 = 7849;
/// Standard XInput / Xbox right thumbstick deadzone.
#[allow(dead_code)]
pub const RIGHT_THUMB_DEADZONE: i16 = 8689;
/// Standard XInput / Xbox trigger threshold.
#[allow(dead_code)]
pub const TRIGGER_THRESHOLD: u8 = 30;

/// Normalized controller snapshot forwarded via [`nectarlink_core::MirrorInput::Gamepad`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GamepadState {
    pub buttons: u16,
    pub lx: i16,
    pub ly: i16,
    pub rx: i16,
    pub ry: i16,
    pub lt: u8,
    pub rt: u8,
}

impl GamepadState {
    /// Converts this controller snapshot into a [`nectarlink_core::MirrorInput::Gamepad`] message.
    pub fn to_mirror_input(self) -> nectarlink_core::MirrorInput {
        nectarlink_core::MirrorInput::Gamepad {
            buttons: self.buttons,
            lx: self.lx,
            ly: self.ly,
            rx: self.rx,
            ry: self.ry,
            lt: self.lt,
            rt: self.rt,
        }
    }
}

#[allow(dead_code)]
fn deadzone_axis(val: i16, deadzone: i16) -> i16 {
    if val.saturating_abs() <= deadzone { 0 } else { val }
}

/// Applies stick deadzones and trigger thresholds so idle controller noise does
/// not generate redundant [`nectarlink_core::MirrorInput::Gamepad`] frames.
#[allow(dead_code)]
pub fn normalize_gamepad_state(raw: GamepadState) -> GamepadState {
    GamepadState {
        buttons: raw.buttons,
        lx: deadzone_axis(raw.lx, LEFT_THUMB_DEADZONE),
        ly: deadzone_axis(raw.ly, LEFT_THUMB_DEADZONE),
        rx: deadzone_axis(raw.rx, RIGHT_THUMB_DEADZONE),
        ry: deadzone_axis(raw.ry, RIGHT_THUMB_DEADZONE),
        lt: if raw.lt <= TRIGGER_THRESHOLD { 0 } else { raw.lt },
        rt: if raw.rt <= TRIGGER_THRESHOLD { 0 } else { raw.rt },
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn poll_xinput_gamepad() -> Option<GamepadState> {
    use windows::Win32::UI::Input::XboxController::{XINPUT_STATE, XInputGetState};
    for user_index in 0..4u32 {
        let mut xstate = XINPUT_STATE::default();
        // SAFETY: xstate is a valid stack-allocated XINPUT_STATE for slot 0..3.
        if unsafe { XInputGetState(user_index, &mut xstate) } == 0 {
            let gp = xstate.Gamepad;
            return Some(normalize_gamepad_state(GamepadState {
                buttons: gp.wButtons.0,
                lx: gp.sThumbLX,
                ly: gp.sThumbLY,
                rx: gp.sThumbRX,
                ry: gp.sThumbRY,
                lt: gp.bLeftTrigger,
                rt: gp.bRightTrigger,
            }));
        }
    }
    None
}

#[cfg(windows)]
fn poll_winrt_gamepad() -> Option<GamepadState> {
    use windows::Gaming::Input::{Gamepad, GamepadButtons};
    let gamepads = Gamepad::Gamepads().ok()?;
    if gamepads.Size().ok()? == 0 {
        return None;
    }
    let pad = gamepads.GetAt(0).ok()?;
    let reading = pad.GetCurrentReading().ok()?;
    let b = reading.Buttons;
    let mut buttons = 0u16;
    if (b & GamepadButtons::DPadUp) != GamepadButtons::None {
        buttons |= 0x0001;
    }
    if (b & GamepadButtons::DPadDown) != GamepadButtons::None {
        buttons |= 0x0002;
    }
    if (b & GamepadButtons::DPadLeft) != GamepadButtons::None {
        buttons |= 0x0004;
    }
    if (b & GamepadButtons::DPadRight) != GamepadButtons::None {
        buttons |= 0x0008;
    }
    if (b & GamepadButtons::Menu) != GamepadButtons::None {
        buttons |= 0x0010;
    }
    if (b & GamepadButtons::View) != GamepadButtons::None {
        buttons |= 0x0020;
    }
    if (b & GamepadButtons::LeftThumbstick) != GamepadButtons::None {
        buttons |= 0x0040;
    }
    if (b & GamepadButtons::RightThumbstick) != GamepadButtons::None {
        buttons |= 0x0080;
    }
    if (b & GamepadButtons::LeftShoulder) != GamepadButtons::None {
        buttons |= 0x0100;
    }
    if (b & GamepadButtons::RightShoulder) != GamepadButtons::None {
        buttons |= 0x0200;
    }
    if (b & GamepadButtons::A) != GamepadButtons::None {
        buttons |= 0x1000;
    }
    if (b & GamepadButtons::B) != GamepadButtons::None {
        buttons |= 0x2000;
    }
    if (b & GamepadButtons::X) != GamepadButtons::None {
        buttons |= 0x4000;
    }
    if (b & GamepadButtons::Y) != GamepadButtons::None {
        buttons |= 0x8000;
    }
    let scale_axis = |v: f64| -> i16 { (v.clamp(-1.0, 1.0) * f64::from(i16::MAX)).round() as i16 };
    let scale_trigger = |v: f64| -> u8 { (v.clamp(0.0, 1.0) * 255.0).round() as u8 };
    Some(normalize_gamepad_state(GamepadState {
        buttons,
        lx: scale_axis(reading.LeftThumbstickX),
        ly: scale_axis(reading.LeftThumbstickY),
        rx: scale_axis(reading.RightThumbstickX),
        ry: scale_axis(reading.RightThumbstickY),
        lt: scale_trigger(reading.LeftTrigger),
        rt: scale_trigger(reading.RightTrigger),
    }))
}

#[cfg(windows)]
fn poll_gamepad() -> Option<GamepadState> {
    poll_xinput_gamepad().or_else(poll_winrt_gamepad)
}

#[cfg(target_os = "linux")]
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct JsEvent {
    time: u32,
    value: i16,
    type_: u8,
    number: u8,
}

#[cfg(target_os = "linux")]
fn apply_js_event(state: &mut GamepadState, ev: JsEvent) {
    let kind = ev.type_ & !0x80; // Mask out JS_EVENT_INIT (0x80).
    match kind {
        0x01 => {
            // JS_EVENT_BUTTON
            let mask: u16 = match ev.number {
                0 => 0x1000,  // A
                1 => 0x2000,  // B
                2 => 0x4000,  // X
                3 => 0x8000,  // Y
                4 => 0x0100,  // LB
                5 => 0x0200,  // RB
                6 => 0x0020,  // View / Back
                7 => 0x0010,  // Menu / Start
                9 => 0x0040,  // LThumb
                10 => 0x0080, // RThumb
                12 => 0x0001, // DPad Up
                13 => 0x0002, // DPad Down
                14 => 0x0004, // DPad Left
                15 => 0x0008, // DPad Right
                _ => 0,
            };
            if mask != 0 {
                if ev.value != 0 {
                    state.buttons |= mask;
                } else {
                    state.buttons &= !mask;
                }
            }
        }
        0x02 => {
            // JS_EVENT_AXIS
            match ev.number {
                0 => state.lx = ev.value,
                1 => state.ly = ev.value.saturating_neg(),
                2 => {
                    state.lt = ((i32::from(ev.value) + 32768) * 255 / 65535).clamp(0, 255) as u8;
                }
                3 => state.rx = ev.value,
                4 => state.ry = ev.value.saturating_neg(),
                5 => {
                    state.rt = ((i32::from(ev.value) + 32768) * 255 / 65535).clamp(0, 255) as u8;
                }
                6 => {
                    state.buttons &= !(0x0004 | 0x0008);
                    if ev.value < 0 {
                        state.buttons |= 0x0004; // DPad Left
                    } else if ev.value > 0 {
                        state.buttons |= 0x0008; // DPad Right
                    }
                }
                7 => {
                    state.buttons &= !(0x0001 | 0x0002);
                    if ev.value < 0 {
                        state.buttons |= 0x0001; // DPad Up
                    } else if ev.value > 0 {
                        state.buttons |= 0x0002; // DPad Down
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
}

/// Whether `/dev/input/js{n}` is a game controller: one with the gamepad
/// buttons (`BTN_GAMEPAD`), unlike a sensor or a flight stick.
#[cfg(target_os = "linux")]
fn is_gamepad(n: usize) -> bool {
    std::fs::read_to_string(format!("/sys/class/input/js{n}/device/capabilities/key"))
        .is_ok_and(|caps| has_key(&caps, BTN_GAMEPAD, usize::BITS))
}

/// The first gamepad button (`BTN_SOUTH`).
#[cfg(any(target_os = "linux", test))]
const BTN_GAMEPAD: u32 = 0x130;

/// Whether a sysfs key bitmap (hex words, most significant first, each
/// `word_bits` wide) has `code` set.
#[cfg(any(target_os = "linux", test))]
fn has_key(caps: &str, code: u32, word_bits: u32) -> bool {
    let words: Vec<&str> = caps.split_whitespace().collect();
    let index = (code / word_bits) as usize;
    let Some(word) = words.len().checked_sub(index + 1).and_then(|i| words.get(i)) else { return false };
    u64::from_str_radix(word, 16).is_ok_and(|w| w >> (code % word_bits) & 1 == 1)
}

#[cfg(target_os = "linux")]
struct LinuxGamepadPoller {
    fd: std::ffi::c_int,
    state: GamepadState,
}

#[cfg(target_os = "linux")]
impl Default for LinuxGamepadPoller {
    fn default() -> Self {
        Self { fd: -1, state: GamepadState::default() }
    }
}

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
fn poll_gamepad() -> Option<GamepadState> {
    static POLLER: Mutex<LinuxGamepadPoller> = Mutex::new(LinuxGamepadPoller {
        fd: -1,
        state: GamepadState { buttons: 0, lx: 0, ly: 0, rx: 0, ry: 0, lt: 0, rt: 0 },
    });
    let mut poller = POLLER.lock().unwrap_or_else(|e| e.into_inner());
    if poller.fd < 0 {
        let paths = [c"/dev/input/js0", c"/dev/input/js1", c"/dev/input/js2", c"/dev/input/js3"];
        for (n, path) in paths.into_iter().enumerate() {
            // Not every joystick device is a controller: a laptop's accelerometer
            // is one too, and its tilt read as a held stick scrolled the phone (#332).
            if !is_gamepad(n) {
                continue;
            }
            let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK) };
            if fd >= 0 {
                poller.fd = fd;
                poller.state = GamepadState::default();
                break;
            }
        }
    }
    if poller.fd < 0 {
        return None;
    }

    loop {
        let mut ev = JsEvent { time: 0, value: 0, type_: 0, number: 0 };
        let n = unsafe {
            libc::read(poller.fd, (&mut ev as *mut JsEvent).cast(), std::mem::size_of::<JsEvent>())
        };
        if n == std::mem::size_of::<JsEvent>() as isize {
            apply_js_event(&mut poller.state, ev);
        } else if n < 0 {
            let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if err == libc::EAGAIN || err == libc::EWOULDBLOCK {
                break;
            }
            unsafe {
                libc::close(poller.fd);
            }
            poller.fd = -1;
            poller.state = GamepadState::default();
            return None;
        } else {
            unsafe {
                libc::close(poller.fd);
            }
            poller.fd = -1;
            poller.state = GamepadState::default();
            return None;
        }
    }

    Some(normalize_gamepad_state(poller.state))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn poll_gamepad() -> Option<GamepadState> {
    None
}

fn ensure_gamepad_loop() {
    static RUNNING: AtomicBool = AtomicBool::new(false);
    if RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    core_host::spawn(async move {
        let mut last_state = GamepadState::default();
        loop {
            let target = active_or_showing_window().filter(|w| matches!(phase(w), Some(Phase::Showing)));
            let Some(window) = target else {
                RUNNING.store(false, Ordering::Release);
                // A window that started showing just now found the loop still
                // running and left it to this one.
                let showing =
                    active_or_showing_window().is_some_and(|w| matches!(phase(&w), Some(Phase::Showing)));
                if showing && !RUNNING.swap(true, Ordering::AcqRel) {
                    continue;
                }
                break;
            };
            match poll_gamepad() {
                Some(current) => {
                    if current != last_state {
                        last_state = current;
                        input(window, current.to_mirror_input());
                    }
                    tokio::time::sleep(Duration::from_millis(16)).await;
                }
                None => {
                    if last_state != GamepadState::default() {
                        last_state = GamepadState::default();
                        input(window, last_state.to_mirror_input());
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }
        }
    });
}

/// Where the core puts a phone's video (or the screen's sound, from a
/// stream of its own): only for mirrorings this PC asked for.
pub fn sink(device: &DeviceId, session: u32) -> Option<Arc<dyn MirrorSink>> {
    let window = Window { device: *device, session };
    matches!(phase(&window), Some(Phase::Asking | Phase::Showing))
        .then(|| Arc::new(Sink::new(window)) as Arc<dyn MirrorSink>)
        // No window: the phone's sound on its own, if it was asked for.
        .or_else(|| if session == MIRROR_SCREEN { crate::phone_sound::sink(device) } else { None })
}

enum Item {
    Config(MirrorConfig),
    Packet { keyframe: bool, time_us: u64, data: Vec<u8>, sequence: usize, received: Instant },
    Ended,
}

/// Hands the core's packets to the decoder thread, or its sound to the
/// player; each starts with its stream's first packet (the format).
struct Sink {
    window: Window,
    video: OnceLock<Video>,
    sound: Mutex<Option<Player>>,
}

/// Asks the phone for a keyframe, at most every 500 ms per window: a
/// decoder that's behind would otherwise ask on every dropped frame.
fn ask_keyframe(window: Window) {
    static ASKED: Mutex<Option<HashMap<Window, std::time::Instant>>> = Mutex::new(None);
    {
        let mut asked = ASKED.lock().unwrap_or_else(|e| e.into_inner());
        let asked = asked.get_or_insert_with(HashMap::new);
        if asked.get(&window).is_some_and(|at| at.elapsed() < Duration::from_millis(500)) {
            return;
        }
        asked.insert(window, std::time::Instant::now());
    }
    if let Some(node) = core_host::node() {
        core_host::spawn(async move { node.mirror_keyframe(window.device, window.session).await });
    }
}

/// Set when the app quits: decoder threads end instead of waiting for packets.
static STOPPING: AtomicBool = AtomicBool::new(false);
/// The decoder threads, so quitting can wait for them.
static DECODERS: Mutex<Vec<std::thread::JoinHandle<()>>> = Mutex::new(Vec::new());
/// How often an idle decoder looks at [`STOPPING`].
const STOP_CHECK: Duration = Duration::from_millis(100);
/// How long quitting waits for a decoder in the middle of a picture.
const STOP_WAIT: Duration = Duration::from_secs(2);

/// Ends every decoder thread and waits for it. Call before the core and Qt go
/// away: a decoder still running while the process exits used the GPU driver
/// after it was unloaded, and the app crashed on quit (Linux, an app window open).
pub fn stop_decoders() {
    STOPPING.store(true, Ordering::Release);
    let threads = std::mem::take(&mut *DECODERS.lock().unwrap_or_else(|e| e.into_inner()));
    let left = join_by(threads, Instant::now() + STOP_WAIT);
    if left > 0 {
        tracing::warn!(left, "mirror decoders still busy at quit");
    }
}

/// Joins the threads that finish before `deadline`; returns how many didn't.
fn join_by(threads: Vec<std::thread::JoinHandle<()>>, deadline: Instant) -> usize {
    let mut left = 0;
    for thread in threads {
        while !thread.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        if thread.is_finished() {
            let _ = thread.join();
        } else {
            left += 1;
        }
    }
    left
}

/// The next item for a decoder, or `None` once the stream ends or the app quits.
fn next_item(items: &Receiver<Item>, stopping: &AtomicBool) -> Option<Item> {
    loop {
        if stopping.load(Ordering::Acquire) {
            return None;
        }
        match items.recv_timeout(STOP_CHECK) {
            Ok(item) => return Some(item),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return None,
        }
    }
}

/// The decoder thread's end of a sink.
struct Video {
    queue: SyncSender<Item>,
    /// Packets waiting for the decoder.
    waiting: Arc<AtomicUsize>,
    sequence: AtomicUsize,
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
        if STOPPING.load(Ordering::Acquire) {
            return Video { queue, waiting, sequence: AtomicUsize::new(0) };
        }
        let started = std::thread::Builder::new()
            .name("mirror-decoder".into())
            .spawn(move || decode(window, &items, &counter));
        match started {
            Ok(thread) => {
                let mut threads = DECODERS.lock().unwrap_or_else(|e| e.into_inner());
                threads.retain(|t| !t.is_finished());
                threads.push(thread);
            }
            Err(e) => tracing::error!(error = %e, "can't start the video decoder"),
        }
        Video { queue, waiting, sequence: AtomicUsize::new(0) }
    }
}

impl Sink {
    fn keyframe_please(&self) {
        ask_keyframe(self.window);
    }
}

impl MirrorSink for Sink {
    fn config(&self, config: MirrorConfig) {
        // A picture is coming: this quality works on this phone.
        if self.window.session == MIRROR_SCREEN {
            set_unproven(self.window.device, false);
        }
        tracing::info!(
            width = config.width,
            height = config.height,
            session = self.window.session,
            "mirror stream size"
        );
        LAST_CONFIG
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_or_insert_with(HashMap::new)
            .insert(self.window, (config.width, config.height));
        if let Some(rec) =
            RECORDERS.lock().unwrap_or_else(|e| e.into_inner()).as_mut().and_then(|m| m.get_mut(&self.window))
        {
            rec.on_config(config.width, config.height);
        }
        let video = self.video.get_or_init(|| Video::start(self.window));
        // The format matters: wait for room rather than drop it.
        let _ = video.queue.send(Item::Config(config));
    }

    fn packet(&self, keyframe: bool, time_us: u64, data: Vec<u8>) {
        if keyframe {
            let mut sps_map = LAST_SPS_PPS.lock().unwrap_or_else(|e| e.into_inner());
            let entry = sps_map.get_or_insert_with(HashMap::new).entry(self.window).or_default();
            merge_sps_pps(entry, &data);
        }
        if ACTIVE_RECORDERS.load(Ordering::Relaxed) > 0
            && let Some(rec) = RECORDERS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_mut()
                .and_then(|m| m.get_mut(&self.window))
            && let Err(e) = rec.on_packet(keyframe, time_us, &data)
        {
            tracing::warn!(error = %e, "mirror recording packet failed");
        }
        let Some(video) = self.video.get() else { return };
        let sequence = video.sequence.fetch_add(1, Ordering::Relaxed);
        // Count before publishing: the decoder can receive immediately.
        video.waiting.fetch_add(1, Ordering::AcqRel);
        match video.queue.try_send(Item::Packet {
            keyframe,
            time_us,
            data,
            sequence,
            received: Instant::now(),
        }) {
            Ok(()) => {}
            Err(TrySendError::Disconnected(_)) => {
                video.waiting.fetch_sub(1, Ordering::AcqRel);
            }
            // The decoder is behind; it starts over at the next keyframe.
            Err(TrySendError::Full(_)) => {
                video.waiting.fetch_sub(1, Ordering::AcqRel);
                self.keyframe_please();
            }
        }
    }

    fn ended(&self) {
        stop_recording(self.window);
        if let Some(video) = self.video.get() {
            let _ = video.queue.send(Item::Ended);
        }
    }

    fn audio_config(&self, config: MirrorAudioConfig) {
        LAST_AUDIO_CONFIG
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_or_insert_with(HashMap::new)
            .insert(self.window.device, (config.rate, config.channels));
        if self.window.session == MIRROR_SCREEN
            && ACTIVE_RECORDERS.load(Ordering::Relaxed) > 0
            && let Some(rec) = RECORDERS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_mut()
                .and_then(|m| m.get_mut(&self.window))
        {
            rec.on_audio_config(config.rate, config.channels);
        }
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

    fn audio(&self, time_us: u64, data: Vec<u8>) {
        if self.window.session == MIRROR_SCREEN
            && ACTIVE_RECORDERS.load(Ordering::Relaxed) > 0
            && let Some(rec) = RECORDERS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_mut()
                .and_then(|m| m.get_mut(&self.window))
            && let Err(e) = rec.on_audio(time_us, &data)
        {
            tracing::debug!(error = %e, "mirror recording audio packet failed");
        }
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
    // On the GPU when the PC has one (falling back to software on its own),
    // so big phone screens and several app windows stay smooth.
    let mut decoder = match Decoder::new_gpu("h264") {
        Ok(decoder) => decoder,
        Err(e) => {
            tracing::error!(error = %e, "no H.264 decoder");
            set_phase(window, Phase::Ended(Some(tr!("Mirror", "This PC can't decode the phone's video."))));
            return;
        }
    };
    tracing::info!(gpu = decoder.on_gpu(), codec = decoder.codec(), "mirror decoder ready");
    let keyframe_please = || ask_keyframe(window);
    let mut size = (0, 0);
    // After a gap, pictures are garbage until the next keyframe.
    let mut gate = DecodeGate::new();
    let mut stats = Stats::new();
    let mut last_shown = Instant::now();
    while let Some(item) = next_item(items, &STOPPING) {
        match item {
            Item::Config(config) => {
                if decoder.codec() != config.codec {
                    match Decoder::new_gpu(&config.codec) {
                        Ok(next) => {
                            decoder = next;
                            tracing::info!(
                                gpu = decoder.on_gpu(),
                                codec = decoder.codec(),
                                "mirror decoder ready"
                            );
                        }
                        Err(e) => {
                            tracing::error!(error = %e, codec = %config.codec, "no decoder for codec");
                            set_phase(
                                window,
                                Phase::Ended(Some(format!(
                                    "This PC can't decode {} video.",
                                    config.codec.to_uppercase()
                                ))),
                            );
                            return;
                        }
                    }
                } else if size != (0, 0) && size != (config.width, config.height) {
                    decoder.flush();
                }
                size = (config.width, config.height);
                gate.reset();
            }
            Item::Packet { keyframe, time_us, data, sequence, received } => {
                waiting.fetch_sub(1, Ordering::AcqRel);
                let age = received.elapsed();
                stats.queue_age(age);
                let (accept, reset) = gate.packet(sequence, keyframe);
                if reset {
                    decoder.flush();
                    keyframe_please();
                }
                if !accept {
                    keyframe_please();
                    stats.skipped();
                    continue;
                }
                let started = Instant::now();
                let pictures = match decoder.decode(&data, time_us) {
                    Ok(pictures) => pictures,
                    Err(e) => {
                        tracing::debug!(error = %e, "a video packet didn't decode");
                        decoder.flush();
                        gate.reset();
                        keyframe_please();
                        continue;
                    }
                };
                let decoded = started.elapsed();
                // Decode every contiguous packet to preserve references. Only
                // convert the newest available picture, unless none has been
                // shown for a while. Always show the last queued picture even
                // if it is old: a static screen may not send another one.
                // Recheck after decoding: more packets may have arrived.
                let show = should_show_frame(waiting.load(Ordering::Acquire), last_shown.elapsed());
                if let (Some(picture), true) = (pictures.last(), show) {
                    last_shown = Instant::now();
                    let converted = Instant::now();
                    let (w, h) = if size.0 > 0 { size } else { (picture.width, picture.height) };
                    let (src_w, src_h) = (w.min(picture.width), h.min(picture.height));
                    let bgrx = to_bgrx(picture, w, h);
                    if (src_w, src_h) == (w, h) || src_w == 0 || src_h == 0 {
                        ffi::video_frame(&stream, src_w, src_h, &bgrx);
                        save_latest_frame(window, src_w, src_h, bgrx);
                    } else {
                        let scaled = scale_bgrx(&bgrx, src_w, src_h, w, h);
                        ffi::video_frame(&stream, w, h, &scaled);
                        save_latest_frame(window, w, h, scaled);
                    }
                    stats.shown(decoded, converted.elapsed());
                } else {
                    stats.skipped();
                }
            }
            Item::Ended => break,
        }
    }
    clear_window_buffers(&window);
    ffi::video_clear(&stream);
}

fn should_show_frame(queued: usize, since_shown: Duration) -> bool {
    queued == 0 || since_shown >= SHOW_AT_LEAST_EVERY
}

/// A lost packet breaks the reference chain; age alone does not. Resume at
/// a keyframe after a sequence gap, but keep decoding contiguous old packets
/// so that skipping their conversion can catch us up.
struct DecodeGate {
    next: Option<usize>,
    needs_keyframe: bool,
}

impl DecodeGate {
    fn new() -> Self {
        Self { next: None, needs_keyframe: true }
    }

    fn reset(&mut self) {
        self.next = None;
        self.needs_keyframe = true;
    }

    fn packet(&mut self, sequence: usize, keyframe: bool) -> (bool, bool) {
        let gap = self.next.is_some_and(|next| next != sequence);
        self.next = Some(sequence.wrapping_add(1));
        let reset = gap && !self.needs_keyframe;
        self.needs_keyframe |= gap;
        if self.needs_keyframe && !keyframe {
            return (false, reset);
        }
        self.needs_keyframe = false;
        (true, reset)
    }
}

fn scale_bgrx(src: &[u8], src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Vec<u8> {
    let mut dst = vec![0u8; (dst_w * dst_h * 4) as usize];
    for y in 0..dst_h {
        let sy = ((u64::from(y) * u64::from(src_h)) / u64::from(dst_h)) as usize;
        let src_row = &src[sy * (src_w as usize * 4)..(sy + 1) * (src_w as usize * 4)];
        let dst_row = &mut dst[(y as usize) * (dst_w as usize * 4)..(y as usize + 1) * (dst_w as usize * 4)];
        for x in 0..dst_w as usize {
            let sx = ((x as u64 * u64::from(src_w)) / u64::from(dst_w)) as usize;
            dst_row[x * 4..x * 4 + 4].copy_from_slice(&src_row[sx * 4..sx * 4 + 4]);
        }
    }
    dst
}

/// Decode and conversion times, logged now and then.
struct Stats {
    since: Instant,
    shown: u32,
    skipped: u32,
    decode: Duration,
    convert: Duration,
    queue_max: Duration,
}

impl Stats {
    fn new() -> Stats {
        Stats {
            since: Instant::now(),
            shown: 0,
            skipped: 0,
            decode: Duration::ZERO,
            convert: Duration::ZERO,
            queue_max: Duration::ZERO,
        }
    }

    fn queue_age(&mut self, age: Duration) {
        self.queue_max = self.queue_max.max(age);
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
        if elapsed < STATS_EVERY {
            return;
        }
        if self.shown == 0 {
            // Decoding but showing nothing: say so rather than go quiet.
            if self.skipped > 0 {
                tracing::warn!(skipped = self.skipped, "mirroring: no picture shown");
            }
            *self = Stats::new();
            return;
        }
        tracing::info!(
            fps = format!("{:.1}", f64::from(self.shown) / elapsed.as_secs_f64()),
            skipped = self.skipped,
            decode_ms = format!("{:.1}", self.decode.as_secs_f64() * 1000.0 / f64::from(self.shown)),
            convert_ms = format!("{:.1}", self.convert.as_secs_f64() * 1000.0 / f64::from(self.shown)),
            queue_max_ms = self.queue_max.as_millis(),
            "mirroring"
        );
        *self = Stats::new();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_controllers_count_as_gamepads() {
        use super::{BTN_GAMEPAD, has_key};
        // An Xbox controller, as a 64-bit kernel lists its keys.
        assert!(has_key("7cdb000000000000 0 0 0 0", BTN_GAMEPAD, 64));
        // The same on a 32-bit kernel.
        assert!(has_key("7cdb0000 0 0 0 0 0 0 0 0 0", BTN_GAMEPAD, 32));
        // A laptop's accelerometer has no keys at all (#332).
        assert!(!has_key("0", BTN_GAMEPAD, 64));
        assert!(!has_key("", BTN_GAMEPAD, 64));
        // A flight stick has BTN_TRIGGER and friends (0x120..), not BTN_SOUTH.
        assert!(!has_key("ffff00000000 0 0 0 0", BTN_GAMEPAD, 64));
        assert!(!has_key("not hex 0 0 0 0", BTN_GAMEPAD, 64));
    }

    #[test]
    fn decoder_resumes_at_a_fresh_keyframe_after_a_gap() {
        let mut gate = super::DecodeGate::new();
        assert_eq!(gate.packet(0, false), (false, false));
        assert_eq!(gate.packet(1, true), (true, false));
        assert_eq!(gate.packet(2, false), (true, false));
        // Packet 3 was dropped while the queue was full. A previously
        // decoded keyframe must not make packet 4 look safe to decode.
        assert_eq!(gate.packet(4, false), (false, true));
        assert_eq!(gate.packet(5, false), (false, false));
        assert_eq!(gate.packet(6, true), (true, false));
        assert_eq!(gate.packet(7, false), (true, false));
    }

    #[test]
    fn decoder_keeps_contiguous_packets_without_converting_every_picture() {
        let mut gate = super::DecodeGate::new();
        let fresh = std::time::Duration::ZERO;
        for (sequence, keyframe) in [(0, true), (1, false), (2, true), (3, false)] {
            assert_eq!(gate.packet(sequence, keyframe), (true, false));
            assert!(!super::should_show_frame(1, fresh));
        }
        // Once caught up, a dependent frame is still usable without a new IDR.
        assert_eq!(gate.packet(4, false), (true, false));
        assert!(super::should_show_frame(0, fresh));
        gate.reset();
        assert_eq!(gate.packet(5, false), (false, false));
        assert_eq!(gate.packet(6, true), (true, false));
    }

    #[test]
    fn frame_presentation_skips_backlog_but_keeps_updating_under_load() {
        use std::time::Duration;
        let fresh = Duration::ZERO;
        // The queue may end with an old frame and then no more updates.
        assert!(super::should_show_frame(0, fresh));
        assert!(!super::should_show_frame(1, fresh));
        assert!(!super::should_show_frame(8, super::SHOW_AT_LEAST_EVERY - Duration::from_nanos(1)));
        assert!(super::should_show_frame(8, super::SHOW_AT_LEAST_EVERY));
    }

    #[test]
    fn decoder_accepts_a_keyframe_at_the_gap_and_wrapping_sequences() {
        let mut gate = super::DecodeGate::new();
        assert_eq!(gate.packet(2, true), (true, false));
        assert_eq!(gate.packet(4, true), (true, true));
        assert_eq!(gate.packet(5, false), (true, false));
        gate.reset();
        assert_eq!(gate.packet(usize::MAX, true), (true, false));
        assert_eq!(gate.packet(0, false), (true, false));
    }

    use super::{
        App, GamepadState, LEFT_THUMB_DEADZONE, RIGHT_THUMB_DEADZONE, TRIGGER_THRESHOLD, merge_recent_apps,
        normalize_gamepad_state, parse_dropped_file_urls, save_apps, saved_apps, saved_apps_path,
    };

    #[test]
    fn recent_apps_combines_saved_and_notification_packages_without_duplicates() {
        let saved = vec!["com.android.settings".to_owned(), "org.telegram.messenger".to_owned()];
        let active = vec![
            "org.telegram.messenger".to_owned(),
            "com.google.android.apps.messaging".to_owned(),
            String::new(),
        ];
        let history = vec!["com.google.android.apps.messaging".to_owned(), "com.spotify.music".to_owned()];
        assert_eq!(
            merge_recent_apps(&saved, active, history),
            vec![
                "com.android.settings".to_owned(),
                "org.telegram.messenger".to_owned(),
                "com.google.android.apps.messaging".to_owned(),
                "com.spotify.music".to_owned(),
            ]
        );
    }

    #[test]
    fn gamepad_deadzone_filters_idle_noise_and_converts_to_mirror_input() {
        let idle = normalize_gamepad_state(GamepadState {
            buttons: 0,
            lx: LEFT_THUMB_DEADZONE - 10,
            ly: -LEFT_THUMB_DEADZONE,
            rx: RIGHT_THUMB_DEADZONE,
            ry: -(RIGHT_THUMB_DEADZONE - 1),
            lt: TRIGGER_THRESHOLD,
            rt: 5,
        });
        assert_eq!(idle, GamepadState::default());

        let active = normalize_gamepad_state(GamepadState {
            buttons: 0x1001, // A + DPadUp
            lx: 16_000,
            ly: -24_000,
            rx: -12_000,
            ry: 30_000,
            lt: 128,
            rt: 255,
        });
        assert_eq!(
            active.to_mirror_input(),
            nectarlink_core::MirrorInput::Gamepad {
                buttons: 0x1001,
                lx: 16_000,
                ly: -24_000,
                rx: -12_000,
                ry: 30_000,
                lt: 128,
                rt: 255,
            }
        );
    }

    #[test]
    fn parse_dropped_file_urls_extracts_existing_paths_and_optional_window_key() {
        let dir = tempfile::tempdir().unwrap();
        let file_a = dir.path().join("alpha.txt");
        std::fs::write(&file_a, b"hello").unwrap();
        let file_url = format!("file:///{}", file_a.to_string_lossy().replace('\\', "/"));
        let dev = nectarlink_core::DeviceId([0x11; 32]);
        let key = format!("{dev}/0");

        let (win, paths) = parse_dropped_file_urls(&[key, file_url]);
        assert_eq!(win.map(|w| w.device), Some(dev));
        assert_eq!(paths, vec![file_a]);
    }

    #[test]
    fn a_phones_apps_are_kept_for_the_next_run_with_their_icons() {
        let dir = tempfile::tempdir().unwrap();
        let phone = nectarlink_core::DeviceId([0x22; 32]);
        assert!(saved_apps(dir.path(), &phone).is_none(), "nothing saved yet");

        let png = b"\x89PNG\r\n\x1a\nicon";
        let icon = crate::icons::save(dir.path(), "org.example.maps", png).unwrap();
        let apps = vec![
            App { pkg: "org.example.maps".into(), label: "Maps".into(), icon: Some(icon.clone()) },
            App { pkg: "org.example.notes".into(), label: "Notes".into(), icon: None },
        ];
        save_apps(dir.path(), &phone, &apps);
        assert_eq!(saved_apps(dir.path(), &phone), Some(apps));

        // Another phone has its own list; a damaged file is no list.
        assert!(saved_apps(dir.path(), &nectarlink_core::DeviceId([0x33; 32])).is_none());
        std::fs::write(saved_apps_path(dir.path(), &phone), b"{not json").unwrap();
        assert!(saved_apps(dir.path(), &phone).is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_js_event_updates_buttons_and_axes() {
        use super::{JsEvent, apply_js_event};

        let mut state = GamepadState::default();
        // Press A (0 = 0x1000) and RB (5 = 0x0200), plus init flag (0x80) on Menu (7 = 0x0010).
        apply_js_event(&mut state, JsEvent { time: 1, value: 1, type_: 0x01, number: 0 });
        apply_js_event(&mut state, JsEvent { time: 2, value: 1, type_: 0x01, number: 5 });
        apply_js_event(&mut state, JsEvent { time: 3, value: 1, type_: 0x81, number: 7 });
        assert_eq!(state.buttons, 0x1000 | 0x0200 | 0x0010);

        // Release RB.
        apply_js_event(&mut state, JsEvent { time: 4, value: 0, type_: 0x01, number: 5 });
        assert_eq!(state.buttons, 0x1000 | 0x0010);

        // Move LX (0), LY inverted (1), LT (2), RX (3), RY inverted (4), RT (5), DPad X/Y (6/7).
        apply_js_event(&mut state, JsEvent { time: 5, value: 20_000, type_: 0x02, number: 0 });
        apply_js_event(&mut state, JsEvent { time: 6, value: -18_000, type_: 0x02, number: 1 });
        apply_js_event(&mut state, JsEvent { time: 7, value: 32_767, type_: 0x02, number: 2 });
        apply_js_event(&mut state, JsEvent { time: 8, value: -15_000, type_: 0x02, number: 3 });
        apply_js_event(&mut state, JsEvent { time: 9, value: 22_000, type_: 0x02, number: 4 });
        apply_js_event(&mut state, JsEvent { time: 10, value: 0, type_: 0x02, number: 5 });
        apply_js_event(&mut state, JsEvent { time: 11, value: -32_767, type_: 0x02, number: 6 }); // DPad Left
        apply_js_event(&mut state, JsEvent { time: 12, value: -32_767, type_: 0x02, number: 7 }); // DPad Up

        let normalized = normalize_gamepad_state(state);
        assert_eq!(normalized.buttons, 0x1000 | 0x0010 | 0x0004 | 0x0001);
        assert_eq!(normalized.lx, 20_000);
        assert_eq!(normalized.ly, 18_000);
        assert_eq!(normalized.rx, -15_000);
        assert_eq!(normalized.ry, -22_000);
        assert_eq!(normalized.lt, 255);
        assert_eq!(normalized.rt, 127);
    }

    #[test]
    fn an_idle_decoder_ends_when_the_app_quits() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc::sync_channel,
        };
        use std::time::{Duration, Instant};
        let (queue, items) = sync_channel::<super::Item>(4);
        let stopping = Arc::new(AtomicBool::new(false));
        let flag = stopping.clone();
        // Waits for packets that never come, as a phone with a still screen sends none.
        let thread = std::thread::spawn(move || {
            let mut seen = 0;
            while super::next_item(&items, &flag).is_some() {
                seen += 1;
            }
            seen
        });
        queue.send(super::Item::Ended).unwrap();
        std::thread::sleep(Duration::from_millis(250));
        assert!(!thread.is_finished(), "it ended before the app quit");
        stopping.store(true, Ordering::Release);
        let started = Instant::now();
        while !thread.is_finished() && started.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(thread.join().unwrap(), 1);
        assert!(started.elapsed() < Duration::from_secs(1));
        drop(queue);
    }

    #[test]
    fn a_closed_stream_ends_the_decoder_without_quitting() {
        let (queue, items) = std::sync::mpsc::sync_channel::<super::Item>(1);
        drop(queue);
        assert!(super::next_item(&items, &std::sync::atomic::AtomicBool::new(false)).is_none());
    }

    #[test]
    fn quitting_waits_for_finished_decoders_but_not_forever() {
        use std::time::{Duration, Instant};
        let quick = std::thread::spawn(|| std::thread::sleep(Duration::from_millis(50)));
        let (release, stuck_rx) = std::sync::mpsc::channel::<()>();
        let stuck = std::thread::spawn(move || {
            let _ = stuck_rx.recv();
        });
        let started = Instant::now();
        assert_eq!(super::join_by(vec![quick, stuck], Instant::now() + Duration::from_millis(300)), 1);
        assert!(started.elapsed() < Duration::from_secs(2));
        release.send(()).unwrap();
    }
}
