// SPDX-License-Identifier: GPL-3.0-or-later
//! GPU screen capture via DXGI Desktop Duplication and single-pass BGRA-to-NV12
//! scaling and color conversion via `ID3D11VideoProcessor`.

use std::mem::ManuallyDrop;

use windows::{
    Win32::{
        Foundation::{E_FAIL, E_INVALIDARG, POINT, RECT},
        Graphics::{
            Direct3D::D3D_DRIVER_TYPE_UNKNOWN,
            Direct3D11::{
                D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_SDK_VERSION, D3D11_TEX2D_VPIV, D3D11_TEX2D_VPOV,
                D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                D3D11_VIDEO_PROCESSOR_COLOR_SPACE, D3D11_VIDEO_PROCESSOR_CONTENT_DESC,
                D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0,
                D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0,
                D3D11_VIDEO_PROCESSOR_STREAM, D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
                D3D11_VPIV_DIMENSION_TEXTURE2D, D3D11_VPOV_DIMENSION_TEXTURE2D, D3D11CreateDevice,
                ID3D11Device, ID3D11DeviceContext, ID3D11Multithread, ID3D11Texture2D, ID3D11VideoContext,
                ID3D11VideoContext1, ID3D11VideoDevice, ID3D11VideoProcessor, ID3D11VideoProcessorEnumerator,
                ID3D11VideoProcessorInputView, ID3D11VideoProcessorOutputView,
            },
            Dxgi::{
                Common::{
                    DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709, DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709,
                    DXGI_FORMAT, DXGI_FORMAT_NV12, DXGI_RATIONAL, DXGI_SAMPLE_DESC,
                },
                CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_ERROR_NOT_FOUND,
                DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO, IDXGIFactory1, IDXGIOutput1,
                IDXGIOutputDuplication, IDXGIResource,
            },
            Gdi::{MONITOR_DEFAULTTOPRIMARY, MonitorFromPoint},
        },
    },
    core::Interface,
};

/// Captures a single Windows monitor via DXGI Desktop Duplication into a
/// persistent GPU `ID3D11Texture2D`.
pub struct DxgiScreenCapture {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    adapter_luid: u64,
    duplication: IDXGIOutputDuplication,
    texture: Option<ID3D11Texture2D>,
    texture_desc: Option<(u32, u32, DXGI_FORMAT)>,
    has_frame: bool,
}

impl std::fmt::Debug for DxgiScreenCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DxgiScreenCapture")
            .field("adapter_luid", &self.adapter_luid)
            .field("texture_desc", &self.texture_desc)
            .field("has_frame", &self.has_frame)
            .finish_non_exhaustive()
    }
}

impl DxgiScreenCapture {
    /// Opens DXGI Desktop Duplication for `target` (matching
    /// `DXGI_OUTPUT_DESC.DesktopCoordinates`) or the primary monitor when `None`.
    pub fn new(target: Option<RECT>) -> windows::core::Result<Self> {
        // SAFETY: DXGI factory/adapter/output enumeration and D3D11 device creation with owned COM interfaces.
        unsafe {
            let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
            let primary_hmon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
            let mut last_error: Option<windows::core::Error> = None;

            for adapter_idx in 0.. {
                let adapter = match factory.EnumAdapters1(adapter_idx) {
                    Ok(adapter) => adapter,
                    Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
                    Err(e) => return Err(e),
                };
                let Ok(adapter_desc) = adapter.GetDesc1() else {
                    continue;
                };
                if adapter_desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                    continue;
                }
                let adapter_luid = u64::from(adapter_desc.AdapterLuid.LowPart)
                    | ((adapter_desc.AdapterLuid.HighPart as u64) << 32);

                for output_idx in 0.. {
                    let output = match adapter.EnumOutputs(output_idx) {
                        Ok(output) => output,
                        Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
                        Err(e) => return Err(e),
                    };
                    let Ok(output_desc) = output.GetDesc() else {
                        continue;
                    };
                    if !output_desc.AttachedToDesktop.as_bool() {
                        continue;
                    }
                    let coords = output_desc.DesktopCoordinates;
                    let matches = match target {
                        Some(r) => {
                            coords.left == r.left
                                && coords.top == r.top
                                && coords.right == r.right
                                && coords.bottom == r.bottom
                        }
                        None => {
                            if !primary_hmon.is_invalid() {
                                output_desc.Monitor == primary_hmon
                            } else {
                                coords.left == 0 && coords.top == 0
                            }
                        }
                    };
                    if !matches {
                        continue;
                    }

                    let mut device: Option<ID3D11Device> = None;
                    let mut context: Option<ID3D11DeviceContext> = None;
                    if let Err(e) = D3D11CreateDevice(
                        &adapter,
                        D3D_DRIVER_TYPE_UNKNOWN,
                        Default::default(),
                        D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                        None,
                        D3D11_SDK_VERSION,
                        Some(&mut device),
                        None,
                        Some(&mut context),
                    ) {
                        last_error = Some(e);
                        continue;
                    }
                    let device = device.ok_or_else(|| windows::core::Error::from(E_FAIL))?;
                    let context = context.ok_or_else(|| windows::core::Error::from(E_FAIL))?;
                    if let Ok(mt) = device.cast::<ID3D11Multithread>() {
                        let _ = mt.SetMultithreadProtected(true);
                    }

                    let output1: IDXGIOutput1 = output.cast()?;
                    match output1.DuplicateOutput(&device) {
                        Ok(duplication) => {
                            return Ok(Self {
                                device,
                                context,
                                adapter_luid,
                                duplication,
                                texture: None,
                                texture_desc: None,
                                has_frame: false,
                            });
                        }
                        Err(e) => {
                            last_error = Some(e);
                        }
                    }
                }
            }

            Err(last_error.unwrap_or_else(|| windows::core::Error::from(DXGI_ERROR_NOT_FOUND)))
        }
    }

    /// The D3D11 device driving this duplication session.
    pub fn device(&self) -> &ID3D11Device {
        &self.device
    }

    /// The DXGI adapter LUID backing `device()`.
    pub fn adapter_luid(&self) -> u64 {
        self.adapter_luid
    }

    /// Acquires the latest desktop frame non-blockingly (`AcquireNextFrame(0)`),
    /// copies it to a persistent `DEFAULT` GPU texture, and releases the DXGI
    /// frame immediately. Reuses the previous texture on `DXGI_ERROR_WAIT_TIMEOUT`,
    /// and propagates `DXGI_ERROR_ACCESS_LOST` or other fatal errors so the caller
    /// can recreate or fall back.
    pub fn acquire_texture(&mut self) -> windows::core::Result<Option<&ID3D11Texture2D>> {
        let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource: Option<IDXGIResource> = None;
        // SAFETY: `AcquireNextFrame` and `ReleaseFrame` are paired whenever `AcquireNextFrame` returns `Ok(())`.
        let acquired = unsafe { self.duplication.AcquireNextFrame(0, &mut frame_info, &mut resource) };
        match acquired {
            Ok(()) => {
                let copy_result = (|| -> windows::core::Result<()> {
                    if frame_info.LastPresentTime == 0 {
                        return Ok(());
                    }
                    let resource = resource.ok_or_else(|| windows::core::Error::from(E_FAIL))?;
                    let src: ID3D11Texture2D = resource.cast()?;
                    let mut desc = D3D11_TEXTURE2D_DESC::default();
                    // SAFETY: valid D3D11 texture interface.
                    unsafe { src.GetDesc(&mut desc) };
                    if self.texture_desc != Some((desc.Width, desc.Height, desc.Format)) {
                        let tex_desc = D3D11_TEXTURE2D_DESC {
                            Width: desc.Width,
                            Height: desc.Height,
                            MipLevels: 1,
                            ArraySize: 1,
                            Format: desc.Format,
                            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                            Usage: D3D11_USAGE_DEFAULT,
                            // The GPU converter's video processor reads it; some
                            // drivers (NVIDIA) only take one that's a render target too.
                            BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
                            CPUAccessFlags: 0,
                            MiscFlags: 0,
                        };
                        let mut dst = None;
                        // SAFETY: creates a GPU-only DEFAULT texture matching the acquired frame.
                        unsafe { self.device.CreateTexture2D(&tex_desc, None, Some(&mut dst))? };
                        self.texture = Some(dst.ok_or_else(|| windows::core::Error::from(E_FAIL))?);
                        self.texture_desc = Some((desc.Width, desc.Height, desc.Format));
                    }
                    if let Some(dst) = self.texture.as_ref() {
                        // SAFETY: both textures belong to `self.device` and share dimensions and format.
                        unsafe { self.context.CopyResource(dst, &src) };
                        self.has_frame = true;
                    }
                    Ok(())
                })();
                // SAFETY: paired with the successful `AcquireNextFrame` above.
                let release_result = unsafe { self.duplication.ReleaseFrame() };
                copy_result?;
                release_result?;
                Ok(if self.has_frame { self.texture.as_ref() } else { None })
            }
            Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => {
                Ok(if self.has_frame { self.texture.as_ref() } else { None })
            }
            Err(e) => Err(e),
        }
    }
}

/// Converts, crops, and scales a BGRA8 GPU texture into a persistent NV12
/// `ID3D11Texture2D` in a single `ID3D11VideoProcessor` pass.
pub struct GpuBgraToNv12 {
    device: ID3D11Device,
    video: ID3D11VideoDevice,
    context: ID3D11VideoContext,
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    src_size: (u32, u32),
    dst_size: (u32, u32),
    output_texture: ID3D11Texture2D,
    output_view: ID3D11VideoProcessorOutputView,
    cached_input: Option<(ID3D11Texture2D, ID3D11VideoProcessorInputView)>,
}

impl std::fmt::Debug for GpuBgraToNv12 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuBgraToNv12")
            .field("src_size", &self.src_size)
            .field("dst_size", &self.dst_size)
            .finish_non_exhaustive()
    }
}

impl GpuBgraToNv12 {
    /// Creates a video processor and persistent NV12 render-target texture on `device`.
    pub fn new(
        device: &ID3D11Device,
        src_size: (u32, u32),
        dst_size: (u32, u32),
    ) -> windows::core::Result<Self> {
        let (sw, sh) = src_size;
        let (dw, dh) = dst_size;
        if sw == 0 || sh == 0 || dw == 0 || dh == 0 || !dw.is_multiple_of(2) || !dh.is_multiple_of(2) {
            return Err(windows::core::Error::from(E_INVALIDARG));
        }
        // SAFETY: D3D11 video processor and texture creation on a live `ID3D11Device`.
        unsafe {
            let video: ID3D11VideoDevice = device.cast()?;
            let context: ID3D11VideoContext = device.GetImmediateContext()?.cast()?;
            let enumerator = video.CreateVideoProcessorEnumerator(&D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
                InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                InputFrameRate: DXGI_RATIONAL { Numerator: 60, Denominator: 1 },
                InputWidth: sw,
                InputHeight: sh,
                OutputFrameRate: DXGI_RATIONAL { Numerator: 60, Denominator: 1 },
                OutputWidth: dw,
                OutputHeight: dh,
                Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
            })?;
            let processor = video.CreateVideoProcessor(&enumerator, 0)?;

            let mut output_texture = None;
            device.CreateTexture2D(
                &D3D11_TEXTURE2D_DESC {
                    Width: dw,
                    Height: dh,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_NV12,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
                    CPUAccessFlags: 0,
                    MiscFlags: 0,
                },
                None,
                Some(&mut output_texture),
            )?;
            let output_texture = output_texture.ok_or_else(|| windows::core::Error::from(E_FAIL))?;

            let mut output_view = None;
            video.CreateVideoProcessorOutputView(
                &output_texture,
                &enumerator,
                &D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                    ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                    Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                        Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
                    },
                },
                Some(&mut output_view),
            )?;
            let output_view = output_view.ok_or_else(|| windows::core::Error::from(E_FAIL))?;

            let full = RECT { left: 0, top: 0, right: dw as i32, bottom: dh as i32 };
            context.VideoProcessorSetStreamFrameFormat(&processor, 0, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE);
            context.VideoProcessorSetStreamAutoProcessingMode(&processor, 0, false);
            context.VideoProcessorSetStreamDestRect(&processor, 0, true, Some(&full));
            context.VideoProcessorSetOutputTargetRect(&processor, true, Some(&full));

            if let Ok(colors) = context.cast::<ID3D11VideoContext1>() {
                colors.VideoProcessorSetStreamColorSpace1(
                    &processor,
                    0,
                    DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709,
                );
                colors.VideoProcessorSetOutputColorSpace1(
                    &processor,
                    DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709,
                );
            } else {
                // Fallback for pre-WDDM 2.0 video contexts: full-range sRGB in, BT.709 studio NV12 out.
                context.VideoProcessorSetStreamColorSpace(
                    &processor,
                    0,
                    &D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: 0x24 },
                );
                context.VideoProcessorSetOutputColorSpace(
                    &processor,
                    &D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: 0x14 },
                );
            }

            Ok(Self {
                device: device.clone(),
                video,
                context,
                enumerator,
                processor,
                src_size,
                dst_size,
                output_texture,
                output_view,
                cached_input: None,
            })
        }
    }

    /// Returns whether this processor already targets `device`, `src_size`, and `dst_size`.
    pub fn matches(&self, device: &ID3D11Device, src_size: (u32, u32), dst_size: (u32, u32)) -> bool {
        self.device.as_raw() == device.as_raw() && self.src_size == src_size && self.dst_size == dst_size
    }

    /// Crops (`src_rect`), scales, and converts `src` into the persistent NV12 output texture.
    pub fn convert(
        &mut self,
        src: &ID3D11Texture2D,
        src_rect: Option<RECT>,
    ) -> windows::core::Result<&ID3D11Texture2D> {
        let (sw, sh) = self.src_size;
        let source_rect = match src_rect {
            Some(r) => RECT {
                left: r.left.clamp(0, sw as i32),
                top: r.top.clamp(0, sh as i32),
                right: r.right.clamp(0, sw as i32),
                bottom: r.bottom.clamp(0, sh as i32),
            },
            None => RECT { left: 0, top: 0, right: sw as i32, bottom: sh as i32 },
        };
        if source_rect.right <= source_rect.left || source_rect.bottom <= source_rect.top {
            return Err(windows::core::Error::from(E_INVALIDARG));
        }

        // SAFETY: `src` lives on `self.device`; keeping the cloned `ID3D11Texture2D`
        // alongside the cached view ensures the raw pointer cannot be recycled while cached.
        unsafe {
            let need_new_view =
                self.cached_input.as_ref().is_none_or(|(cached, _)| cached.as_raw() != src.as_raw());
            if need_new_view {
                let mut input_view = None;
                self.video.CreateVideoProcessorInputView(
                    src,
                    &self.enumerator,
                    &D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
                        FourCC: 0,
                        ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
                        Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                            Texture2D: D3D11_TEX2D_VPIV { MipSlice: 0, ArraySlice: 0 },
                        },
                    },
                    Some(&mut input_view),
                )?;
                let input_view = input_view.ok_or_else(|| windows::core::Error::from(E_FAIL))?;
                self.cached_input = Some((src.clone(), input_view));
            }
            let input_view = self
                .cached_input
                .as_ref()
                .map(|(_, v)| v.clone())
                .ok_or_else(|| windows::core::Error::from(E_FAIL))?;

            self.context.VideoProcessorSetStreamSourceRect(&self.processor, 0, true, Some(&source_rect));
            let mut stream = D3D11_VIDEO_PROCESSOR_STREAM {
                Enable: true.into(),
                pInputSurface: ManuallyDrop::new(Some(input_view)),
                ..Default::default()
            };
            let result = self.context.VideoProcessorBlt(
                &self.processor,
                &self.output_view,
                0,
                std::slice::from_ref(&stream),
            );
            drop(ManuallyDrop::take(&mut stream.pInputSurface));
            result?;
            self.device.GetImmediateContext()?.Flush();
            Ok(&self.output_texture)
        }
    }
}
