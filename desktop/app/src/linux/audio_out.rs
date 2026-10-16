use std::collections::VecDeque;
use std::io::Result;
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const BYTES_PER_SAMPLE: usize = 2;
const JITTER_START_MS: usize = 40;
const JITTER_LOW_MS: usize = 20;
const JITTER_TARGET_MS: usize = 80;
const JITTER_HIGH_MS: usize = 160;
const JITTER_MAX_MS: usize = 220;

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

    fn pull(&mut self, dst: &mut [u8], frames: usize) {
        let total_bytes = (frames * self.frame_bytes).min(dst.len());
        if !self.primed {
            dst[..total_bytes].fill(0);
            return;
        }

        if self.pcm.len() > self.high_bytes && frames >= 8 {
            for _ in 0..self.frame_bytes {
                let _ = self.pcm.pop_front();
            }
        }

        let available_frames = self.pcm.len() / self.frame_bytes;
        if available_frames == 0 {
            self.primed = false;
            for byte in &mut self.last_frame {
                *byte = 0;
            }
            dst[..total_bytes].fill(0);
            return;
        }

        let stretch = self.pcm.len() < self.low_bytes && frames >= 16;
        let mut written = 0usize;
        let mut produced_frames = 0usize;
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
                    dst[written..written + self.frame_bytes].copy_from_slice(&self.last_frame);
                    written += self.frame_bytes;
                    produced_frames += 1;
                }
            } else {
                self.primed = false;
                dst[written..total_bytes].fill(0);
                break;
            }
        }
    }
}

#[derive(Debug)]
pub struct Player {
    tx: SyncSender<Vec<u8>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Player {
    pub fn start(rate: u32, channels: u16) -> Result<Player> {
        let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(32);
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);

        let handle = thread::spawn(move || {
            let mut jitter = Jitter::new(rate.max(8_000), channels.max(1));
            let frame_bytes = jitter.frame_bytes;
            let frames_per_tick = ((rate.max(8_000) as usize) / 100).max(1);
            let mut scratch = vec![0u8; frames_per_tick * frame_bytes];
            while !thread_stop.load(Ordering::Acquire) {
                while let Ok(chunk) = rx.try_recv() {
                    jitter.push(&chunk);
                }
                jitter.pull(&mut scratch, frames_per_tick);
                thread::sleep(Duration::from_millis(10));
            }
        });

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
        jitter.pull(&mut out, 480);
        assert!(out.iter().all(|&b| b == 0));

        jitter.push(&vec![0x22u8; 480 * 4 * 4]);
        jitter.pull(&mut out, 480);
        assert!(out.iter().any(|&b| b != 0));
    }

    #[test]
    fn jitter_caps_overflow_to_target_depth() {
        let mut jitter = Jitter::new(48_000, 2);
        jitter.push(&vec![0x33u8; jitter.max_bytes + 4096]);
        assert!(jitter.pcm.len() <= jitter.max_bytes);
        assert!(jitter.pcm.len() >= jitter.target_bytes);
    }
}
