// SPDX-License-Identifier: GPL-3.0-or-later
//! Capture a single window on Linux (via in-process X11 `XGetImage`, with `import` fallback),
//! matching the public API of `desktop/app/src/win/window_capture.rs`.

#![allow(unsafe_code)]

use std::process::Command;

use super::browser::{HWND, RECT, frame_of};

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

#[repr(C)]
struct XImageFuncs {
    create_image: *mut std::ffi::c_void,
    destroy_image: Option<unsafe extern "C" fn(*mut XImage) -> std::ffi::c_int>,
    get_pixel: *mut std::ffi::c_void,
    put_pixel: *mut std::ffi::c_void,
    sub_image: *mut std::ffi::c_void,
    add_pixel: *mut std::ffi::c_void,
}

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

type XErrorHandler =
    Option<unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void) -> std::ffi::c_int>;

struct X11Api {
    open_display: unsafe extern "C" fn(*const std::ffi::c_char) -> *mut std::ffi::c_void,
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
    set_error_handler: unsafe extern "C" fn(XErrorHandler) -> XErrorHandler,
    close_display: unsafe extern "C" fn(*mut std::ffi::c_void) -> std::ffi::c_int,
    free: unsafe extern "C" fn(*mut std::ffi::c_void) -> std::ffi::c_int,
}

unsafe extern "C" fn ignore_x11_error(
    _dpy: *mut std::ffi::c_void,
    _err: *mut std::ffi::c_void,
) -> std::ffi::c_int {
    0
}

unsafe fn dl_sym<T: Copy>(lib: *mut std::ffi::c_void, name: &std::ffi::CStr) -> Option<T> {
    let raw = unsafe { libc::dlsym(lib, name.as_ptr()) };
    if raw.is_null() { None } else { Some(unsafe { std::mem::transmute_copy(&raw) }) }
}

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
            get_window_attributes: dl_sym(lib, c"XGetWindowAttributes")?,
            get_image: dl_sym(lib, c"XGetImage")?,
            set_error_handler: dl_sym(lib, c"XSetErrorHandler")?,
            close_display: dl_sym(lib, c"XCloseDisplay")?,
            free: dl_sym(lib, c"XFree")?,
        })
    })
    .as_ref()
}

struct ThreadX11Display(*mut std::ffi::c_void);

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

fn has_executable(name: &str) -> bool {
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path_var).any(|dir| dir.join(name).is_file())
}

/// Whether single-window capture is supported in the current Linux session.
pub fn is_available() -> bool {
    std::env::var_os("DISPLAY").is_some() && (x11_api().is_some() || has_executable("import"))
}

pub struct WindowCapture {
    hwnd: HWND,
    rgba: Vec<u8>,
}

// SAFETY: WindowCapture only holds a window XID and an owned byte buffer.
unsafe impl Send for WindowCapture {}

impl WindowCapture {
    pub fn new(hwnd: HWND) -> Result<WindowCapture, String> {
        if hwnd.0.is_null() {
            return Err("invalid window handle".to_owned());
        }
        if !is_available() {
            return Err("X11 window capture unavailable".to_owned());
        }
        Ok(WindowCapture { hwnd, rgba: Vec::new() })
    }

    pub fn window(&self) -> HWND {
        self.hwnd
    }

    /// Capture `self.hwnd`, optionally cropped to `crop` (in window-relative coordinates),
    /// and scale the result into `bgra` (`dst_w` x `dst_h`, tightly packed BGRA).
    pub fn capture(&mut self, crop: RECT, dst_w: u32, dst_h: u32, bgra: &mut Vec<u8>) -> bool {
        if dst_w == 0 || dst_h == 0 {
            return false;
        }
        let xid = self.hwnd.0 as usize;
        if let Some((src_w, src_h)) = capture_x11_window_bgra(self.hwnd, xid, &mut self.rgba) {
            let (cx0, cy0, cw, ch) = compute_window_crop(crop, src_w, src_h);
            scale_bgra(&self.rgba, src_w, src_w * 4, cx0, cy0, cw, ch, dst_w, dst_h, bgra);
            return true;
        }

        false
    }
}

fn compute_window_crop(crop: RECT, src_w: u32, src_h: u32) -> (u32, u32, u32, u32) {
    let cw = crop.right - crop.left;
    let ch = crop.bottom - crop.top;
    if cw >= 64 && ch >= 64 {
        let x0 = crop.left.clamp(0, src_w as i32) as u32;
        let y0 = crop.top.clamp(0, src_h as i32) as u32;
        let x1 = crop.right.clamp(x0 as i32, src_w as i32) as u32;
        let y1 = crop.bottom.clamp(y0 as i32, src_h as i32) as u32;
        if x1 - x0 >= 64 && y1 - y0 >= 64 {
            return (x0, y0, x1 - x0, y1 - y0);
        }
    }
    (0, 0, src_w, src_h)
}

fn capture_x11_window_in_process(xid: usize, out: &mut Vec<u8>) -> Option<(u32, u32)> {
    with_x11_display(|api, dpy| unsafe {
        // It's process-wide, and a window can vanish at any frame: set it once.
        static ERROR_HANDLER: std::sync::Once = std::sync::Once::new();
        ERROR_HANDLER.call_once(|| {
            let _ = (api.set_error_handler)(Some(ignore_x11_error));
        });
        let mut attrs: XWindowAttributes = std::mem::zeroed();
        if (api.get_window_attributes)(dpy, xid as std::ffi::c_ulong, &mut attrs) == 0
            || attrs.width < 32
            || attrs.height < 32
        {
            return None;
        }
        let src_w = attrs.width as u32;
        let src_h = attrs.height as u32;
        let img = (api.get_image)(
            dpy,
            xid as std::ffi::c_ulong,
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
        let bytes_per_line = (*img).bytes_per_line.max(0) as usize;
        let bits = (*img).bits_per_pixel;
        let bpp = bits.max(0) as usize / 8;
        if (*img).data.is_null() || !matches!(bits, 24 | 32) || bytes_per_line < src_w as usize * bpp {
            if let Some(destroy) = (*img).f.destroy_image {
                let _ = destroy(img);
            } else {
                libc::free((*img).data.cast());
                let _ = (api.free)(img.cast());
            }
            return None;
        }

        let needed = (src_w as usize).checked_mul(src_h as usize)?.checked_mul(4)?;
        out.resize(needed, 0);

        for y in 0..src_h as usize {
            let src_row = (*img).data.add(y * bytes_per_line);
            let dst_row_offset = y * (src_w as usize) * 4;
            for x in 0..src_w as usize {
                let px = src_row.add(x * bpp);
                let dst_idx = dst_row_offset + x * 4;
                out[dst_idx] = *px;
                out[dst_idx + 1] = *px.add(1);
                out[dst_idx + 2] = *px.add(2);
                out[dst_idx + 3] = 0xFF;
            }
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

fn capture_x11_window_bgra(hwnd: HWND, xid: usize, out: &mut Vec<u8>) -> Option<(u32, u32)> {
    if std::env::var_os("DISPLAY").is_none() || xid == 0 {
        return None;
    }
    if let Some(dims) = capture_x11_window_in_process(xid, out) {
        return Some(dims);
    }
    let frame = frame_of(hwnd)?;
    let w = (frame.right - frame.left).max(0) as u32;
    let h = (frame.bottom - frame.top).max(0) as u32;
    if w < 32 || h < 32 {
        return None;
    }
    let expected = (w as usize).checked_mul(h as usize)?.checked_mul(4)?;
    let output = Command::new("import")
        .args(["-silent", "-window", &xid.to_string(), "-depth", "8", "bgra:-"])
        .output()
        .ok()?;
    if !output.status.success() || output.stdout.len() < expected {
        return None;
    }
    out.clear();
    out.extend_from_slice(&output.stdout[..expected]);
    Some((w, h))
}

/// Area-averaging box filter from a cropped BGRA region `(crop_x, crop_y, crop_w, crop_h)`
/// of `src` into a tightly packed `dst_w` x `dst_h` BGRA buffer.
#[allow(clippy::too_many_arguments)]
pub(crate) fn scale_bgra(
    src: &[u8],
    src_w: u32,
    src_pitch: u32,
    crop_x: u32,
    crop_y: u32,
    crop_w: u32,
    crop_h: u32,
    dst_w: u32,
    dst_h: u32,
    dst: &mut Vec<u8>,
) {
    let dst_len = (dst_w as usize) * (dst_h as usize) * 4;
    dst.resize(dst_len, 0);
    if crop_w == 0 || crop_h == 0 || dst_w == 0 || dst_h == 0 || src_w == 0 {
        return;
    }
    let pitch = src_pitch as usize;

    if crop_w == dst_w && crop_h == dst_h {
        let row_bytes = (dst_w as usize) * 4;
        let x_off = (crop_x as usize) * 4;
        for y in 0..dst_h as usize {
            let s_off = (crop_y as usize + y) * pitch + x_off;
            let d_off = y * row_bytes;
            if s_off + row_bytes <= src.len() {
                dst[d_off..d_off + row_bytes].copy_from_slice(&src[s_off..s_off + row_bytes]);
            }
        }
        return;
    }

    for dy in 0..dst_h {
        let sy0 = crop_y + ((dy as u64 * crop_h as u64) / dst_h as u64) as u32;
        let sy1 = (crop_y + (((dy + 1) as u64 * crop_h as u64) / dst_h as u64) as u32).max(sy0 + 1);
        for dx in 0..dst_w {
            let sx0 = crop_x + ((dx as u64 * crop_w as u64) / dst_w as u64) as u32;
            let sx1 = (crop_x + (((dx + 1) as u64 * crop_w as u64) / dst_w as u64) as u32).max(sx0 + 1);
            let mut b_sum = 0u32;
            let mut g_sum = 0u32;
            let mut r_sum = 0u32;
            let mut count = 0u32;
            for sy in sy0..sy1 {
                let row = (sy as usize) * pitch;
                for sx in sx0..sx1 {
                    let idx = row + (sx as usize) * 4;
                    if idx + 4 <= src.len() {
                        b_sum += src[idx] as u32;
                        g_sum += src[idx + 1] as u32;
                        r_sum += src[idx + 2] as u32;
                        count += 1;
                    }
                }
            }
            let out_idx = ((dy * dst_w + dx) as usize) * 4;
            if let (Some(b), Some(g), Some(r)) =
                (b_sum.checked_div(count), g_sum.checked_div(count), r_sum.checked_div(count))
            {
                dst[out_idx] = b as u8;
                dst[out_idx + 1] = g as u8;
                dst[out_idx + 2] = r as u8;
                dst[out_idx + 3] = 0xFF;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_size_is_a_copy() {
        let src = vec![
            10, 20, 30, 255, 40, 50, 60, 255, //
            70, 80, 90, 255, 100, 110, 120, 255,
        ];
        let mut dst = Vec::new();
        scale_bgra(&src, 2, 8, 0, 0, 2, 2, 2, 2, &mut dst);
        assert_eq!(dst, src);
    }

    #[test]
    fn halving_averages_and_a_flat_picture_stays_flat() {
        let mut src = Vec::with_capacity(4 * 4 * 4);
        for _ in 0..16 {
            src.extend_from_slice(&[50, 100, 150, 255]);
        }
        let mut dst = Vec::new();
        scale_bgra(&src, 4, 16, 0, 0, 4, 4, 2, 2, &mut dst);
        assert_eq!(dst.len(), 2 * 2 * 4);
        for px in dst.as_chunks::<4>().0 {
            assert_eq!(px, &[50, 100, 150, 255]);
        }
    }

    #[test]
    fn compute_window_crop_uses_window_relative_coordinates() {
        let crop = RECT { left: 100, top: 80, right: 740, bottom: 440 };
        assert_eq!(compute_window_crop(crop, 1280, 720), (100, 80, 640, 360));
        let tiny = RECT { left: 10, top: 10, right: 30, bottom: 30 };
        assert_eq!(compute_window_crop(tiny, 1280, 720), (0, 0, 1280, 720));
    }
}
