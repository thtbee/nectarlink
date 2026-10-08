// SPDX-License-Identifier: GPL-3.0-or-later
//! Explorer's "Send to" menu: a shortcut per paired phone, which starts
//! Nectarlink with `--send-to <device> <files…>`. The running app sends the
//! files and follows the transfer in a Windows notification with a progress
//! bar, so nothing else needs to open.

use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    sync::{
        Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use nectarlink_core::{DeviceId, Error, NodeEvent, Transfer, TransferFailure, TransferState};

use crate::{
    bridge::app::{describe, request_activation},
    core_host,
    launch::Request,
    state::{Changes, CoreStatus},
    win::{
        shortcut::{self, Shortcut},
        toast::{self, Progress, Toast},
    },
};

/// The command-line option the shortcuts pass.
pub const ARG: &str = "--send-to";
/// The toast "device" for sends from Explorer; their key is the transfer ID.
pub const TOAST_GROUP: &str = "sends";
/// The toast's "Cancel" action.
pub const ACTION_CANCEL: &str = "cancel";

/// How long a send from Explorer waits for the app to start.
const START_TIMEOUT: Duration = Duration::from_secs(60);
/// Progress bars move at most this often (Windows animates in between).
const PROGRESS_INTERVAL: Duration = Duration::from_millis(500);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ---- Shortcuts ----

/// Whether this instance keeps the shortcuts: only the installed app with
/// its usual data folder, never a test instance with its own.
static MANAGED: AtomicBool = AtomicBool::new(false);
/// The "Show your phones in Send to" preference.
static ENABLED: AtomicBool = AtomicBool::new(true);
/// The devices the shortcuts should currently be made for, and what was last applied.
static DESIRED: Mutex<Option<Vec<(DeviceId, String)>>> = Mutex::new(None);
static APPLIED: Mutex<Option<Vec<(DeviceId, String)>>> = Mutex::new(None);
/// One update of the folder at a time.
static UPDATING: Mutex<()> = Mutex::new(());

/// Keeps the shortcuts in step with the paired devices from now on.
pub fn start() {
    MANAGED.store(true, Ordering::Relaxed);
    core_host::host().hub.subscribe(Changes::DEVICES | Changes::STATUS, || {
        sync();
        true
    });
}

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
    sync();
}

fn sync() {
    if !MANAGED.load(Ordering::Relaxed) {
        return;
    }
    let host = core_host::host();
    // Until the core is up, the device list is empty, not known.
    let desired = host.hub.read(|s| {
        matches!(s.status, Some(CoreStatus::Ready { .. }))
            .then(|| s.devices.iter().map(|d| (d.id, d.info.name.clone())).collect::<Vec<_>>())
    });
    let Some(mut desired) = desired else { return };
    if !ENABLED.load(Ordering::Relaxed) {
        desired.clear();
    }
    {
        let mut target = lock(&DESIRED);
        if target.as_ref() == Some(&desired) && lock(&APPLIED).as_ref() == Some(&desired) {
            return;
        }
        *target = Some(desired);
    }
    let spawned = std::thread::Builder::new().name("send-to".into()).spawn(|| {
        let _one = lock(&UPDATING);
        let (Some(dir), Ok(exe)) = (shortcut::send_to_dir(), std::env::current_exe()) else { return };
        loop {
            let next = lock(&DESIRED).clone();
            let Some(desired) = next else { return };
            if lock(&APPLIED).as_ref() == Some(&desired) {
                return;
            }
            match reconcile(&dir, &desired, &exe) {
                Ok(()) => *lock(&APPLIED) = Some(desired),
                Err(e) => {
                    tracing::warn!(error = %e, "can't update the Send to menu");
                    *lock(&APPLIED) = None;
                    return;
                }
            }
        }
    });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "can't update the Send to menu");
    }
}

/// Removes this app's shortcuts (when uninstalling).
pub fn remove_all() -> io::Result<()> {
    match (shortcut::send_to_dir(), std::env::current_exe()) {
        (Some(dir), Ok(exe)) => reconcile(&dir, &[], &exe),
        _ => Ok(()),
    }
}

/// Makes `dir` hold exactly one shortcut per device in `desired` (named
/// after it) and none for any other; other apps' shortcuts are left alone.
fn reconcile(dir: &Path, desired: &[(DeviceId, String)], exe: &Path) -> io::Result<()> {
    let mut ours = Vec::new();
    let mut theirs = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().to_lowercase()) else { continue };
        let is_ours = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("lnk"))
            && shortcut::arguments(&path).is_some_and(|a| a.starts_with(&format!("{ARG} ")));
        if is_ours { ours.push(path) } else { theirs.push(name) }
    }

    let mut planned: Vec<(PathBuf, Shortcut)> = Vec::new();
    for (id, name) in desired {
        let base = file_name(name);
        let taken = |candidate: &str| {
            let lower = format!("{candidate}.lnk").to_lowercase();
            theirs.contains(&lower)
                || planned
                    .iter()
                    .any(|(p, _)| p.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase() == lower))
        };
        let candidate = std::iter::once(base.clone())
            .chain(std::iter::once(format!("{base} (Nectarlink)")))
            .chain((2..).map(|n| format!("{base} (Nectarlink {n})")))
            .find(|c| !taken(c))
            .expect("an endless list has a free name");
        planned.push((
            dir.join(format!("{candidate}.lnk")),
            Shortcut {
                target: exe.to_owned(),
                arguments: format!("{ARG} {id}"),
                description: format!("Send to {name} with Nectarlink"),
            },
        ));
    }

    for path in ours {
        let lower = path.to_string_lossy().to_lowercase();
        if !planned.iter().any(|(p, _)| p.to_string_lossy().to_lowercase() == lower) {
            std::fs::remove_file(&path)?;
        }
    }
    // Always rewritten: keeps them pointing at this copy of the app.
    for (path, link) in &planned {
        shortcut::write(path, link).map_err(io::Error::other)?;
    }
    Ok(())
}

/// A device name as a file name Explorer accepts, and shows as the device.
fn file_name(device: &str) -> String {
    let cleaned: String =
        device.chars().map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { ' ' } else { c }).collect();
    let mut name = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    name = name.trim_end_matches(['.', ' ']).chars().take(64).collect();
    let stem = name.split('.').next().unwrap_or_default().to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    if name.is_empty() {
        "Phone".into()
    } else if reserved {
        format!("{name} phone")
    } else {
        name
    }
}

// ---- Sending ----

/// What another launch asked for.
pub fn handle(request: Request) {
    match request {
        Request::Show => request_activation(),
        Request::Quit => crate::bridge::app::request_quit(),
        Request::Send { device, paths } => send(device, paths),
        Request::TaskNotify { device, task } => task_notify(device, task),
    }
}

fn task_notify(device: Option<String>, task: nectarlink_core::TaskNotify) {
    core_host::spawn(async move {
        let Ok(Some(node)) = tokio::time::timeout(START_TIMEOUT, core_host::wait_for_node()).await else {
            tracing::warn!("task notification wasn't sent: the core didn't start");
            return;
        };
        let query = device.as_deref().map(str::trim).filter(|s| !s.is_empty());
        match query {
            None => {
                if let Err(e) = node.task_notify(None, task).await {
                    tracing::debug!(error = %e, "task.notify wasn't delivered");
                }
            }
            Some(q) => {
                let Ok(devices) = node.paired_devices() else { return };
                let lower = q.to_lowercase();
                for d in devices.into_iter().filter(|d| {
                    d.id.to_string().starts_with(&lower) || d.info.name.to_lowercase().contains(&lower)
                }) {
                    if let Err(e) = node.task_notify(Some(d.id), task.clone()).await {
                        tracing::debug!(device = %d.id, error = %e, "task.notify wasn't delivered");
                    }
                }
            }
        }
    });
}

/// A send from Explorer, followed in a notification.
#[derive(Debug)]
struct Tracked {
    device_name: String,
    shown: Option<(Instant, Progress)>,
}

static TRACKED: Mutex<Option<HashMap<String, Tracked>>> = Mutex::new(None);

fn send(device: String, paths: Vec<PathBuf>) {
    core_host::spawn(async move {
        let Ok(Some(node)) = tokio::time::timeout(START_TIMEOUT, core_host::wait_for_node()).await else {
            tracing::warn!("files from Explorer weren't sent: the core didn't start");
            return;
        };
        let name = device
            .parse::<DeviceId>()
            .ok()
            .and_then(|id| core_host::host().hub.read(|s| s.name_of(&id)).map(|name| (id, name)));
        let Some((id, name)) = name else {
            return failed("Couldn't send", "That device isn't paired with this PC anymore.");
        };
        let outgoing = match tokio::task::spawn_blocking(move || nectarlink_core::outgoing_paths(&paths))
            .await
        {
            Ok(Ok(files)) if !files.is_empty() => files,
            Ok(Ok(_)) => {
                return failed(&format!("Couldn't send to {name}"), "There's nothing to send in there.");
            }
            Ok(Err(Error::TooLarge)) => {
                return failed(&format!("Couldn't send to {name}"), "That's too many files to send at once.");
            }
            Ok(Err(_)) | Err(_) => {
                return failed(&format!("Couldn't send to {name}"), "Those files can't be read.");
            }
        };
        match node.send_files(id, outgoing).await {
            Ok(transfer_id) => track(transfer_id, name),
            Err(Error::Denied) => failed(
                &format!("Couldn't send to {name}"),
                "Files are turned off for this device. Turn them on in Nectarlink.",
            ),
            Err(e) => failed(&format!("Couldn't send to {name}"), &describe(&e)),
        }
    });
}

fn failed(title: &str, body: &str) {
    toast::show(Toast {
        device: TOAST_GROUP.into(),
        key: format!("failed-{title}"),
        title: title.into(),
        body: body.into(),
        attribution: "Nectarlink".into(),
        icon: None,
        image: None,
        actions: Vec::new(),
        reply: None,
        silent: false,
        progress: None,
        call: false,
    });
}

fn track(id: String, device_name: String) {
    let mut tracked = lock(&TRACKED);
    let map = tracked.get_or_insert_default();
    map.insert(id.clone(), Tracked { device_name, shown: None });
    // Events before this point went to the hub only; catch up from there.
    let now = core_host::host()
        .hub
        .read(|s| s.transfers.iter().find(|t| t.transfer.id == id).map(|t| t.transfer.clone()));
    if let Some(transfer) = now {
        update(map, &transfer);
    }
}

/// Follows sends from Explorer in their notification.
pub fn on_event(event: &NodeEvent) {
    let NodeEvent::Transfer(transfer) = event else { return };
    let mut tracked = lock(&TRACKED);
    if let Some(map) = tracked.as_mut() {
        update(map, transfer);
    }
}

fn update(map: &mut HashMap<String, Tracked>, transfer: &Transfer) {
    let Some(tracked) = map.get_mut(&transfer.id) else { return };
    let title = crate::transfers::title(transfer);
    let device = tracked.device_name.clone();
    let finished = |title: String, body: String| Toast {
        device: TOAST_GROUP.into(),
        key: transfer.id.clone(),
        title,
        body,
        attribution: "Nectarlink".into(),
        icon: None,
        image: None,
        actions: Vec::new(),
        reply: None,
        silent: false,
        progress: None,
        call: false,
    };
    match &transfer.state {
        TransferState::Waiting | TransferState::Running => {
            let progress = Progress {
                status: if transfer.state == TransferState::Waiting {
                    format!("Waiting for {device}…")
                } else {
                    format!("Sending to {device}…")
                },
                value: (transfer.total > 0).then(|| transfer.done as f64 / transfer.total as f64),
                label: format!("{} of {}", size(transfer.done), size(transfer.total)),
            };
            match &tracked.shown {
                None => toast::show(Toast {
                    actions: vec![(ACTION_CANCEL.into(), "Cancel".into())],
                    progress: Some(progress.clone()),
                    ..finished(title, String::new())
                }),
                Some((at, last))
                    if *last != progress
                        && (at.elapsed() >= PROGRESS_INTERVAL || last.status != progress.status) =>
                {
                    toast::update_progress(TOAST_GROUP, &transfer.id, progress.clone());
                }
                Some(_) => return,
            }
            tracked.shown = Some((Instant::now(), progress));
        }
        TransferState::Done { .. } => {
            toast::show(finished(title, format!("Sent to {device}")));
            map.remove(&transfer.id);
        }
        TransferState::Failed(failure) => {
            let body = match failure {
                TransferFailure::Denied => format!("{device} doesn't accept files from this PC."),
                TransferFailure::Unreachable => format!("Couldn't reach {device}."),
                TransferFailure::NoSpace => format!("{device} is out of space."),
                TransferFailure::Interrupted | TransferFailure::Other(_) => {
                    "The transfer didn't go through.".into()
                }
            };
            toast::show(finished(format!("Couldn't send {title}"), body));
            map.remove(&transfer.id);
        }
        TransferState::Cancelled => {
            toast::remove(TOAST_GROUP, &transfer.id);
            map.remove(&transfer.id);
        }
    }
}

/// "12.5 MB": decimal units, as Explorer shows them.
fn size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    match unit {
        0 => format!("{bytes} bytes"),
        _ if value < 10.0 => format!("{value:.1} {}", UNITS[unit]),
        _ => format!("{value:.0} {}", UNITS[unit]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_names_become_file_names() {
        assert_eq!(file_name("Pixel 9 Pro"), "Pixel 9 Pro");
        assert_eq!(file_name("Bee's \"phone\": <1/2>?"), "Bee's phone 1 2");
        assert_eq!(file_name("  phone.. "), "phone");
        assert_eq!(file_name("\u{7}"), "Phone");
        assert_eq!(file_name("CON"), "CON phone");
        assert_eq!(file_name("com1"), "com1 phone");
        assert_eq!(file_name("Combo"), "Combo");
        assert_eq!(file_name(&"x".repeat(100)).len(), 64);
    }

    #[test]
    fn sizes_read_like_explorer() {
        assert_eq!(size(0), "0 bytes");
        assert_eq!(size(999), "999 bytes");
        assert_eq!(size(1_500), "1.5 KB");
        assert_eq!(size(12_600_000), "13 MB");
        assert_eq!(size(3_200_000_000), "3.2 GB");
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn shortcuts_follow_the_paired_devices() {
        let dir = tempfile::tempdir().unwrap();
        let exe = std::env::current_exe().unwrap();
        // Someone else's entry with a device's name is kept.
        std::fs::write(dir.path().join("Tab.lnk"), b"not ours").unwrap();
        let (a, b, c) = (DeviceId([1; 32]), DeviceId([2; 32]), DeviceId([3; 32]));

        reconcile(dir.path(), &[(a, "Pixel".into()), (b, "Tab".into()), (c, "Pixel".into())], &exe).unwrap();
        assert_eq!(
            names(dir.path()),
            ["Pixel (Nectarlink).lnk", "Pixel.lnk", "Tab (Nectarlink).lnk", "Tab.lnk"]
        );
        assert_eq!(shortcut::arguments(&dir.path().join("Pixel.lnk")).unwrap(), format!("{ARG} {a}"));
        assert_eq!(
            shortcut::arguments(&dir.path().join("Pixel (Nectarlink).lnk")).unwrap(),
            format!("{ARG} {c}")
        );

        // Unpairing and renaming.
        reconcile(dir.path(), &[(c, "Pixel 9".into())], &exe).unwrap();
        assert_eq!(names(dir.path()), ["Pixel 9.lnk", "Tab.lnk"]);
        assert_eq!(shortcut::arguments(&dir.path().join("Pixel 9.lnk")).unwrap(), format!("{ARG} {c}"));

        // Turned off.
        reconcile(dir.path(), &[], &exe).unwrap();
        assert_eq!(names(dir.path()), ["Tab.lnk"]);
    }
}
