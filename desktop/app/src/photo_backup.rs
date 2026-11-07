// SPDX-License-Identifier: GPL-3.0-or-later
//! Photo backup: copies a phone's new photos (and, if chosen, videos) into a
//! folder on this PC, filed by year and month, over the photos service
//! (docs/protocol/photos.md). It only needs the phone's photos permission,
//! unlike Folder Sync (crate::folder_sync), which works on any folder
//! through the storage service.
//!
//! Nothing is ever deleted, here or on the phone: an item counts as backed
//! up once its file is in the folder, and stays counted if the file is later
//! moved or deleted there. Downloads are quiet (no Timeline entries, toasts
//! or transfer rows) and one item at a time, so a lost connection costs at
//! most one file, and a transfer that resumes after a restart is still
//! filed: what each download is for is saved with the backup's state.

use crate::i18n::tr;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use nectarlink_core::{DeviceId, Error, LinkState, NodeEvent, PhotoItem, Transfer, TransferState};
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use crate::{core_host, state::Changes};

/// Items asked for per page while looking for new ones (the protocol's cap).
const PAGE: u32 = 200;
/// A library larger than this many pages is backed up from the newest end only.
const MAX_PAGES: usize = 2_000;
/// Looks through the whole library again after this long, to catch items
/// an earlier pass missed (one added with an old date, say).
const FULL_SCAN_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// How often to look for new photos while nothing else wakes the backup.
const LOOK_EVERY: Duration = Duration::from_secs(15 * 60);
/// A download that makes no progress for this long is left to resume later.
const STALLED_AFTER: Duration = Duration::from_secs(3 * 60);
/// Free space kept on the folder's drive besides the file itself.
const SPACE_MARGIN: u64 = 512 * 1024 * 1024;
/// Saves the list of backed-up items after this many new ones (and at the end).
const SAVE_EVERY: usize = 25;
/// How long a finished transfer is remembered before the backup claims it.
const KEEP_RECENT: Duration = Duration::from_secs(120);
/// A download with no progress for this long (or, after a restart, none
/// since) is given up, and the next pass asks for the item again.
const GIVE_UP_AFTER: Duration = Duration::from_secs(30 * 60);

/// One phone's backup settings and progress, saved in
/// `<data dir>/photo-backup/<device>.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DeviceBackup {
    pub enabled: bool,
    /// Where files go; `None` means the default (Pictures\NectarLink\<phone>).
    pub folder: Option<PathBuf>,
    pub videos: bool,
    /// Items backed up, by the phone's ID, with their size.
    pub done: HashMap<String, u64>,
    /// Downloads under way: transfer ID to the item it brings.
    pub pending: HashMap<String, Pending>,
    /// When the whole library was last looked through (Unix ms).
    pub full_scan_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pending {
    pub id: String,
    pub name: String,
    pub size: u64,
    /// When it was taken (Unix ms), for its year and month folder.
    pub date: i64,
}

impl From<&PhotoItem> for Pending {
    fn from(item: &PhotoItem) -> Self {
        Pending { id: item.id.clone(), name: item.name.clone(), size: item.size, date: item.date }
    }
}

/// What a phone's backup is doing, for Settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Activity {
    #[default]
    Idle,
    /// Off, or the phone isn't connected.
    Waiting,
    /// Held back by "Sync on Wi-Fi only" while the phone is away.
    WaitingForWifi,
    Looking,
    Copying {
        left: usize,
    },
    /// Stopped by a problem the user can fix; the text says what.
    Problem(String),
}

#[derive(Default)]
struct State {
    loaded: bool,
    /// When this state was first used (after a restart, downloads from
    /// before have had no progress seen yet).
    since: Option<Instant>,
    devices: HashMap<DeviceId, DeviceBackup>,
    activity: HashMap<DeviceId, Activity>,
    /// Last progress seen for each download under way.
    seen: HashMap<String, Instant>,
    /// Finished incoming transfers no one has claimed yet: one of ours may
    /// finish before its ID is known.
    recent: HashMap<String, (Instant, DeviceId, TransferState)>,
    /// Transfers being filed right now.
    filing: HashSet<String>,
    /// Phones a download is being asked for (its transfer ID not known yet).
    asking: HashSet<DeviceId>,
    /// Phones whose backup should run again soon.
    wanted: HashSet<DeviceId>,
    /// Downloads that finished but couldn't be filed, and why.
    failures: HashMap<String, Failure>,
}

/// Why a finished download wasn't filed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Failure {
    /// The transfer itself failed or was cancelled: tried again later.
    Transfer,
    /// No room on the folder's drive.
    Space,
    /// The folder can't be written, or the file came out wrong.
    Write,
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();
static WAKE: OnceLock<Notify> = OnceLock::new();

fn state() -> MutexGuard<'static, State> {
    let mut guard = STATE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if !guard.loaded {
        guard.loaded = true;
        guard.since = Some(Instant::now());
        guard.devices = load_all();
    }
    guard
}

fn wake() -> &'static Notify {
    WAKE.get_or_init(Notify::new)
}

/// Looks at every phone now instead of at the next regular look.
pub fn look_again() {
    wake().notify_one();
}

fn changed() {
    core_host::host().hub.changed(Changes::PHOTO_BACKUP);
}

fn set_activity(device: DeviceId, activity: Activity) {
    let differs = {
        let mut s = state();
        let old = s.activity.insert(device, activity.clone());
        old.as_ref() != Some(&activity)
    };
    if differs {
        changed();
    }
}

// ---- Settings ----

/// One phone's settings and progress, for Settings.
#[derive(Debug, Clone)]
pub struct Summary {
    pub enabled: bool,
    pub folder: PathBuf,
    pub videos: bool,
    pub done: usize,
    pub activity: Activity,
}

pub fn summary(device: DeviceId, name: &str) -> Summary {
    let s = state();
    let backup = s.devices.get(&device).cloned().unwrap_or_default();
    Summary {
        enabled: backup.enabled,
        folder: folder_of(&backup, name),
        videos: backup.videos,
        done: backup.done.len(),
        activity: s.activity.get(&device).cloned().unwrap_or_default(),
    }
}

fn update(device: DeviceId, f: impl FnOnce(&mut DeviceBackup)) {
    let snapshot = {
        let mut s = state();
        let backup = s.devices.entry(device).or_default();
        f(backup);
        backup.clone()
    };
    save(device, &snapshot);
    changed();
}

pub fn set_enabled(device: DeviceId, on: bool) {
    update(device, |b| b.enabled = on);
    if on {
        back_up_now(device);
    } else {
        set_activity(device, Activity::Waiting);
    }
}

pub fn set_videos(device: DeviceId, on: bool) {
    update(device, |b| {
        b.videos = on;
        // Videos the earlier passes skipped are found by looking again.
        b.full_scan_ms = 0;
    });
    back_up_now(device);
}

pub fn set_folder(device: DeviceId, folder: PathBuf) {
    if !folder.is_absolute() {
        return;
    }
    update(device, |b| b.folder = Some(folder));
}

/// Looks for new photos on `device` right away.
pub fn back_up_now(device: DeviceId) {
    state().wanted.insert(device);
    wake().notify_one();
}

/// The folder a phone's backup goes to.
fn folder_of(backup: &DeviceBackup, name: &str) -> PathBuf {
    backup.folder.clone().unwrap_or_else(|| default_folder(name))
}

pub fn default_folder(phone_name: &str) -> PathBuf {
    let pictures = dirs::picture_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Pictures")))
        .unwrap_or_else(|| PathBuf::from("."));
    pictures.join("NectarLink").join(folder_name(phone_name))
}

/// A phone's name as a folder name: nothing Windows refuses in one.
fn folder_name(phone_name: &str) -> String {
    let cleaned: String = phone_name
        .chars()
        .map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches(['.', ' ']).to_owned();
    if trimmed.is_empty() { "Phone".into() } else { trimmed }
}

// ---- Saved state ----

fn dir() -> PathBuf {
    core_host::host().data_dir.join("photo-backup")
}

fn load_all() -> HashMap<DeviceId, DeviceBackup> {
    let Some(data_dir) = core_host::try_data_dir() else { return HashMap::new() };
    load_from(&data_dir.join("photo-backup"))
}

fn load_from(dir: &Path) -> HashMap<DeviceId, DeviceBackup> {
    let Ok(entries) = std::fs::read_dir(dir) else { return HashMap::new() };
    entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let device: DeviceId = path.file_stem()?.to_str()?.parse().ok()?;
            if path.extension()? != "json" {
                return None;
            }
            let backup = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok())?;
            Some((device, backup))
        })
        .collect()
}

fn save(device: DeviceId, backup: &DeviceBackup) {
    if let Err(e) = save_to(&dir(), device, backup) {
        tracing::warn!(error = %e, "can't save the photo backup's state");
    }
}

/// Writes a new file and moves it over the old one, so a crash never leaves half a file.
fn save_to(dir: &Path, device: DeviceId, backup: &DeviceBackup) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{device}.json"));
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(backup).map_err(std::io::Error::other)?)?;
    std::fs::rename(&tmp, &path)
}

// ---- Events ----

pub fn on_event(event: &NodeEvent) {
    match event {
        NodeEvent::LinkChanged { device, link: LinkState::Online { .. } }
        | NodeEvent::PhotoAdded { device, .. }
        | NodeEvent::PhotosChanged { device } => {
            if state().devices.get(device).is_some_and(|b| b.enabled) {
                back_up_now(*device);
            }
        }
        NodeEvent::Transfer(t) => on_transfer(t),
        _ => {}
    }
}

/// Whether this transfer is (or may be) one of the backup's own downloads,
/// which aren't listed with the user's transfers or announced.
pub fn claims(t: &Transfer) -> bool {
    let s = state();
    s.filing.contains(&t.id)
        || s.devices.get(&t.device).is_some_and(|b| b.pending.contains_key(&t.id))
        || (s.asking.contains(&t.device) && t.direction == nectarlink_core::Direction::Incoming)
}

fn on_transfer(t: &Transfer) {
    if t.direction != nectarlink_core::Direction::Incoming {
        return;
    }
    let finished =
        matches!(t.state, TransferState::Done { .. } | TransferState::Failed(_) | TransferState::Cancelled);
    let ours = {
        let mut s = state();
        let ours = s.devices.get(&t.device).is_some_and(|b| b.pending.contains_key(&t.id));
        if ours {
            s.seen.insert(t.id.clone(), Instant::now());
        } else if finished {
            let now = Instant::now();
            s.recent.retain(|_, (at, _, _)| now.duration_since(*at) < KEEP_RECENT);
            s.recent.insert(t.id.clone(), (now, t.device, t.state.clone()));
        }
        ours
    };
    if ours && finished {
        finish(t.device, t.id.clone(), t.state.clone());
    }
}

/// Files a finished download (off the event thread: it may copy a large file).
fn finish(device: DeviceId, transfer: String, outcome: TransferState) {
    {
        let mut s = state();
        if !s.filing.insert(transfer.clone()) {
            return;
        }
    }
    std::thread::Builder::new()
        .name("photo-backup-file".into())
        .spawn(move || {
            let filed = file_download(device, &transfer, &outcome);
            let snapshot = {
                let mut s = state();
                s.filing.remove(&transfer);
                s.seen.remove(&transfer);
                if let Err(failure) = &filed {
                    s.failures.insert(transfer.clone(), *failure);
                }
                let backup = s.devices.entry(device).or_default();
                let pending = backup.pending.remove(&transfer);
                if let (Some(item), Ok(size)) = (pending, &filed) {
                    backup.done.insert(item.id, *size);
                }
                backup.clone()
            };
            save(device, &snapshot);
            changed();
            wake().notify_one();
        })
        .ok();
}

/// Moves a finished download into the folder; its size on success. A
/// download that can't be filed is deleted: the phone still has the item,
/// and the next pass asks for it again.
fn file_download(device: DeviceId, transfer: &str, outcome: &TransferState) -> Result<u64, Failure> {
    let TransferState::Done { saved } = outcome else {
        return Err(Failure::Transfer);
    };
    let staged = saved.first().ok_or(Failure::Transfer)?;
    let found = {
        let s = state();
        s.devices.get(&device).and_then(|backup| {
            let item = backup.pending.get(transfer).cloned()?;
            let name = core_host::host().hub.read(|h| h.name_of(&device)).unwrap_or_default();
            Some((item, folder_of(backup, &name)))
        })
    };
    let Some((item, folder)) = found else {
        return Err(Failure::Transfer);
    };
    place(staged, &folder, &item).map_err(|e| {
        tracing::warn!(error = %e, "photo backup couldn't file a download");
        let _ = std::fs::remove_file(staged);
        if e.kind() == std::io::ErrorKind::StorageFull { Failure::Space } else { Failure::Write }
    })
}

// ---- Filing ----

/// The year and month a photo goes under, in this PC's time zone.
fn year_month(date_ms: i64) -> (i32, u32) {
    let local_ms = date_ms.saturating_add(local_offset_ms(date_ms));
    let days = local_ms.div_euclid(86_400_000);
    let (y, m, _) = civil_from_days(days);
    (y, m)
}

/// Days since 1970-01-01 to a calendar date (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((if m <= 2 { y + 1 } else { y }) as i32, m, d)
}

/// How far this PC's time zone is from UTC at `at_ms` (daylight saving included).
fn local_offset_ms(at_ms: i64) -> i64 {
    crate::win::local_offset_ms(at_ms)
}

/// Moves `staged` into `folder\<year>\<month>`, under the item's name. A file
/// already there with the same bytes is this item (backed up before the
/// backup's list was lost); any other one keeps its name and the new file
/// gets a numbered one. Returns the file's size.
fn place(staged: &Path, folder: &Path, item: &Pending) -> std::io::Result<u64> {
    let size = std::fs::metadata(staged)?.len();
    if item.size > 0 && size != item.size {
        return Err(std::io::Error::other(format!("got {size} bytes, expected {}", item.size)));
    }
    let (year, month) = year_month(item.date);
    let dir = folder.join(format!("{year:04}")).join(format!("{month:02}"));
    std::fs::create_dir_all(&dir)?;
    let name = crate::photos::sanitize_drag_filename(&item.name, "jpg");
    let same = dir.join(&name);
    // Same name and size isn't enough: another photo can share both.
    if std::fs::metadata(&same).is_ok_and(|m| m.is_file() && m.len() == size) && same_bytes(&same, staged) {
        let _ = std::fs::remove_file(staged);
        return Ok(size);
    }
    let dest = crate::photos::free_path(&dir, &name);
    if !has_room(&dir, size) {
        return Err(std::io::Error::from(std::io::ErrorKind::StorageFull));
    }
    crate::photos::move_or_copy(staged, &dest)?;
    if let Some(when) = u64::try_from(item.date).ok().filter(|ms| *ms > 0) {
        let modified = UNIX_EPOCH + Duration::from_millis(when);
        if let Ok(file) = std::fs::OpenOptions::new().write(true).open(&dest) {
            let _ = file.set_modified(modified);
        }
    }
    Ok(size)
}

/// Whether two files of the same length hold the same bytes.
fn same_bytes(a: &Path, b: &Path) -> bool {
    use std::io::Read;
    let (Ok(mut a), Ok(mut b)) = (std::fs::File::open(a), std::fs::File::open(b)) else { return false };
    let (mut x, mut y) = (vec![0u8; 64 * 1024], vec![0u8; 64 * 1024]);
    loop {
        let Ok(n) = a.read(&mut x) else { return false };
        if n == 0 {
            return b.read(&mut y[..1]).is_ok_and(|rest| rest == 0);
        }
        if b.read_exact(&mut y[..n]).is_err() || x[..n] != y[..n] {
            return false;
        }
    }
}

/// Whether the drive holding `dir` has room for `size` more bytes and a margin
/// (yes when it can't tell).
fn has_room(dir: &Path, size: u64) -> bool {
    crate::win::free_space(dir).is_none_or(|free| free >= size.saturating_add(SPACE_MARGIN))
}

// ---- Looking for new items ----

/// What one page of the library adds: the items to back up (newest first)
/// and whether everything on it was backed up already.
fn new_on_page(page: &[PhotoItem], done: &HashMap<String, u64>, videos: bool) -> (Vec<PhotoItem>, bool) {
    let mut all_known = true;
    let mut new = Vec::new();
    for item in page {
        if item.duration.is_some() && !videos {
            continue;
        }
        if done.contains_key(&item.id) {
            continue;
        }
        all_known = false;
        new.push(item.clone());
    }
    (new, all_known)
}

/// Whether this pass may stop at a page with nothing new: only between full scans.
fn may_stop_early(full_scan_ms: i64, now_ms: i64) -> bool {
    full_scan_ms > 0 && now_ms.saturating_sub(full_scan_ms) < FULL_SCAN_EVERY.as_millis() as i64
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

// ---- The backup ----

/// Starts the backup (once the node is up). Transfers left from before a
/// restart stay quiet and are filed when they finish.
pub fn start() {
    core_host::spawn(async {
        let Some(node) = core_host::wait_for_node().await else { return };
        let transfers: Vec<String> = {
            let s = state();
            s.devices.values().flat_map(|b| b.pending.keys().cloned()).collect()
        };
        for transfer in transfers {
            node.keep_transfer_quiet(&transfer);
        }
        loop {
            let enabled: Vec<DeviceId> = {
                let mut s = state();
                let wanted: Vec<DeviceId> = s.wanted.drain().collect();
                let all: Vec<DeviceId> =
                    s.devices.iter().filter(|(_, b)| b.enabled).map(|(d, _)| *d).collect();
                if wanted.is_empty() { all } else { wanted.into_iter().filter(|d| all.contains(d)).collect() }
            };
            for device in enabled {
                run(&node, device).await;
            }
            tokio::select! {
                _ = wake().notified() => {}
                _ = tokio::time::sleep(LOOK_EVERY) => {}
            }
        }
    });
}

/// One pass for one phone: find what's new, then copy it, one item at a time.
async fn run(node: &nectarlink_core::Node, device: DeviceId) {
    let online = core_host::host()
        .hub
        .read(|s| s.devices.iter().any(|d| d.id == device && matches!(d.link, LinkState::Online { .. })));
    if !online {
        set_activity(device, Activity::Waiting);
        return;
    }
    if !crate::folder_sync::sync_allowed(device) {
        set_activity(device, Activity::WaitingForWifi);
        return;
    }
    set_activity(device, Activity::Looking);
    give_up_stalled(device);
    let (done, videos, full_scan_ms) = {
        let s = state();
        let Some(b) = s.devices.get(&device) else { return };
        (b.done.clone(), b.videos, b.full_scan_ms)
    };
    let pending_ids: HashSet<String> = state()
        .devices
        .get(&device)
        .map(|b| b.pending.values().map(|p| p.id.clone()).collect())
        .unwrap_or_default();
    let early = may_stop_early(full_scan_ms, now_ms());
    let mut todo = Vec::new();
    let mut before: Option<(i64, String)> = None;
    let mut complete = false;
    for _ in 0..MAX_PAGES {
        let page = match node.photo_list(device, None, before.clone(), PAGE).await {
            Ok(page) => page,
            Err(e) => return stopped(device, &e),
        };
        let (new, all_known) = new_on_page(&page, &done, videos);
        todo.extend(new.into_iter().filter(|i| !pending_ids.contains(&i.id)));
        let Some(last) = page.last() else {
            complete = true;
            break;
        };
        before = Some((last.date, last.id.clone()));
        if (page.len() as u32) < PAGE {
            complete = true;
            break;
        }
        if early && all_known {
            break;
        }
    }
    if complete {
        update(device, |b| b.full_scan_ms = now_ms());
    }
    let total = todo.len();
    for (copied, item) in todo.iter().enumerate() {
        let still_on = state().devices.get(&device).is_some_and(|b| b.enabled);
        if !still_on {
            set_activity(device, Activity::Waiting);
            return;
        }
        if !crate::folder_sync::sync_allowed(device) {
            set_activity(device, Activity::WaitingForWifi);
            return;
        }
        set_activity(device, Activity::Copying { left: total - copied });
        match copy_one(node, device, item).await {
            Ok(()) => {}
            // Deleted on the phone since the list came.
            Err(Error::NotFound) => continue,
            Err(problem) => return stopped(device, &problem),
        }
        if (copied + 1) % SAVE_EVERY == 0 {
            let snapshot = state().devices.get(&device).cloned();
            if let Some(b) = snapshot {
                save(device, &b);
            }
        }
    }
    set_activity(device, Activity::Idle);
}

/// Asks for one item and waits until it's filed, or stops making progress.
async fn copy_one(node: &nectarlink_core::Node, device: DeviceId, item: &PhotoItem) -> Result<(), Error> {
    let name = core_host::host().hub.read(|s| s.name_of(&device)).unwrap_or_default();
    let folder =
        state().devices.get(&device).map(|b| folder_of(b, &name)).unwrap_or_else(|| default_folder(&name));
    if std::fs::create_dir_all(&folder).is_err() {
        return Err(Error::Io(std::io::Error::other("folder")));
    }
    if !has_room(&folder, item.size) {
        return Err(Error::Io(std::io::Error::from(std::io::ErrorKind::StorageFull)));
    }
    state().asking.insert(device);
    let asked = node.fetch_photos_quietly(device, vec![item.id.clone()]).await;
    let transfer = {
        let mut s = state();
        s.asking.remove(&device);
        let transfer = asked?;
        s.devices.entry(device).or_default().pending.insert(transfer.clone(), Pending::from(item));
        s.seen.insert(transfer.clone(), Instant::now());
        transfer
    };
    if let Some(b) = state().devices.get(&device).cloned() {
        save(device, &b);
    }
    // It may have finished before its ID was known.
    let early = state().recent.remove(&transfer);
    if let Some((_, _, outcome)) = early {
        finish(device, transfer.clone(), outcome);
    }
    loop {
        let (pending, last) = {
            let s = state();
            let pending = s.devices.get(&device).is_some_and(|b| b.pending.contains_key(&transfer))
                || s.filing.contains(&transfer);
            (pending, s.seen.get(&transfer).copied())
        };
        if !pending {
            return match state().failures.remove(&transfer) {
                None => Ok(()),
                Some(Failure::Transfer) => Err(Error::Offline),
                Some(Failure::Space) => Err(Error::Io(std::io::Error::from(std::io::ErrorKind::StorageFull))),
                Some(Failure::Write) => Err(Error::Io(std::io::Error::other("write"))),
            };
        }
        if last.is_some_and(|at| at.elapsed() > STALLED_AFTER) {
            // Left pending: the phone resumes it, and it's filed when it finishes.
            return Err(Error::Timeout);
        }
        let online = core_host::host()
            .hub
            .read(|s| s.devices.iter().any(|d| d.id == device && matches!(d.link, LinkState::Online { .. })));
        if !online {
            return Err(Error::Offline);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// Forgets downloads that stopped without finishing (the phone restarted,
/// say), so their items are asked for again.
fn give_up_stalled(device: DeviceId) {
    let snapshot = {
        let mut s = state();
        let since = s.since.unwrap_or_else(Instant::now);
        let stalled: Vec<String> = s
            .devices
            .get(&device)
            .map(|b| {
                b.pending
                    .keys()
                    .filter(|t| !s.filing.contains(*t))
                    .filter(|t| gave_up(s.seen.get(*t).copied(), since))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        if stalled.is_empty() {
            return;
        }
        for transfer in &stalled {
            s.seen.remove(transfer);
        }
        let backup = s.devices.entry(device).or_default();
        backup.pending.retain(|t, _| !stalled.contains(t));
        backup.clone()
    };
    tracing::info!("photo backup gave up on downloads that stopped");
    save(device, &snapshot);
}

/// Whether a download last seen making progress at `seen` (`None`: not
/// since `since`, this run's start) has stopped for good.
fn gave_up(seen: Option<Instant>, since: Instant) -> bool {
    seen.unwrap_or(since).elapsed() > GIVE_UP_AFTER
}

fn stopped(device: DeviceId, e: &Error) {
    let name = core_host::host().hub.read(|s| s.name_of(&device)).unwrap_or_else(|| "your phone".into());
    let activity = match e {
        Error::Offline | Error::Timeout => Activity::Waiting,
        Error::Denied => {
            Activity::Problem(tr!("PhotoBackup", "Photos are turned off for this PC on %1.", name))
        }
        Error::Unsupported => {
            Activity::Problem(tr!("PhotoBackup", "Update NectarLink on %1 to back up its photos.", name))
        }
        Error::Io(io) if io.kind() == std::io::ErrorKind::StorageFull => {
            Activity::Problem(tr!("PhotoBackup", "The backup folder's drive is full."))
        }
        Error::Io(_) => Activity::Problem(tr!("PhotoBackup", "Can't write to the backup folder.")),
        other => {
            tracing::warn!(error = %other, "photo backup stopped");
            Activity::Problem(tr!("PhotoBackup", "The backup stopped. It tries again in a few minutes."))
        }
    };
    set_activity(device, activity);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, date: i64, video: bool) -> PhotoItem {
        PhotoItem {
            id: id.into(),
            name: format!("{id}.jpg"),
            date,
            size: 10,
            width: 0,
            height: 0,
            duration: video.then_some(1_000),
            album: None,
        }
    }

    #[test]
    fn calendar_dates_come_out_right() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29), "a leap day");
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }

    #[test]
    fn a_page_adds_only_items_not_backed_up_and_videos_when_chosen() {
        let page = [item("a", 3, false), item("b", 2, true), item("c", 1, false)];
        let mut done = HashMap::new();
        done.insert("a".to_owned(), 10);
        let (new, all_known) = new_on_page(&page, &done, false);
        assert_eq!(new.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["c"]);
        assert!(!all_known);
        let (new, _) = new_on_page(&page, &done, true);
        assert_eq!(new.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["b", "c"]);
        done.insert("c".into(), 10);
        let (new, all_known) = new_on_page(&page, &done, false);
        assert!(new.is_empty() && all_known, "a skipped video doesn't count as new");
    }

    #[test]
    fn a_pass_stops_early_only_between_full_scans() {
        let now = 10 * 24 * 3_600_000;
        assert!(!may_stop_early(0, now), "never looked through everything");
        assert!(may_stop_early(now - 3_600_000, now));
        assert!(!may_stop_early(now - 25 * 3_600_000, now), "a day since the last full scan");
    }

    #[test]
    fn a_download_is_given_up_only_after_a_long_time_without_progress() {
        let now = Instant::now();
        let long_ago = now.checked_sub(GIVE_UP_AFTER + Duration::from_secs(1)).unwrap();
        assert!(!gave_up(Some(now), long_ago), "progress just now");
        assert!(gave_up(Some(long_ago), now), "no progress for long");
        assert!(!gave_up(None, now), "just restarted: it may still resume");
        assert!(gave_up(None, long_ago), "nothing since the restart, long ago");
    }

    #[test]
    fn phone_names_become_safe_folder_names() {
        assert_eq!(folder_name("Pixel 9a"), "Pixel 9a");
        assert_eq!(folder_name("Bee's: phone?"), "Bee's_ phone_");
        assert_eq!(folder_name("  ..  "), "Phone");
        assert_eq!(folder_name("Galaxy. "), "Galaxy");
    }

    #[test]
    fn files_are_filed_by_month_and_never_overwrite_another() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("backup");
        let staging = dir.path().join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        // 2024-03-15 12:00 UTC: March in every time zone.
        let date = 1_710_504_000_000;
        let pending = Pending { id: "1".into(), name: "IMG_1.jpg".into(), size: 5, date };

        let staged = staging.join("IMG_1.jpg");
        std::fs::write(&staged, b"first").unwrap();
        assert_eq!(place(&staged, &folder, &pending).unwrap(), 5);
        let filed = folder.join("2024").join("03").join("IMG_1.jpg");
        assert_eq!(std::fs::read(&filed).unwrap(), b"first");
        assert!(!staged.exists());
        let modified = std::fs::metadata(&filed).unwrap().modified().unwrap();
        assert_eq!(modified.duration_since(UNIX_EPOCH).unwrap().as_millis() as i64, date);

        // The same file again (the backup's list was lost): kept once.
        std::fs::write(&staged, b"first").unwrap();
        place(&staged, &folder, &pending).unwrap();
        assert_eq!(std::fs::read_dir(folder.join("2024").join("03")).unwrap().count(), 1);

        // Same name and size, other bytes: another photo, kept too.
        std::fs::write(&staged, b"fiRst").unwrap();
        place(&staged, &folder, &pending).unwrap();
        assert_eq!(std::fs::read(folder.join("2024").join("03").join("IMG_1 (2).jpg")).unwrap(), b"fiRst");
        std::fs::remove_file(folder.join("2024").join("03").join("IMG_1 (2).jpg")).unwrap();

        // A different file with that name: both kept.
        let other = Pending { id: "2".into(), name: "IMG_1.jpg".into(), size: 6, date };
        std::fs::write(&staged, b"second").unwrap();
        place(&staged, &folder, &other).unwrap();
        assert_eq!(std::fs::read(folder.join("2024").join("03").join("IMG_1 (2).jpg")).unwrap(), b"second");

        // A download of the wrong size is refused and left where it was.
        let short = Pending { id: "3".into(), name: "IMG_3.jpg".into(), size: 100, date };
        let partial = staging.join("IMG_3.jpg");
        std::fs::write(&partial, b"cut").unwrap();
        assert!(place(&partial, &folder, &short).is_err());
        assert!(partial.exists());
    }

    #[test]
    fn saved_state_round_trips_and_skips_strange_files() {
        let dir = tempfile::tempdir().unwrap();
        let device = DeviceId([7; 32]);
        let mut backup = DeviceBackup { enabled: true, videos: true, full_scan_ms: 42, ..Default::default() };
        backup.done.insert("media:1".into(), 1_234);
        backup
            .pending
            .insert("t1".into(), Pending { id: "media:2".into(), name: "a.jpg".into(), size: 9, date: 5 });
        save_to(dir.path(), device, &backup).unwrap();
        std::fs::write(dir.path().join("not-a-device.json"), b"{}").unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"x").unwrap();
        let loaded = load_from(dir.path());
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded.get(&device), Some(&backup));
        assert!(!dir.path().join(format!("{device}.json.tmp")).exists());
    }
}
