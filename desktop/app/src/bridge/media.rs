// SPDX-License-Identifier: GPL-3.0-or-later
//! `MediaList`: what plays on paired devices as a Qt list model, plus the
//! commands QML sends to those players.

use std::{pin::Pin, time::UNIX_EPOCH};

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QList, QModelIndex, QString, QUrl, QVariant};
use nectarlink_core::MediaAction;

use super::{Edit, diff};
use crate::{
    core_host, media,
    state::{Changes, PlayerView},
};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!(<QtCore/QAbstractListModel>);
        type QAbstractListModel;

        include!("cxx-qt-lib/qmodelindex.h");
        type QModelIndex = cxx_qt_lib::QModelIndex;
        include!("cxx-qt-lib/qvariant.h");
        type QVariant = cxx_qt_lib::QVariant;
        include!("cxx-qt-lib/qhash.h");
        type QHash_i32_QByteArray = cxx_qt_lib::QHash<cxx_qt_lib::QHashPair_i32_QByteArray>;
        include!("cxx-qt-lib/qlist.h");
        type QList_i32 = cxx_qt_lib::QList<i32>;
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[base = QAbstractListModel]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(i32, count)]
        /// Bumped when players come or go, for bindings on [`first_for`].
        #[qproperty(i32, revision)]
        #[qproperty(QString, active_art, cxx_name = "activeArt")]
        #[qproperty(QString, active_colors, cxx_name = "activeColors")]
        type MediaList = super::MediaListRust;
    }

    // Inherited from QAbstractListModel.
    unsafe extern "RustQt" {
        #[inherit]
        #[cxx_name = "beginInsertRows"]
        unsafe fn begin_insert_rows(self: Pin<&mut MediaList>, parent: &QModelIndex, first: i32, last: i32);
        #[inherit]
        #[cxx_name = "endInsertRows"]
        unsafe fn end_insert_rows(self: Pin<&mut MediaList>);
        #[inherit]
        #[cxx_name = "beginRemoveRows"]
        unsafe fn begin_remove_rows(self: Pin<&mut MediaList>, parent: &QModelIndex, first: i32, last: i32);
        #[inherit]
        #[cxx_name = "endRemoveRows"]
        unsafe fn end_remove_rows(self: Pin<&mut MediaList>);
        #[inherit]
        fn index(self: &MediaList, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex;

        #[inherit]
        #[qsignal]
        #[cxx_name = "dataChanged"]
        fn data_changed(
            self: Pin<&mut MediaList>,
            top_left: &QModelIndex,
            bottom_right: &QModelIndex,
            roles: &QList_i32,
        );
    }

    unsafe extern "RustQt" {
        #[qinvokable]
        #[cxx_override]
        fn data(self: &MediaList, index: &QModelIndex, role: i32) -> QVariant;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &MediaList) -> QHash_i32_QByteArray;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &MediaList, parent: &QModelIndex) -> i32;

        /// Runs "play", "pause", "next", "previous" or "seek" (to
        /// `position_ms`) on a device's player.
        #[qinvokable]
        fn command(self: &MediaList, device: &QString, player: &QString, action: &QString, position_ms: f64);
        /// The row of a device's first player (the one to show), or -1.
        #[qinvokable]
        #[cxx_name = "firstFor"]
        fn first_for(self: &MediaList, device: &QString) -> i32;
        /// Whether the player at `row` is currently playing.
        #[qinvokable]
        #[cxx_name = "isPlaying"]
        fn is_playing(self: &MediaList, row: i32) -> bool;
    }

    impl cxx_qt::Threading for MediaList {}
    impl cxx_qt::Initialize for MediaList {}
}

#[derive(Default)]
pub struct MediaListRust {
    count: i32,
    revision: i32,
    active_art: QString,
    active_colors: QString,
    rows: Vec<PlayerView>,
}

/// Model roles (Qt::UserRole + n).
const ROLES: &[&str] = &[
    "deviceId",
    "playerId",
    "app",
    "title",
    "artist",
    "album",
    "playing",
    "duration",
    "position",
    "positionAt",
    "art",
    "canPlay",
    "canPause",
    "canNext",
    "canPrevious",
    "canSeek",
];
const USER_ROLE: i32 = 0x0100;

fn role_value(row: &PlayerView, role: &str) -> QVariant {
    let text = |s: &str| QVariant::from(&QString::from(s));
    let p = &row.player;
    let can = |action: &str| QVariant::from(&p.actions.iter().any(|a| a == action));
    match role {
        "deviceId" => {
            if row.device == media::LOCAL_DEVICE {
                text("local")
            } else {
                text(&row.device.to_string())
            }
        }
        "playerId" => text(&p.id),
        "app" => text(&p.app),
        "title" => text(p.title.as_deref().unwrap_or_default()),
        "artist" => text(p.artist.as_deref().unwrap_or_default()),
        "album" => text(p.album.as_deref().unwrap_or_default()),
        "playing" => QVariant::from(&p.playing),
        // Milliseconds; -1 when the player doesn't say.
        "duration" => QVariant::from(&p.duration.map_or(-1.0, |d| d as f64)),
        "position" => QVariant::from(&p.position.map_or(-1.0, |d| d as f64)),
        // When `position` was current, as JavaScript time.
        "positionAt" => {
            QVariant::from(&row.at.duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_millis() as f64))
        }
        "art" => text(
            &row.art
                .as_deref()
                .map(|path| {
                    String::from(
                        &QUrl::from_local_file(&QString::from(&*path.to_string_lossy())).to_qstring(),
                    )
                })
                .unwrap_or_default(),
        ),
        "canPlay" => can("play"),
        "canPause" => can("pause"),
        "canNext" => can("next"),
        "canPrevious" => can("previous"),
        "canSeek" => can("seek"),
        _ => QVariant::default(),
    }
}

impl cxx_qt::Initialize for qobject::MediaList {
    fn initialize(self: Pin<&mut Self>) {
        super::subscribe(self.qt_thread(), Changes::MEDIA, Self::refresh);
    }
}

impl qobject::MediaList {
    fn refresh(mut self: Pin<&mut Self>) {
        let mut new: Vec<PlayerView> = core_host::host().hub.read(|s| s.media.clone());
        new.extend(media::local_views());
        let root = QModelIndex::default();
        for edit in diff(&self.rows, &new, |r| (r.device, r.player.id.clone())) {
            match edit {
                Edit::Remove(row) => {
                    let r = row as i32;
                    // SAFETY: row exists; begin/end are balanced.
                    unsafe {
                        self.as_mut().begin_remove_rows(&root, r, r);
                        self.as_mut().rust_mut().rows.remove(row);
                        self.as_mut().end_remove_rows();
                    }
                }
                Edit::Insert(row) => {
                    let r = row as i32;
                    // SAFETY: row <= len; begin/end are balanced.
                    unsafe {
                        self.as_mut().begin_insert_rows(&root, r, r);
                        self.as_mut().rust_mut().rows.insert(row, new[row].clone());
                        self.as_mut().end_insert_rows();
                    }
                }
                Edit::Change(row) => {
                    self.as_mut().rust_mut().rows[row] = new[row].clone();
                    let index = self.index(row as i32, 0, &root);
                    self.as_mut().data_changed(&index, &index, &QList::default());
                }
            }
        }
        let count = self.rows.len() as i32;
        self.as_mut().set_count(count);
        let (art_url, colors_json) = match media::active_artwork() {
            Some((path, colors)) => {
                let url = QUrl::from_local_file(&QString::from(&*path.to_string_lossy())).to_qstring();
                (url, QString::from(&colors))
            }
            None => (QString::default(), QString::default()),
        };
        if *self.active_art() != art_url {
            self.as_mut().set_active_art(art_url);
        }
        if *self.active_colors() != colors_json {
            self.as_mut().set_active_colors(colors_json);
        }
        let revision = self.revision.wrapping_add(1);
        self.set_revision(revision);
    }

    pub fn first_for(&self, device: &QString) -> i32 {
        let device = String::from(device);
        let matches_dev = |r: &PlayerView| {
            if device == "local" {
                r.device == media::LOCAL_DEVICE
            } else {
                r.device != media::LOCAL_DEVICE && r.device.to_string() == device
            }
        };
        self.rows
            .iter()
            .position(|r| matches_dev(r) && r.player.playing)
            .or_else(|| self.rows.iter().position(matches_dev))
            .map_or(-1, |row| row as i32)
    }

    pub fn is_playing(&self, row: i32) -> bool {
        usize::try_from(row).ok().and_then(|r| self.rows.get(r)).is_some_and(|r| r.player.playing)
    }

    pub fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let row = usize::try_from(index.row()).ok();
        let role = usize::try_from(role - USER_ROLE).ok().and_then(|r| ROLES.get(r));
        match (row.and_then(|r| self.rows.get(r)), role) {
            (Some(row), Some(role)) => role_value(row, role),
            _ => QVariant::default(),
        }
    }

    pub fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        for (i, name) in ROLES.iter().enumerate() {
            roles.insert(USER_ROLE + i as i32, QByteArray::from(*name));
        }
        roles
    }

    pub fn row_count(&self, _parent: &QModelIndex) -> i32 {
        self.rows.len() as i32
    }

    pub fn command(&self, device: &QString, player: &QString, action: &QString, position_ms: f64) {
        let device_str = String::from(device);
        let action_str = String::from(action);
        let resolved_device =
            if device_str == "local" { Some(media::LOCAL_DEVICE) } else { super::parse_device(device) };
        let Some(device) = resolved_device else { return };
        if action_str == "volume" {
            let level = (position_ms.round() as i32).clamp(0, 100) as u8;
            if device == media::LOCAL_DEVICE {
                let _ = crate::win::audio::set_speaker_volume(level);
                crate::deck::broadcast_live_state();
            } else if let Some(node) = core_host::node() {
                core_host::host().hub.update(|s| {
                    let Some(mut next) = s.toggles.get(&device).cloned() else { return Changes::NONE };
                    next.volume = level;
                    s.set_toggles(device, Some(next))
                });
                core_host::spawn(async move {
                    let _ = node
                        .set_phone_toggle(
                            device,
                            "volume".into(),
                            nectarlink_core::PhoneToggleValue::Level(level),
                        )
                        .await;
                });
            }
            return;
        }
        if action_str == "mute" {
            if device == media::LOCAL_DEVICE {
                let _ = crate::win::audio::toggle_speaker_mute();
                crate::deck::broadcast_live_state();
            } else if let Some(node) = core_host::node() {
                let target = if position_ms > 0.0 { 0u8 } else { 50u8 };
                core_host::host().hub.update(|s| {
                    let Some(mut next) = s.toggles.get(&device).cloned() else { return Changes::NONE };
                    next.volume = target;
                    s.set_toggles(device, Some(next))
                });
                core_host::spawn(async move {
                    let _ = node
                        .set_phone_toggle(
                            device,
                            "volume".into(),
                            nectarlink_core::PhoneToggleValue::Level(target),
                        )
                        .await;
                });
            }
            return;
        }
        let Some(action) = MediaAction::parse(&action_str) else { return };
        let position = (action == MediaAction::Seek).then(|| position_ms.max(0.0) as u64);
        media::command(device, String::from(player), action, position);
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use nectarlink_core::{DeviceId, MediaPlayer};

    use super::*;

    #[test]
    fn roles_describe_the_player() {
        let row = PlayerView {
            device: DeviceId([4; 32]),
            player: MediaPlayer {
                id: "com.music".into(),
                app: "Music".into(),
                title: Some("Song".into()),
                artist: None,
                album: None,
                playing: true,
                duration: Some(90_000),
                position: None,
                actions: vec!["pause".into(), "seek".into()],
                art_key: None,
                art: None,
            },
            #[cfg(windows)]
            art: Some(PathBuf::from(r"C:\data\cache\art\a.jpg")),
            #[cfg(not(windows))]
            art: Some(PathBuf::from("/data/cache/art/a.jpg")),
            at: UNIX_EPOCH + std::time::Duration::from_millis(1_500),
        };
        let get = |role| role_value(&row, role);
        assert_eq!(get("title").value::<QString>().map(|s| String::from(&s)), Some("Song".into()));
        assert_eq!(get("artist").value::<QString>().map(|s| String::from(&s)), Some(String::new()));
        assert_eq!(get("duration").value::<f64>(), Some(90_000.0));
        assert_eq!(get("position").value::<f64>(), Some(-1.0));
        assert_eq!(get("positionAt").value::<f64>(), Some(1_500.0));
        assert_eq!(get("canPause").value::<bool>(), Some(true));
        assert_eq!(get("canPlay").value::<bool>(), Some(false));
        #[cfg(windows)]
        let expected_art = "file:///C:/data/cache/art/a.jpg";
        #[cfg(not(windows))]
        let expected_art = "file:///data/cache/art/a.jpg";
        assert_eq!(
            get("art").value::<QString>().map(|s| String::from(&s)),
            Some(expected_art.into())
        );
    }
}
