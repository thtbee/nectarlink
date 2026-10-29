// SPDX-License-Identifier: GPL-3.0-or-later
//! VideoToolbox decompression sessions for H.264, with each decoded NV12
//! pixel buffer copied out as a [`Picture`]. The declarations follow the
//! macOS SDK headers (VTDecompressionSession.h, VTSession.h, VTErrors.h,
//! CMFormatDescription.h, CMBlockBuffer.h, CMSampleBuffer.h, CMTime.h,
//! CVPixelBuffer.h, CVBuffer.h, CVImageBuffer.h, CFBase.h, CFDictionary.h,
//! CFArray.h, CFNumber.h).

#![allow(unsafe_code)]

use std::{
    ffi::{c_int, c_void},
    panic::{AssertUnwindSafe, catch_unwind},
    ptr::{self, NonNull},
    slice,
    sync::Mutex,
};

use super::{Colors, Picture};

type OSStatus = i32;
type Boolean = u8;
type CFIndex = isize;
type CFTypeID = usize;
type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CMFormatDescriptionRef = *const c_void;
type CMBlockBufferRef = *mut c_void;
type CMSampleBufferRef = *mut c_void;
type VTDecompressionSessionRef = *mut c_void;
type CVImageBufferRef = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct CMTime {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}

#[repr(C)]
struct CMSampleTimingInfo {
    duration: CMTime,
    presentation_time_stamp: CMTime,
    decode_time_stamp: CMTime,
}

#[repr(C)]
struct CMVideoDimensions {
    width: i32,
    height: i32,
}

type OutputCallback = unsafe extern "C" fn(
    decompression_output_ref_con: *mut c_void,
    source_frame_ref_con: *mut c_void,
    status: OSStatus,
    info_flags: u32,
    image_buffer: CVImageBufferRef,
    presentation_time_stamp: CMTime,
    presentation_duration: CMTime,
);

#[repr(C)]
struct OutputCallbackRecord {
    decompression_output_callback: Option<OutputCallback>,
    decompression_output_ref_con: *mut c_void,
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    // Callback tables, only ever passed by address.
    static kCFTypeDictionaryKeyCallBacks: u8;
    static kCFTypeDictionaryValueCallBacks: u8;
    static kCFTypeArrayCallBacks: u8;
    static kCFBooleanTrue: CFTypeRef;

    fn CFRelease(cf: CFTypeRef);
    fn CFEqual(cf1: CFTypeRef, cf2: CFTypeRef) -> Boolean;
    fn CFGetTypeID(cf: CFTypeRef) -> CFTypeID;
    fn CFBooleanGetTypeID() -> CFTypeID;
    fn CFBooleanGetValue(boolean: CFTypeRef) -> Boolean;
    fn CFNumberCreate(allocator: CFTypeRef, the_type: CFIndex, value_ptr: *const c_void) -> CFTypeRef;
    fn CFArrayCreate(
        allocator: CFTypeRef,
        values: *const CFTypeRef,
        num_values: CFIndex,
        call_backs: *const c_void,
    ) -> CFTypeRef;
    fn CFDictionaryCreate(
        allocator: CFTypeRef,
        keys: *const CFTypeRef,
        values: *const CFTypeRef,
        num_values: CFIndex,
        key_call_backs: *const c_void,
        value_call_backs: *const c_void,
    ) -> CFTypeRef;
}

#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    fn CMVideoFormatDescriptionCreateFromH264ParameterSets(
        allocator: CFTypeRef,
        parameter_set_count: usize,
        parameter_set_pointers: *const *const u8,
        parameter_set_sizes: *const usize,
        nal_unit_header_length: c_int,
        format_description_out: *mut CMFormatDescriptionRef,
    ) -> OSStatus;
    fn CMVideoFormatDescriptionGetDimensions(video_desc: CMFormatDescriptionRef) -> CMVideoDimensions;
    fn CMBlockBufferCreateWithMemoryBlock(
        structure_allocator: CFTypeRef,
        memory_block: *mut c_void,
        block_length: usize,
        block_allocator: CFTypeRef,
        custom_block_source: *const c_void,
        offset_to_data: usize,
        data_length: usize,
        flags: u32,
        block_buffer_out: *mut CMBlockBufferRef,
    ) -> OSStatus;
    fn CMBlockBufferReplaceDataBytes(
        source_bytes: *const c_void,
        destination_buffer: CMBlockBufferRef,
        offset_into_destination: usize,
        data_length: usize,
    ) -> OSStatus;
    fn CMSampleBufferCreateReady(
        allocator: CFTypeRef,
        data_buffer: CMBlockBufferRef,
        format_description: CMFormatDescriptionRef,
        num_samples: CFIndex,
        num_sample_timing_entries: CFIndex,
        sample_timing_array: *const CMSampleTimingInfo,
        num_sample_size_entries: CFIndex,
        sample_size_array: *const usize,
        sample_buffer_out: *mut CMSampleBufferRef,
    ) -> OSStatus;
}

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    static kCVPixelBufferPixelFormatTypeKey: CFStringRef;
    static kCVImageBufferYCbCrMatrixKey: CFStringRef;
    static kCVImageBufferYCbCrMatrix_ITU_R_601_4: CFStringRef;

    fn CVPixelBufferGetTypeID() -> CFTypeID;
    fn CVPixelBufferLockBaseAddress(pixel_buffer: CVImageBufferRef, lock_flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(pixel_buffer: CVImageBufferRef, unlock_flags: u64) -> i32;
    fn CVPixelBufferGetWidth(pixel_buffer: CVImageBufferRef) -> usize;
    fn CVPixelBufferGetHeight(pixel_buffer: CVImageBufferRef) -> usize;
    fn CVPixelBufferGetPixelFormatType(pixel_buffer: CVImageBufferRef) -> u32;
    fn CVPixelBufferIsPlanar(pixel_buffer: CVImageBufferRef) -> Boolean;
    fn CVPixelBufferGetPlaneCount(pixel_buffer: CVImageBufferRef) -> usize;
    fn CVPixelBufferGetHeightOfPlane(pixel_buffer: CVImageBufferRef, plane_index: usize) -> usize;
    fn CVPixelBufferGetBytesPerRowOfPlane(pixel_buffer: CVImageBufferRef, plane_index: usize) -> usize;
    fn CVPixelBufferGetBaseAddressOfPlane(pixel_buffer: CVImageBufferRef, plane_index: usize) -> *mut c_void;
    fn CVBufferCopyAttachment(
        buffer: CVImageBufferRef,
        key: CFStringRef,
        attachment_mode: *mut u32,
    ) -> CFTypeRef;
}

#[link(name = "VideoToolbox", kind = "framework")]
unsafe extern "C" {
    static kVTVideoDecoderSpecification_RequireHardwareAcceleratedVideoDecoder: CFStringRef;
    static kVTDecompressionPropertyKey_UsingHardwareAcceleratedVideoDecoder: CFStringRef;

    fn VTDecompressionSessionCreate(
        allocator: CFTypeRef,
        video_format_description: CMFormatDescriptionRef,
        video_decoder_specification: CFTypeRef,
        destination_image_buffer_attributes: CFTypeRef,
        output_callback: *const OutputCallbackRecord,
        decompression_session_out: *mut VTDecompressionSessionRef,
    ) -> OSStatus;
    fn VTDecompressionSessionDecodeFrame(
        session: VTDecompressionSessionRef,
        sample_buffer: CMSampleBufferRef,
        decode_flags: u32,
        source_frame_ref_con: *mut c_void,
        info_flags_out: *mut u32,
    ) -> OSStatus;
    fn VTDecompressionSessionCanAcceptFormatDescription(
        session: VTDecompressionSessionRef,
        new_format_desc: CMFormatDescriptionRef,
    ) -> Boolean;
    fn VTDecompressionSessionWaitForAsynchronousFrames(session: VTDecompressionSessionRef) -> OSStatus;
    fn VTDecompressionSessionInvalidate(session: VTDecompressionSessionRef);
    fn VTSessionCopyProperty(
        session: CFTypeRef,
        property_key: CFStringRef,
        allocator: CFTypeRef,
        property_value_out: *mut c_void,
    ) -> OSStatus;
}

const CF_NUMBER_SINT32: CFIndex = 3;
const CM_TIME_VALID: u32 = 1;
const CM_TIME_INVALID: CMTime = CMTime { value: 0, timescale: 0, flags: 0, epoch: 0 };
const CM_BLOCK_BUFFER_ASSURE_MEMORY_NOW: u32 = 1;
const CV_LOCK_READ_ONLY: u64 = 1;
const NV12_VIDEO_RANGE: u32 = u32::from_be_bytes(*b"420v");
const NV12_FULL_RANGE: u32 = u32::from_be_bytes(*b"420f");
const DECODE_INFO_FRAME_DROPPED: u32 = 1 << 1;
const DECODE_INFO_FRAME_INTERRUPTED: u32 = 1 << 4;

/// Largest width or height decoded (8K); larger streams are refused.
const MAX_DIMENSION: usize = 8192;
/// Pictures one decode call keeps, at most (one sample normally gives one).
const MAX_PENDING: usize = 4;
/// `kVTCouldNotFindVideoDecoderErr` (VTErrors.h).
const VT_COULD_NOT_FIND_VIDEO_DECODER: i32 = -12906;
/// `kVTCouldNotCreateInstanceErr`.
const VT_COULD_NOT_CREATE_INSTANCE: i32 = -12907;
/// `kVTVideoDecoderNotAvailableNowErr`.
const VT_VIDEO_DECODER_NOT_AVAILABLE_NOW: i32 = -12913;

/// Whether `status` is one of the three VideoToolbox codes that mean a
/// hardware decoder cannot be opened. Other failures (bad parameters, bad
/// data, allocation) must surface instead of being treated as "no hardware".
pub(super) fn hardware_decoder_unavailable(status: i32) -> bool {
    matches!(
        status,
        VT_COULD_NOT_FIND_VIDEO_DECODER | VT_COULD_NOT_CREATE_INSTANCE | VT_VIDEO_DECODER_NOT_AVAILABLE_NOW
    )
}

/// Hardware in use for a session that was just created.
/// An explicit VideoToolbox answer wins. When the property cannot be read,
/// a create that required hardware is hardware; otherwise unknown stays false.
pub(super) fn hardware_in_use(require_hardware: bool, reported: Option<bool>) -> bool {
    reported.unwrap_or(require_hardware)
}

/// A failure from [`Session::new`], with the OSStatus when VideoToolbox returned one.
pub(super) struct StartError {
    status: Option<i32>,
    message: String,
}

impl StartError {
    fn from_status(status: i32, doing: &str) -> StartError {
        StartError { status: Some(status), message: describe(status, doing) }
    }

    pub(super) fn hardware_unavailable(&self) -> bool {
        self.status.is_some_and(hardware_decoder_unavailable)
    }

    pub(super) fn status_code(&self) -> i32 {
        self.status.unwrap_or(0)
    }

    pub(super) fn into_message(self) -> String {
        self.message
    }
}

impl From<String> for StartError {
    fn from(message: String) -> StartError {
        StartError { status: None, message }
    }
}

impl From<&str> for StartError {
    fn from(message: &str) -> StartError {
        StartError { status: None, message: message.to_owned() }
    }
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// One reference to a Core Foundation object, released on drop.
#[derive(Debug)]
struct Owned(NonNull<c_void>);

impl Owned {
    /// Takes over the reference a Create or Copy call returned.
    fn new(object: *const c_void) -> Option<Owned> {
        NonNull::new(object.cast_mut()).map(Owned)
    }

    fn get(&self) -> *mut c_void {
        self.0.as_ptr()
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: this holds exactly one reference.
        unsafe { CFRelease(self.0.as_ptr()) }
    }
}

/// A stream's format, from its SPS and PPS.
#[derive(Debug)]
pub(super) struct Format(Owned);

impl Format {
    pub(super) fn h264(sps: &[u8], pps: &[&[u8]]) -> Result<Format, String> {
        let sets: Vec<&[u8]> = std::iter::once(sps).chain(pps.iter().copied()).collect();
        let pointers: Vec<*const u8> = sets.iter().map(|set| set.as_ptr()).collect();
        let sizes: Vec<usize> = sets.iter().map(|set| set.len()).collect();
        let mut out = ptr::null();
        // SAFETY: `pointers` and `sizes` describe `sets.len()` live slices;
        // the description comes back with one reference, ours.
        let status = unsafe {
            CMVideoFormatDescriptionCreateFromH264ParameterSets(
                ptr::null(),
                sets.len(),
                pointers.as_ptr(),
                sizes.as_ptr(),
                4,
                &mut out,
            )
        };
        let format = Owned::new(out);
        check(status, "reading the SPS and PPS")?;
        let format = Format(format.ok_or("no H.264 format description")?);
        // SAFETY: a live video format description.
        let CMVideoDimensions { width, height } =
            unsafe { CMVideoFormatDescriptionGetDimensions(format.0.get()) };
        let valid = |side: i32| usize::try_from(side).is_ok_and(|side| (1..=MAX_DIMENSION).contains(&side));
        if !valid(width) || !valid(height) {
            return Err(format!("an H.264 stream of {width}x{height}"));
        }
        Ok(format)
    }
}

/// Where the output callback leaves what it decoded.
#[derive(Debug, Default)]
struct Sink {
    frames: Mutex<Frames>,
}

#[derive(Debug, Default)]
struct Frames {
    pictures: Vec<Picture>,
    error: Option<String>,
}

/// A VideoToolbox decompression session decoding synchronously: each frame's
/// picture is out before [`Session::decode`] returns.
#[derive(Debug)]
pub(super) struct Session {
    session: Owned,
    /// Owned here (a leaked `Box`) and freed only after the session is
    /// invalidated, as the callback writes to it until then.
    sink: NonNull<Sink>,
    hardware: bool,
}

impl Session {
    /// A session for `format`; with `require_hardware`, only on Apple's
    /// hardware decoder (else VideoToolbox picks, hardware first).
    pub(super) fn new(format: &Format, require_hardware: bool) -> Result<Session, StartError> {
        // SAFETY: the dictionaries retain their keys and values; constants
        // are the frameworks' own.
        let (specification, attributes) = unsafe {
            let specification = if require_hardware {
                Some(dictionary(&[(
                    kVTVideoDecoderSpecification_RequireHardwareAcceleratedVideoDecoder,
                    kCFBooleanTrue,
                )])?)
            } else {
                None
            };
            // NV12 in whichever range the stream has; the format type says which.
            let number = |value: i32| {
                Owned::new(CFNumberCreate(ptr::null(), CF_NUMBER_SINT32, (&raw const value).cast()))
            };
            let (Some(video), Some(full)) = (number(NV12_VIDEO_RANGE as i32), number(NV12_FULL_RANGE as i32))
            else {
                return Err("CFNumberCreate failed".into());
            };
            let values = [video.get().cast_const(), full.get().cast_const()];
            let array = Owned::new(CFArrayCreate(
                ptr::null(),
                values.as_ptr(),
                values.len() as CFIndex,
                (&raw const kCFTypeArrayCallBacks).cast(),
            ))
            .ok_or("CFArrayCreate failed")?;
            (specification, dictionary(&[(kCVPixelBufferPixelFormatTypeKey, array.get().cast_const())])?)
        };
        let sink = NonNull::from(Box::leak(Box::<Sink>::default()));
        let record = OutputCallbackRecord {
            decompression_output_callback: Some(on_output),
            decompression_output_ref_con: sink.as_ptr().cast(),
        };
        let mut out = ptr::null_mut();
        // SAFETY: all arguments are live; the session copies `record` and
        // comes back with one reference, ours.
        let status = unsafe {
            VTDecompressionSessionCreate(
                ptr::null(),
                format.0.get(),
                specification.as_ref().map_or(ptr::null(), |s| s.get().cast_const()),
                attributes.get(),
                &record,
                &mut out,
            )
        };
        let Some(session) = Owned::new(out).filter(|_| status == 0) else {
            // SAFETY: no session refers to the sink.
            drop(unsafe { Box::from_raw(sink.as_ptr()) });
            return Err(StartError::from_status(status, "starting the H.264 decoder"));
        };
        // SAFETY: a live session and the framework's key. A missing answer
        // after a required-hardware create is still hardware (VTDecompressionProperties.h).
        let reported = unsafe {
            bool_property(session.get(), kVTDecompressionPropertyKey_UsingHardwareAcceleratedVideoDecoder)
        };
        let hardware = hardware_in_use(require_hardware, reported);
        Ok(Session { session, sink, hardware })
    }

    /// Whether Apple's hardware decoder runs this session, as VideoToolbox says.
    pub(super) fn on_gpu(&self) -> bool {
        self.hardware
    }

    /// Whether this session can go on with a new format (new SPS or PPS).
    pub(super) fn can_accept(&self, format: &Format) -> bool {
        // SAFETY: both are live.
        unsafe { VTDecompressionSessionCanAcceptFormatDescription(self.session.get(), format.0.get()) != 0 }
    }

    /// Decodes one access unit (`avcc`, in `format`) into its pictures.
    pub(super) fn decode(
        &mut self,
        format: &Format,
        avcc: &[u8],
        presentation_us: i64,
    ) -> Result<Vec<Picture>, String> {
        let sample = sample(format, avcc, presentation_us)?;
        let mut info = 0;
        // SAFETY: both are live. With no flags, the callback has run by the
        // time this returns (VTDecompressionSession.h).
        let status = unsafe {
            VTDecompressionSessionDecodeFrame(self.session.get(), sample.get(), 0, ptr::null_mut(), &mut info)
        };
        let frames = std::mem::take(&mut *self.sink().frames.lock().unwrap_or_else(|e| e.into_inner()));
        check(status, "decoding H.264")?;
        match frames.error {
            Some(error) => Err(error),
            None => Ok(frames.pictures),
        }
    }

    fn sink(&self) -> &Sink {
        // SAFETY: freed only in `drop`.
        unsafe { self.sink.as_ref() }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: once invalidated the session calls back no more, so the
        // sink can go; the session's reference is released after this.
        unsafe {
            VTDecompressionSessionWaitForAsynchronousFrames(self.session.get());
            VTDecompressionSessionInvalidate(self.session.get());
            drop(Box::from_raw(self.sink.as_ptr()));
        }
    }
}

/// A sample buffer holding `avcc`, a copy CoreMedia allocates.
fn sample(format: &Format, avcc: &[u8], presentation_us: i64) -> Result<Owned, String> {
    if avcc.is_empty() {
        return Err("an empty access unit".into());
    }
    let len = avcc.len();
    let mut block = ptr::null_mut();
    // SAFETY: CoreMedia allocates `len` bytes now, with the default allocator.
    let status = unsafe {
        CMBlockBufferCreateWithMemoryBlock(
            ptr::null(),
            ptr::null_mut(),
            len,
            ptr::null(),
            ptr::null(),
            0,
            len,
            CM_BLOCK_BUFFER_ASSURE_MEMORY_NOW,
            &mut block,
        )
    };
    let block = Owned::new(block);
    check(status, "allocating a sample")?;
    let block = block.ok_or("no block buffer")?;
    // SAFETY: `avcc` is `len` bytes, as is the block.
    let status = unsafe { CMBlockBufferReplaceDataBytes(avcc.as_ptr().cast(), block.get(), 0, len) };
    check(status, "copying a sample")?;
    let timing = CMSampleTimingInfo {
        duration: CM_TIME_INVALID,
        presentation_time_stamp: CMTime {
            value: presentation_us,
            timescale: 1_000_000,
            flags: CM_TIME_VALID,
            epoch: 0,
        },
        decode_time_stamp: CM_TIME_INVALID,
    };
    let mut sample = ptr::null_mut();
    // SAFETY: one sample of `len` bytes with one timing entry, all live;
    // the sample comes back with one reference, ours.
    let status = unsafe {
        CMSampleBufferCreateReady(
            ptr::null(),
            block.get(),
            format.0.get(),
            1,
            1,
            &timing,
            1,
            &len,
            &mut sample,
        )
    };
    let sample = Owned::new(sample);
    check(status, "making a sample")?;
    sample.ok_or_else(|| "no sample buffer".into())
}

/// The session's output callback: copies the picture out, or notes the error.
unsafe extern "C" fn on_output(
    sink: *mut c_void,
    _frame: *mut c_void,
    status: OSStatus,
    info: u32,
    image: CVImageBufferRef,
    _presentation: CMTime,
    _duration: CMTime,
) {
    // A panic must not unwind into VideoToolbox.
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: the session's sink, alive until the session is invalidated.
        let sink = unsafe { &*sink.cast::<Sink>() };
        let result = if status != 0 {
            Err(describe(status, "decoding H.264"))
        } else if info & (DECODE_INFO_FRAME_DROPPED | DECODE_INFO_FRAME_INTERRUPTED) != 0 {
            // Dropped, or decoded but not handed over (maybe black): nothing to show.
            return;
        } else if image.is_null() {
            Err("VideoToolbox decoded no picture".into())
        } else {
            // SAFETY: VideoToolbox keeps `image` alive during the callback.
            unsafe { copy_picture(image) }
        };
        let mut frames = sink.frames.lock().unwrap_or_else(|e| e.into_inner());
        match result {
            Ok(picture) => {
                if frames.pictures.len() == MAX_PENDING {
                    frames.pictures.remove(0);
                }
                frames.pictures.push(picture);
            }
            Err(error) => {
                frames.error.get_or_insert(error);
            }
        }
    }));
}

/// Copies an NV12 pixel buffer into a tightly packed [`Picture`].
///
/// # Safety
/// `image` is a live CoreVideo image buffer.
unsafe fn copy_picture(image: CVImageBufferRef) -> Result<Picture, String> {
    // SAFETY: CoreVideo queries on a live buffer; planes are read only while
    // locked, and only at a checked offset inside the capped stride.
    unsafe {
        if CFGetTypeID(image) != CVPixelBufferGetTypeID() {
            return Err("VideoToolbox decoded into something other than a pixel buffer".into());
        }
        let full_range = match CVPixelBufferGetPixelFormatType(image) {
            NV12_VIDEO_RANGE => false,
            NV12_FULL_RANGE => true,
            other => {
                let code = String::from_utf8_lossy(&other.to_be_bytes()).into_owned();
                return Err(format!("VideoToolbox decoded into '{code}', not NV12"));
            }
        };
        if CVPixelBufferIsPlanar(image) == 0 || CVPixelBufferGetPlaneCount(image) != 2 {
            return Err("an NV12 picture without two planes".into());
        }
        let (width, height) = (CVPixelBufferGetWidth(image), CVPixelBufferGetHeight(image));
        if !(1..=MAX_DIMENSION).contains(&width) || !(1..=MAX_DIMENSION).contains(&height) {
            return Err(format!("a decoded picture of {width}x{height}"));
        }
        // Chroma rows hold `width` rounded up to even bytes (Cb, Cr pairs).
        let stride = width.next_multiple_of(2);
        let chroma_rows = height.div_ceil(2);
        let _lock = Lock::new(image)?;
        let (luma, luma_pitch) = plane(image, 0, width, height)?;
        let (chroma, chroma_pitch) = plane(image, 1, stride, chroma_rows)?;
        let mut nv12 = vec![0; stride * (height + chroma_rows)];
        let (luma_out, chroma_out) = nv12.split_at_mut(stride * height);
        for (row, out) in luma_out.chunks_exact_mut(stride).enumerate() {
            let offset = plane_row_offset(row, luma_pitch, width)?;
            // `offset` is `row * pitch` and does not wrap; the row holds `width` bytes.
            out[..width].copy_from_slice(slice::from_raw_parts(luma.add(offset), width));
        }
        for (row, out) in chroma_out.chunks_exact_mut(stride).enumerate() {
            let offset = plane_row_offset(row, chroma_pitch, stride)?;
            out.copy_from_slice(slice::from_raw_parts(chroma.add(offset), stride));
        }
        Ok(Picture {
            width: width as u32,
            height: height as u32,
            stride,
            plane_rows: height,
            nv12,
            colors: Colors { bt601: matrix_is_bt601(image), full_range },
        })
    }
}

/// A plane's start and bytes per row, if it holds `rows` rows of `row_bytes`.
///
/// # Safety
/// `image` is a live pixel buffer, locked.
unsafe fn plane(
    image: CVImageBufferRef,
    index: usize,
    row_bytes: usize,
    rows: usize,
) -> Result<(*const u8, usize), String> {
    // SAFETY: as the caller promises.
    let (base, pitch, plane_rows) = unsafe {
        (
            CVPixelBufferGetBaseAddressOfPlane(image, index).cast::<u8>().cast_const(),
            CVPixelBufferGetBytesPerRowOfPlane(image, index),
            CVPixelBufferGetHeightOfPlane(image, index),
        )
    };
    if base.is_null() || rows == 0 || plane_rows < rows {
        return Err(format!(
            "NV12 plane {index} is {pitch} bytes x {plane_rows} rows, short of {row_bytes} x {rows}"
        ));
    }
    // The last row's offset must fit. A later row in the copy uses the same check.
    plane_row_offset(rows - 1, pitch, row_bytes).map_err(|error| format!("NV12 plane {index}: {error}"))?;
    Ok((base, pitch))
}

/// Largest bytes-per-row accepted for a row of `row_bytes` pixels: sixteen
/// times the row, and at least 4 KiB of padding, so alignment is allowed and
/// a corrupt stride cannot be indexed.
pub(super) fn max_plane_pitch(row_bytes: usize) -> usize {
    row_bytes.saturating_mul(16).max(row_bytes.saturating_add(4096))
}

/// Byte offset of `row` when each row is `pitch` bytes and `row_bytes` are read.
/// `pitch` must sit between `row_bytes` and [`max_plane_pitch`], and the
/// multiply must not wrap.
pub(super) fn plane_row_offset(row: usize, pitch: usize, row_bytes: usize) -> Result<usize, String> {
    let cap = max_plane_pitch(row_bytes);
    if row_bytes == 0 || pitch < row_bytes || pitch > cap {
        return Err(format!("an NV12 stride of {pitch} bytes for a row of {row_bytes}"));
    }
    let offset = row.checked_mul(pitch).ok_or("an NV12 plane stride overflows")?;
    offset.checked_add(row_bytes).ok_or("an NV12 plane stride overflows")?;
    Ok(offset)
}

/// Whether the picture is tagged BT.601 (the stream's VUI, as VideoToolbox
/// passes it on); untagged pictures are taken as BT.709, as phones are asked.
///
/// # Safety
/// `image` is a live CoreVideo buffer.
unsafe fn matrix_is_bt601(image: CVImageBufferRef) -> bool {
    // SAFETY: the attachment comes back with one reference, ours.
    unsafe {
        Owned::new(CVBufferCopyAttachment(image, kCVImageBufferYCbCrMatrixKey, ptr::null_mut()))
            .is_some_and(|matrix| CFEqual(matrix.get(), kCVImageBufferYCbCrMatrix_ITU_R_601_4) != 0)
    }
}

/// A pixel buffer's base address locked for reading, unlocked on drop.
struct Lock(CVImageBufferRef);

impl Lock {
    /// # Safety
    /// `image` is a live pixel buffer that outlives the lock.
    unsafe fn new(image: CVImageBufferRef) -> Result<Lock, String> {
        // SAFETY: as the caller promises.
        let status = unsafe { CVPixelBufferLockBaseAddress(image, CV_LOCK_READ_ONLY) };
        if status != 0 {
            return Err(format!("CVPixelBufferLockBaseAddress failed ({status})"));
        }
        Ok(Lock(image))
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        // SAFETY: locked in `new` with the same flags.
        unsafe { CVPixelBufferUnlockBaseAddress(self.0, CV_LOCK_READ_ONLY) };
    }
}

/// A CFDictionary of `pairs`, which it retains.
///
/// # Safety
/// Every key and value is a live CF object.
unsafe fn dictionary(pairs: &[(CFTypeRef, CFTypeRef)]) -> Result<Owned, String> {
    let keys: Vec<CFTypeRef> = pairs.iter().map(|&(key, _)| key).collect();
    let values: Vec<CFTypeRef> = pairs.iter().map(|&(_, value)| value).collect();
    // SAFETY: `keys` and `values` hold `pairs.len()` live objects; the
    // callback tables are CoreFoundation's own.
    unsafe {
        Owned::new(CFDictionaryCreate(
            ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            pairs.len() as CFIndex,
            (&raw const kCFTypeDictionaryKeyCallBacks).cast(),
            (&raw const kCFTypeDictionaryValueCallBacks).cast(),
        ))
    }
    .ok_or_else(|| "CFDictionaryCreate failed".into())
}

/// A session's boolean property, if it has one.
///
/// # Safety
/// `session` is a live VideoToolbox session; `key` a CFString.
unsafe fn bool_property(session: VTDecompressionSessionRef, key: CFStringRef) -> Option<bool> {
    let mut value: CFTypeRef = ptr::null();
    // SAFETY: the value comes back with one reference, ours; its type is
    // checked before it's read as a boolean.
    unsafe {
        let status = VTSessionCopyProperty(session, key, ptr::null(), (&raw mut value).cast());
        let value = Owned::new(value);
        if status != 0 {
            return None;
        }
        let value = value?;
        (CFGetTypeID(value.get()) == CFBooleanGetTypeID()).then(|| CFBooleanGetValue(value.get()) != 0)
    }
}

fn check(status: OSStatus, doing: &str) -> Result<(), String> {
    if status == 0 { Ok(()) } else { Err(describe(status, doing)) }
}

/// An OSStatus from VideoToolbox or CoreMedia, named where VTErrors.h names it.
fn describe(status: OSStatus, doing: &str) -> String {
    let name = match status {
        -12902 => "kVTParameterErr",
        -12903 => "kVTInvalidSessionErr",
        -12904 => "kVTAllocationFailedErr",
        VT_COULD_NOT_FIND_VIDEO_DECODER => "kVTCouldNotFindVideoDecoderErr",
        VT_COULD_NOT_CREATE_INSTANCE => "kVTCouldNotCreateInstanceErr",
        -12909 => "kVTVideoDecoderBadDataErr",
        -12910 => "kVTVideoDecoderUnsupportedDataFormatErr",
        -12911 => "kVTVideoDecoderMalfunctionErr",
        VT_VIDEO_DECODER_NOT_AVAILABLE_NOW => "kVTVideoDecoderNotAvailableNowErr",
        -12916 => "kVTFormatDescriptionChangeNotSupportedErr",
        -17690 => "kVTVideoDecoderRemovedErr",
        -17691 => "kVTSessionMalfunctionErr",
        -17694 => "kVTVideoDecoderReferenceMissingErr",
        _ => "",
    };
    if name.is_empty() {
        format!("{doing} failed ({status})")
    } else {
        format!("{doing} failed: {name} ({status})")
    }
}
