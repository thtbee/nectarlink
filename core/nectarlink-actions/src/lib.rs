// SPDX-License-Identifier: MPL-2.0
//! Agent access to NectarLink: one list of actions, and one gateway every
//! agent request passes through, whichever way it arrives (MCP, the CLI or
//! the local HTTP API).
//!
//! The gateway checks, in order: agent access is on, the token, the action,
//! the client's permissions, read-only, the device and its own permission,
//! the rate limit and the user's confirmation; then it runs the action and
//! logs it. See `docs/agents/README.md`.

pub mod def;
pub mod error;
pub mod gateway;
pub mod handlers;
pub mod host;
pub mod paths;
pub mod ratelimit;
pub mod registry;
pub mod store;
pub mod untrusted;

pub use def::{Access, ActionDef, ConfirmRule, Grant, Risk, Scope, Toolset};
pub use error::{ActionError, Code};
pub use gateway::{Caller, Ctx, Gateway, Switches};
pub use host::{ActionHost, ConfirmRequest, Decision};
pub use store::{Activity, Client, Store};

/// Grants for the "Read only" preset: every read, except the clipboard and
/// one-time codes, which only a custom choice gives.
pub fn preset_read_only() -> std::collections::BTreeSet<Grant> {
    Scope::ALL
        .into_iter()
        .filter(|s| !matches!(s, Scope::Clipboard | Scope::NotificationsSensitive))
        .map(|s| Grant::new(s, Access::Read))
        .collect()
}

/// Grants for the "Everyday" preset: reads, plus writing notifications,
/// messages, files, media and devices. Still no clipboard or codes.
pub fn preset_everyday() -> std::collections::BTreeSet<Grant> {
    let mut grants = preset_read_only();
    for s in [Scope::Devices, Scope::Notifications, Scope::Messages, Scope::Files, Scope::Media] {
        grants.insert(Grant::new(s, Access::Write));
    }
    grants
}
