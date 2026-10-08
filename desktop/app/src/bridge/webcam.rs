// SPDX-License-Identifier: GPL-3.0-or-later
//! `Webcam`: QML singleton exposing the Windows virtual camera add-on status,
//! webcam preferences (phone, 720p/1080p, mirror), and live stream state.

use std::pin::Pin;

use cxx_qt::Threading;
use cxx_qt_lib::QString;

use crate::{
    state::Changes,
    webcam::{self, Phase},
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
        #[qproperty(bool, addon_registered, cxx_name = "addonRegistered")]
        #[qproperty(bool, addon_busy, cxx_name = "addonBusy")]
        #[qproperty(QString, selected_phone, cxx_name = "selectedPhone")]
        #[qproperty(QString, selected_phone_name, cxx_name = "selectedPhoneName")]
        #[qproperty(i32, height)]
        #[qproperty(bool, mirror)]
        #[qproperty(QString, phase)]
        #[qproperty(QString, active_device, cxx_name = "activeDevice")]
        #[qproperty(QString, status_text, cxx_name = "statusText")]
        type Webcam = super::WebcamRust;
    }

    impl cxx_qt::Threading for Webcam {}
    impl cxx_qt::Initialize for Webcam {}

    unsafe extern "RustQt" {
        #[qinvokable]
        #[cxx_name = "setupAddon"]
        fn setup_addon(self: &Webcam);
        #[qinvokable]
        #[cxx_name = "removeAddon"]
        fn remove_addon(self: &Webcam);
        #[qinvokable]
        #[cxx_name = "selectPhone"]
        fn select_phone(self: &Webcam, device: &QString);
        #[qinvokable]
        #[cxx_name = "setResolution"]
        fn set_resolution(self: &Webcam, height: i32);
        #[qinvokable]
        #[cxx_name = "setMirrorImage"]
        fn set_mirror_image(self: &Webcam, on: bool);
        #[qinvokable]
        fn start(self: &Webcam, device: &QString);
        #[qinvokable]
        fn stop(self: &Webcam);
    }
}

#[derive(Default)]
pub struct WebcamRust {
    addon_registered: bool,
    addon_busy: bool,
    selected_phone: QString,
    selected_phone_name: QString,
    height: i32,
    mirror: bool,
    phase: QString,
    active_device: QString,
    status_text: QString,
}

impl cxx_qt::Initialize for qobject::Webcam {
    fn initialize(self: Pin<&mut Self>) {
        super::subscribe(
            self.qt_thread(),
            Changes::WEBCAM | Changes::DEVICES | Changes::CAPABILITIES,
            Self::refresh,
        );
    }
}

impl qobject::Webcam {
    fn refresh(mut self: Pin<&mut Self>) {
        let hub = &crate::core_host::host().hub;
        self.as_mut().set_addon_registered(webcam::addon_registered());
        self.as_mut().set_addon_busy(webcam::addon_busy());
        self.as_mut().set_height(webcam::height() as i32);
        self.as_mut().set_mirror(webcam::mirror());

        let selected = webcam::selected_phone();
        let selected_id = selected.map(|id| id.to_string()).unwrap_or_default();
        let selected_name = selected.and_then(|id| hub.read(|s| s.name_of(&id))).unwrap_or_default();
        self.as_mut().set_selected_phone(QString::from(&selected_id));
        self.as_mut().set_selected_phone_name(QString::from(&selected_name));

        let (phase_str, active_dev, status) = match webcam::phase() {
            Phase::Idle => ("idle", String::new(), String::new()),
            Phase::Asking { device } => {
                let name = hub.read(|s| s.name_of(&device)).unwrap_or_else(|| "phone".into());
                ("asking", device.to_string(), format!("Starting camera on {name}…"))
            }
            Phase::Streaming { device, width, height, fps } => {
                let name = hub.read(|s| s.name_of(&device)).unwrap_or_else(|| "phone".into());
                (
                    "streaming",
                    device.to_string(),
                    format!("Streaming {width} × {height} · {fps} fps from {name}"),
                )
            }
            Phase::Ended { reason } => ("ended", String::new(), reason),
        };
        self.as_mut().set_phase(QString::from(phase_str));
        self.as_mut().set_active_device(QString::from(&active_dev));
        self.as_mut().set_status_text(QString::from(&status));
    }

    pub fn setup_addon(&self) {
        webcam::setup_addon();
    }

    pub fn remove_addon(&self) {
        webcam::remove_addon();
    }

    pub fn select_phone(&self, device: &QString) {
        if let Some(id) = super::parse_device(device) {
            webcam::set_selected_phone(id);
        }
    }

    pub fn set_resolution(&self, height: i32) {
        webcam::set_height(height.max(0) as u32);
    }

    pub fn set_mirror_image(&self, on: bool) {
        webcam::set_mirror(on);
    }

    pub fn start(&self, device: &QString) {
        webcam::start(super::parse_device(device));
    }

    pub fn stop(&self) {
        webcam::stop();
    }
}
