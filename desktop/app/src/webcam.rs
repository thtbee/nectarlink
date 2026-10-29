// SPDX-License-Identifier: GPL-3.0-or-later
//! Phone as a webcam (`docs/protocol/webcam.md`): decodes a phone's H.264
//! camera stream on a dedicated thread, publishes shared GPU NV12 textures or
//! fallback BGRX frames into the mapping read by `nectarlink_vcam.dll` (the global one the
//! camera creates inside Windows' Frame Server, or this session's while it
//! isn't there), and feeds the live preview `VideoView` (`stream:
//! "webcam"`) in Settings.
//!
//! ## 4K (3840×2160) Support & Why 10-bit HDR Is Omitted
//!
//! End-to-end 4K UHD (`3840×2160` @ 30 fps) is supported across the phone
//! camera encoder, the desktop H.264 decoder, the shared-memory frame
//! transport (`nectarlink-vcam` v2), and the Media Foundation virtual camera
//! source (`RGB32` and `NV12`).
//!
//! 10-bit HDR (`P010` / BT.2020 PQ or HLG) is intentionally omitted: Windows'
//! inbox H.264 Media Foundation Transform (`CLSID_CMSH264DecoderMFT`) decodes
//! 8-bit 4:2:0 profiles into 8-bit `NV12` (`MFVideoFormat_NV12`), and the
//! shared-memory virtual camera pipeline operates in 8-bit-per-channel SDR
//! Rec.709 (`BGRX` / `MFVideoFormat_RGB32` and `MFVideoFormat_NV12` with
//! `MFVideoTransferMatrix_BT709`) for universal compatibility with conferencing
//! apps.

use std::{
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
        mpsc::{Receiver, SyncSender, TrySendError, sync_channel},
    },
    time::{Duration, Instant},
};

use nectarlink_core::{DeviceId, Error, NodeEvent, WebcamConfig, WebcamSink, WebcamStart};
use nectarlink_vcam::{
    DEFAULT_MAPPING_NAME, LOCAL_MAPPING_NAME, SharedFrameMapping, render_placeholder_bgrx,
};

#[cfg(all(target_os = "macos", feature = "macos-native-camera"))]
use crate::win::camera_output;

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
static CAMERA_FRONT: AtomicBool = AtomicBool::new(false);
static HEIGHT: AtomicU32 = AtomicU32::new(720);
/// Whether the phone's camera in use can stream 4K, as its last stream said
/// (offered until a phone says it can't).
static UHD: AtomicBool = AtomicBool::new(true);
static PREVIEW_ACTIVE: AtomicBool = AtomicBool::new(false);
struct PreviewRequests(AtomicU32);
const HOME_PREVIEW: u32 = 1 << 0;
const SETTINGS_PREVIEW: u32 = 1 << 1;
impl PreviewRequests {
    fn set(&self, bit: u32, requested: bool) -> bool {
        let mask = if requested {
            self.0.fetch_or(bit, Ordering::AcqRel) | bit
        } else {
            self.0.fetch_and(!bit, Ordering::AcqRel) & !bit
        };
        mask != 0
    }
    fn clear(&self) {
        self.0.store(0, Ordering::Release);
    }
}
static PREVIEW_DEMAND: PreviewRequests = PreviewRequests(AtomicU32::new(0));

fn state<T>(f: impl FnOnce(&mut State) -> T) -> T {
    f(STATE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_default())
}

fn normalize_height(h: u32) -> u32 {
    match h {
        2160 => 2160,
        1080 => 1080,
        _ => 720,
    }
}

/// 720, 1080 or 2160 for a picture of `width` × `height`, either way up (an
/// upright phone streams 1080 × 1920 for 1080p).
fn resolution_of(width: u32, height: u32) -> u32 {
    match width.min(height) {
        2000.. => 2160,
        1000.. => 1080,
        _ => 720,
    }
}

/// What the phone is streaming now, chosen here or on the phone (or 1080p
/// when it can't do 4K): the resolution buttons follow it, and the next
/// stream starts with it. Doesn't ask the phone again.
fn follow_stream(width: u32, height: u32) {
    let h = resolution_of(width, height);
    let prev = HEIGHT.swap(h, Ordering::Relaxed);
    #[cfg(not(test))]
    if prev != h {
        save_settings(|s| s.webcam_height = h);
    }
    #[cfg(test)]
    let _ = prev;
}

/// Maps a resolution label (`"720"`, `"1080"`, `"2160"`, `"4k"`, `"4K"`) to
/// `(width, height)` in pixels.
pub fn dimensions(res: &str) -> (u16, u16) {
    match res.trim() {
        "2160" | "4k" | "4K" => (3840, 2160),
        "1080" => (1920, 1080),
        _ => (1280, 720),
    }
}

/// Returns the currently selected vertical resolution as `"720"`, `"1080"`, or `"2160"`.
pub fn resolution() -> &'static str {
    match height() {
        2160 => "2160",
        1080 => "1080",
        _ => "720",
    }
}

/// Sets the webcam resolution from a string token (`"720"`, `"1080"`, or `"2160"`).
/// Unrecognized values are ignored.
#[cfg(test)]
pub fn set_resolution(res: &str) {
    let h = match res.trim() {
        "2160" => 2160,
        "1080" => 1080,
        "720" => 720,
        _ => return,
    };
    set_height(h);
}

/// Initializes webcam preferences and (if registered) the Windows virtual
/// camera session.
pub fn init(settings: &Settings) {
    MIRROR.store(settings.webcam_mirror, Ordering::Relaxed);
    CAMERA_FRONT.store(settings.webcam_camera.trim().eq_ignore_ascii_case("front"), Ordering::Relaxed);
    let h = normalize_height(settings.webcam_height);
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
    if registered {
        refresh_idle_frame();
    }
}

/// Stops the virtual camera session on app exit.
pub fn shutdown() {
    #[cfg(all(target_os = "macos", feature = "macos-native-camera"))]
    camera_output::shutdown();
    state(|s| {
        if let Some(mapping) = &s.mapping {
            mapping.set_idle("");
        }
        s.mapping = None;
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

/// Currently selected phone (`preferred_phone` if online, otherwise the first
/// online paired phone, falling back to `preferred_phone` if paired or the
/// first paired phone).
pub fn selected_phone() -> Option<DeviceId> {
    let preferred = state(|s| s.preferred_phone);
    core_host::host().hub.read(|app| {
        if let Some(id) = preferred
            && app
                .devices
                .iter()
                .any(|d| d.id == id && matches!(d.link, nectarlink_core::LinkState::Online { .. }))
        {
            return Some(id);
        }
        if let Some(online_id) = app
            .devices
            .iter()
            .find(|d| matches!(d.link, nectarlink_core::LinkState::Online { .. }))
            .map(|d| d.id)
        {
            return Some(online_id);
        }
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

/// Whether to offer 4K: the phone's camera in use can stream it, as far as
/// the PC knows.
pub fn uhd() -> bool {
    UHD.load(Ordering::Relaxed)
}

pub fn set_height(h: u32) {
    let h = normalize_height(h);
    let prev = HEIGHT.swap(h, Ordering::Relaxed);
    #[cfg(not(test))]
    {
        save_settings(|s| s.webcam_height = h);
        core_host::host().hub.changed(Changes::WEBCAM);
        if prev != h {
            reconfigure_active_stream();
        }
    }
    #[cfg(test)]
    let _ = prev;
}

pub fn camera() -> &'static str {
    if CAMERA_FRONT.load(Ordering::Relaxed) { "front" } else { "back" }
}

pub fn set_camera(cam: &str) {
    let front = cam.trim().eq_ignore_ascii_case("front");
    let prev = CAMERA_FRONT.swap(front, Ordering::Relaxed);
    #[cfg(not(test))]
    {
        save_settings(|s| s.webcam_camera = camera().to_owned());
        core_host::host().hub.changed(Changes::WEBCAM);
        if prev != front {
            reconfigure_active_stream();
        }
    }
    #[cfg(test)]
    let _ = prev;
}

pub fn mirror() -> bool {
    MIRROR.load(Ordering::Relaxed)
}

pub fn set_mirror(on: bool) {
    MIRROR.store(on, Ordering::Relaxed);
    #[cfg(not(test))]
    {
        save_settings(|s| s.webcam_mirror = on);
        core_host::host().hub.changed(Changes::WEBCAM);
    }
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
    #[cfg(target_os = "macos")]
    {
        return None;
    }
    #[cfg(not(target_os = "macos"))]
    state(|s| {
        let due = s.global_tried.is_none_or(|at| at.elapsed() >= std::time::Duration::from_secs(1));
        if !s.mapping_global && due {
            s.global_tried = Some(std::time::Instant::now());
            #[cfg(windows)]
            let global_res = SharedFrameMapping::open_existing(DEFAULT_MAPPING_NAME);
            #[cfg(not(windows))]
            let global_res = SharedFrameMapping::open_or_create(DEFAULT_MAPPING_NAME);
            if let Ok(global) = global_res {
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

/// Updates the camera's global mapping (if a camera client has opened it)
/// without allocating the 33 MB local fallback mapping while idle.
fn idle_mapping() -> Option<Arc<SharedFrameMapping>> {
    #[cfg(target_os = "macos")]
    {
        return None;
    }
    #[cfg(not(target_os = "macos"))]
    state(|s| {
        let due = s.global_tried.is_none_or(|at| at.elapsed() >= std::time::Duration::from_secs(1));
        if !s.mapping_global && due {
            s.global_tried = Some(std::time::Instant::now());
            #[cfg(windows)]
            let global_res = SharedFrameMapping::open_existing(DEFAULT_MAPPING_NAME);
            #[cfg(not(windows))]
            let global_res = SharedFrameMapping::open_or_create(DEFAULT_MAPPING_NAME);
            if let Ok(global) = global_res {
                tracing::debug!("joined the virtual camera's frame mapping");
                s.mapping = Some(Arc::new(global));
                s.mapping_global = true;
            }
        }
        s.mapping.clone()
    })
}

/// Updates the shared-memory header and (when Settings is visible) the QML
/// preview `VideoView` with the idle placeholder frame when no stream is active.
fn refresh_idle_frame() {
    let is_streaming = state(|s| matches!(s.phase, Phase::Streaming { .. }));
    if is_streaming {
        return;
    }
    let phone = selected_phone();
    let name = phone_name_for(phone);
    if let Some(mapping) = idle_mapping() {
        mapping.set_idle(&name);
    }
    if PREVIEW_ACTIVE.load(Ordering::Relaxed) {
        let (w, h) = (640u32, 360u32);
        let mut bgrx = vec![0u8; (w * h * 4) as usize];
        render_placeholder_bgrx(&mut bgrx, w, h, &name);
        ffi::video_frame(PREVIEW_STREAM, w, h, &bgrx);
    }
}

/// Each view owns its request so hiding one cannot turn off another visible
/// preview. Window visibility participates in the QML demand expression.
pub fn set_preview_visible(owner: &str, active: bool) {
    let bit = match owner {
        "home" => HOME_PREVIEW,
        "settings" => SETTINGS_PREVIEW,
        _ => return,
    };
    set_preview_demand(bit, active);
}

fn set_preview_demand(bit: u32, requested: bool) {
    let active = PREVIEW_DEMAND.set(bit, requested);
    let prev = PREVIEW_ACTIVE.swap(active, Ordering::AcqRel);
    if active && !prev {
        refresh_idle_frame();
    } else if !active {
        let is_streaming = state(|s| matches!(s.phase, Phase::Streaming { .. }));
        if !is_streaming {
            ffi::video_clear(PREVIEW_STREAM);
        }
    }
}

/// Releases the idle preview image and any local-only shared mapping when the
/// main window closes to the tray.
pub fn release_idle_resources() {
    PREVIEW_DEMAND.clear();
    PREVIEW_ACTIVE.store(false, Ordering::Release);
    let is_streaming = state(|s| {
        let streaming = matches!(s.phase, Phase::Streaming { .. });
        if !streaming && !s.mapping_global {
            s.mapping = None;
        }
        streaming
    });
    if !is_streaming {
        ffi::video_clear(PREVIEW_STREAM);
    }
}

fn current_webcam_start() -> WebcamStart {
    let (w_u16, h_u16) = dimensions(resolution());
    let (w, h) = (u32::from(w_u16), u32::from(h_u16));
    let bitrate = match h {
        2160 => 20_000_000,
        1080 => 8_000_000,
        _ => 4_000_000,
    };
    WebcamStart { width: w, height: h, fps: 30, bitrate, camera: camera().to_owned() }
}

#[cfg(not(test))]
fn reconfigure_active_stream() {
    let active_device = state(|s| match s.phase {
        Phase::Streaming { device, .. } | Phase::Asking { device } => Some(device),
        Phase::Idle | Phase::Ended { .. } => None,
    });
    if let (Some(device), Some(node)) = (active_device, core_host::node()) {
        let options = current_webcam_start();
        core_host::spawn(async move {
            let _ = node.webcam_start(device, options).await;
        });
    }
}

/// Requests a webcam stream from `device` (or the selected phone).
pub fn start(device: Option<DeviceId>) {
    #[cfg(target_os = "macos")]
    if !vcam::stream_admission_ready() {
        state(|s| {
            s.phase = Phase::Ended {
                reason: crate::i18n::tr!(
                    "Webcam",
                    "Native camera streaming requires owned webcam protocol support."
                ),
            }
        });
        core_host::host().hub.changed(Changes::WEBCAM);
        return;
    }
    #[cfg(target_os = "macos")]
    if !vcam::is_registered() {
        state(|s| {
            s.phase = Phase::Ended {
                reason: crate::i18n::tr!("Webcam", "The signed camera extension is not ready."),
            }
        });
        core_host::host().hub.changed(Changes::WEBCAM);
        return;
    }
    let Some(device) = device.or_else(selected_phone) else { return };
    set_selected_phone(device);
    state(|s| s.phase = Phase::Asking { device });
    core_host::host().hub.changed(Changes::WEBCAM);

    let Some(node) = core_host::node() else { return };
    let options = current_webcam_start();
    core_host::spawn(async move {
        if let Err(e) = node.webcam_start(device, options).await {
            let reason = match e {
                Error::Denied => "Webcam is turned off for this phone.".to_owned(),
                Error::Unsupported => {
                    "This phone's app doesn't support Webcam yet. Update NectarLink on the phone.".into()
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
    #[cfg(all(target_os = "macos", feature = "macos-native-camera"))]
    let closed_owned_output = camera_output::shutdown();
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
    // The exact admitted sink already stopped its QUIC stream. A deferred
    // peer-wide Stop could otherwise stop a successor from the same phone.
    #[cfg(all(target_os = "macos", feature = "macos-native-camera"))]
    if closed_owned_output {
        return;
    }
    if let (Some(device), Some(node)) = (device, core_host::node()) {
        core_host::spawn(async move {
            node.webcam_stop(device).await;
        });
    }
}

pub fn on_event(event: &NodeEvent) {
    match event {
        #[cfg(not(target_os = "macos"))]
        NodeEvent::Webcam { device, on: true } => {
            state(|s| {
                s.preferred_phone = Some(*device);
                if !matches!(s.phase, Phase::Streaming { device: d, .. } if d == *device) {
                    s.phase = Phase::Streaming { device: *device, width: 1280, height: 720, fps: 30 };
                }
            });
            core_host::host().hub.changed(Changes::WEBCAM);
        }
        #[cfg(not(all(target_os = "macos", feature = "macos-native-camera")))]
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
    #[cfg(target_os = "macos")]
    if !vcam::stream_admission_ready() || !vcam::is_registered() {
        return None;
    }
    #[cfg(all(target_os = "macos", feature = "macos-native-camera"))]
    {
        let stopped = Arc::new(tokio::sync::Notify::new());
        let lease = camera_output::claim(*device, stopped.clone()).ok()?;
        Some(Arc::new(Sink { device: *device, video: OnceLock::new(), lease, stopped }))
    }
    #[cfg(not(all(target_os = "macos", feature = "macos-native-camera")))]
    {
        Some(Arc::new(Sink::new(*device)))
    }
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
    #[cfg(all(target_os = "macos", feature = "macos-native-camera"))]
    lease: camera_output::Lease,
    #[cfg(all(target_os = "macos", feature = "macos-native-camera"))]
    stopped: Arc<tokio::sync::Notify>,
    device: DeviceId,
    video: OnceLock<Video>,
}

impl Sink {
    #[cfg(not(all(target_os = "macos", feature = "macos-native-camera")))]
    fn new(device: DeviceId) -> Self {
        Self { device, video: OnceLock::new() }
    }

    fn configure(&self, config: WebcamConfig) {
        let update = || {
            follow_stream(config.width, config.height);
            UHD.store(config.max_height >= 2160, Ordering::Relaxed);
            state(|s| {
                s.preferred_phone = Some(self.device);
                s.phase = Phase::Streaming {
                    device: self.device,
                    width: config.width,
                    height: config.height,
                    fps: config.fps,
                };
            });
        };
        #[cfg(all(target_os = "macos", feature = "macos-native-camera"))]
        if camera_output::with_owner(self.lease, update).is_none() {
            return;
        }
        #[cfg(not(all(target_os = "macos", feature = "macos-native-camera")))]
        update();
        core_host::host().hub.changed(Changes::WEBCAM);
        let video = self.video.get_or_init(|| {
            Video::start(
                self.device,
                #[cfg(all(target_os = "macos", feature = "macos-native-camera"))]
                self.lease,
            )
        });
        let _ = video.queue.send(Item::Config(config));
    }

    #[cfg(all(target_os = "macos", feature = "macos-native-camera"))]
    fn end_output(&self) {
        let _ = camera_output::with_owner(self.lease, || {
            state(|s| {
                if matches!(s.phase, Phase::Streaming { device, .. } if device == self.device) {
                    s.phase = Phase::Idle;
                }
            })
        });
        camera_output::release(self.lease);
        refresh_idle_frame();
        core_host::host().hub.changed(Changes::WEBCAM);
    }

    fn keyframe_please(&self) {
        let device = self.device;
        if let Some(node) = core_host::node() {
            core_host::spawn(async move { node.webcam_keyframe(device).await });
        }
    }
}

impl Video {
    fn start(
        device: DeviceId,
        #[cfg(all(target_os = "macos", feature = "macos-native-camera"))] lease: camera_output::Lease,
    ) -> Self {
        let (queue, items) = sync_channel(BACKLOG);
        let waiting = Arc::new(AtomicUsize::new(0));
        let counter = waiting.clone();
        let started = std::thread::Builder::new().name("webcam-decoder".into()).spawn(move || {
            decode(
                device,
                &items,
                &counter,
                #[cfg(all(target_os = "macos", feature = "macos-native-camera"))]
                lease,
            )
        });
        if let Err(e) = started {
            tracing::error!(error = %e, "can't start the webcam decoder");
            #[cfg(all(target_os = "macos", feature = "macos-native-camera"))]
            {
                let _ = camera_output::with_owner(lease, || {
                    state(|s| s.phase = Phase::Ended { reason: "The camera decoder could not start.".into() })
                });
                camera_output::release(lease);
            }
        }
        Self { queue, waiting }
    }
}

impl WebcamSink for Sink {
    fn config(&self, config: WebcamConfig) {
        self.configure(config);
    }

    #[cfg(all(target_os = "macos", feature = "macos-native-camera"))]
    fn closed(&self) -> Option<Arc<tokio::sync::Notify>> {
        Some(self.stopped.clone())
    }

    fn packet(&self, keyframe: bool, time_us: u64, data: Vec<u8>) {
        let Some(video) = self.video.get() else { return };
        // Count before publishing: the receiver may consume immediately.
        video.waiting.fetch_add(1, Ordering::AcqRel);
        match video.queue.try_send(Item::Packet { keyframe, time_us, data }) {
            Ok(()) => {}
            Err(TrySendError::Disconnected(_)) => {
                video.waiting.fetch_sub(1, Ordering::AcqRel);
            }
            Err(TrySendError::Full(_)) => {
                video.waiting.fetch_sub(1, Ordering::AcqRel);
                self.keyframe_please();
            }
        }
    }

    fn ended(&self) {
        #[cfg(all(target_os = "macos", feature = "macos-native-camera"))]
        self.end_output();
        if let Some(video) = self.video.get() {
            let _ = video.queue.send(Item::Ended);
        }
    }
}

#[cfg(all(target_os = "macos", feature = "macos-native-camera"))]
impl Drop for Sink {
    fn drop(&mut self) {
        self.end_output();
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

#[cfg(windows)]
pub fn supports_hevc() -> bool {
    static SUPPORTED: OnceLock<bool> = OnceLock::new();
    *SUPPORTED.get_or_init(|| match Decoder::new_gpu("h265") {
        Ok(decoder) => {
            let supported = decoder.on_gpu();
            tracing::info!(supported, "HEVC webcam decoder probe");
            supported
        }
        Err(error) => {
            tracing::info!(%error, "HEVC webcam decoder unavailable; advertising H.264 only");
            false
        }
    })
}

fn decode(
    device: DeviceId,
    items: &Receiver<Item>,
    waiting: &AtomicUsize,
    #[cfg(all(target_os = "macos", feature = "macos-native-camera"))] lease: camera_output::Lease,
) {
    #[cfg(all(target_os = "macos", feature = "macos-native-camera"))]
    struct Release(camera_output::Lease);
    #[cfg(all(target_os = "macos", feature = "macos-native-camera"))]
    impl Drop for Release {
        fn drop(&mut self) {
            camera_output::release(self.0);
        }
    }
    #[cfg(all(target_os = "macos", feature = "macos-native-camera"))]
    let _release = Release(lease);
    // On the GPU when there is one: 4K takes about 24 ms a frame in software
    // and 11 ms on a GPU, against 33 ms per frame at 30 fps.
    let mut decoder = match Decoder::new_gpu("h264") {
        Ok(decoder) => decoder,
        Err(e) => {
            tracing::error!(error = %e, "no H.264 decoder for webcam");
            let fail = || {
                state(|s| {
                    s.phase =
                        Phase::Ended { reason: "This PC can't decode the phone's camera video.".into() };
                });
            };
            #[cfg(all(target_os = "macos", feature = "macos-native-camera"))]
            let _ = camera_output::with_owner(lease, fail);
            #[cfg(not(all(target_os = "macos", feature = "macos-native-camera")))]
            fail();
            refresh_idle_frame();
            core_host::host().hub.changed(Changes::WEBCAM);
            return;
        }
    };
    tracing::info!(gpu = decoder.on_gpu(), "webcam decoder ready");
    let keyframe_please = || {
        if let Some(node) = core_host::node() {
            core_host::spawn(async move { node.webcam_keyframe(device).await });
        }
    };
    let phone_name = phone_name_for(Some(device));
    let mut size = (0u32, 0u32);
    let mut need_keyframe = true;
    let mut local_mapping = mapping();
    let mut last_mapping_check = Instant::now();
    #[cfg(windows)]
    let mut gpu_publisher = nectarlink_vcam::gpu::GpuPublisher::new();
    #[cfg(windows)]
    let mut shared_transport = false;

    while let Ok(item) = items.recv() {
        #[cfg(all(target_os = "macos", feature = "macos-native-camera"))]
        if camera_output::with_owner(lease, || ()).is_none() {
            break;
        }
        match item {
            Item::Config(config) => {
                if decoder.codec() != config.codec {
                    match Decoder::new_gpu(&config.codec) {
                        Ok(next) => {
                            decoder = next;
                            tracing::info!(codec = %config.codec, gpu = decoder.on_gpu(), "webcam decoder changed");
                        }
                        Err(error) => {
                            tracing::error!(codec = %config.codec, %error, "negotiated webcam decoder unavailable");
                            break;
                        }
                    }
                }
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
                #[cfg(windows)]
                let decoded = decoder.decode_frames(&data, time_us);
                #[cfg(not(windows))]
                let decoded = decoder.decode(&data, time_us);
                let pictures = match decoded {
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
                    let flip = mirror();
                    let preview = PREVIEW_ACTIVE.load(Ordering::Relaxed);
                    #[cfg(not(windows))]
                    let published = false;
                    #[cfg(windows)]
                    let published = if !flip {
                        match picture.gpu_surface() {
                            Ok(Some((texture, index))) => match gpu_publisher.publish(
                                &texture,
                                index,
                                &picture.sample,
                                (src_w, src_h),
                                picture.colors.bt601,
                                picture.colors.full_range,
                            ) {
                                Ok(published) => published,
                                Err(error) => {
                                    tracing::warn!(%error,"GPU webcam sharing unavailable; using CPU frames");
                                    gpu_publisher.disable();
                                    false
                                }
                            },
                            _ => false,
                        }
                    } else {
                        false
                    };
                    #[cfg(windows)]
                    if shared_transport != published {
                        tracing::info!(shared_textures = published, "webcam transport changed");
                        shared_transport = published;
                    }
                    if published && !preview {
                        continue;
                    }
                    #[cfg(windows)]
                    let cpu_picture = match decoder.read_frame(picture) {
                        Ok(picture) => picture,
                        Err(error) => {
                            tracing::warn!(%error,"webcam frame readback failed");
                            need_keyframe = true;
                            keyframe_please();
                            continue;
                        }
                    };
                    #[cfg(windows)]
                    let picture = &cpu_picture;
                    let mut bgrx = to_bgrx(picture, w, h);
                    if flip {
                        mirror_bgrx_in_place(&mut bgrx, src_w, src_h);
                    }
                    #[cfg(all(target_os = "macos", feature = "macos-native-camera"))]
                    if let Err(error) = camera_output::publish(lease, src_w, src_h, &bgrx) {
                        tracing::warn!(%error, "native camera output ended");
                        break;
                    }
                    if local_mapping.is_none() && last_mapping_check.elapsed() >= Duration::from_secs(1) {
                        local_mapping = mapping();
                        last_mapping_check = Instant::now();
                    }
                    if !published && let Some(ref map) = local_mapping {
                        // Already flipped in place if `flip` is true, so pass `false` here.
                        map.write_frame(&phone_name, src_w, src_h, time_us, &bgrx, false);
                    }
                    // Publish the CPU fallback before deselecting the previous
                    // GPU frame so a transport switch cannot expose an idle gap.
                    #[cfg(windows)]
                    if !published {
                        gpu_publisher.idle();
                    }
                    if preview {
                        #[cfg(all(target_os = "macos", feature = "macos-native-camera"))]
                        let _ = camera_output::with_owner(lease, || {
                            ffi::video_frame(PREVIEW_STREAM, src_w, src_h, &bgrx)
                        });
                        #[cfg(not(all(target_os = "macos", feature = "macos-native-camera")))]
                        ffi::video_frame(PREVIEW_STREAM, src_w, src_h, &bgrx);
                    }
                }
            }
            Item::Ended => break,
        }
    }

    #[cfg(windows)]
    drop(gpu_publisher);
    let finish = || {
        state(|s| {
            if matches!(s.phase, Phase::Streaming { device: d, .. } if d == device) {
                s.phase = Phase::Idle;
            }
        });
    };
    #[cfg(all(target_os = "macos", feature = "macos-native-camera"))]
    let _ = camera_output::with_owner(lease, finish);
    #[cfg(not(all(target_os = "macos", feature = "macos-native-camera")))]
    finish();
    refresh_idle_frame();
    core_host::host().hub.changed(Changes::WEBCAM);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hiding_one_preview_does_not_disable_another_visible_view() {
        let requests = PreviewRequests(AtomicU32::new(0));
        assert!(requests.set(HOME_PREVIEW, true));
        assert!(requests.set(SETTINGS_PREVIEW, true));
        assert!(requests.set(HOME_PREVIEW, false));
        assert!(!requests.set(SETTINGS_PREVIEW, false));
        assert!(requests.set(HOME_PREVIEW, true));
        requests.clear();
        assert!(!requests.set(SETTINGS_PREVIEW, false));
    }

    #[test]
    fn maps_720_1080_and_2160_4k_dimensions() {
        assert_eq!(dimensions("720"), (1280, 720));
        assert_eq!(dimensions("1080"), (1920, 1080));
        assert_eq!(dimensions("2160"), (3840, 2160));
        assert_eq!(dimensions("4k"), (3840, 2160));
        assert_eq!(dimensions("4K"), (3840, 2160));
        assert_eq!(dimensions("unknown"), (1280, 720));
    }

    #[test]
    fn the_resolution_comes_from_the_short_side() {
        assert_eq!(resolution_of(1280, 720), 720);
        assert_eq!(resolution_of(1920, 1080), 1080);
        assert_eq!(resolution_of(1080, 1920), 1080);
        assert_eq!(resolution_of(3840, 2160), 2160);
        assert_eq!(resolution_of(2160, 3840), 2160);
        // Encoders round to their block size.
        assert_eq!(resolution_of(1920, 1088), 1080);
    }

    #[test]
    fn selects_and_validates_720_1080_and_2160_resolutions() {
        set_resolution("720");
        assert_eq!(height(), 720);
        assert_eq!(resolution(), "720");

        set_resolution("1080");
        assert_eq!(height(), 1080);
        assert_eq!(resolution(), "1080");

        set_resolution("2160");
        assert_eq!(height(), 2160);
        assert_eq!(resolution(), "2160");

        // Invalid resolution string leaves selection unchanged.
        set_resolution("9999");
        assert_eq!(height(), 2160);
        assert_eq!(resolution(), "2160");

        set_height(720);
        assert_eq!(height(), 720);
    }
}
