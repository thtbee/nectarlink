// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS CoreMediaIO Camera Extension (`CMIOExtension`) discovery, registration,
//! and shared-memory frame sink for `"NectarLink Webcam"`.

use std::{
    ffi::{CString, c_char, c_int, c_void},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use nectarlink_vcam::{DEFAULT_MAPPING_NAME, SharedFrameMapping};

use crate::i18n::tr;

/// The extension's bundle ID. macOS only installs a system extension whose ID
/// starts with its host app's (`app.nectarlink.desktop`).
pub const BUNDLE_ID: &str = "app.nectarlink.desktop.camera-extension";
pub const EXTENSION_BUNDLE_NAME: &str = "app.nectarlink.desktop.camera-extension.systemextension";
pub const APP_GROUP_ID: &str = "group.com.nectarlink.desktop";
pub const APP_GROUP_FILE: &str = "nectarlink_vcam_frame_v2";
pub const REGISTER_ARG: &str = "--register-vcam";
pub const UNREGISTER_ARG: &str = "--unregister-vcam";

/// Parses `systemextensionsctl list` output and returns `true` when
/// [`BUNDLE_ID`] is both activated and enabled (`[activated enabled]`).
pub fn parse_systemextensionsctl_list(output: &str) -> bool {
    output.lines().any(|line| {
        let Some(pos) = line.find(BUNDLE_ID) else {
            return false;
        };
        let rest = &line[pos + BUNDLE_ID.len()..];
        let boundary_ok = rest.is_empty() || rest.starts_with(char::is_whitespace) || rest.starts_with('(');
        boundary_ok && line.contains("[activated enabled]")
    })
}

/// Returns `true` if the NectarLink CoreMediaIO Camera Extension is currently
/// activated and enabled in macOS (`systemextensionsctl list`).
pub fn is_registered() -> bool {
    for bin in ["/usr/bin/systemextensionsctl", "systemextensionsctl"] {
        if let Ok(out) = Command::new(bin).arg("list").stdin(Stdio::null()).stderr(Stdio::null()).output()
            && out.status.success()
        {
            return parse_systemextensionsctl_list(&String::from_utf8_lossy(&out.stdout));
        }
    }
    false
}

/// Locates the `.systemextension` bundle inside the `.app` bundle (or alongside
/// the executable in development builds).
pub fn extension_bundle_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    find_extension_bundle_near(exe_dir)
}

fn find_extension_bundle_near(exe_dir: &Path) -> Option<PathBuf> {
    let candidates = [
        exe_dir.join("../Library/SystemExtensions").join(EXTENSION_BUNDLE_NAME),
        exe_dir.join("../Library/SystemExtensions/NectarLinkCamera.systemextension"),
        exe_dir.join(EXTENSION_BUNDLE_NAME),
        exe_dir.join("NectarLinkCamera.systemextension"),
    ];
    candidates.into_iter().find(|p| p.is_dir())
}

/// Returns `true` when the macOS Camera Extension is registered or its
/// `.systemextension` bundle is present in the app bundle.
pub fn is_available() -> bool {
    is_registered() || extension_bundle_path().is_some()
}

/// Resolves the shared-memory file path inside the App Group container when
/// that container directory exists on this Mac.
pub fn app_group_shm_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let containers = home.join("Library/Group Containers");
    let direct = containers.join(APP_GROUP_ID);
    if direct.is_dir() {
        return Some(direct.join(APP_GROUP_FILE));
    }
    let suffix = format!(".{APP_GROUP_ID}");
    let entries = std::fs::read_dir(&containers).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().ends_with(&suffix) && entry.path().is_dir() {
            return Some(entry.path().join(APP_GROUP_FILE));
        }
    }
    None
}

/// How long to wait for the user to approve the extension in System Settings.
const APPROVAL_TIMEOUT_SECS: c_int = 15 * 60;

/// macOS is waiting for the user to allow the extension in System Settings.
static AWAITING_APPROVAL: AtomicBool = AtomicBool::new(false);
static SETUP_STATUS: Mutex<String> = Mutex::new(String::new());
static STATUS_HOOK: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// Whether macOS is waiting for the user to allow the extension.
pub fn awaiting_approval() -> bool {
    AWAITING_APPROVAL.load(Ordering::Relaxed)
}

/// Opens System Settings at Login Items & Extensions, where the extension is allowed.
pub fn open_approval_settings() {
    let _ = Command::new("/usr/bin/open")
        .arg("x-apple.systempreferences:com.apple.LoginItems-Settings.extension")
        .stdin(Stdio::null())
        .spawn();
}

/// What the last "Set up" is waiting for or why it failed, ready to show;
/// empty when there is nothing to say.
pub fn setup_status() -> String {
    SETUP_STATUS.lock().map(|s| s.clone()).unwrap_or_default()
}

/// Calls `hook` whenever [`setup_status`] changes, from the setup thread.
pub fn set_status_hook(hook: impl Fn() + Send + Sync + 'static) {
    let _ = STATUS_HOOK.set(Box::new(hook));
}

fn set_status(text: impl Into<String>) {
    let text = text.into();
    if let Ok(mut s) = SETUP_STATUS.lock() {
        if *s == text {
            return;
        }
        *s = text;
    }
    if let Some(hook) = STATUS_HOOK.get() {
        hook();
    }
}

fn approval_message() -> String {
    tr!(
        "Webcam",
        "Waiting for your approval: turn on NectarLink Webcam in System Settings > General > Login Items & Extensions > Camera Extensions."
    )
}

// `cpp/mac_sysext.mm`. It runs in this executable because macOS only honors the
// system-extension entitlement for an app bundle's main executable.
const SYSEXT_DONE: c_int = 0;
const SYSEXT_RESTART: c_int = 2;
const SYSEXT_TIMEOUT: c_int = 3;

#[allow(unsafe_code)]
unsafe extern "C" {
    fn nl_sysext_request(
        bundle_id: *const c_char,
        deactivate: c_int,
        timeout_secs: c_int,
        on_approval: extern "C" fn(*mut c_void),
        ctx: *mut c_void,
        error_code: *mut c_int,
        error_text: *mut c_char,
        error_text_len: usize,
    ) -> c_int;
}

extern "C" fn on_approval_needed(_ctx: *mut c_void) {
    AWAITING_APPROVAL.store(true, Ordering::Relaxed);
    set_status(approval_message());
}

/// Words for an `OSSystemExtensionError` code, the system's own `text` when
/// there are none better.
fn describe_sysext_error(code: i32, text: &str) -> String {
    match code {
        2 => tr!(
            "Webcam",
            "This NectarLink build isn't allowed to install the camera extension (missing the system-extension entitlement or provisioning profile). Install the official signed NectarLink release."
        ),
        3 => tr!("Webcam", "Move NectarLink.app to the Applications folder, then try again."),
        4 => tr!("Webcam", "The camera extension isn't inside this copy of NectarLink.app."),
        8..=10 => tr!("Webcam", "macOS rejected the camera extension: %1", text),
        11 | 12 => tr!("Webcam", "The camera extension request was canceled."),
        13 => tr!(
            "Webcam",
            "Allow NectarLink Webcam in System Settings > General > Login Items & Extensions > Camera Extensions, then try again."
        ),
        _ if text.is_empty() => tr!("Webcam", "The camera extension request failed (%1).", code),
        _ => tr!("Webcam", "The camera extension request failed: %1", text),
    }
}

#[allow(unsafe_code)]
/// Asks macOS to activate (or remove) the camera extension and waits, telling
/// the status hook if the user has to approve it. Fails with a message fit to show.
fn request_extension(deactivate: bool) -> Result<(), String> {
    let id = CString::new(BUNDLE_ID).map_err(|e| e.to_string())?;
    let mut code: c_int = 0;
    let mut text = [0 as c_char; 512];
    // SAFETY: `id` and `text` outlive the call, and `text`'s length is passed along.
    let result = unsafe {
        nl_sysext_request(
            id.as_ptr(),
            c_int::from(deactivate),
            APPROVAL_TIMEOUT_SECS,
            on_approval_needed,
            std::ptr::null_mut(),
            &mut code,
            text.as_mut_ptr(),
            text.len(),
        )
    };
    match result {
        SYSEXT_DONE => Ok(()),
        SYSEXT_RESTART => {
            Err(tr!("Webcam", "The camera extension will be ready after you restart your Mac."))
        }
        SYSEXT_TIMEOUT => Err(tr!(
            "Webcam",
            "The camera extension wasn't approved in time. Click Set up again, then turn on NectarLink Webcam in System Settings > General > Login Items & Extensions > Camera Extensions."
        )),
        _ => {
            // SAFETY: the function wrote a NUL-terminated string into `text` on failure.
            let text = unsafe { std::ffi::CStr::from_ptr(text.as_ptr()) }.to_string_lossy().into_owned();
            Err(describe_sysext_error(code, &text))
        }
    }
}

/// Activates the camera extension through `OSSystemExtensionManager` when it's
/// bundled in a signed `.app`. macOS may ask the user to approve it.
pub fn register_hklm() -> Result<(), String> {
    if is_registered() {
        return Ok(());
    }
    if extension_bundle_path().is_none() {
        return Err(tr!(
            "Webcam",
            "This copy of NectarLink doesn't include the camera extension. Install NectarLink from its DMG into the Applications folder."
        ));
    }
    request_extension(false)
}

/// Deactivates the camera extension through `OSSystemExtensionManager`.
pub fn unregister_hklm() -> Result<(), String> {
    if !is_registered() {
        return Ok(());
    }
    request_extension(true)
}

/// Runs Camera Extension registration or unregistration on a background thread
/// and invokes `on_done` with whether the operation succeeded. A failure or a
/// wait for approval is left in [`setup_status`].
pub fn run_elevated(arg: &'static str, on_done: impl FnOnce(bool) + Send + 'static) {
    thread::spawn(move || {
        AWAITING_APPROVAL.store(false, Ordering::Relaxed);
        set_status("");
        let res = match arg {
            REGISTER_ARG => register_hklm(),
            UNREGISTER_ARG => unregister_hklm(),
            _ => Err(format!("unknown vcam action: {arg}")),
        };
        AWAITING_APPROVAL.store(false, Ordering::Relaxed);
        match &res {
            Ok(()) => set_status(""),
            Err(e) => {
                tracing::warn!(error = %e, arg, "macOS virtual camera setup failed");
                set_status(e.clone());
            }
        }
        on_done(res.is_ok());
    });
}

/// Active macOS virtual camera frame sink bridging `webcam.rs` decoded BGRX
/// frames into the shared-memory mapping read by `NectarLinkCamera.systemextension`.
#[derive(Debug, Default)]
pub struct VirtualCameraHandle {
    app_group_mapping: Option<Arc<SharedFrameMapping>>,
}

impl VirtualCameraHandle {
    /// Opens or creates the shared-memory frame sink for the macOS Camera Extension.
    pub fn start() -> Result<Self, String> {
        let app_group_mapping =
            app_group_shm_path().and_then(|path| match SharedFrameMapping::open_or_create_at(&path) {
                Ok(m) => Some(Arc::new(m)),
                Err(e) => {
                    tracing::warn!(error = %e, path = %path.display(), "can't open App Group vcam mapping");
                    None
                }
            });
        // Ensure the primary mapping is also initialized for local/dev extension readers.
        SharedFrameMapping::open_or_create(DEFAULT_MAPPING_NAME)
            .map_err(|e| format!("open macOS vcam shared mapping: {e}"))?;
        Ok(Self { app_group_mapping })
    }

    /// Mirrors a live BGRX frame into the App Group shared mapping when present.
    pub fn write_frame(
        &self,
        phone_name: &str,
        width: u32,
        height: u32,
        timestamp_us: u64,
        bgrx: &[u8],
        mirror: bool,
    ) {
        if let Some(ref map) = self.app_group_mapping {
            map.write_frame(phone_name, width, height, timestamp_us, bgrx, mirror);
        }
    }

    /// Marks the App Group shared mapping idle when present.
    pub fn set_idle(&self, phone_name: &str) {
        if let Some(ref map) = self.app_group_mapping {
            map.set_idle(phone_name);
        }
    }
}

impl Drop for VirtualCameraHandle {
    fn drop(&mut self) {
        self.set_idle("");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_systemextensionsctl_list_matches_activated_enabled() {
        let sample = "\
5 extension(s)
--- com.apple.system_extension.cmio (Go to 'System Settings > General > Login Items & Extensions > Camera Extensions')
enabled\tactive\tteamID\tbundleID (version)\tname\t[state]
*\t*\tABC1234567\tapp.nectarlink.desktop.camera-extension (0.1.0/1)\tNectarLink Webcam\t[activated enabled]
";
        assert!(parse_systemextensionsctl_list(sample));
    }

    #[test]
    fn parse_systemextensionsctl_list_ignores_waiting_for_user_and_other_bundles() {
        let waiting = "\
--- com.apple.system_extension.cmio
\t*\tABC1234567\tapp.nectarlink.desktop.camera-extension (0.1.0/1)\tNectarLink Webcam\t[activated waiting for user]
";
        assert!(!parse_systemextensionsctl_list(waiting));

        let other = "\
--- com.apple.system_extension.cmio
*\t*\tR84MX49LQY\tcom.iriun.macwebcam.extension (1.2/3)\tExtension\t[activated enabled]
";
        assert!(!parse_systemextensionsctl_list(other));
    }

    #[test]
    fn describe_sysext_error_explains_the_codes_users_hit() {
        assert!(describe_sysext_error(2, "").contains("isn't allowed to install"));
        assert!(describe_sysext_error(3, "").contains("Applications folder"));
        assert!(describe_sysext_error(13, "").contains("Camera Extensions"));
        assert!(describe_sysext_error(10, "blocked").contains("blocked"));
        assert!(describe_sysext_error(99, "Boom").contains("Boom"));
        assert!(describe_sysext_error(99, "").contains("99"));
    }

    #[test]
    fn find_extension_bundle_in_app_layout() {
        let root = std::env::temp_dir().join(format!("nl_vcam_bundle_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let macos_dir = root.join("NectarLink.app/Contents/MacOS");
        let sysx_dir =
            root.join("NectarLink.app/Contents/Library/SystemExtensions").join(EXTENSION_BUNDLE_NAME);
        std::fs::create_dir_all(&macos_dir).unwrap();
        std::fs::create_dir_all(&sysx_dir).unwrap();

        let found = find_extension_bundle_near(&macos_dir);
        assert_eq!(found.and_then(|p| p.canonicalize().ok()), sysx_dir.canonicalize().ok());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn virtual_camera_handle_starts_and_clears_idle_frame() {
        let handle = VirtualCameraHandle::start().expect("should open macOS vcam shared mapping");
        handle.set_idle("Test Phone");
        drop(handle);
    }
}
