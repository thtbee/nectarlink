// SPDX-License-Identifier: GPL-3.0-or-later
//! Logs go to `<data>\logs\nectarlink.log` and `nectarlink.1.log`, at most
//! [`SEGMENT_BYTES`] each, so a bug report can include them. The log keeps
//! names, messages, files, folders and addresses out by construction
//! (`nectarlink_core::diag`).
//!
//! `RUST_LOG` changes what's logged. `NECTARLINK_LOG_FULL=1` writes every
//! field as it is instead, for chasing a hard bug on your own PC; that log
//! may hold personal details, so check it before sharing it.

use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

use nectarlink_core::diag::{self, RingLayer, RingLog};
use tracing_subscriber::{EnvFilter, Registry, fmt, prelude::*, reload};

/// One of the two log files: days of normal use.
const SEGMENT_BYTES: u64 = 1024 * 1024;
/// The full log, which only trims itself at startup.
const FULL_ROTATE_AT_BYTES: u64 = 4 * 1024 * 1024;
/// mDNS warns several times a second for every network adapter it can't
/// reach (a disconnected VPN, Hyper-V's), which buried everything else.
const DEFAULT_FILTER: &str = "info,iroh=warn,iroh_quinn=warn,iroh_quinn_proto=warn,swarm_discovery=error";

/// What "Detailed log" adds: the USB cable, the connection and the call's sound, at the
/// level that says what each step did. Still within the private log's rules.
const DETAILED_FILTER: &str = ",nectarlink_core::usb=debug,nectarlink_adb=debug,nectarlink_core::session=debug,nectarlink_core::node=debug,nectarlink_desktop::call_audio=debug";

/// Changes what's logged while the app runs.
static FILTER: OnceLock<reload::Handle<EnvFilter, Registry>> = OnceLock::new();

/// `RUST_LOG` when it's set (a developer's choice stays), else the usual filter, with the
/// detailed one added on request.
fn filter_for(detailed: bool) -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(if detailed {
            format!("{DEFAULT_FILTER}{DETAILED_FILTER}")
        } else {
            DEFAULT_FILTER.to_owned()
        })
    })
}

/// Turns the detailed log on or off at once.
pub fn set_detailed(on: bool) {
    if let Some(handle) = FILTER.get() {
        let _ = handle.reload(filter_for(on));
    }
}

pub fn logs_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("logs")
}

/// Starts logging. Failing to open the log file is not fatal: the app then
/// logs to stderr only.
pub fn init(data_dir: &Path) {
    diag::init_roots(data_dir);
    let (filter, handle) =
        reload::Layer::new(filter_for(crate::settings::Settings::load(data_dir).detailed_log));
    let _ = FILTER.set(handle);
    let dir = logs_dir(data_dir);
    let full = std::env::var_os("NECTARLINK_LOG_FULL").is_some_and(|v| v == "1");
    let (ring_layer, full_layer) = if full {
        let layer = match open_full_log(&dir) {
            Ok(file) => Some(fmt::layer().with_ansi(false).with_writer(Mutex::new(file))),
            Err(e) => {
                eprintln!("nectarlink: can't open the log file: {e}");
                None
            }
        };
        (None, layer)
    } else {
        forget_old_logs(&dir);
        let layer = match RingLog::open(&dir, SEGMENT_BYTES) {
            Ok(ring) => Some(RingLayer::new(Arc::new(ring))),
            Err(e) => {
                eprintln!("nectarlink: can't open the log file: {e}");
                None
            }
        };
        (layer, None)
    };
    // Debug builds also log to the terminal.
    let stderr_layer = cfg!(debug_assertions).then(|| fmt::layer().with_writer(std::io::stderr));
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(ring_layer)
        .with(full_layer)
        .with(stderr_layer)
        .try_init();
}

/// How much of the log's end the viewer reads: the whole of both files at most.
const VIEW_BYTES: u64 = 2 * 1024 * 1024;
/// The most lines the viewer shows.
const VIEW_LINES: usize = 600;

/// The last lines of the log that contain `filter` (any case), oldest first, for the viewer.
pub fn tail(filter: &str) -> String {
    // Newest file first; the older one comes before it in the text.
    let mut text = String::new();
    for path in diag::ring::active_files().iter().rev() {
        let Ok(file) = fs::File::open(path) else { continue };
        let mut part = String::new();
        let _ = file.take(VIEW_BYTES).read_to_string(&mut part);
        text.push_str(&part);
    }
    last_lines(&text, filter, VIEW_LINES)
}

fn last_lines(text: &str, filter: &str, max: usize) -> String {
    let filter = filter.trim().to_lowercase();
    let mut lines: Vec<&str> =
        text.lines().filter(|l| filter.is_empty() || l.to_lowercase().contains(&filter)).collect();
    let skip = lines.len().saturating_sub(max);
    lines.drain(..skip);
    lines.join("\n")
}

/// Marks a logs folder that only holds the private log.
const PRIVATE_MARK: &str = ".private-log";

/// Deletes the log files of a version before the private log once, since
/// they may hold folders with the user's name and other details; and again
/// after a run with the full log.
fn forget_old_logs(dir: &Path) {
    let mark = dir.join(PRIVATE_MARK);
    if mark.exists() {
        return;
    }
    for name in [diag::ring::CURRENT, diag::ring::PREVIOUS] {
        let _ = fs::remove_file(dir.join(name));
    }
    let _ = fs::create_dir_all(dir);
    let _ = fs::write(mark, b"");
}

fn open_full_log(dir: &Path) -> std::io::Result<fs::File> {
    fs::create_dir_all(dir)?;
    let _ = fs::remove_file(dir.join(PRIVATE_MARK));
    let path = dir.join(diag::ring::CURRENT);
    if fs::metadata(&path).is_ok_and(|m| m.len() > FULL_ROTATE_AT_BYTES) {
        let _ = fs::rename(&path, dir.join(diag::ring::PREVIOUS));
    }
    fs::OpenOptions::new().create(true).append(true).open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_viewer_shows_the_last_matching_lines() {
        let text = "a one\nb two\nA three\nb four\n";
        assert_eq!(last_lines(text, "", 10), "a one\nb two\nA three\nb four");
        assert_eq!(last_lines(text, "A", 10), "a one\nA three");
        assert_eq!(last_lines(text, "b", 1), "b four");
        assert_eq!(last_lines(text, "none", 10), "");
    }

    #[test]
    fn logs_from_before_the_private_log_are_deleted_once() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join("logs");
        fs::create_dir_all(&logs).unwrap();
        fs::write(logs.join(diag::ring::CURRENT), r"adb=C:\Users\Alice\adb.exe").unwrap();
        fs::write(logs.join(diag::ring::PREVIOUS), "older").unwrap();
        forget_old_logs(&logs);
        assert!(!logs.join(diag::ring::CURRENT).exists());
        assert!(!logs.join(diag::ring::PREVIOUS).exists());
        // Afterwards the private log is kept from run to run.
        fs::write(logs.join(diag::ring::CURRENT), "new").unwrap();
        forget_old_logs(&logs);
        assert_eq!(fs::read_to_string(logs.join(diag::ring::CURRENT)).unwrap(), "new");
        // A run with the full log counts as old again.
        drop(open_full_log(&logs).unwrap());
        forget_old_logs(&logs);
        assert!(!logs.join(diag::ring::CURRENT).exists());
    }
}
