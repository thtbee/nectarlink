// SPDX-License-Identifier: GPL-3.0-or-later
//! Reverse Mirroring ("Your PC in Your Pocket"): captures the PC's primary
//! display, encodes it to low-latency H.264 via Windows Media Foundation, and
//! streams it over `mirror` / `video` (`session = 0`) to a paired phone, while
//! injecting touch, scroll, key, and text input back into Windows (`docs/PLAN.md` §8.6).

#![allow(unsafe_code)]

#[cfg(windows)]
use std::mem::{ManuallyDrop, size_of};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
#[cfg(any(windows, target_os = "linux"))]
use std::time::{Duration, Instant};

use nectarlink_core::{DeviceId, KeyMod, MirrorInput, MirrorStart, mirror_keys};
#[cfg(any(windows, target_os = "linux"))]
use nectarlink_core::{MirrorConfig, MirrorSend, PacketKind};
#[cfg(windows)]
use windows::Win32::{
    Foundation::POINT,
    Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleBitmap, CreateCompatibleDC, DESKTOPHORZRES,
        DESKTOPVERTRES, DeleteDC, DeleteObject, GetDC, GetDIBits, GetDeviceCaps, HALFTONE, ReleaseDC,
        SRCCOPY, SelectObject, SetStretchBltMode, StretchBlt,
    },
    Media::MediaFoundation::{
        CODECAPI_AVEncCommonMeanBitRate, CODECAPI_AVEncMPVGOPSize, CODECAPI_AVEncVideoForceKeyFrame,
        CODECAPI_AVLowLatencyMode, ICodecAPI, IMFMediaBuffer, IMFMediaType, IMFSample, IMFTransform,
        MF_E_NOTACCEPTING, MF_E_TRANSFORM_NEED_MORE_INPUT, MF_E_TRANSFORM_STREAM_CHANGE, MF_LOW_LATENCY,
        MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE,
        MF_MT_MPEG_SEQUENCE_HEADER, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE, MFCreateMediaType,
        MFCreateMemoryBuffer, MFCreateSample, MFMediaType_Video, MFSampleExtension_CleanPoint,
        MFT_CATEGORY_VIDEO_ENCODER, MFT_MESSAGE_COMMAND_FLUSH, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
        MFT_MESSAGE_NOTIFY_END_STREAMING, MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_OUTPUT_DATA_BUFFER,
        MFT_OUTPUT_STREAM_PROVIDES_SAMPLES, MFVideoFormat_H264, MFVideoFormat_NV12,
        MFVideoInterlace_Progressive,
    },
    System::Variant::VARIANT,
    UI::WindowsAndMessaging::{
        CURSOR_SHOWING, CURSORINFO, DI_NORMAL, DrawIconEx, GetCursorInfo, GetSystemMetrics, SM_CXSCREEN,
        SM_CYSCREEN,
    },
};

#[cfg(windows)]
use crate::win::h264::{create_mft, ensure_mf_started};
use crate::{core_host, win};

struct ActiveSession {
    peer: DeviceId,
    stop: Arc<AtomicBool>,
    force_keyframe: Arc<AtomicBool>,
}

static ACTIVE: Mutex<Option<ActiveSession>> = Mutex::new(None);

/// The lowest bitrate the stream backs off to on a slow link.
#[cfg(any(windows, target_os = "linux"))]
const MIN_BITRATE: u32 = 600_000;

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

/// Starts or replaces the active PC-to-phone screen mirror stream for `peer`.
pub fn start(peer: DeviceId, options: MirrorStart) {
    stop_all();
    let stop = Arc::new(AtomicBool::new(false));
    let force_keyframe = Arc::new(AtomicBool::new(false));
    {
        let mut guard = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some(ActiveSession {
            peer,
            stop: Arc::clone(&stop),
            force_keyframe: Arc::clone(&force_keyframe),
        });
    }
    core_host::host().hub.changed(crate::state::Changes::REMOTE);
    core_host::refresh_capabilities();

    std::thread::Builder::new()
        .name("nectarlink-pc-mirror".into())
        .spawn(move || run_capture_loop(peer, options, stop, force_keyframe))
        .ok();
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
        if let Some(node) = core_host::node() {
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
    if active_peer() != Some(*peer) {
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
pub(crate) struct H264Encoder {
    transform: IMFTransform,
    /// The encoder's settings interface, when it has one: keyframes on demand and live
    /// bitrate changes without rebuilding the encoder.
    codec_api: Option<ICodecAPI>,
    width: u32,
    height: u32,
    provides_samples: bool,
    output_size: u32,
    seq_header: Vec<u8>,
    sent_first_frame: bool,
}

#[cfg(windows)]
impl Drop for H264Encoder {
    fn drop(&mut self) {
        // SAFETY: flushes and ends streaming on the MFT before drop.
        unsafe {
            let _ = self.transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
            let _ = self.transform.ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0);
        }
    }
}

#[cfg(windows)]
impl H264Encoder {
    pub(crate) fn new(width: u32, height: u32, fps: u32, bitrate: u32) -> windows::core::Result<Self> {
        ensure_mf_started()?;
        // SAFETY: Media Foundation COM calls with valid parameters.
        unsafe {
            let transform = create_mft(MFT_CATEGORY_VIDEO_ENCODER, MFVideoFormat_NV12, MFVideoFormat_H264)?;
            if let Ok(attrs) = transform.GetAttributes() {
                let _ = attrs.SetUINT32(&MF_LOW_LATENCY, 1);
            }

            let out_type: IMFMediaType = MFCreateMediaType()?;
            out_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            out_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
            let _ = out_type.SetUINT32(
                &windows::Win32::Media::MediaFoundation::MF_MT_MPEG2_PROFILE,
                windows::Win32::Media::MediaFoundation::eAVEncH264VProfile_Base.0 as u32,
            );
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

            // A keyframe every two seconds keeps late joiners and lost packets recoverable
            // without rebuilding the encoder, and low-latency mode stops it holding frames back.
            let codec_api: Option<ICodecAPI> = windows::core::Interface::cast(&transform).ok();
            if let Some(api) = codec_api.as_ref() {
                let _ = api.SetValue(&CODECAPI_AVLowLatencyMode, &VARIANT::from(true));
                let _ = api.SetValue(&CODECAPI_AVEncMPVGOPSize, &VARIANT::from(fps.max(1) * 2));
            }

            let _ = transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0);
            let _ = transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0);

            Ok(Self {
                transform,
                codec_api,
                width,
                height,
                provides_samples,
                output_size,
                seq_header,
                sent_first_frame: false,
            })
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
    /// Annex-B H.264 access units `(is_keyframe, bytes)`.
    pub(crate) fn encode(
        &mut self,
        nv12: &[u8],
        hns: i64,
        dur_hns: i64,
    ) -> windows::core::Result<Vec<(bool, Vec<u8>)>> {
        let expected = (self.width as usize) * (self.height as usize) * 3 / 2;
        if nv12.len() < expected {
            return Ok(Vec::new());
        }
        // SAFETY: Media Foundation sample creation and MFT ProcessInput/ProcessOutput.
        unsafe {
            let sample = make_sample(&nv12[..expected], hns, dur_hns)?;
            let mut out = Vec::new();
            match self.transform.ProcessInput(0, &sample, 0) {
                Ok(()) => self.drain(&mut out)?,
                Err(e) if e.code() == MF_E_NOTACCEPTING => {
                    self.drain(&mut out)?;
                    self.transform.ProcessInput(0, &sample, 0)?;
                    self.drain(&mut out)?;
                }
                Err(e) => return Err(e),
            }
            Ok(out)
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
                            let is_idr = !self.sent_first_frame || clean || has_h264_idr(slice);
                            let mut bytes = Vec::new();
                            if is_idr && !has_h264_sps(slice) {
                                if self.seq_header.is_empty()
                                    && let Ok(cur_out) = self.transform.GetOutputCurrentType(0)
                                    && let Ok(hdr_len) = cur_out.GetBlobSize(&MF_MT_MPEG_SEQUENCE_HEADER)
                                    && hdr_len > 0
                                {
                                    let mut hdr = vec![0u8; hdr_len as usize];
                                    if cur_out.GetBlob(&MF_MT_MPEG_SEQUENCE_HEADER, &mut hdr, None).is_ok() {
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
}

#[cfg(target_os = "linux")]
#[repr(C)]
struct SFrameBSInfo {
    i_layer_num: std::ffi::c_int,
    s_layer_info: [SLayerBSInfo; 128],
    e_frame_type: std::ffi::c_int, // 1 = IDR, 2 = I, 3 = P, 4 = Skip
    i_frame_size_in_bytes: std::ffi::c_int,
    ui_time_stamp: i64,
}

#[cfg(target_os = "linux")]
#[repr(C)]
struct SBitrateInfo {
    i_layer: std::ffi::c_int, // 4 = SPATIAL_LAYER_ALL, 0 = SPATIAL_LAYER_0
    i_bitrate: std::ffi::c_int,
}

#[cfg(target_os = "linux")]
struct OpenH264EncApi {
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
        let candidates = [c"libopenh264.so.7", c"libopenh264.so.6", c"libopenh264.so"];
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
        Some(OpenH264EncApi {
            create_encoder: dl_sym(lib, c"WelsCreateSVCEncoder")?,
            destroy_encoder: dl_sym(lib, c"WelsDestroySVCEncoder")?,
        })
    })
    .as_ref()
}

#[cfg(target_os = "linux")]
pub(crate) struct H264Encoder {
    encoder: *mut std::ffi::c_void,
    width: u32,
    height: u32,
    i420_buf: Vec<u8>,
    seq_header: Vec<u8>,
    sent_first_frame: bool,
}

#[cfg(target_os = "linux")]
impl Drop for H264Encoder {
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
impl H264Encoder {
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
    ) -> Result<Vec<(bool, Vec<u8>)>, String> {
        let w = self.width as usize;
        let h = self.height as usize;
        let y_size = w * h;
        let uv_size = y_size / 2;
        let expected = y_size + uv_size;
        if nv12.len() < expected || self.encoder.is_null() {
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
            let y_ptr = self.i420_buf.as_mut_ptr();
            let u_ptr = y_ptr.add(y_size);
            let v_ptr = y_ptr.add(y_size + quarter);
            let pic = SSourcePicture {
                i_color_format: 23, // videoFormatI420
                i_stride: [w as std::ffi::c_int, (w / 2) as std::ffi::c_int, (w / 2) as std::ffi::c_int, 0],
                p_data: [y_ptr, u_ptr, v_ptr, std::ptr::null_mut()],
                i_pic_width: w as std::ffi::c_int,
                i_pic_height: h as std::ffi::c_int,
                ui_time_stamp: hns / 10_000,
            };

            let mut bs_info: SFrameBSInfo = std::mem::zeroed();
            let vtable = *(self.encoder as *const *const usize);
            let encode_frame: unsafe extern "C" fn(
                *mut std::ffi::c_void,
                *const SSourcePicture,
                *mut SFrameBSInfo,
            ) -> std::ffi::c_int = std::mem::transmute(*vtable.add(4));

            let rc = encode_frame(self.encoder, &pic, &mut bs_info);
            if rc != 0 {
                return Err(format!("OpenH264 EncodeFrame failed: {rc}"));
            }
            if bs_info.e_frame_type == 4 || bs_info.i_layer_num <= 0 {
                return Ok(Vec::new());
            }

            let layer_count = (bs_info.i_layer_num as usize).min(bs_info.s_layer_info.len());
            let mut raw = Vec::new();
            for idx in 0..layer_count {
                let layer = &bs_info.s_layer_info[idx];
                if layer.p_bs_buf.is_null() || layer.p_nal_length_in_byte.is_null() || layer.i_nal_count <= 0
                {
                    continue;
                }
                let nal_lens =
                    std::slice::from_raw_parts(layer.p_nal_length_in_byte, layer.i_nal_count as usize);
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

            if raw.is_empty() {
                return Ok(Vec::new());
            }

            let is_idr = !self.sent_first_frame || bs_info.e_frame_type == 1 || has_h264_idr(&raw);
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

/// Captures the primary desktop monitor into `bgra` scaled to `(dst_w, dst_h)`
/// and returns the current physical screen dimensions `(src_w, src_h)`.
#[cfg(windows)]
fn capture_primary_screen(dst_w: u32, dst_h: u32, bgra: &mut Vec<u8>) -> Option<(u32, u32)> {
    // SAFETY: Win32 GDI device context and bitmap calls with balanced cleanup.
    unsafe {
        let hdc_screen = GetDC(None);
        if hdc_screen.is_invalid() {
            return None;
        }
        let phys_w = GetDeviceCaps(Some(hdc_screen), DESKTOPHORZRES);
        let phys_h = GetDeviceCaps(Some(hdc_screen), DESKTOPVERTRES);
        let src_w = if phys_w > 0 { phys_w } else { GetSystemMetrics(SM_CXSCREEN) }.max(1) as u32;
        let src_h = if phys_h > 0 { phys_h } else { GetSystemMetrics(SM_CYSCREEN) }.max(1) as u32;

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
            0,
            0,
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
        if GetCursorInfo(&mut ci).is_ok() && (ci.flags.0 & CURSOR_SHOWING.0) != 0 {
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
        if (api.get_window_attributes)(dpy, root, &mut attrs) != 0 && attrs.width > 0 && attrs.height > 0 {
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
fn capture_primary_screen(dst_w: u32, dst_h: u32, bgra: &mut Vec<u8>) -> Option<(u32, u32)> {
    with_x11_display(|api, dpy| unsafe {
        let root = (api.default_root_window)(dpy);
        let mut attrs: XWindowAttributes = std::mem::zeroed();
        if (api.get_window_attributes)(dpy, root, &mut attrs) == 0 || attrs.width <= 0 || attrs.height <= 0 {
            return None;
        }
        let src_w = attrs.width as u32;
        let src_h = attrs.height as u32;

        let img = (api.get_image)(
            dpy,
            root,
            0,
            0,
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
                let sx =
                    ((u64::from(x) * u64::from(src_w)) / u64::from(dst_w)).min(u64::from(src_w - 1)) as usize;
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
        if (api.query_pointer)(
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

    let mut bgra = Vec::new();
    let mut nv12 = Vec::new();
    let mut stream_dims: Option<(u32, u32)> = None;
    let mut encoder: Option<H264Encoder> = None;
    let started = Instant::now();
    let mut last_reinit = Instant::now() - Duration::from_secs(5);
    let mut last_drop: Option<Instant> = None;
    let mut last_rate_change = Instant::now();
    let mut stats = Stats::default();
    let mut stats_since = Instant::now();
    tracing::info!(%peer, fps, bitrate, "sharing this PC's screen");

    while !stop.load(Ordering::Relaxed) {
        let tick_start = Instant::now();

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
        let keyframe_in_place = requested_kf && encoder.as_ref().is_some_and(H264Encoder::request_keyframe);
        let need_reinit = encoder.is_none()
            || (requested_kf && !keyframe_in_place && last_reinit.elapsed() >= Duration::from_millis(1400));

        if need_reinit {
            match H264Encoder::new(dst_w, dst_h, fps, bitrate) {
                Ok(enc) => {
                    let cfg = MirrorConfig {
                        codec: nectarlink_core::MIRROR_H264.into(),
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
                    tracing::warn!(error = %e, "PC mirror H.264 encoder init failed");
                    break;
                }
            }
        }

        if capture_primary_screen(dst_w, dst_h, &mut bgra).is_some()
            && let Some(enc) = encoder.as_mut()
        {
            bgra_to_nv12(&bgra, dst_w as usize, dst_h as usize, &mut nv12);
            let time_us = started.elapsed().as_micros() as u64;
            let hns = (time_us * 10) as i64;
            match enc.encode(&nv12, hns, dur_hns) {
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
                                // The link is slower than the stream: ask for less, at most every 2 s.
                                if last_rate_change.elapsed() >= Duration::from_secs(2) {
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
                    // Clean for 10 s: step the bitrate back up toward what the phone asked for.
                    let clean = last_drop.is_none_or(|t| t.elapsed() >= Duration::from_secs(10));
                    if clean && bitrate < max_bitrate && last_rate_change.elapsed() >= Duration::from_secs(10)
                    {
                        let higher = (bitrate / 4 * 5).min(max_bitrate);
                        if enc.set_bitrate(higher) {
                            bitrate = higher;
                            last_rate_change = Instant::now();
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "PC mirror frame encode error; rebuilding the encoder");
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

    tracing::info!(%peer, "stopped sharing this PC's screen");
    crate::input_share::leave_remote();
    sender.close();
    cleanup_session(&stop);
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

    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn encodes_nv12_frames_and_decodes_back_with_mft() {
        let (w, h) = target_dimensions(1920, 1080, 1280);
        assert_eq!(w % 16, 0);
        assert_eq!(h % 16, 0);

        let mut enc = H264Encoder::new(w, h, 30, 4_000_000).expect("H264Encoder MFT");
        let mut dec = crate::win::h264::Decoder::new_for_codec("h264").expect("H264 Decoder MFT");

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
        for i in 0..8 {
            let packets = enc.encode(&nv12, i * 333_333, 333_333).expect("encode frame");
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
        assert!(decoded_any, "decoder must reconstruct at least one frame from H264Encoder");
    }

    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn keyframes_come_on_request_without_rebuilding_the_encoder() {
        let (w, h) = target_dimensions(1280, 720, 1280);
        let mut enc = H264Encoder::new(w, h, 30, 2_000_000).expect("H264Encoder MFT");
        let mut nv12 = Vec::new();
        bgra_to_nv12(&vec![128u8; (w as usize) * (h as usize) * 4], w as usize, h as usize, &mut nv12);

        let keyframes_after = |enc: &mut H264Encoder, start: i64, count: i64| {
            (start..start + count)
                .flat_map(|i| enc.encode(&nv12, i * 333_333, 333_333).expect("encode frame"))
                .filter(|(keyframe, data)| *keyframe && has_h264_idr(data))
                .count()
        };
        // The first frame is a keyframe; the next few (well inside the 2 s GOP) are not.
        assert_eq!(keyframes_after(&mut enc, 0, 1), 1, "the stream starts with a keyframe");
        assert_eq!(keyframes_after(&mut enc, 1, 5), 0, "no keyframes mid-GOP unprompted");
        assert!(enc.request_keyframe(), "the Windows H.264 encoder takes keyframe requests");
        assert_eq!(keyframes_after(&mut enc, 6, 2), 1, "a requested keyframe arrives promptly");
        assert!(enc.set_bitrate(1_000_000), "the bitrate changes in place");
    }
}
