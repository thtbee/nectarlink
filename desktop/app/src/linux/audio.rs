use std::process::Command;
use std::sync::Mutex;

use nectarlink_core::AudioOutputDevice;

static FALLBACK_SPEAKER: Mutex<(u8, bool)> = Mutex::new((50, false));
static FALLBACK_MIC_MUTED: Mutex<bool> = Mutex::new(false);

pub fn speaker_state() -> Option<(u8, bool)> {
    if let Ok(out) = Command::new("wpctl")
        .args(["get-volume", "@DEFAULT_AUDIO_SINK@"])
        .output()
        && out.status.success()
    {
        let text = String::from_utf8_lossy(&out.stdout);
        // e.g. "Volume: 0.45" or "Volume: 0.45 [MUTED]"
        let muted = text.contains("[MUTED]");
        if let Some(vol_str) = text.split_whitespace().nth(1)
            && let Ok(val) = vol_str.parse::<f32>()
        {
            let pct = (val * 100.0).round().clamp(0.0, 100.0) as u8;
            return Some((pct, muted));
        }
    }
    Some(*FALLBACK_SPEAKER.lock().ok()?)
}

pub fn set_speaker_volume(level: u8) -> Result<(), String> {
    let clamped = level.min(100);
    if let Ok(mut guard) = FALLBACK_SPEAKER.lock() {
        guard.0 = clamped;
    }
    let arg = format!("{clamped}%");
    let _ = Command::new("wpctl")
        .args(["set-volume", "@DEFAULT_AUDIO_SINK@", &arg])
        .status()
        .or_else(|_| {
            Command::new("pactl")
                .args(["set-sink-volume", "@DEFAULT_SINK@", &arg])
                .status()
        });
    Ok(())
}

pub fn set_speaker_mute(muted: bool) -> Result<(), String> {
    if let Ok(mut guard) = FALLBACK_SPEAKER.lock() {
        guard.1 = muted;
    }
    let flag = if muted { "1" } else { "0" };
    let _ = Command::new("wpctl")
        .args(["set-mute", "@DEFAULT_AUDIO_SINK@", flag])
        .status()
        .or_else(|_| {
            Command::new("pactl")
                .args(["set-sink-mute", "@DEFAULT_SINK@", flag])
                .status()
        });
    Ok(())
}

pub fn speaker_volume_up() -> Result<(u8, bool), String> {
    let (current, muted) = speaker_state().unwrap_or((50, false));
    let next = current.saturating_add(5).min(100);
    set_speaker_volume(next)?;
    Ok((next, muted))
}

pub fn speaker_volume_down() -> Result<(u8, bool), String> {
    let (current, muted) = speaker_state().unwrap_or((50, false));
    let next = current.saturating_sub(5);
    set_speaker_volume(next)?;
    Ok((next, muted))
}

pub fn toggle_speaker_mute() -> Result<bool, String> {
    let (_, muted) = speaker_state().unwrap_or((50, false));
    let next = !muted;
    set_speaker_mute(next)?;
    Ok(next)
}

pub fn mic_muted() -> Option<bool> {
    if let Ok(out) = Command::new("wpctl")
        .args(["get-volume", "@DEFAULT_AUDIO_SOURCE@"])
        .output()
        && out.status.success()
    {
        let text = String::from_utf8_lossy(&out.stdout);
        return Some(text.contains("[MUTED]"));
    }
    Some(*FALLBACK_MIC_MUTED.lock().ok()?)
}

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
        .or_else(|_| {
            Command::new("pactl")
                .args(["set-source-mute", "@DEFAULT_SOURCE@", flag])
                .status()
        });
    Ok(next)
}

pub fn output_devices() -> Vec<AudioOutputDevice> {
    vec![AudioOutputDevice {
        id: "default".to_string(),
        name: "Default Output".to_string(),
        is_default: true,
    }]
}

pub fn watch(_changed: fn()) {}
