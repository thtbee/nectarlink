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
        AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, DEVICE_STATE_ACTIVE, IAudioCaptureClient, IAudioClient,
        IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, WAVE_FORMAT_PCM, WAVEFORMATEX, eCapture,
        eCommunications, eConsole,
    },
    System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize},
};

/// Why the microphone can't be recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MicError {
    /// Windows' privacy settings keep desktop apps off the microphone.
    Denied,
    /// No microphone is plugged in or turned on.
    Missing,
    /// Something else, as Windows said it.
    Other(String),
}

impl std::fmt::Display for MicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MicError::Denied => f.write_str("access to the microphone is off in Windows' privacy settings"),
            MicError::Missing => f.write_str("no microphone"),
            MicError::Other(e) => f.write_str(e),
        }
    }
}

impl From<windows::core::Error> for MicError {
    fn from(e: windows::core::Error) -> Self {
        mic_error(e.code())
    }
}

/// What an HRESULT from opening the microphone means.
fn mic_error(code: windows::core::HRESULT) -> MicError {
    use windows::Win32::{
        Foundation::{E_ACCESSDENIED, ERROR_NOT_FOUND},
        Media::Audio::{AUDCLNT_E_DEVICE_INVALIDATED, AUDCLNT_E_ENDPOINT_CREATE_FAILED},
    };
    if code == E_ACCESSDENIED {
        MicError::Denied
    } else if code == ERROR_NOT_FOUND.to_hresult()
        || code == AUDCLNT_E_DEVICE_INVALIDATED
        || code == AUDCLNT_E_ENDPOINT_CREATE_FAILED
    {
        MicError::Missing
    } else {
        MicError::Other(windows::core::Error::from(code).to_string())
    }
}

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
    ) -> Result<Recorder, MicError> {
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
            .map_err(|e| MicError::Other(e.to_string()))?;
        match started.recv().unwrap_or_else(|_| Err(MicError::Other("the microphone thread ended".into()))) {
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

fn open(device: Option<&str>, rate: u32) -> Result<(IAudioClient, IAudioCaptureClient), MicError> {
    // SAFETY: COM on this thread (the recorder's).
    let enumerator: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }?;
    // The chosen microphone while it's plugged in and on, then Windows'
    // default communications one, then its default one: a PC whose
    // communications default is a turned-off or virtual device still talks.
    let mut tried: Vec<String> = Vec::new();
    let mut last = MicError::Missing;
    let chosen = device.and_then(|id| {
        // SAFETY: an endpoint ID from input_devices.
        unsafe { enumerator.GetDevice(&windows::core::HSTRING::from(id)) }.ok()
    });
    for role in [None, Some(eCommunications), Some(eConsole)] {
        let candidate = match role {
            None => chosen.clone(),
            // SAFETY: a plain enumerator call.
            Some(role) => match unsafe { enumerator.GetDefaultAudioEndpoint(eCapture, role) } {
                Ok(d) => Some(d),
                Err(e) => {
                    if last == MicError::Missing {
                        last = e.into();
                    }
                    None
                }
            },
        };
        let Some(candidate) = candidate else { continue };
        // SAFETY: a plain endpoint call.
        let active = unsafe { candidate.GetState() }.is_ok_and(|s| s == DEVICE_STATE_ACTIVE);
        let id = super::audio::device_id_string(&candidate).unwrap_or_default();
        if !active || tried.contains(&id) {
            continue;
        }
        tried.push(id);
        match start_on(&candidate, rate) {
            Ok(opened) => return Ok(opened),
            Err(e) => {
                tracing::info!(error = %e, "can't record this microphone, trying the next");
                // Denied says the most: the others would be denied too.
                if last != MicError::Denied {
                    last = e;
                }
            }
        }
    }
    Err(last)
}

/// Starts shared capture on `device` in 16-bit mono at `rate`.
fn start_on(device: &IMMDevice, rate: u32) -> Result<(IAudioClient, IAudioCaptureClient), MicError> {
    let format = WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_PCM as u16,
        nChannels: 1,
        nSamplesPerSec: rate,
        nAvgBytesPerSec: rate * 2,
        nBlockAlign: 2,
        wBitsPerSample: 16,
        cbSize: 0,
    };
    // SAFETY: WASAPI shared capture in a format we own; Windows converts the
    // microphone's own to it.
    unsafe {
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
        // 20 ms of buffer, in 100-ns units.
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            200_000,
            0,
            &format,
            None,
        )?;
        let capture: IAudioCaptureClient = client.GetService()?;
        client.Start()?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::{
        Foundation::{E_ACCESSDENIED, E_FAIL, ERROR_NOT_FOUND},
        Media::Audio::AUDCLNT_E_DEVICE_INVALIDATED,
    };

    #[test]
    fn privacy_block_and_missing_microphone_are_told_apart() {
        assert_eq!(mic_error(E_ACCESSDENIED), MicError::Denied);
        assert_eq!(mic_error(ERROR_NOT_FOUND.to_hresult()), MicError::Missing);
        assert_eq!(mic_error(AUDCLNT_E_DEVICE_INVALIDATED), MicError::Missing);
        assert!(matches!(mic_error(E_FAIL), MicError::Other(_)));
    }

    /// Opens whatever microphone this PC has; skipped on one without any.
    #[test]
    fn records_the_default_microphone_when_there_is_one() {
        let (tx, rx) = mpsc::channel();
        match Recorder::start(Some("not-a-real-endpoint".into()), 16_000, move |pcm| {
            let _ = tx.send(pcm.len());
        }) {
            Ok(recorder) => {
                let got = rx.recv_timeout(Duration::from_secs(3));
                drop(recorder);
                assert!(got.is_ok_and(|n| n % 2 == 0), "no sound came from the microphone");
            }
            Err(MicError::Missing) | Err(MicError::Denied) => {}
            Err(e) => panic!("{e}"),
        }
    }
}
