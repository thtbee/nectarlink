// SPDX-License-Identifier: GPL-3.0-or-later
//! Phone as a webcam (`docs/protocol/webcam.md`): decodes a phone's H.264
//! camera stream on a dedicated thread, publishes BGRX frames into the
//! shared-memory mapping read by `nectarlink_vcam.dll` (the global one the
//! camera creates inside Windows' Frame Server, or this session's while it
//! isn't there), and feeds the live preview `VideoView` (`stream:
//! "webcam"`) in Settings.

use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
    mpsc::{Receiver, SyncSender, TrySendError, sync_channel},
};

use nectarlink_core::{DeviceId, Error, NodeEvent, WebcamConfig, WebcamSink, WebcamStart};
use nectarlink_vcam::{
    DEFAULT_MAPPING_NAME, LOCAL_MAPPING_NAME, SharedFrameMapping, render_placeholder_bgrx,
};

use crate::{
    bridge::{app::describe, native::ffi},
    core_host,
    settings::Settings,
    state::Changes,
    win::{
        h264::{Decoder, to_bgrx},
        vcam::{self, VirtualCameraHandle},
    },
};

/// Stream key used by `VideoView` in `SettingsPage.qml`.
pub const PREVIEW_STREAM: &str = "webcam";

/// Packets waiting for the decoder, at most (~2 seconds at 30 fps).
const BACKLOG: usize = 60;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Phase {
    #[default]
    Idle,
    Asking {
        device: DeviceId,
    },
    Streaming {
        device: DeviceId,
        width: u32,
        height: u32,
        fps: u32,
    },
    Ended {
        reason: String,
    },
}

#[derive(Debug, Default)]
struct State {
    phase: Phase,
    preferred_phone: Option<DeviceId>,
    addon_registered: bool,
    addon_busy: bool,
    vcam_handle: Option<VirtualCameraHandle>,
    mapping: Option<Arc<SharedFrameMapping>>,
    /// Whether `mapping` is the camera's global one (else this session's).
    mapping_global: bool,
    /// When joining the global one was last tried.
    global_tried: Option<std::time::Instant>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static MIRROR: AtomicBool = AtomicBool::new(false);
static HEIGHT: AtomicU32 = AtomicU32::new(720);

fn state<T>(f: impl FnOnce(&mut State) -> T) -> T {
    f(STATE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_default())
}

/// Initializes shared memory, placeholder preview, and (if registered) the
/// Windows virtual camera session.
pub fn init(settings: &Settings) {
    MIRROR.store(settings.webcam_mirror, Ordering::Relaxed);
    let h = if settings.webcam_height == 1080 { 1080 } else { 720 };
    HEIGHT.store(h, Ordering::Relaxed);
    let preferred = settings.webcam_phone.as_deref().and_then(|s| s.parse().ok());
    let registered = vcam::is_registered();
    let vcam_handle = if registered {
        match VirtualCameraHandle::start() {
            Ok(handle) => Some(handle),
            Err(e) => {
                tracing::warn!(error = %e, "can't start IMFVirtualCamera session");
                None
            }
        }
    } else {
        None
    };
    state(|s| {
        s.preferred_phone = preferred;
        s.addon_registered = registered;
        s.vcam_handle = vcam_handle;
    });
    refresh_idle_frame();
}

/// Stops the virtual camera session on app exit.
pub fn shutdown() {
    state(|s| {
        if let Some(mapping) = &s.mapping {
            mapping.set_idle("");
        }
        s.vcam_handle = None;
    });
}

fn save_settings(update: impl FnOnce(&mut Settings)) {
    let dir = &core_host::host().data_dir;
    let mut s = Settings::load(dir);
    update(&mut s);
    if let Err(e) = s.save(dir) {
        tracing::warn!(error = %e, "can't save webcam settings");
    }
}

/// Currently selected phone (`preferred_phone` if still paired, otherwise the
/// first paired phone).
pub fn selected_phone() -> Option<DeviceId> {
    let preferred = state(|s| s.preferred_phone);
    core_host::host().hub.read(|app| {
        if let Some(id) = preferred
            && app.devices.iter().any(|d| d.id == id)
        {
            return Some(id);
        }
        app.devices.first().map(|d| d.id)
    })
}

pub fn set_selected_phone(device: DeviceId) {
    state(|s| s.preferred_phone = Some(device));
    save_settings(|s| s.webcam_phone = Some(device.to_string()));
    refresh_idle_frame();
    core_host::host().hub.changed(Changes::WEBCAM);
}

pub fn height() -> u32 {
    HEIGHT.load(Ordering::Relaxed)
}

pub fn set_height(h: u32) {
    let h = if h == 1080 { 1080 } else { 720 };
    HEIGHT.store(h, Ordering::Relaxed);
    save_settings(|s| s.webcam_height = h);
    core_host::host().hub.changed(Changes::WEBCAM);
}

pub fn mirror() -> bool {
    MIRROR.load(Ordering::Relaxed)
}

pub fn set_mirror(on: bool) {
    MIRROR.store(on, Ordering::Relaxed);
    save_settings(|s| s.webcam_mirror = on);
    core_host::host().hub.changed(Changes::WEBCAM);
}

pub fn phase() -> Phase {
    state(|s| s.phase.clone())
}

pub fn addon_registered() -> bool {
    state(|s| s.addon_registered)
}

pub fn addon_busy() -> bool {
    state(|s| s.addon_busy)
}

/// Launches the elevated helper (`--register-vcam`) when the user clicks "Set up".
pub fn setup_addon() {
    let already_busy = state(|s| {
        if s.addon_busy {
            return true;
        }
        s.addon_busy = true;
        false
    });
    if already_busy {
        return;
    }
    core_host::host().hub.changed(Changes::WEBCAM);
    vcam::run_elevated(vcam::REGISTER_ARG, |_ok| {
        let registered = vcam::is_registered();
        let handle = if registered { VirtualCameraHandle::start().ok() } else { None };
        state(|s| {
            s.addon_busy = false;
            s.addon_registered = registered;
            if handle.is_some() {
                s.vcam_handle = handle;
            }
        });
        core_host::host().hub.changed(Changes::WEBCAM);
    });
}

/// Launches the elevated helper (`--unregister-vcam`) when the user clicks "Remove".
pub fn remove_addon() {
    let already_busy = state(|s| {
        if s.addon_busy {
            return true;
        }
        s.addon_busy = true;
        s.vcam_handle = None;
        false
    });
    if already_busy {
        return;
    }
    core_host::host().hub.changed(Changes::WEBCAM);
    vcam::run_elevated(vcam::UNREGISTER_ARG, |_ok| {
        let registered = vcam::is_registered();
        state(|s| {
            s.addon_busy = false;
            s.addon_registered = registered;
        });
        core_host::host().hub.changed(Changes::WEBCAM);
    });
}

fn phone_name_for(device: Option<DeviceId>) -> String {
    let Some(id) = device else { return String::new() };
    core_host::host().hub.read(|s| s.name_of(&id).unwrap_or_default())
}

/// The shared mapping to publish frames into: the camera's global one as soon
/// as it exists (the camera creates it when an app first opens it, so look
/// again every second until then), else this session's.
fn mapping() -> Option<Arc<SharedFrameMapping>> {
    state(|s| {
        let due = s.global_tried.is_none_or(|at| at.elapsed() >= std::time::Duration::from_secs(1));
        if !s.mapping_global && due {
            s.global_tried = Some(std::time::Instant::now());
            if let Ok(global) = SharedFrameMapping::open_existing(DEFAULT_MAPPING_NAME) {
                tracing::debug!("joined the virtual camera's frame mapping");
                s.mapping = Some(Arc::new(global));
                s.mapping_global = true;
            }
        }
        if s.mapping.is_none() {
            s.mapping = SharedFrameMapping::open_or_create(LOCAL_MAPPING_NAME).ok().map(Arc::new);
        }
        s.mapping.clone()
    })
}

/// Updates the shared-memory header and the QML preview `VideoView` with the
/// idle placeholder frame when no stream is active.
fn refresh_idle_frame() {
    let is_streaming = state(|s| matches!(s.phase, Phase::Streaming { .. }));
    if is_streaming {
        return;
    }
    let phone = selected_phone();
    let name = phone_name_for(phone);
    if let Some(mapping) = mapping() {
        mapping.set_idle(&name);
    }
    let (w, h) = (640u32, 360u32);
    let mut bgrx = vec![0u8; (w * h * 4) as usize];
    render_placeholder_bgrx(&mut bgrx, w, h, &name);
    ffi::video_frame(PREVIEW_STREAM, w, h, &bgrx);
}

/// Requests a webcam stream from `device` (or the selected phone).
pub fn start(device: Option<DeviceId>) {
    let Some(device) = device.or_else(selected_phone) else { return };
    set_selected_phone(device);
    let h = height();
    let w = if h == 1080 { 1920 } else { 1280 };
    state(|s| s.phase = Phase::Asking { device });
    core_host::host().hub.changed(Changes::WEBCAM);

    let Some(node) = core_host::node() else { return };
    let bitrate = if h == 1080 { 8_000_000 } else { 4_000_000 };
    let options = WebcamStart { width: w, height: h, fps: 30, bitrate, camera: "back".into() };
    core_host::spawn(async move {
        if let Err(e) = node.webcam_start(device, options).await {
            let reason = match e {
                Error::Denied => "Webcam is turned off for this phone.".to_owned(),
                Error::Unsupported => {
                    "This phone's app doesn't support Webcam yet. Update Nectarlink on the phone.".into()
                }
                Error::Offline | Error::NotPaired => "The phone isn't connected.".into(),
                e => describe(&e),
            };
            state(|s| {
                if matches!(s.phase, Phase::Asking { device: d } if d == device) {
                    s.phase = Phase::Ended { reason };
                }
            });
            refresh_idle_frame();
            core_host::host().hub.changed(Changes::WEBCAM);
        }
    });
}

/// Stops the active or pending webcam stream.
pub fn stop() {
    let device = state(|s| {
        let dev = match s.phase {
            Phase::Asking { device } | Phase::Streaming { device, .. } => Some(device),
            Phase::Idle | Phase::Ended { .. } => None,
        };
        s.phase = Phase::Idle;
        dev
    });
    refresh_idle_frame();
    core_host::host().hub.changed(Changes::WEBCAM);
    if let (Some(device), Some(node)) = (device, core_host::node()) {
        core_host::spawn(async move {
            node.webcam_stop(device).await;
        });
    }
}

pub fn on_event(event: &NodeEvent) {
    match event {
        NodeEvent::Webcam { device, on: true } => {
            state(|s| {
                s.preferred_phone = Some(*device);
                if !matches!(s.phase, Phase::Streaming { device: d, .. } if d == *device) {
                    s.phase = Phase::Streaming { device: *device, width: 1280, height: 720, fps: 30 };
                }
            });
            core_host::host().hub.changed(Changes::WEBCAM);
        }
        NodeEvent::Webcam { device, on: false }
        | NodeEvent::LinkChanged { device, link: nectarlink_core::LinkState::Offline { .. } } => {
            let changed = state(|s| {
                if matches!(s.phase, Phase::Asking { device: d } | Phase::Streaming { device: d, .. } if d == *device)
                {
                    s.phase = Phase::Idle;
                    true
                } else {
                    false
                }
            });
            if changed {
                refresh_idle_frame();
                core_host::host().hub.changed(Changes::WEBCAM);
            }
        }
        NodeEvent::DeviceAdded(_) | NodeEvent::DeviceRemoved(_) | NodeEvent::PeerInfoChanged { .. } => {
            refresh_idle_frame();
            core_host::host().hub.changed(Changes::WEBCAM);
        }
        _ => {}
    }
}

/// Returns a [`WebcamSink`] for an incoming `"webcam"` stream from `device`.
pub fn sink(device: &DeviceId) -> Option<Arc<dyn WebcamSink>> {
    Some(Arc::new(Sink::new(*device)))
}

enum Item {
    Config(WebcamConfig),
    Packet { keyframe: bool, time_us: u64, data: Vec<u8> },
    Ended,
}

struct Video {
    queue: SyncSender<Item>,
    waiting: Arc<AtomicUsize>,
}

struct Sink {
    device: DeviceId,
    video: OnceLock<Video>,
}

impl Sink {
    fn new(device: DeviceId) -> Self {
        Self { device, video: OnceLock::new() }
    }

    fn keyframe_please(&self) {
        let device = self.device;
        if let Some(node) = core_host::node() {
            core_host::spawn(async move { node.webcam_keyframe(device).await });
        }
    }
}

impl Video {
    fn start(device: DeviceId) -> Self {
        let (queue, items) = sync_channel(BACKLOG);
        let waiting = Arc::new(AtomicUsize::new(0));
        let counter = waiting.clone();
        let started = std::thread::Builder::new()
            .name("webcam-decoder".into())
            .spawn(move || decode(device, &items, &counter));
        if let Err(e) = started {
            tracing::error!(error = %e, "can't start the webcam decoder");
        }
        Self { queue, waiting }
    }
}

impl WebcamSink for Sink {
    fn config(&self, config: WebcamConfig) {
        state(|s| {
            s.preferred_phone = Some(self.device);
            s.phase = Phase::Streaming {
                device: self.device,
                width: config.width,
                height: config.height,
                fps: config.fps,
            };
        });
        core_host::host().hub.changed(Changes::WEBCAM);
        let video = self.video.get_or_init(|| Video::start(self.device));
        let _ = video.queue.send(Item::Config(config));
    }

    fn packet(&self, keyframe: bool, time_us: u64, data: Vec<u8>) {
        let Some(video) = self.video.get() else { return };
        match video.queue.try_send(Item::Packet { keyframe, time_us, data }) {
            Ok(()) => {
                video.waiting.fetch_add(1, Ordering::AcqRel);
            }
            Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => self.keyframe_please(),
        }
    }

    fn ended(&self) {
        if let Some(video) = self.video.get() {
            let _ = video.queue.send(Item::Ended);
        }
    }
}

fn mirror_bgrx_in_place(bgrx: &mut [u8], width: u32, height: u32) {
    let row_bytes = (width as usize) * 4;
    for y in 0..height as usize {
        let row = &mut bgrx[y * row_bytes..(y + 1) * row_bytes];
        for x in 0..(width as usize) / 2 {
            let l = x * 4;
            let r = (width as usize - 1 - x) * 4;
            for c in 0..4 {
                row.swap(l + c, r + c);
            }
        }
    }
}

fn decode(device: DeviceId, items: &Receiver<Item>, waiting: &AtomicUsize) {
    let mut decoder = match Decoder::new() {
        Ok(decoder) => decoder,
        Err(e) => {
            tracing::error!(error = %e, "no H.264 decoder for webcam");
            state(|s| {
                s.phase = Phase::Ended { reason: "This PC can't decode the phone's camera video.".into() };
            });
            refresh_idle_frame();
            core_host::host().hub.changed(Changes::WEBCAM);
            return;
        }
    };
    let keyframe_please = || {
        if let Some(node) = core_host::node() {
            core_host::spawn(async move { node.webcam_keyframe(device).await });
        }
    };
    let phone_name = phone_name_for(Some(device));
    let mut size = (0u32, 0u32);
    let mut need_keyframe = true;

    while let Ok(item) = items.recv() {
        match item {
            Item::Config(config) => {
                if size != (0, 0) && size != (config.width, config.height) {
                    decoder.flush();
                }
                size = (config.width, config.height);
                need_keyframe = true;
            }
            Item::Packet { keyframe, time_us, data } => {
                let behind = waiting.fetch_sub(1, Ordering::AcqRel) > 1;
                if need_keyframe && !keyframe {
                    continue;
                }
                need_keyframe = false;
                let pictures = match decoder.decode(&data, time_us) {
                    Ok(pictures) => pictures,
                    Err(e) => {
                        tracing::debug!(error = %e, "webcam packet didn't decode");
                        decoder.flush();
                        need_keyframe = true;
                        keyframe_please();
                        continue;
                    }
                };
                if let (Some(picture), false) = (pictures.last(), behind) {
                    let (w, h) = if size.0 > 0 { size } else { (picture.width, picture.height) };
                    let (src_w, src_h) = (w.min(picture.width), h.min(picture.height));
                    if src_w == 0 || src_h == 0 {
                        continue;
                    }
                    let mut bgrx = to_bgrx(picture, w, h);
                    let flip = mirror();
                    if flip {
                        mirror_bgrx_in_place(&mut bgrx, src_w, src_h);
                    }
                    if let Some(mapping) = mapping() {
                        // Already flipped in place if `flip` is true, so pass `false` here.
                        mapping.write_frame(&phone_name, src_w, src_h, time_us, &bgrx, false);
                    }
                    ffi::video_frame(PREVIEW_STREAM, src_w, src_h, &bgrx);
                }
            }
            Item::Ended => break,
        }
    }

    state(|s| {
        if matches!(s.phase, Phase::Streaming { device: d, .. } if d == device) {
            s.phase = Phase::Idle;
        }
    });
    refresh_idle_frame();
    core_host::host().hub.changed(Changes::WEBCAM);
}
