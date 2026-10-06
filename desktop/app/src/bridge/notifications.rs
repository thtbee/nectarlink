// SPDX-License-Identifier: GPL-3.0-or-later
//! `NotificationList`: phone notifications (newest first) as a Qt list
//! model, plus the commands QML runs on them.

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
        /// Dismisses all of a device's notifications.
        #[qinvokable]
        #[cxx_name = "dismissAll"]
        fn dismiss_all(self: &NotificationList, device: &QString);
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
}

#[derive(Default)]
pub struct NotificationListRust {
    count: i32,
    history_enabled: bool,
    apps: QString,
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
    "replies",
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
        let (new, apps, history): (Vec<Row>, String, bool) = core_host::host().hub.read(|s| {
            let rows = s
                .notifications
                .iter()
                .filter(|view| s.app_rule(&view.notification.app) != AppRule::Hidden)
                .map(|view| Row {
                    device_name: s.name_of(&view.device).unwrap_or_default(),
                    icon_url: s
                        .app_icons
                        .get(&view.notification.app)
                        .map(|p| icons::file_url(p))
                        .unwrap_or_default(),
                    image_url: s
                        .notification_images
                        .get(&(view.device, view.notification.key.clone()))
                        .map(|p| icons::file_url(p))
                        .unwrap_or_default(),
                    replies: s
                        .replies
                        .get(&(view.device, view.notification.key.clone()))
                        .cloned()
                        .unwrap_or_default(),
                    view: view.clone(),
                })
                .collect();
            (rows, apps_json(s), s.history_enabled)
        });
        if String::from(&self.apps) != apps {
            self.as_mut().set_apps(QString::from(&apps));
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

    pub fn count_for(&self, device: &QString) -> i32 {
        let device = String::from(device);
        self.rows.iter().filter(|r| r.view.device.to_string() == device).count() as i32
    }

    pub fn dismiss(&self, device: &QString, key: &QString) {
        crate::notifications::dismiss(&String::from(device), String::from(key));
    }

    pub fn dismiss_all(&self, device: &QString) {
        let device = String::from(device);
        for row in self.rows.iter().filter(|r| r.view.device.to_string() == device) {
            crate::notifications::dismiss(&row.view.device.to_string(), row.view.notification.key.clone());
        }
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
    use nectarlink_core::{Notification, NotificationAction};

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
                    text: None,
                    sub: None,
                    when: 1_760_000_000_000,
                    actions: vec![
                        NotificationAction { id: "r".into(), title: "Reply".into(), reply: true },
                        NotificationAction { id: "m".into(), title: "Mute".into(), reply: false },
                    ],
                    silent: false,
                    icon: None,
                    image: None,
                },
            },
            device_name: "Pixel".into(),
            icon_url: String::new(),
            image_url: "file:///C:/data/cache/images/1.jpg".into(),
            replies: vec![SentReply { text: "On my way".into(), pending: true }],
        };
        let text = |role| role_value(&row, role).value::<QString>().map(String::from).unwrap_or_default();
        assert_eq!(text("title"), "Sam");
        assert_eq!(text("text"), "");
        assert_eq!(text("replyAction"), "r");
        assert_eq!(text("actions"), r#"[{"id":"m","title":"Mute"}]"#);
        assert_eq!(role_value(&row, "when").value::<f64>(), Some(1_760_000_000_000.0));
        assert_eq!(text("replies"), r#"[{"pending":true,"text":"On my way"}]"#);
    }
}
