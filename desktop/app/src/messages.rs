// SPDX-License-Identifier: GPL-3.0-or-later
//! Text and chat messages on the PC (docs/protocol/sms.md,
//! docs/protocol/notifications.md §2.3): a phone's SMS conversations and
//! mirrored MessagingStyle chat conversations, read on demand and unified in
//! one inbox. Texts, MMS image attachments, and inline chat replies sent from
//! here go out through the phone.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Mutex,
};

use nectarlink_core::{
    ChatMessageRecord, ChatThreadRecord, DeviceId, Error, LinkState, NodeEvent, SmsAttachment, SmsMessage,
    SmsPart, SmsThread,
};
use serde_json::{Value, json};

use crate::{core_host, state::Changes};

/// Conversations listed, and messages read at a time.
const THREADS: u32 = 60;
const PAGE: u32 = 100;

/// Maximum MMS image attachment size (`nectarlink_protocol::messages::sms::MAX_ATTACHMENT_BYTES`).
pub const MAX_ATTACHMENT_BYTES: usize = 900 * 1024;

/// What the Messages page can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Status {
    /// No phone chosen yet.
    #[default]
    Idle,
    Loading,
    Ready,
    /// The phone isn't connected.
    Offline,
    /// Messages are off for this phone (here or there).
    Off,
    /// The phone doesn't share messages (no permission, or an old app).
    Unsupported,
    Failed,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Idle => "idle",
            Status::Loading => "loading",
            Status::Ready => "ready",
            Status::Offline => "offline",
            Status::Off => "off",
            Status::Unsupported => "unsupported",
            Status::Failed => "failed",
        }
    }

    pub(crate) fn of(error: &Error) -> Status {
        match error {
            Error::Denied => Status::Off,
            Error::Unsupported => Status::Unsupported,
            Error::Offline | Error::NotPaired | Error::Timeout => Status::Offline,
            _ => Status::Failed,
        }
    }
}

/// An image staged in the composer to be sent as an MMS attachment.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct StagedAttachment {
    name: String,
    mime: String,
    data: Vec<u8>,
    preview_path: Option<PathBuf>,
    preview_url: String,
    size_text: String,
    error: String,
}

#[derive(Debug, Default)]
struct State {
    device: Option<DeviceId>,
    status: Status,
    threads: Vec<SmsThread>,
    chat_threads: Vec<ChatThreadRecord>,
    /// The open conversation (`chat:...` for MessagingStyle chats, or SMS thread ID).
    thread: Option<String>,
    /// Its SMS messages, newest first.
    messages: Vec<SmsMessage>,
    /// Its MessagingStyle chat messages, when `thread` starts with `"chat:"`.
    chat_messages: Vec<ChatMessageRecord>,
    /// Older messages may exist (SMS threads only).
    more: bool,
    loading_older: bool,
    sending: bool,
    /// Pictures fetched, by part ID.
    pictures: HashMap<String, PathBuf>,
    /// Contact/conversation photos saved, by thread ID.
    photos: HashMap<String, PathBuf>,
    /// App icons saved, by Android package name.
    app_icons: HashMap<String, PathBuf>,
    /// A number pre-filled in the new-message box (from Calls / Contacts).
    compose_to: Option<String>,
    compose_name: Option<String>,
    /// Active app filter (`""` = All, `"sms"` = SMS only, or package name).
    app_filter: String,
    /// Staged MMS image attachment for the current SMS composer.
    staged_attachment: Option<StagedAttachment>,
    /// Bumped when the phone changes, so late answers are dropped.
    generation: u64,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn state<T>(f: impl FnOnce(&mut State) -> T) -> T {
    f(STATE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_default())
}

fn changed() {
    core_host::host().hub.changed(Changes::MESSAGES);
}

/// Drops in-memory SMS threads and messages while the window is closed to the
/// tray; reopening the Messages page reloads them.
pub fn release_idle_resources() {
    let mut released = false;
    state(|s| {
        if !s.sending
            && (s.device.is_some()
                || !s.threads.is_empty()
                || !s.chat_threads.is_empty()
                || !s.messages.is_empty()
                || !s.chat_messages.is_empty())
        {
            let generation = s.generation.wrapping_add(1);
            *s = State { generation, ..State::default() };
            released = true;
        }
    });
    if released {
        changed();
    }
}

fn files_dir() -> PathBuf {
    crate::notifications::images_dir()
}

fn is_message_cache_file(name: &str) -> bool {
    name.starts_with("sms-") || name.starts_with("mms-") || name.starts_with("chat-")
}

/// Clears cached SMS/MMS threads, messages, and downloaded attachment/avatar files on disk.
pub fn clear_cache() {
    if let Ok(entries) = std::fs::read_dir(files_dir()) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.file_name().and_then(|n| n.to_str()).is_some_and(is_message_cache_file) {
                let _ = std::fs::remove_file(path);
            }
        }
    }
    state(|s| {
        s.generation = s.generation.wrapping_add(1);
        s.threads.clear();
        s.chat_threads.clear();
        s.messages.clear();
        s.chat_messages.clear();
        s.pictures.clear();
        s.photos.clear();
        s.app_icons.clear();
        s.staged_attachment = None;
        s.more = false;
        s.loading_older = false;
    });
    changed();
}

/// Returns `(cached_sms_threads, cached_sms_messages, attachment_bytes)` for the
/// Data & storage summary in Settings.
pub fn cache_stats() -> (usize, usize, u64) {
    let (threads, messages) = state(|s| (s.threads.len(), s.messages.len()));
    let bytes: u64 = std::fs::read_dir(files_dir())
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let name_str = name.to_str()?;
            if !is_message_cache_file(name_str) {
                return None;
            }
            let meta = e.metadata().ok()?;
            meta.is_file().then_some(meta.len())
        })
        .sum();
    (threads, messages, bytes)
}

// ---- What the page shows ----

/// Everything the page shows, for QML.
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub device: Option<DeviceId>,
    pub status: Status,
    pub threads: Value,
    pub thread: Option<String>,
    pub messages: Value,
    pub more: bool,
    pub loading_older: bool,
    pub sending: bool,
    pub compose_to: Option<String>,
    pub compose_name: Option<String>,
    pub app_filter: String,
    pub available_apps: Value,
    pub has_chat_threads: bool,
    pub current_is_chat: bool,
    pub current_app_name: String,
    pub current_can_reply: bool,
    pub current_is_group: bool,
    pub has_attachment: bool,
    pub attachment_preview: String,
    pub attachment_name: String,
    pub attachment_size: String,
    pub attachment_error: String,
}

pub fn view() -> View {
    state(|s| {
        let mut combined: Vec<(i64, String, Value)> = Vec::new();
        if s.app_filter.is_empty() || s.app_filter == "sms" {
            for t in &s.threads {
                combined.push((t.date, t.id.clone(), thread_json(t, s.photos.get(&t.id))));
            }
        }
        if s.app_filter != "sms" {
            for ct in &s.chat_threads {
                if s.app_filter.is_empty() || ct.app == s.app_filter {
                    combined.push((
                        ct.updated_ms,
                        ct.thread_id.clone(),
                        chat_thread_json(ct, s.photos.get(&ct.thread_id), s.app_icons.get(&ct.app)),
                    ));
                }
            }
        }
        combined.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        let threads = Value::Array(combined.into_iter().map(|(_, _, v)| v).collect());

        let mut apps = vec![json!({ "key": "", "label": "All" }), json!({ "key": "sms", "label": "SMS" })];
        let mut seen = HashSet::new();
        for ct in &s.chat_threads {
            if !ct.app.is_empty() && seen.insert(ct.app.clone()) {
                let label = if ct.app_name.trim().is_empty() { ct.app.clone() } else { ct.app_name.clone() };
                apps.push(json!({ "key": ct.app, "label": label }));
            }
        }
        let available_apps = Value::Array(apps);

        let open_id = s.thread.as_deref();
        let current_is_chat = open_id.is_some_and(|id| id.starts_with("chat:"));
        let (current_app_name, current_can_reply, current_is_group) = if current_is_chat {
            let ct = open_id.and_then(|id| s.chat_threads.iter().find(|c| c.thread_id == id));
            (
                ct.map(|c| if c.app_name.trim().is_empty() { "Chat".into() } else { c.app_name.clone() })
                    .unwrap_or_else(|| "Chat".into()),
                ct.is_some_and(|c| c.active && c.reply_action_id.is_some()),
                ct.is_some_and(|c| c.is_group),
            )
        } else {
            let sms_thread = open_id.and_then(|id| s.threads.iter().find(|t| t.id == id));
            ("SMS".into(), true, sms_thread.is_some_and(|t| t.addresses.len() > 1))
        };

        let messages = if current_is_chat {
            let mut ordered = s.chat_messages.clone();
            ordered.sort_by(|a, b| a.time_ms.cmp(&b.time_ms).then_with(|| a.id.cmp(&b.id)));
            Value::Array(ordered.iter().map(chat_message_json).collect())
        } else {
            // In a group, who sent each message (by contact name when known).
            let thread = s.thread.as_ref().and_then(|id| s.threads.iter().find(|t| &t.id == id));
            let group = thread.filter(|t| t.addresses.len() > 1);
            Value::Array(
                s.messages
                    .iter()
                    .rev()
                    .map(|m| message_json(m, &s.pictures, group.map(|t| sender_of(t, &m.address))))
                    .collect(),
            )
        };

        let (has_attachment, attachment_preview, attachment_name, attachment_size, attachment_error) =
            match &s.staged_attachment {
                Some(att) => (
                    true,
                    att.preview_url.clone(),
                    att.name.clone(),
                    att.size_text.clone(),
                    att.error.clone(),
                ),
                None => (false, String::new(), String::new(), String::new(), String::new()),
            };

        View {
            device: s.device,
            status: s.status,
            threads,
            thread: s.thread.clone(),
            messages,
            more: if current_is_chat { false } else { s.more },
            loading_older: if current_is_chat { false } else { s.loading_older },
            sending: s.sending,
            compose_to: s.compose_to.clone(),
            compose_name: s.compose_name.clone(),
            app_filter: s.app_filter.clone(),
            available_apps,
            has_chat_threads: !s.chat_threads.is_empty(),
            current_is_chat,
            current_app_name,
            current_can_reply,
            current_is_group,
            has_attachment,
            attachment_preview,
            attachment_name,
            attachment_size,
            attachment_error,
        }
    })
}

/// Who a conversation is with: contact names, or numbers.
pub fn title_of(thread: &SmsThread) -> String {
    thread
        .addresses
        .iter()
        .enumerate()
        .map(|(i, address)| thread.names.get(i).filter(|n| !n.trim().is_empty()).unwrap_or(address).clone())
        .collect::<Vec<_>>()
        .join(", ")
}

fn thread_json(t: &SmsThread, photo: Option<&PathBuf>) -> Value {
    json!({
        "id": t.id,
        "title": title_of(t),
        "addresses": t.addresses.join(", "),
        "snippet": t.snippet,
        "date": t.date,
        "unread": t.unread,
        "group": t.addresses.len() > 1,
        "photo": photo.map(|p| crate::icons::file_url(p)).unwrap_or_default(),
        "isChat": false,
        "app": "",
        "appName": "SMS",
        "appIcon": "",
        "canReply": true,
        "lastSender": "",
        "active": true,
    })
}

fn chat_thread_json(t: &ChatThreadRecord, photo: Option<&PathBuf>, app_icon: Option<&PathBuf>) -> Value {
    json!({
        "id": t.thread_id,
        "title": t.title,
        "addresses": "",
        "snippet": t.snippet,
        "date": t.updated_ms,
        "unread": t.unread,
        "group": t.is_group,
        "photo": photo.map(|p| crate::icons::file_url(p)).unwrap_or_default(),
        "isChat": true,
        "app": t.app,
        "appName": if t.app_name.trim().is_empty() { &t.app } else { &t.app_name },
        "appIcon": app_icon.map(|p| crate::icons::file_url(p)).unwrap_or_default(),
        "canReply": t.active && t.reply_action_id.is_some(),
        "lastSender": t.last_sender.clone().unwrap_or_default(),
        "active": t.active,
    })
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// A group member's name, or their number.
fn sender_of(thread: &SmsThread, address: &str) -> String {
    let wanted = digits(address);
    thread
        .addresses
        .iter()
        .position(|a| digits(a) == wanted)
        .and_then(|i| thread.names.get(i))
        .filter(|n| !n.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| address.to_owned())
}

/// A one-time code in a text ("243928 is your OTP"): 4 to 8 digits (or
/// common alphanumeric formats like "G-123456") near an OTP keyword.
pub fn one_time_code(body: &str) -> Option<String> {
    nectarlink_core::otp::one_time_code(body)
}

fn message_json(m: &SmsMessage, pictures: &HashMap<String, PathBuf>, sender: Option<String>) -> Value {
    let images: Vec<Value> = m
        .parts
        .iter()
        .filter(|p| p.mime.starts_with("image/"))
        .map(|p| json!(pictures.get(&p.id).map(|f| crate::icons::file_url(f)).unwrap_or_default()))
        .collect();
    let others = m.parts.iter().filter(|p| !p.mime.starts_with("image/")).count();
    json!({
        "id": m.id,
        "body": m.body,
        "date": m.date,
        "outgoing": m.outgoing,
        "status": m.status.clone().unwrap_or_default(),
        "images": images,
        "attachments": others,
        "sender": if m.outgoing { None } else { sender },
        "code": if m.outgoing { None } else { one_time_code(&m.body) },
    })
}

fn chat_message_json(m: &ChatMessageRecord) -> Value {
    let sender = if m.self_sent {
        None
    } else {
        m.sender.as_ref().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
    };
    json!({
        "id": m.id.to_string(),
        "body": m.text,
        "date": m.time_ms,
        "outgoing": m.self_sent,
        "status": if m.id < 0 { "pending" } else { "" },
        "images": [],
        "attachments": 0,
        "sender": sender,
        "code": if m.self_sent { None } else { one_time_code(&m.text) },
    })
}

// ---- What the page asks ----

/// Sets the active app filter (`""` for all, `"sms"` for SMS only, or package name).
pub fn set_app_filter(app: String) {
    let app = app.trim().to_owned();
    let updated = state(|s| {
        if s.app_filter == app {
            false
        } else {
            s.app_filter = app;
            true
        }
    });
    if updated {
        changed();
    }
}

/// Deletes a persisted MessagingStyle chat conversation locally.
pub fn delete_chat_thread(thread_id: String) {
    let Some(device) = state(|s| s.device) else { return };
    let Some(node) = core_host::node() else { return };
    if node.delete_chat_thread(device, &thread_id) {
        state(|s| {
            s.chat_threads.retain(|t| t.thread_id != thread_id);
            if s.thread.as_deref() == Some(&thread_id) {
                s.thread = None;
                s.chat_messages.clear();
            }
        });
        changed();
    }
}

/// Shows `device`'s messages (again: reloads).
pub fn open_device(device: DeviceId) {
    let same = state(|s| {
        let same = s.device == Some(device);
        if !same {
            *s = State { device: Some(device), generation: s.generation + 1, ..State::default() };
        }
        same
    });
    if !same {
        changed();
    }
    load_threads();
}

/// Synchronously refreshes persisted `MessagingStyle` chat threads from SQLite.
pub fn load_chat_threads() {
    let Some(device) = state(|s| s.device) else { return };
    let Some(node) = core_host::node() else { return };
    let chat_threads = node.chat_threads(device);
    let (photos, app_icons) = save_chat_assets(device, &chat_threads);
    let open_chat = state(|s| {
        s.chat_threads = chat_threads;
        s.photos.extend(photos);
        s.app_icons.extend(app_icons);
        if !s.chat_threads.is_empty() && matches!(s.status, Status::Idle | Status::Loading) {
            s.status = Status::Ready;
        }
        if !s.app_filter.is_empty()
            && s.app_filter != "sms"
            && !s.chat_threads.iter().any(|ct| ct.app == s.app_filter)
        {
            s.app_filter.clear();
        }
        s.thread.clone().filter(|t| t.starts_with("chat:"))
    });
    if let Some(thread) = open_chat {
        let _ = node.mark_chat_thread_read(device, &thread);
        let msgs = node.chat_messages(device, &thread, PAGE);
        let refreshed = node.chat_threads(device);
        state(|s| {
            s.chat_threads = refreshed;
            if s.thread.as_deref() == Some(&thread) {
                s.chat_messages = msgs;
            }
        });
    }
    changed();
}

pub fn load_threads() {
    load_chat_threads();
    let Some((device, generation)) = state(|s| {
        if s.threads.is_empty() && s.chat_threads.is_empty() {
            s.status = Status::Loading;
        }
        s.device.map(|d| (d, s.generation))
    }) else {
        return;
    };
    changed();
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        let result = node.sms_threads(device, THREADS).await;
        let photos = match &result {
            Ok(threads) => save_photos(device, threads),
            Err(_) => HashMap::new(),
        };
        state(|s| {
            if s.generation != generation {
                return;
            }
            match result {
                Ok(threads) => {
                    crate::bridge::app::update_home_sms(device, &threads);
                    s.threads = threads;
                    s.photos.extend(photos);
                    s.status = Status::Ready;
                }
                Err(e) => {
                    tracing::debug!(error = %e, "can't list conversations");
                    if s.chat_threads.is_empty() {
                        s.status = Status::of(&e);
                    } else {
                        s.status = Status::Ready;
                    }
                }
            }
        });
        changed();
        open_pending();
    });
}

/// Contact photos as files, for QML.
fn save_photos(device: DeviceId, threads: &[SmsThread]) -> HashMap<String, PathBuf> {
    let dir = files_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return HashMap::new();
    }
    threads
        .iter()
        .filter_map(|t| {
            let photo = t.photo.as_ref()?;
            let path = dir.join(format!(
                "sms-{:016x}.jpg",
                crate::photos::fingerprint(&format!("{device} {}", t.id))
                    ^ crate::photos::fingerprint_bytes(photo)
            ));
            if !path.exists() {
                std::fs::write(&path, photo).ok()?;
            }
            Some((t.id.clone(), path))
        })
        .collect()
}

/// Saves chat conversation avatars and app icons to disk for QML `RoundedImage`.
fn save_chat_assets(
    device: DeviceId,
    threads: &[ChatThreadRecord],
) -> (HashMap<String, PathBuf>, HashMap<String, PathBuf>) {
    let dir = files_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return (HashMap::new(), HashMap::new());
    }
    let mut photos = HashMap::new();
    let mut app_icons = HashMap::new();
    for t in threads {
        if let Some(avatar) = t.avatar.as_ref().filter(|b| !b.is_empty()) {
            let ext = if avatar.starts_with(b"\x89PNG") { "png" } else { "jpg" };
            let path = dir.join(format!(
                "chat-avatar-{:016x}.{ext}",
                crate::photos::fingerprint(&format!("{device} {}", t.thread_id))
                    ^ crate::photos::fingerprint_bytes(avatar)
            ));
            if path.exists() || std::fs::write(&path, avatar).is_ok() {
                photos.insert(t.thread_id.clone(), path);
            }
        }
        if !app_icons.contains_key(&t.app)
            && let Some(icon) = t.app_icon.as_ref().filter(|b| !b.is_empty())
        {
            let path = dir.join(format!(
                "chat-app-{:016x}.png",
                crate::photos::fingerprint(&format!("{device} {}", t.app))
                    ^ crate::photos::fingerprint_bytes(icon)
            ));
            if path.exists() || std::fs::write(&path, icon).is_ok() {
                app_icons.insert(t.app.clone(), path);
            }
        }
    }
    (photos, app_icons)
}

/// Opens a conversation: its latest messages.
pub fn open_thread(thread: String) {
    let is_chat = thread.starts_with("chat:");
    let device = state(|s| {
        s.compose_to = None;
        s.compose_name = None;
        if s.thread.as_deref() != Some(&thread) {
            s.thread = Some(thread.clone());
            s.messages.clear();
            s.chat_messages.clear();
            s.more = false;
            s.staged_attachment = None;
        }
        s.device
    });
    if is_chat {
        if let (Some(device), Some(node)) = (device, core_host::node()) {
            let _ = node.mark_chat_thread_read(device, &thread);
            let msgs = node.chat_messages(device, &thread, PAGE);
            let refreshed = node.chat_threads(device);
            state(|s| {
                s.chat_threads = refreshed;
                if s.thread.as_deref() == Some(&thread) {
                    s.chat_messages = msgs;
                }
            });
        }
        changed();
    } else {
        changed();
        load_messages(None);
    }
}

/// Opens an existing 1-on-1 conversation with `number` on `device`, or starts
/// a new one with `number` pre-filled.
pub fn start_chat(device: DeviceId, number: String, name: String) {
    let number = number.trim().to_owned();
    if number.is_empty() {
        return;
    }
    let name = name.trim().to_owned();
    let wanted = digits(&number);
    let existing = state(|s| {
        if s.device != Some(device) {
            *s = State { device: Some(device), generation: s.generation + 1, ..State::default() };
        }
        let found = (!wanted.is_empty()).then(|| {
            s.threads
                .iter()
                .find(|t| t.addresses.len() == 1 && digits(&t.addresses[0]) == wanted)
                .map(|t| t.id.clone())
        })?;
        if found.is_none() {
            s.thread = None;
            s.messages.clear();
            s.chat_messages.clear();
            s.more = false;
            s.compose_to = Some(number.clone());
            s.compose_name = (!name.is_empty() && name != number).then_some(name);
        } else {
            s.compose_to = None;
            s.compose_name = None;
        }
        found
    });
    if let Some(thread) = existing {
        PENDING_OPEN.lock().unwrap_or_else(|e| e.into_inner()).take();
        open_thread(thread);
        load_threads();
    } else {
        if !wanted.is_empty() {
            PENDING_OPEN.lock().unwrap_or_else(|e| e.into_inner()).replace(vec![wanted]);
        }
        changed();
        load_threads();
    }
}

/// Returns cached `(display_name, address_or_app, thread_id)` triples for
/// `device` synchronously (0 ms) for the Command Palette, and triggers
/// background thread loading if not yet cached.
pub fn palette_recipients(device: DeviceId) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let should_load_sms = state(|s| {
        if s.device == Some(device) {
            for t in &s.threads {
                let title = title_of(t);
                let addr = t.addresses.first().cloned().unwrap_or_default();
                out.push((title, addr, t.id.clone()));
            }
            s.threads.is_empty() && s.status != Status::Loading
        } else if s.device.is_none() {
            *s = State { device: Some(device), generation: s.generation + 1, ..State::default() };
            true
        } else {
            false
        }
    });
    if let Some(node) = core_host::node() {
        for ct in node.chat_threads(device) {
            let app_label = if ct.app_name.trim().is_empty() { ct.app } else { ct.app_name };
            out.push((ct.title, app_label, ct.thread_id));
        }
    }
    if should_load_sms {
        load_threads();
    }
    out
}

/// Opens a specific SMS or chat thread on `device` from the Command Palette.
pub fn open_thread_from_palette(device: DeviceId, thread_id: &str) {
    let thread_id = thread_id.trim();
    if thread_id.is_empty() {
        return;
    }
    open_device(device);
    open_thread(thread_id.to_owned());
}

/// The open conversation's older messages.
pub fn load_older() {
    let before = state(|s| {
        if s.loading_older || !s.more || s.thread.as_deref().is_some_and(|t| t.starts_with("chat:")) {
            return None;
        }
        s.loading_older = true;
        s.messages.last().map(|m| m.date)
    });
    if before.is_some() {
        changed();
        load_messages(before);
    }
}

fn load_messages(before: Option<i64>) {
    let Some((device, thread, generation)) = state(|s| Some((s.device?, s.thread.clone()?, s.generation)))
    else {
        return;
    };
    if thread.starts_with("chat:") {
        load_chat_threads();
        return;
    }
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        let result = node.sms_messages(device, thread.clone(), before, PAGE).await;
        let wanted = state(|s| {
            s.loading_older = false;
            if s.generation != generation || s.thread.as_deref() != Some(&thread) {
                return Vec::new();
            }
            match result {
                Ok(page) => {
                    let full = page.len() as u32 >= PAGE;
                    if before.is_none() {
                        // The latest page replaces what was there, keeping
                        // older pages already read.
                        let oldest = page.last().map(|m| m.date).unwrap_or(i64::MAX);
                        let older: Vec<SmsMessage> =
                            s.messages.drain(..).filter(|m| m.date < oldest).collect();
                        let had_older = !older.is_empty();
                        s.messages = page;
                        s.messages.extend(older);
                        s.more = full || had_older && s.more;
                    } else {
                        s.messages.extend(page);
                        s.more = full;
                    }
                    s.messages
                        .iter()
                        .flat_map(|m| m.parts.iter())
                        .filter(|p| p.mime.starts_with("image/") && !s.pictures.contains_key(&p.id))
                        .map(|p| p.id.clone())
                        .collect()
                }
                Err(e) => {
                    tracing::debug!(error = %e, "can't read a conversation");
                    if s.chat_threads.is_empty() {
                        s.status = Status::of(&e);
                    }
                    Vec::new()
                }
            }
        });
        changed();
        fetch_pictures(device, generation, wanted).await;
    });
}

async fn fetch_pictures(device: DeviceId, generation: u64, parts: Vec<String>) {
    let Some(node) = core_host::node() else { return };
    let dir = files_dir();
    for id in parts {
        let Ok((mime, data)) = node.sms_part(device, id.clone()).await else { continue };
        let ext = if mime.contains("png") {
            "png"
        } else if mime.contains("gif") {
            "gif"
        } else {
            "jpg"
        };
        let path = dir
            .join(format!("sms-part-{:016x}.{ext}", crate::photos::fingerprint(&format!("{device} {id}"))));
        if std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, &data)).is_err() {
            continue;
        }
        let current = state(|s| {
            let current = s.generation == generation;
            if current {
                s.pictures.insert(id, path);
            }
            current
        });
        if !current {
            return;
        }
        changed();
    }
}

// ---- MMS Attachment Staging ----

/// Formats a byte count into a human-readable string (`245 KB`, `1.4 MB`).
pub fn format_attachment_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        let kb = (bytes + 512) / 1024;
        format!("{} KB", kb.max(1))
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn url_to_path(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    let stripped = if let Some(rest) = trimmed.strip_prefix("file:///") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("file://") {
        rest
    } else {
        return PathBuf::from(trimmed);
    };
    let bytes = stripped.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3])
            && let Ok(val) = u8::from_str_radix(hex, 16)
        {
            out.push(val);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    PathBuf::from(String::from_utf8_lossy(&out).into_owned())
}

fn detect_or_convert_image(path: &Path, raw: Vec<u8>) -> Result<(String, Vec<u8>), String> {
    if raw.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Ok(("image/png".into(), raw));
    }
    if raw.starts_with(b"\xff\xd8\xff") {
        return Ok(("image/jpeg".into(), raw));
    }
    if raw.starts_with(b"GIF87a") || raw.starts_with(b"GIF89a") {
        return Ok(("image/gif".into(), raw));
    }
    if raw.len() >= 12 && &raw[0..4] == b"RIFF" && &raw[8..12] == b"WEBP" {
        return Ok(("image/webp".into(), raw));
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    match ext.as_str() {
        "png" => return Ok(("image/png".into(), raw)),
        "jpg" | "jpeg" => return Ok(("image/jpeg".into(), raw)),
        "gif" => return Ok(("image/gif".into(), raw)),
        "webp" => return Ok(("image/webp".into(), raw)),
        _ => {}
    }
    // Convert BMP/TIFF/ICO or other Windows-decodable images to PNG.
    let bitmap = crate::win::image::decode(&raw)
        .map_err(|_| "Only image files (PNG, JPEG, WebP, GIF, BMP) can be attached.".to_owned())?;
    let png = crate::win::image::encode_png(&bitmap).map_err(|e| e.to_string())?;
    Ok(("image/png".into(), png))
}

fn stage_image_bytes(name: String, mime: String, data: Vec<u8>) -> bool {
    let size_text = format_attachment_size(data.len());
    let ext = if mime.contains("png") {
        "png"
    } else if mime.contains("gif") {
        "gif"
    } else if mime.contains("webp") {
        "webp"
    } else {
        "jpg"
    };
    let dir = files_dir();
    let preview_path = std::fs::create_dir_all(&dir).ok().and_then(|()| {
        let p = dir.join(format!("mms-stage-{:016x}.{ext}", crate::photos::fingerprint_bytes(&data)));
        (p.exists() || std::fs::write(&p, &data).is_ok()).then_some(p)
    });
    let preview_url = preview_path.as_ref().map(|p| crate::icons::file_url(p)).unwrap_or_default();
    let (data, error) = if data.len() > MAX_ATTACHMENT_BYTES {
        (Vec::new(), format!("Image is {size_text} — MMS supports up to 900 KB"))
    } else {
        (data, String::new())
    };
    state(|s| {
        s.staged_attachment =
            Some(StagedAttachment { name, mime, data, preview_path, preview_url, size_text, error });
    });
    changed();
    true
}

/// Stages an image file from disk (via drag-and-drop or file picker) as an MMS attachment.
pub fn attach_file(path_or_url: String) -> bool {
    let path = url_to_path(&path_or_url);
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("image.png").to_owned();
    let raw = match std::fs::read(&path) {
        Ok(b) if !b.is_empty() => b,
        _ => return false,
    };
    match detect_or_convert_image(&path, raw) {
        Ok((mime, data)) => stage_image_bytes(name, mime, data),
        Err(error) => {
            state(|s| {
                s.staged_attachment = Some(StagedAttachment {
                    name,
                    mime: String::new(),
                    data: Vec::new(),
                    preview_path: None,
                    preview_url: String::new(),
                    size_text: String::new(),
                    error,
                });
            });
            changed();
            false
        }
    }
}

/// Stages an image from the Windows clipboard (`Ctrl+V`) as an MMS attachment,
/// returning `true` when an image was on the clipboard.
pub fn paste_clipboard_image() -> bool {
    let Some(png) = crate::win::clipboard::read_image_png() else {
        return false;
    };
    stage_image_bytes("Clipboard image.png".into(), "image/png".into(), png)
}

/// Clears any staged MMS image attachment.
pub fn clear_attachment() {
    let had = state(|s| s.staged_attachment.take().is_some());
    if had {
        changed();
    }
}

/// Why a text wasn't sent, for the page.
pub fn send_problem(error: &Error) -> &'static str {
    match error {
        Error::Denied => "Messages are turned off for this phone.",
        Error::Unsupported => {
            "The phone doesn't send texts for this PC. Allow it in the Nectarlink app on the phone."
        }
        Error::Offline | Error::NotPaired | Error::Timeout => "The phone isn't connected.",
        Error::TooLarge => "That attachment or message is too large (MMS supports up to 900 KB).",
        _ => "The phone couldn't send it.",
    }
}

fn chat_reply_problem(error: &Error) -> &'static str {
    match error {
        Error::Denied => "Notifications are turned off for this phone.",
        Error::Offline | Error::NotPaired | Error::Timeout => "The phone isn't connected.",
        _ => "Couldn't send that reply — the notification may have been dismissed on your phone.",
    }
}

/// Sends a text or inline chat reply in the open conversation.
pub fn send(body: String) {
    let open = state(|s| s.thread.clone());
    if let Some(thread) = open.as_deref()
        && thread.starts_with("chat:")
    {
        send_chat_reply(thread.to_owned(), body);
        return;
    }
    let to = state(|s| {
        let thread = s.thread.as_ref()?;
        s.threads.iter().find(|t| &t.id == thread).map(|t| t.addresses.clone())
    });
    match to {
        Some(to) if to.len() == 1 => send_to(to, body),
        Some(_) => crate::bridge::app::show_message("Group texts can't be sent from the PC yet."),
        None => {}
    }
}

fn send_chat_reply(thread_id: String, body: String) {
    let text = body.trim().to_owned();
    if text.is_empty() {
        return;
    }
    let now = now_ms();
    let local_id = -now.max(1);
    let Some(device) = state(|s| {
        s.sending = true;
        s.chat_messages.push(ChatMessageRecord {
            id: local_id,
            peer: s.device?,
            thread_id: thread_id.clone(),
            sender: None,
            text: text.clone(),
            time_ms: now,
            self_sent: true,
            avatar: None,
        });
        s.device
    }) else {
        return;
    };
    changed();
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        let result = node.reply_chat_thread(device, &thread_id, text).await;
        let refreshed_threads = node.chat_threads(device);
        let refreshed_messages = node.chat_messages(device, &thread_id, PAGE);
        state(|s| {
            s.sending = false;
            s.chat_threads = refreshed_threads;
            if s.thread.as_deref() == Some(&thread_id) {
                s.chat_messages = refreshed_messages;
            }
        });
        changed();
        if let Err(e) = result {
            crate::bridge::app::show_message(chat_reply_problem(&e));
        }
    });
}

/// Sends a text (and optional staged MMS image attachment) to a number, then opens it.
pub fn send_to(to: Vec<String>, body: String) {
    static NEXT_LOCAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let now = now_ms();
    let seq = NEXT_LOCAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let local_id = format!("local:{now}:{seq}");
    let Some((device, attachments)) = state(|s| {
        let valid_att =
            s.staged_attachment.as_ref().filter(|a| a.error.is_empty() && !a.data.is_empty()).cloned();
        if body.trim().is_empty() && valid_att.is_none() {
            return None;
        }
        s.staged_attachment = None;
        s.sending = true;
        s.compose_to = None;
        s.compose_name = None;
        let mut parts = Vec::new();
        let mut attachments = Vec::new();
        if let Some(att) = valid_att {
            let part_id = format!("local-part:{now}:{seq}");
            if let Some(path) = att.preview_path {
                s.pictures.insert(part_id.clone(), path);
            }
            parts.push(SmsPart { id: part_id, mime: att.mime.clone(), size: att.data.len() as u64 });
            attachments.push(SmsAttachment { mime: att.mime, data: att.data });
        }
        // Shown at once as "Sending…"; the phone's copy replaces it.
        if let Some(thread) = s.thread.clone() {
            s.messages.insert(
                0,
                SmsMessage {
                    id: local_id.clone(),
                    thread,
                    address: to.join(", "),
                    body: body.clone(),
                    date: now,
                    outgoing: true,
                    status: Some("pending".into()),
                    parts,
                },
            );
        }
        Some((s.device?, attachments))
    }) else {
        return;
    };
    changed();
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        let result = if attachments.is_empty() {
            node.sms_send(device, to.clone(), body).await
        } else {
            node.send_sms_with_attachments(device, to.clone(), body, attachments).await
        };
        state(|s| {
            s.sending = false;
            if result.is_err() {
                // Not sent: say so on it.
                if let Some(m) = s.messages.iter_mut().find(|m| m.id == local_id) {
                    m.status = Some("failed".into());
                }
            }
        });
        changed();
        match result {
            Ok(()) => {
                // The phone also says so when it has stored it; this is sooner.
                tokio::time::sleep(std::time::Duration::from_millis(600)).await;
                reload_after_send(&to);
            }
            Err(e) => crate::bridge::app::show_message(send_problem(&e)),
        }
    });
}

fn reload_after_send(to: &[String]) {
    let open = state(|s| s.thread.clone());
    if open.is_none() {
        // A new conversation: open it once the phone lists it.
        let wanted: Vec<String> = to.iter().map(|a| digits(a)).collect();
        PENDING_OPEN.lock().unwrap_or_else(|e| e.into_inner()).replace(wanted);
    }
    load_threads();
    if open.is_some() {
        load_messages(None);
    }
}

/// A new conversation to open once it shows up, by its numbers' digits.
static PENDING_OPEN: Mutex<Option<Vec<String>>> = Mutex::new(None);

/// The last digits of a number, to match "+1 555-0100" with "5550100".
pub(crate) fn digits(number: &str) -> String {
    let all: String = number.chars().filter(char::is_ascii_digit).collect();
    all[all.len().saturating_sub(9)..].to_owned()
}

/// Reads the conversations and the open one again.
pub fn reload() {
    load_threads();
    if state(|s| s.thread.is_some()) {
        load_messages(None);
    }
}

/// Closes the open conversation (for writing a new one).
pub fn close_thread() {
    PENDING_OPEN.lock().unwrap_or_else(|e| e.into_inner()).take();
    state(|s| {
        s.thread = None;
        s.compose_to = None;
        s.compose_name = None;
        s.messages.clear();
        s.chat_messages.clear();
        s.more = false;
        s.staged_attachment = None;
    });
    changed();
}

pub fn on_event(event: &NodeEvent) {
    let device = state(|s| s.device);
    match event {
        NodeEvent::SmsChanged { device: d, .. } => {
            if Some(*d) == device {
                load_threads();
                if state(|s| s.thread.is_some()) {
                    load_messages(None);
                }
            }
            if crate::notifications::auto_copy_otp_enabled()
                && let Some(node) = core_host::node()
            {
                let dev = *d;
                core_host::spawn(async move {
                    if let Ok(threads) = node.sms_threads(dev, 1).await
                        && let Some(top) = threads.first()
                        && top.unread > 0
                        && now_ms().saturating_sub(top.date) < 60_000
                        && let Some(code) = one_time_code(&top.snippet)
                    {
                        crate::notifications::maybe_auto_copy_otp(&code);
                    }
                });
            }
        }
        NodeEvent::NotificationPosted { device: d, .. }
        | NodeEvent::NotificationRemoved { device: d, .. }
        | NodeEvent::NotificationsReset { device: d, .. } => {
            if Some(*d) == device {
                load_chat_threads();
            }
        }
        NodeEvent::LinkChanged { device: d, link: LinkState::Online { .. } } if Some(*d) == device => {
            load_threads();
        }
        NodeEvent::LinkChanged { device: d, link: LinkState::Offline { .. } } if Some(*d) == device => {
            load_chat_threads();
            state(|s| {
                if s.threads.is_empty() && s.chat_threads.is_empty() {
                    s.status = Status::Offline;
                }
            });
            changed();
        }
        _ => {}
    }
}

/// Opens a conversation just started from the PC, once it's listed.
fn open_pending() {
    let Some(wanted) = PENDING_OPEN.lock().unwrap_or_else(|e| e.into_inner()).clone() else { return };
    let found = state(|s| {
        s.threads
            .iter()
            .find(|t| {
                let have: Vec<String> = t.addresses.iter().map(|a| digits(a)).collect();
                have == wanted
            })
            .map(|t| t.id.clone())
    });
    if let Some(thread) = found {
        PENDING_OPEN.lock().unwrap_or_else(|e| e.into_inner()).take();
        open_thread(thread);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread(addresses: &[&str], names: &[&str]) -> SmsThread {
        SmsThread {
            id: "1".into(),
            addresses: addresses.iter().map(|a| (*a).into()).collect(),
            names: names.iter().map(|n| (*n).into()).collect(),
            snippet: String::new(),
            date: 0,
            unread: 0,
            photo: None,
        }
    }

    #[test]
    fn conversations_are_named_by_contact_or_number() {
        assert_eq!(title_of(&thread(&["+1555"], &["Sam"])), "Sam");
        assert_eq!(title_of(&thread(&["+1555", "+1666"], &["Sam", ""])), "Sam, +1666");
        assert_eq!(title_of(&thread(&["+1555"], &[])), "+1555");
    }

    #[test]
    fn one_time_codes_are_found() {
        let found = |body: &str| one_time_code(body);
        assert_eq!(
            found(
                "Dear Customer,
243928 is your one time password (OTP)."
            ),
            Some("243928".into())
        );
        assert_eq!(found("840941 is your OTP to create JioID"), Some("840941".into()));
        assert_eq!(found("740421 is OTP for Aadhaar (XX3465) valid for 10 mins"), Some("740421".into()));
        // Google's "G-482193" is typed without the "G-".
        assert_eq!(found("G-482193 is your Google verification code."), Some("482193".into()));
        assert_eq!(found("Your code is AB4821CD"), None, "letters stuck to it");
        assert_eq!(found("Use code 1234-5678"), Some("12345678".into()));
        assert_eq!(found("Payment received for Rs. 2000 - thanks"), None, "no code words");
        assert_eq!(found("Your code is ready. Pay Rs.5000 at the counter"), None, "an amount");
        assert_eq!(found("OTP 12"), None, "too short");
        assert_eq!(found("PIN 1,234 used"), None, "a formatted number");
    }

    #[test]
    fn numbers_match_however_they_are_written() {
        assert_eq!(digits("+1 (555) 010-0100"), digits("5550100100"));
        assert_eq!(digits("12"), "12");
    }

    #[test]
    fn attachment_sizes_and_url_paths_are_formatted() {
        assert_eq!(format_attachment_size(512), "512 B");
        assert_eq!(format_attachment_size(245 * 1024), "245 KB");
        assert_eq!(format_attachment_size(1_468_006), "1.4 MB");
        assert_eq!(
            url_to_path("file:///C:/Users/Sam/My%20Photo.png"),
            PathBuf::from("C:/Users/Sam/My Photo.png")
        );
    }
}
