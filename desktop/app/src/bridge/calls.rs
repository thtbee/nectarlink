// SPDX-License-Identifier: GPL-3.0-or-later
//! `PhoneCall`: the call in progress on the phone Home and Calls show, the
//! controls QML sends to it, the phone's call log and contacts, and dialing
//! a number from the PC.

use crate::i18n::tr;
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
        /// A call is in progress or ringing on it.
        #[qproperty(bool, active)]
        /// Whether the call is currently ringing (incoming).
        #[qproperty(bool, ringing)]
        /// Who's on the call, as well as the phone knows.
        #[qproperty(QString, caller)]
        /// The number, when there's a name to show above it.
        #[qproperty(QString, number)]
        /// The app the call is in (WhatsApp...), empty for a phone call (#434).
        #[qproperty(QString, app)]
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
        /// The phone's SIMs as JSON `[{ id, label }]`, when it has more than one.
        #[qproperty(QString, sims)]
        /// The SIM calls go out on (an id from `sims`), or `""`.
        #[qproperty(QString, sim)]
        /// Whether a paired Bluetooth HFP / audio playback endpoint is available for the phone.
        #[qproperty(bool, hfp_available, cxx_name = "hfpAvailable")]
        /// Whether call audio is currently routed through this PC over Bluetooth.
        #[qproperty(bool, hfp_connected, cxx_name = "hfpConnected")]
        /// Human-readable Bluetooth HFP status or pairing guidance.
        #[qproperty(QString, hfp_detail, cxx_name = "hfpDetail")]
        /// The call in progress can sound on this PC (`call.audio`).
        #[qproperty(bool, call_audio_available, cxx_name = "callAudioAvailable")]
        /// The call sounds on this PC (or is moving here).
        #[qproperty(bool, call_audio, cxx_name = "callAudio")]
        /// The PC's microphones, as JSON `[{ id, name }]` (after `refreshCallMics`).
        #[qproperty(QString, call_mics, cxx_name = "callMics")]
        /// The microphone calls record on this PC (an id from `callMics`), or `""`.
        #[qproperty(QString, call_mic, cxx_name = "callMic")]
        /// This PC can test the sound of a call (Windows and macOS).
        #[qproperty(bool, call_audio_test_available, cxx_name = "callAudioTestAvailable")]
        /// What the call audio test shows now (JSON, after `pollCallAudioTest`).
        #[qproperty(QString, call_audio_test, cxx_name = "callAudioTest")]
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
        /// Calls go out on the SIM `sim` (an id from `sims`) from now on.
        #[qinvokable]
        #[cxx_name = "chooseSim"]
        fn choose_sim(self: &PhoneCall, sim: &QString);
        /// Puts text on this PC's clipboard (it isn't sent back to the phone).
        #[qinvokable]
        fn copy(self: &PhoneCall, text: &QString);
        /// Answers an incoming ringing call.
        #[qinvokable]
        fn answer(self: &PhoneCall);
        /// Hangs up or declines.
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
        /// A keypad key: "0".."9", "*" or "#".
        #[qinvokable]
        fn press(self: &PhoneCall, key: &QString);
        /// The call's volume up (true) or down.
        #[qinvokable]
        fn volume(self: &PhoneCall, up: bool);
        /// Connects Bluetooth HFP / audio playback for the active phone.
        #[qinvokable]
        #[cxx_name = "connectPcAudio"]
        fn connect_pc_audio(self: Pin<&mut PhoneCall>);
        /// Opens Windows Bluetooth & devices settings (`ms-settings:bluetooth`).
        #[qinvokable]
        #[cxx_name = "openBluetoothSettings"]
        fn open_bluetooth_settings(self: Pin<&mut PhoneCall>);
        /// Moves the call's sound to this PC (true) or back to the phone.
        #[qinvokable]
        #[cxx_name = "moveCallAudio"]
        fn move_call_audio(self: &PhoneCall, here: bool);
        /// Lists the PC's microphones again (into `callMics` and `callMic`).
        #[qinvokable]
        #[cxx_name = "refreshCallMics"]
        fn refresh_call_mics(self: Pin<&mut PhoneCall>);
        /// Calls record this microphone (an id from `callMics`) from now on.
        #[qinvokable]
        #[cxx_name = "chooseCallMic"]
        fn choose_call_mic(self: Pin<&mut PhoneCall>, id: &QString);
        /// Starts the call audio test: through the phone when it offers one, else on this PC alone.
        #[qinvokable]
        #[cxx_name = "startCallAudioTest"]
        fn start_call_audio_test(self: &PhoneCall);
        /// Ends the call audio test.
        #[qinvokable]
        #[cxx_name = "stopCallAudioTest"]
        fn stop_call_audio_test(self: &PhoneCall);
        /// Reads the test's levels and counters again (into `callAudioTest`).
        #[qinvokable]
        #[cxx_name = "pollCallAudioTest"]
        fn poll_call_audio_test(self: Pin<&mut PhoneCall>);
    }
}

#[derive(Default)]
pub struct PhoneCallRust {
    device: QString,
    active: bool,
    ringing: bool,
    caller: QString,
    number: QString,
    app: QString,
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
    sims: QString,
    sim: QString,
    hfp_available: bool,
    hfp_connected: bool,
    hfp_detail: QString,
    call_audio_available: bool,
    call_audio: bool,
    call_mics: QString,
    call_mic: QString,
    call_audio_test_available: bool,
    call_audio_test: QString,
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
        self.as_mut().set_sims(QString::from("[]"));
        self.as_mut().set_call_mics(QString::from("[]"));
        self.as_mut().set_call_audio_test(QString::from("{}"));
        self.as_mut().set_call_audio_test_available(crate::call_audio_test::supported());
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

    pub fn choose_sim(&self, sim: &QString) {
        if let Some(device) = super::parse_device(&self.rust().device) {
            calls::choose_sim(device, String::from(sim));
        }
    }

    pub fn dial(&self, number: &QString) {
        if let Some(device) = super::parse_device(&self.rust().device) {
            calls::dial(device, String::from(number));
        }
    }

    pub fn copy(&self, text: &QString) {
        match crate::win::clipboard::write(&String::from(text)) {
            Ok(()) => crate::bridge::app::show_message(crate::i18n::tr!("App", "Copied to clipboard.")),
            Err(reason) => {
                tracing::warn!(reason, "can't copy");
                crate::bridge::app::show_message(crate::i18n::tr!("App", "Could not copy to the clipboard."));
            }
        }
    }

    pub fn connect_pc_audio(self: Pin<&mut Self>) {
        let device = super::parse_device(&self.rust().device);
        calls::connect_pc_audio(device);
        self.refresh_state();
    }

    pub fn open_bluetooth_settings(self: Pin<&mut Self>) {
        calls::open_bluetooth_settings();
    }

    fn refresh_state(mut self: Pin<&mut Self>) {
        let preferred_device = super::parse_device(&self.rust().device);
        let active_call = calls::active_or_ringing(preferred_device);
        let call_device = active_call.as_ref().map(|(d, _, _)| *d).or(preferred_device);
        let is_ringing = active_call.as_ref().is_some_and(|(_, _, r)| *r);
        let call = active_call.map(|(_, c, _)| c);
        let controls = call.as_ref().and_then(|c| c.controls);
        self.as_mut().rust_mut().call = call_device.zip(call.as_ref().map(|c| c.id.clone()));
        let (caller, number) = match &call {
            Some(call) => {
                let name = call.name.clone().filter(|n| !n.trim().is_empty());
                let number = call.number.clone().filter(|n| !n.trim().is_empty());
                match name {
                    Some(name) => (name, number.unwrap_or_default()),
                    None => (number.unwrap_or_else(|| tr!("Calls", "Unknown caller")), String::new()),
                }
            }
            None => Default::default(),
        };
        self.as_mut().set_caller(QString::from(&caller));
        self.as_mut().set_number(QString::from(&number));
        let app = call.as_ref().and_then(|c| c.app.clone()).unwrap_or_default();
        self.as_mut().set_app(QString::from(&app));
        self.as_mut().set_since(call.as_ref().and_then(|c| c.since).unwrap_or(0) as f64);
        self.as_mut()
            .set_can_end(call_device.zip(call.as_ref()).is_some_and(|(d, c)| calls::can_control_call(d, c)));
        self.as_mut().set_can_dial(preferred_device.or(call_device).is_some_and(calls::can_dial));
        self.as_mut().set_controls(controls.is_some());
        let controls = controls.unwrap_or_default();
        self.as_mut().set_muted(controls.muted);
        self.as_mut().set_speaker(controls.speaker);
        self.as_mut().set_held(controls.held);
        self.as_mut().set_can_hold(controls.can_hold);
        self.as_mut().set_ringing(is_ringing);
        self.as_mut().set_active(call.is_some());

        let hfp = calls::hfp_status(call_device);
        self.as_mut().set_hfp_available(hfp.available);
        self.as_mut().set_hfp_connected(hfp.connected);
        self.as_mut().set_hfp_detail(QString::from(&hfp.detail));

        let here = call_device.is_some_and(|d| crate::call_audio::phase().device() == Some(d));
        self.as_mut().set_call_audio(here);
        // An app call's sound stays on the phone (#434).
        self.as_mut().set_call_audio_available(
            !is_ringing && app.is_empty() && (here || call_device.is_some_and(crate::call_audio::available)),
        );

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
            if differs(|v| v.sims.to_string()) {
                self.as_mut().set_sims(QString::from(&view.sims.to_string()));
            }
            self.as_mut().set_sim(QString::from(&view.sim));
            self.as_mut().rust_mut().last_view = Some(view);
        }
    }

    fn send(&self, command: CallCommand) {
        if let Some((device, call)) = self.rust().call.clone() {
            calls::command(device, call, command);
        }
    }

    pub fn answer(&self) {
        self.send(CallCommand::Answer);
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

    pub fn refresh_call_mics(mut self: Pin<&mut Self>) {
        let (devices, current) = crate::call_audio::mics();
        let list: Vec<_> =
            devices.iter().map(|d| serde_json::json!({ "id": d.id, "name": d.name })).collect();
        self.as_mut().set_call_mics(QString::from(&serde_json::Value::from(list).to_string()));
        self.as_mut().set_call_mic(QString::from(&current.unwrap_or_default()));
    }

    pub fn choose_call_mic(self: Pin<&mut Self>, id: &QString) {
        crate::call_audio::set_mic(String::from(id));
        self.refresh_call_mics();
    }

    pub fn start_call_audio_test(&self) {
        crate::call_audio_test::start();
    }

    pub fn stop_call_audio_test(&self) {
        crate::call_audio_test::stop();
    }

    pub fn poll_call_audio_test(mut self: Pin<&mut Self>) {
        let view = QString::from(&crate::call_audio_test::view());
        if *self.call_audio_test() != view {
            self.as_mut().set_call_audio_test(view);
        }
    }

    pub fn move_call_audio(&self, here: bool) {
        let Some((device, _)) = self.rust().call.clone() else { return };
        if here {
            crate::call_audio::start(device);
        } else {
            crate::call_audio::stop(device);
        }
    }
}
