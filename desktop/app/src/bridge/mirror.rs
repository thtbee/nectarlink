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
        /// The PC's mouse and keyboard work on the phone.
        #[qproperty(bool, can_control, cxx_name = "canControl")]
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
        /// A finger on the screen: "down", "move" or "up", at a fraction of
        /// the screen's width and height.
        #[qinvokable]
        fn touch(self: &Mirror, action: &QString, x: f64, y: f64);
        /// The mouse wheel, in notches (positive: down / right).
        #[qinvokable]
        fn scroll(self: &Mirror, x: f64, y: f64, dx: f64, dy: f64);
        /// A key without text ("back", "home", "enter"...).
        #[qinvokable]
        fn key(self: &Mirror, key: &QString);
        /// Typed text.
        #[qinvokable]
        fn text(self: &Mirror, text: &QString);
        /// Types the PC's clipboard text on the phone.
        #[qinvokable]
        fn paste(self: &Mirror);
    }
}

#[derive(Default)]
pub struct MirrorRust {
    device: QString,
    name: QString,
    phase: QString,
    reason: QString,
    can_control: bool,
    shown: Option<DeviceId>,
}

impl cxx_qt::Initialize for qobject::Mirror {
    fn initialize(self: Pin<&mut Self>) {
        super::subscribe(self.qt_thread(), Changes::MIRROR | Changes::CAPABILITIES, Self::refresh);
    }
}

impl qobject::Mirror {
    fn refresh(mut self: Pin<&mut Self>) {
        let Some(device) = self.rust().shown else {
            self.as_mut().set_phase(QString::default());
            return;
        };
        let control = crate::core_host::host().hub.read(|s| {
            s.matrices.get(&device).and_then(|m| m.state("mirroring.control"))
                == Some(nectarlink_core::FeatureState::Available)
        });
        self.as_mut().set_can_control(control);
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

    fn send(&self, input: nectarlink_core::MirrorInput) {
        if let (Some(device), true) = (self.rust().shown, self.rust().can_control) {
            mirror::input(device, input);
        }
    }

    pub fn touch(&self, action: &QString, x: f64, y: f64) {
        use nectarlink_core::TouchAction;
        let action = match String::from(action).as_str() {
            "down" => TouchAction::Down,
            "move" => TouchAction::Move,
            "up" => TouchAction::Up,
            _ => return,
        };
        self.send(nectarlink_core::MirrorInput::Touch { action, x: fraction(x), y: fraction(y) });
    }

    pub fn scroll(&self, x: f64, y: f64, dx: f64, dy: f64) {
        self.send(nectarlink_core::MirrorInput::Scroll {
            x: fraction(x),
            y: fraction(y),
            dx: dx.clamp(-100.0, 100.0) as f32,
            dy: dy.clamp(-100.0, 100.0) as f32,
        });
    }

    pub fn key(&self, key: &QString) {
        self.send(nectarlink_core::MirrorInput::Key { key: String::from(key) });
    }

    pub fn text(&self, text: &QString) {
        let text = String::from(text);
        if !text.is_empty() && text.len() <= nectarlink_core::MIRROR_MAX_TEXT_BYTES {
            self.send(nectarlink_core::MirrorInput::Text { text });
        }
    }

    pub fn paste(&self) {
        if let crate::win::clipboard::Clip::Text(text) = crate::win::clipboard::read() {
            // Long text goes in pieces, each a whole number of characters.
            let mut piece = String::new();
            for c in text.chars() {
                if piece.len() + c.len_utf8() > nectarlink_core::MIRROR_MAX_TEXT_BYTES {
                    self.send(nectarlink_core::MirrorInput::Text { text: std::mem::take(&mut piece) });
                }
                piece.push(c);
            }
            if !piece.is_empty() {
                self.send(nectarlink_core::MirrorInput::Text { text: piece });
            }
        }
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

/// A position on the screen, kept on it.
fn fraction(v: f64) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) as f32 } else { 0.0 }
}
