// SPDX-License-Identifier: GPL-3.0-or-later
//! Voice recordings received from a paired phone (docs/protocol/recorder.md):
//! naming by date and time without overwriting, optional Media Foundation
//! conversion (`M4A` kept as recorded, `MP3`, `WAV`, or `FLAC`), marker sidecar
//! files (`<stem>.markers.txt`), and the Windows notification on completion.

#![allow(unsafe_code)]

use crate::i18n::tr;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Instant, SystemTime},
};

use nectarlink_core::{RecordingMarker, Transfer, TransferState};
#[cfg(windows)]
use windows::{
    Win32::{
        Media::MediaFoundation::{
            IMFAttributes, IMFMediaType, IMFSample, IMFSourceReader, MF_MT_AUDIO_AVG_BYTES_PER_SECOND,
            MF_MT_AUDIO_BITS_PER_SAMPLE, MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND,
            MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_SOURCE_READER_FIRST_AUDIO_STREAM,
            MF_SOURCE_READERF_ENDOFSTREAM, MF_TRANSCODE_CONTAINERTYPE, MFAudioFormat_FLAC, MFAudioFormat_MP3,
            MFAudioFormat_PCM, MFCreateAttributes, MFCreateMediaType, MFCreateSinkWriterFromURL,
            MFCreateSourceReaderFromURL, MFMediaType_Audio, MFT_ENUM_FLAG_ALL, MFTranscodeContainerType_MP3,
            MFTranscodeGetAudioOutputAvailableTypes,
        },
        System::SystemInformation::GetLocalTime,
    },
    core::{HSTRING, Interface},
};

#[cfg(windows)]
use crate::win::h264::ensure_mf_started;
use crate::{
    core_host,
    settings::{RecordingFormat, Settings},
    transfers::{ACTION_OPEN, ACTION_SHOW, TOAST_GROUP},
    win::toast::{self, Toast},
};

#[derive(Debug, Default)]
struct RuntimeConfig {
    folder: Option<PathBuf>,
    format: RecordingFormat,
    updated_at: Option<SystemTime>,
}

static CONFIG: Mutex<RuntimeConfig> =
    Mutex::new(RuntimeConfig { folder: None, format: RecordingFormat::M4a, updated_at: None });

/// Initializes the in-memory recording preferences from loaded settings.
pub fn init(settings: &Settings) {
    let mut cfg = CONFIG.lock().unwrap_or_else(|e| e.into_inner());
    cfg.folder = settings.recordings_folder.clone();
    cfg.format = settings.recordings_format;
    cfg.updated_at = Some(SystemTime::now());
}

pub fn set_folder(folder: Option<PathBuf>) {
    let mut cfg = CONFIG.lock().unwrap_or_else(|e| e.into_inner());
    cfg.folder = folder;
    cfg.updated_at = Some(SystemTime::now());
}

pub fn set_format(format: RecordingFormat) {
    let mut cfg = CONFIG.lock().unwrap_or_else(|e| e.into_inner());
    cfg.format = format;
    cfg.updated_at = Some(SystemTime::now());
}

/// Default folder: `Documents\Nectarlink Recordings`.
pub fn default_folder() -> PathBuf {
    dirs::document_dir()
        .unwrap_or_else(|| core_host::host().data_dir.join("recordings"))
        .join("Nectarlink Recordings")
}

/// The folder where recordings go right now (`configured` if set, otherwise
/// `Documents\Nectarlink Recordings`).
pub fn effective_folder(configured: Option<&Path>) -> PathBuf {
    configured.filter(|p| !p.as_os_str().is_empty()).map(Path::to_path_buf).unwrap_or_else(default_folder)
}

fn active_config() -> (PathBuf, RecordingFormat) {
    let data_dir = &core_host::host().data_dir;
    let settings_path = data_dir.join("desktop-settings.json");
    let disk_mtime = fs::metadata(&settings_path).and_then(|m| m.modified()).ok();
    let cfg = CONFIG.lock().unwrap_or_else(|e| e.into_inner());
    let disk_is_newer = match (disk_mtime, cfg.updated_at) {
        (Some(m), Some(u)) => m > u,
        (Some(_), None) => true,
        _ => false,
    };
    if disk_is_newer {
        drop(cfg);
        let loaded = Settings::load(data_dir);
        let folder = effective_folder(loaded.recordings_folder.as_deref());
        return (folder, loaded.recordings_format);
    }
    (effective_folder(cfg.folder.as_deref()), cfg.format)
}

/// Formats a recording's base name (`Recording 2026-10-07 14.32`).
pub fn format_recording_stem(year: u16, month: u16, day: u16, hour: u16, minute: u16) -> String {
    format!("Recording {year:04}-{month:02}-{day:02} {hour:02}.{minute:02}")
}

/// Base name for a recording saved right now, in local time.
#[cfg(windows)]
pub fn current_recording_stem() -> String {
    // SAFETY: GetLocalTime always succeeds and writes a SYSTEMTIME struct.
    let st = unsafe { GetLocalTime() };
    format_recording_stem(st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute)
}

#[cfg(not(windows))]
pub fn current_recording_stem() -> String {
    // SAFETY: `libc::time` and `libc::localtime_r` are thread-safe POSIX calls
    // operating on stack-allocated `time_t` and `tm` values.
    unsafe {
        let mut t: libc::time_t = 0;
        libc::time(&mut t);
        let mut tm = std::mem::zeroed::<libc::tm>();
        libc::localtime_r(&t, &mut tm);
        format_recording_stem(
            (tm.tm_year + 1900) as u16,
            (tm.tm_mon + 1) as u16,
            tm.tm_mday as u16,
            tm.tm_hour as u16,
            tm.tm_min as u16,
        )
    }
}

/// The name the phone gave a recording (`Recording 2026-10-07 14.32`, from
/// when it started), so one that waited for the PC keeps its own time.
/// `None` for anything else.
pub fn stem_from_phone(name: &str) -> Option<String> {
    let stem = Path::new(name).file_stem()?.to_str()?;
    // The phone's own " (2)" for two in a minute goes; this folder numbers its own.
    let stem = stem.rsplit_once(" (").filter(|(_, n)| n.ends_with(')')).map_or(stem, |(s, _)| s);
    let time = stem.strip_prefix("Recording ")?;
    let ok = time.len() == "2026-10-07 14.32".len()
        && time.chars().enumerate().all(|(i, c)| match i {
            4 | 7 => c == '-',
            10 => c == ' ',
            13 => c == '.',
            _ => c.is_ascii_digit(),
        });
    ok.then(|| stem.to_owned())
}

/// Picks `dir/<stem>.<ext>`, or `dir/<stem> (2).<ext>` and so on if either the
/// audio file or its `.markers.txt` companion already exists. Never overwrites.
pub fn unique_recording_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let ext = ext.trim_start_matches('.');
    let first = dir.join(format!("{stem}.{ext}"));
    let first_markers = dir.join(format!("{stem}.markers.txt"));
    if !first.exists() && !first_markers.exists() {
        return first;
    }
    (2..)
        .map(|n| {
            let candidate_stem = format!("{stem} ({n})");
            (dir.join(format!("{candidate_stem}.{ext}")), dir.join(format!("{candidate_stem}.markers.txt")))
        })
        .find(|(audio, markers)| !audio.exists() && !markers.exists())
        .map(|(audio, _)| audio)
        .unwrap_or_else(|| first)
}

/// Companion marker file path next to `audio_path` (`<stem>.markers.txt`).
pub fn markers_path_for(audio_path: &Path) -> PathBuf {
    let stem = audio_path.file_stem().and_then(|s| s.to_str()).unwrap_or("Recording");
    audio_path.with_file_name(format!("{stem}.markers.txt"))
}

/// Formats recording markers as human-readable lines (`MM:SS.mmm  <label>`).
pub fn format_markers(markers: &[RecordingMarker]) -> String {
    let mut out = String::new();
    for (i, m) in markers.iter().enumerate() {
        let ts = format_timestamp_ms(m.at_ms);
        match m.label.as_deref().map(str::trim).filter(|l| !l.is_empty()) {
            Some(label) => out.push_str(&format!("{ts}  {label}\n")),
            None => out.push_str(&format!("{ts}  Marker {}\n", i + 1)),
        }
    }
    out
}

fn format_timestamp_ms(ms: u64) -> String {
    let total_secs = ms / 1000;
    let millis = ms % 1000;
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    if hours > 0 {
        format!("{hours:02}:{mins:02}:{secs:02}.{millis:03}")
    } else {
        format!("{mins:02}:{secs:02}.{millis:03}")
    }
}

/// Moves or converts a received `.m4a` recording into `dest_dir` under `stem`
/// in `format`, falling back to keeping the original `.m4a` if conversion
/// fails, and writes `<stem>.markers.txt` when `markers` is non-empty.
pub fn save_received_recording(
    src_m4a: &Path,
    dest_dir: &Path,
    stem: &str,
    format: RecordingFormat,
    markers: &[RecordingMarker],
) -> std::io::Result<PathBuf> {
    fs::create_dir_all(dest_dir)?;

    let final_path = match format {
        RecordingFormat::M4a => {
            let dest = unique_recording_path(dest_dir, stem, "m4a");
            move_file(src_m4a, &dest)?;
            dest
        }
        RecordingFormat::Mp3 | RecordingFormat::Wav | RecordingFormat::Flac => {
            let dest = unique_recording_path(dest_dir, stem, format.as_str());
            match convert_audio(src_m4a, &dest, format) {
                Ok(()) if dest.exists() && fs::metadata(&dest).is_ok_and(|m| m.len() > 0) => {
                    let _ = fs::remove_file(src_m4a);
                    dest
                }
                Ok(()) => {
                    let _ = fs::remove_file(&dest);
                    tracing::warn!(format = %format.as_str(), "converted audio was empty; keeping original .m4a");
                    let fallback = unique_recording_path(dest_dir, stem, "m4a");
                    move_file(src_m4a, &fallback)?;
                    fallback
                }
                Err(e) => {
                    let _ = fs::remove_file(&dest);
                    tracing::warn!(error = %e, format = %format.as_str(), "audio conversion failed; keeping original .m4a");
                    let fallback = unique_recording_path(dest_dir, stem, "m4a");
                    move_file(src_m4a, &fallback)?;
                    fallback
                }
            }
        }
    };

    if !markers.is_empty() {
        let markers_file = markers_path_for(&final_path);
        if let Err(e) = fs::write(&markers_file, format_markers(markers)) {
            tracing::warn!(error = %e, "can't write recording markers file");
        }
    }

    Ok(final_path)
}

fn move_file(src: &Path, dest: &Path) -> std::io::Result<()> {
    if fs::rename(src, dest).is_ok() {
        return Ok(());
    }
    fs::copy(src, dest)?;
    let _ = fs::remove_file(src);
    Ok(())
}

/// ffmpeg arguments that convert `src` into `dest` in `format`. The paths
/// are arguments of their own (never parsed by a shell), and a relative path
/// starting with `-` gets `./` so ffmpeg can't take it for an option.
#[cfg(target_os = "linux")]
pub(crate) fn ffmpeg_args(src: &Path, dest: &Path, format: RecordingFormat) -> Vec<std::ffi::OsString> {
    fn not_an_option(path: &Path) -> std::ffi::OsString {
        if path.to_string_lossy().starts_with('-') { Path::new(".").join(path).into() } else { path.into() }
    }
    let codec: &[&str] = match format {
        RecordingFormat::M4a => &["-c", "copy"],
        RecordingFormat::Mp3 => &["-c:a", "libmp3lame", "-q:a", "2"],
        RecordingFormat::Wav => &["-c:a", "pcm_s16le"],
        RecordingFormat::Flac => &["-c:a", "flac"],
    };
    let mut args: Vec<std::ffi::OsString> =
        ["-nostdin", "-v", "error", "-y", "-i"].into_iter().map(Into::into).collect();
    args.push(not_an_option(src));
    args.extend(codec.iter().map(Into::into));
    args.push(not_an_option(dest));
    args
}

/// Converts `src` (`.m4a`) to `dest` in `format` on Linux, using in-process
/// `libavcodec` + pure-Rust WAV/FLAC + `libmp3lame` first and falling back to
/// the `ffmpeg` CLI only if needed.
#[cfg(target_os = "linux")]
pub fn convert_audio(src: &Path, dest: &Path, format: RecordingFormat) -> std::io::Result<()> {
    if matches!(format, RecordingFormat::M4a) {
        fs::copy(src, dest)?;
        return Ok(());
    }
    if convert_audio_in_process(src, dest, format).is_some() {
        return Ok(());
    }
    let status = std::process::Command::new("ffmpeg")
        .args(ffmpeg_args(src, dest, format))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                std::io::Error::other("converting recordings needs libavcodec or ffmpeg")
            } else {
                e
            }
        })?;
    if status.success() { Ok(()) } else { Err(std::io::Error::other(format!("ffmpeg exited with {status}"))) }
}

#[cfg(all(target_os = "linux", linux_ffmpeg_video))]
struct DemuxedM4a {
    sample_rate: u32,
    asc: [u8; 2],
    frames: Vec<Vec<u8>>,
}

#[cfg(all(target_os = "linux", linux_ffmpeg_video))]
fn default_aac_lc_asc(sample_rate: u32, channels: u8) -> [u8; 2] {
    let freq_idx: u8 = match sample_rate {
        96_000 => 0,
        88_200 => 1,
        64_000 => 2,
        48_000 => 3,
        44_100 => 4,
        32_000 => 5,
        24_000 => 6,
        22_050 => 7,
        16_000 => 8,
        12_000 => 9,
        11_025 => 10,
        8_000 => 11,
        7_350 => 12,
        _ => 3,
    };
    let ch = channels.clamp(1, 2);
    let b0 = (2u8 << 3) | ((freq_idx >> 1) & 0x07);
    let b1 = ((freq_idx & 0x01) << 7) | ((ch & 0x0F) << 3);
    [b0, b1]
}

#[cfg(all(target_os = "linux", linux_ffmpeg_video))]
fn demux_m4a_aac(bytes: &[u8]) -> Option<DemuxedM4a> {
    #[derive(Default)]
    struct TrackTables {
        sample_rate: u32,
        channels: u8,
        asc: Option<[u8; 2]>,
        sample_sizes: Vec<u32>,
        stsc: Vec<(u32, u32)>,
        chunk_offsets: Vec<u64>,
    }

    fn parse_esds_asc(esds_body: &[u8]) -> Option<[u8; 2]> {
        // Scan for DecoderSpecificInfo tag (0x05) inside esds body (after 4-byte FullBox header).
        let data = esds_body.get(4..)?;
        for i in 0..data.len() {
            if data[i] == 0x05 {
                let mut pos = i + 1;
                let mut len = 0usize;
                for _ in 0..4 {
                    let b = *data.get(pos)?;
                    pos += 1;
                    len = (len << 7) | usize::from(b & 0x7F);
                    if b & 0x80 == 0 {
                        break;
                    }
                }
                if (2..=16).contains(&len) && pos + 2 <= data.len() {
                    let b0 = data[pos];
                    let b1 = data[pos + 1];
                    let obj_type = (b0 >> 3) & 0x1F;
                    if (1..=5).contains(&obj_type) {
                        return Some([b0, b1]);
                    }
                }
            }
        }
        None
    }

    fn walk_boxes(data: &[u8], tables: &mut TrackTables) {
        let mut pos = 0usize;
        while pos + 8 <= data.len() {
            let size32 =
                u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
            let fourcc = &data[pos + 4..pos + 8];
            let (header_len, box_len) = if size32 == 1 {
                if pos + 16 > data.len() {
                    break;
                }
                let size64 = u64::from_be_bytes([
                    data[pos + 8],
                    data[pos + 9],
                    data[pos + 10],
                    data[pos + 11],
                    data[pos + 12],
                    data[pos + 13],
                    data[pos + 14],
                    data[pos + 15],
                ]) as usize;
                (16usize, size64)
            } else if size32 == 0 {
                (8usize, data.len() - pos)
            } else {
                (8usize, size32)
            };
            if box_len < header_len || pos + box_len > data.len() {
                break;
            }
            let body = &data[pos + header_len..pos + box_len];
            match fourcc {
                b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" => walk_boxes(body, tables),
                b"stsd" if body.len() >= 8 => walk_boxes(&body[8..], tables),
                b"mp4a" if body.len() >= 28 => {
                    tables.channels = u16::from_be_bytes([body[16], body[17]]).clamp(1, 2) as u8;
                    let sr = u32::from_be_bytes([body[24], body[25], body[26], body[27]]) >> 16;
                    if sr > 0 {
                        tables.sample_rate = sr;
                    }
                    walk_boxes(&body[28..], tables);
                }
                b"esds" => {
                    if let Some(asc) = parse_esds_asc(body) {
                        tables.asc = Some(asc);
                    }
                }
                b"stsz" if body.len() >= 12 => {
                    let fixed_size = u32::from_be_bytes([body[4], body[5], body[6], body[7]]);
                    let count = u32::from_be_bytes([body[8], body[9], body[10], body[11]]) as usize;
                    if fixed_size > 0 {
                        tables.sample_sizes = vec![fixed_size; count];
                    } else if body.len() >= 12 + count.saturating_mul(4) {
                        tables.sample_sizes = (0..count)
                            .map(|i| {
                                let o = 12 + i * 4;
                                u32::from_be_bytes([body[o], body[o + 1], body[o + 2], body[o + 3]])
                            })
                            .collect();
                    }
                }
                b"stsc" if body.len() >= 8 => {
                    let count = u32::from_be_bytes([body[4], body[5], body[6], body[7]]) as usize;
                    if body.len() >= 8 + count.saturating_mul(12) {
                        tables.stsc = (0..count)
                            .map(|i| {
                                let o = 8 + i * 12;
                                let first_chunk =
                                    u32::from_be_bytes([body[o], body[o + 1], body[o + 2], body[o + 3]]);
                                let spc =
                                    u32::from_be_bytes([body[o + 4], body[o + 5], body[o + 6], body[o + 7]]);
                                (first_chunk, spc)
                            })
                            .collect();
                    }
                }
                b"stco" if body.len() >= 8 => {
                    let count = u32::from_be_bytes([body[4], body[5], body[6], body[7]]) as usize;
                    if body.len() >= 8 + count.saturating_mul(4) {
                        tables.chunk_offsets = (0..count)
                            .map(|i| {
                                let o = 8 + i * 4;
                                u64::from(u32::from_be_bytes([
                                    body[o],
                                    body[o + 1],
                                    body[o + 2],
                                    body[o + 3],
                                ]))
                            })
                            .collect();
                    }
                }
                b"co64" if body.len() >= 8 => {
                    let count = u32::from_be_bytes([body[4], body[5], body[6], body[7]]) as usize;
                    if body.len() >= 8 + count.saturating_mul(8) {
                        tables.chunk_offsets = (0..count)
                            .map(|i| {
                                let o = 8 + i * 8;
                                u64::from_be_bytes([
                                    body[o],
                                    body[o + 1],
                                    body[o + 2],
                                    body[o + 3],
                                    body[o + 4],
                                    body[o + 5],
                                    body[o + 6],
                                    body[o + 7],
                                ])
                            })
                            .collect();
                    }
                }
                _ => {}
            }
            pos += box_len;
        }
    }

    let mut tables = TrackTables::default();
    walk_boxes(bytes, &mut tables);
    if tables.sample_sizes.is_empty() || tables.chunk_offsets.is_empty() {
        return None;
    }

    let sample_rate = if tables.sample_rate > 0 { tables.sample_rate } else { 48_000 };
    let channels = if tables.channels > 0 { tables.channels } else { 1 };
    let asc = tables.asc.unwrap_or_else(|| default_aac_lc_asc(sample_rate, channels));

    let mut frames = Vec::with_capacity(tables.sample_sizes.len());
    let mut sample_idx = 0usize;
    for (chunk_idx, &chunk_off) in tables.chunk_offsets.iter().enumerate() {
        if sample_idx >= tables.sample_sizes.len() {
            break;
        }
        let chunk_num = (chunk_idx + 1) as u32;
        let samples_in_chunk = tables
            .stsc
            .iter()
            .rev()
            .find(|&&(first, _)| first <= chunk_num)
            .map(|&(_, spc)| spc as usize)
            .unwrap_or(tables.sample_sizes.len());
        let mut off = chunk_off as usize;
        for _ in 0..samples_in_chunk {
            if sample_idx >= tables.sample_sizes.len() {
                break;
            }
            let sz = tables.sample_sizes[sample_idx] as usize;
            sample_idx += 1;
            let end = off.checked_add(sz)?;
            let slice = bytes.get(off..end)?;
            frames.push(slice.to_vec());
            off = end;
        }
    }

    if frames.is_empty() {
        return None;
    }
    Some(DemuxedM4a { sample_rate, asc, frames })
}

#[cfg(all(target_os = "linux", linux_ffmpeg_video))]
unsafe extern "C" {
    fn nectarlink_video_aac_decoder_create(
        error: *mut std::ffi::c_char,
        capacity: usize,
    ) -> *mut std::ffi::c_void;
    fn nectarlink_video_aac_decoder_destroy(decoder: *mut std::ffi::c_void);
    fn nectarlink_video_aac_decoder_send(
        decoder: *mut std::ffi::c_void,
        bytes: *const u8,
        length: usize,
        error: *mut std::ffi::c_char,
        capacity: usize,
    ) -> std::ffi::c_int;
    fn nectarlink_video_aac_decoder_receive(
        decoder: *mut std::ffi::c_void,
        pcm: *mut *const i16,
        frames: *mut usize,
        sample_rate: *mut std::ffi::c_int,
        channels: *mut std::ffi::c_int,
        error: *mut std::ffi::c_char,
        capacity: usize,
    ) -> std::ffi::c_int;
}

#[cfg(all(target_os = "linux", linux_ffmpeg_video))]
fn decode_m4a_to_pcm_s16le(m4a_bytes: &[u8]) -> Option<(u32, u8, Vec<i16>)> {
    let demuxed = demux_m4a_aac(m4a_bytes)?;

    let obj_type = ((demuxed.asc[0] >> 3) & 0x1F).max(1);
    let profile = (obj_type - 1) & 0x03;
    let freq_idx = ((demuxed.asc[0] & 0x07) << 1) | ((demuxed.asc[1] >> 7) & 0x01);
    let chan_cfg = ((demuxed.asc[1] >> 3) & 0x0F).clamp(1, 2);

    unsafe {
        let mut error = [0 as std::ffi::c_char; 256];
        let dec = nectarlink_video_aac_decoder_create(error.as_mut_ptr(), error.len());
        if dec.is_null() {
            return None;
        }
        struct Guard(*mut std::ffi::c_void);
        impl Drop for Guard {
            fn drop(&mut self) {
                unsafe { nectarlink_video_aac_decoder_destroy(self.0) };
            }
        }
        let guard = Guard(dec);

        let mut sample_rate = demuxed.sample_rate;
        let mut out_channels = chan_cfg;
        let mut pcm_samples = Vec::new();

        let drain_frames = |dec: *mut std::ffi::c_void,
                            sample_rate: &mut u32,
                            out_channels: &mut u8,
                            pcm_samples: &mut Vec<i16>| {
            let mut err = [0 as std::ffi::c_char; 256];
            loop {
                let mut pcm_ptr: *const i16 = std::ptr::null();
                let mut frames = 0usize;
                let mut sr = 0 as std::ffi::c_int;
                let mut ch = 0 as std::ffi::c_int;
                let status = nectarlink_video_aac_decoder_receive(
                    dec,
                    &mut pcm_ptr,
                    &mut frames,
                    &mut sr,
                    &mut ch,
                    err.as_mut_ptr(),
                    err.len(),
                );
                if status <= 0 {
                    break;
                }
                if sr >= 8_000 {
                    *sample_rate = sr as u32;
                }
                if (1..=2).contains(&ch) {
                    *out_channels = ch as u8;
                }
                if !pcm_ptr.is_null() && frames > 0 {
                    let count = frames * usize::from((*out_channels).clamp(1, 2));
                    pcm_samples.extend_from_slice(std::slice::from_raw_parts(pcm_ptr, count));
                }
            }
        };

        let mut adts_buf = Vec::new();
        for raw_frame in &demuxed.frames {
            let full_len = (7 + raw_frame.len()) as u32;
            let header: [u8; 7] = [
                0xFF,
                0xF1,
                (profile << 6) | ((freq_idx & 0x0F) << 2) | ((chan_cfg >> 2) & 0x01),
                ((chan_cfg & 0x03) << 6) | (((full_len >> 11) & 0x03) as u8),
                ((full_len >> 3) & 0xFF) as u8,
                (((full_len & 0x07) as u8) << 5) | 0x1F,
                0xFC,
            ];
            adts_buf.clear();
            adts_buf.extend_from_slice(&header);
            adts_buf.extend_from_slice(raw_frame);

            if nectarlink_video_aac_decoder_send(
                guard.0,
                adts_buf.as_ptr(),
                adts_buf.len(),
                error.as_mut_ptr(),
                error.len(),
            ) >= 0
            {
                drain_frames(guard.0, &mut sample_rate, &mut out_channels, &mut pcm_samples);
            }
        }
        let _ =
            nectarlink_video_aac_decoder_send(guard.0, std::ptr::null(), 0, error.as_mut_ptr(), error.len());
        drain_frames(guard.0, &mut sample_rate, &mut out_channels, &mut pcm_samples);

        if pcm_samples.is_empty() {
            return None;
        }
        Some((sample_rate, out_channels, pcm_samples))
    }
}

#[cfg(all(target_os = "linux", not(linux_ffmpeg_video)))]
fn decode_m4a_to_pcm_s16le(_m4a_bytes: &[u8]) -> Option<(u32, u8, Vec<i16>)> {
    None
}

#[cfg(target_os = "linux")]
fn encode_wav_s16le(sample_rate: u32, channels: u8, samples: &[i16]) -> Vec<u8> {
    let ch = u16::from(channels.clamp(1, 2));
    let block_align = ch * 2;
    let byte_rate = sample_rate * u32::from(block_align);
    let data_len = (samples.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + samples.len() * 2);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&36u32.saturating_add(data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&ch.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        wav.extend_from_slice(&s.to_le_bytes());
    }
    wav
}

#[cfg(target_os = "linux")]
fn encode_flac_verbatim(sample_rate: u32, channels: u8, samples: &[i16]) -> Vec<u8> {
    fn crc8_flac(data: &[u8]) -> u8 {
        let mut crc = 0u8;
        for &b in data {
            crc ^= b;
            for _ in 0..8 {
                crc = if crc & 0x80 != 0 { (crc << 1) ^ 0x07 } else { crc << 1 };
            }
        }
        crc
    }

    fn crc16_flac(data: &[u8]) -> u16 {
        let mut crc = 0u16;
        for &b in data {
            crc ^= u16::from(b) << 8;
            for _ in 0..8 {
                crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x8005 } else { crc << 1 };
            }
        }
        crc
    }

    fn write_utf8_u32(val: u32, out: &mut Vec<u8>) {
        if val < 0x80 {
            out.push(val as u8);
        } else if val < 0x800 {
            out.push(0xC0 | ((val >> 6) as u8));
            out.push(0x80 | ((val & 0x3F) as u8));
        } else if val < 0x10000 {
            out.push(0xE0 | ((val >> 12) as u8));
            out.push(0x80 | (((val >> 6) & 0x3F) as u8));
            out.push(0x80 | ((val & 0x3F) as u8));
        } else {
            out.push(0xF0 | ((val >> 18) as u8));
            out.push(0x80 | (((val >> 12) & 0x3F) as u8));
            out.push(0x80 | (((val >> 6) & 0x3F) as u8));
            out.push(0x80 | ((val & 0x3F) as u8));
        }
    }

    let ch = usize::from(channels.clamp(1, 2));
    let total_frames = samples.len() / ch;
    let block_size = 4096usize;
    let mut out = Vec::new();
    out.extend_from_slice(b"fLaC");

    // Metadata block header: last-metadata-block = 1 (0x80) | STREAMINFO (0), length = 34.
    out.extend_from_slice(&[0x80, 0x00, 0x00, 34]);
    out.extend_from_slice(&(block_size as u16).to_be_bytes()); // min block size
    out.extend_from_slice(&(block_size as u16).to_be_bytes()); // max block size
    out.extend_from_slice(&[0u8; 6]); // min/max frame size (0 = unknown)
    // 20 bits sample rate, 3 bits (channels - 1), 5 bits (bits_per_sample - 1 = 15), 36 bits total samples.
    let sr = sample_rate.clamp(1, 655_350);
    let ch_minus_1 = ((ch - 1) as u8) & 0x07;
    let bps_minus_1 = 15u8; // 16-bit
    let total_u64 = (total_frames as u64) & 0x0F_FFFF_FFFF;
    let packed: u64 =
        (u64::from(sr) << 44) | (u64::from(ch_minus_1) << 41) | (u64::from(bps_minus_1) << 36) | total_u64;
    out.extend_from_slice(&packed.to_be_bytes());
    out.extend_from_slice(&[0u8; 16]); // MD5 signature (0 = unset per RFC 9639)

    let mut frame_buf = Vec::new();
    let mut frame_number = 0u32;
    let mut offset_frames = 0usize;
    while offset_frames < total_frames {
        let n = (total_frames - offset_frames).min(block_size);
        frame_buf.clear();
        // Sync code (14 bits: 0x3FFE) + reserved(0) + blocking_strategy(0 = fixed) -> 0xFF, 0xF8
        frame_buf.push(0xFF);
        frame_buf.push(0xF8);
        // Block size: 0b0111 (16-bit (n-1) at end of header); Sample rate: 0b1100 (16-bit Hz at end of header)
        frame_buf.push(0x7C);
        // Channel assignment: (ch - 1) << 4 | sample size 16-bit (0b100 << 1) | reserved(0)
        frame_buf.push((ch_minus_1 << 4) | 0x08);
        write_utf8_u32(frame_number, &mut frame_buf);
        frame_buf.extend_from_slice(&((n - 1) as u16).to_be_bytes());
        frame_buf.extend_from_slice(&(sr.min(65535) as u16).to_be_bytes());
        let hdr_crc = crc8_flac(&frame_buf);
        frame_buf.push(hdr_crc);

        for c in 0..ch {
            // Subframe header: 0 (pad) | 000001 (VERBATIM) | 0 (no wasted bits) = 0x02
            frame_buf.push(0x02);
            for i in 0..n {
                let s = samples[(offset_frames + i) * ch + c];
                frame_buf.extend_from_slice(&s.to_be_bytes());
            }
        }
        let ftr_crc = crc16_flac(&frame_buf);
        frame_buf.extend_from_slice(&ftr_crc.to_be_bytes());
        out.extend_from_slice(&frame_buf);

        offset_frames += n;
        frame_number += 1;
    }

    out
}

#[cfg(target_os = "linux")]
struct LameApi {
    lame_init: unsafe extern "C" fn() -> *mut std::ffi::c_void,
    lame_set_in_samplerate: unsafe extern "C" fn(*mut std::ffi::c_void, std::ffi::c_int) -> std::ffi::c_int,
    lame_set_num_channels: unsafe extern "C" fn(*mut std::ffi::c_void, std::ffi::c_int) -> std::ffi::c_int,
    lame_set_vbr: unsafe extern "C" fn(*mut std::ffi::c_void, std::ffi::c_int) -> std::ffi::c_int,
    lame_set_vbr_q: unsafe extern "C" fn(*mut std::ffi::c_void, std::ffi::c_int) -> std::ffi::c_int,
    lame_init_params: unsafe extern "C" fn(*mut std::ffi::c_void) -> std::ffi::c_int,
    lame_encode_buffer: unsafe extern "C" fn(
        *mut std::ffi::c_void,
        *const i16,
        *const i16,
        std::ffi::c_int,
        *mut u8,
        std::ffi::c_int,
    ) -> std::ffi::c_int,
    lame_encode_buffer_interleaved: unsafe extern "C" fn(
        *mut std::ffi::c_void,
        *const i16,
        std::ffi::c_int,
        *mut u8,
        std::ffi::c_int,
    ) -> std::ffi::c_int,
    lame_encode_flush:
        unsafe extern "C" fn(*mut std::ffi::c_void, *mut u8, std::ffi::c_int) -> std::ffi::c_int,
    lame_close: unsafe extern "C" fn(*mut std::ffi::c_void) -> std::ffi::c_int,
}

#[cfg(target_os = "linux")]
fn lame_api() -> Option<&'static LameApi> {
    static API: std::sync::OnceLock<Option<LameApi>> = std::sync::OnceLock::new();
    API.get_or_init(|| unsafe {
        unsafe fn sym<T: Copy>(handle: *mut std::ffi::c_void, name: &std::ffi::CStr) -> Option<T> {
            let p = unsafe { libc::dlsym(handle, name.as_ptr()) };
            if p.is_null() { None } else { Some(unsafe { std::mem::transmute_copy(&p) }) }
        }
        let mut handle = std::ptr::null_mut();
        for name in [c"libmp3lame.so.0", c"libmp3lame.so"] {
            handle = libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
            if !handle.is_null() {
                break;
            }
        }
        if handle.is_null() {
            return None;
        }
        Some(LameApi {
            lame_init: sym(handle, c"lame_init")?,
            lame_set_in_samplerate: sym(handle, c"lame_set_in_samplerate")?,
            lame_set_num_channels: sym(handle, c"lame_set_num_channels")?,
            lame_set_vbr: sym(handle, c"lame_set_VBR")?,
            lame_set_vbr_q: sym(handle, c"lame_set_VBR_q")?,
            lame_init_params: sym(handle, c"lame_init_params")?,
            lame_encode_buffer: sym(handle, c"lame_encode_buffer")?,
            lame_encode_buffer_interleaved: sym(handle, c"lame_encode_buffer_interleaved")?,
            lame_encode_flush: sym(handle, c"lame_encode_flush")?,
            lame_close: sym(handle, c"lame_close")?,
        })
    })
    .as_ref()
}

#[cfg(target_os = "linux")]
fn encode_mp3_with_lame(sample_rate: u32, channels: u8, samples: &[i16]) -> Option<Vec<u8>> {
    let api = lame_api()?;
    let ch = channels.clamp(1, 2) as usize;
    let num_frames = samples.len() / ch;
    if num_frames == 0 {
        return None;
    }

    unsafe {
        let gfp = (api.lame_init)();
        if gfp.is_null() {
            return None;
        }
        struct Guard {
            api: &'static LameApi,
            gfp: *mut std::ffi::c_void,
        }
        impl Drop for Guard {
            fn drop(&mut self) {
                unsafe {
                    let _ = (self.api.lame_close)(self.gfp);
                }
            }
        }
        let g = Guard { api, gfp };
        let _ = (api.lame_set_in_samplerate)(g.gfp, sample_rate as std::ffi::c_int);
        let _ = (api.lame_set_num_channels)(g.gfp, ch as std::ffi::c_int);
        let _ = (api.lame_set_vbr)(g.gfp, 4); // vbr_default
        let _ = (api.lame_set_vbr_q)(g.gfp, 2); // -q:a 2
        if (api.lame_init_params)(g.gfp) < 0 {
            return None;
        }

        let mp3_cap = (num_frames * 5 / 4) + 7200;
        let mut mp3_buf = vec![0u8; mp3_cap];
        let written = if ch == 1 {
            (api.lame_encode_buffer)(
                g.gfp,
                samples.as_ptr(),
                std::ptr::null(),
                num_frames as std::ffi::c_int,
                mp3_buf.as_mut_ptr(),
                mp3_cap as std::ffi::c_int,
            )
        } else {
            (api.lame_encode_buffer_interleaved)(
                g.gfp,
                samples.as_ptr(),
                num_frames as std::ffi::c_int,
                mp3_buf.as_mut_ptr(),
                mp3_cap as std::ffi::c_int,
            )
        };
        if written < 0 {
            return None;
        }
        let mut total = written as usize;
        let flushed = (api.lame_encode_flush)(
            g.gfp,
            mp3_buf[total..].as_mut_ptr(),
            (mp3_cap - total) as std::ffi::c_int,
        );
        if flushed > 0 {
            total += flushed as usize;
        }
        mp3_buf.truncate(total);
        if mp3_buf.is_empty() { None } else { Some(mp3_buf) }
    }
}

#[cfg(target_os = "linux")]
fn convert_audio_in_process(src: &Path, dest: &Path, format: RecordingFormat) -> Option<()> {
    let m4a_bytes = fs::read(src).ok()?;
    let (sample_rate, channels, samples) = decode_m4a_to_pcm_s16le(&m4a_bytes)?;
    let out_bytes = match format {
        RecordingFormat::M4a => m4a_bytes,
        RecordingFormat::Wav => encode_wav_s16le(sample_rate, channels, &samples),
        RecordingFormat::Flac => encode_flac_verbatim(sample_rate, channels, &samples),
        RecordingFormat::Mp3 => encode_mp3_with_lame(sample_rate, channels, &samples)?,
    };
    fs::write(dest, out_bytes).ok()
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn convert_audio(_src: &Path, _dest: &Path, _format: RecordingFormat) -> Result<(), String> {
    Err("audio conversion via Media Foundation is only supported on Windows".into())
}

#[cfg(windows)]
/// Converts `src` (`.m4a`) to `dest` in `format` (`Mp3`, `Wav`, or `Flac`)
/// using Windows Media Foundation.
pub fn convert_audio(src: &Path, dest: &Path, format: RecordingFormat) -> windows::core::Result<()> {
    ensure_mf_started()?;
    let stream_idx = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;

    // SAFETY: Media Foundation COM interfaces with owned parameters.
    unsafe {
        let reader: IMFSourceReader = MFCreateSourceReaderFromURL(&HSTRING::from(src.as_os_str()), None)?;
        let pcm_req: IMFMediaType = MFCreateMediaType()?;
        pcm_req.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        pcm_req.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
        pcm_req.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        reader.SetCurrentMediaType(stream_idx, None, &pcm_req)?;

        let pcm_type: IMFMediaType = reader.GetCurrentMediaType(stream_idx)?;
        let sample_rate = pcm_type.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)?;
        let channels = pcm_type.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)?;

        match format {
            RecordingFormat::M4a => {
                fs::copy(src, dest).map_err(|e| {
                    windows::core::Error::from_hresult(windows::core::HRESULT::from_win32(
                        e.raw_os_error().unwrap_or(1) as u32,
                    ))
                })?;
                Ok(())
            }
            RecordingFormat::Wav => write_decoded_wav(&reader, stream_idx, sample_rate, channels, dest),
            RecordingFormat::Mp3 => {
                transcode_with_sink_writer(&reader, stream_idx, &pcm_type, sample_rate, channels, dest, true)
            }
            RecordingFormat::Flac => {
                transcode_with_sink_writer(&reader, stream_idx, &pcm_type, sample_rate, channels, dest, false)
            }
        }
    }
}

#[cfg(windows)]
unsafe fn write_decoded_wav(
    reader: &IMFSourceReader,
    stream_idx: u32,
    sample_rate: u32,
    channels: u32,
    dest: &Path,
) -> windows::core::Result<()> {
    let mut pcm_bytes = Vec::new();
    loop {
        let mut flags = 0u32;
        let mut sample: Option<IMFSample> = None;
        // SAFETY: valid out-pointers for ReadSample.
        unsafe {
            reader.ReadSample(stream_idx, 0, None, Some(&mut flags), None, Some(&mut sample))?;
        }
        if flags & (MF_SOURCE_READERF_ENDOFSTREAM.0 as u32) != 0 {
            break;
        }
        if let Some(sample) = sample {
            // SAFETY: buffer is locked while copied and unlocked immediately.
            unsafe {
                let buffer = sample.ConvertToContiguousBuffer()?;
                let mut ptr = std::ptr::null_mut();
                let mut len = 0u32;
                buffer.Lock(&mut ptr, None, Some(&mut len))?;
                if !ptr.is_null() && len > 0 {
                    pcm_bytes.extend_from_slice(std::slice::from_raw_parts(ptr, len as usize));
                }
                buffer.Unlock()?;
            }
        }
    }

    let ch = u16::try_from(channels.clamp(1, 2)).unwrap_or(1);
    let block_align = ch * 2;
    let byte_rate = sample_rate * u32::from(block_align);
    let data_len = u32::try_from(pcm_bytes.len()).unwrap_or(u32::MAX);
    let mut wav = Vec::with_capacity(44 + pcm_bytes.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&36u32.saturating_add(data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&ch.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(&pcm_bytes);

    fs::write(dest, wav).map_err(|_| windows::core::Error::from_thread())
}

#[cfg(windows)]
unsafe fn transcode_with_sink_writer(
    reader: &IMFSourceReader,
    stream_idx: u32,
    pcm_type: &IMFMediaType,
    sample_rate: u32,
    channels: u32,
    dest: &Path,
    mp3: bool,
) -> windows::core::Result<()> {
    let subtype = if mp3 { MFAudioFormat_MP3 } else { MFAudioFormat_FLAC };
    // SAFETY: Media Foundation sink writer setup and sample loop.
    unsafe {
        let out_type = choose_encoder_output_type(&subtype, sample_rate, channels, mp3)?;
        let mut attrs: Option<IMFAttributes> = None;
        if mp3 {
            MFCreateAttributes(&mut attrs, 1)?;
            if let Some(a) = &attrs {
                a.SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MP3)?;
            }
        }
        let writer = MFCreateSinkWriterFromURL(&HSTRING::from(dest.as_os_str()), None, attrs.as_ref())?;
        let out_stream = writer.AddStream(&out_type)?;
        writer.SetInputMediaType(out_stream, pcm_type, None)?;
        writer.BeginWriting()?;

        loop {
            let mut flags = 0u32;
            let mut timestamp = 0i64;
            let mut sample: Option<IMFSample> = None;
            reader.ReadSample(
                stream_idx,
                0,
                None,
                Some(&mut flags),
                Some(&mut timestamp),
                Some(&mut sample),
            )?;
            if flags & (MF_SOURCE_READERF_ENDOFSTREAM.0 as u32) != 0 {
                break;
            }
            if let Some(sample) = sample {
                let _ = sample.SetSampleTime(timestamp);
                writer.WriteSample(out_stream, &sample)?;
            }
        }
        writer.Finalize()?;
    }
    Ok(())
}

#[cfg(windows)]
unsafe fn choose_encoder_output_type(
    subtype: &windows::core::GUID,
    sample_rate: u32,
    channels: u32,
    mp3: bool,
) -> windows::core::Result<IMFMediaType> {
    // SAFETY: queries Media Foundation's registered audio encoder output types.
    unsafe {
        if let Ok(collection) =
            MFTranscodeGetAudioOutputAvailableTypes(subtype, MFT_ENUM_FLAG_ALL.0 as u32, None)
        {
            let count = collection.GetElementCount().unwrap_or(0);
            let mut best: Option<(IMFMediaType, u32)> = None;
            for i in 0..count {
                let Ok(unk) = collection.GetElement(i) else { continue };
                let Ok(mt) = unk.cast::<IMFMediaType>() else { continue };
                let Ok(sr) = mt.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND) else { continue };
                let Ok(ch) = mt.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS) else { continue };
                if sr != sample_rate || ch != channels {
                    continue;
                }
                if !mp3
                    && let Ok(bits) = mt.GetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE)
                    && bits != 16
                {
                    continue;
                }
                let byterate = mt.GetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND).unwrap_or(16_000);
                // Prefer ~128 kbps (16000 B/s) for MP3.
                let dist = byterate.abs_diff(16_000);
                if best.as_ref().is_none_or(|(_, best_dist)| dist < *best_dist) {
                    best = Some((mt, dist));
                }
            }
            if let Some((chosen, _)) = best {
                return Ok(chosen);
            }
        }

        let mt: IMFMediaType = MFCreateMediaType()?;
        mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        mt.SetGUID(&MF_MT_SUBTYPE, subtype)?;
        mt.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, sample_rate)?;
        mt.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, channels)?;
        if mp3 {
            mt.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 16_000)?;
        } else {
            mt.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        }
        Ok(mt)
    }
}

/// Handles a completed incoming voice recording transfer off the UI thread:
/// moves/converts the file into the Recordings folder, writes markers when
/// present, updates the transfer entry in Home, and posts the Windows toast.
pub fn on_done(transfer: Transfer) {
    let TransferState::Done { ref saved } = transfer.state else { return };
    let Some(src) = saved.first().cloned() else { return };
    let (dest_dir, format) = active_config();
    let stem = transfer.names.first().and_then(|n| stem_from_phone(n)).unwrap_or_else(current_recording_stem);

    core_host::spawn(async move {
        let markers = transfer.markers.clone();
        let src_for_job = src.clone();
        let dir_for_job = dest_dir.clone();
        let result = tokio::task::spawn_blocking(move || {
            save_received_recording(&src_for_job, &dir_for_job, &stem, format, &markers)
        })
        .await;

        let final_path = match result {
            Ok(Ok(path)) => path,
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "couldn't move or convert recording; keeping received path");
                src
            }
            Err(e) => {
                tracing::warn!(error = %e, "recording task panicked");
                src
            }
        };

        let file_name = final_path
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_owned)
            .unwrap_or_else(|| "Recording.m4a".into());

        let mut updated = transfer.clone();
        updated.names = vec![file_name.clone()];
        if let Ok(meta) = fs::metadata(&final_path) {
            updated.total = meta.len();
            updated.done = meta.len();
        }
        updated.state = TransferState::Done { saved: vec![final_path.clone()] };
        let total_bytes = updated.total;
        core_host::host().hub.update(|s| s.update_transfer(updated, Instant::now()));
        if let Some(node) = core_host::node() {
            let final_path_str = final_path.to_string_lossy();
            let detail = match transfer.markers.len() {
                0 => String::new(),
                1 => "1 marker".to_owned(),
                n => format!("{n} markers"),
            };
            node.update_timeline_by_ref(
                &transfer.id,
                nectarlink_core::TimelineKind::Recording,
                &file_name,
                &detail,
                &final_path_str,
                total_bytes,
            );
        }

        let device = core_host::host()
            .hub
            .read(|s| s.name_of(&transfer.device))
            .unwrap_or_else(|| "your phone".into());

        let body = if transfer.markers.is_empty() {
            file_name
        } else if transfer.markers.len() == 1 {
            format!("{file_name} · 1 marker")
        } else {
            format!("{file_name} · {} markers", transfer.markers.len())
        };

        toast::show(Toast {
            device: TOAST_GROUP.into(),
            key: final_path.to_string_lossy().into_owned(),
            title: tr!("Recordings", "Recording from %1 saved", device),
            body,
            attribution: "NectarLink".into(),
            icon: None,
            image: None,
            actions: vec![(ACTION_OPEN.into(), "Open".into()), (ACTION_SHOW.into(), "Show in folder".into())],
            reply: None,
            silent: false,
            progress: None,
            call: false,
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recordings_keep_the_phones_time() {
        assert_eq!(
            stem_from_phone("Recording 2026-10-07 14.32.m4a").as_deref(),
            Some("Recording 2026-10-07 14.32")
        );
        assert_eq!(
            stem_from_phone("Recording 2026-10-07 14.32 (2).m4a").as_deref(),
            Some("Recording 2026-10-07 14.32")
        );
        assert_eq!(stem_from_phone("Recording ../../x.m4a"), None);
        assert_eq!(stem_from_phone("Voice 2026-10-07 14.32.m4a"), None);
        assert_eq!(stem_from_phone("Recording 2026-10-07 14.32 extra.m4a"), None);
    }

    /// Builds a valid 48 kHz mono AAC-LC `.m4a` file for Media Foundation tests.
    #[allow(dead_code)]
    fn test_m4a(duration_secs: u32) -> Vec<u8> {
        const AAC_FRAME: [u8; 6] = [0x01, 0x48, 0x00, 0x84, 0x21, 0x7E];
        const SAMPLE_RATE: u32 = 48_000;
        let num_frames = (duration_secs.max(1) * SAMPLE_RATE).div_ceil(1024).max(1);
        let total_samples = num_frames * 1024;

        fn mp4_box(fourcc: &[u8; 4], payload: &[u8]) -> Vec<u8> {
            let len = (8 + payload.len()) as u32;
            let mut out = Vec::with_capacity(len as usize);
            out.extend_from_slice(&len.to_be_bytes());
            out.extend_from_slice(fourcc);
            out.extend_from_slice(payload);
            out
        }

        fn full_box(fourcc: &[u8; 4], version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
            let mut payload = Vec::with_capacity(4 + body.len());
            payload.push(version);
            payload.extend_from_slice(&flags.to_be_bytes()[1..4]);
            payload.extend_from_slice(body);
            mp4_box(fourcc, &payload)
        }

        let mut ftyp_body = Vec::new();
        ftyp_body.extend_from_slice(b"M4A ");
        ftyp_body.extend_from_slice(&0u32.to_be_bytes());
        ftyp_body.extend_from_slice(b"M4A mp42isom\0\0\0\0");
        let ftyp = mp4_box(b"ftyp", &ftyp_body);

        let build_moov = |chunk_offset: u32| -> Vec<u8> {
            let mut mvhd = Vec::new();
            mvhd.extend_from_slice(&0u32.to_be_bytes());
            mvhd.extend_from_slice(&0u32.to_be_bytes());
            mvhd.extend_from_slice(&SAMPLE_RATE.to_be_bytes());
            mvhd.extend_from_slice(&total_samples.to_be_bytes());
            mvhd.extend_from_slice(&0x0001_0000u32.to_be_bytes());
            mvhd.extend_from_slice(&0x0100u16.to_be_bytes());
            mvhd.extend_from_slice(&[0u8; 10]);
            for &m in &[0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000] {
                mvhd.extend_from_slice(&m.to_be_bytes());
            }
            mvhd.extend_from_slice(&[0u8; 24]);
            mvhd.extend_from_slice(&2u32.to_be_bytes());
            let mvhd_box = full_box(b"mvhd", 0, 0, &mvhd);

            let mut tkhd = Vec::new();
            tkhd.extend_from_slice(&0u32.to_be_bytes());
            tkhd.extend_from_slice(&0u32.to_be_bytes());
            tkhd.extend_from_slice(&1u32.to_be_bytes());
            tkhd.extend_from_slice(&0u32.to_be_bytes());
            tkhd.extend_from_slice(&total_samples.to_be_bytes());
            tkhd.extend_from_slice(&[0u8; 8]);
            tkhd.extend_from_slice(&0u16.to_be_bytes());
            tkhd.extend_from_slice(&0u16.to_be_bytes());
            tkhd.extend_from_slice(&0x0100u16.to_be_bytes());
            tkhd.extend_from_slice(&0u16.to_be_bytes());
            for &m in &[0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000] {
                tkhd.extend_from_slice(&m.to_be_bytes());
            }
            tkhd.extend_from_slice(&0u32.to_be_bytes());
            tkhd.extend_from_slice(&0u32.to_be_bytes());
            let tkhd_box = full_box(b"tkhd", 0, 3, &tkhd);

            let mut mdhd = Vec::new();
            mdhd.extend_from_slice(&0u32.to_be_bytes());
            mdhd.extend_from_slice(&0u32.to_be_bytes());
            mdhd.extend_from_slice(&SAMPLE_RATE.to_be_bytes());
            mdhd.extend_from_slice(&total_samples.to_be_bytes());
            mdhd.extend_from_slice(&0x55c4u16.to_be_bytes());
            mdhd.extend_from_slice(&0u16.to_be_bytes());
            let mdhd_box = full_box(b"mdhd", 0, 0, &mdhd);

            let mut hdlr = Vec::new();
            hdlr.extend_from_slice(&0u32.to_be_bytes());
            hdlr.extend_from_slice(b"soun");
            hdlr.extend_from_slice(&[0u8; 12]);
            hdlr.extend_from_slice(b"SoundHandler\0");
            let hdlr_box = full_box(b"hdlr", 0, 0, &hdlr);

            let smhd_box = full_box(b"smhd", 0, 0, &[0u8; 4]);
            let url_box = full_box(b"url ", 0, 1, &[]);
            let mut dref = Vec::new();
            dref.extend_from_slice(&1u32.to_be_bytes());
            dref.extend_from_slice(&url_box);
            let dinf_box = mp4_box(b"dinf", &full_box(b"dref", 0, 0, &dref));

            let esds_body: [u8; 27] = [
                0x03, 25, 0x00, 0x01, 0x00, 0x04, 17, 0x40, 0x15, 0x00, 0x00, 0x00, 0x00, 0x01, 0xF4, 0x00,
                0x00, 0x01, 0xF4, 0x00, 0x05, 2, 0x11, 0x88, 0x06, 1, 0x02,
            ];
            let esds_box = full_box(b"esds", 0, 0, &esds_body);

            let mut mp4a = Vec::new();
            mp4a.extend_from_slice(&[0u8; 6]);
            mp4a.extend_from_slice(&1u16.to_be_bytes());
            mp4a.extend_from_slice(&[0u8; 8]);
            mp4a.extend_from_slice(&1u16.to_be_bytes());
            mp4a.extend_from_slice(&16u16.to_be_bytes());
            mp4a.extend_from_slice(&0u16.to_be_bytes());
            mp4a.extend_from_slice(&0u16.to_be_bytes());
            mp4a.extend_from_slice(&(SAMPLE_RATE << 16).to_be_bytes());
            mp4a.extend_from_slice(&esds_box);
            let mp4a_box = mp4_box(b"mp4a", &mp4a);

            let mut stsd = Vec::new();
            stsd.extend_from_slice(&1u32.to_be_bytes());
            stsd.extend_from_slice(&mp4a_box);
            let stsd_box = full_box(b"stsd", 0, 0, &stsd);

            let mut stts = Vec::new();
            stts.extend_from_slice(&1u32.to_be_bytes());
            stts.extend_from_slice(&num_frames.to_be_bytes());
            stts.extend_from_slice(&1024u32.to_be_bytes());
            let stts_box = full_box(b"stts", 0, 0, &stts);

            let mut stsc = Vec::new();
            stsc.extend_from_slice(&1u32.to_be_bytes());
            stsc.extend_from_slice(&1u32.to_be_bytes());
            stsc.extend_from_slice(&num_frames.to_be_bytes());
            stsc.extend_from_slice(&1u32.to_be_bytes());
            let stsc_box = full_box(b"stsc", 0, 0, &stsc);

            let mut stsz = Vec::new();
            stsz.extend_from_slice(&(AAC_FRAME.len() as u32).to_be_bytes());
            stsz.extend_from_slice(&num_frames.to_be_bytes());
            let stsz_box = full_box(b"stsz", 0, 0, &stsz);

            let mut stco = Vec::new();
            stco.extend_from_slice(&1u32.to_be_bytes());
            stco.extend_from_slice(&chunk_offset.to_be_bytes());
            let stco_box = full_box(b"stco", 0, 0, &stco);

            let mut stbl_body = Vec::new();
            stbl_body.extend_from_slice(&stsd_box);
            stbl_body.extend_from_slice(&stts_box);
            stbl_body.extend_from_slice(&stsc_box);
            stbl_body.extend_from_slice(&stsz_box);
            stbl_body.extend_from_slice(&stco_box);
            let stbl_box = mp4_box(b"stbl", &stbl_body);

            let mut minf_body = Vec::new();
            minf_body.extend_from_slice(&smhd_box);
            minf_body.extend_from_slice(&dinf_box);
            minf_body.extend_from_slice(&stbl_box);
            let minf_box = mp4_box(b"minf", &minf_body);

            let mut mdia_body = Vec::new();
            mdia_body.extend_from_slice(&mdhd_box);
            mdia_body.extend_from_slice(&hdlr_box);
            mdia_body.extend_from_slice(&minf_box);
            let mdia_box = mp4_box(b"mdia", &mdia_body);

            let mut trak_body = Vec::new();
            trak_body.extend_from_slice(&tkhd_box);
            trak_body.extend_from_slice(&mdia_box);
            let trak_box = mp4_box(b"trak", &trak_body);

            let mut moov_body = Vec::new();
            moov_body.extend_from_slice(&mvhd_box);
            moov_body.extend_from_slice(&trak_box);
            mp4_box(b"moov", &moov_body)
        };

        let moov_len = build_moov(0).len();
        let chunk_offset = (ftyp.len() + moov_len + 8) as u32;
        let moov = build_moov(chunk_offset);
        let mdat_payload = AAC_FRAME.repeat(num_frames as usize);
        let mdat = mp4_box(b"mdat", &mdat_payload);

        let mut out = Vec::with_capacity(ftyp.len() + moov.len() + mdat.len());
        out.extend_from_slice(&ftyp);
        out.extend_from_slice(&moov);
        out.extend_from_slice(&mdat);
        out
    }

    #[test]
    fn naming_and_collision_avoidance_never_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let stem = format_recording_stem(2026, 10, 7, 14, 32);
        assert_eq!(stem, "Recording 2026-10-07 14.32");

        let p1 = unique_recording_path(dir.path(), &stem, "m4a");
        assert_eq!(p1, dir.path().join("Recording 2026-10-07 14.32.m4a"));
        fs::write(&p1, b"first").unwrap();

        let p2 = unique_recording_path(dir.path(), &stem, "m4a");
        assert_eq!(p2, dir.path().join("Recording 2026-10-07 14.32 (2).m4a"));
        fs::write(&p2, b"second").unwrap();

        // Even if only the marker file for (3) exists, (3) is skipped so it isn't overwritten.
        fs::write(dir.path().join("Recording 2026-10-07 14.32 (3).markers.txt"), b"m").unwrap();
        let p4 = unique_recording_path(dir.path(), &stem, "m4a");
        assert_eq!(p4, dir.path().join("Recording 2026-10-07 14.32 (4).m4a"));
    }

    #[test]
    fn marker_files_format_timestamps_and_labels() {
        let markers = vec![
            RecordingMarker { at_ms: 1_250, label: Some("Intro".into()) },
            RecordingMarker { at_ms: 65_000, label: None },
            RecordingMarker { at_ms: 3_661_005, label: Some("Wrap-up".into()) },
        ];
        let text = format_markers(&markers);
        assert_eq!(text, "00:01.250  Intro\n01:05.000  Marker 2\n01:01:01.005  Wrap-up\n");

        let stem = current_recording_stem();
        assert!(stem.starts_with("Recording "));
        assert_eq!(stem_from_phone(&format!("{stem}.m4a")).as_deref(), Some(stem.as_str()));

        #[cfg(target_os = "linux")]
        {
            let args = ffmpeg_args(
                Path::new("-odd name!.m4a"),
                Path::new("/tmp/out it's.mp3"),
                RecordingFormat::Mp3,
            );
            let args: Vec<String> = args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
            assert_eq!(args[..5], ["-nostdin", "-v", "error", "-y", "-i"]);
            assert_eq!(args[5], "./-odd name!.m4a", "a path can't be read as an option");
            assert!(args.windows(2).any(|w| w == ["-c:a", "libmp3lame"]));
            assert_eq!(
                args.last().map(String::as_str),
                Some("/tmp/out it's.mp3"),
                "names are passed as they are"
            );
        }
    }

    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn media_foundation_converts_m4a_to_m4a_mp3_wav_and_flac_with_markers() {
        let dir = tempfile::tempdir().unwrap();
        let stem = "Recording 2026-10-07 14.32";
        let markers = vec![
            RecordingMarker { at_ms: 500, label: Some("Start".into()) },
            RecordingMarker { at_ms: 1_500, label: None },
        ];

        for format in
            [RecordingFormat::M4a, RecordingFormat::Mp3, RecordingFormat::Wav, RecordingFormat::Flac]
        {
            let src = dir.path().join(format!("incoming-{}.m4a", format.as_str()));
            fs::write(&src, test_m4a(2)).unwrap();
            let out_dir = dir.path().join(format.as_str());
            let saved = save_received_recording(&src, &out_dir, stem, format, &markers).unwrap();
            assert!(!src.exists(), "temporary incoming file is removed for {}", format.as_str());
            assert_eq!(
                saved.extension().and_then(|e| e.to_str()),
                Some(format.as_str()),
                "saved in requested format {}",
                format.as_str()
            );
            let bytes = fs::read(&saved).unwrap();
            assert!(bytes.len() > 100, "{} output has non-trivial size ({} B)", format.as_str(), bytes.len());
            match format {
                RecordingFormat::M4a => assert_eq!(&bytes[4..8], b"ftyp"),
                RecordingFormat::Wav => {
                    assert_eq!(&bytes[0..4], b"RIFF");
                    assert_eq!(&bytes[8..12], b"WAVE");
                }
                RecordingFormat::Flac => assert_eq!(&bytes[0..4], b"fLaC"),
                RecordingFormat::Mp3 => {
                    // Either ID3 tag or MP3 frame sync (0xFF 0xFB / 0xF3 / 0xF2).
                    assert!(
                        &bytes[0..3] == b"ID3" || (bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0),
                        "valid MP3 header: {:02X?}",
                        &bytes[..4]
                    );
                }
            }
            let marker_path = markers_path_for(&saved);
            assert!(marker_path.exists(), "marker file written next to {}", saved.display());
            assert_eq!(fs::read_to_string(&marker_path).unwrap(), "00:00.500  Start\n00:01.500  Marker 2\n");
        }
    }

    #[cfg(all(target_os = "linux", linux_ffmpeg_video))]
    #[test]
    fn in_process_audio_conversion_works_without_ffmpeg() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source.m4a");
        fs::write(&src, test_m4a(1)).unwrap();

        for format in [RecordingFormat::Wav, RecordingFormat::Flac, RecordingFormat::Mp3] {
            let dest = dir.path().join(format!("out.{}", format.as_str()));
            convert_audio_in_process(&src, &dest, format)
                .unwrap_or_else(|| panic!("in-process conversion to {} should succeed", format.as_str()));
            let bytes = fs::read(&dest).unwrap();
            assert!(bytes.len() > 100);
        }
    }
}
