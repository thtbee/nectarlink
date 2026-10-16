// SPDX-License-Identifier: GPL-3.0-or-later
//! Running a process with root privileges on Linux via `pkexec` (Polkit).

use std::{
    path::Path,
    process::{Child, Command, Stdio},
    sync::Mutex,
};

/// Why starting the elevated process failed.
#[derive(Debug)]
pub enum Error {
    /// The user dismissed or declined the Polkit prompt (`pkexec` exit status 126 or 127).
    Declined,
    Other(String),
}

/// A running elevated child process.
#[derive(Debug)]
pub struct Process(Mutex<Child>);

unsafe extern "C" {
    fn geteuid() -> u32;
}

/// Returns `true` if the current process is already running as `root` (`euid == 0`).
pub fn is_elevated() -> bool {
    // SAFETY: `geteuid` is always safe and has no preconditions.
    unsafe { geteuid() == 0 }
}

fn split_params(params: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    for ch in params.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            c if c.is_whitespace() && !in_quotes => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Starts `program` with `params` under `pkexec` (or directly when already `root`),
/// without waiting for it to finish.
pub fn start(program: &Path, params: &str) -> Result<Process, Error> {
    let parsed = split_params(params);
    let mut cmd = if is_elevated() {
        let mut c = Command::new(program);
        c.args(&parsed);
        c
    } else {
        let mut c = Command::new("pkexec");
        c.arg(program).args(&parsed);
        c
    };
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    match cmd.spawn() {
        Ok(child) => Ok(Process(Mutex::new(child))),
        Err(e) => Err(Error::Other(e.to_string())),
    }
}

/// Runs `program` with `args` under `pkexec` and waits for its exit code.
pub fn run_elevated(program: &Path, args: &[&str]) -> Result<i32, String> {
    let mut cmd = if is_elevated() {
        let mut c = Command::new(program);
        c.args(args);
        c
    } else {
        let mut c = Command::new("pkexec");
        c.arg(program).args(args);
        c
    };
    let status = cmd.status().map_err(|e| e.to_string())?;
    Ok(status.code().unwrap_or(1))
}

impl Process {
    /// `None` while the process is still running; `Some(code)` once it has exited.
    pub fn exit_code(&self) -> Option<u32> {
        let mut child = self.0.lock().unwrap_or_else(|e| e.into_inner());
        match child.try_wait() {
            Ok(Some(status)) => Some(status.code().unwrap_or(1) as u32),
            Ok(None) => None,
            Err(_) => Some(1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_quoted_and_unquoted_params() {
        let args = split_params("--install-to \"/opt/NectarLink App\" --quiet");
        assert_eq!(args, vec!["--install-to", "/opt/NectarLink App", "--quiet"]);
    }
}
