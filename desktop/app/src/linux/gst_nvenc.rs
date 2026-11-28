// SPDX-License-Identifier: GPL-3.0-or-later
//! Wayland HEVC capture on NVIDIA, isolated from the desktop process.
#![allow(unsafe_code)]

use std::{
    io::{Read, Write},
    os::{
        fd::{AsRawFd, OwnedFd},
        unix::process::CommandExt,
    },
    path::Path,
    process::{Child, ChildStdin, Command, ExitCode, Stdio},
    sync::mpsc::{self, Receiver, TryRecvError},
    time::{Duration, Instant},
};

const WORKER: &str = "--pc-mirror-nvenc-worker";
const MAX_PACKET: usize = 8 * 1024 * 1024;

unsafe extern "C" {
    fn nectarlink_run_nvenc_worker(fd: i32, node: u32, w: u32, h: u32, fps: u32, bitrate: u32) -> i32;
}

pub(crate) fn worker_main() -> Option<ExitCode> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(WORKER.as_ref()) {
        return None;
    }
    let Ok([node, w, h, fps, bitrate]): Result<[u32; 5], _> = args
        .map(|v| v.to_str().and_then(|s| s.parse::<u32>().ok()))
        .collect::<Option<Vec<_>>>()
        .unwrap_or_default()
        .try_into()
    else {
        return Some(ExitCode::FAILURE);
    };
    if !(10..=60).contains(&fps)
        || !(64..=1920).contains(&w)
        || !(64..=1920).contains(&h)
        || !(500_000..=20_000_000).contains(&bitrate)
    {
        return Some(ExitCode::FAILURE);
    }
    // The desktop passes the portal connection as fd 3.
    Some(if unsafe { nectarlink_run_nvenc_worker(3, node, w, h, fps, bitrate) } == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

pub(crate) fn enabled() -> bool {
    std::env::var_os("NECTARLINK_DISABLE_WAYLAND_NVENC").as_deref() != Some("1".as_ref())
        && nvidia_outputs(Path::new("/sys/class/drm"))
}

// Don't assume a portal stream comes from NVIDIA on a mixed-GPU desktop.
fn nvidia_outputs(drm_root: &Path) -> bool {
    let Ok(drm) = std::fs::read_dir(drm_root) else { return false };
    let mut found = false;
    for entry in drm.flatten() {
        let name = entry.file_name();
        let Some((card, _)) = name.to_str().and_then(|s| s.split_once('-')) else { continue };
        if std::fs::read_to_string(entry.path().join("enabled")).is_ok_and(|s| s.trim() == "enabled") {
            let vendor = drm_root.join(card).join("device/vendor");
            if !std::fs::read_to_string(vendor).is_ok_and(|s| s.trim() == "0x10de") {
                return false;
            }
            found = true;
        }
    }
    found
}

type Packet = (u64, bool, Vec<u8>);

fn read_packet(mut input: impl Read) -> std::io::Result<Packet> {
    let mut header = [0; 13];
    input.read_exact(&mut header)?;
    let len = u32::from_ne_bytes(header[..4].try_into().unwrap()) as usize;
    if !(1..=MAX_PACKET).contains(&len) || header[4] > 1 {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    let mut bytes = vec![0; len];
    input.read_exact(&mut bytes)?;
    Ok((u64::from_ne_bytes(header[5..].try_into().unwrap()), header[4] != 0, bytes))
}

pub(crate) struct Capture {
    child: Child,
    control: ChildStdin,
    packets: Receiver<Packet>,
    last_packet: Instant,
    clock: Option<(u64, u64)>,
}

impl Capture {
    pub(crate) fn start(
        remote: OwnedFd,
        node: u32,
        (w, h): (u32, u32),
        fps: u32,
        bitrate: u32,
    ) -> std::io::Result<Self> {
        let fd = remote.as_raw_fd();
        let mut command = Command::new(std::env::current_exe()?);
        command
            .arg(WORKER)
            .args([node, w, h, fps, bitrate].map(|n| n.to_string()))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped());
        // SAFETY: only async-signal-safe calls run after fork.
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) < 0
                    || if fd == 3 { libc::fcntl(fd, libc::F_SETFD, 0) } else { libc::dup2(fd, 3) } < 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command.spawn()?;
        drop(remote);
        let mut output = child.stdout.take().unwrap();
        let control = child.stdin.take().unwrap();
        let (tx, packets) = mpsc::sync_channel(4);
        let init = (|| {
            // An encoder or driver may hang while processing the last command.
            // Never let a full control pipe stall the desktop.
            if unsafe { libc::fcntl(control.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) } < 0 {
                return Err(std::io::Error::last_os_error());
            }
            std::thread::Builder::new().name("nvenc-packets".into()).spawn(move || {
                while let Ok(packet) = read_packet(&mut output) {
                    if tx.send(packet).is_err() {
                        break;
                    }
                }
            })?;
            Ok(())
        })();
        if let Err(error) = init {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        Ok(Self { child, control, packets, last_packet: Instant::now(), clock: None })
    }

    pub(crate) fn command(&mut self, value: u32) -> bool {
        self.control.write_all(&value.to_ne_bytes()).is_ok()
    }

    pub(crate) fn read(&mut self, now_us: u64) -> Result<Vec<Packet>, String> {
        let mut out = Vec::new();
        for _ in 0..4 {
            match self.packets.try_recv() {
                Ok((pts, key, bytes)) => {
                    self.last_packet = Instant::now();
                    let (first_pts, start_us) = *self.clock.get_or_insert_with(|| {
                        tracing::info!("Wayland CUDA/NVENC capture is producing HEVC frames");
                        (pts, now_us)
                    });
                    out.push((start_us + pts.saturating_sub(first_pts) / 1000, key, bytes));
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Err("NVENC worker stopped".into()),
            }
        }
        if self.last_packet.elapsed() >= Duration::from_secs(3) {
            return Err("NVENC worker stopped supplying frames".into());
        }
        Ok(out)
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_active_nvidia_outputs_enable_nvenc() {
        use std::fs;

        let drm = tempfile::tempdir().unwrap();
        let root = drm.path();
        assert!(!nvidia_outputs(root));

        for (card, output, vendor, state) in [
            ("card0", "card0-DP-1", "0x10de\n", "enabled\n"),
            ("card1", "card1-DP-2", "0x1002\n", "disabled\n"),
        ] {
            fs::create_dir_all(root.join(card).join("device")).unwrap();
            fs::create_dir(root.join(output)).unwrap();
            fs::write(root.join(card).join("device/vendor"), vendor).unwrap();
            fs::write(root.join(output).join("enabled"), state).unwrap();
        }
        assert!(nvidia_outputs(root));

        fs::write(root.join("card1-DP-2/enabled"), "enabled\n").unwrap();
        assert!(!nvidia_outputs(root));
        fs::write(root.join("card0-DP-1/enabled"), "disabled\n").unwrap();
        assert!(!nvidia_outputs(root));
    }

    #[test]
    fn reads_one_bounded_access_unit() {
        let mut bytes = 3_u32.to_ne_bytes().to_vec();
        bytes.push(1);
        bytes.extend_from_slice(&500_u64.to_ne_bytes());
        bytes.extend_from_slice(&[1, 2, 3]);
        assert_eq!(read_packet(bytes.as_slice()).unwrap(), (500, true, vec![1, 2, 3]));
        assert!(read_packet(&bytes[..15]).is_err());
        bytes[..4].copy_from_slice(&u32::MAX.to_ne_bytes());
        assert!(read_packet(bytes.as_slice()).is_err());
    }
}
