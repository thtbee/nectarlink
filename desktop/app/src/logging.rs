// SPDX-License-Identifier: GPL-3.0-or-later
//! Logs go to `<data>\logs\nectarlink.log`, rotated at startup once large,
//! so a bug report can include them. Set `RUST_LOG` to change verbosity.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use tracing_subscriber::{EnvFilter, fmt, prelude::*};

const FILE_NAME: &str = "nectarlink.log";
const PREVIOUS_FILE_NAME: &str = "nectarlink.1.log";
const ROTATE_AT_BYTES: u64 = 4 * 1024 * 1024;
const DEFAULT_FILTER: &str = "info,iroh=warn,iroh_quinn=warn,iroh_quinn_proto=warn,swarm_discovery=warn";

pub fn logs_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("logs")
}

/// Starts logging. Failing to open the log file is not fatal: the app then
/// logs to stderr only.
pub fn init(data_dir: &Path) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    let file_layer = match open_log_file(&logs_dir(data_dir)) {
        Ok(file) => Some(fmt::layer().with_ansi(false).with_writer(Mutex::new(file))),
        Err(e) => {
            eprintln!("nectarlink: can't open the log file: {e}");
            None
        }
    };
    // Debug builds also log to the terminal.
    let stderr_layer = cfg!(debug_assertions).then(|| fmt::layer().with_writer(std::io::stderr));
    let _ = tracing_subscriber::registry().with(filter).with(file_layer).with(stderr_layer).try_init();
}

fn open_log_file(dir: &Path) -> std::io::Result<fs::File> {
    fs::create_dir_all(dir)?;
    let path = dir.join(FILE_NAME);
    if fs::metadata(&path).is_ok_and(|m| m.len() > ROTATE_AT_BYTES) {
        let _ = fs::rename(&path, dir.join(PREVIOUS_FILE_NAME));
    }
    fs::OpenOptions::new().create(true).append(true).open(path)
}
