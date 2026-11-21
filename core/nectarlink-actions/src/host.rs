// SPDX-License-Identifier: MPL-2.0
//! What the app hosting the gateway supplies: the confirmation window, and
//! the things only it knows (this PC's clipboard, its lock, the live lists
//! it keeps). The desktop app implements this; tests use a fake.

use std::{future::Future, path::PathBuf, pin::Pin};

use nectarlink_core::{DeviceId, PowerAction};
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

    /// Where files the agent asks for (photos, phone files) are saved on this PC.
    fn save_dir(&self) -> Option<PathBuf> {
        None
    }

    /// An agent called: its activity row and "last used" are saved, so the
    /// app can show them.
    fn agent_called(&self) {}

    /// Locks, sleeps, hibernates or shuts down this PC.
    fn pc_power(&self, _action: PowerAction) -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }

    /// Opens the phone's screen (`app` None) or one of its apps, by package
    /// and label, in a window on this PC.
    fn mirror_open(&self, _device: DeviceId, _app: Option<(String, String)>) -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }

    /// The phone screens and app windows open on this PC, as JSON.
    fn mirror_windows(&self) -> Vec<serde_json::Value> {
        Vec::new()
    }

    /// Closes the phone's screen (`app` None) or an app window. False when
    /// none was open.
    fn mirror_close(&self, _device: DeviceId, _app: Option<&str>) -> Result<bool, String> {
        Err(UNSUPPORTED.into())
    }

    /// This PC's Deck tiles, as JSON.
    fn deck_tiles(&self) -> Vec<serde_json::Value> {
        Vec::new()
    }

    /// Runs one of this PC's Deck tiles, as pressing it on the phone would.
    fn deck_run(&self, _tile: &str) -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }
}

/// What a host answers for something it can't do on this system.
pub const UNSUPPORTED: &str = "This PC can't do that.";
