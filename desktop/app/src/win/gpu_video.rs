// SPDX-License-Identifier: GPL-3.0-or-later
//! GPU NV12 → RGBA8 conversion, cropping, and scaling into a shared D3D11
//! texture for Qt's scene graph (`cpp/video_view.cpp`), avoiding per-frame
//! CPU readback and color conversion on the decoder thread.

use std::{
    mem::ManuallyDrop,
    sync::atomic::{AtomicU64, Ordering},
};

use windows::{
    Win32::{
        Foundation::{CloseHandle, E_FAIL, HANDLE, RECT, S_OK},
        Graphics::{
            Direct3D11::{
                D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_READ, D3D11_MAP_READ,
                D3D11_MAPPED_SUBRESOURCE, D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX,
                D3D11_RESOURCE_MISC_SHARED_NTHANDLE, D3D11_TEX2D_VPIV, D3D11_TEX2D_VPOV,
                D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING, D3D11_VIDEO_COLOR,
                D3D11_VIDEO_COLOR_0, D3D11_VIDEO_COLOR_RGBA, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                D3D11_VIDEO_PROCESSOR_ALPHA_FILL_MODE_OPAQUE, D3D11_VIDEO_PROCESSOR_COLOR_SPACE,
                D3D11_VIDEO_PROCESSOR_CONTENT_DESC, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC,
                D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC,
                D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_STREAM,
                D3D11_VIDEO_USAGE_PLAYBACK_NORMAL, D3D11_VPIV_DIMENSION_TEXTURE2D,
                D3D11_VPOV_DIMENSION_TEXTURE2D, ID3D11Device, ID3D11Multithread, ID3D11Texture2D,
                ID3D11VideoContext, ID3D11VideoContext1, ID3D11VideoDevice, ID3D11VideoProcessor,
                ID3D11VideoProcessorEnumerator, ID3D11VideoProcessorOutputView,
            },
            Dxgi::{
                Common::{
                    DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709, DXGI_COLOR_SPACE_YCBCR_FULL_G22_LEFT_P601,
                    DXGI_COLOR_SPACE_YCBCR_FULL_G22_LEFT_P709, DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P601,
                    DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709, DXGI_FORMAT_NV12,
                    DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_RATIONAL, DXGI_SAMPLE_DESC,
                },
                DXGI_SHARED_RESOURCE_READ, DXGI_SHARED_RESOURCE_WRITE, IDXGIKeyedMutex, IDXGIResource1,
            },
        },
    },
    core::{Interface, PCWSTR},
};

static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);

/// Metadata returned when a frame is published into the shared GPU texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublishedFrame {
    pub epoch: u64,
    pub handle: usize,
    pub width: u32,
    pub height: u32,
}

/// Lightweight handle to the current shared GPU texture so screenshots can read
/// back pixels on demand without touching the 60fps decode path.
#[derive(Clone)]
pub struct GpuScreenshotTarget {
    pub epoch: u64,
    pub width: u32,
    pub height: u32,
    device: ID3D11Device,
    texture: ID3D11Texture2D,
    mutex: IDXGIKeyedMutex,
}

impl std::fmt::Debug for GpuScreenshotTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuScreenshotTarget")
            .field("epoch", &self.epoch)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl GpuScreenshotTarget {
    /// Reads the latest shared RGBA8 frame back to CPU memory as straight-alpha
    /// BGRA (`[B, G, R, 0xFF]`), matching `win::image::Bitmap`.
    pub fn read_bgra(&self) -> windows::core::Result<Vec<u8>> {
        read_texture_bgra(&self.device, &self.texture, &self.mutex, self.width, self.height, 100)
    }
}

struct SharedOutput {
    device: ID3D11Device,
    width: u32,
    height: u32,
    epoch: u64,
    texture: ID3D11Texture2D,
    mutex: IDXGIKeyedMutex,
    handle: HANDLE,
}

impl Drop for SharedOutput {
    fn drop(&mut self) {
        // SAFETY: `handle` is an owned NT handle returned by `CreateSharedHandle`.
        // Consumers duplicate their own handle before `video_gpu_frame` returns.
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

struct ProcessorState {
    device_raw: usize,
    input: (u32, u32),
    output: (u32, u32),
    output_epoch: u64,
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    output_view: ID3D11VideoProcessorOutputView,
}

/// Converts and scales decoder-owned NV12 GPU surfaces into a persistent shared
/// `DXGI_FORMAT_R8G8B8A8_UNORM` texture guarded by a keyed mutex (key 0).
pub struct GpuVideoPublisher {
    output: Option<SharedOutput>,
    processor: Option<ProcessorState>,
}

impl std::fmt::Debug for GpuVideoPublisher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuVideoPublisher")
            .field("epoch", &self.output.as_ref().map(|o| o.epoch))
            .field("size", &self.output.as_ref().map(|o| (o.width, o.height)))
            .finish_non_exhaustive()
    }
}

impl Default for GpuVideoPublisher {
    fn default() -> Self {
        Self::new()
    }
}

impl GpuVideoPublisher {
    pub fn new() -> Self {
        Self { output: None, processor: None }
    }

    /// Returns a screenshot reader for the current shared output texture, if any.
    pub fn screenshot_target(&self) -> Option<GpuScreenshotTarget> {
        self.output.as_ref().map(|o| GpuScreenshotTarget {
            epoch: o.epoch,
            width: o.width,
            height: o.height,
            device: o.device.clone(),
            texture: o.texture.clone(),
            mutex: o.mutex.clone(),
        })
    }

    /// On-demand CPU readback of the latest published frame as BGRX/BGRA (`[B, G, R, 0xFF]`).
    pub fn read_bgrx(&self) -> windows::core::Result<Option<(u32, u32, Vec<u8>)>> {
        let Some(output) = &self.output else {
            return Ok(None);
        };
        let bgrx = read_texture_bgra(
            &output.device,
            &output.texture,
            &output.mutex,
            output.width,
            output.height,
            100,
        )?;
        Ok(Some((output.width, output.height, bgrx)))
    }

    /// Blits `source[index]` (NV12) cropped to `src_w x src_h` into the persistent
    /// shared RGBA8 texture sized `dst_w x dst_h`.
    ///
    /// Returns `Ok(Some(...))` when a new frame is ready to present, `Ok(None)`
    /// if the consumer currently holds the keyed mutex (skip without stalling),
    /// or `Err(...)` if D3D11 video processing is unavailable on this surface.
    #[allow(clippy::too_many_arguments)]
    pub fn publish(
        &mut self,
        source: &ID3D11Texture2D,
        index: u32,
        src_w: u32,
        src_h: u32,
        dst_w: u32,
        dst_h: u32,
        bt601: bool,
        full_range: bool,
    ) -> windows::core::Result<Option<PublishedFrame>> {
        if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
            return Err(E_FAIL.into());
        }
        // SAFETY: plain D3D11/DXGI calls on live COM interfaces owned by the decoder
        // and this publisher. Keyed mutex ownership is released on all exit paths.
        unsafe {
            let mut src_desc = D3D11_TEXTURE2D_DESC::default();
            source.GetDesc(&mut src_desc);
            if src_desc.Format != DXGI_FORMAT_NV12 {
                return Err(E_FAIL.into());
            }
            let src_w = src_w.min(src_desc.Width);
            let src_h = src_h.min(src_desc.Height);
            if src_w == 0 || src_h == 0 {
                return Err(E_FAIL.into());
            }

            let device = source.GetDevice()?;
            if let Ok(multithread) = device.cast::<ID3D11Multithread>() {
                let _ = multithread.SetMultithreadProtected(true);
            }
            let device_raw = device.as_raw() as usize;

            let output_matches = self.output.as_ref().is_some_and(|o| {
                o.device.as_raw() as usize == device_raw && (o.width, o.height) == (dst_w, dst_h)
            });
            if !output_matches {
                self.processor = None;
                self.output = Some(create_shared_output(&device, dst_w, dst_h)?);
            }
            let output = self.output.as_ref().ok_or(E_FAIL)?;

            let proc_matches = self.processor.as_ref().is_some_and(|p| {
                p.device_raw == device_raw
                    && p.input == (src_desc.Width, src_desc.Height)
                    && p.output == (dst_w, dst_h)
                    && p.output_epoch == output.epoch
            });
            if !proc_matches {
                match create_processor(&device, (src_desc.Width, src_desc.Height), (dst_w, dst_h), output) {
                    Ok(state) => self.processor = Some(state),
                    Err(err) => {
                        self.processor = None;
                        self.output = None;
                        return Err(err);
                    }
                }
            }
            let proc_state = self.processor.as_ref().ok_or(E_FAIL)?;

            if !acquire_mutex_nowait(&output.mutex)? {
                return Ok(None);
            }

            let blit_result = blit_surface(
                &device,
                proc_state,
                source,
                &src_desc,
                index,
                (src_w, src_h),
                (dst_w, dst_h),
                bt601,
                full_range,
            );
            let _ = output.mutex.ReleaseSync(0);
            blit_result?;

            Ok(Some(PublishedFrame {
                epoch: output.epoch,
                handle: output.handle.0 as usize,
                width: output.width,
                height: output.height,
            }))
        }
    }
}

unsafe fn create_shared_output(
    device: &ID3D11Device,
    width: u32,
    height: u32,
) -> windows::core::Result<SharedOutput> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
        CPUAccessFlags: 0,
        MiscFlags: (D3D11_RESOURCE_MISC_SHARED_NTHANDLE.0 | D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX.0) as u32,
    };
    let mut texture = None;
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
    let texture = texture.ok_or(E_FAIL)?;
    let mutex: IDXGIKeyedMutex = texture.cast()?;
    let resource: IDXGIResource1 = texture.cast()?;
    let handle = unsafe {
        resource.CreateSharedHandle(
            None,
            (DXGI_SHARED_RESOURCE_READ | DXGI_SHARED_RESOURCE_WRITE).0,
            PCWSTR::null(),
        )?
    };
    let epoch = NEXT_EPOCH.fetch_add(1, Ordering::Relaxed);
    Ok(SharedOutput { device: device.clone(), width, height, epoch, texture, mutex, handle })
}

unsafe fn create_processor(
    device: &ID3D11Device,
    input: (u32, u32),
    output_size: (u32, u32),
    output: &SharedOutput,
) -> windows::core::Result<ProcessorState> {
    let video: ID3D11VideoDevice = device.cast()?;
    let enumerator = unsafe {
        video.CreateVideoProcessorEnumerator(&D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
            InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            InputFrameRate: DXGI_RATIONAL { Numerator: 60, Denominator: 1 },
            InputWidth: input.0,
            InputHeight: input.1,
            OutputFrameRate: DXGI_RATIONAL { Numerator: 60, Denominator: 1 },
            OutputWidth: output_size.0,
            OutputHeight: output_size.1,
            Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
        })?
    };
    let processor = unsafe { video.CreateVideoProcessor(&enumerator, 0)? };
    let mut output_view = None;
    unsafe {
        video.CreateVideoProcessorOutputView(
            &output.texture,
            &enumerator,
            &D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
                },
            },
            Some(&mut output_view),
        )?;
    }
    let output_view = output_view.ok_or(E_FAIL)?;
    Ok(ProcessorState {
        device_raw: device.as_raw() as usize,
        input,
        output: output_size,
        output_epoch: output.epoch,
        enumerator,
        processor,
        output_view,
    })
}

#[allow(clippy::too_many_arguments)]
unsafe fn blit_surface(
    device: &ID3D11Device,
    proc_state: &ProcessorState,
    source: &ID3D11Texture2D,
    src_desc: &D3D11_TEXTURE2D_DESC,
    index: u32,
    visible: (u32, u32),
    dst_size: (u32, u32),
    bt601: bool,
    full_range: bool,
) -> windows::core::Result<()> {
    let video: ID3D11VideoDevice = device.cast()?;
    let immediate = unsafe { device.GetImmediateContext()? };
    let video_ctx: ID3D11VideoContext = immediate.cast()?;

    let mip_levels = src_desc.MipLevels.max(1);
    let mut input_view = None;
    unsafe {
        video.CreateVideoProcessorInputView(
            source,
            &proc_state.enumerator,
            &D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
                FourCC: 0,
                ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VPIV {
                        MipSlice: index % mip_levels,
                        ArraySlice: index / mip_levels,
                    },
                },
            },
            Some(&mut input_view),
        )?;
    }

    let source_rect = RECT { left: 0, top: 0, right: visible.0 as i32, bottom: visible.1 as i32 };
    let target_rect = RECT { left: 0, top: 0, right: dst_size.0 as i32, bottom: dst_size.1 as i32 };
    unsafe {
        video_ctx.VideoProcessorSetStreamSourceRect(&proc_state.processor, 0, true, Some(&source_rect));
        video_ctx.VideoProcessorSetStreamDestRect(&proc_state.processor, 0, true, Some(&target_rect));
        video_ctx.VideoProcessorSetOutputTargetRect(&proc_state.processor, true, Some(&target_rect));
        video_ctx.VideoProcessorSetStreamAutoProcessingMode(&proc_state.processor, 0, false);
        video_ctx.VideoProcessorSetOutputAlphaFillMode(
            &proc_state.processor,
            D3D11_VIDEO_PROCESSOR_ALPHA_FILL_MODE_OPAQUE,
            0,
        );
        video_ctx.VideoProcessorSetOutputBackgroundColor(
            &proc_state.processor,
            false,
            &D3D11_VIDEO_COLOR {
                Anonymous: D3D11_VIDEO_COLOR_0 {
                    RGBA: D3D11_VIDEO_COLOR_RGBA { R: 0.0, G: 0.0, B: 0.0, A: 1.0 },
                },
            },
        );
        if let Ok(colors) = video_ctx.cast::<ID3D11VideoContext1>() {
            let input_cs = match (bt601, full_range) {
                (false, false) => DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709,
                (false, true) => DXGI_COLOR_SPACE_YCBCR_FULL_G22_LEFT_P709,
                (true, false) => DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P601,
                (true, true) => DXGI_COLOR_SPACE_YCBCR_FULL_G22_LEFT_P601,
            };
            colors.VideoProcessorSetStreamColorSpace1(&proc_state.processor, 0, input_cs);
            colors.VideoProcessorSetOutputColorSpace1(
                &proc_state.processor,
                DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709,
            );
        } else {
            let ycbcr_matrix = u32::from(!bt601) << 2;
            let nominal_range = (if full_range { 2u32 } else { 1u32 }) << 4;
            video_ctx.VideoProcessorSetStreamColorSpace(
                &proc_state.processor,
                0,
                &D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: ycbcr_matrix | nominal_range },
            );
            video_ctx.VideoProcessorSetOutputColorSpace(
                &proc_state.processor,
                &D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: 0 },
            );
        }

        let mut stream = D3D11_VIDEO_PROCESSOR_STREAM {
            Enable: true.into(),
            pInputSurface: ManuallyDrop::new(input_view),
            ..Default::default()
        };
        let result = video_ctx.VideoProcessorBlt(
            &proc_state.processor,
            &proc_state.output_view,
            0,
            std::slice::from_ref(&stream),
        );
        drop(ManuallyDrop::take(&mut stream.pInputSurface));
        result?;
        immediate.Flush();
    }
    Ok(())
}

/// Calls `IDXGIKeyedMutex::AcquireSync(0, 0)` through the vtable so `WAIT_TIMEOUT`
/// (258, a success HRESULT) is distinguished from `S_OK`.
fn acquire_mutex_nowait(mutex: &IDXGIKeyedMutex) -> windows::core::Result<bool> {
    for attempt in 0..3 {
        // SAFETY: valid interface pointer; key 0 is the only key used.
        let status = unsafe { (mutex.vtable().AcquireSync)(mutex.as_raw(), 0, 0) };
        if status == S_OK {
            return Ok(true);
        }
        if status.0 == 258 {
            if attempt < 2 {
                std::thread::yield_now();
                continue;
            }
            return Ok(false);
        }
        if status.is_err() {
            return Err(windows::core::Error::from_hresult(status));
        }
        return Err(E_FAIL.into());
    }
    Ok(false)
}

fn acquire_mutex_wait(mutex: &IDXGIKeyedMutex, timeout_ms: u32) -> windows::core::Result<bool> {
    // SAFETY: valid interface pointer; key 0 is the only key used.
    let status = unsafe { (mutex.vtable().AcquireSync)(mutex.as_raw(), 0, timeout_ms) };
    if status == S_OK {
        Ok(true)
    } else if status.0 == 258 {
        Ok(false)
    } else if status.is_err() {
        Err(windows::core::Error::from_hresult(status))
    } else {
        Err(E_FAIL.into())
    }
}

fn read_texture_bgra(
    device: &ID3D11Device,
    texture: &ID3D11Texture2D,
    mutex: &IDXGIKeyedMutex,
    width: u32,
    height: u32,
    timeout_ms: u32,
) -> windows::core::Result<Vec<u8>> {
    if width == 0 || height == 0 {
        return Err(E_FAIL.into());
    }
    if !acquire_mutex_wait(mutex, timeout_ms)? {
        return Err(E_FAIL.into());
    }
    // SAFETY: `device` is multithread-protected, `texture` belongs to `device`,
    // and keyed mutex key 0 is held while copying into our local staging texture.
    unsafe {
        let copy_result = (|| -> windows::core::Result<ID3D11Texture2D> {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };
            let mut staging = None;
            device.CreateTexture2D(&desc, None, Some(&mut staging))?;
            let staging = staging.ok_or(E_FAIL)?;
            let context = device.GetImmediateContext()?;
            context.CopyResource(&staging, texture);
            Ok(staging)
        })();
        let _ = mutex.ReleaseSync(0);
        let staging = copy_result?;

        let context = device.GetImmediateContext()?;
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
        let row_bytes = width as usize * 4;
        let mut out = vec![0u8; row_bytes * height as usize];
        let base = mapped.pData.cast::<u8>();
        let pitch = mapped.RowPitch as usize;
        if !base.is_null() && pitch >= row_bytes {
            for y in 0..height as usize {
                let src_row = std::slice::from_raw_parts(base.add(y * pitch), row_bytes);
                let dst_row = &mut out[y * row_bytes..(y + 1) * row_bytes];
                for (src_px, dst_px) in
                    src_row.as_chunks::<4>().0.iter().zip(dst_row.as_chunks_mut::<4>().0.iter_mut())
                {
                    *dst_px = [src_px[2], src_px[1], src_px[0], 0xFF];
                }
            }
        }
        context.Unmap(&staging, 0);
        if base.is_null() || pitch < row_bytes {
            return Err(E_FAIL.into());
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::win::h264::{Decoder, video_adapters};
    use windows::Win32::Graphics::{
        Direct3D::D3D_DRIVER_TYPE_UNKNOWN,
        Direct3D11::{D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice},
    };

    #[test]
    fn gpu_publisher_converts_nv12_and_reuses_epoch_until_size_changes() {
        let Ok(adapters) = video_adapters() else { return };
        let Some(adapter) = adapters.first() else { return };
        let mut device = None;
        let ok = unsafe {
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
            )
        };
        if ok.is_err() {
            return;
        }
        let Some(device) = device else { return };

        let mut nv12 = None;
        let tex_desc = D3D11_TEXTURE2D_DESC {
            Width: 64,
            Height: 64,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_NV12,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: 0,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        if unsafe { device.CreateTexture2D(&tex_desc, None, Some(&mut nv12)) }.is_err() {
            return;
        }
        let Some(nv12) = nv12 else { return };

        let mut publisher = GpuVideoPublisher::new();
        let first = match publisher.publish(&nv12, 0, 64, 64, 64, 64, false, false) {
            Ok(Some(published)) => published,
            _ => return,
        };
        assert_ne!(first.epoch, 0);
        assert_ne!(first.handle, 0);
        assert_eq!((first.width, first.height), (64, 64));

        // Same dimensions + device reuses the shared texture and epoch.
        let second = publisher.publish(&nv12, 0, 60, 60, 64, 64, true, true).unwrap().unwrap();
        assert_eq!(second.epoch, first.epoch);

        // Changing target dimensions allocates a new shared texture and bumps epoch.
        let third = publisher.publish(&nv12, 0, 64, 64, 128, 128, false, false).unwrap().unwrap();
        assert_ne!(third.epoch, first.epoch);
        assert_eq!((third.width, third.height), (128, 128));

        let (w, h, bgrx) = publisher.read_bgrx().unwrap().unwrap();
        assert_eq!((w, h), (128, 128));
        assert_eq!(bgrx.len(), 128 * 128 * 4);
        assert!(bgrx.as_chunks::<4>().0.iter().all(|px| px[3] == 0xFF));
    }

    #[test]
    fn gpu_publisher_consumes_decoded_gpu_frames_with_zero_readback() {
        let Ok(mut decoder) = Decoder::new_gpu("h264") else { return };
        if !decoder.on_gpu() {
            return;
        }
        let stream = include_bytes!("testdata/testsrc.h264");
        let starts: Vec<usize> =
            (0..stream.len()).filter(|&i| stream[i..].starts_with(&[0, 0, 0, 1, 9])).collect();
        let units: Vec<&[u8]> = starts
            .iter()
            .enumerate()
            .map(|(n, &s)| &stream[s..*starts.get(n + 1).unwrap_or(&stream.len())])
            .collect();

        let mut publisher = GpuVideoPublisher::new();
        let mut published_count = 0;
        for (i, unit) in units.iter().enumerate() {
            for frame in decoder.decode_frames(unit, i as u64 * 33_333).unwrap() {
                let Some((tex, idx)) = frame.gpu_surface().unwrap() else { continue };
                if let Ok(Some(published)) = publisher.publish(
                    &tex,
                    idx,
                    frame.width,
                    frame.height,
                    frame.width,
                    frame.height,
                    frame.colors.bt601,
                    frame.colors.full_range,
                ) {
                    assert_ne!(published.handle, 0);
                    published_count += 1;
                }
            }
        }
        if published_count > 0 {
            let (w, h, bgrx) = publisher.read_bgrx().unwrap().unwrap();
            assert_eq!((w, h), (320, 240));
            assert_eq!(bgrx.len(), 320 * 240 * 4);
        }
    }
}
