// SPDX-License-Identifier: GPL-3.0-or-later
//! Linux Virtual Camera (`v4l2loopback`) discovery, registration, and
//! shared-memory-to-V4L2 YUYV frame output for `"NectarLink Webcam"`.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::fd::AsRawFd,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use nectarlink_vcam::{DEFAULT_MAPPING_NAME, LOCAL_MAPPING_NAME, SharedFrameMapping};

pub const CLSID_STR: &str = "{8E6C3B74-5D4A-4B9E-9A12-7C8F1E2D3A40}";
pub const FRIENDLY_NAME: &str = "NectarLink Webcam";
pub const REGISTER_ARG: &str = "--register-vcam";
pub const UNREGISTER_ARG: &str = "--unregister-vcam";

const MODULES_LOAD_CONF: &str = "/etc/modules-load.d/nectarlink-vcam.conf";
const MODPROBE_CONF: &str = "/etc/modprobe.d/nectarlink-vcam.conf";

const V4L2_BUF_TYPE_VIDEO_OUTPUT: u32 = 2;
const V4L2_PIX_FMT_YUYV: u32 = 0x5659_5559; // 'Y' 'U' 'Y' 'V'
const V4L2_FIELD_NONE: u32 = 1;
const V4L2_COLORSPACE_SMPTE170M: u32 = 1;
/// `_IOWR('V', 5, struct v4l2_format)` where `sizeof(struct v4l2_format) == 208` (`0xD0`).
const VIDIOC_S_FMT: libc::c_ulong = 0xC0D0_5605;

#[repr(C)]
#[derive(Clone, Copy)]
struct V4l2PixFormat {
    width: u32,
    height: u32,
    pixelformat: u32,
    field: u32,
    bytesperline: u32,
    sizeimage: u32,
    colorspace: u32,
    priv_: u32,
    flags: u32,
    ycbcr_enc: u32,
    quantization: u32,
    xfer_func: u32,
}

#[repr(C)]
struct V4l2Format {
    type_: u32,
    _pad: u32,
    pix: V4l2PixFormat,
    _rest: [u8; 200 - std::mem::size_of::<V4l2PixFormat>()],
}

/// Locates a `v4l2loopback` `/dev/videoN` device node by inspecting `/sys/class/video4linux`.
pub fn find_v4l2loopback_device() -> Option<PathBuf> {
    find_v4l2loopback_in_sysfs(Path::new("/sys"), Path::new("/dev"))
}

/// Testable helper that scans `<sysfs_root>/class/video4linux/video*` and `<dev_root>/video*`.
pub fn find_v4l2loopback_in_sysfs(sysfs_root: &Path, dev_root: &Path) -> Option<PathBuf> {
    let v4l_dir = sysfs_root.join("class/video4linux");
    if let Ok(entries) = fs::read_dir(&v4l_dir) {
        let mut candidates: Vec<(u8, String, PathBuf)> = Vec::new();
        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let dev_name = file_name.to_string_lossy().into_owned();
            if !dev_name.starts_with("video") {
                continue;
            }
            let dev_node = dev_root.join(&dev_name);
            if !dev_node.exists() {
                continue;
            }
            let Ok(card_name) = fs::read_to_string(entry.path().join("name")) else {
                continue;
            };
            let lower = card_name.to_ascii_lowercase();
            if lower.contains("nectarlink") {
                candidates.push((0, dev_name, dev_node));
            } else if lower.contains("dummy video device") || lower.contains("v4l2loopback") {
                candidates.push((1, dev_name, dev_node));
            }
        }
        candidates.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        if let Some((_, _, node)) = candidates.into_iter().next() {
            return Some(node);
        }
    }

    if sysfs_root.join("module/v4l2loopback").exists() {
        for idx in 0..16 {
            let dev = dev_root.join(format!("video{idx}"));
            if dev.exists() {
                return Some(dev);
            }
        }
    }
    None
}

/// Returns the active `v4l2loopback` device node, module sysfs directory, or `modprobe` binary
/// so the desktop UI knows virtual camera setup is supported on this Linux host.
pub fn dll_path() -> Option<PathBuf> {
    if let Some(dev) = find_v4l2loopback_device() {
        return Some(dev);
    }
    let sys_mod = PathBuf::from("/sys/module/v4l2loopback");
    if sys_mod.exists() {
        return Some(sys_mod);
    }
    for candidate in ["/sbin/modprobe", "/usr/sbin/modprobe", "/bin/modprobe", "/usr/bin/modprobe"] {
        let p = PathBuf::from(candidate);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// Returns `true` if `v4l2loopback` is currently loaded in the kernel or exposes a loopback node.
pub fn is_registered() -> bool {
    find_v4l2loopback_device().is_some() || PathBuf::from("/sys/module/v4l2loopback").exists()
}

/// Ensures `v4l2loopback` is registered, attempting `register_hklm()` when not already active.
pub fn ensure_registered() -> bool {
    if is_registered() {
        return true;
    }
    register_hklm().is_ok() && is_registered()
}

fn run_modprobe(args: &[&str]) -> Result<(), String> {
    let bins = ["/sbin/modprobe", "/usr/sbin/modprobe", "modprobe"];
    let mut last_err = String::from("modprobe not found");
    for bin in bins {
        match Command::new(bin)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
        {
            Ok(out) if out.status.success() => return Ok(()),
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
                last_err =
                    if stderr.is_empty() { format!("{bin} exited with {}", out.status) } else { stderr };
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => last_err = e.to_string(),
        }
    }
    Err(last_err)
}

/// Loads `v4l2loopback` with `"NectarLink Webcam"` and writes persistent module config files.
pub fn register_hklm() -> Result<(), String> {
    if let Some(parent) = Path::new(MODULES_LOAD_CONF).parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(MODULES_LOAD_CONF, "v4l2loopback\n");

    if let Some(parent) = Path::new(MODPROBE_CONF).parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(
        MODPROBE_CONF,
        format!("options v4l2loopback exclusive_caps=1 card_label=\"{FRIENDLY_NAME}\"\n"),
    );

    run_modprobe(&["v4l2loopback", "exclusive_caps=1", &format!("card_label={FRIENDLY_NAME}")])
}

/// Removes persistent `v4l2loopback` config files and unloads the kernel module.
pub fn unregister_hklm() -> Result<(), String> {
    let _ = fs::remove_file(MODULES_LOAD_CONF);
    let _ = fs::remove_file(MODPROBE_CONF);
    run_modprobe(&["-r", "v4l2loopback"])
}

/// Launches `nectarlink-desktop <arg>` under `pkexec` and invokes `on_done` when it exits.
pub fn run_elevated(arg: &'static str, on_done: impl FnOnce(bool) + Send + 'static) {
    thread::spawn(move || {
        let Ok(exe) = std::env::current_exe() else {
            on_done(false);
            return;
        };
        let ok = super::elevated::run_elevated(&exe, &[arg]).is_ok_and(|code| code == 0);
        on_done(ok);
    });
}

/// Converts top-down BGRX (`width * height * 4`) to packed YUYV 4:2:2 (`even_width * height * 2`)
/// using BT.601 studio-swing integer math.
pub fn bgrx_to_yuyv(bgrx: &[u8], width: u32, height: u32, yuyv: &mut Vec<u8>) {
    let w = width as usize;
    let h = height as usize;
    if w == 0 || h == 0 || bgrx.len() < w.saturating_mul(h).saturating_mul(4) {
        yuyv.clear();
        return;
    }
    let even_w = (w + 1) & !1;
    let total_bytes = even_w * h * 2;
    yuyv.resize(total_bytes, 0);

    for y in 0..h {
        let src_row = &bgrx[y * w * 4..(y + 1) * w * 4];
        let dst_row = &mut yuyv[y * even_w * 2..(y + 1) * even_w * 2];
        for pair in 0..(even_w / 2) {
            let x0 = pair * 2;
            let x1 = (x0 + 1).min(w - 1);

            let b0 = i32::from(src_row[x0 * 4]);
            let g0 = i32::from(src_row[x0 * 4 + 1]);
            let r0 = i32::from(src_row[x0 * 4 + 2]);

            let b1 = i32::from(src_row[x1 * 4]);
            let g1 = i32::from(src_row[x1 * 4 + 1]);
            let r1 = i32::from(src_row[x1 * 4 + 2]);

            let y0 = (((66 * r0 + 129 * g0 + 25 * b0 + 128) >> 8) + 16).clamp(16, 235) as u8;
            let y1 = (((66 * r1 + 129 * g1 + 25 * b1 + 128) >> 8) + 16).clamp(16, 235) as u8;

            let r_avg = (r0 + r1) >> 1;
            let g_avg = (g0 + g1) >> 1;
            let b_avg = (b0 + b1) >> 1;

            let u = (((-38 * r_avg - 74 * g_avg + 112 * b_avg + 128) >> 8) + 128).clamp(16, 240) as u8;
            let v = (((112 * r_avg - 94 * g_avg - 18 * b_avg + 128) >> 8) + 128).clamp(16, 240) as u8;

            let dst_off = pair * 4;
            dst_row[dst_off] = y0;
            dst_row[dst_off + 1] = u;
            dst_row[dst_off + 2] = y1;
            dst_row[dst_off + 3] = v;
        }
    }
}

/// Active Linux `v4l2loopback` feeder handle.
pub struct VirtualCameraHandle {
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl std::fmt::Debug for VirtualCameraHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VirtualCameraHandle").field("running", &!self.stop.load(Ordering::Relaxed)).finish()
    }
}

impl VirtualCameraHandle {
    /// Spawns the `"vcam-v4l2"` background worker that reads BGRX frames from
    /// [`SharedFrameMapping`] and writes YUYV frames into the `v4l2loopback` device node.
    pub fn start() -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_worker = stop.clone();
        let worker = thread::Builder::new()
            .name("vcam-v4l2".into())
            .spawn(move || v4l2_worker_loop(&stop_worker))
            .map_err(|e| format!("spawn vcam-v4l2 thread: {e}"))?;
        Ok(Self { stop, worker: Some(worker) })
    }
}

impl Drop for VirtualCameraHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn configure_v4l2_yuyv(fd: i32, width: u32, height: u32) -> bool {
    let even_w = (width + 1) & !1;
    let stride = even_w * 2;
    let sizeimage = stride * height;
    let mut fmt = V4l2Format {
        type_: V4L2_BUF_TYPE_VIDEO_OUTPUT,
        _pad: 0,
        pix: V4l2PixFormat {
            width: even_w,
            height,
            pixelformat: V4L2_PIX_FMT_YUYV,
            field: V4L2_FIELD_NONE,
            bytesperline: stride,
            sizeimage,
            colorspace: V4L2_COLORSPACE_SMPTE170M,
            priv_: 0,
            flags: 0,
            ycbcr_enc: 0,
            quantization: 0,
            xfer_func: 0,
        },
        _rest: [0u8; 200 - std::mem::size_of::<V4l2PixFormat>()],
    };
    // SAFETY: `VIDIOC_S_FMT` is called on a valid V4L2 device file descriptor with a properly
    // sized `v4l2_format` structure.
    unsafe { libc::ioctl(fd, VIDIOC_S_FMT, &raw mut fmt) == 0 }
}

fn v4l2_worker_loop(stop: &AtomicBool) {
    const OUT_W: u32 = 1280;
    const OUT_H: u32 = 720;

    let mut mapping: Option<SharedFrameMapping> = None;
    let mut v4l2_file: Option<fs::File> = None;
    let mut configured_dims: Option<(u32, u32)> = None;
    let mut bgrx = Vec::new();
    let mut yuyv = Vec::new();
    let mut last_ts: Option<(bool, u64)> = None;

    while !stop.load(Ordering::Acquire) {
        if mapping.is_none() {
            mapping = SharedFrameMapping::open_existing(DEFAULT_MAPPING_NAME)
                .or_else(|_| SharedFrameMapping::open_existing(LOCAL_MAPPING_NAME))
                .ok();
        }

        if v4l2_file.is_none()
            && let Some(dev_path) = find_v4l2loopback_device()
            && let Ok(file) = OpenOptions::new().read(true).write(true).open(&dev_path)
        {
            v4l2_file = Some(file);
            configured_dims = None;
            last_ts = None;
        }

        let (Some(map), Some(dev)) = (mapping.as_ref(), v4l2_file.as_mut()) else {
            thread::sleep(Duration::from_millis(50));
            continue;
        };

        if bgrx.len() != (OUT_W * OUT_H * 4) as usize {
            bgrx.resize((OUT_W * OUT_H * 4) as usize, 0);
        }

        let state = map.read_bgrx(OUT_W, OUT_H, &mut bgrx);
        if state.0 && last_ts == Some(state) {
            thread::sleep(Duration::from_millis(8));
            continue;
        }
        last_ts = Some(state);

        if configured_dims != Some((OUT_W, OUT_H)) {
            let _ = configure_v4l2_yuyv(dev.as_raw_fd(), OUT_W, OUT_H);
            configured_dims = Some((OUT_W, OUT_H));
        }

        bgrx_to_yuyv(&bgrx, OUT_W, OUT_H, &mut yuyv);
        if dev.write_all(&yuyv).is_err() {
            v4l2_file = None;
            configured_dims = None;
            thread::sleep(Duration::from_millis(50));
        } else if !state.0 {
            thread::sleep(Duration::from_millis(33));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v4l2_format_struct_has_kernel_abi_size() {
        assert_eq!(std::mem::size_of::<V4l2Format>(), 208);
    }

    #[test]
    fn bgrx_to_yuyv_converts_colors_and_rounds_odd_widths() {
        // 2x1 pure black and pure white pixels in BGRX.
        let bgrx = [
            0, 0, 0, 255, // black
            255, 255, 255, 255, // white
        ];
        let mut yuyv = Vec::new();
        bgrx_to_yuyv(&bgrx, 2, 1, &mut yuyv);
        assert_eq!(yuyv.len(), 4);
        assert_eq!(yuyv[0], 16, "black Y should be studio-swing 16");
        assert_eq!(yuyv[2], 235, "white Y should be studio-swing 235");
        assert!((126..=130).contains(&yuyv[1]), "neutral U should be ~128, got {}", yuyv[1]);
        assert!((126..=130).contains(&yuyv[3]), "neutral V should be ~128, got {}", yuyv[3]);

        // Odd width (3x2) rounds up to 4x2 -> 16 bytes of YUYV.
        let odd_bgrx = vec![0x80u8; 3 * 2 * 4];
        bgrx_to_yuyv(&odd_bgrx, 3, 2, &mut yuyv);
        assert_eq!(yuyv.len(), 4 * 2 * 2);
    }

    #[test]
    fn find_v4l2loopback_in_sysfs_prefers_nectarlink_and_matches_loopback() {
        let dir = tempfile::tempdir().unwrap();
        let sysfs = dir.path().join("sys");
        let dev = dir.path().join("dev");
        let v0 = sysfs.join("class/video4linux/video0");
        let v1 = sysfs.join("class/video4linux/video1");
        fs::create_dir_all(&v0).unwrap();
        fs::create_dir_all(&v1).unwrap();
        fs::create_dir_all(&dev).unwrap();

        fs::write(v0.join("name"), "Integrated Camera: USB\n").unwrap();
        fs::write(v1.join("name"), "NectarLink Webcam\n").unwrap();
        fs::write(dev.join("video0"), b"").unwrap();
        fs::write(dev.join("video1"), b"").unwrap();

        let found = find_v4l2loopback_in_sysfs(&sysfs, &dev);
        assert_eq!(found, Some(dev.join("video1")));
    }

    #[test]
    fn virtual_camera_handle_starts_and_drops_cleanly() {
        let handle = VirtualCameraHandle::start().expect("should spawn vcam-v4l2 worker");
        drop(handle);
    }
}
