// SPDX-License-Identifier: MPL-2.0
//! What happened lately, for agents that wait for something instead of
//! asking again and again: a notification arrives, a text comes in, a
//! transfer finishes, a device connects.
//!
//! The app feeds every core event to [`EventLog::push`]; only the few kinds
//! below are kept, without message text where a later read gets it anyway.
//! An agent reads them with `events.wait`, which shows only the events its
//! permissions and devices allow.

use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
    time::Duration,
};

use nectarlink_core::{DeviceId, LinkState, NodeEvent, TransferState};
use serde_json::{Value, json};
use tokio::sync::Notify;

use crate::{def::Scope, store::now};

/// How many events are kept. Older ones are dropped, and a reader that falls
/// that far behind is told so.
pub const KEEP: usize = 500;
/// The longest one wait may take, so HTTP and MCP clients don't time out.
pub const MAX_WAIT: Duration = Duration::from_secs(50);

/// One event as agents see it.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    /// Grows by one per event; pass the last one seen as `since`.
    pub seq: u64,
    /// Unix seconds.
    pub ts: i64,
    /// `notification.posted`, `sms.changed`, `transfer.finished`...
    pub kind: &'static str,
    /// The permission an agent needs to see it.
    pub scope: Scope,
    pub device: Option<DeviceId>,
    /// Details. Text in here comes from other people and is cleaned when read.
    pub data: Value,
}

/// Every event kind, for `events.wait`'s filter.
pub const KINDS: [&str; 13] = [
    "device.connected",
    "device.disconnected",
    "notification.posted",
    "notification.removed",
    "sms.changed",
    "call.changed",
    "transfer.started",
    "transfer.finished",
    "transfer.failed",
    "clipboard.received",
    "photo.added",
    "storage.changed",
    "phone_settings.changed",
];

#[derive(Debug, Default)]
struct Inner {
    events: VecDeque<Event>,
    next: u64,
    /// The last state seen per transfer, so progress doesn't flood the log.
    transfers: HashMap<String, &'static str>,
    /// Whether each device was last seen online.
    online: HashMap<DeviceId, bool>,
}

#[derive(Debug, Default)]
pub struct EventLog {
    inner: Mutex<Inner>,
    arrived: Notify,
}

impl EventLog {
    pub fn new() -> EventLog {
        EventLog::default()
    }

    /// Keeps the core event if agents care about it.
    pub fn push(&self, event: &NodeEvent) {
        let mut inner = lock(&self.inner);
        let Some((kind, scope, device, data)) = convert(&mut inner, event) else { return };
        inner.next += 1;
        let seq = inner.next;
        inner.events.push_back(Event { seq, ts: now(), kind, scope, device, data });
        while inner.events.len() > KEEP {
            inner.events.pop_front();
        }
        drop(inner);
        self.arrived.notify_waiters();
    }

    /// The newest sequence number, 0 before anything happened.
    pub fn last_seq(&self) -> u64 {
        lock(&self.inner).next
    }

    /// Events after `since` that `keep` lets through, oldest first, and
    /// whether some were already dropped. Waits up to `wait` for the first.
    pub async fn wait(
        &self,
        since: u64,
        wait: Duration,
        limit: usize,
        keep: impl Fn(&Event) -> bool,
    ) -> (Vec<Event>, bool) {
        let deadline = tokio::time::Instant::now() + wait.min(MAX_WAIT);
        loop {
            // Register before looking, so an event between the look and the
            // wait isn't missed.
            let arrived = self.arrived.notified();
            tokio::pin!(arrived);
            arrived.as_mut().enable();
            let (found, missed) = self.after(since, limit, &keep);
            if !found.is_empty() || tokio::time::Instant::now() >= deadline {
                return (found, missed);
            }
            if tokio::time::timeout_at(deadline, arrived).await.is_err() {
                return self.after(since, limit, &keep);
            }
        }
    }

    fn after(&self, since: u64, limit: usize, keep: &impl Fn(&Event) -> bool) -> (Vec<Event>, bool) {
        let inner = lock(&self.inner);
        let oldest = inner.events.front().map(|e| e.seq).unwrap_or(inner.next + 1);
        let missed = since > 0 && since + 1 < oldest;
        let found = inner.events.iter().filter(|e| e.seq > since && keep(e)).take(limit).cloned().collect();
        (found, missed)
    }
}

type Converted = (&'static str, Scope, Option<DeviceId>, Value);

fn convert(inner: &mut Inner, event: &NodeEvent) -> Option<Converted> {
    Some(match event {
        NodeEvent::LinkChanged { device, link } => {
            let online = matches!(link, LinkState::Online { .. });
            if matches!(link, LinkState::Connecting) || inner.online.insert(*device, online) == Some(online) {
                return None;
            }
            let kind = if online { "device.connected" } else { "device.disconnected" };
            (kind, Scope::Devices, Some(*device), json!({}))
        }
        NodeEvent::NotificationPosted { device, notification: n } => (
            "notification.posted",
            Scope::Notifications,
            Some(*device),
            json!({
                "key": n.key,
                "app": n.app,
                "app_name": n.app_name,
                "title": n.title,
                "body": n.text,
            }),
        ),
        NodeEvent::NotificationRemoved { device, key } => {
            ("notification.removed", Scope::Notifications, Some(*device), json!({ "key": key }))
        }
        NodeEvent::SmsChanged { device, thread } => {
            ("sms.changed", Scope::Messages, Some(*device), json!({ "thread": thread }))
        }
        NodeEvent::Call { device, call } => (
            "call.changed",
            Scope::Calls,
            Some(*device),
            json!({ "id": call.id, "state": call.state, "incoming": call.incoming }),
        ),
        NodeEvent::Transfer(t) => {
            let state = match &t.state {
                TransferState::Done { .. } => "transfer.finished",
                TransferState::Failed(_) | TransferState::Cancelled => "transfer.failed",
                _ => "transfer.started",
            };
            if inner.transfers.insert(t.id.clone(), state) == Some(state) {
                return None;
            }
            if inner.transfers.len() > KEEP {
                inner.transfers.retain(|_, s| *s == "transfer.started");
            }
            (
                state,
                Scope::Files,
                Some(t.device),
                json!({
                    "id": t.id,
                    "direction": format!("{:?}", t.direction).to_lowercase(),
                    "files": t.files,
                    "bytes_total": t.total,
                    "names": t.names.iter().take(5).collect::<Vec<_>>(),
                }),
            )
        }
        NodeEvent::ClipboardReceived { device } => {
            ("clipboard.received", Scope::Clipboard, Some(*device), json!({}))
        }
        NodeEvent::PhotoAdded { device, photo } => (
            "photo.added",
            Scope::Photos,
            Some(*device),
            json!({ "id": photo.id, "name": photo.name, "screenshot": photo.screenshot }),
        ),
        NodeEvent::StorageChanged { device, path } => {
            ("storage.changed", Scope::Storage, Some(*device), json!({ "path": path }))
        }
        NodeEvent::PhoneToggles { device, toggles } => (
            "phone_settings.changed",
            Scope::PhoneSettings,
            Some(*device),
            serde_json::to_value(toggles).unwrap_or_default(),
        ),
        _ => return None,
    })
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
