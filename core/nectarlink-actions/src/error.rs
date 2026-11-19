// SPDX-License-Identifier: MPL-2.0
//! The one error shape every channel returns: a stable code an agent can
//! act on, a plain sentence, and what the user would have to do.

use serde::Serialize;

/// Stable error codes. Agents and scripts match on these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Code {
    AgentAccessOff,
    Unauthenticated,
    ClientPaused,
    UnknownAction,
    ScopeDenied,
    ReadOnly,
    DeviceNotAllowed,
    DeviceAmbiguous,
    DeviceNotFound,
    DeviceOffline,
    Denied,
    Unsupported,
    RateLimited,
    InvalidArguments,
    ConfirmDenied,
    ConfirmationPending,
    ConfirmTimeout,
    NotFound,
    Timeout,
    AuditUnavailable,
    AppNotReady,
    Internal,
}

impl Code {
    /// Whether trying again later could work.
    pub fn retryable(self) -> bool {
        matches!(
            self,
            Code::DeviceOffline
                | Code::RateLimited
                | Code::Timeout
                | Code::ConfirmationPending
                | Code::AppNotReady
        )
    }

    /// The name as it appears on the wire (`SCOPE_DENIED`).
    pub fn as_str(self) -> String {
        serde_json::to_value(self).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
    }
}

/// An action that didn't run, or didn't finish.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize)]
#[error("{code:?}: {message}")]
pub struct ActionError {
    pub code: Code,
    pub message: String,
    pub retryable: bool,
    /// What the user can do about it, for the agent to pass on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_action: Option<String>,
    /// Extra fields some codes carry (`retry_after_ms`, `confirmation_id`, `devices`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl ActionError {
    pub fn new(code: Code, message: impl Into<String>) -> ActionError {
        ActionError {
            code,
            message: message.into(),
            retryable: code.retryable(),
            user_action: None,
            details: None,
        }
    }

    pub fn user_action(mut self, what: impl Into<String>) -> ActionError {
        self.user_action = Some(what.into());
        self
    }

    pub fn details(mut self, details: serde_json::Value) -> ActionError {
        self.details = Some(details);
        self
    }

    pub fn invalid(message: impl Into<String>) -> ActionError {
        ActionError::new(Code::InvalidArguments, message)
    }

    pub fn internal(message: impl Into<String>) -> ActionError {
        ActionError::new(Code::Internal, message)
    }

    /// The JSON every channel sends: `{ "ok": false, "error": { … } }`.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({ "ok": false, "error": self })
    }
}

impl From<nectarlink_core::Error> for ActionError {
    fn from(e: nectarlink_core::Error) -> ActionError {
        use nectarlink_core::Error as E;
        match e {
            E::NotPaired => ActionError::new(Code::DeviceNotFound, "That device isn't paired."),
            E::Offline => ActionError::new(Code::DeviceOffline, "The device isn't connected right now.")
                .user_action("Make sure the phone is on, nearby or on the same network, and NectarLink is running on it."),
            E::Denied => ActionError::new(Code::Denied, "The device refused this. It may be turned off for this PC.")
                .user_action("Check the permissions for this device in NectarLink on both devices."),
            E::Declined => ActionError::new(Code::Denied, "Someone declined it on the device."),
            E::Timeout => ActionError::new(Code::Timeout, "The device didn't answer in time."),
            E::VersionTooOld { .. } => ActionError::new(Code::Unsupported, "One of the NectarLink apps is too old for this.")
                .user_action("Update NectarLink on the PC and the phone."),
            E::Unsupported => ActionError::new(Code::Unsupported, "The device doesn't support this."),
            E::NotFound => ActionError::new(Code::NotFound, "It no longer exists."),
            E::TooLarge => ActionError::invalid("It's too large to send."),
            E::QueueFull(m) => ActionError::new(Code::RateLimited, m),
            other => ActionError::internal(other.to_string()),
        }
    }
}
