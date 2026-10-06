// SPDX-License-Identifier: GPL-3.0-or-later
//! `Messages`: a phone's conversations and the open one's messages for the
//! Messages page (JSON arrays QML reads), and what the page asks of them.

use std::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;

use crate::{messages, state::Changes};

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
        /// The phone shown.
        #[qproperty(QString, device)]
        /// "idle", "loading", "ready", "offline", "off", "unsupported" or "failed".
        #[qproperty(QString, status)]
        /// Conversations, newest first (JSON).
        #[qproperty(QString, threads)]
        /// The open conversation's ID ("" for none).
        #[qproperty(QString, thread)]
        /// Its messages, oldest first (JSON).
        #[qproperty(QString, messages)]
        /// Older messages can be loaded.
        #[qproperty(bool, more)]
        #[qproperty(bool, loading_older, cxx_name = "loadingOlder")]
        #[qproperty(bool, sending)]
        type Messages = super::MessagesRust;
    }

    impl cxx_qt::Threading for Messages {}
    impl cxx_qt::Initialize for Messages {}

    unsafe extern "RustQt" {
        /// Shows a phone's messages (reloads when it's the one shown).
        #[qinvokable]
        fn open(self: &Messages, device: &QString);
        #[qinvokable]
        #[cxx_name = "openThread"]
        fn open_thread(self: &Messages, thread: &QString);
        #[qinvokable]
        #[cxx_name = "closeThread"]
        fn close_thread(self: &Messages);
        #[qinvokable]
        #[cxx_name = "loadOlder"]
        fn load_older(self: &Messages);
        /// Sends a text in the open conversation.
        #[qinvokable]
        fn send(self: &Messages, body: &QString);
        /// Sends a text to a number, starting a conversation.
        #[qinvokable]
        #[cxx_name = "sendTo"]
        fn send_to(self: &Messages, to: &QString, body: &QString);
        /// Reads the conversations (and the open one) again.
        #[qinvokable]
        fn refresh(self: &Messages);
        /// Puts text on this PC's clipboard (it isn't sent back to the phone).
        #[qinvokable]
        fn copy(self: &Messages, text: &QString);
    }
}

#[derive(Default)]
pub struct MessagesRust {
    device: QString,
    status: QString,
    threads: QString,
    thread: QString,
    messages: QString,
    more: bool,
    loading_older: bool,
    sending: bool,
    /// What was last shown, to skip refreshes that change nothing.
    last: Option<messages::View>,
}

impl cxx_qt::Initialize for qobject::Messages {
    fn initialize(mut self: Pin<&mut Self>) {
        self.as_mut().set_status(QString::from("idle"));
        self.as_mut().set_threads(QString::from("[]"));
        self.as_mut().set_messages(QString::from("[]"));
        super::subscribe(self.qt_thread(), Changes::MESSAGES, Self::update_view);
        self.update_view();
    }
}

impl qobject::Messages {
    fn update_view(mut self: Pin<&mut Self>) {
        let view = messages::view();
        if self.rust().last.as_ref() == Some(&view) {
            return;
        }
        let last = self.rust().last.clone();
        let differs = |f: fn(&messages::View) -> String| last.as_ref().is_none_or(|l| f(l) != f(&view));
        self.as_mut().set_device(QString::from(&view.device.map(|d| d.to_string()).unwrap_or_default()));
        self.as_mut().set_status(QString::from(view.status.as_str()));
        if differs(|v| v.threads.to_string()) {
            self.as_mut().set_threads(QString::from(&view.threads.to_string()));
        }
        self.as_mut().set_thread(QString::from(view.thread.as_deref().unwrap_or_default()));
        if differs(|v| v.messages.to_string()) {
            self.as_mut().set_messages(QString::from(&view.messages.to_string()));
        }
        self.as_mut().set_more(view.more);
        self.as_mut().set_loading_older(view.loading_older);
        self.as_mut().set_sending(view.sending);
        self.as_mut().rust_mut().last = Some(view);
    }

    pub fn open(&self, device: &QString) {
        if let Some(device) = super::parse_device(device) {
            messages::open_device(device);
        }
    }

    pub fn open_thread(&self, thread: &QString) {
        messages::open_thread(String::from(thread));
    }

    pub fn close_thread(&self) {
        messages::close_thread();
    }

    pub fn load_older(&self) {
        messages::load_older();
    }

    pub fn send(&self, body: &QString) {
        let body = String::from(body);
        if !body.trim().is_empty() {
            messages::send(body);
        }
    }

    pub fn refresh(&self) {
        messages::reload();
    }

    pub fn copy(&self, text: &QString) {
        if let Err(reason) = crate::win::clipboard::write(&String::from(text)) {
            tracing::warn!(reason, "can't copy");
        }
    }

    pub fn send_to(&self, to: &QString, body: &QString) {
        let (to, body) = (String::from(to), String::from(body));
        let to: Vec<String> =
            to.split([',', ';']).map(str::trim).filter(|t| !t.is_empty()).map(Into::into).collect();
        if !to.is_empty() && !body.trim().is_empty() {
            messages::send_to(to, body);
        }
    }
}
