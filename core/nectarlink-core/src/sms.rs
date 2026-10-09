// SPDX-License-Identifier: MPL-2.0
//! Text messages (docs/protocol/sms.md): a PC lists a phone's
//! conversations and their messages, fetches pictures in them, and sends
//! texts through the phone; the phone says when something changed.

use std::sync::Arc;

use nectarlink_protocol::{
    DeviceId, Envelope, ErrorCode, MAX_FRAME_LEN,
    messages::{
        SmsAttachment, SmsChanged, SmsMessage, SmsMessages, SmsMessagesGet, SmsPartData, SmsPartGet, SmsSend,
        SmsThread, SmsThreads, SmsThreadsGet, sms, types,
    },
};

use crate::{Error, Result, events::NodeEvent, node::Shared, session::Session};

/// The device toggle that allows messages for a device.
pub(crate) const TOGGLE: &str = "messages";
/// Room left in a frame for the envelope around a list.
const FRAME_MARGIN: usize = 4 * 1024;

impl Shared {
    fn sms_target(&self, session: &Session) -> bool {
        self.is_desktop(&session.peer)
            && self.toggle_on(&session.peer, TOGGLE)
            && self.store.get_peer(&session.peer).ok().flatten().is_some_and(|p| p.caps.contains(sms::SHOW))
    }

    pub(crate) async fn sms_changed(&self, thread: Option<String>) {
        let Ok(env) = Envelope::new(types::SMS_CHANGED, &SmsChanged { thread }) else { return };
        for session in self.live_sessions().into_iter().filter(|s| self.sms_target(s)) {
            let _ = session.send(env.clone()).await;
        }
    }

    fn offers_locally(&self, capability: &str) -> bool {
        self.local_capabilities().iter().any(|c| c == capability)
    }
}

// ---- A PC asking ----

fn allowed(shared: &Shared, peer: &DeviceId) -> Result<()> {
    if shared.toggle_on(peer, TOGGLE) { Ok(()) } else { Err(Error::Denied) }
}

pub(crate) async fn threads(shared: &Shared, session: &Session, limit: u32) -> Result<Vec<SmsThread>> {
    allowed(shared, &session.peer)?;
    let env = Envelope::new(types::SMS_THREADS, &SmsThreadsGet { limit: limit.min(sms::MAX_PAGE) })?;
    let SmsThreads { threads } =
        session.request(env, crate::session::REQUEST_TIMEOUT).await?.expect_body(types::SMS_THREADS)?;
    Ok(threads)
}

pub(crate) async fn messages(
    shared: &Shared,
    session: &Session,
    thread: String,
    before: Option<i64>,
    limit: u32,
) -> Result<Vec<SmsMessage>> {
    allowed(shared, &session.peer)?;
    let env = Envelope::new(
        types::SMS_MESSAGES,
        &SmsMessagesGet { thread, before, limit: limit.min(sms::MAX_PAGE) },
    )?;
    let SmsMessages { messages } =
        session.request(env, crate::session::REQUEST_TIMEOUT).await?.expect_body(types::SMS_MESSAGES)?;
    Ok(messages)
}

pub(crate) async fn send(
    shared: &Shared,
    session: &Session,
    to: Vec<String>,
    body: String,
    attachments: Vec<SmsAttachment>,
) -> Result<()> {
    allowed(shared, &session.peer)?;
    let send = SmsSend { to, body, attachments };
    if !send.is_valid() {
        let any_oversized = send.attachments.iter().any(|a| a.data.len() > sms::MAX_ATTACHMENT_BYTES);
        return Err(
            if send.body.len() > sms::MAX_SEND_BYTES
                || any_oversized
                || send.attachments.len() > sms::MAX_ATTACHMENTS
            {
                Error::TooLarge
            } else {
                Error::Protocol("invalid text".into())
            },
        );
    }
    let env = Envelope::new(types::SMS_SEND, &send)?;
    session.request(env, crate::session::REQUEST_TIMEOUT).await?.expect(types::OK)?;
    Ok(())
}

pub(crate) async fn part(shared: &Shared, session: &Session, id: String) -> Result<(String, Vec<u8>)> {
    allowed(shared, &session.peer)?;
    let env = Envelope::new(types::SMS_PART, &SmsPartGet { id })?;
    let SmsPartData { mime, data } =
        session.request(env, crate::session::REQUEST_TIMEOUT).await?.expect_body(types::SMS_PART)?;
    Ok((mime, data))
}

// ---- The phone answering ----

/// Handles `sms.*`. Returns false for other types.
pub(crate) async fn handle(shared: &Arc<Shared>, session: &Arc<Session>, env: &Envelope) -> Result<bool> {
    if env.t == types::SMS_CHANGED {
        let SmsChanged { thread } = env.body()?;
        if shared.offers_locally(sms::SHOW) && shared.toggle_on(&session.peer, TOGGLE) {
            shared.emit(NodeEvent::SmsChanged { device: session.peer, thread });
        }
        return Ok(true);
    }
    if ![types::SMS_THREADS, types::SMS_MESSAGES, types::SMS_SEND, types::SMS_PART].contains(&env.t.as_str())
    {
        return Ok(false);
    }
    // Reading the phone's messages takes a moment: not on the session's loop.
    let (shared, session, env) = (shared.clone(), session.clone(), env.clone());
    tokio::spawn(async move {
        let reply = answer(&shared, &session, &env).await.unwrap_or_else(|e| {
            tracing::debug!(error = %e, "bad messages request");
            Envelope::error(ErrorCode::BadMessage, "invalid request")
        });
        let _ = session.send(reply.reply_to(env.id)).await;
    });
    Ok(true)
}

async fn answer(shared: &Arc<Shared>, session: &Session, env: &Envelope) -> Result<Envelope> {
    if !shared.toggle_on(&session.peer, TOGGLE) {
        return Ok(Envelope::error(ErrorCode::Denied, "messages are off for this device"));
    }
    let needs = if env.t == types::SMS_SEND { sms::SEND } else { sms::READ };
    if !shared.offers_locally(needs) {
        return Ok(Envelope::error(ErrorCode::Unsupported, "this phone doesn't share messages"));
    }
    let platform = shared.platform.clone();
    let blocking = |f: Box<dyn FnOnce() -> std::result::Result<Envelope, String> + Send>| async move {
        tokio::task::spawn_blocking(f).await.unwrap_or_else(|e| Err(e.to_string()))
    };
    let result = match env.t.as_str() {
        types::SMS_THREADS => {
            let SmsThreadsGet { limit } = env.body()?;
            blocking(Box::new(move || {
                let mut threads = platform.sms_threads(limit.min(sms::MAX_PAGE))?;
                for t in &mut threads {
                    if t.photo.as_ref().is_some_and(|p| p.len() > sms::MAX_PHOTO_BYTES) {
                        t.photo = None;
                    }
                }
                fit(threads, |threads| Envelope::new(types::SMS_THREADS, &SmsThreads { threads }))
            }))
            .await
        }
        types::SMS_MESSAGES => {
            let SmsMessagesGet { thread, before, limit } = env.body()?;
            blocking(Box::new(move || {
                let messages = platform.sms_messages(&thread, before, limit.min(sms::MAX_PAGE))?;
                fit(messages, |messages| Envelope::new(types::SMS_MESSAGES, &SmsMessages { messages }))
            }))
            .await
        }
        types::SMS_SEND => {
            let send: SmsSend = env.body()?;
            if !send.is_valid() {
                return Ok(Envelope::error(ErrorCode::BadMessage, "invalid text"));
            }
            blocking(Box::new(move || {
                platform
                    .send_sms_with_attachments(&send.to, &send.body, &send.attachments)
                    .map(|()| Envelope::empty(types::OK))
            }))
            .await
        }
        _ => {
            let SmsPartGet { id } = env.body()?;
            blocking(Box::new(move || {
                let (mime, data) = platform.sms_part(&id)?;
                if data.len() > sms::MAX_PART_BYTES {
                    return Ok(Envelope::error(ErrorCode::Busy, "too large"));
                }
                Envelope::new(types::SMS_PART, &SmsPartData { mime, data }).map_err(|e| e.to_string())
            }))
            .await
        }
    };
    Ok(result.unwrap_or_else(|reason| {
        tracing::warn!(reason, t = env.t, "a messages request failed");
        Envelope::error(ErrorCode::Internal, "the phone couldn't do it")
    }))
}

/// The envelope for as many of `items` (newest first) as fit in a frame.
pub(crate) fn fit<T: Clone>(
    mut items: Vec<T>,
    wrap: impl Fn(Vec<T>) -> std::result::Result<Envelope, nectarlink_protocol::ProtocolError>,
) -> std::result::Result<Envelope, String> {
    loop {
        let env = wrap(items.clone()).map_err(|e| e.to_string())?;
        if env.to_cbor().len() + FRAME_MARGIN <= MAX_FRAME_LEN || items.is_empty() {
            return Ok(env);
        }
        // Drop the oldest quarter and try again.
        let keep = items.len() - items.len().div_ceil(4);
        items.truncate(keep);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_lists_are_cut_to_fit_a_frame() {
        let message = |i: usize| SmsMessage {
            id: i.to_string(),
            thread: "1".into(),
            address: "+1555".into(),
            body: "x".repeat(20_000),
            date: i as i64,
            outgoing: false,
            status: None,
            parts: Vec::new(),
        };
        let env = fit((0..100).map(message).collect(), |messages| {
            Envelope::new(types::SMS_MESSAGES, &SmsMessages { messages })
        })
        .unwrap();
        assert!(env.to_cbor().len() <= MAX_FRAME_LEN);
        let SmsMessages { messages } = env.body().unwrap();
        assert!(messages.len() >= 30 && messages.len() < 100, "{}", messages.len());
        assert_eq!(messages[0].id, "0", "the newest stay");
    }
}
