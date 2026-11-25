// SPDX-License-Identifier: GPL-3.0-or-later
//! `Report`: QML singleton for Report a problem (`nectarlink_core::diag::report`).
//! It gathers this PC's part off the UI thread and shows the exact text.
//! Send hands that text to the report service (`diag::send`), which files
//! it; Open on GitHub opens a new issue with the short parts filled in and
//! the full text on the clipboard to paste into it.

use std::{
    pin::Pin,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use cxx_qt::Threading;
use cxx_qt_lib::QString;
use nectarlink_core::{
    DeviceId, LinkState, PeerLog,
    diag::{
        crash, known,
        report::{self, Area, DeviceFacts, DevicePart, Parts},
        send::{self, Answer, Kind, Outgoing, Sent},
        usage,
    },
};

use crate::core_host;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        /// The report exactly as it would go, or "" before one is made.
        #[qproperty(QString, preview)]
        /// Gathering this PC's part.
        #[qproperty(bool, busy)]
        /// Where Send got to: "", `sending`, `filed`, `queued` (the service
        /// files it shortly), `later` (kept, sent at the next chance),
        /// `limited` (too many today) or `failed`.
        #[qproperty(QString, send_state, cxx_name = "sendState")]
        /// The issue the report became, once `filed`.
        #[qproperty(i32, sent_issue, cxx_name = "sentIssue")]
        /// A paired phone was asked for its log: its user has to tap Share on it.
        #[qproperty(bool, phone_log_waiting, cxx_name = "phoneLogWaiting")]
        /// A crash the crash card offers to report: its signature, or "".
        #[qproperty(QString, crash)]
        /// The crash card's Send: "", `sending`, `sent` or `failed`.
        #[qproperty(QString, crash_state, cxx_name = "crashState")]
        /// The version that fixes the card's crash, when a newer one does.
        #[qproperty(QString, crash_fixed_in, cxx_name = "crashFixedIn")]
        type Report = super::ReportRust;
    }

    impl cxx_qt::Threading for Report {}

    unsafe extern "RustQt" {
        /// Opens Report a problem.
        #[qinvokable]
        fn open(self: Pin<&mut Report>);
        #[qsignal]
        fn requested(self: Pin<&mut Report>);

        /// The area a recent crash or failure points to: an `Area` id.
        #[qinvokable]
        #[cxx_name = "guessArea"]
        fn guess_area(self: &Report) -> QString;

        /// Makes the report for `area` (an `Area` id; `other` is the user's
        /// own name for it), `text` and `just_now`, leaving out the parts
        /// whose bit isn't set in `parts` (1 devices, 2 checks, 4 health,
        /// 8 crashes, 16 log). The text lands in `preview`.
        #[qinvokable]
        fn prepare(
            self: Pin<&mut Report>,
            area: &QString,
            other: &QString,
            text: &QString,
            just_now: bool,
            parts: i32,
        );

        /// Whether `text` seems to have an email address or phone number in it.
        #[qinvokable]
        #[cxx_name = "looksLikeContact"]
        fn looks_like_contact(self: &Report, text: &QString) -> bool;

        /// Opens a new GitHub issue for the report made last, with its
        /// area, versions and description filled in. False when there's
        /// none, or the browser didn't open.
        #[qinvokable]
        #[cxx_name = "openOnGithub"]
        fn open_on_github(self: &Report) -> bool;

        /// Sends the report made last to the report service.
        #[qinvokable]
        fn send(self: Pin<&mut Report>);

        /// Opens the issue the sent report became.
        #[qinvokable]
        #[cxx_name = "openSentIssue"]
        fn open_sent_issue(self: &Report) -> bool;

        /// Looks for a crash to offer on the crash card (once per crash
        /// per version, unless turned off).
        #[qinvokable]
        #[cxx_name = "checkCrash"]
        fn check_crash(self: Pin<&mut Report>);

        /// Sends a crash report for the card's crash: the crash, versions,
        /// checks and feature health, never the log.
        #[qinvokable]
        #[cxx_name = "crashSend"]
        fn crash_send(self: Pin<&mut Report>);

        /// Puts the card away for this crash; `look` opens Report a problem.
        #[qinvokable]
        #[cxx_name = "crashDone"]
        fn crash_done(self: Pin<&mut Report>, look: bool);

        /// "Don't ask about crashes".
        #[qinvokable]
        #[cxx_name = "crashOff"]
        fn crash_off(self: Pin<&mut Report>);
    }
}

#[derive(Default)]
pub struct ReportRust {
    preview: QString,
    busy: bool,
    send_state: QString,
    sent_issue: i32,
    phone_log_waiting: bool,
    crash: QString,
    crash_state: QString,
    crash_fixed_in: QString,
}

/// The crash on the crash card.
static CARD: Mutex<Option<nectarlink_core::diag::crash::CrashRecord>> = Mutex::new(None);

/// The report made last, for Send and Open on GitHub.
static LAST: Mutex<Option<report::Report>> = Mutex::new(None);
/// Logs paired devices shared for the report being made, by device, with
/// the nonce they were asked with; `None` lines until they arrive.
static PEER_LOGS: Mutex<Vec<PeerLogEntry>> = Mutex::new(Vec::new());
/// A device asked for its log, the nonce it was asked with, and its lines once they arrive.
type PeerLogEntry = (DeviceId, u64, Option<Vec<String>>);
/// The last report's inputs, to make it again when a device's log arrives.
static INPUTS: Mutex<Option<Inputs>> = Mutex::new(None);
/// Area, the user's name for it, the description, "just now", parts.
type Inputs = (Area, String, String, bool, Parts);
/// The Report object, to show the remade report.
static QT: Mutex<Option<cxx_qt::CxxQtThread<qobject::Report>>> = Mutex::new(None);
/// Bumped by each `prepare`, so an older one finishing late is dropped.
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl qobject::Report {
    pub fn open(mut self: Pin<&mut Self>) {
        self.as_mut().set_send_state(QString::default());
        self.as_mut().set_sent_issue(0);
        self.requested();
    }

    pub fn guess_area(&self) -> QString {
        QString::from(report::guess_area(&diag_dir()).id())
    }

    pub fn prepare(
        mut self: Pin<&mut Self>,
        area: &QString,
        other: &QString,
        text: &QString,
        just_now: bool,
        parts: i32,
    ) {
        let generation = GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        self.as_mut().set_send_state(QString::default());
        let area = Area::parse(&String::from(area)).unwrap_or(Area::Other);
        let (other, text) = (String::from(other), String::from(text));
        let parts = Parts {
            devices: parts & 1 != 0,
            checks: parts & 2 != 0,
            health: parts & 4 != 0,
            crashes: parts & 8 != 0,
            log: parts & 16 != 0,
        };
        self.as_mut().set_busy(true);
        let qt = self.qt_thread();
        *QT.lock().unwrap_or_else(|e| e.into_inner()) = Some(self.qt_thread());
        *INPUTS.lock().unwrap_or_else(|e| e.into_inner()) =
            Some((area, other.clone(), text.clone(), just_now, parts));
        let spawned = std::thread::Builder::new().name("report".into()).spawn(move || {
            let made = std::panic::catch_unwind(|| build(area, &other, &text, just_now, parts, true)).ok();
            let rendered = made.as_ref().map(report::Report::render).unwrap_or_default();
            if GENERATION.load(std::sync::atomic::Ordering::SeqCst) != generation {
                return;
            }
            *LAST.lock().unwrap_or_else(|e| e.into_inner()) = made;
            let waiting = log_waiting();
            let _ = qt.queue(move |mut object| {
                object.as_mut().set_preview(QString::from(&rendered));
                object.as_mut().set_phone_log_waiting(waiting);
                object.set_busy(false);
            });
        });
        if spawned.is_err() {
            self.set_busy(false);
        }
    }

    pub fn looks_like_contact(&self, text: &QString) -> bool {
        report::looks_like_contact(&String::from(text))
    }

    pub fn open_on_github(&self) -> bool {
        let Some(url) =
            LAST.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(report::Report::issue_url)
        else {
            return false;
        };
        match crate::win::shell::open_url(&url) {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!(error = %e, "can't open the new issue page");
                false
            }
        }
    }
}

impl qobject::Report {
    pub fn send(mut self: Pin<&mut Self>) {
        let Some(made) = LAST.lock().unwrap_or_else(|e| e.into_inner()).clone() else { return };
        self.as_mut().set_send_state(QString::from("sending"));
        let qt = self.qt_thread();
        let spawned = std::thread::Builder::new().name("report-send".into()).spawn(move || {
            let outgoing = Outgoing::new(&made, Kind::Person, None, now());
            let (state, issue) = match deliver(&outgoing, true) {
                Some(Answer::Filed { issue }) => ("filed", issue),
                Some(Answer::Queued) => ("queued", 0),
                Some(Answer::Limited) => ("limited", 0),
                Some(Answer::Refused) => ("failed", 0),
                Some(Answer::Later) | None => {
                    send::keep_pending(&diag_dir(), &outgoing);
                    ("later", 0)
                }
            };
            let _ = qt.queue(move |mut object| {
                object.as_mut().set_sent_issue(i32::try_from(issue).unwrap_or(0));
                object.set_send_state(QString::from(state));
            });
        });
        if spawned.is_err() {
            self.set_send_state(QString::from("failed"));
        }
    }

    pub fn open_sent_issue(&self) -> bool {
        let issue = *self.sent_issue();
        issue > 0 && crate::win::shell::open_url(&send::issue_url(issue as u64)).is_ok()
    }
}

impl qobject::Report {
    pub fn check_crash(mut self: Pin<&mut Self>) {
        let found = crash::for_card(&diag_dir(), env!("CARGO_PKG_VERSION"), now());
        let sig = found.as_ref().map(|r| r.sig.clone()).unwrap_or_default();
        *CARD.lock().unwrap_or_else(|e| e.into_inner()) = found;
        self.as_mut().set_crash_fixed_in(QString::default());
        self.as_mut().set_crash(QString::from(&sig));
        if sig.is_empty() {
            return;
        }
        // Already fixed in a newer version? Then the card offers the update.
        let qt = self.qt_thread();
        let _ = std::thread::Builder::new().name("known-crashes".into()).spawn(move || {
            let Ok(list) = crate::win::http::get(known::KNOWN_URL, known::MAX_BYTES) else { return };
            if let Some(version) = known::fixed_in(&list, &sig, env!("CARGO_PKG_VERSION")) {
                let _ = qt.queue(move |object| object.set_crash_fixed_in(QString::from(&version)));
            }
        });
    }

    pub fn crash_send(mut self: Pin<&mut Self>) {
        let Some(record) = CARD.lock().unwrap_or_else(|e| e.into_inner()).clone() else { return };
        self.as_mut().set_crash_state(QString::from("sending"));
        let qt = self.qt_thread();
        let spawned = std::thread::Builder::new().name("crash-report".into()).spawn(move || {
            let area = Area::guess(Some(&record.location), None);
            let parts = Parts { devices: true, checks: true, health: true, crashes: true, log: false };
            let state = std::panic::catch_unwind(|| build(area, "", "", false, parts, false))
                .ok()
                .map(|made| Outgoing::new(&made, Kind::Crash, Some(&record.sig), now()))
                .map_or("failed", |outgoing| match deliver(&outgoing, false) {
                    Some(Answer::Filed { .. } | Answer::Queued) => "sent",
                    Some(Answer::Later) | None => {
                        send::keep_pending(&diag_dir(), &outgoing);
                        "sent"
                    }
                    _ => "failed",
                });
            crash::card_shown(&diag_dir(), &record);
            let _ = qt.queue(move |object| object.set_crash_state(QString::from(state)));
        });
        if spawned.is_err() {
            self.set_crash_state(QString::from("failed"));
        }
    }

    pub fn crash_done(mut self: Pin<&mut Self>, look: bool) {
        if let Some(record) = CARD.lock().unwrap_or_else(|e| e.into_inner()).take() {
            crash::card_shown(&diag_dir(), &record);
        }
        self.as_mut().set_crash(QString::default());
        self.as_mut().set_crash_state(QString::default());
        if look {
            self.open();
        }
    }

    pub fn crash_off(mut self: Pin<&mut Self>) {
        crash::card_off(&diag_dir());
        CARD.lock().unwrap_or_else(|e| e.into_inner()).take();
        self.as_mut().set_crash(QString::default());
        self.set_crash_state(QString::default());
    }
}

/// Posts `outgoing` and notes what became of it; with `follow`, its issue
/// is checked for replies later. `None` when the service couldn't be
/// reached.
fn deliver(outgoing: &Outgoing, follow: bool) -> Option<Answer> {
    let url = format!("{}/v1/report", send::REPORT_SERVICE);
    let answer = match crate::win::http::post_json(
        &url,
        &[("X-Proof", &outgoing.proof)],
        outgoing.body.as_bytes(),
        16 * 1024,
    ) {
        Ok((status, body)) => Answer::read(status, &body),
        Err(e) => {
            tracing::info!(error = e, "the report service can't be reached");
            return None;
        }
    };
    tracing::info!(state = ?answer, "report sent");
    let issue = match answer {
        Answer::Filed { issue } => Some(issue),
        Answer::Queued => None,
        _ => return Some(answer),
    };
    if !follow {
        return Some(answer);
    }
    send::remember(
        &diag_dir(),
        Sent { rid: outgoing.rid.clone(), issue, at: outgoing.at, comments: 0, closed: false },
        now(),
    );
    Some(answer)
}

/// At an update check: sends the daily count and the report that couldn't
/// go before, and asks which issue queued ones became.
pub fn follow_up() {
    daily_count();
    let dir = diag_dir();
    if let Some(pending) = send::pending(&dir) {
        match deliver(&pending, true) {
            Some(Answer::Later) | None => {}
            Some(_) => send::clear_pending(&dir),
        }
    }
    // Replies and closed reports: one GET per followed report, numbers only.
    for followed in send::sent(&dir, now()) {
        let Some(issue) = followed.issue else { continue };
        if let Ok((status, body)) = crate::win::http::get_status(&send::issue_api_url(issue), 64 * 1024)
            && let Some(notice) = send::check_issue(&dir, &followed.rid, status, &body, now())
        {
            crate::report_requests::notice(notice);
        }
    }
    for queued in send::sent(&dir, now()).into_iter().filter(|s| s.issue.is_none()) {
        let url = format!("{}/v1/report/{}", send::REPORT_SERVICE, queued.rid);
        match crate::win::http::get_status(&url, 4096).map(|(status, body)| Answer::read(status, &body)) {
            Ok(Answer::Filed { issue }) => {
                send::remember(&dir, Sent { issue: Some(issue), ..queued }, now());
            }
            Ok(Answer::Refused) => {
                // Unknown to the service: it waited too long and was dropped.
                send::forget(&dir, &queued.rid);
            }
            _ => {}
        }
    }
}

/// Whether the daily count goes: Settings, on by default (#534).
static DAILY_COUNT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub fn set_daily_count(on: bool) {
    DAILY_COUNT.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Whether the count may go: the setting, and `NECTARLINK_NO_DAILY_COUNT`
/// for packagers who want it off for everyone.
fn daily_count_allowed(setting: bool, env: Option<std::ffi::OsString>) -> bool {
    setting && env.is_none_or(|v| v.is_empty() || v == "0")
}

/// Sends today's count (`diag::usage`) if it hasn't gone yet.
fn daily_count() {
    // Builds from source and test runs aren't installs.
    if cfg!(debug_assertions) {
        return;
    }
    if !daily_count_allowed(
        DAILY_COUNT.load(std::sync::atomic::Ordering::Relaxed),
        std::env::var_os("NECTARLINK_NO_DAILY_COUNT"),
    ) {
        return;
    }
    let dir = diag_dir();
    let facts = pc_facts();
    let install = usage::Install {
        app: facts.app_version,
        channel: facts.channel,
        platform: std::env::consts::OS.into(),
        os: facts.os,
        hardware: String::new(),
    };
    let Some((body, proof)) = usage::due(&dir, &install, now()) else { return };
    let url = format!("{}/v1/ping", send::REPORT_SERVICE);
    match crate::win::http::post_json(&url, &[("X-Proof", &proof)], body.as_bytes(), 4096) {
        // Counted, or refused for good: either way, not again today.
        Ok((status, _)) if status < 500 && status != 429 => usage::sent(&dir, now()),
        Ok((status, _)) => tracing::info!(code = status, "the daily count wasn't taken"),
        Err(e) => tracing::info!(error = e, "the daily count couldn't be sent"),
    }
}

fn diag_dir() -> std::path::PathBuf {
    core_host::host().data_dir.join("diag")
}

/// The report with this PC's part and what the PC knows of each paired
/// device (shared when they paired: model and Android version).
fn build(area: Area, other: &str, text: &str, just_now: bool, parts: Parts, ask: bool) -> report::Report {
    let host = core_host::host();
    let diag_dir = host.data_dir.join("diag");
    let log_dir = crate::logging::logs_dir(&host.data_dir);
    let mut made = report::Report::new(area, other, text, just_now);
    made.parts = parts;
    let checks = doctor_checks();
    made.devices.push(report::local_part(&diag_dir, &log_dir, pc_facts(), checks, just_now));
    let node = core_host::node();
    let paired = node.as_ref().and_then(|node| node.paired_devices().ok()).unwrap_or_default();
    if paired.is_empty() {
        made.missing.push("no_paired_device".into());
    }
    if ask {
        PEER_LOGS.lock().unwrap_or_else(|e| e.into_inner()).clear();
        ASKED.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
    for device in paired {
        let online = matches!(device.link, LinkState::Online { .. });
        if ask
            && online
            && let Some(node) = node.clone()
        {
            let asked = core_host::host()
                .runtime
                .block_on(async move { node.diag_ask(device.id, parts.log, just_now).await });
            match asked {
                Ok(part) => {
                    if part.log == PeerLog::Asking {
                        PEER_LOGS
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push((device.id, part.nonce, None));
                    }
                    ASKED.lock().unwrap_or_else(|e| e.into_inner()).push((device.id, part));
                }
                Err(e) => tracing::info!(error = %e, "a paired device didn't send its part"),
            }
        }
        let asked =
            ASKED.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(id, _)| *id == device.id).cloned();
        if let Some((_, peer)) = asked {
            let mut part = peer.part;
            part.facts.role = "Phone".into();
            let logs = PEER_LOGS.lock().unwrap_or_else(|e| e.into_inner());
            match logs.iter().find(|(id, _, _)| *id == device.id) {
                Some((_, _, Some(lines))) => part.log = lines.clone(),
                Some((_, _, None)) if parts.log => {
                    made.missing.push("phone_log_waiting_for_share_on_the_phone".into())
                }
                _ => {}
            }
            made.devices.push(part);
            continue;
        }
        let link = match device.link {
            LinkState::Online { path, .. } => format!("online ({path:?})").to_ascii_lowercase(),
            LinkState::Connecting => "connecting".into(),
            LinkState::Offline { .. } => "offline".into(),
        };
        let facts = DeviceFacts {
            role: "Phone".into(),
            app_version: String::new(),
            channel: String::new(),
            install: String::new(),
            os: format!("{} {}", device.info.os, device.info.os_ver),
            arch: String::new(),
            hardware: device.info.model.into_iter().collect(),
            extra: vec![("link".into(), link)],
        };
        made.devices.push(DevicePart { facts, ..DevicePart::default() });
    }
    made
}

/// The parts paired devices sent for the report being made.
static ASKED: Mutex<Vec<(DeviceId, nectarlink_core::PeerPart)>> = Mutex::new(Vec::new());

/// A paired device's log arrived for the report being made (`lines`), or
/// its user said no: makes the report again with it.
pub fn peer_log_arrived(device: DeviceId, nonce: u64, lines: Option<Vec<String>>) {
    {
        let mut logs = PEER_LOGS.lock().unwrap_or_else(|e| e.into_inner());
        let Some(entry) = logs.iter_mut().find(|(id, n, _)| *id == device && *n == nonce) else { return };
        match lines {
            Some(lines) => entry.2 = Some(lines),
            None => {
                logs.retain(|(id, _, _)| *id != device);
            }
        }
    }
    let Some((area, other, text, just_now, parts)) = INPUTS.lock().unwrap_or_else(|e| e.into_inner()).clone()
    else {
        return;
    };
    let Some(qt) = QT.lock().unwrap_or_else(|e| e.into_inner()).clone() else { return };
    let generation = GENERATION.load(std::sync::atomic::Ordering::SeqCst);
    let _ = std::thread::Builder::new().name("report".into()).spawn(move || {
        let Ok(made) = std::panic::catch_unwind(|| build(area, &other, &text, just_now, parts, false)) else {
            return;
        };
        if GENERATION.load(std::sync::atomic::Ordering::SeqCst) != generation {
            return;
        }
        let rendered = made.render();
        *LAST.lock().unwrap_or_else(|e| e.into_inner()) = Some(made);
        let waiting = log_waiting();
        let _ = qt.queue(move |mut object| {
            object.as_mut().set_preview(QString::from(&rendered));
            object.set_phone_log_waiting(waiting);
        });
    });
}

/// Whether a paired device's log is still waiting for its user.
fn log_waiting() -> bool {
    PEER_LOGS.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|(_, _, lines)| lines.is_none())
}

/// What this PC says about itself in a phone's report, with its checks.
pub fn own_facts() -> (DeviceFacts, Vec<(String, String)>) {
    (pc_facts(), doctor_checks())
}

fn doctor_checks() -> Vec<(String, String)> {
    std::panic::catch_unwind(|| crate::doctor::diagnose(&crate::doctor::gather(), now() as i64))
        .unwrap_or_default()
        .into_iter()
        .map(|c| (c.id, format!("{:?}", c.outcome).to_ascii_lowercase()))
        .collect()
}

/// What this PC says about itself.
fn pc_facts() -> DeviceFacts {
    let version = env!("CARGO_PKG_VERSION");
    let channel = match crate::version::channel_of(version) {
        crate::version::Channel::Nightly => "nightly",
        crate::version::Channel::Stable => "stable",
    };
    let install = if cfg!(target_os = "linux") {
        "linux"
    } else if crate::updater::can_update() {
        "setup"
    } else {
        "portable or built from source"
    };
    let os = match std::env::consts::OS {
        "windows" => format!("Windows {}", crate::win::os_version()),
        "macos" => format!("macOS {}", crate::win::os_version()),
        "linux" => format!("Linux {}", crate::win::os_version()),
        other => other.to_owned(),
    };
    DeviceFacts {
        role: "PC".into(),
        app_version: version.into(),
        channel: channel.into(),
        install: install.into(),
        os,
        arch: std::env::consts::ARCH.into(),
        hardware: Vec::new(),
        extra: Vec::new(),
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::daily_count_allowed;

    #[test]
    fn the_daily_count_follows_the_setting_and_the_packager_switch() {
        assert!(daily_count_allowed(true, None));
        assert!(!daily_count_allowed(false, None));
        assert!(!daily_count_allowed(true, Some("1".into())));
        assert!(!daily_count_allowed(true, Some("yes".into())));
        // Set but empty or "0" leaves it to the setting.
        assert!(daily_count_allowed(true, Some("".into())));
        assert!(daily_count_allowed(true, Some("0".into())));
        assert!(!daily_count_allowed(false, Some("0".into())));
    }
}
