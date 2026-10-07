// SPDX-License-Identifier: MPL-2.0
//! The Nectarlink engine: device identity, pairing, sessions with paired
//! devices, and storage. Shared by the Windows and Android apps.
//!
//! The entry point is [`Node`]. Commands are async methods; everything that
//! changes is reported as a [`NodeEvent`]. See
//! `docs/architecture/core-api.md` for the design.

mod actions;
mod calls;
mod clipboard;
mod config;
mod contacts;
mod error;
mod events;
pub mod features;
mod identity;
mod media;
mod mirror;
mod node;
mod notifications;
mod pairing;
mod photos;
mod session;
mod sms;
mod store;
mod transfer;

pub use actions::PowerAction;
pub use calls::CallCommand;
pub use config::{NodeConfig, NoopPlatform, Platform};
pub use error::{Error, Result, Side};
pub use events::{
    ConnectionPath, DiscoveredDevice, LinkState, NodeEvent, PairedDevice, PairingEvent, PairingFailure,
};
pub use features::{CapabilityMatrix, FeatureState};
pub use identity::{KeyProtector, PlainKeyProtector, default_protector};
pub use media::{MediaAction, MediaError};
pub use mirror::{MirrorSend, MirrorSink, MirrorStream};
pub use nectarlink_protocol::messages::clip::{
    IMAGE_TYPES as CLIP_IMAGE_TYPES, MAX_IMAGE_BYTES as CLIP_MAX_IMAGE_BYTES,
};
pub use nectarlink_protocol::{
    DeviceId, PacketKind,
    messages::{
        Battery, CLIP_MAX_BYTES, CallControls, CallLogEntry, CallState, Contact, ContactNumber, DeviceInfo,
        DeviceKind, MediaPlayer, MirrorAudioConfig, MirrorConfig, MirrorInput, MirrorStart, Notification,
        NotificationAction, PhoneApp, PhotoNew as Photo, PowerLevel, SmsMessage, SmsPart, SmsThread,
        TouchAction,
        calls::{
            CONTROL as CALLS_CONTROL, DIAL as CALLS_DIAL, IN_CALL as CALLS_IN_CALL, LOG as CALLS_LOG,
            SHOW as CALLS_SHOW, STATE as CALLS_STATE,
        },
        contacts::{READ as CONTACTS_READ, SHOW as CONTACTS_SHOW},
        is_package_name,
        mirror::{
            AUDIO as MIRROR_AUDIO, AUDIO_PLAYBACK as MIRROR_AUDIO_PLAYBACK, CAPTURE as MIRROR_CAPTURE,
            INPUT as MIRROR_INPUT, LISTEN as MIRROR_LISTEN, MAX_TEXT_BYTES as MIRROR_MAX_TEXT_BYTES,
            PCM as MIRROR_PCM, SCREEN as MIRROR_SCREEN, VIEW as MIRROR_VIEW,
            VIRTUAL_DISPLAY as MIRROR_VIRTUAL_DISPLAY,
        },
        mirror_keys,
        photos::{MAX_THUMB_BYTES as PHOTO_MAX_THUMB_BYTES, READ as PHOTOS_READ, SHOW as PHOTOS_SHOW},
        sms::{READ as SMS_READ, SEND as SMS_SEND, SHOW as SMS_SHOW},
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
