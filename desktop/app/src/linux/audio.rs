// SPDX-License-Identifier: GPL-3.0-or-later
//! Linux speaker volume, mute, output device enumeration, microphone mute
//! state, and event-driven change notifications via PipeWire (`wpctl`) and
//! PulseAudio (`pactl`).

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock, mpsc};
use std::time::Duration;

use nectarlink_core::AudioOutputDevice;

static FALLBACK_SPEAKER: Mutex<(u8, bool)> = Mutex::new((50, false));
static FALLBACK_MIC_MUTED: Mutex<bool> = Mutex::new(false);
static WATCH_STARTED: OnceLock<()> = OnceLock::new();

/// Parses `wpctl get-volume` output such as `"Volume: 0.45"` or
/// `"Volume: 0.45 [MUTED]"` into `(volume_percent, muted)`.
pub(crate) fn parse_wpctl_volume(output: &str) -> Option<(u8, bool)> {
    let muted = output.contains("[MUTED]");
    let after_colon = output.split_once("Volume:")?.1;
    let vol_token = after_colon.split_whitespace().next()?;
    let val = vol_token.parse::<f32>().ok()?;
    if !val.is_finite() {
        return None;
    }
    let pct = (val * 100.0).round().clamp(0.0, 100.0) as u8;
    Some((pct, muted))
}

/// Parses `pactl get-sink-mute` / `pactl get-source-mute` output such as
/// `"Mute: yes"` or `"Mute: no"`.
pub(crate) fn parse_pactl_mute(mute_out: &str) -> Option<bool> {
    let rhs = mute_out.split_once(':').map(|(_, r)| r).unwrap_or(mute_out).trim();
    if rhs.eq_ignore_ascii_case("yes") || rhs.eq_ignore_ascii_case("true") || rhs == "1" {
        Some(true)
    } else if rhs.eq_ignore_ascii_case("no") || rhs.eq_ignore_ascii_case("false") || rhs == "0" {
        Some(false)
    } else {
        None
    }
}

/// Parses `pactl get-sink-volume` and optional `pactl get-sink-mute` output
/// into `(volume_percent, muted)`.
pub(crate) fn parse_pactl_volume_and_mute(vol_out: &str, mute_out: Option<&str>) -> Option<(u8, bool)> {
    let pct_token = vol_out.split_whitespace().find_map(|tok| tok.strip_suffix('%'))?;
    let pct_val = pct_token.trim().parse::<f32>().ok()?;
    if !pct_val.is_finite() {
        return None;
    }
    let pct = pct_val.round().clamp(0.0, 100.0) as u8;
    let muted = mute_out.and_then(parse_pactl_mute).unwrap_or(false);
    Some((pct, muted))
}

/// Parses `pactl list sinks` output along with `pactl get-default-sink` into
/// sorted `AudioOutputDevice` entries (default first, then alphabetical by name).
pub(crate) fn parse_pactl_sinks(list_out: &str, default_sink: Option<&str>) -> Vec<AudioOutputDevice> {
    let default_trimmed = default_sink.map(str::trim).filter(|s| !s.is_empty());
    let mut out = Vec::new();
    let mut current_id: Option<String> = None;
    let mut current_desc: Option<String> = None;

    let flush = |id: &mut Option<String>, desc: &mut Option<String>, out: &mut Vec<AudioOutputDevice>| {
        if let Some(sink_id) = id.take() {
            let name = desc.take().filter(|d| !d.is_empty()).unwrap_or_else(|| sink_id.clone());
            let is_default = default_trimmed == Some(sink_id.as_str());
            out.push(AudioOutputDevice { id: sink_id, name, is_default });
        } else {
            *desc = None;
        }
    };

    for raw_line in list_out.lines() {
        let line = raw_line.trim();
        if line.starts_with("Sink #") {
            flush(&mut current_id, &mut current_desc, &mut out);
        } else if let Some(rest) = line.strip_prefix("Name:") {
            let id = rest.trim();
            if !id.is_empty() && current_id.is_none() {
                current_id = Some(id.to_owned());
            }
        } else if let Some(rest) = line.strip_prefix("Description:") {
            let desc = rest.trim();
            if !desc.is_empty() && current_desc.is_none() {
                current_desc = Some(desc.to_owned());
            }
        }
    }
    flush(&mut current_id, &mut current_desc, &mut out);

    out.sort_by(|a, b| b.is_default.cmp(&a.is_default).then_with(|| a.name.cmp(&b.name)));
    out
}

/// Returns `true` when a `pactl subscribe` event line indicates a sink, source,
/// or server state/device change (ignoring per-stream `sink-input` / `source-output` noise).
pub(crate) fn parse_pactl_subscribe_line(line: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.starts_with("Event '") {
        return false;
    }
    (trimmed.contains(" on sink #")
        || trimmed.ends_with(" on sink")
        || trimmed.contains(" on source #")
        || trimmed.ends_with(" on source")
        || trimmed.contains(" on server #")
        || trimmed.ends_with(" on server"))
        && !trimmed.contains("sink-input")
        && !trimmed.contains("source-output")
}

/// Reads the default speaker endpoint's `(volume_percent, muted)`.
pub fn speaker_state() -> Option<(u8, bool)> {
    if let Ok(out) = Command::new("wpctl").args(["get-volume", "@DEFAULT_AUDIO_SINK@"]).output()
        && out.status.success()
        && let Some(state) = parse_wpctl_volume(&String::from_utf8_lossy(&out.stdout))
    {
        if let Ok(mut guard) = FALLBACK_SPEAKER.lock() {
            *guard = state;
        }
        return Some(state);
    }

    if let Ok(vol_out) = Command::new("pactl").args(["get-sink-volume", "@DEFAULT_SINK@"]).output()
        && vol_out.status.success()
    {
        let mute_out = Command::new("pactl")
            .args(["get-sink-mute", "@DEFAULT_SINK@"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
        if let Some(state) =
            parse_pactl_volume_and_mute(&String::from_utf8_lossy(&vol_out.stdout), mute_out.as_deref())
        {
            if let Ok(mut guard) = FALLBACK_SPEAKER.lock() {
                *guard = state;
            }
            return Some(state);
        }
    }

    Some(*FALLBACK_SPEAKER.lock().ok()?)
}

/// Sets the default speaker endpoint's master volume (`0..=100`).
pub fn set_speaker_volume(level: u8) -> Result<(), String> {
    let clamped = level.min(100);
    if let Ok(mut guard) = FALLBACK_SPEAKER.lock() {
        guard.0 = clamped;
    }
    let arg = format!("{clamped}%");
    let _ = Command::new("wpctl")
        .args(["set-volume", "@DEFAULT_AUDIO_SINK@", &arg])
        .status()
        .and_then(|s| if s.success() { Ok(s) } else { Err(std::io::Error::other("wpctl failed")) })
        .or_else(|_| Command::new("pactl").args(["set-sink-volume", "@DEFAULT_SINK@", &arg]).status());
    Ok(())
}

/// Sets the default speaker endpoint's mute state.
pub fn set_speaker_mute(muted: bool) -> Result<(), String> {
    if let Ok(mut guard) = FALLBACK_SPEAKER.lock() {
        guard.1 = muted;
    }
    let flag = if muted { "1" } else { "0" };
    let _ = Command::new("wpctl")
        .args(["set-mute", "@DEFAULT_AUDIO_SINK@", flag])
        .status()
        .and_then(|s| if s.success() { Ok(s) } else { Err(std::io::Error::other("wpctl failed")) })
        .or_else(|_| Command::new("pactl").args(["set-sink-mute", "@DEFAULT_SINK@", flag]).status());
    Ok(())
}

/// Steps the default speaker volume up by one increment (2%) and unmutes if muted.
pub fn speaker_volume_up() -> Result<(u8, bool), String> {
    let (current, muted) = speaker_state().unwrap_or((50, false));
    let next = current.saturating_add(2).min(100);
    set_speaker_volume(next)?;
    if muted {
        set_speaker_mute(false)?;
    }
    Ok((next, false))
}

/// Steps the default speaker volume down by one increment (2%).
pub fn speaker_volume_down() -> Result<(u8, bool), String> {
    let (current, muted) = speaker_state().unwrap_or((50, false));
    let next = current.saturating_sub(2);
    set_speaker_volume(next)?;
    Ok((next, muted))
}

/// Toggles mute on the default speaker endpoint and returns the new muted state.
pub fn toggle_speaker_mute() -> Result<bool, String> {
    let (_, muted) = speaker_state().unwrap_or((50, false));
    let next = !muted;
    set_speaker_mute(next)?;
    Ok(next)
}

/// Reads whether the default capture (microphone) endpoint is muted.
pub fn mic_muted() -> Option<bool> {
    if let Ok(out) = Command::new("wpctl").args(["get-volume", "@DEFAULT_AUDIO_SOURCE@"]).output()
        && out.status.success()
        && let Some((_, muted)) = parse_wpctl_volume(&String::from_utf8_lossy(&out.stdout))
    {
        if let Ok(mut guard) = FALLBACK_MIC_MUTED.lock() {
            *guard = muted;
        }
        return Some(muted);
    }

    if let Ok(out) = Command::new("pactl").args(["get-source-mute", "@DEFAULT_SOURCE@"]).output()
        && out.status.success()
        && let Some(muted) = parse_pactl_mute(&String::from_utf8_lossy(&out.stdout))
    {
        if let Ok(mut guard) = FALLBACK_MIC_MUTED.lock() {
            *guard = muted;
        }
        return Some(muted);
    }

    Some(*FALLBACK_MIC_MUTED.lock().ok()?)
}

/// Toggles mute on the default capture (microphone) endpoint and returns the new muted state.
pub fn toggle_mic_mute() -> Result<bool, String> {
    let current = mic_muted().unwrap_or(false);
    let next = !current;
    if let Ok(mut guard) = FALLBACK_MIC_MUTED.lock() {
        *guard = next;
    }
    let flag = if next { "1" } else { "0" };
    let _ = Command::new("wpctl")
        .args(["set-mute", "@DEFAULT_AUDIO_SOURCE@", flag])
        .status()
        .and_then(|s| if s.success() { Ok(s) } else { Err(std::io::Error::other("wpctl failed")) })
        .or_else(|_| Command::new("pactl").args(["set-source-mute", "@DEFAULT_SOURCE@", flag]).status());
    Ok(next)
}

/// Lists active audio output sinks on this PC, marking the current default
/// output (`is_default: true`) and sorting the default first.
pub fn output_devices() -> Vec<AudioOutputDevice> {
    let default_sink = Command::new("pactl")
        .arg("get-default-sink")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|s| !s.is_empty());

    if let Ok(list_out) = Command::new("pactl").args(["list", "sinks"]).output()
        && list_out.status.success()
    {
        let parsed = parse_pactl_sinks(&String::from_utf8_lossy(&list_out.stdout), default_sink.as_deref());
        if !parsed.is_empty() {
            return parsed;
        }
    }

    vec![AudioOutputDevice {
        id: "default".to_string(),
        name: "Default Output".to_string(),
        is_default: true,
    }]
}

/// Switches the system default output sink to `id`.
pub fn set_default_output_device(id: &str) -> Result<(), String> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err("output device id cannot be empty".to_string());
    }
    let status = Command::new("pactl")
        .args(["set-default-sink", trimmed])
        .status()
        .and_then(|s| if s.success() { Ok(s) } else { Err(std::io::Error::other("pactl failed")) })
        .or_else(|_| Command::new("wpctl").args(["set-default", trimmed]).status())
        .map_err(|e| format!("failed to set default output device: {e}"))?;

    if status.success() { Ok(()) } else { Err(format!("audio backend rejected default sink '{trimmed}'")) }
}

/// Subscribes to PipeWire/PulseAudio sink/source/server events and invokes
/// `changed` after coalescing rapid event bursts over 80 ms.
pub fn watch(changed: fn()) {
    if WATCH_STARTED.set(()).is_err() {
        return;
    }
    let _ = std::thread::Builder::new().name("audio-watch".into()).spawn(move || watch_thread(changed));
}

fn watch_thread(changed: fn()) {
    let Ok(mut child) =
        Command::new("pactl").arg("subscribe").stdout(Stdio::piped()).stderr(Stdio::null()).spawn()
    else {
        return;
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return;
    };

    let (tx, rx) = mpsc::channel::<()>();
    let _ = std::thread::Builder::new().name("audio-watch-reader".into()).spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if parse_pactl_subscribe_line(&line) && tx.send(()).is_err() {
                break;
            }
        }
    });

    while rx.recv().is_ok() {
        while rx.recv_timeout(Duration::from_millis(80)).is_ok() {}
        changed();
    }

    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_wpctl_volume_handles_normal_and_muted_lines() {
        assert_eq!(parse_wpctl_volume("Volume: 0.45\n"), Some((45, false)));
        assert_eq!(parse_wpctl_volume("Volume: 0.72 [MUTED]\n"), Some((72, true)));
        assert_eq!(parse_wpctl_volume("Volume: 1.35"), Some((100, false)));
        assert_eq!(parse_wpctl_volume("garbage"), None);
    }

    #[test]
    fn parse_pactl_volume_and_mute_extracts_percentage_and_mute() {
        let vol = "Volume: front-left: 32768 /  50% / -18.06 dB,   front-right: 32768 /  50% / -18.06 dB\n";
        assert_eq!(parse_pactl_volume_and_mute(vol, Some("Mute: no\n")), Some((50, false)));
        assert_eq!(parse_pactl_volume_and_mute(vol, Some("Mute: yes\n")), Some((50, true)));
        assert_eq!(parse_pactl_volume_and_mute("invalid output", None), None);
    }

    #[test]
    fn parse_pactl_sinks_parses_and_sorts_default_first() {
        let sample = "\
Sink #41
\tState: RUNNING
\tName: alsa_output.pci-0000_00_1f.3.analog-stereo
\tDescription: Built-in Audio Analog Stereo
\tVolume: front-left: 40000 /  61%

Sink #52
\tState: IDLE
\tName: bluez_output.AA_BB_CC_DD_EE_FF.1
\tDescription: Pixel Buds Pro
";
        let devices = parse_pactl_sinks(sample, Some("bluez_output.AA_BB_CC_DD_EE_FF.1"));
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].id, "bluez_output.AA_BB_CC_DD_EE_FF.1");
        assert_eq!(devices[0].name, "Pixel Buds Pro");
        assert!(devices[0].is_default);
        assert_eq!(devices[1].id, "alsa_output.pci-0000_00_1f.3.analog-stereo");
        assert_eq!(devices[1].name, "Built-in Audio Analog Stereo");
        assert!(!devices[1].is_default);
    }

    #[test]
    fn parse_pactl_subscribe_line_filters_relevant_events() {
        assert!(parse_pactl_subscribe_line("Event 'change' on sink #0"));
        assert!(parse_pactl_subscribe_line("Event 'change' on source #2"));
        assert!(parse_pactl_subscribe_line("Event 'change' on server #0"));
        assert!(!parse_pactl_subscribe_line("Event 'change' on sink-input #44"));
        assert!(!parse_pactl_subscribe_line("Event 'change' on source-output #12"));
        assert!(!parse_pactl_subscribe_line("Event 'new' on client #99"));
    }

    #[test]
    fn volume_stepping_increments_by_two_percent_and_unmutes_on_up() {
        let _ = set_speaker_volume(40);
        let _ = set_speaker_mute(true);
        let (vol_up, muted_after_up) = speaker_volume_up().unwrap();
        assert!(!muted_after_up);
        assert!(vol_up <= 100);

        let (vol_down, _) = speaker_volume_down().unwrap();
        assert!(vol_down <= 100);
    }
}
