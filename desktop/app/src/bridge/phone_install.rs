// SPDX-License-Identifier: GPL-3.0-or-later
//! `PhoneInstall`: QML singleton for Settings › Install on a phone
//! (crate::phone_install): the USB tools, the phones on a cable, and an
//! install in progress, as one JSON object the page reads.

use std::pin::Pin;

use cxx_qt::Threading;
use cxx_qt_lib::QString;
use serde_json::{Value, json};

use crate::{
    i18n::tr,
    phone_install::{self, Job, PhoneState, Problem, Tools},
    state::Changes,
};

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
        /// Everything the page shows, as JSON (see `PhoneInstallRust::json`).
        #[qproperty(QString, state)]
        type PhoneInstall = super::PhoneInstallRust;
    }

    impl cxx_qt::Threading for PhoneInstall {}
    impl cxx_qt::Initialize for PhoneInstall {}

    unsafe extern "RustQt" {
        /// The page opened or closed: phones are only looked for while it's open.
        #[qinvokable]
        fn watch(self: &PhoneInstall, on: bool);
        /// Downloads Google's USB tools (platform-tools).
        #[qinvokable]
        #[cxx_name = "getTools"]
        fn get_tools(self: &PhoneInstall);
        #[qinvokable]
        fn install(self: &PhoneInstall, serial: &QString);
        /// Removes a differently signed build of the app, then installs this one.
        #[qinvokable]
        fn replace(self: &PhoneInstall, serial: &QString);
        #[qinvokable]
        #[cxx_name = "openOnPhone"]
        fn open_on_phone(self: &PhoneInstall, serial: &QString);
        /// Clears a finished install's result.
        #[qinvokable]
        fn dismiss(self: &PhoneInstall);
        #[qinvokable]
        fn refresh(self: Pin<&mut PhoneInstall>);
    }
}

#[derive(Default)]
pub struct PhoneInstallRust {
    state: QString,
}

impl cxx_qt::Initialize for qobject::PhoneInstall {
    fn initialize(mut self: Pin<&mut Self>) {
        super::subscribe(self.qt_thread(), Changes::PHONE_INSTALL, Self::refresh);
        self.as_mut().refresh();
    }
}

impl qobject::PhoneInstall {
    pub fn refresh(self: Pin<&mut Self>) {
        let state = json().to_string();
        self.set_state(QString::from(&state));
    }

    pub fn watch(&self, on: bool) {
        phone_install::watch(on);
    }

    pub fn get_tools(&self) {
        phone_install::get_tools();
    }

    pub fn install(&self, serial: &QString) {
        phone_install::install(String::from(serial));
    }

    pub fn replace(&self, serial: &QString) {
        phone_install::replace(String::from(serial));
    }

    pub fn open_on_phone(&self, serial: &QString) {
        let serial = String::from(serial);
        std::thread::spawn(move || phone_install::open_on_phone(&serial));
    }

    pub fn dismiss(&self) {
        phone_install::dismiss();
    }
}

fn platform() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

/// The page's state: `{ usbOff, platform, server, driverMissing, latest, tools, phones, job }`.
fn json() -> Value {
    let status = phone_install::status();
    let tools = match &status.tools {
        Tools::Unknown => json!({ "kind": "unknown" }),
        Tools::Ready { own } => json!({ "kind": "ready", "own": own }),
        Tools::Unavailable => json!({ "kind": "unavailable" }),
        Tools::Missing => json!({ "kind": "missing" }),
        Tools::Getting { done, total } => json!({ "kind": "getting", "done": done, "total": total }),
        Tools::Failed(e) => json!({ "kind": "failed", "error": e }),
    };
    let latest = status.latest.clone().unwrap_or_default();
    let phones: Vec<Value> = status
        .phones
        .iter()
        .map(|p| {
            let state = match p.state {
                PhoneState::Ready => "ready",
                PhoneState::Unauthorized => "unauthorized",
                PhoneState::NoPermissions => "noPermissions",
                PhoneState::Offline => "offline",
            };
            let installed = p.installed.clone().unwrap_or_default();
            json!({
                "serial": p.serial,
                "state": state,
                "name": p.name,
                "android": p.android.clone().unwrap_or_default(),
                "known": p.known,
                "installed": installed,
                "outdated": !installed.is_empty() && !latest.is_empty()
                    && phone_install::is_outdated(&installed, &latest),
                "tooOld": p.sdk.is_some_and(|sdk| sdk < 26),
            })
        })
        .collect();
    let job = match &status.job {
        Job::Idle => json!({ "kind": "idle" }),
        Job::Downloading { serial, done, total } => {
            json!({ "kind": "downloading", "serial": serial, "done": done, "total": total })
        }
        Job::Sending { serial, done, total } => {
            json!({ "kind": "sending", "serial": serial, "done": done, "total": total })
        }
        Job::Installing { serial } => json!({ "kind": "installing", "serial": serial }),
        Job::Uninstalling { serial } => json!({ "kind": "uninstalling", "serial": serial }),
        Job::Done { serial, version } => json!({ "kind": "done", "serial": serial, "version": version }),
        Job::Failed { serial, problem } => {
            json!({ "kind": "failed", "serial": serial, "problem": problem_json(problem) })
        }
    };
    json!({
        "usbOff": status.usb_off,
        "platform": platform(),
        "server": status.server,
        "driverMissing": status.driver_missing,
        "latest": latest,
        "tools": tools,
        "phones": phones,
        "job": job,
    })
}

/// A problem as the page shows it: `{ code, title, text, action }`, where
/// `action` is what the button next to it does ("retry", "replace", or "").
fn problem_json(problem: &Problem) -> Value {
    let (code, title, text, action) = match problem {
        Problem::Download(e) => (
            "download",
            tr!("PhoneInstall", "Couldn't download the phone app"),
            tr!("PhoneInstall", "Check this PC's internet connection and try again. (%1)", e),
            "retry",
        ),
        Problem::Damaged => (
            "damaged",
            tr!("PhoneInstall", "The download was damaged"),
            tr!("PhoneInstall", "It didn't match its checksum, so it wasn't installed. Try again."),
            "retry",
        ),
        Problem::Lost(_) => (
            "lost",
            tr!("PhoneInstall", "The phone stopped answering"),
            tr!("PhoneInstall", "Keep it plugged in and unlocked until the install is done, then try again."),
            "retry",
        ),
        Problem::OldAndroid => (
            "oldAndroid",
            tr!("PhoneInstall", "This phone's Android is too old"),
            tr!("PhoneInstall", "NectarLink needs Android 8.0 or newer."),
            "",
        ),
        Problem::Restricted => (
            "restricted",
            tr!("PhoneInstall", "The phone blocks installs over USB"),
            tr!(
                "PhoneInstall",
                "In Developer options, turn on Install via USB, then try again. On Xiaomi, Redmi and \
                 POCO phones it may ask you to sign in to a Xiaomi account first."
            ),
            "retry",
        ),
        Problem::OtherBuild => (
            "otherBuild",
            tr!("PhoneInstall", "Another build of NectarLink is on this phone"),
            tr!(
                "PhoneInstall",
                "It was signed differently (a test build, or one from another store), so Android \
                 won't update it. Replacing it removes it with its data, including the PCs it's \
                 paired with, then installs this one."
            ),
            "replace",
        ),
        Problem::Newer => (
            "newer",
            tr!("PhoneInstall", "The phone already has a newer version"),
            tr!("PhoneInstall", "There's nothing to install. Open NectarLink on the phone to pair it."),
            "",
        ),
        Problem::NoSpace => (
            "noSpace",
            tr!("PhoneInstall", "The phone is out of space"),
            tr!("PhoneInstall", "Free up some storage on the phone, then try again."),
            "retry",
        ),
        Problem::Declined => (
            "declined",
            tr!("PhoneInstall", "The install was stopped on the phone"),
            tr!(
                "PhoneInstall",
                "The phone asked whether to allow it and the answer was no, or Google Play Protect \
                 stopped it. Try again and allow it on the phone. If Play Protect stops it, turn \
                 off Verify apps over USB in Developer options for the install."
            ),
            "retry",
        ),
        Problem::Other(reason) => (
            "other",
            tr!("PhoneInstall", "The phone didn't install it"),
            tr!("PhoneInstall", "Android said: %1", reason),
            "retry",
        ),
    };
    json!({ "code": code, "title": title, "text": text, "action": action })
}
