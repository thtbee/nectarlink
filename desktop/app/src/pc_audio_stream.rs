// SPDX-License-Identifier: GPL-3.0-or-later
//! Phone as Speaker / Headphones for PC Audio (§3.8 P2): captures Windows
//! system audio via WASAPI loopback (`AUDCLNT_STREAMFLAGS_LOOPBACK`), converts
//! 32-bit float stereo samples to 16-bit signed little-endian PCM, and streams
//! them to connected phones over NectarLink's low-latency audio stream.

#![allow(unsafe_code)]

use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

#[cfg(windows)]
use windows::Win32::{
    Media::Audio::{
        AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
        IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator, WAVEFORMATEX, eConsole,
        eRender,
    },
    System::{
        Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree},
        Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL},
    },
};

use crate::core_host;

static ENABLED: AtomicBool = AtomicBool::new(false);
static WORKER_RUNNING: AtomicBool = AtomicBool::new(false);
/// The phone that last turned "Phone speaker" on. Audio goes there while it's online;
/// when the PC turned it on (or that phone is away), to the first phone that is.
static TARGET: std::sync::Mutex<Option<nectarlink_core::DeviceId>> = std::sync::Mutex::new(None);

/// Records which phone asked to be the speaker.
pub fn set_target(phone: Option<nectarlink_core::DeviceId>) {
    *TARGET.lock().unwrap_or_else(|e| e.into_inner()) = phone;
}

/// Returns whether PC-to-phone audio streaming is currently enabled.
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::SeqCst)
}

/// Enables or disables PC-to-phone audio loopback streaming.
pub fn set_enabled(enabled: bool) {
    let prev = ENABLED.swap(enabled, Ordering::SeqCst);
    if enabled && !prev && core_host::node().is_some() {
        ensure_worker();
    }
}

/// Converts interleaved stereo `f32` samples (`[-1.0, 1.0]`) into 16-bit
/// signed little-endian PCM bytes (`pcm_s16le`).
#[cfg(test)]
pub fn f32_stereo_to_i16le(samples: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for &sample in samples {
        let clean = if sample.is_finite() { sample.clamp(-1.0, 1.0) } else { 0.0 };
        let scaled = (clean * 32767.0).round() as i16;
        out.extend_from_slice(&scaled.to_le_bytes());
    }
    out
}

#[allow(dead_code)]
#[derive(Clone, Copy)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1_l: f32,
    z2_l: f32,
    z1_r: f32,
    z2_r: f32,
}

#[allow(dead_code)]
impl Biquad {
    fn low_shelf(sample_rate: f32, freq: f32, gain_db: f32) -> Self {
        let a = 10.0_f32.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * (freq / sample_rate.max(8_000.0)).clamp(0.001, 0.45);
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / 2.0 * std::f32::consts::SQRT_2;
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;

        let b0 = a * ((a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha);
        let b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0);
        let b2 = a * ((a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha);
        let a0 = (a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
        let a1 = -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0);
        let a2 = (a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            z1_l: 0.0,
            z2_l: 0.0,
            z1_r: 0.0,
            z2_r: 0.0,
        }
    }

    fn high_shelf(sample_rate: f32, freq: f32, gain_db: f32) -> Self {
        let a = 10.0_f32.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * (freq / sample_rate.max(8_000.0)).clamp(0.001, 0.45);
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / 2.0 * std::f32::consts::SQRT_2;
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;

        let b0 = a * ((a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha);
        let b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0);
        let b2 = a * ((a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha);
        let a0 = (a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
        let a1 = 2.0 * ((a - 1.0) - (a + 1.0) * cos_w0);
        let a2 = (a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            z1_l: 0.0,
            z2_l: 0.0,
            z1_r: 0.0,
            z2_r: 0.0,
        }
    }

    #[inline]
    fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        let out_l = self.b0 * l + self.z1_l;
        self.z1_l = self.b1 * l - self.a1 * out_l + self.z2_l;
        self.z2_l = self.b2 * l - self.a2 * out_l;

        let out_r = self.b0 * r + self.z1_r;
        self.z1_r = self.b1 * r - self.a1 * out_r + self.z2_r;
        self.z2_r = self.b2 * r - self.a2 * out_r;
        (out_l, out_r)
    }
}

#[allow(dead_code)]
struct SpeakerEnhancer {
    low_shelf: Biquad,
    high_shelf: Biquad,
}

#[allow(dead_code)]
impl SpeakerEnhancer {
    fn new(sample_rate: u32) -> Self {
        let sr = sample_rate as f32;
        Self { low_shelf: Biquad::low_shelf(sr, 155.0, 3.5), high_shelf: Biquad::high_shelf(sr, 3800.0, 1.5) }
    }

    #[inline]
    fn soft_limit(x: f32) -> f32 {
        if !x.is_finite() {
            return 0.0;
        }
        let boosted = x * 1.12;
        (boosted / (1.0 + 0.18 * boosted.abs())).clamp(-1.0, 1.0)
    }

    fn encode_stereo_f32(&mut self, floats: &[f32], channels: usize) -> Vec<u8> {
        let frames = floats.len() / channels.max(1);
        let mut out = Vec::with_capacity(frames * 4);
        for frame in floats.chunks_exact(channels.max(1)) {
            let l_in = if frame[0].is_finite() { frame[0] } else { 0.0 };
            let r_in = if channels > 1 && frame[1].is_finite() { frame[1] } else { l_in };
            let (l1, r1) = self.low_shelf.process_stereo(l_in, r_in);
            let (l2, r2) = self.high_shelf.process_stereo(l1, r1);
            let l_i16 = (Self::soft_limit(l2) * 32767.0).round() as i16;
            let r_i16 = (Self::soft_limit(r2) * 32767.0).round() as i16;
            out.extend_from_slice(&l_i16.to_le_bytes());
            out.extend_from_slice(&r_i16.to_le_bytes());
        }
        out
    }

    fn encode_stereo_i16(&mut self, samples: &[i16], channels: usize) -> Vec<u8> {
        let frames = samples.len() / channels.max(1);
        let mut out = Vec::with_capacity(frames * 4);
        for frame in samples.chunks_exact(channels.max(1)) {
            let l_in = (frame[0] as f32) / 32768.0;
            let r_in = if channels > 1 { (frame[1] as f32) / 32768.0 } else { l_in };
            let (l1, r1) = self.low_shelf.process_stereo(l_in, r_in);
            let (l2, r2) = self.high_shelf.process_stereo(l1, r1);
            let l_i16 = (Self::soft_limit(l2) * 32767.0).round() as i16;
            let r_i16 = (Self::soft_limit(r2) * 32767.0).round() as i16;
            out.extend_from_slice(&l_i16.to_le_bytes());
            out.extend_from_slice(&r_i16.to_le_bytes());
        }
        out
    }
}

fn ensure_worker() {
    if WORKER_RUNNING.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return;
    }
    std::thread::Builder::new()
        .name("nectarlink-pc-audio-loopback".into())
        .spawn(|| {
            run_loopback_worker();
            WORKER_RUNNING.store(false, Ordering::SeqCst);
        })
        .ok();
}

#[cfg(target_os = "linux")]
fn run_loopback_worker() {
    while is_enabled() {
        std::thread::sleep(Duration::from_millis(500));
    }
}

#[cfg(windows)]
fn run_loopback_worker() {
    // SAFETY: Initialize COM and elevate thread priority for low-latency audio capture.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL);
    }

    while is_enabled() {
        if let Err(e) = capture_and_stream_once() {
            tracing::debug!(error = %e, "PC audio loopback stream cycle ended");
            std::thread::sleep(Duration::from_millis(500));
        }
    }
}

#[cfg(windows)]
fn capture_and_stream_once() -> Result<(), String> {
    let node = core_host::node().ok_or_else(|| "core node not running".to_owned())?;
    let peers: Vec<_> = node
        .paired_devices()
        .unwrap_or_default()
        .into_iter()
        .filter(|d| matches!(d.link, nectarlink_core::LinkState::Online { .. }))
        .map(|d| d.id)
        .collect();
    let wanted = *TARGET.lock().unwrap_or_else(|e| e.into_inner());
    let Some(&peer) = wanted.as_ref().filter(|w| peers.contains(w)).or_else(|| peers.first()) else {
        std::thread::sleep(Duration::from_millis(500));
        return Ok(());
    };

    // SAFETY: WASAPI loopback initialization on the default render endpoint.
    let (client, capture, rate, channels, bits_per_sample) = unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole).map_err(|e| e.to_string())?;
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None).map_err(|e| e.to_string())?;
        let mix_ptr: *mut WAVEFORMATEX = client.GetMixFormat().map_err(|e| e.to_string())?;
        if mix_ptr.is_null() {
            return Err("GetMixFormat returned null".into());
        }
        let mix = *mix_ptr;
        let rate = mix.nSamplesPerSec.clamp(8_000, 96_000);
        let channels = mix.nChannels.max(1) as usize;
        let bits = mix.wBitsPerSample;
        // 10 ms shared loopback buffer in 100-ns units = 100_000.
        let init_res = client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK,
            100_000,
            0,
            mix_ptr,
            None,
        );
        CoTaskMemFree(Some(mix_ptr.cast()));
        init_res.map_err(|e| e.to_string())?;
        let capture: IAudioCaptureClient = client.GetService().map_err(|e| e.to_string())?;
        client.Start().map_err(|e| e.to_string())?;
        (client, capture, rate, channels, bits)
    };

    let mut enhancer = SpeakerEnhancer::new(rate);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<(u32, u8, Vec<u8>)>(64);
    let send_node = node.clone();
    core_host::spawn(async move {
        while let Some((r, ch, pcm)) = rx.recv().await {
            // Slice into 960-byte (5 ms at 48 kHz stereo) sub-MTU chunks so each
            // chunk travels in a single QUIC datagram without HOL blocking.
            for chunk in pcm.chunks(960) {
                if send_node.send_pc_audio_pcm(&peer, r, ch, chunk.to_vec()).await.is_err() {
                    return;
                }
            }
        }
    });

    // Keep sending zero-filled PCM for ~500 ms after audible audio stops so brief
    // pauses between words or beats never starve the phone's AudioTrack ringbuffer.
    let mut silence_tail_packets: u32 = 0;

    'capture: while is_enabled() {
        std::thread::sleep(Duration::from_millis(2));
        // SAFETY: Drain available capture packets from `IAudioCaptureClient`.
        unsafe {
            loop {
                let Ok(packet_frames) = capture.GetNextPacketSize() else { break };
                if packet_frames == 0 {
                    break;
                }
                let mut data_ptr: *mut u8 = std::ptr::null_mut();
                let mut frames_to_read: u32 = 0;
                let mut flags: u32 = 0;
                if capture.GetBuffer(&mut data_ptr, &mut frames_to_read, &mut flags, None, None).is_err() {
                    break;
                }
                let silent = (flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)) != 0 || data_ptr.is_null();
                let frames = frames_to_read as usize;
                let pcm_bytes = if silent {
                    if silence_tail_packets > 0 {
                        silence_tail_packets -= 1;
                        vec![0u8; frames * 4]
                    } else {
                        Vec::new()
                    }
                } else if bits_per_sample == 32 {
                    let floats = std::slice::from_raw_parts(data_ptr.cast::<f32>(), frames * channels);
                    let has_signal = floats.iter().any(|&s| s.is_finite() && s.abs() > 1e-5);
                    if has_signal {
                        silence_tail_packets = 50;
                        enhancer.encode_stereo_f32(floats, channels)
                    } else if silence_tail_packets > 0 {
                        silence_tail_packets -= 1;
                        vec![0u8; frames * 4]
                    } else {
                        Vec::new()
                    }
                } else if bits_per_sample == 16 {
                    let i16s = std::slice::from_raw_parts(data_ptr.cast::<i16>(), frames * channels);
                    let has_signal = i16s.iter().any(|&s| s.unsigned_abs() > 2);
                    if has_signal {
                        silence_tail_packets = 50;
                        enhancer.encode_stereo_i16(i16s, channels)
                    } else if silence_tail_packets > 0 {
                        silence_tail_packets -= 1;
                        vec![0u8; frames * 4]
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                };
                let _ = capture.ReleaseBuffer(frames_to_read);

                if !pcm_bytes.is_empty()
                    && matches!(
                        tx.try_send((rate, 2, pcm_bytes)),
                        Err(tokio::sync::mpsc::error::TrySendError::Closed(_))
                    )
                {
                    break 'capture;
                }
            }
        }
    }

    // SAFETY: Stop the WASAPI client before dropping.
    unsafe {
        let _ = client.Stop();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f32_stereo_to_i16le_clamps_and_encodes_little_endian() {
        let samples = [0.0_f32, 1.0, -1.0, 2.5, -3.0, f32::NAN];
        let bytes = f32_stereo_to_i16le(&samples);
        assert_eq!(bytes.len(), samples.len() * 2);
        let decoded: Vec<i16> = bytes.as_chunks::<2>().0.iter().map(|&c| i16::from_le_bytes(c)).collect();
        assert_eq!(decoded, vec![0, 32767, -32767, 32767, -32767, 0]);
    }

    #[test]
    fn set_enabled_toggles_atomic_state_cleanly() {
        set_enabled(false);
        assert!(!is_enabled());
        set_enabled(true);
        assert!(is_enabled());
        set_enabled(false);
        assert!(!is_enabled());
    }
}
