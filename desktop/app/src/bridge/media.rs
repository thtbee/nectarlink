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
    }

    impl cxx_qt::Threading for MediaList {}
    impl cxx_qt::Initialize for MediaList {}
}

#[derive(Default)]
pub struct MediaListRust {
    count: i32,
    revision: i32,
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
        "deviceId" => text(&row.device.to_string()),
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
        let new: Vec<PlayerView> = core_host::host().hub.read(|s| s.media.clone());
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
        let revision = self.revision.wrapping_add(1);
        self.set_revision(revision);
    }

    pub fn first_for(&self, device: &QString) -> i32 {
        let device = String::from(device);
        self.rows.iter().position(|r| r.device.to_string() == device).map_or(-1, |row| row as i32)
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
        let Some(device) = super::parse_device(device) else { return };
        let Some(action) = MediaAction::parse(&String::from(action)) else { return };
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
            art: Some(PathBuf::from(r"C:\data\cache\art\a.jpg")),
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
        assert_eq!(
            get("art").value::<QString>().map(|s| String::from(&s)),
            Some("file:///C:/data/cache/art/a.jpg".into())
        );
    }
}
