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
        Foundation::{E_INVALIDARG, E_NOTIMPL, ERROR_TIMEOUT, POINT, RECT},
        Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleBitmap, CreateCompatibleDC, DESKTOPHORZRES,
            DESKTOPVERTRES, DeleteDC, DeleteObject, GetDC, GetDIBits, GetDeviceCaps, HALFTONE, ReleaseDC,
            SRCCOPY, SelectObject, SetStretchBltMode, StretchBlt,
        },
        Media::MediaFoundation::{
            CODECAPI_AVEncCommonMeanBitRate, CODECAPI_AVEncMPVGOPSize, CODECAPI_AVEncVideoForceKeyFrame,
            CODECAPI_AVLowLatencyMode, ICodecAPI, IMFActivate, IMFAsyncCallback, IMFAsyncCallback_Impl,
            IMFAsyncResult, IMFAttributes, IMFMediaBuffer, IMFMediaEventGenerator, IMFMediaType, IMFSample,
            IMFTransform, MEError, METransformHaveOutput, METransformNeedInput, MF_E_NOTACCEPTING,
            MF_E_OPERATION_CANCELLED, MF_E_TRANSFORM_NEED_MORE_INPUT, MF_E_TRANSFORM_STREAM_CHANGE,
            MF_LOW_LATENCY, MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE,
            MF_MT_MAJOR_TYPE, MF_MT_MPEG_SEQUENCE_HEADER, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE,
            MF_TRANSFORM_ASYNC, MF_TRANSFORM_ASYNC_UNLOCK, MFCreateAttributes, MFCreateMediaType,
            MFCreateMemoryBuffer, MFCreateSample, MFMediaType_Video, MFSampleExtension_CleanPoint,
            MFT_CATEGORY_VIDEO_ENCODER, MFT_ENUM_ADAPTER_LUID, MFT_ENUM_FLAG_ASYNCMFT,
            MFT_ENUM_FLAG_HARDWARE, MFT_ENUM_FLAG_LOCALMFT, MFT_ENUM_FLAG_SORTANDFILTER,
            MFT_ENUM_FLAG_SYNCMFT, MFT_MESSAGE_COMMAND_FLUSH, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
            MFT_MESSAGE_NOTIFY_END_STREAMING, MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_OUTPUT_DATA_BUFFER,
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
    use crate::win::h264::{ensure_mf_started, video_adapters};
    use crate::{core_host, win};

    struct ActiveSession {
        peer: DeviceId,
        stop: Arc<AtomicBool>,
        force_keyframe: Arc<AtomicBool>,
        /// Play on phone: the phone watches a player, and its input is ignored.
        watching: bool,
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

    /// Whether this PC's screen is currently being shared with a phone.
    pub fn is_active() -> bool {
        active_peer().is_some()
    }

    /// Whether this PC's display session supports screen capture for `mirror.capture`.
    ///
    /// Linux needs an H.264 encoder and either X11 or the ScreenCast portal with PipeWire.
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
            let has_x11 = std::env::var_os("DISPLAY").is_some() && x11_api().is_some();
            has_x11 || is_wayland_screencast_available()
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
        // The same phone asking again replaced its own stream: telling it to
        // stop could land after the new one starts and end that.
        stop_session(Some(peer));
        let stop = Arc::new(AtomicBool::new(false));
        let force_keyframe = Arc::new(AtomicBool::new(false));
        {
            let mut guard = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
            *guard = Some(ActiveSession {
                peer,
                stop: Arc::clone(&stop),
                force_keyframe: Arc::clone(&force_keyframe),
                watching: options.watch.is_some(),
            });
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
        stop_session(None);
    }

    /// Stops the active session, telling its phone unless it's `replaced_by`.
    fn stop_session(replaced_by: Option<DeviceId>) {
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
            if let Some(node) = core_host::node()
                && replaced_by != Some(peer)
            {
                node.set_pc_mirror_session_allowed(peer, false);
                core_host::spawn(async move {
                    node.mirror_stop(peer, nectarlink_core::MIRROR_SCREEN).await;
                });
            }
            core_host::host().hub.changed(crate::state::Changes::REMOTE);
            core_host::refresh_capabilities();
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
        if active_peer() != Some(*peer) || is_watching() {
            return;
        }
        let remote_input_on = core_host::node()
            .and_then(|n| n.device_toggles(*peer).ok())
            .is_some_and(|toggles| toggles.iter().any(|(k, v)| *k == "remote_input" && *v));
        if !remote_input_on {
            return;
        }
        let _ = crate::input_share::enter_remote(*peer);
        match input {
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
        }
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
        configure: impl FnMut(IMFTransform) -> windows::core::Result<ScreenEncoder>,
    ) -> windows::core::Result<ScreenEncoder> {
        let mut candidates = Vec::new();
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
            create_video_encoder(subtype, |transform| {
                Self::from_transform(codec, transform, width, height, fps, bitrate)
            })
        }

        fn from_transform(
            codec: &str,
            transform: IMFTransform,
            width: u32,
            height: u32,
            fps: u32,
            bitrate: u32,
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
                    async_events,
                    pending_input: 0,
                    pending_output: 0,
                    codec_api,
                    width,
                    height,
                    provides_samples,
                    output_size,
                    seq_header,
                    sent_first_frame: false,
                    alternatives: Vec::new(),
                })
            }
        }

        fn next_encoder(&mut self, codec: &str, width: u32, height: u32, fps: u32, bitrate: u32) -> bool {
            let candidates = std::mem::take(&mut self.alternatives);
            match try_video_encoders(candidates, |transform| {
                Self::from_transform(codec, transform, width, height, fps, bitrate)
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
            self.wait_for_async_event(METransformNeedInput.0 as u32, stop)?;
            // SAFETY: Media Foundation sample creation and MFT ProcessInput/ProcessOutput.
            unsafe {
                let sample = make_sample(&nv12[..expected], hns, dur_hns)?;
                let mut out = Vec::new();
                match self.transform.ProcessInput(0, &sample, 0) {
                    Ok(()) => {
                        self.wait_for_async_event(METransformHaveOutput.0 as u32, stop)?;
                        self.drain(&mut out)?;
                    }
                    Err(e) if e.code() == MF_E_NOTACCEPTING => {
                        self.wait_for_async_event(METransformHaveOutput.0 as u32, stop)?;
                        self.drain(&mut out)?;
                        self.wait_for_async_event(METransformNeedInput.0 as u32, stop)?;
                        self.transform.ProcessInput(0, &sample, 0)?;
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
                // SAFETY: MFT output draining with allocated or MFT-provided sample.
                unsafe {
                    let allocated = if self.provides_samples {
                        None
                    } else {
                        let s: IMFSample = MFCreateSample()?;
                        let b: IMFMediaBuffer = MFCreateMemoryBuffer(self.output_size)?;
                        s.AddBuffer(&b)?;
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
                                        && let Ok(hdr_len) = cur_out.GetBlobSize(&MF_MT_MPEG_SEQUENCE_HEADER)
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
                            }
                            // Each METransformHaveOutput event permits one ProcessOutput call.
                            if self.async_events.is_some() {
                                return Ok(());
                            }
                        }
                        Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(()),
                        Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
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
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
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
                    for idx in 0..layer_count {
                        let layer = &bs_info.s_layer_info[idx];
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
                    for idx in 0..layer_count {
                        let layer = &bs_info.s_layer_info[idx];
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

        pub(crate) fn request_keyframe(&self) -> bool {
            match self {
                Self::OpenH264(encoder) => encoder.request_keyframe(),
                #[cfg(linux_ffmpeg_video)]
                Self::Hardware(encoder) => encoder.request_keyframe(),
            }
        }

        pub(crate) fn set_bitrate(&self, bitrate: u32) -> bool {
            match self {
                Self::OpenH264(encoder) => encoder.set_bitrate(bitrate),
                #[cfg(linux_ffmpeg_video)]
                Self::Hardware(encoder) => encoder.set_bitrate(bitrate),
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
            if region.is_none() && GetCursorInfo(&mut ci).is_ok() && (ci.flags.0 & CURSOR_SHOWING.0) != 0 {
                let cx = ((ci.ptScreenPos.x as i64) * (dst_w as i64) / (src_w as i64)) as i32;
                let cy = ((ci.ptScreenPos.y as i64) * (dst_h as i64) / (src_h as i64)) as i32;
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
            && std::process::Command::new("gst-inspect-1.0")
                .args(["--exists", "pipewiresrc"])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
    }

    #[cfg(target_os = "linux")]
    fn is_wayland_screencast_available() -> bool {
        if std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return false;
        }
        static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *AVAILABLE.get_or_init(|| {
            if !pipewire_capture_installed() {
                return false;
            }
            let Ok(conn) = zbus::blocking::Connection::session() else {
                return false;
            };
            let Ok(dbus) = zbus::blocking::fdo::DBusProxy::new(&conn) else {
                return false;
            };
            "org.freedesktop.portal.Desktop"
                .try_into()
                .ok()
                .and_then(|name| dbus.name_has_owner(name).ok())
                .unwrap_or(false)
        })
    }

    #[cfg(target_os = "linux")]
    struct WaylandScreenCaster {
        _conn: zbus::blocking::Connection,
        /// The PipeWire connection the portal opened for us; the stream is
        /// only readable through it.
        remote: std::os::fd::OwnedFd,
        node_id: u32,
        started: Instant,
        src_w: u32,
        src_h: u32,
        dst_w: u32,
        dst_h: u32,
        child: Option<std::process::Child>,
        latest_frame: Arc<Mutex<Option<Vec<u8>>>>,
    }

    #[cfg(target_os = "linux")]
    impl Drop for WaylandScreenCaster {
        fn drop(&mut self) {
            if let Some(mut child) = self.child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn wait_portal_response(
        iter: &mut zbus::blocking::MessageIterator,
        expected_path: &str,
    ) -> Option<std::collections::HashMap<String, zbus::zvariant::OwnedValue>> {
        for msg in iter.by_ref() {
            let msg = msg.ok()?;
            let hdr = msg.header();
            if hdr.interface().map(|i| i.as_str()) == Some("org.freedesktop.portal.Request")
                && hdr.member().map(|m| m.as_str()) == Some("Response")
                && hdr.path().map(|p| p.as_str()) == Some(expected_path)
            {
                let (code, results): (u32, std::collections::HashMap<String, zbus::zvariant::OwnedValue>) =
                    msg.body().deserialize().ok()?;
                return if code == 0 { Some(results) } else { None };
            }
        }
        None
    }

    #[cfg(target_os = "linux")]
    impl WaylandScreenCaster {
        #[allow(clippy::type_complexity)]
        fn negotiate_portal() -> Option<(zbus::blocking::Connection, std::os::fd::OwnedFd, u32, u32, u32)> {
            use std::collections::HashMap;
            use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

            let conn = zbus::blocking::Connection::session().ok()?;
            let rule = zbus::MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .interface("org.freedesktop.portal.Request")
                .and_then(|b| b.member("Response"))
                .map(|b| b.build())
                .ok()?;
            let dbus = zbus::blocking::fdo::DBusProxy::new(&conn).ok()?;
            dbus.add_match_rule(rule).ok()?;
            let mut iter = zbus::blocking::MessageIterator::from(&conn);

            let pid = std::process::id();

            // 1. CreateSession
            let mut create_opts: HashMap<&str, Value<'_>> = HashMap::new();
            let req_tok_1 = format!("nectarlink_sc_req1_{pid}");
            let sess_tok = format!("nectarlink_sc_sess_{pid}");
            create_opts.insert("handle_token", Value::from(req_tok_1.as_str()));
            create_opts.insert("session_handle_token", Value::from(sess_tok.as_str()));

            let reply = conn
                .call_method(
                    Some("org.freedesktop.portal.Desktop"),
                    "/org/freedesktop/portal/desktop",
                    Some("org.freedesktop.portal.ScreenCast"),
                    "CreateSession",
                    &(&create_opts,),
                )
                .ok()?;
            let req_path_1: OwnedObjectPath = reply.body().deserialize().ok()?;
            let create_res = wait_portal_response(&mut iter, req_path_1.as_str())?;
            let session_path_str = create_res.get("session_handle").and_then(|v| {
                String::try_from(v.clone())
                    .ok()
                    .or_else(|| OwnedObjectPath::try_from(v.clone()).ok().map(|p| p.to_string()))
            })?;
            let session_obj = ObjectPath::try_from(session_path_str.as_str()).ok()?;

            // 2. SelectSources (types = 1 MONITOR, cursor_mode = 2 EMBEDDED)
            let mut select_opts: HashMap<&str, Value<'_>> = HashMap::new();
            let req_tok_2 = format!("nectarlink_sc_req2_{pid}");
            select_opts.insert("handle_token", Value::from(req_tok_2.as_str()));
            select_opts.insert("types", Value::from(1u32));
            select_opts.insert("multiple", Value::from(false));
            select_opts.insert("cursor_mode", Value::from(2u32));
            // Ask once while the app runs: the portal hands back a token that
            // restores the same choice without a new dialog.
            select_opts.insert("persist_mode", Value::from(1u32));
            let restore = RESTORE_TOKEN.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if let Some(token) = restore.as_deref() {
                select_opts.insert("restore_token", Value::from(token));
            }

            let reply = conn
                .call_method(
                    Some("org.freedesktop.portal.Desktop"),
                    "/org/freedesktop/portal/desktop",
                    Some("org.freedesktop.portal.ScreenCast"),
                    "SelectSources",
                    &(&session_obj, &select_opts),
                )
                .ok()?;
            let req_path_2: OwnedObjectPath = reply.body().deserialize().ok()?;
            wait_portal_response(&mut iter, req_path_2.as_str())?;

            // 3. Start
            let mut start_opts: HashMap<&str, Value<'_>> = HashMap::new();
            let req_tok_3 = format!("nectarlink_sc_req3_{pid}");
            start_opts.insert("handle_token", Value::from(req_tok_3.as_str()));

            let reply = conn
                .call_method(
                    Some("org.freedesktop.portal.Desktop"),
                    "/org/freedesktop/portal/desktop",
                    Some("org.freedesktop.portal.ScreenCast"),
                    "Start",
                    &(&session_obj, "", &start_opts),
                )
                .ok()?;
            let req_path_3: OwnedObjectPath = reply.body().deserialize().ok()?;
            let start_res = wait_portal_response(&mut iter, req_path_3.as_str())?;
            if let Some(token) = start_res.get("restore_token").and_then(|v| String::try_from(v.clone()).ok())
            {
                *RESTORE_TOKEN.lock().unwrap_or_else(|e| e.into_inner()) = Some(token);
            }

            let streams_val = start_res.get("streams")?;
            let streams: Vec<(u32, HashMap<String, OwnedValue>)> = streams_val.clone().try_into().ok()?;
            let (node_id, props) = streams.into_iter().next()?;
            let (src_w, src_h) = props
                .get("size")
                .and_then(|v| <(i32, i32)>::try_from(v.clone()).ok())
                .filter(|&(w, h)| w > 0 && h > 0)
                .map(|(w, h)| (w as u32, h as u32))
                .unwrap_or((1920, 1080));

            // The stream is only readable through the PipeWire connection
            // the portal opens for this session.
            let empty_opts: HashMap<&str, Value<'_>> = HashMap::new();
            let reply = conn
                .call_method(
                    Some("org.freedesktop.portal.Desktop"),
                    "/org/freedesktop/portal/desktop",
                    Some("org.freedesktop.portal.ScreenCast"),
                    "OpenPipeWireRemote",
                    &(&session_obj, &empty_opts),
                )
                .ok()?;
            let remote: zbus::zvariant::OwnedFd = reply.body().deserialize().ok()?;
            let remote: std::os::fd::OwnedFd = remote.into();

            Some((conn, remote, node_id, src_w, src_h))
        }

        /// The fd number the portal's PipeWire connection gets in the capture process.
        const REMOTE_FD: i32 = 3;

        /// Starts gst-launch reading the stream through `remote` and writing
        /// raw BGRx frames of `dst_w` x `dst_h` to its stdout.
        #[allow(clippy::type_complexity)]
        fn spawn_pipeline(
            remote: &std::os::fd::OwnedFd,
            node_id: u32,
            dst_w: u32,
            dst_h: u32,
        ) -> Option<(std::process::Child, Arc<Mutex<Option<Vec<u8>>>>)> {
            use std::io::Read;
            use std::os::fd::AsRawFd;
            use std::os::unix::process::CommandExt;
            use std::process::{Command, Stdio};

            let raw = remote.as_raw_fd();
            let mut cmd = Command::new("gst-launch-1.0");
            cmd.args(pipeline_args(node_id, dst_w, dst_h))
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
            // SAFETY: runs in the child between fork and exec and only calls
            // async-signal-safe functions (dup2, fcntl) on descriptors we own.
            unsafe {
                cmd.pre_exec(move || {
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
            let mut child = cmd.spawn().ok()?;

            let mut stdout = child.stdout.take()?;
            let latest_frame: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));
            let frame_slot = Arc::clone(&latest_frame);
            let frame_bytes = (dst_w as usize) * (dst_h as usize) * 4;

            std::thread::Builder::new()
                .name("wayland-screencast-reader".into())
                .spawn(move || {
                    let mut buf = vec![0u8; frame_bytes];
                    while stdout.read_exact(&mut buf).is_ok() {
                        for chunk in buf.as_chunks_mut::<4>().0 {
                            chunk[3] = 0xFF;
                        }
                        *frame_slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(buf.clone());
                    }
                })
                .ok()?;

            Some((child, latest_frame))
        }

        fn start(dst_w: u32, dst_h: u32) -> Option<Self> {
            let (conn, remote, node_id, src_w, src_h) = Self::negotiate_portal()?;
            let (child, latest_frame) = Self::spawn_pipeline(&remote, node_id, dst_w, dst_h)?;
            Some(Self {
                _conn: conn,
                remote,
                node_id,
                started: Instant::now(),
                src_w,
                src_h,
                dst_w,
                dst_h,
                child: Some(child),
                latest_frame,
            })
        }

        /// Whether the capture process is still running.
        fn alive(&mut self) -> bool {
            self.child.as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(None)))
        }

        fn capture(&mut self, dst_w: u32, dst_h: u32, bgra: &mut Vec<u8>) -> Option<(u32, u32)> {
            if (self.dst_w, self.dst_h) != (dst_w, dst_h) {
                if let Some(mut old) = self.child.take() {
                    let _ = old.kill();
                    let _ = old.wait();
                }
                let (child, latest_frame) = Self::spawn_pipeline(&self.remote, self.node_id, dst_w, dst_h)?;
                self.started = Instant::now();
                self.dst_w = dst_w;
                self.dst_h = dst_h;
                self.child = Some(child);
                self.latest_frame = latest_frame;
            }

            // The first frame can take a while (the pipeline negotiates
            // formats); after that a frame is late only for a moment.
            let wait = if self.started.elapsed() < Duration::from_secs(3) {
                Duration::from_secs(3).saturating_sub(self.started.elapsed())
            } else {
                Duration::from_millis(250)
            };
            // Its last frame would otherwise show as a live screen for good.
            if !self.alive() {
                return None;
            }
            let deadline = Instant::now() + wait;
            loop {
                if let Some(frame) = self.latest_frame.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                    bgra.clear();
                    bgra.extend_from_slice(frame);
                    return Some((self.src_w, self.src_h));
                }
                if Instant::now() >= deadline {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }

    #[cfg(target_os = "linux")]
    static WAYLAND_CASTER: Mutex<Option<WaylandScreenCaster>> = Mutex::new(None);

    /// The ScreenCast portal's restore token, so later sessions in this run
    /// reuse the screen the user chose instead of asking again.
    #[cfg(target_os = "linux")]
    static RESTORE_TOKEN: Mutex<Option<String>> = Mutex::new(None);

    /// gst-launch arguments: read stream `node_id` through the portal's
    /// PipeWire connection (fd 3) and write raw BGRx frames to stdout.
    #[cfg(target_os = "linux")]
    pub(crate) fn pipeline_args(node_id: u32, dst_w: u32, dst_h: u32) -> Vec<String> {
        [
            "-q".to_owned(),
            "pipewiresrc".to_owned(),
            format!("fd={}", WaylandScreenCaster::REMOTE_FD),
            format!("path={node_id}"),
            "do-timestamp=true".to_owned(),
            "!".to_owned(),
            "videoconvert".to_owned(),
            "!".to_owned(),
            "videoscale".to_owned(),
            "!".to_owned(),
            format!("video/x-raw,format=BGRx,width={dst_w},height={dst_h}"),
            "!".to_owned(),
            "fdsink".to_owned(),
            "fd=1".to_owned(),
        ]
        .into()
    }

    #[cfg(target_os = "linux")]
    fn capture_screen_wayland(dst_w: u32, dst_h: u32, bgra: &mut Vec<u8>) -> Option<(u32, u32)> {
        if !is_wayland_screencast_available() {
            return None;
        }
        let mut guard = WAYLAND_CASTER.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            *guard = WaylandScreenCaster::start(dst_w, dst_h);
        }
        let caster = guard.as_mut()?;
        let res = caster.capture(dst_w, dst_h, bgra);
        // A late frame is skipped; only a stopped capture ends the session
        // (starting a new one would ask the user again).
        if res.is_none() && !caster.alive() {
            *guard = None;
        }
        res
    }

    #[cfg(target_os = "linux")]
    fn stop_wayland_screencast() {
        *WAYLAND_CASTER.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    #[cfg(target_os = "linux")]
    fn capture_screen(
        region: Option<crate::win::browser::RECT>,
        dst_w: u32,
        dst_h: u32,
        bgra: &mut Vec<u8>,
    ) -> Option<(u32, u32)> {
        if region.is_none()
            && std::env::var_os("WAYLAND_DISPLAY").is_some()
            && let Some(dims) = capture_screen_wayland(dst_w, dst_h, bgra)
        {
            return Some(dims);
        }
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
        let Ok(sender) = runtime.block_on(node.mirror_open(peer)) else {
            tracing::warn!(%peer, "couldn't open PC mirror stream to phone");
            cleanup_session(&stop);
            return;
        };

        // Give the phone's PcMirrorView TextureView and MediaCodec a brief moment to
        // mount before sending the initial MirrorConfig and IDR keyframe.
        std::thread::sleep(Duration::from_millis(120));

        let fps = options.fps.clamp(10, 60);
        let max_bitrate = options.bitrate.clamp(500_000, 20_000_000);
        // Lowered when the link can't keep up (frames dropped to hold the delay down),
        // raised back once it has been clean for a while.
        let mut bitrate = max_bitrate;
        let frame_interval = Duration::from_micros(1_000_000 / u64::from(fps));
        let dur_hns = (10_000_000 / u64::from(fps)) as i64;

        let mut codec = if node.mirror_codecs(peer).is_ok_and(|codecs| codecs.iter().any(|c| c == "h265")) {
            "h265".to_owned()
        } else {
            "h264".to_owned()
        };

        let mut bgra = Vec::new();
        let mut nv12 = Vec::new();
        let mut stream_dims: Option<(u32, u32)> = None;
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
            tracing::info!(%peer, fps, bitrate, "streaming a player to the phone");
        } else {
            tracing::info!(%peer, fps, bitrate, "sharing this PC's screen");
        }

        while !stop.load(Ordering::Relaxed) {
            let tick_start = Instant::now();

            let place = watched.as_ref().and_then(crate::play_on_phone::Watched::place);
            if watched.is_some() && place.is_none() && started.elapsed() < FIND_WINDOW {
                // Not found yet: nothing to show.
                std::thread::sleep(frame_interval);
                continue;
            }
            if watched.is_some() && place.is_none() {
                // The player's window can't be found. Not the screen instead:
                // that would show the phone whatever else is on it.
                tracing::info!(%peer, "can't find the player's window; stopping");
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
                    window_capture = win::window_capture::WindowCapture::new(place.window())
                        .inspect_err(
                            |e| tracing::info!(error = %e, "can't capture the window; capturing the screen"),
                        )
                        .ok();
                }
            }

            let (dst_w, dst_h) = match stream_dims {
                Some(dims) => dims,
                None => {
                    let (src_w, src_h) = primary_screen_dimensions();
                    let dims = target_dimensions(src_w, src_h, options.max_size);
                    stream_dims = Some(dims);
                    dims
                }
            };

            // A keyframe on request comes from the running encoder. Rebuilding it (a new
            // stream config plus a full keyframe) is the fallback for encoders that can't.
            let requested_kf = force_keyframe.swap(false, Ordering::SeqCst);
            let keyframe_in_place =
                requested_kf && encoder.as_ref().is_some_and(ScreenEncoder::request_keyframe);
            let need_reinit = encoder.is_none()
                || (requested_kf
                    && !keyframe_in_place
                    && last_reinit.elapsed() >= Duration::from_millis(1400));

            if need_reinit {
                match new_pc_encoder_with_fallback(&mut codec, dst_w, dst_h, fps, bitrate) {
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
                        encoder = Some(enc);
                        last_reinit = Instant::now();
                        stats.rebuilds += 1;
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, codec, "PC mirror encoder init failed");
                        break;
                    }
                }
            }

            let captured = match (place, window_capture.as_mut()) {
                (Some(place), Some(capture)) => capture.capture(place.in_window, dst_w, dst_h, &mut bgra),
                (Some(place), None) => {
                    capture_screen(Some(place.on_screen), dst_w, dst_h, &mut bgra).is_some()
                }
                (None, _) => capture_screen(None, dst_w, dst_h, &mut bgra).is_some(),
            };
            if captured && let Some(enc) = encoder.as_mut() {
                bgra_to_nv12(&bgra, dst_w as usize, dst_h as usize, &mut nv12);
                let time_us = started.elapsed().as_micros() as u64;
                let hns = (time_us * 10) as i64;
                match enc.encode(&nv12, hns, dur_hns, &stop) {
                    Ok(packets) => {
                        let mut send_failed = false;
                        for (is_keyframe, bytes) in packets {
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
                        #[cfg(any(windows, linux_ffmpeg_video))]
                        if enc.next_encoder(&codec, dst_w, dst_h, fps, bitrate) {
                            tracing::warn!(error = %e, codec, "Video encoding failed; trying another encoder");
                            stats.rebuilds += 1;
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
                    %peer,
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

        // Only when no newer session took over: the screencast is shared.
        #[cfg(target_os = "linux")]
        if ACTIVE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_none_or(|s| Arc::ptr_eq(&s.stop, &stop))
        {
            stop_wayland_screencast();
        }
        tracing::info!(%peer, "stopped sharing this PC's screen");
        crate::input_share::leave_remote();
        sender.close();
        cleanup_session(&stop);
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

    fn cleanup_session(stop: &Arc<AtomicBool>) {
        let removed = {
            let mut guard = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
            if guard.as_ref().is_some_and(|s| Arc::ptr_eq(&s.stop, stop)) {
                guard.take().is_some()
            } else {
                false
            }
        };
        if removed {
            core_host::host().hub.changed(crate::state::Changes::REMOTE);
            core_host::refresh_capabilities();
        }
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
                ScreenEncoder::from_transform("h264", transform, 320, 240, 30, 2_000_000)
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
            let args = pipeline_args(57, 1280, 720);
            assert_eq!(args[1], "pipewiresrc");
            assert!(args.contains(&"fd=3".to_owned()), "the portal's PipeWire connection, as fd 3");
            assert!(args.contains(&"path=57".to_owned()));
            assert!(args.contains(&"video/x-raw,format=BGRx,width=1280,height=720".to_owned()));
            assert_eq!(args.last().map(String::as_str), Some("fd=1"));
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
