// SPDX-License-Identifier: GPL-3.0-or-later
//! `Photos`: a phone's photo and video gallery as a Qt list model for the
//! virtualized `GridView`, plus albums and full-file actions (Save, Copy,
//! Open, and in-app viewing).

use std::{path::PathBuf, pin::Pin};

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{
    QByteArray, QHash, QHashPair_i32_QByteArray, QList, QModelIndex, QString, QStringList, QUrl, QVariant,
};
use serde_json::json;

use super::{Edit, diff};
use crate::{
    photos::{self, CellKind, ItemRow, Rows},
    state::Changes,
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
        #[qproperty(QString, device)]
        /// "idle", "loading", "ready", "offline", "off", "unsupported" or "failed".
        #[qproperty(QString, status)]
        /// Albums on the phone (JSON array).
        #[qproperty(QString, albums)]
        /// Selected album ID ("" for All photos).
        #[qproperty(QString, album)]
        /// Rows in the grid, with the day rows and gaps around the photos.
        #[qproperty(i32, count)]
        /// Photos and videos loaded.
        #[qproperty(i32, photo_count, cxx_name = "photoCount")]
        #[qproperty(bool, more)]
        #[qproperty(bool, loading_older, cxx_name = "loadingOlder")]
        #[qproperty(bool, saving)]
        #[qproperty(QString, busy_item, cxx_name = "busyItem")]
        #[qproperty(QString, save_folder, cxx_name = "saveFolder")]
        #[qproperty(i32, revision)]
        type Photos = super::PhotosRust;
    }

    // Inherited from QAbstractListModel.
    unsafe extern "RustQt" {
        #[inherit]
        #[cxx_name = "beginInsertRows"]
        unsafe fn begin_insert_rows(self: Pin<&mut Photos>, parent: &QModelIndex, first: i32, last: i32);
        #[inherit]
        #[cxx_name = "endInsertRows"]
        unsafe fn end_insert_rows(self: Pin<&mut Photos>);
        #[inherit]
        #[cxx_name = "beginRemoveRows"]
        unsafe fn begin_remove_rows(self: Pin<&mut Photos>, parent: &QModelIndex, first: i32, last: i32);
        #[inherit]
        #[cxx_name = "endRemoveRows"]
        unsafe fn end_remove_rows(self: Pin<&mut Photos>);
        #[inherit]
        #[cxx_name = "beginResetModel"]
        unsafe fn begin_reset_model(self: Pin<&mut Photos>);
        #[inherit]
        #[cxx_name = "endResetModel"]
        unsafe fn end_reset_model(self: Pin<&mut Photos>);
        #[inherit]
        fn index(self: &Photos, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex;

        #[inherit]
        #[qsignal]
        #[cxx_name = "dataChanged"]
        fn data_changed(
            self: Pin<&mut Photos>,
            top_left: &QModelIndex,
            bottom_right: &QModelIndex,
            roles: &QList_i32,
        );
    }

    unsafe extern "RustQt" {
        #[qinvokable]
        #[cxx_override]
        fn data(self: &Photos, index: &QModelIndex, role: i32) -> QVariant;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &Photos) -> QHash_i32_QByteArray;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &Photos, parent: &QModelIndex) -> i32;

        #[qinvokable]
        fn open(self: &Photos, device: &QString);
        #[qinvokable]
        #[cxx_name = "selectAlbum"]
        fn select_album(self: &Photos, album: &QString);
        #[qinvokable]
        #[cxx_name = "loadOlder"]
        fn load_older(self: &Photos);
        #[qinvokable]
        fn refresh(self: &Photos);
        #[qinvokable]
        #[cxx_name = "needThumb"]
        fn need_thumb(self: &Photos, id: &QString);
        #[qinvokable]
        #[cxx_name = "dropThumb"]
        fn drop_thumb(self: &Photos, id: &QString);
        #[qinvokable]
        #[cxx_name = "ensureFull"]
        fn ensure_full(self: &Photos, id: &QString);
        #[qinvokable]
        #[cxx_name = "openItem"]
        fn open_item(self: &Photos, id: &QString);
        #[qinvokable]
        #[cxx_name = "copyItem"]
        fn copy_item(self: &Photos, id: &QString);
        #[qinvokable]
        #[cxx_name = "shareItems"]
        fn share_items(self: &Photos, ids: &QStringList);
        #[qinvokable]
        #[cxx_name = "deleteItems"]
        fn delete_items(self: &Photos, ids: &QStringList);
        #[qinvokable]
        #[cxx_name = "saveItems"]
        fn save_items(self: &Photos, ids: &QStringList, folder_url: &QString);
        #[qinvokable]
        #[cxx_name = "chooseSaveFolder"]
        fn choose_save_folder(self: &Photos, folder_url: &QString);
        #[qinvokable]
        #[cxx_name = "itemAt"]
        fn item_at(self: &Photos, row: i32) -> QString;
        #[qinvokable]
        #[cxx_name = "idAt"]
        fn id_at(self: &Photos, row: i32) -> QString;
        #[qinvokable]
        #[cxx_name = "dateAt"]
        fn date_at(self: &Photos, row: i32) -> f64;
        /// Lays the grid out for this many columns, each day on rows of its own.
        #[qinvokable]
        #[cxx_name = "setColumns"]
        fn set_columns(self: &Photos, columns: i32);
        /// The row of the photo `delta` photos from `row` (-1 if none).
        #[qinvokable]
        fn step(self: &Photos, row: i32, delta: i32) -> i32;
        /// Which photo `row` is, counting from 1 (0 for a day row or gap).
        #[qinvokable]
        fn ordinal(self: &Photos, row: i32) -> i32;
        /// The row showing photo `id` now (-1 if it isn't loaded).
        #[qinvokable]
        #[cxx_name = "rowOf"]
        fn row_of(self: &Photos, id: &QString) -> i32;
        #[qinvokable]
        #[cxx_name = "startDrag"]
        fn start_drag(self: &Photos, id: &QString) -> bool;
    }

    impl cxx_qt::Threading for Photos {}
    impl cxx_qt::Initialize for Photos {}
}

#[derive(Default)]
pub struct PhotosRust {
    device: QString,
    status: QString,
    albums: QString,
    album: QString,
    count: i32,
    photo_count: i32,
    more: bool,
    loading_older: bool,
    saving: bool,
    busy_item: QString,
    save_folder: QString,
    revision: i32,
    rows: Vec<ItemRow>,
    last_albums: String,
}

const ROLES: &[&str] = &[
    "kind",
    "itemId",
    "name",
    "date",
    "prevDate",
    "size",
    "itemWidth",
    "itemHeight",
    "duration",
    "isVideo",
    "album",
    "thumb",
    "fullUrl",
];
const USER_ROLE: i32 = 0x0100;
/// More row edits than this replace the whole list at once.
const MAX_ROW_EDITS: usize = 64;

fn role_value(row: &ItemRow, role: &str) -> QVariant {
    let text = |s: &str| QVariant::from(&QString::from(s));
    match role {
        "kind" => text(row.kind.name()),
        // A day row or gap is no photo: nothing to select or fetch.
        "itemId" => text(if row.kind == CellKind::Photo { &row.id } else { "" }),
        "name" => text(&row.name),
        "date" => QVariant::from(&(row.date as f64)),
        "prevDate" => QVariant::from(&(row.prev_date as f64)),
        "size" => QVariant::from(&(row.size as f64)),
        "itemWidth" => QVariant::from(&(row.width as i32)),
        "itemHeight" => QVariant::from(&(row.height as i32)),
        "duration" => QVariant::from(&(row.duration as i32)),
        "isVideo" => QVariant::from(&row.is_video),
        "album" => text(&row.album),
        "thumb" => text(&row.thumb),
        "fullUrl" => text(&row.full_url),
        _ => QVariant::default(),
    }
}

fn folder_from_url(url: &QString) -> Option<PathBuf> {
    let raw = String::from(url);
    if raw.trim().is_empty() {
        return None;
    }
    if let Some(local) = QUrl::from(url).to_local_file() {
        let s = String::from(&local);
        if !s.is_empty() {
            return Some(PathBuf::from(s));
        }
    }
    Some(PathBuf::from(raw))
}

impl cxx_qt::Initialize for qobject::Photos {
    fn initialize(mut self: Pin<&mut Self>) {
        #[cfg(windows)]
        const DEFAULT_SAVE_FOLDER: &str = r"Downloads\Nectarlink";
        #[cfg(not(windows))]
        const DEFAULT_SAVE_FOLDER: &str = "Downloads/Nectarlink";
        self.as_mut().set_status(QString::from("idle"));
        self.as_mut().set_albums(QString::from("[]"));
        self.as_mut().set_save_folder(QString::from(DEFAULT_SAVE_FOLDER));
        super::subscribe(self.qt_thread(), Changes::PHOTOS, Self::update_view);
        self.update_view();
    }
}

impl qobject::Photos {
    fn update_view(mut self: Pin<&mut Self>) {
        if let Some(files) = photos::take_share_ready()
            && let Err(reason) = crate::win::share::show(&files)
        {
            tracing::warn!(reason, "can't open the share sheet");
            photos::show_share_failed();
        }
        let view = photos::view_since(&self.rows);
        self.as_mut().set_device(QString::from(&view.device.map(|d| d.to_string()).unwrap_or_default()));
        self.as_mut().set_status(QString::from(view.status.as_str()));
        let albums_str = view.albums.to_string();
        if self.rust().last_albums != albums_str {
            self.as_mut().set_albums(QString::from(&albums_str));
            self.as_mut().rust_mut().last_albums = albums_str;
        }
        self.as_mut().set_album(QString::from(&view.album));
        self.as_mut().set_more(view.more);
        self.as_mut().set_loading_older(view.loading_older);
        self.as_mut().set_saving(view.saving);
        self.as_mut().set_busy_item(QString::from(&view.busy_item));
        self.as_mut().set_save_folder(QString::from(&view.save_folder));

        let root = QModelIndex::default();
        match view.rows {
            Rows::Patch { keep, changed, appended } => {
                if keep < self.rows.len() {
                    let first = keep as i32;
                    let last = self.rows.len() as i32 - 1;
                    // SAFETY: keep..len exists; begin/end are balanced.
                    unsafe {
                        self.as_mut().begin_remove_rows(&root, first, last);
                        self.as_mut().rust_mut().rows.truncate(keep);
                        self.as_mut().end_remove_rows();
                    }
                }
                for (row, item) in changed {
                    self.as_mut().rust_mut().rows[row] = item;
                    let index = self.index(row as i32, 0, &root);
                    self.as_mut().data_changed(&index, &index, &QList::default());
                }
                if !appended.is_empty() {
                    let first = self.rows.len() as i32;
                    let last = first + appended.len() as i32 - 1;
                    // SAFETY: first..=last appends to the end; begin/end are balanced.
                    unsafe {
                        self.as_mut().begin_insert_rows(&root, first, last);
                        self.as_mut().rust_mut().rows.extend(appended);
                        self.as_mut().end_insert_rows();
                    }
                }
            }
            Rows::All(rows) => self.as_mut().replace_rows(rows),
        }
        let count = self.rows.len() as i32;
        self.as_mut().set_count(count);
        self.as_mut().set_photo_count(i32::try_from(view.photo_count).unwrap_or(i32::MAX));
        let rev = self.revision().wrapping_add(1);
        self.set_revision(rev);
    }

    /// Shows `rows` instead of the current ones: row by row for a few
    /// changes, so tiles keep their state, or all at once for many, since
    /// thousands of single-row signals each relayout the grid.
    fn replace_rows(mut self: Pin<&mut Self>, rows: Vec<ItemRow>) {
        let root = QModelIndex::default();
        if rows.is_empty() && self.rows.is_empty() {
            return;
        }
        let edits = if self.rows.is_empty() || rows.is_empty() {
            None
        } else {
            Some(diff(&self.rows, &rows, |r| r.id.clone()))
        };
        match edits {
            Some(edits) if edits.len() <= MAX_ROW_EDITS => {
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
                                self.as_mut().rust_mut().rows.insert(row, rows[row].clone());
                                self.as_mut().end_insert_rows();
                            }
                        }
                        Edit::Change(row) => {
                            self.as_mut().rust_mut().rows[row] = rows[row].clone();
                            let index = self.index(row as i32, 0, &root);
                            self.as_mut().data_changed(&index, &index, &QList::default());
                        }
                    }
                }
            }
            _ => {
                // SAFETY: begin/end are balanced around the swap.
                unsafe {
                    self.as_mut().begin_reset_model();
                    self.as_mut().rust_mut().rows = rows;
                    self.as_mut().end_reset_model();
                }
            }
        }
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

    pub fn open(&self, device: &QString) {
        if let Some(device) = super::parse_device(device) {
            photos::open_device(device);
        }
    }

    pub fn select_album(&self, album: &QString) {
        photos::select_album(String::from(album));
    }

    pub fn load_older(&self) {
        photos::load_older();
    }

    pub fn refresh(&self) {
        photos::reload();
    }

    pub fn need_thumb(&self, id: &QString) {
        photos::need_thumb(String::from(id));
    }

    pub fn drop_thumb(&self, id: &QString) {
        photos::drop_thumb(&String::from(id));
    }

    pub fn ensure_full(&self, id: &QString) {
        photos::ensure_full(String::from(id));
    }

    pub fn open_item(&self, id: &QString) {
        photos::open_item(String::from(id));
    }

    pub fn copy_item(&self, id: &QString) {
        photos::copy_item(String::from(id));
    }

    pub fn share_items(&self, ids: &QStringList) {
        let list: Vec<String> =
            QList::<QString>::from(ids).iter().map(String::from).filter(|id| !id.is_empty()).collect();
        photos::share_items(list);
    }

    pub fn delete_items(&self, ids: &QStringList) {
        let list: Vec<String> =
            QList::<QString>::from(ids).iter().map(String::from).filter(|id| !id.is_empty()).collect();
        photos::delete_items(list);
    }

    pub fn save_items(&self, ids: &QStringList, folder_url: &QString) {
        let list: Vec<String> =
            QList::<QString>::from(ids).iter().map(String::from).filter(|id| !id.is_empty()).collect();
        let folder = folder_from_url(folder_url);
        photos::save_items(list, folder);
    }

    pub fn choose_save_folder(&self, folder_url: &QString) {
        photos::set_save_folder(folder_from_url(folder_url));
    }

    pub fn item_at(&self, row: i32) -> QString {
        let Some(r) =
            usize::try_from(row).ok().and_then(|i| self.rows.get(i)).filter(|r| r.kind == CellKind::Photo)
        else {
            return QString::from("null");
        };
        QString::from(
            &json!({
                "id": r.id,
                "name": r.name,
                "date": r.date,
                "prevDate": r.prev_date,
                "size": r.size,
                "width": r.width,
                "height": r.height,
                "duration": r.duration,
                "isVideo": r.is_video,
                "album": r.album,
                "thumb": r.thumb,
                "fullUrl": r.full_url,
            })
            .to_string(),
        )
    }

    pub fn set_columns(&self, columns: i32) {
        photos::set_columns(u32::try_from(columns).unwrap_or(0));
    }

    pub fn step(&self, row: i32, delta: i32) -> i32 {
        let Ok(start) = usize::try_from(row) else { return -1 };
        let mut left = delta.unsigned_abs();
        let mut at = start;
        while left > 0 {
            at = match if delta > 0 { at.checked_add(1) } else { at.checked_sub(1) } {
                Some(next) if next < self.rows.len() => next,
                _ => return -1,
            };
            if self.rows[at].kind == CellKind::Photo {
                left -= 1;
            }
        }
        i32::try_from(at).unwrap_or(-1)
    }

    pub fn row_of(&self, id: &QString) -> i32 {
        let id = String::from(id);
        self.rows
            .iter()
            .position(|r| r.kind == CellKind::Photo && r.id == id)
            .and_then(|i| i32::try_from(i).ok())
            .unwrap_or(-1)
    }

    pub fn ordinal(&self, row: i32) -> i32 {
        let Some(end) = usize::try_from(row).ok().filter(|&r| r < self.rows.len()) else { return 0 };
        if self.rows[end].kind != CellKind::Photo {
            return 0;
        }
        let n = self.rows[..=end].iter().filter(|r| r.kind == CellKind::Photo).count();
        i32::try_from(n).unwrap_or(0)
    }

    pub fn id_at(&self, row: i32) -> QString {
        usize::try_from(row)
            .ok()
            .and_then(|i| self.rows.get(i))
            .filter(|r| r.kind == CellKind::Photo)
            .map(|r| QString::from(&r.id))
            .unwrap_or_default()
    }

    pub fn date_at(&self, row: i32) -> f64 {
        usize::try_from(row).ok().and_then(|i| self.rows.get(i)).map_or(0.0, |r| r.date as f64)
    }

    pub fn start_drag(&self, id: &QString) -> bool {
        let Some(path) = photos::stage_for_drag(&String::from(id)) else {
            return false;
        };
        let path_str = path.to_string_lossy();
        crate::bridge::native::ffi::start_external_drag(&path_str, "")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_describe_gallery_item() {
        let row = ItemRow {
            kind: photos::CellKind::Photo,
            id: "media:42".into(),
            name: "IMG_0042.jpg".into(),
            date: 1_710_000_000_000,
            prev_date: 1_709_900_000_000,
            size: 204_800,
            width: 4000,
            height: 3000,
            duration: 0,
            is_video: false,
            album: "bucket:1".into(),
            thumb: "file:///C:/cache/thumb.jpg".into(),
            full_url: String::new(),
        };
        let text = |role| role_value(&row, role).value::<QString>().map(String::from).unwrap_or_default();
        assert_eq!(text("itemId"), "media:42");
        assert_eq!(text("name"), "IMG_0042.jpg");
        assert_eq!(text("thumb"), "file:///C:/cache/thumb.jpg");
        assert_eq!(role_value(&row, "itemWidth").value::<i32>(), Some(4000));
        assert_eq!(role_value(&row, "isVideo").value::<bool>(), Some(false));
    }
}
