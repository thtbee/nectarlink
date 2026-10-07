// SPDX-License-Identifier: GPL-3.0-or-later
//! `PhoneCall`: the call in progress on the phone Home and Calls show, the
//! controls QML sends to it, the phone's call log and contacts, and dialing
//! a number from the PC.

use std::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use nectarlink_core::{CallCommand, DeviceId};

use crate::{calls, state::Changes};

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
        /// The phone whose call is shown (set by Home).
        #[qproperty(QString, device, READ, WRITE = set_device, NOTIFY)]
        /// A call is in progress on it.
        #[qproperty(bool, active)]
        /// Who's on the call, as well as the phone knows.
        #[qproperty(QString, caller)]
        /// The number, when there's a name to show above it.
        #[qproperty(QString, number)]
        /// When it was answered (Unix ms), or 0 when the phone didn't say.
        #[qproperty(f64, since)]
        /// The PC can end the call (and change the volume).
        #[qproperty(bool, can_end, cxx_name = "canEnd")]
        /// The phone controls the call for the PC: mute, speaker, hold, keypad.
        #[qproperty(bool, controls)]
        #[qproperty(bool, muted)]
        #[qproperty(bool, speaker)]
        #[qproperty(bool, held)]
        #[qproperty(bool, can_hold, cxx_name = "canHold")]
        /// Call log status: "idle", "loading", "ready", "offline", "off", "unsupported" or "failed".
        #[qproperty(QString, log_status, cxx_name = "logStatus")]
        /// Recent calls, newest first (JSON).
        #[qproperty(QString, call_log, cxx_name = "callLog")]
        /// Older call log entries can be loaded.
        #[qproperty(bool, more_log, cxx_name = "moreLog")]
        #[qproperty(bool, loading_older, cxx_name = "loadingOlder")]
        /// Contacts status: "idle", "loading", "ready", "offline", "off", "unsupported" or "failed".
        #[qproperty(QString, contacts_status, cxx_name = "contactsStatus")]
        /// Contacts, favorites first (JSON).
        #[qproperty(QString, contacts)]
        /// The PC can ask the phone to dial a number.
        #[qproperty(bool, can_dial, cxx_name = "canDial")]
        /// A dial request is on its way to the phone.
        #[qproperty(bool, dialing)]
        type PhoneCall = super::PhoneCallRust;
    }

    impl cxx_qt::Threading for PhoneCall {}
    impl cxx_qt::Initialize for PhoneCall {}

    unsafe extern "RustQt" {
        #[cxx_name = "setDevice"]
        fn set_device(self: Pin<&mut PhoneCall>, device: QString);
        /// Opens a phone's call log and contacts (reloads when it's the one shown).
        #[qinvokable]
        fn open(self: Pin<&mut PhoneCall>, device: &QString);
        /// Loads a phone's contacts in the background if not yet loaded.
        #[qinvokable]
        #[cxx_name = "ensureContacts"]
        fn ensure_contacts(self: &PhoneCall, device: &QString);
        /// Reads the call log and contacts again.
        #[qinvokable]
        fn refresh(self: &PhoneCall);
        #[qinvokable]
        #[cxx_name = "loadOlder"]
        fn load_older(self: &PhoneCall);
        /// Asks the phone to dial `number`.
        #[qinvokable]
        fn dial(self: &PhoneCall, number: &QString);
        /// Puts text on this PC's clipboard (it isn't sent back to the phone).
        #[qinvokable]
        fn copy(self: &PhoneCall, text: &QString);
        /// Hangs up.
        #[qinvokable]
        fn end(self: &PhoneCall);
        #[qinvokable]
        #[cxx_name = "setMute"]
        fn set_mute(self: &PhoneCall, on: bool);
        #[qinvokable]
        #[cxx_name = "setSpeakerOn"]
        fn set_speaker_on(self: &PhoneCall, on: bool);
        #[qinvokable]
        #[cxx_name = "setHold"]
        fn set_hold(self: &PhoneCall, on: bool);
        /// A keypad key: "0"–"9", "*" or "#".
        #[qinvokable]
        fn press(self: &PhoneCall, key: &QString);
        /// The call's volume up (true) or down.
        #[qinvokable]
        fn volume(self: &PhoneCall, up: bool);
    }
}

#[derive(Default)]
pub struct PhoneCallRust {
    device: QString,
    active: bool,
    caller: QString,
    number: QString,
    since: f64,
    can_end: bool,
    controls: bool,
    muted: bool,
    speaker: bool,
    held: bool,
    can_hold: bool,
    log_status: QString,
    call_log: QString,
    more_log: bool,
    loading_older: bool,
    contacts_status: QString,
    contacts: QString,
    can_dial: bool,
    dialing: bool,
    /// The call's ID on the phone, for its commands.
    call: Option<(DeviceId, String)>,
    /// What the Calls page last showed, to skip JSON updates that changed nothing.
    last_view: Option<calls::View>,
}

impl cxx_qt::Initialize for qobject::PhoneCall {
    fn initialize(mut self: Pin<&mut Self>) {
        self.as_mut().set_log_status(QString::from("idle"));
        self.as_mut().set_call_log(QString::from("[]"));
        self.as_mut().set_contacts_status(QString::from("idle"));
        self.as_mut().set_contacts(QString::from("[]"));
        super::subscribe(self.qt_thread(), Changes::CALLS | Changes::CAPABILITIES, Self::refresh_state);
    }
}

impl qobject::PhoneCall {
    pub fn set_device(mut self: Pin<&mut Self>, device: QString) {
        if self.rust().device == device {
            return;
        }
        self.as_mut().rust_mut().device = device;
        self.as_mut().device_changed();
        self.refresh_state();
    }

    pub fn open(mut self: Pin<&mut Self>, device: &QString) {
        if self.rust().device != *device {
            self.as_mut().rust_mut().device = device.clone();
            self.as_mut().device_changed();
        }
        if let Some(id) = super::parse_device(device) {
            calls::open_device(id);
        }
        self.refresh_state();
    }

    pub fn ensure_contacts(&self, device: &QString) {
        if let Some(id) = super::parse_device(device) {
            calls::ensure_contacts(id);
        }
    }

    pub fn refresh(&self) {
        calls::reload();
    }

    pub fn load_older(&self) {
        calls::load_older();
    }

    pub fn dial(&self, number: &QString) {
        if let Some(device) = super::parse_device(&self.rust().device) {
            calls::dial(device, String::from(number));
        }
    }

    pub fn copy(&self, text: &QString) {
        if let Err(reason) = crate::win::clipboard::write(&String::from(text)) {
            tracing::warn!(reason, "can't copy");
        }
    }

    fn refresh_state(mut self: Pin<&mut Self>) {
        let device = super::parse_device(&self.rust().device);
        let call = device.and_then(calls::active);
        let controls = call.as_ref().and_then(|c| c.controls);
        self.as_mut().rust_mut().call = device.zip(call.as_ref().map(|c| c.id.clone()));
        let (caller, number) = match &call {
            Some(call) => {
                let name = call.name.clone().filter(|n| !n.trim().is_empty());
                let number = call.number.clone().filter(|n| !n.trim().is_empty());
                match name {
                    Some(name) => (name, number.unwrap_or_default()),
                    None => (number.unwrap_or_else(|| "Unknown caller".into()), String::new()),
                }
            }
            None => Default::default(),
        };
        self.as_mut().set_caller(QString::from(&caller));
        self.as_mut().set_number(QString::from(&number));
        self.as_mut().set_since(call.as_ref().and_then(|c| c.since).unwrap_or(0) as f64);
        self.as_mut().set_can_end(device.is_some_and(calls::can_control));
        self.as_mut().set_can_dial(device.is_some_and(calls::can_dial));
        self.as_mut().set_controls(controls.is_some());
        let controls = controls.unwrap_or_default();
        self.as_mut().set_muted(controls.muted);
        self.as_mut().set_speaker(controls.speaker);
        self.as_mut().set_held(controls.held);
        self.as_mut().set_can_hold(controls.can_hold);
        self.as_mut().set_active(call.is_some());

        let view = calls::view();
        if self.rust().last_view.as_ref() != Some(&view) {
            let last = self.rust().last_view.clone();
            let differs = |f: fn(&calls::View) -> String| last.as_ref().is_none_or(|l| f(l) != f(&view));
            self.as_mut().set_log_status(QString::from(view.log_status.as_str()));
            if differs(|v| v.call_log.to_string()) {
                self.as_mut().set_call_log(QString::from(&view.call_log.to_string()));
            }
            self.as_mut().set_more_log(view.more_log);
            self.as_mut().set_loading_older(view.loading_older);
            self.as_mut().set_contacts_status(QString::from(view.contacts_status.as_str()));
            if differs(|v| v.contacts.to_string()) {
                self.as_mut().set_contacts(QString::from(&view.contacts.to_string()));
            }
            self.as_mut().set_dialing(view.dialing);
            self.as_mut().rust_mut().last_view = Some(view);
        }
    }

    fn send(&self, command: CallCommand) {
        if let Some((device, call)) = self.rust().call.clone() {
            calls::command(device, call, command);
        }
    }

    pub fn end(&self) {
        self.send(CallCommand::Decline);
    }

    pub fn set_mute(&self, on: bool) {
        self.send(CallCommand::Mute(on));
    }

    pub fn set_speaker_on(&self, on: bool) {
        self.send(CallCommand::Speaker(on));
    }

    pub fn set_hold(&self, on: bool) {
        self.send(CallCommand::Hold(on));
    }

    pub fn press(&self, key: &QString) {
        let key = String::from(key);
        if let [digit] = key.chars().collect::<Vec<_>>()[..]
            && (digit.is_ascii_digit() || digit == '*' || digit == '#')
        {
            self.send(CallCommand::Dtmf(digit));
        }
    }

    pub fn volume(&self, up: bool) {
        self.send(CallCommand::Volume(up));
    }
}
