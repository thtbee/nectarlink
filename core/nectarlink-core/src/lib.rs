// SPDX-License-Identifier: MPL-2.0
//! The Nectarlink engine: device identity, pairing, sessions with paired
//! devices, and storage. Shared by the Windows and Android apps.
//!
//! The entry point is [`Node`]. Commands are async methods; everything that
//! changes is reported as a [`NodeEvent`]. See
//! `docs/architecture/core-api.md` for the design.

mod actions;
mod clipboard;
mod config;
mod error;
mod events;
pub mod features;
mod identity;
mod media;
mod node;
mod notifications;
mod pairing;
mod session;
mod store;
mod transfer;

pub use actions::PowerAction;
pub use config::{NodeConfig, NoopPlatform, Platform};
pub use error::{Error, Result, Side};
pub use events::{
    ConnectionPath, DiscoveredDevice, LinkState, NodeEvent, PairedDevice, PairingEvent, PairingFailure,
};
pub use features::{CapabilityMatrix, FeatureState};
pub use identity::{KeyProtector, PlainKeyProtector, default_protector};
pub use media::{MediaAction, MediaError};
pub use nectarlink_protocol::messages::clip::{
    IMAGE_TYPES as CLIP_IMAGE_TYPES, MAX_IMAGE_BYTES as CLIP_MAX_IMAGE_BYTES,
};
pub use nectarlink_protocol::{
    DeviceId,
    messages::{
        Battery, CLIP_MAX_BYTES, DeviceInfo, DeviceKind, MediaPlayer, Notification, NotificationAction,
        PowerLevel,
    },
    pairing::PairingUri,
};
pub use node::Node;
pub use notifications::NotificationError;
pub use transfer::{
    Direction, FileSource, OutgoingFile, Transfer, TransferFailure, TransferState, outgoing_paths,
    safe_file_name,
};

pub(crate) fn device_id(key: &iroh::PublicKey) -> DeviceId {
    DeviceId(*key.as_bytes())
}

pub(crate) fn public_key(id: &DeviceId) -> Result<iroh::PublicKey> {
    iroh::PublicKey::from_bytes(id.as_bytes()).map_err(|_| Error::Protocol("invalid device ID".into()))
}

pub(crate) fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}
