// SPDX-License-Identifier: GPL-3.0-or-later
//! What the UI shows, kept in plain Rust: the core's events and the UI's own
//! commands are folded into [`AppState`], and Qt objects are told which parts
//! changed so they can refresh. Nothing here touches Qt, so it is unit-tested.

use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
    time::{Instant, SystemTime},
};

use nectarlink_core::{
    Battery, CapabilityMatrix, DeviceId, DeviceInfo, DiscoveredDevice, LinkState, MediaPlayer, NodeEvent,
    Notification, PairedDevice, PairingEvent, PairingFailure, PowerLevel, Transfer, TransferState,
};

/// Which parts of the state changed, so listeners refresh only what they show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Changes(u16);

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
    /// The device that asked this PC to ring, while it rings.
    pub ringing_from: Option<DeviceId>,
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
    /// Notifications that went away, newest first, while history is kept.
    pub history: Vec<HistoryEntry>,
    pub history_enabled: bool,
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

    /// A name for any device we know of (paired or nearby).
    pub fn name_of(&self, id: &DeviceId) -> Option<String> {
        self.device(id)
            .map(|d| d.info.name.clone())
            .or_else(|| self.discovered.iter().find(|d| d.id == *id).and_then(|d| d.name.clone()))
    }

    /// Replaces the device list (at startup, from the core's store).
    pub fn set_devices(&mut self, devices: Vec<PairedDevice>) -> Changes {
        self.devices = devices.into_iter().map(DeviceView::from).collect();
        self.devices.sort_by_key(|d| d.paired_at);
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
                        self.devices.sort_by_key(|d| d.paired_at);
                    }
                }
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
                if self.ringing_from == Some(*id) {
                    self.ringing_from = None;
                    changes |= Changes::RINGING;
                }
                let notifications = self.notifications.len();
                self.notifications.retain(|n| n.device != *id);
                if self.notifications.len() != notifications {
                    changes |= Changes::NOTIFICATIONS;
                }
                changes | self.set_media(*id, Vec::new())
            }
            NodeEvent::LinkChanged { device, link } => {
                let mut changes = self.update_device(device, |d| d.link = link.clone());
                // A device that comes back sends what plays then.
                if matches!(link, LinkState::Offline { .. }) {
                    changes |= self.set_media(*device, Vec::new());
                }
                changes
            }
            NodeEvent::PeerInfoChanged { device, info } => {
                self.update_device(device, |d| d.info = info.clone())
            }
            NodeEvent::PeerPowerChanged { device, power } => self.update_device(device, |d| d.power = *power),
            NodeEvent::Battery { device, battery } => {
                self.update_device(device, |d| d.battery = Some(battery.clone()))
            }
            NodeEvent::Capabilities(matrix) => {
                if self.matrices.get(&matrix.device) == Some(matrix) {
                    return Changes::NONE;
                }
                self.set_matrix(matrix.clone())
            }
            NodeEvent::Ring { device, on } => {
                let next = on.then_some(*device);
                if self.ringing_from == next {
                    return Changes::NONE;
                }
                self.ringing_from = next;
                Changes::RINGING
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
                    changes |= self.insert_notification(*device, n.clone());
                }
                changes | self.remember(gone)
            }
            NodeEvent::NotificationPosted { device, notification } => {
                self.notifications
                    .retain(|n| !(n.device == *device && n.notification.key == notification.key));
                Changes::NOTIFICATIONS | self.insert_notification(*device, notification.clone())
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
            // Feedback (crate::clipboard) and notifications (crate::photos,
            // crate::calls) only; nothing to keep.
            NodeEvent::ClipboardReceived { .. } | NodeEvent::PhotoAdded { .. } | NodeEvent::Call { .. } => {
                Changes::NONE
            }
            NodeEvent::Transfer(transfer) => self.update_transfer(transfer.clone(), Instant::now()),
            // With artwork saved first (see crate::media).
            NodeEvent::MediaChanged { .. } => Changes::NONE,
        }
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
        match self.transfers.iter_mut().find(|t| t.transfer.id == transfer.id) {
            Some(view) => {
                let elapsed = now.saturating_duration_since(view.sampled).as_secs_f64();
                let running = transfer.state == TransferState::Running;
                if running && elapsed > 0.0 && transfer.done >= view.transfer.done {
                    let instant = (transfer.done - view.transfer.done) as f64 / elapsed;
                    // Smoothed, so the number doesn't jump around.
                    view.rate = if view.rate == 0.0 { instant } else { view.rate * 0.7 + instant * 0.3 };
                } else if !running {
                    view.rate = 0.0;
                }
                view.sampled = now;
                view.transfer = transfer;
            }
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
        Changes::TRANSFERS
    }

    /// Forgets finished transfers.
    pub fn clear_finished_transfers(&mut self) -> Changes {
        let before = self.transfers.len();
        self.transfers.retain(|t| !t.transfer.state.is_finished());
        if self.transfers.len() == before { Changes::NONE } else { Changes::TRANSFERS }
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

    /// Adds a notification in time order (newest first), without its icon.
    fn insert_notification(&mut self, device: DeviceId, mut notification: Notification) -> Changes {
        // Icons and pictures live on disk (see crate::notifications).
        notification.icon = None;
        notification.image = None;
        let mut changes = Changes::NONE;
        if self.app_names.get(&notification.app) != Some(&notification.app_name) {
            self.app_names.insert(notification.app.clone(), notification.app_name.clone());
            changes |= Changes::APPS;
        }
        let at = self.notifications.partition_point(|n| n.notification.when >= notification.when);
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
        }
        Changes::HISTORY
    }

    pub fn clear_history(&mut self) -> Changes {
        if self.history.is_empty() {
            return Changes::NONE;
        }
        self.history.clear();
        Changes::HISTORY
    }

    /// Records where a notification's picture is stored.
    pub fn set_notification_image(&mut self, key: (DeviceId, String), path: PathBuf) -> Changes {
        if self.notification_images.get(&key) == Some(&path) {
            return Changes::NONE;
        }
        self.notification_images.insert(key, path);
        Changes::NOTIFICATIONS
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

    pub fn set_pairing(&mut self, next: PairingView) -> Changes {
        if self.pairing == next {
            return Changes::NONE;
        }
        self.pairing = next;
        Changes::PAIRING
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
            },
            paired_at: at,
            link: LinkState::Offline { last_seen: None },
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
        assert_eq!(s.apply(&NodeEvent::LinkChanged { device: id, link: online.clone() }), Changes::DEVICES);
        assert_eq!(
            s.apply(&NodeEvent::LinkChanged { device: id, link: online }),
            Changes::NONE,
            "no change, no refresh"
        );
        let battery = Battery { level: 50, charging: false, plugged: None };
        s.apply(&NodeEvent::Battery { device: id, battery: battery.clone() });
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
            icon: Some(vec![1, 2, 3]),
            image: None,
        }
    }

    fn keys(s: &AppState) -> Vec<&str> {
        s.notifications.iter().map(|n| n.notification.key.as_str()).collect()
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
            s.update_transfer(transfer(&i.to_string(), 1000, TransferState::Cancelled), t0);
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
}
