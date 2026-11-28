// SPDX-License-Identifier: GPL-3.0-or-later
//! What's waiting on each paired device: texts that came in and weren't read
//! here, and missed calls that weren't looked at. The navigation rail shows
//! them for the device shown (on Messages and Calls) and the total of the
//! others on the device button. Read again when a phone says a text arrived,
//! a call was missed or it comes back online, and when a conversation is read
//! here.
//! Only what arrives after a phone was first read counts, and only the new
//! texts of a conversation, not those the phone still calls unread after they
//! were read here: only its own messaging app can clear its count, so using
//! it would keep a number up that nothing here could take away.

use std::{collections::HashMap, path::PathBuf, sync::Mutex};

use nectarlink_core::{DeviceId, LinkState, NodeEvent};
use serde::{Deserialize, Serialize};

use crate::core_host;

/// Conversations and calls read to count from.
const THREADS: u32 = 40;
const CALLS: u32 = 25;
/// Most texts of a conversation read to tell the new ones.
const MESSAGES: u32 = 20;
/// How long after a notification goes a phone with something waiting is read again.
const RECHECK_AFTER: std::time::Duration = std::time::Duration::from_millis(1500);

/// What is kept for a phone between runs. Dates are the phone's.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Remembered {
    /// Each conversation's latest message when the phone was first read:
    /// texts up to there don't count. `None` until read.
    #[serde(default)]
    texts: Option<HashMap<String, i64>>,
    /// The same for the chats read from notifications.
    #[serde(default)]
    chats: Option<HashMap<String, i64>>,
    /// Missed calls up to here were looked at. `None` until read.
    #[serde(default)]
    calls_seen: Option<i64>,
}

/// A conversation of a phone, as last read.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Conversation {
    id: String,
    /// Its latest message.
    date: i64,
    /// The phone's count, which includes texts read here.
    unread: u32,
    /// When the received texts that could be new were, when the phone said.
    incoming: Option<Vec<i64>>,
}

/// What a phone has waiting, as last read from it.
#[derive(Debug, Default)]
struct Waiting {
    texts: Vec<Conversation>,
    /// When its missed calls were.
    missed: Vec<i64>,
}

#[derive(Debug, Default)]
struct Badges {
    /// The device shown.
    device: Option<DeviceId>,
    waiting: HashMap<DeviceId, Waiting>,
    /// The Calls page is showing: what's missed now is seen.
    calls_open: bool,
    remembered: HashMap<DeviceId, Remembered>,
    loaded: bool,
    /// Every device has been read once.
    started: bool,
    /// What the rail shows: texts and missed calls of the device shown, and
    /// each device's total (JSON).
    shown: (u32, u32, String),
    /// Devices with something waiting, as last counted.
    pending: std::collections::HashSet<DeviceId>,
    /// Devices about to be read again after a notification went.
    rechecking: std::collections::HashSet<DeviceId>,
}

static BADGES: Mutex<Option<Badges>> = Mutex::new(None);

fn state<T>(f: impl FnOnce(&mut Badges) -> T) -> T {
    let mut guard = BADGES.lock().unwrap_or_else(|e| e.into_inner());
    let badges = guard.get_or_insert_default();
    // What was kept comes first: nothing may be saved over it before.
    load(badges);
    f(badges)
}

fn file() -> Option<PathBuf> {
    core_host::try_data_dir().map(|d| d.join("badges.json"))
}

fn load(b: &mut Badges) {
    if b.loaded {
        return;
    }
    b.loaded = true;
    let read: HashMap<String, Remembered> = file()
        .and_then(|f| std::fs::read(f).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    b.remembered = read.into_iter().filter_map(|(id, r)| Some((id.parse().ok()?, r))).collect();
}

fn save(b: &Badges) {
    let (Some(file), Ok(bytes)) = (
        file(),
        serde_json::to_vec(
            &b.remembered.iter().map(|(id, r)| (id.to_string(), r.clone())).collect::<HashMap<_, _>>(),
        ),
    ) else {
        return;
    };
    if let Err(e) = std::fs::write(file, bytes) {
        tracing::debug!(error = %e, "can't save what was looked at");
    }
}

/// Texts that came in and weren't read here. In each conversation that has
/// something after what was there at first (`base`) and what was read here
/// (`seen`): the received texts after both, or the phone's count when it
/// couldn't say which. Not the open conversation, which is being read.
fn unread_texts(
    texts: &[Conversation],
    base: &HashMap<String, i64>,
    seen: &HashMap<String, i64>,
    open: Option<&str>,
) -> u32 {
    texts
        .iter()
        .filter(|c| open != Some(c.id.as_str()))
        .map(|c| {
            let after = base.get(&c.id).copied().unwrap_or(0).max(seen.get(&c.id).copied().unwrap_or(0));
            if c.date <= after {
                return 0;
            }
            match &c.incoming {
                Some(dates) => dates.iter().filter(|d| **d > after).count() as u32,
                None => c.unread,
            }
        })
        .sum()
}

/// Missed calls after `seen`.
fn unseen_missed(missed: &[i64], seen: i64) -> u32 {
    missed.iter().filter(|date| **date > seen).count() as u32
}

/// Shows the counts of `device` on the rail (none for no device).
pub fn watch(device: Option<DeviceId>) {
    let (changed, first) = state(|b| {
        load(b);
        let first = !std::mem::replace(&mut b.started, true);
        let changed = b.device != device;
        b.device = device;
        (changed, first)
    });
    if first {
        // The others' totals are on the device button: read them all once.
        let all: Vec<DeviceId> = core_host::host().hub.read(|s| s.devices.iter().map(|d| d.id).collect());
        for id in all {
            refresh(id);
        }
    } else if changed && let Some(id) = device {
        refresh(id);
    }
    if changed {
        publish();
    }
}

/// The Calls page is showing (or not): its missed calls are being looked at.
pub fn set_calls_open(open: bool) {
    state(|b| b.calls_open = open);
    publish();
}

/// Reads a phone's conversations and calls again.
pub fn refresh(device: DeviceId) {
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        let threads = node.sms_threads(device, THREADS).await.ok();
        let missed = node.call_log(device, None, CALLS).await.ok().map(|entries| {
            entries.into_iter().filter(|e| e.direction == "missed").map(|e| e.date).collect::<Vec<_>>()
        });
        // The first time, what's there is old.
        let chats: HashMap<String, i64> =
            node.chat_threads(device).into_iter().map(|t| (t.thread_id, t.updated_ms)).collect();
        state(|b| {
            let r = b.remembered.entry(device).or_default();
            if let Some(threads) = &threads
                && r.texts.is_none()
            {
                r.texts = Some(threads.iter().map(|t| (t.id.clone(), t.date)).collect());
            }
            if let Some(missed) = &missed
                && r.calls_seen.is_none()
            {
                r.calls_seen = Some(missed.iter().copied().max().unwrap_or(0));
            }
            if r.chats.is_none() {
                r.chats = Some(chats);
            }
            save(b);
        });
        tracing::info!(
            device = %device.short(),
            conversations = threads.as_ref().map(Vec::len),
            missed = missed.as_ref().map(Vec::len),
            "read what's waiting on a phone"
        );
        let conversations = match threads {
            Some(threads) => Some(conversations(&node, device, threads).await),
            None => None,
        };
        state(|b| {
            let waiting = b.waiting.entry(device).or_default();
            if let Some(texts) = conversations {
                waiting.texts = texts;
            }
            if let Some(missed) = missed {
                waiting.missed = missed;
            }
        });
        publish();
    });
}

/// The phone's conversations, with the received texts that could be new in
/// those that have something after what was read here.
async fn conversations(
    node: &nectarlink_core::Node,
    device: DeviceId,
    threads: Vec<nectarlink_core::SmsThread>,
) -> Vec<Conversation> {
    let base = state(|b| b.remembered.get(&device).and_then(|r| r.texts.clone())).unwrap_or_default();
    let seen = crate::messages::seen_for(device);
    let mut out = Vec::new();
    for t in threads {
        let after = base.get(&t.id).copied().unwrap_or(0).max(seen.get(&t.id).copied().unwrap_or(0));
        let incoming = if t.unread > 0 && t.date > after {
            node.sms_messages(device, t.id.clone(), None, t.unread.min(MESSAGES))
                .await
                .ok()
                .map(|m| m.into_iter().filter(|m| !m.outgoing).map(|m| m.date).collect())
        } else {
            None
        };
        out.push(Conversation { id: t.id, date: t.date, unread: t.unread, incoming });
    }
    out
}

/// A call was missed on a phone: its log has it a moment later.
pub fn missed_call(device: DeviceId) {
    core_host::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        refresh(device);
    });
}

/// A notification went from a phone that has something waiting: it may have
/// been dealt with there (the messaging app's own "Mark as read" button, say),
/// which changes its messages in an order that can't be told in advance. Read
/// it again once it has settled. Only while something waits: notifications
/// come and go all the time.
fn recheck_if_waiting(device: DeviceId) {
    let schedule = state(|b| b.pending.contains(&device) && b.rechecking.insert(device));
    if !schedule {
        return;
    }
    core_host::spawn(async move {
        tokio::time::sleep(RECHECK_AFTER).await;
        state(|b| b.rechecking.remove(&device));
        refresh(device);
    });
}

pub fn on_event(event: &NodeEvent) {
    match event {
        NodeEvent::SmsChanged { device, .. } => refresh(*device),
        // Chats read from their notifications are counted from the store.
        NodeEvent::NotificationPosted { .. } | NodeEvent::NotificationsReset { .. } => publish(),
        NodeEvent::NotificationRemoved { device, .. } => {
            publish();
            recheck_if_waiting(*device);
        }
        NodeEvent::LinkChanged { device, link: LinkState::Online { .. } } => refresh(*device),
        NodeEvent::DeviceRemoved(device) => {
            state(|b| {
                if b.remembered.remove(device).is_some() {
                    save(b);
                }
                b.waiting.remove(device);
                if b.device == Some(*device) {
                    b.device = None;
                }
            });
            publish();
        }
        _ => {}
    }
}

/// What's waiting on `device`: texts, then missed calls.
fn counts(device: DeviceId, watched: bool) -> (u32, u32) {
    // What the open conversation and the texts read here say is kept by
    // `messages`: asked outside this lock.
    let Some((texts, missed, r)) = state(|b| {
        let r = b.remembered.get(&device).cloned()?;
        let w = b.waiting.get(&device)?;
        Some((w.texts.clone(), w.missed.clone(), r))
    }) else {
        return (0, 0);
    };
    let open = crate::messages::open_thread_of(device);
    // Chats of apps hidden on the PC aren't in Messages, so they don't count (#700).
    let hidden = core_host::try_host()
        .map(|host| host.hub.read(crate::state::AppState::hidden_apps))
        .unwrap_or_default();
    // Deleted here, they don't count. Marked unread doesn't either: that's only shown bold.
    let local = crate::messages_local::read(device);
    let counted = |id: &str, date: i64| !local.thread_hidden(id, date);
    let chats: u32 = match (&r.chats, core_host::node()) {
        (Some(base), Some(node)) => {
            let threads = crate::messages::chat_threads_of(&node, device);
            threads
                .iter()
                .filter(|t| {
                    counted(&t.thread_id, t.updated_ms)
                        && t.updated_ms > base.get(&t.thread_id).copied().unwrap_or(0)
                        && open.as_deref() != Some(t.thread_id.as_str())
                        && !hidden.contains(&t.app)
                })
                .map(|t| t.unread)
                .sum()
        }
        _ => 0,
    };
    let texts: Vec<Conversation> = texts.into_iter().filter(|c| counted(&c.id, c.date)).collect();
    let texts = match &r.texts {
        // Reading the files of what was read here only when something could count.
        Some(base)
            if texts.iter().any(|c| c.unread > 0 && c.date > base.get(&c.id).copied().unwrap_or(0)) =>
        {
            unread_texts(&texts, base, &crate::messages::seen_for(device), open.as_deref())
        }
        _ => 0,
    };
    // On the Calls page what's missed is seen as it comes.
    let calls_seen = state(|b| {
        let seen = b.remembered.get(&device).and_then(|r| r.calls_seen)?;
        if watched
            && b.calls_open
            && let Some(latest) = missed.iter().max().copied().filter(|d| *d > seen)
        {
            if let Some(r) = b.remembered.get_mut(&device) {
                r.calls_seen = Some(latest);
            }
            save(b);
            return Some(latest);
        }
        Some(seen)
    });
    (texts + chats, calls_seen.map_or(0, |seen| unseen_missed(&missed, seen)))
}

/// Works out the counts and tells the rail if they changed.
pub fn publish() {
    let (watched, devices) = state(|b| (b.device, b.remembered.keys().copied().collect::<Vec<_>>()));
    let mut totals = serde_json::Map::new();
    let mut totals_devices = std::collections::HashSet::new();
    let mut shown = (0, 0);
    for device in devices {
        let (texts, missed) = counts(device, watched == Some(device));
        if watched == Some(device) {
            shown = (texts, missed);
        }
        if texts + missed > 0 {
            totals.insert(device.to_string(), (texts + missed).into());
            totals_devices.insert(device);
        }
    }
    let all = serde_json::Value::Object(totals).to_string();
    let changed = state(|b| {
        b.pending = totals_devices.clone();
        let now = (shown.0, shown.1, all.clone());
        let changed = b.shown != now;
        b.shown = now;
        changed
    });
    if changed {
        tracing::info!(texts = shown.0, missed = shown.1, devices = %all, "badges changed");
        crate::bridge::app::show_badges(shown.0, shown.1, &all);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conversation(id: &str, date: i64, unread: u32, incoming: Option<Vec<i64>>) -> Conversation {
        Conversation { id: id.into(), date, unread, incoming }
    }

    #[test]
    fn only_new_received_texts_not_read_here_count() {
        let base = HashMap::from([("1".to_owned(), 100), ("2".to_owned(), 200)]);
        let none = HashMap::new();
        // The phone says 2 unread, but one of them was there at first.
        let texts = vec![conversation("1", 300, 2, Some(vec![300, 90]))];
        assert_eq!(unread_texts(&texts, &base, &none, None), 1);
        // Read here up to the newest: nothing. Up to a text before it: that one.
        let seen = HashMap::from([("1".to_owned(), 300)]);
        assert_eq!(unread_texts(&texts, &base, &seen, None), 0);
        let texts = vec![conversation("1", 400, 3, Some(vec![400, 300, 250]))];
        assert_eq!(unread_texts(&texts, &base, &seen, None), 1);
        // A conversation that wasn't there at first counts from its start.
        let texts = vec![conversation("9", 50, 2, Some(vec![50, 40]))];
        assert_eq!(unread_texts(&texts, &base, &none, None), 2);
        // Nothing after what was there: not counted, whatever the phone says.
        let texts = vec![conversation("2", 200, 5, None)];
        assert_eq!(unread_texts(&texts, &base, &none, None), 0);
        // When the phone couldn't say which texts, its count stands.
        let texts = vec![conversation("2", 250, 3, None)];
        assert_eq!(unread_texts(&texts, &base, &none, None), 3);
        // The one being read doesn't count.
        let texts = vec![conversation("1", 300, 2, Some(vec![300]))];
        assert_eq!(unread_texts(&texts, &base, &none, Some("1")), 0);
    }

    #[test]
    fn missed_calls_count_until_looked_at() {
        assert_eq!(unseen_missed(&[100, 200, 300], 150), 2);
        assert_eq!(unseen_missed(&[100, 200, 300], 300), 0);
        assert_eq!(unseen_missed(&[], 0), 0);
    }
}
