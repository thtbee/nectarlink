// SPDX-License-Identifier: MPL-2.0
//! Calls (docs/protocol/calls.md): a phone tells its PCs when a call rings,
//! is answered and ends; a PC can answer, decline or silence it. The call's
//! audio stays on the phone.

use std::sync::{Arc, Mutex};

use nectarlink_protocol::{
    Envelope, ErrorCode,
    messages::{
        CallAction, CallDial, CallLog, CallLogChanged, CallLogEntry, CallLogGet, CallState, calls,
        is_dtmf_digit, types,
    },
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
    /// Mutes (true) or unmutes the phone's microphone on the call.
    Mute(bool),
    /// The speaker (true) or the earpiece.
    Speaker(bool),
    /// Puts the call on hold (true) or takes it off.
    Hold(bool),
    /// A keypad tone: `0`–`9`, `*` or `#`.
    Dtmf(char),
    /// The call's volume up (true) or down.
    Volume(bool),
}

impl CallCommand {
    pub fn as_str(self) -> &'static str {
        match self {
            CallCommand::Answer => calls::ANSWER,
            CallCommand::Decline => calls::DECLINE,
            CallCommand::Silence => calls::SILENCE,
            CallCommand::Mute(true) => calls::MUTE,
            CallCommand::Mute(false) => calls::UNMUTE,
            CallCommand::Speaker(true) => calls::SPEAKER,
            CallCommand::Speaker(false) => calls::EARPIECE,
            CallCommand::Hold(true) => calls::HOLD,
            CallCommand::Hold(false) => calls::UNHOLD,
            CallCommand::Dtmf(_) => calls::DTMF,
            CallCommand::Volume(true) => calls::VOLUME_UP,
            CallCommand::Volume(false) => calls::VOLUME_DOWN,
        }
    }

    /// The command for an action (and, for `dtmf`, its digit).
    pub fn parse(action: &str, digit: Option<&str>) -> Option<CallCommand> {
        Some(match action {
            calls::ANSWER => CallCommand::Answer,
            calls::DECLINE => CallCommand::Decline,
            calls::SILENCE => CallCommand::Silence,
            calls::MUTE => CallCommand::Mute(true),
            calls::UNMUTE => CallCommand::Mute(false),
            calls::SPEAKER => CallCommand::Speaker(true),
            calls::EARPIECE => CallCommand::Speaker(false),
            calls::HOLD => CallCommand::Hold(true),
            calls::UNHOLD => CallCommand::Hold(false),
            calls::DTMF => CallCommand::Dtmf(digit.filter(|d| is_dtmf_digit(d))?.chars().next()?),
            calls::VOLUME_UP => CallCommand::Volume(true),
            calls::VOLUME_DOWN => CallCommand::Volume(false),
            _ => return None,
        })
    }

    /// The `digit` field it goes with.
    fn digit(self) -> Option<String> {
        match self {
            CallCommand::Dtmf(c) => Some(c.to_string()),
            _ => None,
        }
    }

    /// Whether it applies to a call in `state` ("ringing" or "active"),
    /// and whether it needs the phone to control the call (`call.incall`).
    fn fits(self, state: &str) -> bool {
        match self {
            CallCommand::Answer | CallCommand::Silence => state == calls::RINGING,
            CallCommand::Decline => true,
            _ => state == calls::ACTIVE,
        }
    }

    fn needs_in_call(self) -> bool {
        matches!(
            self,
            CallCommand::Mute(_) | CallCommand::Speaker(_) | CallCommand::Hold(_) | CallCommand::Dtmf(_)
        )
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

    pub(crate) async fn call_log_changed(&self) {
        let Ok(env) = Envelope::new(types::CALL_LOG_CHANGED, &CallLogChanged {}) else { return };
        for session in self.live_sessions().into_iter().filter(|s| self.call_target(s)) {
            let _ = session.send(env.clone()).await;
        }
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
    let action = CallAction { id, action: command.as_str().into(), digit: command.digit() };
    let env = Envelope::new(types::CALL_ACTION, &action)?;
    session.request(env, crate::session::REQUEST_TIMEOUT).await?.expect(types::OK)?;
    Ok(())
}

pub(crate) async fn log(
    shared: &Shared,
    session: &Session,
    before: Option<i64>,
    limit: u32,
) -> Result<Vec<CallLogEntry>> {
    if !shared.toggle_on(&session.peer, TOGGLE) {
        return Err(Error::Denied);
    }
    let env =
        Envelope::new(types::CALL_LOG, &CallLogGet { before, limit: limit.clamp(1, calls::MAX_LOG_PAGE) })?;
    let CallLog { entries } =
        session.request(env, crate::session::REQUEST_TIMEOUT).await?.expect_body(types::CALL_LOG)?;
    Ok(entries)
}

pub(crate) async fn dial(shared: &Shared, session: &Session, number: String) -> Result<()> {
    if !shared.toggle_on(&session.peer, TOGGLE) {
        return Err(Error::Denied);
    }
    let dial = CallDial { number: number.trim().to_owned() };
    if !dial.is_valid() {
        return Err(if dial.number.len() > calls::MAX_TEXT_BYTES {
            Error::TooLarge
        } else {
            Error::Protocol("invalid phone number".into())
        });
    }
    let env = Envelope::new(types::CALL_DIAL, &dial)?;
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
        types::CALL_LOG_CHANGED => {
            let CallLogChanged {} = env.body()?;
            let shows = shared.local_capabilities().iter().any(|c| c == calls::SHOW);
            if shows && shared.toggle_on(&peer, TOGGLE) {
                shared.emit(NodeEvent::CallLogChanged { device: peer });
            }
            Ok(true)
        }
        types::CALL_ACTION => {
            let CallAction { id, action, digit } = env.body()?;
            let current = shared.calls.get();
            let offers = |cap: &str| shared.local_capabilities().iter().any(|c| c == cap);
            let command = CallCommand::parse(&action, digit.as_deref());
            let reply = if !shared.toggle_on(&peer, TOGGLE) {
                Envelope::error(ErrorCode::Denied, "calls are off for this device")
            } else if !offers(calls::CONTROL)
                || command.is_some_and(|c| c.needs_in_call() && !offers(calls::IN_CALL))
            {
                Envelope::error(ErrorCode::Unsupported, "this phone doesn't take that call action")
            } else {
                match (command, current) {
                    (None, _) => Envelope::error(ErrorCode::Unsupported, "unknown action"),
                    (Some(command), Some(call)) if call.id == id && command.fits(&call.state) => {
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
        types::CALL_LOG | types::CALL_DIAL => {
            // Reading the call log or starting a call takes a moment: run off the session's loop,
            // while keeping `call.state` and `call.action` in order on the stream.
            let (shared, session, env) = (shared.clone(), session.clone(), env.clone());
            tokio::spawn(async move {
                let reply = answer_log_or_dial(&shared, &session, &env).await.unwrap_or_else(|e| {
                    tracing::debug!(error = %e, "bad call request");
                    Envelope::error(ErrorCode::BadMessage, "invalid request")
                });
                let _ = session.send(reply.reply_to(env.id)).await;
            });
            Ok(true)
        }
        _ => Ok(false),
    }
}

async fn answer_log_or_dial(shared: &Arc<Shared>, session: &Session, env: &Envelope) -> Result<Envelope> {
    if !shared.toggle_on(&session.peer, TOGGLE) {
        return Ok(Envelope::error(ErrorCode::Denied, "calls are off for this device"));
    }
    let offers = |cap: &str| shared.local_capabilities().iter().any(|c| c == cap);
    let platform = shared.platform.clone();
    let result = match env.t.as_str() {
        types::CALL_LOG => {
            if !offers(calls::LOG) {
                return Ok(Envelope::error(ErrorCode::Unsupported, "this phone doesn't share recent calls"));
            }
            let CallLogGet { before, limit } = env.body()?;
            let limit = limit.clamp(1, calls::MAX_LOG_PAGE);
            tokio::task::spawn_blocking(move || {
                let mut entries = platform.call_log(before, limit)?;
                for e in &mut entries {
                    if e.photo.as_ref().is_some_and(|p| p.len() > calls::MAX_LOG_PHOTO_BYTES) {
                        e.photo = None;
                    }
                }
                crate::sms::fit(entries, |entries| Envelope::new(types::CALL_LOG, &CallLog { entries }))
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()))
        }
        _ => {
            if !offers(calls::DIAL) {
                return Ok(Envelope::error(ErrorCode::Unsupported, "this phone doesn't place calls"));
            }
            let dial: CallDial = env.body()?;
            if !dial.is_valid() {
                return Ok(Envelope::error(ErrorCode::BadMessage, "invalid number"));
            }
            let number = dial.number.trim().to_owned();
            tokio::task::spawn_blocking(move || {
                platform.call_dial(&number).map(|()| Envelope::empty(types::OK))
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()))
        }
    };
    Ok(result.unwrap_or_else(|reason| {
        tracing::warn!(reason, t = env.t, "a call request failed");
        Envelope::error(ErrorCode::Internal, "the phone couldn't do it")
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_round_trip() {
        let all = [
            CallCommand::Answer,
            CallCommand::Decline,
            CallCommand::Silence,
            CallCommand::Mute(true),
            CallCommand::Mute(false),
            CallCommand::Speaker(true),
            CallCommand::Speaker(false),
            CallCommand::Hold(true),
            CallCommand::Hold(false),
            CallCommand::Dtmf('#'),
            CallCommand::Volume(true),
            CallCommand::Volume(false),
        ];
        for c in all {
            assert_eq!(CallCommand::parse(c.as_str(), c.digit().as_deref()), Some(c));
        }
        assert_eq!(CallCommand::parse("transfer", None), None);
        assert_eq!(CallCommand::parse("dtmf", Some("12")), None);
        assert_eq!(CallCommand::parse("dtmf", Some("x")), None);
        assert_eq!(CallCommand::parse("dtmf", None), None);
    }

    #[test]
    fn commands_fit_the_call() {
        assert!(CallCommand::Answer.fits("ringing") && !CallCommand::Answer.fits("active"));
        assert!(CallCommand::Mute(true).fits("active") && !CallCommand::Mute(true).fits("ringing"));
        assert!(CallCommand::Decline.fits("ringing") && CallCommand::Decline.fits("active"));
        assert!(CallCommand::Volume(true).fits("active") && !CallCommand::Volume(true).needs_in_call());
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
            since: None,
            controls: None,
        };
        current.set(&call);
        assert_eq!(current.get().map(|c| c.id), Some("7".into()));
        current.set(&CallState { state: calls::ENDED.into(), ..call });
        assert!(current.get().is_none());
    }
}
