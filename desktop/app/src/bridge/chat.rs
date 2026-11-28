// SPDX-License-Identifier: GPL-3.0-or-later
//! `ChatModel`: the chat with one paired device (#698), oldest first, for the
//! Chat page; and what the page asks of it (send, change, delete). The notes
//! are timeline entries of kind `message`, so the model reads them from the
//! timeline and follows its changes. The files, photos and recordings that
//! went between the two show among them as attachments, from the same
//! timeline.

use std::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QList, QModelIndex, QString, QVariant};
use nectarlink_core::{DeviceId, LinkState, TimelineEntry, TimelineKind, TimelineQuery};
use serde_json::json;

use super::{
    Edit, diff,
    timeline::{self, Attachment},
};
use crate::{chat, core_host, state::Changes};

/// Items read at first, and how many more each `loadMore` adds.
const PAGE_SIZE: u32 = 200;

/// The timeline kinds the chat shows: notes, and what was sent as files.
const KINDS: [TimelineKind; 4] =
    [TimelineKind::Message, TimelineKind::File, TimelineKind::Photo, TimelineKind::Recording];

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
        /// Older items can be loaded.
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
        /// Reads older items too, when `has_more`.
        #[qinvokable]
        fn load_more(self: Pin<&mut ChatModel>);
        /// Opens an attachment's file.
        #[qinvokable]
        fn open_attachment(self: &ChatModel, entry_id: &QString);
        /// Shows an attachment's file in File Explorer.
        #[qinvokable]
        fn show_attachment_in_folder(self: &ChatModel, entry_id: &QString);
        /// Takes an attachment out of the chat (and this PC's timeline).
        #[qinvokable]
        fn remove_attachment(self: &ChatModel, entry_id: &QString);
        /// Whether the chat shown is in front (its page in the active window),
        /// so a note that comes in needs no notification.
        #[qinvokable]
        fn set_in_front(self: &ChatModel, on: bool);
        /// The latest item in the chat with `device` as JSON (`{ kind, text,
        /// incoming, timestamp }`, `text` being an attachment's name), or ""
        /// when there's none.
        #[qinvokable]
        fn latest(self: &ChatModel, device: &QString) -> QString;

        /// A note couldn't be sent: its text, to put back in the box.
        #[qsignal]
        fn send_failed(self: Pin<&mut ChatModel>, text: QString);
    }

    impl cxx_qt::Threading for ChatModel {}
    impl cxx_qt::Initialize for ChatModel {}
}

/// A note, or a file, photo or recording that went between the two.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    entry: TimelineEntry,
    /// A note's ID ("" for an attachment).
    note_id: String,
    attachment: Option<Attachment>,
    prev_timestamp: i64,
}

impl Row {
    fn kind(&self) -> &'static str {
        match self.entry.kind {
            TimelineKind::Message => "note",
            other => other.as_str(),
        }
    }
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

const ROLES: &[&str] = &[
    "entryId",
    "kind",
    "noteId",
    "text",
    "incoming",
    "timestamp",
    "prevTimestamp",
    "title",
    "size",
    "thumbUrl",
    "itemCount",
    "canOpen",
    "canShowInFolder",
];
const USER_ROLE: i32 = 0x0100;

fn role_value(row: &Row, role: &str) -> QVariant {
    let text = |s: &str| QVariant::from(&QString::from(s));
    let e = &row.entry;
    let a = row.attachment.as_ref();
    match role {
        "entryId" => text(&e.id.to_string()),
        "kind" => text(row.kind()),
        "noteId" => text(&row.note_id),
        "text" => text(if a.is_some() { "" } else { &e.detail }),
        "incoming" => QVariant::from(&e.incoming),
        "timestamp" => QVariant::from(&(e.timestamp as f64)),
        "prevTimestamp" => QVariant::from(&(row.prev_timestamp as f64)),
        "title" => text(a.map_or("", |a| &a.title)),
        "size" => text(a.map_or("", |a| &a.size)),
        "thumbUrl" => text(a.map_or("", |a| &a.thumb_url)),
        "itemCount" => QVariant::from(&(a.map_or(1, |a| a.item_count) as i32)),
        "canOpen" => QVariant::from(&a.is_some_and(|a| a.can_open)),
        "canShowInFolder" => QVariant::from(&a.is_some_and(|a| a.can_show_in_folder)),
        _ => QVariant::default(),
    }
}

/// The chat as rows, oldest first, from timeline entries newest first: its
/// notes, and the files, photos and recordings, each on the side of the
/// device that sent it (`incoming` is the other device's).
fn build_rows(newest_first: &[TimelineEntry]) -> Vec<Row> {
    let mut prev = 0;
    newest_first
        .iter()
        .rev()
        .filter_map(|e| {
            let (note_id, attachment) = match e.kind {
                TimelineKind::Message => (e.ref_id.as_deref().and_then(chat::note_id)?.to_owned(), None),
                TimelineKind::File | TimelineKind::Photo | TimelineKind::Recording => {
                    (String::new(), Some(timeline::attachment(e)))
                }
                _ => return None,
            };
            let row = Row { entry: e.clone(), note_id, attachment, prev_timestamp: prev };
            prev = e.timestamp;
            Some(row)
        })
        .collect()
}

/// The newest `limit` entries of lists read newest first, one per kind (each
/// its kind's newest `limit`, and whether that kind has older ones), newest
/// first; and whether there are older ones.
fn newest_first(lists: Vec<(Vec<TimelineEntry>, bool)>, limit: usize) -> (Vec<TimelineEntry>, bool) {
    let mut has_more = false;
    let mut all = Vec::new();
    for (entries, more) in lists {
        has_more |= more;
        all.extend(entries);
    }
    // The timeline's own order.
    all.sort_by(|a, b| b.timestamp.cmp(&a.timestamp).then(b.id.cmp(&a.id)));
    if all.len() > limit {
        all.truncate(limit);
        has_more = true;
    }
    (all, has_more)
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
            (Some(id), Some(node)) => read_chat(node, id, self.limit),
            _ => (Vec::new(), false),
        };
        self.as_mut().set_has_more(has_more);
        let new = build_rows(&entries);
        let root = QModelIndex::default();
        for edit in diff(&self.rows, &new, |r| r.entry.id) {
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

    pub fn open_attachment(&self, entry_id: &QString) {
        if let Some((row, a)) = self.find_attachment(entry_id) {
            timeline::open(&row.entry, &a.primary_path);
        }
    }

    pub fn show_attachment_in_folder(&self, entry_id: &QString) {
        if let Some((_, a)) = self.find_attachment(entry_id) {
            timeline::reveal(&a.primary_path);
        }
    }

    pub fn remove_attachment(&self, entry_id: &QString) {
        if let Some((row, _)) = self.find_attachment(entry_id) {
            timeline::remove(row.entry.id);
        }
    }

    fn find_attachment(&self, entry_id: &QString) -> Option<(&Row, &Attachment)> {
        let id = String::from(entry_id).parse::<i64>().ok()?;
        let row = self.rows.iter().find(|r| r.entry.id == id)?;
        Some((row, row.attachment.as_ref()?))
    }

    pub fn set_in_front(&self, on: bool) {
        chat::set_in_front(on);
    }

    pub fn latest(&self, device: &QString) -> QString {
        let (Some(peer), Some(node)) = (super::parse_device(device), core_host::node()) else {
            return QString::default();
        };
        let (entries, _) = read_chat(&node, peer, 1);
        match build_rows(&entries).pop() {
            Some(row) => {
                let text = row.attachment.as_ref().map_or(&row.entry.detail, |a| &a.title);
                let e = &row.entry;
                QString::from(
                    &json!({ "kind": row.kind(), "text": text, "incoming": e.incoming, "timestamp": e.timestamp })
                        .to_string(),
                )
            }
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

/// The latest `limit` items in the chat with `peer`, newest first, and
/// whether there are older ones.
fn read_chat(node: &nectarlink_core::Node, peer: DeviceId, limit: u32) -> (Vec<TimelineEntry>, bool) {
    let lists = KINDS.iter().map(|&kind| read_kind(node, peer, kind, limit)).collect();
    newest_first(lists, limit as usize)
}

/// The latest `limit` entries of `kind` with `peer`, newest first, and
/// whether there are older ones.
fn read_kind(
    node: &nectarlink_core::Node,
    peer: DeviceId,
    kind: TimelineKind,
    limit: u32,
) -> (Vec<TimelineEntry>, bool) {
    let mut entries = Vec::new();
    let mut has_more = false;
    let mut offset = 0u32;
    while offset < limit {
        let chunk = (limit - offset).min(nectarlink_core::MAX_TIMELINE_PAGE_LIMIT);
        let query =
            TimelineQuery { kind: Some(kind), device: Some(peer), search: None, offset, limit: chunk };
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
        assert_eq!(
            (rows[0].note_id.as_str(), rows[0].prev_timestamp, rows[0].entry.incoming),
            ("a", 0, true)
        );
        assert_eq!(
            (rows[1].note_id.as_str(), rows[1].prev_timestamp, rows[1].entry.incoming),
            ("c", 100, false)
        );
        assert_eq!(rows[1].entry.detail, "note 3");
    }

    fn sent(id: i64, kind: TimelineKind, timestamp: i64, incoming: bool, target: &str) -> TimelineEntry {
        TimelineEntry {
            kind,
            title: format!("file {id}"),
            detail: String::new(),
            target: target.into(),
            size_bytes: 2048,
            ref_id: Some(format!("transfer-{id}")),
            ..note(id, None, timestamp, incoming)
        }
    }

    #[test]
    fn files_photos_and_recordings_sit_among_the_notes_in_time_order() {
        let dir = tempfile::tempdir().unwrap();
        let photo = dir.path().join("beach.png");
        std::fs::write(&photo, b"png").unwrap();
        let photo = photo.to_string_lossy().into_owned();
        let (entries, has_more) = newest_first(
            vec![
                (vec![note(5, Some("chat:e"), 500, true), note(1, Some("chat:a"), 100, false)], false),
                (vec![sent(4, TimelineKind::File, 400, false, "C:\\gone\\report.pdf")], false),
                (vec![sent(3, TimelineKind::Photo, 300, true, &photo)], false),
                (vec![sent(2, TimelineKind::Recording, 200, true, "")], false),
            ],
            10,
        );
        assert!(!has_more);
        let rows = build_rows(&entries);
        let order: Vec<_> = rows.iter().map(|r| (r.entry.id, r.kind())).collect();
        assert_eq!(order, [(1, "note"), (2, "recording"), (3, "photo"), (4, "file"), (5, "note")]);
        let sides: Vec<_> = rows.iter().map(|r| r.entry.incoming).collect();
        assert_eq!(sides, [false, true, true, false, true], "each on the side of whoever sent it");
        let prev: Vec<_> = rows.iter().map(|r| r.prev_timestamp).collect();
        assert_eq!(prev, [0, 100, 200, 300, 400]);

        assert!(rows[0].attachment.is_none());
        let image = rows[2].attachment.as_ref().unwrap();
        assert_eq!(image.title, "file 3");
        assert_eq!(image.size, "2 KB");
        assert!(image.thumb_url.starts_with("file:///"), "a photo on this PC shows itself");
        assert!(image.can_open && image.can_show_in_folder);
        let gone = rows[3].attachment.as_ref().unwrap();
        assert!(gone.thumb_url.is_empty() && !gone.can_open && !gone.can_show_in_folder);
    }

    #[test]
    fn other_kinds_and_other_messages_stay_out_of_the_chat() {
        let link = TimelineEntry { kind: TimelineKind::Link, ..note(2, None, 200, true) };
        let clip = TimelineEntry { kind: TimelineKind::Clip, ..note(3, None, 300, true) };
        let rows = build_rows(&[clip, link, note(1, Some("chat:a"), 100, true)]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].note_id, "a");
    }

    #[test]
    fn only_the_newest_items_across_kinds_are_kept() {
        let (entries, has_more) = newest_first(
            vec![
                (vec![note(4, Some("chat:d"), 400, true), note(1, Some("chat:a"), 100, true)], false),
                (
                    vec![
                        sent(3, TimelineKind::File, 300, false, ""),
                        sent(2, TimelineKind::File, 300, true, ""),
                    ],
                    false,
                ),
            ],
            3,
        );
        let ids: Vec<_> = entries.iter().map(|e| e.id).collect();
        assert_eq!(ids, [4, 3, 2], "newest first, the later of two at the same second first");
        assert!(has_more, "one was left out");

        let (entries, has_more) = newest_first(vec![(vec![note(1, Some("chat:a"), 100, true)], true)], 5);
        assert_eq!(entries.len(), 1);
        assert!(has_more, "a kind with older entries means there's more");
    }
}
