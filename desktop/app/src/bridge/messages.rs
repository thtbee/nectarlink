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
        /// A number pre-filled in the new-message box ("" for none).
        #[qproperty(QString, compose_to, cxx_name = "composeTo")]
        #[qproperty(QString, compose_name, cxx_name = "composeName")]
        /// Active app filter ("" = All, "sms" = SMS only, or package name).
        #[qproperty(QString, app_filter, cxx_name = "appFilter")]
        /// Filter chips JSON array (`[{ key, label }, ...]`).
        #[qproperty(QString, available_apps, cxx_name = "availableApps")]
        /// True when at least one MessagingStyle chat thread exists for this phone.
        #[qproperty(bool, has_chat_threads, cxx_name = "hasChatThreads")]
        /// True when the open conversation is a MessagingStyle chat thread (`chat:...`).
        #[qproperty(bool, current_is_chat, cxx_name = "currentIsChat")]
        /// Display name of the open conversation's app ("SMS", "WhatsApp", etc.).
        #[qproperty(QString, current_app_name, cxx_name = "currentAppName")]
        /// True when the open conversation supports sending/replying right now.
        #[qproperty(bool, current_can_reply, cxx_name = "currentCanReply")]
        /// True when the open conversation is a group thread.
        #[qproperty(bool, current_is_group, cxx_name = "currentIsGroup")]
        /// True when an image is staged in the composer for MMS.
        #[qproperty(bool, has_attachment, cxx_name = "hasAttachment")]
        /// File URL (`file:///...`) for previewing the staged attachment.
        #[qproperty(QString, attachment_preview, cxx_name = "attachmentPreview")]
        /// File name of the staged attachment.
        #[qproperty(QString, attachment_name, cxx_name = "attachmentName")]
        /// Human-readable size of the staged attachment (e.g. "245 KB").
        #[qproperty(QString, attachment_size, cxx_name = "attachmentSize")]
        /// Non-empty when the staged attachment exceeds the 900 KB MMS limit or is invalid.
        #[qproperty(QString, attachment_error, cxx_name = "attachmentError")]
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
        /// Opens an existing conversation with `number`, or starts a new one.
        #[qinvokable]
        #[cxx_name = "startChat"]
        fn start_chat(self: &Messages, device: &QString, number: &QString, name: &QString);
        #[qinvokable]
        #[cxx_name = "closeThread"]
        fn close_thread(self: &Messages);
        #[qinvokable]
        #[cxx_name = "loadOlder"]
        fn load_older(self: &Messages);
        /// Sends a text or inline chat reply in the open conversation.
        #[qinvokable]
        fn send(self: &Messages, body: &QString);
        /// Sends a text (and optional staged MMS attachment) to a number, starting a conversation.
        #[qinvokable]
        #[cxx_name = "sendTo"]
        fn send_to(self: &Messages, to: &QString, body: &QString);
        /// Reads the conversations (and the open one) again.
        #[qinvokable]
        fn refresh(self: &Messages);
        /// Puts text on this PC's clipboard (it isn't sent back to the phone).
        #[qinvokable]
        fn copy(self: &Messages, text: &QString);
        /// Filters the conversation list by app (`""` = All, `"sms"` = SMS, or package name).
        #[qinvokable]
        #[cxx_name = "filterByApp"]
        fn filter_by_app(self: &Messages, app: &QString);
        /// Deletes a persisted MessagingStyle chat conversation locally.
        #[qinvokable]
        #[cxx_name = "deleteChatThread"]
        fn delete_chat_thread(self: &Messages, thread: &QString);
        /// Stages an image file (from drag-and-drop or file picker) as an MMS attachment.
        #[qinvokable]
        #[cxx_name = "attachFile"]
        fn attach_file(self: &Messages, path_or_url: &QString) -> bool;
        /// Stages an image from the PC clipboard (`Ctrl+V`) as an MMS attachment.
        #[qinvokable]
        #[cxx_name = "pasteClipboardImage"]
        fn paste_clipboard_image(self: &Messages) -> bool;
        /// Clears any staged MMS image attachment.
        #[qinvokable]
        #[cxx_name = "clearAttachment"]
        fn clear_attachment(self: &Messages);
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
    compose_to: QString,
    compose_name: QString,
    app_filter: QString,
    available_apps: QString,
    has_chat_threads: bool,
    current_is_chat: bool,
    current_app_name: QString,
    current_can_reply: bool,
    current_is_group: bool,
    has_attachment: bool,
    attachment_preview: QString,
    attachment_name: QString,
    attachment_size: QString,
    attachment_error: QString,
    /// What was last shown, to skip refreshes that change nothing.
    last: Option<messages::View>,
}

impl cxx_qt::Initialize for qobject::Messages {
    fn initialize(mut self: Pin<&mut Self>) {
        self.as_mut().set_status(QString::from("idle"));
        self.as_mut().set_threads(QString::from("[]"));
        self.as_mut().set_messages(QString::from("[]"));
        self.as_mut().set_available_apps(QString::from("[]"));
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
        self.as_mut().set_compose_to(QString::from(view.compose_to.as_deref().unwrap_or_default()));
        self.as_mut().set_compose_name(QString::from(view.compose_name.as_deref().unwrap_or_default()));
        self.as_mut().set_app_filter(QString::from(&view.app_filter));
        if differs(|v| v.available_apps.to_string()) {
            self.as_mut().set_available_apps(QString::from(&view.available_apps.to_string()));
        }
        self.as_mut().set_has_chat_threads(view.has_chat_threads);
        self.as_mut().set_current_is_chat(view.current_is_chat);
        self.as_mut().set_current_app_name(QString::from(&view.current_app_name));
        self.as_mut().set_current_can_reply(view.current_can_reply);
        self.as_mut().set_current_is_group(view.current_is_group);
        self.as_mut().set_has_attachment(view.has_attachment);
        self.as_mut().set_attachment_preview(QString::from(&view.attachment_preview));
        self.as_mut().set_attachment_name(QString::from(&view.attachment_name));
        self.as_mut().set_attachment_size(QString::from(&view.attachment_size));
        self.as_mut().set_attachment_error(QString::from(&view.attachment_error));
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

    pub fn start_chat(&self, device: &QString, number: &QString, name: &QString) {
        if let Some(device) = super::parse_device(device) {
            messages::start_chat(device, String::from(number), String::from(name));
        }
    }

    pub fn close_thread(&self) {
        messages::close_thread();
    }

    pub fn load_older(&self) {
        messages::load_older();
    }

    pub fn send(&self, body: &QString) {
        messages::send(String::from(body));
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
        if !to.is_empty() {
            messages::send_to(to, body);
        }
    }

    pub fn filter_by_app(&self, app: &QString) {
        messages::set_app_filter(String::from(app));
    }

    pub fn delete_chat_thread(&self, thread: &QString) {
        messages::delete_chat_thread(String::from(thread));
    }

    pub fn attach_file(&self, path_or_url: &QString) -> bool {
        messages::attach_file(String::from(path_or_url))
    }

    pub fn paste_clipboard_image(&self) -> bool {
        messages::paste_clipboard_image()
    }

    pub fn clear_attachment(&self) {
        messages::clear_attachment();
    }
}
