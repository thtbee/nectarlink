// SPDX-License-Identifier: GPL-3.0-or-later
//! `NotificationHistory`: notifications that went away in the last day,
//! newest first, as a Qt list model (apps the user hid are left out).

use std::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QList, QModelIndex, QString, QVariant};

use super::{Edit, diff};
use crate::{
    core_host, icons,
    state::{AppRule, Changes, HistoryEntry},
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
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[base = QAbstractListModel]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(i32, count)]
        type NotificationHistory = super::NotificationHistoryRust;
    }

    // Inherited from QAbstractListModel.
    unsafe extern "RustQt" {
        #[inherit]
        #[cxx_name = "beginInsertRows"]
        unsafe fn begin_insert_rows(
            self: Pin<&mut NotificationHistory>,
            parent: &QModelIndex,
            first: i32,
            last: i32,
        );
        #[inherit]
        #[cxx_name = "endInsertRows"]
        unsafe fn end_insert_rows(self: Pin<&mut NotificationHistory>);
        #[inherit]
        #[cxx_name = "beginRemoveRows"]
        unsafe fn begin_remove_rows(
            self: Pin<&mut NotificationHistory>,
            parent: &QModelIndex,
            first: i32,
            last: i32,
        );
        #[inherit]
        #[cxx_name = "endRemoveRows"]
        unsafe fn end_remove_rows(self: Pin<&mut NotificationHistory>);
        #[inherit]
        fn index(self: &NotificationHistory, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex;

        #[inherit]
        #[qsignal]
        #[cxx_name = "dataChanged"]
        fn data_changed(
            self: Pin<&mut NotificationHistory>,
            top_left: &QModelIndex,
            bottom_right: &QModelIndex,
            roles: &QList_i32,
        );
    }

    unsafe extern "RustQt" {
        #[qinvokable]
        #[cxx_override]
        fn data(self: &NotificationHistory, index: &QModelIndex, role: i32) -> QVariant;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &NotificationHistory) -> QHash_i32_QByteArray;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &NotificationHistory, parent: &QModelIndex) -> i32;
    }

    impl cxx_qt::Threading for NotificationHistory {}
    impl cxx_qt::Initialize for NotificationHistory {}
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    entry: HistoryEntry,
    device_name: String,
    icon_url: String,
}

#[derive(Default)]
pub struct NotificationHistoryRust {
    count: i32,
    rows: Vec<Row>,
}

/// Model roles (Qt::UserRole + n).
const ROLES: &[&str] =
    &["deviceId", "deviceName", "app", "appName", "title", "text", "sub", "when", "removedAt", "iconUrl"];
const USER_ROLE: i32 = 0x0100;

fn role_value(row: &Row, role: &str) -> QVariant {
    let text = |s: &str| QVariant::from(&QString::from(s));
    let n = &row.entry.notification;
    match role {
        "deviceId" => text(&row.entry.device.to_string()),
        "deviceName" => text(&row.device_name),
        "app" => text(&n.app),
        "appName" => text(&n.app_name),
        "title" => text(n.title.as_deref().unwrap_or_default()),
        "text" => text(n.text.as_deref().unwrap_or_default()),
        "sub" => text(n.sub.as_deref().unwrap_or_default()),
        "when" => QVariant::from(&(n.when as f64)),
        "removedAt" => QVariant::from(&(row.entry.removed_at as f64)),
        "iconUrl" => text(&row.icon_url),
        _ => QVariant::default(),
    }
}

impl cxx_qt::Initialize for qobject::NotificationHistory {
    fn initialize(self: Pin<&mut Self>) {
        super::subscribe(
            self.qt_thread(),
            Changes::HISTORY | Changes::APPS | Changes::DEVICES | Changes::NOTIFICATIONS,
            Self::refresh,
        );
    }
}

impl qobject::NotificationHistory {
    fn refresh(mut self: Pin<&mut Self>) {
        let new: Vec<Row> = core_host::host().hub.read(|s| {
            s.history
                .iter()
                .filter(|h| s.app_rule(&h.notification.app) != AppRule::Hidden)
                .map(|h| Row {
                    device_name: s.name_of(&h.device).unwrap_or_default(),
                    icon_url: s
                        .app_icons
                        .get(&h.notification.app)
                        .map(|p| icons::file_url(p))
                        .unwrap_or_default(),
                    entry: h.clone(),
                })
                .collect()
        });
        let root = QModelIndex::default();
        let key = |r: &Row| (r.entry.device, r.entry.notification.key.clone(), r.entry.removed_at);
        for edit in diff(&self.rows, &new, key) {
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
        self.set_count(count);
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
}
