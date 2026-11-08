// SPDX-License-Identifier: GPL-3.0-or-later
//! Windows Share sheet target (`windows.shareTarget`): receives files, web
//! links, text, or bitmaps shared from any Windows app (when registered via the
//! Sparse Package `AppxManifest.xml`) or passed on the command line with
//! `--share <target…>`.

use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use windows::{
    ApplicationModel::{
        Activation::{ActivationKind, IShareTargetActivatedEventArgs},
        AppInstance,
        DataTransfer::StandardDataFormats,
    },
    Storage::Streams::DataReader,
    core::Interface,
};

/// Command-line flag to share files, URLs, or text with a paired phone.
pub const ARG: &str = "--share";

/// Payload extracted from a Windows Share sheet activation or `--share` CLI invocation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SharedPayload {
    pub files: Vec<PathBuf>,
    pub url: Option<String>,
    pub text: Option<String>,
}

impl SharedPayload {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.url.is_none() && self.text.is_none()
    }
}

fn is_share_url(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() || s.len() > 4096 || s.chars().any(char::is_whitespace) {
        return false;
    }
    let lower: String = s.chars().take(8).collect::<String>().to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || (lower.starts_with("geo:") && s.len() > 4)
}

fn looks_like_path(s: &str) -> bool {
    let bytes = s.as_bytes();
    Path::new(s).exists()
        || s.contains('\\')
        || s.contains('/')
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
}

/// Classifies `--share <target…>` CLI arguments into files, a link, or text.
pub fn parse_targets(targets: &[String]) -> SharedPayload {
    let mut payload = SharedPayload::default();
    let mut text_parts = Vec::new();
    for raw in targets {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if payload.url.is_none() && is_share_url(trimmed) {
            payload.url = Some(trimmed.to_owned());
        } else if looks_like_path(trimmed) {
            payload.files.push(PathBuf::from(trimmed));
        } else {
            text_parts.push(trimmed.to_owned());
        }
    }
    if !text_parts.is_empty() {
        payload.text = Some(text_parts.join(" "));
    }
    payload
}

/// The flag that starts the app as the elevated helper registering the package.
pub const REGISTER_ARG: &str = "--register-package";

/// Makes sure the sparse package is registered for this Windows user, which the
/// Share sheet needs (#84).
///
/// The package isn't signed, so Windows only registers it with Developer Mode on;
/// without it this just logs why. Setup registers it on a fresh install. An update
/// runs as SYSTEM, which can't, and the app can't either without admin rights:
/// Windows creates `microsoft.system.package.metadata` in the install folder under
/// Program Files. So when that's what stops it, this asks for admin rights, once
/// per version of the app, so saying no isn't asked again until the next update.
/// A registration from an earlier version keeps working and is left alone.
pub fn ensure_sparse_package(data_dir: PathBuf) {
    let Ok(exe) = std::env::current_exe() else { return };
    let Some(dir) = exe.parent().map(Path::to_path_buf) else { return };
    let Ok(manifest) = std::fs::read_to_string(dir.join("AppxManifest.xml")) else { return };
    let Some(name) = manifest_name(&manifest) else { return };
    std::thread::spawn(move || {
        // After the window: the prompt shouldn't be the first thing on screen.
        std::thread::sleep(std::time::Duration::from_secs(5));
        if registered(&name) {
            return;
        }
        match register(&dir) {
            Ok(()) => return tracing::info!("sparse package registered"),
            Err(error) if !needs_admin(&error) => {
                return tracing::info!(%error, "sparse package not registered (an unsigned one needs Developer Mode)");
            }
            Err(error) => tracing::info!(%error, "can't register the sparse package without admin rights"),
        }
        let asked = data_dir.join("sparse-package-asked");
        let version = env!("CARGO_PKG_VERSION");
        if std::fs::read_to_string(&asked).is_ok_and(|v| v.trim() == version) {
            return tracing::warn!(
                "sparse package not registered: Share and notification buttons won't work"
            );
        }
        if let Err(error) = std::fs::write(&asked, version) {
            tracing::warn!(%error, "can't note that admin rights were asked for");
        }
        match run_elevated(&exe) {
            Ok(true) if registered(&name) => tracing::info!("sparse package registered with admin rights"),
            Ok(true) => tracing::warn!("sparse package still not registered after the elevated helper"),
            Ok(false) => {
                tracing::warn!("sparse package not registered: admin rights declined or the helper failed")
            }
            Err(error) => tracing::warn!(%error, "can't start the helper that registers the sparse package"),
        }
    });
}

/// The elevated helper ([`REGISTER_ARG`]): registers the package next to this exe.
pub fn register_elevated() -> std::process::ExitCode {
    let dir = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf));
    match dir.map(|dir| register(&dir)) {
        Some(Ok(())) => std::process::ExitCode::SUCCESS,
        Some(Err(error)) => {
            eprintln!("nectarlink: register-package failed: {error}");
            std::process::ExitCode::FAILURE
        }
        None => std::process::ExitCode::FAILURE,
    }
}

/// Whether admin rights would get the registration past `error`: access denied
/// (0x80070005), creating the metadata folder under Program Files. Not, say, the
/// missing developer license (0x80073CFF), which admin rights don't change.
fn needs_admin(error: &str) -> bool {
    error.contains("0x80070005")
}

/// The package name in an `AppxManifest.xml` (`<Identity Name="...">`).
fn manifest_name(manifest: &str) -> Option<String> {
    let identity = &manifest[manifest.find("<Identity")?..];
    let identity = &identity[..identity.find('>')?];
    let start = identity.find("Name=\"")? + "Name=\"".len();
    let name = &identity[start..];
    let name = &name[..name.find('"')?];
    (!name.is_empty()).then(|| name.to_owned())
}

fn powershell(script: &str, dir: Option<&Path>, name: Option<&str>) -> std::io::Result<std::process::Output> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = std::process::Command::new("powershell.exe");
    command
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script])
        .stdin(std::process::Stdio::null())
        .creation_flags(CREATE_NO_WINDOW);
    if let Some(dir) = dir {
        command.env("NECTARLINK_INSTDIR", dir);
    }
    if let Some(name) = name {
        command.env("NECTARLINK_PACKAGE", name);
    }
    command.output()
}

/// Whether any version of the package is registered for this user.
fn registered(name: &str) -> bool {
    powershell("(Get-AppxPackage -Name $env:NECTARLINK_PACKAGE | Measure-Object).Count", None, Some(name))
        .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).trim().parse::<u32>().is_ok_and(|n| n > 0))
}

/// Registers the package in `dir`; Windows' message if it can't.
fn register(dir: &Path) -> Result<(), String> {
    let script = "try { Add-AppxPackage -Register \"$env:NECTARLINK_INSTDIR\\AppxManifest.xml\" \
                  -ExternalLocation $env:NECTARLINK_INSTDIR -ErrorAction Stop } \
                  catch { $_.Exception.Message; exit 1 }";
    let out = powershell(script, Some(dir), None).map_err(|e| e.to_string())?;
    if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stdout).trim().to_owned()) }
}

/// Starts this exe as the elevated helper (the UAC prompt) and waits for it.
/// `Ok(false)` when the prompt is declined or the helper fails.
fn run_elevated(exe: &Path) -> windows::core::Result<bool> {
    use windows::{
        Win32::{
            Foundation::{CloseHandle, WAIT_OBJECT_0},
            System::Threading::{GetExitCodeProcess, WaitForSingleObject},
            UI::{
                Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
                WindowsAndMessaging::SW_HIDE,
            },
        },
        core::{HSTRING, PCWSTR, w},
    };
    let file = HSTRING::from(exe.as_os_str());
    let parameters = HSTRING::from(REGISTER_ARG);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    // SAFETY: the structure and its strings outlive the call; the process
    // handle is waited on and closed.
    unsafe {
        if ShellExecuteExW(&mut info).is_err() {
            // Most often: the user said no to the prompt.
            return Ok(false);
        }
        if info.hProcess.is_invalid() {
            return Ok(false);
        }
        let waited = WaitForSingleObject(info.hProcess, 60_000);
        let mut exit_code = 1u32;
        let _ = GetExitCodeProcess(info.hProcess, &mut exit_code);
        let _ = CloseHandle(info.hProcess);
        Ok(waited == WAIT_OBJECT_0 && exit_code == 0)
    }
}

/// Checks if the current process was activated by the Windows Share sheet
/// (`ActivationKind::ShareTarget`). If so, reads the shared storage items,
/// link, text, or bitmap, marks the share operation completed, and returns
/// the extracted [`SharedPayload`].
pub fn take_activated_share(data_dir: &Path) -> Option<SharedPayload> {
    super::with_com(|| {
        let args = AppInstance::GetActivatedEventArgs().ok()?;
        if args.Kind().ok()? != ActivationKind::ShareTarget {
            return None;
        }
        let share_args: IShareTargetActivatedEventArgs = args.cast().ok()?;
        let share_op = share_args.ShareOperation().ok()?;
        let data = share_op.Data().ok()?;
        let mut payload = SharedPayload::default();

        if let Ok(fmt) = StandardDataFormats::StorageItems()
            && data.Contains(&fmt).unwrap_or(false)
            && let Ok(op) = data.GetStorageItemsAsync()
            && let Ok(items) = op.join()
        {
            for item in items {
                if let Ok(path) = item.Path() {
                    let s = path.to_string();
                    if !s.is_empty() {
                        payload.files.push(PathBuf::from(s));
                    }
                }
            }
        }

        if payload.url.is_none()
            && let Ok(fmt) = StandardDataFormats::WebLink()
            && data.Contains(&fmt).unwrap_or(false)
            && let Ok(op) = data.GetWebLinkAsync()
            && let Ok(uri) = op.join()
            && let Ok(raw) = uri.RawUri()
        {
            let s = raw.to_string();
            if !s.is_empty() {
                payload.url = Some(s);
            }
        }

        if payload.url.is_none()
            && let Ok(fmt) = StandardDataFormats::Uri()
            && data.Contains(&fmt).unwrap_or(false)
            && let Ok(op) = data.GetUriAsync()
            && let Ok(uri) = op.join()
            && let Ok(raw) = uri.RawUri()
        {
            let s = raw.to_string();
            if !s.is_empty() {
                payload.url = Some(s);
            }
        }

        if let Ok(fmt) = StandardDataFormats::Text()
            && data.Contains(&fmt).unwrap_or(false)
            && let Ok(op) = data.GetTextAsync()
            && let Ok(text) = op.join()
        {
            let s = text.to_string();
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                if payload.url.is_none() && is_share_url(trimmed) {
                    payload.url = Some(trimmed.to_owned());
                } else {
                    payload.text = Some(s);
                }
            }
        }

        if payload.files.is_empty()
            && let Ok(fmt) = StandardDataFormats::Bitmap()
            && data.Contains(&fmt).unwrap_or(false)
            && let Some(saved) = save_shared_bitmap(&data, data_dir)
        {
            payload.files.push(saved);
        }

        let _ = share_op.ReportCompleted();
        (!payload.is_empty()).then_some(payload)
    })
}

fn save_shared_bitmap(
    data: &windows::ApplicationModel::DataTransfer::DataPackageView,
    data_dir: &Path,
) -> Option<PathBuf> {
    let stream_ref = data.GetBitmapAsync().ok()?.join().ok()?;
    let stream = stream_ref.OpenReadAsync().ok()?.join().ok()?;
    let size = stream.Size().ok()?;
    let reader = DataReader::CreateDataReader(&stream).ok()?;
    let loaded = reader.LoadAsync(u32::try_from(size).unwrap_or(u32::MAX)).ok()?.join().ok()?;
    let mut bytes = vec![0u8; loaded as usize];
    reader.ReadBytes(&mut bytes).ok()?;
    if bytes.is_empty() {
        return None;
    }
    let png = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        bytes
    } else {
        let bmp = super::image::decode(&bytes).ok()?;
        super::image::encode_png(&bmp).ok()?
    };
    let dir = data_dir.join("cache").join("shared");
    std::fs::create_dir_all(&dir).ok()?;
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let path = dir.join(format!("shared-{nanos}.png"));
    std::fs::write(&path, png).ok()?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use windows::{Data::Xml::Dom::XmlDocument, core::HSTRING};

    use super::*;

    #[test]
    fn parses_cli_share_targets() {
        let files = parse_targets(&[r"C:\Users\Alice\Pictures\photo.jpg".into(), r"D:\notes.txt".into()]);
        assert_eq!(
            files,
            SharedPayload {
                files: vec![
                    PathBuf::from(r"C:\Users\Alice\Pictures\photo.jpg"),
                    PathBuf::from(r"D:\notes.txt")
                ],
                url: None,
                text: None,
            }
        );

        let link = parse_targets(&["https://example.com/watch?v=42".into()]);
        assert_eq!(
            link,
            SharedPayload {
                files: Vec::new(),
                url: Some("https://example.com/watch?v=42".into()),
                text: None,
            }
        );

        let text = parse_targets(&["Meet me at 7pm".into()]);
        assert_eq!(text, SharedPayload { files: Vec::new(), url: None, text: Some("Meet me at 7pm".into()) });
    }

    #[test]
    fn sparse_package_manifest_is_valid_xml_with_share_target() {
        let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../installer/AppxManifest.xml");
        let xml_str = std::fs::read_to_string(&manifest_path).expect("read AppxManifest.xml");
        let doc = XmlDocument::new().unwrap();
        doc.LoadXml(&HSTRING::from(&xml_str)).expect("valid AppxManifest.xml");
        assert!(xml_str.contains(r#"Name="Nectarlink.Desktop""#));
        assert!(xml_str.contains("uap10:AllowExternalContent"));
        assert!(xml_str.contains(r#"Category="windows.shareTarget""#));
        for format in ["StorageItems", "WebLink", "Uri", "Text", "Bitmap"] {
            assert!(xml_str.contains(&format!("<uap:DataFormat>{format}</uap:DataFormat>")));
        }
        // Without braces: the schema rejects them, and the package then never registers (#84).
        assert!(xml_str.contains(r#"ToastActivatorCLSID="7B9E0E64-5C5A-4E8B-9D31-5A4C2B10F6E1""#));
    }

    #[test]
    fn the_package_name_comes_from_the_installed_manifest() {
        let manifest = include_str!("../../../installer/AppxManifest.xml");
        assert_eq!(manifest_name(manifest).as_deref(), Some("Nectarlink.Desktop"));
        assert_eq!(manifest_name(r#"<Identity Version="1.0.0.0" Name="A.B" />"#).as_deref(), Some("A.B"));
        // Only the Identity's own Name, never one further on.
        assert_eq!(manifest_name(r#"<Identity Version="1" /><Application Name="x" />"#), None);
        assert_eq!(manifest_name("<Package />"), None);
    }

    #[test]
    fn only_the_program_files_error_asks_for_admin_rights() {
        // An auto-updated install with Developer Mode on (#84).
        assert!(needs_admin(
            "Deployment failed with HRESULT: 0x80070005, Access is denied. Creating the package metadata \
             directory C:\\Program Files\\Nectarlink\\microsoft.system.package.metadata failed."
        ));
        // Without Developer Mode: admin rights don't help, so no prompt.
        assert!(!needs_admin(
            "Deployment failed with HRESULT: 0x80073CFF, To install this application you need either a \
             Windows developer license or a sideloading-enabled system."
        ));
        assert!(!needs_admin(""));
    }

    #[test]
    fn a_package_that_isnt_there_is_not_registered() {
        assert!(!registered("Nectarlink.NoSuchPackage"));
    }
}
