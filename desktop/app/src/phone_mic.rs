// SPDX-License-Identifier: GPL-3.0-or-later
//! Phone as microphone (docs/protocol/mic.md), on this PC: asks a phone for
//! its microphone and plays it into an output device. Played into a virtual
//! audio cable (VB-Audio's CABLE Input, Voicemeeter), the cable's other end
//! is a microphone every app can pick; played to the speakers, it's for
//! listening in. A virtual microphone of NectarLink's own needs an audio
//! driver and is a later, separate step (GitHub #7).

use crate::i18n::tr;
use std::{
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU32, AtomicU64, Ordering},
    },
    time::Duration,
};

use nectarlink_core::{
    AudioOutputDevice, DeviceId, Error, LinkState, MicSetupResponse, MicSink, MicStatus, MirrorAudioConfig,
    NodeEvent,
};

use crate::{
    bridge::app::{describe, show_message},
    core_host,
    state::Changes,
    win::audio_out::Player,
};

/// Where the phone's microphone is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phase {
    #[default]
    Off,
    /// Asked; waiting for the phone's user to tap Start.
    Asking(DeviceId),
    /// Its sound is arriving.
    Live(DeviceId),
}

static PHASE: Mutex<Phase> = Mutex::new(Phase::Off);
/// How loud the microphone plays, in percent.
static GAIN: AtomicU32 = AtomicU32::new(DEFAULT_GAIN);
/// Phones' microphones come in quieter than a headset's: a little louder
/// by default.
pub const DEFAULT_GAIN: u32 = 150;
pub const MIN_GAIN: u32 = 50;
pub const MAX_GAIN: u32 = 400;
/// The output device chosen in Settings (an endpoint ID), if any.
static OUTPUT: Mutex<Option<String>> = Mutex::new(None);
/// The sink the microphone plays through now, to move it when the output changes.
static ACTIVE: Mutex<Weak<Sink>> = Mutex::new(Weak::new());
/// Counts asks, so a timeout only ends the ask it was set for.
static ASKS: AtomicU64 = AtomicU64::new(0);
/// How long the phone's request stays up (its notification times out then too).
const ASK_TIMEOUT: Duration = Duration::from_secs(2 * 60);

fn set_phase(phase: Phase) {
    let mut current = PHASE.lock().unwrap_or_else(|e| e.into_inner());
    if *current != phase {
        tracing::info!(?phase, "phone microphone");
        *current = phase;
        drop(current);
        core_host::host().hub.changed(Changes::MIC);
    }
}

pub fn phase() -> Phase {
    *PHASE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Reports whether this PC has a virtual microphone backend ready for phone mic streaming.
pub fn mic_status() -> MicStatus {
    let active = matches!(phase(), Phase::Live(_));
    #[cfg(target_os = "linux")]
    {
        let ready = crate::win::vmic::is_available();
        MicStatus {
            ready,
            active,
            is_default_input: false,
            device_name: ready.then(|| "NectarLink Phone Mic".to_owned()),
            setup_reason: (!ready)
                .then(|| "Install PipeWire or PulseAudio (pactl) to create a virtual microphone.".to_owned()),
            can_open_setup: false,
        }
    }
    #[cfg(windows)]
    {
        let (devices, _) = outputs();
        let cable = devices.into_iter().find(|d| is_cable(&d.name));
        let ready = cable.is_some();
        let is_default_input =
            crate::win::audio::input_devices().into_iter().any(|d| d.is_default && is_cable(&d.name));
        MicStatus {
            ready,
            active,
            is_default_input,
            device_name: cable.map(|d| d.name),
            setup_reason: (!ready).then(|| {
                "Install VB-Audio Virtual Cable to expose your phone as a Windows microphone.".to_owned()
            }),
            can_open_setup: true,
        }
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let (devices, _) = outputs();
        let cable = devices.into_iter().find(|d| is_cable(&d.name));
        let ready = cable.is_some();
        MicStatus {
            ready,
            active,
            is_default_input: false,
            device_name: cable.map(|d| d.name),
            setup_reason: (!ready)
                .then(|| "Install a virtual audio cable to expose your phone as a microphone.".to_owned()),
            can_open_setup: false,
        }
    }
}

/// Opens the local virtual microphone setup helper (VB-Audio Virtual Cable on Windows).
pub fn open_setup() -> MicSetupResponse {
    #[cfg(windows)]
    {
        match crate::win::shell::open_url("https://vb-audio.com/Cable/") {
            Ok(()) => MicSetupResponse {
                ok: true,
                message: Some("Opened the VB-Audio Virtual Cable download page.".to_owned()),
            },
            Err(err) => MicSetupResponse { ok: false, message: Some(err) },
        }
    }
    #[cfg(target_os = "linux")]
    {
        let ready = crate::win::vmic::is_available();
        MicSetupResponse {
            ok: ready,
            message: Some(if ready {
                "PipeWire / PulseAudio virtual microphone is ready.".to_owned()
            } else {
                "Install PipeWire or PulseAudio (pactl) to enable the virtual microphone.".to_owned()
            }),
        }
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        MicSetupResponse {
            ok: false,
            message: Some("Virtual microphone setup is not automated on this platform.".to_owned()),
        }
    }
}

/// Reads the chosen output from the settings.
pub fn init(settings: &crate::settings::Settings) {
    #[cfg(target_os = "linux")]
    let _ = std::thread::Builder::new()
        .name("vmic-cleanup".into())
        .spawn(crate::win::vmic::cleanup_stale_sources);
    *OUTPUT.lock().unwrap_or_else(|e| e.into_inner()) = settings.phone_mic_output.clone();
    GAIN.store(settings.phone_mic_gain.clamp(MIN_GAIN, MAX_GAIN), Ordering::Relaxed);
}

/// How loud the microphone plays, in percent.
pub fn gain() -> u32 {
    GAIN.load(Ordering::Relaxed)
}

/// Plays the microphone this loud from now on (and the next time).
pub fn set_gain(percent: u32) {
    let percent = percent.clamp(MIN_GAIN, MAX_GAIN);
    if GAIN.swap(percent, Ordering::Relaxed) == percent {
        return;
    }
    save(|s| s.phone_mic_gain = percent);
    core_host::host().hub.changed(Changes::MIC);
}

fn save(update: impl FnOnce(&mut crate::settings::Settings)) {
    let dir = &core_host::host().data_dir;
    let mut settings = crate::settings::Settings::load(dir);
    update(&mut settings);
    if let Err(e) = settings.save(dir) {
        tracing::warn!(error = %e, "can't save the microphone's settings");
    }
}

/// 16-bit samples made `percent` louder, leveled off softly near full scale
/// so loud speech doesn't crackle.
pub(crate) fn amplify(pcm: &mut [u8], percent: u32) {
    if percent == 100 {
        return;
    }
    let gain = percent as f32 / 100.0;
    // Below this (a fraction of full scale) samples scale as they are.
    const KNEE: f32 = 0.7;
    for sample in pcm.as_chunks_mut::<2>().0 {
        let x = f32::from(i16::from_le_bytes(*sample)) / 32768.0 * gain;
        let magnitude = x.abs();
        let y = if magnitude <= KNEE {
            magnitude
        } else {
            KNEE + (1.0 - KNEE) * ((magnitude - KNEE) / (1.0 - KNEE)).tanh()
        };
        let out = (y.copysign(x) * 32767.0).round().clamp(-32768.0, 32767.0) as i16;
        *sample = out.to_le_bytes();
    }
}

/// The output chosen in Settings, if any (kept when other settings save).
pub fn chosen_output() -> Option<String> {
    OUTPUT.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Names that virtual audio cables give their playback end.
const CABLES: &[&str] =
    &["cable input", "vb-audio", "voicemeeter input", "virtual cable", "virtual microphone"];

#[cfg(target_os = "linux")]
pub const VIRTUAL_MIC_ID: &str = "nectarlink:virtual_mic";
#[cfg(target_os = "linux")]
pub const VIRTUAL_MIC_NAME: &str = "NectarLink Phone Mic (Virtual Microphone, Stereo)";
#[cfg(target_os = "linux")]
pub const VIRTUAL_MIC_MONO_ID: &str = "nectarlink:virtual_mic_mono";
#[cfg(target_os = "linux")]
pub const VIRTUAL_MIC_MONO_NAME: &str = "NectarLink Phone Mic (Virtual Microphone, Mono)";

/// The output to play into: the one chosen, while it's there; else a
/// virtual audio cable, if there is one; else `None` (the default output).
pub(crate) fn preferred(devices: &[AudioOutputDevice], chosen: Option<&str>) -> Option<String> {
    if let Some(id) = chosen
        && devices.iter().any(|d| d.id == id)
    {
        return Some(id.to_owned());
    }
    devices.iter().find(|d| is_cable(&d.name)).map(|d| d.id.clone())
}

/// Whether an output with this name is a virtual audio cable.
pub fn is_cable(name: &str) -> bool {
    let name = name.to_lowercase();
    CABLES.iter().any(|cable| name.contains(cable))
}

/// The output devices, and the one the microphone plays into.
pub fn outputs() -> (Vec<AudioOutputDevice>, Option<String>) {
    #[allow(unused_mut)]
    let mut devices = crate::win::audio::output_devices();
    #[cfg(target_os = "linux")]
    if crate::win::vmic::is_available() {
        if !devices.iter().any(|d| d.id == VIRTUAL_MIC_MONO_ID) {
            devices.insert(
                0,
                AudioOutputDevice {
                    id: VIRTUAL_MIC_MONO_ID.into(),
                    name: VIRTUAL_MIC_MONO_NAME.into(),
                    is_default: false,
                },
            );
        }
        if !devices.iter().any(|d| d.id == VIRTUAL_MIC_ID) {
            devices.insert(
                0,
                AudioOutputDevice {
                    id: VIRTUAL_MIC_ID.into(),
                    name: VIRTUAL_MIC_NAME.into(),
                    is_default: false,
                },
            );
        }
    }
    let chosen = OUTPUT.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let output = preferred(&devices, chosen.as_deref())
        .or_else(|| devices.iter().find(|d| d.is_default).map(|d| d.id.clone()));
    (devices, output)
}

/// Plays the microphone into this output from now on (and the next time).
pub fn set_output(id: String) {
    {
        let mut current = OUTPUT.lock().unwrap_or_else(|e| e.into_inner());
        if current.as_deref() == Some(id.as_str()) {
            return;
        }
        *current = Some(id.clone());
    }
    save(|s| s.phone_mic_output = Some(id));
    // Already playing: move there now rather than on the next start.
    if let Some(sink) = ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).upgrade() {
        sink.restart();
    }
    core_host::host().hub.changed(Changes::MIC);
}

/// The phone whose microphone to use: the one asked or listened to, else
/// the first online one that can lend it, else the first online one (which
/// says so if it can't).
pub fn phone() -> Option<(DeviceId, String)> {
    let current = match phase() {
        Phase::Asking(id) | Phase::Live(id) => Some(id),
        Phase::Off => None,
    };
    core_host::host().hub.read(|s| {
        let can = |id: &DeviceId| {
            s.matrices.get(id).and_then(|m| m.state("media.phone_mic"))
                == Some(nectarlink_core::FeatureState::Available)
        };
        s.devices
            .iter()
            .find(|d| Some(d.id) == current)
            .or_else(|| s.devices.iter().find(|d| matches!(d.link, LinkState::Online { .. }) && can(&d.id)))
            .or_else(|| s.devices.iter().find(|d| matches!(d.link, LinkState::Online { .. })))
            .map(|d| (d.id, d.info.name.clone()))
    })
}

/// Asks `phone` for its microphone; its user taps Start there.
pub fn start(phone: DeviceId) {
    let Some(node) = core_host::node() else { return };
    let name = core_host::host().hub.read(|s| s.name_of(&phone)).unwrap_or_else(|| "your phone".into());
    set_phase(Phase::Asking(phone));
    let ask = ASKS.fetch_add(1, Ordering::Relaxed) + 1;
    // Nobody tapped Start or Not now on the phone: stop waiting when its request goes.
    core_host::spawn(async move {
        tokio::time::sleep(ASK_TIMEOUT).await;
        if ASKS.load(Ordering::Relaxed) == ask && phase() == Phase::Asking(phone) {
            stop();
        }
    });
    core_host::spawn(async move {
        match node.mic_start(phone).await {
            Ok(()) => show_message(tr!("PhoneMic", "Tap Start on %1 to use its microphone.", name)),
            Err(e) => {
                set_phase(Phase::Off);
                show_message(match e {
                    Error::Unsupported => {
                        tr!("PhoneMic", "Update NectarLink on %1 to use its microphone.", name)
                    }
                    Error::Denied => tr!("PhoneMic", "The microphone is turned off for this PC on %1.", name),
                    other => describe(&other),
                });
            }
        }
    });
}

/// Stops listening (and asking).
pub fn stop() {
    let (Phase::Asking(phone) | Phase::Live(phone)) = phase() else { return };
    set_phase(Phase::Off);
    if let Some(node) = core_host::node() {
        core_host::spawn(async move { node.mic_stop(phone).await });
    }
}

pub fn on_event(event: &NodeEvent) {
    set_phase(after(phase(), event));
}

/// Where the microphone is after `event`.
fn after(phase: Phase, event: &NodeEvent) -> Phase {
    let ours = |device: &DeviceId| matches!(phase, Phase::Asking(d) | Phase::Live(d) if d == *device);
    match event {
        NodeEvent::Mic { device, on: true } => Phase::Live(*device),
        // Stopped on the phone, or declined there before any sound came.
        NodeEvent::Mic { device, on: false } if ours(device) => Phase::Off,
        NodeEvent::LinkChanged { device, link: LinkState::Offline { .. } }
        | NodeEvent::DeviceRemoved(device)
            if ours(device) =>
        {
            Phase::Off
        }
        _ => phase,
    }
}

/// Where a phone's microphone goes: [`Platform::mic_sink`](nectarlink_core::Platform::mic_sink).
pub fn sink(peer: &DeviceId) -> Option<Arc<dyn MicSink>> {
    let sink = Arc::new(Sink {
        #[cfg(target_os = "linux")]
        peer: *peer,
        player: Mutex::new(None),
        config: Mutex::new(None),
        #[cfg(target_os = "linux")]
        vmic: Mutex::new(None),
    });
    let _ = peer;
    *ACTIVE.lock().unwrap_or_else(|e| e.into_inner()) = Arc::downgrade(&sink);
    Some(sink)
}

struct Sink {
    #[cfg(target_os = "linux")]
    peer: DeviceId,
    player: Mutex<Option<Player>>,
    /// The stream's format, to start again on another output.
    config: Mutex<Option<MirrorAudioConfig>>,
    #[cfg(target_os = "linux")]
    vmic: Mutex<Option<crate::win::vmic::VirtualMic>>,
}

impl Sink {
    /// Plays into the output chosen now.
    fn restart(&self) {
        let Some(config) = self.config.lock().unwrap_or_else(|e| e.into_inner()).clone() else { return };
        let (_, output) = outputs();
        #[cfg(target_os = "linux")]
        if matches!(output.as_deref(), Some(VIRTUAL_MIC_ID | VIRTUAL_MIC_MONO_ID)) {
            self.player.lock().unwrap_or_else(|e| e.into_inner()).take();
            let out_channels = if output.as_deref() == Some(VIRTUAL_MIC_MONO_ID) { 1 } else { 2 };
            let phone_name = core_host::host().hub.read(|s| s.name_of(&self.peer));
            match crate::win::vmic::VirtualMic::open_with_label(
                config.rate,
                config.channels,
                out_channels,
                phone_name.as_deref(),
            ) {
                Ok(vmic) => {
                    *self.vmic.lock().unwrap_or_else(|e| e.into_inner()) = Some(vmic);
                    return;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "can't create virtual microphone; falling back to speaker output");
                }
            }
        }
        #[cfg(target_os = "linux")]
        self.vmic.lock().unwrap_or_else(|e| e.into_inner()).take();
        match Player::start_on(output, config.rate, u16::from(config.channels)) {
            Ok(player) => *self.player.lock().unwrap_or_else(|e| e.into_inner()) = Some(player),
            Err(e) => tracing::warn!(error = %e, "can't play the phone's microphone"),
        }
    }
}

impl MicSink for Sink {
    fn config(&self, config: MirrorAudioConfig) {
        *self.config.lock().unwrap_or_else(|e| e.into_inner()) = Some(config);
        self.restart();
    }

    fn audio(&self, _time_us: u64, mut data: Vec<u8>) {
        #[cfg(target_os = "linux")]
        if let Some(vmic) = self.vmic.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            amplify(&mut data, gain());
            let _ = vmic.push_pcm(&data);
            return;
        }
        if let Some(player) = self.player.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            amplify(&mut data, gain());
            player.push(data);
        }
    }

    fn ended(&self) {
        *self.config.lock().unwrap_or_else(|e| e.into_inner()) = None;
        #[cfg(target_os = "linux")]
        self.vmic.lock().unwrap_or_else(|e| e.into_inner()).take();
        self.player.lock().unwrap_or_else(|e| e.into_inner()).take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: &str, name: &str, is_default: bool) -> AudioOutputDevice {
        AudioOutputDevice { id: id.into(), name: name.into(), is_default }
    }

    #[test]
    fn a_virtual_cable_is_preferred_until_another_output_is_chosen() {
        let devices = [
            device("spk", "Speakers (Realtek Audio)", true),
            device("cable", "CABLE Input (VB-Audio Virtual Cable)", false),
        ];
        assert_eq!(preferred(&devices, None).as_deref(), Some("cable"));
        assert_eq!(preferred(&devices, Some("spk")).as_deref(), Some("spk"));
        // The chosen one was unplugged: the cable again.
        assert_eq!(preferred(&devices, Some("usb-headset")).as_deref(), Some("cable"));
        assert_eq!(
            preferred(&[device("vm", "Voicemeeter Input (VB-Audio Voicemeeter VAIO)", false)], None)
                .as_deref(),
            Some("vm")
        );
    }

    fn samples(values: &[i16]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    fn read(pcm: &[u8]) -> Vec<i16> {
        pcm.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect()
    }

    #[test]
    fn louder_doubles_quiet_sound_and_levels_off_loud_sound() {
        let mut pcm = samples(&[1000, -1000, 0, 30000, -32768]);
        amplify(&mut pcm, 200);
        let out = read(&pcm);
        assert_eq!(&out[..3], &[2000, -2000, 0], "quiet sound doubles exactly");
        // Loud sound stays inside full scale, still loud, sign kept.
        assert!(out[3] > 28000 && out[3] < 32767, "{}", out[3]);
        assert!(out[4] < -28000 && out[4] > -32768, "{}", out[4]);
        // 100% leaves it as it is.
        let mut same = samples(&[123, -4567]);
        amplify(&mut same, 100);
        assert_eq!(read(&same), [123, -4567]);
    }

    #[test]
    fn the_phone_declining_or_stopping_ends_the_ask_and_the_sound() {
        let phone = DeviceId([1; 32]);
        let other = DeviceId([2; 32]);
        let mic = |device, on| NodeEvent::Mic { device, on };
        assert_eq!(after(Phase::Off, &mic(phone, true)), Phase::Live(phone));
        // Not now on the phone: the PC stops waiting.
        assert_eq!(after(Phase::Asking(phone), &mic(phone, false)), Phase::Off);
        assert_eq!(after(Phase::Live(phone), &mic(phone, false)), Phase::Off);
        // Another phone stopping changes nothing here.
        assert_eq!(after(Phase::Asking(phone), &mic(other, false)), Phase::Asking(phone));
        assert_eq!(after(Phase::Live(phone), &NodeEvent::DeviceRemoved(phone)), Phase::Off);
    }

    #[test]
    fn without_a_cable_it_is_the_default_output() {
        assert_eq!(preferred(&[device("spk", "Speakers", true)], None), None);
    }

    #[test]
    fn virtual_microphone_is_recognized_as_preferred_cable() {
        let devices = [
            device("nectarlink:virtual_mic", "NectarLink Phone Mic (Virtual Microphone)", false),
            device("spk", "Speakers", true),
        ];
        assert!(is_cable("NectarLink Phone Mic (Virtual Microphone)"));
        assert_eq!(preferred(&devices, None).as_deref(), Some("nectarlink:virtual_mic"));
        assert_eq!(preferred(&devices, Some("spk")).as_deref(), Some("spk"));
    }
}
