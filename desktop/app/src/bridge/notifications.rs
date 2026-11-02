// SPDX-License-Identifier: GPL-3.0-or-later
//! `NotificationList`: phone notifications (newest first) as a Qt list
//! model, plus the commands QML runs on them.

use crate::i18n::tr;
use std::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QList, QModelIndex, QString, QVariant};
use nectarlink_core::DeviceId;

use super::{Edit, diff};
use crate::{
    core_host, icons,
    state::{AppRule, AppState, Changes, NotificationView, SentReply},
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
        /// Keep a day of history.
        #[qproperty(bool, history_enabled, cxx_name = "historyEnabled", READ, NOTIFY)]
        /// Apps that sent notifications, as JSON sorted by name:
        /// [{ "app", "name", "icon", "rule": "show" | "quiet" | "hidden" }].
        #[qproperty(QString, apps, READ, NOTIFY)]
        /// Non-empty when a recent dismissal can be undone in the feed.
        #[qproperty(QString, undo_label, cxx_name = "undoLabel", READ, NOTIFY)]
        type NotificationList = super::NotificationListRust;
    }

    // Inherited from QAbstractListModel.
    unsafe extern "RustQt" {
        #[inherit]
        #[cxx_name = "beginInsertRows"]
        unsafe fn begin_insert_rows(
            self: Pin<&mut NotificationList>,
            parent: &QModelIndex,
            first: i32,
            last: i32,
        );
        #[inherit]
        #[cxx_name = "endInsertRows"]
        unsafe fn end_insert_rows(self: Pin<&mut NotificationList>);
        #[inherit]
        #[cxx_name = "beginRemoveRows"]
        unsafe fn begin_remove_rows(
            self: Pin<&mut NotificationList>,
            parent: &QModelIndex,
            first: i32,
            last: i32,
        );
        #[inherit]
        #[cxx_name = "endRemoveRows"]
        unsafe fn end_remove_rows(self: Pin<&mut NotificationList>);
        #[inherit]
        fn index(self: &NotificationList, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex;

        #[inherit]
        #[qsignal]
        #[cxx_name = "dataChanged"]
        fn data_changed(
            self: Pin<&mut NotificationList>,
            top_left: &QModelIndex,
            bottom_right: &QModelIndex,
            roles: &QList_i32,
        );
    }

    unsafe extern "RustQt" {
        #[qinvokable]
        #[cxx_override]
        fn data(self: &NotificationList, index: &QModelIndex, role: i32) -> QVariant;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &NotificationList) -> QHash_i32_QByteArray;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &NotificationList, parent: &QModelIndex) -> i32;

        /// How many notifications a device has here.
        #[qinvokable]
        #[cxx_name = "countFor"]
        fn count_for(self: &NotificationList, device: &QString) -> i32;
        /// Dismisses it here and on the phone.
        #[qinvokable]
        fn dismiss(self: &NotificationList, device: &QString, key: &QString);
        /// Dismisses all notifications from one app on a device (here and on the phone).
        #[qinvokable]
        #[cxx_name = "dismissGroup"]
        fn dismiss_group(self: &NotificationList, device: &QString, app: &QString);
        /// Dismisses all of a device's notifications.
        #[qinvokable]
        #[cxx_name = "dismissAll"]
        fn dismiss_all(self: &NotificationList, device: &QString);
        /// Toggles whether an app's notification group is expanded in the feed.
        #[qinvokable]
        #[cxx_name = "toggleGroup"]
        fn toggle_group(self: &NotificationList, app: &QString);
        /// Restores the most recently dismissed notification or group to the PC feed.
        #[qinvokable]
        #[cxx_name = "undoDismiss"]
        fn undo_dismiss(self: &NotificationList);
        /// Hides the undo banner.
        #[qinvokable]
        #[cxx_name = "clearUndo"]
        fn clear_undo(self: &NotificationList);
        /// Runs one of its actions on the phone.
        #[qinvokable]
        #[cxx_name = "runAction"]
        fn run_action(self: &NotificationList, device: &QString, key: &QString, action: &QString);
        /// Sends a reply through its reply action.
        #[qinvokable]
        fn reply(self: &NotificationList, device: &QString, key: &QString, action: &QString, text: &QString);
        /// "show", "quiet" (no pop-ups) or "hidden".
        #[qinvokable]
        #[cxx_name = "appRule"]
        fn app_rule(self: &NotificationList, app: &QString) -> QString;
        #[qinvokable]
        #[cxx_name = "setAppRule"]
        fn set_app_rule(self: &NotificationList, app: &QString, rule: &QString);
        #[qinvokable]
        #[cxx_name = "setHistoryEnabled"]
        fn set_history_enabled_by_user(self: &NotificationList, on: bool);
        #[qinvokable]
        #[cxx_name = "clearHistory"]
        fn clear_history(self: &NotificationList);
        /// Copies a detected one-time code to the PC clipboard without syncing it back.
        #[qinvokable]
        #[cxx_name = "copyCode"]
        fn copy_code(self: &NotificationList, code: &QString);
    }

    impl cxx_qt::Threading for NotificationList {}
    impl cxx_qt::Initialize for NotificationList {}
}

/// A row: the notification plus what the UI needs to show it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    view: NotificationView,
    device_name: String,
    /// A file URL, or "" (the UI then shows the app's initial).
    icon_url: String,
    /// The notification's picture as a file URL, or "".
    image_url: String,
    /// Replies sent from this PC.
    replies: Vec<SentReply>,
    /// Total notifications in this `(device, app)` group (`1` if ungrouped or live).
    group_count: i32,
    /// Index within the `(device, app)` group (`0` is the newest / lead item).
    group_index: i32,
    /// Whether this `(device, app)` group is currently expanded.
    group_expanded: bool,
    /// Short preview of older notifications in the group when collapsed.
    group_summary: String,
}

#[derive(Default)]
pub struct NotificationListRust {
    count: i32,
    history_enabled: bool,
    apps: QString,
    undo_label: QString,
    rows: Vec<Row>,
}

/// Model roles (Qt::UserRole + n).
const ROLES: &[&str] = &[
    "deviceId",
    "deviceName",
    "key",
    "app",
    "appName",
    "title",
    "text",
    "sub",
    "when",
    "iconUrl",
    "imageUrl",
    "actions",
    "replyAction",
    "replyLabel",
    "silent",
    "ongoing",
    "replies",
    "otpCode",
    "live",
    "groupCount",
    "groupIndex",
    "groupExpanded",
    "groupSummary",
];
const USER_ROLE: i32 = 0x0100;

fn role_value(row: &Row, role: &str) -> QVariant {
    let text = |s: &str| QVariant::from(&QString::from(s));
    let n = &row.view.notification;
    let reply = n.actions.iter().find(|a| a.reply);
    match role {
        "deviceId" => text(&row.view.device.to_string()),
        "deviceName" => text(&row.device_name),
        "key" => text(&n.key),
        "app" => text(&n.app),
        "appName" => text(&n.app_name),
        "title" => text(n.title.as_deref().unwrap_or_default()),
        "text" => text(n.text.as_deref().unwrap_or_default()),
        "sub" => text(n.sub.as_deref().unwrap_or_default()),
        "when" => QVariant::from(&(n.when as f64)),
        "iconUrl" => text(&row.icon_url),
        "imageUrl" => text(&row.image_url),
        // Buttons other than the reply, as JSON: [{ "id", "title" }].
        "actions" => text(
            &serde_json::Value::from(
                n.actions
                    .iter()
                    .filter(|a| !a.reply)
                    .map(|a| serde_json::json!({ "id": a.id, "title": a.title }))
                    .collect::<Vec<_>>(),
            )
            .to_string(),
        ),
        "replyAction" => text(reply.map_or("", |a| a.id.as_str())),
        "replyLabel" => text(reply.map_or("", |a| a.title.as_str())),
        "silent" => QVariant::from(&n.silent),
        "ongoing" => QVariant::from(&n.ongoing),
        // As JSON: [{ "text", "pending" }].
        "replies" => text(
            &serde_json::Value::from(
                row.replies
                    .iter()
                    .map(|r| serde_json::json!({ "text": r.text, "pending": r.pending }))
                    .collect::<Vec<_>>(),
            )
            .to_string(),
        ),
        "otpCode" => text(&crate::notifications::notification_otp(n).unwrap_or_default()),
        "live" => match &n.live {
            Some(live) => text(
                &serde_json::json!({
                    "progress": live.progress.unwrap_or(0),
                    "max": live.max.unwrap_or(0),
                    "indeterminate": live.indeterminate,
                    "chip": live.chip.as_deref().unwrap_or_default(),
                    "segments": live.segments.iter().map(|s| serde_json::json!({
                        "length": s.length,
                        "color": s.color.map(|c| format!("#{:06X}", c & 0x00FF_FFFF)).unwrap_or_default(),
                    })).collect::<Vec<_>>(),
                    "points": live.points.iter().map(|p| serde_json::json!({
                        "position": p.position,
                        "color": p.color.map(|c| format!("#{:06X}", c & 0x00FF_FFFF)).unwrap_or_default(),
                    })).collect::<Vec<_>>(),
                    "chronometer": live.chronometer,
                    "countdown": live.countdown,
                    "when": n.when,
                })
                .to_string(),
            ),
            None => text(""),
        },
        "groupCount" => QVariant::from(&row.group_count),
        "groupIndex" => QVariant::from(&row.group_index),
        "groupExpanded" => QVariant::from(&row.group_expanded),
        "groupSummary" => text(&row.group_summary),
        _ => QVariant::default(),
    }
}

fn rule_name(rule: AppRule) -> &'static str {
    match rule {
        AppRule::Show => "show",
        AppRule::Quiet => "quiet",
        AppRule::Hidden => "hidden",
    }
}

/// The apps list for Settings (see the `apps` property).
fn apps_json(s: &AppState) -> String {
    let mut apps: Vec<(&String, &String)> = s.app_names.iter().collect();
    apps.sort_by_key(|(app, name)| (name.to_lowercase(), (*app).clone()));
    serde_json::Value::from(
        apps.into_iter()
            .map(|(app, name)| {
                serde_json::json!({
                    "app": app,
                    "name": name,
                    "icon": s.app_icons.get(app).map(|p| icons::file_url(p)).unwrap_or_default(),
                    "rule": rule_name(s.app_rule(app)),
                })
            })
            .collect::<Vec<_>>(),
    )
    .to_string()
}

fn make_row(
    s: &AppState,
    view: &NotificationView,
    group_count: i32,
    group_index: i32,
    group_expanded: bool,
    group_summary: String,
) -> Row {
    Row {
        device_name: s.name_of(&view.device).unwrap_or_default(),
        icon_url: s.app_icons.get(&view.notification.app).map(|p| icons::file_url(p)).unwrap_or_default(),
        image_url: s
            .notification_images
            .get(&(view.device, view.notification.key.clone()))
            .map(|p| icons::file_url(p))
            .unwrap_or_default(),
        replies: s.replies.get(&(view.device, view.notification.key.clone())).cloned().unwrap_or_default(),
        view: view.clone(),
        group_count,
        group_index,
        group_expanded,
        group_summary,
    }
}

/// Builds display rows from [`AppState`], keeping Live Updates ungrouped at the top
/// and grouping regular notifications by `(device, app)` in order of each app's
/// newest notification. Collapsed groups emit only their lead item (`group_index == 0`).
fn build_rows(s: &AppState) -> Vec<Row> {
    let visible: Vec<&NotificationView> =
        s.notifications.iter().filter(|view| s.app_rule(&view.notification.app) != AppRule::Hidden).collect();
    let mut rows = Vec::with_capacity(visible.len());
    let mut seen_groups: std::collections::HashSet<(DeviceId, String)> = std::collections::HashSet::new();

    for view in &visible {
        if view.notification.live.is_some() {
            rows.push(make_row(s, view, 1, 0, false, String::new()));
            continue;
        }
        let group_key = (view.device, view.notification.app.clone());
        if !seen_groups.insert(group_key.clone()) {
            continue;
        }
        let members: Vec<&NotificationView> = visible
            .iter()
            .copied()
            .filter(|v| {
                v.notification.live.is_none() && v.device == group_key.0 && v.notification.app == group_key.1
            })
            .collect();
        let group_count = members.len() as i32;
        let group_expanded = group_count > 1 && s.is_app_expanded(&group_key.1);
        let group_summary = if group_count > 1 {
            members[1..]
                .iter()
                .filter_map(|v| {
                    v.notification
                        .title
                        .as_deref()
                        .filter(|t| !t.trim().is_empty())
                        .or_else(|| v.notification.text.as_deref().filter(|t| !t.trim().is_empty()))
                })
                .map(str::trim)
                .take(2)
                .collect::<Vec<_>>()
                .join(" · ")
        } else {
            String::new()
        };
        rows.push(make_row(s, members[0], group_count, 0, group_expanded, group_summary));
        if group_expanded {
            for (idx, member) in members.iter().enumerate().skip(1) {
                rows.push(make_row(s, member, group_count, idx as i32, true, String::new()));
            }
        }
    }
    rows
}

fn key_of(row: &Row) -> (DeviceId, String) {
    (row.view.device, row.view.notification.key.clone())
}

impl cxx_qt::Initialize for qobject::NotificationList {
    fn initialize(self: Pin<&mut Self>) {
        super::subscribe(
            self.qt_thread(),
            Changes::NOTIFICATIONS | Changes::DEVICES | Changes::APPS | Changes::HISTORY,
            Self::refresh,
        );
    }
}

impl qobject::NotificationList {
    fn refresh(mut self: Pin<&mut Self>) {
        let (new, apps, history, undo_label): (Vec<Row>, String, bool, String) = core_host::host()
            .hub
            .read(|s| (build_rows(s), apps_json(s), s.history_enabled, s.last_dismissed_label.clone()));
        if String::from(&self.apps) != apps {
            self.as_mut().set_apps(QString::from(&apps));
        }
        if String::from(&self.undo_label) != undo_label {
            self.as_mut().set_undo_label(QString::from(&undo_label));
        }
        self.as_mut().set_history_enabled(history);
        let root = QModelIndex::default();
        for edit in diff(&self.rows, &new, key_of) {
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
        let count: i32 = self.rows.iter().filter(|r| r.group_index == 0).map(|r| r.group_count).sum();
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

    pub fn count_for(&self, device: &QString) -> i32 {
        let device = String::from(device);
        self.rows
            .iter()
            .filter(|r| r.group_index == 0 && r.view.device.to_string() == device)
            .map(|r| r.group_count)
            .sum()
    }

    pub fn dismiss(&self, device: &QString, key: &QString) {
        let dev_str = String::from(device);
        let key_str = String::from(key);
        if let Ok(dev_id) = dev_str.parse::<DeviceId>() {
            let found = core_host::host().hub.read(|s| {
                s.notifications.iter().find(|n| n.device == dev_id && n.notification.key == key_str).cloned()
            });
            // The phone won't dismiss these, so it would only come back.
            if found.as_ref().is_some_and(|v| v.notification.ongoing) {
                return;
            }
            if let Some(view) = found {
                let label = if view.notification.app_name.is_empty() {
                    tr!("Notifications", "Notification dismissed")
                } else {
                    tr!("Notifications", "Dismissed from %1", view.notification.app_name)
                };
                core_host::host().hub.update(|s| s.record_dismissed(vec![view], label));
            }
        }
        crate::notifications::dismiss(&dev_str, key_str);
    }

    pub fn dismiss_group(&self, device: &QString, app: &QString) {
        let dev_str = String::from(device);
        let app_str = String::from(app);
        let Ok(dev_id) = dev_str.parse::<DeviceId>() else { return };
        let views: Vec<NotificationView> = core_host::host().hub.read(|s| {
            s.notifications
                .iter()
                .filter(|n| n.device == dev_id && n.notification.app == app_str && !n.notification.ongoing)
                .cloned()
                .collect()
        });
        if views.is_empty() {
            return;
        }
        let app_name = views[0].notification.app_name.clone();
        let count = views.len();
        let label = if count == 1 {
            tr!("Notifications", "Dismissed from %1", app_name)
        } else {
            tr!("Notifications", "Dismissed %1 from %2", count, app_name)
        };
        let keys: Vec<String> = views.iter().map(|v| v.notification.key.clone()).collect();
        core_host::host().hub.update(|s| s.record_dismissed(views, label));
        for key in keys {
            crate::notifications::dismiss(&dev_str, key);
        }
    }

    pub fn dismiss_all(&self, device: &QString) {
        let dev_str = String::from(device);
        let Ok(dev_id) = dev_str.parse::<DeviceId>() else { return };
        let views: Vec<NotificationView> = core_host::host().hub.read(|s| {
            s.notifications
                .iter()
                .filter(|n| {
                    n.device == dev_id
                        && !n.notification.ongoing
                        && s.app_rule(&n.notification.app) != AppRule::Hidden
                })
                .cloned()
                .collect()
        });
        if views.is_empty() {
            return;
        }
        let count = views.len();
        let label = if count == 1 {
            tr!("Notifications", "Notification dismissed")
        } else {
            tr!("Notifications", "Dismissed %1 notifications", count)
        };
        let keys: Vec<String> = views.iter().map(|v| v.notification.key.clone()).collect();
        core_host::host().hub.update(|s| s.record_dismissed(views, label));
        for key in keys {
            crate::notifications::dismiss(&dev_str, key);
        }
    }

    pub fn toggle_group(&self, app: &QString) {
        let app_str = String::from(app);
        core_host::host().hub.update(|s| s.toggle_app_expanded(&app_str));
    }

    pub fn undo_dismiss(&self) {
        core_host::host().hub.update(AppState::undo_dismiss);
    }

    pub fn clear_undo(&self) {
        core_host::host().hub.update(AppState::clear_undo);
    }

    pub fn run_action(&self, device: &QString, key: &QString, action: &QString) {
        crate::notifications::run_action(
            &String::from(device),
            String::from(key),
            String::from(action),
            None,
        );
    }

    pub fn app_rule(&self, app: &QString) -> QString {
        let rule = core_host::host().hub.read(|s| s.app_rule(&String::from(app)));
        QString::from(rule_name(rule))
    }

    pub fn set_app_rule(&self, app: &QString, rule: &QString) {
        let rule = match String::from(rule).as_str() {
            "quiet" => AppRule::Quiet,
            "hidden" => AppRule::Hidden,
            _ => AppRule::Show,
        };
        crate::notifications::set_app_rule(&String::from(app), rule);
    }

    pub fn set_history_enabled_by_user(&self, on: bool) {
        core_host::host().hub.update(|s| s.set_history_enabled(on));
    }

    pub fn clear_history(&self) {
        core_host::host().hub.update(AppState::clear_history);
    }

    pub fn copy_code(&self, code: &QString) {
        crate::notifications::copy_otp(&String::from(code));
    }

    fn set_history_enabled(mut self: Pin<&mut Self>, on: bool) {
        if self.history_enabled != on {
            self.as_mut().rust_mut().history_enabled = on;
            self.history_enabled_changed();
        }
    }

    fn set_apps(mut self: Pin<&mut Self>, apps: QString) {
        self.as_mut().rust_mut().apps = apps;
        self.apps_changed();
    }

    fn set_undo_label(mut self: Pin<&mut Self>, label: QString) {
        self.as_mut().rust_mut().undo_label = label;
        self.undo_label_changed();
    }

    pub fn reply(&self, device: &QString, key: &QString, action: &QString, text: &QString) {
        let text = String::from(text).trim().to_owned();
        if text.is_empty() {
            return;
        }
        crate::notifications::run_action(
            &String::from(device),
            String::from(key),
            String::from(action),
            Some(text),
        );
    }
}

#[cfg(test)]
mod tests {
    use nectarlink_core::{NodeEvent, Notification, NotificationAction};

    use super::*;

    #[test]
    fn roles_describe_the_notification() {
        let row = Row {
            view: NotificationView {
                device: DeviceId([7; 32]),
                notification: Notification {
                    key: "k".into(),
                    app: "com.chat".into(),
                    app_name: "Chat".into(),
                    title: Some("Sam".into()),
                    text: Some("Your verification code is 482913".into()),
                    sub: None,
                    when: 1_760_000_000_000,
                    actions: vec![
                        NotificationAction { id: "r".into(), title: "Reply".into(), reply: true },
                        NotificationAction { id: "m".into(), title: "Mute".into(), reply: false },
                    ],
                    silent: false,
                    ongoing: false,
                    icon: None,
                    image: None,
                    live: Some(nectarlink_core::NotificationLive {
                        v: 1,
                        progress: Some(40),
                        max: Some(100),
                        indeterminate: false,
                        chip: Some("5 min".into()),
                        segments: Vec::new(),
                        points: Vec::new(),
                        chronometer: false,
                        countdown: false,
                    }),
                    conversation: None,
                },
            },
            device_name: "Pixel".into(),
            icon_url: String::new(),
            image_url: "file:///C:/data/cache/images/1.jpg".into(),
            replies: vec![SentReply { text: "On my way".into(), pending: true }],
            group_count: 3,
            group_index: 0,
            group_expanded: false,
            group_summary: "Alex · Taylor".into(),
        };
        let text = |role| role_value(&row, role).value::<QString>().map(String::from).unwrap_or_default();
        assert_eq!(text("title"), "Sam");
        assert_eq!(text("text"), "Your verification code is 482913");
        assert_eq!(text("otpCode"), "482913");
        assert_eq!(text("replyAction"), "r");
        assert_eq!(text("actions"), r#"[{"id":"m","title":"Mute"}]"#);
        assert_eq!(role_value(&row, "when").value::<f64>(), Some(1_760_000_000_000.0));
        assert_eq!(text("replies"), r#"[{"pending":true,"text":"On my way"}]"#);
        assert!(text("live").contains(r#""chip":"5 min""#));
        assert_eq!(role_value(&row, "groupCount").value::<i32>(), Some(3));
        assert_eq!(role_value(&row, "groupIndex").value::<i32>(), Some(0));
        assert_eq!(role_value(&row, "groupExpanded").value::<bool>(), Some(false));
        assert_eq!(text("groupSummary"), "Alex · Taylor");
    }

    #[test]
    fn build_rows_groups_by_app_collapses_by_default_and_expands_on_toggle() {
        let phone = DeviceId([1; 32]);
        let mut s = AppState::default();
        let make = |key: &str, app: &str, title: &str, when: i64| Notification {
            key: key.into(),
            app: app.into(),
            app_name: app.into(),
            title: Some(title.into()),
            text: Some("body".into()),
            sub: None,
            when,
            actions: Vec::new(),
            silent: false,
            ongoing: false,
            icon: None,
            image: None,
            live: None,
            conversation: None,
        };
        s.apply(&NodeEvent::NotificationPosted {
            device: phone,
            notification: make("c1", "com.chat", "Oldest chat", 10),
        });
        s.apply(&NodeEvent::NotificationPosted {
            device: phone,
            notification: make("m1", "com.mail", "Mail 1", 20),
        });
        s.apply(&NodeEvent::NotificationPosted {
            device: phone,
            notification: make("c2", "com.chat", "Middle chat", 30),
        });
        s.apply(&NodeEvent::NotificationPosted {
            device: phone,
            notification: make("c3", "com.chat", "Newest chat", 40),
        });

        // Collapsed by default: 2 rows (lead chat + mail), total count 4.
        let collapsed = build_rows(&s);
        assert_eq!(collapsed.len(), 2);
        assert_eq!(collapsed[0].view.notification.key, "c3");
        assert_eq!(collapsed[0].group_count, 3);
        assert_eq!(collapsed[0].group_index, 0);
        assert!(!collapsed[0].group_expanded);
        assert_eq!(collapsed[0].group_summary, "Middle chat · Oldest chat");
        assert_eq!(collapsed[1].view.notification.key, "m1");
        assert_eq!(collapsed[1].group_count, 1);

        // Expanding com.chat reveals all 3 chat notifications in newest-first order before mail.
        s.toggle_app_expanded("com.chat");
        let expanded = build_rows(&s);
        assert_eq!(expanded.len(), 4);
        let keys: Vec<&str> = expanded.iter().map(|r| r.view.notification.key.as_str()).collect();
        assert_eq!(keys, ["c3", "c2", "c1", "m1"]);
        assert!(expanded[0].group_expanded);
        assert_eq!(expanded[1].group_index, 1);
        assert_eq!(expanded[2].group_index, 2);
    }
}
