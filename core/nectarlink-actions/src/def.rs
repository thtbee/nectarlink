// SPDX-License-Identifier: MPL-2.0
//! What an action is: its name, what it may touch, how risky it is and when
//! the user must approve it. Everything the gateway enforces comes from here.

use std::{future::Future, pin::Pin};

use serde_json::Value;

use crate::{error::ActionError, gateway::Ctx};

/// The group an action is listed under, so a client with a tool limit can
/// load only some of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Toolset {
    Core,
    Notifications,
    Messages,
    Calls,
    Clipboard,
    Files,
    Media,
    Pc,
}

impl Toolset {
    pub const ALL: [Toolset; 8] = [
        Toolset::Core,
        Toolset::Notifications,
        Toolset::Messages,
        Toolset::Calls,
        Toolset::Clipboard,
        Toolset::Files,
        Toolset::Media,
        Toolset::Pc,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Toolset::Core => "core",
            Toolset::Notifications => "notifications",
            Toolset::Messages => "messages",
            Toolset::Calls => "calls",
            Toolset::Clipboard => "clipboard",
            Toolset::Files => "files",
            Toolset::Media => "media",
            Toolset::Pc => "pc",
        }
    }

    pub fn parse(s: &str) -> Option<Toolset> {
        Toolset::ALL.into_iter().find(|t| t.as_str() == s)
    }
}

/// A permission area. The names follow the per-device permissions, so a
/// user meets the same words in both places; a scope can only narrow what a
/// device permission already allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Scope {
    Devices,
    Notifications,
    /// One-time codes and notifications the phone marks sensitive.
    NotificationsSensitive,
    Messages,
    Calls,
    Contacts,
    Clipboard,
    Files,
    Media,
    PcActions,
}

impl Scope {
    pub const ALL: [Scope; 10] = [
        Scope::Devices,
        Scope::Notifications,
        Scope::NotificationsSensitive,
        Scope::Messages,
        Scope::Calls,
        Scope::Contacts,
        Scope::Clipboard,
        Scope::Files,
        Scope::Media,
        Scope::PcActions,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Devices => "devices",
            Scope::Notifications => "notifications",
            Scope::NotificationsSensitive => "notifications.sensitive",
            Scope::Messages => "messages",
            Scope::Calls => "calls",
            Scope::Contacts => "contacts",
            Scope::Clipboard => "clipboard",
            Scope::Files => "files",
            Scope::Media => "media",
            Scope::PcActions => "pc_actions",
        }
    }

    pub fn parse(s: &str) -> Option<Scope> {
        Scope::ALL.into_iter().find(|t| t.as_str() == s)
    }

    /// The per-device permission this scope sits under, if any.
    pub fn device_toggle(self) -> Option<&'static str> {
        match self {
            Scope::Devices => None,
            Scope::Notifications | Scope::NotificationsSensitive => Some("notifications"),
            Scope::Messages => Some("messages"),
            Scope::Calls => Some("calls"),
            Scope::Contacts => Some("contacts"),
            Scope::Clipboard => Some("clipboard"),
            Scope::Files => Some("files"),
            Scope::Media => Some("media"),
            Scope::PcActions => Some("pc_actions"),
        }
    }
}

/// Reading leaves everything as it was; writing changes something or reaches
/// someone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Access {
    Read,
    Write,
}

impl Access {
    pub fn as_str(self) -> &'static str {
        match self {
            Access::Read => "read",
            Access::Write => "write",
        }
    }
}

/// A grant: a scope with read or write. Write includes read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Grant {
    pub scope: Scope,
    pub access: Access,
}

impl Grant {
    pub fn new(scope: Scope, access: Access) -> Grant {
        Grant { scope, access }
    }

    /// `messages:write`
    pub fn to_text(self) -> String {
        format!("{}:{}", self.scope.as_str(), self.access.as_str())
    }

    pub fn parse(s: &str) -> Option<Grant> {
        let (scope, access) = s.rsplit_once(':')?;
        let access = match access {
            "read" => Access::Read,
            "write" => Access::Write,
            _ => return None,
        };
        Some(Grant::new(Scope::parse(scope)?, access))
    }
}

/// How much harm a mistaken or hijacked call could do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Risk {
    Low,
    Medium,
    High,
}

/// When the user must approve a call in NectarLink's own window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConfirmRule {
    /// No prompt.
    Never,
    /// The first time a client uses the action; the user may then allow it
    /// for that client from then on.
    FirstUse,
    /// Every call. It can't be remembered or lowered.
    Always,
}

impl ConfirmRule {
    pub fn as_str(self) -> &'static str {
        match self {
            ConfirmRule::Never => "never",
            ConfirmRule::FirstUse => "first_use",
            ConfirmRule::Always => "always",
        }
    }
}

/// Which rate limit bucket a call counts against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RateClass {
    Read,
    Write,
    Sensitive,
}

/// Which device an action works on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// None: it's about this PC or NectarLink itself.
    None,
    /// A paired device, named by the `device` argument.
    Device,
}

pub type HandlerFuture = Pin<Box<dyn Future<Output = Result<Value, ActionError>> + Send>>;
/// Runs an action with checked arguments. Gets the gateway's context and
/// the arguments as JSON (already validated against the input schema).
pub type Handler = fn(Ctx, Value) -> HandlerFuture;

/// One thing an agent can do.
#[derive(Debug, Clone, Copy)]
pub struct ActionDef {
    /// `sms.send`. The MCP tool name is the same with `_` for `.`.
    pub id: &'static str,
    /// For the user, in the confirmation and activity log: "Send a text message".
    pub title: &'static str,
    /// For a model: one or two sentences. Never contains runtime data.
    pub description: &'static str,
    pub toolset: Toolset,
    pub scope: Scope,
    pub access: Access,
    pub risk: Risk,
    pub confirm: ConfirmRule,
    pub rate: RateClass,
    pub target: Target,
    /// The output carries text written by other people or apps.
    pub tainted: bool,
    pub idempotent: bool,
    /// Reaches people or systems outside the user's own devices.
    pub open_world: bool,
    pub input: fn() -> Value,
    pub output: fn() -> Value,
    pub run: Handler,
}

impl ActionDef {
    /// The MCP tool name: `sms.send` becomes `sms_send`.
    pub fn tool_name(&self) -> String {
        self.id.replace('.', "_")
    }

    pub fn grant(&self) -> Grant {
        Grant::new(self.scope, self.access)
    }

    /// MCP `readOnlyHint`.
    pub fn read_only_hint(&self) -> bool {
        self.access == Access::Read
    }

    /// MCP `destructiveHint`.
    pub fn destructive_hint(&self) -> bool {
        self.access == Access::Write && self.risk == Risk::High
    }
}

/// The JSON Schema of `T`, as a plain JSON value.
pub fn schema_of<T: schemars::JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).unwrap_or(Value::Null)
}

impl serde::Serialize for Grant {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_text())
    }
}

impl serde::Serialize for Toolset {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}
