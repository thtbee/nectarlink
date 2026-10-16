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

const LATEST: &str = "https://api.github.com/repos/thtbee/nectarlink/releases/latest";
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
            Some(format!("NectarLink {} is available.", update.version))
        }
        Ok(None) => told.then(|| "NectarLink is up to date.".to_owned()),
        Err(e) => {
            tracing::info!(error = %e, "couldn't check for updates");
            told.then(|| "Couldn't check for updates. Check your internet connection.".to_owned())
        }
    }
}

fn announce(update: &Update) {
    toast::show(toast::Toast {
        device: TOAST_GROUP.into(),
        key: update.version.clone(),
        title: format!("NectarLink {} is available", update.version),
        body: "Update now, or later from Settings.".into(),
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

const INSTALLER_SUFFIX: &str =
    if cfg!(target_arch = "aarch64") { "-arm64-setup.exe" } else { "-x64-setup.exe" };

/// The release as an update, if it's a newer published one with an
/// installer for this architecture and its sums.
fn newer_release(release: Release, current: &str) -> Option<Update> {
    newer_release_for(release, current, INSTALLER_SUFFIX)
}

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
            && a.name.starts_with("Nectarlink-")
            && a.name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'))
            && !a.name.contains("..")
    })?;
    let sums = release.assets.iter().find(|a| a.name == "SHA256SUMS.txt")?;
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

#[cfg(target_os = "linux")]
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
                "The update couldn't be downloaded. Check your internet connection and try again.".into()
            }
            StageError::Damaged => "The download was damaged, so it wasn't installed. Try again.".into(),
            StageError::Disk(_) => {
                "The update couldn't be saved. Check that there's free space and try again.".into()
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
fn staging_dir() -> PathBuf {
    std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
        .join("Nectarlink")
        .join("updates")
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
    std::fs::create_dir_all(&dir).map_err(|e| StageError::Disk(e.to_string()))?;
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
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS);
    cmd.spawn().map(|_| ()).map_err(|e| format!("relauncher: {e}"))
}

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
    let mut cmd = std::process::Command::new(&path);
    cmd.arg("/S");
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS);
    match cmd.spawn() {
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
fn service_dir() -> std::io::Result<PathBuf> {
    let windows =
        std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let system_temp = windows.join("SystemTemp");
    let parent = if system_temp.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&system_temp) {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with("nectarlink-update-") {
                    let _ = std::fs::remove_dir_all(entry.path());
                }
            }
        }
        system_temp
    } else {
        std::env::temp_dir()
    };
    let mut random = [0u8; 8];
    if !crate::win::secure_random(&mut random) {
        return Err(std::io::Error::other("no secure random source"));
    }
    let name = format!("nectarlink-update-{}", random.iter().map(|b| format!("{b:02x}")).collect::<String>());
    let dir = parent.join(name);
    std::fs::create_dir(&dir)?;
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
fn install_with_prompt(update: &Update, staged: &Path) -> Result<(), String> {
    let failed = |e: String| {
        tracing::warn!(error = e, "update failed");
        "The update couldn't be started. Try again.".to_owned()
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
        format!("NectarLink is updated to {current}"),
        "Your phone reconnects on its own.".into(),
    );
}

/// The relauncher opened this version again because the update didn't
/// install (`update.log` says why).
pub fn announce_update_failed(shown: bool) {
    tracing::warn!("the update didn't install");
    after_start(
        shown,
        "The update didn't install".into(),
        "NectarLink is still on the version you had. Try again from Settings.".into(),
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
            html_url: "https://github.com/thtbee/nectarlink/releases/tag/v9".into(),
            draft: false,
            prerelease: false,
            assets: vec![
                Asset {
                    name: "Nectarlink-9.0.0-x64-setup.exe".into(),
                    browser_download_url: "https://x/setup".into(),
                },
                Asset {
                    name: "Nectarlink-9.0.0-arm64-setup.exe".into(),
                    browser_download_url: "https://x/setup-arm64".into(),
                },
                Asset { name: "SHA256SUMS.txt".into(), browser_download_url: "https://x/sums".into() },
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
        assert_eq!((x64.version.as_str(), x64.installer_url.as_str()), ("9.0.0", "https://x/setup"));
        let arm64 = newer_release_for(release("v9.0.0"), "0.0.1", "-arm64-setup.exe").unwrap();
        assert_eq!(
            (arm64.installer_name.as_str(), arm64.installer_url.as_str()),
            ("Nectarlink-9.0.0-arm64-setup.exe", "https://x/setup-arm64")
        );
        assert!(newer_release(release("v0.0.1"), "0.0.1").is_none());
        let mut draft = release("v9.0.0");
        draft.prerelease = true;
        assert!(newer_release(draft, "0.0.1").is_none());
        let mut no_sums = release("v9.0.0");
        no_sums.assets.pop();
        assert!(newer_release(no_sums, "0.0.1").is_none());
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
