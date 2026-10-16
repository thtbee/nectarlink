// SPDX-License-Identifier: GPL-3.0-or-later
use std::ffi::{CStr, c_char, c_int, c_uchar, c_void};
use std::io::{Error, ErrorKind, Result};
use std::ptr;
use std::sync::OnceLock;

#[derive(Debug, Clone)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub plane_rows: usize,
    pub nv12: Vec<u8>,
    pub colors: Colors,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Colors {
    pub bt601: bool,
    pub full_range: bool,
}

struct AvCodecApi {
    avcodec_find_decoder_by_name: unsafe extern "C" fn(*const c_char) -> *const c_void,
    avcodec_alloc_context3: unsafe extern "C" fn(*const c_void) -> *mut c_void,
    avcodec_open2: unsafe extern "C" fn(*mut c_void, *const c_void, *mut *mut c_void) -> c_int,
    avcodec_send_packet: unsafe extern "C" fn(*mut c_void, *const c_void) -> c_int,
    avcodec_receive_frame: unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int,
    avcodec_flush_buffers: unsafe extern "C" fn(*mut c_void),
    avcodec_free_context: unsafe extern "C" fn(*mut *mut c_void),
    av_packet_alloc: unsafe extern "C" fn() -> *mut c_void,
    av_packet_free: unsafe extern "C" fn(*mut *mut c_void),
    av_new_packet: unsafe extern "C" fn(*mut c_void, c_int) -> c_int,
    av_packet_unref: unsafe extern "C" fn(*mut c_void),
    av_frame_alloc: unsafe extern "C" fn() -> *mut c_void,
    av_frame_free: unsafe extern "C" fn(*mut *mut c_void),
    av_frame_unref: unsafe extern "C" fn(*mut c_void),
    av_opt_set_int: Option<unsafe extern "C" fn(*mut c_void, *const c_char, i64, c_int) -> c_int>,
}

struct OpenH264Api {
    create_decoder: unsafe extern "C" fn(*mut *mut c_void) -> c_int,
    destroy_decoder: unsafe extern "C" fn(*mut c_void),
    initialize: unsafe extern "C" fn(*mut c_void, *const u8) -> c_int,
    uninitialize: unsafe extern "C" fn(*mut c_void) -> c_int,
    decode_frame_no_delay: unsafe extern "C" fn(
        *mut c_void,
        *const c_uchar,
        c_int,
        *mut *mut c_uchar,
        *mut OpenH264BufferInfo,
    ) -> c_int,
}

#[repr(C)]
struct OpenH264BufferInfo {
    buffer_status: c_int,
    in_bs_time_stamp: u64,
    out_yuv_time_stamp: u64,
    width: c_int,
    height: c_int,
    format: c_int,
    y_stride: c_int,
    uv_stride: c_int,
    _reserved: [u8; 24],
}

unsafe fn load_sym<T: Copy>(lib: *mut c_void, name: &CStr) -> Option<T> {
    let raw = unsafe { libc::dlsym(lib, name.as_ptr()) };
    if raw.is_null() { None } else { Some(unsafe { std::mem::transmute_copy(&raw) }) }
}

fn avcodec_api() -> Option<&'static AvCodecApi> {
    static API: OnceLock<Option<AvCodecApi>> = OnceLock::new();
    API.get_or_init(|| unsafe {
        let avutil_candidates = [c"libavutil.so.58", c"libavutil.so.59", c"libavutil.so.57", c"libavutil.so"];
        let mut avutil = ptr::null_mut();
        for name in avutil_candidates {
            avutil = libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL);
            if !avutil.is_null() {
                break;
            }
        }
        if avutil.is_null() {
            return None;
        }

        let avcodec_candidates =
            [c"libavcodec.so.60", c"libavcodec.so.61", c"libavcodec.so.59", c"libavcodec.so"];
        let mut avcodec = ptr::null_mut();
        for name in avcodec_candidates {
            avcodec = libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
            if !avcodec.is_null() {
                break;
            }
        }
        if avcodec.is_null() {
            return None;
        }

        Some(AvCodecApi {
            avcodec_find_decoder_by_name: load_sym(avcodec, c"avcodec_find_decoder_by_name")?,
            avcodec_alloc_context3: load_sym(avcodec, c"avcodec_alloc_context3")?,
            avcodec_open2: load_sym(avcodec, c"avcodec_open2")?,
            avcodec_send_packet: load_sym(avcodec, c"avcodec_send_packet")?,
            avcodec_receive_frame: load_sym(avcodec, c"avcodec_receive_frame")?,
            avcodec_flush_buffers: load_sym(avcodec, c"avcodec_flush_buffers")?,
            avcodec_free_context: load_sym(avcodec, c"avcodec_free_context")?,
            av_packet_alloc: load_sym(avcodec, c"av_packet_alloc")?,
            av_packet_free: load_sym(avcodec, c"av_packet_free")?,
            av_new_packet: load_sym(avcodec, c"av_new_packet")?,
            av_packet_unref: load_sym(avcodec, c"av_packet_unref")?,
            av_frame_alloc: load_sym(avutil, c"av_frame_alloc")?,
            av_frame_free: load_sym(avutil, c"av_frame_free")?,
            av_frame_unref: load_sym(avutil, c"av_frame_unref")?,
            av_opt_set_int: load_sym(avutil, c"av_opt_set_int"),
        })
    })
    .as_ref()
}

fn openh264_api() -> Option<&'static OpenH264Api> {
    static API: OnceLock<Option<OpenH264Api>> = OnceLock::new();
    API.get_or_init(|| unsafe {
        let candidates = [c"libopenh264.so.7", c"libopenh264.so.6", c"libopenh264.so"];
        let mut lib = ptr::null_mut();
        for name in candidates {
            lib = libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
            if !lib.is_null() {
                break;
            }
        }
        if lib.is_null() {
            return None;
        }
        Some(OpenH264Api {
            create_decoder: load_sym(lib, c"WelsCreateDecoder")?,
            destroy_decoder: load_sym(lib, c"WelsDestroyDecoder")?,
            initialize: load_sym(lib, c"_ZN7WelsDec12CWelsDecoder10InitializeEPK19TagSVCDecodingParam")?,
            uninitialize: load_sym(lib, c"_ZN7WelsDec12CWelsDecoder12UninitializeEv")?,
            decode_frame_no_delay: load_sym(
                lib,
                c"_ZN7WelsDec12CWelsDecoder18DecodeFrameNoDelayEPKhiPPhP13TagBufferInfo",
            )?,
        })
    })
    .as_ref()
}

enum Backend {
    AvCodec { ctx: *mut c_void, pkt: *mut c_void, frame: *mut c_void },
    OpenH264 { dec: *mut c_void },
}

// SAFETY: Decoder is owned and used exclusively by a single decode worker thread at a time.
unsafe impl Send for Backend {}

impl std::fmt::Debug for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Backend::AvCodec { .. } => f.write_str("Backend::AvCodec"),
            Backend::OpenH264 { .. } => f.write_str("Backend::OpenH264"),
        }
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        match self {
            Backend::AvCodec { ctx, pkt, frame } => {
                if let Some(api) = avcodec_api() {
                    unsafe {
                        if !frame.is_null() {
                            (api.av_frame_free)(frame);
                        }
                        if !pkt.is_null() {
                            (api.av_packet_free)(pkt);
                        }
                        if !ctx.is_null() {
                            (api.avcodec_free_context)(ctx);
                        }
                    }
                }
            }
            Backend::OpenH264 { dec } => {
                if let Some(api) = openh264_api()
                    && !dec.is_null()
                {
                    unsafe {
                        let _ = (api.uninitialize)(*dec);
                        (api.destroy_decoder)(*dec);
                    }
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct Decoder {
    codec_name: &'static str,
    backend: Backend,
}

pub(crate) fn ensure_mf_started() -> Result<()> {
    Ok(())
}

impl Decoder {
    pub fn new() -> Result<Decoder> {
        Self::new_for_codec("h264")
    }

    pub fn new_for_codec(codec: &str) -> Result<Decoder> {
        ensure_mf_started()?;
        let (codec_name, c_name) = if codec.eq_ignore_ascii_case("hevc") || codec.eq_ignore_ascii_case("h265")
        {
            ("hevc", c"hevc")
        } else if codec.eq_ignore_ascii_case("av1") {
            ("av1", c"av1")
        } else {
            ("h264", c"h264")
        };

        if let Some(backend) = Self::try_new_avcodec(c_name) {
            return Ok(Decoder { codec_name, backend });
        }

        if codec_name == "h264"
            && let Some(backend) = Self::try_new_openh264()
        {
            return Ok(Decoder { codec_name, backend });
        }

        Err(Error::new(
            ErrorKind::Unsupported,
            format!("no native Linux video decoder available for {codec_name}"),
        ))
    }

    fn try_new_openh264() -> Option<Backend> {
        let api = openh264_api()?;
        unsafe {
            let mut dec: *mut c_void = ptr::null_mut();
            if (api.create_decoder)(&mut dec) != 0 || dec.is_null() {
                return None;
            }
            let param = [0u8; 64];
            if (api.initialize)(dec, param.as_ptr()) != 0 {
                (api.destroy_decoder)(dec);
                return None;
            }
            Some(Backend::OpenH264 { dec })
        }
    }

    fn try_new_avcodec(c_name: &CStr) -> Option<Backend> {
        let api = avcodec_api()?;
        unsafe {
            let codec = (api.avcodec_find_decoder_by_name)(c_name.as_ptr());
            if codec.is_null() {
                return None;
            }
            let mut ctx = (api.avcodec_alloc_context3)(codec);
            if ctx.is_null() {
                return None;
            }
            if let Some(opt_set_int) = api.av_opt_set_int {
                // AV_CODEC_FLAG_LOW_DELAY = 1 << 19; threads = 1 avoids multi-frame buffering delay.
                let _ = opt_set_int(ctx, c"flags".as_ptr(), 1 << 19, 0);
                let _ = opt_set_int(ctx, c"threads".as_ptr(), 1, 0);
            }
            if (api.avcodec_open2)(ctx, codec, ptr::null_mut()) < 0 {
                (api.avcodec_free_context)(&mut ctx);
                return None;
            }
            let mut pkt = (api.av_packet_alloc)();
            let mut frame = (api.av_frame_alloc)();
            if pkt.is_null() || frame.is_null() {
                if !frame.is_null() {
                    (api.av_frame_free)(&mut frame);
                }
                if !pkt.is_null() {
                    (api.av_packet_free)(&mut pkt);
                }
                (api.avcodec_free_context)(&mut ctx);
                return None;
            }
            Some(Backend::AvCodec { ctx, pkt, frame })
        }
    }

    pub fn new_gpu(codec: &str) -> Result<Decoder> {
        Self::new_for_codec(codec)
    }

    pub fn on_gpu(&self) -> bool {
        false
    }

    pub fn codec(&self) -> &str {
        self.codec_name
    }

    pub fn decode(&mut self, data: &[u8], _time_us: u64) -> Result<Vec<Picture>> {
        if data.is_empty() {
            return Ok(Vec::new());
        }
        match &mut self.backend {
            Backend::OpenH264 { dec } => Self::decode_openh264(*dec, data),
            Backend::AvCodec { ctx, pkt, frame } => Self::decode_avcodec(*ctx, *pkt, *frame, data),
        }
    }

    fn decode_openh264(dec: *mut c_void, data: &[u8]) -> Result<Vec<Picture>> {
        let api = openh264_api().ok_or_else(|| Error::new(ErrorKind::Unsupported, "OpenH264 unavailable"))?;
        let mut pictures = Vec::new();
        // OpenH264's DecodeFrameNoDelay processes one NAL or a full access unit; feeding each
        // Annex-B NAL unit in order guarantees SPS, PPS, SEI, and IDR/slice NALs in a single
        // packet are all consumed cleanly.
        for nal in split_annexb_nals(data) {
            unsafe {
                let mut dst: [*mut c_uchar; 3] = [ptr::null_mut(); 3];
                let mut info: OpenH264BufferInfo = std::mem::zeroed();
                let rc = (api.decode_frame_no_delay)(
                    dec,
                    nal.as_ptr(),
                    nal.len() as c_int,
                    dst.as_mut_ptr(),
                    &mut info,
                );
                if rc != 0 {
                    return Err(Error::new(
                        ErrorKind::InvalidData,
                        format!("OpenH264 decode error 0x{rc:x}"),
                    ));
                }
                if info.buffer_status == 1
                    && info.width > 0
                    && info.height > 0
                    && !dst[0].is_null()
                    && !dst[1].is_null()
                    && !dst[2].is_null()
                    && let Some(pic) = i420_to_nv12_picture(
                        dst[0],
                        dst[1],
                        dst[2],
                        info.width as u32,
                        info.height as u32,
                        info.y_stride.max(info.width) as usize,
                        info.uv_stride.max(info.width / 2) as usize,
                        Colors::default(),
                    )
                {
                    pictures.push(pic);
                }
            }
        }
        Ok(pictures)
    }

    fn decode_avcodec(
        ctx: *mut c_void,
        pkt: *mut c_void,
        frame: *mut c_void,
        data: &[u8],
    ) -> Result<Vec<Picture>> {
        let api =
            avcodec_api().ok_or_else(|| Error::new(ErrorKind::Unsupported, "libavcodec unavailable"))?;
        unsafe {
            let alloc_rc = (api.av_new_packet)(pkt, data.len() as c_int);
            if alloc_rc < 0 {
                return Err(Error::new(ErrorKind::OutOfMemory, "av_new_packet failed"));
            }
            // In FFmpeg AVPacket (libavcodec 58..61), `data` pointer is at offset 0x18 and `size` at 0x20.
            let pkt_data_ptr = *(pkt.byte_add(0x18) as *const *mut u8);
            if pkt_data_ptr.is_null() {
                (api.av_packet_unref)(pkt);
                return Err(Error::new(ErrorKind::InvalidData, "null AVPacket buffer"));
            }
            ptr::copy_nonoverlapping(data.as_ptr(), pkt_data_ptr, data.len());
            let send_rc = (api.avcodec_send_packet)(ctx, pkt);
            (api.av_packet_unref)(pkt);
            if send_rc < 0 {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    format!("avcodec_send_packet failed ({send_rc})"),
                ));
            }

            let mut pictures = Vec::new();
            loop {
                let recv_rc = (api.avcodec_receive_frame)(ctx, frame);
                if recv_rc < 0 {
                    break;
                }
                if let Some(pic) = extract_avframe_picture(frame) {
                    pictures.push(pic);
                }
                (api.av_frame_unref)(frame);
            }
            Ok(pictures)
        }
    }

    pub fn flush(&mut self) {
        match &mut self.backend {
            Backend::AvCodec { ctx, .. } => {
                if let Some(api) = avcodec_api()
                    && !ctx.is_null()
                {
                    unsafe {
                        (api.avcodec_flush_buffers)(*ctx);
                    }
                }
            }
            Backend::OpenH264 { dec } => {
                if let Some(api) = openh264_api()
                    && !dec.is_null()
                {
                    unsafe {
                        let _ = (api.uninitialize)(*dec);
                        let param = [0u8; 64];
                        let _ = (api.initialize)(*dec, param.as_ptr());
                    }
                }
            }
        }
    }
}

/// Split an Annex-B bitstream (`00 00 01` or `00 00 00 01` start codes) into individual
/// NAL units (each retaining its Annex-B start code prefix).
fn split_annexb_nals(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 < data.len() {
        if data[i] == 0 && data[i + 1] == 0 {
            if data[i + 2] == 1 {
                starts.push(i);
                i += 3;
                continue;
            } else if i + 4 <= data.len() && data[i + 2] == 0 && data[i + 3] == 1 {
                starts.push(i);
                i += 4;
                continue;
            }
        }
        i += 1;
    }
    if starts.len() <= 1 {
        return vec![data];
    }
    let mut nals = Vec::with_capacity(starts.len());
    for idx in 0..starts.len() {
        let start = starts[idx];
        let end = starts.get(idx + 1).copied().unwrap_or(data.len());
        if end > start {
            nals.push(&data[start..end]);
        }
    }
    nals
}

unsafe fn extract_avframe_picture(frame: *const c_void) -> Option<Picture> {
    unsafe {
        let data = ptr::slice_from_raw_parts(frame as *const *const u8, 8).as_ref()?;
        let linesize = ptr::slice_from_raw_parts(frame.byte_add(0x40) as *const c_int, 8).as_ref()?;
        let width = *(frame.byte_add(0x68) as *const c_int);
        let height = *(frame.byte_add(0x6c) as *const c_int);
        let format = *(frame.byte_add(0x74) as *const c_int);
        if width <= 0 || height <= 0 || data[0].is_null() {
            return None;
        }
        let w = width as u32;
        let h = height as u32;
        let y_stride = linesize[0].max(width) as usize;
        // AV_PIX_FMT_NV12 = 23
        if format == 23 && !data[1].is_null() {
            let uv_stride = linesize[1].max(width) as usize;
            let even_w = (w as usize) & !1;
            let even_h = (h as usize) & !1;
            if even_w == 0 || even_h == 0 {
                return None;
            }
            let mut nv12 = vec![0u8; even_w * even_h + even_w * (even_h / 2)];
            for row in 0..even_h {
                let src = std::slice::from_raw_parts(data[0].add(row * y_stride), even_w);
                nv12[row * even_w..(row + 1) * even_w].copy_from_slice(src);
            }
            let uv_base = even_w * even_h;
            for row in 0..(even_h / 2) {
                let src = std::slice::from_raw_parts(data[1].add(row * uv_stride), even_w);
                nv12[uv_base + row * even_w..uv_base + (row + 1) * even_w].copy_from_slice(src);
            }
            return Some(Picture {
                width: even_w as u32,
                height: even_h as u32,
                stride: even_w,
                plane_rows: even_h,
                nv12,
                colors: Colors::default(),
            });
        }

        // AV_PIX_FMT_YUV420P = 0, AV_PIX_FMT_YUVJ420P = 12
        if !data[1].is_null() && !data[2].is_null() {
            let uv_stride = linesize[1].max(width / 2) as usize;
            let colors = Colors { bt601: false, full_range: format == 12 };
            return i420_to_nv12_picture(data[0], data[1], data[2], w, h, y_stride, uv_stride, colors);
        }
        None
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn i420_to_nv12_picture(
    y_ptr: *const u8,
    u_ptr: *const u8,
    v_ptr: *const u8,
    width: u32,
    height: u32,
    y_stride: usize,
    uv_stride: usize,
    colors: Colors,
) -> Option<Picture> {
    let w = (width as usize) & !1;
    let h = (height as usize) & !1;
    if w == 0 || h == 0 {
        return None;
    }
    let uv_rows = h / 2;
    let uv_cols = w / 2;
    let mut nv12 = vec![0u8; w * h + w * uv_rows];
    unsafe {
        for row in 0..h {
            let src = std::slice::from_raw_parts(y_ptr.add(row * y_stride), w);
            nv12[row * w..(row + 1) * w].copy_from_slice(src);
        }
        let uv_base = w * h;
        for row in 0..uv_rows {
            let u_row = std::slice::from_raw_parts(u_ptr.add(row * uv_stride), uv_cols);
            let v_row = std::slice::from_raw_parts(v_ptr.add(row * uv_stride), uv_cols);
            let dst_row = &mut nv12[uv_base + row * w..uv_base + (row + 1) * w];
            for col in 0..uv_cols {
                dst_row[col * 2] = u_row[col];
                dst_row[col * 2 + 1] = v_row[col];
            }
        }
    }
    Some(Picture { width: w as u32, height: h as u32, stride: w, plane_rows: h, nv12, colors })
}

pub fn to_bgrx(picture: &Picture, width: u32, height: u32) -> Vec<u8> {
    let w = (width as usize).min(picture.width as usize);
    let h = (height as usize).min(picture.height as usize);
    let stride = picture.stride.max(w);
    let uv_base = stride * picture.plane_rows.max(h);

    let (y_scale, y_bias, c_rv, c_gu, c_gv, c_bu) = match (picture.colors.bt601, picture.colors.full_range) {
        (false, false) => (298, 16, 459, 55, 136, 541),
        (false, true) => (256, 0, 403, 48, 120, 475),
        (true, false) => (298, 16, 409, 100, 208, 516),
        (true, true) => (256, 0, 359, 88, 183, 454),
    };

    let mut out = vec![0u8; w * h * 4];
    for row in 0..h {
        let y_row = row * stride;
        let uv_row = uv_base + (row / 2) * stride;
        let dst_row = row * w * 4;
        for col in 0..w {
            let y_val = i32::from(*picture.nv12.get(y_row + col).unwrap_or(&0));
            let uv_col = (col & !1) + uv_row;
            let u_val = i32::from(*picture.nv12.get(uv_col).unwrap_or(&128)) - 128;
            let v_val = i32::from(*picture.nv12.get(uv_col + 1).unwrap_or(&128)) - 128;

            let c = (y_val - y_bias).max(0) * y_scale;
            let r = (c + c_rv * v_val + 128) >> 8;
            let g = (c - c_gu * u_val - c_gv * v_val + 128) >> 8;
            let b = (c + c_bu * u_val + 128) >> 8;

            let dst = dst_row + col * 4;
            out[dst] = b.clamp(0, 255) as u8;
            out[dst + 1] = g.clamp(0, 255) as u8;
            out[dst + 2] = r.clamp(0, 255) as u8;
            out[dst + 3] = 0xFF;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_H264_KEYFRAME_64X64: &[u8] = &[
        0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0xc0, 0x0a, 0xd9, 0x04, 0x26, 0xc0, 0x44, 0x00, 0x00, 0x03, 0x00,
        0x04, 0x00, 0x00, 0x03, 0x00, 0xc8, 0x3c, 0x48, 0x99, 0x20, 0x00, 0x00, 0x00, 0x01, 0x68, 0xcb, 0x83,
        0xcb, 0x20, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84, 0x0a, 0xf1, 0x18, 0xa0, 0x00, 0x27, 0xa7, 0x1c, 0x00,
        0x04, 0xbe, 0x23, 0x80, 0x00, 0x82, 0xec, 0x9c, 0x9c, 0x9d, 0x75, 0xd7, 0x5d, 0x75, 0xd7, 0x5d, 0x75,
        0xd7, 0x5e,
    ];

    #[test]
    fn to_bgrx_converts_neutral_luma_to_grayscale() {
        let width = 4;
        let height = 2;
        let stride = 4;
        let plane_rows = 2;
        let mut nv12 = vec![128u8; stride * plane_rows];
        nv12.extend(vec![128u8; stride * (plane_rows / 2)]);
        let pic = Picture {
            width: width as u32,
            height: height as u32,
            stride,
            plane_rows,
            nv12,
            colors: Colors { bt601: false, full_range: true },
        };
        let bgrx = to_bgrx(&pic, width as u32, height as u32);
        assert_eq!(bgrx.len(), width * height * 4);
        for px in bgrx.as_chunks::<4>().0 {
            assert_eq!(px[0], 128);
            assert_eq!(px[1], 128);
            assert_eq!(px[2], 128);
            assert_eq!(px[3], 255);
        }
    }

    #[test]
    fn split_annexb_nals_splits_sps_pps_idr() {
        let nals = split_annexb_nals(SAMPLE_H264_KEYFRAME_64X64);
        assert_eq!(nals.len(), 3);
        assert_eq!(nals[0][4] & 0x1f, 7); // SPS
        assert_eq!(nals[1][4] & 0x1f, 8); // PPS
        assert_eq!(nals[2][3] & 0x1f, 5); // IDR
    }

    #[test]
    fn decoder_decodes_real_h264_keyframe_immediately() {
        let mut dec = Decoder::new().expect("H.264 decoder should initialize on Linux");
        let pics = dec.decode(SAMPLE_H264_KEYFRAME_64X64, 0).expect("H.264 keyframe should decode");
        assert_eq!(pics.len(), 1, "expected immediate zero-delay decoded frame");
        let pic = &pics[0];
        assert_eq!(pic.width, 64);
        assert_eq!(pic.height, 64);
        let bgrx = to_bgrx(pic, pic.width, pic.height);
        assert_eq!(bgrx.len(), 64 * 64 * 4);
        // The sample frame is solid blue, so blue channel (bgrx[0]) should dominate red (bgrx[2]).
        assert!(bgrx[0] > 180, "expected strong blue channel, got {}", bgrx[0]);
        assert!(bgrx[2] < 50, "expected low red channel, got {}", bgrx[2]);
    }
}
