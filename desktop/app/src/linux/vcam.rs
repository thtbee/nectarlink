use std::path::PathBuf;
use std::thread;

pub const CLSID_STR: &str = "{8E6C3B74-5D4A-4B9E-9A12-7C8F1E2D3A40}";
pub const FRIENDLY_NAME: &str = "NectarLink Webcam";
pub const REGISTER_ARG: &str = "--register-vcam";
pub const UNREGISTER_ARG: &str = "--unregister-vcam";

pub fn dll_path() -> Option<PathBuf> {
    if PathBuf::from("/sys/module/v4l2loopback").exists() {
        return Some(PathBuf::from("/sys/module/v4l2loopback"));
    }
    for idx in 0..16 {
        let dev = PathBuf::from(format!("/dev/video{idx}"));
        if dev.exists() {
            return Some(dev);
        }
    }
    None
}

pub fn is_registered() -> bool {
    PathBuf::from("/sys/module/v4l2loopback").exists()
}

pub fn register_hklm() -> Result<(), String> {
    Ok(())
}

pub fn unregister_hklm() -> Result<(), String> {
    Ok(())
}

pub fn run_elevated(arg: &'static str, on_done: impl FnOnce(bool) + Send + 'static) {
    thread::spawn(move || {
        let exe = match std::env::current_exe() {
            Ok(p) => p,
            Err(_) => {
                on_done(false);
                return;
            }
        };
        let ok = super::elevated::run_elevated(&exe, &[arg]).is_ok_and(|code| code == 0);
        on_done(ok);
    });
}

#[derive(Debug)]
pub struct VirtualCameraHandle;

impl VirtualCameraHandle {
    pub fn start() -> Result<Self, String> {
        Ok(VirtualCameraHandle)
    }
}
