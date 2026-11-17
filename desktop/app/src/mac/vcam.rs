// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS CoreMediaIO Camera Extension (`CMIOExtension`) discovery, registration,
//! and shared-memory frame sink for `"NectarLink Webcam"`.

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    thread,
};

use nectarlink_vcam::{DEFAULT_MAPPING_NAME, SharedFrameMapping};

pub const BUNDLE_ID: &str = "com.nectarlink.desktop.camera-extension";
pub const EXTENSION_BUNDLE_NAME: &str = "com.nectarlink.desktop.camera-extension.systemextension";
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

fn activation_helper_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    let candidate = exe_dir.join("nectarlink-vcam-activate");
    candidate.is_file().then_some(candidate)
}

/// Activates `com.nectarlink.desktop.camera-extension` via `OSSystemExtensionManager`
/// when bundled and signed in an `.app` bundle.
pub fn register_hklm() -> Result<(), String> {
    if is_registered() {
        return Ok(());
    }
    let Some(_bundle) = extension_bundle_path() else {
        return Err(
            "NectarLinkCamera.systemextension is not installed in Contents/Library/SystemExtensions/. \
             macOS Camera Extensions require an Apple Developer-signed .app bundle."
                .into(),
        );
    };
    let Some(helper) = activation_helper_path() else {
        return Err(
            "Camera Extension activation helper is missing; activate from a signed NectarLink.app bundle."
                .into(),
        );
    };
    let out = Command::new(&helper)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("run {}: {e}", helper.display()))?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
    Err(if stderr.is_empty() {
        format!(
            "Camera Extension activation failed (exit {}); check Apple Developer signing and System Settings -> Privacy & Security",
            out.status
        )
    } else {
        stderr
    })
}

/// Deactivates `com.nectarlink.desktop.camera-extension` via `OSSystemExtensionManager`.
pub fn unregister_hklm() -> Result<(), String> {
    if !is_registered() {
        return Ok(());
    }
    let Some(helper) = activation_helper_path() else {
        return Err(
            "Camera Extension deactivation requires a signed NectarLink.app bundle or System Settings -> Login Items & Extensions."
                .into(),
        );
    };
    let out = Command::new(&helper)
        .arg("--deactivate")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("run {}: {e}", helper.display()))?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
    Err(if stderr.is_empty() {
        format!("Camera Extension deactivation failed (exit {})", out.status)
    } else {
        stderr
    })
}

/// Runs Camera Extension registration or unregistration on a background thread
/// and invokes `on_done` with whether the operation succeeded.
pub fn run_elevated(arg: &'static str, on_done: impl FnOnce(bool) + Send + 'static) {
    thread::spawn(move || {
        let res = match arg {
            REGISTER_ARG => register_hklm(),
            UNREGISTER_ARG => unregister_hklm(),
            _ => Err(format!("unknown vcam action: {arg}")),
        };
        if let Err(ref e) = res {
            tracing::warn!(error = %e, arg, "macOS virtual camera setup failed");
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
*\t*\tABC1234567\tcom.nectarlink.desktop.camera-extension (0.1.0/1)\tNectarLink Webcam\t[activated enabled]
";
        assert!(parse_systemextensionsctl_list(sample));
    }

    #[test]
    fn parse_systemextensionsctl_list_ignores_waiting_for_user_and_other_bundles() {
        let waiting = "\
--- com.apple.system_extension.cmio
\t*\tABC1234567\tcom.nectarlink.desktop.camera-extension (0.1.0/1)\tNectarLink Webcam\t[activated waiting for user]
";
        assert!(!parse_systemextensionsctl_list(waiting));

        let other = "\
--- com.apple.system_extension.cmio
*\t*\tR84MX49LQY\tcom.iriun.macwebcam.extension (1.2/3)\tExtension\t[activated enabled]
";
        assert!(!parse_systemextensionsctl_list(other));
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
