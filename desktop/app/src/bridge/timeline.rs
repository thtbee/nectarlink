// SPDX-License-Identifier: GPL-3.0-or-later
//! `TimelineModel`: paged, searchable local history of everything that moved
//! between this PC and paired devices (files, folders, clipboard items, links,
//! saved photos, voice recordings, and screen mirroring / webcam sessions).
//! Loads 100 entries per page on demand so QML never instantiates all 5,000
//! retained entries at once.

use std::{path::Path, pin::Pin};

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QList, QModelIndex, QString, QVariant};
use nectarlink_core::{DeviceId, Error, LinkState, TimelineEntry, TimelineKind, TimelineQuery};
use serde_json::json;

use super::{Edit, diff};
use crate::{
    bridge::app::{describe, show_message},
    core_host,
    state::Changes,
};

const PAGE_SIZE: u32 = 100;

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
        #[qproperty(i32, count)]
        #[qproperty(bool, has_more)]
        #[qproperty(QString, search_query)]
        #[qproperty(QString, kind_filter)]
        #[qproperty(QString, device_filter)]
        type TimelineModel = super::TimelineModelRust;
    }

    // Inherited from QAbstractListModel.
    unsafe extern "RustQt" {
        #[inherit]
        #[cxx_name = "beginInsertRows"]
        unsafe fn begin_insert_rows(
            self: Pin<&mut TimelineModel>,
            parent: &QModelIndex,
            first: i32,
            last: i32,
        );
        #[inherit]
        #[cxx_name = "endInsertRows"]
        unsafe fn end_insert_rows(self: Pin<&mut TimelineModel>);
        #[inherit]
        #[cxx_name = "beginRemoveRows"]
        unsafe fn begin_remove_rows(
            self: Pin<&mut TimelineModel>,
            parent: &QModelIndex,
            first: i32,
            last: i32,
        );
        #[inherit]
        #[cxx_name = "endRemoveRows"]
        unsafe fn end_remove_rows(self: Pin<&mut TimelineModel>);
        #[inherit]
        fn index(self: &TimelineModel, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex;

        #[inherit]
        #[qsignal]
        #[cxx_name = "dataChanged"]
        fn data_changed(
            self: Pin<&mut TimelineModel>,
            top_left: &QModelIndex,
            bottom_right: &QModelIndex,
            roles: &QList_i32,
        );
    }

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qinvokable]
        #[cxx_override]
        fn data(self: &TimelineModel, index: &QModelIndex, role: i32) -> QVariant;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &TimelineModel) -> QHash_i32_QByteArray;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &TimelineModel, parent: &QModelIndex) -> i32;

        /// Loads the next 100-entry page when `has_more` is true.
        #[qinvokable]
        fn load_more(self: Pin<&mut TimelineModel>);
        /// Opens a file, photo, recording, or web link in its default application.
        #[qinvokable]
        fn open_entry(self: &TimelineModel, entry_id: &QString);
        /// Reveals a saved file, folder, photo, or recording in File Explorer.
        #[qinvokable]
        fn reveal_entry(self: &TimelineModel, entry_id: &QString);
        /// Copies a clip, link URL, or saved photo to the Windows clipboard.
        #[qinvokable]
        fn copy_entry(self: &TimelineModel, entry_id: &QString);
        /// Sends a file, folder, clip, or link again to `device` (or the entry's device when `""`).
        #[qinvokable]
        fn send_again(self: &TimelineModel, entry_id: &QString, device: &QString);
        /// Removes one entry from the local timeline.
        #[qinvokable]
        fn remove_entry(self: Pin<&mut TimelineModel>, entry_id: &QString);
        /// Clears the entire local timeline.
        #[qinvokable]
        fn clear_all(self: Pin<&mut TimelineModel>);
    }

    impl cxx_qt::Threading for TimelineModel {}
    impl cxx_qt::Initialize for TimelineModel {}
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    entry: TimelineEntry,
    prev_timestamp: i64,
    device_name: String,
    title: String,
    subtitle: String,
    thumb_url: String,
    primary_path: String,
    item_count: u32,
    can_open: bool,
    can_show_in_folder: bool,
    can_copy: bool,
    can_send_again: bool,
}

#[derive(Default)]
pub struct TimelineModelRust {
    count: i32,
    has_more: bool,
    search_query: QString,
    kind_filter: QString,
    device_filter: QString,
    rows: Vec<Row>,
}

const ROLES: &[&str] = &[
    "entryId",
    "kind",
    "direction",
    "incoming",
    "deviceId",
    "deviceName",
    "timestamp",
    "prevTimestamp",
    "title",
    "subtitle",
    "sizeBytes",
    "durationSecs",
    "itemCount",
    "thumbUrl",
    "canOpen",
    "canShowInFolder",
    "canCopy",
    "canSendAgain",
];
const USER_ROLE: i32 = 0x0100;

fn role_value(row: &Row, role: &str) -> QVariant {
    let text = |s: &str| QVariant::from(&QString::from(s));
    let e = &row.entry;
    match role {
        "entryId" => text(&e.id.to_string()),
        "kind" => text(e.kind.as_str()),
        "direction" => text(if e.incoming { "incoming" } else { "outgoing" }),
        "incoming" => QVariant::from(&e.incoming),
        "deviceId" => text(&e.device_id.to_string()),
        "deviceName" => text(&row.device_name),
        "timestamp" => QVariant::from(&(e.timestamp as f64)),
        "prevTimestamp" => QVariant::from(&(row.prev_timestamp as f64)),
        "title" => text(&row.title),
        "subtitle" => text(&row.subtitle),
        "sizeBytes" => QVariant::from(&(e.size_bytes as f64)),
        "durationSecs" => QVariant::from(&(e.duration_secs as i32)),
        "itemCount" => QVariant::from(&(row.item_count as i32)),
        "thumbUrl" => text(&row.thumb_url),
        "canOpen" => QVariant::from(&row.can_open),
        "canShowInFolder" => QVariant::from(&row.can_show_in_folder),
        "canCopy" => QVariant::from(&row.can_copy),
        "canSendAgain" => QVariant::from(&row.can_send_again),
        _ => QVariant::default(),
    }
}

impl cxx_qt::Initialize for qobject::TimelineModel {
    fn initialize(mut self: Pin<&mut Self>) {
        self.as_mut().on_search_query_changed(Self::reset_and_refresh).release();
        self.as_mut().on_kind_filter_changed(Self::reset_and_refresh).release();
        self.as_mut().on_device_filter_changed(Self::reset_and_refresh).release();
        super::subscribe(
            self.qt_thread(),
            Changes::TIMELINE | Changes::DEVICES | Changes::CLIPBOARD | Changes::STATUS,
            Self::refresh,
        );
    }
}

impl qobject::TimelineModel {
    fn current_query(&self, offset: u32, limit: u32) -> TimelineQuery {
        let search_raw = String::from(&self.search_query);
        let search = search_raw.trim();
        let kind_raw = String::from(&self.kind_filter);
        let device_raw = String::from(&self.device_filter);
        TimelineQuery {
            kind: TimelineKind::from_str_opt(kind_raw.trim()),
            device: device_raw.trim().parse::<DeviceId>().ok(),
            search: (!search.is_empty()).then(|| search.to_owned()),
            offset,
            limit,
        }
    }

    fn reset_and_refresh(self: Pin<&mut Self>) {
        self.refresh_with_limit(PAGE_SIZE);
    }

    fn refresh(self: Pin<&mut Self>) {
        let loaded = (self.rows.len() as u32).clamp(PAGE_SIZE, 5_000);
        self.refresh_with_limit(loaded);
    }

    fn refresh_with_limit(mut self: Pin<&mut Self>, limit: u32) {
        let Some(node) = core_host::node() else { return };
        let mut all_entries = Vec::new();
        let mut has_more = false;
        let mut offset = 0u32;
        while offset < limit {
            let chunk = (limit - offset).min(nectarlink_core::MAX_TIMELINE_PAGE_LIMIT);
            let query = self.current_query(offset, chunk);
            let Ok(page) = node.timeline_page(&query) else { break };
            let fetched = page.entries.len() as u32;
            has_more = page.has_more;
            all_entries.extend(page.entries);
            if !has_more || fetched == 0 {
                break;
            }
            offset += fetched;
        }
        let new = build_rows(&all_entries, None);
        self.as_mut().set_has_more(has_more);

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

    pub fn load_more(mut self: Pin<&mut Self>) {
        if !self.has_more {
            return;
        }
        let Some(node) = core_host::node() else { return };
        let offset = self.rows.len() as u32;
        let query = self.current_query(offset, PAGE_SIZE);
        let Ok(page) = node.timeline_page(&query) else { return };
        self.as_mut().set_has_more(page.has_more);

        if page.entries.is_empty() {
            return;
        }
        let last_ts = self.rows.last().map(|r| r.entry.timestamp);
        let appended = build_rows(&page.entries, last_ts);
        let start = self.rows.len() as i32;
        let end = start + (appended.len() as i32) - 1;
        let root = QModelIndex::default();
        // SAFETY: appending a contiguous slice at the end of `rows`.
        unsafe {
            self.as_mut().begin_insert_rows(&root, start, end);
            self.as_mut().rust_mut().rows.extend(appended);
            self.as_mut().end_insert_rows();
        }
        let count = self.rows.len() as i32;
        self.set_count(count);
    }

    pub fn open_entry(&self, entry_id: &QString) {
        let Some(row) = self.find_row(entry_id) else { return };
        match row.entry.kind {
            TimelineKind::File | TimelineKind::Photo | TimelineKind::Recording => {
                if !row.primary_path.is_empty() {
                    let path = Path::new(&row.primary_path);
                    if path.exists() {
                        crate::transfers::open(path);
                    } else {
                        show_message("That file was moved or deleted.");
                    }
                }
            }
            TimelineKind::Link => {
                let url = if row.entry.target.is_empty() { &row.entry.title } else { &row.entry.target };
                if let Err(e) = crate::links::open_here(url) {
                    show_message(e);
                }
            }
            TimelineKind::Clip | TimelineKind::Session => {}
        }
    }

    pub fn reveal_entry(&self, entry_id: &QString) {
        let Some(row) = self.find_row(entry_id) else { return };
        if !row.primary_path.is_empty() {
            let path = Path::new(&row.primary_path);
            if path.exists() {
                crate::transfers::show_in_folder(path);
            } else {
                show_message("That file was moved or deleted.");
            }
        }
    }

    pub fn copy_entry(&self, entry_id: &QString) {
        let Some(row) = self.find_row(entry_id) else { return };
        match row.entry.kind {
            TimelineKind::Clip => {
                if let Some(clip_id) = row.entry.ref_id.as_deref() {
                    crate::clipboard::copy_history_item(clip_id);
                }
            }
            TimelineKind::Link => {
                let url = if row.entry.target.is_empty() { &row.entry.title } else { &row.entry.target };
                match crate::win::clipboard::write(url) {
                    Ok(()) => show_message("Copied link to clipboard."),
                    Err(e) => show_message(e),
                }
            }
            TimelineKind::Photo => {
                let path = Path::new(&row.primary_path);
                if !path.exists() {
                    return show_message("That photo was moved or deleted.");
                }
                let mime =
                    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
                        Some("png") => "image/png",
                        _ => "image/jpeg",
                    };
                match std::fs::read(path)
                    .map_err(|e| e.to_string())
                    .and_then(|bytes| crate::win::clipboard::write_image(mime, &bytes))
                {
                    Ok(()) => show_message("Copied to clipboard."),
                    Err(_) => show_message("The photo couldn't be copied to the clipboard."),
                }
            }
            _ => {}
        }
    }

    pub fn send_again(&self, entry_id: &QString, device: &QString) {
        let Some(row) = self.find_row(entry_id) else { return };
        let target_device = resolve_target_device(device, Some(row.entry.device_id));
        let Some(target_device) = target_device else {
            return show_message("No paired phone is connected right now.");
        };
        let device_name =
            core_host::host().hub.read(|s| s.name_of(&target_device)).unwrap_or_else(|| "your phone".into());

        match row.entry.kind {
            TimelineKind::File | TimelineKind::Photo | TimelineKind::Recording => {
                let paths = existing_paths(&row.entry.target);
                if paths.is_empty() {
                    return show_message("That file was moved or deleted.");
                }
                crate::transfers::send(target_device, paths);
                show_message(format!("Sending to {device_name}…"));
            }
            TimelineKind::Clip => {
                let Some(clip_id) = row.entry.ref_id.clone() else { return };
                let Some(node) = core_host::node() else { return };
                core_host::spawn(async move {
                    match node.resend_clipboard_history(target_device, &clip_id).await {
                        Ok(()) => show_message(format!("Sent clip to {device_name}.")),
                        Err(Error::NotFound) => show_message("That clip is no longer in clipboard history."),
                        Err(e) => show_message(describe(&e)),
                    }
                });
            }
            TimelineKind::Link => {
                let url = if row.entry.target.is_empty() {
                    row.entry.title.clone()
                } else {
                    row.entry.target.clone()
                };
                if !url.is_empty() {
                    crate::links::send_to(target_device, url);
                }
            }
            TimelineKind::Session => {}
        }
    }

    pub fn remove_entry(self: Pin<&mut Self>, entry_id: &QString) {
        let Ok(id) = String::from(entry_id).parse::<i64>() else { return };
        if let Some(node) = core_host::node() {
            let _ = node.delete_timeline_entry(id);
        }
    }

    pub fn clear_all(self: Pin<&mut Self>) {
        if let Some(node) = core_host::node() {
            let _ = node.clear_timeline();
        }
    }

    fn find_row(&self, entry_id: &QString) -> Option<&Row> {
        let id = String::from(entry_id).parse::<i64>().ok()?;
        self.rows.iter().find(|r| r.entry.id == id)
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

fn resolve_target_device(requested: &QString, fallback: Option<DeviceId>) -> Option<DeviceId> {
    if let Some(id) = super::parse_device(requested) {
        return Some(id);
    }
    core_host::host().hub.read(|s| {
        if let Some(id) = fallback
            && s.devices.iter().any(|d| d.id == id && matches!(d.link, LinkState::Online { .. }))
        {
            return Some(id);
        }
        s.devices.iter().find(|d| matches!(d.link, LinkState::Online { .. })).map(|d| d.id).or(fallback)
    })
}

fn existing_paths(target: &str) -> Vec<std::path::PathBuf> {
    target
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .filter(|p| p.exists())
        .collect()
}

fn build_rows(entries: &[TimelineEntry], initial_prev_ts: Option<i64>) -> Vec<Row> {
    let device_names: std::collections::HashMap<DeviceId, String> =
        core_host::host().hub.read(|s| s.devices.iter().map(|d| (d.id, d.info.name.clone())).collect());

    let mut rows = Vec::with_capacity(entries.len());
    let mut prev_ts = initial_prev_ts.unwrap_or(0);
    for entry in entries {
        let device_name = device_names
            .get(&entry.device_id)
            .cloned()
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| entry.device_name.clone());
        let (title, subtitle, thumb_url, primary_path, item_count, can_open, can_show, can_copy, can_send) =
            describe_entry(entry, &device_name);
        rows.push(Row {
            entry: entry.clone(),
            prev_timestamp: prev_ts,
            device_name,
            title,
            subtitle,
            thumb_url,
            primary_path,
            item_count,
            can_open,
            can_show_in_folder: can_show,
            can_copy,
            can_send_again: can_send,
        });
        prev_ts = entry.timestamp;
    }
    rows
}

fn describe_entry(
    entry: &TimelineEntry,
    device_name: &str,
) -> (String, String, String, String, u32, bool, bool, bool, bool) {
    let dir_verb = match (entry.kind, entry.incoming) {
        (TimelineKind::Link, true) => "Opened from",
        (TimelineKind::Link, false) => "Opened on",
        (TimelineKind::Photo, _) => "Saved from",
        (TimelineKind::Recording, _) => "Recorded on",
        (TimelineKind::Session, _) => "With",
        (_, true) => "From",
        (_, false) => "Sent to",
    };
    let dev = if device_name.is_empty() { "device" } else { device_name };
    let who = format!("{dir_verb} {dev}");

    match entry.kind {
        TimelineKind::File | TimelineKind::Photo | TimelineKind::Recording => {
            let paths = existing_paths(&entry.target);
            let item_count =
                entry.target.lines().map(str::trim).filter(|s| !s.is_empty()).count().max(1) as u32;
            let primary_path = paths
                .first()
                .map(|p| p.to_string_lossy().into_owned())
                .or_else(|| {
                    entry.target.lines().next().map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned)
                })
                .unwrap_or_default();
            let exists = !paths.is_empty();
            let title = if entry.title.is_empty() {
                match entry.kind {
                    TimelineKind::Photo => "Photo".into(),
                    TimelineKind::Recording => "Voice recording".into(),
                    _ => "File".into(),
                }
            } else {
                entry.title.clone()
            };
            let mut parts = vec![who];
            if entry.size_bytes > 0 {
                parts.push(format_bytes(entry.size_bytes));
            }
            if !entry.detail.is_empty() {
                parts.push(entry.detail.clone());
            }
            let thumb = entry.image_data_url.clone().unwrap_or_else(|| {
                if entry.kind == TimelineKind::Photo && exists {
                    let p = Path::new(&primary_path);
                    let ext = p
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(str::to_ascii_lowercase)
                        .unwrap_or_default();
                    if matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp") {
                        return crate::icons::file_url(p);
                    }
                }
                String::new()
            });
            let is_image_photo = entry.kind == TimelineKind::Photo
                && exists
                && Path::new(&primary_path)
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "jpg" | "jpeg" | "png"));
            (
                title,
                parts.join(" · "),
                thumb,
                primary_path,
                item_count,
                exists,
                exists,
                is_image_photo,
                exists,
            )
        }
        TimelineKind::Clip => {
            let clip_thumb = entry
                .ref_id
                .as_deref()
                .and_then(crate::clipboard::thumb_for_clip)
                .or_else(|| entry.image_data_url.clone())
                .unwrap_or_default();
            let available = entry.clip_available;
            let title = if !entry.title.is_empty() && entry.title != "Image clip" {
                entry.title.lines().next().unwrap_or(&entry.title).trim().to_owned()
            } else if !clip_thumb.is_empty() || entry.detail == "image" {
                "Copied image".into()
            } else {
                "Clip (removed from clipboard history)".into()
            };
            let mut parts = vec![who];
            if entry.size_bytes > 0 {
                parts.push(format_bytes(entry.size_bytes));
            }
            (title, parts.join(" · "), clip_thumb, String::new(), 1, false, false, available, available)
        }
        TimelineKind::Link => {
            let url = if entry.target.is_empty() { entry.title.clone() } else { entry.target.clone() };
            let title = if entry.title.is_empty() { url.clone() } else { entry.title.clone() };
            let mut parts = vec![who];
            if !entry.detail.is_empty() {
                parts.push(entry.detail.clone());
            }
            let valid = url.starts_with("http://") || url.starts_with("https://");
            (title, parts.join(" · "), String::new(), String::new(), 1, valid, false, !url.is_empty(), valid)
        }
        TimelineKind::Session => {
            let title = if entry.title.is_empty() { "Session".into() } else { entry.title.clone() };
            let mut parts = vec![who];
            if !entry.detail.is_empty() {
                parts.push(entry.detail.clone());
            }
            if entry.duration_secs > 0 {
                parts.push(format_duration(entry.duration_secs as u32));
            } else {
                parts.push("In progress".into());
            }
            (title, parts.join(" · "), String::new(), String::new(), 1, false, false, false, false)
        }
    }
}

fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

fn format_duration(secs: u32) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}h {m:02}m")
    } else if m > 0 {
        format!("{m}m {s:02}s")
    } else {
        format!("{s}s")
    }
}

/// Compact JSON summary of the latest 4 timeline items for the Home page card.
pub fn preview_json() -> String {
    let Some(node) = core_host::node() else {
        return "[]".into();
    };
    let Ok(page) = node.timeline_page(&TimelineQuery { limit: 4, ..TimelineQuery::default() }) else {
        return "[]".into();
    };
    let rows = build_rows(&page.entries, None);
    let items: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|r| {
            json!({
                "id": r.entry.id.to_string(),
                "kind": r.entry.kind.as_str(),
                "incoming": r.entry.incoming,
                "deviceName": r.device_name,
                "timestamp": r.entry.timestamp,
                "title": r.title,
                "subtitle": r.subtitle,
                "thumbUrl": r.thumb_url,
            })
        })
        .collect();
    serde_json::Value::Array(items).to_string()
}
