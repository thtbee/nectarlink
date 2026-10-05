// SPDX-License-Identifier: GPL-3.0-or-later
//! The app's mark as an image file, for places that want one (the
//! notification sender icon).

use crate::mark::render;

/// Writes the mark as a PNG file (for places that want an image file, such
/// as the notification sender icon).
pub fn write_png(path: &std::path::Path, size: u32) -> windows::core::Result<()> {
    use windows::{
        Win32::{
            Foundation::GENERIC_WRITE,
            Graphics::Imaging::{
                CLSID_WICImagingFactory, GUID_ContainerFormatPng, GUID_WICPixelFormat32bppBGRA,
                IWICImagingFactory, WICBitmapEncoderNoCache,
            },
            System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
        },
        core::HSTRING,
    };
    let rgba = render(size as usize);
    let bgra: Vec<u8> = rgba.as_chunks::<4>().0.iter().flat_map(|&[r, g, b, a]| [b, g, r, a]).collect();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| windows::core::Error::new(windows::core::HRESULT(-1), e.to_string()))?;
    }
    // SAFETY: plain WIC calls on live interfaces; the pixel buffer holds
    // size * size BGRA pixels, matching the frame size and stride.
    unsafe {
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let stream = factory.CreateStream()?;
        stream.InitializeFromFilename(&HSTRING::from(path.as_os_str()), GENERIC_WRITE.0)?;
        let encoder = factory.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let mut frame = None;
        encoder.CreateNewFrame(&mut frame, std::ptr::null_mut())?;
        let frame = frame.ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_FAIL))?;
        frame.Initialize(None)?;
        frame.SetSize(size, size)?;
        let mut format = GUID_WICPixelFormat32bppBGRA;
        frame.SetPixelFormat(&mut format)?;
        frame.WritePixels(size, size * 4, &bgra)?;
        frame.Commit()?;
        encoder.Commit()
    }
}
