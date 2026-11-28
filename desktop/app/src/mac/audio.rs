// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS speaker volume, mute, and state via CoreAudio (`AudioToolbox`
//! virtual main volume for sub-millisecond updates, with `osascript` fallback).

#![allow(unsafe_code)]

use std::{ffi::c_void, mem::size_of, ptr};

type OSStatus = i32;
type AudioObjectID = u32;

const K_AUDIO_OBJECT_SYSTEM_OBJECT: AudioObjectID = 1;
const K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN: u32 = 0;

const K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL: u32 = u32::from_be_bytes(*b"glob");
const K_AUDIO_OBJECT_PROPERTY_SCOPE_OUTPUT: u32 = u32::from_be_bytes(*b"outp");
const K_AUDIO_HARDWARE_PROPERTY_DEFAULT_OUTPUT_DEVICE: u32 = u32::from_be_bytes(*b"dOut");
const K_AUDIO_DEVICE_PROPERTY_MUTE: u32 = u32::from_be_bytes(*b"mute");
const K_AUDIO_HARDWARE_SERVICE_VIRTUAL_MAIN_VOLUME: u32 = u32::from_be_bytes(*b"vmvc");

#[repr(C)]
#[derive(Clone, Copy)]
struct AudioObjectPropertyAddress {
    m_selector: u32,
    m_scope: u32,
    m_element: u32,
}

#[link(name = "CoreAudio", kind = "framework")]
unsafe extern "C" {
    fn AudioObjectGetPropertyDataSize(
        in_object_id: AudioObjectID,
        in_address: *const AudioObjectPropertyAddress,
        in_qualifier_data_size: u32,
        in_qualifier_data: *const c_void,
        out_data_size: *mut u32,
    ) -> OSStatus;
    fn AudioObjectGetPropertyData(
        in_object_id: AudioObjectID,
        in_address: *const AudioObjectPropertyAddress,
        in_qualifier_data_size: u32,
        in_qualifier_data: *const c_void,
        io_data_size: *mut u32,
        out_data: *mut c_void,
    ) -> OSStatus;
    fn AudioObjectSetPropertyData(
        in_object_id: AudioObjectID,
        in_address: *const AudioObjectPropertyAddress,
        in_qualifier_data_size: u32,
        in_qualifier_data: *const c_void,
        in_data_size: u32,
        in_data: *const c_void,
    ) -> OSStatus;
}

#[link(name = "AudioToolbox", kind = "framework")]
unsafe extern "C" {
    fn AudioHardwareServiceGetPropertyData(
        in_object_id: AudioObjectID,
        in_address: *const AudioObjectPropertyAddress,
        in_qualifier_data_size: u32,
        in_qualifier_data: *const c_void,
        io_data_size: *mut u32,
        out_data: *mut c_void,
    ) -> OSStatus;
    fn AudioHardwareServiceSetPropertyData(
        in_object_id: AudioObjectID,
        in_address: *const AudioObjectPropertyAddress,
        in_qualifier_data_size: u32,
        in_qualifier_data: *const c_void,
        in_data_size: u32,
        in_data: *const c_void,
    ) -> OSStatus;
}

fn default_output_device() -> Option<AudioObjectID> {
    let addr = AudioObjectPropertyAddress {
        m_selector: K_AUDIO_HARDWARE_PROPERTY_DEFAULT_OUTPUT_DEVICE,
        m_scope: K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL,
        m_element: K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN,
    };
    let mut device: AudioObjectID = 0;
    let mut size = size_of::<AudioObjectID>() as u32;
    // SAFETY: stack-allocated `AudioObjectID` matching `size`.
    let status = unsafe {
        AudioObjectGetPropertyData(
            K_AUDIO_OBJECT_SYSTEM_OBJECT,
            &addr,
            0,
            ptr::null(),
            &mut size,
            (&raw mut device).cast::<c_void>(),
        )
    };
    if status == 0 && device != 0 { Some(device) } else { None }
}

fn coreaudio_speaker_state() -> Option<(u8, bool)> {
    let dev = default_output_device()?;
    let vol_addr = AudioObjectPropertyAddress {
        m_selector: K_AUDIO_HARDWARE_SERVICE_VIRTUAL_MAIN_VOLUME,
        m_scope: K_AUDIO_OBJECT_PROPERTY_SCOPE_OUTPUT,
        m_element: K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN,
    };
    let mut scalar: f32 = 0.0;
    let mut vol_size = size_of::<f32>() as u32;
    // SAFETY: stack-allocated `f32` matching `vol_size`.
    let vol_status = unsafe {
        AudioHardwareServiceGetPropertyData(
            dev,
            &vol_addr,
            0,
            ptr::null(),
            &mut vol_size,
            (&raw mut scalar).cast::<c_void>(),
        )
    };
    if vol_status != 0 {
        return None;
    }

    let mute_addr = AudioObjectPropertyAddress {
        m_selector: K_AUDIO_DEVICE_PROPERTY_MUTE,
        m_scope: K_AUDIO_OBJECT_PROPERTY_SCOPE_OUTPUT,
        m_element: K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN,
    };
    let mut muted: u32 = 0;
    let mut mute_size = size_of::<u32>() as u32;
    // SAFETY: stack-allocated `u32` matching `mute_size`.
    let mute_status = unsafe {
        AudioObjectGetPropertyData(
            dev,
            &mute_addr,
            0,
            ptr::null(),
            &mut mute_size,
            (&raw mut muted).cast::<c_void>(),
        )
    };
    let vol = (scalar.clamp(0.0, 1.0) * 100.0).round() as u8;
    Some((vol, mute_status == 0 && muted != 0))
}

fn coreaudio_set_volume(level: u8) -> Result<(), String> {
    let dev = default_output_device().ok_or_else(|| "no default output device".to_string())?;
    let addr = AudioObjectPropertyAddress {
        m_selector: K_AUDIO_HARDWARE_SERVICE_VIRTUAL_MAIN_VOLUME,
        m_scope: K_AUDIO_OBJECT_PROPERTY_SCOPE_OUTPUT,
        m_element: K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN,
    };
    let scalar: f32 = f32::from(level.min(100)) / 100.0;
    // SAFETY: `scalar` is a valid stack `f32` of size `size_of::<f32>()`.
    let status = unsafe {
        AudioHardwareServiceSetPropertyData(
            dev,
            &addr,
            0,
            ptr::null(),
            size_of::<f32>() as u32,
            (&raw const scalar).cast::<c_void>(),
        )
    };
    if status == 0 { Ok(()) } else { Err(format!("CoreAudio volume failed (OSStatus {status})")) }
}

fn coreaudio_set_mute(muted: bool) -> Result<(), String> {
    let dev = default_output_device().ok_or_else(|| "no default output device".to_string())?;
    let addr = AudioObjectPropertyAddress {
        m_selector: K_AUDIO_DEVICE_PROPERTY_MUTE,
        m_scope: K_AUDIO_OBJECT_PROPERTY_SCOPE_OUTPUT,
        m_element: K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN,
    };
    let val: u32 = u32::from(muted);
    // SAFETY: `val` is a valid stack `u32` of size `size_of::<u32>()`.
    let status = unsafe {
        AudioObjectSetPropertyData(
            dev,
            &addr,
            0,
            ptr::null(),
            size_of::<u32>() as u32,
            (&raw const val).cast::<c_void>(),
        )
    };
    if status == 0 { Ok(()) } else { Err(format!("CoreAudio mute failed (OSStatus {status})")) }
}

fn parse_speaker_state(text: &str) -> Option<(u8, bool)> {
    let mut parts = text.split_whitespace();
    let vol: u8 = parts.next()?.parse().ok()?;
    let muted: bool = parts.next()?.parse().unwrap_or(false);
    Some((vol, muted))
}

pub fn speaker_state() -> Option<(u8, bool)> {
    if let Some(state) = coreaudio_speaker_state() {
        return Some(state);
    }
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
    if coreaudio_set_volume(level).is_ok() {
        return Ok(());
    }
    run_osascript(&format!("set volume output volume {level}"))
}

pub fn set_speaker_mute(muted: bool) -> Result<(), String> {
    if coreaudio_set_mute(muted).is_ok() {
        return Ok(());
    }
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

const K_AUDIO_OBJECT_PROPERTY_SCOPE_INPUT: u32 = u32::from_be_bytes(*b"inpt");
const K_AUDIO_HARDWARE_PROPERTY_DEVICES: u32 = u32::from_be_bytes(*b"dev#");
const K_AUDIO_HARDWARE_PROPERTY_DEFAULT_INPUT_DEVICE: u32 = u32::from_be_bytes(*b"dIn ");
const K_AUDIO_DEVICE_PROPERTY_STREAM_CONFIGURATION: u32 = u32::from_be_bytes(*b"slay");
const K_AUDIO_DEVICE_PROPERTY_DEVICE_UID: u32 = u32::from_be_bytes(*b"uid ");
const K_AUDIO_OBJECT_PROPERTY_NAME: u32 = u32::from_be_bytes(*b"lnam");
const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

type CFStringRef = *const c_void;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringGetCString(string: CFStringRef, buffer: *mut u8, buffer_size: isize, encoding: u32) -> u8;
    fn CFStringGetMaximumSizeForEncoding(length: isize, encoding: u32) -> isize;
    fn CFStringGetLength(string: CFStringRef) -> isize;
    fn CFStringCreateWithCString(
        alloc: *const c_void,
        c_str: *const std::ffi::c_char,
        encoding: u32,
    ) -> *const c_void;
    fn CFRelease(object: *const c_void);
}

/// A CoreFoundation string, released when dropped.
pub(super) struct CfString(CFStringRef);

impl CfString {
    pub(super) fn new(text: &str) -> Option<CfString> {
        let c = std::ffi::CString::new(text).ok()?;
        // SAFETY: `c` is a NUL-terminated string that outlives the call.
        let string =
            unsafe { CFStringCreateWithCString(ptr::null(), c.as_ptr().cast(), K_CF_STRING_ENCODING_UTF8) };
        (!string.is_null()).then_some(CfString(string))
    }

    /// Takes a string CoreAudio handed over (it follows the "Get" rule for
    /// properties: the caller releases it).
    fn owned(string: CFStringRef) -> Option<CfString> {
        (!string.is_null()).then_some(CfString(string))
    }

    pub(super) fn as_raw(&self) -> CFStringRef {
        self.0
    }

    fn to_text(&self) -> Option<String> {
        // SAFETY: `self.0` is a live CFString.
        let size = unsafe {
            CFStringGetMaximumSizeForEncoding(CFStringGetLength(self.0), K_CF_STRING_ENCODING_UTF8)
        } + 1;
        let mut buf = vec![0u8; usize::try_from(size).ok()?];
        // SAFETY: `buf` holds `size` bytes.
        if unsafe { CFStringGetCString(self.0, buf.as_mut_ptr(), size, K_CF_STRING_ENCODING_UTF8) } == 0 {
            return None;
        }
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        String::from_utf8(buf[..end].to_vec()).ok()
    }
}

impl Drop for CfString {
    fn drop(&mut self) {
        // SAFETY: a string this wrapper owns one reference to.
        unsafe { CFRelease(self.0) };
    }
}

fn device_string(device: AudioObjectID, selector: u32) -> Option<String> {
    let addr = AudioObjectPropertyAddress {
        m_selector: selector,
        m_scope: K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL,
        m_element: K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN,
    };
    let mut string: CFStringRef = ptr::null();
    let mut size = size_of::<CFStringRef>() as u32;
    // SAFETY: a stack pointer of the size CoreAudio writes for a string property.
    let status = unsafe {
        AudioObjectGetPropertyData(
            device,
            &addr,
            0,
            ptr::null(),
            &mut size,
            (&raw mut string).cast::<c_void>(),
        )
    };
    if status != 0 {
        return None;
    }
    CfString::owned(string)?.to_text()
}

/// How many input channels `device` has (none for a speaker).
fn input_channels(device: AudioObjectID) -> u32 {
    let addr = AudioObjectPropertyAddress {
        m_selector: K_AUDIO_DEVICE_PROPERTY_STREAM_CONFIGURATION,
        m_scope: K_AUDIO_OBJECT_PROPERTY_SCOPE_INPUT,
        m_element: K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN,
    };
    let mut size = 0u32;
    // SAFETY: `size` is a stack u32.
    if unsafe { AudioObjectGetPropertyDataSize(device, &addr, 0, ptr::null(), &mut size) } != 0 || size < 8 {
        return 0;
    }
    // An AudioBufferList: a count, then that many { channels, byte size, data }.
    // A u64 buffer keeps it aligned for the pointers inside.
    let mut list = vec![0u64; (size as usize).div_ceil(8)];
    // SAFETY: `list` has at least `size` bytes.
    if unsafe {
        AudioObjectGetPropertyData(device, &addr, 0, ptr::null(), &mut size, list.as_mut_ptr().cast())
    } != 0
    {
        return 0;
    }
    let bytes: &[u8] = {
        // SAFETY: viewing the same allocation as bytes.
        unsafe { std::slice::from_raw_parts(list.as_ptr().cast::<u8>(), (size as usize).min(list.len() * 8)) }
    };
    let word =
        |at: usize| bytes.get(at..at + 4).map(|b| u32::from_ne_bytes(b.try_into().unwrap_or_default()));
    let count = word(0).unwrap_or(0) as usize;
    (0..count).filter_map(|i| word(8 + i * 16)).sum()
}

/// The device the system records from by default.
pub(super) fn default_input_device() -> Option<AudioObjectID> {
    let addr = AudioObjectPropertyAddress {
        m_selector: K_AUDIO_HARDWARE_PROPERTY_DEFAULT_INPUT_DEVICE,
        m_scope: K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL,
        m_element: K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN,
    };
    let mut device: AudioObjectID = 0;
    let mut size = size_of::<AudioObjectID>() as u32;
    // SAFETY: a stack `AudioObjectID` of the size given.
    let status = unsafe {
        AudioObjectGetPropertyData(
            K_AUDIO_OBJECT_SYSTEM_OBJECT,
            &addr,
            0,
            ptr::null(),
            &mut size,
            (&raw mut device).cast::<c_void>(),
        )
    };
    (status == 0 && device != 0).then_some(device)
}

/// The Mac's microphones: each one's ID is its CoreAudio UID, which stays the
/// same across restarts and replugging, and the one the system records from by
/// default is marked.
pub fn input_devices() -> Vec<nectarlink_core::AudioOutputDevice> {
    let addr = AudioObjectPropertyAddress {
        m_selector: K_AUDIO_HARDWARE_PROPERTY_DEVICES,
        m_scope: K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL,
        m_element: K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN,
    };
    let mut size = 0u32;
    // SAFETY: `size` is a stack u32.
    if unsafe {
        AudioObjectGetPropertyDataSize(K_AUDIO_OBJECT_SYSTEM_OBJECT, &addr, 0, ptr::null(), &mut size)
    } != 0
    {
        return Vec::new();
    }
    let mut ids = vec![0 as AudioObjectID; size as usize / size_of::<AudioObjectID>()];
    // SAFETY: `ids` holds the `size` bytes.
    let status = unsafe {
        AudioObjectGetPropertyData(
            K_AUDIO_OBJECT_SYSTEM_OBJECT,
            &addr,
            0,
            ptr::null(),
            &mut size,
            ids.as_mut_ptr().cast::<c_void>(),
        )
    };
    if status != 0 {
        return Vec::new();
    }
    let default = default_input_device();
    ids.into_iter()
        .filter(|&device| input_channels(device) > 0)
        .filter_map(|device| {
            let id = device_string(device, K_AUDIO_DEVICE_PROPERTY_DEVICE_UID)?;
            let name = device_string(device, K_AUDIO_OBJECT_PROPERTY_NAME).unwrap_or_else(|| id.clone());
            Some(nectarlink_core::AudioOutputDevice { id, name, is_default: Some(device) == default })
        })
        .collect()
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

    #[test]
    fn coreaudio_reads_speaker_state_when_device_present() {
        if default_output_device().is_some()
            && let Some((vol, _muted)) = coreaudio_speaker_state()
        {
            assert!(vol <= 100);
        }
    }
}
