// SPDX-License-Identifier: GPL-3.0-or-later
//! Text and chat messages on the PC (docs/protocol/sms.md,
//! docs/protocol/notifications.md §2.3): a phone's SMS conversations and
//! mirrored MessagingStyle chat conversations, read on demand and unified in
//! one inbox. Texts, MMS image attachments, and inline chat replies sent from
//! here go out through the phone.

use crate::i18n::tr;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use nectarlink_core::{
    CallSims, ChatMessageRecord, ChatThreadRecord, DeviceId, Error, LinkState, NodeEvent, SmsAttachment,
    SmsMessage, SmsPart, SmsThread,
};
use serde_json::{Value, json};

use crate::{
    core_host, messages_local,
    state::{AppState, Changes},
};

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
    /// Conversations read lately, so opening one again is instant while the
    /// phone is asked for what's new. The most recent is last.
    recent: Vec<(String, Vec<SmsMessage>)>,
    /// When each conversation was last read here, by thread ID. Android lets only
    /// the default SMS app mark texts read, so the phone keeps counting them.
    seen: HashMap<String, i64>,
    /// The SIMs the phone texts from (#474), once asked.
    sims: CallSims,
    sims_loaded: bool,
    /// The SIM picked here, by conversation ("" for a new one).
    chosen_sims: HashMap<String, String>,
    /// Bumped when the phone changes, so late answers are dropped.
    generation: u64,
    /// Chat replies that didn't go out, kept (marked "Not sent") when the
    /// conversation is read again from the database, which never had them.
    failed_replies: Vec<ChatMessageRecord>,
    /// The text a notification opened this conversation for, to show and
    /// highlight (SMS threads only).
    focus_message: Option<String>,
}

/// What was typed and not sent, by `"<device>|<conversation>"` (`""` for a new
/// message), kept while the app runs so a page change doesn't lose it.
static DRAFTS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

/// Keeps `text` as the draft for `key`; an empty one is dropped.
pub fn save_draft(key: String, text: String) {
    let mut drafts = DRAFTS.lock().unwrap_or_else(|e| e.into_inner());
    let drafts = drafts.get_or_insert_default();
    if text.is_empty() {
        drafts.remove(&key);
    } else {
        drafts.insert(key, text);
    }
}

/// The draft kept for `key`, or nothing.
pub fn draft(key: &str) -> String {
    DRAFTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|d| d.get(key).cloned())
        .unwrap_or_default()
}

/// Shows `messages` read for `thread`, with the replies to it that didn't go out.
fn set_chat_messages(s: &mut State, thread: &str, mut messages: Vec<ChatMessageRecord>) {
    messages.extend(s.failed_replies.iter().filter(|m| m.thread_id == thread).cloned());
    s.chat_messages = messages;
}

impl State {
    /// The SIM a text in the open conversation goes out on: the one picked
    /// here, else the one the conversation last used, else the phone's
    /// default, else the first. `None` with fewer than two SIMs.
    fn sim(&self) -> Option<String> {
        let sims = &self.sims.sims;
        if sims.len() < 2 {
            return None;
        }
        let listed = |id: &&str| sims.iter().any(|s| s.id == *id);
        let key = self.thread.as_deref().unwrap_or_default();
        self.chosen_sims
            .get(key)
            .map(String::as_str)
            .filter(listed)
            .or_else(|| {
                // Messages are newest first.
                self.thread.as_ref()?;
                self.messages.iter().find_map(|m| m.sim.as_deref().filter(listed))
            })
            .or(self.sims.default.as_deref().filter(listed))
            .or(sims.first().map(|s| s.id.as_str()))
            .map(str::to_owned)
    }
}

/// Whether reading a text conversation here marks it read on the phone too.
static MARK_READ_ON_PHONE: AtomicBool = AtomicBool::new(false);

/// Whether Messages is on screen: a conversation left open in a window that's
/// closed to the tray or minimized isn't being read.
static ON_SCREEN: AtomicBool = AtomicBool::new(false);

/// Messages came on screen or went off it; on, the open conversation counts as read.
pub fn set_on_screen(on: bool) {
    ON_SCREEN.store(on, Ordering::Relaxed);
    if on {
        mark_open_read();
    }
}

/// Conversations the phone was asked to mark read, as of their latest message:
/// asked once until something newer arrives, unless the ask failed for a reason
/// that may pass (the link was down).
static MARKED: Mutex<Option<HashSet<(DeviceId, String, i64)>>> = Mutex::new(None);

pub fn set_mark_read_on_phone(on: bool) {
    MARK_READ_ON_PHONE.store(on, Ordering::Relaxed);
    // Turned on: conversations open or unread now are worth asking about again.
    MARKED.lock().unwrap_or_else(|e| e.into_inner()).take();
    // The page says whether marking unread reaches the phone.
    if let Some(host) = core_host::try_host() {
        host.hub.changed(Changes::MESSAGES);
    }
    if on {
        mark_open_read();
    }
}

/// Asks the phone to mark the open text conversation read, if the setting is
/// on and it has texts the phone calls unread. The phone says no unless it
/// offers it (Elevated), which is fine: it's asked once per latest message.
fn mark_open_read() {
    if !MARK_READ_ON_PHONE.load(Ordering::Relaxed) || !ON_SCREEN.load(Ordering::Relaxed) {
        return;
    }
    let Some((device, thread, date)) = state(|s| {
        let thread = s.thread.clone().filter(|t| !t.starts_with("chat:"))?;
        let t = s.threads.iter().find(|t| t.id == thread && t.unread > 0)?;
        Some((s.device?, thread, t.date))
    }) else {
        return;
    };
    let asked = MARKED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_default().insert((
        device,
        thread.clone(),
        date,
    ));
    let Some(node) = core_host::node().filter(|_| asked) else {
        MARKED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_or_insert_default()
            .remove(&(device, thread, date));
        return;
    };
    tracing::info!(device = %device.short(), "asking the phone to mark a conversation read");
    core_host::spawn(async move {
        match node.sms_mark_read(device, thread.clone()).await {
            Ok(()) => {
                // Shown read here at once; the phone's own list follows.
                state(|s| {
                    if s.device == Some(device)
                        && let Some(t) = s.threads.iter_mut().find(|t| t.id == thread)
                    {
                        t.unread = 0;
                    }
                });
                changed();
            }
            Err(e) => {
                tracing::info!(device = %device.short(), error = %e, "the phone didn't mark a conversation read");
                // A phone that doesn't offer it won't later; any other failure (the link
                // was down, a timeout) is worth asking again at the next chance.
                if !matches!(e, nectarlink_core::Error::Unsupported) {
                    MARKED
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .get_or_insert_default()
                        .remove(&(device, thread, date));
                }
            }
        }
    });
}

/// How many conversations [`State::recent`] keeps.
const RECENT_CONVERSATIONS: usize = 16;

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
        s.recent.clear();
        s.seen.clear();
    });
    changed();
}

fn seen_file(device: DeviceId) -> PathBuf {
    // Named like the rest of this cache, so clearing it clears this too.
    files_dir().join(format!("sms-seen-{device}.json"))
}

fn load_seen(device: DeviceId) -> HashMap<String, i64> {
    std::fs::read(seen_file(device)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// What was read here on `device`, by conversation, to the date of the latest message.
pub(crate) fn seen_for(device: DeviceId) -> HashMap<String, i64> {
    state(|s| if s.device == Some(device) { Some(s.seen.clone()) } else { None })
        .unwrap_or_else(|| load_seen(device))
}

/// The conversation open on `device`, if it's the one shown.
pub(crate) fn open_thread_of(device: DeviceId) -> Option<String> {
    state(|s| if s.device == Some(device) { s.thread.clone() } else { None })
}

/// Remembers reading `thread`'s messages up to `date`.
fn note_seen(s: &mut State, thread: &str, date: i64) -> Option<(DeviceId, HashMap<String, i64>)> {
    if s.seen.get(thread).is_some_and(|seen| *seen >= date) {
        return None;
    }
    s.seen.insert(thread.to_owned(), date);
    Some((s.device?, s.seen.clone()))
}

fn save_seen(device: DeviceId, seen: &HashMap<String, i64>) {
    let Ok(bytes) = serde_json::to_vec(seen) else { return };
    let _ = std::fs::create_dir_all(files_dir());
    if let Err(e) = std::fs::write(seen_file(device), bytes) {
        tracing::debug!(error = %e, "can't save which texts were read");
    }
}

/// Puts `messages`, the conversation being left, among the recent ones.
fn remember_recent(s: &mut State, thread: String, messages: Vec<SmsMessage>) {
    s.recent.retain(|(t, _)| *t != thread);
    if !messages.is_empty() {
        s.recent.push((thread, messages));
    }
    if s.recent.len() > RECENT_CONVERSATIONS {
        s.recent.remove(0);
    }
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
    /// The phone deletes texts when asked (it offers `sms.delete`): deleting here can delete there too.
    pub phone_can_delete: bool,
    /// Marking a conversation unread here marks it on the phone too: the phone offers
    /// `sms.mark_unread` and the user turned on syncing the read state.
    pub unread_syncs: bool,
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
    /// The phone's SIMs to text from, when it has two or more:
    /// `[{ "id", "label" }]`.
    pub sims: Value,
    /// The one the open conversation sends on, or "".
    pub sim: String,
    /// A message to bring into view and highlight, or "".
    pub focus_message: String,
}

pub fn view() -> View {
    // Read before taking the Messages lock so the two locks never nest.
    let hidden = core_host::try_host().map(|host| host.hub.read(AppState::hidden_apps)).unwrap_or_default();
    let device = state(|s| s.device);
    let node = core_host::node();
    let offers = |cap: &str| device.zip(node.as_ref()).is_some_and(|(d, n)| n.peer_offers(d, cap));
    let phone_can_delete = offers(nectarlink_core::SMS_DELETE);
    let unread_syncs = offers(nectarlink_core::SMS_MARK_UNREAD);
    state(|s| {
        // A filter on an app that has since been hidden shows everything.
        let app_filter = if hidden.contains(&s.app_filter) { "" } else { s.app_filter.as_str() };
        // What was deleted or marked unread here (not on the phone).
        let local = s.device.map(messages_local::read).unwrap_or_default();
        let mut combined: Vec<(i64, String, Value)> = Vec::new();
        if app_filter.is_empty() || app_filter == "sms" {
            for t in &s.threads {
                if local.thread_hidden(&t.id, t.date) {
                    continue;
                }
                let mut json = thread_json(t, s.photos.get(&t.id));
                // Read here: nothing new to badge. The phone still calls it unread (it is, until
                // read there or the read state is synced), so it stays bold.
                if s.seen.get(&t.id).is_some_and(|seen| *seen >= t.date) {
                    json["fresh"] = json!(0);
                }
                mark_thread(&mut json, &local, &t.id, t.date);
                combined.push((t.date, t.id.clone(), json));
            }
        }
        if app_filter != "sms" {
            for ct in s.chat_threads.iter().filter(|ct| !hidden.contains(&ct.app)) {
                if (app_filter.is_empty() || ct.app == app_filter)
                    && !local.thread_hidden(&ct.thread_id, ct.updated_ms)
                {
                    let mut json =
                        chat_thread_json(ct, s.photos.get(&ct.thread_id), s.app_icons.get(&ct.app));
                    mark_thread(&mut json, &local, &ct.thread_id, ct.updated_ms);
                    combined.push((ct.updated_ms, ct.thread_id.clone(), json));
                }
            }
        }
        combined.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        let threads = Value::Array(combined.into_iter().map(|(_, _, v)| v).collect());

        let mut apps = vec![json!({ "key": "", "label": "All" }), json!({ "key": "sms", "label": "SMS" })];
        let mut seen = HashSet::new();
        for ct in s.chat_threads.iter().filter(|ct| !hidden.contains(&ct.app)) {
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
            Value::Array(
                ordered
                    .iter()
                    .filter(|m| !local.message_hidden(&m.thread_id, &chat_message_key(m), m.time_ms))
                    .map(|m| chat_message_json(m, s.failed_replies.iter().any(|f| f.id == m.id)))
                    .collect(),
            )
        } else {
            // In a group, who sent each message (by contact name when known).
            let thread = s.thread.as_ref().and_then(|id| s.threads.iter().find(|t| &t.id == id));
            let group = thread.filter(|t| t.addresses.len() > 1);
            let open = s.thread.as_deref().unwrap_or_default();
            Value::Array(
                s.messages
                    .iter()
                    .rev()
                    .filter(|m| !local.message_hidden(open, &m.id, m.date))
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
            app_filter: app_filter.to_owned(),
            phone_can_delete,
            unread_syncs,
            available_apps,
            has_chat_threads: s.chat_threads.iter().any(|ct| !hidden.contains(&ct.app)),
            current_is_chat,
            current_app_name,
            current_can_reply,
            current_is_group,
            has_attachment,
            attachment_preview,
            attachment_name,
            attachment_size,
            attachment_error,
            sims: if s.sims.sims.len() < 2 {
                json!([])
            } else {
                Value::Array(
                    s.sims.sims.iter().map(|sim| json!({ "id": sim.id, "label": sim.label })).collect(),
                )
            },
            sim: s.sim().unwrap_or_default(),
            focus_message: s.focus_message.clone().unwrap_or_default(),
        }
    })
}

/// Texts in the open conversation (or a new one) go out on `sim` from now on.
pub fn choose_sim(sim: String) {
    state(|s| {
        let key = s.thread.clone().unwrap_or_default();
        s.chosen_sims.insert(key, sim);
    });
    changed();
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

/// Shows what was done to a conversation here: marked unread counts as one
/// unread text, a deleted latest text leaves no snippet.
fn mark_thread(json: &mut Value, local: &messages_local::Local, id: &str, date: i64) {
    let marked = local.is_marked_unread(id);
    if marked {
        let unread = json["unread"].as_u64().unwrap_or(0).max(1);
        json["unread"] = json!(unread);
    }
    if local.snippet_hidden(id, date) {
        json["snippet"] = json!("");
    }
    json["markedUnread"] = json!(marked);
}

/// A chat message's key among those deleted here.
fn chat_message_key(m: &ChatMessageRecord) -> String {
    messages_local::chat_message_key(
        &m.thread_id,
        m.sender.as_deref().unwrap_or_default(),
        &m.text,
        m.time_ms,
        m.self_sent,
    )
}

fn thread_json(t: &SmsThread, photo: Option<&PathBuf>) -> Value {
    json!({
        "id": t.id,
        "title": title_of(t),
        "addresses": t.addresses.join(", "),
        "snippet": t.snippet,
        "date": t.date,
        "unread": t.unread,
        // The new ones, with a notification on the phone: what the badge counts. The rest of
        // the unread are only shown bold. A phone that doesn't tell them apart counts all.
        "fresh": t.fresh.map_or(t.unread, |f| f.min(t.unread)),
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
        "fresh": t.unread,
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
    let audios: Vec<Value> = m
        .parts
        .iter()
        .filter(|p| p.mime.starts_with("audio/"))
        .map(|p| {
            json!({
                "url": pictures.get(&p.id).map(|f| crate::icons::file_url(f)).unwrap_or_default(),
                "mime": p.mime,
                "size": format_attachment_size(p.size as usize),
            })
        })
        .collect();
    let others =
        m.parts.iter().filter(|p| !p.mime.starts_with("image/") && !p.mime.starts_with("audio/")).count();
    json!({
        "id": m.id,
        "body": m.body,
        "date": m.date,
        "outgoing": m.outgoing,
        "status": m.status.clone().unwrap_or_default(),
        "images": images,
        "audios": audios,
        "attachments": others,
        "sender": if m.outgoing { None } else { sender },
        "code": if m.outgoing { None } else { one_time_code(&m.body) },
    })
}

fn chat_message_json(m: &ChatMessageRecord, failed: bool) -> Value {
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
        "status": if failed { "failed" } else if m.id < 0 { "pending" } else { "" },
        "images": [],
        "audios": [],
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

/// The date of `thread`'s latest message: as listed, or newer when the open
/// conversation has one the list doesn't yet.
fn thread_date(s: &State, thread: &str) -> Option<i64> {
    if thread.starts_with("chat:") {
        return s.chat_threads.iter().find(|t| t.thread_id == thread).map(|t| t.updated_ms);
    }
    let listed = s.threads.iter().find(|t| t.id == thread).map(|t| t.date);
    let newest = (s.thread.as_deref() == Some(thread)).then(|| s.messages.first().map(|m| m.date)).flatten();
    listed.into_iter().chain(newest).max()
}

/// Leaves `ids`' conversation if it's open: deleted or marked unread, it's done with.
fn close_if_open(ids: &[String]) {
    if state(|s| s.thread.as_ref().is_some_and(|t| ids.contains(t))) {
        close_thread();
    }
}

/// What the page (and the badges) show changed with what was done to conversations.
fn managed(device: DeviceId) {
    changed();
    let threads = state(|s| s.threads.clone());
    crate::bridge::app::update_home_sms(device, &threads);
    crate::badges::publish();
}

/// Marks conversations unread, until they're opened, and on the phone too when it offers it.
/// Choosing it is the ask, so it doesn't wait for "Sync read state with the phone" (which is
/// only about marking read on its own).
pub fn mark_threads_unread(ids: Vec<String>) {
    let Some(device) = state(|s| s.device) else { return };
    messages_local::update(device, |l| {
        for id in &ids {
            l.mark_unread(id);
        }
    });
    close_if_open(&ids);
    managed(device);
    mark_unread_on_phone(device, ids.into_iter().map(|thread| (thread, None)).collect());
}

/// Marks messages of the open conversation unread (what's chosen of them that was received),
/// and leaves it: on the phone each of them, so its count follows; here the conversation is
/// marked unread until it's opened.
pub fn mark_messages_unread(ids: Vec<String>) {
    let Some((device, thread, received)) = state(|s| {
        let thread = s.thread.clone()?;
        let received: Vec<String> =
            s.messages.iter().filter(|m| !m.outgoing && ids.contains(&m.id)).map(|m| m.id.clone()).collect();
        Some((s.device?, thread, received))
    }) else {
        return;
    };
    messages_local::update(device, |l| l.mark_unread(&thread));
    close_if_open(std::slice::from_ref(&thread));
    managed(device);
    mark_unread_on_phone(device, received.into_iter().map(|id| (thread.clone(), Some(id))).collect());
}

/// Asks the phone to mark unread what `targets` name: a text conversation's latest received
/// text (no message) or one message of it. Not the chats read from notifications, which the
/// phone has no messages for. Done only when the phone offers it; they stay marked unread here
/// either way, and a phone that can't is only logged.
fn mark_unread_on_phone(device: DeviceId, targets: Vec<(String, Option<String>)>) {
    let Some(node) = core_host::node().filter(|n| n.peer_offers(device, nectarlink_core::SMS_MARK_UNREAD))
    else {
        tracing::info!(device = %device.short(), "the phone can't mark texts unread: only marked here");
        return;
    };
    let targets: Vec<(String, Option<String>)> =
        targets.into_iter().filter(|(t, _)| !t.starts_with("chat:")).collect();
    if targets.is_empty() {
        return;
    }
    // Opened again later, they're worth marking read on the phone again.
    MARKED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_default()
        .retain(|(d, t, _)| *d != device || !targets.iter().any(|(thread, _)| thread == t));
    core_host::spawn(async move {
        for (thread, message) in targets {
            match node.sms_mark_unread(device, thread.clone(), message.clone()).await {
                Ok(()) => {
                    tracing::info!(device = %device.short(), "the phone marked a text unread");
                    state(|s| {
                        if s.device == Some(device)
                            && let Some(t) = s.threads.iter_mut().find(|t| t.id == thread)
                        {
                            // A message each, or the conversation's latest.
                            t.unread = if message.is_some() { t.unread + 1 } else { t.unread.max(1) };
                        }
                    });
                    changed();
                }
                Err(e) => {
                    tracing::info!(device = %device.short(), error = %e, "the phone didn't mark a text unread");
                }
            }
        }
    });
}

/// Deletes conversations from this PC. A text conversation stays on the phone
/// and shows again, with only the newer texts, when one arrives, unless `on_phone` (the user
/// chose it, each time) also has the phone delete it; a chat read from notifications is
/// removed from the PC's own store.
pub fn delete_threads(ids: Vec<String>, on_phone: bool) {
    let Some((device, dates)) = state(|s| {
        let dates: Vec<(String, i64)> =
            ids.iter().filter_map(|id| Some((id.clone(), thread_date(s, id)?))).collect();
        Some((s.device?, dates))
    }) else {
        return;
    };
    if let Some(node) = core_host::node() {
        for (id, _) in dates.iter().filter(|(id, _)| id.starts_with("chat:")) {
            node.delete_chat_thread(device, id);
        }
    }
    messages_local::update(device, |l| {
        for (id, date) in &dates {
            l.delete_thread(id, *date);
        }
    });
    close_if_open(&ids);
    state(|s| {
        s.chat_threads.retain(|t| !ids.contains(&t.thread_id));
        s.recent.retain(|(t, _)| !ids.contains(t));
    });
    managed(device);
    if on_phone {
        delete_on_phone(
            device,
            ids.into_iter().filter(|t| !t.starts_with("chat:")).map(|t| (t, None)).collect(),
        );
    }
}

/// Why the phone didn't delete, in words for the user. What was deleted here stays deleted.
pub fn delete_problem(error: &Error) -> String {
    match error {
        Error::Offline | Error::NotPaired | Error::Timeout | Error::Network(_) => tr!(
            "Messages",
            "Couldn't delete on the phone: it isn't connected. It was removed from this PC only."
        ),
        Error::Denied => tr!(
            "Messages",
            "Couldn't delete on the phone: messages are turned off for this PC there. It was removed from this PC only."
        ),
        Error::Unsupported => tr!(
            "Messages",
            "Couldn't delete on the phone: it doesn't delete texts for this PC. It was removed from this PC only."
        ),
        _ => tr!(
            "Messages",
            "Couldn't delete on the phone: its messages refused the change (some phones don't allow it). It was removed from this PC only."
        ),
    }
}

/// Asks the phone to delete conversations (`None`) or only those messages of them, the user
/// having chosen to. Not undoable, and never on its own: a list of messages that is empty
/// deletes nothing, not the conversation. A phone that couldn't says so, in words; the PC's
/// side was already done.
fn delete_on_phone(device: DeviceId, requests: Vec<(String, Option<Vec<String>>)>) {
    let requests: Vec<_> = requests
        .into_iter()
        .filter(|(_, messages)| messages.as_ref().is_none_or(|m| !m.is_empty()))
        .collect();
    if requests.is_empty() {
        return;
    }
    let Some(node) = core_host::node() else {
        crate::bridge::app::show_message(delete_problem(&Error::Offline));
        return;
    };
    core_host::spawn(async move {
        let mut failed = None;
        for (thread, messages) in requests {
            let batches: Vec<Vec<String>> = match messages {
                None => vec![Vec::new()],
                Some(messages) => {
                    messages.chunks(nectarlink_core::SMS_MAX_DELETE).map(<[String]>::to_vec).collect()
                }
            };
            for batch in batches {
                if let Err(e) = node.sms_delete(device, thread.clone(), batch).await {
                    tracing::info!(device = %device.short(), error = %e, "the phone didn't delete");
                    failed.get_or_insert(e);
                }
            }
        }
        match failed {
            Some(e) => crate::bridge::app::show_message(delete_problem(&e)),
            // The phone's list lost them.
            None => {
                if state(|s| s.device == Some(device)) {
                    load_threads();
                }
            }
        }
    });
}

/// Deletes messages of the open conversation from this PC, by the IDs the page
/// shows. Texts stay on the phone and stay hidden here, unless `on_phone` (the user chose
/// it, each time) also has the phone delete them; chat messages are removed from the PC's
/// own store as well.
pub fn delete_messages(ids: Vec<String>, on_phone: bool) {
    let Some((device, thread, newest, keys, rows, date)) = state(|s| {
        let thread = s.thread.clone()?;
        let date = thread_date(s, &thread)?;
        let local = messages_local::read(s.device?);
        if thread.starts_with("chat:") {
            let wanted: Vec<&ChatMessageRecord> =
                s.chat_messages.iter().filter(|m| ids.contains(&m.id.to_string())).collect();
            let newest = s
                .chat_messages
                .iter()
                .filter(|m| !local.message_hidden(&thread, &chat_message_key(m), m.time_ms))
                .max_by_key(|m| (m.time_ms, m.id))
                .is_some_and(|m| wanted.iter().any(|w| w.id == m.id));
            let rows: Vec<i64> = wanted.iter().map(|m| m.id).collect();
            Some((
                s.device?,
                thread,
                newest,
                wanted.iter().map(|m| chat_message_key(m)).collect(),
                rows,
                date,
            ))
        } else {
            let newest = s
                .messages
                .iter()
                .find(|m| !local.message_hidden(&thread, &m.id, m.date))
                .is_some_and(|m| ids.contains(&m.id));
            Some((s.device?, thread, newest, ids.clone(), Vec::new(), date))
        }
    }) else {
        return;
    };
    let is_chat = thread.starts_with("chat:");
    messages_local::update(device, |l| {
        l.delete_messages(&thread, keys.iter().map(String::as_str), newest.then_some(date));
    });
    if is_chat {
        if let Some(node) = core_host::node() {
            for id in rows.iter().filter(|id| **id > 0) {
                node.delete_chat_message(device, &thread, *id);
            }
        }
        state(|s| {
            s.chat_messages.retain(|m| !rows.contains(&m.id));
            s.failed_replies.retain(|m| !rows.contains(&m.id));
        });
    }
    managed(device);
    if on_phone && !is_chat {
        // Only what the phone listed: a text still being sent has no number there yet.
        let phone_ids: Vec<String> =
            keys.into_iter().filter(|id| id.starts_with("sms:") || id.starts_with("mms:")).collect();
        delete_on_phone(device, vec![(thread, Some(phone_ids))]);
    }
}

/// Shows `device`'s messages (again: reloads).
pub fn open_device(device: DeviceId) {
    let same = state(|s| {
        let same = s.device == Some(device);
        if !same {
            *s = State {
                device: Some(device),
                generation: s.generation + 1,
                seen: load_seen(device),
                ..State::default()
            };
        }
        same
    });
    if !same {
        changed();
    }
    load_threads();
}

/// `device`'s chat conversations from the store, newest first, minus what was
/// deleted here. A deleted one that comes back (the app repeats its recent
/// messages in the next notification) counts as unread only what is newer.
pub(crate) fn chat_threads_of(node: &nectarlink_core::Node, device: DeviceId) -> Vec<ChatThreadRecord> {
    let mut threads = node.chat_threads(device);
    let local = messages_local::read(device);
    if threads.iter().any(|t| local.cleared_at(&t.thread_id).is_some()) {
        for t in &mut threads {
            if let Some(at) = local.cleared_at(&t.thread_id).filter(|at| t.updated_ms > *at && t.unread > 0) {
                let newer = node
                    .chat_messages(device, &t.thread_id, PAGE)
                    .iter()
                    .filter(|m| !m.self_sent && m.time_ms > at)
                    .count();
                t.unread = t.unread.min(u32::try_from(newer).unwrap_or(u32::MAX));
            }
        }
    }
    // A newer message ends what deleting the latest one blanked.
    messages_local::update(device, |l| {
        l.reconcile(threads.iter().map(|t| (t.thread_id.as_str(), t.updated_ms)))
    });
    threads
}

/// Synchronously refreshes persisted `MessagingStyle` chat threads from SQLite.
pub fn load_chat_threads() {
    let Some((device, generation)) = state(|s| s.device.map(|d| (d, s.generation))) else { return };
    let Some(node) = core_host::node() else { return };
    let chat_threads = chat_threads_of(&node, device);
    let (photos, app_icons) = save_chat_assets(device, &chat_threads);
    // Another phone was picked meanwhile: what was read here is for the old one.
    let open_chat = state(|s| {
        if s.generation != generation {
            return Err(());
        }
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
        Ok(s.thread.clone().filter(|t| t.starts_with("chat:")))
    });
    let Ok(open_chat) = open_chat else { return };
    if let Some(thread) = open_chat {
        let _ = node.mark_chat_thread_read(device, &thread);
        let msgs = node.chat_messages(device, &thread, PAGE);
        let refreshed = chat_threads_of(&node, device);
        state(|s| {
            if s.generation != generation {
                return;
            }
            s.chat_threads = refreshed;
            if s.thread.as_deref() == Some(&thread) {
                set_chat_messages(s, &thread, msgs);
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
        // Which SIMs it texts from, asked once per phone shown.
        let sims = if result.is_ok() && !state(|s| s.sims_loaded) {
            match node.sms_sims(device).await {
                Ok(sims) => Some(sims),
                Err(e) => {
                    tracing::debug!(error = %e, "can't list the phone's SIMs for texts");
                    None
                }
            }
        } else {
            None
        };
        state(|s| {
            if s.generation != generation {
                return;
            }
            if let Some(sims) = sims {
                s.sims = sims;
                s.sims_loaded = true;
            }
            match result {
                Ok(threads) => {
                    // A newer message ends what deleting the latest one blanked.
                    messages_local::update(device, |l| {
                        l.reconcile(threads.iter().map(|t| (t.id.as_str(), t.date)));
                    });
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
        mark_open_read();
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
            let already_valid =
                path.is_file() && std::fs::metadata(&path).is_ok_and(|m| m.len() == avatar.len() as u64);
            if already_valid || std::fs::write(&path, avatar).is_ok() {
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
            let already_valid =
                path.is_file() && std::fs::metadata(&path).is_ok_and(|m| m.len() == icon.len() as u64);
            if already_valid || std::fs::write(&path, icon).is_ok() {
                app_icons.insert(t.app.clone(), path);
            }
        }
    }
    (photos, app_icons)
}

/// Opens a conversation: its latest messages.
pub fn open_thread(thread: String) {
    let is_chat = thread.starts_with("chat:");
    let (device, seen) = state(|s| {
        s.compose_to = None;
        s.compose_name = None;
        if s.thread.as_deref() != Some(&thread) {
            let left = s.thread.replace(thread.clone());
            let messages = std::mem::take(&mut s.messages);
            if let Some(left) = left.filter(|t| !t.starts_with("chat:")) {
                remember_recent(s, left, messages);
            }
            if let Some(at) = s.recent.iter().position(|(t, _)| *t == thread) {
                s.messages = s.recent.remove(at).1;
            }
            s.chat_messages.clear();
            s.more = false;
            s.staged_attachment = None;
            s.focus_message = None;
        }
        let date = s.threads.iter().find(|t| t.id == thread).map(|t| t.date);
        (s.device, date.and_then(|date| note_seen(s, &thread, date)))
    });
    if let Some((device, seen)) = seen {
        std::thread::spawn(move || save_seen(device, &seen));
    }
    // Opened: no longer marked unread.
    if let Some(device) = device {
        messages_local::update(device, |l| l.clear_unread(&thread));
    }
    if is_chat {
        if let (Some(device), Some(node)) = (device, core_host::node()) {
            let _ = node.mark_chat_thread_read(device, &thread);
            let msgs = node.chat_messages(device, &thread, PAGE);
            let refreshed = chat_threads_of(&node, device);
            state(|s| {
                s.chat_threads = refreshed;
                if s.thread.as_deref() == Some(&thread) {
                    set_chat_messages(s, &thread, msgs);
                }
            });
        }
        changed();
    } else {
        changed();
        load_messages(None);
        mark_open_read();
    }
    crate::badges::publish();
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
    let local = messages_local::read(device);
    let should_load_sms = state(|s| {
        if s.device == Some(device) {
            for t in s.threads.iter().filter(|t| !local.thread_hidden(&t.id, t.date)) {
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
        for ct in chat_threads_of(&node, device)
            .into_iter()
            .filter(|t| !local.thread_hidden(&t.thread_id, t.updated_ms))
        {
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
            // Only the older page's own load ends it: a refresh can finish first.
            if before.is_some() {
                s.loading_older = false;
            }
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
                        // Texts sent from here that the phone doesn't list
                        // yet stay, marked: a failed one is on no phone's
                        // list, and some phones store a sent one late or never.
                        let failed = unlisted_local(&s.messages, &page);
                        let older: Vec<SmsMessage> = s
                            .messages
                            .drain(..)
                            .filter(|m| m.date < oldest && !m.id.starts_with("local:"))
                            .collect();
                        let had_older = !older.is_empty();
                        s.messages = page;
                        s.messages.extend(older);
                        if !failed.is_empty() {
                            s.messages.extend(failed);
                            s.messages.sort_by_key(|m| std::cmp::Reverse(m.date));
                        }
                        s.more = full || had_older && s.more;
                        let mut asked = NOTIFICATION.lock().unwrap_or_else(|e| e.into_inner());
                        if let Some(n) =
                            asked.as_ref().filter(|n| n.thread.as_deref() == Some(thread.as_str()))
                        {
                            s.focus_message = message_for_notification(&s.messages, &n.text, n.when);
                            if s.focus_message.is_some() {
                                asked.take();
                            }
                        }
                    } else {
                        s.messages.extend(page);
                        s.more = full;
                    }
                    s.messages
                        .iter()
                        .flat_map(|m| m.parts.iter())
                        .filter(|p| {
                            (p.mime.starts_with("image/") || p.mime.starts_with("audio/"))
                                && !s.pictures.contains_key(&p.id)
                        })
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
        } else if mime.contains("webp") {
            "webp"
        } else if mime.contains("mp4") || mime.contains("m4a") || mime.contains("aac") {
            "m4a"
        } else if mime.contains("mpeg") || mime.contains("mp3") {
            "mp3"
        } else if mime.contains("ogg") {
            "ogg"
        } else if mime.contains("wav") {
            "wav"
        } else if mime.contains("amr") {
            "amr"
        } else if mime.contains("3gp") {
            "3gp"
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

// ---- Pictures in a conversation ----

/// The cached file for a picture's URL, when it is one of this page's own.
fn cached_picture(url: &str) -> Result<PathBuf, String> {
    let path = url_to_path(url);
    let own = path.file_name().and_then(|n| n.to_str()).is_some_and(is_message_cache_file);
    let same = |a: &Path, b: &Path| match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    };
    if own && path.parent().is_some_and(|dir| same(dir, &files_dir())) && path.is_file() {
        Ok(path)
    } else {
        Err("not a picture of a conversation".to_owned())
    }
}

/// The picture's MIME type, from what the file starts with.
fn picture_mime(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") { "image/png" } else { "image/jpeg" }
}

/// Where the Save dialog for a picture starts: a file URL for "Picture" in
/// the Pictures folder (else Downloads), with the picture's own extension.
pub fn picture_save_suggestion(url: &str) -> String {
    let ext = Path::new(&url_to_path(url))
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| !e.is_empty())
        .unwrap_or("jpg")
        .to_owned();
    let dir = dirs::picture_dir().or_else(dirs::download_dir).unwrap_or_default();
    crate::icons::file_url(&dir.join(format!("Picture.{ext}")))
}

/// Puts a picture on the clipboard.
pub fn copy_picture(url: &str) -> Result<(), String> {
    let bytes = std::fs::read(cached_picture(url)?).map_err(|e| e.to_string())?;
    crate::win::clipboard::write_image(picture_mime(&bytes), &bytes)
}

/// Saves a picture where the user chose (a file URL or a path).
pub fn save_picture(url: &str, to: &str) -> Result<(), String> {
    let from = cached_picture(url)?;
    let to = url_to_path(to);
    if to.as_os_str().is_empty() || to.is_dir() {
        return Err("no file chosen".to_owned());
    }
    std::fs::copy(from, to).map(|_| ()).map_err(|e| e.to_string())
}

/// Opens the system's share sheet for a picture, where there is one (Windows
/// and macOS). Has to run on Qt's thread.
pub fn share_picture(url: &str) -> Result<(), String> {
    let from = cached_picture(url)?;
    let ext = from.extension().and_then(|e| e.to_str()).unwrap_or("jpg");
    let staged = crate::photos::stage_for_share(
        &crate::photos::share_dir(),
        vec![(from.clone(), format!("Picture.{ext}"))],
        true,
    );
    if staged.is_empty() {
        return Err("can't get the picture ready to share".to_owned());
    }
    crate::win::share::show(&staged)
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
    let Some(rest) = trimmed.strip_prefix("file://") else {
        return PathBuf::from(trimmed);
    };
    // `file:///home/a.png` is `/home/a.png`, and `file:///C:/a.png` is
    // `C:/a.png`; `file://server/share/a.png` is a network path.
    let stripped: std::borrow::Cow<'_, str> = match rest.strip_prefix('/') {
        Some(local) if is_drive_path(local) => local.into(),
        Some(_) => rest.into(),
        None => format!("//{rest}").into(),
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

/// Whether `path` starts with a Windows drive, as in `C:/` (a file URL's path
/// after its first `/`). Never on Linux or macOS, where `/C:/x` is a real path.
fn is_drive_path(path: &str) -> bool {
    let b = path.as_bytes();
    cfg!(windows) && b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

fn detect_or_convert_attachment(path: &Path, raw: Vec<u8>) -> Result<(String, Vec<u8>), String> {
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
        "m4a" | "mp4" => return Ok(("audio/mp4".into(), raw)),
        "aac" => return Ok(("audio/aac".into(), raw)),
        "mp3" => return Ok(("audio/mpeg".into(), raw)),
        "ogg" | "oga" => return Ok(("audio/ogg".into(), raw)),
        "wav" => return Ok(("audio/wav".into(), raw)),
        "amr" => return Ok(("audio/amr".into(), raw)),
        "3gp" => return Ok(("audio/3gpp".into(), raw)),
        _ => {}
    }
    // Convert BMP/TIFF/ICO or other Windows-decodable images to PNG.
    let bitmap = crate::win::image::decode(&raw)
        .map_err(|_| tr!("Messages", "Only image or voice-note audio files can be attached."))?;
    let png = crate::win::image::encode_png(&bitmap).map_err(|e| e.to_string())?;
    Ok(("image/png".into(), png))
}

fn stage_attachment_bytes(name: String, mime: String, data: Vec<u8>) -> bool {
    let size_text = format_attachment_size(data.len());
    let is_audio = mime.starts_with("audio/");
    let ext = if mime.contains("png") {
        "png"
    } else if mime.contains("gif") {
        "gif"
    } else if mime.contains("webp") {
        "webp"
    } else if mime.contains("mp4") || mime.contains("m4a") || mime.contains("aac") {
        "m4a"
    } else if mime.contains("mpeg") || mime.contains("mp3") {
        "mp3"
    } else if mime.contains("ogg") {
        "ogg"
    } else if mime.contains("wav") {
        "wav"
    } else if mime.contains("amr") {
        "amr"
    } else if mime.contains("3gp") {
        "3gp"
    } else {
        "jpg"
    };
    let dir = files_dir();
    let preview_path = std::fs::create_dir_all(&dir).ok().and_then(|()| {
        let p = dir.join(format!("mms-stage-{:016x}.{ext}", crate::photos::fingerprint_bytes(&data)));
        (p.exists() || std::fs::write(&p, &data).is_ok()).then_some(p)
    });
    let preview_url = if is_audio {
        String::new()
    } else {
        preview_path.as_ref().map(|p| crate::icons::file_url(p)).unwrap_or_default()
    };
    let (data, error) = if data.len() > MAX_ATTACHMENT_BYTES {
        (Vec::new(), tr!("Messages", "Attachment is %1 (MMS supports up to 900 KB)", size_text))
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

/// Stages an image or audio file from disk (via drag-and-drop or file picker) as an MMS attachment.
pub fn attach_file(path_or_url: String) -> bool {
    let path = url_to_path(&path_or_url);
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("attachment").to_owned();
    // Images are made smaller to fit, but no picture needs more than this:
    // a video, say, isn't read into memory to find out.
    const LARGEST_SOURCE: u64 = 50 * 1024 * 1024;
    let size = std::fs::metadata(&path).map_or(0, |m| m.len());
    let converted = if size > LARGEST_SOURCE {
        Err(tr!(
            "Messages",
            "Attachment is %1 (MMS supports up to 900 KB)",
            format_attachment_size(usize::try_from(size).unwrap_or(usize::MAX))
        ))
    } else {
        match std::fs::read(&path) {
            Ok(raw) if !raw.is_empty() => detect_or_convert_attachment(&path, raw),
            _ => return false,
        }
    };
    match converted {
        Ok((mime, data)) => stage_attachment_bytes(name, mime, data),
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
    stage_attachment_bytes("Clipboard image.png".into(), "image/png".into(), png)
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
            "The phone doesn't send texts for this PC. Allow it in the NectarLink app on the phone."
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
        _ => "Couldn't send that reply. The notification may have been dismissed on your phone.",
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
        Some(_) => {
            crate::bridge::app::show_message(tr!("Messages", "Group texts can't be sent from the PC yet."))
        }
        // The box is already cleared: say so rather than lose it quietly.
        None => crate::bridge::app::show_message(tr!(
            "Messages",
            "The text wasn't sent: this conversation isn't loaded yet. Try again in a moment."
        )),
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
        let refreshed_threads = chat_threads_of(&node, device);
        let refreshed_messages = node.chat_messages(device, &thread_id, PAGE);
        state(|s| {
            s.sending = false;
            // Only for the phone still shown; and a reply that failed stays,
            // marked "Not sent", rather than vanish or say "Sending…" forever.
            if s.device == Some(device) {
                s.chat_threads = refreshed_threads;
                if result.is_err()
                    && let Some(reply) = s.chat_messages.iter().find(|m| m.id == local_id)
                {
                    s.failed_replies.push(reply.clone());
                }
                if result.is_ok() && s.thread.as_deref() == Some(&thread_id) {
                    set_chat_messages(s, &thread_id, refreshed_messages);
                }
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
    let Some((device, attachments, unsent, sim)) = state(|s| {
        let valid_att =
            s.staged_attachment.as_ref().filter(|a| a.error.is_empty() && !a.data.is_empty()).cloned();
        if body.trim().is_empty() && valid_att.is_none() {
            return None;
        }
        // A new conversation has no message to mark "Not sent": if it fails,
        // the recipient, text and picture come back to the composer instead.
        let unsent = s.thread.is_none().then(|| Unsent {
            generation: s.generation,
            to: to.join(", "),
            name: s.compose_name.clone(),
            body: body.clone(),
            attachment: s.staged_attachment.clone(),
        });
        let sim = s.sim();
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
                    sim: sim.clone(),
                },
            );
        }
        Some((s.device?, attachments, unsent, sim))
    }) else {
        return;
    };
    changed();
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        let result = node.send_sms_on(device, to.clone(), body, attachments, sim).await;
        state(|s| {
            s.sending = false;
            if result.is_err() {
                // Not sent: say so on it.
                if let Some(m) = s.messages.iter_mut().find(|m| m.id == local_id) {
                    m.status = Some("failed".into());
                }
                if let Some(unsent) = unsent.filter(|u| u.generation == s.generation && s.thread.is_none()) {
                    save_draft(format!("{device}|to:{}", unsent.to), unsent.body);
                    if s.staged_attachment.is_none() {
                        s.staged_attachment = unsent.attachment;
                    }
                    s.compose_name = unsent.name;
                    s.compose_to = Some(unsent.to);
                }
            }
        });
        changed();
        match result {
            Ok(()) => {
                state(|s| {
                    if let Some(m) = s.messages.iter_mut().find(|m| m.id == local_id) {
                        m.status = Some("sent".into());
                    }
                });
                changed();
                // The phone also says so when it has stored it; this is sooner.
                tokio::time::sleep(std::time::Duration::from_millis(600)).await;
                reload_after_send(&to);
            }
            Err(e) => crate::bridge::app::show_message(send_problem(&e)),
        }
    });
}

/// How far apart a text sent from here and the phone's stored copy may be dated.
const SENT_MATCH_MS: i64 = 10 * 60 * 1000;

/// The texts sent from here (`local:` ones) that `page`, the phone's latest
/// list, doesn't have: failed ones always, and sent or pending ones until an
/// outgoing text with the same body shows up near the same time.
fn unlisted_local(messages: &[SmsMessage], page: &[SmsMessage]) -> Vec<SmsMessage> {
    messages
        .iter()
        .filter(|m| m.id.starts_with("local:"))
        .filter(|m| {
            m.status.as_deref() == Some("failed")
                || !page.iter().any(|p| {
                    p.outgoing && p.body.trim() == m.body.trim() && (p.date - m.date).abs() <= SENT_MATCH_MS
                })
        })
        .cloned()
        .collect()
}

/// A new conversation's first message while it's being sent.
struct Unsent {
    generation: u64,
    to: String,
    name: Option<String>,
    body: String,
    attachment: Option<StagedAttachment>,
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

/// Package names of the phone's SMS apps: their notifications open the
/// conversation here when clicked.
const SMS_APPS: &[&str] = &[
    "com.google.android.apps.messaging",
    "com.samsung.android.messaging",
    "com.android.messaging",
    "com.android.mms",
    "com.oneplus.mms",
    "com.huawei.message",
    "com.sonyericsson.conversations",
];

/// What a text's notification says, to find its conversation and message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextNotification {
    pub app: String,
    pub title: String,
    pub text: String,
    /// Unix milliseconds.
    pub when: i64,
    /// The conversation's title, when it came as a chat (RCS).
    pub conversation: Option<String>,
}

/// A text notification being opened, until its message is in view.
struct OpenFor {
    title: String,
    text: String,
    when: i64,
    /// Its conversation, once listed.
    thread: Option<String>,
}

static NOTIFICATION: Mutex<Option<OpenFor>> = Mutex::new(None);

/// Opens the conversation a text's notification is from, with its message in
/// view. False when the notification isn't from an SMS app, or when Messages
/// can't show this phone's texts (permission off, or SMS not shared).
pub fn open_from_notification(device: DeviceId, n: TextNotification) -> bool {
    if !SMS_APPS.contains(&n.app.as_str()) || !messages_available(device) {
        return false;
    }
    open_device(device);
    match n.conversation.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        // Texts that came as a chat (RCS) have one from their first notification.
        Some(title) => open_thread(nectarlink_core::chat_thread_id(&n.app, title)),
        None => {
            *NOTIFICATION.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(OpenFor { title: n.title, text: n.text, when: n.when, thread: None });
            open_notification_thread();
        }
    }
    crate::bridge::app::show_messages(device);
    true
}

/// Whether Messages can show this phone's texts.
fn messages_available(device: DeviceId) -> bool {
    core_host::host().hub.read(|s| {
        s.matrices.get(&device).and_then(|m| m.state("messages.sms"))
            == Some(nectarlink_core::FeatureState::Available)
    })
}

/// Opens the waiting notification's conversation, if the phone's list has it
/// yet; the list is read again when a text arrives, so a new one is there soon.
fn open_notification_thread() {
    let Some((title, text, when)) = NOTIFICATION
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .filter(|n| n.thread.is_none())
        .map(|n| (n.title.clone(), n.text.clone(), n.when))
    else {
        return;
    };
    let Some(thread) = state(|s| thread_for_notification(&s.threads, &title, &text, when)) else { return };
    if let Some(n) = NOTIFICATION.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        n.thread = Some(thread.clone());
    }
    open_thread(thread);
}

/// The conversation a notification is from: the one with this title (a
/// contact's name, or the number the text came from), the one whose latest
/// text is the notification's when several are, then the most recent.
fn thread_for_notification(threads: &[SmsThread], title: &str, text: &str, when: i64) -> Option<String> {
    let title = title.trim().to_lowercase();
    let number = digits(&title);
    let text = text.trim();
    threads
        .iter()
        .filter(|t| {
            title_of(t).trim().to_lowercase() == title
                || (!number.is_empty()
                    && title.chars().all(|c| c.is_ascii_digit() || " +-().".contains(c))
                    && t.addresses.iter().any(|a| digits(a) == number))
        })
        .min_by_key(|t| (t.snippet.trim() != text, (t.date - when).abs()))
        .map(|t| t.id.clone())
}

/// The message a notification is about, in `messages` (newest first): the
/// received one with its text, closest to when it was posted. A text long
/// enough for the notification to cut it short still matches by its start.
fn message_for_notification(messages: &[SmsMessage], text: &str, when: i64) -> Option<String> {
    let text = text.trim().trim_end_matches('…').trim_end();
    if text.is_empty() {
        return None;
    }
    messages
        .iter()
        .filter(|m| !m.outgoing && m.body.trim().starts_with(text))
        .min_by_key(|m| (m.date - when).abs())
        .map(|m| m.id.clone())
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
    NOTIFICATION.lock().unwrap_or_else(|e| e.into_inner()).take();
    // What came in while it was open was read there.
    let seen = state(|s| {
        let thread = s.thread.clone()?;
        let date = s.threads.iter().find(|t| t.id == thread)?.date;
        note_seen(s, &thread, date)
    });
    if let Some((device, seen)) = seen {
        std::thread::spawn(move || save_seen(device, &seen));
    }
    state(|s| {
        s.thread = None;
        s.focus_message = None;
        s.compose_to = None;
        s.compose_name = None;
        s.messages.clear();
        s.chat_messages.clear();
        s.more = false;
        s.staged_attachment = None;
    });
    changed();
    crate::badges::publish();
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
        NodeEvent::DeviceRemoved(d) => {
            messages_local::forget(*d);
            // The files don't say which phone they came from, so all of them go
            // and another open phone fetches its own again.
            clear_cache();
            if Some(*d) == device {
                state(|s| *s = State { generation: s.generation.wrapping_add(1), ..State::default() });
                changed();
            } else if device.is_some() {
                load_threads();
                if state(|s| s.thread.is_some()) {
                    load_messages(None);
                }
            }
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
    open_notification_thread();
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
    fn text(id: &str, body: &str, date: i64, outgoing: bool, status: Option<&str>) -> super::SmsMessage {
        super::SmsMessage {
            id: id.into(),
            thread: "1".into(),
            address: "+100".into(),
            body: body.into(),
            date,
            outgoing,
            status: status.map(Into::into),
            parts: Vec::new(),
            sim: None,
        }
    }

    #[test]
    fn a_text_sent_from_here_stays_until_the_phone_lists_it() {
        let shown = vec![
            text("local:1000:1", "on my way", 1_000, true, Some("sent")),
            text("local:2000:2", "not this one", 2_000, true, Some("failed")),
            text("7", "hi", 500, false, None),
        ];
        // The phone hasn't stored it yet (or never will): both stay.
        let kept = super::unlisted_local(&shown, &[text("7", "hi", 500, false, None)]);
        assert_eq!(kept.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["local:1000:1", "local:2000:2"]);
        // Its stored copy arrived, dated a little later: only the failed one stays.
        let page =
            vec![text("8", " on my way", 4_000, true, Some("sent")), text("7", "hi", 500, false, None)];
        let kept = super::unlisted_local(&shown, &page);
        assert_eq!(kept.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["local:2000:2"]);
        // An incoming text with the same words, or one sent long before, isn't its copy.
        let page = vec![
            text("9", "on my way", 1_500, false, None),
            text("10", "on my way", 1_000 - super::SENT_MATCH_MS - 1, true, Some("sent")),
        ];
        assert_eq!(super::unlisted_local(&shown, &page).len(), 2);
    }

    fn sms_thread(id: &str, name: &str, number: &str, snippet: &str, date: i64) -> super::SmsThread {
        super::SmsThread {
            id: id.into(),
            addresses: vec![number.into()],
            names: vec![name.into()],
            snippet: snippet.into(),
            date,
            unread: 0,
            fresh: None,
            photo: None,
        }
    }

    fn sms_text(id: &str, body: &str, date: i64, outgoing: bool) -> super::SmsMessage {
        super::SmsMessage {
            id: id.into(),
            thread: "t".into(),
            address: "a".into(),
            body: body.into(),
            date,
            outgoing,
            status: None,
            parts: Vec::new(),
            sim: None,
        }
    }

    #[test]
    fn a_text_notification_finds_its_conversation_by_name_or_number() {
        let threads = vec![
            sms_thread("1", "Alice", "+1 555-0100", "see you", 100),
            sms_thread("2", "", "+44 20 7946 0958", "hello", 200),
            sms_thread("3", "Alice", "+1 555-0199", "on my way", 300),
        ];
        // By the contact's name, preferring the one whose latest text it is.
        assert_eq!(super::thread_for_notification(&threads, "alice", "see you", 400).as_deref(), Some("1"));
        // Otherwise the most recent.
        assert_eq!(super::thread_for_notification(&threads, "Alice", "other", 400).as_deref(), Some("3"));
        // A number, written differently than the phone's.
        assert_eq!(
            super::thread_for_notification(&threads, "020 7946 0958", "hello", 200).as_deref(),
            Some("2")
        );
        assert_eq!(super::thread_for_notification(&threads, "Bob", "hi", 400), None);
        // A title that isn't a number never matches by digits.
        assert_eq!(super::thread_for_notification(&threads, "Order 7946 0958", "hi", 400), None);
    }

    #[test]
    fn a_text_notification_finds_its_message() {
        // Newest first.
        let messages = vec![
            sms_text("3", "ok", 300, true),
            sms_text("2", "On my way, back in ten minutes", 250, false),
            sms_text("1", "On my way, back in ten minutes", 100, false),
        ];
        assert_eq!(
            super::message_for_notification(&messages, "On my way, back in ten minutes", 260).as_deref(),
            Some("2")
        );
        assert_eq!(
            super::message_for_notification(&messages, "On my way, back in ten minutes", 90).as_deref(),
            Some("1")
        );
        // The notification cut a long text short.
        assert_eq!(
            super::message_for_notification(&messages, "On my way, back in…", 255).as_deref(),
            Some("2")
        );
        // What was sent from the phone isn't a notification's.
        assert_eq!(super::message_for_notification(&messages, "ok", 300), None);
        assert_eq!(super::message_for_notification(&messages, "", 300), None);
    }

    #[test]
    fn a_text_goes_out_on_the_sim_picked_then_the_conversations_then_the_default() {
        let sim = |id: &str| nectarlink_core::CallSim { id: id.into(), label: format!("SIM {id}") };
        let message = |sim: Option<&str>| super::SmsMessage {
            id: "m".into(),
            thread: "t".into(),
            address: "a".into(),
            body: "b".into(),
            date: 1,
            outgoing: false,
            status: None,
            parts: Vec::new(),
            sim: sim.map(Into::into),
        };
        // One SIM, or none listed: nothing to pick.
        let mut s = super::State {
            sims: super::CallSims { sims: vec![sim("1")], default: Some("1".into()) },
            ..Default::default()
        };
        assert_eq!(s.sim(), None);

        s.sims = super::CallSims { sims: vec![sim("1"), sim("2")], default: None };
        assert_eq!(s.sim().as_deref(), Some("1"), "the first, with no default");
        s.sims.default = Some("2".into());
        assert_eq!(s.sim().as_deref(), Some("2"), "the phone's default");

        // In a conversation, the SIM its newest message used, skipping ones that don't say.
        s.thread = Some("t".into());
        s.messages = vec![message(None), message(Some("1")), message(Some("2"))];
        assert_eq!(s.sim().as_deref(), Some("1"));
        // A SIM that's gone since doesn't count.
        s.messages = vec![message(Some("9"))];
        assert_eq!(s.sim().as_deref(), Some("2"));

        // Picked here wins, for that conversation only.
        s.chosen_sims.insert("t".into(), "1".into());
        assert_eq!(s.sim().as_deref(), Some("1"));
        s.thread = None;
        s.messages.clear();
        assert_eq!(s.sim().as_deref(), Some("2"), "a new conversation has its own");
    }

    #[test]
    fn recent_conversations_are_bounded_and_keep_the_latest() {
        let mut s = super::State::default();
        let message = |id: &str| super::SmsMessage {
            id: id.into(),
            thread: "t".into(),
            address: "a".into(),
            body: "b".into(),
            date: 1,
            outgoing: false,
            status: None,
            parts: Vec::new(),
            sim: None,
        };
        for i in 0..(super::RECENT_CONVERSATIONS + 3) {
            super::remember_recent(&mut s, i.to_string(), vec![message("m")]);
        }
        assert_eq!(s.recent.len(), super::RECENT_CONVERSATIONS);
        assert!(!s.recent.iter().any(|(t, _)| t == "0"), "the oldest went");
        super::remember_recent(&mut s, "5".into(), vec![message("new")]);
        assert_eq!(s.recent.last().unwrap().0, "5", "reading one again makes it the latest");
        assert_eq!(s.recent.iter().filter(|(t, _)| t == "5").count(), 1);
    }

    #[test]
    fn a_conversation_read_here_stops_counting_until_something_newer_arrives() {
        let mut s = super::State::default();
        assert!(super::note_seen(&mut s, "t", 100).is_none(), "no phone open, nothing to save");
        assert_eq!(s.seen["t"], 100);
        assert!(super::note_seen(&mut s, "t", 100).is_none(), "already known");
        super::note_seen(&mut s, "t", 200);
        assert_eq!(s.seen["t"], 200);
    }

    use super::*;

    fn thread(addresses: &[&str], names: &[&str]) -> SmsThread {
        SmsThread {
            id: "1".into(),
            addresses: addresses.iter().map(|a| (*a).into()).collect(),
            names: names.iter().map(|n| (*n).into()).collect(),
            snippet: String::new(),
            date: 0,
            unread: 0,
            fresh: None,
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
    fn the_badge_counts_the_new_texts_and_the_rest_are_only_unread() {
        let mut t = thread(&["+1555"], &["Sam"]);
        t.unread = 3;
        // A phone that doesn't tell them apart: all of them.
        assert_eq!(thread_json(&t, None)["fresh"], 3);
        t.fresh = Some(1);
        let json = thread_json(&t, None);
        assert_eq!((json["unread"].as_u64(), json["fresh"].as_u64()), (Some(3), Some(1)));
        t.fresh = Some(0);
        assert_eq!(thread_json(&t, None)["fresh"], 0);
        // Never more than the unread.
        t.fresh = Some(9);
        assert_eq!(thread_json(&t, None)["fresh"], 3);
    }

    #[test]
    fn drafts_outlive_the_page_and_go_when_emptied() {
        save_draft("dev|t1".into(), "half a reply".into());
        assert_eq!(draft("dev|t1"), "half a reply");
        assert_eq!(draft("dev|t2"), "");
        save_draft("dev|t1".into(), String::new());
        assert_eq!(draft("dev|t1"), "");
    }

    #[test]
    fn a_reply_that_didnt_go_out_stays_when_the_conversation_is_read_again() {
        let record = |id: i64, text: &str| ChatMessageRecord {
            id,
            peer: DeviceId([1; 32]),
            thread_id: "chat:a".into(),
            sender: None,
            text: text.into(),
            time_ms: 1,
            self_sent: true,
            avatar: None,
        };
        let mut s = State { failed_replies: vec![record(-5, "unsent")], ..State::default() };
        set_chat_messages(&mut s, "chat:a", vec![record(1, "from the phone")]);
        assert_eq!(s.chat_messages.len(), 2);
        assert_eq!(chat_message_json(&s.chat_messages[1], true)["status"], "failed");
        assert_eq!(chat_message_json(&record(-6, "x"), false)["status"], "pending");
        set_chat_messages(&mut s, "chat:b", Vec::new());
        assert!(s.chat_messages.is_empty(), "only in its own conversation");
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
        if cfg!(windows) {
            assert_eq!(
                url_to_path("file:///C:/Users/Sam/My%20Photo.png"),
                PathBuf::from("C:/Users/Sam/My Photo.png")
            );
        }
        // Linux and macOS paths stay absolute.
        assert_eq!(
            url_to_path("file:///home/alice/Pictures/a%20b.png"),
            PathBuf::from("/home/alice/Pictures/a b.png")
        );
        assert_eq!(url_to_path("file://server/share/a.png"), PathBuf::from("//server/share/a.png"));
        assert_eq!(url_to_path("/already/a/path.png"), PathBuf::from("/already/a/path.png"));
    }

    #[test]
    fn what_was_done_to_a_conversation_shows_in_its_row() {
        let mut local = messages_local::Local::default();
        let mut row = json!({ "unread": 0, "snippet": "see you" });
        mark_thread(&mut row, &local, "7", 100);
        assert_eq!((row["unread"].as_u64(), row["markedUnread"].as_bool()), (Some(0), Some(false)));

        // Marked unread counts as one text, and more when the phone has more.
        local.mark_unread("7");
        local.delete_messages("7", ["sms:1"], Some(100));
        mark_thread(&mut row, &local, "7", 100);
        assert_eq!((row["unread"].as_u64(), row["markedUnread"].as_bool()), (Some(1), Some(true)));
        assert_eq!(row["snippet"].as_str(), Some(""));
        let mut busy = json!({ "unread": 4, "snippet": "x" });
        mark_thread(&mut busy, &local, "7", 100);
        assert_eq!(busy["unread"], 4);

        // A newer message: its own text.
        let mut fresh = json!({ "unread": 1, "snippet": "new text" });
        mark_thread(&mut fresh, &local, "7", 200);
        assert_eq!(fresh["snippet"].as_str(), Some("new text"));
    }

    /// A deleted conversation leaves the list, and shows again with a newer message.
    #[test]
    fn a_deleted_conversation_leaves_the_list_until_a_newer_message() {
        let device = DeviceId([9; 32]);
        let threads = ["1", "2", "3"].map(|id| {
            sms_thread(id, "Person", &format!("+1555{id}"), "hello", 100 + id.parse::<i64>().unwrap())
        });
        state(|s| {
            *s = State { device: Some(device), threads: threads.to_vec(), ..State::default() };
        });
        messages_local::update(device, |l| l.delete_thread("2", 102));
        let ids = |v: &View| -> Vec<String> {
            v.threads.as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap().to_owned()).collect()
        };

        assert_eq!(ids(&view()), ["3", "1"]);
        state(|s| s.threads[1].date = 500);
        assert_eq!(ids(&view()), ["2", "3", "1"]);

        messages_local::forget(device);
        state(|s| *s = State::default());
    }

    #[test]
    fn a_phone_that_could_not_delete_is_told_in_words_and_never_by_its_own_reason() {
        let said = |e: Error| delete_problem(&e);
        for e in [Error::Offline, Error::Timeout, Error::NotPaired, Error::Network("x".into())] {
            assert!(said(e).contains("isn't connected"));
        }
        assert!(said(Error::Denied).contains("turned off"));
        assert!(said(Error::Unsupported).contains("doesn't delete"));
        let refused = said(Error::Protocol("INTERNAL: the phone couldn't do it".into()));
        assert!(refused.contains("refused"), "{refused}");
        // The PC's own side is always said to be done, and the phone's reason is not repeated.
        for e in [Error::Offline, Error::Denied, Error::Unsupported, Error::Protocol("secret".into())] {
            let text = said(e);
            assert!(
                text.contains("removed from this PC only") || text.contains("removed from this PC only.")
            );
            assert!(!text.contains("secret") && !text.contains("INTERNAL"));
        }
    }

    #[test]
    fn a_picture_is_saved_under_its_own_kind_of_name() {
        assert_eq!(picture_mime(b"\x89PNG\r\n\x1a\nrest"), "image/png");
        assert_eq!(picture_mime(b"\xff\xd8\xff\xe0"), "image/jpeg");
        assert_eq!(picture_mime(b""), "image/jpeg");
        let suggestion = picture_save_suggestion("file:///cache/sms-part-1.png");
        assert!(suggestion.starts_with("file://") && suggestion.ends_with("/Picture.png"), "{suggestion}");
        assert!(picture_save_suggestion("file:///cache/sms-part-2").ends_with("/Picture.jpg"));
    }
}
