// SPDX-License-Identifier: MPL-2.0
//! The event log behind `events.wait`: what it keeps, what it drops, and
//! that a waiting agent wakes up when something arrives.

use std::{sync::Arc, time::Duration};

use nectarlink_actions::{Scope, events::EventLog};
use nectarlink_core::{ConnectionPath, DeviceId, LinkState, NodeEvent};

fn device(n: u8) -> DeviceId {
    DeviceId([n; 32])
}

fn online(d: DeviceId) -> NodeEvent {
    NodeEvent::LinkChanged { device: d, link: LinkState::Online { path: ConnectionPath::Lan, rtt_ms: 3 } }
}

fn offline(d: DeviceId) -> NodeEvent {
    NodeEvent::LinkChanged { device: d, link: LinkState::Offline { last_seen: None } }
}

#[tokio::test]
async fn only_changes_are_kept_and_noise_is_dropped() {
    let log = EventLog::new();
    let d = device(1);
    log.push(&online(d));
    log.push(&online(d));
    log.push(&NodeEvent::LinkChanged { device: d, link: LinkState::Connecting });
    log.push(&NodeEvent::ClipboardHistoryChanged);
    log.push(&offline(d));
    log.push(&NodeEvent::SmsChanged { device: d, thread: Some("7".into()) });
    let (events, missed) = log.wait(0, Duration::ZERO, 100, |_| true).await;
    let kinds: Vec<_> = events.iter().map(|e| e.kind).collect();
    assert_eq!(kinds, ["device.connected", "device.disconnected", "sms.changed"]);
    assert!(!missed);
    assert_eq!(events[2].scope, Scope::Messages);
    assert!(events.windows(2).all(|w| w[0].seq < w[1].seq));
}

#[tokio::test]
async fn readers_see_only_what_they_may_and_after_their_cursor() {
    let log = EventLog::new();
    log.push(&NodeEvent::SmsChanged { device: device(1), thread: None });
    log.push(&NodeEvent::ClipboardReceived { device: device(1) });
    log.push(&NodeEvent::SmsChanged { device: device(2), thread: None });
    let (sms, _) = log.wait(0, Duration::ZERO, 100, |e| e.scope == Scope::Messages).await;
    assert_eq!(sms.len(), 2);
    let (later, _) = log.wait(sms[0].seq, Duration::ZERO, 100, |e| e.scope == Scope::Messages).await;
    assert_eq!(later.len(), 1);
    assert_eq!(later[0].device, Some(device(2)));
    let (limited, _) = log.wait(0, Duration::ZERO, 1, |_| true).await;
    assert_eq!(limited.len(), 1);
}

#[tokio::test]
async fn a_reader_that_fell_behind_is_told() {
    let log = EventLog::new();
    for i in 0..(nectarlink_actions::events::KEEP + 10) {
        log.push(&NodeEvent::SmsChanged { device: device(1), thread: Some(i.to_string()) });
    }
    let (events, missed) = log.wait(1, Duration::ZERO, 100, |_| true).await;
    assert!(missed);
    assert_eq!(events.len(), 100);
    let (_, fresh) = log.wait(log.last_seq() - 5, Duration::ZERO, 100, |_| true).await;
    assert!(!fresh);
}

#[tokio::test(start_paused = true)]
async fn a_waiting_reader_wakes_when_something_arrives() {
    let log = Arc::new(EventLog::new());
    let since = log.last_seq();
    let reader = {
        let log = log.clone();
        tokio::spawn(async move { log.wait(since, Duration::from_secs(30), 10, |_| true).await })
    };
    tokio::time::sleep(Duration::from_secs(2)).await;
    // Something the reader may not see doesn't end the wait...
    log.push(&NodeEvent::ClipboardHistoryChanged);
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(!reader.is_finished());
    log.push(&NodeEvent::ClipboardReceived { device: device(3) });
    let (events, _) = reader.await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "clipboard.received");
}

#[tokio::test(start_paused = true)]
async fn a_wait_with_nothing_happening_ends_empty_and_never_beyond_the_cap() {
    let log = EventLog::new();
    let started = tokio::time::Instant::now();
    let (events, _) = log.wait(0, Duration::from_secs(600), 10, |_| true).await;
    assert!(events.is_empty());
    assert!(started.elapsed() <= nectarlink_actions::events::MAX_WAIT + Duration::from_secs(1));
}
