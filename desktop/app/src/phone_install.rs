// SPDX-License-Identifier: GPL-3.0-or-later
//! Settings › Install on a phone: puts the phone app on a phone over a USB
//! cable. It's for phones where installing from a browser is blocked (Play
//! Protect, some makers' installers) and for anyone who'd rather not hunt for
//! an APK.
//!
//! Three steps, each checked again while the page is open, and nothing at
//! all while it's closed:
//!
//! 1. **USB tools.** Android's `adb`: the PC's own when it has one (Android
//!    Studio, a package manager, `PATH`), else Google's platform-tools,
//!    downloaded on request from Google into NectarLink's data folder and
//!    checked against the SHA-1 Google publishes for them. The core's USB
//!    connections use that copy too from then on.
//! 2. **The phone.** Plugged in, with USB debugging on and this PC allowed.
//!    On Windows, a phone whose USB debugging has no driver is noticed, since
//!    `adb` can't see it at all then.
//! 3. **The app.** The newest phone app on this PC's update channel, from the
//!    same release feed the updater uses and checked against its SHA-256,
//!    sent straight to the phone's package manager, then opened. What the
//!    phone says when it refuses is turned into what to do about it.

use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use sha2::{Digest, Sha256};

use crate::{core_host, state::Changes, version, win::http};

/// The phone app's package.
pub const PACKAGE: &str = "app.nectarlink";
/// The oldest Android the phone app runs on (8.0).
const MIN_SDK: u32 = 26;

/// Google's SDK repository: which platform-tools are current, with checksums.
const REPOSITORY: &str = "https://dl.google.com/android/repository/repository2-3.xml";
const DOWNLOADS: &str = "https://dl.google.com/android/repository/";
/// The repository file is about 450 KB; the tools about 8 to 17 MB.
const MAX_REPOSITORY: usize = 8 * 1024 * 1024;
const MAX_TOOLS: u64 = 64 * 1024 * 1024;
const MAX_APK: u64 = 256 * 1024 * 1024;

/// How often an open page looks for phones.
const POLL: Duration = Duration::from_millis(1500);
/// How often a phone's installed version is asked again (it may change on the phone).
const RECHECK_INSTALLED: Duration = Duration::from_secs(8);

/// What the page shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Status {
    /// USB is turned off for this copy (`NECTARLINK_NO_ADB`, a development copy).
    pub usb_off: bool,
    pub tools: Tools,
    /// An adb server answers.
    pub server: bool,
    pub phones: Vec<Phone>,
    /// Windows: an Android phone is plugged in whose USB debugging has no driver.
    pub driver_missing: bool,
    pub job: Job,
    /// The version an install puts on a phone, once the feed has said.
    pub latest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Tools {
    /// Not looked for yet.
    #[default]
    Unknown,
    /// This PC has adb (`own`: the copy NectarLink downloaded).
    Ready {
        own: bool,
    },
    /// None, and Google has none for this PC either (Linux on ARM).
    Unavailable,
    /// None yet; Google's can be downloaded.
    Missing,
    /// Downloading Google's: bytes so far, and of how many (0 when unknown).
    Getting {
        done: u64,
        total: u64,
    },
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Phone {
    pub serial: String,
    pub state: PhoneState,
    /// "Google Pixel 7", once the phone has said.
    pub name: String,
    pub android: Option<String>,
    pub sdk: Option<u32>,
    /// The phone app's version on it, `None` when it isn't there.
    pub installed: Option<String>,
    /// The phone's details have been read (only once it's allowed this PC).
    pub known: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhoneState {
    Ready,
    /// Waiting for "Allow USB debugging?" on the phone.
    Unauthorized,
    /// Linux without the udev rules that let a user reach the phone.
    NoPermissions,
    /// adb sees it, but it doesn't answer; replugging usually helps.
    Offline,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Job {
    #[default]
    Idle,
    /// Downloading the phone app (bytes so far, of how many).
    Downloading {
        serial: String,
        done: u64,
        total: u64,
    },
    /// Sending it to the phone.
    Sending {
        serial: String,
        done: u64,
        total: u64,
    },
    /// Sent; the phone is checking it, or asking its user.
    Installing {
        serial: String,
    },
    Uninstalling {
        serial: String,
    },
    Done {
        serial: String,
        version: String,
    },
    Failed {
        serial: String,
        problem: Problem,
    },
}

/// Why an install didn't go through, as the page explains it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// The phone app couldn't be downloaded (network, or no feed).
    Download(String),
    /// The download didn't match its checksum.
    Damaged,
    /// The phone went away, or adb stopped answering.
    Lost(String),
    /// Android older than the app needs.
    OldAndroid,
    /// Installing over USB is turned off on the phone (Xiaomi and others).
    Restricted,
    /// Another build of the app, signed differently, is on the phone.
    OtherBuild,
    /// The phone has a newer version already.
    Newer,
    NoSpace,
    /// Stopped on the phone: its user said no, or a scan (Play Protect) did.
    Declined,
    /// Anything else, with the package manager's own reason.
    Other(String),
}

static STATUS: Mutex<Option<Status>> = Mutex::new(None);
/// Whether the page is open, so the phones are watched.
static WATCHING: AtomicBool = AtomicBool::new(false);
/// Bumped each time watching starts, so an old loop knows to stop.
static WATCH_GENERATION: AtomicU64 = AtomicU64::new(0);
/// Something is downloading or installing (one thing at a time).
static BUSY: AtomicBool = AtomicBool::new(false);
/// When each ready phone's installed version was last read.
static CHECKED: Mutex<Vec<(String, Instant)>> = Mutex::new(Vec::new());

fn data_dir() -> Option<PathBuf> {
    core_host::try_data_dir().map(Path::to_path_buf)
}

/// Where the downloaded platform-tools live.
fn tools_dir(data: &Path) -> PathBuf {
    data.join("platform-tools")
}

fn adb_name() -> &'static str {
    if cfg!(windows) { "adb.exe" } else { "adb" }
}

/// At startup: adb downloaded before is used for USB connections too.
pub fn init(data: &Path) {
    let adb = tools_dir(data).join(adb_name());
    if adb.is_file() {
        nectarlink_adb::set_fallback_adb(Some(adb));
    }
}

/// The page's state now.
pub fn status() -> Status {
    STATUS.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default()
}

/// Changes the state, telling the page only if something did change.
fn update(f: impl FnOnce(&mut Status)) {
    let changed = {
        let mut guard = STATUS.lock().unwrap_or_else(|e| e.into_inner());
        let status = guard.get_or_insert_with(Status::default);
        let before = status.clone();
        f(status);
        *status != before
    };
    if changed {
        core_host::host().hub.changed(Changes::PHONE_INSTALL);
    }
}

fn server() -> std::net::SocketAddr {
    nectarlink_adb::configured_server()
}

/// The page opened (`true`) or closed: phones are watched only while it's open.
pub fn watch(on: bool) {
    if !on {
        WATCHING.store(false, Ordering::Release);
        return;
    }
    if WATCHING.swap(true, Ordering::AcqRel) {
        return;
    }
    let generation = WATCH_GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    let _ = std::thread::Builder::new().name("phone-install".into()).spawn(move || {
        let mut started_server_at: Option<Instant> = None;
        while WATCHING.load(Ordering::Acquire) && WATCH_GENERATION.load(Ordering::Acquire) == generation {
            look(&mut started_server_at);
            std::thread::sleep(POLL);
        }
    });
    // The version an install would put on a phone, for the page to show.
    let _ = std::thread::Builder::new().name("phone-install-feed".into()).spawn(|| {
        if let Ok(file) = latest_apk() {
            update(|s| s.latest = Some(file.0));
        }
    });
}

/// One look: the tools, the adb server (started when it isn't running), and the phones.
fn look(started_server_at: &mut Option<Instant>) {
    if !core_host::adb_enabled() {
        update(|s| {
            s.usb_off = true;
            s.phones.clear();
        });
        return;
    }
    let adb = nectarlink_adb::locate_adb();
    let own = adb.as_deref().is_some_and(|a| data_dir().is_some_and(|d| a.starts_with(tools_dir(&d))));
    update(|s| {
        s.usb_off = false;
        s.tools = match (&s.tools, &adb) {
            (Tools::Getting { .. }, _) => s.tools.clone(),
            (_, Some(_)) => Tools::Ready { own },
            (Tools::Failed(e), None) => Tools::Failed(e.clone()),
            (_, None) if tools_archive_host().is_none() => Tools::Unavailable,
            (_, None) => Tools::Missing,
        };
    });
    if adb.is_none() {
        update(|s| s.server = false);
        return;
    }
    let mut answering = nectarlink_adb::server_answers(server());
    // Started at most every 15 s: a server that won't start shouldn't be retried in a loop.
    if !answering && started_server_at.is_none_or(|at| at.elapsed() > Duration::from_secs(15)) {
        *started_server_at = Some(Instant::now());
        nectarlink_adb::start_server(server());
        answering = nectarlink_adb::server_answers(server());
    }
    update(|s| s.server = answering);
    if !answering {
        return;
    }
    let devices = block_on(nectarlink_adb::list_devices(server())).unwrap_or_default();
    let mut phones: Vec<Phone> = Vec::new();
    for device in devices {
        // Phones on a cable, and the emulator; not phones adb reaches over Wi-Fi.
        if nectarlink_adb::is_network_serial(&device.serial) {
            continue;
        }
        let state = match device.state.as_str() {
            "device" => PhoneState::Ready,
            "unauthorized" | "authorizing" => PhoneState::Unauthorized,
            "no permissions" => PhoneState::NoPermissions,
            "offline" | "connecting" => PhoneState::Offline,
            _ => continue,
        };
        let known = status().phones.into_iter().find(|p| p.serial == device.serial && p.known);
        let fallback_name = device.model.as_deref().unwrap_or(&device.serial).replace('_', " ");
        let mut phone = known.unwrap_or(Phone {
            serial: device.serial.clone(),
            state,
            name: fallback_name,
            android: None,
            sdk: None,
            installed: None,
            known: false,
        });
        phone.state = state;
        if state == PhoneState::Ready && !BUSY.load(Ordering::Acquire) {
            if !phone.known {
                describe(&mut phone);
            }
            if phone.known && due(&phone.serial) {
                phone.installed = installed_version(&phone.serial);
            }
        }
        phones.push(phone);
    }
    // Windows can't show adb a phone it has no driver for; it's worth saying why.
    let driver_missing = phones.is_empty() && driver_missing();
    update(|s| {
        s.phones = phones;
        s.driver_missing = driver_missing;
    });
}

/// Whether a ready phone's installed version is due to be read again.
fn due(serial: &str) -> bool {
    let mut checked = CHECKED.lock().unwrap_or_else(|e| e.into_inner());
    match checked.iter_mut().find(|(s, _)| s == serial) {
        Some((_, at)) if at.elapsed() < RECHECK_INSTALLED => false,
        Some((_, at)) => {
            *at = Instant::now();
            true
        }
        None => {
            checked.push((serial.to_owned(), Instant::now()));
            true
        }
    }
}

/// Reads a ready phone's maker, model and Android version.
fn describe(phone: &mut Phone) {
    let said = block_on(nectarlink_adb::shell_output(
        server(),
        &phone.serial,
        "getprop ro.product.manufacturer; getprop ro.product.model; \
         getprop ro.build.version.release; getprop ro.build.version.sdk",
        4096,
    ));
    let Ok(said) = said else { return };
    let lines: Vec<&str> = said.lines().map(str::trim).collect();
    let (maker, model) = (lines.first().copied().unwrap_or(""), lines.get(1).copied().unwrap_or(""));
    phone.name = phone_name(maker, model).unwrap_or_else(|| phone.name.clone());
    phone.android = lines.get(2).filter(|v| !v.is_empty()).map(|v| (*v).to_owned());
    phone.sdk = lines.get(3).and_then(|v| v.parse().ok());
    phone.installed = installed_version(&phone.serial);
    phone.known = true;
    if let Ok(mut checked) = CHECKED.lock() {
        checked.retain(|(s, _)| s != &phone.serial);
        checked.push((phone.serial.clone(), Instant::now()));
    }
}

/// "Google Pixel 7" from a maker and a model, without saying the maker twice
/// ("samsung" + "SM-S911B" gives "Samsung SM-S911B"; "Xiaomi" + "Xiaomi 14" gives "Xiaomi 14").
fn phone_name(maker: &str, model: &str) -> Option<String> {
    let model = model.trim();
    if model.is_empty() {
        return None;
    }
    let maker = maker.trim();
    if maker.is_empty() || model.to_lowercase().starts_with(&maker.to_lowercase()) {
        return Some(model.to_owned());
    }
    let mut chars = maker.chars();
    let maker: String = chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default();
    Some(format!("{maker} {model}"))
}

/// The phone app's version on `serial`, `None` when it isn't installed.
fn installed_version(serial: &str) -> Option<String> {
    let said = block_on(nectarlink_adb::shell_output(
        server(),
        serial,
        &format!("dumpsys package {PACKAGE} | grep -m1 versionName"),
        4096,
    ))
    .ok()?;
    version_name(&said)
}

/// `0.1.5` from dumpsys's `versionName=0.1.5` line.
fn version_name(dumpsys: &str) -> Option<String> {
    dumpsys
        .lines()
        .find_map(|l| l.trim().strip_prefix("versionName="))
        .map(str::trim)
        .filter(|v| !v.is_empty() && v.len() <= 64)
        .map(str::to_owned)
}

/// Runs an adb request from a plain thread.
fn block_on<T>(fut: impl std::future::Future<Output = T>) -> T {
    // A small runtime of its own: the core's is busy with connections and
    // this mustn't hold one of its two threads.
    thread_local! {
        static RUNTIME: tokio::runtime::Runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime for adb requests");
    }
    RUNTIME.with(|rt| rt.block_on(fut))
}

// ---- USB tools ----

/// Google's platform-tools archive for this PC: (`host-os` in the repository).
fn tools_archive_host() -> Option<&'static str> {
    if cfg!(windows) {
        Some("windows")
    } else if cfg!(target_os = "macos") {
        Some("macosx")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("linux")
    } else {
        None
    }
}

/// One platform-tools download, as the repository lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Archive {
    file: String,
    size: u64,
    sha1: String,
}

/// The stable channel's (`channel-0`) platform-tools for `host` in Google's
/// repository XML.
fn tools_archive(repository: &str, host: &str) -> Option<Archive> {
    let mut rest = repository;
    while let Some(start) = rest.find("<remotePackage path=\"platform-tools\">") {
        let block = &rest[start..];
        let end = block.find("</remotePackage>")?;
        let package = &block[..end];
        rest = &block[end..];
        if !package.contains("<channelRef ref=\"channel-0\"/>") {
            continue;
        }
        let mut archives = package;
        while let Some(a) = archives.find("<archive>") {
            let archive = &archives[a..];
            let close = archive.find("</archive>")?;
            let body = &archive[..close];
            archives = &archive[close..];
            if element(body, "host-os") != Some(host) {
                continue;
            }
            let file = element(body, "url")?;
            let size = element(body, "size")?.parse().ok()?;
            let sha1 = element(body, "checksum")?.to_ascii_lowercase();
            if !crate::feed::is_plain_name(file)
                || sha1.len() != 40
                || !sha1.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return None;
            }
            return Some(Archive { file: file.to_owned(), size, sha1 });
        }
    }
    None
}

/// The text of the first `<name ...>text</name>` in `xml`.
fn element<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let open = xml.find(&format!("<{name}"))?;
    let after = &xml[open..];
    let start = after.find('>')? + 1;
    let end = after.find(&format!("</{name}>"))?;
    (start <= end).then(|| after[start..end].trim())
}

/// The files of platform-tools that adb needs (and its notice); the rest
/// (fastboot, sqlite3 and so on) are left in the archive.
fn wanted_entry(name: &str) -> bool {
    matches!(
        name,
        "platform-tools/adb"
            | "platform-tools/adb.exe"
            | "platform-tools/AdbWinApi.dll"
            | "platform-tools/AdbWinUsbApi.dll"
            | "platform-tools/libwinpthread-1.dll"
            | "platform-tools/NOTICE.txt"
            | "platform-tools/source.properties"
    )
}

/// Downloads Google's platform-tools and keeps adb from them.
pub fn get_tools() {
    if BUSY.swap(true, Ordering::AcqRel) {
        return;
    }
    let _ = std::thread::Builder::new().name("platform-tools".into()).spawn(|| {
        update(|s| s.tools = Tools::Getting { done: 0, total: 0 });
        let result = fetch_tools();
        BUSY.store(false, Ordering::Release);
        match result {
            Ok(adb) => {
                tracing::info!(adb = %adb.display(), "downloaded Android's USB tools");
                nectarlink_adb::set_fallback_adb(Some(adb));
                nectarlink_adb::start_server(server());
                update(|s| s.tools = Tools::Ready { own: true });
            }
            Err(e) => {
                tracing::warn!(error = %e, "can't get Android's USB tools");
                update(|s| s.tools = Tools::Failed(e));
            }
        }
    });
}

fn fetch_tools() -> Result<PathBuf, String> {
    let host = tools_archive_host().ok_or("Google has no USB tools for this PC")?;
    let data = data_dir().ok_or("no data folder")?;
    let repository = http::get_retrying(REPOSITORY, MAX_REPOSITORY)?;
    let repository = String::from_utf8_lossy(&repository);
    let archive = tools_archive(&repository, host).ok_or("Google's list has no USB tools for this PC")?;
    if archive.size > MAX_TOOLS {
        return Err("the USB tools are larger than expected".into());
    }
    let cache = data.join("cache");
    std::fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
    let zip = cache.join(&archive.file);
    let total = archive.size;
    let mut last = Instant::now();
    http::download(&format!("{DOWNLOADS}{}", archive.file), &zip, MAX_TOOLS, |done, _| {
        if last.elapsed() > Duration::from_millis(120) || done == total {
            last = Instant::now();
            update(|s| s.tools = Tools::Getting { done, total });
        }
    })?;
    let bytes = std::fs::read(&zip).map_err(|e| e.to_string())?;
    let sha1 = sha1_smol::Sha1::from(&bytes).digest().to_string();
    if bytes.len() as u64 != archive.size || sha1 != archive.sha1 {
        let _ = std::fs::remove_file(&zip);
        return Err("the download didn't match Google's checksum".into());
    }
    let files = unzip(&bytes, wanted_entry)?;
    if !files.iter().any(|(name, _)| name.ends_with(&format!("/{}", adb_name()))) {
        return Err("the archive has no adb".into());
    }
    // Written beside the old copy, then swapped in, so a half-written one is never used.
    let dir = tools_dir(&data);
    let fresh = data.join("platform-tools.new");
    let _ = std::fs::remove_dir_all(&fresh);
    std::fs::create_dir_all(&fresh).map_err(|e| e.to_string())?;
    for (name, content) in &files {
        let file = fresh.join(name.trim_start_matches("platform-tools/"));
        std::fs::write(&file, content).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if !name.ends_with(".txt") && !name.ends_with(".properties") {
                std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755))
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("the old USB tools are in use: {e}"))?;
    }
    std::fs::rename(&fresh, &dir).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&zip);
    Ok(dir.join(adb_name()))
}

/// The entries of a zip archive `want` picks, by name, inflated and checked
/// against their CRC-32. Only what platform-tools uses is read: stored or
/// deflated entries, no ZIP64, no encryption.
fn unzip(zip: &[u8], want: impl Fn(&str) -> bool) -> Result<Vec<(String, Vec<u8>)>, String> {
    let u16_at = |at: usize| zip.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |at: usize| zip.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let damaged = || "the archive is damaged".to_owned();
    // The end of central directory record: within the last 64 KB plus its own 22 bytes.
    let floor = zip.len().saturating_sub(22 + 65_535);
    let eocd = (floor..=zip.len().saturating_sub(22))
        .rev()
        .find(|&at| u32_at(at) == Some(0x0605_4b50))
        .ok_or_else(damaged)?;
    let entries = u16_at(eocd + 10).ok_or_else(damaged)? as usize;
    let mut at = u32_at(eocd + 16).ok_or_else(damaged)? as usize;
    let mut out = Vec::new();
    for _ in 0..entries {
        if u32_at(at) != Some(0x0201_4b50) {
            return Err(damaged());
        }
        let flags = u16_at(at + 8).ok_or_else(damaged)?;
        let method = u16_at(at + 10).ok_or_else(damaged)?;
        let crc = u32_at(at + 16).ok_or_else(damaged)?;
        let packed = u32_at(at + 20).ok_or_else(damaged)? as usize;
        let size = u32_at(at + 24).ok_or_else(damaged)? as usize;
        let name_len = u16_at(at + 28).ok_or_else(damaged)? as usize;
        let extra_len = u16_at(at + 30).ok_or_else(damaged)? as usize;
        let comment_len = u16_at(at + 32).ok_or_else(damaged)? as usize;
        let local = u32_at(at + 42).ok_or_else(damaged)? as usize;
        let name = zip.get(at + 46..at + 46 + name_len).ok_or_else(damaged)?;
        let name = String::from_utf8_lossy(name).into_owned();
        at += 46 + name_len + extra_len + comment_len;
        if !want(&name) {
            continue;
        }
        if flags & 1 != 0 {
            return Err(format!("{name} is encrypted"));
        }
        if u32_at(local) != Some(0x0403_4b50) {
            return Err(damaged());
        }
        let data_at = local
            + 30
            + u16_at(local + 26).ok_or_else(damaged)? as usize
            + u16_at(local + 28).ok_or_else(damaged)? as usize;
        let data = zip.get(data_at..data_at + packed).ok_or_else(damaged)?;
        let content = match method {
            0 => data.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(data, size).map_err(|_| damaged())?,
            other => return Err(format!("{name} uses compression {other}")),
        };
        if content.len() != size || crc32(&content) != crc {
            return Err(damaged());
        }
        out.push((name, content));
    }
    Ok(out)
}

/// CRC-32 (IEEE), as zip archives use.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

// ---- The phone app ----

/// The newest phone app on this PC's update channel: (version, file).
fn latest_apk() -> Result<(String, crate::feed::FeedFile), String> {
    let feed = crate::feed::fetch(crate::updater::channel())?.ok_or("no release feed")?;
    let file = feed
        .files
        .into_iter()
        .find(|f| f.name.ends_with("-android.apk"))
        .ok_or("no phone app in the release")?;
    Ok((feed.version, file))
}

/// The phone app, downloaded (or already here) and checked. Older ones go.
fn phone_app(serial: &str) -> Result<(String, PathBuf), Problem> {
    let (version, file) = latest_apk().map_err(Problem::Download)?;
    let data = data_dir().ok_or_else(|| Problem::Download("no data folder".into()))?;
    let dir = data.join("cache").join("phone-app");
    std::fs::create_dir_all(&dir).map_err(|e| Problem::Download(e.to_string()))?;
    for old in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        if old.file_name().to_string_lossy() != file.name {
            let _ = std::fs::remove_file(old.path());
        }
    }
    let path = dir.join(&file.name);
    if sha256_file(&path).is_ok_and(|h| h == file.sha256) {
        return Ok((version, path));
    }
    let total = file.size;
    let mut last = Instant::now();
    http::download(&file.url, &path, MAX_APK, |done, of| {
        if last.elapsed() > Duration::from_millis(120) {
            last = Instant::now();
            let total = if total > 0 { total } else { of };
            update(|s| s.job = Job::Downloading { serial: serial.to_owned(), done, total });
        }
    })
    .map_err(Problem::Download)?;
    if sha256_file(&path).ok().as_deref() != Some(file.sha256.as_str()) {
        let _ = std::fs::remove_file(&path);
        return Err(Problem::Damaged);
    }
    Ok((version, path))
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
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

/// What the package manager's reason means for the person installing.
fn problem_for(reason: &str) -> Problem {
    match reason {
        "INSTALL_FAILED_USER_RESTRICTED" => Problem::Restricted,
        "INSTALL_FAILED_UPDATE_INCOMPATIBLE" | "INSTALL_FAILED_SHARED_USER_INCOMPATIBLE" => {
            Problem::OtherBuild
        }
        "INSTALL_FAILED_VERSION_DOWNGRADE" => Problem::Newer,
        "INSTALL_FAILED_INSUFFICIENT_STORAGE" | "INSTALL_FAILED_MEDIA_UNAVAILABLE" => Problem::NoSpace,
        "INSTALL_FAILED_OLDER_SDK" => Problem::OldAndroid,
        "INSTALL_FAILED_ABORTED"
        | "INSTALL_FAILED_VERIFICATION_FAILURE"
        | "INSTALL_FAILED_VERIFICATION_TIMEOUT"
        | "INSTALL_FAILED_REJECTED_BY_BUILDTYPE" => Problem::Declined,
        other => Problem::Other(other.to_owned()),
    }
}

/// Installs the newest phone app on `serial` and opens it.
pub fn install(serial: String) {
    if !nectarlink_adb::is_serial_allowed(&serial) || BUSY.swap(true, Ordering::AcqRel) {
        return;
    }
    let _ = std::thread::Builder::new().name("phone-install".into()).spawn(move || {
        let result = install_now(&serial);
        BUSY.store(false, Ordering::Release);
        let installed = installed_version(&serial);
        update(|s| {
            if let Some(phone) = s.phones.iter_mut().find(|p| p.serial == serial) {
                phone.installed = installed;
            }
            s.job = match result {
                Ok(version) => Job::Done { serial: serial.clone(), version },
                Err(problem) => Job::Failed { serial: serial.clone(), problem },
            };
        });
    });
}

fn install_now(serial: &str) -> Result<String, Problem> {
    if status()
        .phones
        .iter()
        .find(|p| p.serial == serial)
        .and_then(|p| p.sdk)
        .is_some_and(|sdk| sdk < MIN_SDK)
    {
        return Err(Problem::OldAndroid);
    }
    update(|s| s.job = Job::Downloading { serial: serial.to_owned(), done: 0, total: 0 });
    let (version, apk) = phone_app(serial)?;
    let mut last = Instant::now();
    let answer = block_on(nectarlink_adb::install_apk(server(), serial, &apk, |done, total| {
        if last.elapsed() > Duration::from_millis(100) || done == total {
            last = Instant::now();
            update(|s| {
                s.job = if done < total {
                    Job::Sending { serial: serial.to_owned(), done, total }
                } else {
                    Job::Installing { serial: serial.to_owned() }
                };
            });
        }
    }))
    .map_err(|e| Problem::Lost(e.to_string()))?;
    if let Some(reason) = nectarlink_adb::install_failure(&answer) {
        tracing::info!(reason, "the phone didn't take the app");
        return Err(problem_for(&reason));
    }
    tracing::info!(version, "installed the phone app over USB");
    open_on_phone(serial);
    Ok(version)
}

/// Opens the phone app on `serial`.
pub fn open_on_phone(serial: &str) {
    if !nectarlink_adb::is_serial_allowed(serial) {
        return;
    }
    // Woken first: started on a sleeping phone, it opens out of sight.
    let command =
        format!("input keyevent KEYCODE_WAKEUP; monkey -p {PACKAGE} -c android.intent.category.LAUNCHER 1");
    let _ = block_on(nectarlink_adb::shell_output(server(), serial, &command, 4096));
}

/// Removes the phone app (another build of it, signed differently) from
/// `serial`, then installs this one.
pub fn replace(serial: String) {
    if !nectarlink_adb::is_serial_allowed(&serial) || BUSY.swap(true, Ordering::AcqRel) {
        return;
    }
    let _ = std::thread::Builder::new().name("phone-install".into()).spawn(move || {
        update(|s| s.job = Job::Uninstalling { serial: serial.clone() });
        let answer = block_on(nectarlink_adb::uninstall(server(), &serial, PACKAGE));
        BUSY.store(false, Ordering::Release);
        match answer {
            Ok(answer) if nectarlink_adb::install_failure(&answer).is_none() => install(serial),
            Ok(answer) => update(|s| {
                s.job = Job::Failed {
                    serial: serial.clone(),
                    problem: Problem::Other(nectarlink_adb::install_failure(&answer).unwrap_or_default()),
                }
            }),
            Err(e) => update(|s| {
                s.job = Job::Failed { serial: serial.clone(), problem: Problem::Lost(e.to_string()) }
            }),
        }
    });
}

/// Back to the start (after a result was read).
pub fn dismiss() {
    if !BUSY.load(Ordering::Acquire) {
        update(|s| s.job = Job::Idle);
    }
}

/// Whether the phone app on a phone is older than the one an install puts there.
pub fn is_outdated(installed: &str, latest: &str) -> bool {
    version::is_newer(latest, installed)
}

#[cfg(windows)]
fn driver_missing() -> bool {
    crate::win::usb_driver::android_without_driver()
}

#[cfg(not(windows))]
fn driver_missing() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPOSITORY_SAMPLE: &str = r#"<sdk:sdk-repository>
<remotePackage path="platform-tools">
  <revision><major>37</major><minor>0</minor><micro>3</micro></revision>
  <channelRef ref="channel-2"/>
  <archives><archive><complete><size>1</size><checksum type="sha1">aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa</checksum><url>platform-tools_r37.0.3-win.zip</url></complete><host-os>windows</host-os></archive></archives>
</remotePackage>
<remotePackage path="platform-tools">
  <revision><major>37</major><minor>0</minor><micro>1</micro></revision>
  <channelRef ref="channel-0"/>
  <archives>
    <archive><complete><size>9054187</size><checksum type="sha1">477254AA5F903C15CF51001717BDF347FB6B53E0</checksum><url>platform-tools_r37.0.1-linux.zip</url></complete><host-os>linux</host-os></archive>
    <archive><complete><size>8044989</size><checksum type="sha1">e03e78b1d80b396f1c3358e31251cb31740e1110</checksum><url>platform-tools_r37.0.1-win.zip</url></complete><host-os>windows</host-os></archive>
  </archives>
</remotePackage>
<remotePackage path="build-tools;37.0.0"><channelRef ref="channel-0"/></remotePackage>
</sdk:sdk-repository>"#;

    #[test]
    fn the_stable_platform_tools_for_this_pc_are_picked_from_googles_list() {
        assert_eq!(
            tools_archive(REPOSITORY_SAMPLE, "windows"),
            Some(Archive {
                file: "platform-tools_r37.0.1-win.zip".into(),
                size: 8_044_989,
                sha1: "e03e78b1d80b396f1c3358e31251cb31740e1110".into(),
            })
        );
        assert_eq!(tools_archive(REPOSITORY_SAMPLE, "linux").map(|a| a.sha1.len()), Some(40));
        assert_eq!(tools_archive(REPOSITORY_SAMPLE, "macosx"), None);
        // A file name that's a path is never followed.
        let sneaky = REPOSITORY_SAMPLE.replace("platform-tools_r37.0.1-win.zip", "../evil.zip");
        assert_eq!(tools_archive(&sneaky, "windows"), None);
    }

    /// A zip with one stored and one deflated entry, as platform-tools has.
    fn sample_zip(entries: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut zip = Vec::new();
        let mut central = Vec::new();
        for (name, content, deflate) in entries {
            let packed =
                if *deflate { miniz_oxide::deflate::compress_to_vec(content, 6) } else { content.to_vec() };
            let offset = zip.len() as u32;
            let method: u16 = if *deflate { 8 } else { 0 };
            let crc = crc32(content);
            let mut header = Vec::new();
            header.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            header.extend_from_slice(&[20, 0, 0, 0]);
            header.extend_from_slice(&method.to_le_bytes());
            header.extend_from_slice(&[0; 4]);
            header.extend_from_slice(&crc.to_le_bytes());
            header.extend_from_slice(&(packed.len() as u32).to_le_bytes());
            header.extend_from_slice(&(content.len() as u32).to_le_bytes());
            header.extend_from_slice(&(name.len() as u16).to_le_bytes());
            header.extend_from_slice(&0u16.to_le_bytes());
            zip.extend_from_slice(&header);
            zip.extend_from_slice(name.as_bytes());
            zip.extend_from_slice(&packed);
            central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            central.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
            central.extend_from_slice(&method.to_le_bytes());
            central.extend_from_slice(&[0; 4]);
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&(packed.len() as u32).to_le_bytes());
            central.extend_from_slice(&(content.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&[0; 12]);
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let central_at = zip.len() as u32;
        zip.extend_from_slice(&central);
        zip.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        zip.extend_from_slice(&[0; 4]);
        zip.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        zip.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        zip.extend_from_slice(&(central.len() as u32).to_le_bytes());
        zip.extend_from_slice(&central_at.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip
    }

    #[test]
    fn only_what_adb_needs_comes_out_of_the_archive() {
        let adb = b"MZ pretend adb ".repeat(500);
        let zip = sample_zip(&[
            ("platform-tools/adb.exe", &adb, true),
            ("platform-tools/fastboot.exe", b"not wanted", true),
            ("platform-tools/source.properties", b"Pkg.Revision=37.0.1\n", false),
        ]);
        let files = unzip(&zip, wanted_entry).unwrap();
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["platform-tools/adb.exe", "platform-tools/source.properties"]);
        assert_eq!(files[0].1, adb);
        // A damaged byte in adb's data fails its CRC instead of being written.
        let mut bad = zip.clone();
        let at = 30 + "platform-tools/adb.exe".len() + 4;
        bad[at] ^= 0xff;
        assert!(unzip(&bad, wanted_entry).is_err());
        assert!(unzip(b"not a zip at all", wanted_entry).is_err());
    }

    #[test]
    fn crc32_is_the_zip_one() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn phones_are_named_like_people_say_them() {
        assert_eq!(phone_name("Google", "Pixel 7").as_deref(), Some("Google Pixel 7"));
        assert_eq!(phone_name("samsung", "SM-S911B").as_deref(), Some("Samsung SM-S911B"));
        assert_eq!(phone_name("Xiaomi", "Xiaomi 14").as_deref(), Some("Xiaomi 14"));
        assert_eq!(phone_name("", "sdk_gphone64_x86_64").as_deref(), Some("sdk_gphone64_x86_64"));
        assert_eq!(phone_name("Google", " "), None);
    }

    #[test]
    fn the_installed_version_comes_from_dumpsys() {
        assert_eq!(version_name("    versionName=0.1.6-nightly.10\n").as_deref(), Some("0.1.6-nightly.10"));
        assert_eq!(version_name("Unable to find package: app.nectarlink\n"), None);
        assert_eq!(version_name(""), None);
    }

    #[test]
    fn the_phones_refusals_say_what_to_do() {
        assert_eq!(problem_for("INSTALL_FAILED_USER_RESTRICTED"), Problem::Restricted);
        assert_eq!(problem_for("INSTALL_FAILED_UPDATE_INCOMPATIBLE"), Problem::OtherBuild);
        assert_eq!(problem_for("INSTALL_FAILED_VERSION_DOWNGRADE"), Problem::Newer);
        assert_eq!(problem_for("INSTALL_FAILED_INSUFFICIENT_STORAGE"), Problem::NoSpace);
        assert_eq!(problem_for("INSTALL_FAILED_VERIFICATION_FAILURE"), Problem::Declined);
        assert_eq!(problem_for("INSTALL_FAILED_OLDER_SDK"), Problem::OldAndroid);
        assert_eq!(problem_for("-99"), Problem::Other("-99".into()));
    }

    #[test]
    fn an_older_app_on_the_phone_is_outdated() {
        assert!(is_outdated("0.1.5", "0.1.6"));
        assert!(is_outdated("0.1.6-nightly.9", "0.1.6-nightly.10"));
        assert!(!is_outdated("0.1.6", "0.1.6"));
        assert!(!is_outdated("0.1.7", "0.1.6"));
    }

    /// Downloads the real platform-tools from Google (network; run by hand).
    #[test]
    #[ignore = "downloads from Google"]
    fn googles_platform_tools_download_and_unpack() {
        let host = tools_archive_host().expect("a host Google builds for");
        let repository = http::get_retrying(REPOSITORY, MAX_REPOSITORY).unwrap();
        let archive = tools_archive(&String::from_utf8_lossy(&repository), host).unwrap();
        let dir = std::env::temp_dir().join(format!("nectarlink-pt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let zip = dir.join(&archive.file);
        http::download(&format!("{DOWNLOADS}{}", archive.file), &zip, MAX_TOOLS, |_, _| {}).unwrap();
        let bytes = std::fs::read(&zip).unwrap();
        assert_eq!(sha1_smol::Sha1::from(&bytes).digest().to_string(), archive.sha1);
        let files = unzip(&bytes, wanted_entry).unwrap();
        assert!(
            files.iter().any(|(n, c)| n.ends_with(adb_name()) && c.len() > 1_000_000),
            "{:?}",
            files.iter().map(|f| &f.0).collect::<Vec<_>>()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
