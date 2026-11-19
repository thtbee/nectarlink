// SPDX-License-Identifier: MPL-2.0
//! What the app hosting the gateway supplies: the confirmation window, and
//! the things only it knows (this PC's clipboard, its lock, the live lists
//! it keeps). The desktop app implements this; tests use a fake.

use std::{future::Future, pin::Pin};

use nectarlink_core::DeviceId;
use nectarlink_protocol::messages::{MediaPlayer, Notification};
use serde::Serialize;

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// What the user is asked to approve. Built by the gateway from the
/// checked arguments, so the window shows what will really happen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfirmRequest {
    pub id: String,
    /// The agent, as the user named it.
    pub client: String,
    pub action: String,
    /// "Send a text message"
    pub title: String,
    /// The device it acts on, by name.
    pub device: Option<String>,
    /// Label and value pairs, shown in full.
    pub details: Vec<(String, String)>,
    /// Whether "Always allow for this agent" may be offered.
    pub can_remember: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    /// Allow, and don't ask this client about this action again.
    AllowAlways,
    Deny,
    /// Nobody answered before the request expired.
    Expired,
}

/// The app's side of the gateway.
pub trait ActionHost: Send + Sync + std::fmt::Debug {
    /// Shows the request in NectarLink's own window and resolves with the
    /// user's answer, or `Expired` after `expires_s`.
    fn confirm(&self, request: ConfirmRequest, expires_s: u64) -> BoxFuture<Decision>;

    /// The notifications currently on each paired device, as the app shows them.
    fn notifications(&self) -> Vec<(DeviceId, Notification)>;

    /// Media players on paired devices and on this PC (`None` as the device).
    fn media_players(&self) -> Vec<(Option<DeviceId>, MediaPlayer)>;

    /// The text on this PC's clipboard.
    fn pc_clipboard_read(&self) -> Result<String, String>;

    /// Puts text on this PC's clipboard.
    fn pc_clipboard_write(&self, text: &str) -> Result<(), String>;

    /// Locks this PC.
    fn pc_lock(&self) -> Result<(), String>;

    /// The transfers the app shows (running and recent), as JSON objects.
    fn transfers(&self) -> Vec<serde_json::Value>;

    /// The current call on each device, as JSON, if there is one.
    fn calls(&self) -> Vec<(DeviceId, serde_json::Value)>;
}
