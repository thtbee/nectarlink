// SPDX-License-Identifier: GPL-3.0-or-later
//! `ChatModel`: the chat with one paired device (#698), notes oldest first,
//! for the Chat page; and what the page asks of it (send, change, delete).
//! The notes are timeline entries of kind `message`, so the model reads them
//! from the timeline and follows its changes.

use std::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QList, QModelIndex, QString, QVariant};
use nectarlink_core::{DeviceId, LinkState, TimelineEntry, TimelineKind, TimelineQuery};
use serde_json::json;

use super::{Edit, diff};
use crate::{chat, core_host, state::Changes};

/// Notes read at first, and how many more each `loadMore` adds.
const PAGE_SIZE: u32 = 200;

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

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[base = QAbstractListModel]
        #[qml_element]
        #[qml_singleton]
        /// The device whose chat is shown ("" for none).
        #[qproperty(QString, device)]
        #[qproperty(QString, device_name)]
        /// It's connected now.
        #[qproperty(bool, online)]
        /// Its app takes chat notes (as last heard).
        #[qproperty(bool, supported)]
        #[qproperty(i32, count)]
        /// Older notes can be loaded.
        #[qproperty(bool, has_more)]
        /// A note is on its way.
        #[qproperty(bool, sending)]
        /// Goes up whenever any chat changes, for what shows a chat's latest note.
        #[qproperty(i32, revision)]
        type ChatModel = super::ChatModelRust;
    }

    // Inherited from QAbstractListModel.
    unsafe extern "RustQt" {
        #[inherit]
        #[cxx_name = "beginInsertRows"]
        unsafe fn begin_insert_rows(self: Pin<&mut ChatModel>, parent: &QModelIndex, first: i32, last: i32);
        #[inherit]
        #[cxx_name = "endInsertRows"]
        unsafe fn end_insert_rows(self: Pin<&mut ChatModel>);
        #[inherit]
        #[cxx_name = "beginRemoveRows"]
        unsafe fn begin_remove_rows(self: Pin<&mut ChatModel>, parent: &QModelIndex, first: i32, last: i32);
        #[inherit]
        #[cxx_name = "endRemoveRows"]
        unsafe fn end_remove_rows(self: Pin<&mut ChatModel>);
        #[inherit]
        fn index(self: &ChatModel, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex;

        #[inherit]
        #[qsignal]
        #[cxx_name = "dataChanged"]
        fn data_changed(
            self: Pin<&mut ChatModel>,
            top_left: &QModelIndex,
            bottom_right: &QModelIndex,
            roles: &QList_i32,
        );
    }

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qinvokable]
        #[cxx_override]
        fn data(self: &ChatModel, index: &QModelIndex, role: i32) -> QVariant;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &ChatModel) -> QHash_i32_QByteArray;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &ChatModel, parent: &QModelIndex) -> i32;

        /// Sends `text` as a note to the device shown.
        #[qinvokable]
        fn send(self: Pin<&mut ChatModel>, text: &QString);
        /// Changes the text of a note this PC sent.
        #[qinvokable]
        fn edit(self: &ChatModel, note_id: &QString, text: &QString);
        /// Deletes a note, here and on the device.
        #[qinvokable]
        fn remove(self: &ChatModel, note_id: &QString);
        /// Reads older notes too, when `has_more`.
        #[qinvokable]
        fn load_more(self: Pin<&mut ChatModel>);
        /// Whether the chat shown is in front (its page in the active window),
        /// so a note that comes in needs no notification.
        #[qinvokable]
        fn set_in_front(self: &ChatModel, on: bool);
        /// The latest note with `device` as JSON (`{ text, incoming, timestamp }`),
        /// or "" when there's none.
        #[qinvokable]
        fn latest(self: &ChatModel, device: &QString) -> QString;

        /// A note couldn't be sent: its text, to put back in the box.
        #[qsignal]
        fn send_failed(self: Pin<&mut ChatModel>, text: QString);
    }

    impl cxx_qt::Threading for ChatModel {}
    impl cxx_qt::Initialize for ChatModel {}
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    entry_id: i64,
    note_id: String,
    text: String,
    incoming: bool,
    timestamp: i64,
    prev_timestamp: i64,
}

pub struct ChatModelRust {
    device: QString,
    device_name: QString,
    online: bool,
    supported: bool,
    count: i32,
    has_more: bool,
    sending: bool,
    revision: i32,
    rows: Vec<Row>,
    /// How many of the latest notes are read.
    limit: u32,
}

impl Default for ChatModelRust {
    fn default() -> Self {
        Self {
            device: QString::default(),
            device_name: QString::default(),
            online: false,
            supported: false,
            count: 0,
            has_more: false,
            sending: false,
            revision: 0,
            rows: Vec::new(),
            limit: PAGE_SIZE,
        }
    }
}

const ROLES: &[&str] = &["entryId", "noteId", "text", "incoming", "timestamp", "prevTimestamp"];
const USER_ROLE: i32 = 0x0100;

fn role_value(row: &Row, role: &str) -> QVariant {
    let text = |s: &str| QVariant::from(&QString::from(s));
    match role {
        "entryId" => text(&row.entry_id.to_string()),
        "noteId" => text(&row.note_id),
        "text" => text(&row.text),
        "incoming" => QVariant::from(&row.incoming),
        "timestamp" => QVariant::from(&(row.timestamp as f64)),
        "prevTimestamp" => QVariant::from(&(row.prev_timestamp as f64)),
        _ => QVariant::default(),
    }
}

/// The notes as rows, oldest first, from timeline entries newest first.
fn build_rows(newest_first: &[TimelineEntry]) -> Vec<Row> {
    let mut prev = 0;
    newest_first
        .iter()
        .rev()
        .filter_map(|e| {
            let note_id = e.ref_id.as_deref().and_then(chat::note_id)?;
            let row = Row {
                entry_id: e.id,
                note_id: note_id.to_owned(),
                text: e.detail.clone(),
                incoming: e.incoming,
                timestamp: e.timestamp,
                prev_timestamp: prev,
            };
            prev = e.timestamp;
            Some(row)
        })
        .collect()
}

impl cxx_qt::Initialize for qobject::ChatModel {
    fn initialize(mut self: Pin<&mut Self>) {
        self.as_mut().on_device_changed(Self::device_switched).release();
        super::subscribe(
            self.qt_thread(),
            Changes::TIMELINE | Changes::DEVICES | Changes::CAPABILITIES | Changes::STATUS,
            Self::refresh,
        );
    }
}

impl qobject::ChatModel {
    fn peer(&self) -> Option<DeviceId> {
        super::parse_device(&self.device)
    }

    fn device_switched(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().limit = PAGE_SIZE;
        chat::set_shown(self.peer());
        self.refresh();
    }

    fn refresh(mut self: Pin<&mut Self>) {
        // Never negative: QML reads it as `revision >= 0`.
        let revision = self.revision.checked_add(1).unwrap_or(0);
        self.as_mut().set_revision(revision);
        let peer = self.peer();
        let (name, online) = peer
            .map(|id| {
                core_host::host().hub.read(|s| {
                    let online = s.device(&id).is_some_and(|d| matches!(d.link, LinkState::Online { .. }));
                    (s.name_of(&id).unwrap_or_default(), online)
                })
            })
            .unwrap_or_default();
        let node = core_host::node();
        let supported = match (peer, &node) {
            (Some(id), Some(node)) => node.peer_offers(id, nectarlink_protocol::messages::chat::CAP),
            _ => false,
        };
        self.as_mut().set_device_name(QString::from(&name));
        self.as_mut().set_online(online);
        self.as_mut().set_supported(supported);

        let (entries, has_more) = match (peer, &node) {
            (Some(id), Some(node)) => read_notes(node, id, self.limit),
            _ => (Vec::new(), false),
        };
        self.as_mut().set_has_more(has_more);
        let new = build_rows(&entries);
        let root = QModelIndex::default();
        for edit in diff(&self.rows, &new, |r| r.entry_id) {
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

    pub fn send(mut self: Pin<&mut Self>, text: &QString) {
        let text = String::from(text);
        let Some(peer) = self.peer() else { return };
        if text.trim().is_empty() || self.sending {
            return;
        }
        self.as_mut().set_sending(true);
        let qt = self.qt_thread();
        chat::send(peer, text, move |failed| {
            let _ = qt.queue(move |mut object| {
                object.as_mut().set_sending(false);
                if let Some(text) = failed {
                    object.send_failed(QString::from(&text));
                }
            });
        });
    }

    pub fn edit(&self, note_id: &QString, text: &QString) {
        if let Some(peer) = self.peer() {
            chat::edit(peer, String::from(note_id), String::from(text));
        }
    }

    pub fn remove(&self, note_id: &QString) {
        if let Some(peer) = self.peer() {
            chat::delete(peer, String::from(note_id));
        }
    }

    pub fn load_more(mut self: Pin<&mut Self>) {
        if !self.has_more {
            return;
        }
        let limit = self.limit.saturating_add(PAGE_SIZE);
        self.as_mut().rust_mut().limit = limit;
        self.refresh();
    }

    pub fn set_in_front(&self, on: bool) {
        chat::set_in_front(on);
    }

    pub fn latest(&self, device: &QString) -> QString {
        let (Some(peer), Some(node)) = (super::parse_device(device), core_host::node()) else {
            return QString::default();
        };
        let query = TimelineQuery {
            kind: Some(TimelineKind::Message),
            device: Some(peer),
            limit: 1,
            ..TimelineQuery::default()
        };
        match node.timeline_page(&query).ok().and_then(|p| p.entries.into_iter().next()) {
            Some(e) => QString::from(
                &json!({ "text": e.detail, "incoming": e.incoming, "timestamp": e.timestamp }).to_string(),
            ),
            None => QString::default(),
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
}

/// The latest `limit` notes with `peer`, newest first, and whether there are older ones.
fn read_notes(node: &nectarlink_core::Node, peer: DeviceId, limit: u32) -> (Vec<TimelineEntry>, bool) {
    let mut entries = Vec::new();
    let mut has_more = false;
    let mut offset = 0u32;
    while offset < limit {
        let chunk = (limit - offset).min(nectarlink_core::MAX_TIMELINE_PAGE_LIMIT);
        let query = TimelineQuery {
            kind: Some(TimelineKind::Message),
            device: Some(peer),
            search: None,
            offset,
            limit: chunk,
        };
        let Ok(page) = node.timeline_page(&query) else { break };
        let fetched = page.entries.len() as u32;
        has_more = page.has_more;
        entries.extend(page.entries);
        if !has_more || fetched == 0 {
            break;
        }
        offset += fetched;
    }
    (entries, has_more)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: i64, ref_id: Option<&str>, timestamp: i64, incoming: bool) -> TimelineEntry {
        TimelineEntry {
            id,
            kind: TimelineKind::Message,
            device_id: DeviceId([1; 32]),
            device_name: "Pixel".into(),
            incoming,
            timestamp,
            title: String::new(),
            detail: format!("note {id}"),
            target: String::new(),
            size_bytes: 0,
            duration_secs: 0,
            ref_id: ref_id.map(Into::into),
            clip_available: false,
            image_data_url: None,
        }
    }

    #[test]
    fn notes_read_newest_first_show_oldest_first_with_the_one_before() {
        let entries = [
            note(3, Some("chat:c"), 300, false),
            note(2, None, 200, true),
            note(1, Some("chat:a"), 100, true),
        ];
        let rows = build_rows(&entries);
        assert_eq!(rows.len(), 2, "an entry without a note ID isn't a note");
        assert_eq!((rows[0].note_id.as_str(), rows[0].prev_timestamp, rows[0].incoming), ("a", 0, true));
        assert_eq!((rows[1].note_id.as_str(), rows[1].prev_timestamp, rows[1].incoming), ("c", 100, false));
        assert_eq!(rows[1].text, "note 3");
    }
}
