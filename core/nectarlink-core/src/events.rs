// SPDX-License-Identifier: MPL-2.0
//! Events the core pushes to the UI. UIs render from these; they never poll.

use std::path::PathBuf;

use nectarlink_protocol::{
    DeviceId,
    messages::{Battery, CameraCaptureMode, DeviceInfo, MediaPlayer, Notification, PowerLevel},
};

use crate::{features::CapabilityMatrix, transfer::Transfer};

/// How a session is currently carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionPath {
    /// A direct connection on the local network.
    Lan,
    /// Through a relay server (away mode).
    Relay,
}

/// Connection state of a paired device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkState {
    Offline { last_seen: Option<i64> },
    Connecting,
    Online { path: ConnectionPath, rtt_ms: u32 },
}

/// A device this device is paired with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairedDevice {
    pub id: DeviceId,
    pub info: DeviceInfo,
    /// Unix seconds.
    pub paired_at: i64,
    pub link: LinkState,
    /// Whether a stored Wake-on-LAN address (`pc.wake_info`) is available
    /// for this device.
    pub can_wake: bool,
}

/// A device found on the local network that could be paired with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredDevice {
    pub id: DeviceId,
    /// The name it announces, if any.
    pub name: Option<String>,
}

/// Why pairing failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairingFailure {
    /// The pairing code, proof or confirmation didn't match.
    Rejected,
    /// The user (on either device) declined.
    Declined,
    /// Pairing mode expired before anyone connected.
    Expired,
    /// The other device couldn't be reached.
    Unreachable,
    /// Anything else; the string is for logs.
    Other(String),
}

/// Progress of a pairing ceremony.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairingEvent {
    /// Show this 6-digit code and ask the user to confirm it matches.
    SasCode {
        peer: DeviceId,
        code: String,
    },
    Paired(PairedDevice),
    Failed(PairingFailure),
}

/// Everything the UI needs to know about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeEvent {
    DeviceAdded(PairedDevice),
    DeviceRemoved(DeviceId),
    LinkChanged {
        device: DeviceId,
        link: LinkState,
    },
    PeerInfoChanged {
        device: DeviceId,
        info: DeviceInfo,
    },
    PeerPowerChanged {
        device: DeviceId,
        power: PowerLevel,
    },
    /// What works with a device changed (see `docs/architecture/capabilities.md`).
    Capabilities(CapabilityMatrix),
    Battery {
        device: DeviceId,
        battery: Battery,
    },
    Ring {
        device: DeviceId,
        on: bool,
    },
    Discovered(DiscoveredDevice),
    DiscoveryExpired(DeviceId),
    Pairing(PairingEvent),
    /// Everything a phone shows now, replacing what was known before (no
    /// alerts for these). Empty when the phone stopped sharing them.
    NotificationsReset {
        device: DeviceId,
        items: Vec<Notification>,
    },
    /// A new notification, or an update of the one with the same key.
    NotificationPosted {
        device: DeviceId,
        notification: Notification,
    },
    NotificationRemoved {
        device: DeviceId,
        key: String,
    },
    /// A device put text on this device's clipboard.
    ClipboardReceived {
        device: DeviceId,
    },
    /// The local clipboard history changed (a clip was added, pinned, deleted, or cleared).
    ClipboardHistoryChanged,
    /// A file transfer started, progressed or finished.
    Transfer(Transfer),
    /// A device's media players changed (all of them; empty when nothing
    /// plays there, or the user turned media off for it). Artwork is
    /// attached when it's new for this session; keep it by `art_key`.
    MediaChanged {
        device: DeviceId,
        players: Vec<MediaPlayer>,
    },
    /// A phone took a photo or screenshot (with a preview); fetch it with
    /// [`Node::fetch_photo`](crate::Node::fetch_photo).
    PhotoAdded {
        device: DeviceId,
        photo: crate::Photo,
    },
    /// A phone's photo or video library changed.
    PhotosChanged {
        device: DeviceId,
    },
    /// A call on a phone rang, was answered or ended.
    Call {
        device: DeviceId,
        call: crate::CallState,
    },
    /// A phone's call history changed.
    CallLogChanged {
        device: DeviceId,
    },
    /// A phone's contacts changed.
    ContactsChanged {
        device: DeviceId,
    },
    /// A phone's screen started or stopped showing here.
    Mirroring {
        device: DeviceId,
        /// Which mirroring: 0 for the screen, others for app windows.
        session: u32,
        on: bool,
    },
    /// A phone's messages changed: in `thread`, or anywhere when `None`.
    SmsChanged {
        device: DeviceId,
        thread: Option<String>,
    },
    /// A paired phone asked to control this PC's mouse and keyboard while its
    /// `remote_input` toggle is off (emitted once per phone so the UI can show
    /// a one-time prompt).
    RemoteInputRequested {
        device: DeviceId,
    },
    /// A paired phone's quick settings state (on connect and on change).
    PhoneToggles {
        device: DeviceId,
        toggles: crate::PhoneToggles,
    },
    /// A paired PC's stored Wake-on-LAN info arrived, changed, or was cleared.
    WakeInfoChanged {
        device: DeviceId,
        can_wake: bool,
    },
    /// A paired PC's Deck layout arrived or changed (`docs/protocol/deck.md`).
    DeckLayout {
        device: DeviceId,
        layout: crate::DeckLayout,
    },
    /// A paired PC's live Deck state arrived or changed (`docs/protocol/deck.md`).
    DeckState {
        device: DeviceId,
        state: crate::DeckState,
    },
    /// A paired PC asked to access this phone's storage while its `storage`
    /// toggle is off (emitted once per PC so the phone UI can prompt the user).
    StorageRequested {
        device: DeviceId,
    },
    /// A folder in a paired phone's shared storage changed (`docs/protocol/storage.md`).
    StorageChanged {
        device: DeviceId,
        path: String,
    },
    /// A paired phone's webcam stream started or stopped arriving on this PC.
    Webcam {
        device: DeviceId,
        on: bool,
    },
    /// A paired PC requested a Continuity Camera capture on this phone.
    CameraCaptureRequested {
        device: DeviceId,
        request_id: String,
        mode: CameraCaptureMode,
    },
    /// A paired phone completed a Continuity Camera capture and sent the image to this PC.
    CameraCaptureReceived {
        device: DeviceId,
        request_id: String,
        mode: CameraCaptureMode,
        file_name: String,
        mime: String,
        width: u32,
        height: u32,
        data: Vec<u8>,
        saved_path: Option<PathBuf>,
    },
    /// A Continuity Camera capture was cancelled by the peer.
    CameraCaptureCancelled {
        device: DeviceId,
        request_id: String,
        reason: Option<String>,
    },
    /// The local timeline of items shared with paired devices changed.
    TimelineChanged,
    /// LocalSend interop state or discovered LocalSend peers changed.
    LocalSendChanged,
}
