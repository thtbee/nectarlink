// SPDX-License-Identifier: MPL-2.0
//! Problem reports across devices (docs/protocol/diag.md): a device making
//! a report asks a paired one for its part with `diag.ask`. Facts, checks,
//! feature health and crashes come back at once; the recent activity log
//! only after the user taps Share on the device it's on, every time.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use nectarlink_protocol::{
    DeviceId, Envelope, ErrorCode,
    messages::{DiagAsk, DiagCrash, DiagDay, DiagFacts, DiagLog, DiagLogState, DiagPart, diag, types},
};

use crate::{
    Error, Result,
    diag::{crash, health, redact, report},
    events::NodeEvent,
    node::Shared,
    session::Session,
};

/// How long the user has to answer a request for the log.
const ANSWER_WITHIN: Duration = Duration::from_secs(60);
/// One request for the log per device in this long; more are refused.
const ASK_EVERY: Duration = Duration::from_secs(60);
/// How long the facts may take to come back.
const FACTS_WITHIN: Duration = Duration::from_secs(5);

/// The requests in flight, both ways.
#[derive(Debug, Default)]
pub(crate) struct State {
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    /// Asked by us: nonce, to whom and until when an answer counts.
    asked: HashMap<u64, (DeviceId, Instant)>,
    /// Asked of us, waiting for our user: by device, its nonce, whether
    /// "just now", and until when.
    waiting: HashMap<DeviceId, (u64, bool, Instant)>,
    /// When each device last asked us for the log.
    last_ask: HashMap<DeviceId, Instant>,
}

impl State {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A paired device's part of a report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerPart {
    pub part: report::DevicePart,
    /// Whether its log was asked for, and whether its user is being asked.
    pub log: PeerLog,
    /// Matches the [`NodeEvent::DiagLog`] that brings its log.
    pub nonce: u64,
}

/// What became of the request for a device's log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerLog {
    NotAsked,
    /// Its user is being asked; the log follows as a [`NodeEvent::DiagLog`].
    Asking,
    /// It was asked too recently.
    Busy,
}

// ---- Asking ----

pub(crate) async fn ask(shared: &Shared, session: &Session, log: bool, just_now: bool) -> Result<PeerPart> {
    let nonce: u64 = rand::random::<u64>() >> 1;
    if log {
        let mut state = shared.diag_asks.lock();
        let now = Instant::now();
        state.asked.retain(|_, (_, until)| *until > now);
        state.asked.insert(nonce, (session.peer, now + ANSWER_WITHIN + FACTS_WITHIN));
    }
    let env = Envelope::new(types::DIAG_ASK, &DiagAsk { log, just_now, nonce })?;
    let part: DiagPart = session.request(env, FACTS_WITHIN).await?.expect_body(types::DIAG_PART)?;
    let part = part.sanitized();
    let log = match part.log {
        DiagLogState::Asking if log => PeerLog::Asking,
        DiagLogState::Busy => PeerLog::Busy,
        _ => PeerLog::NotAsked,
    };
    if log != PeerLog::Asking {
        shared.diag_asks.lock().asked.remove(&nonce);
    }
    Ok(PeerPart { part: from_wire(part), log, nonce })
}

fn from_wire(part: DiagPart) -> report::DevicePart {
    let clean = |s: String| redact::text(&s);
    let f = part.facts;
    report::DevicePart {
        facts: report::DeviceFacts {
            role: String::new(),
            app_version: clean(f.app_version),
            channel: clean(f.channel),
            install: clean(f.install),
            os: clean(f.os),
            arch: clean(f.arch),
            hardware: f.hardware.into_iter().map(clean).collect(),
            extra: f.extra.into_iter().map(|(n, v)| (clean(n), clean(v))).collect(),
        },
        checks: part.checks.into_iter().map(|(n, v)| (clean(n), clean(v))).collect(),
        health: part
            .health
            .into_iter()
            .map(|d| health::Day {
                day: clean(d.day),
                counts: d
                    .counts
                    .into_iter()
                    .map(|(feature, codes)| {
                        (clean(feature), codes.into_iter().map(|(c, n)| (clean(c), n)).collect())
                    })
                    .collect(),
            })
            .collect(),
        crashes: part
            .crashes
            .into_iter()
            .map(|c| crash::CrashRecord {
                sig: clean(c.sig),
                kind: clean(c.kind),
                location: clean(c.location),
                message: clean(c.message),
                thread: None,
                version: clean(c.version),
                at: c.at,
            })
            .collect(),
        log: Vec::new(),
    }
}

// ---- Answering ----

/// Handles `diag.*`. Returns false for other types.
pub(crate) async fn handle(shared: &Arc<Shared>, session: &Arc<Session>, env: &Envelope) -> Result<bool> {
    match env.t.as_str() {
        types::DIAG_ASK => {
            let (shared, session, env) = (shared.clone(), session.clone(), env.clone());
            tokio::spawn(async move {
                let reply = answer(&shared, &session, &env).await.unwrap_or_else(|e| {
                    tracing::debug!(error = %e, "bad diag request");
                    Envelope::error(ErrorCode::BadMessage, "invalid request")
                });
                let _ = session.send(reply.reply_to(env.id)).await;
            });
            Ok(true)
        }
        types::DIAG_LOG => {
            let log = env.body::<DiagLog>()?.sanitized();
            let expected = {
                let mut state = shared.diag_asks.lock();
                match state.asked.get(&log.ask) {
                    Some((peer, until)) if *peer == session.peer && *until > Instant::now() => {
                        state.asked.remove(&log.ask);
                        true
                    }
                    _ => false,
                }
            };
            if !expected {
                tracing::debug!(peer = %session.peer.short(), "ignored a log nobody asked for");
                return Ok(true);
            }
            let lines = (!log.declined).then(|| log.lines.iter().map(|l| redact::text(l)).collect());
            shared.emit(NodeEvent::DiagLog { device: session.peer, nonce: log.ask, lines });
            Ok(true)
        }
        _ => Ok(false),
    }
}

async fn answer(shared: &Arc<Shared>, session: &Session, env: &Envelope) -> Result<Envelope> {
    if !shared.we_offer(diag::REPORT) {
        return Ok(Envelope::error(ErrorCode::Unsupported, "this device doesn't answer for reports"));
    }
    let ask: DiagAsk = env.body()?;
    let platform = shared.platform.clone();
    let facts = tokio::task::spawn_blocking(move || platform.diag_facts()).await.ok().flatten();
    let mut part = own_part(facts);
    if ask.log {
        let now = Instant::now();
        let mut state = shared.diag_asks.lock();
        let recent = state.last_ask.get(&session.peer).is_some_and(|at| now.duration_since(*at) < ASK_EVERY);
        if recent {
            part.log = DiagLogState::Busy;
        } else {
            state.last_ask.insert(session.peer, now);
            state.waiting.insert(session.peer, (ask.nonce, ask.just_now, now + ANSWER_WITHIN));
            part.log = DiagLogState::Asking;
        }
    }
    let asking = part.log == DiagLogState::Asking;
    let reply = Envelope::new(types::DIAG_PART, &part)?;
    if asking {
        shared.emit(NodeEvent::DiagLogAsked { device: session.peer });
    }
    Ok(reply)
}

/// This device's part, without its log: what the app says about itself,
/// and the core's own health counts and crashes.
fn own_part(facts: Option<(report::DeviceFacts, Vec<(String, String)>)>) -> DiagPart {
    let (facts, checks) = facts.unwrap_or_default();
    let dir = crash::dir();
    DiagPart {
        facts: DiagFacts {
            app_version: facts.app_version,
            channel: facts.channel,
            install: facts.install,
            os: facts.os,
            arch: facts.arch,
            hardware: facts.hardware,
            extra: facts.extra,
        },
        checks,
        health: dir
            .map(health::days)
            .unwrap_or_default()
            .into_iter()
            .rev()
            .take(diag::MAX_DAYS)
            .rev()
            .map(|d| DiagDay {
                day: d.day,
                counts: d.counts.into_iter().map(|(f, codes)| (f, codes.into_iter().collect())).collect(),
            })
            .collect(),
        crashes: dir
            .map(crash::records)
            .unwrap_or_default()
            .into_iter()
            .take(diag::MAX_CRASHES)
            .map(|c| DiagCrash {
                sig: c.sig,
                kind: c.kind,
                location: c.location,
                message: c.message,
                version: c.version,
                at: c.at,
            })
            .collect(),
        log: DiagLogState::NotAsked,
    }
    .sanitized()
}

/// The user answered `peer`'s request for the log: sends it, or says no.
/// Does nothing when there's no request from `peer`, or it ran out.
pub(crate) async fn answer_log(shared: &Shared, peer: DeviceId, share: bool) -> Result<()> {
    let waiting = shared.diag_asks.lock().waiting.remove(&peer);
    let Some((nonce, just_now, until)) = waiting else { return Ok(()) };
    if until < Instant::now() {
        return Ok(());
    }
    let session = shared.session(&peer).ok_or(Error::Offline)?;
    let lines = if share {
        let files = crate::diag::ring::active_files();
        let now =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        report::log_lines(&files, just_now, now)
    } else {
        Vec::new()
    };
    let log = DiagLog { ask: nonce, declined: !share, lines }.sanitized();
    tracing::info!(peer = %peer.short(), state = if share { "shared" } else { "declined" }, "answered a log request");
    session.send(Envelope::new(types::DIAG_LOG, &log)?).await
}

/// Whether `peer` is waiting for this device's user to answer.
pub(crate) fn is_waiting(shared: &Shared, peer: &DeviceId) -> bool {
    shared.diag_asks.lock().waiting.get(peer).is_some_and(|(_, _, until)| *until > Instant::now())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_from_the_wire_are_cleaned() {
        let part = DiagPart {
            facts: DiagFacts {
                os: "Android 16 at 192.168.1.20".into(),
                hardware: vec!["/storage/emulated/0/Secret Photos/x.jpg".into()],
                ..DiagFacts::default()
            },
            crashes: vec![DiagCrash { location: "C:\\Users\\alice\\x.rs".into(), ..DiagCrash::default() }],
            ..DiagPart::default()
        };
        let out = from_wire(part);
        assert!(!out.facts.os.contains("192.168"), "{}", out.facts.os);
        assert!(!out.facts.hardware[0].contains("Secret"), "{:?}", out.facts.hardware);
        assert!(!out.crashes[0].location.contains("alice"), "{}", out.crashes[0].location);
    }
}
