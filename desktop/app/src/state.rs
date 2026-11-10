// SPDX-License-Identifier: GPL-3.0-or-later
//! What the UI shows, kept in plain Rust: the core's events and the UI's own
//! commands are folded into [`AppState`], and Qt objects are told which parts
//! changed so they can refresh. Nothing here touches Qt, so it is unit-tested.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
    time::{Instant, SystemTime},
};

use nectarlink_core::{
    Battery, CapabilityMatrix, DeviceId, DeviceInfo, DiscoveredDevice, LinkState, LocalSendPeer, MediaPlayer,
    NodeEvent, Notification, PairedDevice, PairingEvent, PairingFailure, PhoneToggles, PowerLevel, Transfer,
    TransferState,
};

/// Which parts of the state changed, so listeners refresh only what they show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Changes(u32);

impl Changes {
    pub const NONE: Changes = Changes(0);
    pub const STATUS: Changes = Changes(1);
    pub const DEVICES: Changes = Changes(1 << 1);
    pub const DISCOVERED: Changes = Changes(1 << 2);
    pub const PAIRING: Changes = Changes(1 << 3);
    pub const CAPABILITIES: Changes = Changes(1 << 4);
    pub const RINGING: Changes = Changes(1 << 5);
    pub const NOTIFICATIONS: Changes = Changes(1 << 6);
    pub const TRANSFERS: Changes = Changes(1 << 7);
    pub const MEDIA: Changes = Changes(1 << 8);
    /// Notification history, or whether it's kept.
    pub const HISTORY: Changes = Changes(1 << 9);
    /// Apps that sent notifications, and the user's rules for them.
    pub const APPS: Changes = Changes(1 << 10);
    /// An update was found (see crate::updater).
    pub const UPDATE: Changes = Changes(1 << 11);
    /// A phone's text messages (kept by crate::messages).
    pub const MESSAGES: Changes = Changes(1 << 12);
    /// Mirroring a phone's screen (kept by crate::mirror).
    pub const MIRROR: Changes = Changes(1 << 13);
    /// A call in progress on a phone (kept by crate::calls).
    pub const CALLS: Changes = Changes(1 << 14);
    /// A phone's photo gallery (kept by crate::photos).
    pub const PHOTOS: Changes = Changes(1 << 15);
    /// Remote input prompt or laser pointer overlay state.
    pub const REMOTE: Changes = Changes(1 << 16);
    /// A phone's quick settings toggles.
    pub const TOGGLES: Changes = Changes(1 << 17);
    /// This PC's Deck layout or live state (kept by crate::deck).
    pub const DECK: Changes = Changes(1 << 18);
    /// Phone as a webcam state or preferences (kept by crate::webcam).
    pub const WEBCAM: Changes = Changes(1 << 19);
    /// Encrypted local clipboard history (kept by nectarlink_core).
    pub const CLIPBOARD: Changes = Changes(1 << 20);
    /// Local cross-device timeline (kept by nectarlink_core).
    pub const TIMELINE: Changes = Changes(1 << 21);
    /// Continuity Camera capture request or fallback paste prompt state.
    pub const CONTINUITY: Changes = Changes(1 << 22);
    /// Command Palette open/search/results state (kept by crate::command_palette).
    pub const PALETTE: Changes = Changes(1 << 23);
    /// Data retention & local storage summary (Settings → Data & storage).
    pub const DATA_RETENTION: Changes = Changes(1 << 24);
    /// LocalSend LAN interop settings or discovered peers changed.
    pub const LOCALSEND: Changes = Changes(1 << 25);
    /// Desktop preferences changed from a background/remote event.
    pub const PREFS: Changes = Changes(1 << 26);
    /// High-frequency per-device metrics (RTT, battery, power) changed without structural device changes.
    pub const DEVICE_METRICS: Changes = Changes(1 << 27);
    /// Offline send queue items changed.
    pub const QUEUE: Changes = Changes(1 << 28);
    /// Phone as microphone state or output (kept by crate::phone_mic).
    pub const MIC: Changes = Changes(1 << 29);
    /// Photo backup settings or progress (kept by crate::photo_backup).
    pub const PHOTO_BACKUP: Changes = Changes(1 << 30);

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn intersects(self, other: Changes) -> bool {
        self.0 & other.0 != 0
    }
}

impl std::ops::BitOr for Changes {
    type Output = Changes;
    fn bitor(self, rhs: Changes) -> Changes {
        Changes(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for Changes {
    fn bitor_assign(&mut self, rhs: Changes) {
        self.0 |= rhs.0;
    }
}

/// Whether the core is running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreStatus {
    Starting,
    Ready { device_id: DeviceId, name: String },
    Failed(String),
}

/// A paired device as the UI shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceView {
    pub id: DeviceId,
    pub info: DeviceInfo,
    pub paired_at: i64,
    pub link: LinkState,
    pub battery: Option<Battery>,
    pub power: PowerLevel,
    /// Whether this PC reconnects to it by itself (`Node::set_reconnect`).
    pub reconnect: bool,
}

impl From<PairedDevice> for DeviceView {
    fn from(d: PairedDevice) -> Self {
        DeviceView {
            id: d.id,
            info: d.info,
            paired_at: d.paired_at,
            link: d.link,
            battery: None,
            power: PowerLevel::Basic,
            reconnect: d.reconnect,
        }
    }
}

/// A player on a paired device (docs/protocol/media.md).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerView {
    pub device: DeviceId,
    /// Without its artwork, which is in `art`.
    pub player: MediaPlayer,
    pub art: Option<PathBuf>,
    /// When `player.position` was current.
    pub at: SystemTime,
}

/// The pairing screen's state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PairingView {
    #[default]
    Idle,
    /// Asking the core for a pairing link.
    Starting,
    /// Showing a QR code for this link until `expires_at`.
    Hosting {
        uri: String,
        expires_at: SystemTime,
    },
    /// Dialing a nearby device that is in pairing mode.
    Connecting {
        peer: DeviceId,
    },
    /// Both screens show this code; the user confirms it matches.
    Comparing {
        peer: DeviceId,
        code: String,
    },
    /// This user confirmed; waiting for the other device.
    Confirmed {
        peer: DeviceId,
        code: String,
    },
    Paired {
        device: DeviceId,
        name: String,
    },
    Failed(PairingFailure),
}

/// A phone notification as the UI shows it. The icon bytes are kept on disk
/// (see [`AppState::app_icons`]), not here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationView {
    pub device: DeviceId,
    pub notification: Notification,
}

/// What the user wants from an app's notifications on this PC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppRule {
    /// In the feed, and as Windows notifications.
    #[default]
    Show,
    /// In the feed only: no pop-ups or sounds.
    Quiet,
    /// Not shown at all.
    Hidden,
}

/// A notification that went away on the phone (or was dismissed here).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub device: DeviceId,
    pub notification: Notification,
    /// Unix milliseconds.
    pub removed_at: i64,
}

/// How long history is kept, and how much of it.
pub const HISTORY_MS: i64 = 24 * 3600 * 1000;
pub const MAX_HISTORY: usize = 300;

/// A reply sent from this PC to a notification, shown under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentReply {
    pub text: String,
    /// Still on its way to the phone.
    pub pending: bool,
}

/// A file transfer as the UI shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct TransferView {
    pub transfer: Transfer,
    /// Recent speed, bytes per second (0 when not running).
    pub rate: f64,
    /// When progress was last reported.
    pub(crate) sampled: Instant,
}

pub fn unix_ms(time: SystemTime) -> i64 {
    time.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// Finished transfers kept in the list.
pub const MAX_FINISHED_TRANSFERS: usize = 20;

/// The most notifications kept; the oldest go first.
pub const MAX_NOTIFICATIONS: usize = 200;

#[derive(Debug, Default)]
pub struct AppState {
    pub status: Option<CoreStatus>,
    /// Paired devices, in pairing order.
    pub devices: Vec<DeviceView>,
    pub discovered: Vec<DiscoveredDevice>,
    pub pairing: PairingView,
    pub matrices: HashMap<DeviceId, CapabilityMatrix>,
    /// Bumped whenever `matrices` changes.
    pub matrices_version: u32,
    /// Latest quick settings toggles per paired phone.
    pub toggles: HashMap<DeviceId, PhoneToggles>,
    /// Bumped whenever `toggles` changes.
    pub toggles_version: u32,
    /// The device that asked this PC to ring, while it rings.
    pub ringing_from: Option<DeviceId>,
    pub ringing_devices: BTreeSet<DeviceId>,
    /// Phone notifications, newest first.
    pub notifications: Vec<NotificationView>,
    /// App icon files by package name.
    pub app_icons: HashMap<String, PathBuf>,
    /// Picture files of notifications that have one, by device and key.
    pub notification_images: HashMap<(DeviceId, String), PathBuf>,
    /// Replies sent from this PC, per notification, oldest first.
    pub replies: HashMap<(DeviceId, String), Vec<SentReply>>,
    /// File transfers, newest first.
    pub transfers: Vec<TransferView>,
    /// Media players on paired devices, each device's in its order.
    pub media: Vec<PlayerView>,
    /// Apps that sent notifications (package → name), and the user's rules
    /// for those that aren't shown normally.
    pub app_names: BTreeMap<String, String>,
    pub app_rules: BTreeMap<String, AppRule>,
    /// Apps whose notification groups are currently expanded in the feed.
    pub expanded_apps: BTreeSet<String>,
    /// Most recently dismissed notification(s) for in-feed undo.
    pub last_dismissed: Vec<NotificationView>,
    pub last_dismissed_label: String,
    /// Notifications cleared here, by device and key, with the post time they
    /// had. The phone can't always remove them (missed calls, alarms) or
    /// isn't reachable yet, and its next list (after a reconnect, or Refresh)
    /// mustn't bring them back. A newer post under the same key shows again.
    pub cleared: HashMap<(DeviceId, String), i64>,
    /// Notifications that went away, newest first, while history is kept.
    pub history: Vec<HistoryEntry>,
    pub history_enabled: bool,
    /// Phone asking to control this PC for the first time while `remote_input` is off.
    pub remote_prompt: Option<DeviceId>,
    /// Phone asking to view this PC's screen while `pc_mirror` is off.
    pub pc_mirror_prompt: Option<(DeviceId, nectarlink_core::MirrorStart)>,
    /// Active laser pointer overlay (`(device, x, y)` in `0.0..=1.0`).
    pub laser: Option<(DeviceId, f32, f32)>,
    /// This PC's physical adapters and Wake-on-LAN readiness.
    pub wake: crate::win::wol::WakeStatus,
    /// Whether LocalSend LAN interop is enabled and whether port 53317 is bound for receiving.
    pub localsend_enabled: bool,
    pub localsend_receiving: bool,
    /// Discovered LocalSend peers on the LAN.
    pub localsend_peers: Vec<LocalSendPeer>,
    /// Persistent outbound queue items waiting for an offline device or in flight.
    pub send_queue: Vec<nectarlink_core::QueueItem>,
}

impl AppState {
    pub fn core_status(&self) -> CoreStatus {
        self.status.clone().unwrap_or(CoreStatus::Starting)
    }

    pub fn device(&self, id: &DeviceId) -> Option<&DeviceView> {
        self.devices.iter().find(|d| d.id == *id)
    }

    fn device_mut(&mut self, id: &DeviceId) -> Option<&mut DeviceView> {
        self.devices.iter_mut().find(|d| d.id == *id)
    }

    /// A name for any device we know of (paired, nearby, or LocalSend peer).
    /// If the user set a custom nickname for a paired device on this PC, it takes precedence.
    pub fn name_of(&self, id: &DeviceId) -> Option<String> {
        crate::settings::nickname_for(&id.to_string())
            .or_else(|| self.device(id).map(|d| d.info.name.clone()))
            .or_else(|| self.discovered.iter().find(|d| d.id == *id).and_then(|d| d.name.clone()))
            .or_else(|| self.localsend_peers.iter().find(|p| p.id == *id).map(|p| p.alias.clone()))
    }

    /// Sorts paired devices so connected (`Online`) phones appear first, then
    /// `Connecting`, then `Offline` (most recently seen first, falling back to
    /// `paired_at` order).
    fn sort_devices(&mut self) {
        let tier = |link: &LinkState| match link {
            LinkState::Online { .. } => 0u8,
            LinkState::Connecting => 1u8,
            LinkState::Offline { .. } => 2u8,
        };
        let last_seen = |link: &LinkState| match link {
            LinkState::Offline { last_seen } => last_seen.unwrap_or(0),
            _ => 0,
        };
        self.devices.sort_by(|a, b| {
            tier(&a.link)
                .cmp(&tier(&b.link))
                .then_with(|| last_seen(&b.link).cmp(&last_seen(&a.link)))
                .then_with(|| a.paired_at.cmp(&b.paired_at))
        });
    }

    /// Replaces the device list (at startup, from the core's store).
    pub fn set_devices(&mut self, devices: Vec<PairedDevice>) -> Changes {
        self.devices = devices.into_iter().map(DeviceView::from).collect();
        self.sort_devices();
        Changes::DEVICES
    }

    /// Folds one core event into the state.
    pub fn apply(&mut self, event: &NodeEvent) -> Changes {
        let changes = self.apply_event(event);
        if changes.intersects(Changes::NOTIFICATIONS) {
            self.prune_replies();
        }
        changes
    }

    fn apply_event(&mut self, event: &NodeEvent) -> Changes {
        match event {
            NodeEvent::DeviceAdded(device) => {
                match self.device_mut(&device.id) {
                    Some(existing) => {
                        existing.info = device.info.clone();
                        existing.paired_at = device.paired_at;
                    }
                    None => {
                        self.devices.push(DeviceView::from(device.clone()));
                    }
                }
                self.sort_devices();
                Changes::DEVICES
            }
            NodeEvent::DeviceRemoved(id) => {
                let before = self.devices.len();
                self.devices.retain(|d| d.id != *id);
                let mut changes = Changes::NONE;
                if self.devices.len() != before {
                    changes |= Changes::DEVICES;
                }
                if self.matrices.remove(id).is_some() {
                    self.matrices_version = self.matrices_version.wrapping_add(1);
                    changes |= Changes::CAPABILITIES;
                }
                if self.toggles.remove(id).is_some() {
                    self.toggles_version = self.toggles_version.wrapping_add(1);
                    changes |= Changes::TOGGLES;
                }
                if self.ringing_devices.remove(id) {
                    changes |= Changes::RINGING;
                }
                if self.ringing_from == Some(*id) {
                    self.ringing_from = None;
                    changes |= Changes::RINGING;
                }
                if self.remote_prompt == Some(*id) {
                    self.remote_prompt = None;
                    changes |= Changes::REMOTE;
                }
                if self.pc_mirror_prompt.as_ref().is_some_and(|(d, _)| d == id) {
                    self.pc_mirror_prompt = None;
                    changes |= Changes::REMOTE;
                }
                if self.laser.is_some_and(|(d, _, _)| d == *id) {
                    self.laser = None;
                    changes |= Changes::REMOTE;
                }
                let notifications = self.notifications.len();
                self.notifications.retain(|n| n.device != *id);
                if self.notifications.len() != notifications {
                    changes |= Changes::NOTIFICATIONS;
                }
                // Nothing of an unpaired phone's stays, nor comes back with Undo.
                let kept = (self.history.len(), self.last_dismissed.len());
                self.history.retain(|h| h.device != *id);
                self.last_dismissed.retain(|n| n.device != *id);
                self.notification_images.retain(|(device, _), _| device != id);
                self.cleared.retain(|(device, _), _| device != id);
                if self.history.len() != kept.0 {
                    // Saved again without them.
                    changes |= Changes::HISTORY;
                }
                if self.last_dismissed.len() != kept.1 {
                    changes |= Changes::NOTIFICATIONS;
                }
                let transfers = self.transfers.len();
                self.transfers.retain(|t| t.transfer.device != *id);
                if self.transfers.len() != transfers {
                    changes |= Changes::TRANSFERS;
                }
                let queue_len = self.send_queue.len();
                self.send_queue.retain(|q| q.peer != *id);
                if self.send_queue.len() != queue_len {
                    changes |= Changes::QUEUE | Changes::TRANSFERS;
                }
                changes | self.set_media(*id, Vec::new())
            }
            NodeEvent::ReconnectChanged { device, on } => self.update_device(device, |d| d.reconnect = *on),
            NodeEvent::LinkChanged { device, link } => {
                let tier_or_path_changed = self.device(device).is_some_and(|d| {
                    !matches!(
                        (&d.link, link),
                        (LinkState::Online { path: p1, .. }, LinkState::Online { path: p2, .. }) if p1 == p2
                    )
                });
                let mut changes = if tier_or_path_changed {
                    let c = self.update_device(device, |d| d.link = link.clone());
                    if !c.is_empty() {
                        self.sort_devices();
                        c | Changes::DEVICE_METRICS
                    } else {
                        c
                    }
                } else {
                    self.update_device_metrics(device, |d| d.link = link.clone())
                };
                // A device that comes back sends what plays and its toggles then.
                if matches!(link, LinkState::Offline { .. }) {
                    changes |= self.set_toggles(*device, None);
                    changes |= self.set_media(*device, Vec::new());
                }
                changes
            }
            NodeEvent::PeerInfoChanged { device, info } => {
                self.update_device(device, |d| d.info = info.clone())
            }
            NodeEvent::PeerPowerChanged { device, power } => {
                self.update_device_metrics(device, |d| d.power = *power)
            }
            NodeEvent::Battery { device, battery } => {
                self.update_device_metrics(device, |d| d.battery = Some(battery.clone()))
            }
            NodeEvent::PhoneToggles { device, toggles } => self.set_toggles(*device, Some(toggles.clone())),
            NodeEvent::Capabilities(matrix) => {
                if self.matrices.get(&matrix.device) == Some(matrix) {
                    return Changes::NONE;
                }
                self.set_matrix(matrix.clone())
            }
            NodeEvent::Ring { device, on } => {
                let next = if *on {
                    Some(*device)
                } else {
                    self.ringing_from.filter(|id| id != device)
                };
                if self.ringing_from == next {
                    return Changes::NONE;
                }
                self.ringing_from = next;
                Changes::RINGING
            }
            NodeEvent::RemoteRing { device, on } => {
                let changed = if *on {
                    self.ringing_devices.insert(*device)
                } else {
                    self.ringing_devices.remove(device)
                };
                if changed { Changes::RINGING } else { Changes::NONE }
            }
            NodeEvent::RemoteInputRequested { device } => {
                if self.remote_prompt == Some(*device) {
                    return Changes::NONE;
                }
                self.remote_prompt = Some(*device);
                Changes::REMOTE
            }
            NodeEvent::PcMirrorRequested { device, options } => {
                self.pc_mirror_prompt = Some((*device, options.clone()));
                Changes::REMOTE
            }
            NodeEvent::Discovered(found) => {
                match self.discovered.iter_mut().find(|d| d.id == found.id) {
                    Some(existing) if existing == found => return Changes::NONE,
                    Some(existing) => *existing = found.clone(),
                    None => self.discovered.push(found.clone()),
                }
                Changes::DISCOVERED
            }
            NodeEvent::DiscoveryExpired(id) => {
                let before = self.discovered.len();
                self.discovered.retain(|d| d.id != *id);
                if self.discovered.len() == before { Changes::NONE } else { Changes::DISCOVERED }
            }
            NodeEvent::Pairing(event) => self.apply_pairing(event),
            NodeEvent::NotificationsReset { device, items } => {
                // What the phone no longer shows can't come back as the same post.
                self.cleared.retain(|(d, key), _| d != device || items.iter().any(|i| i.key == *key));
                let before = self.notifications.len();
                // What's gone since the last list went away on the phone.
                let gone: Vec<NotificationView> = self
                    .notifications
                    .iter()
                    .filter(|n| n.device == *device && !items.iter().any(|i| i.key == n.notification.key))
                    .cloned()
                    .collect();
                self.notifications.retain(|n| n.device != *device);
                if before == self.notifications.len() && items.is_empty() {
                    return Changes::NONE;
                }
                let mut changes = Changes::NOTIFICATIONS;
                for n in items {
                    if !self.is_cleared(*device, n) {
                        changes |= self.insert_notification(*device, n.clone());
                    }
                }
                changes | self.remember(gone)
            }
            NodeEvent::NotificationPosted { device, notification } => {
                if self.is_cleared(*device, notification) {
                    return Changes::NONE;
                }
                self.cleared.remove(&(*device, notification.key.clone()));
                Changes::NOTIFICATIONS | self.upsert_notification(*device, notification.clone())
            }
            NodeEvent::NotificationRemoved { device, key } => {
                let Some(at) =
                    self.notifications.iter().position(|n| n.device == *device && n.notification.key == *key)
                else {
                    return Changes::NONE;
                };
                let gone = self.notifications.remove(at);
                Changes::NOTIFICATIONS | self.remember(vec![gone])
            }
            // Feedback (crate::clipboard), notifications (crate::photos,
            // crate::calls) and state kept elsewhere (crate::messages,
            // crate::mirror).
            NodeEvent::ClipboardReceived { .. }
            | NodeEvent::PhotoAdded { .. }
            | NodeEvent::PhotosChanged { .. }
            | NodeEvent::Call { .. }
            | NodeEvent::CallLogChanged { .. }
            | NodeEvent::ContactsChanged { .. }
            | NodeEvent::SmsChanged { .. }
            | NodeEvent::Mirroring { .. }
            | NodeEvent::WakeInfoChanged { .. }
            | NodeEvent::DeckLayout { .. }
            | NodeEvent::DeckState { .. }
            | NodeEvent::StorageRequested { .. }
            | NodeEvent::StorageChanged { .. }
            | NodeEvent::Webcam { .. }
            | NodeEvent::Mic { .. }
            | NodeEvent::PcAudioFrame { .. }
            | NodeEvent::CameraCaptureRequested { .. }
            | NodeEvent::CameraCaptureCancelled { .. }
            // The updater acts on it (update_relay); nothing shows.
            | NodeEvent::UpdateHint { .. } => Changes::NONE,
            NodeEvent::CameraCaptureReceived { .. } => Changes::TIMELINE,
            NodeEvent::ClipboardHistoryChanged => Changes::CLIPBOARD | Changes::TIMELINE,
            NodeEvent::TimelineChanged => Changes::TIMELINE,
            NodeEvent::LocalSendChanged => Changes::LOCALSEND,
            NodeEvent::QueueChanged | NodeEvent::QueueDroppedOnUnpair { .. } => {
                Changes::QUEUE | Changes::TRANSFERS
            }
            // The gallery's own downloads (for its viewer, or the clipboard)
            // aren't files the user keeps.
            NodeEvent::Transfer(transfer)
                if crate::photos::is_private(&transfer.id) || crate::photo_backup::claims(transfer) =>
            {
                self.forget_transfer(&transfer.id)
            }
            NodeEvent::Transfer(transfer) => self.update_transfer(transfer.clone(), Instant::now()),
            // With artwork saved first (see crate::media).
            NodeEvent::MediaChanged { .. } => Changes::NONE,
        }
    }

    /// Replaces the outbound send queue snapshot from the core.
    pub fn set_send_queue(&mut self, items: Vec<nectarlink_core::QueueItem>) -> Changes {
        self.send_queue = items;
        Changes::QUEUE | Changes::TRANSFERS
    }

    /// Replaces a device's players.
    pub fn set_media(&mut self, device: DeviceId, players: Vec<PlayerView>) -> Changes {
        let at = self.media.iter().position(|p| p.device == device).unwrap_or(self.media.len());
        let before: Vec<PlayerView> = self.media.iter().filter(|p| p.device == device).cloned().collect();
        if before.is_empty() && players.is_empty() {
            return Changes::NONE;
        }
        self.media.retain(|p| p.device != device);
        let at = at.min(self.media.len());
        self.media.splice(at..at, players);
        Changes::MEDIA
    }

    /// Adds or updates a transfer, keeping a smoothed speed.
    pub fn update_transfer(&mut self, transfer: Transfer, now: Instant) -> Changes {
        let mut queue_updated = false;
        if let Some(q) = self.send_queue.iter_mut().find(|q| q.id == transfer.id) {
            q.bytes_done = transfer.done;
            if transfer.total > 0 {
                q.bytes_total = transfer.total;
            }
            queue_updated = true;
        }
        match self.transfers.iter_mut().find(|t| t.transfer.id == transfer.id) {
            Some(view) => {
                let running = transfer.state == TransferState::Running;
                let was_running = view.transfer.state == TransferState::Running;
                if !running || !was_running {
                    // State transition (e.g. Preparing/Waiting -> Running or Running -> Waiting):
                    // reset rate and anchor timestamp to current `done` baseline.
                    view.rate = 0.0;
                    view.sampled = now;
                    view.transfer = transfer;
                } else if transfer.done < view.transfer.done {
                    // Retry rolled back un-acknowledged buffered bytes after a disconnect.
                    view.rate = 0.0;
                    view.sampled = now;
                    view.transfer = transfer;
                } else if transfer.done > view.transfer.done {
                    let elapsed = now.saturating_duration_since(view.sampled).as_secs_f64();
                    if elapsed >= 0.15 {
                        // Cap single-window instantaneous rate to 500 MB/s so initial QUIC
                        // socket buffer fills cannot display multi-GB/s spikes.
                        let raw_instant = (transfer.done - view.transfer.done) as f64 / elapsed;
                        let instant = raw_instant.min(500.0 * 1024.0 * 1024.0);
                        view.rate =
                            if view.rate == 0.0 { instant } else { view.rate * 0.72 + instant * 0.28 };
                        view.sampled = now;
                        view.transfer = transfer;
                    } else {
                        // Keep `view.sampled` and `view.transfer.done` anchored until at least
                        // 150ms have elapsed so bytes accumulate over a meaningful window.
                        let baseline_done = view.transfer.done;
                        view.transfer = transfer;
                        view.transfer.done = baseline_done;
                    }
                } else {
                    // Duplicate or metadata-only update while running: if stalled > 2.5s, decay rate.
                    let elapsed = now.saturating_duration_since(view.sampled).as_secs_f64();
                    if elapsed > 2.5 {
                        view.rate = 0.0;
                    }
                    view.transfer = transfer;
                }
            }
            // Unpairing cancels a phone's transfers after removing their rows.
            None if transfer.state == TransferState::Cancelled => return Changes::NONE,
            None => self.transfers.insert(0, TransferView { transfer, rate: 0.0, sampled: now }),
        }
        // Keep every unfinished transfer and the latest finished ones.
        let mut finished = 0;
        self.transfers.retain(|t| {
            if !t.transfer.state.is_finished() {
                return true;
            }
            finished += 1;
            finished <= MAX_FINISHED_TRANSFERS
        });
        if queue_updated { Changes::TRANSFERS | Changes::QUEUE } else { Changes::TRANSFERS }
    }

    /// Forgets a transfer, finished or not.
    pub fn forget_transfer(&mut self, id: &str) -> Changes {
        let before = self.transfers.len();
        self.transfers.retain(|t| t.transfer.id != id);
        if self.transfers.len() == before { Changes::NONE } else { Changes::TRANSFERS }
    }

    /// Forgets finished transfers.
    pub fn clear_finished_transfers(&mut self) -> Changes {
        let before = self.transfers.len();
        self.transfers.retain(|t| !t.transfer.state.is_finished());
        if self.transfers.len() == before { Changes::NONE } else { Changes::TRANSFERS }
    }

    /// Clears finished transfer records from in-memory state.
    pub fn clear_transfer_history(&mut self) -> Changes {
        self.clear_finished_transfers()
    }

    /// Records a reply on its way to the phone.
    pub fn reply_sending(&mut self, device: DeviceId, key: &str, text: &str) -> Changes {
        let reply = SentReply { text: text.to_owned(), pending: true };
        self.replies.entry((device, key.to_owned())).or_default().push(reply);
        Changes::NOTIFICATIONS
    }

    /// The phone took the reply (`delivered`), or it failed and is dropped.
    pub fn reply_done(&mut self, device: DeviceId, key: &str, text: &str, delivered: bool) -> Changes {
        let id = (device, key.to_owned());
        let Some(replies) = self.replies.get_mut(&id) else { return Changes::NONE };
        let Some(at) = replies.iter().position(|r| r.pending && r.text == text) else { return Changes::NONE };
        if delivered {
            replies[at].pending = false;
        } else {
            replies.remove(at);
        }
        Changes::NOTIFICATIONS
    }

    /// Forgets replies to notifications that are gone.
    fn prune_replies(&mut self) {
        let notifications = &self.notifications;
        self.replies.retain(|(device, key), _| {
            notifications.iter().any(|n| n.device == *device && n.notification.key == *key)
        });
    }

    /// Updates an existing live notification in place (so `ListView.add` animations
    /// don't re-run on progress updates) or inserts at its sorted position.
    fn upsert_notification(&mut self, device: DeviceId, mut notification: Notification) -> Changes {
        notification.icon = None;
        notification.image = None;
        let mut changes = Changes::NONE;
        if self.app_names.get(&notification.app) != Some(&notification.app_name) {
            self.app_names.insert(notification.app.clone(), notification.app_name.clone());
            changes |= Changes::APPS;
        }
        if notification.live.is_some()
            && let Some(existing) = self.notifications.iter_mut().find(|n| {
                n.device == device && n.notification.key == notification.key && n.notification.live.is_some()
            })
        {
            existing.notification = notification;
            return changes;
        }
        self.notifications.retain(|n| !(n.device == device && n.notification.key == notification.key));
        changes | self.insert_notification(device, notification)
    }

    /// Adds a notification (live items first, then newest first), without its icon.
    fn insert_notification(&mut self, device: DeviceId, mut notification: Notification) -> Changes {
        // Icons and pictures live on disk (see crate::notifications).
        notification.icon = None;
        notification.image = None;
        let mut changes = Changes::NONE;
        if self.app_names.get(&notification.app) != Some(&notification.app_name) {
            self.app_names.insert(notification.app.clone(), notification.app_name.clone());
            changes |= Changes::APPS;
        }
        let is_live = notification.live.is_some();
        let at = self.notifications.partition_point(|n| {
            let existing_live = n.notification.live.is_some();
            match (existing_live, is_live) {
                (true, false) => true,
                (false, true) => false,
                _ => n.notification.when >= notification.when,
            }
        });
        self.notifications.insert(at, NotificationView { device, notification });
        self.notifications.truncate(MAX_NOTIFICATIONS);
        changes
    }

    /// The rule for an app's notifications.
    pub fn app_rule(&self, app: &str) -> AppRule {
        self.app_rules.get(app).copied().unwrap_or_default()
    }

    pub fn set_app_rule(&mut self, app: &str, rule: AppRule) -> Changes {
        let changed = match rule {
            AppRule::Show => self.app_rules.remove(app).is_some(),
            rule => self.app_rules.insert(app.to_owned(), rule) != Some(rule),
        };
        if changed { Changes::APPS | Changes::NOTIFICATIONS | Changes::HISTORY } else { Changes::NONE }
    }

    /// Whether an app's notification group is expanded in the PC feed.
    pub fn is_app_expanded(&self, app: &str) -> bool {
        self.expanded_apps.contains(app)
    }

    pub fn set_app_expanded(&mut self, app: &str, expanded: bool) -> Changes {
        let changed =
            if expanded { self.expanded_apps.insert(app.to_owned()) } else { self.expanded_apps.remove(app) };
        if changed { Changes::APPS | Changes::NOTIFICATIONS } else { Changes::NONE }
    }

    pub fn toggle_app_expanded(&mut self, app: &str) -> Changes {
        let next = !self.is_app_expanded(app);
        self.set_app_expanded(app, next)
    }

    /// Records a user-initiated dismissal so the feed can offer Undo.
    /// Whether `n` is a post the user cleared here.
    pub fn is_cleared(&self, device: DeviceId, n: &Notification) -> bool {
        self.cleared.get(&(device, n.key.clone())).is_some_and(|&when| when == n.when)
    }

    /// Remembers that the user cleared a notification here (before it's removed).
    pub fn mark_cleared(&mut self, device: DeviceId, key: &str) {
        if let Some(n) = self.notifications.iter().find(|n| n.device == device && n.notification.key == key) {
            self.cleared.insert((device, key.to_owned()), n.notification.when);
        }
    }

    pub fn record_dismissed(&mut self, views: Vec<NotificationView>, label: String) -> Changes {
        if views.is_empty() {
            return Changes::NONE;
        }
        self.last_dismissed = views;
        self.last_dismissed_label = label;
        Changes::NOTIFICATIONS
    }

    /// Restores the most recently dismissed notification(s) to the PC feed.
    pub fn undo_dismiss(&mut self) -> Changes {
        let views = std::mem::take(&mut self.last_dismissed);
        self.last_dismissed_label.clear();
        if views.is_empty() {
            return Changes::NONE;
        }
        let mut changes = Changes::NOTIFICATIONS;
        for view in views {
            let before = self.history.len();
            self.history
                .retain(|h| !(h.device == view.device && h.notification.key == view.notification.key));
            if self.history.len() != before {
                changes |= Changes::HISTORY;
            }
            self.notifications
                .retain(|n| !(n.device == view.device && n.notification.key == view.notification.key));
            self.cleared.remove(&(view.device, view.notification.key.clone()));
            changes |= self.insert_notification(view.device, view.notification);
        }
        changes
    }

    pub fn clear_undo(&mut self) -> Changes {
        if self.last_dismissed.is_empty() && self.last_dismissed_label.is_empty() {
            return Changes::NONE;
        }
        self.last_dismissed.clear();
        self.last_dismissed_label.clear();
        Changes::NOTIFICATIONS
    }

    /// Keeps notifications that went away, while history is on.
    fn remember(&mut self, gone: Vec<NotificationView>) -> Changes {
        if !self.history_enabled || gone.is_empty() {
            return Changes::NONE;
        }
        let now = unix_ms(SystemTime::now());
        for view in gone {
            self.history
                .retain(|h| !(h.device == view.device && h.notification.key == view.notification.key));
            self.history.insert(
                0,
                HistoryEntry { device: view.device, notification: view.notification, removed_at: now },
            );
        }
        self.prune_history(now);
        Changes::HISTORY
    }

    /// Drops history older than a day, and beyond the most kept.
    pub fn prune_history(&mut self, now: i64) -> Changes {
        let before = self.history.len();
        self.history.retain(|h| now - h.removed_at < HISTORY_MS);
        self.history.truncate(MAX_HISTORY);
        if self.history.len() == before { Changes::NONE } else { Changes::HISTORY }
    }

    pub fn set_history_enabled(&mut self, on: bool) -> Changes {
        if self.history_enabled == on {
            return Changes::NONE;
        }
        self.history_enabled = on;
        if !on {
            self.history.clear();
            self.notification_images.clear();
            // Notifications still showing lose their pictures too.
            return Changes::HISTORY | Changes::NOTIFICATIONS;
        }
        Changes::HISTORY
    }

    pub fn clear_history(&mut self) -> Changes {
        if self.history.is_empty() && self.notification_images.is_empty() {
            return Changes::NONE;
        }
        self.history.clear();
        self.notification_images.clear();
        // Notifications still showing lose their pictures too.
        Changes::HISTORY | Changes::NOTIFICATIONS
    }

    pub fn clear_notification_history(&mut self) -> Changes {
        self.clear_history()
    }

    /// Records where a notification's picture is stored.
    pub fn set_notification_image(&mut self, key: (DeviceId, String), path: PathBuf) -> Changes {
        if self.notification_images.get(&key) == Some(&path) {
            return Changes::NONE;
        }
        self.notification_images.insert(key, path);
        Changes::NOTIFICATIONS
    }

    /// Applies a notification event along with where its pictures were saved
    /// (`None`: the notification came without one).
    pub fn apply_with_images(
        &mut self,
        event: &NodeEvent,
        images: Vec<((DeviceId, String), Option<PathBuf>)>,
    ) -> Changes {
        // A snapshot leaves pictures out when it runs short of room, so there a
        // missing picture only means "none" for a notification this PC hasn't
        // seen yet (a new post under a reused key). Single posts always carry theirs.
        let unchanged: Vec<(DeviceId, String)> = match event {
            NodeEvent::NotificationsReset { device, items } => items
                .iter()
                .filter(|n| {
                    n.image.is_none()
                        && self.notifications.iter().any(|v| {
                            v.device == *device
                                && v.notification.key == n.key
                                && v.notification.when == n.when
                                && v.notification.title == n.title
                                && v.notification.text == n.text
                        })
                })
                .map(|n| (*device, n.key.clone()))
                .collect(),
            _ => Vec::new(),
        };
        let mut changes = self.apply(event);
        for (key, path) in images {
            changes |= match path {
                Some(path) => self.set_notification_image(key, path),
                None if unchanged.contains(&key) => Changes::NONE,
                None => self.clear_notification_image(&key),
            };
        }
        changes
    }

    /// Forgets a notification's picture. Apps reuse a key for new posts (X
    /// does), and one without a picture must not show the previous one's.
    pub fn clear_notification_image(&mut self, key: &(DeviceId, String)) -> Changes {
        if self.notification_images.remove(key).is_some() { Changes::NOTIFICATIONS } else { Changes::NONE }
    }

    /// Records where an app's icon is stored.
    pub fn set_app_icon(&mut self, app: &str, path: PathBuf) -> Changes {
        if self.app_icons.get(app) == Some(&path) {
            return Changes::NONE;
        }
        self.app_icons.insert(app.to_owned(), path);
        Changes::NOTIFICATIONS
    }

    fn update_device(&mut self, id: &DeviceId, f: impl FnOnce(&mut DeviceView)) -> Changes {
        let Some(device) = self.device_mut(id) else { return Changes::NONE };
        let before = device.clone();
        f(device);
        if *device == before { Changes::NONE } else { Changes::DEVICES }
    }

    fn update_device_metrics(&mut self, id: &DeviceId, f: impl FnOnce(&mut DeviceView)) -> Changes {
        let Some(device) = self.device_mut(id) else { return Changes::NONE };
        let before = device.clone();
        f(device);
        if *device == before { Changes::NONE } else { Changes::DEVICE_METRICS }
    }

    fn apply_pairing(&mut self, event: &PairingEvent) -> Changes {
        let next = match event {
            PairingEvent::SasCode { peer, code } => {
                PairingView::Comparing { peer: *peer, code: code.clone() }
            }
            PairingEvent::Paired(device) => {
                PairingView::Paired { device: device.id, name: device.info.name.clone() }
            }
            PairingEvent::Failed(failure) => {
                // The core reports failures of attempts the UI isn't showing
                // (e.g. a stranger's wrong code while idle); keep idle then.
                if self.pairing == PairingView::Idle {
                    return Changes::NONE;
                }
                PairingView::Failed(failure.clone())
            }
        };
        self.set_pairing(next)
    }

    pub fn set_matrix(&mut self, matrix: CapabilityMatrix) -> Changes {
        self.matrices.insert(matrix.device, matrix);
        self.matrices_version = self.matrices_version.wrapping_add(1);
        Changes::CAPABILITIES
    }

    pub fn set_toggles(&mut self, device: DeviceId, toggles: Option<PhoneToggles>) -> Changes {
        let changed = match toggles {
            Some(t) if self.toggles.get(&device) == Some(&t) => false,
            Some(t) => {
                self.toggles.insert(device, t);
                true
            }
            None => self.toggles.remove(&device).is_some(),
        };
        if changed {
            self.toggles_version = self.toggles_version.wrapping_add(1);
            Changes::TOGGLES
        } else {
            Changes::NONE
        }
    }

    pub fn set_pairing(&mut self, next: PairingView) -> Changes {
        if self.pairing == next {
            return Changes::NONE;
        }
        self.pairing = next;
        Changes::PAIRING
    }

    pub fn set_wake(&mut self, next: crate::win::wol::WakeStatus) -> Changes {
        if self.wake == next {
            return Changes::NONE;
        }
        self.wake = next;
        Changes::STATUS
    }
}

/// A Qt object interested in some parts of the state. `notify` schedules a
/// refresh on the Qt thread and returns false once the object is gone.
struct Listener {
    interest: Changes,
    notify: Box<dyn Fn() -> bool + Send + Sync>,
}

/// The shared state plus the listeners to tell about changes.
#[derive(Default)]
pub struct Hub {
    state: Mutex<AppState>,
    listeners: Mutex<Vec<Listener>>,
}

impl std::fmt::Debug for Hub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hub").finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Hub {
    pub fn new() -> Arc<Hub> {
        Arc::new(Hub::default())
    }

    /// Changes the state and notifies the listeners interested in what changed.
    pub fn update(&self, f: impl FnOnce(&mut AppState) -> Changes) {
        let changes = f(&mut lock(&self.state));
        if !changes.is_empty() {
            self.notify(changes);
        }
    }

    pub fn read<T>(&self, f: impl FnOnce(&AppState) -> T) -> T {
        f(&lock(&self.state))
    }

    /// Registers a listener; it is called right away so it starts in sync.
    pub fn subscribe(&self, interest: Changes, notify: impl Fn() -> bool + Send + Sync + 'static) {
        if notify() {
            lock(&self.listeners).push(Listener { interest, notify: Box::new(notify) });
        }
    }

    /// Tells listeners about changes to state kept outside the hub.
    pub fn changed(&self, changes: Changes) {
        self.notify(changes);
    }

    fn notify(&self, changes: Changes) {
        lock(&self.listeners).retain(|l| !l.interest.intersects(changes) || (l.notify)());
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use nectarlink_core::{ConnectionPath, DeviceKind};

    use super::*;

    fn paired(n: u8, at: i64) -> PairedDevice {
        PairedDevice {
            id: DeviceId([n; 32]),
            info: DeviceInfo {
                name: format!("Phone {n}"),
                kind: DeviceKind::Phone,
                os: "android".into(),
                os_ver: "16".into(),
                model: None,
                accent: None,
                screen: None,
                install_id: None,
            },
            paired_at: at,
            link: LinkState::Offline { last_seen: None },
            can_wake: false,
            reconnect: true,
        }
    }

    #[test]
    fn devices_follow_core_events() {
        let mut s = AppState::default();
        assert_eq!(s.apply(&NodeEvent::DeviceAdded(paired(2, 20))), Changes::DEVICES);
        assert_eq!(s.apply(&NodeEvent::DeviceAdded(paired(1, 10))), Changes::DEVICES);
        assert_eq!(
            s.devices.iter().map(|d| d.paired_at).collect::<Vec<_>>(),
            [10, 20],
            "kept in pairing order"
        );

        let online = LinkState::Online { path: ConnectionPath::Lan, rtt_ms: 4 };
        let id = DeviceId([1; 32]);
        assert_eq!(
            s.apply(&NodeEvent::LinkChanged { device: id, link: online.clone() }),
            Changes::DEVICES | Changes::DEVICE_METRICS
        );
        assert_eq!(
            s.apply(&NodeEvent::LinkChanged { device: id, link: online }),
            Changes::NONE,
            "no change, no refresh"
        );
        assert_eq!(
            s.apply(&NodeEvent::LinkChanged {
                device: id,
                link: LinkState::Online { path: ConnectionPath::Lan, rtt_ms: 9 }
            }),
            Changes::DEVICE_METRICS,
            "RTT-only update emits DEVICE_METRICS without DEVICES"
        );
        let battery = Battery {
            level: 50,
            charging: false,
            plugged: None,
            full_in: None,
            cell_bars: None,
            cell_type: None,
            wifi_bars: None,
        };
        assert_eq!(
            s.apply(&NodeEvent::Battery { device: id, battery: battery.clone() }),
            Changes::DEVICE_METRICS
        );
        assert_eq!(s.device(&id).unwrap().battery, Some(battery));

        // Events about unknown devices are ignored.
        assert_eq!(
            s.apply(&NodeEvent::PeerPowerChanged { device: DeviceId([9; 32]), power: PowerLevel::Elevated }),
            Changes::NONE
        );

        assert_eq!(s.apply(&NodeEvent::DeviceRemoved(id)), Changes::DEVICES);
        assert_eq!(s.devices.len(), 1);
    }

    #[test]
    fn online_devices_sort_before_offline_devices() {
        let mut s = AppState::default();
        s.apply(&NodeEvent::DeviceAdded(paired(1, 10)));
        s.apply(&NodeEvent::DeviceAdded(paired(2, 20)));
        assert_eq!(s.devices[0].id, DeviceId([1; 32]));
        assert_eq!(s.devices[1].id, DeviceId([2; 32]));

        let online = LinkState::Online { path: ConnectionPath::Lan, rtt_ms: 3 };
        assert_eq!(
            s.apply(&NodeEvent::LinkChanged { device: DeviceId([2; 32]), link: online }),
            Changes::DEVICES | Changes::DEVICE_METRICS
        );
        assert_eq!(s.devices[0].id, DeviceId([2; 32]), "online device moves to index 0");
        assert_eq!(s.devices[1].id, DeviceId([1; 32]));
    }

    #[test]
    fn pairing_flow() {
        let mut s = AppState::default();
        let peer = DeviceId([3; 32]);
        // A failure nobody is looking at doesn't open the pairing screen.
        assert_eq!(
            s.apply(&NodeEvent::Pairing(PairingEvent::Failed(PairingFailure::Rejected))),
            Changes::NONE
        );

        s.set_pairing(PairingView::Starting);
        let code = PairingEvent::SasCode { peer, code: "123456".into() };
        assert_eq!(s.apply(&NodeEvent::Pairing(code)), Changes::PAIRING);
        assert_eq!(s.pairing, PairingView::Comparing { peer, code: "123456".into() });

        let mut device = paired(3, 30);
        device.info.name = "Pixel".into();
        s.apply(&NodeEvent::Pairing(PairingEvent::Paired(device)));
        assert_eq!(s.pairing, PairingView::Paired { device: peer, name: "Pixel".into() });

        s.apply(&NodeEvent::Pairing(PairingEvent::Failed(PairingFailure::Expired)));
        assert_eq!(s.pairing, PairingView::Failed(PairingFailure::Expired));
    }

    #[test]
    fn ringing_and_discovery() {
        let mut s = AppState::default();
        let id = DeviceId([4; 32]);
        assert_eq!(s.apply(&NodeEvent::Ring { device: id, on: true }), Changes::RINGING);
        assert_eq!(s.ringing_from, Some(id));
        assert_eq!(s.apply(&NodeEvent::Ring { device: id, on: true }), Changes::NONE);
        s.apply(&NodeEvent::Ring { device: id, on: false });
        assert_eq!(s.ringing_from, None);

        let found = DiscoveredDevice { id, name: Some("Pixel".into()) };
        assert_eq!(s.apply(&NodeEvent::Discovered(found.clone())), Changes::DISCOVERED);
        assert_eq!(s.apply(&NodeEvent::Discovered(found)), Changes::NONE);
        assert_eq!(s.name_of(&id).as_deref(), Some("Pixel"));
        assert_eq!(s.apply(&NodeEvent::DiscoveryExpired(id)), Changes::DISCOVERED);
        assert!(s.discovered.is_empty());
    }

    fn note(key: &str, when: i64) -> Notification {
        Notification {
            key: key.into(),
            app: "com.chat".into(),
            app_name: "Chat".into(),
            title: Some(key.to_uppercase()),
            text: Some("hi".into()),
            sub: None,
            when,
            actions: Vec::new(),
            silent: false,
            ongoing: false,
            icon: Some(vec![1, 2, 3]),
            image: None,
            live: None,
            conversation: None,
        }
    }

    #[test]
    fn a_new_post_under_a_reused_key_never_shows_the_old_picture() {
        let mut s = AppState::default();
        let phone = DeviceId([1; 32]);
        let key = (phone, "x".to_owned());
        let pic = PathBuf::from("helicopter.jpg");
        let mut first = note("x", 100);
        first.image = Some(vec![9]);
        s.apply_with_images(
            &NodeEvent::NotificationPosted { device: phone, notification: first.clone() },
            vec![(key.clone(), Some(pic.clone()))],
        );
        assert_eq!(s.notification_images.get(&key), Some(&pic));

        // A reconnect snapshot that left the same post's picture out keeps it.
        let mut same = first.clone();
        same.image = None;
        s.apply_with_images(
            &NodeEvent::NotificationsReset { device: phone, items: vec![same] },
            vec![(key.clone(), None)],
        );
        assert_eq!(s.notification_images.get(&key), Some(&pic), "same post, picture kept");

        // A new post under the same key without a picture drops the old one.
        let mut next = note("x", 200);
        next.text = Some("Day 8".into());
        s.apply_with_images(
            &NodeEvent::NotificationPosted { device: phone, notification: next.clone() },
            vec![(key.clone(), None)],
        );
        assert_eq!(s.notification_images.get(&key), None, "new post, no stale picture");

        // And a snapshot with a different post under that key does the same.
        s.set_notification_image(key.clone(), pic);
        let mut other = note("x", 300);
        other.text = Some("another post".into());
        s.apply_with_images(
            &NodeEvent::NotificationsReset { device: phone, items: vec![other] },
            vec![(key.clone(), None)],
        );
        assert_eq!(s.notification_images.get(&key), None);
    }

    fn keys(s: &AppState) -> Vec<&str> {
        s.notifications.iter().map(|n| n.notification.key.as_str()).collect()
    }

    #[test]
    fn live_notifications_stay_at_top_and_update_in_place() {
        let mut s = AppState::default();
        let phone = DeviceId([1; 32]);
        s.apply(&NodeEvent::NotificationPosted { device: phone, notification: note("msg1", 100) });

        let mut live1 = note("ride", 50);
        live1.live = Some(nectarlink_core::NotificationLive {
            v: 1,
            progress: Some(20),
            max: Some(100),
            indeterminate: false,
            chip: Some("8 min".into()),
            segments: Vec::new(),
            points: Vec::new(),
            chronometer: false,
            countdown: false,
        });
        s.apply(&NodeEvent::NotificationPosted { device: phone, notification: live1.clone() });
        assert_eq!(keys(&s), ["ride", "msg1"], "live notification sits above newer regular notification");

        // Even when a newer regular notification arrives, live stays at the top.
        s.apply(&NodeEvent::NotificationPosted { device: phone, notification: note("msg2", 200) });
        assert_eq!(keys(&s), ["ride", "msg2", "msg1"]);

        // Updating live progress keeps it in place at index 0.
        let mut live2 = live1;
        live2.live.as_mut().unwrap().progress = Some(65);
        s.apply(&NodeEvent::NotificationPosted { device: phone, notification: live2 });
        assert_eq!(keys(&s), ["ride", "msg2", "msg1"]);
        assert_eq!(s.notifications[0].notification.live.as_ref().unwrap().progress, Some(65));
    }

    #[test]
    fn notifications_stay_newest_first_and_follow_the_phone() {
        let mut s = AppState::default();
        let (phone, other) = (DeviceId([1; 32]), DeviceId([2; 32]));
        let reset =
            NodeEvent::NotificationsReset { device: phone, items: vec![note("a", 10), note("b", 30)] };
        assert_eq!(s.apply(&reset), Changes::NOTIFICATIONS | Changes::APPS, "and the app is new");
        s.apply(&NodeEvent::NotificationPosted { device: other, notification: note("c", 20) });
        assert_eq!(keys(&s), ["b", "c", "a"]);
        assert!(s.notifications.iter().all(|n| n.notification.icon.is_none()), "icons live on disk");

        // An update replaces the old version and moves to its new time.
        s.apply(&NodeEvent::NotificationPosted { device: phone, notification: note("a", 40) });
        assert_eq!(keys(&s), ["a", "b", "c"]);

        assert_eq!(
            s.apply(&NodeEvent::NotificationRemoved { device: phone, key: "b".into() }),
            Changes::NOTIFICATIONS
        );
        assert_eq!(
            s.apply(&NodeEvent::NotificationRemoved { device: phone, key: "b".into() }),
            Changes::NONE
        );

        // A snapshot replaces only that phone's notifications.
        s.apply(&NodeEvent::NotificationsReset { device: phone, items: Vec::new() });
        assert_eq!(keys(&s), ["c"]);
        assert_eq!(
            s.apply(&NodeEvent::NotificationsReset { device: phone, items: Vec::new() }),
            Changes::NONE
        );

        // Unpairing forgets them.
        assert!(s.apply(&NodeEvent::DeviceRemoved(other)).intersects(Changes::NOTIFICATIONS));
        assert!(s.notifications.is_empty());
    }

    #[test]
    fn replies_show_until_their_notification_goes() {
        let mut s = AppState::default();
        let phone = DeviceId([1; 32]);
        s.apply(&NodeEvent::NotificationPosted { device: phone, notification: note("a", 10) });
        s.reply_sending(phone, "a", "On my way");
        s.reply_sending(phone, "a", "Five minutes");
        s.reply_done(phone, "a", "On my way", true);
        s.reply_done(phone, "a", "Five minutes", false);
        let replies = &s.replies[&(phone, "a".to_owned())];
        assert_eq!(
            replies,
            &[SentReply { text: "On my way".into(), pending: false }],
            "failed ones are dropped"
        );

        s.apply(&NodeEvent::NotificationRemoved { device: phone, key: "a".into() });
        assert!(s.replies.is_empty());
    }

    fn transfer(id: &str, done: u64, state: TransferState) -> Transfer {
        Transfer {
            id: id.into(),
            device: DeviceId([1; 32]),
            direction: nectarlink_core::Direction::Outgoing,
            names: vec!["a.bin".into()],
            files: 1,
            total: 1000,
            done,
            state,
            recording: false,
            markers: Vec::new(),
            open_on_arrival: false,
        }
    }

    #[test]
    fn transfers_update_in_place_with_a_speed() {
        let mut s = AppState::default();
        let t0 = Instant::now();
        s.update_transfer(transfer("a", 0, TransferState::Running), t0);
        s.update_transfer(transfer("b", 0, TransferState::Waiting), t0);
        assert_eq!(s.transfers.iter().map(|t| t.transfer.id.as_str()).collect::<Vec<_>>(), ["b", "a"]);
        s.update_transfer(transfer("a", 500, TransferState::Running), t0 + std::time::Duration::from_secs(1));
        let a = s.transfers.iter().find(|t| t.transfer.id == "a").unwrap();
        assert_eq!((a.transfer.done, a.rate), (500, 500.0));
        s.update_transfer(transfer("a", 1000, TransferState::Done { saved: Vec::new() }), t0);
        assert_eq!(s.transfers[1].rate, 0.0, "no speed once finished");

        assert_eq!(s.clear_finished_transfers(), Changes::TRANSFERS);
        assert_eq!(s.transfers.len(), 1, "the waiting one stays");
        for i in 0..(MAX_FINISHED_TRANSFERS + 5) {
            s.update_transfer(transfer(&i.to_string(), 1000, TransferState::Done { saved: Vec::new() }), t0);
        }
        assert_eq!(s.transfers.len(), MAX_FINISHED_TRANSFERS + 1);
    }

    #[test]
    fn the_feed_is_bounded() {
        let mut s = AppState::default();
        for i in 0..(MAX_NOTIFICATIONS as i64 + 10) {
            s.apply(&NodeEvent::NotificationPosted {
                device: DeviceId([1; 32]),
                notification: note(&i.to_string(), i),
            });
        }
        assert_eq!(s.notifications.len(), MAX_NOTIFICATIONS);
        assert_eq!(s.notifications[0].notification.when, MAX_NOTIFICATIONS as i64 + 9, "the oldest go first");
    }

    #[test]
    fn hub_notifies_interested_listeners_and_drops_dead_ones() {
        let hub = Hub::new();
        let devices = Arc::new(AtomicUsize::new(0));
        let pairing = Arc::new(AtomicUsize::new(0));
        let d = devices.clone();
        hub.subscribe(Changes::DEVICES, move || {
            d.fetch_add(1, Ordering::SeqCst);
            true
        });
        let p = pairing.clone();
        hub.subscribe(Changes::PAIRING, move || p.fetch_add(1, Ordering::SeqCst) < 1);
        assert_eq!((devices.load(Ordering::SeqCst), pairing.load(Ordering::SeqCst)), (1, 1), "initial sync");

        hub.update(|s| s.apply(&NodeEvent::DeviceAdded(paired(1, 1))));
        assert_eq!(devices.load(Ordering::SeqCst), 2);
        assert_eq!(pairing.load(Ordering::SeqCst), 1, "not interested");

        // The pairing listener reports it's gone on its next call...
        hub.update(|s| s.set_pairing(PairingView::Starting));
        hub.update(|s| s.set_pairing(PairingView::Idle));
        // ...and isn't called again.
        assert_eq!(pairing.load(Ordering::SeqCst), 2);
        hub.update(|_| Changes::NONE);
        assert_eq!(devices.load(Ordering::SeqCst), 2, "empty changes notify nobody");
    }

    #[test]
    fn gone_notifications_are_kept_for_a_day() {
        let phone = DeviceId([1; 32]);
        let mut s = AppState { history_enabled: true, ..AppState::default() };
        s.apply(&NodeEvent::NotificationsReset { device: phone, items: vec![note("a", 10), note("b", 20)] });
        assert!(s.history.is_empty());

        // Removed on the phone (or dismissed here).
        let changes = s.apply(&NodeEvent::NotificationRemoved { device: phone, key: "a".into() });
        assert!(changes.intersects(Changes::HISTORY));
        assert_eq!(s.history.iter().map(|h| h.notification.key.as_str()).collect::<Vec<_>>(), ["a"]);

        // Gone while disconnected: the next full list leaves it out.
        s.apply(&NodeEvent::NotificationsReset { device: phone, items: vec![note("c", 30)] });
        assert_eq!(s.history.iter().map(|h| h.notification.key.as_str()).collect::<Vec<_>>(), ["b", "a"]);

        // Old entries go.
        let later = s.history[0].removed_at + HISTORY_MS + 1;
        assert!(s.prune_history(later).intersects(Changes::HISTORY));
        assert!(s.history.is_empty());

        // Turned off: nothing kept, and what was is cleared.
        s.apply(&NodeEvent::NotificationRemoved { device: phone, key: "c".into() });
        assert_eq!(s.history.len(), 1);
        s.set_history_enabled(false);
        assert!(s.history.is_empty());
        s.apply(&NodeEvent::NotificationPosted { device: phone, notification: note("d", 40) });
        s.apply(&NodeEvent::NotificationRemoved { device: phone, key: "d".into() });
        assert!(s.history.is_empty());
    }

    #[test]
    fn apps_and_their_rules() {
        let phone = DeviceId([1; 32]);
        let mut s = AppState::default();
        let changes = s.apply(&NodeEvent::NotificationPosted { device: phone, notification: note("a", 10) });
        assert!(changes.intersects(Changes::APPS), "a new app");
        let app = s.notifications[0].notification.app.clone();
        assert!(s.app_names.contains_key(&app));
        assert_eq!(s.app_rule(&app), AppRule::Show);
        assert!(s.set_app_rule(&app, AppRule::Hidden).intersects(Changes::NOTIFICATIONS));
        assert_eq!(s.app_rule(&app), AppRule::Hidden);
        assert_eq!(s.set_app_rule(&app, AppRule::Hidden), Changes::NONE);
        s.set_app_rule(&app, AppRule::Show);
        assert!(s.app_rules.is_empty(), "the default isn't stored");
    }

    #[test]
    fn phone_toggles_follow_events_and_clear_on_offline_or_unpair() {
        let mut s = AppState::default();
        let phone = DeviceId([1; 32]);
        s.apply(&NodeEvent::DeviceAdded(paired(1, 10)));
        let online = LinkState::Online { path: ConnectionPath::Lan, rtt_ms: 4 };
        s.apply(&NodeEvent::LinkChanged { device: phone, link: online });

        let toggles = PhoneToggles {
            dnd: false,
            ringer: "ring".into(),
            flashlight: Some(false),
            volume: 60,
            brightness: 70,
            wifi: true,
            bluetooth: true,
        };
        assert_eq!(
            s.apply(&NodeEvent::PhoneToggles { device: phone, toggles: toggles.clone() }),
            Changes::TOGGLES
        );
        assert_eq!(s.toggles.get(&phone), Some(&toggles));
        assert_eq!(
            s.apply(&NodeEvent::PhoneToggles { device: phone, toggles: toggles.clone() }),
            Changes::NONE
        );

        let offline = LinkState::Offline { last_seen: Some(100) };
        let changes = s.apply(&NodeEvent::LinkChanged { device: phone, link: offline });
        assert!(changes.intersects(Changes::TOGGLES));
        assert!(!s.toggles.contains_key(&phone));

        s.apply(&NodeEvent::PhoneToggles { device: phone, toggles });
        assert!(s.apply(&NodeEvent::DeviceRemoved(phone)).intersects(Changes::TOGGLES));
        assert!(s.toggles.is_empty());
    }

    #[test]
    fn cleared_notifications_stay_cleared_until_a_new_post() {
        let phone = DeviceId([1; 32]);
        let mut s = AppState::default();
        let missed_call = note("call", 10);
        let alarm = note("alarm", 20);
        s.apply(&NodeEvent::NotificationsReset {
            device: phone,
            items: vec![missed_call.clone(), alarm.clone()],
        });

        // Cleared here; the phone couldn't remove it (or wasn't reachable).
        s.mark_cleared(phone, "call");
        s.apply(&NodeEvent::NotificationRemoved { device: phone, key: "call".into() });
        assert_eq!(keys(&s), ["alarm"]);

        // A reconnect or Refresh sends the phone's list again: it stays gone.
        s.apply(&NodeEvent::NotificationsReset {
            device: phone,
            items: vec![missed_call.clone(), alarm.clone()],
        });
        assert_eq!(keys(&s), ["alarm"]);
        // So does the same post sent again on its own.
        assert_eq!(
            s.apply(&NodeEvent::NotificationPosted { device: phone, notification: missed_call.clone() }),
            Changes::NONE
        );
        assert_eq!(keys(&s), ["alarm"]);

        // A newer post under the same key (another missed call) shows.
        s.apply(&NodeEvent::NotificationPosted { device: phone, notification: note("call", 30) });
        assert!(keys(&s).contains(&"call"));
        assert!(s.cleared.is_empty());
    }

    #[test]
    fn clears_are_forgotten_once_the_phone_drops_them() {
        let phone = DeviceId([1; 32]);
        let mut s = AppState::default();
        s.apply(&NodeEvent::NotificationsReset { device: phone, items: vec![note("a", 10)] });
        s.mark_cleared(phone, "a");
        s.apply(&NodeEvent::NotificationRemoved { device: phone, key: "a".into() });
        assert_eq!(s.cleared.len(), 1);
        // Gone from the phone too: nothing left to hold back.
        s.apply(&NodeEvent::NotificationsReset { device: phone, items: vec![] });
        assert!(s.cleared.is_empty());
    }

    #[test]
    fn undo_brings_a_cleared_notification_back_for_good() {
        let phone = DeviceId([1; 32]);
        let mut s = AppState { history_enabled: true, ..AppState::default() };
        s.apply(&NodeEvent::NotificationsReset { device: phone, items: vec![note("a", 10)] });
        let views = s.notifications.clone();
        s.mark_cleared(phone, "a");
        s.apply(&NodeEvent::NotificationRemoved { device: phone, key: "a".into() });
        s.record_dismissed(views, "Notification dismissed".into());
        s.undo_dismiss();
        assert_eq!(keys(&s), ["a"]);
        s.apply(&NodeEvent::NotificationsReset { device: phone, items: vec![note("a", 10)] });
        assert_eq!(keys(&s), ["a"]);
    }

    #[test]
    fn app_group_expansion_and_undo_dismiss_work() {
        let phone = DeviceId([1; 32]);
        let mut s = AppState { history_enabled: true, ..AppState::default() };
        s.apply(&NodeEvent::NotificationPosted { device: phone, notification: note("a", 10) });
        s.apply(&NodeEvent::NotificationPosted { device: phone, notification: note("b", 20) });
        let app = s.notifications[0].notification.app.clone();

        assert!(!s.is_app_expanded(&app));
        assert!(s.toggle_app_expanded(&app).intersects(Changes::NOTIFICATIONS | Changes::APPS));
        assert!(s.is_app_expanded(&app));
        assert!(s.toggle_app_expanded(&app).intersects(Changes::NOTIFICATIONS | Changes::APPS));
        assert!(!s.is_app_expanded(&app));

        let dismissed = s.notifications.clone();
        s.apply(&NodeEvent::NotificationRemoved { device: phone, key: "a".into() });
        s.apply(&NodeEvent::NotificationRemoved { device: phone, key: "b".into() });
        s.record_dismissed(dismissed, "Dismissed 2 from Chat".into());
        assert!(s.notifications.is_empty());
        assert_eq!(s.history.len(), 2);
        assert_eq!(s.last_dismissed_label, "Dismissed 2 from Chat");

        let changes = s.undo_dismiss();
        assert!(changes.intersects(Changes::NOTIFICATIONS));
        assert_eq!(keys(&s), ["b", "a"]);
        assert!(s.history.is_empty());
        assert!(s.last_dismissed_label.is_empty());
    }

    #[test]
    fn unpairing_a_phone_removes_its_transfers_for_good() {
        let mut s = AppState::default();
        let phone = DeviceId([1; 32]);
        s.apply(&NodeEvent::Transfer(transfer("t1", 450, TransferState::Running)));
        let mut other = transfer("t2", 450, TransferState::Running);
        other.device = DeviceId([2; 32]);
        s.apply(&NodeEvent::Transfer(other));

        assert!(s.apply(&NodeEvent::DeviceRemoved(phone)).intersects(Changes::TRANSFERS));
        assert_eq!(s.transfers.len(), 1);
        assert_eq!(s.transfers[0].transfer.id, "t2");

        // Unpairing cancels it, and that news arrives after the row has gone.
        assert_eq!(
            s.apply(&NodeEvent::Transfer(transfer("t1", 450, TransferState::Cancelled))),
            Changes::NONE
        );
        assert_eq!(s.transfers.len(), 1);
    }

    #[test]
    fn transfer_events_update_matching_send_queue_item_in_place() {
        let mut s = AppState::default();
        let peer = DeviceId([1; 32]);
        s.set_send_queue(vec![nectarlink_core::QueueItem {
            id: "q-transfer-1".into(),
            seq: 1,
            peer,
            peer_name: "Pixel".into(),
            kind: nectarlink_core::QueueKind::Files,
            status: nectarlink_core::QueueStatus::Sending,
            created_at: 100,
            expires_at: 200,
            attempts: 1,
            title: "a.bin".into(),
            detail: "1000 B".into(),
            bytes_total: 1000,
            bytes_done: 0,
            files_total: 1,
            files_done: 0,
            files: Vec::new(),
        }]);

        let t = transfer("q-transfer-1", 450, TransferState::Running);
        let changes = s.apply(&NodeEvent::Transfer(t));
        assert_eq!(changes, Changes::TRANSFERS | Changes::QUEUE);
        assert_eq!(s.send_queue[0].bytes_done, 450);
        assert_eq!(s.send_queue[0].bytes_total, 1000);
    }
}
