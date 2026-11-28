// SPDX-License-Identifier: GPL-3.0-or-later
//! Shares the PC's display with a paired phone over `mirror` / `video` (session 0).
//! Encodes H.264 or HEVC using Media Foundation on Windows and FFmpeg or OpenH264
//! on Linux, and handles remote touch, keyboard, and pointer input.

#[cfg(any(windows, target_os = "linux"))]
mod imp {

    #![allow(unsafe_code)]

    #[cfg(windows)]
    use std::mem::{ManuallyDrop, size_of};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    };
    #[cfg(any(windows, target_os = "linux"))]
    use std::time::{Duration, Instant};

    use nectarlink_core::{DeviceId, KeyMod, MirrorInput, MirrorStart, mirror_keys};
    #[cfg(any(windows, target_os = "linux"))]
    use nectarlink_core::{MirrorConfig, MirrorSend, PacketKind};
    #[cfg(windows)]
    use windows::Win32::{
        Foundation::{E_FAIL, E_INVALIDARG, E_NOTIMPL, ERROR_TIMEOUT, POINT, RECT},
        Graphics::{
            Direct3D11::{D3D11_TEXTURE2D_DESC, ID3D11Device, ID3D11Texture2D},
            Gdi::{
                BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleBitmap, CreateCompatibleDC,
                DESKTOPHORZRES, DESKTOPVERTRES, DeleteDC, DeleteObject, GetDC, GetDIBits, GetDeviceCaps,
                HALFTONE, ReleaseDC, SRCCOPY, SelectObject, SetStretchBltMode, StretchBlt,
            },
        },
        Media::MediaFoundation::{
            CODECAPI_AVEncCommonMeanBitRate, CODECAPI_AVEncMPVGOPSize, CODECAPI_AVEncVideoForceKeyFrame,
            CODECAPI_AVLowLatencyMode, ICodecAPI, IMF2DBuffer, IMFActivate, IMFAsyncCallback,
            IMFAsyncCallback_Impl, IMFAsyncResult, IMFAttributes, IMFDXGIDeviceManager, IMFMediaBuffer,
            IMFMediaEventGenerator, IMFMediaType, IMFSample, IMFTransform, MEError, METransformHaveOutput,
            METransformNeedInput, MF_E_NOTACCEPTING, MF_E_OPERATION_CANCELLED,
            MF_E_TRANSFORM_NEED_MORE_INPUT, MF_E_TRANSFORM_STREAM_CHANGE, MF_LOW_LATENCY, MF_MT_AVG_BITRATE,
            MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE,
            MF_MT_MPEG_SEQUENCE_HEADER, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE, MF_SA_D3D11_AWARE,
            MF_TRANSFORM_ASYNC, MF_TRANSFORM_ASYNC_UNLOCK, MFCreateAttributes, MFCreateDXGIDeviceManager,
            MFCreateDXGISurfaceBuffer, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample,
            MFMediaType_Video, MFSampleExtension_CleanPoint, MFT_CATEGORY_VIDEO_ENCODER,
            MFT_ENUM_ADAPTER_LUID, MFT_ENUM_FLAG_ASYNCMFT, MFT_ENUM_FLAG_HARDWARE, MFT_ENUM_FLAG_LOCALMFT,
            MFT_ENUM_FLAG_SORTANDFILTER, MFT_ENUM_FLAG_SYNCMFT, MFT_MESSAGE_COMMAND_FLUSH,
            MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, MFT_MESSAGE_NOTIFY_END_STREAMING,
            MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_MESSAGE_SET_D3D_MANAGER, MFT_OUTPUT_DATA_BUFFER,
            MFT_OUTPUT_STREAM_PROVIDES_SAMPLES, MFT_REGISTER_TYPE_INFO, MFTEnum2, MFVideoFormat_H264,
            MFVideoFormat_HEVC, MFVideoFormat_NV12, MFVideoInterlace_Progressive,
        },
        System::{Com::CoTaskMemFree, Variant::VARIANT},
        UI::WindowsAndMessaging::{
            CURSOR_SHOWING, CURSORINFO, DI_NORMAL, DrawIconEx, GetCursorInfo, GetSystemMetrics, SM_CXSCREEN,
            SM_CYSCREEN,
        },
    };
    #[cfg(windows)]
    use windows::core::Interface;

    #[cfg(windows)]
    use crate::win::{
        h264::{ensure_mf_started, video_adapters},
        screen_capture::{DxgiScreenCapture, GpuBgraToNv12},
    };
    use crate::{core_host, win};

    struct ActiveSession {
        peer: DeviceId,
        stop: Arc<AtomicBool>,
        force_keyframe: Arc<AtomicBool>,
        /// Play on phone: the phone watches a player, and its input is ignored.
        watching: bool,
        video_ready: bool,
        /// The screen shown, when it isn't the main one (#442): taps land there.
        #[cfg(windows)]
        area: Option<RECT>,
        #[cfg(target_os = "linux")]
        input_portal: Option<Arc<win::input::RemoteDesktopPortal>>,
    }

    static ACTIVE: Mutex<Option<ActiveSession>> = Mutex::new(None);

    /// How long Play on phone looks for the player's window before streaming
    /// the screen instead.
    #[cfg(any(windows, target_os = "linux"))]
    const FIND_WINDOW: Duration = Duration::from_secs(5);

    /// The lowest bitrate the stream backs off to on a slow link (encoders
    /// go no lower).
    #[cfg(any(windows, target_os = "linux"))]
    const MIN_BITRATE: u32 = 500_000;

    /// What a sharing session sent in the last stats window.
    #[cfg(any(windows, target_os = "linux"))]
    #[derive(Default)]
    struct Stats {
        frames: u64,
        keyframes: u64,
        bytes: u64,
        dropped: u64,
        rebuilds: u64,
    }

    /// Returns the device currently viewing this PC's screen, if any.
    pub fn active_peer() -> Option<DeviceId> {
        ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|s| s.peer)
    }

    fn remote_input_allowed(peer: DeviceId) -> bool {
        core_host::node()
            .and_then(|n| n.device_toggles(peer).ok())
            .is_some_and(|toggles| toggles.iter().any(|(k, v)| *k == "remote_input" && *v))
    }

    /// Whether this PC's screen is currently being shared with a phone.
    pub fn is_active() -> bool {
        active_peer().is_some()
    }

    /// Whether this PC's display session supports screen capture for `mirror.capture`.
    ///
    /// Linux needs an H.264 encoder and either X11 or Wayland capture tools.
    pub fn is_capture_available() -> bool {
        #[cfg(windows)]
        {
            true
        }
        #[cfg(target_os = "linux")]
        {
            let can_encode = openh264_enc_api().is_some();
            #[cfg(linux_ffmpeg_video)]
            let can_encode = can_encode || {
                static HARDWARE_AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
                *HARDWARE_AVAILABLE.get_or_init(|| {
                    crate::win::video_encoder::Encoder::new("h264", 320, 240, 30, 1_000_000).is_ok()
                })
            };
            if !can_encode {
                return false;
            }
            // XWayland's root window isn't the desktop: a Wayland session
            // shares the screen through the portal only (#548).
            if wayland_session() {
                // The session bus can start the portal on demand. Its current
                // ownership must not decide whether phones can request capture.
                pipewire_capture_installed()
            } else {
                std::env::var_os("DISPLAY").is_some() && x11_api().is_some()
            }
        }
    }

    /// Whether a phone watches one of this PC's players (Play on phone).
    pub fn is_watching() -> bool {
        ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|s| s.watching)
    }

    /// Stops a phone watching one of this PC's players. That ends the
    /// session: a watching session is the one capture there is.
    pub fn stop_watching() {
        let peer =
            ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().filter(|s| s.watching).map(|s| s.peer);
        if let Some(peer) = peer {
            stop_for(&peer);
        }
    }

    /// Starts or replaces the active PC-to-phone screen mirror stream for `peer`.
    pub fn start(peer: DeviceId, options: MirrorStart) {
        let stop = Arc::new(AtomicBool::new(false));
        let force_keyframe = Arc::new(AtomicBool::new(false));
        let previous = {
            let mut guard = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(previous) = guard.as_ref() {
                previous.stop.store(true, Ordering::Release);
            }
            guard.replace(ActiveSession {
                peer,
                stop: Arc::clone(&stop),
                force_keyframe: Arc::clone(&force_keyframe),
                watching: options.watch.is_some(),
                video_ready: false,
                #[cfg(windows)]
                area: None,
                #[cfg(target_os = "linux")]
                input_portal: None,
            })
        };
        if let Some(previous) = previous {
            win::input::release_held_buttons();
            crate::input_share::leave_remote();
            // A stop for this phone could arrive after the replacement started.
            if previous.peer != peer {
                notify_stopped(previous.peer);
            }
        }
        core_host::host().hub.changed(crate::state::Changes::REMOTE);
        core_host::refresh_capabilities();

        let stop_on_failure = Arc::clone(&stop);
        if let Err(e) = std::thread::Builder::new()
            .name("nectarlink-pc-mirror".into())
            .spawn(move || run_capture_loop(peer, options, stop, force_keyframe))
        {
            // Not a session that claims to share with nothing behind it.
            tracing::warn!(error = %e, "can't start sharing this PC's screen");
            cleanup_session(&stop_on_failure);
        }
    }

    /// Stops PC-to-phone mirroring if `peer` matches the active viewer.
    pub fn stop_for(peer: &DeviceId) {
        let stopped = {
            let mut guard = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
            if guard.as_ref().is_some_and(|s| &s.peer == peer)
                && let Some(s) = guard.take()
            {
                s.stop.store(true, Ordering::SeqCst);
                true
            } else {
                false
            }
        };
        if stopped {
            win::input::release_held_buttons();
            crate::input_share::leave_remote();
            core_host::host().hub.changed(crate::state::Changes::REMOTE);
            core_host::refresh_capabilities();
        }
    }

    /// Stops any active PC-to-phone mirroring session.
    pub fn stop_all() {
        let stopped = {
            let mut guard = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(s) = guard.take() {
                s.stop.store(true, Ordering::SeqCst);
                Some(s.peer)
            } else {
                None
            }
        };
        if let Some(peer) = stopped {
            win::input::release_held_buttons();
            crate::input_share::leave_remote();
            notify_stopped(peer);
            core_host::host().hub.changed(crate::state::Changes::REMOTE);
            core_host::refresh_capabilities();
        }
    }

    fn notify_stopped(peer: DeviceId) {
        if let Some(node) = core_host::node() {
            node.set_pc_mirror_session_allowed(peer, false);
            core_host::spawn(async move {
                node.mirror_stop(peer, nectarlink_core::MIRROR_SCREEN).await;
            });
        }
    }

    /// Requests an immediate IDR keyframe for `peer`.
    pub fn request_keyframe(peer: &DeviceId) {
        let guard = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = guard.as_ref()
            && &s.peer == peer
        {
            s.force_keyframe.store(true, Ordering::SeqCst);
        }
    }

    /// Injects touch, scroll, key, or text input from a phone viewing the PC screen.
    pub fn handle_input(peer: &DeviceId, session: u32, input: MirrorInput) {
        if session != nectarlink_core::MIRROR_SCREEN {
            return;
        }
        #[cfg(target_os = "linux")]
        let input_portal;
        {
            let guard = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
            let Some(active) = guard.as_ref().filter(|s| s.peer == *peer && !s.watching && s.video_ready)
            else {
                return;
            };
            #[cfg(target_os = "linux")]
            {
                input_portal = active.input_portal.clone();
            }
            // Taps go to the screen this session shows, whatever another
            // session set before it.
            #[cfg(windows)]
            win::input::set_absolute_area(active.area);
        }
        if !remote_input_allowed(*peer) {
            return;
        }
        let _ = crate::input_share::enter_remote(*peer);
        let inject = || match input {
            MirrorInput::Touch { action, x, y } => {
                win::input::mirror_touch(action, x, y);
            }
            MirrorInput::Scroll { x, y, dx, dy } => {
                win::input::move_absolute(x, y);
                win::input::scroll(dx, dy);
            }
            MirrorInput::Text { text } => {
                win::input::type_text(&text);
            }
            MirrorInput::Key { key } => match key.as_str() {
                mirror_keys::BACK => win::input::press_key("escape", &[]),
                mirror_keys::HOME => win::input::press_key("a", &[KeyMod::Win]),
                mirror_keys::RECENTS => win::input::press_key("task_view", &[]),
                mirror_keys::ENTER => win::input::press_key("enter", &[]),
                mirror_keys::BACKSPACE => win::input::press_key("backspace", &[]),
                mirror_keys::DELETE => win::input::press_key("delete", &[]),
                mirror_keys::LEFT => win::input::press_key("left", &[]),
                mirror_keys::RIGHT => win::input::press_key("right", &[]),
                mirror_keys::UP => win::input::press_key("up", &[]),
                mirror_keys::DOWN => win::input::press_key("down", &[]),
                mirror_keys::TAB => win::input::press_key("tab", &[]),
                mirror_keys::NOTIFICATIONS => win::input::press_key("n", &[KeyMod::Win]),
                _ => {}
            },
            MirrorInput::Gamepad { .. } => {}
        };
        #[cfg(windows)]
        inject();
        #[cfg(target_os = "linux")]
        win::input::with_screencast_input(input_portal, inject);
    }

    pub fn on_event(event: &nectarlink_core::NodeEvent) {
        match event {
            nectarlink_core::NodeEvent::LinkChanged {
                device,
                link: nectarlink_core::LinkState::Offline { .. },
            }
            | nectarlink_core::NodeEvent::DeviceRemoved(device) => {
                stop_for(device);
            }
            nectarlink_core::NodeEvent::PcMirrorRequested { .. } => {
                core_host::host().hub.update(|s| s.apply(event));
                crate::bridge::app::request_activation();
            }
            _ => {}
        }
    }

    /// Computes target encoder dimensions (multiples of 16) fitting within `max_size`.
    pub(crate) fn target_dimensions(src_w: u32, src_h: u32, max_size: u32) -> (u32, u32) {
        let max_edge = max_size.clamp(320, 1920) as f64;
        let sw = src_w.max(64) as f64;
        let sh = src_h.max(64) as f64;
        let longest = sw.max(sh);
        let scale = if longest > max_edge { max_edge / longest } else { 1.0 };
        let w = (((sw * scale).round() as u32) / 16).max(4) * 16;
        let h = (((sh * scale).round() as u32) / 16).max(4) * 16;
        (w, h)
    }

    /// Converts a top-down 32-bit BGRA image (`width * height * 4` bytes) into
    /// BT.709 limited-range NV12 (`width * height * 3 / 2` bytes).
    pub(crate) fn bgra_to_nv12(bgra: &[u8], width: usize, height: usize, nv12: &mut Vec<u8>) {
        let y_size = width * height;
        let uv_size = y_size / 2;
        nv12.resize(y_size + uv_size, 0);
        let (y_plane, uv_plane) = nv12.split_at_mut(y_size);

        for row in 0..height {
            let src_row = row * width * 4;
            let y_row = row * width;
            for col in 0..width {
                let px = src_row + col * 4;
                let b = i32::from(bgra[px]);
                let g = i32::from(bgra[px + 1]);
                let r = i32::from(bgra[px + 2]);
                let y = ((47 * r + 157 * g + 16 * b + 128) >> 8) + 16;
                y_plane[y_row + col] = y.clamp(16, 235) as u8;
            }
        }

        for row in (0..height).step_by(2) {
            let src0 = row * width * 4;
            let src1 = (row + 1).min(height - 1) * width * 4;
            let uv_row = (row / 2) * width;
            for col in (0..width).step_by(2) {
                let p00 = src0 + col * 4;
                let p01 = src0 + (col + 1).min(width - 1) * 4;
                let p10 = src1 + col * 4;
                let p11 = src1 + (col + 1).min(width - 1) * 4;

                let b = (i32::from(bgra[p00])
                    + i32::from(bgra[p01])
                    + i32::from(bgra[p10])
                    + i32::from(bgra[p11])
                    + 2)
                    >> 2;
                let g = (i32::from(bgra[p00 + 1])
                    + i32::from(bgra[p01 + 1])
                    + i32::from(bgra[p10 + 1])
                    + i32::from(bgra[p11 + 1])
                    + 2)
                    >> 2;
                let r = (i32::from(bgra[p00 + 2])
                    + i32::from(bgra[p01 + 2])
                    + i32::from(bgra[p10 + 2])
                    + i32::from(bgra[p11 + 2])
                    + 2)
                    >> 2;

                let u = ((-26 * r - 87 * g + 112 * b + 128) >> 8) + 128;
                let v = ((112 * r - 102 * g - 10 * b + 128) >> 8) + 128;
                uv_plane[uv_row + col] = u.clamp(16, 240) as u8;
                uv_plane[uv_row + col + 1] = v.clamp(16, 240) as u8;
            }
        }
    }

    #[cfg(windows)]
    fn video_encoder_activations(
        output_subtype: windows::core::GUID,
        adapter: Option<u64>,
    ) -> windows::core::Result<Vec<IMFActivate>> {
        let input_info =
            MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: MFVideoFormat_NV12 };
        let output_info =
            MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: output_subtype };
        let flags = if adapter.is_some() {
            MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SORTANDFILTER
        } else {
            MFT_ENUM_FLAG_SYNCMFT
                | MFT_ENUM_FLAG_ASYNCMFT
                | MFT_ENUM_FLAG_LOCALMFT
                | MFT_ENUM_FLAG_SORTANDFILTER
        };
        let mut activates: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut count = 0u32;
        // SAFETY: MFTEnum2 returns a CoTaskMemAlloc array; take each interface before freeing it.
        unsafe {
            let mut attributes: Option<IMFAttributes> = None;
            if let Some(adapter) = adapter {
                MFCreateAttributes(&mut attributes, 1)?;
                attributes
                    .as_ref()
                    .ok_or_else(|| windows::core::Error::from(E_INVALIDARG))?
                    .SetBlob(&MFT_ENUM_ADAPTER_LUID, &adapter.to_le_bytes())?;
            }
            MFTEnum2(
                MFT_CATEGORY_VIDEO_ENCODER,
                flags,
                Some(&input_info),
                Some(&output_info),
                attributes.as_ref(),
                &mut activates,
                &mut count,
            )?;
            let mut candidates = Vec::new();
            if !activates.is_null() {
                let entries = std::slice::from_raw_parts_mut(activates, count as usize);
                for entry in entries {
                    if let Some(activation) = entry.take() {
                        candidates.push(activation);
                    }
                }
                CoTaskMemFree(Some(activates.cast()));
            }
            Ok(candidates)
        }
    }

    #[cfg(windows)]
    fn create_video_encoder(
        subtype: windows::core::GUID,
        preferred_adapter: Option<u64>,
        configure: impl FnMut(IMFTransform) -> windows::core::Result<ScreenEncoder>,
    ) -> windows::core::Result<ScreenEncoder> {
        let mut candidates = Vec::new();
        if let Some(luid) = preferred_adapter {
            match video_encoder_activations(subtype, Some(luid)) {
                Ok(found) => candidates.extend(found),
                Err(error) => {
                    tracing::debug!(%error, adapter = luid, "GPU encoder enumeration failed")
                }
            }
            if candidates.is_empty() {
                return Err(windows::core::Error::from(E_FAIL));
            }
            return try_video_encoders(candidates, configure);
        }
        if let Ok(adapters) = video_adapters() {
            for adapter in adapters {
                match video_encoder_activations(subtype, Some(adapter.luid)) {
                    Ok(found) => candidates.extend(found),
                    Err(error) => {
                        tracing::debug!(%error, adapter = adapter.luid, "GPU encoder enumeration failed")
                    }
                }
            }
        }
        match video_encoder_activations(subtype, None) {
            Ok(found) => candidates.extend(found),
            Err(error) if candidates.is_empty() => return Err(error),
            Err(error) => tracing::debug!(%error, "Software encoder enumeration failed"),
        }
        try_video_encoders(candidates, configure)
    }

    #[cfg(windows)]
    fn try_video_encoders(
        candidates: Vec<IMFActivate>,
        mut configure: impl FnMut(IMFTransform) -> windows::core::Result<ScreenEncoder>,
    ) -> windows::core::Result<ScreenEncoder> {
        let mut last_error = windows::core::Error::from(E_INVALIDARG);
        let mut candidates = candidates.into_iter();
        while let Some(activation) = candidates.next() {
            // SAFETY: each activation owns an encoder; fully configure it before selecting it.
            let result = unsafe { activation.ActivateObject::<IMFTransform>() }.and_then(&mut configure);
            match result {
                Ok(mut encoder) => {
                    encoder.alternatives = candidates.collect();
                    return Ok(encoder);
                }
                Err(error) => {
                    tracing::debug!(%error, "Video encoder candidate unavailable");
                    last_error = error;
                    // SAFETY: this failed candidate will not be used again.
                    let _ = unsafe { activation.ShutdownObject() };
                }
            }
        }
        Err(last_error)
    }

    #[cfg(windows)]
    #[windows::core::implement(IMFAsyncCallback)]
    struct EncoderEventCallback {
        cookie: u32,
        sender: std::sync::mpsc::Sender<windows::core::Result<u32>>,
    }

    #[cfg(windows)]
    impl IMFAsyncCallback_Impl for EncoderEventCallback_Impl {
        fn GetParameters(&self, _: *mut u32, _: *mut u32) -> windows::core::Result<()> {
            Err(E_NOTIMPL.into())
        }

        fn Invoke(&self, result: windows::core::Ref<IMFAsyncResult>) -> windows::core::Result<()> {
            // SAFETY: the global interface table marshals the generator into this callback's apartment.
            let event = unsafe {
                global_interface_table().and_then(|table| {
                    let mut raw = std::ptr::null_mut();
                    table.GetInterfaceFromGlobal(
                        self.cookie,
                        &<IMFMediaEventGenerator as windows::core::Interface>::IID,
                        &mut raw,
                    )?;
                    let generator = <IMFMediaEventGenerator as windows::core::Interface>::from_raw(raw);
                    result.ok().and_then(|result| generator.EndGetEvent(result)).and_then(|event| {
                        let kind = event.GetType()?;
                        if kind == MEError.0 as u32 {
                            event.GetStatus()?.ok()?;
                        }
                        Ok(kind)
                    })
                })
            };
            let _ = self.sender.send(event);
            Ok(())
        }
    }

    #[cfg(windows)]
    unsafe fn global_interface_table()
    -> windows::core::Result<windows::Win32::System::Com::IGlobalInterfaceTable> {
        use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
        const CLSID_STD_GLOBAL_INTERFACE_TABLE: windows::core::GUID =
            windows::core::GUID::from_u128(0x00000323_0000_0000_c000_000000000046);
        unsafe { CoCreateInstance(&CLSID_STD_GLOBAL_INTERFACE_TABLE, None, CLSCTX_INPROC_SERVER) }
    }

    #[cfg(windows)]
    struct EncoderEvents {
        generator: IMFMediaEventGenerator,
        table: windows::Win32::System::Com::IGlobalInterfaceTable,
        cookie: u32,
        callback: IMFAsyncCallback,
        receiver: std::sync::mpsc::Receiver<windows::core::Result<u32>>,
        pending: bool,
    }

    #[cfg(windows)]
    impl EncoderEvents {
        fn new(generator: IMFMediaEventGenerator) -> windows::core::Result<Self> {
            // SAFETY: registers the interface so the callback can retrieve it in its own apartment.
            let table = unsafe { global_interface_table()? };
            let cookie = unsafe {
                table.RegisterInterfaceInGlobal(
                    &generator,
                    &<IMFMediaEventGenerator as windows::core::Interface>::IID,
                )?
            };
            let (sender, receiver) = std::sync::mpsc::channel();
            let callback = EncoderEventCallback { cookie, sender }.into();
            Ok(Self { generator, table, cookie, callback, receiver, pending: false })
        }

        fn receive(&mut self, timeout: Duration) -> windows::core::Result<Option<u32>> {
            if !self.pending {
                // SAFETY: only one event request is pending at a time.
                unsafe { self.generator.BeginGetEvent(&self.callback, None)? };
                self.pending = true;
            }
            match self.receiver.recv_timeout(timeout) {
                Ok(event) => {
                    self.pending = false;
                    event.map(Some)
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Ok(None),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(E_INVALIDARG.into()),
            }
        }
    }

    #[cfg(windows)]
    impl Drop for EncoderEvents {
        fn drop(&mut self) {
            // SAFETY: this cookie is owned by this registration and is revoked once.
            let _ = unsafe { self.table.RevokeInterfaceFromGlobal(self.cookie) };
        }
    }

    #[cfg(windows)]
    pub(crate) struct ScreenEncoder {
        codec: String,
        transform: IMFTransform,
        /// Kept after `transform` so the MFT finishes flushing while the DXGI device manager is valid.
        d3d_manager: Option<(ID3D11Device, IMFDXGIDeviceManager)>,
        async_events: Option<EncoderEvents>,
        pending_input: u32,
        pending_output: u32,
        /// The encoder's settings interface, when it has one: keyframes on demand and live
        /// bitrate changes without rebuilding the encoder.
        codec_api: Option<ICodecAPI>,
        width: u32,
        height: u32,
        provides_samples: bool,
        output_size: u32,
        output_sample: Option<IMFSample>,
        seq_header: Vec<u8>,
        sent_first_frame: bool,
        alternatives: Vec<IMFActivate>,
    }

    #[cfg(windows)]
    impl Drop for ScreenEncoder {
        fn drop(&mut self) {
            // SAFETY: flushes and ends streaming on the MFT before drop.
            unsafe {
                let _ = self.transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
                let _ = self.transform.ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0);
            }
        }
    }

    #[cfg(windows)]
    impl ScreenEncoder {
        pub(crate) fn new_for_codec(
            codec: &str,
            width: u32,
            height: u32,
            fps: u32,
            bitrate: u32,
        ) -> windows::core::Result<Self> {
            ensure_mf_started()?;
            let subtype = match codec {
                "h264" => MFVideoFormat_H264,
                "h265" => MFVideoFormat_HEVC,
                _ => return Err(windows::core::Error::from(E_INVALIDARG)),
            };
            create_video_encoder(subtype, None, |transform| {
                Self::from_transform(codec, transform, width, height, fps, bitrate, None)
            })
        }

        pub(crate) fn new_gpu(
            codec: &str,
            width: u32,
            height: u32,
            fps: u32,
            bitrate: u32,
            device: &ID3D11Device,
            adapter_luid: u64,
        ) -> windows::core::Result<Self> {
            ensure_mf_started()?;
            let subtype = match codec {
                "h264" => MFVideoFormat_H264,
                "h265" => MFVideoFormat_HEVC,
                _ => return Err(windows::core::Error::from(E_INVALIDARG)),
            };
            let mut token = 0u32;
            let mut manager: Option<IMFDXGIDeviceManager> = None;
            // SAFETY: creates a DXGI device manager bound to the caller's D3D11 device.
            unsafe {
                MFCreateDXGIDeviceManager(&mut token, &mut manager)?;
            }
            let manager = manager.ok_or_else(|| windows::core::Error::from(E_FAIL))?;
            // SAFETY: `device` is a live multithread-protected `ID3D11Device` and `token` came from `MFCreateDXGIDeviceManager`.
            unsafe { manager.ResetDevice(device, token)? };
            let gpu = (device.clone(), manager);
            create_video_encoder(subtype, Some(adapter_luid), |transform| {
                Self::from_transform(codec, transform, width, height, fps, bitrate, Some(gpu.clone()))
            })
        }

        pub(crate) fn on_gpu(&self) -> bool {
            self.d3d_manager.is_some()
        }

        pub(crate) fn matches_gpu_device(&self, device: &ID3D11Device) -> bool {
            self.d3d_manager.as_ref().is_some_and(|(d, _)| d.as_raw() == device.as_raw())
        }

        fn from_transform(
            codec: &str,
            transform: IMFTransform,
            width: u32,
            height: u32,
            fps: u32,
            bitrate: u32,
            d3d_manager: Option<(ID3D11Device, IMFDXGIDeviceManager)>,
        ) -> windows::core::Result<Self> {
            // SAFETY: Media Foundation COM calls with valid parameters.
            unsafe {
                let subtype = match codec {
                    "h264" => MFVideoFormat_H264,
                    "h265" => MFVideoFormat_HEVC,
                    _ => return Err(windows::core::Error::from(E_INVALIDARG)),
                };
                let attributes = transform.GetAttributes().ok();
                let is_async = attributes
                    .as_ref()
                    .is_some_and(|attrs| attrs.GetUINT32(&MF_TRANSFORM_ASYNC).is_ok_and(|value| value != 0));
                if is_async {
                    attributes
                        .as_ref()
                        .ok_or_else(|| windows::core::Error::from(E_INVALIDARG))?
                        .SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1)?;
                }
                if let Some(attrs) = attributes.as_ref() {
                    let _ = attrs.SetUINT32(&MF_LOW_LATENCY, 1);
                }
                if let Some((_, manager)) = d3d_manager.as_ref() {
                    let aware = attributes
                        .as_ref()
                        .is_some_and(|a| a.GetUINT32(&MF_SA_D3D11_AWARE).is_ok_and(|v| v != 0));
                    if !aware {
                        return Err(windows::core::Error::from(E_FAIL));
                    }
                    transform.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, manager.as_raw() as usize)?;
                }

                let out_type: IMFMediaType = MFCreateMediaType()?;
                out_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                out_type.SetGUID(&MF_MT_SUBTYPE, &subtype)?;
                if codec == "h264" {
                    let _ = out_type.SetUINT32(
                        &windows::Win32::Media::MediaFoundation::MF_MT_MPEG2_PROFILE,
                        windows::Win32::Media::MediaFoundation::eAVEncH264VProfile_Base.0 as u32,
                    );
                } else {
                    let _ = out_type.SetUINT32(
                        &windows::Win32::Media::MediaFoundation::MF_MT_MPEG2_PROFILE,
                        windows::Win32::Media::MediaFoundation::eAVEncH265VProfile_Main_420_8.0 as u32,
                    );
                }
                out_type.SetUINT32(&MF_MT_AVG_BITRATE, bitrate.clamp(500_000, 20_000_000))?;
                out_type.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
                out_type.SetUINT64(&MF_MT_FRAME_SIZE, (u64::from(width) << 32) | u64::from(height))?;
                out_type.SetUINT64(&MF_MT_FRAME_RATE, (u64::from(fps.max(1)) << 32) | 1)?;
                out_type.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)?;
                transform.SetOutputType(0, Some(&out_type), 0)?;

                let in_type: IMFMediaType = MFCreateMediaType()?;
                in_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                in_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
                in_type.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
                in_type.SetUINT64(&MF_MT_FRAME_SIZE, (u64::from(width) << 32) | u64::from(height))?;
                in_type.SetUINT64(&MF_MT_FRAME_RATE, (u64::from(fps.max(1)) << 32) | 1)?;
                in_type.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)?;
                transform.SetInputType(0, Some(&in_type), 0)?;

                let mut seq_header = Vec::new();
                if let Ok(cur_out) = transform.GetOutputCurrentType(0)
                    && let Ok(len) = cur_out.GetBlobSize(&MF_MT_MPEG_SEQUENCE_HEADER)
                    && len > 0
                {
                    let mut buf = vec![0u8; len as usize];
                    if cur_out.GetBlob(&MF_MT_MPEG_SEQUENCE_HEADER, &mut buf, None).is_ok() {
                        seq_header = buf;
                    }
                }

                let info = transform.GetOutputStreamInfo(0)?;
                let provides_samples = (info.dwFlags & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32)) != 0;
                let output_size = info.cbSize.max(256 * 1024);
                let output_sample =
                    if provides_samples { None } else { Some(allocate_output_sample(output_size)?) };

                // Periodic keyframes allow recovery after packet loss without restarting the encoder.
                let codec_api: Option<ICodecAPI> = windows::core::Interface::cast(&transform).ok();
                let async_events = if is_async {
                    Some(EncoderEvents::new(windows::core::Interface::cast(&transform)?)?)
                } else {
                    None
                };
                if let Some(api) = codec_api.as_ref() {
                    let _ = api.SetValue(&CODECAPI_AVLowLatencyMode, &VARIANT::from(true));
                    let _ = api.SetValue(&CODECAPI_AVEncMPVGOPSize, &VARIANT::from(fps.max(1) * 2));
                }

                transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
                transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;

                Ok(Self {
                    codec: codec.to_owned(),
                    transform,
                    d3d_manager,
                    async_events,
                    pending_input: 0,
                    pending_output: 0,
                    codec_api,
                    width,
                    height,
                    provides_samples,
                    output_size,
                    output_sample,
                    seq_header,
                    sent_first_frame: false,
                    alternatives: Vec::new(),
                })
            }
        }

        fn next_encoder(&mut self, codec: &str, width: u32, height: u32, fps: u32, bitrate: u32) -> bool {
            let candidates = std::mem::take(&mut self.alternatives);
            let d3d_manager = self.d3d_manager.clone();
            match try_video_encoders(candidates, |transform| {
                Self::from_transform(codec, transform, width, height, fps, bitrate, d3d_manager.clone())
            }) {
                Ok(encoder) => {
                    *self = encoder;
                    true
                }
                Err(_) => false,
            }
        }

        /// Asks for the next frame to be a keyframe. False when the encoder can't, so the
        /// caller must rebuild it instead.
        pub(crate) fn request_keyframe(&self) -> bool {
            // SAFETY: ICodecAPI::SetValue with a valid GUID and VARIANT.
            self.codec_api.as_ref().is_some_and(|api| {
                unsafe { api.SetValue(&CODECAPI_AVEncVideoForceKeyFrame, &VARIANT::from(1u32)) }.is_ok()
            })
        }

        /// Changes the target bitrate in place. False when the encoder can't.
        pub(crate) fn set_bitrate(&self, bitrate: u32) -> bool {
            // SAFETY: ICodecAPI::SetValue with a valid GUID and VARIANT.
            self.codec_api.as_ref().is_some_and(|api| {
                unsafe { api.SetValue(&CODECAPI_AVEncCommonMeanBitRate, &VARIANT::from(bitrate)) }.is_ok()
            })
        }

        /// Feeds one NV12 frame (`width * height * 3 / 2` bytes) and drains encoded
        /// Annex-B H.264 or H.265 access units `(is_keyframe, bytes)`.
        pub(crate) fn encode(
            &mut self,
            nv12: &[u8],
            hns: i64,
            dur_hns: i64,
            stop: &AtomicBool,
        ) -> windows::core::Result<Vec<(bool, Vec<u8>)>> {
            let expected = (self.width as usize) * (self.height as usize) * 3 / 2;
            if nv12.len() < expected {
                return Ok(Vec::new());
            }
            // SAFETY: Media Foundation sample creation and MFT ProcessInput/ProcessOutput.
            unsafe {
                let sample = make_sample(&nv12[..expected], hns, dur_hns)?;
                self.encode_sample(&sample, stop)
            }
        }

        /// Feeds one GPU NV12 `ID3D11Texture2D` surface without CPU readback and
        /// drains encoded Annex-B H.264 or H.265 access units `(is_keyframe, bytes)`.
        pub(crate) fn encode_surface(
            &mut self,
            tex: &ID3D11Texture2D,
            hns: i64,
            dur_hns: i64,
            stop: &AtomicBool,
        ) -> windows::core::Result<Vec<(bool, Vec<u8>)>> {
            // SAFETY: wraps `tex` in a DXGI surface buffer and submits it to the MFT.
            unsafe {
                let sample = make_dxgi_sample(tex, hns, dur_hns)?;
                self.encode_sample(&sample, stop)
            }
        }

        unsafe fn encode_sample(
            &mut self,
            sample: &IMFSample,
            stop: &AtomicBool,
        ) -> windows::core::Result<Vec<(bool, Vec<u8>)>> {
            self.wait_for_async_event(METransformNeedInput.0 as u32, stop)?;
            // SAFETY: Media Foundation MFT ProcessInput/ProcessOutput on live COM interfaces.
            unsafe {
                let mut out = Vec::new();
                match self.transform.ProcessInput(0, sample, 0) {
                    Ok(()) => {
                        self.wait_for_async_event(METransformHaveOutput.0 as u32, stop)?;
                        self.drain(&mut out)?;
                    }
                    Err(e) if e.code() == MF_E_NOTACCEPTING => {
                        self.wait_for_async_event(METransformHaveOutput.0 as u32, stop)?;
                        self.drain(&mut out)?;
                        self.wait_for_async_event(METransformNeedInput.0 as u32, stop)?;
                        self.transform.ProcessInput(0, sample, 0)?;
                        self.wait_for_async_event(METransformHaveOutput.0 as u32, stop)?;
                        self.drain(&mut out)?;
                    }
                    Err(e) => return Err(e),
                }
                Ok(out)
            }
        }

        fn wait_for_async_event(&mut self, expected: u32, stop: &AtomicBool) -> windows::core::Result<()> {
            let Some(events) = self.async_events.as_mut() else {
                return Ok(());
            };
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                if stop.load(Ordering::Relaxed) {
                    return Err(windows::core::Error::from(MF_E_OPERATION_CANCELLED));
                }
                let pending = if expected == METransformNeedInput.0 as u32 {
                    &mut self.pending_input
                } else {
                    &mut self.pending_output
                };
                if *pending > 0 {
                    *pending -= 1;
                    return Ok(());
                }
                if Instant::now() >= deadline {
                    return Err(windows::core::Error::from(ERROR_TIMEOUT.to_hresult()));
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                let Some(kind) = events.receive(remaining.min(Duration::from_millis(20)))? else {
                    continue;
                };
                if kind == METransformNeedInput.0 as u32 {
                    self.pending_input += 1;
                } else if kind == METransformHaveOutput.0 as u32 {
                    self.pending_output += 1;
                }
            }
        }

        unsafe fn drain(&mut self, out: &mut Vec<(bool, Vec<u8>)>) -> windows::core::Result<()> {
            loop {
                // SAFETY: MFT output draining with reused pre-allocated or MFT-provided sample.
                unsafe {
                    let allocated = if self.provides_samples {
                        None
                    } else {
                        let s = match self.output_sample.take() {
                            Some(s) => s,
                            None => allocate_output_sample(self.output_size)?,
                        };
                        if let Ok(b) = s.GetBufferByIndex(0) {
                            let _ = b.SetCurrentLength(0);
                        }
                        Some(s)
                    };
                    let mut data = MFT_OUTPUT_DATA_BUFFER {
                        dwStreamID: 0,
                        pSample: ManuallyDrop::new(allocated),
                        dwStatus: 0,
                        pEvents: ManuallyDrop::new(None),
                    };
                    let mut status = 0u32;
                    let res = self.transform.ProcessOutput(0, std::slice::from_mut(&mut data), &mut status);
                    let sample = ManuallyDrop::take(&mut data.pSample);
                    let _ = ManuallyDrop::take(&mut data.pEvents);

                    match res {
                        Ok(()) => {
                            if let Some(sample) = sample {
                                let extract_result = (|| -> windows::core::Result<()> {
                                    let clean = sample
                                        .GetUINT32(&MFSampleExtension_CleanPoint)
                                        .map(|v| v != 0)
                                        .unwrap_or(false);
                                    let buf: IMFMediaBuffer = sample.ConvertToContiguousBuffer()?;
                                    let mut ptr = std::ptr::null_mut();
                                    let mut len = 0u32;
                                    buf.Lock(&mut ptr, None, Some(&mut len))?;
                                    let slice = std::slice::from_raw_parts(ptr, len as usize);
                                    let is_idr = !self.sent_first_frame
                                        || clean
                                        || match self.codec.as_str() {
                                            "h265" => has_h265_irap(slice),
                                            _ => has_h264_idr(slice),
                                        };
                                    let mut bytes = Vec::new();
                                    let has_parameters = match self.codec.as_str() {
                                        "h265" => has_h265_parameters(slice),
                                        _ => has_h264_sps(slice),
                                    };
                                    if is_idr && !has_parameters {
                                        if self.seq_header.is_empty()
                                            && let Ok(cur_out) = self.transform.GetOutputCurrentType(0)
                                            && let Ok(hdr_len) =
                                                cur_out.GetBlobSize(&MF_MT_MPEG_SEQUENCE_HEADER)
                                            && hdr_len > 0
                                        {
                                            let mut hdr = vec![0u8; hdr_len as usize];
                                            if cur_out
                                                .GetBlob(&MF_MT_MPEG_SEQUENCE_HEADER, &mut hdr, None)
                                                .is_ok()
                                            {
                                                self.seq_header = hdr;
                                            }
                                        }
                                        if !self.seq_header.is_empty() {
                                            bytes.extend_from_slice(&self.seq_header);
                                        }
                                    }
                                    bytes.extend_from_slice(slice);
                                    buf.Unlock()?;
                                    if !bytes.is_empty() {
                                        self.sent_first_frame = true;
                                        out.push((is_idr, bytes));
                                    }
                                    Ok(())
                                })();
                                if !self.provides_samples {
                                    self.output_sample = Some(sample);
                                }
                                extract_result?;
                            }
                            // Each METransformHaveOutput event permits one ProcessOutput call.
                            if self.async_events.is_some() {
                                return Ok(());
                            }
                        }
                        Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => {
                            if !self.provides_samples
                                && let Some(s) = sample
                            {
                                self.output_sample = Some(s);
                            }
                            return Ok(());
                        }
                        Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                            self.output_sample = None;
                            if let Ok(cur_out) = self.transform.GetOutputAvailableType(0, 0) {
                                self.transform.SetOutputType(0, Some(&cur_out), 0)?;
                                if let Ok(len) = cur_out.GetBlobSize(&MF_MT_MPEG_SEQUENCE_HEADER)
                                    && len > 0
                                {
                                    let mut buf = vec![0u8; len as usize];
                                    if cur_out.GetBlob(&MF_MT_MPEG_SEQUENCE_HEADER, &mut buf, None).is_ok() {
                                        self.seq_header = buf;
                                    }
                                }
                            }
                            if let Ok(info) = self.transform.GetOutputStreamInfo(0) {
                                self.provides_samples =
                                    (info.dwFlags & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32)) != 0;
                                self.output_size = info.cbSize.max(256 * 1024);
                            }
                        }
                        Err(e) => {
                            if !self.provides_samples
                                && let Some(s) = sample
                            {
                                self.output_sample = Some(s);
                            }
                            return Err(e);
                        }
                    }
                }
            }
        }
    }

    #[cfg(windows)]
    unsafe fn allocate_output_sample(size: u32) -> windows::core::Result<IMFSample> {
        // SAFETY: allocates a reusable IMFSample backed by a single IMFMediaBuffer.
        unsafe {
            let sample: IMFSample = MFCreateSample()?;
            let buffer: IMFMediaBuffer = MFCreateMemoryBuffer(size.max(1))?;
            sample.AddBuffer(&buffer)?;
            Ok(sample)
        }
    }

    #[cfg(windows)]
    unsafe fn make_sample(bytes: &[u8], hns: i64, dur_hns: i64) -> windows::core::Result<IMFSample> {
        // SAFETY: allocates an IMFMediaBuffer of `bytes.len()` and copies `bytes` into it.
        unsafe {
            let buf: IMFMediaBuffer = MFCreateMemoryBuffer(bytes.len() as u32)?;
            let mut dst = std::ptr::null_mut();
            buf.Lock(&mut dst, None, None)?;
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, bytes.len());
            buf.Unlock()?;
            buf.SetCurrentLength(bytes.len() as u32)?;
            let sample: IMFSample = MFCreateSample()?;
            sample.AddBuffer(&buf)?;
            sample.SetSampleTime(hns)?;
            sample.SetSampleDuration(dur_hns.max(1))?;
            Ok(sample)
        }
    }

    #[cfg(windows)]
    unsafe fn make_dxgi_sample(
        tex: &ID3D11Texture2D,
        hns: i64,
        dur_hns: i64,
    ) -> windows::core::Result<IMFSample> {
        // SAFETY: wraps `tex` in a DXGI surface buffer and attaches sample timing.
        unsafe {
            let buf: IMFMediaBuffer = MFCreateDXGISurfaceBuffer(&ID3D11Texture2D::IID, tex, 0, false)?;
            if let Ok(two_d) = buf.cast::<IMF2DBuffer>()
                && let Ok(len) = two_d.GetContiguousLength()
            {
                let _ = buf.SetCurrentLength(len);
            }
            let sample: IMFSample = MFCreateSample()?;
            sample.AddBuffer(&buf)?;
            sample.SetSampleTime(hns)?;
            sample.SetSampleDuration(dur_hns.max(1))?;
            Ok(sample)
        }
    }

    #[cfg(any(windows, target_os = "linux"))]
    fn has_h264_idr(data: &[u8]) -> bool {
        data.windows(4).any(|w| w[..3] == [0, 0, 1] && (w[3] & 0x1f) == 5)
    }

    #[cfg(any(windows, target_os = "linux"))]
    fn has_h264_sps(data: &[u8]) -> bool {
        data.windows(4).any(|w| w[..3] == [0, 0, 1] && (w[3] & 0x1f) == 7)
    }

    #[cfg(any(windows, test))]
    fn hevc_nal_types(data: &[u8]) -> impl Iterator<Item = u8> + '_ {
        data.windows(4).filter_map(|w| (w[..3] == [0, 0, 1]).then_some((w[3] >> 1) & 0x3f))
    }

    #[cfg(any(windows, test))]
    fn has_h265_irap(data: &[u8]) -> bool {
        hevc_nal_types(data).any(|kind| (16..=21).contains(&kind))
    }

    #[cfg(any(windows, test))]
    fn has_h265_parameters(data: &[u8]) -> bool {
        let mut vps = false;
        let mut sps = false;
        let mut pps = false;
        for kind in hevc_nal_types(data) {
            match kind {
                32 => vps = true,
                33 => sps = true,
                34 => pps = true,
                _ => {}
            }
        }
        vps && sps && pps
    }

    #[cfg(target_os = "linux")]
    #[repr(C)]
    struct SEncParamBase {
        i_usage_type: std::ffi::c_int, // 0 = CAMERA_VIDEO_REAL_TIME
        i_pic_width: std::ffi::c_int,
        i_pic_height: std::ffi::c_int,
        i_target_bitrate: std::ffi::c_int,
        i_rc_mode: std::ffi::c_int, // 1 = RC_BITRATE_MODE
        f_max_frame_rate: f32,
    }

    #[cfg(target_os = "linux")]
    #[repr(C)]
    struct SSourcePicture {
        i_color_format: std::ffi::c_int, // 23 = videoFormatI420
        i_stride: [std::ffi::c_int; 4],
        p_data: [*mut u8; 4],
        i_pic_width: std::ffi::c_int,
        i_pic_height: std::ffi::c_int,
        ui_time_stamp: i64,
        b_psnr_y: bool,
        b_psnr_u: bool,
        b_psnr_v: bool,
    }

    #[cfg(target_os = "linux")]
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct SLayerBSInfoV20 {
        ui_temporal_id: u8,
        ui_spatial_id: u8,
        ui_quality_id: u8,
        e_frame_type: std::ffi::c_int,
        ui_layer_type: u8,
        i_sub_seq_id: std::ffi::c_int,
        i_nal_count: std::ffi::c_int,
        p_nal_length_in_byte: *mut std::ffi::c_int,
        p_bs_buf: *mut u8,
    }

    #[cfg(target_os = "linux")]
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct SLayerBSInfo {
        ui_temporal_id: u8,
        ui_spatial_id: u8,
        ui_quality_id: u8,
        e_frame_type: std::ffi::c_int,
        ui_layer_type: u8,
        i_sub_seq_id: std::ffi::c_int,
        i_nal_count: std::ffi::c_int,
        p_nal_length_in_byte: *mut std::ffi::c_int,
        p_bs_buf: *mut u8,
        r_psnr: [f32; 3],
    }

    #[cfg(target_os = "linux")]
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct SFrameBSInfoV20 {
        i_layer_num: std::ffi::c_int,
        s_layer_info: [SLayerBSInfoV20; 128],
        e_frame_type: std::ffi::c_int, // 1 = IDR, 2 = I, 3 = P, 4 = Skip
        i_frame_size_in_bytes: std::ffi::c_int,
        ui_time_stamp: i64,
    }

    #[cfg(target_os = "linux")]
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct SFrameBSInfo {
        i_layer_num: std::ffi::c_int,
        s_layer_info: [SLayerBSInfo; 128],
        e_frame_type: std::ffi::c_int, // 1 = IDR, 2 = I, 3 = P, 4 = Skip
        i_frame_size_in_bytes: std::ffi::c_int,
        ui_time_stamp: i64,
    }

    /// Upstream OpenH264 2.3+ adds `r_psnr: [f32; 3]` to `SLayerBSInfo`, whereas
    /// Debian/Ubuntu's `libopenh264.so.7` (2.4.1) preserves the 40-byte `so.7`
    /// `SLayerBSInfoV20` layout despite reporting `u_minor == 4`. Zero-initializing
    /// this union before `EncodeFrame` lets us inspect whether `v23.e_frame_type`
    /// (at byte 7176, past the end of `SFrameBSInfoV20`) was populated.
    #[cfg(target_os = "linux")]
    #[repr(C)]
    union FrameBSInfo {
        v20: SFrameBSInfoV20,
        v23: SFrameBSInfo,
    }

    #[cfg(target_os = "linux")]
    #[repr(C)]
    struct SBitrateInfo {
        i_layer: std::ffi::c_int, // 4 = SPATIAL_LAYER_ALL, 0 = SPATIAL_LAYER_0
        i_bitrate: std::ffi::c_int,
    }

    #[cfg(target_os = "linux")]
    #[repr(C)]
    struct OpenH264Version {
        u_major: u32,
        u_minor: u32,
        u_revision: u32,
        u_reserved: u32,
    }

    #[cfg(target_os = "linux")]
    struct OpenH264EncApi {
        minor: u32,
        create_encoder: unsafe extern "C" fn(*mut *mut std::ffi::c_void) -> std::ffi::c_int,
        destroy_encoder: unsafe extern "C" fn(*mut std::ffi::c_void),
    }

    #[cfg(target_os = "linux")]
    unsafe fn dl_sym<T: Copy>(lib: *mut std::ffi::c_void, name: &std::ffi::CStr) -> Option<T> {
        let raw = unsafe { libc::dlsym(lib, name.as_ptr()) };
        if raw.is_null() { None } else { Some(unsafe { std::mem::transmute_copy(&raw) }) }
    }

    #[cfg(target_os = "linux")]
    fn openh264_enc_api() -> Option<&'static OpenH264EncApi> {
        static API: std::sync::OnceLock<Option<OpenH264EncApi>> = std::sync::OnceLock::new();
        API.get_or_init(|| unsafe {
            let candidates =
                [c"libopenh264.so.8", c"libopenh264.so.7", c"libopenh264.so.6", c"libopenh264.so"];
            let mut lib = std::ptr::null_mut();
            for name in candidates {
                lib = libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
                if !lib.is_null() {
                    break;
                }
            }
            if lib.is_null() {
                return None;
            }
            let get_version: unsafe extern "C" fn() -> OpenH264Version = dl_sym(lib, c"WelsGetCodecVersion")?;
            let ver = get_version();
            if ver.u_major != 2 {
                tracing::warn!(
                    major = ver.u_major,
                    minor = ver.u_minor,
                    "unsupported OpenH264 ABI version; skipping OpenH264 encoder"
                );
                return None;
            }
            Some(OpenH264EncApi {
                minor: ver.u_minor,
                create_encoder: dl_sym(lib, c"WelsCreateSVCEncoder")?,
                destroy_encoder: dl_sym(lib, c"WelsDestroySVCEncoder")?,
            })
        })
        .as_ref()
    }

    #[cfg(target_os = "linux")]
    pub(crate) struct OpenH264Encoder {
        encoder: *mut std::ffi::c_void,
        width: u32,
        height: u32,
        i420_buf: Vec<u8>,
        seq_header: Vec<u8>,
        sent_first_frame: bool,
    }

    #[cfg(target_os = "linux")]
    impl Drop for OpenH264Encoder {
        fn drop(&mut self) {
            if !self.encoder.is_null() {
                unsafe {
                    let vtable = *(self.encoder as *const *const usize);
                    let uninitialize: unsafe extern "C" fn(*mut std::ffi::c_void) -> std::ffi::c_int =
                        std::mem::transmute(*vtable.add(3));
                    let _ = uninitialize(self.encoder);
                    if let Some(api) = openh264_enc_api() {
                        (api.destroy_encoder)(self.encoder);
                    }
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    impl OpenH264Encoder {
        pub(crate) fn new(width: u32, height: u32, fps: u32, bitrate: u32) -> Result<Self, String> {
            let api = openh264_enc_api().ok_or_else(|| "libopenh264 encoder unavailable".to_owned())?;
            unsafe {
                let mut encoder: *mut std::ffi::c_void = std::ptr::null_mut();
                if (api.create_encoder)(&mut encoder) != 0 || encoder.is_null() {
                    return Err("WelsCreateSVCEncoder failed".to_owned());
                }
                let vtable = *(encoder as *const *const usize);
                let initialize: unsafe extern "C" fn(
                    *mut std::ffi::c_void,
                    *const SEncParamBase,
                ) -> std::ffi::c_int = std::mem::transmute(*vtable.add(0));
                let set_option: unsafe extern "C" fn(
                    *mut std::ffi::c_void,
                    std::ffi::c_int,
                    *mut std::ffi::c_void,
                ) -> std::ffi::c_int = std::mem::transmute(*vtable.add(7));

                let mut trace_level: std::ffi::c_int = 0;
                let _ = set_option(
                    encoder,
                    25, // ENCODER_OPTION_TRACE_LEVEL
                    (&mut trace_level as *mut std::ffi::c_int).cast(),
                );

                let param = SEncParamBase {
                    i_usage_type: 0, // CAMERA_VIDEO_REAL_TIME
                    i_pic_width: width as std::ffi::c_int,
                    i_pic_height: height as std::ffi::c_int,
                    i_target_bitrate: bitrate.clamp(500_000, 20_000_000) as std::ffi::c_int,
                    i_rc_mode: 1, // RC_BITRATE_MODE
                    f_max_frame_rate: fps.max(1) as f32,
                };
                let rc = initialize(encoder, &param);
                if rc != 0 {
                    (api.destroy_encoder)(encoder);
                    return Err(format!("OpenH264 Initialize failed with code {rc}"));
                }

                let mut idr_interval = (fps.max(1) * 2) as std::ffi::c_int;
                let _ = set_option(
                    encoder,
                    1, // ENCODER_OPTION_IDR_INTERVAL
                    (&mut idr_interval as *mut std::ffi::c_int).cast(),
                );

                Ok(Self {
                    encoder,
                    width,
                    height,
                    i420_buf: Vec::new(),
                    seq_header: Vec::new(),
                    sent_first_frame: false,
                })
            }
        }

        pub(crate) fn request_keyframe(&self) -> bool {
            if self.encoder.is_null() {
                return false;
            }
            unsafe {
                let vtable = *(self.encoder as *const *const usize);
                let force_intra_frame: unsafe extern "C" fn(
                    *mut std::ffi::c_void,
                    bool,
                    std::ffi::c_int,
                ) -> std::ffi::c_int = std::mem::transmute(*vtable.add(6));
                force_intra_frame(self.encoder, true, -1) == 0
            }
        }

        pub(crate) fn set_bitrate(&self, bitrate: u32) -> bool {
            if self.encoder.is_null() {
                return false;
            }
            unsafe {
                let vtable = *(self.encoder as *const *const usize);
                let set_option: unsafe extern "C" fn(
                    *mut std::ffi::c_void,
                    std::ffi::c_int,
                    *mut std::ffi::c_void,
                ) -> std::ffi::c_int = std::mem::transmute(*vtable.add(7));
                let target = bitrate.clamp(500_000, 20_000_000) as std::ffi::c_int;
                let mut info_all = SBitrateInfo {
                    i_layer: 4, // SPATIAL_LAYER_ALL
                    i_bitrate: target,
                };
                let mut info_0 = SBitrateInfo {
                    i_layer: 0, // SPATIAL_LAYER_0
                    i_bitrate: target,
                };
                let rc_all = set_option(
                    self.encoder,
                    5, // ENCODER_OPTION_BITRATE
                    (&mut info_all as *mut SBitrateInfo).cast(),
                );
                let rc_0 = set_option(
                    self.encoder,
                    5, // ENCODER_OPTION_BITRATE
                    (&mut info_0 as *mut SBitrateInfo).cast(),
                );
                rc_all == 0 || rc_0 == 0
            }
        }

        pub(crate) fn encode(
            &mut self,
            nv12: &[u8],
            hns: i64,
            _dur_hns: i64,
            stop: &AtomicBool,
        ) -> Result<Vec<(bool, Vec<u8>)>, String> {
            let w = self.width as usize;
            let h = self.height as usize;
            let y_size = w * h;
            let uv_size = y_size / 2;
            let expected = y_size + uv_size;
            if stop.load(Ordering::Relaxed) || nv12.len() < expected || self.encoder.is_null() {
                return Ok(Vec::new());
            }

            let quarter = y_size / 4;
            self.i420_buf.resize(expected, 0);
            self.i420_buf[..y_size].copy_from_slice(&nv12[..y_size]);
            {
                let (y_and_u, v_plane) = self.i420_buf.split_at_mut(y_size + quarter);
                let u_plane = &mut y_and_u[y_size..];
                let src_uv = &nv12[y_size..expected];
                for i in 0..quarter {
                    u_plane[i] = src_uv[i * 2];
                    v_plane[i] = src_uv[i * 2 + 1];
                }
            }

            unsafe {
                let api = openh264_enc_api().ok_or_else(|| "libopenh264 encoder unavailable".to_owned())?;
                let y_ptr = self.i420_buf.as_mut_ptr();
                let u_ptr = y_ptr.add(y_size);
                let v_ptr = y_ptr.add(y_size + quarter);
                let pic = SSourcePicture {
                    i_color_format: 23, // videoFormatI420
                    i_stride: [
                        w as std::ffi::c_int,
                        (w / 2) as std::ffi::c_int,
                        (w / 2) as std::ffi::c_int,
                        0,
                    ],
                    p_data: [y_ptr, u_ptr, v_ptr, std::ptr::null_mut()],
                    i_pic_width: w as std::ffi::c_int,
                    i_pic_height: h as std::ffi::c_int,
                    ui_time_stamp: hns / 10_000,
                    b_psnr_y: false,
                    b_psnr_u: false,
                    b_psnr_v: false,
                };

                let vtable = *(self.encoder as *const *const usize);
                let mut raw = Vec::new();
                let e_frame_type: std::ffi::c_int;

                let mut bs_union: FrameBSInfo = std::mem::zeroed();
                let encode_frame: unsafe extern "C" fn(
                    *mut std::ffi::c_void,
                    *const SSourcePicture,
                    *mut FrameBSInfo,
                ) -> std::ffi::c_int = std::mem::transmute(*vtable.add(4));

                let rc = encode_frame(self.encoder, &pic, &mut bs_union);
                if rc != 0 {
                    return Err(format!("OpenH264 EncodeFrame failed: {rc}"));
                }

                if api.minor >= 3 && bs_union.v23.e_frame_type != 0 {
                    let bs_info = &bs_union.v23;
                    if bs_info.e_frame_type == 4 || bs_info.i_layer_num <= 0 {
                        return Ok(Vec::new());
                    }
                    e_frame_type = bs_info.e_frame_type;
                    let layer_count = (bs_info.i_layer_num as usize).min(bs_info.s_layer_info.len());
                    for layer in bs_info.s_layer_info.iter().take(layer_count) {
                        if layer.p_bs_buf.is_null()
                            || layer.p_nal_length_in_byte.is_null()
                            || layer.i_nal_count <= 0
                        {
                            continue;
                        }
                        let nal_lens = std::slice::from_raw_parts(
                            layer.p_nal_length_in_byte,
                            layer.i_nal_count as usize,
                        );
                        let total_bytes: usize = nal_lens.iter().map(|&n| n.max(0) as usize).sum();
                        if total_bytes == 0 {
                            continue;
                        }
                        let layer_bytes = std::slice::from_raw_parts(layer.p_bs_buf, total_bytes);
                        if layer.ui_layer_type == 0 {
                            self.seq_header.clear();
                            self.seq_header.extend_from_slice(layer_bytes);
                        }
                        raw.extend_from_slice(layer_bytes);
                    }
                } else {
                    let bs_info = &bs_union.v20;
                    if bs_info.e_frame_type == 4 || bs_info.i_layer_num <= 0 {
                        return Ok(Vec::new());
                    }
                    e_frame_type = bs_info.e_frame_type;
                    let layer_count = (bs_info.i_layer_num as usize).min(bs_info.s_layer_info.len());
                    for layer in bs_info.s_layer_info.iter().take(layer_count) {
                        if layer.p_bs_buf.is_null()
                            || layer.p_nal_length_in_byte.is_null()
                            || layer.i_nal_count <= 0
                        {
                            continue;
                        }
                        let nal_lens = std::slice::from_raw_parts(
                            layer.p_nal_length_in_byte,
                            layer.i_nal_count as usize,
                        );
                        let total_bytes: usize = nal_lens.iter().map(|&n| n.max(0) as usize).sum();
                        if total_bytes == 0 {
                            continue;
                        }
                        let layer_bytes = std::slice::from_raw_parts(layer.p_bs_buf, total_bytes);
                        if layer.ui_layer_type == 0 {
                            self.seq_header.clear();
                            self.seq_header.extend_from_slice(layer_bytes);
                        }
                        raw.extend_from_slice(layer_bytes);
                    }
                }

                if raw.is_empty() {
                    return Ok(Vec::new());
                }

                let is_idr = !self.sent_first_frame || e_frame_type == 1 || has_h264_idr(&raw);
                let mut bytes = Vec::new();
                if is_idr && !has_h264_sps(&raw) && !self.seq_header.is_empty() {
                    bytes.extend_from_slice(&self.seq_header);
                }
                bytes.extend_from_slice(&raw);
                self.sent_first_frame = true;
                Ok(vec![(is_idr, bytes)])
            }
        }
    }

    #[cfg(target_os = "linux")]
    pub(crate) enum ScreenEncoder {
        OpenH264(OpenH264Encoder),
        #[cfg(linux_ffmpeg_video)]
        Hardware(crate::win::video_encoder::Encoder),
        #[cfg(linux_gst_nvenc)]
        Nvenc(win::gst_nvenc::Capture),
    }

    #[cfg(target_os = "linux")]
    impl ScreenEncoder {
        pub(crate) fn new_for_codec(
            codec: &str,
            width: u32,
            height: u32,
            fps: u32,
            bitrate: u32,
        ) -> Result<Self, String> {
            #[cfg(linux_ffmpeg_video)]
            match crate::win::video_encoder::Encoder::new(codec, width, height, fps, bitrate) {
                Ok(encoder) => return Ok(Self::Hardware(encoder)),
                Err(error) if codec != "h264" => return Err(error),
                Err(error) => tracing::debug!(%error, "Hardware H.264 unavailable; using OpenH264"),
            }
            if codec != "h264" {
                return Err(
                    "HEVC hardware encoding requires FFmpeg development libraries at build time".into()
                );
            }
            OpenH264Encoder::new(width, height, fps, bitrate).map(Self::OpenH264)
        }

        #[cfg(linux_ffmpeg_video)]
        fn next_encoder(&mut self, codec: &str, width: u32, height: u32, fps: u32, bitrate: u32) -> bool {
            if let Self::Hardware(encoder) = self {
                if encoder.next_device() {
                    return true;
                }
                if codec == "h264"
                    && let Ok(software) = OpenH264Encoder::new(width, height, fps, bitrate)
                {
                    *self = Self::OpenH264(software);
                    return true;
                }
            }
            false
        }

        pub(crate) fn request_keyframe(&mut self) -> bool {
            match self {
                Self::OpenH264(encoder) => encoder.request_keyframe(),
                #[cfg(linux_ffmpeg_video)]
                Self::Hardware(encoder) => encoder.request_keyframe(),
                #[cfg(linux_gst_nvenc)]
                Self::Nvenc(capture) => capture.command(0),
            }
        }

        pub(crate) fn set_bitrate(&mut self, bitrate: u32) -> bool {
            match self {
                Self::OpenH264(encoder) => encoder.set_bitrate(bitrate),
                #[cfg(linux_ffmpeg_video)]
                Self::Hardware(encoder) => encoder.set_bitrate(bitrate),
                #[cfg(linux_gst_nvenc)]
                Self::Nvenc(capture) => capture.command(bitrate),
            }
        }

        pub(crate) fn encode(
            &mut self,
            nv12: &[u8],
            hns: i64,
            dur_hns: i64,
            stop: &AtomicBool,
        ) -> Result<Vec<(bool, Vec<u8>)>, String> {
            match self {
                Self::OpenH264(encoder) => encoder.encode(nv12, hns, dur_hns, stop),
                #[cfg(linux_ffmpeg_video)]
                Self::Hardware(encoder) => encoder.encode(nv12, hns, stop),
                #[cfg(linux_gst_nvenc)]
                Self::Nvenc(_) => Err("NVENC worker does not accept CPU frames".into()),
            }
        }
    }

    #[cfg(windows)]
    fn new_pc_encoder(
        codec: &str,
        width: u32,
        height: u32,
        fps: u32,
        bitrate: u32,
    ) -> Result<ScreenEncoder, String> {
        ScreenEncoder::new_for_codec(codec, width, height, fps, bitrate).map_err(|error| error.to_string())
    }

    #[cfg(target_os = "linux")]
    fn new_pc_encoder(
        codec: &str,
        width: u32,
        height: u32,
        fps: u32,
        bitrate: u32,
    ) -> Result<ScreenEncoder, String> {
        ScreenEncoder::new_for_codec(codec, width, height, fps, bitrate)
    }

    #[cfg(windows)]
    fn new_pc_gpu_encoder_with_fallback(
        codec: &mut String,
        width: u32,
        height: u32,
        fps: u32,
        bitrate: u32,
        device: &ID3D11Device,
        adapter_luid: u64,
    ) -> Result<ScreenEncoder, String> {
        match ScreenEncoder::new_gpu(codec, width, height, fps, bitrate, device, adapter_luid) {
            Ok(encoder) => Ok(encoder),
            Err(error) if codec == "h265" => {
                tracing::info!(%error, "HEVC GPU screen encoder unavailable; trying H.264 GPU encoder");
                match ScreenEncoder::new_gpu("h264", width, height, fps, bitrate, device, adapter_luid) {
                    Ok(encoder) => {
                        *codec = "h264".into();
                        Ok(encoder)
                    }
                    Err(h264_error) => Err(h264_error.to_string()),
                }
            }
            Err(error) => Err(error.to_string()),
        }
    }

    #[cfg(windows)]
    fn convert_to_gpu_nv12<'a>(
        gpu_nv12: &'a mut Option<GpuBgraToNv12>,
        device: &ID3D11Device,
        src_tex: &ID3D11Texture2D,
        dst_w: u32,
        dst_h: u32,
    ) -> windows::core::Result<&'a ID3D11Texture2D> {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: valid D3D11 texture interface.
        unsafe { src_tex.GetDesc(&mut desc) };
        let src_size = (desc.Width, desc.Height);
        let dst_size = (dst_w, dst_h);
        if !gpu_nv12.as_ref().is_some_and(|conv| conv.matches(device, src_size, dst_size)) {
            *gpu_nv12 = Some(GpuBgraToNv12::new(device, src_size, dst_size)?);
        }
        gpu_nv12.as_mut().ok_or_else(|| windows::core::Error::from(E_FAIL))?.convert(src_tex, None)
    }

    #[cfg(any(windows, target_os = "linux"))]
    fn new_pc_encoder_with_fallback(
        codec: &mut String,
        width: u32,
        height: u32,
        fps: u32,
        bitrate: u32,
    ) -> Result<ScreenEncoder, String> {
        match new_pc_encoder(codec, width, height, fps, bitrate) {
            Ok(encoder) => Ok(encoder),
            Err(error) if codec == "h265" => {
                tracing::info!(%error, "HEVC screen encoder unavailable; using H.264");
                *codec = "h264".into();
                new_pc_encoder(codec, width, height, fps, bitrate)
            }
            Err(error) => Err(error),
        }
    }

    #[cfg(windows)]
    fn primary_screen_dimensions() -> (u32, u32) {
        // SAFETY: Win32 GDI device context and system metrics calls with balanced ReleaseDC.
        unsafe {
            let hdc = GetDC(None);
            let pw = if !hdc.is_invalid() { GetDeviceCaps(Some(hdc), DESKTOPHORZRES) } else { 0 };
            let ph = if !hdc.is_invalid() { GetDeviceCaps(Some(hdc), DESKTOPVERTRES) } else { 0 };
            if !hdc.is_invalid() {
                let _ = ReleaseDC(None, hdc);
            }
            (
                if pw > 0 { pw as u32 } else { GetSystemMetrics(SM_CXSCREEN).max(640) as u32 },
                if ph > 0 { ph as u32 } else { GetSystemMetrics(SM_CYSCREEN).max(480) as u32 },
            )
        }
    }

    /// Captures the primary desktop monitor into `bgra` scaled to `(dst_w, dst_h)`,
    /// with the pointer, and returns the current physical screen dimensions
    /// `(src_w, src_h)`. With `region` (physical screen pixels), captures only
    /// that, without the pointer (Play on phone), and returns its size.
    #[cfg(windows)]
    fn capture_screen(
        region: Option<RECT>,
        dst_w: u32,
        dst_h: u32,
        bgra: &mut Vec<u8>,
    ) -> Option<(u32, u32)> {
        capture_rect(region.map(|r| (r, false)), dst_w, dst_h, bgra)
    }

    /// What the phone views: the screen at `area` (another screen than the
    /// main one, #442) or else the main one, with the pointer.
    #[cfg(windows)]
    fn capture_shown(area: Option<RECT>, dst_w: u32, dst_h: u32, bgra: &mut Vec<u8>) -> bool {
        capture_rect(area.map(|a| (a, true)), dst_w, dst_h, bgra).is_some()
    }

    #[cfg(target_os = "linux")]
    fn capture_shown(_area: Option<()>, dst_w: u32, dst_h: u32, bgra: &mut Vec<u8>) -> bool {
        capture_screen(None, dst_w, dst_h, bgra).is_some()
    }

    /// [`capture_screen`], with `region`'s flag saying whether to draw the
    /// pointer there (a whole screen) or not (a player's window).
    #[cfg(windows)]
    fn capture_rect(
        region: Option<(RECT, bool)>,
        dst_w: u32,
        dst_h: u32,
        bgra: &mut Vec<u8>,
    ) -> Option<(u32, u32)> {
        let pointer = region.is_none_or(|(_, pointer)| pointer);
        let region = region.map(|(r, _)| r);
        // SAFETY: Win32 GDI device context and bitmap calls with balanced cleanup.
        unsafe {
            let hdc_screen = GetDC(None);
            if hdc_screen.is_invalid() {
                return None;
            }
            let (src_x, src_y, src_w, src_h) = match region {
                Some(r) => {
                    (r.left, r.top, (r.right - r.left).max(1) as u32, (r.bottom - r.top).max(1) as u32)
                }
                None => {
                    let phys_w = GetDeviceCaps(Some(hdc_screen), DESKTOPHORZRES);
                    let phys_h = GetDeviceCaps(Some(hdc_screen), DESKTOPVERTRES);
                    let w = if phys_w > 0 { phys_w } else { GetSystemMetrics(SM_CXSCREEN) }.max(1) as u32;
                    let h = if phys_h > 0 { phys_h } else { GetSystemMetrics(SM_CYSCREEN) }.max(1) as u32;
                    (0, 0, w, h)
                }
            };

            let hdc_mem = CreateCompatibleDC(Some(hdc_screen));
            if hdc_mem.is_invalid() {
                let _ = ReleaseDC(None, hdc_screen);
                return None;
            }
            let hbm = CreateCompatibleBitmap(hdc_screen, dst_w as i32, dst_h as i32);
            if hbm.is_invalid() {
                let _ = DeleteDC(hdc_mem);
                let _ = ReleaseDC(None, hdc_screen);
                return None;
            }
            let old = SelectObject(hdc_mem, hbm.into());
            let _ = SetStretchBltMode(hdc_mem, HALFTONE);
            let ok = StretchBlt(
                hdc_mem,
                0,
                0,
                dst_w as i32,
                dst_h as i32,
                Some(hdc_screen),
                src_x,
                src_y,
                src_w as i32,
                src_h as i32,
                SRCCOPY,
            )
            .as_bool();

            // Draw the mouse cursor at its scaled position so the phone user sees where they point.
            let mut ci = CURSORINFO {
                cbSize: size_of::<CURSORINFO>() as u32,
                flags: Default::default(),
                hCursor: Default::default(),
                ptScreenPos: POINT::default(),
            };
            if pointer && GetCursorInfo(&mut ci).is_ok() && (ci.flags.0 & CURSOR_SHOWING.0) != 0 {
                let cx = (((ci.ptScreenPos.x - src_x) as i64) * (dst_w as i64) / (src_w as i64)) as i32;
                let cy = (((ci.ptScreenPos.y - src_y) as i64) * (dst_h as i64) / (src_h as i64)) as i32;
                let _ = DrawIconEx(hdc_mem, cx, cy, ci.hCursor.into(), 0, 0, 0, None, DI_NORMAL);
            }

            let needed = (dst_w as usize) * (dst_h as usize) * 4;
            bgra.resize(needed, 0);

            let mut bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: dst_w as i32,
                    biHeight: -(dst_h as i32), // Top-down DIB.
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    biSizeImage: needed as u32,
                    ..Default::default()
                },
                ..Default::default()
            };

            let lines = GetDIBits(
                hdc_mem,
                hbm,
                0,
                dst_h,
                Some(bgra.as_mut_ptr().cast()),
                &mut bmi,
                windows::Win32::Graphics::Gdi::DIB_RGB_COLORS,
            );

            let _ = SelectObject(hdc_mem, old);
            let _ = DeleteObject(hbm.into());
            let _ = DeleteDC(hdc_mem);
            let _ = ReleaseDC(None, hdc_screen);

            if ok && lines > 0 { Some((src_w, src_h)) } else { None }
        }
    }

    #[cfg(target_os = "linux")]
    #[repr(C)]
    struct XWindowAttributes {
        x: std::ffi::c_int,
        y: std::ffi::c_int,
        width: std::ffi::c_int,
        height: std::ffi::c_int,
        border_width: std::ffi::c_int,
        depth: std::ffi::c_int,
        visual: *mut std::ffi::c_void,
        root: std::ffi::c_ulong,
        class: std::ffi::c_int,
        bit_gravity: std::ffi::c_int,
        win_gravity: std::ffi::c_int,
        backing_store: std::ffi::c_int,
        backing_planes: std::ffi::c_ulong,
        backing_pixel: std::ffi::c_ulong,
        save_under: std::ffi::c_int,
        colormap: std::ffi::c_ulong,
        map_installed: std::ffi::c_int,
        map_state: std::ffi::c_int,
        all_event_masks: std::ffi::c_long,
        your_event_mask: std::ffi::c_long,
        do_not_propagate_mask: std::ffi::c_long,
        override_redirect: std::ffi::c_int,
        screen: *mut std::ffi::c_void,
    }

    #[cfg(target_os = "linux")]
    #[repr(C)]
    struct XImageFuncs {
        create_image: *mut std::ffi::c_void,
        destroy_image: Option<unsafe extern "C" fn(*mut XImage) -> std::ffi::c_int>,
        get_pixel: *mut std::ffi::c_void,
        put_pixel: *mut std::ffi::c_void,
        sub_image: *mut std::ffi::c_void,
        add_pixel: *mut std::ffi::c_void,
    }

    #[cfg(target_os = "linux")]
    #[repr(C)]
    struct XImage {
        width: std::ffi::c_int,
        height: std::ffi::c_int,
        xoffset: std::ffi::c_int,
        format: std::ffi::c_int,
        data: *mut u8,
        byte_order: std::ffi::c_int,
        bitmap_unit: std::ffi::c_int,
        bitmap_bit_order: std::ffi::c_int,
        bitmap_pad: std::ffi::c_int,
        depth: std::ffi::c_int,
        bytes_per_line: std::ffi::c_int,
        bits_per_pixel: std::ffi::c_int,
        red_mask: std::ffi::c_ulong,
        green_mask: std::ffi::c_ulong,
        blue_mask: std::ffi::c_ulong,
        obdata: *mut std::ffi::c_void,
        f: XImageFuncs,
    }

    #[cfg(target_os = "linux")]
    struct X11Api {
        open_display: unsafe extern "C" fn(*const std::ffi::c_char) -> *mut std::ffi::c_void,
        default_root_window: unsafe extern "C" fn(*mut std::ffi::c_void) -> std::ffi::c_ulong,
        get_window_attributes: unsafe extern "C" fn(
            *mut std::ffi::c_void,
            std::ffi::c_ulong,
            *mut XWindowAttributes,
        ) -> std::ffi::c_int,
        get_image: unsafe extern "C" fn(
            *mut std::ffi::c_void,
            std::ffi::c_ulong,
            std::ffi::c_int,
            std::ffi::c_int,
            std::ffi::c_uint,
            std::ffi::c_uint,
            std::ffi::c_ulong,
            std::ffi::c_int,
        ) -> *mut XImage,
        query_pointer: unsafe extern "C" fn(
            *mut std::ffi::c_void,
            std::ffi::c_ulong,
            *mut std::ffi::c_ulong,
            *mut std::ffi::c_ulong,
            *mut std::ffi::c_int,
            *mut std::ffi::c_int,
            *mut std::ffi::c_int,
            *mut std::ffi::c_int,
            *mut std::ffi::c_uint,
        ) -> std::ffi::c_int,
        close_display: unsafe extern "C" fn(*mut std::ffi::c_void) -> std::ffi::c_int,
        free: unsafe extern "C" fn(*mut std::ffi::c_void) -> std::ffi::c_int,
    }

    /// Xlib's own error handler ends the process. A failed capture call
    /// (BadMatch from XGetImage, #548) only fails that frame instead.
    #[cfg(target_os = "linux")]
    unsafe extern "C" fn x11_capture_error(
        _display: *mut std::ffi::c_void,
        event: *mut XErrorEvent,
    ) -> std::ffi::c_int {
        // SAFETY: Xlib passes a valid event for the call.
        let (code, request) = unsafe { ((*event).error_code, (*event).request_code) };
        tracing::warn!(code, request, "X11 error during screen capture");
        0
    }

    #[cfg(target_os = "linux")]
    #[repr(C)]
    struct XErrorEvent {
        kind: std::ffi::c_int,
        display: *mut std::ffi::c_void,
        resourceid: std::ffi::c_ulong,
        serial: std::ffi::c_ulong,
        error_code: u8,
        request_code: u8,
        minor_code: u8,
    }

    #[cfg(target_os = "linux")]
    type XErrorHandler =
        Option<unsafe extern "C" fn(*mut std::ffi::c_void, *mut XErrorEvent) -> std::ffi::c_int>;

    #[cfg(target_os = "linux")]
    fn x11_api() -> Option<&'static X11Api> {
        static API: std::sync::OnceLock<Option<X11Api>> = std::sync::OnceLock::new();
        API.get_or_init(|| unsafe {
            let candidates = [c"libX11.so.6", c"libX11.so"];
            let mut lib = std::ptr::null_mut();
            for name in candidates {
                lib = libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
                if !lib.is_null() {
                    break;
                }
            }
            if lib.is_null() {
                return None;
            }
            let set_error_handler: unsafe extern "C" fn(XErrorHandler) -> XErrorHandler =
                dl_sym(lib, c"XSetErrorHandler")?;
            set_error_handler(Some(x11_capture_error));
            Some(X11Api {
                open_display: dl_sym(lib, c"XOpenDisplay")?,
                default_root_window: dl_sym(lib, c"XDefaultRootWindow")?,
                get_window_attributes: dl_sym(lib, c"XGetWindowAttributes")?,
                get_image: dl_sym(lib, c"XGetImage")?,
                query_pointer: dl_sym(lib, c"XQueryPointer")?,
                close_display: dl_sym(lib, c"XCloseDisplay")?,
                free: dl_sym(lib, c"XFree")?,
            })
        })
        .as_ref()
    }

    #[cfg(target_os = "linux")]
    struct ThreadX11Display(*mut std::ffi::c_void);

    #[cfg(target_os = "linux")]
    impl Drop for ThreadX11Display {
        fn drop(&mut self) {
            if !self.0.is_null()
                && let Some(api) = x11_api()
            {
                unsafe {
                    let _ = (api.close_display)(self.0);
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn with_x11_display<R>(f: impl FnOnce(&X11Api, *mut std::ffi::c_void) -> Option<R>) -> Option<R> {
        std::thread_local! {
            static DISPLAY: std::cell::RefCell<Option<ThreadX11Display>> = const { std::cell::RefCell::new(None) };
        }
        let api = x11_api()?;
        DISPLAY.with(|cell| {
            let mut borrow = cell.borrow_mut();
            if borrow.is_none() {
                let dpy = unsafe { (api.open_display)(std::ptr::null()) };
                if !dpy.is_null() {
                    *borrow = Some(ThreadX11Display(dpy));
                }
            }
            let dpy = borrow.as_ref()?.0;
            f(api, dpy)
        })
    }

    #[cfg(target_os = "linux")]
    fn primary_screen_dimensions() -> (u32, u32) {
        with_x11_display(|api, dpy| unsafe {
            let root = (api.default_root_window)(dpy);
            let mut attrs: XWindowAttributes = std::mem::zeroed();
            if (api.get_window_attributes)(dpy, root, &mut attrs) != 0 && attrs.width > 0 && attrs.height > 0
            {
                Some((attrs.width as u32, attrs.height as u32))
            } else {
                None
            }
        })
        .unwrap_or((1920, 1080))
    }

    #[cfg(target_os = "linux")]
    fn draw_cursor_pointer(bgra: &mut [u8], dst_w: u32, dst_h: u32, cx: i32, cy: i32) {
        // 12x16 arrow cursor bitmap (1 = black border, 2 = white fill).
        const CURSOR: [[u8; 10]; 14] = [
            [1, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            [1, 1, 0, 0, 0, 0, 0, 0, 0, 0],
            [1, 2, 1, 0, 0, 0, 0, 0, 0, 0],
            [1, 2, 2, 1, 0, 0, 0, 0, 0, 0],
            [1, 2, 2, 2, 1, 0, 0, 0, 0, 0],
            [1, 2, 2, 2, 2, 1, 0, 0, 0, 0],
            [1, 2, 2, 2, 2, 2, 1, 0, 0, 0],
            [1, 2, 2, 2, 2, 2, 2, 1, 0, 0],
            [1, 2, 2, 2, 2, 2, 2, 2, 1, 0],
            [1, 2, 2, 2, 2, 1, 1, 1, 1, 1],
            [1, 2, 2, 1, 2, 1, 0, 0, 0, 0],
            [1, 2, 1, 0, 1, 2, 1, 0, 0, 0],
            [1, 1, 0, 0, 1, 2, 1, 0, 0, 0],
            [0, 0, 0, 0, 0, 1, 1, 0, 0, 0],
        ];
        for (dy, row) in CURSOR.iter().enumerate() {
            let py = cy + dy as i32;
            if py < 0 || py >= dst_h as i32 {
                continue;
            }
            for (dx, &cell) in row.iter().enumerate() {
                if cell == 0 {
                    continue;
                }
                let px = cx + dx as i32;
                if px < 0 || px >= dst_w as i32 {
                    continue;
                }
                let idx = ((py as usize) * (dst_w as usize) + (px as usize)) * 4;
                let val = if cell == 2 { 0xFF } else { 0x00 };
                bgra[idx] = val;
                bgra[idx + 1] = val;
                bgra[idx + 2] = val;
                bgra[idx + 3] = 0xFF;
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn has_executable(name: &str) -> bool {
        let Some(path_var) = std::env::var_os("PATH") else {
            return false;
        };
        std::env::split_paths(&path_var).any(|dir| dir.join(name).is_file())
    }

    /// Whether gst-launch and its PipeWire source are installed (gstreamer1.0-tools
    /// and gstreamer1.0-pipewire on Debian and Ubuntu), which Wayland capture uses.
    #[cfg(target_os = "linux")]
    pub(crate) fn pipewire_capture_installed() -> bool {
        has_executable("gst-launch-1.0")
            && win::run_cmd_timeout("gst-inspect-1.0", &["--exists", "pipewiresrc"], Duration::from_secs(2))
                .is_some_and(|out| out.status.success())
    }

    #[cfg(target_os = "linux")]
    fn wayland_session() -> bool {
        std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty())
    }

    #[cfg(target_os = "linux")]
    type LatestFrame = Arc<Mutex<Option<WaylandFrame>>>;

    #[cfg(target_os = "linux")]
    struct WaylandFrame {
        received: Instant,
        pixels: Vec<u8>,
        black_since: Option<Instant>,
    }

    #[cfg(target_os = "linux")]
    const WAYLAND_BLACK_TIMEOUT: Duration = Duration::from_secs(3);

    #[cfg(target_os = "linux")]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum WaylandCaptureMode {
        PipeWire,
        OpenGl,
        RetryPipeWire,
        LastOpenGl,
    }

    #[cfg(target_os = "linux")]
    impl WaylandCaptureMode {
        fn uses_gl(self) -> bool {
            matches!(self, Self::OpenGl | Self::LastOpenGl)
        }

        fn fallback(self) -> Option<Self> {
            match self {
                Self::PipeWire => Some(Self::OpenGl),
                Self::OpenGl => Some(Self::RetryPipeWire),
                Self::RetryPipeWire => Some(Self::LastOpenGl),
                Self::LastOpenGl => None,
            }
        }
    }

    /// A black desktop can be legitimate. Use this only to trigger one retry,
    /// allowing a few non-black samples for the cursor embedded by the portal.
    #[cfg(target_os = "linux")]
    fn wayland_frame_is_nearly_black(pixels: &[u8]) -> bool {
        const SAMPLE_STRIDE: usize = 257;
        let samples = pixels.as_chunks::<4>().0.iter().step_by(SAMPLE_STRIDE);
        let allowed = samples.len() / 100;
        samples.filter(|pixel| pixel[..3].iter().any(|&value| value > 4)).nth(allowed).is_none()
    }

    #[cfg(target_os = "linux")]
    struct WaylandScreenCaster {
        portal: win::screen_portal::ScreenPortal,
        started: Instant,
        child: std::process::Child,
        latest_frame: LatestFrame,
        /// The frame size the pipeline scales to.
        size: (u32, u32),
        mode: WaylandCaptureMode,
    }

    #[cfg(target_os = "linux")]
    impl Drop for WaylandScreenCaster {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    #[cfg(target_os = "linux")]
    impl WaylandScreenCaster {
        /// The fd number the portal's PipeWire connection gets in the capture process.
        const REMOTE_FD: i32 = 3;

        /// Starts gst-launch reading the portal's stream and writing
        /// raw BGRx frames of `dst_w` x `dst_h` to its stdout.
        fn spawn_pipeline(
            runtime: &tokio::runtime::Runtime,
            portal: &win::screen_portal::ScreenPortal,
            (dst_w, dst_h): (u32, u32),
            mode: WaylandCaptureMode,
        ) -> anyhow::Result<(std::process::Child, LatestFrame)> {
            use std::io::{BufRead, BufReader, Read};
            use std::os::fd::AsRawFd;
            use std::os::unix::process::CommandExt;
            use std::process::{Command, Stdio};

            let remote = runtime.block_on(portal.open_remote())?;
            let raw = remote.as_raw_fd();
            let node_id = portal.node_id;
            let mut cmd = Command::new("gst-launch-1.0");
            cmd.args(pipeline_args(node_id, dst_w, dst_h, mode))
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            // SAFETY: runs in the child between fork and exec and only makes
            // async-signal-safe system calls.
            unsafe {
                cmd.pre_exec(move || {
                    let _ = libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                    if raw == Self::REMOTE_FD {
                        // Already in place: only let it survive exec.
                        if libc::fcntl(raw, libc::F_SETFD, 0) < 0 {
                            return Err(std::io::Error::last_os_error());
                        }
                    } else if libc::dup2(raw, Self::REMOTE_FD) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            let mut child = cmd.spawn()?;
            drop(remote);
            let mut stdout = child.stdout.take().expect("GStreamer stdout is piped");
            let stderr = child.stderr.take().expect("GStreamer stderr is piped");
            let latest_frame = Arc::new(Mutex::new(None));
            let frame_slot = Arc::clone(&latest_frame);
            let frame_bytes = (dst_w as usize) * (dst_h as usize) * 4;
            let started = Instant::now();
            let diagnostics =
                std::thread::Builder::new().name("wayland-screencast-log".into()).spawn(move || {
                    for (index, line) in BufReader::new(stderr).lines().map_while(Result::ok).enumerate() {
                        if index < 20 {
                            tracing::warn!(node_id, ?mode, detail = %line, "GStreamer screen capture");
                        }
                    }
                });
            let readers = diagnostics.and_then(|_| {
                std::thread::Builder::new().name("wayland-screencast-reader".into()).spawn(move || {
                    let mut buf = vec![0u8; frame_bytes];
                    let mut first_frame = true;
                    let mut black_since = None;
                    while stdout.read_exact(&mut buf).is_ok() {
                        let received = Instant::now();
                        if first_frame {
                            first_frame = false;
                            tracing::info!(
                                node_id,
                                ?mode,
                                elapsed_ms = started.elapsed().as_millis() as u64,
                                "received the first Wayland screen capture frame"
                            );
                        }
                        if mode == WaylandCaptureMode::OpenGl && wayland_frame_is_nearly_black(&buf) {
                            black_since.get_or_insert(received);
                        } else {
                            black_since = None;
                        }
                        for chunk in buf.as_chunks_mut::<4>().0 {
                            chunk[3] = 0xFF;
                        }
                        *frame_slot.lock().unwrap_or_else(|e| e.into_inner()) =
                            Some(WaylandFrame { received, pixels: buf.clone(), black_since });
                    }
                })
            });
            if let Err(error) = readers {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.into());
            }

            Ok((child, latest_frame))
        }

        fn start(
            runtime: &tokio::runtime::Runtime,
            portal: win::screen_portal::ScreenPortal,
            size: (u32, u32),
        ) -> anyhow::Result<Self> {
            let mode = WaylandCaptureMode::PipeWire;
            let (child, latest_frame) = Self::spawn_pipeline(runtime, &portal, size, mode)?;
            Ok(Self { portal, started: Instant::now(), child, latest_frame, size, mode })
        }

        fn recover(&mut self, runtime: &tokio::runtime::Runtime, reason: &str) -> anyhow::Result<bool> {
            let Some(next) = self.mode.fallback().filter(|next| !next.uses_gl() || gl_elements_installed())
            else {
                anyhow::bail!("{reason}");
            };
            tracing::warn!(node_id = self.portal.node_id, from = ?self.mode, to = ?next, reason,
                "switching Wayland screen capture pipeline");
            let _ = self.child.kill();
            let _ = self.child.wait();
            let (child, latest_frame) = Self::spawn_pipeline(runtime, &self.portal, self.size, next)?;
            self.child = child;
            self.latest_frame = latest_frame;
            self.started = Instant::now();
            self.mode = next;
            Ok(false)
        }

        fn capture(&mut self, runtime: &tokio::runtime::Runtime, bgra: &mut Vec<u8>) -> anyhow::Result<bool> {
            const FRAME_TIMEOUT: Duration = Duration::from_secs(8);
            const TRY_GL_AFTER: Duration = Duration::from_secs(3);
            if self.portal.is_closed() {
                anyhow::bail!("the Wayland portal closed the screen sharing session");
            }
            if let Some(status) = self.child.try_wait()? {
                return self.recover(runtime, &format!("GStreamer capture exited ({status})"));
            }
            let latest = self.latest_frame.lock().unwrap_or_else(|e| e.into_inner());
            let reason = match latest.as_ref() {
                Some(frame) if frame.received.elapsed() >= FRAME_TIMEOUT => {
                    "capture stopped supplying frames"
                }
                Some(frame)
                    if frame.black_since.is_some_and(|at| frame.received - at >= WAYLAND_BLACK_TIMEOUT) =>
                {
                    "OpenGL capture has been nearly black for 3 seconds"
                }
                Some(frame) => {
                    bgra.clear();
                    bgra.extend_from_slice(&frame.pixels);
                    return Ok(true);
                }
                None if self.started.elapsed() >= FRAME_TIMEOUT => "capture supplied no frames",
                None if !self.mode.uses_gl() && self.started.elapsed() >= TRY_GL_AFTER => {
                    drop(latest);
                    if !gl_elements_installed()
                        || self.latest_frame.lock().unwrap_or_else(|e| e.into_inner()).is_some()
                    {
                        return Ok(false);
                    }
                    return self.recover(runtime, "PipeWire capture supplied no frame in 3 seconds");
                }
                None => return Ok(false),
            };
            drop(latest);
            self.recover(runtime, reason)
        }
    }

    /// Whether GStreamer's OpenGL elements are installed (gst-plugins-base's gl).
    #[cfg(target_os = "linux")]
    fn gl_elements_installed() -> bool {
        static HAS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *HAS.get_or_init(|| {
            ["glupload", "glcolorconvert", "gldownload"].iter().all(|element| {
                win::run_cmd_timeout("gst-inspect-1.0", &["--exists", element], Duration::from_secs(2))
                    .is_some_and(|out| out.status.success())
            })
        })
    }

    /// gst-launch arguments: read stream `node_id` through the portal's
    /// PipeWire connection (fd 3) and write raw BGRx frames to stdout.
    #[cfg(target_os = "linux")]
    fn pipeline_args(node_id: u32, dst_w: u32, dst_h: u32, mode: WaylandCaptureMode) -> Vec<String> {
        static HAS_ON_DISCONNECT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let mut args = vec![
            "-q".to_owned(),
            "pipewiresrc".to_owned(),
            format!("fd={}", WaylandScreenCaster::REMOTE_FD),
            format!("path={node_id}"),
            "do-timestamp=true".to_owned(),
            // Static desktops can go without damage events.
            "keepalive-time=1000".to_owned(),
        ];
        if *HAS_ON_DISCONNECT.get_or_init(|| {
            win::run_cmd_timeout("gst-inspect-1.0", &["pipewiresrc"], Duration::from_secs(2)).is_some_and(
                |out| out.status.success() && String::from_utf8_lossy(&out.stdout).contains("on-disconnect "),
            )
        }) {
            args.push("on-disconnect=error".to_owned());
        }
        if mode.uses_gl() {
            args.extend(["!", "glupload", "!", "glcolorconvert"].map(str::to_owned));
            if mode == WaylandCaptureMode::OpenGl {
                // Shrink the texture before reading it back to system memory.
                // LastOpenGl keeps the original path if this fails.
                args.extend(
                    ["!", "video/x-raw(memory:GLMemory),format=RGBA", "!", "glcolorscale"].map(str::to_owned),
                );
                args.push("!".to_owned());
                args.push(format!("video/x-raw(memory:GLMemory),format=RGBA,width={dst_w},height={dst_h}"));
            }
            args.extend(["!", "gldownload", "!", "video/x-raw,format=RGBA"].map(str::to_owned));
        } else {
            // Request system memory from PipeWire. videoconvert also accepts
            // DMA-BUF caps, which would let the two paths select the same buffers.
            args.extend(["!", "video/x-raw"].map(str::to_owned));
        }
        args.extend([
            "!".to_owned(),
            "videoconvert".to_owned(),
            "!".to_owned(),
            "videoscale".to_owned(),
            "!".to_owned(),
            format!("video/x-raw,format=BGRx,width={dst_w},height={dst_h}"),
            "!".to_owned(),
            "fdsink".to_owned(),
            "fd=1".to_owned(),
        ]);
        args
    }

    #[cfg(target_os = "linux")]
    fn capture_screen(
        region: Option<crate::win::browser::RECT>,
        dst_w: u32,
        dst_h: u32,
        bgra: &mut Vec<u8>,
    ) -> Option<(u32, u32)> {
        with_x11_display(|api, dpy| unsafe {
            let root = (api.default_root_window)(dpy);
            let mut attrs: XWindowAttributes = std::mem::zeroed();
            if (api.get_window_attributes)(dpy, root, &mut attrs) == 0
                || attrs.width <= 0
                || attrs.height <= 0
            {
                return None;
            }
            let full_w = attrs.width;
            let full_h = attrs.height;
            let (src_x, src_y, src_w, src_h) = match region {
                Some(r) => {
                    let x0 = r.left.clamp(0, full_w - 1);
                    let y0 = r.top.clamp(0, full_h - 1);
                    let x1 = r.right.clamp(x0 + 1, full_w);
                    let y1 = r.bottom.clamp(y0 + 1, full_h);
                    (x0, y0, (x1 - x0) as u32, (y1 - y0) as u32)
                }
                None => (0, 0, full_w as u32, full_h as u32),
            };

            let img = (api.get_image)(
                dpy,
                root,
                src_x,
                src_y,
                src_w,
                src_h,
                !0 as std::ffi::c_ulong, // AllPlanes
                2,                       // ZPixmap
            );
            if img.is_null() {
                return None;
            }
            if (*img).data.is_null() {
                if let Some(destroy) = (*img).f.destroy_image {
                    let _ = destroy(img);
                } else {
                    let _ = (api.free)(img.cast());
                }
                return None;
            }

            let bytes_per_line = (*img).bytes_per_line.max(0) as usize;
            let bpp = ((*img).bits_per_pixel.max(24) as usize) / 8;
            let needed = (dst_w as usize) * (dst_h as usize) * 4;
            bgra.resize(needed, 0);

            for y in 0..dst_h {
                let sy =
                    ((u64::from(y) * u64::from(src_h)) / u64::from(dst_h)).min(u64::from(src_h - 1)) as usize;
                let src_row = (*img).data.add(sy * bytes_per_line);
                let dst_row_offset = (y as usize) * (dst_w as usize) * 4;
                for x in 0..dst_w {
                    let sx = ((u64::from(x) * u64::from(src_w)) / u64::from(dst_w)).min(u64::from(src_w - 1))
                        as usize;
                    let px = src_row.add(sx * bpp);
                    let dst_idx = dst_row_offset + (x as usize) * 4;
                    bgra[dst_idx] = *px;
                    bgra[dst_idx + 1] = *px.add(1);
                    bgra[dst_idx + 2] = *px.add(2);
                    bgra[dst_idx + 3] = 0xFF;
                }
            }

            let mut root_ret: std::ffi::c_ulong = 0;
            let mut child_ret: std::ffi::c_ulong = 0;
            let mut root_x: std::ffi::c_int = 0;
            let mut root_y: std::ffi::c_int = 0;
            let mut win_x: std::ffi::c_int = 0;
            let mut win_y: std::ffi::c_int = 0;
            let mut mask_ret: std::ffi::c_uint = 0;
            if region.is_none()
                && (api.query_pointer)(
                    dpy,
                    root,
                    &mut root_ret,
                    &mut child_ret,
                    &mut root_x,
                    &mut root_y,
                    &mut win_x,
                    &mut win_y,
                    &mut mask_ret,
                ) != 0
                && root_x >= 0
                && root_y >= 0
            {
                let cx = ((root_x as i64) * (dst_w as i64) / (src_w as i64)) as i32;
                let cy = ((root_y as i64) * (dst_h as i64) / (src_h as i64)) as i32;
                draw_cursor_pointer(bgra, dst_w, dst_h, cx, cy);
            }

            if let Some(destroy) = (*img).f.destroy_image {
                let _ = destroy(img);
            } else {
                libc::free((*img).data.cast());
                let _ = (api.free)(img.cast());
            }

            Some((src_w, src_h))
        })
    }

    #[cfg(any(windows, target_os = "linux"))]
    fn run_capture_loop(
        peer: DeviceId,
        options: MirrorStart,
        stop: Arc<AtomicBool>,
        force_keyframe: Arc<AtomicBool>,
    ) {
        let Some(node) = core_host::node() else {
            cleanup_session(&stop);
            return;
        };
        let runtime = &core_host::host().runtime;
        let fps = options.fps.clamp(10, 60);
        let max_bitrate = options.bitrate.clamp(500_000, 20_000_000);
        let mut codec = if node.mirror_codecs(peer).is_ok_and(|codecs| codecs.iter().any(|c| c == "h265")) {
            "h265".to_owned()
        } else {
            "h264".to_owned()
        };
        #[cfg(linux_gst_nvenc)]
        let (mut nvenc_portal, mut nvenc) = (None, None);
        // Dropping the Wayland portal's zbus signal stream schedules a Tokio task.
        // Keep the runtime entered through capture and teardown on this thread.
        #[cfg(target_os = "linux")]
        let _runtime_guard = runtime.enter();
        #[cfg(target_os = "linux")]
        let (mut wayland, wayland_dims) = if options.watch.is_none() && wayland_session() {
            if !pipewire_capture_installed() {
                let error = anyhow::anyhow!(
                    "sharing the screen on Wayland needs GStreamer's PipeWire plugin (gst-launch-1.0 and pipewiresrc)"
                );
                finish_failed_capture(peer, &stop, &error);
                return;
            }
            let portal = runtime.block_on(win::screen_portal::ScreenPortal::open(
                remote_input_allowed(peer),
                &core_host::host().data_dir,
                || stop.load(Ordering::Acquire),
            ));
            match portal.and_then(|portal| {
                let dims = target_dimensions(portal.size.0, portal.size.1, options.max_size);
                if portal.input {
                    let mut active = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(active) = active.as_mut().filter(|s| Arc::ptr_eq(&s.stop, &stop)) {
                        active.input_portal = Some(Arc::new(portal.input_portal()));
                    }
                }
                #[cfg(linux_gst_nvenc)]
                if codec == "h265" && win::gst_nvenc::enabled() {
                    let worker = runtime.block_on(portal.open_remote()).and_then(|remote| {
                        Ok(win::gst_nvenc::Capture::start(remote, portal.node_id, dims, fps, max_bitrate)?)
                    });
                    match worker {
                        Ok(worker) => {
                            nvenc = Some(ScreenEncoder::Nvenc(worker));
                            nvenc_portal = Some(portal);
                            return Ok((None, Some(dims)));
                        }
                        Err(error) => {
                            tracing::info!(%error, "NVENC worker unavailable; using raw Wayland capture")
                        }
                    }
                }
                WaylandScreenCaster::start(runtime, portal, dims).map(|caster| (Some(caster), Some(dims)))
            }) {
                Ok(result) => result,
                Err(error) => {
                    finish_failed_capture(peer, &stop, &error);
                    return;
                }
            }
        } else {
            (None, None)
        };

        if stop.load(Ordering::Acquire) {
            cleanup_session(&stop);
            return;
        }
        let sender = match runtime.block_on(node.mirror_open(peer)) {
            Ok(sender) => sender,
            Err(error) => {
                tracing::warn!(peer = %peer.short(), %error, "couldn't open PC mirror stream to phone");
                #[cfg(target_os = "linux")]
                if wayland_dims.is_some() {
                    drop(wayland);
                    finish_failed_capture(peer, &stop, &error.into());
                    return;
                }
                cleanup_session(&stop);
                return;
            }
        };

        // The screen the phone picked (#442), when it isn't the main one: its
        // taps land there too, until this sharing ends. Looked up again now
        // and then, as screens come, go and change size.
        #[cfg(windows)]
        let shown_area = || {
            options
                .display
                .filter(|_| options.watch.is_none())
                .and_then(win::displays::by_id)
                .filter(|d| !d.primary)
                .map(|d| d.rect)
        };
        #[cfg(windows)]
        let set_area = |area: Option<RECT>| {
            let mut active = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(active) = active.as_mut().filter(|s| Arc::ptr_eq(&s.stop, &stop)) {
                active.area = area;
            }
        };
        #[cfg(windows)]
        let mut screen_area = shown_area();
        #[cfg(windows)]
        set_area(screen_area);
        #[cfg(windows)]
        let mut screens_checked = Instant::now();
        #[cfg(target_os = "linux")]
        let screen_area: Option<()> = None;

        // Give the phone's PcMirrorView TextureView and MediaCodec a brief moment to
        // mount before sending the initial MirrorConfig and IDR keyframe.
        std::thread::sleep(Duration::from_millis(120));

        // Lowered when the link can't keep up (frames dropped to hold the delay down),
        // raised back once it has been clean for a while.
        let mut bitrate = max_bitrate;
        let frame_interval = Duration::from_micros(1_000_000 / u64::from(fps));
        let dur_hns = (10_000_000 / u64::from(fps)) as i64;

        let mut bgra = Vec::new();
        let mut nv12 = Vec::new();
        #[cfg(windows)]
        let mut stream_dims: Option<(u32, u32)> = None;
        #[cfg(target_os = "linux")]
        let mut stream_dims = wayland_dims;
        let mut encoder: Option<ScreenEncoder> = None;
        let started = Instant::now();
        let mut last_reinit = Instant::now() - Duration::from_secs(5);
        let mut last_drop: Option<Instant> = None;
        let mut last_rate_change = Instant::now();
        let mut link_measured = false;
        let mut stats = Stats::default();
        let mut stats_since = Instant::now();
        // Play on phone: the player's picture instead of the screen, and the
        // PC's sound to that phone while it watches. The picture's place comes
        // in physical pixels; so does this thread's capture.
        if options.watch.is_some() {
            win::browser::use_physical_pixels();
        }
        let watched = options.watch.as_deref().map(crate::play_on_phone::Watched::follow);
        let mut window_capture: Option<win::window_capture::WindowCapture> = None;
        let _sound = watched.is_some().then(|| SoundToPhone::start(peer));
        if watched.is_some() {
            tracing::info!(peer = %peer.short(), fps, bitrate, "streaming a player to the phone");
        } else {
            tracing::info!(peer = %peer.short(), fps, bitrate, "sharing this PC's screen");
        }

        #[cfg(windows)]
        let mut dxgi_capture: Option<DxgiScreenCapture> = None;
        #[cfg(windows)]
        let mut dxgi_retry_after: Option<Instant> = None;
        #[cfg(windows)]
        let mut dxgi_wait_started: Option<Instant> = None;
        #[cfg(windows)]
        let mut gpu_nv12: Option<GpuBgraToNv12> = None;
        #[cfg(windows)]
        let mut gpu_pipeline_disabled = false;
        #[cfg(target_os = "linux")]
        let mut capture_failed = None;
        #[cfg(target_os = "linux")]
        let mut last_video = None;
        while !stop.load(Ordering::Relaxed) && !sender.is_closed() {
            let tick_start = Instant::now();

            #[cfg(windows)]
            if options.display.is_some() && screens_checked.elapsed() >= Duration::from_secs(2) {
                screens_checked = Instant::now();
                let now = shown_area();
                if now != screen_area {
                    // Moved, resized or gone (then the main screen): a new stream config.
                    screen_area = now;
                    set_area(screen_area);
                    dxgi_capture = None;
                    dxgi_retry_after = None;
                    dxgi_wait_started = None;
                    gpu_nv12 = None;
                    stream_dims = None;
                    encoder = None;
                }
            }

            let place = watched.as_ref().and_then(crate::play_on_phone::Watched::place);
            if watched.is_some() && place.is_none() && started.elapsed() < FIND_WINDOW {
                // Not found yet: nothing to show.
                std::thread::sleep(frame_interval);
                continue;
            }
            if watched.is_some() && place.is_none() {
                // The player's window can't be found. Not the screen instead:
                // that would show the phone whatever else is on it.
                tracing::info!(peer = %peer.short(), "can't find the player's window; stopping");
                crate::bridge::app::show_message(crate::i18n::tr!(
                    "PcMirror",
                    "Can't find the player's window. Play the video on this PC again, then turn on Play on phone."
                ));
                break;
            }
            if let Some(place) = place {
                let r = place.on_screen;
                let dims =
                    target_dimensions((r.right - r.left) as u32, (r.bottom - r.top) as u32, options.max_size);
                if stream_dims != Some(dims) {
                    // A new size (the window moved to full screen, say): a new stream config.
                    stream_dims = Some(dims);
                    encoder = None;
                }
                // The window's own pixels, whatever is in front of it; the
                // screen there when Windows can't capture the window.
                if window_capture.as_ref().is_none_or(|c| c.window() != place.window()) {
                    #[cfg(windows)]
                    {
                        gpu_nv12 = None;
                    }
                    window_capture = win::window_capture::WindowCapture::new(place.window())
                        .inspect_err(
                            |e| tracing::info!(error = %e, "can't capture the window; capturing the screen"),
                        )
                        .ok();
                }
            }

            #[cfg(windows)]
            if watched.is_none()
                && !gpu_pipeline_disabled
                && dxgi_capture.is_none()
                && dxgi_retry_after.is_none_or(|at| Instant::now() >= at)
            {
                match DxgiScreenCapture::new(screen_area) {
                    Ok(capture) => {
                        dxgi_capture = Some(capture);
                        dxgi_retry_after = None;
                        dxgi_wait_started = None;
                    }
                    Err(error) => {
                        tracing::debug!(%error, "DXGI desktop duplication unavailable; using GDI fallback");
                        dxgi_retry_after = Some(Instant::now() + Duration::from_secs(3));
                    }
                }
            }

            #[cfg(windows)]
            let gpu_device = if gpu_pipeline_disabled {
                None
            } else if watched.is_some() {
                window_capture.as_ref().and_then(|c| c.adapter_luid().map(|luid| (c.device().clone(), luid)))
            } else {
                dxgi_capture.as_ref().map(|c| (c.device().clone(), c.adapter_luid()))
            };

            #[cfg(windows)]
            if let Some(enc) = encoder.as_ref() {
                let mismatch = match (&gpu_device, enc.on_gpu()) {
                    (Some((dev, _)), true) => !enc.matches_gpu_device(dev),
                    (Some(_), false) => true,
                    (None, true) => true,
                    (None, false) => false,
                };
                if mismatch {
                    encoder = None;
                }
            }

            let (dst_w, dst_h) = match stream_dims {
                Some(dims) => dims,
                None => {
                    #[cfg(windows)]
                    let (src_w, src_h) = screen_area.map_or_else(primary_screen_dimensions, |a| {
                        ((a.right - a.left).max(1) as u32, (a.bottom - a.top).max(1) as u32)
                    });
                    #[cfg(target_os = "linux")]
                    let (src_w, src_h) = primary_screen_dimensions();
                    let dims = target_dimensions(src_w, src_h, options.max_size);
                    stream_dims = Some(dims);
                    dims
                }
            };

            #[cfg(target_os = "linux")]
            if let Some(caster) = wayland.as_mut() {
                match caster.capture(runtime, &mut bgra) {
                    Ok(true) => {
                        if last_video.is_none() && encoder.is_some() {
                            force_keyframe.store(true, Ordering::Relaxed);
                        }
                        if last_video.get_or_insert_with(Instant::now).elapsed() >= Duration::from_secs(12) {
                            capture_failed =
                                Some(anyhow::anyhow!("the video encoder stopped producing frames"));
                            break;
                        }
                    }
                    Ok(false) => {
                        last_video = None;
                        std::thread::sleep(frame_interval);
                        continue;
                    }
                    Err(error) => {
                        capture_failed = Some(error);
                        break;
                    }
                }
            }

            // A keyframe on request comes from the running encoder. Rebuilding it (a new
            // stream config plus a full keyframe) is the fallback for encoders that can't.
            let requested_kf = force_keyframe.swap(false, Ordering::SeqCst);
            let keyframe_in_place =
                requested_kf && encoder.as_mut().is_some_and(|enc| enc.request_keyframe());
            #[cfg(linux_gst_nvenc)]
            let keyframe_in_place = keyframe_in_place
                || if requested_kf && nvenc_portal.is_some() {
                    force_keyframe.store(true, Ordering::Relaxed);
                    true
                } else {
                    false
                };
            let need_reinit = encoder.is_none()
                || (requested_kf
                    && !keyframe_in_place
                    && last_reinit.elapsed() >= Duration::from_millis(1400));

            if need_reinit {
                #[cfg(windows)]
                let init_result = if let Some((ref dev, luid)) = gpu_device {
                    match new_pc_gpu_encoder_with_fallback(&mut codec, dst_w, dst_h, fps, bitrate, dev, luid)
                    {
                        Ok(enc) => Ok(enc),
                        Err(error) => {
                            tracing::info!(%error, "D3D11-aware hardware encoder unavailable; using CPU fallback");
                            gpu_pipeline_disabled = true;
                            gpu_nv12 = None;
                            new_pc_encoder_with_fallback(&mut codec, dst_w, dst_h, fps, bitrate)
                        }
                    }
                } else {
                    new_pc_encoder_with_fallback(&mut codec, dst_w, dst_h, fps, bitrate)
                };
                #[cfg(all(target_os = "linux", not(linux_gst_nvenc)))]
                let init_result = new_pc_encoder_with_fallback(&mut codec, dst_w, dst_h, fps, bitrate);
                #[cfg(linux_gst_nvenc)]
                let init_result = match nvenc.take() {
                    Some(worker) => Ok(worker),
                    None => new_pc_encoder_with_fallback(&mut codec, dst_w, dst_h, fps, bitrate),
                };

                match init_result {
                    Ok(enc) => {
                        let cfg = MirrorConfig {
                            codec: codec.clone(),
                            width: dst_w,
                            height: dst_h,
                            session: nectarlink_core::MIRROR_SCREEN,
                        };
                        if matches!(sender.send(PacketKind::Config, 0, cfg.to_cbor()), MirrorSend::Closed) {
                            break;
                        }
                        let mut active = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
                        if let Some(active) = active.as_mut().filter(|s| Arc::ptr_eq(&s.stop, &stop)) {
                            active.video_ready = true;
                        }
                        encoder = Some(enc);
                        last_reinit = Instant::now();
                        stats.rebuilds += 1;
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, codec, "PC mirror encoder init failed");
                        #[cfg(target_os = "linux")]
                        if wayland.is_some() {
                            capture_failed =
                                Some(anyhow::anyhow!("couldn't initialize the video encoder: {e}"));
                        }
                        break;
                    }
                }
            }

            #[cfg(windows)]
            let gpu_encode_result = if !gpu_pipeline_disabled
                && encoder.as_ref().is_some_and(ScreenEncoder::on_gpu)
            {
                let mut rebuild_for_cpu = false;
                let mut wait_for_frame = false;
                let mut encoded = None;

                if let Some(place) = place {
                    if let Some(capture) = window_capture.as_mut() {
                        if let Some((dev, src_tex)) = capture.latest_texture(place.in_window) {
                            match convert_to_gpu_nv12(&mut gpu_nv12, dev, src_tex, dst_w, dst_h) {
                                Ok(nv12_tex) => {
                                    let time_us = started.elapsed().as_micros() as u64;
                                    let hns = (time_us * 10) as i64;
                                    if let Some(enc) = encoder.as_mut() {
                                        encoded = Some((
                                            time_us,
                                            enc.encode_surface(nv12_tex, hns, dur_hns, &stop),
                                        ));
                                    }
                                }
                                Err(error) => {
                                    tracing::info!(%error, "GPU BGRA-to-NV12 unavailable; falling back to CPU pipeline");
                                    gpu_pipeline_disabled = true;
                                    gpu_nv12 = None;
                                    rebuild_for_cpu = true;
                                }
                            }
                        } else {
                            wait_for_frame = true;
                        }
                    }
                } else if let Some(dxgi) = dxgi_capture.as_mut() {
                    let dev = dxgi.device().clone();
                    match dxgi.acquire_texture() {
                        Ok(Some(src_tex)) => {
                            dxgi_wait_started = None;
                            match convert_to_gpu_nv12(&mut gpu_nv12, &dev, src_tex, dst_w, dst_h) {
                                Ok(nv12_tex) => {
                                    let time_us = started.elapsed().as_micros() as u64;
                                    let hns = (time_us * 10) as i64;
                                    if let Some(enc) = encoder.as_mut() {
                                        encoded = Some((
                                            time_us,
                                            enc.encode_surface(nv12_tex, hns, dur_hns, &stop),
                                        ));
                                    }
                                }
                                Err(error) => {
                                    tracing::info!(%error, "GPU BGRA-to-NV12 unavailable; falling back to CPU pipeline");
                                    gpu_pipeline_disabled = true;
                                    gpu_nv12 = None;
                                    rebuild_for_cpu = true;
                                }
                            }
                        }
                        Ok(None) => {
                            if dxgi_wait_started.get_or_insert_with(Instant::now).elapsed()
                                >= Duration::from_millis(500)
                            {
                                dxgi_capture = None;
                                dxgi_wait_started = None;
                                dxgi_retry_after = Some(Instant::now() + Duration::from_secs(3));
                                gpu_nv12 = None;
                                rebuild_for_cpu = true;
                            } else {
                                wait_for_frame = true;
                            }
                        }
                        Err(error) => {
                            tracing::info!(%error, "DXGI desktop duplication lost; falling back to GDI");
                            dxgi_capture = None;
                            dxgi_wait_started = None;
                            dxgi_retry_after = Some(Instant::now() + Duration::from_secs(2));
                            gpu_nv12 = None;
                            rebuild_for_cpu = true;
                        }
                    }
                }

                if rebuild_for_cpu {
                    encoder = None;
                    continue;
                }
                if wait_for_frame {
                    let elapsed = tick_start.elapsed();
                    if let Some(rem) = frame_interval.checked_sub(elapsed) {
                        std::thread::sleep(rem);
                    }
                    continue;
                }
                encoded
            } else {
                None
            };

            #[cfg(windows)]
            let encode_attempt = if let Some(gpu_result) = gpu_encode_result {
                Some(gpu_result)
            } else {
                let captured = match (place, window_capture.as_mut()) {
                    (Some(place), Some(capture)) => capture.capture(place.in_window, dst_w, dst_h, &mut bgra),
                    (Some(place), None) => {
                        capture_screen(Some(place.on_screen), dst_w, dst_h, &mut bgra).is_some()
                    }
                    (None, _) => capture_shown(screen_area, dst_w, dst_h, &mut bgra),
                };
                if captured && let Some(enc) = encoder.as_mut() {
                    bgra_to_nv12(&bgra, dst_w as usize, dst_h as usize, &mut nv12);
                    let time_us = started.elapsed().as_micros() as u64;
                    let hns = (time_us * 10) as i64;
                    Some((time_us, enc.encode(&nv12, hns, dur_hns, &stop)))
                } else {
                    None
                }
            };

            #[cfg(target_os = "linux")]
            let encode_attempt = (|| {
                #[cfg(linux_gst_nvenc)]
                if nvenc_portal.is_some() {
                    return None;
                }
                let wayland_captured = wayland.is_some();
                let captured = wayland_captured
                    || match (place, window_capture.as_mut()) {
                        (Some(place), Some(capture)) => {
                            capture.capture(place.in_window, dst_w, dst_h, &mut bgra)
                        }
                        (Some(place), None) => {
                            capture_screen(Some(place.on_screen), dst_w, dst_h, &mut bgra).is_some()
                        }
                        (None, _) => capture_shown(screen_area, dst_w, dst_h, &mut bgra),
                    };
                if captured && let Some(enc) = encoder.as_mut() {
                    bgra_to_nv12(&bgra, dst_w as usize, dst_h as usize, &mut nv12);
                    let time_us = started.elapsed().as_micros() as u64;
                    let hns = (time_us * 10) as i64;
                    Some((time_us, enc.encode(&nv12, hns, dur_hns, &stop)))
                } else {
                    None
                }
            })();

            let result = encode_attempt.map(|(time_us, result)| {
                result.map(|packets| {
                    packets.into_iter().map(|(key, bytes)| (time_us, key, bytes)).collect::<Vec<_>>()
                })
            });
            #[cfg(linux_gst_nvenc)]
            let result = match encoder.as_mut() {
                Some(ScreenEncoder::Nvenc(worker)) => Some(worker.read(started.elapsed().as_micros() as u64)),
                _ => result,
            };
            if let Some(result) = result
                && let Some(enc) = encoder.as_mut()
            {
                match result {
                    Ok(packets) => {
                        #[cfg(target_os = "linux")]
                        if !packets.is_empty() {
                            last_video = Some(Instant::now());
                        }
                        let mut send_failed = false;
                        for (time_us, is_keyframe, bytes) in packets {
                            let kind = if is_keyframe { PacketKind::Keyframe } else { PacketKind::Frame };
                            let len = bytes.len();
                            match sender.send(kind, time_us, bytes) {
                                MirrorSend::Queued => {
                                    stats.frames += 1;
                                    stats.bytes += len as u64;
                                    if is_keyframe {
                                        stats.keyframes += 1;
                                    }
                                }
                                MirrorSend::Dropped => stats.dropped += 1,
                                MirrorSend::NeedKeyframe => {
                                    stats.dropped += 1;
                                    last_drop = Some(Instant::now());
                                    force_keyframe.store(true, Ordering::SeqCst);
                                    // The link is slower than the stream: ask for less, at most every
                                    // 2 s. A phone that reports what it gets lets this be measured instead.
                                    if !link_measured && last_rate_change.elapsed() >= Duration::from_secs(2)
                                    {
                                        let lower = (bitrate / 4 * 3).max(MIN_BITRATE);
                                        if lower < bitrate && enc.set_bitrate(lower) {
                                            bitrate = lower;
                                            last_rate_change = Instant::now();
                                        }
                                    }
                                }
                                MirrorSend::Closed => {
                                    send_failed = true;
                                    break;
                                }
                            }
                        }
                        if send_failed {
                            break;
                        }
                        // Video got ahead of the link, which says how much it carries. Leave
                        // room for sound and keyframes, and at most halve it: one bad moment
                        // on Wi-Fi shouldn't leave the picture blurry for a minute.
                        if let Some(rate) = sender.link_rate() {
                            link_measured = true;
                            last_drop = Some(Instant::now());
                            if last_rate_change.elapsed() >= Duration::from_secs(1) {
                                let room = u32::try_from(rate / 10 * 7).unwrap_or(u32::MAX);
                                let lower = room.max(bitrate / 2).max(MIN_BITRATE);
                                if lower < bitrate && enc.set_bitrate(lower) {
                                    bitrate = lower;
                                    last_rate_change = Instant::now();
                                }
                            }
                        }
                        // Clean for 10 s: step the bitrate back up toward what the phone asked for.
                        let clean = last_drop.is_none_or(|t| t.elapsed() >= Duration::from_secs(10));
                        if clean
                            && bitrate < max_bitrate
                            && last_rate_change.elapsed() >= Duration::from_secs(10)
                        {
                            let higher = (bitrate / 4 * 5).min(max_bitrate);
                            if enc.set_bitrate(higher) {
                                bitrate = higher;
                                last_rate_change = Instant::now();
                            }
                        }
                    }
                    Err(e) => {
                        if stop.load(Ordering::Relaxed) {
                            break;
                        }
                        #[cfg(linux_gst_nvenc)]
                        if let Some(portal) = nvenc_portal.take() {
                            tracing::warn!(error = %e, "NVENC worker failed; using raw Wayland capture");
                            encoder = None;
                            match WaylandScreenCaster::start(runtime, portal, (dst_w, dst_h)) {
                                Ok(caster) => {
                                    wayland = Some(caster);
                                    continue;
                                }
                                Err(error) => {
                                    capture_failed = Some(error);
                                    break;
                                }
                            }
                        }
                        #[cfg(any(windows, linux_ffmpeg_video))]
                        if enc.next_encoder(&codec, dst_w, dst_h, fps, bitrate) {
                            tracing::warn!(error = %e, codec, "Video encoding failed; trying another encoder");
                            stats.rebuilds += 1;
                            continue;
                        }
                        #[cfg(windows)]
                        if enc.on_gpu() && codec == "h264" {
                            tracing::warn!(error = %e, "GPU H.264 encoding failed; falling back to CPU pipeline");
                            gpu_pipeline_disabled = true;
                            gpu_nv12 = None;
                            encoder = None;
                            continue;
                        }
                        if codec == "h265" {
                            tracing::warn!(error = %e, "HEVC screen encoding failed; using H.264");
                            codec = "h264".into();
                        } else {
                            tracing::warn!(error = %e, "PC mirror frame encode error; rebuilding the encoder");
                        }
                        encoder = None;
                    }
                }
            }

            if stats_since.elapsed() >= Duration::from_secs(10) {
                let secs = stats_since.elapsed().as_secs_f64();
                tracing::info!(
                    peer = %peer.short(),
                    fps = format!("{:.1}", stats.frames as f64 / secs),
                    kbps = (stats.bytes as f64 * 8.0 / secs / 1000.0) as u64,
                    keyframes = stats.keyframes,
                    dropped = stats.dropped,
                    rebuilds = stats.rebuilds,
                    bitrate,
                    "sharing this PC's screen"
                );
                stats = Stats::default();
                stats_since = Instant::now();
            }

            let elapsed = tick_start.elapsed();
            if let Some(rem) = frame_interval.checked_sub(elapsed) {
                std::thread::sleep(rem);
            }
        }

        #[cfg(target_os = "linux")]
        {
            if wayland_dims.is_some() && sender.is_closed() && !stop.load(Ordering::Acquire) {
                capture_failed.get_or_insert_with(|| anyhow::anyhow!("the video stream disconnected"));
            }
            drop(wayland);
            if let Some(error) = capture_failed {
                sender.close();
                finish_failed_capture(peer, &stop, &error);
                return;
            }
        }
        tracing::info!(peer = %peer.short(), "stopped sharing this PC's screen");
        sender.close();
        cleanup_session(&stop);
    }

    #[cfg(target_os = "linux")]
    fn finish_failed_capture(peer: DeviceId, stop: &Arc<AtomicBool>, error: &anyhow::Error) {
        if cleanup_session(stop) && !stop.load(Ordering::Acquire) {
            tracing::warn!(peer = %peer.short(), error = %format_args!("{error:#}"), "Wayland screen sharing stopped");
            if let Some(node) = core_host::node() {
                core_host::spawn(async move {
                    // A retry can take over while resources from the old attempt close.
                    if active_peer() != Some(peer) {
                        let _ = node
                            .mirror_notice(
                                peer,
                                nectarlink_core::MIRROR_SCREEN,
                                nectarlink_core::MIRROR_NOTICE_PC_CAPTURE_FAILED,
                            )
                            .await;
                    }
                });
            }
        }
    }

    /// Mute this PC's speakers while a phone watches (the setting).
    static MUTE_WHILE_WATCHED: AtomicBool = AtomicBool::new(true);

    pub fn set_mute_while_watched(on: bool) {
        MUTE_WHILE_WATCHED.store(on, Ordering::Relaxed);
    }

    /// The PC's sound going to a phone that watches one of its players (as
    /// Phone speaker sends it), with the PC's speakers muted if the user wants
    /// (capturing the sound doesn't depend on them), for as long as this lives;
    /// then as before.
    /// The PC's sound goes to the phone while it watches, and comes back
    /// afterwards. A restart starts a new one before the old one ends, so
    /// the settings from before the first are kept here, and only the newest
    /// puts them back.
    struct SoundToPhone {
        generation: u64,
    }

    /// Whether the sound went to the phone, and where, before any of this;
    /// and whether this muted the speakers (they weren't already).
    static BEFORE: Mutex<Option<(bool, Option<DeviceId>, bool)>> = Mutex::new(None);
    static NEWEST: AtomicU64 = AtomicU64::new(0);

    impl SoundToPhone {
        fn start(peer: DeviceId) -> SoundToPhone {
            let generation = NEWEST.fetch_add(1, Ordering::AcqRel) + 1;
            let mut before = BEFORE.lock().unwrap_or_else(|e| e.into_inner());
            if before.is_none() {
                let muted = MUTE_WHILE_WATCHED.load(Ordering::Relaxed)
                    && win::audio::speaker_state().is_some_and(|(_, muted)| !muted)
                    && win::audio::set_speaker_mute(true).is_ok();
                *before =
                    Some((crate::pc_audio_stream::is_enabled(), crate::pc_audio_stream::target(), muted));
            }
            crate::pc_audio_stream::set_target(Some(peer));
            crate::pc_audio_stream::set_enabled(true);
            SoundToPhone { generation }
        }
    }

    impl Drop for SoundToPhone {
        fn drop(&mut self) {
            if NEWEST.load(Ordering::Acquire) != self.generation {
                return;
            }
            let Some((was_on, was_target, muted)) = BEFORE.lock().unwrap_or_else(|e| e.into_inner()).take()
            else {
                return;
            };
            crate::pc_audio_stream::set_target(was_target);
            crate::pc_audio_stream::set_enabled(was_on);
            // Unmuted only if this muted it.
            if muted {
                let _ = win::audio::set_speaker_mute(false);
            }
        }
    }

    fn cleanup_session(stop: &Arc<AtomicBool>) -> bool {
        let removed = {
            let mut guard = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
            if guard.as_ref().is_some_and(|s| Arc::ptr_eq(&s.stop, stop)) {
                guard.take().is_some()
            } else {
                false
            }
        };
        if removed {
            #[cfg(windows)]
            win::input::set_absolute_area(None);
            win::input::release_held_buttons();
            crate::input_share::leave_remote();
            core_host::host().hub.changed(crate::state::Changes::REMOTE);
            core_host::refresh_capabilities();
        }
        removed
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[cfg(windows)]
        #[test]
        fn encoder_failures_advance_to_the_next_candidate() {
            ensure_mf_started().unwrap();
            let first = video_encoder_activations(MFVideoFormat_H264, None).unwrap().remove(0);
            let second = video_encoder_activations(MFVideoFormat_H264, None).unwrap().remove(0);
            let third = video_encoder_activations(MFVideoFormat_H264, None).unwrap().remove(0);
            let mut attempts = 0;
            let mut encoder = try_video_encoders(vec![first, second, third], |transform| {
                attempts += 1;
                if attempts == 1 {
                    return Err(windows::core::Error::from(E_INVALIDARG));
                }
                ScreenEncoder::from_transform("h264", transform, 320, 240, 30, 2_000_000, None)
            })
            .unwrap();
            assert_eq!(attempts, 2);
            let input = vec![128; 320 * 240 * 3 / 2];
            let stop = AtomicBool::new(false);
            let mut packets = 0;
            for i in 0..8 {
                packets += encoder.encode(&input, i * 333_333, 333_333, &stop).unwrap().len();
            }
            assert!(packets > 0);
            assert!(encoder.next_encoder("h264", 320, 240, 30, 2_000_000));
            let mut keys = 0;
            for i in 0..8 {
                keys += encoder
                    .encode(&input, i * 333_333, 333_333, &stop)
                    .unwrap()
                    .iter()
                    .filter(|(keyframe, _)| *keyframe)
                    .count();
            }
            assert!(keys > 0, "a replacement encoder starts with a keyframe");
            assert!(!encoder.next_encoder("h264", 320, 240, 30, 2_000_000));
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn wayland_capture_reads_the_stream_through_the_portal_connection() {
            let args = pipeline_args(57, 1280, 720, WaylandCaptureMode::PipeWire);
            assert_eq!(args[1], "pipewiresrc");
            assert!(args.contains(&"fd=3".to_owned()), "the portal's PipeWire connection, as fd 3");
            assert!(args.contains(&"path=57".to_owned()));
            assert!(args.contains(&"video/x-raw,format=BGRx,width=1280,height=720".to_owned()));
            assert_eq!(args.last().map(String::as_str), Some("fd=1"));
            assert!(!args.iter().any(|a| a.starts_with("gl")));
            assert!(args.windows(3).any(|args| args == ["video/x-raw", "!", "videoconvert"]));

            // Through OpenGL: GPU buffers downloaded as RGBA, then the same tail.
            let gl = pipeline_args(57, 1280, 720, WaylandCaptureMode::OpenGl);
            let at = |name: &str| gl.iter().position(|a| a == name).unwrap_or(usize::MAX);
            assert!(gl.windows(5).any(|args| {
                args == [
                    "glcolorscale",
                    "!",
                    "video/x-raw(memory:GLMemory),format=RGBA,width=1280,height=720",
                    "!",
                    "gldownload",
                ]
            }));
            assert!(at("glupload") < at("glcolorconvert") && at("glcolorconvert") < at("gldownload"));
            assert!(at("gldownload") < at("videoconvert"));
            assert!(!gl.iter().any(|arg| arg == "video/x-raw"));
            assert!(gl.contains(&"video/x-raw,format=BGRx,width=1280,height=720".to_owned()));

            let retry_gl = pipeline_args(57, 1280, 720, WaylandCaptureMode::LastOpenGl);
            assert!(retry_gl.iter().any(|arg| arg == "glupload"));
            assert!(!retry_gl.iter().any(|arg| arg == "glcolorscale"));
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn wayland_capture_can_return_to_pipewire_without_looping() {
            use WaylandCaptureMode::{LastOpenGl, OpenGl, PipeWire, RetryPipeWire};

            assert_eq!(PipeWire.fallback(), Some(OpenGl));
            assert_eq!(OpenGl.fallback(), Some(RetryPipeWire));
            assert!(!RetryPipeWire.uses_gl());
            assert_eq!(RetryPipeWire.fallback(), Some(LastOpenGl));
            assert!(LastOpenGl.uses_gl());
            assert_eq!(LastOpenGl.fallback(), None);
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn wayland_black_frame_probe_ignores_alpha_and_small_cursor() {
            let mut frame = [0u8; 320 * 240 * 4];
            for pixel in frame.as_chunks_mut::<4>().0 {
                pixel[3] = 255;
            }
            frame[..4].fill(255);
            assert!(wayland_frame_is_nearly_black(&frame));

            frame[320 * 4 * 100..320 * 4 * 120].fill(20);
            assert!(!wayland_frame_is_nearly_black(&frame));
        }

        #[test]
        fn computes_target_dimensions_and_nv12() {
            let (w, h) = target_dimensions(1920, 1080, 1280);
            assert_eq!(w % 16, 0);
            assert_eq!(h % 16, 0);
            let bgra = vec![128u8; (w as usize) * (h as usize) * 4];
            let mut nv12 = Vec::new();
            bgra_to_nv12(&bgra, w as usize, h as usize, &mut nv12);
            assert_eq!(nv12.len(), (w as usize) * (h as usize) * 3 / 2);
        }

        #[test]
        fn recognizes_hevc_random_access_frames_and_parameter_sets() {
            let headers = [
                0, 0, 0, 1, 0x40, 1, // VPS
                0, 0, 1, 0x42, 1, // SPS
                0, 0, 0, 1, 0x44, 1, // PPS
            ];
            let idr = [0, 0, 1, 0x26, 1]; // IDR_W_RADL, type 19
            assert!(has_h265_parameters(&headers));
            assert!(!has_h265_irap(&headers));
            assert!(has_h265_irap(&idr));
            assert!(!has_h265_parameters(&idr));
        }

        #[cfg(windows)]
        #[test]
        fn async_encoder_waits_time_out_and_honor_cancellation() {
            let mut encoder = match ScreenEncoder::new_for_codec("h265", 1280, 720, 30, 2_000_000) {
                Ok(encoder) if encoder.async_events.is_some() => encoder,
                _ => {
                    eprintln!("No asynchronous HEVC encoder available");
                    return;
                }
            };
            let stop = AtomicBool::new(false);
            let cancelled = std::thread::scope(|scope| {
                scope.spawn(|| {
                    std::thread::sleep(Duration::from_millis(50));
                    stop.store(true, Ordering::Relaxed);
                });
                encoder.wait_for_async_event(METransformHaveOutput.0 as u32, &stop).unwrap_err()
            });
            assert_eq!(cancelled.code(), MF_E_OPERATION_CANCELLED);

            stop.store(false, Ordering::Relaxed);
            // No frame has been submitted, so an output event cannot arrive.
            let timeout = encoder.wait_for_async_event(METransformHaveOutput.0 as u32, &stop).unwrap_err();
            assert_eq!(timeout.code(), ERROR_TIMEOUT.to_hresult());
        }

        #[test]
        fn hevc_encoder_round_trips_when_available() {
            let (w, h) = target_dimensions(1280, 720, 1280);
            let mut enc = match ScreenEncoder::new_for_codec("h265", w, h, 30, 2_000_000) {
                Ok(encoder) => encoder,
                Err(error) => {
                    eprintln!("HEVC encoder unavailable: {error}");
                    return;
                }
            };
            let mut dec = match crate::win::h264::Decoder::new_for_codec("h265") {
                Ok(decoder) => decoder,
                Err(error) => {
                    eprintln!("HEVC decoder unavailable: {error}");
                    return;
                }
            };
            let nv12 = vec![128u8; (w as usize) * (h as usize) * 3 / 2];
            let mut produced_packets = 0usize;
            let mut decoded_picture = false;
            let stop = AtomicBool::new(false);
            for i in 0..8 {
                for (keyframe, data) in enc
                    .encode(&nv12, i * 333_333, 333_333, &stop)
                    .unwrap_or_else(|error| panic!("encode HEVC frame {i}: {error}"))
                {
                    produced_packets += 1;
                    if keyframe {
                        assert!(has_h265_irap(&data), "keyframes must contain an HEVC IRAP NAL");
                        assert!(has_h265_parameters(&data), "keyframes must carry VPS, SPS and PPS");
                    }
                    decoded_picture |=
                        !dec.decode(&data, i as u64 * 33_333).expect("decode packet").is_empty();
                }
            }
            assert!(produced_packets > 0, "encoder must emit H.265 access units");
            assert!(decoded_picture, "decoder must reconstruct an H.265 picture");
        }

        #[cfg(any(windows, target_os = "linux"))]
        #[test]
        fn encodes_nv12_frames_and_decodes_back() {
            let (w, h) = target_dimensions(1920, 1080, 1280);
            assert_eq!(w % 16, 0);
            assert_eq!(h % 16, 0);

            let mut enc = new_pc_encoder("h264", w, h, 30, 4_000_000).expect("ScreenEncoder");
            let mut dec = crate::win::h264::Decoder::new_for_codec("h264").expect("H264 Decoder");

            let mut bgra = vec![0u8; (w as usize) * (h as usize) * 4];
            for (i, px) in bgra.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                px[0] = (i % 255) as u8;
                px[1] = ((i / 7) % 255) as u8;
                px[2] = ((i / 13) % 255) as u8;
                px[3] = 255;
            }
            let mut nv12 = Vec::new();
            bgra_to_nv12(&bgra, w as usize, h as usize, &mut nv12);

            let mut decoded_any = false;
            let mut produced_packets = 0usize;
            let stop = AtomicBool::new(false);
            for i in 0..8 {
                let packets = enc.encode(&nv12, i * 333_333, 333_333, &stop).expect("encode frame");
                for (_keyframe, data) in packets {
                    produced_packets += 1;
                    let pics = dec.decode(&data, (i as u64) * 33_333).expect("decode packet");
                    if let Some(pic) = pics.first() {
                        assert_eq!(pic.width, w);
                        assert_eq!(pic.height, h);
                        decoded_any = true;
                    }
                }
            }
            assert!(produced_packets > 0, "encoder must emit H.264 access units");
            assert!(decoded_any, "decoder must reconstruct at least one frame from ScreenEncoder");
        }

        /// With a Chromium browser showing a video on a page whose title has
        /// `NECTARLINK_TEST_TAB` in it, in view: captures just the video, as
        /// Play on phone does.
        #[cfg(windows)]
        #[test]
        #[ignore = "needs a browser tab titled NECTARLINK_TEST_TAB"]
        fn captures_a_browsers_video() {
            use crate::win::browser::{Automation, window_for};
            let wanted = std::env::var("NECTARLINK_TEST_TAB").expect("NECTARLINK_TEST_TAB");
            // SAFETY: COM for this test's thread.
            let _ = unsafe {
                windows::Win32::System::Com::CoInitializeEx(
                    None,
                    windows::Win32::System::Com::COINIT_MULTITHREADED,
                )
            };
            crate::win::browser::use_physical_pixels();
            let window = window_for("Chrome", &wanted).expect("the test tab's window");
            let rect = Automation::new().unwrap().video_rect(&window).expect("the video");
            let (sw, sh) = ((rect.right - rect.left) as u32, (rect.bottom - rect.top) as u32);
            let (w, h) = target_dimensions(sw, sh, 1280);
            let mut bgra = Vec::new();
            assert_eq!(capture_screen(Some(rect), w, h, &mut bgra), Some((sw, sh)));
            let distinct: std::collections::HashSet<&[u8; 4]> = bgra.as_chunks::<4>().0.iter().collect();
            eprintln!("{rect:?} as {w}x{h}: {} colors", distinct.len());
            assert!(distinct.len() > 50, "a moving test pattern, not a blank");
        }

        #[cfg(any(windows, target_os = "linux"))]
        #[test]
        fn keyframes_come_on_request_without_rebuilding_the_encoder() {
            let (w, h) = target_dimensions(1280, 720, 1280);
            #[cfg(windows)]
            let mut enc = new_pc_encoder("h264", w, h, 30, 2_000_000).expect("ScreenEncoder");
            #[cfg(target_os = "linux")]
            let mut enc =
                ScreenEncoder::OpenH264(OpenH264Encoder::new(w, h, 30, 2_000_000).expect("OpenH264"));
            let mut nv12 = Vec::new();
            bgra_to_nv12(&vec![128u8; (w as usize) * (h as usize) * 4], w as usize, h as usize, &mut nv12);
            let stop = AtomicBool::new(false);

            let keyframes_after = |enc: &mut ScreenEncoder, start: i64, count: i64| {
                (start..start + count)
                    .flat_map(|i| enc.encode(&nv12, i * 333_333, 333_333, &stop).expect("encode frame"))
                    .filter(|(keyframe, data)| *keyframe && has_h264_idr(data))
                    .count()
            };
            // The first frame is a keyframe; the next few (well inside the 2 s GOP) are not.
            assert_eq!(keyframes_after(&mut enc, 0, 1), 1, "the stream starts with a keyframe");
            assert_eq!(keyframes_after(&mut enc, 1, 5), 0, "no keyframes mid-GOP unprompted");
            assert!(enc.request_keyframe(), "the H.264 encoder takes keyframe requests");
            assert_eq!(keyframes_after(&mut enc, 6, 2), 1, "a requested keyframe arrives promptly");
            assert!(enc.set_bitrate(1_000_000), "the bitrate changes in place");
        }

        #[cfg(windows)]
        #[test]
        fn gpu_bgra_to_nv12_and_surface_encoder_round_trip_when_available() {
            use windows::Win32::Graphics::{
                Direct3D::D3D_DRIVER_TYPE_UNKNOWN,
                Direct3D11::{
                    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_SDK_VERSION, D3D11_SUBRESOURCE_DATA,
                    D3D11_USAGE_DEFAULT, D3D11CreateDevice, ID3D11Multithread,
                },
                Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC},
            };

            let Ok(adapters) = video_adapters() else {
                return;
            };
            let Some(adapter) = adapters.into_iter().next() else {
                eprintln!("No hardware DXGI adapter available");
                return;
            };
            let mut device = None;
            // SAFETY: creates a hardware D3D11 device on the first non-software adapter.
            let created = unsafe {
                D3D11CreateDevice(
                    &adapter.adapter,
                    D3D_DRIVER_TYPE_UNKNOWN,
                    Default::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    None,
                )
            };
            if created.is_err() {
                return;
            }
            let Some(device) = device else {
                return;
            };
            if let Ok(mt) = device.cast::<ID3D11Multithread>() {
                // SAFETY: valid D3D11 multithread interface.
                unsafe {
                    let _ = mt.SetMultithreadProtected(true);
                }
            }
            let (src_w, src_h) = (640u32, 360u32);
            let (dst_w, dst_h) = target_dimensions(src_w, src_h, 1280);
            let Ok(mut converter) = GpuBgraToNv12::new(&device, (src_w, src_h), (dst_w, dst_h)) else {
                eprintln!("ID3D11VideoProcessor BGRA->NV12 unavailable");
                return;
            };
            let mut bgra = vec![0u8; (src_w as usize) * (src_h as usize) * 4];
            for px in bgra.as_chunks_mut::<4>().0.iter_mut() {
                px[0] = 200;
                px[1] = 100;
                px[2] = 50;
                px[3] = 255;
            }
            let mut src_tex: Option<ID3D11Texture2D> = None;
            // SAFETY: creates a BGRA8 texture initialized from `bgra`.
            unsafe {
                device
                    .CreateTexture2D(
                        &D3D11_TEXTURE2D_DESC {
                            Width: src_w,
                            Height: src_h,
                            MipLevels: 1,
                            ArraySize: 1,
                            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                            Usage: D3D11_USAGE_DEFAULT,
                            // As the capture textures are: NVIDIA's video processor
                            // takes nothing else.
                            BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
                            CPUAccessFlags: 0,
                            MiscFlags: 0,
                        },
                        Some(&D3D11_SUBRESOURCE_DATA {
                            pSysMem: bgra.as_ptr().cast(),
                            SysMemPitch: src_w * 4,
                            SysMemSlicePitch: bgra.len() as u32,
                        }),
                        Some(&mut src_tex),
                    )
                    .unwrap();
            }
            let src_tex = src_tex.unwrap();
            let nv12_tex = converter.convert(&src_tex, None).unwrap();
            let mut enc =
                match ScreenEncoder::new_gpu("h264", dst_w, dst_h, 30, 2_000_000, &device, adapter.luid) {
                    Ok(enc) => enc,
                    Err(error) => {
                        eprintln!("D3D11-aware H.264 encoder unavailable: {error}");
                        return;
                    }
                };
            let stop = AtomicBool::new(false);
            let mut packets = 0usize;
            for i in 0..8 {
                packets += enc
                    .encode_surface(nv12_tex, i * 333_333, 333_333, &stop)
                    .expect("encode GPU surface")
                    .len();
            }
            assert!(packets > 0, "D3D11-aware encoder must emit H.264 access units");
        }
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod imp {
    #![allow(dead_code)]

    use nectarlink_core::{DeviceId, MirrorInput, MirrorStart, NodeEvent};

    pub fn active_peer() -> Option<DeviceId> {
        None
    }

    pub fn is_active() -> bool {
        false
    }

    pub fn is_watching() -> bool {
        false
    }

    pub fn stop_watching() {}

    pub fn start(_peer: DeviceId, _options: MirrorStart) {}

    pub fn stop_for(_peer: &DeviceId) {}

    pub fn stop_all() {}

    pub fn request_keyframe(_peer: &DeviceId) {}

    pub fn handle_input(_peer: &DeviceId, _session: u32, _input: MirrorInput) {}

    pub fn on_event(_event: &NodeEvent) {}

    pub fn set_mute_while_watched(_on: bool) {}
}

pub use imp::*;
