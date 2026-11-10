// SPDX-License-Identifier: GPL-3.0-or-later
//! Phone as Speaker / Headphones for PC Audio (§3.8 P2): captures Windows
//! system audio via WASAPI loopback (`AUDCLNT_STREAMFLAGS_LOOPBACK`), converts
//! 32-bit float stereo samples to 16-bit signed little-endian PCM, and streams
//! them to connected phones over NectarLink's low-latency audio stream.

/// Where the PC's sound goes: only to the phone that asked for it, waiting
/// while it's away (never to another phone, which may be someone else's), or
/// to the first online phone when the PC itself turned the speaker on.
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
fn pick_target(
    wanted: Option<nectarlink_core::DeviceId>,
    online: &[nectarlink_core::DeviceId],
) -> Option<nectarlink_core::DeviceId> {
    match wanted {
        Some(phone) => online.contains(&phone).then_some(phone),
        None => online.first().copied(),
    }
}

#[cfg(test)]
mod tests {
    use super::pick_target;
    use nectarlink_core::DeviceId;

    #[test]
    fn sound_never_moves_to_a_phone_that_did_not_ask() {
        let (a, b) = (DeviceId([1; 32]), DeviceId([2; 32]));
        assert_eq!(pick_target(Some(a), &[b, a]), Some(a));
        assert_eq!(pick_target(Some(a), &[b]), None);
        assert_eq!(pick_target(None, &[b, a]), Some(b));
        assert_eq!(pick_target(None, &[]), None);
    }
}

#[cfg(any(windows, target_os = "linux"))]
mod imp {

    #![allow(unsafe_code)]

    use std::{
        sync::atomic::{AtomicBool, Ordering},
        time::Duration,
    };

    #[cfg(windows)]
    use windows::Win32::{
        Media::Audio::{
            AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
            IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator, WAVEFORMATEX,
            eConsole, eRender,
        },
        System::{
            Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree},
            Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL},
        },
    };

    use crate::core_host;

    static ENABLED: AtomicBool = AtomicBool::new(false);
    static WORKER_RUNNING: AtomicBool = AtomicBool::new(false);
    /// The phone that last turned "Phone speaker" on. Audio goes only there; when
    /// the PC turned it on, to the first phone that is online.
    static TARGET: std::sync::Mutex<Option<nectarlink_core::DeviceId>> = std::sync::Mutex::new(None);

    /// The phone the sound goes to first, if one asked.
    pub fn target() -> Option<nectarlink_core::DeviceId> {
        *TARGET.lock().unwrap_or_else(|e| e.into_inner())
    }

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

    struct SpeakerEnhancer {
        low_shelf: Biquad,
        high_shelf: Biquad,
    }

    impl SpeakerEnhancer {
        fn new(sample_rate: u32) -> Self {
            let sr = sample_rate as f32;
            Self {
                low_shelf: Biquad::low_shelf(sr, 155.0, 3.5),
                high_shelf: Biquad::high_shelf(sr, 3800.0, 1.5),
            }
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

        #[cfg_attr(target_os = "linux", allow(dead_code))]
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

    /// Formats a PulseAudio/PipeWire sink name into its monitor source name.
    #[cfg_attr(windows, allow(dead_code))]
    pub(crate) fn format_monitor_source_name(default_sink: Option<&str>) -> String {
        let Some(trimmed) = default_sink.map(str::trim).filter(|s| !s.is_empty()) else {
            return "@DEFAULT_MONITOR@".to_owned();
        };
        if trimmed.ends_with(".monitor") { trimmed.to_owned() } else { format!("{trimmed}.monitor") }
    }

    /// Builds `pw-record` CLI arguments that capture desktop sink monitor loopback
    /// (`-P stream.capture.sink=true`) rather than the default microphone input.
    #[cfg_attr(windows, allow(dead_code))]
    pub(crate) fn pw_record_loopback_args(monitor: &str) -> Vec<String> {
        let mut args = vec![
            "--raw".to_owned(),
            "--format=f32".to_owned(),
            "--rate=48000".to_owned(),
            "--channels=2".to_owned(),
            "-P".to_owned(),
            "stream.capture.sink=true".to_owned(),
        ];
        let trimmed = monitor.trim();
        if !trimmed.is_empty() && trimmed != "@DEFAULT_MONITOR@" {
            let sink_name = trimmed.strip_suffix(".monitor").unwrap_or(trimmed).trim();
            if !sink_name.is_empty() && !sink_name.starts_with('-') {
                args.push("--target".to_owned());
                args.push(sink_name.to_owned());
            }
        }
        args.push("-".to_owned());
        args
    }

    #[cfg(target_os = "linux")]
    fn resolve_default_monitor_source() -> String {
        let default_sink = std::process::Command::new("pactl")
            .arg("get-default-sink")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
        format_monitor_source_name(default_sink.as_deref())
    }

    #[cfg(target_os = "linux")]
    fn run_loopback_worker() {
        while is_enabled() {
            if let Err(e) = capture_and_stream_once() {
                tracing::debug!(error = %e, "PC audio loopback stream cycle ended");
                for _ in 0..30 {
                    if !is_enabled() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn capture_and_stream_once() -> Result<(), String> {
        use std::io::Read;
        use std::process::{Command, Stdio};

        let node = core_host::node().ok_or_else(|| "core node not running".to_owned())?;
        let peers: Vec<_> = node
            .paired_devices()
            .unwrap_or_default()
            .into_iter()
            .filter(|d| matches!(d.link, nectarlink_core::LinkState::Online { .. }))
            .map(|d| d.id)
            .collect();
        let wanted = *TARGET.lock().unwrap_or_else(|e| e.into_inner());
        let Some(peer) = super::pick_target(wanted, &peers) else {
            std::thread::sleep(Duration::from_millis(500));
            return Ok(());
        };

        let monitor = resolve_default_monitor_source();
        let rate: u32 = 48_000;
        let channels: usize = 2;

        let mut child = Command::new("parec")
            .args([
                "--raw",
                "--format=float32le",
                "--rate=48000",
                "--channels=2",
                "--latency-msec=10",
                &format!("--device={monitor}"),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .or_else(|_| {
                let pw_args = pw_record_loopback_args(&monitor);
                Command::new("pw-record").args(&pw_args).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()
            })
            .map_err(|e| format!("failed to spawn parec/pw-record: {e}"))?;

        let Some(mut stdout) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("loopback recorder stdout unavailable".into());
        };

        let mut enhancer = SpeakerEnhancer::new(rate);
        let (tx, mut rx) = tokio::sync::mpsc::channel::<(u32, u8, Vec<u8>)>(64);
        let send_node = node.clone();
        core_host::spawn(async move {
            while let Some((r, ch, pcm)) = rx.recv().await {
                for chunk in pcm.chunks(960) {
                    if send_node.send_pc_audio_pcm(&peer, r, ch, chunk.to_vec()).await.is_err() {
                        return;
                    }
                }
            }
        });

        // 480 stereo f32 frames = 10 ms = 3840 bytes.
        let frames_per_read = 480usize;
        let mut raw_buf = vec![0u8; frames_per_read * channels * 4];
        let mut floats = vec![0.0_f32; frames_per_read * channels];
        let mut silence_tail_packets: u32 = 0;

        while is_enabled() {
            if target() != wanted && target().is_some_and(|t| peers.contains(&t)) {
                break;
            }
            if stdout.read_exact(&mut raw_buf).is_err() {
                break;
            }
            for (dst, &chunk) in floats.iter_mut().zip(raw_buf.as_chunks::<4>().0) {
                *dst = f32::from_le_bytes(chunk);
            }
            let has_signal = floats.iter().any(|&s| s.is_finite() && s.abs() > 1e-5);
            let pcm_bytes = if has_signal {
                silence_tail_packets = 50;
                enhancer.encode_stereo_f32(&floats, channels)
            } else if silence_tail_packets > 0 {
                silence_tail_packets -= 1;
                vec![0u8; frames_per_read * 4]
            } else {
                Vec::new()
            };

            if !pcm_bytes.is_empty()
                && matches!(
                    tx.try_send((rate, 2, pcm_bytes)),
                    Err(tokio::sync::mpsc::error::TrySendError::Closed(_))
                )
            {
                break;
            }
        }

        let _ = child.kill();
        let _ = child.wait();
        Ok(())
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

    /// Tells [`process_loopback_client`] that Windows has made the client.
    #[cfg(windows)]
    #[windows::core::implement(windows::Win32::Media::Audio::IActivateAudioInterfaceCompletionHandler)]
    struct Activated(std::sync::mpsc::Sender<()>);

    #[cfg(windows)]
    impl windows::Win32::Media::Audio::IActivateAudioInterfaceCompletionHandler_Impl for Activated_Impl {
        fn ActivateCompleted(
            &self,
            _: windows::core::Ref<windows::Win32::Media::Audio::IActivateAudioInterfaceAsyncOperation>,
        ) -> windows::core::Result<()> {
            let _ = self.0.send(());
            Ok(())
        }
    }

    /// A capture of everything the PC plays except NectarLink itself: the phone's
    /// own sound playing here must not go back to the phone, round and round (#281).
    #[cfg(windows)]
    fn process_loopback_client() -> Result<IAudioClient, String> {
        use windows::Win32::{
            Media::Audio::{
                AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_PARAMS_0,
                AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
                ActivateAudioInterfaceAsync, IActivateAudioInterfaceCompletionHandler,
                PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE, VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            },
            System::{
                Com::{
                    BLOB,
                    StructuredStorage::{PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0},
                },
                Variant::VT_BLOB,
            },
        };
        use windows::core::{IUnknown, Interface};

        let mut params = AUDIOCLIENT_ACTIVATION_PARAMS {
            ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
            Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
                ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                    TargetProcessId: std::process::id(),
                    ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
                },
            },
        };
        // The parameters go as a blob, which the PROPVARIANT only points to. It's
        // never dropped: dropping one frees its blob, which here is on the stack.
        let blob = std::mem::ManuallyDrop::new(PROPVARIANT {
            Anonymous: PROPVARIANT_0 {
                Anonymous: std::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
                    vt: VT_BLOB,
                    wReserved1: 0,
                    wReserved2: 0,
                    wReserved3: 0,
                    Anonymous: PROPVARIANT_0_0_0 {
                        blob: BLOB {
                            cbSize: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
                            pBlobData: (&raw mut params).cast(),
                        },
                    },
                }),
            },
        });
        let (done, finished) = std::sync::mpsc::channel();
        let handler: IActivateAudioInterfaceCompletionHandler = Activated(done).into();
        // SAFETY: `params` and `blob` outlive the call, which copies them before it returns.
        let operation = unsafe {
            ActivateAudioInterfaceAsync(
                VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
                &IAudioClient::IID,
                Some(&raw const *blob),
                &handler,
            )
        }
        .map_err(|e| e.to_string())?;
        finished.recv_timeout(Duration::from_secs(5)).map_err(|_| "activation timed out".to_owned())?;
        let mut result = windows::core::HRESULT(0);
        let mut unknown: Option<IUnknown> = None;
        // SAFETY: the operation has completed, as the handler said.
        unsafe { operation.GetActivateResult(&mut result, &mut unknown) }.map_err(|e| e.to_string())?;
        result.ok().map_err(|e| e.to_string())?;
        unknown.ok_or("no audio client")?.cast::<IAudioClient>().map_err(|e| e.to_string())
    }

    /// The capture client, its rate, channels and bits per sample. Normally what
    /// the default output plays. With `without_own`, every program's sound but
    /// NectarLink's (Windows version 2004 and later), for while it plays a phone's
    /// sound here: that would go back to the phone and loop. Only then, because
    /// that capture takes each program's sound separately, and a PC whose sound
    /// runs through a mixer app (Voicemeeter, Sonar and the like) gives it twice.
    #[cfg(windows)]
    fn open_capture(
        without_own: bool,
    ) -> Result<(IAudioClient, IAudioCaptureClient, u32, usize, u16), String> {
        use windows::Win32::Media::Audio::{AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, WAVE_FORMAT_PCM};
        let own_left_out = if without_own { process_loopback_client().map(Some) } else { Ok(None) };
        match own_left_out {
            Ok(None) => {}
            Ok(Some(client)) => {
                // This client has no mix format of its own; Windows converts to this.
                let format = WAVEFORMATEX {
                    wFormatTag: WAVE_FORMAT_PCM as u16,
                    nChannels: 2,
                    nSamplesPerSec: 48_000,
                    nAvgBytesPerSec: 48_000 * 4,
                    nBlockAlign: 4,
                    wBitsPerSample: 16,
                    cbSize: 0,
                };
                // SAFETY: a shared-mode loopback capture of a format we own.
                let started = unsafe {
                    client
                        .Initialize(
                            AUDCLNT_SHAREMODE_SHARED,
                            AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
                            100_000,
                            0,
                            &format,
                            None,
                        )
                        .and_then(|()| client.GetService::<IAudioCaptureClient>())
                        .and_then(|capture| client.Start().map(|()| capture))
                };
                match started {
                    Ok(capture) => return Ok((client, capture, 48_000, 2, 16)),
                    Err(e) => tracing::info!(error = %e, "can't capture without NectarLink's own sound"),
                }
            }
            Err(e) => tracing::info!(error = %e, "can't capture without NectarLink's own sound"),
        }
        // SAFETY: WASAPI loopback initialization on the default render endpoint.
        unsafe {
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
            Ok((client, capture, rate, channels, bits))
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
        let Some(peer) = super::pick_target(wanted, &peers) else {
            std::thread::sleep(Duration::from_millis(500));
            return Ok(());
        };

        let without_own = crate::win::audio_out::playing();
        let (client, capture, rate, channels, bits_per_sample) = open_capture(without_own)?;

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
            // Another phone asked for the sound (it watches the PC's playback):
            // start over for that one.
            if target() != wanted && target().is_some_and(|t| peers.contains(&t)) {
                break 'capture;
            }
            // NectarLink started or stopped playing a phone's sound here: capture
            // the other way from now on.
            if crate::win::audio_out::playing() != without_own {
                break 'capture;
            }
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
                    if capture.GetBuffer(&mut data_ptr, &mut frames_to_read, &mut flags, None, None).is_err()
                    {
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

        /// Plays a tone too quiet to hear and checks what each capture picks up:
        /// everything sees it, the one for Phone as speaker doesn't. Needs a sound
        /// device, so it runs by hand: `cargo test -- --ignored own_sound`.
        #[cfg(windows)]
        #[test]
        #[ignore]
        fn phone_as_speaker_leaves_out_nectarlinks_own_sound() {
            use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
            // SAFETY: COM for this test thread.
            let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };

            // The peak of what a capture has picked up since the last look.
            let drain = |capture: &IAudioCaptureClient, bits: u16| -> f32 {
                let mut loudest = 0f32;
                // SAFETY: draining the capture as the worker does.
                unsafe {
                    while capture.GetNextPacketSize().unwrap_or(0) > 0 {
                        let (mut data, mut frames, mut flags) = (std::ptr::null_mut(), 0u32, 0u32);
                        if capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None).is_err() {
                            break;
                        }
                        if !data.is_null() && flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) == 0 {
                            let n = frames as usize * 2;
                            let peak = if bits == 32 {
                                std::slice::from_raw_parts(data.cast::<f32>(), n)
                                    .iter()
                                    .fold(0f32, |m, s| m.max(s.abs()))
                            } else {
                                std::slice::from_raw_parts(data.cast::<i16>(), n)
                                    .iter()
                                    .fold(0f32, |m, s| m.max(f32::from(s.unsigned_abs()) / 32_768.0))
                            };
                            loudest = loudest.max(peak);
                        }
                        let _ = capture.ReleaseBuffer(frames);
                    }
                }
                loudest
            };

            let (ours, ours_capture, _, _, ours_bits) = open_capture(true).expect("a capture");
            assert_eq!(ours_bits, 16, "the capture without NectarLink's own sound opened");
            // SAFETY: the whole output, as before this change.
            let (all, all_capture, all_bits) = unsafe {
                let enumerator: IMMDeviceEnumerator =
                    CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).unwrap();
                let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole).unwrap();
                let client: IAudioClient = device.Activate(CLSCTX_ALL, None).unwrap();
                let mix = client.GetMixFormat().unwrap();
                let bits = (*mix).wBitsPerSample;
                client
                    .Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, 100_000, 0, mix, None)
                    .unwrap();
                let capture: IAudioCaptureClient = client.GetService().unwrap();
                client.Start().unwrap();
                (client, capture, bits)
            };

            let player = crate::win::audio_out::Player::start(48_000, 2).expect("a sound device");
            // About -50 dB, in 100 ms pieces as the mirror plays them.
            let piece: Vec<u8> = (0..4_800)
                .flat_map(|i| {
                    let s = ((i as f32 * 400.0 * std::f32::consts::TAU / 48_000.0).sin() * 100.0) as i16;
                    [s.to_le_bytes(), s.to_le_bytes()].concat()
                })
                .collect();
            let (mut without_us, mut everything) = (0f32, 0f32);
            for _ in 0..20 {
                player.push(piece.clone());
                std::thread::sleep(Duration::from_millis(100));
                without_us = without_us.max(drain(&ours_capture, ours_bits));
                everything = everything.max(drain(&all_capture, all_bits));
            }
            drop(player);
            // SAFETY: done with them.
            unsafe {
                let _ = ours.Stop();
                let _ = all.Stop();
            }
            eprintln!("whole output {everything}, without NectarLink {without_us}");
            assert!(everything > 0.001, "the whole output didn't pick the tone up: {everything}");
            assert!(without_us < everything / 10.0, "own sound got through: {without_us} vs {everything}");
        }

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

        #[test]
        fn format_monitor_source_name_appends_monitor_suffix_or_uses_default() {
            assert_eq!(
                format_monitor_source_name(Some("alsa_output.pci-0000_00_1f.3.analog-stereo\n")),
                "alsa_output.pci-0000_00_1f.3.analog-stereo.monitor"
            );
            assert_eq!(
                format_monitor_source_name(Some("alsa_output.pci.monitor")),
                "alsa_output.pci.monitor"
            );
            assert_eq!(format_monitor_source_name(Some("  ")), "@DEFAULT_MONITOR@");
            assert_eq!(format_monitor_source_name(None), "@DEFAULT_MONITOR@");

            let default_pw = pw_record_loopback_args("@DEFAULT_MONITOR@");
            assert!(default_pw.windows(2).any(|w| w == ["-P", "stream.capture.sink=true"]));
            assert!(!default_pw.iter().any(|a| a == "--target"));
            assert_eq!(default_pw.last().map(String::as_str), Some("-"));

            let targeted_pw = pw_record_loopback_args("alsa_output.pci-0000_00_1f.3.analog-stereo.monitor");
            assert!(targeted_pw.windows(2).any(|w| w == ["-P", "stream.capture.sink=true"]));
            assert!(
                targeted_pw
                    .windows(2)
                    .any(|w| w == ["--target", "alsa_output.pci-0000_00_1f.3.analog-stereo"])
            );
            assert_eq!(targeted_pw.last().map(String::as_str), Some("-"));
        }

        #[test]
        fn speaker_enhancer_encodes_stereo_f32_and_i16_without_clipping() {
            let mut enhancer = SpeakerEnhancer::new(48_000);
            let floats = [0.25_f32, -0.25, 0.95, -0.95, f32::NAN, 0.5];
            let encoded = enhancer.encode_stereo_f32(&floats, 2);
            assert_eq!(encoded.len(), 3 * 4);
            assert!(encoded.iter().any(|&b| b != 0));

            let i16s = [8000i16, -8000, 16000, -16000];
            let encoded_i16 = enhancer.encode_stereo_i16(&i16s, 2);
            assert_eq!(encoded_i16.len(), 2 * 4);
            assert!(encoded_i16.iter().any(|&b| b != 0));
        }
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod imp {
    #![allow(dead_code)]

    pub fn target() -> Option<nectarlink_core::DeviceId> {
        None
    }

    pub fn set_target(_phone: Option<nectarlink_core::DeviceId>) {}

    pub fn is_enabled() -> bool {
        false
    }

    pub fn set_enabled(_enabled: bool) {}

    pub fn f32_stereo_to_i16le(_samples: &[f32]) -> Vec<u8> {
        Vec::new()
    }
}

pub use imp::*;
