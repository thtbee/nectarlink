// SPDX-License-Identifier: GPL-3.0-or-later
//! Phone notifications on this PC: app icons are saved for QML and toasts,
//! notifications become Windows toasts (removed again when the phone clears
//! them), and what the user does with a toast or the feed goes back to the
//! phone.

use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
};

use nectarlink_core::{DeviceId, Error, NodeEvent, Notification};

use crate::{
    core_host, icons,
    state::Changes,
    win::toast::{self, Toast, ToastEvent},
};

/// Keys that have a toast, per device, so a snapshot can remove the toasts
/// of notifications that went away meanwhile.
static TOASTED: Mutex<Option<HashMap<DeviceId, HashSet<String>>>> = Mutex::new(None);

fn toasted<T>(f: impl FnOnce(&mut HashMap<DeviceId, HashSet<String>>) -> T) -> T {
    f(TOASTED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new))
}

/// Saves the app icons an event carries; returns where each app's went.
fn save_icons(event: &NodeEvent) -> Vec<(String, std::path::PathBuf)> {
    let items: Vec<&Notification> = match event {
        NodeEvent::NotificationsReset { items, .. } => items.iter().collect(),
        NodeEvent::NotificationPosted { notification, .. } => vec![notification],
        _ => return Vec::new(),
    };
    let data_dir = &core_host::host().data_dir;
    items
        .into_iter()
        .filter_map(|n| {
            let icon = n.icon.as_ref()?;
            match icons::save(data_dir, &n.app, icon) {
                Ok(path) => Some((n.app.clone(), path)),
                Err(e) => {
                    tracing::debug!(app = %n.app, error = %e, "can't save an app icon");
                    None
                }
            }
        })
        .collect()
}

/// Folds an event into the hub, saving icons first.
pub fn apply(event: &NodeEvent) {
    let icons = save_icons(event);
    core_host::host().hub.update(|s| {
        let mut changes = Changes::NONE;
        for (app, path) in icons {
            changes |= s.set_app_icon(&app, path);
        }
        changes | s.apply(event)
    });
}

/// Shows, updates or removes toasts after an event.
pub fn update_toasts(event: &NodeEvent) {
    match event {
        NodeEvent::NotificationPosted { device, notification } => show(*device, notification),
        NodeEvent::NotificationRemoved { device, key } => {
            toasted(|t| t.get_mut(device).map(|keys| keys.remove(key)));
            toast::remove(&device.to_string(), key);
        }
        NodeEvent::NotificationsReset { device, items } => {
            let keep: HashSet<&str> = items.iter().map(|n| n.key.as_str()).collect();
            let gone: Vec<String> = toasted(|t| {
                let keys = t.entry(*device).or_default();
                let gone = keys.iter().filter(|k| !keep.contains(k.as_str())).cloned().collect();
                keys.retain(|k| keep.contains(k.as_str()));
                gone
            });
            for key in gone {
                toast::remove(&device.to_string(), &key);
            }
        }
        NodeEvent::DeviceRemoved(device) => {
            toasted(|t| t.remove(device));
            toast::remove_device(&device.to_string());
        }
        _ => {}
    }
}

fn show(device: DeviceId, n: &Notification) {
    let (device_name, icon) =
        core_host::host().hub.read(|s| (s.name_of(&device), s.app_icons.get(&n.app).cloned()));
    let reply = n.actions.iter().find(|a| a.reply);
    let toast = Toast {
        device: device.to_string(),
        key: n.key.clone(),
        title: n.title.clone().unwrap_or_else(|| n.app_name.clone()),
        body: n.text.clone().unwrap_or_default(),
        attribution: match device_name {
            Some(name) => format!("{} · {name}", n.app_name),
            None => n.app_name.clone(),
        },
        icon,
        actions: n.actions.iter().filter(|a| !a.reply).map(|a| (a.id.clone(), a.title.clone())).collect(),
        reply: reply.map(|a| (a.id.clone(), a.title.clone())),
        silent: n.silent,
    };
    toasted(|t| t.entry(device).or_default().insert(n.key.clone()));
    toast::show(toast);
}

/// What the user did with a toast.
pub fn on_toast(event: ToastEvent) {
    match event {
        ToastEvent::Opened { .. } => crate::bridge::app::request_activation(),
        ToastEvent::Action { device, key, action } => run_action(&device, key, action, None),
        ToastEvent::Reply { device, key, action, text } => run_action(&device, key, action, Some(text)),
        ToastEvent::Dismissed { device, key } => dismiss(&device, key),
    }
}

/// Dismisses a phone notification there (and here right away).
pub fn dismiss(device: &str, key: String) {
    let (Ok(device), Some(node)) = (device.parse::<DeviceId>(), core_host::node()) else { return };
    core_host::host().hub.update(|s| s.apply(&NodeEvent::NotificationRemoved { device, key: key.clone() }));
    toast::remove(&device.to_string(), &key);
    core_host::spawn(async move {
        if let Err(e) = node.dismiss_notification(device, key).await {
            report(&e);
        }
    });
}

/// Runs a notification's action on the phone (`reply` for a reply action).
pub fn run_action(device: &str, key: String, action: String, reply: Option<String>) {
    let (Ok(device), Some(node)) = (device.parse::<DeviceId>(), core_host::node()) else { return };
    core_host::spawn(async move {
        if let Err(e) = node.run_notification_action(device, key, action, reply).await {
            report(&e);
        }
    });
}

fn report(error: &Error) {
    let message = match error {
        Error::NotFound => "That notification is gone from the phone.".to_owned(),
        other => crate::bridge::app::describe(other),
    };
    crate::bridge::app::show_message(message);
}
