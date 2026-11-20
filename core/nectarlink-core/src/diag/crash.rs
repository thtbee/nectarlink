// SPDX-License-Identifier: MPL-2.0
//! Crashes, kept on this device: what went wrong and where, never what the
//! app was working on.
//!
//! - [`begin_run`] marks a run as started and says how the previous one
//!   ended. A run that never reached [`end_run`] ended badly: a crash, but
//!   also a power cut, a forced quit or a frozen app ended from Task Manager,
//!   so a run that ended badly only counts as a crash with a record from it.
//! - [`install_panic_hook`] keeps a record of every Rust panic.
//! - [`record`] keeps one the app found another way (Windows' crash events,
//!   Android's exit reasons).
//!
//! Records live in `<dir>/crashes/`, the newest [`KEEP`], each a few hundred
//! bytes. A record's [`CrashRecord::sig`] stays the same for the same bug
//! across versions and line changes, so reports of it can be grouped.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// How many crash records are kept.
pub const KEEP: usize = 5;
const RUN_FILE: &str = "run.json";
const CRASH_DIR: &str = "crashes";
/// The longest message or location kept.
const MAX_TEXT: usize = 200;

/// One crash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrashRecord {
    /// The same for the same bug: 12 hex digits.
    pub sig: String,
    /// `panic`, `native` (Windows' record of a crash), `exception`
    /// (Android's Java/Kotlin) or `anr`.
    pub kind: String,
    /// Where: `crate/src/file.rs:12:5`, a module name, a class.
    pub location: String,
    /// What, as far as it's safe to keep: fixed text, never values.
    pub message: String,
    /// The thread's name, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    pub version: String,
    /// Seconds since 1970 (UTC).
    pub at: u64,
}

/// How the previous run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviousRun {
    pub version: String,
    /// When it started, in seconds since 1970.
    pub started: u64,
    /// Whether it quit normally.
    pub clean: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct RunFile {
    version: String,
    started: u64,
    clean: bool,
}

struct Config {
    dir: PathBuf,
    version: String,
}

static CONFIG: OnceLock<Config> = OnceLock::new();

/// Marks this run as started (in `dir`, by this `version`) and returns how
/// the one before ended, if there was one. Also where [`record`] and the
/// panic hook write; only the first call counts.
pub fn begin_run(dir: &Path, version: &str) -> Option<PreviousRun> {
    let _ = CONFIG.set(Config { dir: dir.to_owned(), version: version.to_owned() });
    let path = dir.join(RUN_FILE);
    let previous = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<RunFile>(&bytes).ok())
        .map(|run| PreviousRun { version: run.version, started: run.started, clean: run.clean });
    write_run(dir, RunFile { version: version.to_owned(), started: now(), clean: false });
    previous
}

/// Marks this run as ended normally.
pub fn end_run() {
    let Some(config) = CONFIG.get() else { return };
    let path = config.dir.join(RUN_FILE);
    let started = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<RunFile>(&bytes).ok())
        .map_or_else(now, |run| run.started);
    write_run(&config.dir, RunFile { version: config.version.clone(), started, clean: true });
}

fn write_run(dir: &Path, run: RunFile) {
    let _ = fs::create_dir_all(dir);
    if let Ok(json) = serde_json::to_vec(&run) {
        let tmp = dir.join("run.json.tmp");
        if fs::write(&tmp, json).is_ok() {
            let _ = fs::rename(tmp, dir.join(RUN_FILE));
        }
    }
}

/// Keeps a record of every Rust panic from now on, then lets the panic go
/// on as before (the previous hook still runs).
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()));
        let fixed = info.payload().downcast_ref::<&'static str>().copied();
        let formatted = info.payload().downcast_ref::<String>().map(String::as_str);
        let thread = std::thread::current().name().map(str::to_owned);
        let version = CONFIG.get().map_or("", |c| c.version.as_str());
        let record = panic_record(location.as_deref(), fixed, formatted, thread, version, now());
        tracing::error!(sig = %record.sig, location = %record.location, "panic: {}", record.message);
        if let Some(config) = CONFIG.get() {
            save(&config.dir, &record);
        }
        previous(info);
    }));
}

/// The record of a panic at `location` whose message was `fixed` text or
/// `formatted` from values. Formatted text keeps only what comes before its
/// first `: `, the fixed part of messages like "called `Result::unwrap()`
/// on an `Err` value: …", since the rest may be anything.
fn panic_record(
    location: Option<&str>,
    fixed: Option<&str>,
    formatted: Option<&str>,
    thread: Option<String>,
    version: &str,
    at: u64,
) -> CrashRecord {
    let message = match (fixed, formatted) {
        (Some(text), _) => text.to_owned(),
        (None, Some(text)) => match text.split_once(": ") {
            Some((head, _)) => format!("{head}: <details hidden>"),
            None => "<details hidden>".to_owned(),
        },
        (None, None) => "<no message>".to_owned(),
    };
    let message = cut(&super::redact::text(&message));
    let location = location.map(normalize_location).unwrap_or_else(|| "<unknown>".to_owned());
    // Without the line, so a moved line is still the same bug.
    let place = location.split(':').next().unwrap_or("");
    CrashRecord {
        sig: signature(&["panic", place, &message]),
        kind: "panic".to_owned(),
        location,
        message,
        thread: thread.map(|t| cut(&super::redact::text(&t))),
        version: version.to_owned(),
        at,
    }
}

/// A source location as `crate/src/file.rs:line:col`: a library's from its
/// registry folder without the version, the app's own as it is.
fn normalize_location(location: &str) -> String {
    let slashed = location.replace('\\', "/");
    let trimmed = if let Some(at) = slashed.find("/registry/src/") {
        // …/registry/src/<index>/<crate>-<version>/src/lib.rs
        let rest = &slashed[at + "/registry/src/".len()..];
        let mut parts = rest.splitn(3, '/');
        let _index = parts.next();
        let krate = parts.next().unwrap_or("");
        let file = parts.next().unwrap_or("");
        let name = match krate.rfind('-') {
            Some(dash) if krate[dash + 1..].starts_with(|c: char| c.is_ascii_digit()) => &krate[..dash],
            _ => krate,
        };
        format!("{name}/{file}")
    } else if let Some(at) = slashed.find("/rustc/") {
        // /rustc/<hash>/library/core/src/…
        let rest = &slashed[at + "/rustc/".len()..];
        format!("rust/{}", rest.split_once('/').map_or(rest, |(_, path)| path))
    } else {
        slashed
    };
    cut(&super::redact::text(&trimmed))
}

/// Keeps `record` for this device, newest [`KEEP`] only. Never fails: a
/// record that can't be written is lost.
pub fn record(record: &CrashRecord) {
    if let Some(config) = CONFIG.get() {
        save(&config.dir, record);
    }
}

fn save(dir: &Path, record: &CrashRecord) {
    let crashes = dir.join(CRASH_DIR);
    if fs::create_dir_all(&crashes).is_err() {
        return;
    }
    let Ok(json) = serde_json::to_vec_pretty(record) else { return };
    let _ = fs::write(crashes.join(format!("{:011}-{}.json", record.at, record.sig)), json);
    let mut names: Vec<_> = fs::read_dir(&crashes)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| name.ends_with(".json"))
        .collect();
    names.sort();
    for old in names.iter().rev().skip(KEEP) {
        let _ = fs::remove_file(crashes.join(old));
    }
}

/// The crash records kept here, newest first.
pub fn records(dir: &Path) -> Vec<CrashRecord> {
    let mut records: Vec<CrashRecord> = fs::read_dir(dir.join(CRASH_DIR))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| fs::read(e.path()).ok())
        .filter_map(|bytes| serde_json::from_slice(&bytes).ok())
        .collect();
    records.sort_by_key(|r| std::cmp::Reverse(r.at));
    records
}

/// Whether the run before this one crashed: it ended badly and left a
/// record from while it ran.
pub fn crashed(previous: &PreviousRun, records: &[CrashRecord]) -> Option<CrashRecord> {
    if previous.clean {
        return None;
    }
    records.iter().find(|r| r.at >= previous.started).cloned()
}

/// 12 hex digits of SHA-256 over `parts`.
pub fn signature(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update(part.as_bytes());
        hash.update([0]);
    }
    hash.finalize().iter().take(6).map(|b| format!("{b:02x}")).collect()
}

fn cut(text: &str) -> String {
    match text.char_indices().nth(MAX_TEXT) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_owned(),
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_runs_end_is_remembered() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        write_run(path, RunFile { version: "0.1.5".into(), started: 100, clean: false });
        let previous: RunFile = serde_json::from_slice(&fs::read(path.join(RUN_FILE)).unwrap()).unwrap();
        assert!(!previous.clean);
        write_run(path, RunFile { version: "0.1.5".into(), started: 100, clean: true });
        let ended: RunFile = serde_json::from_slice(&fs::read(path.join(RUN_FILE)).unwrap()).unwrap();
        assert!(ended.clean && ended.started == 100);
    }

    #[test]
    fn only_a_run_that_ended_badly_with_a_record_crashed() {
        let record = panic_record(Some("a.rs:1:1"), Some("boom"), None, None, "0.1.6", 150);
        let badly = PreviousRun { version: "0.1.6".into(), started: 100, clean: false };
        assert_eq!(crashed(&badly, std::slice::from_ref(&record)), Some(record.clone()));
        // Power cut or Task Manager: ended badly, nothing recorded.
        assert_eq!(crashed(&badly, &[]), None);
        // A record from an older run isn't this one's.
        let older = PreviousRun { started: 200, ..badly.clone() };
        assert_eq!(crashed(&older, std::slice::from_ref(&record)), None);
        let clean = PreviousRun { clean: true, ..badly };
        assert_eq!(crashed(&clean, &[record]), None);
    }

    #[test]
    fn formatted_panic_messages_keep_only_their_fixed_part() {
        let r = panic_record(
            Some("core/nectarlink-core/src/node.rs:10:5"),
            None,
            Some("called `Result::unwrap()` on an `Err` value: \"Mum's number 0712345678\""),
            Some("node".into()),
            "0.1.6",
            1,
        );
        assert_eq!(r.message, "called `Result::unwrap()` on an `Err` value: <details hidden>");
        assert!(!r.message.contains("0712"));
        let bare = panic_record(None, None, Some("Mum said hi"), None, "0.1.6", 1);
        assert_eq!(bare.message, "<details hidden>");
    }

    #[test]
    fn the_same_bug_has_the_same_signature_across_versions_and_lines() {
        let a = panic_record(Some("core/src/node.rs:10:5"), Some("boom"), None, None, "0.1.6", 1);
        let b = panic_record(Some("core/src/node.rs:42:9"), Some("boom"), None, None, "0.1.7", 2);
        let c = panic_record(Some("core/src/store.rs:10:5"), Some("boom"), None, None, "0.1.6", 1);
        assert_eq!(a.sig, b.sig);
        assert_ne!(a.sig, c.sig);
        assert_eq!(a.sig.len(), 12);
    }

    #[test]
    fn library_locations_lose_the_build_machine_and_version() {
        assert_eq!(
            normalize_location(
                r"C:\Users\runner\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\iroh-0.95.1\src\magicsock.rs:812:9"
            ),
            "iroh/src/magicsock.rs:812:9"
        );
        assert_eq!(
            normalize_location("/rustc/abc123/library/core/src/option.rs:2033:5"),
            "rust/library/core/src/option.rs:2033:5"
        );
        assert_eq!(normalize_location(r"desktop\app\src\mirror.rs:9:1"), "desktop/app/src/mirror.rs:9:1");
    }

    #[test]
    fn only_the_newest_records_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        for at in 0..8 {
            save(dir.path(), &panic_record(Some("a.rs:1:1"), Some("boom"), None, None, "0.1.6", 1000 + at));
        }
        let kept = records(dir.path());
        assert_eq!(kept.len(), KEEP);
        assert_eq!(kept[0].at, 1007);
        assert_eq!(kept[KEEP - 1].at, 1003);
    }
}
