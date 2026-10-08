// SPDX-License-Identifier: GPL-3.0-or-later
//! Shared-memory frame transport between `nectarlink-desktop.exe` and
//! `nectarlink_vcam.dll`, plus placeholder frame rendering and RGB32/NV12
//! conversion.

use std::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering};

use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, HLOCAL, INVALID_HANDLE_VALUE, LocalFree},
        Security::{
            Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1},
            PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
        },
        System::Memory::{
            CreateFileMappingW, FILE_MAP_READ, FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile,
            OpenFileMappingW, PAGE_READWRITE, UnmapViewOfFile,
        },
    },
    core::{PCWSTR, w},
};

/// The mapping the camera and the app share. The camera's media source runs
/// inside Windows' Frame Server service, in another session than the app, so
/// it has to be in the global namespace: the service creates it (only
/// services may create global objects) and the signed-in user's app opens it.
pub const DEFAULT_MAPPING_NAME: &str = "Global\\NectarlinkVcamFrame_v1";
/// The same mapping in the caller's own session, when the global one can't be
/// created (the media source running in-process, as in tests).
pub const LOCAL_MAPPING_NAME: &str = "Local\\NectarlinkVcamFrame_v1";

/// Who may use the global mapping: the system, the Frame Server's
/// LocalService account and administrators fully, signed-in (interactive)
/// users read and write (the app), nobody else.
const GLOBAL_SDDL: PCWSTR = w!("D:P(A;;GA;;;SY)(A;;GA;;;LS)(A;;GA;;;BA)(A;;GRGW;;;IU)");

pub const MAX_WIDTH: u32 = 1920;
pub const MAX_HEIGHT: u32 = 1080;
pub const MAX_FRAME_BYTES: usize = (MAX_WIDTH as usize) * (MAX_HEIGHT as usize) * 4;
pub const HEADER_BYTES: usize = std::mem::size_of::<SharedHeader>();
pub const TOTAL_MAPPING_BYTES: usize = HEADER_BYTES + MAX_FRAME_BYTES;

const MAGIC: u32 = 0x4E4C_5643; // "NLVC"
const VERSION: u32 = 1;

#[repr(C, align(64))]
#[derive(Debug)]
pub struct SharedHeader {
    pub magic: AtomicU32,
    pub version: AtomicU32,
    /// Even when consistent, odd while a writer is updating the buffer.
    pub seq: AtomicU64,
    /// `1` while a live camera stream is active, `0` when idle.
    pub active: AtomicU32,
    pub width: AtomicU32,
    pub height: AtomicU32,
    pub stride: AtomicU32,
    pub _pad: AtomicU32,
    pub timestamp_us: AtomicU64,
    pub phone_name: [AtomicU8; 24],
}

/// Shared-memory mapping handle and view. Safe to share across threads.
pub struct SharedFrameMapping {
    handle: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
}

#[allow(unsafe_code)]
// SAFETY: the underlying OS file mapping handle and mapped view address are
// process-wide and synchronized via atomic operations in `SharedHeader`.
unsafe impl Send for SharedFrameMapping {}
#[allow(unsafe_code)]
unsafe impl Sync for SharedFrameMapping {}

impl std::fmt::Debug for SharedFrameMapping {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedFrameMapping").field("view", &self.view.Value).finish()
    }
}

impl SharedFrameMapping {
    /// Opens or creates the shared-memory frame mapping with `name` (a
    /// `Global\` name gets [`GLOBAL_SDDL`]'s permissions).
    #[allow(unsafe_code)]
    pub fn open_or_create(name: &str) -> windows::core::Result<Self> {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let mut sd = PSECURITY_DESCRIPTOR::default();
        if name.starts_with("Global\\") {
            // SAFETY: a constant SDDL string; the descriptor is freed below.
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    GLOBAL_SDDL,
                    SDDL_REVISION_1,
                    &mut sd,
                    None,
                )?;
            }
        }
        let attrs = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd.0,
            bInheritHandle: false.into(),
        };
        // SAFETY: `CreateFileMappingW` with `INVALID_HANDLE_VALUE` creates or opens
        // a pagefile-backed named mapping of `TOTAL_MAPPING_BYTES` bytes; `attrs`
        // and the descriptor it points to outlive the call.
        let handle = unsafe {
            let handle = CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                (!sd.0.is_null()).then_some(&raw const attrs),
                PAGE_READWRITE,
                0,
                TOTAL_MAPPING_BYTES as u32,
                PCWSTR(wide.as_ptr()),
            );
            if !sd.0.is_null() {
                let _ = LocalFree(Some(HLOCAL(sd.0)));
            }
            handle?
        };
        Self::map(handle)
    }

    /// For the camera's media source: the global mapping, or the caller's
    /// session's when it may not create global objects (in-process tests).
    pub fn for_camera(name: &str) -> windows::core::Result<Self> {
        if name == DEFAULT_MAPPING_NAME {
            Self::open_or_create(DEFAULT_MAPPING_NAME).or_else(|_| Self::open_or_create(LOCAL_MAPPING_NAME))
        } else {
            Self::open_or_create(name)
        }
    }

    /// Opens an existing mapping (the app joining the camera's global one).
    #[allow(unsafe_code)]
    pub fn open_existing(name: &str) -> windows::core::Result<Self> {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: opens a named mapping by a NUL-terminated name.
        let handle =
            unsafe { OpenFileMappingW((FILE_MAP_READ | FILE_MAP_WRITE).0, false, PCWSTR(wide.as_ptr()))? };
        Self::map(handle)
    }

    #[allow(unsafe_code)]
    fn map(handle: HANDLE) -> windows::core::Result<Self> {
        // SAFETY: maps the whole section, which is `TOTAL_MAPPING_BYTES` long
        // whichever side created it (both use the same size).
        unsafe {
            // Read and write only: that's all the app is granted on the global one.
            let view = MapViewOfFile(handle, FILE_MAP_READ | FILE_MAP_WRITE, 0, 0, TOTAL_MAPPING_BYTES);
            if view.Value.is_null() {
                let err = windows::core::Error::from_thread();
                let _ = CloseHandle(handle);
                return Err(err);
            }
            let mapping = Self { handle, view };
            mapping.ensure_initialized();
            Ok(mapping)
        }
    }

    #[allow(unsafe_code)]
    fn header(&self) -> &SharedHeader {
        // SAFETY: `self.view.Value` points to a valid mapping of `TOTAL_MAPPING_BYTES`
        // aligned to page boundaries (>= 64 bytes).
        unsafe { &*(self.view.Value as *const SharedHeader) }
    }

    #[allow(unsafe_code)]
    fn payload_ptr(&self) -> *mut u8 {
        // SAFETY: `HEADER_BYTES` is within `TOTAL_MAPPING_BYTES`.
        unsafe { (self.view.Value as *mut u8).add(HEADER_BYTES) }
    }

    fn ensure_initialized(&self) {
        let h = self.header();
        if h.magic.load(Ordering::Acquire) != MAGIC {
            h.version.store(VERSION, Ordering::Relaxed);
            h.magic.store(MAGIC, Ordering::Release);
        }
    }

    fn store_phone_name(&self, name: &str) {
        let h = self.header();
        let bytes = name.as_bytes();
        let mut len = bytes.len().min(h.phone_name.len());
        while len > 0 && !name.is_char_boundary(len) {
            len -= 1;
        }
        for (i, slot) in h.phone_name.iter().enumerate() {
            let b = if i < len { bytes[i] } else { 0 };
            slot.store(b, Ordering::Relaxed);
        }
    }

    pub fn load_phone_name(&self) -> String {
        let h = self.header();
        let mut bytes = Vec::with_capacity(h.phone_name.len());
        for slot in &h.phone_name {
            let b = slot.load(Ordering::Relaxed);
            if b == 0 {
                break;
            }
            bytes.push(b);
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// Marks the webcam stream as idle (placeholder shown) and updates the
    /// phone name displayed on the placeholder card.
    pub fn set_idle(&self, phone_name: &str) {
        let h = self.header();
        let seq = h.seq.load(Ordering::Relaxed);
        let odd = if seq & 1 == 0 { seq.wrapping_add(1) } else { seq };
        h.seq.store(odd, Ordering::Release);
        self.store_phone_name(phone_name);
        h.active.store(0, Ordering::Release);
        h.seq.store(odd.wrapping_add(1), Ordering::Release);
    }

    /// Writes a decoded top-down BGRX frame into the shared mapping.
    /// If `width > MAX_WIDTH` or `height > MAX_HEIGHT`, scales down to fit
    /// `1920x1080`. Optionally mirrors horizontally when `mirror` is true.
    #[allow(unsafe_code)]
    pub fn write_frame(
        &self,
        phone_name: &str,
        width: u32,
        height: u32,
        timestamp_us: u64,
        bgrx: &[u8],
        mirror: bool,
    ) {
        if width == 0 || height == 0 || bgrx.len() < (width as usize) * (height as usize) * 4 {
            return;
        }
        let dst_w = width.min(MAX_WIDTH);
        let dst_h = height.min(MAX_HEIGHT);
        let dst_stride = dst_w * 4;
        let dst_len = (dst_stride as usize) * (dst_h as usize);

        let h = self.header();
        let seq = h.seq.load(Ordering::Relaxed);
        let odd = if seq & 1 == 0 { seq.wrapping_add(1) } else { seq };
        h.seq.store(odd, Ordering::Release);
        std::sync::atomic::fence(Ordering::AcqRel);

        self.store_phone_name(phone_name);
        h.width.store(dst_w, Ordering::Relaxed);
        h.height.store(dst_h, Ordering::Relaxed);
        h.stride.store(dst_stride, Ordering::Relaxed);
        h.timestamp_us.store(timestamp_us, Ordering::Relaxed);

        // SAFETY: writer holds the seqlock (`seq` is odd) and writes at most
        // `MAX_FRAME_BYTES` into the payload region.
        let dst = unsafe { std::slice::from_raw_parts_mut(self.payload_ptr(), dst_len) };
        if dst_w == width && dst_h == height && !mirror {
            dst.copy_from_slice(&bgrx[..dst_len]);
        } else {
            scale_or_mirror_bgrx(bgrx, width, height, dst, dst_w, dst_h, mirror);
        }

        h.active.store(1, Ordering::Release);
        h.seq.store(odd.wrapping_add(1), Ordering::Release);
    }

    /// Reads the latest frame into `out_bgrx` (`out_width * out_height * 4` bytes).
    /// Returns `(true, timestamp_us)` when a live frame was read, or
    /// `(false, 0)` when idle (in which case `out_bgrx` is filled with the
    /// placeholder frame).
    #[allow(unsafe_code)]
    pub fn read_bgrx(&self, out_width: u32, out_height: u32, out_bgrx: &mut [u8]) -> (bool, u64) {
        let needed = (out_width as usize) * (out_height as usize) * 4;
        if out_bgrx.len() < needed || out_width == 0 || out_height == 0 {
            return (false, 0);
        }
        let h = self.header();
        if h.magic.load(Ordering::Acquire) == MAGIC && h.active.load(Ordering::Acquire) == 1 {
            for _ in 0..3 {
                let seq1 = h.seq.load(Ordering::Acquire);
                if seq1 == 0 || (seq1 & 1) != 0 {
                    std::hint::spin_loop();
                    continue;
                }
                let src_w = h.width.load(Ordering::Relaxed);
                let src_h = h.height.load(Ordering::Relaxed);
                let ts = h.timestamp_us.load(Ordering::Relaxed);
                let src_len = (src_w as usize).saturating_mul(src_h as usize).saturating_mul(4);
                if src_w > 0 && src_h > 0 && src_len <= MAX_FRAME_BYTES {
                    // SAFETY: `src_len <= MAX_FRAME_BYTES`; seqlock verifies consistency.
                    let src = unsafe { std::slice::from_raw_parts(self.payload_ptr(), src_len) };
                    if src_w == out_width && src_h == out_height {
                        out_bgrx[..needed].copy_from_slice(src);
                    } else {
                        fit_bgrx_aspect(src, src_w, src_h, &mut out_bgrx[..needed], out_width, out_height);
                    }
                    std::sync::atomic::fence(Ordering::Acquire);
                    let seq2 = h.seq.load(Ordering::Acquire);
                    if seq1 == seq2 {
                        return (true, ts);
                    }
                }
            }
        }
        let phone = self.load_phone_name();
        render_placeholder_bgrx(&mut out_bgrx[..needed], out_width, out_height, &phone);
        (false, 0)
    }
}

impl Drop for SharedFrameMapping {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: paired with `MapViewOfFile` and `CreateFileMappingW`.
        unsafe {
            let _ = UnmapViewOfFile(self.view);
            let _ = CloseHandle(self.handle);
        }
    }
}

fn scale_or_mirror_bgrx(
    src: &[u8],
    src_w: u32,
    src_h: u32,
    dst: &mut [u8],
    dst_w: u32,
    dst_h: u32,
    mirror: bool,
) {
    let sw = src_w as usize;
    let sh = src_h as usize;
    let dw = dst_w as usize;
    let dh = dst_h as usize;
    for y in 0..dh {
        let sy = (y * sh / dh).min(sh - 1);
        let src_row = &src[sy * sw * 4..(sy + 1) * sw * 4];
        let dst_row = &mut dst[y * dw * 4..(y + 1) * dw * 4];
        for x in 0..dw {
            let sx = (x * sw / dw).min(sw - 1);
            let sx = if mirror { sw - 1 - sx } else { sx };
            dst_row[x * 4..x * 4 + 4].copy_from_slice(&src_row[sx * 4..sx * 4 + 4]);
        }
    }
}

/// Scales `src` (`src_w x src_h`) into `dst` (`dst_w x dst_h`) preserving aspect
/// ratio with dark pillarbox/letterbox bars when aspect ratios differ (e.g.
/// portrait phone camera in a 16:9 webcam frame).
pub fn fit_bgrx_aspect(src: &[u8], src_w: u32, src_h: u32, dst: &mut [u8], dst_w: u32, dst_h: u32) {
    let sw = src_w as usize;
    let sh = src_h as usize;
    let dw = dst_w as usize;
    let dh = dst_h as usize;
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 {
        return;
    }
    // Fill background with dark neutral (#101216) when letterboxing/pillarboxing.
    let (fit_w, fit_h) = if sw * dh >= sh * dw {
        (dw, (dw * sh / sw).max(1).min(dh))
    } else {
        ((dh * sw / sh).max(1).min(dw), dh)
    };
    let off_x = (dw - fit_w) / 2;
    let off_y = (dh - fit_h) / 2;
    if off_x > 0 || off_y > 0 {
        for px in dst.as_chunks_mut::<4>().0 {
            px.copy_from_slice(&[0x16, 0x12, 0x10, 0xFF]);
        }
    }
    for y in 0..fit_h {
        let sy = (y * sh / fit_h).min(sh - 1);
        let src_row = &src[sy * sw * 4..(sy + 1) * sw * 4];
        let dst_row = &mut dst[(off_y + y) * dw * 4..(off_y + y + 1) * dw * 4];
        for x in 0..fit_w {
            let sx = (x * sw / fit_w).min(sw - 1);
            let dx = (off_x + x) * 4;
            dst_row[dx..dx + 4].copy_from_slice(&src_row[sx * 4..sx * 4 + 4]);
        }
    }
}

/// Converts top-down BGRX (`width * height * 4`) to NV12 (`width * height * 3 / 2`,
/// BT.709 limited range).
pub fn bgrx_to_nv12(bgrx: &[u8], width: u32, height: u32, nv12: &mut [u8]) {
    let w = width as usize;
    let h = height as usize;
    let y_size = w * h;
    let total = y_size + (w * (h / 2));
    if bgrx.len() < w * h * 4 || nv12.len() < total {
        return;
    }
    let (y_plane, uv_plane) = nv12[..total].split_at_mut(y_size);
    for y in 0..h {
        let row = &bgrx[y * w * 4..(y + 1) * w * 4];
        let y_row = &mut y_plane[y * w..(y + 1) * w];
        for x in 0..w {
            let b = i32::from(row[x * 4]);
            let g = i32::from(row[x * 4 + 1]);
            let r = i32::from(row[x * 4 + 2]);
            let luma = ((47 * r + 157 * g + 16 * b + 128) >> 8) + 16;
            y_row[x] = luma.clamp(16, 235) as u8;
        }
        if y % 2 == 0 && (y / 2) * w < uv_plane.len() {
            let uv_row = &mut uv_plane[(y / 2) * w..((y / 2) + 1) * w];
            for x in (0..w).step_by(2) {
                let b = i32::from(row[x * 4]);
                let g = i32::from(row[x * 4 + 1]);
                let r = i32::from(row[x * 4 + 2]);
                let u = ((-26 * r - 87 * g + 112 * b + 128) >> 8) + 128;
                let v = ((112 * r - 102 * g - 10 * b + 128) >> 8) + 128;
                uv_row[x] = u.clamp(16, 240) as u8;
                if x + 1 < w {
                    uv_row[x + 1] = v.clamp(16, 240) as u8;
                }
            }
        }
    }
}

/// Renders a clean placeholder frame ("NECTARLINK WEBCAM" / "START THE WEBCAM ON <PHONE>")
/// into `bgrx` (`width * height * 4`).
pub fn render_placeholder_bgrx(bgrx: &mut [u8], width: u32, height: u32, phone_name: &str) {
    let w = width as usize;
    let h = height as usize;
    if bgrx.len() < w * h * 4 || w == 0 || h == 0 {
        return;
    }
    // Dark slate background (#14171F)
    for px in bgrx[..w * h * 4].as_chunks_mut::<4>().0 {
        px.copy_from_slice(&[0x1F, 0x17, 0x14, 0xFF]);
    }

    // Center card (#1D222E) with subtle amber top accent bar (#F59E0B -> BGR 0x0B, 0x9E, 0xF5)
    let card_w = (w * 3 / 5).clamp(320.min(w), w);
    let card_h = (h * 2 / 5).clamp(180.min(h), h);
    let cx0 = (w - card_w) / 2;
    let cy0 = (h - card_h) / 2;
    fill_rect(bgrx, w, h, cx0, cy0, card_w, card_h, [0x2E, 0x22, 0x1D, 0xFF]);
    fill_rect(bgrx, w, h, cx0, cy0, card_w, 4.min(card_h), [0x0B, 0x9E, 0xF5, 0xFF]);

    // Camera icon in the middle-top of the card
    let scale = (h / 360).clamp(2, 4);
    let icon_w = 18 * scale;
    let icon_h = 12 * scale;
    let ix = w / 2 - icon_w / 2;
    let iy = cy0 + card_h / 4;
    fill_rect(bgrx, w, h, ix, iy, icon_w * 3 / 4, icon_h, [0x0B, 0x9E, 0xF5, 0xFF]);
    fill_rect(
        bgrx,
        w,
        h,
        ix + icon_w * 3 / 4 + scale,
        iy + icon_h / 4,
        icon_w / 4,
        icon_h / 2,
        [0x0B, 0x9E, 0xF5, 0xFF],
    );

    let title = "NECTARLINK WEBCAM";
    let subtitle = if phone_name.trim().is_empty() {
        "START THE WEBCAM ON YOUR PHONE".to_owned()
    } else {
        format!("START THE WEBCAM ON {}", phone_name.trim().to_ascii_uppercase())
    };
    draw_text_centered(bgrx, w, h, iy + icon_h + 8 * scale, scale, title, [0xF4, 0xF0, 0xEC, 0xFF]);
    draw_text_centered(
        bgrx,
        w,
        h,
        iy + icon_h + 20 * scale,
        (scale - 1).max(1),
        &subtitle,
        [0xB8, 0xA8, 0x9C, 0xFF],
    );
}

#[allow(clippy::too_many_arguments)]
fn fill_rect(
    bgrx: &mut [u8],
    w: usize,
    h: usize,
    x0: usize,
    y0: usize,
    rw: usize,
    rh: usize,
    color: [u8; 4],
) {
    let x1 = (x0 + rw).min(w);
    let y1 = (y0 + rh).min(h);
    for y in y0.min(h)..y1 {
        let row = &mut bgrx[y * w * 4..(y + 1) * w * 4];
        for x in x0.min(w)..x1 {
            row[x * 4..x * 4 + 4].copy_from_slice(&color);
        }
    }
}

fn draw_text_centered(
    bgrx: &mut [u8],
    w: usize,
    h: usize,
    y: usize,
    scale: usize,
    text: &str,
    color: [u8; 4],
) {
    let char_adv = 6 * scale;
    let total_w = text.chars().count().saturating_mul(char_adv);
    let start_x = w.saturating_sub(total_w) / 2;
    for (i, ch) in text.chars().enumerate() {
        let glyph = glyph_5x7(ch);
        let gx = start_x + i * char_adv;
        for (row_idx, &bits) in glyph.iter().enumerate() {
            for col_idx in 0..5 {
                if (bits >> (4 - col_idx)) & 1 != 0 {
                    fill_rect(bgrx, w, h, gx + col_idx * scale, y + row_idx * scale, scale, scale, color);
                }
            }
        }
    }
}

fn glyph_5x7(ch: char) -> [u8; 7] {
    match ch.to_ascii_uppercase() {
        'A' => [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'B' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110],
        'C' => [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110],
        'D' => [0b11100, 0b10010, 0b10001, 0b10001, 0b10001, 0b10010, 0b11100],
        'E' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        'F' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000],
        'G' => [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110],
        'H' => [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'I' => [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        'J' => [0b00111, 0b00010, 0b00010, 0b00010, 0b10010, 0b10010, 0b01100],
        'K' => [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001],
        'L' => [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        'M' => [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
        'N' => [0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001],
        'O' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'P' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
        'Q' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101],
        'R' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
        'S' => [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110],
        'T' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        'U' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'V' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100],
        'W' => [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001],
        'X' => [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001],
        'Y' => [0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100],
        'Z' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111],
        '0' => [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
        '1' => [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        '2' => [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
        '3' => [0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110],
        '4' => [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
        '5' => [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
        '6' => [0b01110, 0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
        '7' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
        '8' => [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
        '9' => [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001, 0b01110],
        '-' => [0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000],
        _ => [0; 7],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_camera_always_gets_a_mapping_and_the_app_finds_it() {
        // Outside a service the global name usually can't be created; the
        // camera then falls back to this session's mapping.
        let camera = SharedFrameMapping::for_camera(DEFAULT_MAPPING_NAME).unwrap();
        camera.set_idle("Pixel 9");
        let app = SharedFrameMapping::open_existing(DEFAULT_MAPPING_NAME)
            .or_else(|_| SharedFrameMapping::open_existing(LOCAL_MAPPING_NAME))
            .unwrap();
        assert_eq!(app.load_phone_name(), "Pixel 9");
    }

    #[test]
    fn opening_a_missing_mapping_fails() {
        let name = format!(r"Local\NectarlinkVcamMissing_{}", std::process::id());
        assert!(SharedFrameMapping::open_existing(&name).is_err());
    }
}
