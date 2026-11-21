// SPDX-License-Identifier: MPL-2.0
//! The activity log on disk: two files of at most [`RingLog::open`]'s
//! `segment_bytes` each. When the current one is full it becomes the previous
//! one and a new one starts, so the log holds the latest one to two segments
//! of history and never more, however long the app runs.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

/// The file being written.
pub const CURRENT: &str = "nectarlink.log";
/// The file before it.
pub const PREVIOUS: &str = "nectarlink.1.log";

#[derive(Debug)]
pub struct RingLog {
    dir: PathBuf,
    segment_bytes: u64,
    state: Mutex<State>,
}

#[derive(Debug)]
struct State {
    file: Option<File>,
    written: u64,
}

impl RingLog {
    /// Opens the log in `dir`, carrying on in the current file unless it's
    /// already full.
    pub fn open(dir: &Path, segment_bytes: u64) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let log = RingLog {
            dir: dir.to_owned(),
            segment_bytes: segment_bytes.max(1),
            state: Mutex::new(State { file: None, written: 0 }),
        };
        let current = log.dir.join(CURRENT);
        let written = fs::metadata(&current).map(|m| m.len()).unwrap_or(0);
        let mut state = log.state.lock().unwrap_or_else(|e| e.into_inner());
        if written >= log.segment_bytes {
            log.rotate(&mut state)?;
        } else {
            state.file = Some(OpenOptions::new().create(true).append(true).open(current)?);
            state.written = written;
        }
        drop(state);
        Ok(log)
    }

    /// The two files, newest first.
    pub fn files(&self) -> [PathBuf; 2] {
        [self.dir.join(CURRENT), self.dir.join(PREVIOUS)]
    }

    /// Appends one line (`line` ends with a newline). A line that doesn't
    /// fit in what's left of the current file starts the next one; a single
    /// line longer than a whole file is cut to fit.
    pub fn append(&self, line: &[u8]) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let line = &line[..line.len().min(self.segment_bytes as usize)];
        if state.written > 0
            && state.written + line.len() as u64 > self.segment_bytes
            && self.rotate(&mut state).is_err()
        {
            return;
        }
        let Some(file) = state.file.as_mut() else { return };
        if file.write_all(line).is_ok() {
            state.written += line.len() as u64;
        }
    }

    /// Makes the current file the previous one and starts an empty one. If
    /// the rename fails (another program holding the file open), the current
    /// file is emptied instead, so the log still never grows past its size.
    fn rotate(&self, state: &mut State) -> io::Result<()> {
        state.file = None;
        state.written = 0;
        let current = self.dir.join(CURRENT);
        let _ = fs::rename(&current, self.dir.join(PREVIOUS));
        let file = OpenOptions::new().create(true).write(true).truncate(true).open(&current)?;
        state.file = Some(file);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(path: &Path) -> u64 {
        fs::metadata(path).map(|m| m.len()).unwrap_or(0)
    }

    #[test]
    fn the_log_never_grows_past_two_files() {
        let dir = tempfile::tempdir().unwrap();
        let log = RingLog::open(dir.path(), 100).unwrap();
        for i in 0..1000 {
            log.append(format!("line {i:04}\n").as_bytes());
        }
        let [current, previous] = log.files();
        assert!(size(&current) <= 100, "{}", size(&current));
        assert!(size(&previous) <= 100, "{}", size(&previous));
        let newest = fs::read_to_string(&current).unwrap();
        assert!(newest.ends_with("line 0999\n"), "{newest}");
        // Lines are never split across files.
        assert!(fs::read_to_string(&previous).unwrap().lines().all(|l| l.starts_with("line ")));
    }

    #[test]
    fn a_full_log_from_the_last_run_starts_a_new_file() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(CURRENT), vec![b'x'; 200]).unwrap();
        let log = RingLog::open(dir.path(), 100).unwrap();
        log.append(b"new\n");
        let [current, previous] = log.files();
        assert_eq!(fs::read_to_string(current).unwrap(), "new\n");
        assert_eq!(size(&previous), 200);
    }

    #[test]
    fn the_last_run_carries_on_in_the_same_file() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(CURRENT), "old\n").unwrap();
        let log = RingLog::open(dir.path(), 100).unwrap();
        log.append(b"new\n");
        assert_eq!(fs::read_to_string(&log.files()[0]).unwrap(), "old\nnew\n");
    }

    #[test]
    fn a_line_longer_than_a_file_is_cut() {
        let dir = tempfile::tempdir().unwrap();
        let log = RingLog::open(dir.path(), 10).unwrap();
        log.append(&[b'y'; 50]);
        assert_eq!(size(&log.files()[0]), 10);
    }
}
