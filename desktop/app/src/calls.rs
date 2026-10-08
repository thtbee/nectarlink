// SPDX-License-Identifier: GPL-3.0-or-later
//! Calls on a phone (docs/protocol/calls.md): a ringing Windows
//! notification to answer, decline or silence the call (its audio stays on
//! the phone), the call in progress (for Home's call card and a quiet
//! notification to mute or end it), and a notification for calls nobody
//! answered.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use nectarlink_core::{
    CallCommand, CallLogEntry, CallState, Contact, DeviceId, Error, FeatureState, LinkState, NodeEvent,
};
use serde_json::{Value, json};

use crate::{
    bridge::app::{describe, show_message},
    core_host,
    messages::{Status, digits},
    state::Changes,
    win::toast::{self, CALL_ANSWER, CALL_DECLINE, CALL_END, Toast},
};

/// Whether PC media playback is automatically paused during phone calls.
static PAUSE_MEDIA_ON_CALL: AtomicBool = AtomicBool::new(true);

pub fn set_pause_media_on_call(on: bool) {
    PAUSE_MEDIA_ON_CALL.store(on, Ordering::Relaxed);
}

/// Phones that currently have a ringing or active call, for media auto-pause/resume.
static ONGOING_CALLS: Mutex<Option<HashSet<DeviceId>>> = Mutex::new(None);

fn ongoing_calls<T>(f: impl FnOnce(&mut HashSet<DeviceId>) -> T) -> T {
    f(ONGOING_CALLS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_default())
}

fn note_call_ongoing(device: DeviceId) {
    // Only when the call starts: a later update (answered, muted) mustn't
    // pause music the user started again during the call.
    let started = ongoing_calls(|c| c.insert(device));
    if started && PAUSE_MEDIA_ON_CALL.load(Ordering::Relaxed) {
        crate::win::media_sessions::pause_for_call();
    }
}

fn note_call_ended(device: DeviceId) {
    let all_done = ongoing_calls(|c| c.remove(&device) && c.is_empty());
    if all_done {
        crate::win::media_sessions::resume_after_call();
    }
}

/// Call log entries read at a time, and contacts per page.
const LOG_PAGE: u32 = 50;
const CONTACTS_PAGE: u32 = 200;
const MAX_CONTACT_PAGES: u32 = 3;

/// The toast "device" for calls; their key is `<device ID> <call ID>`.
pub const TOAST_GROUP: &str = "calls";
const ACTION_SILENCE: &str = "silence";
const ACTION_MUTE: &str = "mute";
const ACTION_UNMUTE: &str = "unmute";
const ACTION_SPEAKER: &str = "speaker";
const ACTION_EARPIECE: &str = "earpiece";
/// The in-call notification's key prefix (`active <device> <call>`).
const ACTIVE: &str = "active ";

/// The ringing call shown per phone, to show it again (silenced) or take it
/// away when it stops ringing.
static RINGING: Mutex<Option<HashMap<DeviceId, CallState>>> = Mutex::new(None);

fn ringing<T>(f: impl FnOnce(&mut HashMap<DeviceId, CallState>) -> T) -> T {
    f(RINGING.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_default())
}

/// The call in progress per phone (answered, or held).
static IN_PROGRESS: Mutex<Option<HashMap<DeviceId, CallState>>> = Mutex::new(None);

fn in_progress<T>(f: impl FnOnce(&mut HashMap<DeviceId, CallState>) -> T) -> T {
    f(IN_PROGRESS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_default())
}

/// The call per phone whose in-call notification was shown.
static NOTIFIED: Mutex<Option<HashMap<DeviceId, String>>> = Mutex::new(None);

fn notified<T>(f: impl FnOnce(&mut HashMap<DeviceId, String>) -> T) -> T {
    f(NOTIFIED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_default())
}

/// A phone's call in progress, if any.
pub fn active(device: DeviceId) -> Option<CallState> {
    in_progress(|c| c.get(&device).cloned())
}

/// Whether the PC can answer and end a phone's calls.
pub fn can_control(device: DeviceId) -> bool {
    core_host::host().hub.read(|s| {
        s.matrices.get(&device).and_then(|m| m.state("calls.control")) == Some(FeatureState::Available)
    })
}

/// Whether the PC can ask a phone to dial a number.
pub fn can_dial(device: DeviceId) -> bool {
    core_host::host().hub.read(|s| {
        s.matrices.get(&device).and_then(|m| m.state("calls.dial")) == Some(FeatureState::Available)
    })
}

fn key(device: DeviceId, call: &str) -> String {
    format!("{device} {call}")
}

pub fn on_event(event: &NodeEvent) {
    let page_device = page_state(|s| s.device);
    match event {
        NodeEvent::Call { device, call } => {
            changed(*device, call);
            if Some(*device) == page_device && call.state == "ended" {
                core_host::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
                    load_call_log(None);
                });
            }
        }
        NodeEvent::CallLogChanged { device } if Some(*device) == page_device => {
            load_call_log(None);
        }
        NodeEvent::ContactsChanged { device } if Some(*device) == page_device => {
            load_contacts();
        }
        NodeEvent::LinkChanged { device, link: LinkState::Online { .. } } if Some(*device) == page_device => {
            if page_state(|s| s.log_status != Status::Ready) {
                load_call_log(None);
            }
            if page_state(|s| s.contacts_status != Status::Ready) {
                load_contacts();
            }
        }
        // A phone that's gone can't be answered (or hung up) from here.
        NodeEvent::LinkChanged { device, link: LinkState::Offline { .. } } => {
            stop_ringing(*device);
            finish(*device);
            note_call_ended(*device);
            if Some(*device) == page_device {
                page_state(|s| {
                    s.log_status = Status::Offline;
                    s.contacts_status = Status::Offline;
                });
                core_host::host().hub.changed(Changes::CALLS);
            }
        }
        NodeEvent::DeviceRemoved(device) => {
            stop_ringing(*device);
            finish(*device);
            note_call_ended(*device);
        }
        // A call can arrive before what the phone can do with it: once the
        // PC may end it, its notification shows.
        NodeEvent::Capabilities(matrix) => {
            if matrix.state("calls.control") == Some(FeatureState::Available)
                && let Some(call) = active(matrix.device)
                && notified(|n| n.get(&matrix.device) != Some(&call.id))
            {
                show_in_progress(matrix.device, &call);
            }
            if Some(matrix.device) == page_device {
                if matrix.state("calls.log") == Some(FeatureState::Available)
                    && page_state(|s| !matches!(s.log_status, Status::Ready | Status::Loading))
                {
                    load_call_log(None);
                }
                if matrix.state("contacts.read") == Some(FeatureState::Available)
                    && page_state(|s| !matches!(s.contacts_status, Status::Ready | Status::Loading))
                {
                    load_contacts();
                }
            }
        }
        _ => {}
    }
}

fn changed(device: DeviceId, call: &CallState) {
    match call.state.as_str() {
        "ringing" | "active" => note_call_ongoing(device),
        "ended" => note_call_ended(device),
        _ => {}
    }
    if call.state == "active" {
        match in_progress(|c| c.insert(device, call.clone())) {
            Some(previous) if previous.id == call.id => {}
            previous => {
                if let Some(previous) = previous {
                    toast::remove(TOAST_GROUP, &format!("{ACTIVE}{}", key(device, &previous.id)));
                }
                if can_control(device) {
                    show_in_progress(device, call);
                }
            }
        }
        core_host::host().hub.changed(Changes::CALLS);
    } else if call.state == "ended" || in_progress(|c| c.get(&device).is_some_and(|c| c.id == call.id)) {
        finish(device);
    }
    match call.state.as_str() {
        "ringing" if call.incoming => {
            if let Some(previous) = ringing(|r| r.insert(device, call.clone()))
                && previous.id != call.id
            {
                toast::remove(TOAST_GROUP, &key(device, &previous.id));
            }
            show_ringing(device, call, false);
        }
        "ended" if call.missed => {
            stop_ringing(device);
            show_missed(device, call);
        }
        // Answered (here or on the phone) or over.
        _ => stop_ringing(device),
    }
}

/// The call in progress is over (or out of reach).
fn finish(device: DeviceId) {
    notified(|n| n.remove(&device));
    if let Some(call) = in_progress(|c| c.remove(&device)) {
        toast::remove(TOAST_GROUP, &format!("{ACTIVE}{}", key(device, &call.id)));
        core_host::host().hub.changed(Changes::CALLS);
    }
}

fn stop_ringing(device: DeviceId) {
    if let Some(call) = ringing(|r| r.remove(&device)) {
        toast::remove(TOAST_GROUP, &key(device, &call.id));
    }
}

/// Who's calling, as well as the phone knows.
fn caller(call: &CallState) -> String {
    call.name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .or_else(|| call.number.clone().filter(|n| !n.trim().is_empty()))
        .unwrap_or_else(|| "Unknown caller".into())
}

fn phone_name(device: DeviceId) -> String {
    core_host::host().hub.read(|s| s.name_of(&device)).unwrap_or_else(|| "your phone".into())
}

fn show_ringing(device: DeviceId, call: &CallState, silenced: bool) {
    let mut actions = Vec::new();
    if can_control(device) {
        actions.push((CALL_ANSWER.to_owned(), "Answer".to_owned()));
        actions.push((CALL_DECLINE.to_owned(), "Decline".to_owned()));
        if !silenced {
            actions.push((ACTION_SILENCE.to_owned(), "Silence".to_owned()));
        }
    }
    // The number under the name, when there's a name.
    let body = match (&call.name, &call.number) {
        (Some(name), Some(number)) if !name.trim().is_empty() => {
            format!("{number} · on {}", phone_name(device))
        }
        _ => format!("Calling {}", phone_name(device)),
    };
    toast::show(Toast {
        device: TOAST_GROUP.into(),
        key: key(device, &call.id),
        title: caller(call),
        body,
        attribution: "Nectarlink".into(),
        icon: call.photo.as_deref().and_then(|photo| save_photo(device, &call.id, photo)),
        image: None,
        actions,
        reply: None,
        silent: silenced,
        progress: None,
        call: true,
    });
}

/// A quiet notification for the call in progress: mute, speaker, end
/// (for a phone whose calls the PC controls).
fn show_in_progress(device: DeviceId, call: &CallState) {
    notified(|n| n.insert(device, call.id.clone()));
    let mut actions = Vec::new();
    if let Some(controls) = &call.controls {
        actions.push(if controls.muted {
            (ACTION_UNMUTE.to_owned(), "Unmute".to_owned())
        } else {
            (ACTION_MUTE.to_owned(), "Mute".to_owned())
        });
        actions.push(if controls.speaker {
            (ACTION_EARPIECE.to_owned(), "Earpiece".to_owned())
        } else {
            (ACTION_SPEAKER.to_owned(), "Speaker".to_owned())
        });
    }
    actions.push((CALL_END.to_owned(), "End".to_owned()));
    toast::show(Toast {
        device: TOAST_GROUP.into(),
        key: format!("{ACTIVE}{}", key(device, &call.id)),
        title: format!("On a call with {}", caller(call)),
        body: format!("On {}", phone_name(device)),
        attribution: "Nectarlink".into(),
        icon: None,
        image: None,
        actions,
        reply: None,
        silent: true,
        progress: None,
        call: false,
    });
}

/// Asks a phone to do something with its call; problems are shown.
pub fn command(device: DeviceId, call: String, command: CallCommand) {
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        match node.call_command(device, call, command).await {
            Ok(()) => {}
            Err(Error::NotFound) => show_message("That call is over."),
            Err(e) => show_message(describe(&e)),
        }
    });
}

fn show_missed(device: DeviceId, call: &CallState) {
    toast::show(Toast {
        device: TOAST_GROUP.into(),
        key: format!("missed {}", key(device, &call.id)),
        title: format!("Missed call from {}", caller(call)),
        body: format!("On {}", phone_name(device)),
        attribution: "Nectarlink".into(),
        icon: None,
        image: None,
        actions: Vec::new(),
        reply: None,
        silent: false,
        progress: None,
        call: false,
    });
}

/// The caller's photo as a file, for the notification.
fn save_photo(device: DeviceId, call: &str, photo: &[u8]) -> Option<PathBuf> {
    let dir = crate::notifications::images_dir();
    let path = dir.join(format!("call-{:016x}.jpg", crate::photos::fingerprint(&key(device, call))));
    match std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, photo)) {
        Ok(()) => Some(path),
        Err(e) => {
            tracing::warn!(error = %e, "can't keep a caller's photo");
            None
        }
    }
}

/// The user pressed a button on a call notification.
pub fn on_toast(key: &str, action: &str) {
    if key.starts_with("missed ") {
        return;
    }
    let (key, in_call) = match key.strip_prefix(ACTIVE) {
        Some(key) => (key, true),
        None => (key, false),
    };
    let Some((device, call)) = key.split_once(' ') else { return };
    let Ok(device) = device.parse::<DeviceId>() else { return };
    let command = match action {
        CALL_ANSWER => CallCommand::Answer,
        CALL_DECLINE | CALL_END => CallCommand::Decline,
        ACTION_SILENCE => CallCommand::Silence,
        ACTION_MUTE => CallCommand::Mute(true),
        ACTION_UNMUTE => CallCommand::Mute(false),
        ACTION_SPEAKER => CallCommand::Speaker(true),
        ACTION_EARPIECE => CallCommand::Speaker(false),
        _ => return,
    };
    if in_call {
        // A button took the notification away: it comes back (quietly) once
        // the phone has done it, with the new state, unless the call ended.
        let Some(node) = core_host::node() else { return };
        let call = call.to_owned();
        core_host::spawn(async move {
            match node.call_command(device, call.clone(), command).await {
                Ok(()) if command == CallCommand::Decline => {}
                Ok(()) => {
                    // The phone's new state comes as its own event, right after.
                    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                    if let Some(active) = active(device).filter(|c| c.id == call) {
                        show_in_progress(device, &active);
                    }
                }
                Err(Error::NotFound) => show_message("That call is over."),
                Err(e) => show_message(describe(&e)),
            }
        });
        return;
    }
    let Some(node) = core_host::node() else { return };
    let call = call.to_owned();
    core_host::spawn(async move {
        match node.call_command(device, call.clone(), command).await {
            // Quiet now, but still ringing for the caller: the buttons stay.
            Ok(()) if command == CallCommand::Silence => {
                if let Some(ringing) = ringing(|r| r.get(&device).filter(|c| c.id == call).cloned()) {
                    show_ringing(device, &ringing, true);
                }
            }
            Ok(()) => {}
            Err(Error::NotFound) => show_message("That call is over."),
            Err(e) => show_message(describe(&e)),
        }
    });
}

// ---- Calls page: call log, contacts and dialing ----

#[derive(Debug, Default)]
struct PageState {
    device: Option<DeviceId>,
    log_status: Status,
    log: Vec<CallLogEntry>,
    more_log: bool,
    loading_older: bool,
    contacts_status: Status,
    contacts: Vec<Contact>,
    dialing: bool,
    /// Caller photos saved, by call log entry ID.
    log_photos: HashMap<String, PathBuf>,
    /// Contact photos saved, by contact ID.
    contact_photos: HashMap<String, PathBuf>,
    /// Bumped when the phone changes, so late answers are dropped.
    generation: u64,
}

static PAGE: Mutex<Option<PageState>> = Mutex::new(None);

fn page_state<T>(f: impl FnOnce(&mut PageState) -> T) -> T {
    f(PAGE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_default())
}

fn page_changed() {
    core_host::host().hub.changed(Changes::CALLS);
}

/// Drops in-memory call logs and contacts while the window is closed to the
/// tray; active calls (`IN_PROGRESS`) are kept for notifications.
pub fn release_idle_resources() {
    let mut released = false;
    page_state(|s| {
        if !s.dialing && (s.device.is_some() || !s.log.is_empty() || !s.contacts.is_empty()) {
            let generation = s.generation.wrapping_add(1);
            *s = PageState { generation, ..PageState::default() };
            released = true;
        }
    });
    if released {
        page_changed();
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub device: Option<DeviceId>,
    pub log_status: Status,
    pub call_log: Value,
    pub more_log: bool,
    pub loading_older: bool,
    pub contacts_status: Status,
    pub contacts: Value,
    pub dialing: bool,
}

pub fn view() -> View {
    page_state(|s| {
        // Match call log numbers to contacts for photos and fallback names.
        let mut by_digits: HashMap<String, (&str, Option<&PathBuf>)> = HashMap::new();
        for c in &s.contacts {
            let photo = s.contact_photos.get(&c.id);
            for n in &c.numbers {
                let d = digits(&n.number);
                if !d.is_empty() {
                    by_digits.entry(d).or_insert((c.name.as_str(), photo));
                }
            }
        }
        View {
            device: s.device,
            log_status: s.log_status,
            call_log: Value::Array(
                s.log
                    .iter()
                    .map(|e| {
                        let matched = digits(&e.number);
                        let from_contact =
                            (!matched.is_empty()).then(|| by_digits.get(&matched).copied()).flatten();
                        let photo = s
                            .log_photos
                            .get(&e.id)
                            .or_else(|| from_contact.and_then(|(_, p)| p))
                            .map(|p| crate::icons::file_url(p))
                            .unwrap_or_default();
                        let name = e
                            .name
                            .as_deref()
                            .filter(|n| !n.trim().is_empty())
                            .or_else(|| from_contact.map(|(n, _)| n).filter(|n| !n.trim().is_empty()))
                            .unwrap_or_default();
                        log_entry_json(e, name, photo)
                    })
                    .collect(),
            ),
            more_log: s.more_log,
            loading_older: s.loading_older,
            contacts_status: s.contacts_status,
            contacts: Value::Array(
                s.contacts.iter().map(|c| contact_json(c, s.contact_photos.get(&c.id))).collect(),
            ),
            dialing: s.dialing,
        }
    })
}

fn log_entry_json(e: &CallLogEntry, name: &str, photo: String) -> Value {
    json!({
        "id": e.id,
        "number": e.number,
        "name": name,
        "photo": photo,
        "direction": e.direction,
        "date": e.date,
        "duration": e.duration,
    })
}

fn contact_json(c: &Contact, photo: Option<&PathBuf>) -> Value {
    let numbers: Vec<Value> = c
        .numbers
        .iter()
        .map(|n| {
            json!({
                "number": n.number,
                "label": n.label.clone().unwrap_or_default(),
            })
        })
        .collect();
    json!({
        "id": c.id,
        "name": c.name,
        "numbers": numbers,
        "starred": c.starred,
        "photo": photo.map(|p| crate::icons::file_url(p)).unwrap_or_default(),
    })
}

/// Shows `device`'s call log and contacts (again: reloads).
pub fn open_device(device: DeviceId) {
    let same = page_state(|s| {
        let same = s.device == Some(device);
        if !same {
            *s = PageState { device: Some(device), generation: s.generation + 1, ..PageState::default() };
        }
        same
    });
    if !same {
        page_changed();
    }
    load_call_log(None);
    load_contacts();
}

/// Loads `device`'s contacts in the background if not already loaded for it
/// (used by Messages' recipient autocomplete).
pub fn ensure_contacts(device: DeviceId) {
    let should_load = page_state(|s| {
        if s.device != Some(device) {
            *s = PageState { device: Some(device), generation: s.generation + 1, ..PageState::default() };
            true
        } else {
            s.contacts.is_empty() && s.contacts_status != Status::Loading
        }
    });
    if should_load {
        load_contacts();
    }
}

pub fn reload() {
    load_call_log(None);
    load_contacts();
}

pub fn load_older() {
    let before = page_state(|s| {
        if s.loading_older || !s.more_log {
            return None;
        }
        s.loading_older = true;
        s.log.last().map(|e| e.date)
    });
    if before.is_some() {
        page_changed();
        load_call_log(before);
    }
}

fn load_call_log(before: Option<i64>) {
    let Some((device, generation)) = page_state(|s| {
        if before.is_none() && s.log.is_empty() {
            s.log_status = Status::Loading;
        }
        s.device.map(|d| (d, s.generation))
    }) else {
        return;
    };
    page_changed();
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        let result = node.call_log(device, before, LOG_PAGE).await;
        let photos = match &result {
            Ok(entries) => save_log_photos(device, entries),
            Err(_) => HashMap::new(),
        };
        page_state(|s| {
            s.loading_older = false;
            if s.generation != generation {
                return;
            }
            match result {
                Ok(page) => {
                    let full = page.len() as u32 >= LOG_PAGE;
                    if before.is_none() {
                        let oldest = page.last().map(|e| e.date).unwrap_or(i64::MAX);
                        let older: Vec<CallLogEntry> = s.log.drain(..).filter(|e| e.date < oldest).collect();
                        let had_older = !older.is_empty();
                        s.log = page;
                        s.log.extend(older);
                        s.more_log = full || (had_older && s.more_log);
                    } else {
                        s.log.extend(page);
                        s.more_log = full;
                    }
                    s.log_photos.extend(photos);
                    s.log_status = Status::Ready;
                    crate::bridge::app::update_home_calls(device, &s.log);
                }
                Err(e) => {
                    tracing::debug!(error = %e, "can't read call log");
                    s.log_status = Status::of(&e);
                }
            }
        });
        page_changed();
    });
}

fn load_contacts() {
    let Some((device, generation)) = page_state(|s| {
        if s.contacts.is_empty() {
            s.contacts_status = Status::Loading;
        }
        s.device.map(|d| (d, s.generation))
    }) else {
        return;
    };
    page_changed();
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        let mut all = Vec::new();
        let mut err = None;
        for page in 0..MAX_CONTACT_PAGES {
            let offset = page * CONTACTS_PAGE;
            match node.contacts(device, None, offset, CONTACTS_PAGE).await {
                Ok(items) => {
                    let full = items.len() as u32 >= CONTACTS_PAGE;
                    all.extend(items);
                    if !full {
                        break;
                    }
                }
                Err(e) => {
                    if page == 0 {
                        err = Some(e);
                    }
                    break;
                }
            }
        }
        let photos = if err.is_none() { save_contact_photos(device, &all) } else { HashMap::new() };
        page_state(|s| {
            if s.generation != generation {
                return;
            }
            match err {
                None => {
                    s.contacts = all;
                    s.contact_photos.extend(photos);
                    s.contacts_status = Status::Ready;
                }
                Some(e) => {
                    tracing::debug!(error = %e, "can't list contacts");
                    s.contacts_status = Status::of(&e);
                }
            }
        });
        page_changed();
    });
}

fn save_log_photos(device: DeviceId, entries: &[CallLogEntry]) -> HashMap<String, PathBuf> {
    let dir = crate::notifications::images_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return HashMap::new();
    }
    entries
        .iter()
        .filter_map(|e| {
            let photo = e.photo.as_ref()?;
            let path = dir.join(format!(
                "call-log-{:016x}.jpg",
                crate::photos::fingerprint(&format!("{device} {}", e.id))
                    ^ crate::photos::fingerprint_bytes(photo)
            ));
            if !path.exists() {
                std::fs::write(&path, photo).ok()?;
            }
            Some((e.id.clone(), path))
        })
        .collect()
}

fn save_contact_photos(device: DeviceId, contacts: &[Contact]) -> HashMap<String, PathBuf> {
    let dir = crate::notifications::images_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return HashMap::new();
    }
    contacts
        .iter()
        .filter_map(|c| {
            let photo = c.photo.as_ref()?;
            let path = dir.join(format!(
                "contact-{:016x}.jpg",
                crate::photos::fingerprint(&format!("{device} {}", c.id))
                    ^ crate::photos::fingerprint_bytes(photo)
            ));
            if !path.exists() {
                std::fs::write(&path, photo).ok()?;
            }
            Some((c.id.clone(), path))
        })
        .collect()
}

/// Why a call couldn't be placed, for the user.
pub fn dial_problem(error: &Error) -> &'static str {
    match error {
        Error::Denied => "Calls are turned off for this phone.",
        Error::Unsupported => {
            "The phone doesn't place calls for this PC. Allow it in the Nectarlink app on the phone."
        }
        Error::Offline | Error::NotPaired | Error::Timeout => "The phone isn't connected.",
        _ => "The phone couldn't place that call.",
    }
}

/// Asks `device` to dial `number`.
pub fn dial(device: DeviceId, number: String) {
    let number = number.trim().to_owned();
    if number.is_empty() {
        return;
    }
    page_state(|s| s.dialing = true);
    page_changed();
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        let result = node.call_dial(device, number).await;
        page_state(|s| s.dialing = false);
        page_changed();
        if let Err(e) = result {
            show_message(dial_problem(&e));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: Option<&str>, number: Option<&str>) -> CallState {
        CallState {
            id: "1".into(),
            state: "ringing".into(),
            incoming: true,
            number: number.map(Into::into),
            name: name.map(Into::into),
            photo: None,
            missed: false,
            since: None,
            controls: None,
        }
    }

    #[test]
    fn callers_are_named_as_well_as_known() {
        assert_eq!(caller(&call(Some("Sam"), Some("+1555"))), "Sam");
        assert_eq!(caller(&call(Some(" "), Some("+1555"))), "+1555");
        assert_eq!(caller(&call(None, None)), "Unknown caller");
    }

    #[test]
    fn call_log_and_contacts_view_matches_numbers_and_formats_json() {
        use nectarlink_core::ContactNumber;

        page_state(|s| {
            *s = PageState {
                log_status: Status::Ready,
                contacts_status: Status::Ready,
                log: vec![
                    CallLogEntry {
                        id: "10".into(),
                        number: "+1 (555) 010-1001".into(),
                        name: None,
                        photo: None,
                        direction: "missed".into(),
                        date: 1700000000000,
                        duration: 0,
                    },
                    CallLogEntry {
                        id: "9".into(),
                        number: "+15550109999".into(),
                        name: Some("Explicit Name".into()),
                        photo: None,
                        direction: "outgoing".into(),
                        date: 1699990000000,
                        duration: 95,
                    },
                ],
                contacts: vec![Contact {
                    id: "c1".into(),
                    name: "Alice Rivera".into(),
                    numbers: vec![ContactNumber {
                        number: "555-010-1001".into(),
                        label: Some("Mobile".into()),
                    }],
                    starred: true,
                    photo: None,
                }],
                ..PageState::default()
            };
        });

        let v = view();
        assert_eq!(v.log_status, Status::Ready);
        assert_eq!(v.contacts_status, Status::Ready);
        let log = v.call_log.as_array().unwrap();
        assert_eq!(log.len(), 2);
        // Matched to Alice Rivera by trailing digits!
        assert_eq!(log[0]["name"], "Alice Rivera");
        assert_eq!(log[0]["direction"], "missed");
        assert_eq!(log[1]["name"], "Explicit Name");
        assert_eq!(log[1]["duration"], 95);

        let contacts = v.contacts.as_array().unwrap();
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts[0]["name"], "Alice Rivera");
        assert_eq!(contacts[0]["starred"], true);
        assert_eq!(contacts[0]["numbers"][0]["label"], "Mobile");

        assert_eq!(dial_problem(&Error::Denied), "Calls are turned off for this phone.");
        assert_eq!(dial_problem(&Error::Offline), "The phone isn't connected.");
    }
}
