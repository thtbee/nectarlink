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

    /// Lock or sleep this PC (a paired phone asked). Runs after the phone
    /// got its answer. `Err` holds a reason for logs.
    fn power(&self, _action: crate::actions::PowerAction) -> Result<(), String> {
        Err("this device doesn't lock or sleep on request".into())
    }

    /// Open a web link a paired device sent (http or https; checked).
    fn open_link(&self, _from: &nectarlink_protocol::DeviceId, _url: &str) -> Result<(), String> {
        Err("this device doesn't open links".into())
    }

    /// List photo and video albums on this phone (`photos.albums`).
    fn photo_albums(&self) -> Result<Vec<crate::PhotoAlbum>, String> {
        Err("this device doesn't share photos".into())
    }

    /// List photos and videos on this phone (or in `album`), newest first by
    /// date and then by a stable order of its own (`photos.list`). `before`
    /// is the previous page's last item (date and ID): list only what comes
    /// after it in that order; from the latest when `None`.
    fn photo_list(
        &self,
        _album: Option<&str>,
        _before: Option<(i64, &str)>,
        _limit: u32,
    ) -> Result<Vec<crate::PhotoItem>, String> {
        Err("this device doesn't share photos".into())
    }

    /// Small JPEG thumbnails for the requested item IDs (`photos.thumbs`).
    /// Items whose thumbnail can't be generated are omitted rather than
    /// failing the batch.
    fn photo_thumbs(&self, _ids: &[String]) -> Result<Vec<crate::PhotoThumb>, String> {
        Err("this device doesn't share photos".into())
    }

    /// Open a photo or video on this phone for sending it to a PC that asked
    /// (`photos.get`). `Err` (a reason for logs) when it's gone.
    fn open_photo(&self, _id: &str) -> Result<crate::OutgoingFile, String> {
        Err("this device doesn't share photos".into())
    }

    /// Where a phone's screen (`session` 0, with its sound) or app window
    /// goes on this PC, when it starts streaming (after
    /// [`Node::mirror_start`](crate::Node::mirror_start)). `None` refuses
    /// the stream.
    fn mirror_sink(
        &self,
        _peer: &nectarlink_protocol::DeviceId,
        _session: u32,
    ) -> Option<std::sync::Arc<dyn crate::MirrorSink>> {
        None
    }

    /// A PC asked for this phone's screen: ask the user, then (if they
    /// agree) stream it with [`Node::mirror_open`](crate::Node::mirror_open).
    /// Or, with `options.app`, for an app in a window of its own: run it on
    /// a display of its own and stream that (no asking: that's Elevated).
    /// The stream's config packets carry `options.session`. `Err` (a
    /// reason for logs) when it can't.
    fn mirror_requested(
        &self,
        _peer: &nectarlink_protocol::DeviceId,
        _options: &crate::MirrorStart,
    ) -> Result<(), String> {
        Err("this device doesn't share its screen".into())
    }

    /// The PC stopped watching: stop sharing the screen or app window.
    fn mirror_stop_requested(&self, _peer: &nectarlink_protocol::DeviceId, _session: u32) {}

    /// The PC's mouse or keyboard on this phone's mirrored screen or app
    /// window (checked, and only while it offers `mirror.input`, or
    /// `mirror.virtual_display` for app windows). Must return quickly.
    fn mirror_input(&self, _peer: &nectarlink_protocol::DeviceId, _session: u32, _input: crate::MirrorInput) {
    }

    /// The PC's decoder needs a fresh start: encode a keyframe next.
    fn mirror_keyframe_requested(&self, _peer: &nectarlink_protocol::DeviceId, _session: u32) {}

    /// The PC resized an app window (`session` != 0): resize its display and
    /// restart its encoder at `width × height` pixels.
    fn mirror_resize_requested(
        &self,
        _peer: &nectarlink_protocol::DeviceId,
        _session: u32,
        _width: u32,
        _height: u32,
    ) {
    }

    /// The apps a PC may open in windows of their own (launchable ones),
    /// with small PNG icons (a PC asked).
    fn phone_apps(&self) -> Result<Vec<crate::PhoneApp>, String> {
        Err("this device doesn't open apps in windows".into())
    }

    /// This phone's latest conversations, newest first (a PC asked).
    fn sms_threads(&self, _limit: u32) -> Result<Vec<crate::SmsThread>, String> {
        Err("this device has no messages".into())
    }

    /// A conversation's messages before `before` (Unix ms; the latest when
    /// `None`), newest first.
    fn sms_messages(
        &self,
        _thread: &str,
        _before: Option<i64>,
        _limit: u32,
    ) -> Result<Vec<crate::SmsMessage>, String> {
        Err("this device has no messages".into())
    }

    /// Send a text (checked: 1–20 recipients, not empty).
    fn sms_send(&self, _to: &[String], _body: &str) -> Result<(), String> {
        Err("this device doesn't send texts".into())
    }

    /// A picture in a message: its type and bytes.
    fn sms_part(&self, _id: &str) -> Result<(String, Vec<u8>), String> {
        Err("this device has no messages".into())
    }

    /// Answer, decline or silence this phone's call `id` (a PC asked; the
    /// core checked it's the call in progress). `Err` holds a reason for logs.
    fn call_command(&self, _id: &str, _command: crate::CallCommand) -> Result<(), String> {
        Err("this device doesn't take calls".into())
    }

    /// This phone's recent calls before `before` (Unix ms; the latest when
    /// `None`), newest first (a PC asked).
    fn call_log(&self, _before: Option<i64>, _limit: u32) -> Result<Vec<crate::CallLogEntry>, String> {
        Err("this device has no call history".into())
    }

    /// Place a call to `number` (or open the dialer with it filled in) on
    /// this phone (a PC asked; checked not blank).
    fn call_dial(&self, _number: &str) -> Result<(), String> {
        Err("this device doesn't place calls".into())
    }

    /// This phone's contacts matching `query` (or all with phone numbers
    /// when `None`), favorites first then alphabetical, skipping `offset`
    /// (a PC asked).
    fn contacts(
        &self,
        _query: Option<&str>,
        _offset: u32,
        _limit: u32,
    ) -> Result<Vec<crate::Contact>, String> {
        Err("this device has no contacts".into())
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

    /// Inject pointer, keyboard or presentation input from a paired phone
    /// (`docs/protocol/remote.md`).
    fn remote_input(
        &self,
        _peer: &nectarlink_protocol::DeviceId,
        _input: crate::RemoteInput,
    ) -> Result<(), String> {
        Err("this device doesn't take remote input".into())
    }

    /// Change one of this phone's quick settings (`phone.toggle.set`,
    /// `docs/protocol/toggles.md`). `id` and `value` have been validated and
    /// checked against this phone's capabilities. `Err` holds a reason for logs.
    fn set_phone_toggle(&self, _id: &str, _value: &crate::PhoneToggleValue) -> Result<(), String> {
        Err("this device has no phone toggles".into())
    }

    /// Run a Deck tile's action on this PC (`deck.press`, `docs/protocol/deck.md`).
    /// `tile` has been validated against the current Deck layout and the
    /// required per-device toggles (`remote_input`, plus `commands` for
    /// `run_command` tiles). `Err` holds a reason for logs.
    fn deck_press(&self, _from: &nectarlink_protocol::DeviceId, _tile: &str) -> Result<(), String> {
        Err("this device has no deck actions".into())
    }

    /// List immediate children of `path` (`""` for the shared storage root) on
    /// this phone (`storage.list`, `docs/protocol/storage.md`).
    fn storage_list(&self, _path: &str) -> Result<Vec<crate::StorageEntry>, crate::StorageError> {
        Err(crate::StorageError::Unsupported)
    }

    /// Open a file in this phone's shared storage for ranged reading (`storage.read`).
    fn storage_open_read(&self, _path: &str) -> Result<crate::StorageReadFile, crate::StorageError> {
        Err(crate::StorageError::Unsupported)
    }

    /// Commit a fully staged upload (`staged`) to `path` in this phone's shared
    /// storage (`storage.write`).
    fn storage_write(
        &self,
        _path: &str,
        _staged: &std::path::Path,
        _modified: Option<i64>,
    ) -> Result<crate::StorageWriteDone, crate::StorageError> {
        Err(crate::StorageError::Unsupported)
    }

    /// Create directory `path` in this phone's shared storage (`storage.mkdir`).
    fn storage_mkdir(&self, _path: &str) -> Result<(), crate::StorageError> {
        Err(crate::StorageError::Unsupported)
    }

    /// Rename or move `from` to `to` in this phone's shared storage (`storage.rename`).
    fn storage_rename(&self, _from: &str, _to: &str) -> Result<(), crate::StorageError> {
        Err(crate::StorageError::Unsupported)
    }

    /// Delete `path` in this phone's shared storage (`storage.delete`). Media
    /// items move to the phone's trash where supported; otherwise deletion
    /// requires `confirmed == true`.
    fn storage_delete(&self, _path: &str, _confirmed: bool) -> Result<(), crate::StorageError> {
        Err(crate::StorageError::Unsupported)
    }

    /// Where a phone's webcam video stream goes on this PC when it starts
    /// streaming (`docs/protocol/webcam.md`). `None` refuses the stream.
    fn webcam_sink(
        &self,
        _peer: &nectarlink_protocol::DeviceId,
    ) -> Option<std::sync::Arc<dyn crate::WebcamSink>> {
        None
    }

    /// A PC asked this phone to start streaming its camera (or updated camera /
    /// resolution while already streaming): prompt the user for consent (or
    /// update the running stream) and stream with [`Node::webcam_open`](crate::Node::webcam_open).
    fn webcam_requested(
        &self,
        _peer: &nectarlink_protocol::DeviceId,
        _options: &crate::WebcamStart,
    ) -> Result<(), String> {
        Err("this device doesn't stream a camera".into())
    }

    /// The PC stopped using this phone's webcam: stop streaming.
    fn webcam_stop_requested(&self, _peer: &nectarlink_protocol::DeviceId) {}

    /// The PC's webcam decoder needs a fresh keyframe (`webcam.keyframe`).
    fn webcam_keyframe_requested(&self, _peer: &nectarlink_protocol::DeviceId) {}
}

/// A platform that does nothing; useful for tests and headless tools.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopPlatform;

impl Platform for NoopPlatform {}
