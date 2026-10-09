// SPDX-License-Identifier: GPL-3.0-or-later
//! Phone notifications on this PC: app icons are saved for QML and toasts,
//! notifications become Windows toasts (removed again when the phone clears
//! them), and what the user does with a toast or the feed goes back to the
//! phone.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use nectarlink_core::{DeviceId, Error, NodeEvent, Notification};

use crate::{
    core_host, icons,
    state::{AppRule, Changes},
    win::toast::{self, Toast, ToastEvent},
};

/// Local toast action ID for copying a detected one-time code.
pub const ACTION_COPY_OTP: &str = "copy_otp";

/// Whether incoming one-time codes are copied to the PC clipboard automatically.
static AUTO_COPY_OTP: AtomicBool = AtomicBool::new(false);

/// Whether phone Do Not Disturb quiets phone notification pop-ups on the PC.
static SYNC_DND: AtomicBool = AtomicBool::new(false);

/// Keys that have a toast, per device, so a snapshot can remove the toasts
/// of notifications that went away meanwhile.
static TOASTED: Mutex<Option<HashMap<DeviceId, HashSet<String>>>> = Mutex::new(None);

/// Keys of live notifications that already have a toast with a `<progress>` bar,
/// so subsequent updates call [`toast::update_progress`] in place instead of
/// popping up a new toast.
static LIVE_TOASTS: Mutex<Option<HashSet<(DeviceId, String)>>> = Mutex::new(None);

/// Detected OTP codes by `(device, notification_key)` for active toasts, so
/// the code never needs to be placed in Windows toast XML action arguments.
static OTP_CODES: Mutex<Option<HashMap<(DeviceId, String), String>>> = Mutex::new(None);

/// Last auto-copied OTP and when it was copied, to avoid copying the same code
/// twice when both a notification and an SMS sync event arrive for one text.
static LAST_AUTO_OTP: Mutex<Option<(String, Instant)>> = Mutex::new(None);

fn toasted<T>(f: impl FnOnce(&mut HashMap<DeviceId, HashSet<String>>) -> T) -> T {
    f(TOASTED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new))
}

fn live_toasts<T>(f: impl FnOnce(&mut HashSet<(DeviceId, String)>) -> T) -> T {
    f(LIVE_TOASTS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashSet::new))
}

fn otp_codes<T>(f: impl FnOnce(&mut HashMap<(DeviceId, String), String>) -> T) -> T {
    f(OTP_CODES.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new))
}

pub fn set_auto_copy_otp(on: bool) {
    AUTO_COPY_OTP.store(on, Ordering::Relaxed);
}

pub fn auto_copy_otp_enabled() -> bool {
    AUTO_COPY_OTP.load(Ordering::Relaxed)
}

pub fn set_sync_dnd(on: bool) {
    SYNC_DND.store(on, Ordering::Relaxed);
    if on {
        let dnd_devices: Vec<DeviceId> = core_host::host()
            .hub
            .read(|s| s.toggles.iter().filter(|(_, t)| t.dnd).map(|(id, _)| *id).collect());
        for device in dnd_devices {
            quiet_device_toasts(device);
        }
    }
}

fn quiet_device_toasts(device: DeviceId) {
    toasted(|t| t.remove(&device));
    live_toasts(|l| l.retain(|(d, _)| *d != device));
    toast::remove_device(&device.to_string());
}

/// Extracts a one-time code from a phone notification, if present.
///
/// Checks the notification body first. If the keyword is in the title and the
/// code is in the body (e.g. title `"Verification code"`, body `"482913"`),
/// matches the combined text only when the extracted digits come from the body
/// (so a phone number in the title is never mistaken for a code).
pub fn notification_otp(n: &Notification) -> Option<String> {
    let text = n.text.as_deref().unwrap_or_default();
    if let Some(code) = nectarlink_core::otp::one_time_code(text) {
        return Some(code);
    }
    if let Some(title) = n.title.as_deref().filter(|t| !t.trim().is_empty())
        && !text.trim().is_empty()
    {
        let combined = format!("{title}: {text}");
        if let Some(code) = nectarlink_core::otp::one_time_code(&combined)
            && (text.contains(&code)
                || (code.len() == 6 && text.contains(&format!("{}-{}", &code[..3], &code[3..]))))
        {
            return Some(code);
        }
    }
    None
}

/// Copies a one-time code to the PC clipboard without syncing it back to any
/// phone or keeping it in any clipboard history (ours or Windows'). Never
/// logs the code.
pub fn copy_otp(code: &str) {
    let trimmed = code.trim();
    if trimmed.is_empty() {
        return;
    }
    if crate::win::clipboard::write_private(trimmed).is_ok() {
        crate::bridge::app::show_message("Code copied");
    }
}

/// Automatically copies `code` when the "Copy codes automatically" setting is
/// enabled, deduplicating identical codes within 10 seconds.
pub fn maybe_auto_copy_otp(code: &str) {
    if !AUTO_COPY_OTP.load(Ordering::Relaxed) {
        return;
    }
    let trimmed = code.trim();
    if trimmed.is_empty() {
        return;
    }
    let now = Instant::now();
    {
        let mut last = LAST_AUTO_OTP.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((prev, when)) = last.as_ref()
            && prev == trimmed
            && now.duration_since(*when) < Duration::from_secs(10)
        {
            return;
        }
        *last = Some((trimmed.to_owned(), now));
    }
    copy_otp(trimmed);
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

fn is_notification_image_file(name: &str) -> bool {
    !name.starts_with("sms-")
        && !name.starts_with("mms-")
        && !name.starts_with("chat-")
        && !name.starts_with("photo-")
}

/// Immediately removes cached notification images from `cache/images/`.
pub fn clear_cached_images() {
    let Ok(entries) = std::fs::read_dir(images_dir()) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_notif = path.file_name().and_then(|n| n.to_str()).is_some_and(is_notification_image_file);
        if is_notif {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Total byte size of cached notification images on disk.
pub fn cached_image_bytes() -> u64 {
    let Ok(entries) = std::fs::read_dir(images_dir()) else { return 0 };
    entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let name_str = name.to_str()?;
            if !is_notification_image_file(name_str) {
                return None;
            }
            let meta = e.metadata().ok()?;
            meta.is_file().then_some(meta.len())
        })
        .sum()
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
            live_toasts(|l| l.remove(&(*device, key.clone())));
            otp_codes(|c| c.remove(&(*device, key.clone())));
            toast::remove(&device.to_string(), key);
        }
        NodeEvent::NotificationsReset { device, items } => {
            for n in items {
                if let Some(code) = notification_otp(n) {
                    otp_codes(|c| c.insert((*device, n.key.clone()), code));
                }
            }
            let keep: HashSet<&str> = items.iter().map(|n| n.key.as_str()).collect();
            let gone: Vec<String> = toasted(|t| {
                let keys = t.entry(*device).or_default();
                let gone = keys.iter().filter(|k| !keep.contains(k.as_str())).cloned().collect();
                keys.retain(|k| keep.contains(k.as_str()));
                gone
            });
            live_toasts(|l| l.retain(|(d, k)| d != device || keep.contains(k.as_str())));
            otp_codes(|c| c.retain(|(d, k), _| d != device || keep.contains(k.as_str())));
            for key in gone {
                toast::remove(&device.to_string(), &key);
            }
        }
        NodeEvent::PhoneToggles { device, toggles } => {
            if SYNC_DND.load(Ordering::Relaxed) && toggles.dnd {
                quiet_device_toasts(*device);
            }
        }
        NodeEvent::DeviceRemoved(device) => {
            toasted(|t| t.remove(device));
            live_toasts(|l| l.retain(|(d, _)| d != device));
            otp_codes(|c| c.retain(|(d, _), _| d != device));
            toast::remove_device(&device.to_string());
        }
        _ => {}
    }
}

fn live_toast_progress(n: &Notification) -> Option<toast::Progress> {
    let live = n.live.as_ref()?;
    let status = live
        .chip
        .as_deref()
        .filter(|s| !s.is_empty())
        .or(n.text.as_deref().filter(|s| !s.is_empty()))
        .unwrap_or("Live")
        .to_owned();
    let (value, label) = if live.indeterminate {
        (None, String::new())
    } else if let (Some(progress), Some(max)) = (live.progress, live.max)
        && max > 0
    {
        let ratio = (f64::from(progress) / f64::from(max)).clamp(0.0, 1.0);
        let pct = ((ratio * 100.0).round() as u32).min(100);
        (Some(ratio), format!("{pct}%"))
    } else {
        (None, live.chip.clone().unwrap_or_default())
    };
    Some(toast::Progress { status, value, label })
}

fn show(device: DeviceId, n: &Notification) {
    let otp = notification_otp(n);
    if let Some(code) = &otp {
        otp_codes(|c| c.insert((device, n.key.clone()), code.clone()));
        maybe_auto_copy_otp(code);
    }
    let (device_name, icon, rule, image, phone_dnd) = core_host::host().hub.read(|s| {
        let dnd = s.toggles.get(&device).is_some_and(|t| t.dnd);
        (
            s.name_of(&device),
            s.app_icons.get(&n.app).cloned(),
            s.app_rule(&n.app),
            s.notification_images.get(&(device, n.key.clone())).cloned(),
            dnd,
        )
    });
    // The user chose no pop-ups (or nothing at all) for this app, or the phone
    // is in Do Not Disturb while DND quieting is turned on.
    if rule != AppRule::Show || (SYNC_DND.load(Ordering::Relaxed) && phone_dnd) {
        return;
    }
    let progress = live_toast_progress(n);
    let id = (device, n.key.clone());
    if let Some(prog) = &progress {
        let already_shown = live_toasts(|l| l.contains(&id));
        if already_shown {
            toast::update_progress(&device.to_string(), &n.key, prog.clone());
            return;
        }
    } else {
        live_toasts(|l| l.remove(&id));
    }
    let mut actions: Vec<(String, String)> = Vec::new();
    if otp.is_some() {
        actions.push((ACTION_COPY_OTP.to_owned(), "Copy code".to_owned()));
    }
    actions.extend(n.actions.iter().filter(|a| !a.reply).map(|a| (a.id.clone(), a.title.clone())));
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
        actions,
        reply: reply.map(|a| (a.id.clone(), a.title.clone())),
        silent: n.silent,
        progress: progress.clone(),
        call: false,
    };
    toasted(|t| t.entry(device).or_default().insert(n.key.clone()));
    if progress.is_some() {
        live_toasts(|l| l.insert(id));
    }
    toast::show(toast);
}

/// What the user did with a toast.
pub fn on_toast(event: ToastEvent) {
    // Received-file toasts: open the file, or show it in its folder.
    match &event {
        ToastEvent::Opened { device, key } if device == crate::transfers::TOAST_GROUP => {
            return crate::transfers::open(std::path::Path::new(key));
        }
        ToastEvent::Action { device, key, action } if device == crate::transfers::TOAST_GROUP => {
            if action == crate::transfers::ACTION_ACCEPT {
                return crate::transfers::accept(key.strip_prefix("req:").unwrap_or(key));
            }
            if action == crate::transfers::ACTION_DECLINE {
                return crate::transfers::cancel(key.strip_prefix("req:").unwrap_or(key));
            }
            if action == crate::transfers::ACTION_OPEN {
                return crate::transfers::open(std::path::Path::new(key));
            }
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
        // Calls: answer, decline or silence; dismissing only hides it.
        ToastEvent::Action { device, key, action } if device == crate::calls::TOAST_GROUP => {
            return crate::calls::on_toast(key, action);
        }
        ToastEvent::Dismissed { device, .. } if device == crate::calls::TOAST_GROUP => return,
        ToastEvent::Dismissed { device, .. } | ToastEvent::Action { device, .. }
            if device == crate::battery::TOAST_GROUP =>
        {
            return;
        }
        ToastEvent::Action { device, action, .. } if device == crate::clipboard::TOAST_GROUP => {
            if action == crate::clipboard::ACTION_CLIP_SUGGESTION {
                crate::clipboard::run_last_suggestion();
            }
            return;
        }
        ToastEvent::Dismissed { device, .. } if device == crate::clipboard::TOAST_GROUP => return,
        _ => {}
    }
    match event {
        ToastEvent::Opened { device, key } => {
            if let Ok(dev) = device.parse::<DeviceId>() {
                let app_target = core_host::host().hub.read(|s| {
                    let can_open = s.matrices.get(&dev).and_then(|m| m.state("mirroring.app_windows"))
                        == Some(nectarlink_core::FeatureState::Available);
                    if !can_open {
                        return None;
                    }
                    s.notifications
                        .iter()
                        .find(|n| n.device == dev && n.notification.key == key)
                        .map(|n| (n.notification.app.clone(), n.notification.app_name.clone()))
                        .or_else(|| {
                            s.history
                                .iter()
                                .find(|h| h.device == dev && h.notification.key == key)
                                .map(|h| (h.notification.app.clone(), h.notification.app_name.clone()))
                        })
                        .filter(|(pkg, _)| !pkg.is_empty())
                });
                if let Some((pkg, app_name)) = app_target {
                    let label = if app_name.is_empty() { pkg.clone() } else { app_name };
                    crate::mirror::start_app(dev, pkg, label);
                    return;
                }
            }
            crate::bridge::app::request_activation();
        }
        ToastEvent::Action { device, key, action } if action == ACTION_COPY_OTP => {
            if let Ok(dev) = device.parse::<DeviceId>() {
                let code = otp_codes(|c| c.get(&(dev, key.clone())).cloned()).or_else(|| {
                    core_host::host().hub.read(|s| {
                        s.notifications
                            .iter()
                            .find(|n| n.device == dev && n.notification.key == key)
                            .and_then(|n| notification_otp(&n.notification))
                    })
                });
                if let Some(code) = code {
                    copy_otp(&code);
                }
            }
        }
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
        live_toasts(|l| l.remove(&(device, key.clone())));
        toast::remove(&device.to_string(), &key);
    }
}

/// Dismisses a phone notification there (and here right away).
pub fn dismiss(device: &str, key: String) {
    let (Ok(device), Some(node)) = (device.parse::<DeviceId>(), core_host::node()) else { return };
    core_host::host().hub.update(|s| s.apply(&NodeEvent::NotificationRemoved { device, key: key.clone() }));
    toasted(|t| t.get_mut(&device).map(|keys| keys.remove(&key)));
    live_toasts(|l| l.remove(&(device, key.clone())));
    otp_codes(|c| c.remove(&(device, key.clone())));
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

#[cfg(test)]
mod tests {
    use nectarlink_core::NotificationLive;

    use super::*;

    fn make_note(title: Option<&str>, text: Option<&str>) -> Notification {
        Notification {
            key: "k1".into(),
            app: "com.google.android.apps.messaging".into(),
            app_name: "Messages".into(),
            title: title.map(Into::into),
            text: text.map(Into::into),
            sub: None,
            when: 1_760_000_000_000,
            actions: Vec::new(),
            silent: false,
            icon: None,
            image: None,
            live: None,
            conversation: None,
        }
    }

    #[test]
    fn extracts_otp_from_body_or_title_plus_body_without_confusing_sender_number() {
        assert_eq!(
            notification_otp(&make_note(Some("5550100"), Some("Your verification code is 482913"))),
            Some("482913".into())
        );
        assert_eq!(
            notification_otp(&make_note(Some("Verification code"), Some("482913"))),
            Some("482913".into())
        );
        assert_eq!(
            notification_otp(&make_note(Some("5550100"), Some("Please enter your login code"))),
            None,
            "sender phone number in title must not be extracted as an OTP"
        );
    }

    #[test]
    fn builds_toast_progress_for_live_notifications() {
        let mut n = make_note(Some("Ride"), Some("Arriving in 4 min"));
        assert!(live_toast_progress(&n).is_none());

        n.live = Some(NotificationLive {
            v: 1,
            progress: Some(45),
            max: Some(100),
            indeterminate: false,
            chip: Some("4 min".into()),
            segments: Vec::new(),
            points: Vec::new(),
            chronometer: false,
            countdown: false,
        });
        let p = live_toast_progress(&n).unwrap();
        assert_eq!(p.status, "4 min");
        assert_eq!(p.value, Some(0.45));
        assert_eq!(p.label, "45%");
    }
}
