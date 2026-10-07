// SPDX-License-Identifier: GPL-3.0-or-later
//! Voice recordings received from a paired phone (docs/protocol/recorder.md):
//! naming by date and time without overwriting, optional Media Foundation
//! conversion (`M4A` kept as recorded, `MP3`, `WAV`, or `FLAC`), marker sidecar
//! files (`<stem>.markers.txt`), and the Windows notification on completion.

#![allow(unsafe_code)]

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Instant, SystemTime},
};

use nectarlink_core::{RecordingMarker, Transfer, TransferState};
use windows::{
    Win32::{
        Media::MediaFoundation::{
            IMFAttributes, IMFMediaType, IMFSample, IMFSourceReader, MF_MT_AUDIO_AVG_BYTES_PER_SECOND,
            MF_MT_AUDIO_BITS_PER_SAMPLE, MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND,
            MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_SOURCE_READER_FIRST_AUDIO_STREAM,
            MF_SOURCE_READERF_ENDOFSTREAM, MF_TRANSCODE_CONTAINERTYPE, MF_VERSION, MFAudioFormat_FLAC,
            MFAudioFormat_MP3, MFAudioFormat_PCM, MFCreateAttributes, MFCreateMediaType,
            MFCreateSinkWriterFromURL, MFCreateSourceReaderFromURL, MFMediaType_Audio, MFSTARTUP_NOSOCKET,
            MFShutdown, MFStartup, MFT_ENUM_FLAG_ALL, MFTranscodeContainerType_MP3,
            MFTranscodeGetAudioOutputAvailableTypes,
        },
        System::{
            Com::{COINIT_MULTITHREADED, CoInitializeEx},
            SystemInformation::GetLocalTime,
        },
    },
    core::{HSTRING, Interface},
};

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
pub fn current_recording_stem() -> String {
    // SAFETY: GetLocalTime always succeeds and writes a SYSTEMTIME struct.
    let st = unsafe { GetLocalTime() };
    format_recording_stem(st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute)
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
        .expect("a free recording file name exists")
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

struct MfSession;

impl MfSession {
    fn start() -> windows::core::Result<Self> {
        // SAFETY: plain COM and Media Foundation initialization for this worker thread.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET)?;
        }
        Ok(Self)
    }
}

impl Drop for MfSession {
    fn drop(&mut self) {
        // SAFETY: balances MFStartup in MfSession::start.
        unsafe {
            let _ = MFShutdown();
        }
    }
}

/// Converts `src` (`.m4a`) to `dest` in `format` (`Mp3`, `Wav`, or `Flac`)
/// using Windows Media Foundation.
pub fn convert_audio(src: &Path, dest: &Path, format: RecordingFormat) -> windows::core::Result<()> {
    let _mf = MfSession::start()?;
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
        core_host::host().hub.update(|s| s.update_transfer(updated, Instant::now()));

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
            title: format!("Recording from {device} saved"),
            body,
            attribution: "Nectarlink".into(),
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
    }

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
}
