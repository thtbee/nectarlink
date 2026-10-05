// SPDX-License-Identifier: GPL-3.0-or-later
//! `DeviceList`: the paired devices as a Qt list model, updated in place
//! (insert, remove, change) so QML delegates keep their state and animate.

use std::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QList, QModelIndex, QString, QVariant};
use nectarlink_core::{ConnectionPath, DeviceKind, LinkState, PowerLevel};

use super::{Edit, diff};
use crate::{
    core_host,
    state::{Changes, DeviceView},
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
        type DeviceList = super::DeviceListRust;
    }

    // Inherited from QAbstractListModel.
    unsafe extern "RustQt" {
        #[inherit]
        #[cxx_name = "beginInsertRows"]
        unsafe fn begin_insert_rows(self: Pin<&mut DeviceList>, parent: &QModelIndex, first: i32, last: i32);
        #[inherit]
        #[cxx_name = "endInsertRows"]
        unsafe fn end_insert_rows(self: Pin<&mut DeviceList>);
        #[inherit]
        #[cxx_name = "beginRemoveRows"]
        unsafe fn begin_remove_rows(self: Pin<&mut DeviceList>, parent: &QModelIndex, first: i32, last: i32);
        #[inherit]
        #[cxx_name = "endRemoveRows"]
        unsafe fn end_remove_rows(self: Pin<&mut DeviceList>);
        #[inherit]
        #[cxx_name = "beginResetModel"]
        unsafe fn begin_reset_model(self: Pin<&mut DeviceList>);
        #[inherit]
        #[cxx_name = "endResetModel"]
        unsafe fn end_reset_model(self: Pin<&mut DeviceList>);
        #[inherit]
        fn index(self: &DeviceList, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex;

        #[inherit]
        #[qsignal]
        #[cxx_name = "dataChanged"]
        fn data_changed(
            self: Pin<&mut DeviceList>,
            top_left: &QModelIndex,
            bottom_right: &QModelIndex,
            roles: &QList_i32,
        );
    }

    unsafe extern "RustQt" {
        #[qinvokable]
        #[cxx_override]
        fn data(self: &DeviceList, index: &QModelIndex, role: i32) -> QVariant;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &DeviceList) -> QHash_i32_QByteArray;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &DeviceList, parent: &QModelIndex) -> i32;
        /// The row of a device ID, or -1.
        #[qinvokable]
        #[cxx_name = "rowOf"]
        fn row_of(self: &DeviceList, device_id: &QString) -> i32;
    }

    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    impl cxx_qt::Threading for DeviceList {}
    impl cxx_qt::Initialize for DeviceList {}
}

#[derive(Default)]
pub struct DeviceListRust {
    count: i32,
    rows: Vec<DeviceView>,
}

/// Model roles (Qt::UserRole + n).
const ROLES: &[&str] = &[
    "deviceId",
    "name",
    "kind",
    "os",
    "osVersion",
    "model",
    "online",
    "path",
    "rttMs",
    "battery",
    "charging",
    "power",
    "lastSeen",
    "pairedAt",
    "accent",
];
const USER_ROLE: i32 = 0x0100;

fn role_value(device: &DeviceView, role: &str) -> QVariant {
    let text = |s: &str| QVariant::from(&QString::from(s));
    match role {
        "deviceId" => text(&device.id.to_string()),
        "name" => text(&device.info.name),
        "kind" => text(match device.info.kind {
            DeviceKind::Phone => "phone",
            DeviceKind::Tablet => "tablet",
            DeviceKind::Desktop => "desktop",
            DeviceKind::Laptop => "laptop",
            DeviceKind::Unknown => "unknown",
        }),
        "os" => text(&device.info.os),
        "osVersion" => text(&device.info.os_ver),
        "model" => text(device.info.model.as_deref().unwrap_or_default()),
        "online" => QVariant::from(&matches!(device.link, LinkState::Online { .. })),
        "path" => text(match device.link {
            LinkState::Online { path: ConnectionPath::Lan, .. } => "lan",
            LinkState::Online { path: ConnectionPath::Relay, .. } => "relay",
            LinkState::Connecting => "connecting",
            LinkState::Offline { .. } => "offline",
        }),
        "rttMs" => QVariant::from(&match device.link {
            LinkState::Online { rtt_ms, .. } => i32::try_from(rtt_ms).unwrap_or(i32::MAX),
            _ => -1,
        }),
        "battery" => QVariant::from(&device.battery.as_ref().map_or(-1, |b| i32::from(b.level))),
        "charging" => QVariant::from(&device.battery.as_ref().is_some_and(|b| b.charging)),
        "power" => text(match device.power {
            PowerLevel::Assist => "assist",
            PowerLevel::Elevated => "elevated",
            PowerLevel::NotApplicable => "n/a",
            PowerLevel::Basic | PowerLevel::Unknown => "basic",
        }),
        "lastSeen" => QVariant::from(&match device.link {
            LinkState::Offline { last_seen: Some(at) } => at as f64,
            _ => 0.0,
        }),
        "pairedAt" => QVariant::from(&(device.paired_at as f64)),
        "accent" => {
            text(&device.info.accent.map(|argb| format!("#{:06X}", argb & 0x00FF_FFFF)).unwrap_or_default())
        }
        _ => QVariant::default(),
    }
}

impl cxx_qt::Initialize for qobject::DeviceList {
    fn initialize(self: Pin<&mut Self>) {
        super::subscribe(self.qt_thread(), Changes::DEVICES, Self::refresh);
    }
}

impl qobject::DeviceList {
    fn refresh(mut self: Pin<&mut Self>) {
        let new = core_host::host().hub.read(|s| s.devices.clone());
        let root = QModelIndex::default();
        match diff(&self.rows, &new, |d| d.id) {
            None => {
                // SAFETY: begin/end pairs are balanced around the swap.
                unsafe {
                    self.as_mut().begin_reset_model();
                    self.as_mut().rust_mut().rows = new;
                    self.as_mut().end_reset_model();
                }
            }
            Some(edits) => {
                for edit in edits {
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
            }
        }
        let count = self.rows.len() as i32;
        self.set_count(count);
    }

    pub fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let row = usize::try_from(index.row()).ok();
        let role = usize::try_from(role - USER_ROLE).ok().and_then(|r| ROLES.get(r));
        match (row.and_then(|r| self.rows.get(r)), role) {
            (Some(device), Some(role)) => role_value(device, role),
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

    pub fn row_of(&self, device_id: &QString) -> i32 {
        let id = String::from(device_id);
        self.rows.iter().position(|d| d.id.to_string() == id).map_or(-1, |r| r as i32)
    }
}

#[cfg(test)]
mod tests {
    use nectarlink_core::{DeviceId, DeviceInfo};

    use super::*;

    fn device(n: u8, name: &str) -> DeviceView {
        DeviceView {
            id: DeviceId([n; 32]),
            info: DeviceInfo {
                name: name.into(),
                kind: DeviceKind::Phone,
                os: "android".into(),
                os_ver: "16".into(),
                model: None,
                accent: None,
            },
            paired_at: i64::from(n),
            link: LinkState::Offline { last_seen: None },
            battery: None,
            power: PowerLevel::Basic,
        }
    }

    /// Applies edits the way the model does and checks the result.
    fn apply(old: &[DeviceView], new: &[DeviceView]) -> Vec<DeviceView> {
        let mut rows = old.to_vec();
        for edit in diff(old, new, |d| d.id).expect("incremental") {
            match edit {
                Edit::Remove(r) => {
                    rows.remove(r);
                }
                Edit::Insert(r) => rows.insert(r, new[r].clone()),
                Edit::Change(r) => rows[r] = new[r].clone(),
            }
        }
        rows
    }

    #[test]
    fn inserts_removes_and_changes() {
        let (a, b, c, d) = (device(1, "a"), device(2, "b"), device(3, "c"), device(4, "d"));
        let old = vec![a.clone(), b.clone(), c.clone()];
        let mut b2 = b.clone();
        b2.info.name = "b2".into();
        let new = vec![a.clone(), b2.clone(), d.clone()];
        assert_eq!(diff(&old, &new, |d| d.id).unwrap(), [Edit::Remove(2), Edit::Change(1), Edit::Insert(2)]);
        assert_eq!(apply(&old, &new), new);

        assert_eq!(apply(&[], &new), new);
        assert_eq!(apply(&new, &[]), Vec::<DeviceView>::new());
        assert_eq!(diff(&new, &new, |d| d.id).unwrap(), []);
        let interleaved = vec![d.clone(), a.clone(), c.clone(), b2.clone()];
        assert_eq!(apply(&[a.clone(), b2.clone()], &interleaved), interleaved);
    }

    #[test]
    fn reordering_falls_back_to_a_reset() {
        let (a, b) = (device(1, "a"), device(2, "b"));
        assert_eq!(diff(&[a.clone(), b.clone()], &[b, a], |d| d.id), None);
    }

    #[test]
    fn roles_cover_the_device() {
        let mut d = device(5, "Pixel");
        d.link = LinkState::Online { path: ConnectionPath::Lan, rtt_ms: 7 };
        d.battery = Some(nectarlink_core::Battery { level: 64, charging: true, plugged: None });
        d.info.accent = Some(0xFF8A5100);
        assert_eq!(role_value(&d, "name").value::<QString>().map(String::from).as_deref(), Some("Pixel"));
        assert_eq!(role_value(&d, "online").value::<bool>(), Some(true));
        assert_eq!(role_value(&d, "rttMs").value::<i32>(), Some(7));
        assert_eq!(role_value(&d, "battery").value::<i32>(), Some(64));
        assert_eq!(role_value(&d, "accent").value::<QString>().map(String::from).as_deref(), Some("#8A5100"));
    }
}
