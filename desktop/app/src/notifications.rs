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
    state::{AppRule, Changes},
    win::toast::{self, Toast, ToastEvent},
};

/// Keys that have a toast, per device, so a snapshot can remove the toasts
/// of notifications that went away meanwhile.
static TOASTED: Mutex<Option<HashMap<DeviceId, HashSet<String>>>> = Mutex::new(None);

fn toasted<T>(f: impl FnOnce(&mut HashMap<DeviceId, HashSet<String>>) -> T) -> T {
    f(TOASTED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new))
}

/// Pictures from notifications, kept for a couple of days (history lasts one).
const KEEP_IMAGES: std::time::Duration = std::time::Duration::from_secs(2 * 24 * 3600);

pub(crate) fn images_dir() -> std::path::PathBuf {
    core_host::host().data_dir.join("cache").join("images")
}

/// Saves the pictures an event's notifications carry; returns where each
/// went, by device and key.
fn save_images(event: &NodeEvent) -> Vec<((DeviceId, String), std::path::PathBuf)> {
    let (device, items): (DeviceId, Vec<&Notification>) = match event {
        NodeEvent::NotificationsReset { device, items } => (*device, items.iter().collect()),
        NodeEvent::NotificationPosted { device, notification } => (*device, vec![notification]),
        _ => return Vec::new(),
    };
    let dir = images_dir();
    items
        .into_iter()
        .filter_map(|n| {
            let image = n.image.as_ref()?;
            // Named after the notification and the picture: an update with a
            // new picture gets a new file (the UI caches by name).
            let name = format!(
                "{:016x}.jpg",
                fnv(&[device.to_string().as_bytes(), n.key.as_bytes(), image].concat())
            );
            let path = dir.join(name);
            let saved = std::fs::create_dir_all(&dir)
                .and_then(|()| if path.exists() { Ok(()) } else { std::fs::write(&path, image) });
            match saved {
                Ok(()) => Some(((device, n.key.clone()), path)),
                Err(e) => {
                    tracing::debug!(error = %e, "can't save a notification picture");
                    None
                }
            }
        })
        .collect()
}

/// Deletes pictures older than [`KEEP_IMAGES`].
pub fn prune_images() {
    let Ok(entries) = std::fs::read_dir(images_dir()) else { return };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|t| now.duration_since(t).unwrap_or_default() > KEEP_IMAGES);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
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
    let images = save_images(event);
    core_host::host().hub.update(|s| {
        let mut changes = Changes::NONE;
        for (app, path) in icons {
            changes |= s.set_app_icon(&app, path);
        }
        changes |= s.apply(event);
        for (key, path) in images {
            changes |= s.set_notification_image(key, path);
        }
        changes
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
    let (device_name, icon, rule, image) = core_host::host().hub.read(|s| {
        (
            s.name_of(&device),
            s.app_icons.get(&n.app).cloned(),
            s.app_rule(&n.app),
            s.notification_images.get(&(device, n.key.clone())).cloned(),
        )
    });
    // The user chose no pop-ups (or nothing at all) for this app.
    if rule != AppRule::Show {
        return;
    }
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
        image,
        actions: n.actions.iter().filter(|a| !a.reply).map(|a| (a.id.clone(), a.title.clone())).collect(),
        reply: reply.map(|a| (a.id.clone(), a.title.clone())),
        silent: n.silent,
        progress: None,
    };
    toasted(|t| t.entry(device).or_default().insert(n.key.clone()));
    toast::show(toast);
}

/// What the user did with a toast.
pub fn on_toast(event: ToastEvent) {
    // Received-file toasts: open the file, or show it in its folder.
    match &event {
        ToastEvent::Opened { device, key } if device == crate::transfers::TOAST_GROUP => {
            return crate::transfers::open(std::path::Path::new(key));
        }
        ToastEvent::Action { device, key, .. } if device == crate::transfers::TOAST_GROUP => {
            return crate::transfers::show_in_folder(std::path::Path::new(key));
        }
        ToastEvent::Dismissed { device, .. } if device == crate::transfers::TOAST_GROUP => return,
        // Sends from Explorer: open the app, or cancel.
        ToastEvent::Action { device, key, action } if device == crate::send_to::TOAST_GROUP => {
            if action == crate::send_to::ACTION_CANCEL {
                crate::transfers::cancel(key);
            }
            return;
        }
        ToastEvent::Dismissed { device, .. } if device == crate::send_to::TOAST_GROUP => return,
        // "Update" on the update notice.
        ToastEvent::Action { device, .. } if device == crate::updater::TOAST_GROUP => {
            return crate::bridge::app::install_update_in_background();
        }
        ToastEvent::Dismissed { device, .. } if device == crate::updater::TOAST_GROUP => return,
        // New photos: open, save or copy one.
        ToastEvent::Opened { device, key } if device == crate::photos::TOAST_GROUP => {
            return crate::photos::on_toast(key, None);
        }
        ToastEvent::Action { device, key, action } if device == crate::photos::TOAST_GROUP => {
            return crate::photos::on_toast(key, Some(action));
        }
        ToastEvent::Dismissed { device, .. } if device == crate::photos::TOAST_GROUP => return,
        ToastEvent::Dismissed { device, .. } | ToastEvent::Action { device, .. }
            if device == crate::battery::TOAST_GROUP =>
        {
            return;
        }
        _ => {}
    }
    match event {
        ToastEvent::Opened { .. } => crate::bridge::app::request_activation(),
        ToastEvent::Action { device, key, action } => run_action(&device, key, action, None),
        ToastEvent::Reply { device, key, action, text } => run_action(&device, key, action, Some(text)),
        ToastEvent::Dismissed { device, key } => dismiss(&device, key),
    }
}

/// Sets what an app's notifications do on this PC. Pop-ups it no longer
/// gets are taken away.
pub fn set_app_rule(app: &str, rule: AppRule) {
    let hub = &core_host::host().hub;
    hub.update(|s| s.set_app_rule(app, rule));
    if rule == AppRule::Show {
        return;
    }
    let shown: Vec<(DeviceId, String)> = hub.read(|s| {
        s.notifications
            .iter()
            .filter(|n| n.notification.app == app)
            .map(|n| (n.device, n.notification.key.clone()))
            .collect()
    });
    for (device, key) in shown {
        toasted(|t| t.get_mut(&device).map(|keys| keys.remove(&key)));
        toast::remove(&device.to_string(), &key);
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
    let hub = &core_host::host().hub;
    // A reply shows under its notification right away, faded until the
    // phone has taken it.
    if let Some(text) = &reply {
        hub.update(|s| s.reply_sending(device, &key, text));
    }
    core_host::spawn(async move {
        let result = node.run_notification_action(device, key.clone(), action, reply.clone()).await;
        if let Some(text) = &reply {
            core_host::host().hub.update(|s| s.reply_done(device, &key, text, result.is_ok()));
        }
        if let Err(e) = result {
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
