// SPDX-License-Identifier: GPL-3.0-or-later
//! MP4 recording for mirrored phone screens and app windows using Windows
//! Media Foundation's `IMFSinkWriter`.
//!
//! Video is muxed as H.264 passthrough (`MFVideoFormat_H264` on both input and
//! output streams of the sink writer) so zero video re-encoding occurs on the
//! PC. When phone audio (16-bit PCM) is present, it is encoded to AAC-LC
//! (`MFAudioFormat_AAC`) into the same `.mp4` container.

use std::{
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use windows::{
    Win32::{
        Media::MediaFoundation::{
            IMFMediaBuffer, IMFMediaType, IMFSample, IMFSinkWriter, MF_MT_ALL_SAMPLES_INDEPENDENT,
            MF_MT_AUDIO_AVG_BYTES_PER_SECOND, MF_MT_AUDIO_BITS_PER_SAMPLE, MF_MT_AUDIO_BLOCK_ALIGNMENT,
            MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE,
            MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_MPEG_SEQUENCE_HEADER,
            MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE, MFAudioFormat_AAC, MFAudioFormat_PCM, MFCreateMediaType,
            MFCreateMemoryBuffer, MFCreateSample, MFCreateSinkWriterFromURL, MFMediaType_Audio,
            MFMediaType_Video, MFSampleExtension_CleanPoint, MFVideoFormat_H264,
            MFVideoInterlace_Progressive,
        },
        System::SystemInformation::GetLocalTime,
    },
    core::HSTRING,
};

use super::h264::ensure_mf_started;

/// Formats `<prefix> YYYY-MM-DD HH.MM.SS.<ext>` using the PC's local clock.
pub fn local_timestamp_filename(prefix: &str, ext: &str) -> String {
    // SAFETY: GetLocalTime always succeeds and writes a SYSTEMTIME struct.
    let st = unsafe { GetLocalTime() };
    format!(
        "{prefix} {:04}-{:02}-{:02} {:02}.{:02}.{:02}.{ext}",
        st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond
    )
}

struct ActiveWriter {
    writer: IMFSinkWriter,
    video_stream: u32,
    audio_stream: Option<(u32, u32, u8)>,
    first_video_us: Option<u64>,
    last_video_hns: i64,
    video_frames_written: u64,
    first_audio_us: Option<u64>,
    last_audio_hns: i64,
    audio_samples_written: u64,
}

// SAFETY: Media Foundation runs in the multithreaded apartment (`COINIT_MULTITHREADED`
// via `ensure_mf_started`), and all access to `ActiveWriter` is serialized through `&mut self`.
unsafe impl Send for ActiveWriter {}

/// Records a mirrored H.264 stream (and optional PCM audio stream) into an `.mp4` file.
pub struct MirrorRecorder {
    path: PathBuf,
    width: u32,
    height: u32,
    fps: u32,
    audio_cfg: Option<(u32, u8)>,
    sps_pps: Vec<u8>,
    writer: Option<ActiveWriter>,
    started_at: Instant,
    started_epoch_ms: i64,
}

impl std::fmt::Debug for MirrorRecorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MirrorRecorder")
            .field("path", &self.path)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("active", &self.writer.is_some())
            .finish_non_exhaustive()
    }
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
            width,
            height,
            fps: fps.max(1),
            audio_cfg,
            sps_pps: Vec::new(),
            writer: None,
            started_at: Instant::now(),
            started_epoch_ms,
        }
    }

    pub fn started_at(&self) -> Instant {
        self.started_at
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

    /// Updates stream dimensions before the first keyframe initializes the writer.
    pub fn on_config(&mut self, width: u32, height: u32) {
        if self.writer.is_none() && width > 0 && height > 0 {
            self.width = width;
            self.height = height;
        }
    }

    /// Updates audio configuration before the first keyframe initializes the writer.
    pub fn on_audio_config(&mut self, rate: u32, channels: u8) {
        if self.writer.is_none() && rate > 0 && channels > 0 {
            self.audio_cfg = Some((rate, channels));
        }
    }

    /// Processes an incoming Annex-B H.264 access unit.
    pub fn on_packet(&mut self, keyframe: bool, time_us: u64, data: &[u8]) -> Result<(), String> {
        if data.is_empty() {
            return Ok(());
        }
        merge_sps_pps(&mut self.sps_pps, data);

        // If this packet only carries parameter sets (SPS/PPS) without a coded frame,
        // keep them for the upcoming keyframe without writing an empty video sample.
        if is_parameter_only(data) {
            return Ok(());
        }

        let is_keyframe = keyframe || has_idr_slice(data);

        if self.writer.is_none() {
            if !is_keyframe || self.width == 0 || self.height == 0 {
                return Ok(());
            }
            let active = ActiveWriter::open(
                &self.path,
                self.width,
                self.height,
                self.fps,
                &self.sps_pps,
                self.audio_cfg,
            )
            .map_err(|e| e.to_string())?;
            self.writer = Some(active);
        }

        let Some(w) = self.writer.as_mut() else {
            return Ok(());
        };

        ensure_mf_started().map_err(|e| e.to_string())?;
        let first_us = *w.first_video_us.get_or_insert(time_us);
        let mut hns = (time_us.saturating_sub(first_us) * 10) as i64;
        if w.video_frames_written > 0 && hns <= w.last_video_hns {
            hns = w.last_video_hns + 10_000;
        }
        w.last_video_hns = hns;
        let dur_hns = (10_000_000 / u64::from(self.fps.max(1))) as i64;

        let mut combined = Vec::new();
        let payload: &[u8] = if is_keyframe && !data_has_sps_pps(data) && !self.sps_pps.is_empty() {
            combined.reserve(self.sps_pps.len() + data.len());
            combined.extend_from_slice(&self.sps_pps);
            combined.extend_from_slice(data);
            &combined
        } else {
            data
        };

        write_sample(&w.writer, w.video_stream, payload, hns, dur_hns, is_keyframe)
            .map_err(|e| e.to_string())?;
        w.video_frames_written += 1;
        Ok(())
    }

    /// Processes an incoming 16-bit interleaved PCM audio packet.
    pub fn on_audio(&mut self, time_us: u64, pcm: &[u8]) -> Result<(), String> {
        if pcm.is_empty() {
            return Ok(());
        }
        let Some(w) = self.writer.as_mut() else {
            // Wait until the first video keyframe initializes the writer so audio and video start at t=0.
            return Ok(());
        };
        let Some((stream_idx, rate, channels)) = w.audio_stream else {
            return Ok(());
        };

        ensure_mf_started().map_err(|e| e.to_string())?;
        let first_us = *w.first_audio_us.get_or_insert(time_us);
        let mut hns = (time_us.saturating_sub(first_us) * 10) as i64;
        if w.audio_samples_written > 0 && hns <= w.last_audio_hns {
            hns = w.last_audio_hns + 1;
        }
        let bytes_per_frame = (usize::from(channels.max(1)) * 2).max(2);
        let frames = pcm.len() / bytes_per_frame;
        if frames == 0 {
            return Ok(());
        }
        let dur_hns = ((frames as u64) * 10_000_000 / u64::from(rate.max(1))).max(1) as i64;
        w.last_audio_hns = hns + dur_hns;

        write_sample(&w.writer, stream_idx, pcm, hns, dur_hns, false).map_err(|e| e.to_string())?;
        w.audio_samples_written += 1;
        Ok(())
    }

    /// Finalizes the MP4 container and flushes it to disk.
    pub fn finish(mut self) -> Result<PathBuf, String> {
        if let Some(w) = self.writer.take()
            && w.video_frames_written > 0
        {
            ensure_mf_started().map_err(|e| e.to_string())?;
            if let Some((stream_idx, rate, channels)) = w.audio_stream
                && w.audio_samples_written == 0
            {
                // Media Foundation's MP4 sink rejects Finalize() with MF_E_SINK_NO_SAMPLES_PROCESSED
                // if a declared stream received 0 samples; write a single 20 ms silent frame.
                let silence_len = (rate as usize / 50).max(1) * usize::from(channels.max(1)) * 2;
                let silence = vec![0u8; silence_len];
                let _ = write_sample(&w.writer, stream_idx, &silence, 0, 200_000, false);
            }
            // SAFETY: Finalize flushes and closes the MP4 sink writer.
            unsafe { w.writer.Finalize() }.map_err(|e| e.to_string())?;
            drop(w);
            return Ok(self.path);
        }
        let _ = std::fs::remove_file(&self.path);
        Err("No video frames were recorded yet.".into())
    }
}

impl ActiveWriter {
    fn open(
        path: &Path,
        width: u32,
        height: u32,
        fps: u32,
        sps_pps: &[u8],
        audio_cfg: Option<(u32, u8)>,
    ) -> windows::core::Result<ActiveWriter> {
        ensure_mf_started()?;
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::remove_file(path);

        // SAFETY: Media Foundation COM calls with owned parameters.
        unsafe {
            let writer: IMFSinkWriter =
                MFCreateSinkWriterFromURL(&HSTRING::from(path.as_os_str()), None, None)?;

            let video_out: IMFMediaType = MFCreateMediaType()?;
            video_out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            video_out.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
            video_out.SetUINT32(&MF_MT_AVG_BITRATE, 8_000_000)?;
            video_out.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            video_out.SetUINT64(&MF_MT_FRAME_SIZE, (u64::from(width) << 32) | u64::from(height))?;
            video_out.SetUINT64(&MF_MT_FRAME_RATE, (u64::from(fps.max(1)) << 32) | 1u64)?;
            video_out.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1u64)?;
            if !sps_pps.is_empty() {
                video_out.SetBlob(&MF_MT_MPEG_SEQUENCE_HEADER, sps_pps)?;
            }

            let video_stream = writer.AddStream(&video_out)?;
            writer.SetInputMediaType(video_stream, &video_out, None)?;

            let mut audio_stream = None;
            if let Some((rate, channels)) = audio_cfg
                && rate > 0
                && channels > 0
            {
                match add_aac_stream(&writer, rate, channels) {
                    Ok(idx) => audio_stream = Some((idx, rate, channels)),
                    Err(e) => {
                        tracing::warn!(error = %e, "couldn't add AAC audio stream to MP4; recording video only");
                    }
                }
            }

            writer.BeginWriting()?;

            Ok(ActiveWriter {
                writer,
                video_stream,
                audio_stream,
                first_video_us: None,
                last_video_hns: 0,
                video_frames_written: 0,
                first_audio_us: None,
                last_audio_hns: 0,
                audio_samples_written: 0,
            })
        }
    }
}

unsafe fn add_aac_stream(writer: &IMFSinkWriter, rate: u32, channels: u8) -> windows::core::Result<u32> {
    let ch = u32::from(channels);
    let block_align = ch * 2;
    // SAFETY: Media Foundation media type creation and stream setup.
    unsafe {
        let audio_out: IMFMediaType = MFCreateMediaType()?;
        audio_out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        audio_out.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
        audio_out.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
        audio_out.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, ch)?;
        audio_out.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        audio_out.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 20_000)?;

        let stream_idx = writer.AddStream(&audio_out)?;

        let audio_in: IMFMediaType = MFCreateMediaType()?;
        audio_in.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        audio_in.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
        audio_in.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
        audio_in.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, ch)?;
        audio_in.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        audio_in.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, block_align)?;
        audio_in.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, rate * block_align)?;
        audio_in.SetUINT32(&MF_MT_ALL_SAMPLES_INDEPENDENT, 1)?;

        writer.SetInputMediaType(stream_idx, &audio_in, None)?;
        Ok(stream_idx)
    }
}

fn write_sample(
    writer: &IMFSinkWriter,
    stream_idx: u32,
    bytes: &[u8],
    hns: i64,
    dur_hns: i64,
    clean_point: bool,
) -> windows::core::Result<()> {
    // SAFETY: buffer is sized to `bytes.len()`, locked while copied, and unlocked before WriteSample.
    unsafe {
        let buf: IMFMediaBuffer = MFCreateMemoryBuffer(bytes.len() as u32)?;
        let mut dst = std::ptr::null_mut();
        buf.Lock(&mut dst, None, None)?;
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, bytes.len());
        buf.Unlock()?;
        buf.SetCurrentLength(bytes.len() as u32)?;

        let sample: IMFSample = MFCreateSample()?;
        sample.AddBuffer(&buf)?;
        sample.SetSampleTime(hns)?;
        sample.SetSampleDuration(dur_hns.max(1))?;
        if clean_point {
            sample.SetUINT32(&MFSampleExtension_CleanPoint, 1)?;
        }
        writer.WriteSample(stream_idx, &sample)
    }
}

/// Splits an Annex-B buffer into `(nal_type, nal_payload_without_start_code)` slices.
fn annex_b_nals(data: &[u8]) -> Vec<(u8, &[u8])> {
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
            let payload = &data[payload_start..payload_end];
            out.push((payload[0] & 0x1F, payload));
        }
    }
    out
}

/// Extracts any SPS (NAL 7) and PPS (NAL 8) units from `data` and merges them
/// into `sps_pps` with 4-byte `00 00 00 01` Annex-B start codes.
pub fn merge_sps_pps(sps_pps: &mut Vec<u8>, data: &[u8]) {
    let nals = annex_b_nals(data);
    if !nals.iter().any(|&(t, _)| t == 7 || t == 8) {
        return;
    }
    let existing = annex_b_nals(sps_pps);
    let mut sps: Option<Vec<u8>> = existing.iter().find(|&&(t, _)| t == 7).map(|&(_, p)| p.to_vec());
    let mut pps: Option<Vec<u8>> = existing.iter().find(|&&(t, _)| t == 8).map(|&(_, p)| p.to_vec());
    for (t, payload) in nals {
        if t == 7 {
            sps = Some(payload.to_vec());
        } else if t == 8 {
            pps = Some(payload.to_vec());
        }
    }
    sps_pps.clear();
    if let Some(s) = sps {
        sps_pps.extend_from_slice(&[0, 0, 0, 1]);
        sps_pps.extend_from_slice(&s);
    }
    if let Some(p) = pps {
        sps_pps.extend_from_slice(&[0, 0, 0, 1]);
        sps_pps.extend_from_slice(&p);
    }
}

fn data_has_sps_pps(data: &[u8]) -> bool {
    let nals = annex_b_nals(data);
    nals.iter().any(|&(t, _)| t == 7) && nals.iter().any(|&(t, _)| t == 8)
}

fn has_idr_slice(data: &[u8]) -> bool {
    annex_b_nals(data).iter().any(|&(t, _)| t == 5)
}

fn is_parameter_only(data: &[u8]) -> bool {
    let nals = annex_b_nals(data);
    !nals.is_empty() && nals.iter().all(|&(t, _)| t == 7 || t == 8)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Splits an Annex-B stream into access units at its access unit delimiters (NAL type 9).
    fn access_units(stream: &[u8]) -> Vec<&[u8]> {
        let starts: Vec<usize> =
            (0..stream.len()).filter(|&i| stream[i..].starts_with(&[0, 0, 0, 1, 9])).collect();
        starts
            .iter()
            .enumerate()
            .map(|(n, &s)| &stream[s..*starts.get(n + 1).unwrap_or(&stream.len())])
            .collect()
    }

    fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn muxes_h264_passthrough_to_mp4() {
        let dir = tempfile::tempdir().unwrap();
        let mp4_path = dir.path().join("passthrough.mp4");
        let units = access_units(include_bytes!("testdata/testsrc.h264"));
        assert!(!units.is_empty());

        let mut rec = MirrorRecorder::new(mp4_path.clone(), 320, 240, 30, None);
        for (i, unit) in units.iter().enumerate() {
            let keyframe = i % 5 == 0;
            rec.on_packet(keyframe, (i as u64) * 33_333, unit).unwrap();
        }
        let saved = rec.finish().unwrap();
        assert_eq!(saved, mp4_path);
        let bytes = std::fs::read(&saved).unwrap();
        assert!(bytes.len() > 200, "unexpectedly small MP4: {} B", bytes.len());
        assert_eq!(&bytes[4..8], b"ftyp");
        assert!(contains_subslice(&bytes, b"moov"), "missing moov box");
        assert!(contains_subslice(&bytes, b"mdat"), "missing mdat box");
        assert!(contains_subslice(&bytes, b"avc1"), "missing avc1 sample entry");
        let _ = std::fs::remove_file(&saved);
    }

    #[test]
    fn muxes_h264_and_pcm_audio_to_mp4() {
        let dir = tempfile::tempdir().unwrap();
        let units = access_units(include_bytes!("testdata/testsrc.h264"));

        // 1. Video + 48 kHz stereo PCM audio frames.
        let mp4_with_audio = dir.path().join("with_audio.mp4");
        let mut rec = MirrorRecorder::new(mp4_with_audio.clone(), 320, 240, 30, Some((48_000, 2)));
        let pcm_20ms = vec![0u8; (48_000 / 50) * 2 * 2];
        for (i, unit) in units.iter().enumerate() {
            let t_us = (i as u64) * 33_333;
            rec.on_packet(i % 5 == 0, t_us, unit).unwrap();
            rec.on_audio(t_us, &pcm_20ms).unwrap();
        }
        let saved = rec.finish().unwrap();
        let bytes = std::fs::read(&saved).unwrap();
        assert!(bytes.len() > 200);
        assert_eq!(&bytes[4..8], b"ftyp");
        assert!(contains_subslice(&bytes, b"moov"));
        assert!(contains_subslice(&bytes, b"mdat"));
        assert!(contains_subslice(&bytes, b"avc1"));
        assert!(contains_subslice(&bytes, b"mp4a"));
        let _ = std::fs::remove_file(&saved);

        // 2. Audio stream declared (`Some((48_000, 2))`), but 0 audio packets arrive:
        // the silent-frame safeguard must still allow Finalize() to succeed cleanly.
        let mp4_silent_safeguard = dir.path().join("silent_safeguard.mp4");
        let mut rec2 = MirrorRecorder::new(mp4_silent_safeguard.clone(), 320, 240, 30, Some((48_000, 2)));
        for (i, unit) in units.iter().enumerate() {
            rec2.on_packet(i % 5 == 0, (i as u64) * 33_333, unit).unwrap();
        }
        let saved2 = rec2.finish().unwrap();
        let bytes2 = std::fs::read(&saved2).unwrap();
        assert!(bytes2.len() > 200);
        assert_eq!(&bytes2[4..8], b"ftyp");
        assert!(contains_subslice(&bytes2, b"moov"));
        assert!(contains_subslice(&bytes2, b"avc1"));
        assert!(contains_subslice(&bytes2, b"mp4a"));
        let _ = std::fs::remove_file(&saved2);
    }
}
