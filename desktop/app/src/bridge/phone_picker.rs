// SPDX-License-Identifier: GPL-3.0-or-later
//! `PhonePicker`: the phone's photos and folders for the picker the Messages
//! page opens to attach a file from the phone (JSON arrays QML reads).

use std::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;

use crate::{phone_picker, state::Changes};

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
        /// The Photos tab's state: "idle", "loading", "ready", "offline", "off",
        /// "unsupported" or "failed".
        #[qproperty(QString, photos_status, cxx_name = "photosStatus")]
        /// The phone's photos, newest first (JSON).
        #[qproperty(QString, photos)]
        #[qproperty(bool, photos_more, cxx_name = "photosMore")]
        #[qproperty(bool, photos_loading_older, cxx_name = "photosLoadingOlder")]
        /// Bumped when thumbnails arrive.
        #[qproperty(i32, thumb_revision, cxx_name = "thumbRevision")]
        /// The Files tab's state, as `photosStatus`.
        #[qproperty(QString, files_status, cxx_name = "filesStatus")]
        /// The folder shown ("" for the phone's root).
        #[qproperty(QString, folder)]
        /// The folders from the root to it (JSON: `[{ "name", "path" }]`).
        #[qproperty(QString, crumbs)]
        /// What it holds (JSON).
        #[qproperty(QString, entries)]
        /// The photo ID or path being brought over ("" for none).
        #[qproperty(QString, busy)]
        /// Why the last pick failed ("" when it didn't).
        #[qproperty(QString, problem)]
        /// Counts the files staged as the attachment.
        #[qproperty(i32, picked)]
        type PhonePicker = super::PhonePickerRust;
    }

    impl cxx_qt::Threading for PhonePicker {}
    impl cxx_qt::Initialize for PhonePicker {}

    unsafe extern "RustQt" {
        /// Opens the picker for a phone, with nothing loaded.
        #[qinvokable]
        fn open(self: &PhonePicker, device: &QString);
        #[qinvokable]
        fn close(self: &PhonePicker);
        /// Lists the photos, unless they're listed already.
        #[qinvokable]
        #[cxx_name = "loadPhotos"]
        fn load_photos(self: &PhonePicker);
        #[qinvokable]
        #[cxx_name = "loadOlderPhotos"]
        fn load_older_photos(self: &PhonePicker);
        /// Lists a folder of the phone ("" for its root).
        #[qinvokable]
        #[cxx_name = "openFolder"]
        fn open_folder(self: &PhonePicker, path: &QString);
        /// The photo's thumbnail as a file URL, or "" when it hasn't come.
        #[qinvokable]
        #[cxx_name = "thumbUrl"]
        fn thumb_url(self: &PhonePicker, id: &QString) -> QString;
        /// Brings a photo over and attaches it.
        #[qinvokable]
        #[cxx_name = "pickPhoto"]
        fn pick_photo(self: &PhonePicker, id: &QString);
        /// Brings a file of the folder shown over and attaches it.
        #[qinvokable]
        #[cxx_name = "pickFile"]
        fn pick_file(self: &PhonePicker, path: &QString);
    }
}

#[derive(Default)]
pub struct PhonePickerRust {
    device: QString,
    photos_status: QString,
    photos: QString,
    photos_more: bool,
    photos_loading_older: bool,
    thumb_revision: i32,
    files_status: QString,
    folder: QString,
    crumbs: QString,
    entries: QString,
    busy: QString,
    problem: QString,
    picked: i32,
    /// What was last shown, to skip refreshes that change nothing.
    last: Option<phone_picker::View>,
}

impl cxx_qt::Initialize for qobject::PhonePicker {
    fn initialize(mut self: Pin<&mut Self>) {
        self.as_mut().set_photos_status(QString::from("idle"));
        self.as_mut().set_files_status(QString::from("idle"));
        self.as_mut().set_photos(QString::from("[]"));
        self.as_mut().set_crumbs(QString::from("[]"));
        self.as_mut().set_entries(QString::from("[]"));
        super::subscribe(self.qt_thread(), Changes::MESSAGES, Self::update_view);
        self.update_view();
    }
}

impl qobject::PhonePicker {
    fn update_view(mut self: Pin<&mut Self>) {
        let view = phone_picker::view();
        if self.rust().last.as_ref() == Some(&view) {
            return;
        }
        let last = self.rust().last.clone();
        // Compared as values: printing both lists to compare them cost more than the update.
        let differs =
            |f: fn(&phone_picker::View) -> &serde_json::Value| last.as_ref().is_none_or(|l| f(l) != f(&view));
        self.as_mut().set_device(QString::from(&view.device.map(|d| d.to_string()).unwrap_or_default()));
        self.as_mut().set_photos_status(QString::from(view.photos_status.as_str()));
        if differs(|v| &v.photos) {
            self.as_mut().set_photos(QString::from(&view.photos.to_string()));
        }
        self.as_mut().set_photos_more(view.photos_more);
        self.as_mut().set_photos_loading_older(view.photos_loading_older);
        self.as_mut().set_thumb_revision(i32::try_from(view.thumb_revision).unwrap_or(i32::MAX));
        self.as_mut().set_files_status(QString::from(view.files_status.as_str()));
        self.as_mut().set_folder(QString::from(&view.folder));
        if differs(|v| &v.crumbs) {
            self.as_mut().set_crumbs(QString::from(&view.crumbs.to_string()));
        }
        if differs(|v| &v.entries) {
            self.as_mut().set_entries(QString::from(&view.entries.to_string()));
        }
        self.as_mut().set_busy(QString::from(&view.busy));
        self.as_mut().set_problem(QString::from(&view.problem));
        self.as_mut().set_picked(i32::try_from(view.picked).unwrap_or(i32::MAX));
        self.as_mut().rust_mut().last = Some(view);
    }

    pub fn open(&self, device: &QString) {
        if let Some(device) = super::parse_device(device) {
            phone_picker::open(device);
        }
    }

    pub fn close(&self) {
        phone_picker::close();
    }

    pub fn load_photos(&self) {
        phone_picker::load_photos();
    }

    pub fn load_older_photos(&self) {
        phone_picker::load_older_photos();
    }

    pub fn open_folder(&self, path: &QString) {
        phone_picker::open_folder(String::from(path));
    }

    pub fn thumb_url(&self, id: &QString) -> QString {
        QString::from(&phone_picker::thumb_url(&String::from(id)))
    }

    pub fn pick_photo(&self, id: &QString) {
        phone_picker::pick_photo(String::from(id));
    }

    pub fn pick_file(&self, path: &QString) {
        phone_picker::pick_file(String::from(path));
    }
}
