// SPDX-License-Identifier: GPL-3.0-or-later
//! Single-instance lock and wakeup on Linux using `flock(LOCK_EX | LOCK_NB)` on
//! `<data_dir>/instance.lock` and a Unix domain stream socket at `<data_dir>/instance.sock`.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::{
        fd::AsRawFd,
        unix::net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::Mutex,
};

const LOCK_FILE: &str = "instance.lock";
const SOCKET_FILE: &str = "instance.sock";

const LOCK_EX: i32 = 2;
const LOCK_NB: i32 = 4;
const LOCK_UN: i32 = 8;
const EAGAIN: i32 = 11;
const EWOULDBLOCK: i32 = 11;

unsafe extern "C" {
    fn flock(fd: i32, operation: i32) -> i32;
}

/// Held for the lifetime of the primary process; releasing it (on exit or crash)
/// releases the `flock` lock.
#[derive(Debug)]
pub struct PrimaryInstance {
    _lock_file: File,
    socket_path: PathBuf,
    listener: Mutex<Option<UnixListener>>,
}

#[derive(Debug)]
pub enum Instance {
    Primary(PrimaryInstance),
    Secondary,
}

fn lock_path(data_dir: &Path) -> PathBuf {
    data_dir.join(LOCK_FILE)
}

fn socket_path(data_dir: &Path) -> PathBuf {
    data_dir.join(SOCKET_FILE)
}

/// Acquires the single-instance lock for `data_dir`, or reports that another
/// instance already holds it.
pub fn acquire(data_dir: &Path) -> io::Result<Instance> {
    fs::create_dir_all(data_dir)?;
    let lock_file =
        OpenOptions::new().read(true).write(true).create(true).truncate(false).open(lock_path(data_dir))?;

    // SAFETY: `lock_file` is an open file descriptor owned by this process.
    let rc = unsafe { flock(lock_file.as_raw_fd(), LOCK_EX | LOCK_NB) };
    if rc != 0 {
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::WouldBlock
            || err.raw_os_error() == Some(EWOULDBLOCK)
            || err.raw_os_error() == Some(EAGAIN)
        {
            return Ok(Instance::Secondary);
        }
        return Err(err);
    }

    let sock_path = socket_path(data_dir);
    let _ = fs::remove_file(&sock_path);
    let listener = UnixListener::bind(&sock_path).ok();

    Ok(Instance::Primary(PrimaryInstance {
        _lock_file: lock_file,
        socket_path: sock_path,
        listener: Mutex::new(listener),
    }))
}

/// Signals the running primary instance for `data_dir` to wake up and drain queued requests.
pub fn wake(data_dir: &Path) {
    let sock_path = socket_path(data_dir);
    if let Ok(mut stream) = UnixStream::connect(sock_path) {
        let _ = stream.write_all(b"WAKE\n");
        let _ = stream.flush();
    }
}

impl PrimaryInstance {
    /// Calls `on_activate` on a background thread each time a secondary instance wakes us.
    pub fn watch(&'static self, on_activate: impl Fn() + Send + 'static) {
        let Some(listener) = self.listener.lock().unwrap_or_else(|e| e.into_inner()).take() else {
            return;
        };
        let _ = std::thread::Builder::new().name("single-instance".into()).spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(mut conn) => {
                        let mut buf = [0u8; 16];
                        let _ = conn.read(&mut buf);
                        on_activate();
                    }
                    Err(e) => {
                        tracing::debug!(error = %e, "single-instance socket accept failed");
                        break;
                    }
                }
            }
        });
    }
}

impl Drop for PrimaryInstance {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.socket_path);
        // SAFETY: unlocking our own file descriptor before close.
        unsafe {
            let _ = flock(self._lock_file.as_raw_fd(), LOCK_UN);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{Arc, mpsc},
        time::Duration,
    };

    #[test]
    fn second_instance_sees_primary_and_wakes_it() {
        let dir = tempfile::tempdir().unwrap();
        let primary = match acquire(dir.path()).unwrap() {
            Instance::Primary(p) => Box::leak(Box::new(p)),
            Instance::Secondary => panic!("first must be primary"),
        };
        assert!(matches!(acquire(dir.path()).unwrap(), Instance::Secondary));

        let (tx, rx) = mpsc::channel();
        let tx = Arc::new(Mutex::new(tx));
        primary.watch(move || {
            let _ = tx.lock().unwrap().send(());
        });

        wake(dir.path());
        rx.recv_timeout(Duration::from_secs(2)).expect("primary woken");
    }
}
