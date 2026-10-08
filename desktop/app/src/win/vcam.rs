// SPDX-License-Identifier: GPL-3.0-or-later
//! Windows 11 Virtual Camera (`MFCreateVirtualCamera`) registration and
//! session management for `"Nectarlink Webcam"`.

use std::path::{Path, PathBuf};

use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS},
        Media::{
            KernelStreaming::KSCATEGORY_VIDEO_CAMERA,
            MediaFoundation::{
                IMFVirtualCamera, MF_VERSION, MFCreateVirtualCamera, MFSTARTUP_NOSOCKET, MFStartup,
                MFVirtualCameraAccess_CurrentUser, MFVirtualCameraLifetime_Session,
                MFVirtualCameraType_SoftwareCameraSource,
            },
        },
        System::{
            Registry::{
                HKEY, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ,
                RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegGetValueW, RegSetValueExW,
            },
            Threading::{GetExitCodeProcess, WaitForSingleObject},
        },
        UI::{
            Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
            WindowsAndMessaging::SW_HIDE,
        },
    },
    core::{HSTRING, PCWSTR, w},
};

pub const CLSID_STR: &str = "{8E6C3B74-5D4A-4B9E-9A12-7C8F1E2D3A40}";
pub const FRIENDLY_NAME: &str = "Nectarlink Webcam";
pub const REGISTER_ARG: &str = "--register-vcam";
pub const UNREGISTER_ARG: &str = "--unregister-vcam";

const HKLM_CLSID_PREFIX: &str = r"SOFTWARE\Classes\CLSID";
const HKLM_FRAMESERVER_PREFIX: &str =
    r"SOFTWARE\Microsoft\Windows Media Foundation\Platform\FrameServer\Sources";

/// Resolves `nectarlink_vcam.dll` alongside `nectarlink-desktop.exe`.
pub fn dll_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    Some(dir.join("nectarlink_vcam.dll"))
}

/// Checks whether the virtual camera COM media source is registered under `root\classes_prefix`
/// and its `InprocServer32` DLL exists on disk.
pub fn is_registered_under(root: HKEY, classes_prefix: &str) -> bool {
    let subkey = format!(r"{classes_prefix}\{CLSID_STR}\InprocServer32");
    let mut buf = [0u16; 512];
    let mut size = std::mem::size_of_val(&buf) as u32;
    let status = unsafe {
        RegGetValueW(
            root,
            &HSTRING::from(subkey.as_str()),
            PCWSTR::null(),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS {
        return false;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    if len == 0 {
        return false;
    }
    let path = PathBuf::from(String::from_utf16_lossy(&buf[..len]));
    path.is_file()
}

/// Returns `true` if `nectarlink_vcam.dll` is registered in `HKLM` and exists on disk.
pub fn is_registered() -> bool {
    is_registered_under(HKEY_LOCAL_MACHINE, HKLM_CLSID_PREFIX)
}

fn set_reg_sz(key: HKEY, name: Option<&str>, value: &str) -> Result<(), String> {
    let mut wide: Vec<u16> = value.encode_utf16().collect();
    wide.push(0);
    let bytes: &[u8] = unsafe { std::slice::from_raw_parts(wide.as_ptr().cast::<u8>(), wide.len() * 2) };
    let name_h = name.map(HSTRING::from);
    let name_pcwstr = name_h.as_ref().map_or(PCWSTR::null(), |h| PCWSTR(h.as_ptr()));
    let status = unsafe { RegSetValueExW(key, name_pcwstr, Some(0), REG_SZ, Some(bytes)) };
    if status != ERROR_SUCCESS {
        return Err(format!("RegSetValueExW failed: {}", status.0));
    }
    Ok(())
}

fn create_and_set(root: HKEY, subkey: &str, values: &[(Option<&str>, &str)]) -> Result<(), String> {
    let mut hkey = HKEY::default();
    let status = unsafe {
        RegCreateKeyExW(
            root,
            &HSTRING::from(subkey),
            Some(0),
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE | KEY_READ,
            None,
            &mut hkey,
            None,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(format!("RegCreateKeyExW({subkey}) failed: {}", status.0));
    }
    let res = (|| {
        for &(name, val) in values {
            set_reg_sz(hkey, name, val)?;
        }
        Ok(())
    })();
    unsafe {
        let _ = RegCloseKey(hkey);
    }
    res
}

/// Writes the COM `InprocServer32` and FrameServer source registration keys under `root`.
pub fn register_under(
    root: HKEY,
    classes_prefix: &str,
    frameserver_prefix: &str,
    dll: &Path,
) -> Result<(), String> {
    if !dll.is_file() {
        return Err(format!("missing {}", dll.display()));
    }
    let dll_str = dll.to_string_lossy();
    let clsid_key = format!(r"{classes_prefix}\{CLSID_STR}");
    let inproc_key = format!(r"{clsid_key}\InprocServer32");
    let fs_key = format!(r"{frameserver_prefix}\{CLSID_STR}");

    create_and_set(root, &clsid_key, &[(None, "Nectarlink Virtual Camera Media Source")])?;
    create_and_set(root, &inproc_key, &[(None, dll_str.as_ref()), (Some("ThreadingModel"), "Both")])?;
    create_and_set(root, &fs_key, &[(Some("FriendlyName"), FRIENDLY_NAME)])?;
    Ok(())
}

/// Removes the COM `CLSID` and FrameServer source registration keys under `root`.
pub fn unregister_under(root: HKEY, classes_prefix: &str, frameserver_prefix: &str) -> Result<(), String> {
    for subkey in [format!(r"{classes_prefix}\{CLSID_STR}"), format!(r"{frameserver_prefix}\{CLSID_STR}")] {
        let status = unsafe { RegDeleteTreeW(root, &HSTRING::from(subkey.as_str())) };
        if status != ERROR_SUCCESS && status != ERROR_FILE_NOT_FOUND && status != ERROR_PATH_NOT_FOUND {
            return Err(format!("RegDeleteTreeW({subkey}) failed: {}", status.0));
        }
    }
    Ok(())
}

/// Registers `nectarlink_vcam.dll` in `HKLM` (requires administrator privileges).
pub fn register_hklm() -> Result<(), String> {
    let dll = dll_path().ok_or_else(|| "cannot resolve nectarlink_vcam.dll path".to_owned())?;
    register_under(HKEY_LOCAL_MACHINE, HKLM_CLSID_PREFIX, HKLM_FRAMESERVER_PREFIX, &dll)
}

/// Unregisters `nectarlink_vcam.dll` from `HKLM` (requires administrator privileges).
pub fn unregister_hklm() -> Result<(), String> {
    unregister_under(HKEY_LOCAL_MACHINE, HKLM_CLSID_PREFIX, HKLM_FRAMESERVER_PREFIX)
}

/// Launches `nectarlink-desktop.exe <arg>` elevated via `ShellExecuteExW` (`"runas"`)
/// and invokes `on_done` once the helper process exits.
pub fn run_elevated(arg: &'static str, on_done: impl FnOnce(bool) + Send + 'static) {
    std::thread::spawn(move || {
        let Ok(exe) = std::env::current_exe() else {
            on_done(false);
            return;
        };
        let exe_h = HSTRING::from(exe.as_os_str());
        let params_h = HSTRING::from(arg);
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS,
            lpVerb: w!("runas"),
            lpFile: PCWSTR(exe_h.as_ptr()),
            lpParameters: PCWSTR(params_h.as_ptr()),
            nShow: SW_HIDE.0,
            ..Default::default()
        };
        let ok = unsafe { ShellExecuteExW(&mut info) }.is_ok();
        if !ok || info.hProcess.is_invalid() {
            on_done(false);
            return;
        }
        unsafe {
            let _ = WaitForSingleObject(info.hProcess, 60_000);
            let mut code = 1u32;
            let _ = GetExitCodeProcess(info.hProcess, &mut code);
            let _ = CloseHandle(info.hProcess);
            on_done(code == 0);
        }
    });
}

/// Active `IMFVirtualCamera` session handle.
pub struct VirtualCameraHandle {
    vcam: IMFVirtualCamera,
}

// SAFETY: `IMFVirtualCamera` is a thread-safe COM interface.
unsafe impl Send for VirtualCameraHandle {}
unsafe impl Sync for VirtualCameraHandle {}

impl std::fmt::Debug for VirtualCameraHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VirtualCameraHandle").finish()
    }
}

impl VirtualCameraHandle {
    /// Creates and starts the `"Nectarlink Webcam"` session-lifetime virtual camera.
    /// Succeeds when `nectarlink_vcam.dll` is registered in `HKLM`.
    pub fn start() -> Result<Self, String> {
        unsafe {
            MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET).map_err(|e| e.to_string())?;
            let categories = [KSCATEGORY_VIDEO_CAMERA];
            let vcam = MFCreateVirtualCamera(
                MFVirtualCameraType_SoftwareCameraSource,
                MFVirtualCameraLifetime_Session,
                MFVirtualCameraAccess_CurrentUser,
                &HSTRING::from(FRIENDLY_NAME),
                &HSTRING::from(CLSID_STR),
                Some(&categories),
            )
            .map_err(|e| e.to_string())?;
            vcam.Start(None).map_err(|e| e.to_string())?;
            Ok(Self { vcam })
        }
    }
}

impl Drop for VirtualCameraHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = self.vcam.Stop();
            let _ = self.vcam.Remove();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Registry::HKEY_CURRENT_USER;

    #[test]
    fn registers_checks_and_unregisters_under_test_key() {
        let pid = std::process::id();
        let base = format!(r"Software\NectarlinkTest_Vcam_{pid}");
        let classes = format!(r"{base}\Classes\CLSID");
        let fs = format!(r"{base}\FrameServer\Sources");

        // Create a temporary dummy DLL file so `is_registered_under` sees an existing file.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        assert!(!is_registered_under(HKEY_CURRENT_USER, &classes));

        register_under(HKEY_CURRENT_USER, &classes, &fs, tmp.path()).unwrap();
        assert!(is_registered_under(HKEY_CURRENT_USER, &classes));

        unregister_under(HKEY_CURRENT_USER, &classes, &fs).unwrap();
        assert!(!is_registered_under(HKEY_CURRENT_USER, &classes));

        // Clean up the test root key completely.
        unsafe {
            let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(base.as_str()));
        }
    }
}
