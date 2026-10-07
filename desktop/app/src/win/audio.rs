// SPDX-License-Identifier: GPL-3.0-or-later
//! Speaker volume and microphone mute control and state via Windows Core Audio
//! (`IMMDeviceEnumerator` + `IAudioEndpointVolume`).

use windows::Win32::{
    Media::Audio::{
        EDataFlow, Endpoints::IAudioEndpointVolume, IMMDeviceEnumerator, MMDeviceEnumerator, eCapture,
        eConsole, eRender,
    },
    System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize},
};

struct ComGuard(bool);

impl ComGuard {
    fn new() -> Self {
        // SAFETY: Initializes COM on the current thread if not already initialized.
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        Self(hr.is_ok())
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: Matches a successful `CoInitializeEx` on this thread.
            unsafe { CoUninitialize() };
        }
    }
}

fn endpoint_volume(flow: EDataFlow) -> windows::core::Result<IAudioEndpointVolume> {
    // SAFETY: Standard Core Audio COM activation on the default console endpoint.
    unsafe {
        let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = enumerator.GetDefaultAudioEndpoint(flow, eConsole)?;
        device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)
    }
}

/// Reads the default speaker endpoint's `(volume_percent, muted)`.
pub fn speaker_state() -> Option<(u8, bool)> {
    let _com = ComGuard::new();
    let ep = endpoint_volume(eRender).ok()?;
    // SAFETY: `ep` is a valid activated `IAudioEndpointVolume`.
    unsafe {
        let scalar = ep.GetMasterVolumeLevelScalar().ok()?;
        let muted = ep.GetMute().ok()?.as_bool();
        let pct = (scalar * 100.0).round().clamp(0.0, 100.0) as u8;
        Some((pct, muted))
    }
}

/// Steps the default speaker volume up by one increment (2%) and unmutes if muted.
pub fn speaker_volume_up() -> Result<(u8, bool), String> {
    let _com = ComGuard::new();
    let ep = endpoint_volume(eRender).map_err(|e| format!("no speaker device: {e}"))?;
    // SAFETY: `ep` is a valid activated `IAudioEndpointVolume`.
    unsafe {
        ep.VolumeStepUp(std::ptr::null()).map_err(|e| e.to_string())?;
        if ep.GetMute().map(|b| b.as_bool()).unwrap_or(false) {
            let _ = ep.SetMute(false, std::ptr::null());
        }
        let scalar = ep.GetMasterVolumeLevelScalar().unwrap_or(0.0);
        let muted = ep.GetMute().map(|b| b.as_bool()).unwrap_or(false);
        Ok(((scalar * 100.0).round().clamp(0.0, 100.0) as u8, muted))
    }
}

/// Steps the default speaker volume down by one increment (2%).
pub fn speaker_volume_down() -> Result<(u8, bool), String> {
    let _com = ComGuard::new();
    let ep = endpoint_volume(eRender).map_err(|e| format!("no speaker device: {e}"))?;
    // SAFETY: `ep` is a valid activated `IAudioEndpointVolume`.
    unsafe {
        ep.VolumeStepDown(std::ptr::null()).map_err(|e| e.to_string())?;
        let scalar = ep.GetMasterVolumeLevelScalar().unwrap_or(0.0);
        let muted = ep.GetMute().map(|b| b.as_bool()).unwrap_or(false);
        Ok(((scalar * 100.0).round().clamp(0.0, 100.0) as u8, muted))
    }
}

/// Toggles mute on the default speaker endpoint and returns the new muted state.
pub fn toggle_speaker_mute() -> Result<bool, String> {
    let _com = ComGuard::new();
    let ep = endpoint_volume(eRender).map_err(|e| format!("no speaker device: {e}"))?;
    // SAFETY: `ep` is a valid activated `IAudioEndpointVolume`.
    unsafe {
        let cur = ep.GetMute().map_err(|e| e.to_string())?.as_bool();
        let next = !cur;
        ep.SetMute(next, std::ptr::null()).map_err(|e| e.to_string())?;
        Ok(next)
    }
}

/// Reads whether the default capture (microphone) endpoint is muted, or `None`
/// if this PC has no default capture endpoint.
pub fn mic_muted() -> Option<bool> {
    let _com = ComGuard::new();
    let ep = endpoint_volume(eCapture).ok()?;
    // SAFETY: `ep` is a valid activated `IAudioEndpointVolume`.
    unsafe { Some(ep.GetMute().ok()?.as_bool()) }
}

/// Toggles mute on the default capture (microphone) endpoint and returns the
/// new muted state.
pub fn toggle_mic_mute() -> Result<bool, String> {
    let _com = ComGuard::new();
    let ep = endpoint_volume(eCapture).map_err(|e| format!("no microphone found: {e}"))?;
    // SAFETY: `ep` is a valid activated `IAudioEndpointVolume`.
    unsafe {
        let cur = ep.GetMute().map_err(|e| e.to_string())?.as_bool();
        let next = !cur;
        ep.SetMute(next, std::ptr::null()).map_err(|e| e.to_string())?;
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_speaker_and_mic_state_without_mutating() {
        if let Some((vol, _muted)) = speaker_state() {
            assert!(vol <= 100);
        }
        let _ = mic_muted();
    }
}
