// SPDX-License-Identifier: MPL-2.0
//! A problem report: what the user says went wrong, and what this device
//! (and, when it agrees, the paired one) can tell about it, as one plain
//! text the user reads in full before it goes anywhere.
//!
//! Everything in it is either the user's own words, or comes from places
//! that are private by construction: system facts (versions, models), the
//! Doctor's check ids, [`health`] counts, [`crash`] records and the
//! [`super::ring`] log. [`Report::render`] makes the text; [`Report::issue_url`]
//! a link to a new GitHub issue with the short parts filled in, since a link
//! can't carry the log.

use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use super::{
    crash::CrashRecord,
    health::{self, Day},
    redact,
};

/// The report format's version, in its first line.
pub const SCHEMA: u32 = 1;
/// The largest report: what a GitHub issue can hold, with room to spare.
pub const MAX_BYTES: usize = 60 * 1024;
/// The longest description kept.
pub const MAX_TEXT: usize = 4000;
/// How much of the log the report takes from each device.
const MAX_LOG_BYTES: usize = 24 * 1024;
const REPO: &str = "https://github.com/thtbee/nectarlink";

/// What the problem is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    Connection,
    AppWindows,
    Clipboard,
    Audio,
    Calls,
    Files,
    Notifications,
    Other,
}

impl Area {
    pub const ALL: &'static [Area] = &[
        Area::Connection,
        Area::AppWindows,
        Area::Clipboard,
        Area::Audio,
        Area::Calls,
        Area::Files,
        Area::Notifications,
        Area::Other,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Area::Connection => "connection",
            Area::AppWindows => "app-windows",
            Area::Clipboard => "clipboard",
            Area::Audio => "audio",
            Area::Calls => "calls",
            Area::Files => "files",
            Area::Notifications => "notifications",
            Area::Other => "other",
        }
    }

    /// Its name in English, as reports are read by the maintainers.
    pub fn label(self) -> &'static str {
        match self {
            Area::Connection => "Connection",
            Area::AppWindows => "Mirroring and app windows",
            Area::Clipboard => "Clipboard",
            Area::Audio => "Audio",
            Area::Calls => "Calls and messages",
            Area::Files => "Files and photos",
            Area::Notifications => "Notifications",
            Area::Other => "Something else",
        }
    }

    pub fn parse(id: &str) -> Option<Area> {
        Area::ALL.iter().copied().find(|a| a.id() == id)
    }

    /// The area a crash in `location` or a failing `feature` most likely
    /// belongs to.
    pub fn guess(location: Option<&str>, feature: Option<health::Feature>) -> Area {
        use health::Feature as F;
        if let Some(loc) = location {
            let loc = loc.to_ascii_lowercase();
            for (needle, area) in [
                ("mirror", Area::AppWindows),
                ("clipboard", Area::Clipboard),
                ("audio", Area::Audio),
                ("sound", Area::Audio),
                ("mic", Area::Audio),
                ("call", Area::Calls),
                ("sms", Area::Calls),
                ("transfer", Area::Files),
                ("photo", Area::Files),
                ("storage", Area::Files),
                ("notification", Area::Notifications),
                ("node", Area::Connection),
                ("session", Area::Connection),
                ("iroh", Area::Connection),
            ] {
                if loc.contains(needle) {
                    return area;
                }
            }
        }
        match feature {
            Some(F::Connect | F::Drop | F::Pairing) => Area::Connection,
            Some(F::Mirror | F::AppWindow) => Area::AppWindows,
            Some(F::Clipboard) => Area::Clipboard,
            Some(F::Webcam | F::Mic) => Area::Audio,
            Some(F::Files) => Area::Files,
            _ => Area::Other,
        }
    }
}

/// What a device says about itself: versions and hardware, never names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceFacts {
    /// `PC` or `Phone`.
    pub role: String,
    pub app_version: String,
    /// `stable` or `nightly`.
    pub channel: String,
    /// How it was installed: `setup`, `portable`, `apk`, `deb`...
    pub install: String,
    /// `Windows 11 26100`, `Android 16`.
    pub os: String,
    pub arch: String,
    /// The phone's maker and model, or the PC's graphics.
    pub hardware: Vec<String>,
    /// Other facts as `name: value`, all from fixed lists (power level,
    /// battery saver, permissions on or off).
    pub extra: Vec<(String, String)>,
}

/// The parts the user can leave out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parts {
    pub devices: bool,
    pub checks: bool,
    pub health: bool,
    pub crashes: bool,
    pub log: bool,
}

impl Default for Parts {
    fn default() -> Parts {
        Parts { devices: true, checks: true, health: true, crashes: true, log: true }
    }
}

/// One device's part of a report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DevicePart {
    pub facts: DeviceFacts,
    /// Doctor checks: id and `ok`, `warn` or `fail`.
    pub checks: Vec<(String, String)>,
    pub health: Vec<Day>,
    pub crashes: Vec<CrashRecord>,
    pub log: Vec<String>,
}

/// A report, before it's rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// Random, so the same report sent twice is filed once.
    pub id: String,
    pub area: Area,
    /// The user's own name for the area, with [`Area::Other`].
    pub other_area: String,
    pub text: String,
    pub just_now: bool,
    pub parts: Parts,
    pub devices: Vec<DevicePart>,
    /// What couldn't be included and why: `phone_offline`, `phone_declined`...
    pub missing: Vec<String>,
    /// When it was made, in seconds since 1970.
    pub at: u64,
}

impl Report {
    pub fn new(area: Area, other_area: &str, text: &str, just_now: bool) -> Report {
        Report {
            id: random_id(),
            area,
            other_area: user_text(other_area, 60),
            text: user_text(text, MAX_TEXT),
            just_now,
            parts: Parts::default(),
            devices: Vec::new(),
            missing: Vec::new(),
            at: now(),
        }
    }

    fn area_line(&self) -> String {
        if self.area == Area::Other && !self.other_area.is_empty() {
            format!("{} ({})", self.area.label(), self.other_area)
        } else {
            self.area.label().to_owned()
        }
    }

    /// The report as it's sent, at most [`MAX_BYTES`]: the user's words
    /// first, then each device, logs last so that's what's cut if too long.
    pub fn render(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "NectarLink problem report (format {SCHEMA}, id {})", self.id);
        let _ = writeln!(out, "Area: {}", self.area_line());
        let _ = writeln!(out, "Happened just now: {}", if self.just_now { "yes" } else { "no" });
        out.push_str("\n## What happened\n\n");
        out.push_str(if self.text.is_empty() { "(nothing written)" } else { &self.text });
        out.push('\n');
        let p = self.parts;
        for device in &self.devices {
            let role = &device.facts.role;
            if p.devices {
                let f = &device.facts;
                let _ = write!(out, "\n## About the {}\n\n", role.to_ascii_lowercase());
                if !f.app_version.is_empty() {
                    let _ = writeln!(out, "- NectarLink {} ({}, {})", f.app_version, f.channel, f.install);
                }
                let _ = writeln!(out, "- {}", format!("{} {}", f.os, f.arch).trim());
                for h in &f.hardware {
                    let _ = writeln!(out, "- {h}");
                }
                for (name, value) in &f.extra {
                    let _ = writeln!(out, "- {name}: {value}");
                }
            }
            if p.checks && !device.checks.is_empty() {
                let _ = write!(out, "\n## What the {} checked\n\n", role.to_ascii_lowercase());
                for (id, outcome) in &device.checks {
                    let _ = writeln!(out, "- {id}: {outcome}");
                }
            }
            if p.health && !device.health.is_empty() {
                let _ =
                    write!(out, "\n## How features have been doing on the {}\n\n", role.to_ascii_lowercase());
                for day in device.health.iter().rev().take(7) {
                    let cells: Vec<String> = day
                        .counts
                        .iter()
                        .map(|(feature, codes)| {
                            let codes: Vec<String> =
                                codes.iter().map(|(code, n)| format!("{code} {n}")).collect();
                            format!("{feature} ({})", codes.join(", "))
                        })
                        .collect();
                    let _ = writeln!(out, "- {}: {}", day.day, cells.join("; "));
                }
            }
            if p.crashes && !device.crashes.is_empty() {
                let _ = write!(out, "\n## Crashes on the {}\n\n", role.to_ascii_lowercase());
                for c in device.crashes.iter().take(3) {
                    let _ = writeln!(
                        out,
                        "- {} {} at {}: {} (version {}, {})",
                        c.sig,
                        c.kind,
                        c.location,
                        c.message,
                        c.version,
                        utc(c.at)
                    );
                }
            }
        }
        let missing: Vec<String> = self
            .missing
            .iter()
            .cloned()
            .chain(
                [
                    (!p.devices).then(|| "part_removed:devices".to_owned()),
                    (!p.checks).then(|| "part_removed:checks".to_owned()),
                    (!p.health).then(|| "part_removed:health".to_owned()),
                    (!p.crashes).then(|| "part_removed:crashes".to_owned()),
                    (!p.log).then(|| "part_removed:log".to_owned()),
                ]
                .into_iter()
                .flatten(),
            )
            .collect();
        if !missing.is_empty() {
            let _ = write!(out, "\nNot included: {}\n", missing.join(", "));
        }
        if p.log {
            for device in &self.devices {
                if device.log.is_empty() {
                    continue;
                }
                let _ = write!(out, "\n## Recent activity log ({})\n\n", device.facts.role);
                for line in &device.log {
                    if out.len() + line.len() + 64 > MAX_BYTES {
                        out.push_str("(the rest didn't fit)\n");
                        break;
                    }
                    out.push_str(line);
                    out.push('\n');
                }
            }
        }
        truncate(&mut out, MAX_BYTES);
        out
    }

    /// A new GitHub issue with the area, versions and description filled
    /// in (the report form's fields). The full report doesn't fit in a link;
    /// it's pasted into the form's Report box.
    pub fn issue_url(&self) -> String {
        let versions: Vec<String> = self
            .devices
            .iter()
            .map(|d| format!("{} {} ({})", d.facts.role, d.facts.app_version, d.facts.os))
            .collect();
        let title = format!("[{}] {}", self.area.label(), first_words(&self.text, 60));
        let mut url = url::Url::parse(&format!("{REPO}/issues/new")).expect("fixed URL");
        url.query_pairs_mut()
            .append_pair("template", "app-report.yml")
            .append_pair("title", &title)
            .append_pair("area", &self.area_line())
            .append_pair("versions", &versions.join("; "))
            .append_pair("what", &first_words(&self.text, 1500));
        url.into()
    }
}

/// The log lines a report takes from `files` (newest last): with "just
/// now", everything from the last 5 minutes plus warnings and errors from
/// the last day; otherwise the newest lines. At most [`MAX_LOG_BYTES`].
pub fn log_lines(files: &[PathBuf], just_now: bool, now_secs: u64) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    // The previous file first: it's older.
    for file in files.iter().rev() {
        if let Ok(text) = fs::read_to_string(file) {
            lines.extend(text.lines().map(str::to_owned));
        }
    }
    let keep: Vec<String> = if just_now {
        lines
            .into_iter()
            .filter(|l| match line_time(l) {
                Some(t) => {
                    let age = now_secs.saturating_sub(t);
                    age <= 5 * 60 || (age <= 86_400 && matches!(line_level(l), Some('W' | 'E')))
                }
                None => false,
            })
            .collect()
    } else {
        lines
    };
    // The newest lines that fit.
    let mut size = 0;
    let mut out: Vec<String> = keep
        .into_iter()
        .rev()
        .take_while(|l| {
            size += l.len() + 1;
            size <= MAX_LOG_BYTES
        })
        .collect();
    out.reverse();
    out
}

/// The time at the start of a log line (`2026-11-20T10:01:02.123456Z`).
fn line_time(line: &str) -> Option<u64> {
    let date = line.get(..10)?;
    let time = line.get(11..19)?;
    let mut d = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let mut t = time.split(':').map(|p| p.parse::<i64>().ok());
    let (h, min, s) = (t.next()??, t.next()??, t.next()??);
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    u64::try_from(days * 86_400 + h * 3600 + min * 60 + s).ok()
}

fn line_level(line: &str) -> Option<char> {
    line.split(' ').nth(1).and_then(|l| l.chars().next())
}

/// This device's part: its `facts` and Doctor `checks`, the health days
/// and crash records kept in `diag_dir`, and the log in `log_dir`.
pub fn local_part(
    diag_dir: &Path,
    log_dir: &Path,
    facts: DeviceFacts,
    checks: Vec<(String, String)>,
    just_now: bool,
) -> DevicePart {
    let files = [log_dir.join(super::ring::CURRENT), log_dir.join(super::ring::PREVIOUS)];
    DevicePart {
        facts,
        checks,
        health: health::days(diag_dir),
        crashes: super::crash::records(diag_dir),
        log: log_lines(&files, just_now, now()),
    }
}

/// The area a crash in the last 15 minutes, or the feature that failed most
/// today, points to: what Report a problem starts on.
pub fn guess_area(diag_dir: &Path) -> Area {
    let now = now();
    let crash = super::crash::records(diag_dir).into_iter().find(|r| now.saturating_sub(r.at) < 15 * 60);
    Area::guess(crash.as_ref().map(|r| r.location.as_str()), recent_failure(diag_dir))
}

/// The feature that failed most today, if any did.
fn recent_failure(diag_dir: &Path) -> Option<health::Feature> {
    use health::Feature;
    let today = health::days(diag_dir).pop()?;
    today
        .counts
        .iter()
        .filter_map(|(feature, codes)| {
            let failed: u32 = codes
                .iter()
                .filter(|(code, _)| {
                    !matches!(code.as_str(), "ok" | "lan" | "usb" | "relay" | "bluetooth" | "cancelled")
                })
                .map(|(_, n)| n)
                .sum();
            (failed > 0).then(|| (Feature::parse(feature), failed))
        })
        .filter(|(f, _)| !matches!(f, Some(Feature::Crash | Feature::UncleanExit) | None))
        .max_by_key(|(_, n)| *n)
        .and_then(|(f, _)| f)
}

/// What the user typed, as a report keeps it: at most `max` characters,
/// tabs and line breaks kept, other control characters (which could hide
/// text) dropped, and folders and addresses treated like the log's.
pub fn user_text(text: &str, max: usize) -> String {
    let kept: String = text
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .filter(|c| !matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200b}'..='\u{200f}'))
        .take(max)
        .collect();
    // Line by line, so line breaks survive the log's own cleaning.
    kept.split('\n')
        .map(|line| redact::text(line).trim_end().to_owned())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

/// Whether `text` looks like it has an email address or phone number in
/// it, so the user can be asked whether they mean to make it public.
pub fn looks_like_contact(text: &str) -> bool {
    let email = text.split_whitespace().any(|w| {
        let w = w.trim_matches(|c: char| !c.is_alphanumeric());
        w.split_once('@').is_some_and(|(user, host)| !user.is_empty() && host.contains('.'))
    });
    let mut run = 0;
    let mut phone = false;
    for c in text.chars() {
        if c.is_ascii_digit() {
            run += 1;
            if run >= 9 {
                phone = true;
            }
        } else if !matches!(c, ' ' | '-' | '(' | ')' | '.' | '+') {
            run = 0;
        }
    }
    email || phone
}

fn first_words(text: &str, max: usize) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    if line.chars().count() <= max {
        return line.to_owned();
    }
    let cut: String = line.chars().take(max).collect();
    match cut.rfind(' ') {
        Some(at) if at > max / 2 => format!("{}…", &cut[..at]),
        _ => format!("{cut}…"),
    }
}

fn truncate(text: &mut String, max: usize) {
    if text.len() > max {
        let mut at = max;
        while !text.is_char_boundary(at) {
            at -= 1;
        }
        text.truncate(at);
    }
}

fn utc(secs: u64) -> String {
    let (y, m, d) = super::layer::civil_from_days((secs / 86_400) as i64);
    let rem = secs % 86_400;
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02} UTC", rem / 3600, rem % 3600 / 60)
}

fn random_id() -> String {
    let bytes: [u8; 8] = rand::random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> Report {
        let mut r = Report::new(Area::Files, "", "Sending a big video fails at 90%.\nEvery time.", true);
        r.devices.push(DevicePart {
            facts: DeviceFacts {
                role: "PC".into(),
                app_version: "0.1.6".into(),
                channel: "stable".into(),
                install: "setup".into(),
                os: "Windows 11 26100".into(),
                arch: "x64".into(),
                hardware: vec!["NVIDIA GeForce RTX 3050".into()],
                extra: vec![],
            },
            checks: vec![("firewall".into(), "ok".into())],
            health: vec![],
            crashes: vec![],
            log: vec!["2026-11-20T10:01:02.000000Z W nectarlink_core::transfer: send failed".into()],
        });
        r
    }

    #[test]
    fn a_report_reads_top_to_bottom() {
        let text = report().render();
        assert!(text.starts_with("NectarLink problem report (format 1, id "), "{text}");
        assert!(text.contains("Area: Files and photos"));
        assert!(text.contains("## What happened\n\nSending a big video fails at 90%.\nEvery time."));
        assert!(text.contains("## About the pc\n\n- NectarLink 0.1.6 (stable, setup)"));
        assert!(text.contains("- firewall: ok"));
        assert!(text.contains("## Recent activity log (PC)"));
    }

    #[test]
    fn removed_parts_are_gone_and_say_so() {
        let mut r = report();
        r.parts.log = false;
        r.parts.checks = false;
        let text = r.render();
        assert!(!text.contains("send failed"));
        assert!(!text.contains("firewall"));
        assert!(text.contains("Not included: part_removed:checks, part_removed:log"));
    }

    #[test]
    fn a_report_never_grows_past_its_cap() {
        let mut r = report();
        r.devices[0].log = vec!["x".repeat(1000); 200];
        assert!(r.render().len() <= MAX_BYTES);
    }

    #[test]
    fn hidden_characters_are_dropped_from_what_the_user_types() {
        assert_eq!(user_text("ok\u{202e}evil\u{0007}\r\n second line ", 100), "okevil\n second line");
        assert_eq!(user_text("abcdef", 3), "abc");
    }

    #[test]
    fn contact_details_are_spotted() {
        assert!(looks_like_contact("mail me at alice@example.com"));
        assert!(looks_like_contact("call +44 7700 900123"));
        assert!(!looks_like_contact("version 0.1.6 on Android 16, 90% done"));
    }

    #[test]
    fn the_issue_link_is_encoded_and_short() {
        let mut r = report();
        r.text = format!("a&b=c#d {}", "long ".repeat(1000));
        let url = r.issue_url();
        assert!(url.starts_with("https://github.com/thtbee/nectarlink/issues/new?template=app-report.yml&"));
        assert!(url.contains("a%26b%3Dc%23d"), "{url}");
        assert!(url.len() < 8000, "{}", url.len());
        assert!(!url.contains("send failed"));
    }

    #[test]
    fn areas_round_trip_and_are_guessed() {
        for a in Area::ALL {
            assert_eq!(Area::parse(a.id()), Some(*a));
        }
        assert_eq!(Area::guess(Some("desktop/app/src/mirror.rs:1:1"), None), Area::AppWindows);
        assert_eq!(Area::guess(None, Some(health::Feature::Drop)), Area::Connection);
        assert_eq!(Area::guess(None, None), Area::Other);
    }

    #[test]
    fn just_now_keeps_the_last_minutes_and_the_days_warnings() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("nectarlink.log");
        let now = line_time("2026-11-20T12:00:00Z").unwrap();
        fs::write(
            &file,
            "2026-11-19T10:00:00.0Z W x: too old\n\
             2026-11-20T08:00:00.0Z I x: old info\n\
             2026-11-20T08:00:00.0Z W x: warning today\n\
             2026-11-20T11:58:00.0Z I x: just now\n",
        )
        .unwrap();
        let lines = log_lines(std::slice::from_ref(&file), true, now);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].ends_with("warning today") && lines[1].ends_with("just now"));
        assert_eq!(log_lines(&[file], false, now).len(), 4);
    }
}
