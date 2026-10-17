// SPDX-License-Identifier: GPL-3.0-or-later
//! Find-My-PC audible alarm on Linux: synthesizes a two-tone 880 Hz / 1320 Hz
//! RIFF/WAVE alarm and loops playback via `paplay`, `pw-play`, `aplay`, or
//! `canberra-gtk-play` until stopped or 60 seconds elapse.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

static RINGING: AtomicBool = AtomicBool::new(false);
static ACTIVE_CHILD: Mutex<Option<Child>> = Mutex::new(None);

const MAX_RING_DURATION: Duration = Duration::from_secs(60);
const RING_PAUSE: Duration = Duration::from_millis(250);

/// Synthesizes a 16-bit mono PCM RIFF/WAVE file alternating between 880 Hz and
/// 1320 Hz every 250 ms for `duration_ms` at `sample_rate`.
pub fn generate_alarm_wav(duration_ms: u32, sample_rate: u32) -> Vec<u8> {
    let sr = sample_rate.max(8_000);
    let total_samples = ((u64::from(duration_ms) * u64::from(sr)) / 1000) as u32;
    let data_bytes = total_samples.saturating_mul(2);
    let riff_size = 36u32.saturating_add(data_bytes);

    let mut out = Vec::with_capacity(44 + data_bytes as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_size.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // Mono
    out.extend_from_slice(&sr.to_le_bytes());
    out.extend_from_slice(&(sr * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_bytes.to_le_bytes());

    let segment_samples = (sr / 4).max(1);
    let fade_samples = (sr / 200).max(1); // 5 ms envelope
    let mut phase = 0.0_f32;

    for idx in 0..total_samples {
        let seg = idx / segment_samples;
        let pos_in_seg = idx % segment_samples;
        let freq = if seg.is_multiple_of(2) { 880.0_f32 } else { 1320.0_f32 };
        let env = if pos_in_seg < fade_samples {
            (pos_in_seg as f32) / (fade_samples as f32)
        } else if segment_samples - pos_in_seg <= fade_samples {
            ((segment_samples - pos_in_seg) as f32) / (fade_samples as f32)
        } else {
            1.0
        };

        let sample_f = (phase * 2.0 * std::f32::consts::PI).sin() * 0.65 * env;
        phase = (phase + freq / (sr as f32)).fract();
        let sample_i16 = (sample_f.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        out.extend_from_slice(&sample_i16.to_le_bytes());
    }

    out
}

fn spawn_alarm_player(wav_path: &Path) -> Option<Child> {
    let path_str = wav_path.to_string_lossy();
    Command::new("paplay")
        .arg(path_str.as_ref())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .or_else(|_| {
            Command::new("pw-play")
                .arg(path_str.as_ref())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
        })
        .or_else(|_| {
            Command::new("aplay")
                .args(["-q", path_str.as_ref()])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
        })
        .or_else(|_| {
            Command::new("canberra-gtk-play")
                .args(["-i", "phone-incoming-call"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
        })
        .ok()
}

fn kill_active_child() {
    if let Ok(mut guard) = ACTIVE_CHILD.lock()
        && let Some(mut child) = guard.take()
    {
        let _ = child.kill();
        let _ = child.wait();
    }
}

pub fn start_ringing() {
    if RINGING.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = thread::Builder::new().name("nectarlink-alarm".into()).spawn(|| {
        let wav_path = std::env::temp_dir().join("nectarlink-find-pc-alarm.wav");
        let wav_bytes = generate_alarm_wav(1000, 48_000);
        let _ = std::fs::write(&wav_path, wav_bytes);

        let started = Instant::now();
        while RINGING.load(Ordering::SeqCst) && started.elapsed() < MAX_RING_DURATION {
            if let Some(child) = spawn_alarm_player(&wav_path) {
                if let Ok(mut guard) = ACTIVE_CHILD.lock() {
                    *guard = Some(child);
                }
                loop {
                    if !RINGING.load(Ordering::SeqCst) || started.elapsed() >= MAX_RING_DURATION {
                        kill_active_child();
                        break;
                    }
                    let finished = if let Ok(mut guard) = ACTIVE_CHILD.lock() {
                        if let Some(c) = guard.as_mut() {
                            matches!(c.try_wait(), Ok(Some(_)) | Err(_))
                        } else {
                            true
                        }
                    } else {
                        true
                    };
                    if finished {
                        break;
                    }
                    thread::sleep(Duration::from_millis(40));
                }
            }

            let step = Duration::from_millis(50);
            let mut waited = Duration::ZERO;
            while waited < RING_PAUSE && RINGING.load(Ordering::SeqCst) {
                thread::sleep(step);
                waited += step;
            }
        }
        kill_active_child();
        RINGING.store(false, Ordering::SeqCst);
    });
}

pub fn stop_ringing() {
    RINGING.store(false, Ordering::SeqCst);
    kill_active_child();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_alarm_wav_produces_valid_riff_header_and_waveform() {
        let wav = generate_alarm_wav(500, 48_000);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(&wav[36..40], b"data");

        let expected_samples = 24_000usize;
        let expected_data_bytes = expected_samples * 2;
        assert_eq!(wav.len(), 44 + expected_data_bytes);

        let data_len = u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]) as usize;
        assert_eq!(data_len, expected_data_bytes);

        let non_zero = wav[44..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .any(|s| s.unsigned_abs() > 10_000);
        assert!(non_zero, "synthesized alarm waveform should be audible");
    }

    #[test]
    fn start_and_stop_ringing_terminates_cleanly() {
        start_ringing();
        stop_ringing();
        assert!(!RINGING.load(Ordering::SeqCst));
    }
}
