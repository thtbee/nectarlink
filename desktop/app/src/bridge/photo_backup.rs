// SPDX-License-Identifier: GPL-3.0-or-later
//! `PhotoBackup`: QML singleton for Settings › Files & Sync › Photo backup
//! (crate::photo_backup): each paired phone's backup, on or off, its folder,
//! and what it's doing.

use crate::i18n::tr;
use std::{path::PathBuf, pin::Pin};

use cxx_qt::Threading;
use cxx_qt_lib::{QString, QUrl};
use nectarlink_core::{DeviceKind, LinkState};

use crate::{
    core_host,
    photo_backup::{self, Activity},
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
        /// The paired phones and their backups, as JSON: `[{ "id", "name",
        /// "online", "enabled", "videos", "folder", "done", "status", "problem" }]`.
        #[qproperty(QString, phones)]
        type PhotoBackup = super::PhotoBackupRust;
    }

    impl cxx_qt::Threading for PhotoBackup {}
    impl cxx_qt::Initialize for PhotoBackup {}

    unsafe extern "RustQt" {
        #[qinvokable]
        #[cxx_name = "setEnabled"]
        fn set_enabled(self: &PhotoBackup, device: &QString, on: bool);
        #[qinvokable]
        #[cxx_name = "setVideos"]
        fn set_videos(self: &PhotoBackup, device: &QString, on: bool);
        /// Takes a folder picked in a FolderDialog (a `file:` URL).
        #[qinvokable]
        #[cxx_name = "chooseFolder"]
        fn choose_folder(self: &PhotoBackup, device: &QString, url: &QString);
        #[qinvokable]
        #[cxx_name = "backUpNow"]
        fn back_up_now(self: &PhotoBackup, device: &QString);
        #[qinvokable]
        #[cxx_name = "openFolder"]
        fn open_folder(self: &PhotoBackup, device: &QString);
    }
}

#[derive(Default)]
pub struct PhotoBackupRust {
    phones: QString,
}

impl cxx_qt::Initialize for qobject::PhotoBackup {
    fn initialize(self: Pin<&mut Self>) {
        super::subscribe(
            self.qt_thread(),
            Changes::PHOTO_BACKUP | Changes::DEVICES | Changes::STATUS,
            Self::refresh,
        );
    }
}

/// What a phone's backup is doing, in a few words.
fn status(enabled: bool, online: bool, done: usize, activity: &Activity, phone: &str) -> (String, bool) {
    if !enabled {
        return (tr!("PhotoBackup", "Off"), false);
    }
    match activity {
        Activity::Problem(text) => (text.clone(), true),
        Activity::Looking => (tr!("PhotoBackup", "Looking for new photos…"), false),
        Activity::Copying { left } => (tr!("PhotoBackup", "Copying, %1 left", left), false),
        _ if !online => (tr!("PhotoBackup", "Waiting for %1", phone), false),
        Activity::Waiting => (tr!("PhotoBackup", "Waiting for %1", phone), false),
        Activity::WaitingForWifi => (tr!("PhotoBackup", "Waiting for Wi-Fi"), false),
        Activity::Idle if done > 0 => (tr!("PhotoBackup", "Up to date, %1 backed up", done), false),
        Activity::Idle => (tr!("PhotoBackup", "Up to date"), false),
    }
}

impl qobject::PhotoBackup {
    pub fn refresh(mut self: Pin<&mut Self>) {
        let phones: Vec<_> = core_host::host().hub.read(|s| {
            s.devices
                .iter()
                .filter(|d| {
                    matches!(d.info.kind, DeviceKind::Phone | DeviceKind::Tablet | DeviceKind::Unknown)
                })
                .map(|d| (d.id, d.info.name.clone(), matches!(d.link, LinkState::Online { .. })))
                .collect()
        });
        let list: Vec<_> = phones
            .into_iter()
            .map(|(id, name, online)| {
                let summary = photo_backup::summary(id, &name);
                let (text, problem) = status(summary.enabled, online, summary.done, &summary.activity, &name);
                serde_json::json!({
                    "id": id.to_string(),
                    "name": name,
                    "online": online,
                    "enabled": summary.enabled,
                    "videos": summary.videos,
                    "folder": summary.folder.to_string_lossy(),
                    "done": summary.done,
                    "status": text,
                    "problem": problem,
                })
            })
            .collect();
        let json = QString::from(&serde_json::Value::Array(list).to_string());
        if *self.phones() != json {
            self.as_mut().set_phones(json);
        }
    }

    pub fn set_enabled(&self, device: &QString, on: bool) {
        if let Some(id) = super::parse_device(device) {
            photo_backup::set_enabled(id, on);
        }
    }

    pub fn set_videos(&self, device: &QString, on: bool) {
        if let Some(id) = super::parse_device(device) {
            photo_backup::set_videos(id, on);
        }
    }

    pub fn choose_folder(&self, device: &QString, url: &QString) {
        let Some(id) = super::parse_device(device) else { return };
        let raw = QUrl::from(url)
            .to_local_file()
            .map(|p| String::from(&p))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| String::from(url));
        let path = PathBuf::from(raw.trim());
        if path.is_absolute() {
            photo_backup::set_folder(id, path);
        }
    }

    pub fn back_up_now(&self, device: &QString) {
        if let Some(id) = super::parse_device(device) {
            photo_backup::back_up_now(id);
        }
    }

    pub fn open_folder(&self, device: &QString) {
        let Some(id) = super::parse_device(device) else { return };
        let name = core_host::host().hub.read(|s| s.name_of(&id)).unwrap_or_default();
        let folder = photo_backup::summary(id, &name).folder;
        if std::fs::create_dir_all(&folder).is_ok()
            && let Err(e) = crate::win::shell::open_folder(&folder)
        {
            tracing::warn!(error = %e, "can't open the photo backup folder");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_says_what_the_backup_is_doing() {
        assert_eq!(status(false, true, 3, &Activity::Idle, "Pixel").0, "Off");
        assert_eq!(status(true, false, 0, &Activity::Idle, "Pixel").0, "Waiting for Pixel");
        assert_eq!(status(true, true, 0, &Activity::Copying { left: 4 }, "Pixel").0, "Copying, 4 left");
        assert_eq!(status(true, true, 12, &Activity::Idle, "Pixel").0, "Up to date, 12 backed up");
        let (text, problem) = status(true, true, 0, &Activity::Problem("Full".into()), "Pixel");
        assert_eq!((text.as_str(), problem), ("Full", true));
    }
}
