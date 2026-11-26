// SPDX-License-Identifier: MPL-2.0
//! How each feature has been doing on this device: per day, how often it
//! worked and how often it failed and why, as counts of fixed codes. A
//! report says "files failed 3 times today (no space)" from these, and a
//! check can spot a feature that keeps failing.
//!
//! Counting is one atomic add, from [`observe`] (which sees every event the
//! core sends) and [`count`]. Counts reach `health.json` only when
//! [`flush_if_due`] runs on something that happens anyway (a device
//! connecting or going), at most every [`FLUSH_EVERY`], and when the app
//! quits ([`flush`]). There's no timer. [`DAYS`] days are kept, about 10 KB.

use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU32, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use nectarlink_protocol::DeviceId;
use serde::{Deserialize, Serialize};

use crate::{
    events::{ConnectionPath, LinkState, NodeEvent, PairingEvent, PairingFailure},
    transfer::{TransferFailure, TransferState},
};

/// How many days are kept.
pub const DAYS: usize = 14;
/// The least time between two writes of the file.
pub const FLUSH_EVERY: Duration = Duration::from_secs(5 * 60);
const FILE: &str = "health.json";

macro_rules! closed_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum $name { $($variant),+ }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self { $($name::$variant => $text),+ }
            }

            pub fn parse(text: &str) -> Option<$name> {
                match text { $($text => Some($name::$variant),)+ _ => None }
            }
        }
    };
}

closed_enum! {
    /// What was counted.
    Feature {
        Connect => "connect",
        Drop => "drop",
        Pairing => "pairing",
        Files => "files",
        Mirror => "mirror",
        AppWindow => "app_window",
        Webcam => "webcam",
        Mic => "mic",
        Clipboard => "clipboard",
        Crash => "crash",
        UncleanExit => "unclean_exit",
        BackgroundKill => "background_kill",
    }
}

closed_enum! {
    /// How it went: worked (`ok`, or the way it connected), or why not.
    Code {
        Ok => "ok",
        Lan => "lan",
        Usb => "usb",
        Relay => "relay",
        Bluetooth => "bluetooth",
        Cancelled => "cancelled",
        Denied => "denied",
        Declined => "declined",
        Rejected => "rejected",
        Expired => "expired",
        Unreachable => "unreachable",
        NoSpace => "no_space",
        Interrupted => "interrupted",
        Integrity => "integrity",
        Other => "other",
    }
}

const CELLS: usize = 12 * 15;
const _: () = assert!(Feature::ALL.len() * Code::ALL.len() == CELLS);

static COUNTS: [AtomicU32; CELLS] = [const { AtomicU32::new(0) }; CELLS];
static DIR: OnceLock<PathBuf> = OnceLock::new();
static LAST_FLUSH: AtomicU64 = AtomicU64::new(0);
/// Writes one at a time, and what's needed to count only real changes.
static STATE: Mutex<Seen> = Mutex::new(Seen { online: None, finished: VecDeque::new() });

struct Seen {
    online: Option<HashMap<DeviceId, bool>>,
    finished: VecDeque<String>,
}

/// Where the counts are kept: the diagnostics folder. Counts from before
/// this are kept too, at the next flush.
pub fn init(dir: &Path) {
    let _ = DIR.set(dir.to_owned());
}

/// Counts one `feature` that went `code`.
pub fn count(feature: Feature, code: Code) {
    COUNTS[feature as usize * Code::ALL.len() + code as usize].fetch_add(1, Ordering::Relaxed);
}

/// Counts what `event` says worked or failed.
pub fn observe(event: &NodeEvent) {
    match event {
        NodeEvent::LinkChanged { device, link } => {
            let online = matches!(link, LinkState::Online { .. });
            let was = {
                let mut seen = STATE.lock().unwrap_or_else(|e| e.into_inner());
                seen.online.get_or_insert_with(HashMap::new).insert(*device, online)
            };
            match link {
                LinkState::Online { path, .. } if was != Some(true) => {
                    count(Feature::Connect, path_code(*path))
                }
                LinkState::Offline { .. } if was == Some(true) => count(Feature::Drop, Code::Ok),
                _ => {}
            }
            flush_if_due();
        }
        NodeEvent::Pairing(PairingEvent::Paired(_)) => count(Feature::Pairing, Code::Ok),
        NodeEvent::Pairing(PairingEvent::Failed(why)) => count(Feature::Pairing, pairing_code(why)),
        NodeEvent::Transfer(transfer) if transfer.state.is_finished() => {
            // A finished transfer may be sent more than once; count it once.
            {
                let mut seen = STATE.lock().unwrap_or_else(|e| e.into_inner());
                if seen.finished.contains(&transfer.id) {
                    return;
                }
                if seen.finished.len() >= 64 {
                    seen.finished.pop_front();
                }
                seen.finished.push_back(transfer.id.clone());
            }
            count(Feature::Files, transfer_code(&transfer.state));
        }
        NodeEvent::Mirroring { session, on: true, .. } => {
            count(if *session == 0 { Feature::Mirror } else { Feature::AppWindow }, Code::Ok);
        }
        NodeEvent::Webcam { on: true, .. } => count(Feature::Webcam, Code::Ok),
        NodeEvent::Mic { on: true, .. } => count(Feature::Mic, Code::Ok),
        NodeEvent::ClipboardReceived { .. } => count(Feature::Clipboard, Code::Ok),
        _ => {}
    }
}

fn path_code(path: ConnectionPath) -> Code {
    match path {
        ConnectionPath::Lan => Code::Lan,
        ConnectionPath::Usb => Code::Usb,
        ConnectionPath::Relay => Code::Relay,
        ConnectionPath::Bluetooth => Code::Bluetooth,
    }
}

fn pairing_code(why: &PairingFailure) -> Code {
    match why {
        PairingFailure::Rejected => Code::Rejected,
        PairingFailure::Declined => Code::Declined,
        PairingFailure::Expired => Code::Expired,
        PairingFailure::Unreachable => Code::Unreachable,
        PairingFailure::NotPairing => Code::Denied,
        PairingFailure::Other(_) => Code::Other,
    }
}

fn transfer_code(state: &TransferState) -> Code {
    match state {
        TransferState::Done { .. } => Code::Ok,
        TransferState::Cancelled => Code::Cancelled,
        TransferState::Failed(why) => match why {
            TransferFailure::Denied => Code::Denied,
            TransferFailure::Unreachable => Code::Unreachable,
            TransferFailure::NoSpace => Code::NoSpace,
            TransferFailure::Interrupted => Code::Interrupted,
            TransferFailure::Integrity => Code::Integrity,
            TransferFailure::Other(_) => Code::Other,
        },
        _ => Code::Other,
    }
}

/// One day's counts: feature, then code, then how many.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Day {
    /// `2026-11-20` (UTC).
    pub day: String,
    pub counts: BTreeMap<String, BTreeMap<String, u32>>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct HealthFile {
    days: Vec<Day>,
}

/// Writes the counts so far, if the last write was [`FLUSH_EVERY`] ago.
pub fn flush_if_due() {
    let now = now_secs();
    let last = LAST_FLUSH.load(Ordering::Relaxed);
    if now.saturating_sub(last) >= FLUSH_EVERY.as_secs()
        && LAST_FLUSH.compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed).is_ok()
    {
        flush();
    }
}

/// Writes the counts so far into today's row.
pub fn flush() {
    let Some(dir) = DIR.get() else { return };
    // One writer at a time, so no count is added twice or lost.
    let _guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let taken: Vec<u32> = COUNTS.iter().map(|c| c.swap(0, Ordering::Relaxed)).collect();
    if taken.iter().all(|&n| n == 0) {
        return;
    }
    let mut file = read(dir);
    add(&mut file, &today(), &taken);
    let _ = fs::create_dir_all(dir);
    if let Ok(json) = serde_json::to_vec(&file) {
        let tmp = dir.join("health.json.tmp");
        if fs::write(&tmp, json).is_ok() {
            let _ = fs::rename(tmp, dir.join(FILE));
        }
    }
}

fn add(file: &mut HealthFile, day: &str, taken: &[u32]) {
    if file.days.last().is_none_or(|d| d.day != day) {
        file.days.push(Day { day: day.to_owned(), counts: BTreeMap::new() });
    }
    let row = file.days.last_mut().expect("just added");
    for feature in Feature::ALL {
        for code in Code::ALL {
            let n = taken[*feature as usize * Code::ALL.len() + *code as usize];
            if n > 0 {
                let cell = row
                    .counts
                    .entry(feature.as_str().into())
                    .or_default()
                    .entry(code.as_str().into())
                    .or_default();
                *cell = cell.saturating_add(n);
            }
        }
    }
    let extra = file.days.len().saturating_sub(DAYS);
    file.days.drain(..extra);
}

fn read(dir: &Path) -> HealthFile {
    let mut file: HealthFile =
        fs::read(dir.join(FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    // Only known names: a file edited by hand can't add others to a report.
    for day in &mut file.days {
        day.counts.retain(|feature, codes| {
            codes.retain(|code, _| Code::parse(code).is_some());
            Feature::parse(feature).is_some()
        });
    }
    file
}

/// The days kept in `dir`, oldest first, with what hasn't been written yet
/// added to today's.
pub fn days(dir: &Path) -> Vec<Day> {
    let mut file = read(dir);
    let pending: Vec<u32> = COUNTS.iter().map(|c| c.load(Ordering::Relaxed)).collect();
    if pending.iter().any(|&n| n > 0) {
        add(&mut file, &today(), &pending);
    }
    file.days
}

fn today() -> String {
    let (y, m, d) = super::layer::civil_from_days((now_secs() / 86_400) as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells(pairs: &[(Feature, Code, u32)]) -> Vec<u32> {
        let mut v = vec![0; CELLS];
        for (f, c, n) in pairs {
            v[*f as usize * Code::ALL.len() + *c as usize] = *n;
        }
        v
    }

    #[test]
    fn counts_add_up_per_day_and_old_days_go() {
        let mut file = HealthFile::default();
        add(&mut file, "2026-11-01", &cells(&[(Feature::Files, Code::Ok, 2)]));
        add(
            &mut file,
            "2026-11-01",
            &cells(&[(Feature::Files, Code::Ok, 1), (Feature::Files, Code::NoSpace, 1)]),
        );
        assert_eq!(file.days.len(), 1);
        assert_eq!(file.days[0].counts["files"]["ok"], 3);
        assert_eq!(file.days[0].counts["files"]["no_space"], 1);
        for d in 2..=20 {
            add(&mut file, &format!("2026-11-{d:02}"), &cells(&[(Feature::Connect, Code::Lan, 1)]));
        }
        assert_eq!(file.days.len(), DAYS);
        assert_eq!(file.days[0].day, "2026-11-07");
    }

    #[test]
    fn names_round_trip_and_unknown_ones_are_dropped() {
        for f in Feature::ALL {
            assert_eq!(Feature::parse(f.as_str()), Some(*f));
        }
        for c in Code::ALL {
            assert_eq!(Code::parse(c.as_str()), Some(*c));
        }
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(FILE),
            r#"{"days":[{"day":"2026-11-20","counts":{"files":{"ok":1,"Mum":5},"secret":{"ok":1}}}]}"#,
        )
        .unwrap();
        let file = read(dir.path());
        assert_eq!(file.days[0].counts.len(), 1);
        assert_eq!(file.days[0].counts["files"].len(), 1);
    }

    #[test]
    fn transfers_and_pairing_map_to_codes() {
        assert_eq!(transfer_code(&TransferState::Failed(TransferFailure::NoSpace)), Code::NoSpace);
        assert_eq!(transfer_code(&TransferState::Failed(TransferFailure::Other("x".into()))), Code::Other);
        assert_eq!(transfer_code(&TransferState::Cancelled), Code::Cancelled);
        assert_eq!(pairing_code(&PairingFailure::Expired), Code::Expired);
    }
}
