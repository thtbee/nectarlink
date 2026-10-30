// SPDX-License-Identifier: GPL-3.0-or-later
//! Updates for an installed copy: the latest GitHub release is checked at
//! startup and once a day (unless turned off). A newer one is downloaded,
//! checked against the release's SHA-256 sums, and installed silently.
//!
//! The app lives in Program Files, so installing needs administrator rights.
//! The installer registers a scheduled task, [`TASK`], that runs this program
//! as SYSTEM with [`SERVICE_ARG`]: it takes nothing from the user, fetches
//! the latest release itself, and only ever installs a newer published one.
//! Any signed-in user may start it, so updates need no prompt. With
//! automatic updates on, the app starts it at a quiet moment (nobody at the
//! PC, nothing being mirrored, sent or called); "Update" starts it right
//! away. Without the task (a copy installed before it existed), "Update"
//! runs the installer with the usual administrator prompt.
//!
//! The app downloads the installer first, as the user, showing progress and
//! resuming after a dropped connection ([`stage`]), into a shared folder.
//! The task takes that file only when it matches the release's sums, which
//! it fetches itself, and copies it somewhere only SYSTEM can write before
//! checking and running it; otherwise it downloads the installer itself.
//!
//! The installer replaces the running app, so once the download is ready the
//! app starts a small hidden relauncher and quits; the relauncher opens the
//! new version once the installer is done, or the old one again if the
//! update didn't go through.

use crate::i18n::tr;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::{
    path::{Path, PathBuf},
    process::ExitCode,
    sync::{
        Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{
    core_host,
    state::Changes,
    win::{autostart, http, toast},
};

const LATEST: &str = "https://api.github.com/repos/alejandrokundrah/nectarlink/releases/latest";
/// Where our release files are. An update is only downloaded from here, so a
/// changed or spoofed release listing can't point the installer elsewhere.
const RELEASE_DOWNLOADS: &str = "https://github.com/alejandrokundrah/nectarlink/releases/download/";
const FIRST_CHECK: Duration = Duration::from_secs(30);
const EVERY: Duration = Duration::from_secs(24 * 3600);
const MAX_INSTALLER: u64 = 400 * 1024 * 1024;
/// The toast "device" for update notices.
pub const TOAST_GROUP: &str = "update";
pub const ACTION_UPDATE: &str = "update";

/// A newer release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub version: String,
    pub notes_url: String,
    installer_url: String,
    installer_name: String,
    sums_url: String,
}

/// The scheduled task the installer registers (see the module docs).
pub const TASK: &str = "Nectarlink Update";
/// What the task runs: `nectarlink-desktop.exe --update-service`.
pub const SERVICE_ARG: &str = "--update-service";
/// How long nobody may have used the PC before an update installs by itself.
const QUIET_FOR: Duration = Duration::from_secs(10 * 60);
/// While an update waits for a quiet moment, how often to look for one.
const LOOK_FOR_QUIET: Duration = Duration::from_secs(5 * 60);
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const DETACHED_PROCESS: u32 = 0x0000_0008;

static AUTO: AtomicBool = AtomicBool::new(true);
static AUTO_INSTALL: AtomicBool = AtomicBool::new(true);
static CHECKING: AtomicBool = AtomicBool::new(false);
static FOUND: Mutex<Option<Update>> = Mutex::new(None);

fn found() -> MutexGuard<'static, Option<Update>> {
    FOUND.lock().unwrap_or_else(|e| e.into_inner())
}

/// The update found, if any.
pub fn available() -> Option<Update> {
    found().clone()
}

pub fn set_auto(on: bool) {
    AUTO.store(on, Ordering::Relaxed);
}

/// Whether a newer release installs by itself at a quiet moment.
pub fn set_auto_install(on: bool) {
    AUTO_INSTALL.store(on, Ordering::Relaxed);
}

/// Whether this copy updates itself (an installed one, not a build run
/// from its folder).
pub fn can_update() -> bool {
    std::env::current_exe().is_ok_and(|exe| autostart::is_installed(&exe))
}

/// Checks soon after startup, then daily, while automatic checks are on.
pub fn start() {
    if !can_update() {
        return;
    }
    let spawned = std::thread::Builder::new().name("updates".into()).spawn(|| {
        std::thread::sleep(FIRST_CHECK);
        loop {
            if AUTO.load(Ordering::Relaxed) {
                check_now(false);
            }
            // Daily; while an update waits to install itself, look for a quiet moment.
            let next_check = std::time::Instant::now() + EVERY;
            while let Some(left) = next_check.checked_duration_since(std::time::Instant::now()) {
                if AUTO.load(Ordering::Relaxed)
                    && AUTO_INSTALL.load(Ordering::Relaxed)
                    && available().is_some()
                    && quiet()
                    && task_registered()
                {
                    tracing::info!("installing the update at a quiet moment");
                    let staged = available().map(|update| stage(&update, |_, _| {}));
                    if let Some(Err(e)) = staged {
                        tracing::info!(error = %e, "the automatic update couldn't be downloaded yet");
                    } else if let Err(e) = install_with_task(false) {
                        tracing::warn!(error = e, "the automatic update didn't start");
                    }
                }
                std::thread::sleep(left.min(LOOK_FOR_QUIET));
            }
        }
    });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "won't check for updates");
    }
}

/// Checks now (blocking). `told`: the user asked, so say "up to date" too.
/// Returns what to tell the user.
pub fn check_now(told: bool) -> Option<String> {
    if CHECKING.swap(true, Ordering::AcqRel) {
        return None;
    }
    let result = latest();
    CHECKING.store(false, Ordering::Release);
    match result {
        Ok(Some(update)) => {
            let new = found().as_ref() != Some(&update);
            *found() = Some(update.clone());
            core_host::host().hub.update(|_| Changes::UPDATE);
            if new {
                announce(&update);
            }
            Some(tr!("Updater", "NectarLink %1 is available.", update.version))
        }
        Ok(None) => told.then(|| tr!("Updater", "NectarLink is up to date.")),
        Err(e) => {
            tracing::info!(error = %e, "couldn't check for updates");
            told.then(|| tr!("Updater", "Couldn't check for updates. Check your internet connection."))
        }
    }
}

fn announce(update: &Update) {
    toast::show(toast::Toast {
        device: TOAST_GROUP.into(),
        key: update.version.clone(),
        title: tr!("Updater", "NectarLink %1 is available", update.version),
        body: tr!("Updater", "Update now, or later from Settings."),
        attribution: "NectarLink".into(),
        icon: None,
        image: None,
        actions: vec![(ACTION_UPDATE.into(), "Update".into())],
        reply: None,
        silent: true,
        progress: None,
        call: false,
    });
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

fn latest() -> Result<Option<Update>, String> {
    let body = match http::get(LATEST, 1024 * 1024) {
        Ok(body) => body,
        // GitHub answers 404 until the first release is published: nothing newer.
        Err(e) if e == "HTTP 404" => return Ok(None),
        Err(e) => return Err(e),
    };
    let release: Release = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
    Ok(newer_release(release, env!("CARGO_PKG_VERSION")))
}

#[cfg(windows)]
const INSTALLER_SUFFIX: &str =
    if cfg!(target_arch = "aarch64") { "-arm64-setup.exe" } else { "-x64-setup.exe" };

// Each kind of install only takes its own kind of file: a .deb installed
// under a running AppImage, say, leaves the AppImage as old as it was (and
// the update is offered again), and a tarball beside a package is a second
// copy in /opt.
#[cfg(target_os = "linux")]
const APPIMAGE_SUFFIXES: &[&str] =
    if cfg!(target_arch = "aarch64") { &["-aarch64.AppImage"] } else { &["-x86_64.AppImage"] };

#[cfg(target_os = "linux")]
const DEB_SUFFIXES: &[&str] = if cfg!(target_arch = "aarch64") { &["_arm64.deb"] } else { &["_amd64.deb"] };

#[cfg(target_os = "linux")]
const TARBALL_SUFFIXES: &[&str] =
    if cfg!(target_arch = "aarch64") { &["-linux-aarch64.tar.gz"] } else { &["-linux-x86_64.tar.gz"] };

#[cfg(target_os = "linux")]
const RPM_SUFFIXES: &[&str] =
    if cfg!(target_arch = "aarch64") { &[".aarch64.rpm"] } else { &[".x86_64.rpm"] };

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LinuxInstallKind {
    AppImage,
    Deb,
    Rpm,
    Tarball,
}

#[cfg(target_os = "linux")]
fn detect_linux_install_kind() -> LinuxInstallKind {
    if std::env::var_os("APPIMAGE").is_some() {
        return LinuxInstallKind::AppImage;
    }
    if let Ok(exe) = std::env::current_exe() {
        let resolved = std::fs::canonicalize(&exe).unwrap_or(exe);
        if resolved.starts_with("/opt/nectarlink")
            || resolved.parent().and_then(|p| p.file_name()).is_some_and(|n| n == "nectarlink-app")
        {
            return LinuxInstallKind::Tarball;
        }
    }
    if Path::new("/var/lib/dpkg/info/nectarlink.list").is_file() {
        return LinuxInstallKind::Deb;
    }
    if rpm_installed() {
        return LinuxInstallKind::Rpm;
    }
    // No package owns this copy: it came from the tarball, wherever it was unpacked.
    LinuxInstallKind::Tarball
}

/// Whether the `nectarlink` RPM package is installed.
#[cfg(target_os = "linux")]
fn rpm_installed() -> bool {
    std::process::Command::new("rpm")
        .args(["-q", "--quiet", "nectarlink"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// The release as an update, if it's a newer published one with an
/// installer for this architecture and its sums.
#[cfg(windows)]
fn newer_release(release: Release, current: &str) -> Option<Update> {
    newer_release_for(release, current, INSTALLER_SUFFIX)
}

#[cfg(target_os = "linux")]
fn newer_release(release: Release, current: &str) -> Option<Update> {
    newer_release_linux_kind(release, current, detect_linux_install_kind())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn newer_release(_release: Release, _current: &str) -> Option<Update> {
    None
}

#[cfg(target_os = "linux")]
#[cfg_attr(not(test), allow(dead_code))]
fn newer_release_linux(release: Release, current: &str, is_appimage: bool) -> Option<Update> {
    let kind = if is_appimage { LinuxInstallKind::AppImage } else { LinuxInstallKind::Deb };
    newer_release_linux_kind(release, current, kind)
}

#[cfg(target_os = "linux")]
fn newer_release_linux_kind(release: Release, current: &str, kind: LinuxInstallKind) -> Option<Update> {
    let suffixes = match kind {
        LinuxInstallKind::AppImage => APPIMAGE_SUFFIXES,
        LinuxInstallKind::Deb => DEB_SUFFIXES,
        LinuxInstallKind::Rpm => RPM_SUFFIXES,
        LinuxInstallKind::Tarball => TARBALL_SUFFIXES,
    };
    for &suffix in suffixes {
        if let Some(u) = newer_release_for(release.clone(), current, suffix) {
            return Some(u);
        }
    }
    None
}

#[cfg_attr(not(any(windows, target_os = "linux", test)), allow(dead_code))]
fn newer_release_for(release: Release, current: &str, suffix: &str) -> Option<Update> {
    if release.draft || release.prerelease {
        return None;
    }
    let version = release.tag_name.trim_start_matches('v').to_owned();
    if !is_newer(&version, current) {
        return None;
    }
    // The name becomes a file name under %TEMP%: a plain name only, never a path.
    let installer = release.assets.iter().find(|a| {
        a.name.ends_with(suffix)
            && (a.name.starts_with("Nectarlink-")
                || a.name.starts_with("nectarlink_")
                || a.name.starts_with("nectarlink-"))
            && a.name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'))
            && !a.name.contains("..")
            && a.browser_download_url.starts_with(RELEASE_DOWNLOADS)
    })?;
    let sums = release
        .assets
        .iter()
        .find(|a| a.name == "SHA256SUMS.txt" && a.browser_download_url.starts_with(RELEASE_DOWNLOADS))?;
    Some(Update {
        version,
        notes_url: release.html_url,
        installer_url: installer.browser_download_url.clone(),
        installer_name: installer.name.clone(),
        sums_url: sums.browser_download_url.clone(),
    })
}

/// A version's numbers, and its pre-release part.
type Version<'a> = ((u64, u64, u64), Option<&'a str>);

/// `1.2.3` style versions, with an optional `-pre` part (a release is
/// newer than its pre-releases).
pub(crate) fn is_newer(candidate: &str, current: &str) -> bool {
    fn parse(v: &str) -> Option<Version<'_>> {
        let (numbers, pre) = match v.split_once('-') {
            Some((n, p)) => (n, Some(p)),
            None => (v, None),
        };
        let mut parts = numbers.split('.').map(str::parse::<u64>);
        let version = (parts.next()?.ok()?, parts.next()?.ok()?, parts.next().unwrap_or(Ok(0)).ok()?);
        Some((version, pre))
    }
    let (Some((a, a_pre)), Some((b, b_pre))) = (parse(candidate), parse(current)) else { return false };
    match a.cmp(&b) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => match (a_pre, b_pre) {
            (None, Some(_)) => true,
            (Some(x), Some(y)) => x > y,
            _ => false,
        },
    }
}

/// The SHA-256 a sums file lists for `name` (`<hex>  <name>` lines).
fn listed_hash(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.split_once(char::is_whitespace)?;
        (file.trim().trim_start_matches('*') == name && hash.len() == 64).then(|| hash.to_ascii_lowercase())
    })
}

/// Nobody has used the PC for a while, and nothing is going on that an update
/// would cut off: mirroring, an app window, a transfer, a call, screen sharing
/// or the webcam.
fn quiet() -> bool {
    if crate::win::input::is_user_active_within_ms(QUIET_FOR.as_millis() as u32) {
        return false;
    }
    let mirroring = crate::mirror::windows().iter().any(|(_, shown)| {
        matches!(shown.phase, crate::mirror::Phase::Asking | crate::mirror::Phase::Showing)
    });
    let transferring = core_host::host().hub.read(|s| {
        s.transfers.iter().any(|t| {
            matches!(
                t.transfer.state,
                nectarlink_core::TransferState::Running | nectarlink_core::TransferState::Waiting
            )
        })
    });
    let calling = crate::calls::active_or_ringing(None).is_some();
    let webcam = !matches!(crate::webcam::phase(), crate::webcam::Phase::Idle);
    !(mirroring || transferring || calling || webcam || crate::pc_mirror::is_active())
}

/// Whether the installer registered [`TASK`] on this PC.
#[cfg(windows)]
fn task_registered() -> bool {
    std::process::Command::new("schtasks.exe")
        .args(["/Query", "/TN", TASK])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(not(windows))]
fn task_registered() -> bool {
    false
}

/// Installs the update found: downloads it (reporting `progress(done,
/// total)`), then installs with [`TASK`] when it's there (no prompt),
/// otherwise by running the installer, which asks for administrator rights.
/// Blocking. Returns what went wrong, in words for the user. On success the
/// app is about to quit for the installer.
pub fn install(progress: impl FnMut(u64, u64)) -> Result<(), String> {
    let update = available().ok_or("There's no update to install.")?;
    let staged = stage(&update, progress).map_err(|e| {
        tracing::warn!(error = %e, "update download failed");
        e.for_user()
    })?;
    if task_registered() {
        match install_with_task(true) {
            Ok(()) => return Ok(()),
            Err(e) => {
                tracing::warn!(error = e, "the update task didn't start; asking for permission instead")
            }
        }
    }
    install_with_prompt(&update, &staged)
}

/// Why a download didn't produce a checked installer.
#[derive(Debug)]
pub enum StageError {
    Network(String),
    Damaged,
    Disk(String),
}

impl StageError {
    fn for_user(&self) -> String {
        match self {
            StageError::Network(_) => {
                tr!(
                    "Updater",
                    "The update couldn't be downloaded. Check your internet connection and try again."
                )
            }
            StageError::Damaged => {
                tr!("Updater", "The download was damaged, so it wasn't installed. Try again.")
            }
            StageError::Disk(_) => {
                tr!("Updater", "The update couldn't be saved. Check that there's free space and try again.")
            }
        }
    }
}

impl std::fmt::Display for StageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StageError::Network(e) => write!(f, "network: {e}"),
            StageError::Damaged => f.write_str("checksum mismatch"),
            StageError::Disk(e) => write!(f, "disk: {e}"),
        }
    }
}

/// Where the app leaves a downloaded installer for the task:
/// `%ProgramData%\Nectarlink\updates`. Anyone may write here; the task only
/// runs a copy that matches the release's own sums.
#[cfg(windows)]
fn staging_dir() -> PathBuf {
    std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
        .join("Nectarlink")
        .join("updates")
}

#[cfg(not(windows))]
#[allow(unsafe_code)]
fn current_euid() -> u32 {
    // SAFETY: geteuid takes no arguments and cannot fail.
    unsafe { libc::geteuid() }
}

#[cfg(not(windows))]
fn staging_dir() -> PathBuf {
    if let Some(base) = dirs::cache_dir().filter(|p| p.is_absolute()) {
        base.join("nectarlink").join("updates")
    } else {
        let uid = current_euid();
        std::env::temp_dir().join(format!("nectarlink-{uid}")).join("updates")
    }
}

/// The release's SHA-256 for its installer.
fn expected_hash(update: &Update) -> Result<String, StageError> {
    let sums = http::get_retrying(&update.sums_url, 64 * 1024).map_err(StageError::Network)?;
    listed_hash(&String::from_utf8_lossy(&sums), &update.installer_name).ok_or(StageError::Damaged)
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Downloads the installer into the staging folder (continuing a download
/// cut short before), checks it, and returns its path. Reuses one already
/// downloaded and checked.
pub fn stage(update: &Update, progress: impl FnMut(u64, u64)) -> Result<PathBuf, StageError> {
    let expected = expected_hash(update)?;
    let dir = staging_dir();
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .map_err(|e| StageError::Disk(e.to_string()))?;
        let meta = std::fs::symlink_metadata(&dir).map_err(|e| StageError::Disk(e.to_string()))?;
        let euid = current_euid();
        if meta.file_type().is_symlink() || !meta.is_dir() || meta.uid() != euid {
            return Err(StageError::Disk("untrusted staging directory".into()));
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        std::fs::create_dir_all(&dir).map_err(|e| StageError::Disk(e.to_string()))?;
    }
    let path = dir.join(&update.installer_name);
    let part = dir.join(format!("{}.part", update.installer_name));
    // Older versions' files.
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p != path && p != part {
                let _ = std::fs::remove_file(p);
            }
        }
    }
    if sha256_file(&path).is_ok_and(|h| h == expected) {
        return Ok(path);
    }
    let _ = std::fs::remove_file(&path);
    http::download(&update.installer_url, &part, MAX_INSTALLER, progress).map_err(StageError::Network)?;
    match sha256_file(&part) {
        Ok(actual) if actual == expected => {}
        Ok(_) => {
            let _ = std::fs::remove_file(&part);
            return Err(StageError::Damaged);
        }
        Err(e) => return Err(StageError::Disk(e.to_string())),
    }
    std::fs::rename(&part, &path).map_err(|e| StageError::Disk(e.to_string()))?;
    Ok(path)
}

/// Starts [`TASK`], and quits so the installer can replace this copy; a
/// hidden relauncher opens the new version afterwards. `shown`: open its
/// window again (the user asked for the update) rather than start in the tray.
fn install_with_task(shown: bool) -> Result<(), String> {
    let update = available().ok_or("no update")?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    spawn_relauncher(&exe, &update.version, shown)?;
    let mut cmd = std::process::Command::new("schtasks.exe");
    cmd.args(["/Run", "/TN", TASK]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let started = cmd.status().map_err(|e| e.to_string())?;
    if !started.success() {
        return Err(format!("schtasks /Run: {started}"));
    }
    tracing::info!(version = update.version, "updating; quitting for the installer");
    crate::bridge::app::request_quit();
    Ok(())
}

/// A hidden PowerShell that waits (up to ten minutes) until `version` is
/// installed and the installer has finished, then opens the app again.
#[cfg(windows)]
fn spawn_relauncher(exe: &Path, version: &str, shown: bool) -> Result<(), String> {
    // Both come from this program (its own path, and a version the release
    // check already parsed); checked again so they can't break out of the
    // script's quotes.
    let exe =
        exe.to_str().filter(|e| !e.contains('\'') && !e.contains('\n')).ok_or("unusual install path")?;
    if !version.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-')) {
        return Err("unusual version".into());
    }
    let arguments = if shown { UPDATED_ARG.to_owned() } else { format!("--minimized {UPDATED_ARG}") };
    let script = relauncher_script(exe, version, &arguments);
    let encoded: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut cmd = std::process::Command::new("powershell.exe");
    cmd.args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-EncodedCommand"])
        .arg(data_encoding::BASE64.encode(&encoded))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    cmd.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS);
    cmd.spawn().map(|_| ()).map_err(|e| format!("relauncher: {e}"))
}

#[cfg(not(windows))]
fn spawn_relauncher(exe: &Path, version: &str, shown: bool) -> Result<(), String> {
    let exe =
        exe.to_str().filter(|e| !e.contains('\'') && !e.contains('\n')).ok_or("unusual install path")?;
    if !version.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-')) {
        return Err("unusual version".into());
    }
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c").arg("sleep 2 && exec \"$@\"").arg("sh").arg(exe);
    if !shown {
        cmd.arg("--minimized");
    }
    cmd.arg(UPDATED_ARG);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("relauncher: {e}"))
}

#[cfg_attr(not(windows), allow(dead_code))]
fn relauncher_script(exe: &str, version: &str, arguments: &str) -> String {
    let start = if arguments.is_empty() {
        format!("Start-Process -FilePath '{exe}'")
    } else {
        format!("Start-Process -FilePath '{exe}' -ArgumentList '{arguments}'")
    };
    format!(
        "$key = 'HKLM:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Nectarlink'\n\
         $log = Join-Path (Split-Path -Parent '{exe}') 'update.log'\n\
         $since = Get-Date\n\
         $deadline = $since.AddMinutes(10)\n\
         Start-Sleep -Seconds 3\n\
         while ((Get-Date) -lt $deadline) {{\n\
           $installed = (Get-ItemProperty -Path $key -Name DisplayVersion -ErrorAction SilentlyContinue).DisplayVersion\n\
           $running = Get-Process -ErrorAction SilentlyContinue | Where-Object {{ $_.Name -like 'Nectarlink-*-setup' }}\n\
           if ($installed -eq '{version}' -and -not $running) {{ break }}\n\
           $logged = Get-Item -LiteralPath $log -ErrorAction SilentlyContinue\n\
           if ($logged -and $logged.LastWriteTime -gt $since -and -not $running) {{\n\
             $last = Get-Content -LiteralPath $log -Tail 1 -ErrorAction SilentlyContinue\n\
             if ($last -and $last -notmatch 'installing') {{ break }}\n\
           }}\n\
           Start-Sleep -Seconds 2\n\
         }}\n\
         {start}\n"
    )
}

/// The task's job (as SYSTEM, from [`SERVICE_ARG`]): installs the latest
/// release if it's newer than this copy. Writes what it did to `update.log`
/// beside the program, where only administrators can write.
pub fn run_service() -> ExitCode {
    let log_path =
        std::env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.join("update.log")));
    let log = |message: &str| {
        tracing::info!("{message}");
        if let Some(path) = &log_path {
            let line = format!("{} {message}\n", unix_seconds_now());
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
        }
    };
    let update = match latest() {
        Ok(Some(update)) => update,
        Ok(None) => {
            log("up to date");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            log(&format!("couldn't check: {e}"));
            return ExitCode::FAILURE;
        }
    };
    let dir = match service_dir() {
        Ok(dir) => dir,
        Err(e) => {
            log(&format!("no folder for the download: {e}"));
            return ExitCode::FAILURE;
        }
    };
    let path = match download_verified(&update, &dir) {
        Ok(path) => path,
        Err(e) => {
            log(&format!("{} not installed: {e}", update.version));
            return ExitCode::FAILURE;
        }
    };
    // Not waited for: the installer replaces this program, which must not be running.
    #[cfg(windows)]
    let spawned = {
        let mut cmd = std::process::Command::new(&path);
        cmd.arg("/S");
        cmd.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS);
        cmd.spawn()
    };
    #[cfg(not(windows))]
    let spawned = {
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or_default();
        if name.ends_with(".deb") {
            std::process::Command::new("sh")
                .args(["-c", "apt-get install -y -- \"$1\" || dpkg -i -- \"$1\"", "sh"])
                .arg(&path)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
        } else if name.ends_with(".rpm") {
            std::process::Command::new("sh")
                .args(["-c", RPM_INSTALL_COMMAND, "sh"])
                .arg(&path)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
        } else if name.ends_with(".tar.gz") {
            std::process::Command::new("sh")
                .args([
                    "-e",
                    "-c",
                    "tar -C \"$2\" -xzf \"$1\" && set -- \"$2\"/Nectarlink-*/install.sh && [ -x \"$1\" ] && \"$1\"",
                    "sh",
                ])
                .arg(&path)
                .arg(&dir)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
        } else {
            // An AppImage replaces itself from the app; the service can't.
            Err(std::io::Error::other(format!("{name} isn't installed by the update service")))
        }
    };
    match spawned {
        Ok(_) => {
            log(&format!("installing {}", update.version));
            ExitCode::SUCCESS
        }
        Err(e) => {
            log(&format!("couldn't start the installer: {e}"));
            ExitCode::FAILURE
        }
    }
}

/// Seconds since 1970, for the log (no calendar needed).
fn unix_seconds_now() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    format!("[{secs}]")
}

/// A new, empty folder for the download that only SYSTEM and administrators
/// can change: Windows' `SystemTemp` (older Windows: the SYSTEM account's
/// temp folder, whose new subfolders users can't write to). Earlier ones are
/// removed first (in `SystemTemp` only, where nobody else can create them).
/// Removes the folders earlier runs downloaded into. Links are left alone;
/// only SYSTEM (or root) can create entries where these live.
fn remove_old_service_dirs(parent: &Path) {
    let Ok(entries) = std::fs::read_dir(parent) else { return };
    for entry in entries.flatten() {
        let is_dir = entry.file_type().is_ok_and(|t| t.is_dir() && !t.is_symlink());
        if is_dir && entry.file_name().to_string_lossy().starts_with("nectarlink-update-") {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(windows)]
fn service_dir() -> std::io::Result<PathBuf> {
    let windows =
        std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let system_temp = windows.join("SystemTemp");
    // Older Windows: the SYSTEM account's temp folder, whose subfolders users
    // can't change either.
    let parent = if system_temp.is_dir() { system_temp } else { std::env::temp_dir() };
    remove_old_service_dirs(&parent);
    let mut random = [0u8; 8];
    if !crate::win::secure_random(&mut random) {
        return Err(std::io::Error::other("no secure random source"));
    }
    let name = format!("nectarlink-update-{}", random.iter().map(|b| format!("{b:02x}")).collect::<String>());
    let dir = parent.join(name);
    std::fs::create_dir(&dir)?;
    Ok(dir)
}

#[cfg(not(windows))]
fn service_dir() -> std::io::Result<PathBuf> {
    use std::os::unix::fs::DirBuilderExt;
    let parent = PathBuf::from("/var/cache");
    remove_old_service_dirs(&parent);
    let mut random = [0u8; 8];
    if !crate::win::secure_random(&mut random) {
        return Err(std::io::Error::other("no secure random source"));
    }
    let name = format!("nectarlink-update-{}", random.iter().map(|b| format!("{b:02x}")).collect::<String>());
    let dir = parent.join(name);
    std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
    Ok(dir)
}

/// The installer in `dir` (only SYSTEM can write there), checked against
/// the release's sums: copied from the app's download when that matches,
/// downloaded otherwise. Errors are for the log.
fn download_verified(update: &Update, dir: &Path) -> Result<PathBuf, String> {
    let expected = expected_hash(update).map_err(|e| e.to_string())?;
    let path = dir.join(&update.installer_name);
    // Copied first and checked after, so what's checked is what runs.
    let staged = staging_dir().join(&update.installer_name);
    if staged.is_file() && std::fs::copy(&staged, &path).is_ok() {
        if sha256_file(&path).is_ok_and(|h| h == expected) {
            return Ok(path);
        }
        let _ = std::fs::remove_file(&path);
    }
    http::download(&update.installer_url, &path, MAX_INSTALLER, |_, _| {})?;
    if sha256_file(&path).map_err(|e| e.to_string())? != expected {
        let _ = std::fs::remove_file(&path);
        return Err("checksum mismatch".into());
    }
    Ok(path)
}

/// The installer with Windows' administrator prompt (no update task): a
/// fresh copy of the download in this user's temp folder, checked again, so
/// what runs is what was checked.
#[cfg(windows)]
fn install_with_prompt(update: &Update, staged: &Path) -> Result<(), String> {
    let failed = |e: String| {
        tracing::warn!(error = e, "update failed");
        tr!("Updater", "The update couldn't be started. Try again.")
    };
    let expected = expected_hash(update).map_err(|e| e.for_user())?;
    let dir = std::env::temp_dir().join("Nectarlink update");
    let path: PathBuf = dir.join(&update.installer_name);
    std::fs::create_dir_all(&dir)
        .and_then(|()| match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        })
        .and_then(|()| std::fs::copy(staged, &path).map(|_| ()))
        .map_err(|e| failed(e.to_string()))?;
    if sha256_file(&path).map_err(|e| failed(e.to_string()))? != expected {
        let _ = std::fs::remove_file(&path);
        return Err(StageError::Damaged.for_user());
    }
    // Silent, then opens the new version (/RUN); Windows asks for
    // permission first. The installer closes this copy itself.
    crate::win::shell::run(&path, "/S /RUN").map_err(failed)?;
    Ok(())
}

/// Root-executed shell script that copies a user-staged `.deb` into a
/// root-owned `0700` directory (`/var/cache/nectarlink-update`), verifies its
/// SHA-256 digest as root on the root-owned copy, and only then runs
/// `dpkg -i`. Prevents TOCTOU replacement in the user-writable cache directory.
#[cfg(target_os = "linux")]
const PRIVILEGED_DEB_INSTALL_SCRIPT: &str = "\
dir=\"/var/cache/nectarlink-update\"\n\
rm -rf -- \"$dir\"\n\
install -d -m 0700 -o 0 -g 0 \"$dir\"\n\
trap 'rm -rf -- \"$dir\"' EXIT\n\
dst=\"$dir/nectarlink-update.deb\"\n\
install -m 0600 -o 0 -g 0 \"$1\" \"$dst\"\n\
printf '%s  %s\\n' \"$2\" \"$dst\" | sha256sum -c --status -\n\
apt-get install -y -- \"$dst\" || dpkg -i -- \"$dst\"\n";

/// Installs the RPM in `$1` with whichever package manager the system has.
/// Its SHA-256 is checked before this runs; release RPMs aren't GPG-signed.
#[cfg(not(windows))]
const RPM_INSTALL_COMMAND: &str = "\
if command -v dnf >/dev/null 2>&1; then dnf install -y --nogpgcheck -- \"$1\"; \
elif command -v zypper >/dev/null 2>&1; then zypper --non-interactive --no-gpg-checks install -- \"$1\"; \
else rpm -U --replacepkgs -- \"$1\"; fi";

/// The `.rpm` counterpart of [`PRIVILEGED_DEB_INSTALL_SCRIPT`].
#[cfg(target_os = "linux")]
const PRIVILEGED_RPM_INSTALL_SCRIPT: &str = "\
dir=\"/var/cache/nectarlink-update\"\n\
rm -rf -- \"$dir\"\n\
install -d -m 0700 -o 0 -g 0 \"$dir\"\n\
trap 'rm -rf -- \"$dir\"' EXIT\n\
dst=\"$dir/nectarlink-update.rpm\"\n\
install -m 0600 -o 0 -g 0 \"$1\" \"$dst\"\n\
printf '%s  %s\\n' \"$2\" \"$dst\" | sha256sum -c --status -\n\
set -- \"$dst\"\n\
if command -v dnf >/dev/null 2>&1; then dnf install -y --nogpgcheck -- \"$1\"; \
elif command -v zypper >/dev/null 2>&1; then zypper --non-interactive --no-gpg-checks install -- \"$1\"; \
else rpm -U --replacepkgs -- \"$1\"; fi\n";

#[cfg(target_os = "linux")]
const PRIVILEGED_TAR_INSTALL_SCRIPT: &str = "\
dir=\"/var/cache/nectarlink-update\"\n\
rm -rf -- \"$dir\"\n\
install -d -m 0700 -o 0 -g 0 \"$dir\"\n\
trap 'rm -rf -- \"$dir\"' EXIT\n\
dst=\"$dir/nectarlink-update.tar.gz\"\n\
install -m 0600 -o 0 -g 0 \"$1\" \"$dst\"\n\
printf '%s  %s\\n' \"$2\" \"$dst\" | sha256sum -c --status -\n\
tar -C \"$dir\" -xzf \"$dst\"\n\
set -- \"$dir\"/Nectarlink-*/install.sh\n\
[ -x \"$1\" ] && \"$1\"\n";

/// Starts the new version once this one has quit. If that can't be arranged
/// the app stays open (it would otherwise just disappear) and says the update
/// is in place.
#[cfg(target_os = "linux")]
fn relaunch_after_install(exe: &Path, version: &str) -> Result<(), String> {
    spawn_relauncher(exe, version, true).map_err(|e| {
        tracing::warn!(error = e, "installed, but can't start the new version");
        tr!("Updater", "The update is installed. Restart NectarLink to use it.")
    })
}

#[cfg(target_os = "linux")]
fn install_with_prompt(update: &Update, staged: &Path) -> Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    let failed = |e: String| {
        tracing::warn!(error = e, "update failed");
        tr!("Updater", "The update couldn't be started. Try again.")
    };
    let expected = expected_hash(update).map_err(|e| e.for_user())?;
    if expected.len() != 64 || !expected.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(StageError::Damaged.for_user());
    }
    if sha256_file(staged).map_err(|e| failed(e.to_string()))? != expected {
        let _ = std::fs::remove_file(staged);
        return Err(StageError::Damaged.for_user());
    }

    let staged_name = staged.file_name().and_then(|s| s.to_str()).unwrap_or_default();
    if let Some(appimage_os) = std::env::var_os("APPIMAGE")
        && staged_name.ends_with(".AppImage")
    {
        let appimage_path = PathBuf::from(appimage_os);
        let new_path = appimage_path.with_extension("AppImage.new");
        std::fs::copy(staged, &new_path).map_err(|e| failed(e.to_string()))?;
        std::fs::set_permissions(&new_path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| failed(e.to_string()))?;
        std::fs::rename(&new_path, &appimage_path).map_err(|e| failed(e.to_string()))?;
        spawn_relauncher(&appimage_path, &update.version, true)?;
        crate::bridge::app::request_quit();
        return Ok(());
    }

    if staged_name.ends_with(".deb") {
        let staged_str = staged.to_string_lossy();
        let code = crate::win::elevated::run_elevated(
            Path::new("sh"),
            &["-e", "-c", PRIVILEGED_DEB_INSTALL_SCRIPT, "sh", &staged_str, &expected],
        )
        .map_err(failed)?;
        if code != 0 {
            return Err(failed(format!("privileged .deb install exited with status {code}")));
        }
        if let Ok(exe) = std::env::current_exe() {
            relaunch_after_install(&exe, &update.version)?;
        }
        crate::bridge::app::request_quit();
        return Ok(());
    }

    if staged_name.ends_with(".rpm") {
        let staged_str = staged.to_string_lossy();
        let code = crate::win::elevated::run_elevated(
            Path::new("sh"),
            &["-e", "-c", PRIVILEGED_RPM_INSTALL_SCRIPT, "sh", &staged_str, &expected],
        )
        .map_err(failed)?;
        if code != 0 {
            return Err(failed(format!("privileged .rpm install exited with status {code}")));
        }
        if let Ok(exe) = std::env::current_exe() {
            relaunch_after_install(&exe, &update.version)?;
        }
        crate::bridge::app::request_quit();
        return Ok(());
    }

    if staged_name.ends_with(".tar.gz") {
        let exe = std::env::current_exe().ok();
        let resolved_exe = exe.as_ref().map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()));
        let needs_root =
            resolved_exe.as_ref().is_some_and(|p| p.starts_with("/opt/") || p.starts_with("/usr/"));

        if needs_root {
            let staged_str = staged.to_string_lossy();
            let code = crate::win::elevated::run_elevated(
                Path::new("sh"),
                &["-e", "-c", PRIVILEGED_TAR_INSTALL_SCRIPT, "sh", &staged_str, &expected],
            )
            .map_err(failed)?;
            if code != 0 {
                return Err(failed(format!("privileged .tar.gz install exited with status {code}")));
            }
        } else {
            let extract_dir = staging_dir().join(format!("extract-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&extract_dir);
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&extract_dir)
                .map_err(|e| failed(e.to_string()))?;
            let status = std::process::Command::new("sh")
                .args([
                    "-e",
                    "-c",
                    "tar -C \"$2\" -xzf \"$1\" && set -- \"$2\"/Nectarlink-*/install.sh && [ -x \"$1\" ] && \"$1\"",
                    "sh",
                ])
                .arg(staged)
                .arg(&extract_dir)
                .stdin(std::process::Stdio::null())
                .status()
                .map_err(|e| failed(e.to_string()))?;
            let _ = std::fs::remove_dir_all(&extract_dir);
            if !status.success() {
                return Err(failed(format!("user .tar.gz install exited with status {status}")));
            }
        }
        if let Some(exe) = exe {
            relaunch_after_install(&exe, &update.version)?;
        }
        crate::bridge::app::request_quit();
        return Ok(());
    }

    if crate::win::shell::open_url(&update.notes_url).is_err() {
        return Err(tr!("Updater", "The update couldn't be started. Try again."));
    }
    Ok(())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn install_with_prompt(_update: &Update, _staged: &Path) -> Result<(), String> {
    Err("Updates are not supported on this platform.".into())
}

/// The flag the relauncher passes to the new version.
pub const UPDATED_ARG: &str = "--updated";

/// The file that remembers which version last ran, to notice an update
/// however it was installed (the task, the installer, or setup by hand).
const LAST_VERSION_FILE: &str = "last-version";

/// Records this version as the one that ran, and returns the one before it
/// when this one is newer: the app was just updated.
pub fn note_running_version(data_dir: &Path) -> Option<String> {
    let path = data_dir.join(LAST_VERSION_FILE);
    let previous = std::fs::read_to_string(&path).ok().map(|v| v.trim().to_owned());
    let current = env!("CARGO_PKG_VERSION");
    if previous.as_deref() != Some(current)
        && let Err(e) = std::fs::write(&path, current)
    {
        tracing::warn!(error = %e, "can't note the running version");
    }
    previous.filter(|p| is_newer(current, p))
}

/// Just updated: says so, and when the window was asked for, brings it to
/// the front. Started from the background, Windows would otherwise leave
/// the new window behind whatever the user has open.
pub fn announce_update(previous: String, shown: bool) {
    let current = env!("CARGO_PKG_VERSION");
    tracing::info!(from = previous, to = current, "updated");
    after_start(
        shown,
        tr!("Updater", "NectarLink is updated to %1", current),
        tr!("Updater", "Your phone reconnects on its own."),
    );
}

/// The relauncher opened this version again because the update didn't
/// install (`update.log` says why).
pub fn announce_update_failed(shown: bool) {
    tracing::warn!("the update didn't install");
    after_start(
        shown,
        tr!("Updater", "The update didn't install"),
        tr!("Updater", "NectarLink is still on the version you had. Try again from Settings."),
    );
}

fn after_start(shown: bool, title: String, body: String) {
    std::thread::spawn(move || {
        // Once the window exists.
        std::thread::sleep(Duration::from_millis(1500));
        if shown {
            crate::bridge::app::request_activation();
        }
        toast::show(toast::Toast {
            device: TOAST_GROUP.into(),
            key: "updated".into(),
            title,
            body,
            attribution: "NectarLink".into(),
            icon: None,
            image: None,
            actions: Vec::new(),
            reply: None,
            silent: true,
            progress: None,
            call: false,
        });
    });
}

impl Clone for Asset {
    fn clone(&self) -> Self {
        Self { name: self.name.clone(), browser_download_url: self.browser_download_url.clone() }
    }
}
impl Clone for Release {
    fn clone(&self) -> Self {
        Self {
            tag_name: self.tag_name.clone(),
            html_url: self.html_url.clone(),
            draft: self.draft,
            prerelease: self.prerelease,
            assets: self.assets.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare() {
        assert!(is_newer("0.1.0", "0.0.1"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(is_newer("0.2.0", "0.2.0-beta.1"));
        assert!(is_newer("0.2.0-beta.2", "0.2.0-beta.1"));
        assert!(!is_newer("0.0.1", "0.0.1"));
        assert!(!is_newer("0.0.1", "0.1.0"));
        assert!(!is_newer("garbage", "0.1.0"));
    }

    fn release(tag: &str) -> Release {
        Release {
            tag_name: tag.into(),
            html_url: "https://github.com/alejandrokundrah/nectarlink/releases/tag/v9".into(),
            draft: false,
            prerelease: false,
            assets: vec![
                Asset {
                    name: "Nectarlink-9.0.0-x64-setup.exe".into(),
                    browser_download_url: "https://github.com/alejandrokundrah/nectarlink/releases/download/v9/setup"
                        .into(),
                },
                Asset {
                    name: "Nectarlink-9.0.0-arm64-setup.exe".into(),
                    browser_download_url:
                        "https://github.com/alejandrokundrah/nectarlink/releases/download/v9/setup-arm64".into(),
                },
                Asset {
                    name: "Nectarlink-9.0.0-x86_64.AppImage".into(),
                    browser_download_url:
                        "https://github.com/alejandrokundrah/nectarlink/releases/download/v9/appimage-x64".into(),
                },
                Asset {
                    name: "nectarlink_9.0.0_amd64.deb".into(),
                    browser_download_url:
                        "https://github.com/alejandrokundrah/nectarlink/releases/download/v9/deb-amd64".into(),
                },
                Asset {
                    name: "nectarlink-9.0.0-1.x86_64.rpm".into(),
                    browser_download_url:
                        "https://github.com/alejandrokundrah/nectarlink/releases/download/v9/rpm-x86_64".into(),
                },
                Asset {
                    name: "SHA256SUMS.txt".into(),
                    browser_download_url: "https://github.com/alejandrokundrah/nectarlink/releases/download/v9/sums"
                        .into(),
                },
            ],
        }
    }

    #[test]
    fn the_relauncher_waits_for_the_version_then_opens_the_app() {
        let script = relauncher_script(
            r"C:\Program Files\Nectarlink\nectarlink-desktop.exe",
            "0.2.0",
            "--minimized --updated",
        );
        assert!(script.contains("$installed -eq '0.2.0'"));
        assert!(script.contains("Nectarlink-*-setup"));
        assert!(script.contains(
            r"Start-Process -FilePath 'C:\Program Files\Nectarlink\nectarlink-desktop.exe' -ArgumentList '--minimized --updated'"
        ));
        let shown = relauncher_script(r"C:\Program Files\Nectarlink\nectarlink-desktop.exe", "0.2.0", "");
        assert!(!shown.contains("-ArgumentList"));
    }

    #[test]
    fn notices_a_newer_version_once() {
        let dir = tempfile::tempdir().unwrap();
        let current = env!("CARGO_PKG_VERSION");
        // A first run, then the same version again: no update.
        assert_eq!(note_running_version(dir.path()), None);
        assert_eq!(note_running_version(dir.path()), None);
        // An older version ran before this one: updated, said once.
        std::fs::write(dir.path().join(LAST_VERSION_FILE), "0.0.1\n").unwrap();
        assert_eq!(note_running_version(dir.path()).as_deref(), Some("0.0.1"));
        assert_eq!(note_running_version(dir.path()), None);
        assert_eq!(std::fs::read_to_string(dir.path().join(LAST_VERSION_FILE)).unwrap(), current);
        // Going back to an older build isn't an update.
        std::fs::write(dir.path().join(LAST_VERSION_FILE), "999.0.0").unwrap();
        assert_eq!(note_running_version(dir.path()), None);
    }

    #[test]
    fn the_relauncher_refuses_paths_and_versions_that_could_break_its_quotes() {
        let odd = Path::new(r"C:\Program Files\it's\nectarlink-desktop.exe");
        assert!(spawn_relauncher(odd, "0.2.0", false).is_err());
        let fine = Path::new(r"C:\Program Files\Nectarlink\nectarlink-desktop.exe");
        assert!(spawn_relauncher(fine, "0.2.0'; Remove-Item x; '", false).is_err());
    }

    #[test]
    fn installer_names_that_are_not_plain_file_names_are_refused() {
        for name in [r"..\..\evil-x64-setup.exe", "Nectarlink-9/../x-x64-setup.exe", "evil-x64-setup.exe"] {
            let mut r = release("v9.0.0");
            r.assets[0].name = name.into();
            assert!(newer_release_for(r, "0.0.1", "-x64-setup.exe").is_none(), "{name}");
        }
    }

    #[test]
    fn releases_become_updates_when_newer_and_complete() {
        let x64 = newer_release_for(release("v9.0.0"), "0.0.1", "-x64-setup.exe").unwrap();
        assert_eq!(
            (x64.version.as_str(), x64.installer_url.as_str()),
            ("9.0.0", "https://github.com/alejandrokundrah/nectarlink/releases/download/v9/setup")
        );
        let arm64 = newer_release_for(release("v9.0.0"), "0.0.1", "-arm64-setup.exe").unwrap();
        assert_eq!(
            (arm64.installer_name.as_str(), arm64.installer_url.as_str()),
            (
                "Nectarlink-9.0.0-arm64-setup.exe",
                "https://github.com/alejandrokundrah/nectarlink/releases/download/v9/setup-arm64"
            )
        );
        assert!(newer_release(release("v0.0.1"), "0.0.1").is_none());
        let mut draft = release("v9.0.0");
        draft.prerelease = true;
        assert!(newer_release(draft, "0.0.1").is_none());
        let mut no_sums = release("v9.0.0");
        no_sums.assets.pop();
        assert!(newer_release(no_sums, "0.0.1").is_none());
        // Files anywhere but our own releases are never downloaded.
        for elsewhere in
            ["https://example.com/setup", "https://github.com/someone/nectarlink/releases/download/v9/setup"]
        {
            let mut moved = release("v9.0.0");
            moved.assets[0].browser_download_url = elsewhere.into();
            assert!(newer_release_for(moved, "0.0.1", "-x64-setup.exe").is_none(), "{elsewhere}");
            let mut moved_sums = release("v9.0.0");
            moved_sums.assets.last_mut().unwrap().browser_download_url = elsewhere.into();
            assert!(newer_release_for(moved_sums, "0.0.1", "-x64-setup.exe").is_none(), "{elsewhere}");
        }
    }

    #[test]
    fn matches_linux_appimage_and_deb_assets() {
        let appimage = newer_release_for(release("v9.0.0"), "0.0.1", "-x86_64.AppImage").unwrap();
        assert_eq!(
            (appimage.installer_name.as_str(), appimage.installer_url.as_str()),
            (
                "Nectarlink-9.0.0-x86_64.AppImage",
                "https://github.com/alejandrokundrah/nectarlink/releases/download/v9/appimage-x64"
            )
        );
        let deb = newer_release_for(release("v9.0.0"), "0.0.1", "_amd64.deb").unwrap();
        assert_eq!(
            (deb.installer_name.as_str(), deb.installer_url.as_str()),
            (
                "nectarlink_9.0.0_amd64.deb",
                "https://github.com/alejandrokundrah/nectarlink/releases/download/v9/deb-amd64"
            )
        );
        #[cfg(target_os = "linux")]
        {
            let picked_appimage = newer_release_linux(release("v9.0.0"), "0.0.1", true).unwrap();
            assert!(picked_appimage.installer_name.ends_with(".AppImage"));
            let picked_deb = newer_release_linux(release("v9.0.0"), "0.0.1", false).unwrap();
            assert!(picked_deb.installer_name.ends_with(".deb"));
            assert!(PRIVILEGED_DEB_INSTALL_SCRIPT.contains("install -d -m 0700 -o 0 -g 0"));
            assert!(PRIVILEGED_DEB_INSTALL_SCRIPT.contains("sha256sum -c --status -"));
            assert!(PRIVILEGED_DEB_INSTALL_SCRIPT.contains("dpkg -i -- \"$dst\""));
            let rpm = newer_release_linux_kind(release("v9.0.0"), "0.0.1", LinuxInstallKind::Rpm).unwrap();
            assert_eq!(rpm.installer_name, "nectarlink-9.0.0-1.x86_64.rpm");
            assert!(PRIVILEGED_RPM_INSTALL_SCRIPT.contains("sha256sum -c --status -"));
            assert!(PRIVILEGED_RPM_INSTALL_SCRIPT.contains("dnf install -y --nogpgcheck -- \"$1\""));
            // An RPM install never falls back to a tarball, a second copy in /opt.
            let mut no_rpm = release("v9.0.0");
            no_rpm.assets.retain(|a| !a.name.ends_with(".rpm"));
            assert!(newer_release_linux_kind(no_rpm, "0.0.1", LinuxInstallKind::Rpm).is_none());
            // Nor does an AppImage take a .deb: it would stay as old as it was.
            let mut no_appimage = release("v9.0.0");
            no_appimage.assets.retain(|a| !a.name.ends_with(".AppImage"));
            assert!(newer_release_linux_kind(no_appimage, "0.0.1", LinuxInstallKind::AppImage).is_none());
        }
    }

    #[test]
    fn linux_staging_dir_is_absolute_and_under_nectarlink_updates() {
        let dir = staging_dir();
        assert!(dir.is_absolute(), "staging_dir must be absolute: {}", dir.display());
        assert!(dir.ends_with("updates"));
    }

    #[test]
    fn finds_the_installer_hash() {
        let hash = "a".repeat(64);
        let sums = format!("{hash}  Nectarlink-9.0.0-x64-setup.exe\n{}  other.txt\n", "b".repeat(64));
        assert_eq!(listed_hash(&sums, "Nectarlink-9.0.0-x64-setup.exe"), Some(hash));
        assert_eq!(listed_hash(&sums, "missing.exe"), None);
        assert_eq!(listed_hash(&format!("{}  *x.exe", "C".repeat(64)), "x.exe"), Some("c".repeat(64)));
    }
}
