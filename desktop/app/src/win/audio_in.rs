// SPDX-License-Identifier: GPL-3.0-or-later
//! The PC's microphone as 16-bit mono PCM at a rate the caller picks, from
//! the default communications microphone (WASAPI shared capture, converted
//! by Windows): for a phone call's sound on this PC (`call.audio`).

#![allow(unsafe_code)]

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::JoinHandle,
    time::Duration,
};

use windows::Win32::{
    Media::Audio::{
        AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
        AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator,
        MMDeviceEnumerator, WAVE_FORMAT_PCM, WAVEFORMATEX, eCapture, eCommunications,
    },
    System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize},
};

/// Records until dropped.
pub struct Recorder {
    running: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Recorder {
    /// Starts recording from `device` (an endpoint ID from
    /// [`super::audio::input_devices`]; the default communications microphone
    /// when `None` or gone); `on_pcm` gets each piece of sound as it comes, on
    /// the recorder's thread. An error if there's no microphone to record.
    pub fn start(
        device: Option<String>,
        rate: u32,
        mut on_pcm: impl FnMut(Vec<u8>) + Send + 'static,
    ) -> Result<Recorder, String> {
        let running = Arc::new(AtomicBool::new(true));
        let (ready, started) = mpsc::channel();
        let keep = running.clone();
        let thread = std::thread::Builder::new()
            .name("pc-mic".into())
            .spawn(move || {
                // SAFETY: COM for this thread only, undone below.
                let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
                match open(device.as_deref(), rate) {
                    Ok((client, capture)) => {
                        let _ = ready.send(Ok(()));
                        while keep.load(Ordering::Acquire) {
                            std::thread::sleep(Duration::from_millis(10));
                            drain(&capture, &mut on_pcm);
                        }
                        // SAFETY: the client this thread started.
                        let _ = unsafe { client.Stop() };
                    }
                    Err(e) => {
                        let _ = ready.send(Err(e));
                    }
                }
                if com {
                    // SAFETY: balances the CoInitializeEx above.
                    unsafe { CoUninitialize() };
                }
            })
            .map_err(|e| e.to_string())?;
        match started.recv().unwrap_or_else(|_| Err("the microphone thread ended".into())) {
            Ok(()) => Ok(Recorder { running, thread: Some(thread) }),
            Err(e) => {
                let _ = thread.join();
                Err(e)
            }
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn open(device: Option<&str>, rate: u32) -> Result<(IAudioClient, IAudioCaptureClient), String> {
    let format = WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_PCM as u16,
        nChannels: 1,
        nSamplesPerSec: rate,
        nAvgBytesPerSec: rate * 2,
        nBlockAlign: 2,
        wBitsPerSample: 16,
        cbSize: 0,
    };
    // SAFETY: WASAPI shared capture on the chosen (or default communications)
    // microphone, in a format we own; Windows converts the microphone's own to it.
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        // A chosen microphone that's unplugged or off: the default one.
        let chosen = device
            .and_then(|id| enumerator.GetDevice(&windows::core::HSTRING::from(id)).ok())
            .filter(|d| d.GetState().is_ok_and(|s| s == windows::Win32::Media::Audio::DEVICE_STATE_ACTIVE));
        let device = match chosen {
            Some(device) => device,
            None => {
                enumerator.GetDefaultAudioEndpoint(eCapture, eCommunications).map_err(|e| e.to_string())?
            }
        };
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None).map_err(|e| e.to_string())?;
        // 20 ms of buffer, in 100-ns units.
        client
            .Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                200_000,
                0,
                &format,
                None,
            )
            .map_err(|e| e.to_string())?;
        let capture: IAudioCaptureClient = client.GetService().map_err(|e| e.to_string())?;
        client.Start().map_err(|e| e.to_string())?;
        Ok((client, capture))
    }
}

/// Hands on every packet the microphone has ready.
fn drain(capture: &IAudioCaptureClient, on_pcm: &mut impl FnMut(Vec<u8>)) {
    // SAFETY: each buffer is read before it's released, in the format `open` set (2 bytes a frame).
    unsafe {
        while capture.GetNextPacketSize().is_ok_and(|n| n > 0) {
            let mut data: *mut u8 = std::ptr::null_mut();
            let (mut frames, mut flags) = (0u32, 0u32);
            if capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None).is_err() {
                return;
            }
            let bytes = frames as usize * 2;
            let pcm = if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                vec![0u8; bytes]
            } else {
                std::slice::from_raw_parts(data, bytes).to_vec()
            };
            let _ = capture.ReleaseBuffer(frames);
            if !pcm.is_empty() {
                on_pcm(pcm);
            }
        }
    }
}
