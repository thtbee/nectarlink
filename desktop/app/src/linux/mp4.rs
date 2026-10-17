// SPDX-License-Identifier: GPL-3.0-or-later
//! Linux MP4 recording for mirrored phone screens and app windows.
//!
//! Video frames arrive as Annex-B H.264 access units and are muxed into an
//! ISO-BMFF (`.mp4`) container (`ftyp` + `moov` + `mdat` with `avc1`/`avcC`
//! sample entries) without re-encoding. When phone audio (16-bit PCM) is
//! present and `ffmpeg` is available on `PATH`, the recorded PCM track is
//! encoded to AAC and muxed alongside the H.264 video stream.

use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

/// Formats `<prefix> YYYY-MM-DD HH.MM.SS.<ext>` using the PC's local clock.
pub fn local_timestamp_filename(prefix: &str, ext: &str) -> String {
    let clean_ext = ext.trim_start_matches('.');
    let (year, month, day, hour, min, sec) = local_ymdhms();
    format!("{prefix} {year:04}-{month:02}-{day:02} {hour:02}.{min:02}.{sec:02}.{clean_ext}")
}

fn local_ymdhms() -> (i32, u32, u32, u32, u32, u32) {
    // SAFETY: `libc::time` and `libc::localtime_r` are thread-safe POSIX functions
    // called with stack-allocated `time_t` and `tm` structs.
    unsafe {
        let mut now: libc::time_t = 0;
        libc::time(&raw mut now);
        let mut tm: libc::tm = std::mem::zeroed();
        if !libc::localtime_r(&raw const now, &raw mut tm).is_null() {
            let year = tm.tm_year + 1900;
            let month = (tm.tm_mon + 1).clamp(1, 12) as u32;
            let day = tm.tm_mday.clamp(1, 31) as u32;
            let hour = tm.tm_hour.clamp(0, 23) as u32;
            let min = tm.tm_min.clamp(0, 59) as u32;
            let sec = tm.tm_sec.clamp(0, 60) as u32;
            return (year, month, day, hour, min, sec);
        }
    }

    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let hour = (rem / 3600) as u32;
    let min = ((rem % 3600) / 60) as u32;
    let sec = (rem % 60) as u32;
    let (year, month, day) = civil_from_days(days as i64);
    (year, month, day, hour, min, sec)
}

fn civil_from_days(days_since_epoch: i64) -> (i32, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year as i32, m, d)
}

/// Splits an Annex-B byte stream into individual NAL payloads (without start codes).
pub fn split_annexb(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if i + 4 <= data.len() && data[i..i + 4] == [0, 0, 0, 1] {
            starts.push((i, i + 4));
            i += 4;
        } else if data[i..i + 3] == [0, 0, 1] {
            starts.push((i, i + 3));
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut out = Vec::with_capacity(starts.len());
    for (idx, &(_, payload_start)) in starts.iter().enumerate() {
        let payload_end = starts.get(idx + 1).map_or(data.len(), |&(next_code, _)| next_code);
        if payload_start < payload_end {
            out.push(&data[payload_start..payload_end]);
        }
    }
    out
}

/// Returns the 5-bit H.264 NAL unit type of a NAL payload (without start code).
pub fn nal_type(nal: &[u8]) -> Option<u8> {
    nal.first().map(|b| b & 0x1F)
}

/// Returns `true` if `data` contains an H.264 IDR slice (NAL type 5).
pub fn has_idr_slice(data: &[u8]) -> bool {
    split_annexb(data).into_iter().any(|nal| nal_type(nal) == Some(5))
}

/// Returns `true` if `data` already carries both SPS (NAL 7) and PPS (NAL 8) units in-band.
pub fn has_inband_sps_pps(data: &[u8]) -> bool {
    let nals = split_annexb(data);
    nals.iter().any(|nal| nal_type(nal) == Some(7)) && nals.iter().any(|nal| nal_type(nal) == Some(8))
}

/// Returns `true` if `data` is non-empty and every NAL unit is a parameter set (SPS/PPS).
pub fn is_parameter_only(data: &[u8]) -> bool {
    let nals = split_annexb(data);
    !nals.is_empty() && nals.into_iter().all(|nal| matches!(nal_type(nal), Some(7 | 8)))
}

/// Extracts only SPS (NAL 7) and PPS (NAL 8) units from `data` with 4-byte Annex-B start codes.
pub fn extract_sps_pps(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    for nal in split_annexb(data) {
        if matches!(nal_type(nal), Some(7 | 8)) {
            out.extend_from_slice(&[0, 0, 0, 1]);
            out.extend_from_slice(nal);
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Extracts any SPS (NAL 7) and PPS (NAL 8) units from `incoming` and merges them
/// into `dest` with 4-byte `00 00 00 01` Annex-B start codes, preserving SPS-then-PPS order.
pub fn merge_sps_pps(dest: &mut Vec<u8>, incoming: &[u8]) {
    let nals = split_annexb(incoming);
    if !nals.iter().any(|nal| matches!(nal_type(nal), Some(7 | 8))) {
        return;
    }
    let existing = split_annexb(dest);
    let mut sps: Option<Vec<u8>> =
        existing.iter().find(|nal| nal_type(nal) == Some(7)).map(|nal| (*nal).to_vec());
    let mut pps: Option<Vec<u8>> =
        existing.iter().find(|nal| nal_type(nal) == Some(8)).map(|nal| (*nal).to_vec());
    for nal in nals {
        match nal_type(nal) {
            Some(7) => sps = Some(nal.to_vec()),
            Some(8) => pps = Some(nal.to_vec()),
            _ => {}
        }
    }
    dest.clear();
    if let Some(s) = sps {
        dest.extend_from_slice(&[0, 0, 0, 1]);
        dest.extend_from_slice(&s);
    }
    if let Some(p) = pps {
        dest.extend_from_slice(&[0, 0, 0, 1]);
        dest.extend_from_slice(&p);
    }
}

#[derive(Debug, Clone)]
struct RecordedFrame {
    pts_us: u64,
    keyframe: bool,
    data: Vec<u8>,
}

/// Records a mirrored H.264 stream (and optional PCM audio stream) into an `.mp4` file.
#[derive(Debug)]
pub struct MirrorRecorder {
    path: PathBuf,
    width: u32,
    height: u32,
    fps: u32,
    audio_cfg: Option<(u32, u8)>,
    sps_pps: Vec<u8>,
    started: Instant,
    started_epoch_ms: i64,
    got_keyframe: bool,
    first_pts_us: Option<u64>,
    last_pts_us: u64,
    frames: Vec<RecordedFrame>,
    audio_pcm: Vec<u8>,
}

impl MirrorRecorder {
    pub fn new(
        path: PathBuf,
        width: u32,
        height: u32,
        fps: u32,
        audio_cfg: Option<(u32, u8)>,
    ) -> MirrorRecorder {
        let started_epoch_ms =
            SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
        MirrorRecorder {
            path,
            width: width.max(1),
            height: height.max(1),
            fps: fps.max(1),
            audio_cfg,
            sps_pps: Vec::new(),
            started: Instant::now(),
            started_epoch_ms,
            got_keyframe: false,
            first_pts_us: None,
            last_pts_us: 0,
            frames: Vec::new(),
            audio_pcm: Vec::new(),
        }
    }

    pub fn started_at(&self) -> Instant {
        self.started
    }

    pub fn started_epoch_ms(&self) -> i64 {
        self.started_epoch_ms
    }

    /// Seeds cached SPS/PPS NAL units collected before recording started.
    pub fn set_sps_pps(&mut self, sps_pps: &[u8]) {
        if !sps_pps.is_empty() {
            merge_sps_pps(&mut self.sps_pps, sps_pps);
        }
    }

    /// Updates stream dimensions before the first keyframe initializes the recording.
    pub fn on_config(&mut self, width: u32, height: u32) {
        if !self.got_keyframe && width > 0 && height > 0 {
            self.width = width;
            self.height = height;
        }
    }

    /// Updates audio configuration before the first keyframe initializes the recording.
    pub fn on_audio_config(&mut self, rate: u32, channels: u8) {
        if !self.got_keyframe && rate > 0 && channels > 0 {
            self.audio_cfg = Some((rate, channels));
        }
    }

    /// Processes an incoming Annex-B H.264 access unit.
    pub fn on_packet(&mut self, keyframe: bool, time_us: u64, data: &[u8]) -> Result<(), String> {
        if data.is_empty() {
            return Ok(());
        }
        merge_sps_pps(&mut self.sps_pps, data);

        if is_parameter_only(data) {
            return Ok(());
        }

        let is_keyframe = keyframe || has_idr_slice(data);
        if !self.got_keyframe {
            if !is_keyframe {
                return Ok(());
            }
            self.got_keyframe = true;
        }

        let first_us = *self.first_pts_us.get_or_insert(time_us);
        let frame_duration_us = (1_000_000 / u64::from(self.fps.max(1))).max(1);
        let mut rel_us = time_us.saturating_sub(first_us);
        if !self.frames.is_empty() && rel_us <= self.last_pts_us {
            rel_us = self.last_pts_us + frame_duration_us;
        }
        self.last_pts_us = rel_us;

        let payload = if is_keyframe && !has_inband_sps_pps(data) && !self.sps_pps.is_empty() {
            let mut combined = Vec::with_capacity(self.sps_pps.len() + data.len());
            combined.extend_from_slice(&self.sps_pps);
            combined.extend_from_slice(data);
            combined
        } else {
            data.to_vec()
        };

        self.frames.push(RecordedFrame { pts_us: rel_us, keyframe: is_keyframe, data: payload });
        Ok(())
    }

    /// Processes an incoming 16-bit interleaved PCM audio packet.
    pub fn on_audio(&mut self, _time_us: u64, pcm: &[u8]) -> Result<(), String> {
        if self.audio_cfg.is_some() && !pcm.is_empty() {
            self.audio_pcm.extend_from_slice(pcm);
        }
        Ok(())
    }

    /// Finalizes the MP4 container and flushes it to disk.
    pub fn finish(self) -> Result<PathBuf, String> {
        if self.frames.is_empty() {
            let _ = fs::remove_file(&self.path);
            return Err("No video frames were recorded yet.".into());
        }
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        write_mp4_container(&self.path, self.width, self.height, self.fps, &self.sps_pps, &self.frames)?;

        if !self.audio_pcm.is_empty()
            && let Some((rate, channels)) = self.audio_cfg
            && rate > 0
            && channels > 0
        {
            mux_audio_with_ffmpeg(&self.path, rate, channels, &self.audio_pcm);
        }

        Ok(self.path)
    }
}

fn mux_audio_with_ffmpeg(mp4_path: &Path, rate: u32, channels: u8, pcm: &[u8]) {
    let parent = mp4_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = mp4_path.file_stem().and_then(|s| s.to_str()).unwrap_or("rec");
    let pid = std::process::id();
    let video_tmp = parent.join(format!(".{stem}.{pid}.video.mp4"));
    let audio_tmp = parent.join(format!(".{stem}.{pid}.audio.pcm"));

    if fs::copy(mp4_path, &video_tmp).is_err() {
        return;
    }
    if fs::write(&audio_tmp, pcm).is_err() {
        let _ = fs::remove_file(&video_tmp);
        return;
    }

    let status = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-i"])
        .arg(&video_tmp)
        .args(["-f", "s16le", "-ar", &rate.to_string(), "-ac", &channels.to_string(), "-i"])
        .arg(&audio_tmp)
        .args(["-c:v", "copy", "-c:a", "aac", "-b:a", "128k", "-shortest", "-movflags", "+faststart"])
        .arg(mp4_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    if !status.is_ok_and(|s| s.success()) {
        // Restore pure-Rust video MP4 if ffmpeg was unavailable or failed.
        let _ = fs::copy(&video_tmp, mp4_path);
    }
    let _ = fs::remove_file(&video_tmp);
    let _ = fs::remove_file(&audio_tmp);
}

const TIMESCALE: u32 = 90_000;

fn mp4_box(fourcc: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let total = (8 + payload.len()) as u32;
    let mut out = Vec::with_capacity(total as usize);
    out.extend_from_slice(&total.to_be_bytes());
    out.extend_from_slice(fourcc);
    out.extend_from_slice(payload);
    out
}

fn mp4_full_box(fourcc: &[u8; 4], version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
    let total = (12 + body.len()) as u32;
    let mut out = Vec::with_capacity(total as usize);
    out.extend_from_slice(&total.to_be_bytes());
    out.extend_from_slice(fourcc);
    let vf = (u32::from(version) << 24) | (flags & 0x00FF_FFFF);
    out.extend_from_slice(&vf.to_be_bytes());
    out.extend_from_slice(body);
    out
}

fn write_mp4_container(
    path: &Path,
    width: u32,
    height: u32,
    fps: u32,
    sps_pps: &[u8],
    frames: &[RecordedFrame],
) -> Result<(), String> {
    let default_delta = (TIMESCALE / fps.max(1)).max(1);
    let mut mdat_payload = Vec::new();
    let mut sample_sizes = Vec::with_capacity(frames.len());
    let mut sample_deltas = Vec::with_capacity(frames.len());
    let mut sync_samples = Vec::new();

    for (idx, frame) in frames.iter().enumerate() {
        let mut sample_bytes = Vec::new();
        for nal in split_annexb(&frame.data) {
            if nal.is_empty() || matches!(nal_type(nal), Some(7..=9)) {
                continue;
            }
            let len = nal.len() as u32;
            sample_bytes.extend_from_slice(&len.to_be_bytes());
            sample_bytes.extend_from_slice(nal);
        }
        if sample_bytes.is_empty() {
            for nal in split_annexb(&frame.data) {
                if nal.is_empty() {
                    continue;
                }
                let len = nal.len() as u32;
                sample_bytes.extend_from_slice(&len.to_be_bytes());
                sample_bytes.extend_from_slice(nal);
            }
        }
        mdat_payload.extend_from_slice(&sample_bytes);
        sample_sizes.push(sample_bytes.len() as u32);

        let delta = if let Some(next) = frames.get(idx + 1) {
            let diff_us = next.pts_us.saturating_sub(frame.pts_us);
            ((diff_us * u64::from(TIMESCALE)) / 1_000_000).max(1) as u32
        } else {
            default_delta
        };
        sample_deltas.push(delta);

        if frame.keyframe {
            sync_samples.push((idx + 1) as u32);
        }
    }

    if sync_samples.is_empty() {
        sync_samples.push(1);
    }

    let (sps, pps) = extract_sps_and_pps_vecs(sps_pps, frames);
    let ftyp = build_ftyp();
    let dummy_moov = build_moov(
        width.max(1) as u16,
        height.max(1) as u16,
        &sps,
        &pps,
        &sample_deltas,
        &sync_samples,
        &sample_sizes,
        0,
    );
    let chunk_offset = (ftyp.len() + dummy_moov.len() + 8) as u32;
    let moov = build_moov(
        width.max(1) as u16,
        height.max(1) as u16,
        &sps,
        &pps,
        &sample_deltas,
        &sync_samples,
        &sample_sizes,
        chunk_offset,
    );
    let mdat = mp4_box(b"mdat", &mdat_payload);

    let mut file = File::create(path).map_err(|e| format!("create {}: {e}", path.display()))?;
    file.write_all(&ftyp).map_err(|e| e.to_string())?;
    file.write_all(&moov).map_err(|e| e.to_string())?;
    file.write_all(&mdat).map_err(|e| e.to_string())?;
    file.flush().map_err(|e| e.to_string())?;
    Ok(())
}

fn extract_sps_and_pps_vecs(sps_pps: &[u8], frames: &[RecordedFrame]) -> (Vec<u8>, Vec<u8>) {
    let mut sps = None;
    let mut pps = None;
    for nal in split_annexb(sps_pps) {
        match nal_type(nal) {
            Some(7) => sps = Some(nal.to_vec()),
            Some(8) => pps = Some(nal.to_vec()),
            _ => {}
        }
    }
    if sps.is_none() || pps.is_none() {
        for frame in frames {
            for nal in split_annexb(&frame.data) {
                match nal_type(nal) {
                    Some(7) if sps.is_none() => sps = Some(nal.to_vec()),
                    Some(8) if pps.is_none() => pps = Some(nal.to_vec()),
                    _ => {}
                }
            }
            if sps.is_some() && pps.is_some() {
                break;
            }
        }
    }
    let sps = sps.unwrap_or_else(|| vec![0x67, 0x42, 0xC0, 0x1F, 0xDA, 0x01, 0x40, 0x16, 0xEC, 0x04, 0x40]);
    let pps = pps.unwrap_or_else(|| vec![0x68, 0xCE, 0x3C, 0x80]);
    (sps, pps)
}

fn build_ftyp() -> Vec<u8> {
    let mut body = Vec::with_capacity(24);
    body.extend_from_slice(b"isom");
    body.extend_from_slice(&0x0000_0200u32.to_be_bytes());
    body.extend_from_slice(b"isom");
    body.extend_from_slice(b"iso2");
    body.extend_from_slice(b"avc1");
    body.extend_from_slice(b"mp41");
    mp4_box(b"ftyp", &body)
}

#[allow(clippy::too_many_arguments)]
fn build_moov(
    width: u16,
    height: u16,
    sps: &[u8],
    pps: &[u8],
    sample_deltas: &[u32],
    sync_samples: &[u32],
    sample_sizes: &[u32],
    chunk_offset: u32,
) -> Vec<u8> {
    let total_duration: u32 = sample_deltas.iter().copied().fold(0u32, u32::saturating_add).max(1);
    let mvhd = build_mvhd(total_duration);
    let trak = build_trak(
        width,
        height,
        total_duration,
        sps,
        pps,
        sample_deltas,
        sync_samples,
        sample_sizes,
        chunk_offset,
    );
    let mut body = Vec::with_capacity(mvhd.len() + trak.len());
    body.extend_from_slice(&mvhd);
    body.extend_from_slice(&trak);
    mp4_box(b"moov", &body)
}

fn build_mvhd(duration: u32) -> Vec<u8> {
    let mut body = Vec::with_capacity(96);
    body.extend_from_slice(&0u32.to_be_bytes()); // creation_time
    body.extend_from_slice(&0u32.to_be_bytes()); // modification_time
    body.extend_from_slice(&TIMESCALE.to_be_bytes());
    body.extend_from_slice(&duration.to_be_bytes());
    body.extend_from_slice(&0x0001_0000u32.to_be_bytes()); // rate 1.0
    body.extend_from_slice(&0x0100u16.to_be_bytes()); // volume 1.0
    body.extend_from_slice(&[0u8; 10]); // reserved
    // Unity matrix
    for val in [0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000] {
        body.extend_from_slice(&val.to_be_bytes());
    }
    body.extend_from_slice(&[0u8; 24]); // pre_defined
    body.extend_from_slice(&2u32.to_be_bytes()); // next_track_ID
    mp4_full_box(b"mvhd", 0, 0, &body)
}

#[allow(clippy::too_many_arguments)]
fn build_trak(
    width: u16,
    height: u16,
    duration: u32,
    sps: &[u8],
    pps: &[u8],
    sample_deltas: &[u32],
    sync_samples: &[u32],
    sample_sizes: &[u32],
    chunk_offset: u32,
) -> Vec<u8> {
    let tkhd = build_tkhd(width, height, duration);
    let mdia = build_mdia(
        width,
        height,
        duration,
        sps,
        pps,
        sample_deltas,
        sync_samples,
        sample_sizes,
        chunk_offset,
    );
    let mut body = Vec::with_capacity(tkhd.len() + mdia.len());
    body.extend_from_slice(&tkhd);
    body.extend_from_slice(&mdia);
    mp4_box(b"trak", &body)
}

fn build_tkhd(width: u16, height: u16, duration: u32) -> Vec<u8> {
    let mut body = Vec::with_capacity(80);
    body.extend_from_slice(&0u32.to_be_bytes()); // creation_time
    body.extend_from_slice(&0u32.to_be_bytes()); // modification_time
    body.extend_from_slice(&1u32.to_be_bytes()); // track_ID
    body.extend_from_slice(&0u32.to_be_bytes()); // reserved
    body.extend_from_slice(&duration.to_be_bytes());
    body.extend_from_slice(&[0u8; 8]); // reserved
    body.extend_from_slice(&0u16.to_be_bytes()); // layer
    body.extend_from_slice(&0u16.to_be_bytes()); // alternate_group
    body.extend_from_slice(&0u16.to_be_bytes()); // volume (0 for video)
    body.extend_from_slice(&0u16.to_be_bytes()); // reserved
    for val in [0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000] {
        body.extend_from_slice(&val.to_be_bytes());
    }
    body.extend_from_slice(&(u32::from(width) << 16).to_be_bytes());
    body.extend_from_slice(&(u32::from(height) << 16).to_be_bytes());
    mp4_full_box(b"tkhd", 0, 0x0000_0003, &body)
}

#[allow(clippy::too_many_arguments)]
fn build_mdia(
    width: u16,
    height: u16,
    duration: u32,
    sps: &[u8],
    pps: &[u8],
    sample_deltas: &[u32],
    sync_samples: &[u32],
    sample_sizes: &[u32],
    chunk_offset: u32,
) -> Vec<u8> {
    let mut mdhd_body = Vec::with_capacity(20);
    mdhd_body.extend_from_slice(&0u32.to_be_bytes());
    mdhd_body.extend_from_slice(&0u32.to_be_bytes());
    mdhd_body.extend_from_slice(&TIMESCALE.to_be_bytes());
    mdhd_body.extend_from_slice(&duration.to_be_bytes());
    mdhd_body.extend_from_slice(&0x55C4u16.to_be_bytes()); // "und"
    mdhd_body.extend_from_slice(&0u16.to_be_bytes());
    let mdhd = mp4_full_box(b"mdhd", 0, 0, &mdhd_body);

    let mut hdlr_body = Vec::with_capacity(32);
    hdlr_body.extend_from_slice(&0u32.to_be_bytes()); // pre_defined
    hdlr_body.extend_from_slice(b"vide");
    hdlr_body.extend_from_slice(&[0u8; 12]); // reserved
    hdlr_body.extend_from_slice(b"VideoHandler\0");
    let hdlr = mp4_full_box(b"hdlr", 0, 0, &hdlr_body);

    let minf = build_minf(width, height, sps, pps, sample_deltas, sync_samples, sample_sizes, chunk_offset);

    let mut body = Vec::with_capacity(mdhd.len() + hdlr.len() + minf.len());
    body.extend_from_slice(&mdhd);
    body.extend_from_slice(&hdlr);
    body.extend_from_slice(&minf);
    mp4_box(b"mdia", &body)
}

#[allow(clippy::too_many_arguments)]
fn build_minf(
    width: u16,
    height: u16,
    sps: &[u8],
    pps: &[u8],
    sample_deltas: &[u32],
    sync_samples: &[u32],
    sample_sizes: &[u32],
    chunk_offset: u32,
) -> Vec<u8> {
    let vmhd = mp4_full_box(b"vmhd", 0, 1, &[0u8; 8]);
    let url = mp4_full_box(b"url ", 0, 1, &[]);
    let mut dref_body = Vec::with_capacity(4 + url.len());
    dref_body.extend_from_slice(&1u32.to_be_bytes());
    dref_body.extend_from_slice(&url);
    let dref = mp4_full_box(b"dref", 0, 0, &dref_body);
    let dinf = mp4_box(b"dinf", &dref);

    let stbl = build_stbl(width, height, sps, pps, sample_deltas, sync_samples, sample_sizes, chunk_offset);

    let mut body = Vec::with_capacity(vmhd.len() + dinf.len() + stbl.len());
    body.extend_from_slice(&vmhd);
    body.extend_from_slice(&dinf);
    body.extend_from_slice(&stbl);
    mp4_box(b"minf", &body)
}

#[allow(clippy::too_many_arguments)]
fn build_stbl(
    width: u16,
    height: u16,
    sps: &[u8],
    pps: &[u8],
    sample_deltas: &[u32],
    sync_samples: &[u32],
    sample_sizes: &[u32],
    chunk_offset: u32,
) -> Vec<u8> {
    let stsd = build_stsd(width, height, sps, pps);
    let stts = build_stts(sample_deltas);
    let stss = build_stss(sync_samples);
    let stsc = build_stsc(sample_sizes.len() as u32);
    let stsz = build_stsz(sample_sizes);
    let stco = build_stco(chunk_offset);

    let mut body = Vec::new();
    body.extend_from_slice(&stsd);
    body.extend_from_slice(&stts);
    body.extend_from_slice(&stss);
    body.extend_from_slice(&stsc);
    body.extend_from_slice(&stsz);
    body.extend_from_slice(&stco);
    mp4_box(b"stbl", &body)
}

fn build_stsd(width: u16, height: u16, sps: &[u8], pps: &[u8]) -> Vec<u8> {
    let profile_idc = *sps.get(1).unwrap_or(&0x42);
    let profile_compat = *sps.get(2).unwrap_or(&0xC0);
    let level_idc = *sps.get(3).unwrap_or(&0x1F);

    let mut avcc_body = vec![
        1, // configurationVersion
        profile_idc,
        profile_compat,
        level_idc,
        0xFF, // 4-byte NAL unit lengths
        0xE1, // 1 SPS
    ];
    avcc_body.extend_from_slice(&(sps.len() as u16).to_be_bytes());
    avcc_body.extend_from_slice(sps);
    avcc_body.push(1); // 1 PPS
    avcc_body.extend_from_slice(&(pps.len() as u16).to_be_bytes());
    avcc_body.extend_from_slice(pps);
    let avcc = mp4_box(b"avcC", &avcc_body);

    let mut avc1_body = Vec::with_capacity(78 + avcc.len());
    avc1_body.extend_from_slice(&[0u8; 6]); // reserved
    avc1_body.extend_from_slice(&1u16.to_be_bytes()); // data_reference_index
    avc1_body.extend_from_slice(&[0u8; 16]); // pre_defined + reserved
    avc1_body.extend_from_slice(&width.to_be_bytes());
    avc1_body.extend_from_slice(&height.to_be_bytes());
    avc1_body.extend_from_slice(&0x0048_0000u32.to_be_bytes()); // horizresolution 72 dpi
    avc1_body.extend_from_slice(&0x0048_0000u32.to_be_bytes()); // vertresolution 72 dpi
    avc1_body.extend_from_slice(&0u32.to_be_bytes()); // reserved
    avc1_body.extend_from_slice(&1u16.to_be_bytes()); // frame_count
    avc1_body.extend_from_slice(&[0u8; 32]); // compressorname
    avc1_body.extend_from_slice(&0x0018u16.to_be_bytes()); // depth = 24
    avc1_body.extend_from_slice(&(-1i16).to_be_bytes()); // pre_defined = -1
    avc1_body.extend_from_slice(&avcc);
    let avc1 = mp4_box(b"avc1", &avc1_body);

    let mut stsd_body = Vec::with_capacity(4 + avc1.len());
    stsd_body.extend_from_slice(&1u32.to_be_bytes()); // entry_count = 1
    stsd_body.extend_from_slice(&avc1);
    mp4_full_box(b"stsd", 0, 0, &stsd_body)
}

fn build_stts(sample_deltas: &[u32]) -> Vec<u8> {
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for &delta in sample_deltas {
        if let Some(last) = runs.last_mut()
            && last.1 == delta
        {
            last.0 += 1;
        } else {
            runs.push((1, delta));
        }
    }
    let mut body = Vec::with_capacity(4 + runs.len() * 8);
    body.extend_from_slice(&(runs.len() as u32).to_be_bytes());
    for (count, delta) in runs {
        body.extend_from_slice(&count.to_be_bytes());
        body.extend_from_slice(&delta.to_be_bytes());
    }
    mp4_full_box(b"stts", 0, 0, &body)
}

fn build_stss(sync_samples: &[u32]) -> Vec<u8> {
    let mut body = Vec::with_capacity(4 + sync_samples.len() * 4);
    body.extend_from_slice(&(sync_samples.len() as u32).to_be_bytes());
    for &idx in sync_samples {
        body.extend_from_slice(&idx.to_be_bytes());
    }
    mp4_full_box(b"stss", 0, 0, &body)
}

fn build_stsc(total_samples: u32) -> Vec<u8> {
    let mut body = Vec::with_capacity(16);
    body.extend_from_slice(&1u32.to_be_bytes()); // entry_count = 1
    body.extend_from_slice(&1u32.to_be_bytes()); // first_chunk = 1
    body.extend_from_slice(&total_samples.to_be_bytes()); // samples_per_chunk
    body.extend_from_slice(&1u32.to_be_bytes()); // sample_description_index = 1
    mp4_full_box(b"stsc", 0, 0, &body)
}

fn build_stsz(sample_sizes: &[u32]) -> Vec<u8> {
    let mut body = Vec::with_capacity(8 + sample_sizes.len() * 4);
    body.extend_from_slice(&0u32.to_be_bytes()); // sample_size = 0 (variable)
    body.extend_from_slice(&(sample_sizes.len() as u32).to_be_bytes());
    for &sz in sample_sizes {
        body.extend_from_slice(&sz.to_be_bytes());
    }
    mp4_full_box(b"stsz", 0, 0, &body)
}

fn build_stco(chunk_offset: u32) -> Vec<u8> {
    let mut body = Vec::with_capacity(8);
    body.extend_from_slice(&1u32.to_be_bytes()); // entry_count = 1
    body.extend_from_slice(&chunk_offset.to_be_bytes());
    mp4_full_box(b"stco", 0, 0, &body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn extract_sps_pps_picks_out_parameter_sets_only() {
        let stream = [
            0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1F, // SPS (type 7)
            0x00, 0x00, 0x00, 0x01, 0x68, 0xCE, 0x3C, 0x80, // PPS (type 8)
            0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84, 0x21, // IDR (type 5)
        ];
        let extracted = extract_sps_pps(&stream).expect("should extract SPS and PPS");
        assert_eq!(
            extracted,
            vec![
                0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1F, 0x00, 0x00, 0x00, 0x01, 0x68, 0xCE, 0x3C,
                0x80,
            ]
        );
        assert!(extract_sps_pps(&[0x00, 0x00, 0x00, 0x01, 0x65, 0x88]).is_none());
    }

    #[test]
    fn merge_sps_pps_updates_each_parameter_set_in_place() {
        let mut buf = Vec::new();
        let sps1 = [0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1F];
        let pps1 = [0x00, 0x00, 0x00, 0x01, 0x68, 0xCE, 0x3C, 0x80];
        merge_sps_pps(&mut buf, &sps1);
        assert_eq!(buf, sps1);

        merge_sps_pps(&mut buf, &pps1);
        let mut expected = Vec::new();
        expected.extend_from_slice(&sps1);
        expected.extend_from_slice(&pps1);
        assert_eq!(buf, expected);

        // Updating PPS alone preserves existing SPS and replaces PPS.
        let pps2 = [0x00, 0x00, 0x01, 0x68, 0xDE, 0xAD, 0xBE];
        merge_sps_pps(&mut buf, &pps2);
        let mut expected2 = Vec::new();
        expected2.extend_from_slice(&sps1);
        expected2.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x68, 0xDE, 0xAD, 0xBE]);
        assert_eq!(buf, expected2);

        // Slice NAL without SPS/PPS leaves `buf` untouched.
        let idr = [0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84];
        merge_sps_pps(&mut buf, &idr);
        assert_eq!(buf, expected2);
    }

    #[test]
    fn has_idr_slice_detects_keyframe_nal() {
        let idr = [0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84];
        let non_idr = [0x00, 0x00, 0x00, 0x01, 0x41, 0x9A, 0x22];
        assert!(has_idr_slice(&idr));
        assert!(!has_idr_slice(&non_idr));
    }

    #[test]
    fn is_parameter_only_ignores_slice_packets() {
        let params =
            [0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1F, 0x00, 0x00, 0x00, 0x01, 0x68, 0xCE, 0x3C, 0x80];
        let with_slice =
            [0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1F, 0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84];
        assert!(is_parameter_only(&params));
        assert!(!is_parameter_only(&with_slice));
        assert!(!is_parameter_only(&[]));
    }

    #[test]
    fn local_timestamp_filename_has_expected_shape() {
        let name = local_timestamp_filename("Recording", "mp4");
        assert!(name.starts_with("Recording "), "unexpected filename: {name}");
        assert!(name.ends_with(".mp4"), "unexpected filename: {name}");
        // "Recording YYYY-MM-DD HH.MM.SS.mp4" -> length 33
        assert_eq!(name.len(), "Recording 2026-10-16 19.40.00.mp4".len());
    }

    #[test]
    fn recorder_writes_playable_mp4_from_synthetic_h264() {
        let dir = tempfile::tempdir().unwrap();
        let mp4_path = dir.path().join("synthetic.mp4");

        let mut rec = MirrorRecorder::new(mp4_path.clone(), 320, 240, 30, None);
        let sps_pps = [
            0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0xC0, 0x1F, 0xDA, 0x01, 0x40, 0x16, 0xEC, 0x04, 0x40, 0x00,
            0x00, 0x00, 0x01, 0x68, 0xCE, 0x3C, 0x80,
        ];
        rec.on_packet(false, 0, &sps_pps).unwrap();

        // Pre-keyframe P-slice is ignored until first keyframe arrives.
        let p_slice = [0x00, 0x00, 0x00, 0x01, 0x41, 0x9A, 0x24, 0x6C, 0x88];
        rec.on_packet(false, 10_000, &p_slice).unwrap();

        let mut idr = vec![0x00, 0x00, 0x00, 0x01, 0x65];
        idr.extend_from_slice(&[0x88u8; 64]);
        rec.on_packet(true, 33_333, &idr).unwrap();
        rec.on_packet(false, 66_666, &p_slice).unwrap();
        rec.on_packet(false, 100_000, &p_slice).unwrap();

        let saved = rec.finish().unwrap();
        assert_eq!(saved, mp4_path);
        let bytes = fs::read(&saved).unwrap();
        assert!(bytes.len() > 256, "unexpectedly small MP4: {} B", bytes.len());
        assert_eq!(&bytes[4..8], b"ftyp");
        assert!(contains_subslice(&bytes, b"moov"), "missing moov box");
        assert!(contains_subslice(&bytes, b"mdat"), "missing mdat box");
        assert!(contains_subslice(&bytes, b"avc1"), "missing avc1 sample entry");
        assert!(contains_subslice(&bytes, b"avcC"), "missing avcC config box");
    }
}
