// SPDX-License-Identifier: GPL-3.0-or-later
//! Shared-memory frame transport between `nectarlink-desktop.exe` and
//! `nectarlink_vcam.dll`, plus placeholder frame rendering and RGB32/NV12
//! conversion.

use std::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering};

use ab_glyph::{Font, FontRef, GlyphId, PxScale, ScaleFont, point};

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

const FIGTREE_TTF: &[u8] = include_bytes!("../../../assets/fonts/Figtree.ttf");

/// Honey primary (`docs/design/tokens.json`) and white ink for the Nectarlink mark.
const MARK_BG_BGR: [u8; 3] = [0x00, 0x51, 0x8A];
const MARK_INK_BGR: [u8; 3] = [0xFF, 0xFF, 0xFF];
const MARK_HEXAGON: [(f32, f32); 6] =
    [(12.0, 2.5), (20.2, 7.25), (20.2, 16.75), (12.0, 21.5), (3.8, 16.75), (3.8, 7.25)];
const MARK_BAR: ((f32, f32), (f32, f32)) = ((9.0, 12.0), (15.0, 12.0));

struct CachedPlaceholderCard {
    width: u32,
    height: u32,
    phone_name: String,
    card_x: usize,
    card_y: usize,
    card_w: usize,
    card_h: usize,
    card_bgrx: Vec<u8>,
}

static PLACEHOLDER_CACHE: std::sync::Mutex<Option<CachedPlaceholderCard>> = std::sync::Mutex::new(None);

/// Renders a clean, calm placeholder frame with the Nectarlink mark and
/// `"Start the webcam on <phone>"` in anti-aliased Figtree into `bgrx` (`width * height * 4`).
pub fn render_placeholder_bgrx(bgrx: &mut [u8], width: u32, height: u32, phone_name: &str) {
    let w = width as usize;
    let h = height as usize;
    if bgrx.len() < w * h * 4 || w == 0 || h == 0 {
        return;
    }
    // Theme-neutral dark background (#111318 -> BGR 0x18, 0x13, 0x11)
    let bg_px = [0x18u8, 0x13, 0x11, 0xFF];
    for px in bgrx[..w * h * 4].as_chunks_mut::<4>().0 {
        px.copy_from_slice(&bg_px);
    }

    let trimmed = phone_name.trim();
    if let Ok(mut guard) = PLACEHOLDER_CACHE.lock() {
        let hit =
            guard.as_ref().is_some_and(|c| c.width == width && c.height == height && c.phone_name == trimmed);
        if !hit {
            *guard = Some(build_placeholder_card(width, height, trimmed));
        }
        if let Some(cached) = guard.as_ref() {
            blit_card(bgrx, w, h, cached);
            return;
        }
    }

    let card = build_placeholder_card(width, height, trimmed);
    blit_card(bgrx, w, h, &card);
}

fn blit_card(bgrx: &mut [u8], w: usize, h: usize, card: &CachedPlaceholderCard) {
    let max_rows = card.card_h.min(h.saturating_sub(card.card_y));
    let max_cols = card.card_w.min(w.saturating_sub(card.card_x));
    for row in 0..max_rows {
        let dst_off = ((card.card_y + row) * w + card.card_x) * 4;
        let src_off = row * card.card_w * 4;
        bgrx[dst_off..dst_off + max_cols * 4]
            .copy_from_slice(&card.card_bgrx[src_off..src_off + max_cols * 4]);
    }
}

fn build_placeholder_card(width: u32, height: u32, phone_name: &str) -> CachedPlaceholderCard {
    let w = width as usize;
    let h = height as usize;
    let scale = (h as f32 / 720.0).clamp(0.45, 1.6);

    let headline = if phone_name.is_empty() {
        "Start the webcam on your phone".to_owned()
    } else {
        format!("Start the webcam on {phone_name}")
    };
    let caption = "Nectarlink Camera";

    let font = TrueTypeFont::parse(FIGTREE_TTF);
    let head_px = (22.0 * scale).clamp(12.0, 38.0);
    let cap_px = (14.0 * scale).clamp(10.0, 24.0);
    let head_w = font.as_ref().map_or(260, |f| f.measure_text(&headline, head_px));
    let cap_w = font.as_ref().map_or(140, |f| f.measure_text(caption, cap_px));

    let pad_x = (44.0 * scale).round() as usize;
    let min_card_w = (360.0 * scale).round() as usize;
    let card_w = (head_w.max(cap_w) + pad_x * 2).max(min_card_w).min(w.saturating_sub(16).max(1));
    let card_h = ((208.0 * scale).round() as usize).clamp(96.min(h), h.saturating_sub(16).max(1));
    let card_x = (w - card_w) / 2;
    let card_y = (h - card_h) / 2;

    let mut card_bgrx = vec![0u8; card_w * card_h * 4];
    let bg_bgr = [0x18u8, 0x13, 0x11];
    let surface_bgr = [0x23u8, 0x1C, 0x19]; // #191C23
    let border_bgr = [0x36u8, 0x2C, 0x27]; // #272C36
    let radius = (20.0 * scale).clamp(8.0, 32.0);

    draw_rounded_card(&mut card_bgrx, card_w, card_h, radius, bg_bgr, surface_bgr, border_bgr);

    let mark_size = ((52.0 * scale).round() as usize).clamp(24, 88).min(card_h / 2);
    let mark_x = card_w.saturating_sub(mark_size) / 2;
    let total_content_h = mark_size
        + (18.0 * scale).round() as usize
        + head_px.round() as usize
        + (8.0 * scale).round() as usize
        + cap_px.round() as usize;
    let mark_y = card_h.saturating_sub(total_content_h) / 2;
    blend_mark(&mut card_bgrx, card_w, card_h, mark_x, mark_y, mark_size);

    if let Some(font) = font.as_ref() {
        let head_baseline =
            mark_y + mark_size + (16.0 * scale).round() as usize + (head_px * 0.82).round() as usize;
        // Primary ink (#F2F0EC -> BGR 0xEC, 0xF0, 0xF2)
        font.draw_text_centered(
            &mut card_bgrx,
            card_w,
            card_h,
            head_baseline as f32,
            head_px,
            &headline,
            [0xEC, 0xF0, 0xF2],
        );

        let cap_baseline = head_baseline + (10.0 * scale).round() as usize + (cap_px * 0.85).round() as usize;
        // Secondary muted ink (#9399A6 -> BGR 0xA6, 0x99, 0x93)
        font.draw_text_centered(
            &mut card_bgrx,
            card_w,
            card_h,
            cap_baseline as f32,
            cap_px,
            caption,
            [0xA6, 0x99, 0x93],
        );
    }

    CachedPlaceholderCard {
        width,
        height,
        phone_name: phone_name.to_owned(),
        card_x,
        card_y,
        card_w,
        card_h,
        card_bgrx,
    }
}

fn draw_rounded_card(
    bgrx: &mut [u8],
    w: usize,
    h: usize,
    radius: f32,
    outside_bgr: [u8; 3],
    fill_bgr: [u8; 3],
    border_bgr: [u8; 3],
) {
    let wf = w as f32;
    let hf = h as f32;
    let r = radius.min(wf * 0.5).min(hf * 0.5);
    for py in 0..h {
        for px in 0..w {
            let x = px as f32 + 0.5;
            let y = py as f32 + 0.5;
            let cx = x.clamp(r, wf - r);
            let cy = y.clamp(r, hf - r);
            let dist = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt() - r;
            let fill_alpha = (0.5 - dist).clamp(0.0, 1.0);
            let inner_alpha = (-0.6 - dist).clamp(0.0, 1.0);
            let mut rgb = [0f32; 3];
            for c in 0..3 {
                let card_c = border_bgr[c] as f32 * (1.0 - inner_alpha) + fill_bgr[c] as f32 * inner_alpha;
                rgb[c] = outside_bgr[c] as f32 * (1.0 - fill_alpha) + card_c * fill_alpha;
            }
            let idx = (py * w + px) * 4;
            bgrx[idx] = rgb[0].round() as u8;
            bgrx[idx + 1] = rgb[1].round() as u8;
            bgrx[idx + 2] = rgb[2].round() as u8;
            bgrx[idx + 3] = 0xFF;
        }
    }
}

fn blend_mark(bgrx: &mut [u8], w: usize, h: usize, ox: usize, oy: usize, size: usize) {
    const SS: usize = 4;
    let s = size as f32;
    let radius = s * 0.28;
    let glyph = s * 0.62;
    let scale = glyph / 24.0;
    let offset = (s - glyph) / 2.0;
    let stroke = (1.8 * scale).max(1.4);
    let hex: Vec<(f32, f32)> =
        MARK_HEXAGON.iter().map(|&(x, y)| (offset + x * scale, offset + y * scale)).collect();
    let bar = (
        (offset + MARK_BAR.0.0 * scale, offset + MARK_BAR.0.1 * scale),
        (offset + MARK_BAR.1.0 * scale, offset + MARK_BAR.1.1 * scale),
    );

    for py in 0..size {
        let dy = oy + py;
        if dy >= h {
            break;
        }
        for px in 0..size {
            let dx = ox + px;
            if dx >= w {
                break;
            }
            let (mut tile, mut ink) = (0usize, 0usize);
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = px as f32 + (sx as f32 + 0.5) / SS as f32;
                    let y = py as f32 + (sy as f32 + 0.5) / SS as f32;
                    let cx = x.clamp(radius, s - radius);
                    let cy = y.clamp(radius, s - radius);
                    if (x - cx).powi(2) + (y - cy).powi(2) > radius * radius {
                        continue;
                    }
                    tile += 1;
                    let on_hex = (0..hex.len())
                        .any(|i| dist_to_segment((x, y), hex[i], hex[(i + 1) % hex.len()]) <= stroke / 2.0);
                    if on_hex || dist_to_segment((x, y), bar.0, bar.1) <= stroke / 2.0 {
                        ink += 1;
                    }
                }
            }
            if tile == 0 {
                continue;
            }
            let t = ink as f32 / tile as f32;
            let alpha = tile as f32 / (SS * SS) as f32;
            let idx = (dy * w + dx) * 4;
            for c in 0..3 {
                let mark_c = MARK_BG_BGR[c] as f32 * (1.0 - t) + MARK_INK_BGR[c] as f32 * t;
                bgrx[idx + c] = (bgrx[idx + c] as f32 * (1.0 - alpha) + mark_c * alpha).round() as u8;
            }
        }
    }
}

fn dist_to_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 { 0.0 } else { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) };
    let (qx, qy) = (a.0 + t * dx, a.1 + t * dy);
    ((p.0 - qx).powi(2) + (p.1 - qy).powi(2)).sqrt()
}

/// The bundled Figtree font, laid out and drawn with `ab_glyph`.
struct TrueTypeFont {
    font: FontRef<'static>,
}

impl TrueTypeFont {
    fn parse(data: &'static [u8]) -> Option<Self> {
        FontRef::try_from_slice(data).ok().map(|font| Self { font })
    }

    /// Each glyph of `text` at `px_size` with its pen position (no wrapping),
    /// and the line's width.
    fn layout(&self, text: &str, px_size: f32) -> (Vec<(GlyphId, f32)>, f32) {
        let font = self.font.as_scaled(PxScale::from(px_size));
        let mut pen = 0.0;
        let mut previous = None;
        let mut glyphs = Vec::with_capacity(text.len());
        for ch in text.chars() {
            let id = font.glyph_id(ch);
            if let Some(previous) = previous {
                pen += font.kern(previous, id);
            }
            glyphs.push((id, pen));
            pen += font.h_advance(id);
            previous = Some(id);
        }
        (glyphs, pen)
    }

    fn measure_text(&self, text: &str, px_size: f32) -> usize {
        self.layout(text, px_size).1.ceil() as usize
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_text_centered(
        &self,
        bgrx: &mut [u8],
        w: usize,
        h: usize,
        baseline_y: f32,
        px_size: f32,
        text: &str,
        ink_bgr: [u8; 3],
    ) {
        let (glyphs, width) = self.layout(text, px_size);
        let left = ((w as f32 - width) * 0.5).max(0.0);
        for (id, x) in glyphs {
            let glyph = id.with_scale_and_position(PxScale::from(px_size), point(left + x, baseline_y));
            let Some(outlined) = self.font.outline_glyph(glyph) else { continue };
            let bounds = outlined.px_bounds();
            outlined.draw(|gx, gy, coverage| {
                let px = bounds.min.x as i64 + i64::from(gx);
                let py = bounds.min.y as i64 + i64::from(gy);
                if px < 0 || py < 0 || px as usize >= w || py as usize >= h {
                    return;
                }
                let alpha = coverage.clamp(0.0, 1.0);
                let idx = (py as usize * w + px as usize) * 4;
                for c in 0..3 {
                    bgrx[idx + c] = (f32::from(bgrx[idx + c]) * (1.0 - alpha) + f32::from(ink_bgr[c]) * alpha)
                        .round() as u8;
                }
            });
        }
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

    #[test]
    fn placeholder_renders_mark_and_antialiased_figtree_text() {
        let (w, h) = (1280u32, 720u32);
        let mut frame = vec![0u8; (w * h * 4) as usize];
        render_placeholder_bgrx(&mut frame, w, h, "Pixel 9");
        // Corner pixel is dark neutral background (#111318 -> BGR 0x18, 0x13, 0x11).
        assert_eq!(&frame[0..4], &[0x18, 0x13, 0x11, 0xFF]);
        // Center card contains the honey mark (#8A5100 -> BGR 0x00, 0x51, 0x8A) and anti-aliased text.
        let has_honey_mark = frame
            .as_chunks::<4>()
            .0
            .iter()
            .any(|px| px[0] == 0x00 && px[1] == 0x51 && px[2] == 0x8A && px[3] == 0xFF);
        assert!(has_honey_mark, "placeholder should include the Nectarlink honey mark");
        let has_text_ink =
            frame.as_chunks::<4>().0.iter().any(|px| px[0] > 0xD0 && px[1] > 0xD0 && px[2] > 0xD0);
        assert!(has_text_ink, "placeholder should include bright headline text ink");
    }
}
