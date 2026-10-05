// SPDX-License-Identifier: GPL-3.0-or-later
//! What another launch of Nectarlink asks the running one to do: show its
//! window (Start menu, shortcuts), send files (Explorer's "Send to") or
//! quit (the installer, before replacing or removing the app).
//!
//! The launch writes a request file into the data folder, then wakes the
//! running instance (`win::single_instance`), which drains the folder. The
//! data folder is private to the user, and files survive the moment between
//! a first launch taking over and the app being ready to send.

use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

const DIR: &str = "requests";
/// Requests older than this were left by a launch whose instance went away
/// before handling them; acting on them later would surprise the user.
const MAX_AGE: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Request {
    /// Show the window.
    Show,
    /// Send files to a paired device.
    Send { device: String, paths: Vec<PathBuf> },
    /// Quit.
    Quit,
}

fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join(DIR)
}

/// Leaves a request for the running instance (or for this one, if it turns
/// out to be the first).
pub fn queue(data_dir: &Path, request: &Request) -> io::Result<()> {
    let dir = dir(data_dir);
    fs::create_dir_all(&dir)?;
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    // Sorts by time; the process ID keeps simultaneous launches apart.
    let name = format!("{nanos:024}-{}", std::process::id());
    let partial = dir.join(format!("{name}.tmp"));
    let mut file = fs::File::create(&partial)?;
    file.write_all(&serde_json::to_vec(request).map_err(io::Error::other)?)?;
    file.sync_all()?;
    drop(file);
    // Whole files only: the instance never reads one half-written.
    fs::rename(&partial, dir.join(format!("{name}.json")))
}

/// Takes the waiting requests, oldest first.
pub fn drain(data_dir: &Path) -> Vec<Request> {
    let Ok(entries) = fs::read_dir(dir(data_dir)) else { return Vec::new() };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    let now = SystemTime::now();
    files
        .into_iter()
        .filter_map(|path| {
            let fresh = fs::metadata(&path)
                .and_then(|m| m.modified())
                .is_ok_and(|t| now.duration_since(t).unwrap_or_default() <= MAX_AGE);
            let request = fs::read(&path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok());
            if let Err(e) = fs::remove_file(&path) {
                tracing::warn!(error = %e, "can't remove a handled request");
            }
            match request {
                Some(request) if fresh => Some(request),
                Some(_) => {
                    tracing::info!("dropped a stale request");
                    None
                }
                None => {
                    tracing::warn!("dropped an unreadable request");
                    None
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn send(n: u8) -> Request {
        Request::Send {
            device: format!("device{n}"),
            paths: vec![PathBuf::from(format!(r"C:\files\{n} é.txt"))],
        }
    }

    #[test]
    fn requests_come_back_once_in_order() {
        let data = tempfile::tempdir().unwrap();
        assert!(drain(data.path()).is_empty(), "nothing queued yet");
        queue(data.path(), &send(1)).unwrap();
        queue(data.path(), &Request::Show).unwrap();
        queue(data.path(), &send(2)).unwrap();
        assert_eq!(drain(data.path()), vec![send(1), Request::Show, send(2)]);
        assert!(drain(data.path()).is_empty(), "drained");
    }

    #[test]
    fn unreadable_and_partial_files_are_skipped() {
        let data = tempfile::tempdir().unwrap();
        let dir = dir(data.path());
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("1.json"), b"{not json").unwrap();
        fs::write(dir.join("2.tmp"), b"{}").unwrap();
        queue(data.path(), &Request::Show).unwrap();
        assert_eq!(drain(data.path()), vec![Request::Show]);
        assert!(!dir.join("1.json").exists(), "removed");
        assert!(dir.join("2.tmp").exists(), "a write in progress is left alone");
    }

    #[test]
    fn stale_requests_are_dropped() {
        let data = tempfile::tempdir().unwrap();
        queue(data.path(), &send(1)).unwrap();
        let path = fs::read_dir(dir(data.path())).unwrap().next().unwrap().unwrap().path();
        let old = SystemTime::now() - MAX_AGE - Duration::from_secs(5);
        fs::File::options().write(true).open(&path).unwrap().set_modified(old).unwrap();
        assert!(drain(data.path()).is_empty());
        assert!(!path.exists());
    }
}
