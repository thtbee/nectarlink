// SPDX-License-Identifier: GPL-3.0-or-later
//! `TransferList`: file transfers (newest first) as a Qt list model, plus
//! the commands QML runs on them.

use std::{path::PathBuf, pin::Pin};

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{
    QByteArray, QHash, QHashPair_i32_QByteArray, QList, QModelIndex, QString, QStringList, QUrl, QVariant,
};
use nectarlink_core::{Direction, TransferFailure, TransferState};

use super::{Edit, diff};
use crate::{
    core_host,
    state::{Changes, TransferView},
    transfers,
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
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[base = QAbstractListModel]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(i32, count)]
        /// Transfers still waiting or running.
        #[qproperty(i32, active)]
        type TransferList = super::TransferListRust;
    }

    // Inherited from QAbstractListModel.
    unsafe extern "RustQt" {
        #[inherit]
        #[cxx_name = "beginInsertRows"]
        unsafe fn begin_insert_rows(
            self: Pin<&mut TransferList>,
            parent: &QModelIndex,
            first: i32,
            last: i32,
        );
        #[inherit]
        #[cxx_name = "endInsertRows"]
        unsafe fn end_insert_rows(self: Pin<&mut TransferList>);
        #[inherit]
        #[cxx_name = "beginRemoveRows"]
        unsafe fn begin_remove_rows(
            self: Pin<&mut TransferList>,
            parent: &QModelIndex,
            first: i32,
            last: i32,
        );
        #[inherit]
        #[cxx_name = "endRemoveRows"]
        unsafe fn end_remove_rows(self: Pin<&mut TransferList>);
        #[inherit]
        fn index(self: &TransferList, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex;

        #[inherit]
        #[qsignal]
        #[cxx_name = "dataChanged"]
        fn data_changed(
            self: Pin<&mut TransferList>,
            top_left: &QModelIndex,
            bottom_right: &QModelIndex,
            roles: &QList_i32,
        );
    }

    unsafe extern "RustQt" {
        #[qinvokable]
        #[cxx_override]
        fn data(self: &TransferList, index: &QModelIndex, role: i32) -> QVariant;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &TransferList) -> QHash_i32_QByteArray;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &TransferList, parent: &QModelIndex) -> i32;

        /// Sends files (as `file:` URLs, from a picker or a drop) to a device.
        #[qinvokable]
        fn send(self: &TransferList, device: &QString, urls: &QStringList);
        #[qinvokable]
        fn cancel(self: &TransferList, id: &QString);
        /// Opens what a finished incoming transfer saved (its first file).
        #[qinvokable]
        fn open(self: &TransferList, id: &QString);
        #[qinvokable]
        #[cxx_name = "showInFolder"]
        fn show_in_folder(self: &TransferList, id: &QString);
        #[qinvokable]
        #[cxx_name = "clearFinished"]
        fn clear_finished(self: &TransferList);
    }

    impl cxx_qt::Threading for TransferList {}
    impl cxx_qt::Initialize for TransferList {}
}

#[derive(Debug, Clone, PartialEq)]
struct Row {
    view: TransferView,
    device_name: String,
}

#[derive(Default)]
pub struct TransferListRust {
    count: i32,
    active: i32,
    rows: Vec<Row>,
}

/// Model roles (Qt::UserRole + n).
const ROLES: &[&str] = &[
    "transferId",
    "deviceId",
    "deviceName",
    "incoming",
    "title",
    "fileCount",
    "total",
    "done",
    "progress",
    "rate",
    "status",
    "reason",
    "savedPath",
];
const USER_ROLE: i32 = 0x0100;

fn role_value(row: &Row, role: &str) -> QVariant {
    let text = |s: &str| QVariant::from(&QString::from(s));
    let t = &row.view.transfer;
    match role {
        "transferId" => text(&t.id),
        "deviceId" => text(&t.device.to_string()),
        "deviceName" => text(&row.device_name),
        "incoming" => QVariant::from(&(t.direction == Direction::Incoming)),
        "title" => text(&match t.names.as_slice() {
            [one] => one.clone(),
            [first, rest @ ..] => format!("{first} and {} more", rest.len()),
            [] => String::new(),
        }),
        "fileCount" => QVariant::from(&i32::try_from(t.names.len()).unwrap_or(i32::MAX)),
        "total" => QVariant::from(&(t.total as f64)),
        "done" => QVariant::from(&(t.done as f64)),
        "progress" => QVariant::from(&if t.total == 0 { 1.0 } else { t.done as f64 / t.total as f64 }),
        "rate" => QVariant::from(&row.view.rate),
        "status" => text(match t.state {
            TransferState::Waiting => "waiting",
            TransferState::Running => "running",
            TransferState::Done { .. } => "done",
            TransferState::Failed(_) => "failed",
            TransferState::Cancelled => "cancelled",
        }),
        "reason" => text(match &t.state {
            TransferState::Failed(TransferFailure::Denied) => "denied",
            TransferState::Failed(TransferFailure::Unreachable) => "unreachable",
            TransferState::Failed(TransferFailure::NoSpace) => "noSpace",
            TransferState::Failed(TransferFailure::Interrupted) => "interrupted",
            TransferState::Failed(TransferFailure::Other(_)) => "other",
            _ => "",
        }),
        "savedPath" => text(&saved_path(row).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()),
        _ => QVariant::default(),
    }
}

fn saved_path(row: &Row) -> Option<PathBuf> {
    match &row.view.transfer.state {
        TransferState::Done { saved } => saved.first().cloned(),
        _ => None,
    }
}

impl cxx_qt::Initialize for qobject::TransferList {
    fn initialize(self: Pin<&mut Self>) {
        super::subscribe(self.qt_thread(), Changes::TRANSFERS | Changes::DEVICES, Self::refresh);
    }
}

impl qobject::TransferList {
    fn refresh(mut self: Pin<&mut Self>) {
        let new: Vec<Row> = core_host::host().hub.read(|s| {
            s.transfers
                .iter()
                .map(|view| Row {
                    device_name: s.name_of(&view.transfer.device).unwrap_or_default(),
                    view: view.clone(),
                })
                .collect()
        });
        let root = QModelIndex::default();
        for edit in diff(&self.rows, &new, |r| r.view.transfer.id.clone()) {
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
        let active = self.rows.iter().filter(|r| !r.view.transfer.state.is_finished()).count() as i32;
        self.as_mut().set_count(count);
        self.set_active(active);
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

    pub fn send(&self, device: &QString, urls: &QStringList) {
        let Some(device) = super::parse_device(device) else { return };
        let paths: Vec<PathBuf> = QList::<QString>::from(urls)
            .iter()
            .filter_map(|url| QUrl::from(url).to_local_file().map(|path| String::from(&path)))
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .collect();
        transfers::send(device, paths);
    }

    pub fn cancel(&self, id: &QString) {
        transfers::cancel(&String::from(id));
    }

    fn row(&self, id: &QString) -> Option<&Row> {
        let id = String::from(id);
        self.rows.iter().find(|r| r.view.transfer.id == id)
    }

    pub fn open(&self, id: &QString) {
        if let Some(path) = self.row(id).and_then(saved_path) {
            transfers::open(&path);
        }
    }

    pub fn show_in_folder(&self, id: &QString) {
        if let Some(path) = self.row(id).and_then(saved_path) {
            transfers::show_in_folder(&path);
        }
    }

    pub fn clear_finished(&self) {
        core_host::host().hub.update(|s| s.clear_finished_transfers());
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use nectarlink_core::{DeviceId, Transfer};

    use super::*;

    #[test]
    fn roles_describe_the_transfer() {
        let row = Row {
            view: TransferView {
                transfer: Transfer {
                    id: "t".into(),
                    device: DeviceId([3; 32]),
                    direction: Direction::Incoming,
                    names: vec!["a.jpg".into(), "b.jpg".into(), "c.jpg".into()],
                    total: 400,
                    done: 100,
                    state: TransferState::Done { saved: vec![PathBuf::from(r"C:\Downloads\a.jpg")] },
                },
                rate: 0.0,
                sampled: Instant::now(),
            },
            device_name: "Pixel".into(),
        };
        let text = |role| role_value(&row, role).value::<QString>().map(String::from).unwrap_or_default();
        assert_eq!(text("title"), "a.jpg and 2 more");
        assert_eq!(text("status"), "done");
        assert_eq!(text("savedPath"), r"C:\Downloads\a.jpg");
        assert_eq!(role_value(&row, "progress").value::<f64>(), Some(0.25));
        assert_eq!(role_value(&row, "incoming").value::<bool>(), Some(true));
    }
}
