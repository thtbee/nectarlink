// SPDX-License-Identifier: GPL-3.0-or-later
//! Video decoding with Windows' own decoders (Media Foundation's H.264
//! MFT, licensed with Windows, and the HEVC and AV1 video extensions where
//! installed), in low-latency mode: each access unit in, its picture out
//! right away, as NV12.

use std::{mem::ManuallyDrop, sync::Mutex, time::Duration};

use windows::{
    Win32::{
        Foundation::E_FAIL,
        Graphics::{
            Direct3D::D3D_DRIVER_TYPE_UNKNOWN,
            Direct3D11::{
                D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                D3D11_DECODER_PROFILE_AV1_VLD_PROFILE0, D3D11_DECODER_PROFILE_H264_VLD_NOFGT,
                D3D11_DECODER_PROFILE_HEVC_VLD_MAIN, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE,
                D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING, D3D11CreateDevice,
                ID3D11Device, ID3D11Multithread, ID3D11Texture2D, ID3D11VideoDevice,
            },
            Dxgi::{
                Common::{DXGI_FORMAT_NV12, DXGI_SAMPLE_DESC},
                CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_ERROR_NOT_FOUND, IDXGIAdapter1,
                IDXGIFactory1,
            },
        },
        Media::MediaFoundation::{
            CLSID_MSH264DecoderMFT, CODECAPI_AVDecNumWorkerThreads, IMF2DBuffer, IMFActivate, IMFDXGIBuffer,
            IMFDXGIDeviceManager, IMFMediaType, IMFSample, IMFTransform, MF_E_NOTACCEPTING,
            MF_E_TRANSFORM_NEED_MORE_INPUT, MF_E_TRANSFORM_STREAM_CHANGE, MF_LOW_LATENCY, MF_MT_FRAME_SIZE,
            MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_MT_VIDEO_NOMINAL_RANGE, MF_MT_YUV_MATRIX, MF_SA_D3D11_AWARE,
            MF_VERSION, MFCreateDXGIDeviceManager, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample,
            MFMediaType_Video, MFNominalRange_0_255, MFSTARTUP_NOSOCKET, MFStartup,
            MFT_CATEGORY_VIDEO_DECODER, MFT_ENUM_FLAG_LOCALMFT, MFT_ENUM_FLAG_SORTANDFILTER,
            MFT_ENUM_FLAG_SYNCMFT, MFT_MESSAGE_COMMAND_FLUSH, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
            MFT_MESSAGE_NOTIFY_END_STREAMING, MFT_MESSAGE_NOTIFY_START_OF_STREAM,
            MFT_MESSAGE_SET_D3D_MANAGER, MFT_OUTPUT_DATA_BUFFER, MFT_OUTPUT_STREAM_PROVIDES_SAMPLES,
            MFT_REGISTER_TYPE_INFO, MFTEnumEx, MFVideoFormat_AV1, MFVideoFormat_H264, MFVideoFormat_HEVC,
            MFVideoFormat_NV12, MFVideoTransferMatrix_BT601,
        },
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
        },
    },
    core::Interface,
};

/// A decoded picture: NV12, `stride` bytes per row, the chroma plane after
/// `plane_rows` rows of luma (the decoder pads the height).
#[derive(Debug)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub plane_rows: usize,
    pub nv12: Vec<u8>,
    /// How its colors turn into RGB, as the stream says.
    pub colors: Colors,
}

/// A decoder-owned sample retained until the consumer finishes using its GPU
/// surface. Materializing CPU pixels is optional and explicit.
#[derive(Debug)]
pub struct DecodedFrame {
    pub sample: IMFSample,
    pub width: u32,
    pub height: u32,
    pub colors: Colors,
}
impl DecodedFrame {
    pub fn gpu_surface(&self) -> windows::core::Result<Option<(ID3D11Texture2D, u32)>> {
        unsafe {
            let buffer = self.sample.GetBufferByIndex(0)?;
            let Ok(dxgi) = buffer.cast::<IMFDXGIBuffer>() else { return Ok(None) };
            let mut texture: Option<ID3D11Texture2D> = None;
            dxgi.GetResource(&ID3D11Texture2D::IID, std::ptr::from_mut(&mut texture).cast())?;
            texture.map(|t| dxgi.GetSubresourceIndex().map(|i| (t, i))).transpose()
        }
    }
}

/// The YUV-to-RGB formula a stream uses (its VUI, as the decoder reports
/// it). Phones are asked for BT.709 limited range, but not all comply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Colors {
    pub bt601: bool,
    pub full_range: bool,
}

impl Colors {
    /// Fixed-point (×256) coefficients: luma scale, then V→R, U→G, V→G, U→B.
    fn coefficients(self) -> (i32, i32, i32, i32, i32) {
        let luma = if self.full_range { 256 } else { 298 };
        let chroma = match (self.bt601, self.full_range) {
            (false, false) => (459, 55, 136, 541),
            (false, true) => (403, 48, 120, 475),
            (true, false) => (409, 100, 208, 516),
            (true, true) => (359, 88, 183, 454),
        };
        (luma, chroma.0, chroma.1, chroma.2, chroma.3)
    }
}

/// Initializes COM on the calling thread and starts Media Foundation once for
/// the process so concurrent decoders and transcoders never tear down Media
/// Foundation's platform work queues under each other.
pub(crate) fn ensure_mf_started() -> windows::core::Result<()> {
    static MF_STARTED: Mutex<bool> = Mutex::new(false);
    // SAFETY: plain COM and Media Foundation startup calls.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let mut started = MF_STARTED.lock().unwrap_or_else(|e| e.into_inner());
        if !*started {
            MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET)?;
            *started = true;
        }
        Ok(())
    }
}

/// Enumerates and activates a synchronous local Media Foundation transform for
/// `category` matching `input_subtype` -> `output_subtype`.
pub(crate) fn create_mft(
    category: windows::core::GUID,
    input_subtype: windows::core::GUID,
    output_subtype: windows::core::GUID,
) -> windows::core::Result<IMFTransform> {
    let input_info = MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: input_subtype };
    let output_info =
        MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: output_subtype };
    let mut activates: *mut Option<IMFActivate> = std::ptr::null_mut();
    let mut count = 0u32;
    // SAFETY: MFTEnumEx allocates the `activates` array with CoTaskMemAlloc; we take ownership and free it.
    unsafe {
        MFTEnumEx(
            category,
            MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_LOCALMFT | MFT_ENUM_FLAG_SORTANDFILTER,
            Some(&input_info),
            Some(&output_info),
            &mut activates,
            &mut count,
        )?;
        let mut found: Option<IMFTransform> = None;
        if !activates.is_null() {
            let slice = std::slice::from_raw_parts_mut(activates, count as usize);
            for slot in slice.iter_mut() {
                if let Some(act) = slot.take()
                    && found.is_none()
                    && let Ok(t) = act.ActivateObject::<IMFTransform>()
                {
                    found = Some(t);
                }
            }
            CoTaskMemFree(Some(activates.cast()));
        }
        found.ok_or_else(|| windows::core::Error::from(E_FAIL))
    }
}

pub(crate) struct VideoAdapter {
    pub luid: u64,
    pub adapter: IDXGIAdapter1,
}

struct GpuDevice {
    adapter_luid: u64,
    manager: IMFDXGIDeviceManager,
}

pub(crate) fn video_adapters() -> windows::core::Result<Vec<VideoAdapter>> {
    // SAFETY: enumeration returns owned COM interfaces; no raw pointers escape.
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
        let mut adapters = Vec::new();
        for index in 0.. {
            let adapter = match factory.EnumAdapters1(index) {
                Ok(adapter) => adapter,
                Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(error) => return Err(error),
            };
            let desc = match adapter.GetDesc1() {
                Ok(desc) => desc,
                Err(error) => {
                    tracing::debug!(%error, index, "Video adapter description unavailable");
                    continue;
                }
            };
            if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                continue;
            }
            adapters.push(VideoAdapter {
                luid: u64::from(desc.AdapterLuid.LowPart) | ((desc.AdapterLuid.HighPart as u64) << 32),
                adapter,
            });
        }
        Ok(adapters)
    }
}

fn gpu_device_manager(adapter: &VideoAdapter, codec: &str) -> windows::core::Result<IMFDXGIDeviceManager> {
    // SAFETY: plain Direct3D and Media Foundation calls with owned values.
    unsafe {
        let mut device: Option<ID3D11Device> = None;
        D3D11CreateDevice(
            &adapter.adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            Default::default(),
            D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            None,
        )?;
        let device = device.ok_or_else(|| windows::core::Error::from(E_FAIL))?;
        let video: ID3D11VideoDevice = device.cast()?;
        let profile = match codec {
            "h265" => D3D11_DECODER_PROFILE_HEVC_VLD_MAIN,
            "av1" => D3D11_DECODER_PROFILE_AV1_VLD_PROFILE0,
            _ => D3D11_DECODER_PROFILE_H264_VLD_NOFGT,
        };
        if !video.CheckVideoDecoderFormat(&profile, DXGI_FORMAT_NV12)?.as_bool() {
            return Err(windows::core::Error::from(E_FAIL));
        }
        // The decoder uses the device from its own threads.
        if let Ok(multithread) = device.cast::<ID3D11Multithread>() {
            let _ = multithread.SetMultithreadProtected(true);
        }
        let mut token = 0u32;
        let mut manager: Option<IMFDXGIDeviceManager> = None;
        MFCreateDXGIDeviceManager(&mut token, &mut manager)?;
        let manager = manager.ok_or_else(|| windows::core::Error::from(E_FAIL))?;
        manager.ResetDevice(&device, token)?;
        Ok(manager)
    }
}

/// Whether an access unit buffer contains a coded slice/frame for `codec`.
fn has_slice(codec: &str, data: &[u8]) -> bool {
    match codec {
        "h265" => data.windows(4).any(|w| w[..3] == [0, 0, 1] && ((w[3] >> 1) & 0x3f) <= 21),
        "av1" => !data.is_empty(),
        _ => data.windows(4).any(|w| w[..3] == [0, 0, 1] && matches!(w[3] & 0x1f, 1 | 5)),
    }
}

pub struct Decoder {
    codec: String,
    transform: IMFTransform,
    /// The GPU the decoder uses, kept alive with it; `None` when it decodes in software.
    gpu: Option<GpuDevice>,
    failed_adapters: Vec<u64>,
    /// Where GPU pictures are copied to be read, reused while the size stays.
    staging: Option<(ID3D11Texture2D, D3D11_TEXTURE2D_DESC)>,
    /// The decoded size, as the output type says (padded).
    coded: (u32, u32),
    provides_samples: bool,
    output_size: u32,
    colors: Colors,
}

impl std::fmt::Debug for Decoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Decoder")
            .field("codec", &self.codec)
            .field("coded", &self.coded)
            .finish_non_exhaustive()
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        // SAFETY: tells the transform streaming is over before it is released.
        unsafe {
            let _ = self.transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
            let _ = self.transform.ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0);
        }
    }
}

impl Decoder {
    /// A low-latency software decoder for `"h264"`, `"h265"`, or `"av1"`, for
    /// the calling thread (which it stays on).
    pub fn new_for_codec(codec: &str) -> windows::core::Result<Decoder> {
        Self::create_with_manager(codec, None)
    }

    /// Tries hardware adapters in DXGI order, then falls back to software.
    pub fn new_gpu(codec: &str) -> windows::core::Result<Decoder> {
        Self::create_hardware(codec, &[]).or_else(|e| {
            tracing::info!(error = %e, "no GPU video decoding; decoding in software");
            Self::new_for_codec(codec)
        })
    }

    /// Whether this decoder runs on the GPU.
    pub fn on_gpu(&self) -> bool {
        self.gpu.is_some()
    }

    fn create_hardware(codec: &str, excluded: &[u64]) -> windows::core::Result<Decoder> {
        ensure_mf_started()?;
        let mut last_error = windows::core::Error::from(E_FAIL);
        for adapter in video_adapters()? {
            if excluded.contains(&adapter.luid) {
                continue;
            }
            let result = gpu_device_manager(&adapter, codec).and_then(|manager| {
                Self::create_with_manager(codec, Some(GpuDevice { adapter_luid: adapter.luid, manager }))
            });
            match result {
                Ok(mut decoder) => {
                    decoder.failed_adapters = excluded.to_vec();
                    tracing::info!(codec, adapter = adapter.luid, "GPU video decoder initialized");
                    return Ok(decoder);
                }
                Err(error) => {
                    tracing::debug!(%error, codec, adapter = adapter.luid, "GPU decoder unavailable on adapter");
                    last_error = error;
                }
            }
        }
        Err(last_error)
    }

    fn recover_gpu(&mut self) {
        let Some(gpu) = &self.gpu else { return };
        let mut excluded = self.failed_adapters.clone();
        excluded.push(gpu.adapter_luid);
        let replacement = Self::create_hardware(&self.codec, &excluded).or_else(|_| {
            tracing::info!(codec = self.codec, "GPU adapters exhausted; decoding in software");
            Self::new_for_codec(&self.codec)
        });
        if let Ok(decoder) = replacement {
            *self = decoder;
        }
    }

    fn create_with_manager(codec: &str, gpu: Option<GpuDevice>) -> windows::core::Result<Decoder> {
        // SAFETY: plain COM and Media Foundation calls with owned values.
        unsafe {
            ensure_mf_started()?;
            let (subtype, transform) = match codec {
                "h265" => (
                    MFVideoFormat_HEVC,
                    create_mft(MFT_CATEGORY_VIDEO_DECODER, MFVideoFormat_HEVC, MFVideoFormat_NV12)?,
                ),
                "av1" => (
                    MFVideoFormat_AV1,
                    create_mft(MFT_CATEGORY_VIDEO_DECODER, MFVideoFormat_AV1, MFVideoFormat_NV12)?,
                ),
                _ => (
                    MFVideoFormat_H264,
                    CoCreateInstance(&CLSID_MSH264DecoderMFT, None, CLSCTX_INPROC_SERVER)?,
                ),
            };
            // Avoid decoder worker-thread and buffering overhead for low-latency streams.
            let attributes = transform.GetAttributes().ok();
            if let Some(attributes) = &attributes {
                let _ = attributes.SetUINT32(&MF_LOW_LATENCY, 1);
                let _ = attributes.SetUINT32(&CODECAPI_AVDecNumWorkerThreads, 1);
            }
            // Media Foundation needs the D3D manager before media types are set.
            if let Some(gpu) = &gpu {
                let aware = attributes
                    .as_ref()
                    .is_some_and(|a| a.GetUINT32(&MF_SA_D3D11_AWARE).is_ok_and(|v| v != 0));
                if !aware {
                    return Err(windows::core::Error::from(E_FAIL));
                }
                transform.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, gpu.manager.as_raw() as usize)?;
            }
            let input = MFCreateMediaType()?;
            input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            input.SetGUID(&MF_MT_SUBTYPE, &subtype)?;
            // Windows' HEVC and AV1 decoders (the Store's video extensions)
            // offer no output until the input has a size; any will do, as
            // the stream's own size replaces it (a stream change).
            if subtype != MFVideoFormat_H264 {
                input.SetUINT64(&MF_MT_FRAME_SIZE, (1920u64 << 32) | 1080)?;
            }
            transform.SetInputType(0, &input, 0)?;
            let mut decoder = Decoder {
                codec: codec.to_owned(),
                transform,
                gpu,
                failed_adapters: Vec::new(),
                staging: None,
                coded: (0, 0),
                provides_samples: false,
                output_size: 0,
                colors: Colors::default(),
            };
            decoder.choose_output()?;
            decoder.transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            decoder.transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
            Ok(decoder)
        }
    }

    pub fn codec(&self) -> &str {
        &self.codec
    }

    /// Picks NV12 output (again after the stream's size changed).
    fn choose_output(&mut self) -> windows::core::Result<()> {
        // SAFETY: as above.
        unsafe {
            let mut index = 0;
            let chosen: IMFMediaType = loop {
                let candidate = self.transform.GetOutputAvailableType(0, index)?;
                if candidate.GetGUID(&MF_MT_SUBTYPE)? == MFVideoFormat_NV12 {
                    break candidate;
                }
                index += 1;
            };
            self.transform.SetOutputType(0, &chosen, 0)?;
            let size = chosen.GetUINT64(&MF_MT_FRAME_SIZE).unwrap_or(0);
            self.coded = ((size >> 32) as u32, size as u32);
            // Unknown until the stream says (then the type changes again).
            self.colors = Colors {
                bt601: chosen
                    .GetUINT32(&MF_MT_YUV_MATRIX)
                    .is_ok_and(|m| m == MFVideoTransferMatrix_BT601.0 as u32),
                full_range: chosen
                    .GetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE)
                    .is_ok_and(|r| r == MFNominalRange_0_255.0 as u32),
            };
            let info = self.transform.GetOutputStreamInfo(0)?;
            self.provides_samples = info.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32 != 0;
            self.output_size = info.cbSize;
            Ok(())
        }
    }

    /// Decodes one access unit (Annex B); returns the pictures it completed.
    /// On a GPU failure, tries another adapter before software. The error is
    /// returned so the caller can request a keyframe for the replacement decoder.
    pub fn decode(&mut self, data: &[u8], time_us: u64) -> windows::core::Result<Vec<Picture>> {
        let frames = self.decode_frames(data, time_us)?;
        frames.iter().map(|frame| self.read_frame(frame)).collect()
    }

    /// Decode without reading GPU pixels back. Reference frames are still
    /// decoded even when the caller discards their output to catch up.
    pub fn decode_frames(&mut self, data: &[u8], time_us: u64) -> windows::core::Result<Vec<DecodedFrame>> {
        let result = self.decode_on(data, time_us);
        if result.is_err() && self.gpu.is_some() {
            self.recover_gpu();
        }
        result
    }

    fn decode_on(&mut self, data: &[u8], time_us: u64) -> windows::core::Result<Vec<DecodedFrame>> {
        let expects_picture = has_slice(&self.codec, data);
        // Windows' HEVC decoder holds a picture until the next access unit
        // starts, a frame late; ending each one with an access unit
        // delimiter (type 35, any slice types) lets it out at once.
        let end = if self.codec == "h265" { &HEVC_AUD[..] } else { &[] };
        let sample = sample_of(data, end, time_us)?;
        let mut pictures = Vec::new();
        // SAFETY: as above.
        loop {
            match unsafe { self.transform.ProcessInput(0, &sample, 0) } {
                Ok(()) => break,
                // Full: take what's ready first, then try again.
                Err(e) if e.code() == MF_E_NOTACCEPTING => pictures.extend(self.drain(false)?),
                Err(e) => return Err(e),
            }
        }
        pictures.extend(self.drain(expects_picture)?);
        Ok(pictures)
    }

    /// Starts over (after lost packets): the next keyframe decodes cleanly.
    pub fn flush(&mut self) {
        // SAFETY: as above.
        unsafe {
            let _ = self.transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
        }
    }

    fn drain(&mut self, expects_picture: bool) -> windows::core::Result<Vec<DecodedFrame>> {
        let mut pictures = Vec::new();
        let mut waits = 0u32;
        loop {
            let provided = if self.provides_samples {
                None
            } else {
                // SAFETY: as above.
                let buffer = unsafe { MFCreateMemoryBuffer(self.output_size.max(1))? };
                let sample = unsafe { MFCreateSample()? };
                unsafe { sample.AddBuffer(&buffer)? };
                Some(sample)
            };
            let mut output = [MFT_OUTPUT_DATA_BUFFER {
                dwStreamID: 0,
                pSample: ManuallyDrop::new(provided),
                dwStatus: 0,
                pEvents: ManuallyDrop::new(None),
            }];
            let mut status = 0;
            // SAFETY: `output` holds a sample of the size the decoder asked
            // for (or none when it provides its own); both fields are taken
            // back below so they're released.
            let result = unsafe { self.transform.ProcessOutput(0, &mut output, &mut status) };
            let sample = unsafe { ManuallyDrop::take(&mut output[0].pSample) };
            drop(unsafe { ManuallyDrop::take(&mut output[0].pEvents) });
            match result {
                Ok(()) => {
                    if let Some(sample) = sample {
                        pictures.push(DecodedFrame {
                            sample,
                            width: self.coded.0,
                            height: self.coded.1,
                            colors: self.colors,
                        });
                    }
                }
                Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => {
                    // When an access unit includes both an SPS that triggers
                    // `MF_E_TRANSFORM_STREAM_CHANGE` and a coded slice, the MFT's
                    // worker thread signals the stream change as soon as it parses
                    // the SPS and continues decoding the slice in parallel. Under
                    // heavy CPU load, the next `ProcessOutput` after `SetOutputType`
                    // can run before the worker thread finishes pushing the picture.
                    if expects_picture && pictures.is_empty() && waits < 500 {
                        waits += 1;
                        std::thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                    return Ok(pictures);
                }
                Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => self.choose_output()?,
                Err(e) => return Err(e),
            }
        }
    }

    pub fn read_frame(&mut self, frame: &DecodedFrame) -> windows::core::Result<Picture> {
        let result = self.picture(frame);
        if result.is_err() && self.gpu.is_some() {
            self.recover_gpu();
        }
        result
    }

    fn picture(&mut self, frame: &DecodedFrame) -> windows::core::Result<Picture> {
        let (width, height) = (frame.width, frame.height);
        let colors = frame.colors;
        let sample = &frame.sample;
        // SAFETY: the buffer is locked while it's read and unlocked after.
        unsafe {
            let buffer = sample.ConvertToContiguousBuffer()?;
            if let Ok(dxgi) = buffer.cast::<IMFDXGIBuffer>() {
                let mut texture: Option<ID3D11Texture2D> = None;
                dxgi.GetResource(&ID3D11Texture2D::IID, std::ptr::from_mut(&mut texture).cast())?;
                let texture = texture.ok_or_else(|| windows::core::Error::from(E_FAIL))?;
                let index = dxgi.GetSubresourceIndex()?;
                return self.read_texture(&texture, index, width, height, colors);
            }
            if let Ok(two_d) = buffer.cast::<IMF2DBuffer>() {
                let mut scan0 = std::ptr::null_mut();
                let mut pitch = 0;
                two_d.Lock2D(&mut scan0, &mut pitch)?;
                if pitch > 0 && !scan0.is_null() && (pitch as usize) >= width as usize {
                    let stride = pitch as usize;
                    let rows = height as usize;
                    let len = stride * rows * 3 / 2;
                    let nv12 = std::slice::from_raw_parts(scan0, len).to_vec();
                    two_d.Unlock2D()?;
                    return Ok(Picture { width, height, stride, plane_rows: rows, nv12, colors });
                }
                two_d.Unlock2D()?;
            }
            let mut data = std::ptr::null_mut();
            let mut length = 0;
            buffer.Lock(&mut data, None, Some(&mut length))?;
            let nv12 = if data.is_null() || length == 0 {
                Vec::new()
            } else {
                std::slice::from_raw_parts(data, length as usize).to_vec()
            };
            buffer.Unlock()?;
            let stride = width as usize;
            let plane_rows = (nv12.len() * 2 / 3) / stride.max(1);
            if plane_rows < height as usize || nv12.len() < stride * (height as usize) * 3 / 2 {
                return Err(E_FAIL.into());
            }
            Ok(Picture { width, height, stride, plane_rows, nv12, colors })
        }
    }
}

impl Decoder {
    /// Copies a decoded picture out of GPU memory. Windows' own path makes a
    /// new staging texture and waits on it for every picture (about 14 ms at
    /// 4K); one kept for the stream takes a few.
    fn read_texture(
        &mut self,
        texture: &ID3D11Texture2D,
        index: u32,
        width: u32,
        height: u32,
        colors: Colors,
    ) -> windows::core::Result<Picture> {
        // SAFETY: Direct3D calls on the decoder's own device; the staging
        // texture is mapped while it's read and unmapped after.
        unsafe {
            let device = texture.GetDevice()?;
            let context = device.GetImmediateContext()?;
            let mut source = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut source);
            let reuse = self.staging.as_ref().is_some_and(|(_, d)| {
                (d.Width, d.Height, d.Format) == (source.Width, source.Height, source.Format)
            });
            if !reuse {
                let desc = D3D11_TEXTURE2D_DESC {
                    Width: source.Width,
                    Height: source.Height,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: source.Format,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Usage: D3D11_USAGE_STAGING,
                    BindFlags: 0,
                    CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                    MiscFlags: 0,
                };
                let mut staging: Option<ID3D11Texture2D> = None;
                device.CreateTexture2D(&desc, None, Some(&mut staging))?;
                let staging = staging.ok_or_else(|| windows::core::Error::from(E_FAIL))?;
                self.staging = Some((staging, desc));
            }
            let (staging, _) = self.staging.as_ref().ok_or_else(|| windows::core::Error::from(E_FAIL))?;
            context.CopySubresourceRegion(staging, 0, 0, 0, 0, texture, index, None);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            context.Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            // NV12: the chroma plane follows the texture's full height of luma.
            let stride = mapped.RowPitch as usize;
            let plane_rows = source.Height as usize;
            let len = stride * plane_rows * 3 / 2;
            let nv12 = std::slice::from_raw_parts(mapped.pData.cast::<u8>(), len).to_vec();
            context.Unmap(staging, 0);
            Ok(Picture { width, height, stride, plane_rows, nv12, colors })
        }
    }
}

/// An HEVC access unit delimiter NAL unit: type 35, `pic_type` 2 (I, P and
/// B slices).
const HEVC_AUD: [u8; 7] = [0, 0, 0, 1, 0x46, 0x01, 0x50];

/// A sample holding `data` followed by `end`.
fn sample_of(data: &[u8], end: &[u8], time_us: u64) -> windows::core::Result<IMFSample> {
    let len = data.len() + end.len();
    // SAFETY: the buffer is at least `len` long and locked while it's
    // written.
    unsafe {
        let buffer = MFCreateMemoryBuffer(len as u32)?;
        let mut target = std::ptr::null_mut();
        buffer.Lock(&mut target, None, None)?;
        std::ptr::copy_nonoverlapping(data.as_ptr(), target, data.len());
        std::ptr::copy_nonoverlapping(end.as_ptr(), target.add(data.len()), end.len());
        buffer.Unlock()?;
        buffer.SetCurrentLength(len as u32)?;
        let sample = MFCreateSample()?;
        sample.AddBuffer(&buffer)?;
        // In 100-nanosecond units.
        sample.SetSampleTime(time_us as i64 * 10)?;
        Ok(sample)
    }
}

/// Pixels from which a frame is converted on several cores: 4K takes about
/// 24 ms on one core, most of a 30 fps frame's 33 ms, while 720p is quick
/// enough on the calling thread.
const PARALLEL_FROM_PIXELS: usize = 1920 * 1080;

/// NV12 (in the picture's own colors) to 32-bit BGRX, the visible
/// `width` x `height`. Large frames are split into bands of rows, one per
/// core (scoped threads cost microseconds, against milliseconds of work).
pub fn to_bgrx(picture: &Picture, width: u32, height: u32) -> Vec<u8> {
    let (w, h) = (width.min(picture.width) as usize, height.min(picture.height) as usize);
    let mut out = vec![0u8; w * h * 4];
    if w == 0 || h == 0 {
        return out;
    }
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get()).min(8);
    if w * h < PARALLEL_FROM_PIXELS || cores < 2 {
        convert_rows(picture, w, 0, &mut out);
        return out;
    }
    // Bands of an even number of rows, so each starts on a chroma row.
    let band = h.div_ceil(cores).next_multiple_of(2);
    std::thread::scope(|scope| {
        for (i, rows) in out.chunks_mut(band * w * 4).enumerate() {
            scope.spawn(move || convert_rows(picture, w, i * band, rows));
        }
    });
    out
}

fn convert_rows(p: &Picture, w: usize, first: usize, out: &mut [u8]) {
    let uv_plane = p.stride * p.plane_rows;
    let (luma, vr, ug, vg, ub) = p.colors.coefficients();
    let black = if p.colors.full_range { 0 } else { 16 };
    // Scaled luma for each byte value, looked up instead of multiplied.
    let mut scaled = [0i32; 256];
    for (y, slot) in scaled.iter_mut().enumerate() {
        *slot = (y as i32 - black) * luma;
    }
    let pixel = |c: i32, (b, g, r): (i32, i32, i32)| {
        [clamp((c + b) >> 8), clamp((c + g) >> 8), clamp((c + r) >> 8), 255]
    };
    for (i, row) in out.chunks_exact_mut(w * 4).enumerate() {
        let y_row = first + i;
        let y_line = &p.nv12[y_row * p.stride..][..w];
        let uv_line = &p.nv12[uv_plane + (y_row / 2) * p.stride..][..w.next_multiple_of(2).min(p.stride)];
        // Two neighbouring pixels share their colour: worked out once for both.
        let tint = |k: usize| {
            let d = i32::from(uv_line[2 * k]) - 128;
            let e = i32::from(uv_line[2 * k + 1]) - 128;
            (ub * d + 128, -ug * d - vg * e + 128, vr * e + 128)
        };
        let (pairs, last) = row.as_chunks_mut::<8>();
        for (k, px) in pairs.iter_mut().enumerate() {
            let t = tint(k);
            let (left, right) = px.split_at_mut(4);
            left.copy_from_slice(&pixel(scaled[usize::from(y_line[2 * k])], t));
            right.copy_from_slice(&pixel(scaled[usize::from(y_line[2 * k + 1])], t));
        }
        // An odd width leaves one pixel.
        if !last.is_empty() {
            last.copy_from_slice(&pixel(scaled[usize::from(y_line[w - 1])], tint((w - 1) / 2)));
        }
    }
}

fn clamp(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_recovery_excludes_the_failed_adapter_and_resumes_at_a_keyframe() {
        let Ok(mut decoder) = Decoder::create_hardware("h264", &[]) else { return };
        let failed = decoder.gpu.as_ref().unwrap().adapter_luid;
        decoder.recover_gpu();
        if let Some(gpu) = &decoder.gpu {
            assert_ne!(gpu.adapter_luid, failed);
            assert!(decoder.failed_adapters.contains(&failed));
        }
        let mut pictures = 0;
        for (i, unit) in test_stream("h264").iter().enumerate() {
            pictures += decoder.decode(unit, i as u64 * 33_333).unwrap().len();
        }
        assert_eq!(pictures, 10);
    }

    #[test]
    fn gpu_frames_can_be_discarded_without_readback_then_materialized() {
        let mut decoder = Decoder::new_gpu("h264").unwrap();
        let mut count = 0;
        let mut surfaces = 0;
        for (i, unit) in test_stream("h264").iter().enumerate() {
            for frame in decoder.decode_frames(unit, i as u64 * 33_333).unwrap() {
                count += 1;
                if frame.gpu_surface().unwrap().is_some() {
                    surfaces += 1;
                }
                assert!(decoder.staging.is_none(), "decode_frames must not download discarded frames");
                if count == 10 {
                    let picture = decoder.read_frame(&frame).unwrap();
                    assert_eq!((picture.width, picture.height), (320, 240));
                    assert!(picture.nv12.len() >= 320 * 240 * 3 / 2);
                    assert_eq!(picture.colors, frame.colors);
                }
            }
        }
        assert_eq!(count, 10);
        if decoder.on_gpu() {
            assert_eq!(surfaces, 10);
            assert!(decoder.staging.is_some());
        }
    }

    #[test]
    #[ignore = "benchmark"]
    fn bench_to_bgrx_4k() {
        let (w, h) = (3840u32, 2160u32);
        let stride = w as usize;
        let picture = Picture {
            width: w,
            height: h,
            stride,
            plane_rows: h as usize,
            nv12: (0..stride * h as usize * 3 / 2).map(|i| (i % 251) as u8).collect(),
            colors: Colors::default(),
        };
        let start = std::time::Instant::now();
        let n = 30;
        for _ in 0..n {
            std::hint::black_box(to_bgrx(&picture, w, h));
        }
        eprintln!("BENCH to_bgrx 4K: {:.1} ms/frame", start.elapsed().as_secs_f64() * 1000.0 / n as f64);
    }

    #[test]
    fn decoder_sample_pool_survives_sustained_gpu_publication() {
        let (done, finished) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            use nectarlink_vcam::gpu::{GpuPublisher, GpuReader};
            use windows::Win32::Media::MediaFoundation::MFCreateDXGISurfaceBuffer;
            for codec in ["h264", "h265"] {
                if !installed(codec) {
                    continue;
                }
                let mut decoder = Decoder::new_gpu(codec).unwrap();
                if !decoder.on_gpu() {
                    continue;
                }
                let name = format!("Local\\NectarlinkDecoderPoolTest_{}_{}", std::process::id(), codec);
                let mut reader = GpuReader::new_with_mapping(&name).unwrap();
                let mut publisher = GpuPublisher::new_with_mapping(&name).unwrap();
                let mut output = None;
                let mut count = 0;
                for _ in 0..20 {
                    for unit in test_stream(codec) {
                        for frame in decoder
                            .decode_frames(unit, count * 33_333)
                            .unwrap_or_else(|error| panic!("{codec} after {count} frames: {error}"))
                        {
                            let (texture, index) = frame.gpu_surface().unwrap().unwrap();
                            if output.is_none() {
                                unsafe {
                                    let mut desc = D3D11_TEXTURE2D_DESC::default();
                                    texture.GetDesc(&mut desc);
                                    desc.ArraySize = 1;
                                    desc.MipLevels = 1;
                                    desc.BindFlags =
                                        windows::Win32::Graphics::Direct3D11::D3D11_BIND_RENDER_TARGET.0
                                            as u32;
                                    let mut destination = None;
                                    texture
                                        .GetDevice()
                                        .unwrap()
                                        .CreateTexture2D(&desc, None, Some(&mut destination))
                                        .unwrap();
                                    output = Some(
                                        MFCreateDXGISurfaceBuffer(
                                            &ID3D11Texture2D::IID,
                                            &destination.unwrap(),
                                            0,
                                            false,
                                        )
                                        .unwrap(),
                                    );
                                }
                            }
                            assert!(
                                publisher
                                    .publish(
                                        &texture,
                                        index,
                                        &frame.sample,
                                        (frame.width, frame.height),
                                        false,
                                        false
                                    )
                                    .unwrap()
                            );
                            assert!(
                                reader
                                    .write_nv12(output.as_ref().unwrap(), frame.width, frame.height)
                                    .unwrap()
                            );
                            count += 1;
                        }
                    }
                }
                assert_eq!(count, 200);
                assert!(decoder.staging.is_none(), "publication must not download the decoded frames");
                eprintln!("{codec}: published {count} GPU frames with zero CPU downloads");
            }
            done.send(()).unwrap();
        });
        finished.recv_timeout(std::time::Duration::from_secs(30)).expect("decoder sample pool stalled");
        worker.join().unwrap();
    }

    #[test]
    #[ignore = "benchmark; needs BENCH_4K_H264"]
    fn bench_decode_4k() {
        let path = std::env::var("BENCH_4K_H264").expect("BENCH_4K_H264");
        let stream = std::fs::read(path).unwrap();
        let units = access_units(&stream);
        for gpu in [false, true] {
            let mut decoder =
                if gpu { Decoder::new_gpu("h264").unwrap() } else { Decoder::new_for_codec("h264").unwrap() };
            let on_gpu = decoder.on_gpu();
            let start = std::time::Instant::now();
            let mut pictures = 0;
            let mut first_out = None;
            for (i, u) in units.iter().enumerate() {
                let got = decoder.decode(u, i as u64 * 33_333).unwrap().len();
                if got > 0 && first_out.is_none() {
                    first_out = Some(i);
                }
                pictures += got;
            }
            eprintln!(
                "BENCH decode 4K gpu={on_gpu}: {} units, {} pictures, first picture after unit {:?}, {:.1} ms/unit",
                units.len(),
                pictures,
                first_out,
                start.elapsed().as_secs_f64() * 1000.0 / units.len() as f64
            );
        }
    }

    #[test]
    fn the_stream_says_which_colors() {
        // 3 frames tagged BT.601, full range (as some phones encode).
        let units = access_units(include_bytes!("testdata/bt601-full.h264"));
        let mut decoder = Decoder::new_for_codec("h264").unwrap();
        let pictures: Vec<Picture> =
            units.iter().enumerate().flat_map(|(i, u)| decoder.decode(u, i as u64).unwrap()).collect();
        assert_eq!(pictures.last().unwrap().colors, Colors { bt601: true, full_range: true });
    }

    #[test]
    fn colors_convert() {
        // 4x2: black, white, and BT.709 red and blue.
        let (w, h) = (4usize, 2usize);
        let mut nv12 = vec![0u8; w * h * 3 / 2];
        nv12[..8].copy_from_slice(&[16, 235, 63, 32, 16, 235, 63, 32]);
        // Chroma for pixel pairs (0,1) and (2,3): neutral; then red's... the
        // second pair holds red's chroma (u=102, v=240).
        nv12[8..12].copy_from_slice(&[128, 128, 102, 240]);
        let picture =
            Picture { width: 4, height: 2, stride: 4, plane_rows: 2, nv12, colors: Colors::default() };
        let out = to_bgrx(&picture, 4, 2);
        assert_eq!(&out[..4], &[0, 0, 0, 255], "black");
        assert_eq!(&out[4..8], &[255, 255, 255, 255], "white");
        let red = &out[8..12];
        assert!(red[2] > 240 && red[1] < 20 && red[0] < 20, "red: {red:?}");
    }

    /// The conversion per pixel, as the formula reads: the faster code must
    /// give exactly these bytes.
    fn reference_rows(p: &Picture, w: usize, first: usize, out: &mut [u8]) {
        let uv_plane = p.stride * p.plane_rows;
        let (luma, vr, ug, vg, ub) = p.colors.coefficients();
        let black = if p.colors.full_range { 0 } else { 16 };
        for (i, row) in out.chunks_exact_mut(w * 4).enumerate() {
            let y_row = first + i;
            for (x, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let c = (i32::from(p.nv12[y_row * p.stride + x]) - black) * luma;
                let d = i32::from(p.nv12[uv_plane + (y_row / 2) * p.stride + (x & !1)]) - 128;
                let e = i32::from(p.nv12[uv_plane + (y_row / 2) * p.stride + (x & !1) + 1]) - 128;
                *px = [
                    clamp((c + ub * d + 128) >> 8),
                    clamp((c - ug * d - vg * e + 128) >> 8),
                    clamp((c + vr * e + 128) >> 8),
                    255,
                ];
            }
        }
    }

    #[test]
    fn conversion_matches_the_formula_for_every_size_and_color_space() {
        for (w, h, stride) in
            [(1usize, 2usize, 2usize), (2, 2, 2), (3, 3, 4), (5, 4, 8), (64, 33, 64), (1081, 9, 1088)]
        {
            let plane_rows = h.next_multiple_of(2);
            let nv12: Vec<u8> = (0..stride * plane_rows * 3 / 2).map(|i| (i * 37 % 256) as u8).collect();
            for (bt601, full_range) in [(false, false), (false, true), (true, false), (true, true)] {
                let p = Picture {
                    width: w as u32,
                    height: h as u32,
                    stride,
                    plane_rows,
                    nv12: nv12.clone(),
                    colors: Colors { bt601, full_range },
                };
                // The whole picture, and a band starting further down as the threads get.
                for first in [0, 1, h / 2] {
                    let rows = h - first;
                    let (mut fast, mut slow) = (vec![0u8; w * rows * 4], vec![0u8; w * rows * 4]);
                    convert_rows(&p, w, first, &mut fast);
                    reference_rows(&p, w, first, &mut slow);
                    assert!(fast == slow, "{w}x{h} from row {first}, bt601 {bt601}, full range {full_range}");
                }
            }
        }
    }

    #[test]
    fn odd_sizes_are_cropped_safely() {
        let picture = Picture {
            width: 6,
            height: 4,
            stride: 8,
            plane_rows: 4,
            nv12: vec![128; 8 * 6],
            colors: Colors::default(),
        };
        assert_eq!(to_bgrx(&picture, 5, 3).len(), 5 * 3 * 4);
        assert_eq!(to_bgrx(&picture, 10, 10).len(), 6 * 4 * 4, "never larger than the picture");
    }

    /// Splits an Annex B stream into access units at its access unit
    /// delimiters (a 4-byte start code, then NAL type 9).
    fn access_units(stream: &[u8]) -> Vec<&[u8]> {
        let starts: Vec<usize> =
            (0..stream.len()).filter(|&i| stream[i..].starts_with(&[0, 0, 0, 1, 9])).collect();
        starts
            .iter()
            .enumerate()
            .map(|(n, &s)| &stream[s..*starts.get(n + 1).unwrap_or(&stream.len())])
            .collect()
    }

    /// HEVC access units, each from its delimiter (as FFmpeg writes them).
    fn hevc_units(stream: &[u8]) -> Vec<&[u8]> {
        let starts: Vec<usize> =
            (0..stream.len()).filter(|&i| stream[i..].starts_with(&[0, 0, 0, 1, 0x46])).collect();
        starts
            .iter()
            .enumerate()
            .map(|(n, &s)| &stream[s..*starts.get(n + 1).unwrap_or(&stream.len())])
            .collect()
    }

    /// An AV1 low-overhead OBU stream split into temporal units, each from
    /// its temporal delimiter, as an encoder hands them out.
    fn temporal_units(stream: &[u8]) -> Vec<&[u8]> {
        let mut starts = Vec::new();
        let mut at = 0;
        while at < stream.len() {
            let header = stream[at];
            if (header >> 3) & 0xf == 2 {
                starts.push(at);
            }
            // The header, its extension byte if any, then a LEB128 size.
            let mut i = at + 1 + usize::from(header & 0x4 != 0);
            let mut size = 0usize;
            for shift in (0..).step_by(7) {
                let byte = stream[i];
                i += 1;
                size |= usize::from(byte & 0x7f) << shift;
                if byte & 0x80 == 0 {
                    break;
                }
            }
            at = i + size;
        }
        starts
            .iter()
            .enumerate()
            .map(|(n, &s)| &stream[s..*starts.get(n + 1).unwrap_or(&stream.len())])
            .collect()
    }

    /// 10 frames of FFmpeg's test pattern, 320x240, a keyframe every 5, no
    /// reordering, as a phone's encoder sends them.
    fn test_stream(codec: &str) -> Vec<&'static [u8]> {
        let units = match codec {
            "h265" => hevc_units(include_bytes!("testdata/testsrc.h265")),
            "av1" => temporal_units(include_bytes!("testdata/testsrc.av1")),
            _ => access_units(include_bytes!("testdata/testsrc.h264")),
        };
        assert_eq!(units.len(), 10, "{codec}");
        units
    }

    /// Whether Windows has a decoder for `codec`: H.264's comes with it,
    /// HEVC's and AV1's are optional extensions from the Store (missing on
    /// CI runners).
    fn installed(codec: &str) -> bool {
        let subtype = match codec {
            "h265" => MFVideoFormat_HEVC,
            "av1" => MFVideoFormat_AV1,
            _ => return true,
        };
        ensure_mf_started().unwrap();
        let input = MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: subtype };
        let mut activates: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut count = 0u32;
        // SAFETY: as in create_mft, which frees the array the same way.
        unsafe {
            MFTEnumEx(
                MFT_CATEGORY_VIDEO_DECODER,
                MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_LOCALMFT,
                Some(&input),
                None,
                &mut activates,
                &mut count,
            )
            .unwrap();
            if !activates.is_null() {
                for slot in std::slice::from_raw_parts_mut(activates, count as usize) {
                    drop(slot.take());
                }
                CoTaskMemFree(Some(activates.cast()));
            }
        }
        count > 0
    }

    /// Decodes `units` on the GPU and in software and checks every picture
    /// matches (decoding is exact). Skips when this PC has no GPU decoding
    /// (CI runners) or no decoder for the codec.
    fn gpu_matches_software(codec: &str, units: &[&[u8]]) {
        if !installed(codec) {
            return;
        }
        let mut software = Decoder::new_for_codec(codec).unwrap();
        let mut gpu = Decoder::new_gpu(codec).unwrap();
        if !gpu.on_gpu() {
            return;
        }
        // Pictures are compared in order, not unit by unit: a busy GPU
        // decoder can hand a picture back one unit later than software.
        let (mut queued_a, mut queued_b) =
            (std::collections::VecDeque::new(), std::collections::VecDeque::new());
        let mut compared = 0;
        for (i, unit) in units.iter().enumerate() {
            queued_a.extend(software.decode(unit, i as u64 * 33_333).unwrap());
            queued_b.extend(gpu.decode(unit, i as u64 * 33_333).unwrap());
            assert!(gpu.on_gpu(), "{codec}: fell back to software at unit {i}");
            while let (Some(a), Some(b)) = (queued_a.front(), queued_b.front()) {
                assert_eq!((a.width, a.height), (b.width, b.height));
                let (w, h) = (a.width as usize, a.height as usize);
                for row in 0..h {
                    assert_eq!(
                        a.nv12[row * a.stride..][..w],
                        b.nv12[row * b.stride..][..w],
                        "{codec} picture {compared} luma row {row}"
                    );
                }
                let (ca, cb) = (a.stride * a.plane_rows, b.stride * b.plane_rows);
                for row in 0..h / 2 {
                    assert_eq!(
                        a.nv12[ca + row * a.stride..][..w],
                        b.nv12[cb + row * b.stride..][..w],
                        "{codec} picture {compared} chroma row {row}"
                    );
                }
                queued_a.pop_front();
                queued_b.pop_front();
                compared += 1;
            }
        }
        assert!(compared > 0, "{codec}: no pictures to compare");
        // Whatever one decoder still holds back at the end of the stream.
        assert!(
            queued_a.len() + queued_b.len() <= 2,
            "{codec}: {} pictures from software and {} from the GPU left over",
            queued_a.len(),
            queued_b.len()
        );
    }

    #[test]
    fn the_gpu_decodes_the_same_pictures_as_software() {
        for codec in ["h264", "h265", "av1"] {
            gpu_matches_software(codec, &test_stream(codec));
        }
    }

    #[test]
    fn a_stream_decodes_picture_by_picture() {
        for codec in ["h264", "h265", "av1"] {
            if !installed(codec) {
                eprintln!("{codec}: no decoder on this PC");
                continue;
            }
            let mut decoder = Decoder::new_for_codec(codec).unwrap();
            let mut pictures = Vec::new();
            for (i, unit) in test_stream(codec).iter().enumerate() {
                let got = decoder.decode(unit, i as u64 * 33_333).unwrap();
                // Low-latency mode: each picture comes out with its own unit.
                assert_eq!(got.len(), 1, "{codec} unit {i}");
                pictures.extend(got);
            }
            let first = &pictures[0];
            assert_eq!((first.width, first.height), (320, 240), "{codec}");
            let bgrx = to_bgrx(first, 320, 240);
            // A colorful test pattern, not a blank picture.
            let distinct: std::collections::HashSet<&[u8; 4]> = bgrx.as_chunks::<4>().0.iter().collect();
            assert!(distinct.len() > 50, "{codec}: {} colors", distinct.len());
        }
        let mut h264 = Decoder::new_for_codec("h264").unwrap();
        let first = h264.decode(test_stream("h264")[0], 0).unwrap();
        assert_eq!(first[0].colors, Colors { bt601: false, full_range: false }, "tagged BT.709, limited");
    }
}
