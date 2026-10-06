// SPDX-License-Identifier: MPL-2.0
//! Calls (docs/protocol/calls.md): a phone tells its PCs when a call rings,
//! is answered and ends; a PC can answer, decline or silence it. The call's
//! audio stays on the phone.

use std::sync::{Arc, Mutex};

use nectarlink_protocol::{
    Envelope, ErrorCode,
    messages::{CallAction, CallState, calls, types},
};

use crate::{Error, Result, events::NodeEvent, node::Shared, session::Session};

/// The device toggle that allows calls for a device.
pub(crate) const TOGGLE: &str = "calls";

/// What a PC can ask a phone to do with a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallCommand {
    Answer,
    /// Declines a ringing call, or hangs up an active one.
    Decline,
    /// Stops the ringing; the call still rings on for the caller.
    Silence,
}

impl CallCommand {
    pub fn as_str(self) -> &'static str {
        match self {
            CallCommand::Answer => calls::ANSWER,
            CallCommand::Decline => calls::DECLINE,
            CallCommand::Silence => calls::SILENCE,
        }
    }

    pub fn parse(action: &str) -> Option<CallCommand> {
        match action {
            calls::ANSWER => Some(CallCommand::Answer),
            calls::DECLINE => Some(CallCommand::Decline),
            calls::SILENCE => Some(CallCommand::Silence),
            _ => None,
        }
    }
}

/// The phone's call in progress (ringing or active), for PCs that connect
/// while it's going on.
#[derive(Debug, Default)]
pub(crate) struct Current(Mutex<Option<CallState>>);

impl Current {
    fn get(&self) -> Option<CallState> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn set(&self, call: &CallState) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) =
            (call.state != calls::ENDED).then(|| call.clone());
    }
}

impl Shared {
    /// Whether this PC takes calls from `session`'s phone.
    fn call_target(&self, session: &Session) -> bool {
        self.is_desktop(&session.peer)
            && self.toggle_on(&session.peer, TOGGLE)
            && self.store.get_peer(&session.peer).ok().flatten().is_some_and(|p| p.caps.contains(calls::SHOW))
    }

    pub(crate) async fn call_changed(&self, call: CallState) -> Result<()> {
        if !call.is_valid() {
            return Err(if call.photo.as_ref().is_some_and(|p| p.len() > calls::MAX_PHOTO_BYTES) {
                Error::TooLarge
            } else {
                Error::Protocol("invalid call".into())
            });
        }
        self.calls.set(&call);
        let env = Envelope::new(types::CALL_STATE, &call)?;
        for session in self.live_sessions().into_iter().filter(|s| self.call_target(s)) {
            let _ = session.send(env.clone()).await;
        }
        Ok(())
    }

    /// Tells a PC that just connected about the call going on, if any.
    pub(crate) async fn send_call_state(&self, session: &Arc<Session>) {
        let Some(call) = self.calls.get() else { return };
        if self.call_target(session)
            && let Ok(env) = Envelope::new(types::CALL_STATE, &call)
        {
            let _ = session.send(env).await;
        }
    }
}

pub(crate) async fn command(
    shared: &Shared,
    session: &Session,
    id: String,
    command: CallCommand,
) -> Result<()> {
    if !shared.toggle_on(&session.peer, TOGGLE) {
        return Err(Error::Denied);
    }
    let env = Envelope::new(types::CALL_ACTION, &CallAction { id, action: command.as_str().into() })?;
    session.request(env, crate::session::REQUEST_TIMEOUT).await?.expect(types::OK)?;
    Ok(())
}

/// Handles `call.*`. Returns false for other types.
pub(crate) async fn handle(shared: &Arc<Shared>, session: &Arc<Session>, env: &Envelope) -> Result<bool> {
    let peer = session.peer;
    match env.t.as_str() {
        types::CALL_STATE => {
            let call: CallState = env.body()?;
            let shows = shared.local_capabilities().iter().any(|c| c == calls::SHOW);
            if shows && call.is_valid() && shared.toggle_on(&peer, TOGGLE) {
                shared.emit(NodeEvent::Call { device: peer, call });
            }
            Ok(true)
        }
        types::CALL_ACTION => {
            let CallAction { id, action } = env.body()?;
            let current = shared.calls.get();
            let reply = if !shared.toggle_on(&peer, TOGGLE) {
                Envelope::error(ErrorCode::Denied, "calls are off for this device")
            } else if !shared.local_capabilities().iter().any(|c| c == calls::CONTROL) {
                Envelope::error(ErrorCode::Unsupported, "this phone doesn't take call actions")
            } else {
                match (CallCommand::parse(&action), current) {
                    (None, _) => Envelope::error(ErrorCode::Unsupported, "unknown action"),
                    (Some(command), Some(call))
                        if call.id == id
                            && (call.state == calls::RINGING || command == CallCommand::Decline) =>
                    {
                        let platform = shared.platform.clone();
                        match tokio::task::spawn_blocking(move || platform.call_command(&id, command))
                            .await
                            .unwrap_or_else(|e| Err(e.to_string()))
                        {
                            Ok(()) => Envelope::empty(types::OK),
                            Err(reason) => {
                                tracing::warn!(reason, action = command.as_str(), "a call action failed");
                                Envelope::error(ErrorCode::Internal, "the phone couldn't do it")
                            }
                        }
                    }
                    _ => Envelope::error(ErrorCode::NotFound, "no such call"),
                }
            };
            session.send(reply.reply_to(env.id)).await?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_round_trip() {
        for c in [CallCommand::Answer, CallCommand::Decline, CallCommand::Silence] {
            assert_eq!(CallCommand::parse(c.as_str()), Some(c));
        }
        assert_eq!(CallCommand::parse("hold"), None);
    }

    #[test]
    fn only_calls_in_progress_are_kept() {
        let current = Current::default();
        let call = CallState {
            id: "7".into(),
            state: calls::RINGING.into(),
            incoming: true,
            number: None,
            name: None,
            photo: None,
            missed: false,
        };
        current.set(&call);
        assert_eq!(current.get().map(|c| c.id), Some("7".into()));
        current.set(&CallState { state: calls::ENDED.into(), ..call });
        assert!(current.get().is_none());
    }
}
