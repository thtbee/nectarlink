// SPDX-License-Identifier: GPL-3.0-or-later
//! Text messages on the PC (docs/protocol/sms.md): a phone's conversations
//! and the open one's messages, read through the phone on demand and kept
//! only in memory. Texts sent from here go out through the phone.

use std::{collections::HashMap, path::PathBuf, sync::Mutex};

use nectarlink_core::{DeviceId, Error, LinkState, NodeEvent, SmsMessage, SmsThread};
use serde_json::{Value, json};

use crate::{core_host, state::Changes};

/// Conversations listed, and messages read at a time.
const THREADS: u32 = 60;
const PAGE: u32 = 100;

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

#[derive(Debug, Default)]
struct State {
    device: Option<DeviceId>,
    status: Status,
    threads: Vec<SmsThread>,
    /// The open conversation.
    thread: Option<String>,
    /// Its messages, newest first.
    messages: Vec<SmsMessage>,
    /// Older messages may exist.
    more: bool,
    loading_older: bool,
    sending: bool,
    /// Pictures fetched, by part ID.
    pictures: HashMap<String, PathBuf>,
    /// Contact photos saved, by thread ID.
    photos: HashMap<String, PathBuf>,
    /// A number pre-filled in the new-message box (from Calls / Contacts).
    compose_to: Option<String>,
    compose_name: Option<String>,
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
        if !s.sending && (s.device.is_some() || !s.threads.is_empty() || !s.messages.is_empty()) {
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
}

pub fn view() -> View {
    state(|s| View {
        device: s.device,
        status: s.status,
        threads: Value::Array(s.threads.iter().map(|t| thread_json(t, s.photos.get(&t.id))).collect()),
        thread: s.thread.clone(),
        messages: {
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
        },
        more: s.more,
        loading_older: s.loading_older,
        sending: s.sending,
        compose_to: s.compose_to.clone(),
        compose_name: s.compose_name.clone(),
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

/// A one-time code in a text ("243928 is your OTP"): 4 to 8 digits on
/// their own, in a text that says it's a code.
pub fn one_time_code(body: &str) -> Option<String> {
    let lower = body.to_lowercase();
    const WORDS: [&str; 7] = ["otp", "code", "password", "passcode", "verification", "pin", "one time"];
    if !WORDS.iter().any(|w| lower.contains(w)) {
        return None;
    }
    let chars: Vec<char> = body.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        // On its own: not part of a word ("XX3465"), an amount ("Rs.500",
        // "1,000") or a longer number.
        let before = start.checked_sub(1).map(|b| chars[b]);
        let after = chars.get(i).copied();
        let alone = before
            .is_none_or(|c| !c.is_alphanumeric() && !matches!(c, '.' | ',' | '₹' | '$' | '€' | '£'))
            && after.is_none_or(|c| {
                !c.is_alphanumeric()
                    && !(matches!(c, '.' | ',') && chars.get(i + 1).is_some_and(char::is_ascii_digit))
            });
        if alone && (4..=8).contains(&(i - start)) {
            return Some(chars[start..i].iter().collect());
        }
    }
    None
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

// ---- What the page asks ----

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

pub fn load_threads() {
    let Some((device, generation)) = state(|s| {
        if s.threads.is_empty() {
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
                    s.threads = threads;
                    s.photos.extend(photos);
                    s.status = Status::Ready;
                }
                Err(e) => {
                    tracing::debug!(error = %e, "can't list conversations");
                    s.status = Status::of(&e);
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

/// Opens a conversation: its latest messages.
pub fn open_thread(thread: String) {
    state(|s| {
        s.compose_to = None;
        s.compose_name = None;
        if s.thread.as_deref() != Some(&thread) {
            s.thread = Some(thread);
            s.messages.clear();
            s.more = false;
        }
    });
    changed();
    load_messages(None);
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

/// The open conversation's older messages.
pub fn load_older() {
    let before = state(|s| {
        if s.loading_older || !s.more {
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
                    s.status = Status::of(&e);
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

/// Why a text wasn't sent, for the page.
pub fn send_problem(error: &Error) -> &'static str {
    match error {
        Error::Denied => "Messages are turned off for this phone.",
        Error::Unsupported => {
            "The phone doesn't send texts for this PC. Allow it in the Nectarlink app on the phone."
        }
        Error::Offline | Error::NotPaired | Error::Timeout => "The phone isn't connected.",
        Error::TooLarge => "That's too long for a text.",
        _ => "The phone couldn't send it.",
    }
}

/// Sends a text in the open conversation.
pub fn send(body: String) {
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

/// Sends a text to a number (a new conversation), then opens it.
pub fn send_to(to: Vec<String>, body: String) {
    static NEXT_LOCAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let now = now_ms();
    let local_id = format!("local:{now}:{}", NEXT_LOCAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let Some(device) = state(|s| {
        s.sending = true;
        s.compose_to = None;
        s.compose_name = None;
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
                    parts: Vec::new(),
                },
            );
        }
        s.device
    }) else {
        return;
    };
    changed();
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        let result = node.sms_send(device, to.clone(), body).await;
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
        s.more = false;
    });
    changed();
}

pub fn on_event(event: &NodeEvent) {
    let device = state(|s| s.device);
    match event {
        NodeEvent::SmsChanged { device: d, .. } if Some(*d) == device => {
            load_threads();
            if state(|s| s.thread.is_some()) {
                load_messages(None);
            }
        }
        NodeEvent::LinkChanged { device: d, link: LinkState::Online { .. } }
            if Some(*d) == device && state(|s| s.status != Status::Ready) =>
        {
            load_threads();
        }
        NodeEvent::LinkChanged { device: d, link: LinkState::Offline { .. } } if Some(*d) == device => {
            state(|s| s.status = Status::Offline);
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
        assert_eq!(found("Use code 1234-5678"), Some("1234".into()));
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
}
