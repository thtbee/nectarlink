// SPDX-License-Identifier: MPL-2.0
//! Sending a report to the report service (`tools/report-worker`), which
//! files it as an issue, for people without a GitHub account.
//!
//! The platform does the HTTPS POST; this builds the request, proves the
//! work the service asks for, reads its answer, and keeps the one report
//! that couldn't be sent yet and the last few that were.
//!
//! Each report carries the hash of a key only this device keeps, so the
//! person can read the answers on its issue and reply from the app, without
//! a GitHub account: [`read_thread`] and [`reply_request`].

use std::{fs, path::Path};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::report::{Report, SCHEMA};

/// The report service. Reports go only here.
pub const REPORT_SERVICE: &str = "https://nectarlink-reports.nectarlink.workers.dev";
/// Zero bits the proof needs: about a second on a phone, once per report.
pub const PROOF_BITS: u32 = 20;
/// How many sent reports are kept to check for replies, and for how long.
const TRACKED: usize = 5;
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
    /// The key for replying from the app (64 hex characters); only its
    /// hash goes to the service. Empty for a report made before replies.
    #[serde(default)]
    pub reply_key: String,
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
    reply_hash: String,
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
        let reply_key = hex(&rand::random::<[u8; 32]>());
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
            reply_hash: hex(&Sha256::digest(reply_key.as_bytes())),
        };
        let body = serde_json::to_string(&wire).expect("plain data");
        let proof = prove(&hex(&Sha256::digest(body.as_bytes())), now, PROOF_BITS);
        Outgoing { rid: report.id.clone(), body, proof, at: now, reply_key }
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

/// Clears the pending report if it's `rid` (it went out after all).
pub fn clear_pending_if(dir: &Path, rid: &str) {
    if pending(dir).is_some_and(|p| p.rid == rid) {
        clear_pending(dir);
    }
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
    /// See [`Outgoing::reply_key`].
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reply_key: String,
}

impl Sent {
    /// `outgoing`, sent: filed as `issue`, or queued when `None`.
    pub fn from_outgoing(outgoing: &Outgoing, issue: Option<u64>) -> Sent {
        Sent {
            rid: outgoing.rid.clone(),
            issue,
            at: outgoing.at,
            comments: 0,
            closed: false,
            reply_key: outgoing.reply_key.clone(),
        }
    }

    /// Whether the person can reply to it from the app.
    pub fn can_reply(&self) -> bool {
        self.issue.is_some() && self.reply_key.len() == 64
    }
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

/// Something new on a followed report's issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    /// Someone commented.
    Replied { issue: u64 },
    /// It was closed (fixed, or answered).
    Closed { issue: u64 },
}

/// GitHub's public API address for `issue`: its state and comment count.
pub fn issue_api_url(issue: u64) -> String {
    format!("https://api.github.com/repos/thtbee/nectarlink/issues/{issue}")
}

/// Reads GitHub's answer about the followed report `rid` (`status`, `body`)
/// and says what's new since the last check, if anything: a new comment,
/// or closing. Only numbers are read here; the answers themselves come from
/// [`read_thread`].
pub fn check_issue(dir: &Path, rid: &str, status: u16, body: &[u8], now: u64) -> Option<Notice> {
    #[derive(Deserialize)]
    struct Issue {
        number: u64,
        state: String,
        comments: u64,
    }
    if status != 200 {
        return None;
    }
    let issue: Issue = serde_json::from_slice(body).ok()?;
    let entry = sent(dir, now).into_iter().find(|s| s.rid == rid && s.issue == Some(issue.number))?;
    let closed = issue.state == "closed";
    let notice = if closed && !entry.closed {
        Some(Notice::Closed { issue: issue.number })
    } else if issue.comments > entry.comments {
        Some(Notice::Replied { issue: issue.number })
    } else {
        None
    };
    // A closed report stays followed: its conversation is still shown in
    // the app, and an answer after closing is still news.
    if closed != entry.closed || issue.comments != entry.comments {
        remember(dir, Sent { comments: issue.comments, closed, ..entry }, now);
    }
    notice
}

/// GitHub's public API address for `issue`'s comments, oldest first.
pub fn comments_api_url(issue: u64) -> String {
    format!("https://api.github.com/repos/thtbee/nectarlink/issues/{issue}/comments?per_page=100")
}

/// Who wrote one answer on a report's issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Who {
    /// Someone who works on NectarLink.
    Team,
    /// The person who sent the report, replying from the app.
    You,
    /// Someone else who sent the same crash, replying from their app.
    OtherReporter,
    /// Anyone else on GitHub, by their login.
    Someone(String),
}

/// One answer on a report's issue, as the app shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadEntry {
    pub who: Who,
    pub text: String,
    /// When it was written, as GitHub gives it (`2026-11-25T17:14:26Z`).
    pub at: String,
}

/// Longest answer shown, in characters; the rest is on GitHub.
const MAX_ENTRY_CHARS: usize = 4000;
const REPLY_MARKER: &str = "<!-- nectarlink-reply rid=";

/// Reads GitHub's comments on the issue of the report `rid` (`status`,
/// `body` of [`comments_api_url`]): the answers to show, oldest first.
/// The service's own notes (a crash's tally) are left out; the person's
/// replies from the app are theirs. `None` when GitHub didn't answer.
pub fn read_thread(rid: &str, status: u16, body: &[u8]) -> Option<Vec<ThreadEntry>> {
    #[derive(Deserialize)]
    struct User {
        login: String,
        #[serde(rename = "type", default)]
        kind: String,
    }
    #[derive(Deserialize)]
    struct Comment {
        user: Option<User>,
        #[serde(default)]
        author_association: String,
        #[serde(default)]
        body: String,
        #[serde(default)]
        created_at: String,
    }
    if status != 200 {
        return None;
    }
    let comments: Vec<Comment> = serde_json::from_slice(body).ok()?;
    let mine = format!("{REPLY_MARKER}{rid} -->");
    let mut out = Vec::new();
    for c in comments {
        let Some(user) = c.user else { continue };
        let bot = user.kind == "Bot" || user.login.ends_with("[bot]");
        let who = if bot {
            if c.body.contains(&mine) {
                Who::You
            } else if c.body.contains(REPLY_MARKER) {
                Who::OtherReporter
            } else {
                // The crash tally and anything else the service writes.
                continue;
            }
        } else if matches!(c.author_association.as_str(), "OWNER" | "MEMBER" | "COLLABORATOR") {
            Who::Team
        } else {
            Who::Someone(user.login)
        };
        let text = if bot { unquote_reply(&c.body) } else { clean_comment(&c.body) };
        if text.is_empty() {
            continue;
        }
        out.push(ThreadEntry { who, text, at: c.created_at });
    }
    Some(out)
}

/// The newest answer from anyone but the person, for a notification.
pub fn latest_answer(entries: &[ThreadEntry]) -> Option<&ThreadEntry> {
    entries.iter().rev().find(|e| e.who != Who::You)
}

/// A reply comment's text: the quoted lines, without the service's line,
/// its marker, or the characters it added to stop mentions.
fn unquote_reply(body: &str) -> String {
    let lines: Vec<String> = body
        .lines()
        .filter(|l| l.starts_with('>'))
        .map(|l| l.strip_prefix("> ").or_else(|| l.strip_prefix('>')).unwrap_or(l).replace('\u{200b}', ""))
        .collect();
    limit(&lines.join("\n").replace("&lt;", "<"))
}

/// A comment's text without HTML comments, trimmed and limited.
fn clean_comment(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        match rest[start..].find("-->") {
            Some(end) => rest = &rest[start + end + 3..],
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    limit(out.replace("\r\n", "\n").trim())
}

fn limit(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= MAX_ENTRY_CHARS {
        return text.to_owned();
    }
    let cut: String = text.chars().take(MAX_ENTRY_CHARS).collect();
    format!("{}…", cut.trim_end())
}

/// Longest reply, in characters (the service's limit).
pub const MAX_REPLY_CHARS: usize = 2000;

/// A reply ready to POST to `url`, with `X-Proof`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingReply {
    pub url: String,
    pub body: String,
    pub proof: String,
}

/// Builds and proves a reply to the sent report `sent`. `None` when it
/// can't take one (made before replies, or still queued) or `text` is
/// empty or too long. Takes about a second: off the UI thread.
pub fn reply_request(sent: &Sent, text: &str, now: u64) -> Option<OutgoingReply> {
    let text = text.trim();
    if !sent.can_reply() || text.is_empty() || text.chars().count() > MAX_REPLY_CHARS {
        return None;
    }
    #[derive(Serialize)]
    struct Body<'a> {
        key: &'a str,
        text: &'a str,
    }
    let body = serde_json::to_string(&Body { key: &sent.reply_key, text }).expect("plain data");
    let proof = prove(&hex(&Sha256::digest(body.as_bytes())), now, PROOF_BITS);
    Some(OutgoingReply { url: format!("{REPORT_SERVICE}/v1/report/{}/reply", sent.rid), body, proof })
}

/// What the service said to a reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyAnswer {
    Sent,
    /// Ten replies today already.
    Limited,
    /// Try again later (GitHub busy, a stale proof, no network).
    Later,
    /// Not taken: the report is unknown or too old to reply to.
    Refused,
}

impl ReplyAnswer {
    pub fn read(status: u16) -> ReplyAnswer {
        match status {
            200 => ReplyAnswer::Sent,
            429 => ReplyAnswer::Limited,
            400 | 404 | 405 | 413 | 415 => ReplyAnswer::Refused,
            _ => ReplyAnswer::Later,
        }
    }
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

    #[test]
    fn a_report_carries_only_the_hash_of_its_reply_key() {
        let out = Outgoing::new(&report(), Kind::Person, None, 1_795_000_000);
        assert_eq!(out.reply_key.len(), 64);
        assert!(out.reply_key.bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(!out.body.contains(&out.reply_key));
        let wire: serde_json::Value = serde_json::from_str(&out.body).unwrap();
        assert_eq!(wire["reply_hash"], hex(&Sha256::digest(out.reply_key.as_bytes())));
        // Two reports, two keys.
        assert_ne!(Outgoing::new(&report(), Kind::Person, None, 1_795_000_000).reply_key, out.reply_key);
        let sent = Sent::from_outgoing(&out, Some(5));
        assert!(sent.can_reply());
        assert!(!Sent::from_outgoing(&out, None).can_reply());
    }

    #[test]
    fn a_followed_report_from_before_replies_still_reads() {
        let old = br#"[{"rid":"r1","issue":4,"at":1795000000,"comments":1,"closed":false}]"#;
        let all: Vec<Sent> = serde_json::from_slice(old).unwrap();
        assert_eq!(all[0].reply_key, "");
        assert!(!all[0].can_reply());
        let pending = br#"{"rid":"r2","body":"{}","proof":"1.2","at":1795000000}"#;
        let o: Outgoing = serde_json::from_slice(pending).unwrap();
        assert_eq!(o.reply_key, "");
    }

    #[test]
    fn the_thread_shows_the_team_others_and_the_persons_own_replies() {
        let body = br#"[
            {"user":{"login":"nectarlink-reports[bot]","type":"Bot"},"author_association":"NONE",
             "body":"Reported from the app **2** times.","created_at":"2026-11-25T10:00:00Z"},
            {"user":{"login":"thtbee","type":"User"},"author_association":"OWNER",
             "body":"Thanks. What were you doing?\r\n<!-- hidden note -->","created_at":"2026-11-25T11:00:00Z"},
            {"user":{"login":"nectarlink-reports[bot]","type":"Bot"},"author_association":"NONE",
             "body":"The reporter replied from the app:\n\n> Mirroring, @\u200bthtbee see #\u200b12\n>\n> &lt;b>\n\n<!-- nectarlink-reply rid=abc123 -->",
             "created_at":"2026-11-25T12:00:00Z"},
            {"user":{"login":"nectarlink-reports[bot]","type":"Bot"},"author_association":"NONE",
             "body":"The reporter replied from the app:\n\n> Same here\n\n<!-- nectarlink-reply rid=zzz999 -->",
             "created_at":"2026-11-25T12:30:00Z"},
            {"user":{"login":"helper","type":"User"},"author_association":"CONTRIBUTOR",
             "body":"Try Nightly 13.","created_at":"2026-11-25T13:00:00Z"},
            {"user":null,"author_association":"NONE","body":"ghost","created_at":"x"}
        ]"#;
        let thread = read_thread("abc123", 200, body).unwrap();
        let who: Vec<&Who> = thread.iter().map(|e| &e.who).collect();
        assert_eq!(who, [&Who::Team, &Who::You, &Who::OtherReporter, &Who::Someone("helper".into())]);
        assert_eq!(thread[0].text, "Thanks. What were you doing?");
        assert_eq!(thread[1].text, "Mirroring, @thtbee see #12\n\n<b>");
        assert_eq!(thread[1].at, "2026-11-25T12:00:00Z");
        assert_eq!(latest_answer(&thread).map(|e| e.text.as_str()), Some("Try Nightly 13."));
        assert_eq!(read_thread("abc123", 403, b"[]"), None);
        assert_eq!(read_thread("abc123", 200, b"not json"), None);
    }

    #[test]
    fn a_reply_is_proved_and_sent_to_its_report() {
        let out = Outgoing::new(&report(), Kind::Person, None, 1_795_000_000);
        let sent = Sent::from_outgoing(&out, Some(9));
        let r = reply_request(&sent, "  It happens every time.  ", 1_795_000_100).unwrap();
        assert_eq!(r.url, format!("{REPORT_SERVICE}/v1/report/{}/reply", sent.rid));
        let body: serde_json::Value = serde_json::from_str(&r.body).unwrap();
        assert_eq!(body["key"], sent.reply_key);
        assert_eq!(body["text"], "It happens every time.");
        let (t, n) = r.proof.split_once('.').unwrap();
        assert_eq!(t, "1795000100");
        let line = format!("{}|{t}|{n}", hex(&Sha256::digest(r.body.as_bytes())));
        assert!(leading_zeros(&Sha256::digest(line.as_bytes())) >= PROOF_BITS);
        assert!(reply_request(&sent, "   ", 0).is_none());
        assert!(reply_request(&sent, &"x".repeat(MAX_REPLY_CHARS + 1), 0).is_none());
        assert!(reply_request(&Sent::from_outgoing(&out, None), "hi", 0).is_none());
        assert_eq!(ReplyAnswer::read(200), ReplyAnswer::Sent);
        assert_eq!(ReplyAnswer::read(429), ReplyAnswer::Limited);
        assert_eq!(ReplyAnswer::read(404), ReplyAnswer::Refused);
        assert_eq!(ReplyAnswer::read(503), ReplyAnswer::Later);
    }

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
    fn replies_and_closing_are_noticed_once() {
        let dir = tempfile::tempdir().unwrap();
        let now = 1_000_000;
        remember(
            dir.path(),
            Sent {
                rid: "r".into(),
                issue: Some(412),
                at: now,
                comments: 0,
                closed: false,
                reply_key: String::new(),
            },
            now,
        );
        let issue = |state: &str, comments: u64| {
            format!(r#"{{"number":412,"state":"{state}","comments":{comments},"title":"anything"}}"#)
                .into_bytes()
        };
        assert_eq!(check_issue(dir.path(), "r", 200, &issue("open", 0), now), None);
        assert_eq!(
            check_issue(dir.path(), "r", 200, &issue("open", 2), now),
            Some(Notice::Replied { issue: 412 })
        );
        assert_eq!(check_issue(dir.path(), "r", 200, &issue("open", 2), now), None);
        assert_eq!(check_issue(dir.path(), "r", 304, b"", now), None);
        assert_eq!(
            check_issue(dir.path(), "r", 200, &issue("closed", 3), now),
            Some(Notice::Closed { issue: 412 })
        );
        // Still followed, closed, and an answer after closing is still news.
        assert!(sent(dir.path(), now).iter().any(|s| s.rid == "r" && s.closed && s.comments == 3));
        assert_eq!(check_issue(dir.path(), "r", 200, &issue("closed", 3), now), None);
        assert_eq!(
            check_issue(dir.path(), "r", 200, &issue("closed", 4), now),
            Some(Notice::Replied { issue: 412 })
        );
        // Another issue's answer doesn't count for this report.
        remember(
            dir.path(),
            Sent {
                rid: "q".into(),
                issue: Some(7),
                at: now,
                comments: 0,
                closed: false,
                reply_key: String::new(),
            },
            now,
        );
        assert_eq!(check_issue(dir.path(), "q", 200, &issue("open", 5), now), None);
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
                Sent {
                    rid: format!("r{i}"),
                    issue: Some(i as u64),
                    at,
                    comments: 0,
                    closed: false,
                    reply_key: String::new(),
                },
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
