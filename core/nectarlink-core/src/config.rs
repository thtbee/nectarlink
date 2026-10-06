// SPDX-License-Identifier: MPL-2.0
use std::{fmt, path::PathBuf};

use nectarlink_protocol::messages::{DeviceInfo, PowerLevel};

use crate::media::{MediaAction, MediaError};
use crate::{identity::KeyProtector, notifications::NotificationError};

/// Configuration for starting a [`Node`](crate::Node).
#[derive(Clone)]
pub struct NodeConfig {
    /// Directory for the device key, trust store and caches.
    pub data_dir: PathBuf,
    /// This device's description, sent to peers.
    pub device: DeviceInfo,
    /// The app version, e.g. "0.1.0".
    pub app_version: String,
    /// This device's power level (phones) or [`PowerLevel::NotApplicable`].
    pub power: PowerLevel,
    /// Allow connections through relay servers ("Reach my phone away from
    /// home"). Off by default: LAN only.
    pub away_mode: bool,
    /// Discover and announce on the local network via mDNS.
    pub lan_discovery: bool,
    /// Fixed UDP port to listen on (0 = any). A fixed port makes firewall
    /// rules simpler on the desktop.
    pub port: u16,
    /// How the device key is protected at rest. `None` uses the platform
    /// default (DPAPI on Windows).
    pub key_protector: Option<std::sync::Arc<dyn KeyProtector>>,
    /// Where received files go once complete. `None`: a `received` folder
    /// in `data_dir`.
    pub downloads_dir: Option<PathBuf>,
    /// Capabilities this device offers beyond the ones every build does
    /// (docs/protocol/capabilities.md), e.g. `media.control`.
    pub capabilities: Vec<String>,
}

impl NodeConfig {
    pub fn new(data_dir: impl Into<PathBuf>, device: DeviceInfo, app_version: impl Into<String>) -> Self {
        NodeConfig {
            data_dir: data_dir.into(),
            device,
            app_version: app_version.into(),
            power: PowerLevel::NotApplicable,
            away_mode: false,
            lan_discovery: true,
            port: 0,
            key_protector: None,
            downloads_dir: None,
            capabilities: Vec::new(),
        }
    }
}

impl fmt::Debug for NodeConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeConfig")
            .field("data_dir", &self.data_dir)
            .field("device", &self.device)
            .field("app_version", &self.app_version)
            .field("power", &self.power)
            .field("away_mode", &self.away_mode)
            .field("lan_discovery", &self.lan_discovery)
            .field("port", &self.port)
            .field("downloads_dir", &self.downloads_dir)
            .finish_non_exhaustive()
    }
}

/// Platform services the app provides to the core.
///
/// Implemented by each app (Windows, Android). Calls may come from any
/// thread and must not block for long.
pub trait Platform: Send + Sync + 'static {
    /// Start ringing loudly (find my device), even on silent.
    fn start_ringing(&self) {}
    /// Stop ringing.
    fn stop_ringing(&self) {}

    /// Dismiss one of this device's notifications (a PC dismissed it).
    fn dismiss_notification(&self, _key: &str) -> Result<(), NotificationError> {
        Err(NotificationError::Unsupported)
    }
    /// Run an action of one of this device's notifications; `reply` holds
    /// the text for a reply action.
    fn run_notification_action(
        &self,
        _key: &str,
        _action: &str,
        _reply: Option<&str>,
    ) -> Result<(), NotificationError> {
        Err(NotificationError::Unsupported)
    }

    /// Put text on this device's clipboard (a paired device sent it).
    /// `Err` holds a reason for logs, never the text.
    fn set_clipboard(&self, _text: &str) -> Result<(), String> {
        Err("this device has no clipboard".into())
    }

    /// Put an image on this device's clipboard (a paired device sent it).
    /// `mime` is `image/png` or `image/jpeg`.
    fn set_clipboard_image(&self, _mime: &str, _bytes: &[u8]) -> Result<(), String> {
        Err("this device has no clipboard".into())
    }

    /// Run a command on one of this device's media players (a paired device
    /// asked); `position` is set for [`MediaAction::Seek`].
    fn media_command(
        &self,
        _player: &str,
        _action: MediaAction,
        _position: Option<u64>,
    ) -> Result<(), MediaError> {
        Err(MediaError::Unsupported)
    }
}

/// A platform that does nothing; useful for tests and headless tools.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopPlatform;

impl Platform for NoopPlatform {}
