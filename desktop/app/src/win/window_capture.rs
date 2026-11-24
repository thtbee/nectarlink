// SPDX-License-Identifier: GPL-3.0-or-later
//! One window's own pixels, with Windows Graphics Capture: what the window
//! shows even with other windows in front of it, and never theirs. Play on
//! phone streams a player's window this way, cropped to its video.

use windows::{
    Graphics::{
        Capture::{
            Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureAccess,
            GraphicsCaptureAccessKind, GraphicsCaptureItem, GraphicsCaptureSession,
        },
        DirectX::{Direct3D11::IDirect3DDevice, DirectXPixelFormat},
        SizeInt32,
    },
    Win32::{
        Foundation::{HWND, RECT},
        Graphics::{
            Direct3D::D3D_DRIVER_TYPE_HARDWARE,
            Direct3D11::{
                D3D11_BIND_SHADER_RESOURCE, D3D11_BOX, D3D11_CPU_ACCESS_READ,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_MAP_READ,
                D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
                D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Multithread,
                ID3D11Texture2D,
            },
            Dxgi::{
                Common::DXGI_FORMAT_B8G8R8A8_UNORM, Common::DXGI_SAMPLE_DESC, IDXGIAdapter1, IDXGIDevice,
            },
        },
        System::{
            Com::{COINIT_MULTITHREADED, CoInitializeEx},
            WinRT::{
                Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess},
                Graphics::Capture::IGraphicsCaptureItemInterop,
            },
        },
    },
    core::{Interface, factory},
};

/// Frames the pool holds: one being read, one arriving.
const BUFFERS: i32 = 2;

/// A running capture of one window.
pub struct WindowCapture {
    hwnd: HWND,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    winrt_device: IDirect3DDevice,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    size: SizeInt32,
    staging: Option<(ID3D11Texture2D, u32, u32)>,
    gpu_texture: Option<(ID3D11Texture2D, u32, u32)>,
    /// The latest crop, kept for when no new frame came (nothing changed).
    last: Vec<u8>,
    last_size: (u32, u32),
    columns: Vec<(usize, u32)>,
}

/// Latest cropped frame surface and bounds returned by `WindowCapture::next_cropped_frame`.
struct CroppedFrame {
    _frame: Direct3D11CaptureFrame,
    texture: ID3D11Texture2D,
    area: D3D11_BOX,
    width: u32,
    height: u32,
}

impl WindowCapture {
    /// Starts capturing `hwnd`, without the pointer and, where Windows
    /// allows it, without the yellow capture border.
    pub fn new(hwnd: HWND) -> windows::core::Result<WindowCapture> {
        // SAFETY: COM for this thread (WinRT needs it; a second call is harmless).
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if !GraphicsCaptureSession::IsSupported()? {
            return Err(windows::core::Error::from(windows::Win32::Foundation::E_NOTIMPL));
        }
        let mut device = None;
        let mut context = None;
        // SAFETY: plain Direct3D and WinRT interop calls with owned values.
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                Default::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .or_else(|_| {
                device = None;
                context = None;
                D3D11CreateDevice(
                    None,
                    D3D_DRIVER_TYPE_HARDWARE,
                    Default::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    Some(&mut context),
                )
            })?;
        }
        let device: ID3D11Device =
            device.ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_FAIL))?;
        let context =
            context.ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_FAIL))?;
        if let Ok(mt) = device.cast::<ID3D11Multithread>() {
            // SAFETY: enables thread-safe access when sharing the D3D11 device with Media Foundation.
            unsafe {
                let _ = mt.SetMultithreadProtected(true);
            }
        }
        // SAFETY: as above.
        let (winrt_device, item) = unsafe {
            let dxgi: IDXGIDevice = device.cast()?;
            let winrt_device: IDirect3DDevice = CreateDirect3D11DeviceFromDXGIDevice(&dxgi)?.cast()?;
            let interop = factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
            let item: GraphicsCaptureItem = interop.CreateForWindow(hwnd)?;
            (winrt_device, item)
        };
        let size = item.Size()?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &winrt_device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            BUFFERS,
            size,
        )?;
        let session = pool.CreateCaptureSession(&item)?;
        let _ = session.SetIsCursorCaptureEnabled(false);
        // Windows 11 can leave the border off; earlier versions always draw it.
        if GraphicsCaptureAccess::RequestAccessAsync(GraphicsCaptureAccessKind::Borderless)
            .and_then(|asked| asked.join())
            .is_ok()
        {
            let _ = session.SetIsBorderRequired(false);
        }
        session.StartCapture()?;
        Ok(WindowCapture {
            hwnd,
            device,
            context,
            winrt_device,
            pool,
            session,
            size,
            staging: None,
            gpu_texture: None,
            last: Vec::new(),
            last_size: (0, 0),
            columns: Vec::new(),
        })
    }

    /// The window this captures.
    pub fn window(&self) -> HWND {
        self.hwnd
    }

    /// The D3D11 device capturing this window.
    pub fn device(&self) -> &ID3D11Device {
        &self.device
    }

    /// The DXGI adapter LUID backing `device()`.
    pub fn adapter_luid(&self) -> Option<u64> {
        // SAFETY: queries DXGI adapter description from the live D3D11 device.
        unsafe {
            let dxgi: IDXGIDevice = self.device.cast().ok()?;
            let adapter = dxgi.GetAdapter().ok()?;
            let adapter1: IDXGIAdapter1 = adapter.cast().ok()?;
            let desc = adapter1.GetDesc1().ok()?;
            Some(u64::from(desc.AdapterLuid.LowPart) | ((desc.AdapterLuid.HighPart as u64) << 32))
        }
    }

    /// The window's latest picture of `crop` (in the window's own pixels,
    /// from its top-left) copied into a reusable GPU `DEFAULT` texture, along
    /// with its `ID3D11Device`. Reuses the previous texture when no new frame
    /// arrived; returns `None` before the first frame.
    pub fn latest_texture(&mut self, crop: RECT) -> Option<(&ID3D11Device, &ID3D11Texture2D)> {
        let update = (|| -> windows::core::Result<()> {
            let Some(CroppedFrame { _frame, texture, area, width: w, height: h }) =
                self.next_cropped_frame(crop)?
            else {
                return Ok(());
            };
            let dst = ensure_texture(&self.device, &mut self.gpu_texture, w, h, false)?;
            // SAFETY: `_frame` keeps the pool surface alive while copying the cropped box.
            unsafe {
                self.context.CopySubresourceRegion(&dst, 0, 0, 0, 0, &texture, 0, Some(&area));
            }
            Ok(())
        })();
        if let Err(e) = update {
            tracing::debug!(error = %e, "window capture GPU frame failed");
        }
        self.gpu_texture.as_ref().map(|(tex, _, _)| (&self.device, tex))
    }

    /// The window's latest picture of `crop` (in the window's own pixels,
    /// from its top-left), scaled to `dst_w` × `dst_h` into `bgra`. Without
    /// a new frame (nothing changed on screen), the last one again. False
    /// before the first frame.
    pub fn capture(&mut self, crop: RECT, dst_w: u32, dst_h: u32, bgra: &mut Vec<u8>) -> bool {
        if let Err(e) = self.latest(crop) {
            tracing::debug!(error = %e, "window capture frame failed");
        }
        let (w, h) = self.last_size;
        if w == 0 || h == 0 {
            return false;
        }
        scale_bgra_into(&self.last, w, h, dst_w, dst_h, bgra, &mut self.columns);
        true
    }

    fn next_cropped_frame(&mut self, crop: RECT) -> windows::core::Result<Option<CroppedFrame>> {
        // Only the newest matters: older frames are dropped.
        let mut newest = None;
        while let Ok(frame) = self.pool.TryGetNextFrame() {
            newest = Some(frame);
        }
        let Some(frame) = newest else { return Ok(None) };
        let content = frame.ContentSize()?;
        if content.Width != self.size.Width || content.Height != self.size.Height {
            // The window changed size: frames come at the new size from the next one.
            self.size = content;
            self.pool.Recreate(
                &self.winrt_device,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                BUFFERS,
                content,
            )?;
        }
        // SAFETY: COM calls on live interfaces; `_frame` keeps the surface alive for the caller.
        unsafe {
            let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
            let texture: ID3D11Texture2D = access.GetInterface()?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut desc);
            let (tw, th) =
                (desc.Width.min(content.Width.max(0) as u32), desc.Height.min(content.Height.max(0) as u32));
            let left = crop.left.clamp(0, tw as i32) as u32;
            let top = crop.top.clamp(0, th as i32) as u32;
            let right = crop.right.clamp(0, tw as i32) as u32;
            let bottom = crop.bottom.clamp(0, th as i32) as u32;
            if right <= left || bottom <= top {
                return Ok(None);
            }
            let (width, height) = (right - left, bottom - top);
            let area = D3D11_BOX { left, top, front: 0, right, bottom, back: 1 };
            Ok(Some(CroppedFrame { _frame: frame, texture, area, width, height }))
        }
    }

    /// Reads the newest frame, if one came, into `last`.
    fn latest(&mut self, crop: RECT) -> windows::core::Result<()> {
        let Some(CroppedFrame { _frame, texture, area, width: w, height: h }) =
            self.next_cropped_frame(crop)?
        else {
            return Ok(());
        };
        // SAFETY: COM calls on live interfaces; the staging texture is
        // mapped for reading and unmapped before it's used again.
        unsafe {
            let staging = ensure_texture(&self.device, &mut self.staging, w, h, true)?;
            self.context.CopySubresourceRegion(&staging, 0, 0, 0, 0, &texture, 0, Some(&area));
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            let row = w as usize * 4;
            self.last.resize(row * h as usize, 0);
            let source =
                std::slice::from_raw_parts(mapped.pData.cast::<u8>(), mapped.RowPitch as usize * h as usize);
            for (y, line) in self.last.chunks_exact_mut(row).enumerate() {
                line.copy_from_slice(&source[y * mapped.RowPitch as usize..][..row]);
            }
            self.context.Unmap(&staging, 0);
            self.last_size = (w, h);
        }
        Ok(())
    }
}

/// Allocates or reuses a `w` × `h` BGRA texture (`staging` for CPU readback or `DEFAULT` for GPU use).
fn ensure_texture(
    device: &ID3D11Device,
    slot: &mut Option<(ID3D11Texture2D, u32, u32)>,
    w: u32,
    h: u32,
    staging: bool,
) -> windows::core::Result<ID3D11Texture2D> {
    if let Some((texture, sw, sh)) = slot
        && (*sw, *sh) == (w, h)
    {
        return Ok(texture.clone());
    }
    let desc = D3D11_TEXTURE2D_DESC {
        Width: w,
        Height: h,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: if staging { D3D11_USAGE_STAGING } else { D3D11_USAGE_DEFAULT },
        BindFlags: if staging { 0 } else { D3D11_BIND_SHADER_RESOURCE.0 as u32 },
        CPUAccessFlags: if staging { D3D11_CPU_ACCESS_READ.0 as u32 } else { 0 },
        MiscFlags: 0,
    };
    let mut texture = None;
    // SAFETY: creates a texture from a filled-in description.
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
    let texture = texture.ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_FAIL))?;
    *slot = Some((texture.clone(), w, h));
    Ok(texture)
}

impl Drop for WindowCapture {
    fn drop(&mut self) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

/// Scales a `w` × `h` BGRA picture to `dst_w` × `dst_h` (bilinear), or
/// copies it when the size is the same, reusing `columns` across frames.
fn scale_bgra_into(
    src: &[u8],
    w: u32,
    h: u32,
    dst_w: u32,
    dst_h: u32,
    out: &mut Vec<u8>,
    columns: &mut Vec<(usize, u32)>,
) {
    let (w, h, dw, dh) = (w as usize, h as usize, dst_w as usize, dst_h as usize);
    out.resize(dw * dh * 4, 0);
    // Nothing to scale from (or to): black, rather than a panic.
    if w == 0 || h == 0 || dw == 0 || dh == 0 || src.len() < w * h * 4 {
        out.fill(0);
        return;
    }
    if (w, h) == (dw, dh) {
        out.copy_from_slice(&src[..w * h * 4]);
        return;
    }
    // Where each output column samples from: left pixel and weight (of 256).
    columns.clear();
    columns.extend((0..dw).map(|x| {
        let at = ((x as f32 + 0.5) * w as f32 / dw as f32 - 0.5).clamp(0.0, (w - 1) as f32);
        (at as usize, ((at - at.floor()) * 256.0) as u32)
    }));
    for (y, line) in out.chunks_exact_mut(dw * 4).enumerate() {
        let at = ((y as f32 + 0.5) * h as f32 / dh as f32 - 0.5).clamp(0.0, (h - 1) as f32);
        let (y0, wy) = (at as usize, ((at - at.floor()) * 256.0) as u32);
        let y1 = (y0 + 1).min(h - 1);
        let (top, bottom) = (&src[y0 * w * 4..][..w * 4], &src[y1 * w * 4..][..w * 4]);
        for (x, pixel) in line.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let (x0, wx) = columns[x];
            let x1 = (x0 + 1).min(w - 1);
            for c in 0..4 {
                let row = |r: &[u8]| u32::from(r[x0 * 4 + c]) * (256 - wx) + u32::from(r[x1 * 4 + c]) * wx;
                let value = (row(top) * (256 - wy) + row(bottom) * wy + (1 << 15)) >> 16;
                pixel[c] = value as u8;
            }
        }
    }
}

#[cfg(test)]
pub(crate) fn scale_bgra(src: &[u8], w: u32, h: u32, dst_w: u32, dst_h: u32, out: &mut Vec<u8>) {
    scale_bgra_into(src, w, h, dst_w, dst_h, out, &mut Vec::new());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_size_is_a_copy() {
        let src: Vec<u8> = (0..4 * 3 * 4).map(|i| i as u8).collect();
        let mut out = Vec::new();
        scale_bgra(&src, 4, 3, 4, 3, &mut out);
        assert_eq!(out, src);
    }

    #[test]
    fn halving_averages_and_a_flat_picture_stays_flat() {
        // 4x2: left half black, right half white.
        let mut src = Vec::new();
        for _ in 0..2 {
            for x in 0..4 {
                let v = if x < 2 { 0 } else { 255 };
                src.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let mut out = Vec::new();
        scale_bgra(&src, 4, 2, 2, 1, &mut out);
        assert_eq!(out.len(), 2 * 4);
        assert!(out[0] < 40 && out[4] > 215, "{out:?}");
        assert_eq!(out[3], 255);

        let flat = vec![77u8; 10 * 6 * 4];
        scale_bgra(&flat, 10, 6, 7, 3, &mut out);
        assert!(out.iter().all(|&v| v == 77));
    }
    #[test]
    fn scaling_nothing_gives_black_not_a_panic() {
        let mut out = Vec::new();
        scale_bgra(&[], 0, 0, 4, 2, &mut out);
        assert_eq!(out, vec![0; 32]);
        scale_bgra(&[1; 8], 4, 2, 4, 2, &mut out);
        assert_eq!(out, vec![0; 32], "a short buffer");
        scale_bgra(&[1; 32], 4, 2, 0, 0, &mut out);
        assert!(out.is_empty());
    }

    /// With a Chromium browser showing a page whose title has
    /// `NECTARLINK_TEST_TAB` in it: captures its window's video, covered
    /// or not, and writes it to `NECTARLINK_TEST_DUMP` (raw BGRA) if set.
    #[test]
    #[ignore = "needs a browser tab titled NECTARLINK_TEST_TAB"]
    fn captures_a_window_behind_others() {
        use crate::win::browser::{Automation, frame_of, use_physical_pixels, window_for};
        let wanted = std::env::var("NECTARLINK_TEST_TAB").expect("NECTARLINK_TEST_TAB");
        // SAFETY: COM for this test's thread.
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        use_physical_pixels();
        let window = window_for("Chrome", &wanted).expect("the test tab's window");
        let video = Automation::new().unwrap().video_rect(&window).expect("the video");
        let frame = frame_of(window.hwnd).unwrap();
        let crop = RECT {
            left: video.left - frame.left,
            top: video.top - frame.top,
            right: video.right - frame.left,
            bottom: video.bottom - frame.top,
        };
        let (w, h) = ((crop.right - crop.left) as u32, (crop.bottom - crop.top) as u32);
        let mut capture = WindowCapture::new(window.hwnd).unwrap();
        let mut bgra = Vec::new();
        let started = std::time::Instant::now();
        while !capture.capture(crop, w, h, &mut bgra) {
            assert!(started.elapsed() < std::time::Duration::from_secs(3), "no frame");
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        let distinct: std::collections::HashSet<&[u8; 4]> = bgra.as_chunks::<4>().0.iter().collect();
        eprintln!("{crop:?} {w}x{h}: {} colors", distinct.len());
        if let Ok(path) = std::env::var("NECTARLINK_TEST_DUMP") {
            std::fs::write(path, &bgra).unwrap();
        }
        assert!(distinct.len() > 50);
    }
}
