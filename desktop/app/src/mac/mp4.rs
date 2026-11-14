// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS MP4 recording stubs.

use std::{
    path::PathBuf,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

pub fn local_timestamp_filename(prefix: &str, ext: &str) -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    format!("{prefix} {now}.{ext}")
}

pub struct MirrorRecorder {
    path: PathBuf,
    started_at: Instant,
    started_epoch_ms: i64,
}

impl std::fmt::Debug for MirrorRecorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MirrorRecorder").field("path", &self.path).finish_non_exhaustive()
    }
}

impl MirrorRecorder {
    pub fn new(
        path: PathBuf,
        _width: u32,
        _height: u32,
        _fps: u32,
        _audio_cfg: Option<(u32, u8)>,
    ) -> MirrorRecorder {
        let started_epoch_ms =
            SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
        MirrorRecorder { path, started_at: Instant::now(), started_epoch_ms }
    }

    pub fn started_at(&self) -> Instant {
        self.started_at
    }

    pub fn started_epoch_ms(&self) -> i64 {
        self.started_epoch_ms
    }

    pub fn set_sps_pps(&mut self, _sps_pps: &[u8]) {}

    pub fn on_config(&mut self, _width: u32, _height: u32) {}

    pub fn set_codec(&mut self, _codec: VideoCodec) {}

    pub fn on_audio_config(&mut self, _rate: u32, _channels: u8) {}

    pub fn on_packet(&mut self, _keyframe: bool, _time_us: u64, _data: &[u8]) -> Result<(), String> {
        Err("screen recording is not supported on macOS".into())
    }

    pub fn on_audio(&mut self, _time_us: u64, _pcm: &[u8]) -> Result<(), String> {
        Err("audio recording is not supported on macOS".into())
    }

    pub fn finish(self) -> Result<PathBuf, String> {
        Err("screen recording is not supported on macOS".into())
    }
}

/// The video codec of a mirror stream.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VideoCodec {
    #[default]
    H264,
    Hevc,
}

impl VideoCodec {
    /// From a stream's codec name ("h264", "h265").
    pub fn from_name(name: &str) -> VideoCodec {
        if name.eq_ignore_ascii_case("h265") || name.eq_ignore_ascii_case("hevc") {
            VideoCodec::Hevc
        } else {
            VideoCodec::H264
        }
    }
}

/// Screen recording isn't available on macOS.
pub fn can_record(_codec: VideoCodec) -> bool {
    false
}

pub fn merge_sps_pps(_codec: VideoCodec, sps_pps: &mut Vec<u8>, data: &[u8]) {
    sps_pps.extend_from_slice(data);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mp4_recorder_reports_unsupported() {
        let mut rec = MirrorRecorder::new(PathBuf::from("/tmp/dummy.mp4"), 1920, 1080, 60, None);
        assert!(rec.on_packet(true, 0, &[1, 2, 3]).is_err());
        assert!(rec.on_audio(0, &[1, 2, 3]).is_err());
        assert!(rec.finish().is_err());
    }

    #[test]
    fn local_timestamp_filename_formats_correctly() {
        let name = local_timestamp_filename("Screen", "mp4");
        assert!(name.starts_with("Screen "));
        assert!(name.ends_with(".mp4"));
    }
}
