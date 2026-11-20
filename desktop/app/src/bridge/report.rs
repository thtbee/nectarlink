// SPDX-License-Identifier: GPL-3.0-or-later
//! `Report`: QML singleton for Report a problem (`nectarlink_core::diag::report`).
//! It gathers this PC's part off the UI thread, shows the exact text, and
//! opens a new GitHub issue with the short parts filled in; the full text
//! goes on the clipboard to paste into it. Nothing is sent from here.

use std::{
    pin::Pin,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use cxx_qt::Threading;
use cxx_qt_lib::QString;
use nectarlink_core::{
    LinkState,
    diag::{
        crash,
        health::{self, Feature},
        report::{self, Area, DeviceFacts, DevicePart, Parts},
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
    }
}

#[derive(Default)]
pub struct ReportRust {
    preview: QString,
    busy: bool,
}

/// The report made last, for Open on GitHub.
static LAST: Mutex<Option<report::Report>> = Mutex::new(None);
/// Bumped by each `prepare`, so an older one finishing late is dropped.
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl qobject::Report {
    pub fn open(self: Pin<&mut Self>) {
        self.requested();
    }

    pub fn guess_area(&self) -> QString {
        let diag_dir = core_host::host().data_dir.join("diag");
        let now = now();
        let recent_crash = crash::records(&diag_dir).into_iter().find(|r| now.saturating_sub(r.at) < 15 * 60);
        let failing = recent_failure(&diag_dir);
        QString::from(Area::guess(recent_crash.as_ref().map(|r| r.location.as_str()), failing).id())
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
        let spawned = std::thread::Builder::new().name("report".into()).spawn(move || {
            let made = std::panic::catch_unwind(|| build(area, &other, &text, just_now, parts)).ok();
            let rendered = made.as_ref().map(report::Report::render).unwrap_or_default();
            if GENERATION.load(std::sync::atomic::Ordering::SeqCst) != generation {
                return;
            }
            *LAST.lock().unwrap_or_else(|e| e.into_inner()) = made;
            let _ = qt.queue(move |mut object| {
                object.as_mut().set_preview(QString::from(&rendered));
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

/// The feature that failed most in the last day, if any did.
fn recent_failure(diag_dir: &std::path::Path) -> Option<Feature> {
    let today = health::days(diag_dir).pop()?;
    today
        .counts
        .iter()
        .filter_map(|(feature, codes)| {
            let failed: u32 = codes.iter().filter(|(code, _)| !is_success(code)).map(|(_, n)| n).sum();
            (failed > 0).then(|| (Feature::parse(feature), failed))
        })
        .filter(|(f, _)| !matches!(f, Some(Feature::Crash | Feature::UncleanExit) | None))
        .max_by_key(|(_, n)| *n)
        .and_then(|(f, _)| f)
}

fn is_success(code: &str) -> bool {
    matches!(code, "ok" | "lan" | "usb" | "relay" | "cancelled")
}

/// The report with this PC's part and what the PC knows of each paired
/// device (shared when they paired: model and Android version).
fn build(area: Area, other: &str, text: &str, just_now: bool, parts: Parts) -> report::Report {
    let host = core_host::host();
    let diag_dir = host.data_dir.join("diag");
    let log_dir = crate::logging::logs_dir(&host.data_dir);
    let mut made = report::Report::new(area, other, text, just_now);
    made.parts = parts;
    let checks = std::panic::catch_unwind(|| crate::doctor::diagnose(&crate::doctor::gather(), now() as i64))
        .unwrap_or_default()
        .into_iter()
        .map(|c| (c.id, format!("{:?}", c.outcome).to_ascii_lowercase()))
        .collect();
    made.devices.push(report::local_part(&diag_dir, &log_dir, pc_facts(), checks, just_now));
    let paired = core_host::node().and_then(|node| node.paired_devices().ok()).unwrap_or_default();
    if paired.is_empty() {
        made.missing.push("no_paired_device".into());
    }
    for device in paired {
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
