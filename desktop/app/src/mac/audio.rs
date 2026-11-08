// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS speaker volume, mute, and state.

fn parse_speaker_state(text: &str) -> Option<(u8, bool)> {
    let mut parts = text.split_whitespace();
    let vol: u8 = parts.next()?.parse().ok()?;
    let muted: bool = parts.next()?.parse().unwrap_or(false);
    Some((vol, muted))
}

pub fn speaker_state() -> Option<(u8, bool)> {
    let output = std::process::Command::new("osascript")
        .args([
            "-e",
            "set v to get volume settings",
            "-e",
            "(output volume of v as string) & \" \" & (output muted of v as string)",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_speaker_state(&String::from_utf8_lossy(&output.stdout))
}

fn run_osascript(script: &str) -> Result<(), String> {
    let output =
        std::process::Command::new("osascript").args(["-e", script]).output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let msg = stderr.trim();
        return Err(if msg.is_empty() {
            format!("osascript failed with exit code {:?}", output.status.code())
        } else {
            msg.to_string()
        });
    }
    Ok(())
}

pub fn set_speaker_volume(level: u8) -> Result<(), String> {
    run_osascript(&format!("set volume output volume {level}"))
}

pub fn set_speaker_mute(muted: bool) -> Result<(), String> {
    run_osascript(&format!("set volume output muted {muted}"))
}

pub fn toggle_speaker_mute() -> Result<bool, String> {
    let (_, muted) = speaker_state().unwrap_or((50, false));
    let next = !muted;
    set_speaker_mute(next)?;
    Ok(next)
}

pub fn speaker_volume_up() -> Result<(u8, bool), String> {
    let (vol, muted) = speaker_state().unwrap_or((50, false));
    let next = vol.saturating_add(5).min(100);
    set_speaker_volume(next)?;
    Ok((next, muted))
}

pub fn speaker_volume_down() -> Result<(u8, bool), String> {
    let (vol, muted) = speaker_state().unwrap_or((50, false));
    let next = vol.saturating_sub(5);
    set_speaker_volume(next)?;
    Ok((next, muted))
}

static LAST_MIC_VOL: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(75);

pub fn mic_volume() -> Option<u8> {
    let output = std::process::Command::new("osascript")
        .args(["-e", "input volume of (get volume settings)"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.trim().parse().ok()
}

pub fn mic_muted() -> Option<bool> {
    mic_volume().map(|vol| vol == 0)
}

pub fn toggle_mic_mute() -> Result<bool, String> {
    let current_vol = mic_volume().ok_or_else(|| "Couldn't read microphone volume".to_string())?;
    let muted = current_vol == 0;
    let next = !muted;
    let vol = if next {
        if current_vol > 0 {
            LAST_MIC_VOL.store(current_vol, std::sync::atomic::Ordering::Relaxed);
        }
        0
    } else {
        let saved = LAST_MIC_VOL.load(std::sync::atomic::Ordering::Relaxed);
        if saved > 0 { saved } else { 75 }
    };
    let script = format!("set volume input volume {vol}");
    run_osascript(&script)?;
    Ok(next)
}

pub fn output_devices() -> Vec<nectarlink_core::AudioOutputDevice> {
    vec![nectarlink_core::AudioOutputDevice {
        id: "default".into(),
        name: "Default macOS Output".into(),
        is_default: true,
    }]
}

/// Lists only the system's default output (see [`output_devices`]), so there's
/// nothing to switch to yet.
pub fn set_default_output_device(_id: &str) -> Result<(), String> {
    Err("switching the audio output isn't supported on macOS yet".into())
}

pub fn watch(_changed: fn()) {}

#[allow(dead_code)]
pub fn start(_on_changed: impl Fn() + Send + Sync + 'static) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speaker_state_parses_volume_and_mute() {
        assert_eq!(parse_speaker_state("25 false"), Some((25, false)));
        assert_eq!(parse_speaker_state("100 true\n"), Some((100, true)));
        assert_eq!(parse_speaker_state("0 false"), Some((0, false)));
        assert_eq!(parse_speaker_state(""), None);
        assert_eq!(parse_speaker_state("invalid"), None);
    }

    #[test]
    fn query_live_speaker_state() {
        if let Some((vol, _muted)) = speaker_state() {
            assert!(vol <= 100);
        }
    }
}
