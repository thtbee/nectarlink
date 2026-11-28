// SPDX-License-Identifier: GPL-3.0-or-later
//! Records the Mac's microphone as 16-bit signed little-endian PCM through
//! `AudioToolbox`'s `AudioQueue` API, for a phone call's sound
//! (`call_audio.rs`). The mirror of [`super::audio_out`].

#![allow(unsafe_code)]

use std::{
    ffi::c_void,
    ptr, slice,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
};

use super::audio::{CfString, default_input_device};

/// How long each piece of sound is, and how many are being filled at a time.
const BUFFER_MS: usize = 20;
const BUFFER_COUNT: usize = 4;

const K_AUDIO_FORMAT_LINEAR_PCM: u32 = u32::from_be_bytes(*b"lpcm");
const K_AUDIO_FORMAT_FLAG_IS_SIGNED_INTEGER: u32 = 1 << 2;
const K_AUDIO_FORMAT_FLAG_IS_PACKED: u32 = 1 << 3;
/// `kAudioQueueProperty_CurrentDevice`: the device's UID, as a CFString.
const K_AUDIO_QUEUE_PROPERTY_CURRENT_DEVICE: u32 = u32::from_be_bytes(*b"aqcd");

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

type AudioQueueInputCallback = unsafe extern "C" fn(
    in_user_data: *mut c_void,
    in_aq: AudioQueueRef,
    in_buffer: AudioQueueBufferRef,
    in_start_time: *const c_void,
    in_num_packets: u32,
    in_packet_descs: *const c_void,
);

#[link(name = "AudioToolbox", kind = "framework")]
unsafe extern "C" {
    fn AudioQueueNewInput(
        in_format: *const AudioStreamBasicDescription,
        in_callback_proc: AudioQueueInputCallback,
        in_user_data: *mut c_void,
        in_callback_run_loop: *mut c_void,
        in_callback_run_loop_mode: *const c_void,
        in_flags: u32,
        out_aq: *mut AudioQueueRef,
    ) -> OSStatus;
    fn AudioQueueSetProperty(
        in_aq: AudioQueueRef,
        in_id: u32,
        in_data: *const c_void,
        in_data_size: u32,
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

unsafe extern "C" {
    /// `mac_audio_loopback.mm`: an AVAuthorizationStatus (0 not asked, 1
    /// restricted, 2 denied, 3 allowed), asking first; -1 when the app
    /// can't ask (not run from its bundle).
    fn mac_microphone_access() -> i32;
}

/// Why the microphone can't be recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MicError {
    /// macOS' privacy settings keep NectarLink off the microphone.
    Denied,
    /// No microphone is plugged in or turned on.
    Missing,
    /// Something else, as macOS said it.
    Other(String),
}

impl std::fmt::Display for MicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MicError::Denied => f.write_str("access to the microphone is off in macOS' privacy settings"),
            MicError::Missing => f.write_str("no microphone"),
            MicError::Other(e) => f.write_str(e),
        }
    }
}

/// Whether NectarLink may record the microphone, asking the person the first time and
/// waiting for the answer (up to two minutes). Call it before a call's sound moves here
/// or when a microphone is chosen, off the UI and runtime threads, so the other party
/// isn't left in silence while macOS asks.
pub fn request_access() -> Result<(), MicError> {
    // SAFETY: takes no arguments.
    if matches!(unsafe { mac_microphone_access() }, 0..=2) { Err(MicError::Denied) } else { Ok(()) }
}

/// What the audio queue's thread hands over: each piece of sound goes down
/// `pcm` to the recorder's own thread, so a slow `on_pcm` never holds the
/// audio queue up.
struct Shared {
    pcm: mpsc::Sender<Vec<u8>>,
    stopping: AtomicBool,
}

/// Records the Mac's microphone until dropped.
pub struct Recorder {
    queue: AudioQueueRef,
    shared: *const Shared,
    worker: Option<JoinHandle<()>>,
}

// SAFETY: `AudioQueueRef` is only used to stop and dispose the queue, which
// is safe from any thread, and `Shared` is made for sharing.
unsafe impl Send for Recorder {}

unsafe extern "C" fn input_callback(
    in_user_data: *mut c_void,
    in_aq: AudioQueueRef,
    in_buffer: AudioQueueBufferRef,
    _start_time: *const c_void,
    _num_packets: u32,
    _packet_descs: *const c_void,
) {
    if in_user_data.is_null() || in_aq.is_null() || in_buffer.is_null() {
        return;
    }
    // SAFETY: `in_user_data` is the `Shared` kept alive until after `AudioQueueDispose`.
    let shared = unsafe { &*(in_user_data as *const Shared) };
    if shared.stopping.load(Ordering::Acquire) {
        return;
    }
    // SAFETY: a buffer of this queue, holding `m_audio_data_byte_size` bytes of sound.
    let buffer = unsafe { &*in_buffer };
    let len = buffer.m_audio_data_byte_size as usize;
    if !buffer.m_audio_data.is_null() && len > 0 {
        // SAFETY: as above.
        let pcm = unsafe { slice::from_raw_parts(buffer.m_audio_data.cast::<u8>(), len) }.to_vec();
        let _ = shared.pcm.send(pcm);
    }
    // SAFETY: the queue and buffer are valid and the queue isn't stopping.
    let _ = unsafe { AudioQueueEnqueueBuffer(in_aq, in_buffer, 0, ptr::null()) };
}

impl Recorder {
    /// Starts recording from `device` (a UID from [`super::audio::input_devices`]; the
    /// system's default microphone when `None` or gone) as mono PCM at `rate`;
    /// `on_pcm` gets each piece of sound as it comes, on the recorder's thread.
    /// The first time, macOS asks the person to allow it, and this waits for
    /// the answer.
    pub fn start(
        device: Option<String>,
        rate: u32,
        mut on_pcm: impl FnMut(Vec<u8>) + Send + 'static,
    ) -> Result<Recorder, MicError> {
        if rate == 0 {
            return Err(MicError::Other("the sample rate must be positive".into()));
        }
        request_access()?;
        if default_input_device().is_none() {
            return Err(MicError::Missing);
        }
        let asbd = AudioStreamBasicDescription {
            m_sample_rate: f64::from(rate),
            m_format_id: K_AUDIO_FORMAT_LINEAR_PCM,
            m_format_flags: K_AUDIO_FORMAT_FLAG_IS_SIGNED_INTEGER | K_AUDIO_FORMAT_FLAG_IS_PACKED,
            m_bytes_per_packet: 2,
            m_frames_per_packet: 1,
            m_bytes_per_frame: 2,
            m_channels_per_frame: 1,
            m_bits_per_channel: 16,
            m_reserved: 0,
        };
        let (pcm, pieces) = mpsc::channel();
        let shared = Arc::into_raw(Arc::new(Shared { pcm, stopping: AtomicBool::new(false) }));
        let mut queue: AudioQueueRef = ptr::null_mut();
        // SAFETY: `asbd` and `queue` are on the stack; `shared` lives until the queue is disposed.
        let status = unsafe {
            AudioQueueNewInput(
                &asbd,
                input_callback,
                shared.cast::<c_void>().cast_mut(),
                ptr::null_mut(),
                ptr::null(),
                0,
                &mut queue,
            )
        };
        let release = |queue: AudioQueueRef| {
            // SAFETY: stops the queue for good before the `Shared` it calls is freed.
            unsafe {
                if !queue.is_null() {
                    AudioQueueDispose(queue, 1);
                }
                drop(Arc::from_raw(shared));
            }
        };
        if status != 0 || queue.is_null() {
            release(ptr::null_mut());
            return Err(osstatus_err("AudioQueueNewInput", status));
        }
        // The chosen microphone; the default one when it's gone or not given.
        if let Some(uid) = device.as_deref().and_then(CfString::new) {
            let raw = uid.as_raw();
            // SAFETY: the property takes a CFStringRef, which the queue retains.
            let _ = unsafe {
                AudioQueueSetProperty(
                    queue,
                    K_AUDIO_QUEUE_PROPERTY_CURRENT_DEVICE,
                    (&raw const raw).cast::<c_void>(),
                    size_of::<*const c_void>() as u32,
                )
            };
        }
        let buffer_bytes = (rate as usize * BUFFER_MS / 1000).max(1) as u32 * 2;
        for _ in 0..BUFFER_COUNT {
            let mut buffer: AudioQueueBufferRef = ptr::null_mut();
            // SAFETY: `queue` is a valid queue.
            let status = unsafe { AudioQueueAllocateBuffer(queue, buffer_bytes, &mut buffer) };
            if status != 0 || buffer.is_null() {
                release(queue);
                return Err(osstatus_err("AudioQueueAllocateBuffer", status));
            }
            // SAFETY: the buffer was just allocated for this queue.
            let status = unsafe { AudioQueueEnqueueBuffer(queue, buffer, 0, ptr::null()) };
            if status != 0 {
                release(queue);
                return Err(osstatus_err("AudioQueueEnqueueBuffer", status));
            }
        }
        // SAFETY: every buffer is enqueued.
        let status = unsafe { AudioQueueStart(queue, ptr::null()) };
        if status != 0 {
            release(queue);
            return Err(osstatus_err("AudioQueueStart", status));
        }
        let worker = thread::Builder::new()
            .name("pc-mic".into())
            .spawn(move || {
                while let Ok(pcm) = pieces.recv() {
                    on_pcm(pcm);
                }
            })
            .map_err(|e| {
                release(queue);
                MicError::Other(e.to_string())
            })?;
        Ok(Recorder { queue, shared, worker: Some(worker) })
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        // SAFETY: `shared` is valid until the last line; stopping the queue
        // immediately waits for its callbacks to finish before it's freed.
        unsafe {
            (&*self.shared).stopping.store(true, Ordering::Release);
            AudioQueueStop(self.queue, 1);
            AudioQueueDispose(self.queue, 1);
            // The last sender goes with it, which ends the worker's loop.
            drop(Arc::from_raw(self.shared));
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn osstatus_err(op: &str, status: OSStatus) -> MicError {
    MicError::Other(format!("{op} failed (OSStatus {status})"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_a_zero_rate() {
        assert!(matches!(Recorder::start(None, 0, |_| {}), Err(MicError::Other(_))));
    }

    /// Needs a microphone, and permission for whatever runs the test:
    /// `cargo test -p nectarlink-desktop -- --ignored records_a_second`.
    #[test]
    #[ignore = "records the microphone"]
    fn records_a_second_of_16_khz_mono() {
        let heard = Arc::new(std::sync::Mutex::new(0usize));
        let counted = heard.clone();
        let recorder = Recorder::start(None, 16_000, move |pcm| *counted.lock().unwrap() += pcm.len())
            .expect("the microphone starts");
        std::thread::sleep(std::time::Duration::from_secs(1));
        drop(recorder);
        let bytes = *heard.lock().unwrap();
        // About a second of 16-bit samples at 16 kHz: 32,000 bytes.
        assert!((24_000..=40_000).contains(&bytes), "{bytes} bytes in a second");
    }

    #[test]
    fn lists_microphones_by_uid_with_at_most_one_default() {
        let devices = super::super::audio::input_devices();
        assert!(devices.iter().all(|d| !d.id.is_empty() && !d.name.is_empty()));
        assert!(devices.iter().filter(|d| d.is_default).count() <= 1);
    }
}
