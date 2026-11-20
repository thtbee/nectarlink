// SPDX-License-Identifier: MPL-2.0
//! Sending a report to the report service (`tools/report-worker`), which
//! files it as an issue, for people without a GitHub account.
//!
//! The platform does the HTTPS POST; this builds the request, proves the
//! work the service asks for, reads its answer, and keeps the one report
//! that couldn't be sent yet and the last few that were.

use std::{fs, path::Path};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::report::{Report, SCHEMA};

/// The report service. Reports go only here.
pub const REPORT_SERVICE: &str = "https://nectarlink-reports.nectarlink.workers.dev";
/// Zero bits the proof needs: about a second on a phone, once per report.
pub const PROOF_BITS: u32 = 20;
/// How many sent reports are kept to check for replies, and for how long.
const TRACKED: usize = 3;
const TRACK_SECS: u64 = 30 * 86_400;
const PENDING_FILE: &str = "pending-report.json";
const SENT_FILE: &str = "sent-reports.json";

/// Who made the report: someone writing it, or the crash card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Person,
    Crash,
}

/// A report ready to POST to `{REPORT_SERVICE}/v1/report`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outgoing {
    pub rid: String,
    /// The JSON body, sent as is: the proof covers these exact bytes.
    pub body: String,
    /// Sent as `X-Proof`.
    pub proof: String,
    /// When it was made, in seconds since 1970.
    pub at: u64,
}

#[derive(Serialize)]
struct Wire<'a> {
    schema: u32,
    rid: &'a str,
    kind: &'a str,
    area: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    sig: Option<&'a str>,
    summary: String,
    versions: Vec<String>,
    platforms: Vec<&'static str>,
    body: String,
}

impl Outgoing {
    /// Builds and proves the request for `report`, as rendered. `sig` is
    /// the crash it's about, required for [`Kind::Crash`]. Takes about a
    /// second: call it off the UI thread.
    pub fn new(report: &Report, kind: Kind, sig: Option<&str>, now: u64) -> Outgoing {
        let mut versions: Vec<String> = Vec::new();
        let mut platforms: Vec<&'static str> = Vec::new();
        for device in &report.devices {
            let v = &device.facts.app_version;
            if is_version(v) && !versions.contains(v) {
                versions.push(v.clone());
            }
            let p = platform(&device.facts.role, &device.facts.os);
            if !platforms.contains(&p) {
                platforms.push(p);
            }
        }
        versions.truncate(4);
        platforms.truncate(4);
        let wire = Wire {
            schema: SCHEMA,
            rid: &report.id,
            kind: match kind {
                Kind::Person => "person",
                Kind::Crash => "crash",
            },
            area: report.area.id(),
            sig: sig.filter(|s| s.len() == 12 && s.bytes().all(|b| b.is_ascii_hexdigit())),
            summary: report.text.lines().next().unwrap_or("").chars().take(120).collect(),
            versions,
            platforms,
            body: report.render(),
        };
        let body = serde_json::to_string(&wire).expect("plain data");
        let proof = prove(&hex(&Sha256::digest(body.as_bytes())), now, PROOF_BITS);
        Outgoing { rid: report.id.clone(), body, proof, at: now }
    }
}

/// What the service said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// Filed, or counted on the open issue for the same crash.
    Filed { issue: u64 },
    /// It will be filed shortly (GitHub busy, or the day's limit).
    Queued,
    /// Too many reports from here today.
    Limited,
    /// The service couldn't take it now; keep it and try later.
    Later,
    /// Refused for good: a bug in the app, so don't retry.
    Refused,
}

impl Answer {
    /// Reads an answer from the service: its status and body.
    pub fn read(status: u16, body: &[u8]) -> Answer {
        #[derive(Deserialize)]
        struct Filed {
            issue: u64,
        }
        match status {
            200 => serde_json::from_slice::<Filed>(body)
                .ok()
                .filter(|f| f.issue > 0)
                .map_or(Answer::Later, |f| Answer::Filed { issue: f.issue }),
            202 => Answer::Queued,
            429 => Answer::Limited,
            400 | 404 | 405 | 413 | 415 => Answer::Refused,
            // 403 is a proof that went stale, such as a wrong clock.
            _ => Answer::Later,
        }
    }
}

/// The issue's page, built here rather than taken from the answer.
pub fn issue_url(issue: u64) -> String {
    format!("https://github.com/thtbee/nectarlink/issues/{issue}")
}

/// Finds `n` so that SHA-256 of `<body hash>|<t>|<n>` starts with `bits`
/// zero bits; returns `<t>.<n>` for `X-Proof`.
pub fn prove(body_hash: &str, t: u64, bits: u32) -> String {
    let prefix = format!("{body_hash}|{t}|");
    let mut line = String::with_capacity(prefix.len() + 20);
    for n in 0u64.. {
        line.clear();
        line.push_str(&prefix);
        line.push_str(&n.to_string());
        if leading_zeros(&Sha256::digest(line.as_bytes())) >= bits {
            return format!("{t}.{n}");
        }
    }
    unreachable!("a proof exists")
}

fn leading_zeros(bytes: &[u8]) -> u32 {
    let mut n = 0;
    for b in bytes {
        if *b != 0 {
            return n + b.leading_zeros();
        }
        n += 8;
    }
    n
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn is_version(v: &str) -> bool {
    let (core, suffix) = v.split_once('-').unwrap_or((v, ""));
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .zip([3, 3, 4])
            .all(|(p, max)| !p.is_empty() && p.len() <= max && p.bytes().all(|b| b.is_ascii_digit()))
        && (v.len() == core.len()
            || (!suffix.is_empty()
                && suffix.len() <= 20
                && suffix.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.')))
}

fn platform(role: &str, os: &str) -> &'static str {
    let os = os.to_ascii_lowercase();
    if os.contains("android") || role.eq_ignore_ascii_case("phone") {
        "android"
    } else if os.contains("windows") {
        "windows"
    } else if os.contains("macos") || os.contains("mac os") {
        "macos"
    } else {
        "linux"
    }
}

/// The report waiting to be sent, kept in `dir` until the next update
/// check. Only one: a newer one replaces it.
pub fn keep_pending(dir: &Path, outgoing: &Outgoing) {
    if let Ok(json) = serde_json::to_vec(outgoing) {
        let _ = fs::create_dir_all(dir);
        let _ = fs::write(dir.join(PENDING_FILE), json);
    }
}

pub fn pending(dir: &Path) -> Option<Outgoing> {
    fs::read(dir.join(PENDING_FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

pub fn clear_pending(dir: &Path) {
    let _ = fs::remove_file(dir.join(PENDING_FILE));
}

/// A report that was sent: filed as `issue`, or still queued.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sent {
    pub rid: String,
    pub issue: Option<u64>,
    pub at: u64,
    /// Comments seen when last checked, for "there's a reply".
    #[serde(default)]
    pub comments: u64,
    #[serde(default)]
    pub closed: bool,
}

/// The sent reports still followed: the newest [`TRACKED`], 30 days at most.
pub fn sent(dir: &Path, now: u64) -> Vec<Sent> {
    all_sent(dir).into_iter().filter(|s| now.saturating_sub(s.at) < TRACK_SECS).collect()
}

fn all_sent(dir: &Path) -> Vec<Sent> {
    fs::read(dir.join(SENT_FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Adds or updates `entry` among the followed reports.
pub fn remember(dir: &Path, entry: Sent, now: u64) {
    let mut all = sent(dir, now);
    all.retain(|s| s.rid != entry.rid);
    all.push(entry);
    all.sort_by_key(|s| s.at);
    let drop = all.len().saturating_sub(TRACKED);
    all.drain(..drop);
    save_sent(dir, &all);
}

/// Stops following the report `rid`.
pub fn forget(dir: &Path, rid: &str) {
    let mut all = all_sent(dir);
    let before = all.len();
    all.retain(|s| s.rid != rid);
    if all.len() != before {
        save_sent(dir, &all);
    }
}

fn save_sent(dir: &Path, all: &[Sent]) {
    if let Ok(json) = serde_json::to_vec(all) {
        let _ = fs::create_dir_all(dir);
        let _ = fs::write(dir.join(SENT_FILE), json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::report::{Area, DeviceFacts, DevicePart};

    fn report() -> Report {
        let mut r = Report::new(Area::Connection, "", "The phone drops off\nevery few minutes", false);
        for (role, version, os) in
            [("PC", "0.1.6", "Windows 11 26100"), ("Phone", "0.1.6-nightly.4", "Android 16")]
        {
            r.devices.push(DevicePart {
                facts: DeviceFacts {
                    role: role.into(),
                    app_version: version.into(),
                    os: os.into(),
                    ..DeviceFacts::default()
                },
                ..DevicePart::default()
            });
        }
        r
    }

    #[test]
    fn proof_matches_the_worker() {
        // tools/report-worker/test/worker.test.ts, "matches the app's test vector"
        assert_eq!(
            hex(&Sha256::digest(b"abc|1795000000|0")),
            "096f166aa433a9b7c070d18c9b70d8727ffbcf0d9f0b81f4ae94221aeefe2a89"
        );
        let proof = prove("abc", 1_795_000_000, 12);
        let (t, n) = proof.split_once('.').unwrap();
        assert_eq!(t, "1795000000");
        let digest = Sha256::digest(format!("abc|{t}|{n}").as_bytes());
        assert!(leading_zeros(&digest) >= 12);
        assert_eq!(leading_zeros(&[0, 0, 0x0f]), 20);
        assert_eq!(leading_zeros(&[0x80]), 0);
    }

    #[test]
    fn the_request_is_what_the_service_checks() {
        let r = report();
        let out = Outgoing::new(&r, Kind::Person, None, 1_795_000_000);
        let json: serde_json::Value = serde_json::from_str(&out.body).unwrap();
        assert_eq!(json["schema"], 1);
        assert_eq!(json["rid"], r.id.as_str());
        assert_eq!(json["kind"], "person");
        assert_eq!(json["area"], "connection");
        assert_eq!(json["summary"], "The phone drops off");
        assert_eq!(json["versions"], serde_json::json!(["0.1.6", "0.1.6-nightly.4"]));
        assert_eq!(json["platforms"], serde_json::json!(["windows", "android"]));
        assert!(json.get("sig").is_none());
        assert_eq!(json["body"], r.render());
        let hash = hex(&Sha256::digest(out.body.as_bytes()));
        let (t, n) = out.proof.split_once('.').unwrap();
        assert!(leading_zeros(&Sha256::digest(format!("{hash}|{t}|{n}").as_bytes())) >= PROOF_BITS);
    }

    #[test]
    fn crash_signatures_and_versions_are_checked_before_sending() {
        let mut r = report();
        r.devices[0].facts.app_version = "dev build".into();
        let out = Outgoing::new(&r, Kind::Crash, Some("b3af6c6e31db"), 1);
        let json: serde_json::Value = serde_json::from_str(&out.body).unwrap();
        assert_eq!(json["kind"], "crash");
        assert_eq!(json["sig"], "b3af6c6e31db");
        assert_eq!(json["versions"], serde_json::json!(["0.1.6-nightly.4"]));
        let out = Outgoing::new(&r, Kind::Person, Some("not a sig"), 1);
        assert!(!out.body.contains("\"sig\""));
        for (v, ok) in [
            ("0.1.6", true),
            ("10.20.3000-rc.1", true),
            ("0.1", false),
            ("0.1.6-", false),
            ("1.2.3 x", false),
        ] {
            assert_eq!(is_version(v), ok, "{v}");
        }
    }

    #[test]
    fn answers() {
        assert_eq!(
            Answer::read(200, br#"{"issue":412,"url":"https://evil.example"}"#),
            Answer::Filed { issue: 412 }
        );
        assert_eq!(Answer::read(200, b"nonsense"), Answer::Later);
        assert_eq!(Answer::read(202, br#"{"queued":true}"#), Answer::Queued);
        assert_eq!(Answer::read(429, b""), Answer::Limited);
        assert_eq!(Answer::read(400, b""), Answer::Refused);
        assert_eq!(Answer::read(403, b""), Answer::Later);
        assert_eq!(Answer::read(503, b""), Answer::Later);
        assert_eq!(issue_url(412), "https://github.com/thtbee/nectarlink/issues/412");
    }

    #[test]
    fn one_pending_report_and_three_followed() {
        let dir = tempfile::tempdir().unwrap();
        let out = Outgoing::new(&report(), Kind::Person, None, 5);
        keep_pending(dir.path(), &out);
        assert_eq!(pending(dir.path()), Some(out));
        clear_pending(dir.path());
        assert_eq!(pending(dir.path()), None);

        let now = 40 * 86_400;
        for (i, at) in [1, now - 4, now - 3, now - 2, now - 1].into_iter().enumerate() {
            remember(
                dir.path(),
                Sent { rid: format!("r{i}"), issue: Some(i as u64), at, comments: 0, closed: false },
                now,
            );
        }
        let rids: Vec<String> = sent(dir.path(), now).into_iter().map(|s| s.rid).collect();
        assert_eq!(rids, ["r2", "r3", "r4"]);
        forget(dir.path(), "r3");
        let rids: Vec<String> = sent(dir.path(), now).into_iter().map(|s| s.rid).collect();
        assert_eq!(rids, ["r2", "r4"]);
    }
}
