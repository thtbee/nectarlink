// SPDX-License-Identifier: GPL-3.0-or-later
//! Low-latency 16-bit signed little-endian PCM playback on Linux via
//! PulseAudio (`pacat`), PipeWire (`pw-play`), or ALSA (`aplay`), backed by an
//! adaptive jitter buffer with smooth sample interpolation.

use std::collections::VecDeque;
use std::io::{Result, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const BYTES_PER_SAMPLE: usize = 2;
const JITTER_START_MS: usize = 40;
const JITTER_LOW_MS: usize = 20;
const JITTER_TARGET_MS: usize = 80;
const JITTER_HIGH_MS: usize = 160;
const JITTER_MAX_MS: usize = 220;

/// Linearly interpolates two interleaved `s16le` frames (`a` and `b`) into `out`.
pub(crate) fn interpolate_frame_s16le(a: &[u8], b: &[u8], out: &mut [u8]) {
    let len = a.len().min(b.len()).min(out.len()) & !1;
    for i in (0..len).step_by(2) {
        let sa = i16::from_le_bytes([a[i], a[i + 1]]);
        let sb = i16::from_le_bytes([b[i], b[i + 1]]);
        let mid = ((i32::from(sa) + i32::from(sb)) / 2) as i16;
        let bytes = mid.to_le_bytes();
        out[i] = bytes[0];
        out[i + 1] = bytes[1];
    }
}

#[derive(Debug)]
struct Jitter {
    pcm: VecDeque<u8>,
    frame_bytes: usize,
    start_bytes: usize,
    low_bytes: usize,
    target_bytes: usize,
    high_bytes: usize,
    max_bytes: usize,
    primed: bool,
    last_frame: Vec<u8>,
}

impl Jitter {
    fn new(rate: u32, channels: u16) -> Self {
        let frame_bytes = (channels as usize).max(1) * BYTES_PER_SAMPLE;
        let bytes_per_ms = ((rate as usize) * frame_bytes / 1000).max(frame_bytes);
        let align = |ms: usize| {
            let raw = (ms * bytes_per_ms).max(frame_bytes);
            (raw / frame_bytes) * frame_bytes
        };
        Self {
            pcm: VecDeque::with_capacity(align(JITTER_MAX_MS)),
            frame_bytes,
            start_bytes: align(JITTER_START_MS),
            low_bytes: align(JITTER_LOW_MS),
            target_bytes: align(JITTER_TARGET_MS),
            high_bytes: align(JITTER_HIGH_MS),
            max_bytes: align(JITTER_MAX_MS),
            primed: false,
            last_frame: vec![0u8; frame_bytes],
        }
    }

    fn push(&mut self, data: &[u8]) {
        let usable = (data.len() / self.frame_bytes) * self.frame_bytes;
        if usable == 0 {
            return;
        }
        self.pcm.extend(&data[..usable]);
        if self.pcm.len() > self.max_bytes {
            let drop =
                ((self.pcm.len() - self.target_bytes) / self.frame_bytes) * self.frame_bytes;
            self.pcm.drain(..drop.min(self.pcm.len()));
        }
        if self.pcm.len() >= self.start_bytes {
            self.primed = true;
        }
    }

    fn pull(&mut self, dst: &mut [u8], frames: usize) -> bool {
        let total_bytes = (frames * self.frame_bytes).min(dst.len());
        if !self.primed {
            dst[..total_bytes].fill(0);
            return false;
        }

        if self.pcm.len() > self.high_bytes && frames >= 8 {
            for _ in 0..self.frame_bytes {
                let _ = self.pcm.pop_front();
            }
        }

        let available_frames = self.pcm.len() / self.frame_bytes;
        if available_frames == 0 {
            self.primed = false;
            self.last_frame.fill(0);
            dst[..total_bytes].fill(0);
            return false;
        }

        let stretch = self.pcm.len() < self.low_bytes && frames >= 16;
        let mut written = 0usize;
        let mut produced_frames = 0usize;
        let mut next_peek = vec![0u8; self.frame_bytes];

        while produced_frames < frames && written + self.frame_bytes <= total_bytes {
            if self.pcm.len() >= self.frame_bytes {
                for b in &mut self.last_frame {
                    *b = self.pcm.pop_front().unwrap_or(0);
                }
                dst[written..written + self.frame_bytes].copy_from_slice(&self.last_frame);
                written += self.frame_bytes;
                produced_frames += 1;

                if stretch
                    && produced_frames == frames / 2
                    && written + self.frame_bytes <= total_bytes
                {
                    if self.pcm.len() >= self.frame_bytes {
                        for (i, b) in next_peek.iter_mut().enumerate() {
                            *b = self.pcm.get(i).copied().unwrap_or(0);
                        }
                        interpolate_frame_s16le(
                            &self.last_frame,
                            &next_peek,
                            &mut dst[written..written + self.frame_bytes],
                        );
                    } else {
                        dst[written..written + self.frame_bytes].copy_from_slice(&self.last_frame);
                    }
                    written += self.frame_bytes;
                    produced_frames += 1;
                }
            } else {
                self.primed = false;
                dst[written..total_bytes].fill(0);
                break;
            }
        }
        true
    }
}

fn spawn_pcm_sink(rate: u32, channels: u16, device_id: Option<&str>) -> Option<Child> {
    let rate_s = rate.to_string();
    let ch_s = channels.to_string();
    let dev = device_id
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "default");

    let mut pacat = Command::new("pacat");
    pacat.args([
        "--playback",
        "--raw",
        "--format=s16le",
        &format!("--rate={rate_s}"),
        &format!("--channels={ch_s}"),
        "--latency-msec=20",
    ]);
    if let Some(d) = dev {
        pacat.arg(format!("--device={d}"));
    }
    if let Ok(child) = pacat
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        return Some(child);
    }

    let mut pw = Command::new("pw-play");
    pw.args([
        "--raw",
        "--format=s16",
        &format!("--rate={rate_s}"),
        &format!("--channels={ch_s}"),
    ]);
    if let Some(d) = dev {
        pw.arg(format!("--target={d}"));
    }
    pw.arg("-");
    if let Ok(child) = pw
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        return Some(child);
    }

    Command::new("aplay")
        .args([
            "-q",
            "-t",
            "raw",
            "-f",
            "S16_LE",
            "-r",
            &rate_s,
            "-c",
            &ch_s,
            "--buffer-size=1920",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()
}

#[derive(Debug)]
pub struct Player {
    tx: SyncSender<Vec<u8>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Player {
    pub fn start(rate: u32, channels: u16) -> Result<Player> {
        Self::start_for_device(rate, channels, None)
    }

    pub fn start_for_device(
        rate: u32,
        channels: u16,
        device_id: Option<&str>,
    ) -> Result<Player> {
        let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(64);
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let clean_rate = rate.max(8_000);
        let clean_channels = channels.max(1);
        let dev_owned = device_id.map(str::to_owned);

        let handle = thread::Builder::new()
            .name("nectarlink-audio-out".into())
            .spawn(move || {
                let mut jitter = Jitter::new(clean_rate, clean_channels);
                let frame_bytes = jitter.frame_bytes;
                let frames_per_tick = ((clean_rate as usize) / 100).max(1);
                let mut scratch = vec![0u8; frames_per_tick * frame_bytes];
                let mut child = spawn_pcm_sink(clean_rate, clean_channels, dev_owned.as_deref());

                while !thread_stop.load(Ordering::Acquire) {
                    while let Ok(chunk) = rx.try_recv() {
                        jitter.push(&chunk);
                    }
                    let has_audio = jitter.pull(&mut scratch, frames_per_tick);
                    if has_audio
                        && let Some(proc) = child.as_mut()
                        && let Some(stdin) = proc.stdin.as_mut()
                        && stdin.write_all(&scratch).is_err()
                    {
                        let _ = proc.kill();
                        let _ = proc.wait();
                        child = spawn_pcm_sink(clean_rate, clean_channels, dev_owned.as_deref());
                    }
                    thread::sleep(Duration::from_millis(10));
                }

                if let Some(mut proc) = child {
                    drop(proc.stdin.take());
                    let _ = proc.kill();
                    let _ = proc.wait();
                }
            })?;

        Ok(Player {
            tx,
            stop,
            handle: Some(handle),
        })
    }

    pub fn push(&self, data: Vec<u8>) {
        if data.is_empty() {
            return;
        }
        match self.tx.try_send(data) {
            Ok(()) | Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {}
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jitter_waits_for_prime_threshold_before_emitting_audio() {
        let mut jitter = Jitter::new(48_000, 2);
        let mut out = vec![0xFFu8; 480 * 4];
        jitter.push(&vec![0x11u8; 480 * 4]);
        assert!(!jitter.pull(&mut out, 480));
        assert!(out.iter().all(|&b| b == 0));

        jitter.push(&vec![0x22u8; 480 * 4 * 4]);
        assert!(jitter.pull(&mut out, 480));
        assert!(out.iter().any(|&b| b != 0));
    }

    #[test]
    fn jitter_caps_overflow_to_target_depth() {
        let mut jitter = Jitter::new(48_000, 2);
        jitter.push(&vec![0x33u8; jitter.max_bytes + 4096]);
        assert!(jitter.pcm.len() <= jitter.max_bytes);
        assert!(jitter.pcm.len() >= jitter.target_bytes);
    }

    #[test]
    fn interpolate_frame_s16le_averages_per_channel_samples() {
        let mut a = Vec::new();
        a.extend_from_slice(&1000i16.to_le_bytes());
        a.extend_from_slice(&(-2000i16).to_le_bytes());

        let mut b = Vec::new();
        b.extend_from_slice(&3000i16.to_le_bytes());
        b.extend_from_slice(&(-1000i16).to_le_bytes());

        let mut out = [0u8; 4];
        interpolate_frame_s16le(&a, &b, &mut out);
        assert_eq!(i16::from_le_bytes([out[0], out[1]]), 2000);
        assert_eq!(i16::from_le_bytes([out[2], out[3]]), -1500);
    }

    #[test]
    fn player_starts_accepts_pushes_and_drops_cleanly() {
        let player = Player::start(48_000, 2).expect("player should start");
        player.push(vec![0u8; 1920]);
        drop(player);
    }
}
