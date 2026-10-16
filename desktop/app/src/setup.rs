// SPDX-License-Identifier: GPL-3.0-or-later
//! The installer's and the uninstaller's windows, and the work behind them.
//!
//! `Nectarlink-<version>-x64-setup.exe` (NSIS, `desktop/installer`) unpacks
//! the app to a temporary folder and runs it there, renamed to
//! `Nectarlink Setup.exe`, with `--setup --installer <setup.exe>`. That
//! window asks nothing; Install runs the same setup.exe again with
//! administrator rights and `/S /PROGRESS=<file>`, which installs into
//! Program Files and writes each step to the file, so the window can follow.
//!
//! Removing (Windows Settings > Apps) runs the installed app with `--remove`.
//! A program can't delete its own folder, so it first copies itself to a
//! temporary folder (as `Nectarlink Uninstall.exe`) and carries on from
//! there, then runs `uninstall.exe /S /PROGRESS=<file>` with administrator
//! rights, and finally removes this user's NectarLink data. Files received
//! from phones (`Downloads\Nectarlink`), voice recordings and synced folders
//! belong to the user and are kept.
//!
//! The renamed copies matter: the installer closes running copies of
//! `nectarlink-desktop.exe` by name, which would otherwise include these.

#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub const SETUP_ARG: &str = "--setup";
pub const REMOVE_ARG: &str = "--remove";
pub const INSTALLER_ARG: &str = "--installer";
pub const INSTALLED_ARG: &str = "--installed";

/// Where the Android app is downloaded (the done screen's QR code and link).
pub const ANDROID_URL: &str = "https://github.com/thtbee/nectarlink/releases/latest";

#[cfg(windows)]
const APP_EXE: &str = "nectarlink-desktop.exe";
#[cfg(target_os = "linux")]
const APP_EXE: &str = "nectarlink-desktop";
#[cfg(windows)]
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Nectarlink";
#[cfg(windows)]
const SETUP_COPY_NAME: &str = "Nectarlink Uninstall.exe";
#[cfg(target_os = "linux")]
const SETUP_COPY_NAME: &str = "Nectarlink Uninstall";

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const DETACHED_PROCESS: u32 = 0x0000_0008;

/// What the window is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Installing from `installer` (the setup .exe the user ran).
    Install { installer: PathBuf },
    /// Removing the copy in `installed`.
    Remove { installed: PathBuf },
}

/// The copy already on this PC, from its Apps entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub dir: PathBuf,
    pub version: String,
}

#[cfg(windows)]
pub fn installed() -> Option<Installed> {
    use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;
    let dir = crate::win::reg_string(HKEY_LOCAL_MACHINE, UNINSTALL_KEY, "InstallLocation")?;
    let version =
        crate::win::reg_string(HKEY_LOCAL_MACHINE, UNINSTALL_KEY, "DisplayVersion").unwrap_or_default();
    let dir = PathBuf::from(dir);
    dir.join(APP_EXE).is_file().then_some(Installed { dir, version })
}

#[cfg(target_os = "linux")]
pub fn installed() -> Option<Installed> {
    for candidate in ["/usr/bin", "/usr/local/bin", "/opt/nectarlink"] {
        let dir = PathBuf::from(candidate);
        if dir.join(APP_EXE).is_file() {
            return Some(Installed { dir, version: env!("CARGO_PKG_VERSION").to_owned() });
        }
    }
    None
}

/// How this install relates to the copy already on the PC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Fresh,
    Update,
    Reinstall,
    /// The PC has a newer version than this installer.
    Older,
}

pub fn kind(installed: Option<&str>, this: &str) -> Kind {
    match installed {
        None => Kind::Fresh,
        Some(v) if crate::updater::is_newer(this, v) => Kind::Update,
        Some(v) if crate::updater::is_newer(v, this) => Kind::Older,
        Some(_) => Kind::Reinstall,
    }
}

/// One line of the installer's progress file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Step(Step),
    Done,
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Closing,
    Files,
    Firewall,
    Updates,
    Shortcuts,
    Cleanup,
}

impl Step {
    /// What the window says while this step runs.
    pub fn label(self) -> &'static str {
        match self {
            Step::Closing => "Closing NectarLink",
            Step::Files => "Copying files",
            Step::Firewall => "Allowing through Windows Firewall",
            Step::Updates => "Setting up updates",
            Step::Shortcuts => "Adding shortcuts",
            Step::Cleanup => "Removing files",
        }
    }

    /// How far along the bar is when this step starts (copying files takes
    /// most of the time).
    pub fn progress(self) -> f64 {
        match self {
            Step::Closing => 0.04,
            Step::Files | Step::Cleanup => 0.10,
            Step::Firewall => 0.78,
            Step::Updates => 0.86,
            Step::Shortcuts => 0.94,
        }
    }
}

pub fn parse_event(line: &str) -> Option<Event> {
    let line = line.trim();
    if line == "done" {
        return Some(Event::Done);
    }
    if let Some(reason) = line.strip_prefix("error ") {
        return Some(Event::Failed(reason.trim().to_owned()));
    }
    let step = match line.strip_prefix("step ")?.trim() {
        "closing" => Step::Closing,
        "files" => Step::Files,
        "firewall" => Step::Firewall,
        "updates" => Step::Updates,
        "shortcuts" => Step::Shortcuts,
        "cleanup" => Step::Cleanup,
        _ => return None,
    };
    Some(Event::Step(step))
}

/// How a run ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// The user said no to the administrator prompt.
    Declined,
    Failed(String),
}

/// Installs from `installer` with administrator rights, reporting each step.
pub fn install(installer: &Path, on_event: impl FnMut(Event)) -> Outcome {
    let progress = progress_file("install");
    let params = format!("/S /PROGRESS={}", progress.display());
    run_tracked(installer, &params, &progress, Duration::from_secs(600), true, on_event)
}

/// Removes the copy in `installed` with administrator rights, then this
/// user's data.
pub fn remove(installed: &Path, mut on_event: impl FnMut(Event)) -> Outcome {
    let uninstaller = installed.join("uninstall.exe");
    if !uninstaller.is_file() {
        return Outcome::Failed(
            "The uninstaller is missing. Reinstall NectarLink, then uninstall it.".into(),
        );
    }
    let progress = progress_file("remove");
    let params = format!("/S /PROGRESS={}", progress.display());
    // The uninstaller hands over to a copy of itself and exits at once, so
    // only the progress file says when it's finished.
    let outcome =
        run_tracked(&uninstaller, &params, &progress, Duration::from_secs(300), false, &mut on_event);
    if outcome == Outcome::Done {
        remove_user_data();
    }
    outcome
}

/// Removes this user's settings, pairings, history and caches. Received
/// files, recordings and synced folders live elsewhere and stay.
pub fn remove_user_data() {
    let Some(data) = user_data_dir() else { return };
    if let Err(e) = std::fs::remove_dir_all(&data)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(error = %e, "can't remove all of NectarLink's data");
    }
}

/// `%LOCALAPPDATA%\Nectarlink`, the app's default data folder.
#[cfg(windows)]
fn user_data_dir() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?;
    local.is_absolute().then(|| local.join("Nectarlink"))
}

#[cfg(target_os = "linux")]
fn user_data_dir() -> Option<PathBuf> {
    dirs::data_local_dir().map(|d| d.join("Nectarlink"))
}

/// Runs `program` elevated with `params` and follows `progress` until it
/// reports done or an error. With `wait_for_exit`, the process ending also
/// ends the run (its exit code decides when the file says nothing).
fn run_tracked(
    program: &Path,
    params: &str,
    progress: &Path,
    limit: Duration,
    wait_for_exit: bool,
    mut on_event: impl FnMut(Event),
) -> Outcome {
    let _ = std::fs::remove_file(progress);
    let process = match crate::win::elevated::start(program, params) {
        Ok(process) => process,
        Err(crate::win::elevated::Error::Declined) => return Outcome::Declined,
        Err(crate::win::elevated::Error::Other(e)) => {
            return Outcome::Failed(format!("Couldn't start setup: {e}"));
        }
    };
    let started = Instant::now();
    let mut read = 0usize;
    loop {
        for event in new_events(progress, &mut read) {
            match event {
                Event::Done => {
                    let _ = std::fs::remove_file(progress);
                    return Outcome::Done;
                }
                Event::Failed(reason) => {
                    let _ = std::fs::remove_file(progress);
                    return Outcome::Failed(reason);
                }
                step @ Event::Step(_) => on_event(step),
            }
        }
        if wait_for_exit && let Some(code) = process.exit_code() {
            // Read what it wrote just before exiting.
            for event in new_events(progress, &mut read) {
                match event {
                    Event::Done => return Outcome::Done,
                    Event::Failed(reason) => return Outcome::Failed(reason),
                    step @ Event::Step(_) => on_event(step),
                }
            }
            let _ = std::fs::remove_file(progress);
            return if code == 0 {
                Outcome::Done
            } else {
                Outcome::Failed(format!("Setup stopped with code {code}."))
            };
        }
        if started.elapsed() > limit {
            return Outcome::Failed("Setup is taking too long. Try again after restarting the PC.".into());
        }
        std::thread::sleep(Duration::from_millis(120));
    }
}

/// Lines added to the progress file since the last call.
fn new_events(path: &Path, read: &mut usize) -> Vec<Event> {
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    // A line still being written has no newline yet: only whole lines count.
    let whole = text.rfind('\n').map_or("", |end| &text[..=end]);
    let lines: Vec<&str> = whole.lines().collect();
    let fresh: Vec<Event> = lines.iter().skip(*read).filter_map(|l| parse_event(l)).collect();
    *read = lines.len();
    fresh
}

fn progress_file(what: &str) -> PathBuf {
    std::env::temp_dir().join(format!("nectarlink-{what}-{}.progress", std::process::id()))
}

/// Starts the installed app as this user, not elevated. Through Explorer, so
/// it starts as a fresh process of the shell and inherits nothing from
/// whatever launched setup.
#[cfg(windows)]
pub fn open_app(dir: &Path) -> Result<(), String> {
    let app = dir.join(APP_EXE);
    if !app.is_file() {
        return Err(format!("{} is missing", app.display()));
    }
    std::process::Command::new("explorer.exe")
        .arg(&app)
        .creation_flags(DETACHED_PROCESS)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(target_os = "linux")]
pub fn open_app(dir: &Path) -> Result<(), String> {
    let app = dir.join(APP_EXE);
    if !app.is_file() {
        return Err(format!("{} is missing", app.display()));
    }
    std::process::Command::new(&app)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// For `--remove` started from the installed folder: carries on from a copy
/// in a temporary folder, so the installed one can be deleted. `Ok(true)`
/// when the copy took over and this process should exit.
pub fn hand_over_removal(installed: &Path) -> Result<bool, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let temp = std::env::temp_dir();
    if exe.starts_with(&temp)
        || (cfg!(debug_assertions) && std::env::var_os("NECTARLINK_SETUP_PREVIEW").is_some())
    {
        return Ok(false);
    }
    let copy = temp.join(format!("Nectarlink-uninstall-{}", std::process::id()));
    copy_tree(installed, &copy).map_err(|e| format!("can't prepare removal: {e}"))?;
    let app = copy.join(APP_EXE);
    let renamed = copy.join(SETUP_COPY_NAME);
    std::fs::rename(&app, &renamed).map_err(|e| e.to_string())?;
    std::process::Command::new(&renamed)
        .arg(REMOVE_ARG)
        .arg(INSTALLED_ARG)
        .arg(installed)
        .current_dir(&copy)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(true)
}

/// Deletes this program's temporary copy once it has exited.
#[cfg(windows)]
pub fn delete_own_copy_later() {
    let Ok(exe) = std::env::current_exe() else { return };
    let Some(dir) = exe.parent() else { return };
    let temp = std::env::temp_dir();
    // Only ever a folder this module made under %TEMP%.
    let ours = dir.starts_with(&temp)
        && dir.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("Nectarlink-uninstall-"));
    if !ours {
        return;
    }
    let script = format!("/c ping -n 4 127.0.0.1 >nul & rmdir /s /q \"{}\"", dir.display());
    let _ = std::process::Command::new("cmd.exe")
        .raw_arg(script)
        .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
        .spawn();
}

#[cfg(target_os = "linux")]
pub fn delete_own_copy_later() {
    let Ok(exe) = std::env::current_exe() else { return };
    let Some(dir) = exe.parent() else { return };
    let temp = std::env::temp_dir();
    let ours = dir.starts_with(&temp)
        && dir.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("Nectarlink-uninstall-"));
    if !ours {
        return;
    }
    let _ = std::process::Command::new("sh")
        .arg("-c")
        .arg("sleep 3 && rm -rf \"$1\"")
        .arg("sh")
        .arg(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// The window's QR code for the Android app, as an SVG data URL.
pub fn android_qr(dark: &str, light: &str) -> String {
    match crate::qr::encode(ANDROID_URL) {
        Ok(qr) => format!(
            "data:image/svg+xml;utf8,<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"-2 -2 {s} {s}\" \
             shape-rendering=\"crispEdges\"><rect x=\"-2\" y=\"-2\" width=\"{s}\" height=\"{s}\" fill=\"{light}\"/>\
             <path d=\"{path}\" fill=\"{dark}\"/></svg>",
            s = qr.size + 4,
            path = qr.path,
            light = light.replace('#', "%23"),
            dark = dark.replace('#', "%23"),
        ),
        Err(_) => String::new(),
    }
}

/// The app icon as a PNG data URL, at `size` pixels.
pub fn icon_url(size: usize) -> String {
    use data_encoding::BASE64;
    let mut bgra = crate::mark::render(size);
    for px in bgra.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
    let bitmap = crate::win::image::Bitmap { width: size as u32, height: size as u32, bgra };
    crate::win::image::encode_png(&bitmap)
        .map(|png| format!("data:image/png;base64,{}", BASE64.encode(&png)))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_lines_parse() {
        assert_eq!(parse_event("step files"), Some(Event::Step(Step::Files)));
        assert_eq!(parse_event("step firewall\r"), Some(Event::Step(Step::Firewall)));
        assert_eq!(parse_event("done"), Some(Event::Done));
        assert_eq!(parse_event("error Not enough space"), Some(Event::Failed("Not enough space".into())));
        assert_eq!(parse_event("step something-new"), None);
        assert_eq!(parse_event(""), None);
    }

    #[test]
    fn steps_only_move_the_bar_forward() {
        let order = [Step::Closing, Step::Files, Step::Firewall, Step::Updates, Step::Shortcuts];
        assert!(order.windows(2).all(|w| w[0].progress() < w[1].progress()));
    }

    #[test]
    fn install_kind_compares_versions() {
        assert_eq!(kind(None, "0.1.0"), Kind::Fresh);
        assert_eq!(kind(Some("0.1.0"), "0.1.1"), Kind::Update);
        assert_eq!(kind(Some("0.1.0"), "0.1.0"), Kind::Reinstall);
        assert_eq!(kind(Some("0.2.0"), "0.1.0"), Kind::Older);
    }

    #[test]
    fn new_events_reads_only_what_was_added() {
        let dir = std::env::temp_dir().join(format!("nectarlink-setup-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("p.progress");
        std::fs::write(&file, "step closing\nstep files\n").unwrap();
        let mut read = 0;
        assert_eq!(new_events(&file, &mut read), vec![Event::Step(Step::Closing), Event::Step(Step::Files)]);
        assert_eq!(new_events(&file, &mut read), vec![]);
        std::fs::write(&file, "step closing\nstep files\nstep firewall\ndone\n").unwrap();
        assert_eq!(new_events(&file, &mut read), vec![Event::Step(Step::Firewall), Event::Done]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn copy_tree_copies_nested_files() {
        let base = std::env::temp_dir().join(format!("nectarlink-copytree-{}", std::process::id()));
        let from = base.join("from");
        std::fs::create_dir_all(from.join("qml/QtQuick")).unwrap();
        std::fs::write(from.join("a.dll"), b"a").unwrap();
        std::fs::write(from.join("qml/QtQuick/qmldir"), b"q").unwrap();
        copy_tree(&from, &base.join("to")).unwrap();
        assert_eq!(std::fs::read(base.join("to/qml/QtQuick/qmldir")).unwrap(), b"q");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn qr_is_an_svg_data_url() {
        let url = android_qr("#2C1600", "#FFFFFF");
        assert!(url.starts_with("data:image/svg+xml;utf8,<svg"));
        assert!(url.contains("%232C1600"));
    }
}
