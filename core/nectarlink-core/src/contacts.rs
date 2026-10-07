// SPDX-License-Identifier: MPL-2.0
//! Contacts (docs/protocol/contacts.md): a PC lists and searches a phone's
//! contacts (favorites first, then alphabetical) with their phone numbers and
//! small photos; the phone says when its contacts change.

use std::sync::Arc;

use nectarlink_protocol::{
    DeviceId, Envelope, ErrorCode,
    messages::{Contact, ContactsChanged, ContactsList, ContactsListGet, contacts, types},
};

use crate::{Error, Result, events::NodeEvent, node::Shared, session::Session};

/// The device toggle that allows contacts for a device.
pub(crate) const TOGGLE: &str = "contacts";

impl Shared {
    fn contacts_target(&self, session: &Session) -> bool {
        self.is_desktop(&session.peer)
            && self.toggle_on(&session.peer, TOGGLE)
            && self
                .store
                .get_peer(&session.peer)
                .ok()
                .flatten()
                .is_some_and(|p| p.caps.contains(contacts::SHOW))
    }

    pub(crate) async fn contacts_changed(&self) {
        let Ok(env) = Envelope::new(types::CONTACTS_CHANGED, &ContactsChanged {}) else { return };
        for session in self.live_sessions().into_iter().filter(|s| self.contacts_target(s)) {
            let _ = session.send(env.clone()).await;
        }
    }
}

// ---- A PC asking ----

fn allowed(shared: &Shared, peer: &DeviceId) -> Result<()> {
    if shared.toggle_on(peer, TOGGLE) { Ok(()) } else { Err(Error::Denied) }
}

pub(crate) async fn list(
    shared: &Shared,
    session: &Session,
    query: Option<String>,
    offset: u32,
    limit: u32,
) -> Result<Vec<Contact>> {
    allowed(shared, &session.peer)?;
    let query = query.map(|q| q.trim().to_owned()).filter(|q| !q.is_empty());
    if query.as_ref().is_some_and(|q| q.len() > contacts::MAX_QUERY_BYTES) {
        return Err(Error::TooLarge);
    }
    let env = Envelope::new(
        types::CONTACTS_LIST,
        &ContactsListGet { query, offset, limit: limit.clamp(1, contacts::MAX_PAGE) },
    )?;
    let ContactsList { contacts } =
        session.request(env, crate::session::REQUEST_TIMEOUT).await?.expect_body(types::CONTACTS_LIST)?;
    Ok(contacts)
}

// ---- The phone answering ----

/// Handles `contacts.*`. Returns false for other types.
pub(crate) async fn handle(shared: &Arc<Shared>, session: &Arc<Session>, env: &Envelope) -> Result<bool> {
    if env.t == types::CONTACTS_CHANGED {
        let ContactsChanged {} = env.body()?;
        let shows = shared.local_capabilities().iter().any(|c| c == contacts::SHOW);
        if shows && shared.toggle_on(&session.peer, TOGGLE) {
            shared.emit(NodeEvent::ContactsChanged { device: session.peer });
        }
        return Ok(true);
    }
    if env.t != types::CONTACTS_LIST {
        return Ok(false);
    }
    // Querying contacts on the phone takes a moment: not on the session's loop.
    let (shared, session, env) = (shared.clone(), session.clone(), env.clone());
    tokio::spawn(async move {
        let reply = answer(&shared, &session, &env).await.unwrap_or_else(|e| {
            tracing::debug!(error = %e, "bad contacts request");
            Envelope::error(ErrorCode::BadMessage, "invalid request")
        });
        let _ = session.send(reply.reply_to(env.id)).await;
    });
    Ok(true)
}

async fn answer(shared: &Arc<Shared>, session: &Session, env: &Envelope) -> Result<Envelope> {
    if !shared.toggle_on(&session.peer, TOGGLE) {
        return Ok(Envelope::error(ErrorCode::Denied, "contacts are off for this device"));
    }
    if !shared.local_capabilities().iter().any(|c| c == contacts::READ) {
        return Ok(Envelope::error(ErrorCode::Unsupported, "this phone doesn't share contacts"));
    }
    let ContactsListGet { query, offset, limit } = env.body()?;
    if query.as_ref().is_some_and(|q| q.len() > contacts::MAX_QUERY_BYTES) {
        return Ok(Envelope::error(ErrorCode::BadMessage, "query too long"));
    }
    let query = query.map(|q| q.trim().to_owned()).filter(|q| !q.is_empty());
    let limit = limit.clamp(1, contacts::MAX_PAGE);
    let platform = shared.platform.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut items = platform.contacts(query.as_deref(), offset, limit)?;
        for c in &mut items {
            c.numbers.truncate(contacts::MAX_NUMBERS);
            if c.photo.as_ref().is_some_and(|p| p.len() > contacts::MAX_PHOTO_BYTES) {
                c.photo = None;
            }
        }
        crate::sms::fit(items, |contacts| Envelope::new(types::CONTACTS_LIST, &ContactsList { contacts }))
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));
    Ok(result.unwrap_or_else(|reason| {
        tracing::warn!(reason, "a contacts request failed");
        Envelope::error(ErrorCode::Internal, "the phone couldn't read contacts")
    }))
}
