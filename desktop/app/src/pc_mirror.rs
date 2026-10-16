// SPDX-License-Identifier: GPL-3.0-or-later
//! Reverse Mirroring ("Your PC in Your Pocket"): captures the PC's primary
//! display, encodes it to low-latency H.264 via Windows Media Foundation, and
//! streams it over `mirror` / `video` (`session = 0`) to a paired phone, while
//! injecting touch, scroll, key, and text input back into Windows (`docs/PLAN.md` §8.6).

#![allow(unsafe_code)]

#[cfg(windows)]
use std::{
    mem::{ManuallyDrop, size_of},
    time::{Duration, Instant},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

#[cfg(windows)]
use nectarlink_core::{MirrorConfig, MirrorSend, PacketKind};
use nectarlink_core::{DeviceId, KeyMod, MirrorInput, MirrorStart, mirror_keys};
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
#[cfg(windows)]
const MIN_BITRATE: u32 = 600_000;

/// What a sharing session sent in the last stats window.
#[cfg(windows)]
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

#[cfg(windows)]
fn has_h264_idr(data: &[u8]) -> bool {
    data.windows(4).any(|w| w[..3] == [0, 0, 1] && (w[3] & 0x1f) == 5)
}

#[cfg(windows)]
fn has_h264_sps(data: &[u8]) -> bool {
    data.windows(4).any(|w| w[..3] == [0, 0, 1] && (w[3] & 0x1f) == 7)
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
fn run_capture_loop(
    peer: DeviceId,
    _options: MirrorStart,
    stop: Arc<AtomicBool>,
    _force_keyframe: Arc<AtomicBool>,
) {
    let _ = (target_dimensions(1920, 1080, 1280), bgra_to_nv12 as fn(&[u8], usize, usize, &mut Vec<u8>));
    tracing::warn!(%peer, "PC screen mirroring capture is not yet available on Linux");
    crate::input_share::leave_remote();
    cleanup_session(&stop);
}

#[cfg(windows)]
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
                let (src_w, src_h) = unsafe {
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
                };
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

    #[cfg(windows)]
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

    #[cfg(windows)]
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
