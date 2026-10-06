// SPDX-License-Identifier: GPL-3.0-or-later
//! `Mirror`: the phone whose screen the mirror window shows, and how far
//! along it is.

use std::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use nectarlink_core::DeviceId;

use crate::{
    mirror::{self, Phase},
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
        /// The phone shown ("" when the window is closed).
        #[qproperty(QString, device)]
        #[qproperty(QString, name)]
        /// "asking", "showing" or "ended" ("" when closed).
        #[qproperty(QString, phase)]
        /// Why it ended, when there's something to say.
        #[qproperty(QString, reason)]
        type Mirror = super::MirrorRust;
    }

    impl cxx_qt::Threading for Mirror {}
    impl cxx_qt::Initialize for Mirror {}

    unsafe extern "RustQt" {
        /// Asks a phone for its screen and opens the window.
        #[qinvokable]
        fn start(self: Pin<&mut Mirror>, device: &QString);
        /// Closes the window and stops mirroring.
        #[qinvokable]
        fn stop(self: Pin<&mut Mirror>);
    }
}

#[derive(Default)]
pub struct MirrorRust {
    device: QString,
    name: QString,
    phase: QString,
    reason: QString,
    shown: Option<DeviceId>,
}

impl cxx_qt::Initialize for qobject::Mirror {
    fn initialize(self: Pin<&mut Self>) {
        super::subscribe(self.qt_thread(), Changes::MIRROR, Self::refresh);
    }
}

impl qobject::Mirror {
    fn refresh(mut self: Pin<&mut Self>) {
        let Some(device) = self.rust().shown else {
            self.as_mut().set_phase(QString::default());
            return;
        };
        let (phase, reason) = match mirror::phase(&device) {
            None => ("", String::new()),
            Some(Phase::Asking) => ("asking", String::new()),
            Some(Phase::Showing) => ("showing", String::new()),
            Some(Phase::Ended(reason)) => ("ended", reason.unwrap_or_default()),
        };
        self.as_mut().set_reason(QString::from(&reason));
        self.as_mut().set_phase(QString::from(phase));
    }

    pub fn start(mut self: Pin<&mut Self>, device: &QString) {
        let Some(id) = super::parse_device(device) else { return };
        if let Some(previous) = self.rust().shown.filter(|p| *p != id) {
            mirror::stop(previous);
        }
        self.as_mut().rust_mut().shown = Some(id);
        self.as_mut().set_device(device.clone());
        let name = crate::core_host::host().hub.read(|s| s.name_of(&id)).unwrap_or_default();
        self.as_mut().set_name(QString::from(&name));
        mirror::start(id);
        self.refresh();
    }

    pub fn stop(mut self: Pin<&mut Self>) {
        if let Some(id) = self.as_mut().rust_mut().shown.take() {
            mirror::stop(id);
        }
        self.as_mut().set_device(QString::default());
        self.as_mut().set_reason(QString::default());
        self.set_phase(QString::default());
    }
}
