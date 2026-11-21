// SPDX-License-Identifier: GPL-3.0-or-later
//! Low-latency 16-bit signed little-endian PCM playback on macOS via
//! `AudioToolbox`'s `AudioQueue` API, backed by a small jitter buffer.

#![allow(unsafe_code)]

use std::{
    collections::VecDeque,
    ffi::c_void,
    io::{Error, ErrorKind, Result},
    ptr, slice,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

const BYTES_PER_SAMPLE: usize = 2;
const START_MS: usize = 40;
const MOST_MS: usize = 200;
const TRIM_TO_MS: usize = 60;
const BUFFER_MS: usize = 20;
const BUFFER_COUNT: usize = 4;

const K_AUDIO_FORMAT_LINEAR_PCM: u32 = u32::from_be_bytes(*b"lpcm");
const K_AUDIO_FORMAT_FLAG_IS_SIGNED_INTEGER: u32 = 1 << 2;
const K_AUDIO_FORMAT_FLAG_IS_PACKED: u32 = 1 << 3;

type OSStatus = i32;
type AudioQueueRef = *mut c_void;
type AudioQueueBufferRef = *mut AudioQueueBuffer;

#[repr(C)]
#[derive(Clone, Copy)]
struct AudioStreamBasicDescription {
    m_sample_rate: f64,
    m_format_id: u32,
    m_format_flags: u32,
    m_bytes_per_packet: u32,
    m_frames_per_packet: u32,
    m_bytes_per_frame: u32,
    m_channels_per_frame: u32,
    m_bits_per_channel: u32,
    m_reserved: u32,
}

#[repr(C)]
struct AudioQueueBuffer {
    m_audio_data_bytes_capacity: u32,
    m_audio_data: *mut c_void,
    m_audio_data_byte_size: u32,
    m_user_data: *mut c_void,
    m_packet_description_capacity: u32,
    m_packet_descriptions: *mut c_void,
    m_packet_description_count: u32,
}

type AudioQueueOutputCallback =
    unsafe extern "C" fn(in_user_data: *mut c_void, in_aq: AudioQueueRef, in_buffer: AudioQueueBufferRef);

#[link(name = "AudioToolbox", kind = "framework")]
unsafe extern "C" {
    fn AudioQueueNewOutput(
        in_format: *const AudioStreamBasicDescription,
        in_callback_proc: AudioQueueOutputCallback,
        in_user_data: *mut c_void,
        in_callback_run_loop: *mut c_void,
        in_callback_run_loop_mode: *const c_void,
        in_flags: u32,
        out_aq: *mut AudioQueueRef,
    ) -> OSStatus;
    fn AudioQueueAllocateBuffer(
        in_aq: AudioQueueRef,
        in_buffer_byte_size: u32,
        out_buffer: *mut AudioQueueBufferRef,
    ) -> OSStatus;
    fn AudioQueueEnqueueBuffer(
        in_aq: AudioQueueRef,
        in_buffer: AudioQueueBufferRef,
        in_num_packet_descs: u32,
        in_packet_descs: *const c_void,
    ) -> OSStatus;
    fn AudioQueueStart(in_aq: AudioQueueRef, in_start_time: *const c_void) -> OSStatus;
    fn AudioQueueStop(in_aq: AudioQueueRef, in_immediate: u8) -> OSStatus;
    fn AudioQueueDispose(in_aq: AudioQueueRef, in_immediate: u8) -> OSStatus;
}

/// How many [`Player`]s are alive.
static PLAYERS: AtomicUsize = AtomicUsize::new(0);

/// Whether NectarLink is playing a phone's sound on this Mac right now.
#[allow(dead_code)]
pub fn playing() -> bool {
    PLAYERS.load(Ordering::Relaxed) > 0
}

#[derive(Debug)]
struct Counted;

impl Counted {
    fn new() -> Self {
        PLAYERS.fetch_add(1, Ordering::Relaxed);
        Self
    }
}

impl Drop for Counted {
    fn drop(&mut self) {
        PLAYERS.fetch_sub(1, Ordering::Relaxed);
    }
}

#[derive(Debug)]
struct Jitter {
    pcm: VecDeque<u8>,
    frame_bytes: usize,
    start_bytes: usize,
    trim_bytes: usize,
    most_bytes: usize,
    primed: bool,
}

impl Jitter {
    #[cfg(test)]
    fn new(rate: u32, channels: u16) -> Self {
        Self::with_buffer(rate, channels, START_MS)
    }

    /// `start_ms` from the buffer setting (#321); the trim target stays above
    /// it, and 40 gives the old 40 / 200 / 60.
    fn with_buffer(rate: u32, channels: u16, start_ms: usize) -> Self {
        let start_ms = start_ms.clamp(20, 500);
        let trim_ms = if start_ms == START_MS { TRIM_TO_MS } else { start_ms + (start_ms / 2).max(20) };
        let most_ms = if start_ms == START_MS { MOST_MS } else { trim_ms + 140 };
        let frame_bytes = (channels as usize).max(1) * BYTES_PER_SAMPLE;
        let bytes_per_ms = ((rate as usize) * frame_bytes / 1000).max(frame_bytes);
        let align = |ms: usize| ((ms * bytes_per_ms).max(frame_bytes) / frame_bytes) * frame_bytes;
        Self {
            pcm: VecDeque::with_capacity(align(most_ms)),
            frame_bytes,
            start_bytes: align(start_ms),
            trim_bytes: align(trim_ms),
            most_bytes: align(most_ms),
            primed: false,
        }
    }

    fn push(&mut self, data: &[u8]) {
        let usable = (data.len() / self.frame_bytes) * self.frame_bytes;
        if usable == 0 {
            return;
        }
        self.pcm.extend(&data[..usable]);
        if self.pcm.len() > self.most_bytes {
            let drop = ((self.pcm.len() - self.trim_bytes) / self.frame_bytes) * self.frame_bytes;
            self.pcm.drain(..drop.min(self.pcm.len()));
        }
        if self.pcm.len() >= self.start_bytes {
            self.primed = true;
        }
    }

    fn pull(&mut self, dst: &mut [u8]) {
        if !self.primed || self.pcm.len() < dst.len() {
            dst.fill(0);
            if self.pcm.len() < dst.len() {
                self.primed = false;
            }
            return;
        }
        for byte in dst.iter_mut() {
            *byte = self.pcm.pop_front().unwrap_or(0);
        }
    }
}

struct Shared {
    jitter: Mutex<Jitter>,
    stopping: AtomicBool,
}

/// Streams 16-bit signed little-endian PCM to the default macOS output device.
#[derive(Debug)]
pub struct Player {
    queue: AudioQueueRef,
    shared: *const Shared,
    _counted: Counted,
}

// SAFETY: `AudioQueueRef` and `Shared` are synchronized across threads; `AudioQueueStop(..., 1)`
// and `AudioQueueDispose(..., 1)` tear down the queue synchronously in `Drop`.
unsafe impl Send for Player {}
unsafe impl Sync for Player {}

unsafe extern "C" fn output_callback(
    in_user_data: *mut c_void,
    in_aq: AudioQueueRef,
    in_buffer: AudioQueueBufferRef,
) {
    if in_user_data.is_null() || in_aq.is_null() || in_buffer.is_null() {
        return;
    }
    // SAFETY: `in_user_data` points to `Shared` kept alive until after `AudioQueueDispose`.
    let shared = unsafe { &*(in_user_data as *const Shared) };
    if shared.stopping.load(Ordering::Acquire) {
        return;
    }
    // SAFETY: `in_buffer` was allocated with `m_audio_data_bytes_capacity` bytes of writable memory.
    let buf = unsafe { &mut *in_buffer };
    let cap = buf.m_audio_data_bytes_capacity as usize;
    if !buf.m_audio_data.is_null() && cap > 0 {
        let dst = unsafe { slice::from_raw_parts_mut(buf.m_audio_data.cast::<u8>(), cap) };
        if let Ok(mut jitter) = shared.jitter.lock() {
            jitter.pull(dst);
        } else {
            dst.fill(0);
        }
        buf.m_audio_data_byte_size = cap as u32;
        // SAFETY: `in_aq` and `in_buffer` are valid and not stopping.
        let _ = unsafe { AudioQueueEnqueueBuffer(in_aq, in_buffer, 0, ptr::null()) };
    }
}

impl Player {
    pub fn start(rate: u32, channels: u16) -> Result<Player> {
        Self::start_on(None, rate, channels)
    }

    pub fn start_on(_device: Option<String>, rate: u32, channels: u16) -> Result<Player> {
        if rate == 0 || channels == 0 {
            return Err(Error::new(ErrorKind::InvalidInput, "rate and channels must be positive"));
        }
        let frame_bytes = (channels as usize) * BYTES_PER_SAMPLE;
        let buffer_bytes = (((rate as usize) * BUFFER_MS / 1000).max(1) * frame_bytes) as u32;
        let asbd = AudioStreamBasicDescription {
            m_sample_rate: f64::from(rate),
            m_format_id: K_AUDIO_FORMAT_LINEAR_PCM,
            m_format_flags: K_AUDIO_FORMAT_FLAG_IS_SIGNED_INTEGER | K_AUDIO_FORMAT_FLAG_IS_PACKED,
            m_bytes_per_packet: frame_bytes as u32,
            m_frames_per_packet: 1,
            m_bytes_per_frame: frame_bytes as u32,
            m_channels_per_frame: u32::from(channels),
            m_bits_per_channel: 16,
            m_reserved: 0,
        };
        let shared = Arc::into_raw(Arc::new(Shared {
            jitter: Mutex::new(Jitter::with_buffer(
                rate,
                channels,
                crate::settings::phone_sound_buffer_ms() as usize,
            )),
            stopping: AtomicBool::new(false),
        }));
        let mut queue: AudioQueueRef = ptr::null_mut();
        // SAFETY: `asbd` and `queue` are stack-allocated with valid layout; `shared` stays live until `Drop`.
        let status = unsafe {
            AudioQueueNewOutput(
                &asbd,
                output_callback,
                shared.cast::<c_void>().cast_mut(),
                ptr::null_mut(),
                ptr::null(),
                0,
                &mut queue,
            )
        };
        if status != 0 || queue.is_null() {
            // SAFETY: queue creation failed, so no callback holds `shared`.
            unsafe { drop(Arc::from_raw(shared)) };
            return Err(osstatus_err("AudioQueueNewOutput", status));
        }
        for _ in 0..BUFFER_COUNT {
            let mut buf: AudioQueueBufferRef = ptr::null_mut();
            // SAFETY: `queue` is a valid AudioQueueRef.
            let alloc_status = unsafe { AudioQueueAllocateBuffer(queue, buffer_bytes, &mut buf) };
            if alloc_status != 0 || buf.is_null() {
                // SAFETY: `queue` is disposed synchronously before freeing `shared`.
                unsafe {
                    AudioQueueDispose(queue, 1);
                    drop(Arc::from_raw(shared));
                }
                return Err(osstatus_err("AudioQueueAllocateBuffer", alloc_status));
            }
            // SAFETY: `buf` was just allocated with `buffer_bytes` capacity.
            unsafe {
                let raw = (*buf).m_audio_data.cast::<u8>();
                if !raw.is_null() {
                    ptr::write_bytes(raw, 0, buffer_bytes as usize);
                }
                (*buf).m_audio_data_byte_size = buffer_bytes;
                let enq_status = AudioQueueEnqueueBuffer(queue, buf, 0, ptr::null());
                if enq_status != 0 {
                    AudioQueueDispose(queue, 1);
                    drop(Arc::from_raw(shared));
                    return Err(osstatus_err("AudioQueueEnqueueBuffer", enq_status));
                }
            }
        }
        // SAFETY: all buffers are enqueued; start playback immediately.
        let start_status = unsafe { AudioQueueStart(queue, ptr::null()) };
        if start_status != 0 {
            // SAFETY: `queue` failed to start; dispose before dropping `shared`.
            unsafe {
                AudioQueueDispose(queue, 1);
                drop(Arc::from_raw(shared));
            }
            return Err(osstatus_err("AudioQueueStart", start_status));
        }
        Ok(Player { queue, shared, _counted: Counted::new() })
    }

    pub fn push(&self, data: Vec<u8>) {
        if data.is_empty() {
            return;
        }
        // SAFETY: `self.shared` remains valid for the entire lifetime of `Player`.
        let shared = unsafe { &*self.shared };
        if let Ok(mut jitter) = shared.jitter.lock() {
            jitter.push(&data);
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        // SAFETY: `self.shared` is valid; stopping synchronously drains all pending callbacks before freeing.
        unsafe {
            (&*self.shared).stopping.store(true, Ordering::Release);
            AudioQueueStop(self.queue, 1);
            AudioQueueDispose(self.queue, 1);
            drop(Arc::from_raw(self.shared));
        }
    }
}

fn osstatus_err(op: &str, status: OSStatus) -> Error {
    // kAudioUnitErr_NoConnection / missing hardware (-66681 / -50 / -10851)
    let kind = if status == -10851 || status == -50 { ErrorKind::InvalidInput } else { ErrorKind::Other };
    Error::new(kind, format!("{op} failed (OSStatus {status})"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jitter_outputs_silence_until_primed_then_drains() {
        let mut jitter = Jitter::new(48_000, 2);
        let mut out = vec![0xFFu8; 32];
        jitter.push(&[0x12u8; 16]);
        jitter.pull(&mut out);
        assert!(out.iter().all(|&b| b == 0));

        jitter.push(&vec![0x55u8; jitter.start_bytes]);
        jitter.pull(&mut out);
        assert!(out[..16].iter().all(|&b| b == 0x12));
        assert!(out[16..].iter().all(|&b| b == 0x55));
    }

    #[test]
    fn jitter_caps_overflow_to_trim_depth() {
        let mut jitter = Jitter::new(48_000, 2);
        jitter.push(&vec![0x33u8; jitter.most_bytes + 4096]);
        assert!(jitter.pcm.len() <= jitter.most_bytes);
        assert!(jitter.pcm.len() >= jitter.trim_bytes);
    }

    #[test]
    fn player_starts_accepts_pushes_and_drops_cleanly() {
        match Player::start(48_000, 2) {
            Ok(player) => {
                assert!(playing());
                player.push(vec![0u8; 1920]);
                drop(player);
                assert!(!playing());
            }
            Err(e) => {
                assert_ne!(e.kind(), ErrorKind::Unsupported);
            }
        }
    }
}
