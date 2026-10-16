use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub fn local_timestamp_filename(prefix: &str, ext: &str) -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let hour = rem / 3600;
    let min = (rem % 3600) / 60;
    let sec = rem % 60;
    let (year, month, day) = civil_from_days(days as i64);
    let clean_ext = ext.trim_start_matches('.');
    format!("{prefix}_{year:04}{month:02}{day:02}_{hour:02}{min:02}{sec:02}.{clean_ext}")
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

#[derive(Debug)]
pub struct MirrorRecorder {
    path: PathBuf,
    file: Option<File>,
    started_at: Instant,
    started_epoch_ms: i64,
    sps_pps: Vec<u8>,
    wrote_header: bool,
    frames_written: u64,
}

impl MirrorRecorder {
    pub fn new(
        path: PathBuf,
        _width: u32,
        _height: u32,
        _fps: u32,
        _audio_cfg: Option<(u32, u8)>,
    ) -> MirrorRecorder {
        let started_epoch_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        MirrorRecorder {
            path,
            file: None,
            started_at: Instant::now(),
            started_epoch_ms,
            sps_pps: Vec::new(),
            wrote_header: false,
            frames_written: 0,
        }
    }

    pub fn started_at(&self) -> Instant {
        self.started_at
    }

    pub fn started_epoch_ms(&self) -> i64 {
        self.started_epoch_ms
    }

    pub fn set_sps_pps(&mut self, sps_pps: &[u8]) {
        if !sps_pps.is_empty() {
            merge_sps_pps(&mut self.sps_pps, sps_pps);
        }
    }

    pub fn on_config(&mut self, _width: u32, _height: u32) {}

    pub fn on_audio_config(&mut self, _rate: u32, _channels: u8) {}

    pub fn on_packet(&mut self, keyframe: bool, _time_us: u64, data: &[u8]) -> Result<(), String> {
        if data.is_empty() {
            return Ok(());
        }
        merge_sps_pps(&mut self.sps_pps, data);
        if self.file.is_none() {
            if let Some(parent) = self.path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let file = File::create(&self.path)
                .map_err(|e| format!("create {}: {e}", self.path.display()))?;
            self.file = Some(file);
        }
        let Some(file) = self.file.as_mut() else {
            return Ok(());
        };
        if (!self.wrote_header || keyframe) && !self.sps_pps.is_empty() {
            file.write_all(&self.sps_pps)
                .map_err(|e| e.to_string())?;
            self.wrote_header = true;
        }
        file.write_all(data).map_err(|e| e.to_string())?;
        self.frames_written += 1;
        Ok(())
    }

    pub fn on_audio(&mut self, _time_us: u64, _pcm: &[u8]) -> Result<(), String> {
        Ok(())
    }

    pub fn finish(mut self) -> Result<PathBuf, String> {
        if let Some(mut file) = self.file.take()
            && self.frames_written > 0
        {
            let _ = file.flush();
            return Ok(self.path);
        }
        let _ = fs::remove_file(&self.path);
        Err("No video frames were recorded yet.".into())
    }
}

pub fn merge_sps_pps(sps_pps: &mut Vec<u8>, data: &[u8]) {
    if data.len() < 5 {
        return;
    }
    let nal_type = find_first_nal_type(data);
    if matches!(nal_type, Some(7 | 8 | 32 | 33 | 34)) {
        sps_pps.clear();
        sps_pps.extend_from_slice(data);
    }
}

fn find_first_nal_type(data: &[u8]) -> Option<u8> {
    let mut i = 0;
    while i + 3 < data.len() {
        if data[i] == 0 && data[i + 1] == 0 {
            if data[i + 2] == 1 && i + 3 < data.len() {
                return Some(data[i + 3] & 0x1F);
            }
            if i + 4 < data.len() && data[i + 2] == 0 && data[i + 3] == 1 {
                return Some(data[i + 4] & 0x1F);
            }
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_filename_has_expected_prefix_and_extension() {
        let name = local_timestamp_filename("NectarLink_Rec", "mp4");
        assert!(name.starts_with("NectarLink_Rec_"));
        assert!(name.ends_with(".mp4"));
    }

    #[test]
    fn merge_sps_pps_captures_sps_nal_units() {
        let mut buf = Vec::new();
        let sps = [0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1f];
        merge_sps_pps(&mut buf, &sps);
        assert_eq!(buf, sps);

        let idr = [0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84];
        merge_sps_pps(&mut buf, &idr);
        assert_eq!(buf, sps);
    }
}
