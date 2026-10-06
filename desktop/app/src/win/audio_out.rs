// SPDX-License-Identifier: GPL-3.0-or-later
//! Playing a mirrored phone's sound on the PC's speakers: 16-bit PCM from
//! the phone goes through a small jitter buffer to the default output
//! device (WASAPI, shared mode), on a thread of its own.
//!
//! The buffer favors low delay: playback starts once a few packets are in,
//! falls back to silence (and waits again) when the network stalls, and
//! drops the oldest sound when it piles up, so the sound stays in step
//! with the picture.

use std::{
    collections::VecDeque,
    sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel},
    time::Duration,
};

use windows::Win32::{
    Foundation::{CloseHandle, HANDLE},
    Media::Audio::{
        AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
        AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, IAudioClient, IAudioRenderClient, IMMDeviceEnumerator,
        MMDeviceEnumerator, WAVE_FORMAT_PCM, WAVEFORMATEX, eConsole, eRender,
    },
    System::{
        Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize},
        Threading::{CreateEventW, WaitForSingleObject},
    },
};

/// Sound packets waiting for the player thread, at most.
const QUEUE: usize = 64;
/// How much sound is gathered before playing (and again after a stall).
const START_MS: u32 = 40;
/// More than this waiting, and the oldest is dropped down to [`TRIM_TO_MS`].
const MOST_MS: u32 = 200;
const TRIM_TO_MS: u32 = 60;
/// The device's buffer.
const DEVICE_BUFFER: Duration = Duration::from_millis(40);

/// Plays sound until dropped.
#[derive(Debug)]
pub struct Player {
    queue: SyncSender<Vec<u8>>,
}

impl Player {
    /// Starts playing `channels` × 16-bit PCM at `rate` on the default
    /// output device.
    pub fn start(rate: u32, channels: u16) -> std::io::Result<Player> {
        let (queue, packets) = sync_channel(QUEUE);
        std::thread::Builder::new().name("mirror-sound".into()).spawn(move || {
            let mut attempts = 0;
            // The device can go away (headphones unplugged): start over on
            // the new default, a few times.
            while attempts < 5 {
                match play(rate, channels, &packets) {
                    Ok(()) => return,
                    Err(e) => {
                        tracing::warn!(error = %e, "the sound stopped; trying the default device again");
                        attempts += 1;
                        std::thread::sleep(Duration::from_millis(300));
                    }
                }
            }
        })?;
        Ok(Player { queue })
    }

    /// Hands over a packet without waiting (dropped if the player is far
    /// behind: it would be too late to hear anyway).
    pub fn push(&self, data: Vec<u8>) {
        if let Err(TrySendError::Full(_)) = self.queue.try_send(data) {
            tracing::debug!("a sound packet was dropped");
        }
    }
}

/// The jitter buffer, in bytes of interleaved 16-bit samples.
#[derive(Debug)]
struct Jitter {
    bytes: VecDeque<u8>,
    /// Bytes in one sample frame.
    frame: usize,
    /// Bytes in one millisecond.
    per_ms: usize,
    /// Gathering sound before playing.
    waiting: bool,
    /// Times it ran dry, and bytes dropped for piling up (for the log).
    stalls: u32,
    trimmed: usize,
}

impl Jitter {
    fn new(rate: u32, channels: u16) -> Jitter {
        let frame = 2 * usize::from(channels);
        Jitter {
            bytes: VecDeque::new(),
            frame,
            per_ms: frame * rate as usize / 1000,
            waiting: true,
            stalls: 0,
            trimmed: 0,
        }
    }

    fn push(&mut self, data: &[u8]) {
        self.bytes.extend(data);
        if self.bytes.len() > self.per_ms * MOST_MS as usize {
            let keep = self.per_ms * TRIM_TO_MS as usize;
            // Whole frames only, or the channels swap.
            let drop = (self.bytes.len() - keep) / self.frame * self.frame;
            self.bytes.drain(..drop);
            self.trimmed += drop;
        }
    }

    /// Fills `out` (whole frames): sound when there is some, silence when
    /// gathering or stalled.
    fn fill(&mut self, out: &mut [u8]) {
        if self.waiting && self.bytes.len() >= self.per_ms * START_MS as usize {
            self.waiting = false;
        }
        let mut n = 0;
        if !self.waiting {
            n = out.len().min(self.bytes.len()) / self.frame * self.frame;
            for (o, b) in out[..n].iter_mut().zip(self.bytes.drain(..n)) {
                *o = b;
            }
            if n < out.len() {
                // Ran dry: gather again before playing on.
                self.waiting = true;
                self.stalls += 1;
            }
        }
        out[n..].fill(0);
    }
}

/// Plays until the sender is gone (`Ok`) or the device fails (`Err`).
fn play(rate: u32, channels: u16, packets: &Receiver<Vec<u8>>) -> windows::core::Result<()> {
    // SAFETY: balanced below; this thread is ours and uses only the MTA.
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok()?;
    let event = Event::new()?;
    let result = render(rate, channels, packets, event.0);
    drop(event);
    // SAFETY: pairs with CoInitializeEx above; every COM object is gone.
    unsafe { CoUninitialize() };
    result
}

fn render(rate: u32, channels: u16, packets: &Receiver<Vec<u8>>, event: HANDLE) -> windows::core::Result<()> {
    let block = 2 * channels;
    let format = WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_PCM as u16,
        nChannels: channels,
        nSamplesPerSec: rate,
        nAvgBytesPerSec: rate * u32::from(block),
        nBlockAlign: block,
        wBitsPerSample: 16,
        cbSize: 0,
    };
    // SAFETY: COM calls on live objects, in this thread's apartment; the
    // render buffer is written only within its frame count, between
    // GetBuffer and ReleaseBuffer.
    unsafe {
        let devices: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = devices.GetDefaultAudioEndpoint(eRender, eConsole)?;
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
        // Windows converts the rate and channels to the device's own.
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            (DEVICE_BUFFER.as_nanos() / 100) as i64,
            0,
            &format,
            None,
        )?;
        client.SetEventHandle(event)?;
        let size = client.GetBufferSize()?;
        let output: IAudioRenderClient = client.GetService()?;
        let mut jitter = Jitter::new(rate, channels);
        client.Start()?;
        tracing::debug!(rate, channels, frames = size, "playing a phone's sound");
        let result = loop {
            // Woken when the device wants sound (or after a while regardless,
            // to notice the end).
            let _ = WaitForSingleObject(event, 200);
            // Everything that came in.
            let mut gone = false;
            loop {
                match packets.recv_timeout(Duration::ZERO) {
                    Ok(data) => jitter.push(&data),
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => {
                        gone = true;
                        break;
                    }
                }
            }
            if gone {
                break Ok(());
            }
            let free = match client.GetCurrentPadding() {
                Ok(padding) => size - padding,
                Err(e) => break Err(e),
            };
            if free == 0 {
                continue;
            }
            let buffer = match output.GetBuffer(free) {
                Ok(buffer) => buffer,
                Err(e) => break Err(e),
            };
            let out = std::slice::from_raw_parts_mut(buffer, free as usize * usize::from(block));
            jitter.fill(out);
            if let Err(e) = output.ReleaseBuffer(free, 0) {
                break Err(e);
            }
        };
        let _ = client.Stop();
        tracing::debug!(
            stalls = jitter.stalls,
            dropped_ms = jitter.trimmed / jitter.per_ms.max(1),
            "the phone's sound stopped"
        );
        result
    }
}

/// An auto-reset event, closed when dropped.
struct Event(HANDLE);

impl Event {
    fn new() -> windows::core::Result<Event> {
        // SAFETY: creates an unnamed event with default security.
        unsafe { CreateEventW(None, false, false, None) }.map(Event)
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateEventW and is closed once.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 48 kHz stereo: 192 bytes a millisecond.
    const MS: usize = 192;

    #[test]
    fn plays_once_enough_has_come() {
        let mut jitter = Jitter::new(48_000, 2);
        let mut out = vec![9u8; 10 * MS];
        jitter.push(&vec![1; 30 * MS]);
        jitter.fill(&mut out);
        assert!(out.iter().all(|&b| b == 0), "silence while gathering");
        jitter.push(&vec![1; 10 * MS]);
        jitter.fill(&mut out);
        assert!(out.iter().all(|&b| b == 1), "then the sound");
    }

    #[test]
    fn a_stall_is_silence_then_gathers_again() {
        let mut jitter = Jitter::new(48_000, 2);
        jitter.push(&vec![1; 45 * MS]);
        let mut out = vec![9u8; 50 * MS];
        jitter.fill(&mut out);
        assert!(out[..45 * MS].iter().all(|&b| b == 1));
        assert!(out[45 * MS..].iter().all(|&b| b == 0), "silence when dry");
        jitter.push(&vec![2; 10 * MS]);
        let mut out = vec![9u8; 5 * MS];
        jitter.fill(&mut out);
        assert!(out.iter().all(|&b| b == 0), "gathers before playing on");
    }

    #[test]
    fn a_pile_up_drops_the_oldest_whole_frames() {
        let mut jitter = Jitter::new(48_000, 2);
        jitter.push(&vec![1; 150 * MS]);
        jitter.push(&vec![2; 60 * MS]);
        assert_eq!(jitter.bytes.len(), TRIM_TO_MS as usize * MS);
        assert_eq!(jitter.bytes.len() % 4, 0, "whole frames");
        assert!(jitter.bytes.iter().all(|&b| b == 2), "the newest stays");
    }
}
