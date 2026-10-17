// SPDX-License-Identifier: GPL-3.0-or-later
//! Phone-as-Microphone virtual audio source on Linux via PulseAudio / PipeWire
//! `module-pipe-source`, including FIFO creation (`mkfifo`), non-blocking PCM
//! writes, gain/mute processing, and RMS level metering.

use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const SOURCE_NAME: &str = "NectarLink_Virtual_Mic";
pub const DEFAULT_FIFO_PATH: &str = "/tmp/nectarlink-vmic.pcm";

pub fn is_available() -> bool {
    Command::new("pactl").arg("info").output().map(|o| o.status.success()).unwrap_or(false)
}

/// Ensures that `fifo_path` exists on disk as a Unix FIFO (`0o600`).
/// If a regular file exists at `fifo_path`, it is replaced with a FIFO.
pub fn ensure_fifo(fifo_path: &Path) -> Result<(), String> {
    if let Some(parent) = fifo_path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| format!("create FIFO parent dir: {e}"))?;
    }

    if let Ok(meta) = std::fs::symlink_metadata(fifo_path) {
        if meta.file_type().is_fifo() {
            return Ok(());
        }
        std::fs::remove_file(fifo_path)
            .map_err(|e| format!("remove non-FIFO at {}: {e}", fifo_path.display()))?;
    }

    let c_path =
        CString::new(fifo_path.as_os_str().as_bytes()).map_err(|e| format!("invalid FIFO path: {e}"))?;
    // SAFETY: `c_path` is a valid NUL-terminated path string.
    let rc = unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        if err.kind() == ErrorKind::AlreadyExists
            && std::fs::symlink_metadata(fifo_path).map(|m| m.file_type().is_fifo()).unwrap_or(false)
        {
            return Ok(());
        }
        return Err(format!("mkfifo({}): {err}", fifo_path.display()));
    }
    Ok(())
}

/// Opens `fifo_path` in non-blocking read-write mode (`O_RDWR | O_NONBLOCK`) so opening and
/// writing never block when no PulseAudio/PipeWire reader is currently attached.
pub fn open_fifo_nonblocking(fifo_path: &Path) -> std::io::Result<File> {
    OpenOptions::new().read(true).write(true).custom_flags(libc::O_NONBLOCK).open(fifo_path)
}

pub fn create_source(rate: u32, channels: u8) -> Result<u32, String> {
    create_source_with_path(rate, channels, Path::new(DEFAULT_FIFO_PATH))
}

pub fn create_source_with_path(rate: u32, channels: u8, fifo_path: &Path) -> Result<u32, String> {
    let _ = ensure_fifo(fifo_path);
    let output = Command::new("pactl")
        .args([
            "load-module",
            "module-pipe-source",
            &format!("source_name={SOURCE_NAME}"),
            &format!("file={}", fifo_path.display()),
            "format=s16le",
            &format!("rate={}", rate.max(8_000)),
            &format!("channels={}", channels.max(1)),
            "source_properties=device.description=NectarLink_Phone_Microphone",
        ])
        .output()
        .map_err(|e| format!("failed to run pactl: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u32>()
        .map_err(|e| format!("invalid pactl module id: {e}"))
}

pub fn destroy_source(module_id: u32) {
    let _ = Command::new("pactl").args(["unload-module", &module_id.to_string()]).status();
}

/// Applies `gain_db` (`-12.0..=24.0`) and `muted` to 16-bit signed little-endian
/// PCM samples, returning `(processed_pcm_bytes, rms_level_0_to_1)`.
pub fn process_mic_pcm_s16le(pcm: &[u8], gain_db: f32, muted: bool) -> (Vec<u8>, f32) {
    let sample_count = pcm.len() / 2;
    if sample_count == 0 {
        return (Vec::new(), 0.0);
    }
    if muted {
        return (vec![0u8; sample_count * 2], 0.0);
    }

    let clamped_db = if gain_db.is_finite() { gain_db.clamp(-12.0, 24.0) } else { 0.0 };
    let linear_gain = 10.0_f32.powf(clamped_db / 20.0);
    let mut out = Vec::with_capacity(sample_count * 2);
    let mut sum_sq = 0.0_f64;

    for chunk in pcm.as_chunks::<2>().0 {
        let raw = i16::from_le_bytes([chunk[0], chunk[1]]);
        let norm = (f32::from(raw) / 32768.0) * linear_gain;
        // Soft-knee saturation above 0.9 to prevent harsh digital clipping.
        let limited = if norm.abs() > 0.9 {
            let sign = norm.signum();
            let excess = norm.abs() - 0.9;
            sign * (0.9 + 0.1 * (excess / (0.1 + excess)))
        } else {
            norm
        }
        .clamp(-1.0, 1.0);

        sum_sq += f64::from(limited) * f64::from(limited);
        let scaled = (limited * 32767.0).round() as i16;
        out.extend_from_slice(&scaled.to_le_bytes());
    }

    let rms = ((sum_sq / (sample_count as f64)).sqrt() as f32).clamp(0.0, 1.0);
    (out, rms)
}

/// Manages a PipeWire / PulseAudio virtual microphone pipe source and its
/// real-time gain/mute/RMS state.
#[derive(Debug)]
pub struct VirtualMic {
    fifo_path: PathBuf,
    fifo_file: Option<File>,
    module_id: Option<u32>,
    gain_db: f32,
    muted: bool,
    last_rms: f32,
}

impl VirtualMic {
    /// Creates a new `VirtualMic` controller and attempts to register the
    /// underlying `module-pipe-source` in PulseAudio/PipeWire.
    pub fn new(rate: u32, channels: u8) -> Self {
        let fifo_path = PathBuf::from(DEFAULT_FIFO_PATH);
        let _ = ensure_fifo(&fifo_path);
        let fifo_file = open_fifo_nonblocking(&fifo_path).ok();
        let module_id = create_source_with_path(rate, channels, &fifo_path).ok();
        Self { fifo_path, fifo_file, module_id, gain_db: 0.0, muted: false, last_rms: 0.0 }
    }

    /// Creates a `VirtualMic` backed by a specific FIFO path (creating the FIFO if needed)
    /// without invoking `pactl`.
    pub fn open_fifo_only(fifo_path: impl Into<PathBuf>) -> Result<Self, String> {
        let fifo_path = fifo_path.into();
        ensure_fifo(&fifo_path)?;
        let fifo_file = open_fifo_nonblocking(&fifo_path)
            .map_err(|e| format!("open FIFO {}: {e}", fifo_path.display()))?;
        Ok(Self {
            fifo_path,
            fifo_file: Some(fifo_file),
            module_id: None,
            gain_db: 0.0,
            muted: false,
            last_rms: 0.0,
        })
    }

    pub fn is_active(&self) -> bool {
        self.module_id.is_some()
    }

    pub fn fifo_path(&self) -> &Path {
        &self.fifo_path
    }

    pub fn set_gain_db(&mut self, gain_db: f32) {
        self.gain_db = if gain_db.is_finite() { gain_db.clamp(-12.0, 24.0) } else { 0.0 };
    }

    pub fn gain_db(&self) -> f32 {
        self.gain_db
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        if muted {
            self.last_rms = 0.0;
        }
    }

    pub fn is_muted(&self) -> bool {
        self.muted
    }

    pub fn last_rms(&self) -> f32 {
        self.last_rms
    }

    /// Writes already-processed or raw `s16le` bytes into the underlying non-blocking FIFO.
    /// Returns the number of bytes written (`0` if the kernel pipe buffer is currently full).
    pub fn write_pcm_s16le(&mut self, pcm: &[u8]) -> Result<usize, String> {
        if pcm.is_empty() {
            return Ok(0);
        }
        if self.fifo_file.is_none() {
            ensure_fifo(&self.fifo_path)?;
            self.fifo_file = open_fifo_nonblocking(&self.fifo_path).ok();
        }
        let Some(file) = self.fifo_file.as_mut() else {
            return Err(format!("FIFO {} is not open", self.fifo_path.display()));
        };
        match file.write(pcm) {
            Ok(n) => Ok(n),
            Err(e) if e.kind() == ErrorKind::WouldBlock => Ok(0),
            Err(e) => {
                self.fifo_file = None;
                Err(format!("FIFO write error: {e}"))
            }
        }
    }

    /// Processes incoming `s16le` PCM frames and writes them to the pipe FIFO
    /// if the virtual source or FIFO handle is active.
    pub fn push_pcm(&mut self, pcm: &[u8]) -> Result<f32, String> {
        let (processed, rms) = process_mic_pcm_s16le(pcm, self.gain_db, self.muted);
        self.last_rms = rms;
        if (self.module_id.is_some() || self.fifo_file.is_some()) && !processed.is_empty() {
            let _ = self.write_pcm_s16le(&processed);
        }
        Ok(rms)
    }
}

impl Drop for VirtualMic {
    fn drop(&mut self) {
        self.fifo_file = None;
        if let Some(id) = self.module_id.take() {
            destroy_source(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn process_mic_pcm_s16le_unity_gain_preserves_signal_and_computes_rms() {
        let mut raw = Vec::new();
        for _ in 0..100 {
            raw.extend_from_slice(&16384i16.to_le_bytes());
        }
        let (out, rms) = process_mic_pcm_s16le(&raw, 0.0, false);
        assert_eq!(out.len(), raw.len());
        let first = i16::from_le_bytes([out[0], out[1]]);
        assert!((first - 16384).abs() <= 2);
        assert!(rms > 0.45 && rms < 0.55, "expected ~0.5 RMS, got {rms}");
    }

    #[test]
    fn process_mic_pcm_s16le_mute_zeroes_samples_and_rms() {
        let mut raw = Vec::new();
        for _ in 0..64 {
            raw.extend_from_slice(&20000i16.to_le_bytes());
        }
        let (out, rms) = process_mic_pcm_s16le(&raw, 12.0, true);
        assert!(out.iter().all(|&b| b == 0));
        assert_eq!(rms, 0.0);
    }

    #[test]
    fn process_mic_pcm_s16le_boosts_and_soft_limits_peaks() {
        let mut raw = Vec::new();
        raw.extend_from_slice(&10000i16.to_le_bytes());
        raw.extend_from_slice(&30000i16.to_le_bytes());
        let (out, rms) = process_mic_pcm_s16le(&raw, 6.0, false);
        let s0 = i16::from_le_bytes([out[0], out[1]]);
        let s1 = i16::from_le_bytes([out[2], out[3]]);
        assert!(s0 > 18000, "+6 dB should roughly double moderate sample: {s0}");
        assert!(s1 >= 29000, "soft limiter should keep loud sample near full scale: {s1}");
        assert!(rms > 0.5 && rms <= 1.0);
    }

    #[test]
    fn ensure_fifo_creates_named_pipe_and_nonblocking_writes_succeed_without_reader() {
        let dir = tempfile::tempdir().unwrap();
        let fifo_path = dir.path().join("test-vmic.pcm");

        // Write a regular file first to verify `ensure_fifo` replaces non-FIFOs.
        std::fs::write(&fifo_path, b"stale").unwrap();
        ensure_fifo(&fifo_path).expect("ensure_fifo should replace regular file with FIFO");

        let meta = std::fs::symlink_metadata(&fifo_path).unwrap();
        assert!(meta.file_type().is_fifo(), "path must be a Unix FIFO");
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);

        let mut vmic = VirtualMic::open_fifo_only(&fifo_path).expect("should open FIFO non-blocking");
        let mut raw = Vec::new();
        for _ in 0..32 {
            raw.extend_from_slice(&8192i16.to_le_bytes());
        }
        let rms = vmic.push_pcm(&raw).expect("push_pcm should not block without reader");
        assert!(rms > 0.2 && rms < 0.3, "expected ~0.25 RMS, got {rms}");

        let mut reader = open_fifo_nonblocking(&fifo_path).unwrap();
        let mut buf = vec![0u8; raw.len()];
        let n = reader.read(&mut buf).expect("should read pushed PCM bytes from FIFO");
        assert_eq!(n, raw.len());
    }
}
