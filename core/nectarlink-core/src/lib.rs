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
pub mod clipboard_history;
mod config;
mod contacts;
pub mod deck;
mod error;
mod events;
pub mod features;
mod identity;
mod media;
mod mirror;
mod node;
mod notifications;
pub mod otp;
mod pairing;
mod photos;
pub mod remote;
mod session;
mod sms;
pub mod storage;
mod store;
pub mod toggles;
mod transfer;
pub mod webcam;

pub use actions::{PC_WAKE, PowerAction};
pub use calls::CallCommand;
pub use clipboard_history::{ClipboardHistoryEntry, ClipboardItemKind, MAX_CLIPBOARD_HISTORY};
pub use config::{NodeConfig, NoopPlatform, Platform};
pub use deck::{
    COMMANDS_TOGGLE as DECK_COMMANDS_TOGGLE, DECK_ACTIONS, DeckAction, DeckConfig, DeckLayout, DeckPage,
    DeckPageConfig, DeckPress, DeckState, DeckTile, DeckTileConfig, INPUT_TOGGLE as DECK_INPUT_TOGGLE,
    MAX_DECK_ID_BYTES, MAX_DECK_LABEL_BYTES, MAX_DECK_PAGES, MAX_DECK_TILES_PER_PAGE, deck_colors,
    deck_icons, deck_kinds, format_shortcut, is_valid_app_path, is_valid_command, is_valid_deck_id,
    is_valid_http_url, is_valid_shortcut, is_valid_snippet,
};
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
        DeviceKind, MediaPlayer, MirrorAudioConfig, MirrorConfig, MirrorInput, MirrorResize, MirrorStart,
        Notification, NotificationAction, PcWakeInfo, PhoneApp, PhoneToggleSet, PhoneToggleValue,
        PhoneToggles, PhotoAlbum, PhotoItem, PhotoNew as Photo, PhotoThumb, PowerLevel, SmsMessage, SmsPart,
        SmsThread, StorageChanged, StorageDelete, StorageEntries, StorageEntry, StorageList, StorageMkdir,
        StorageRead, StorageReadMeta, StorageRename, StorageWriteAccept, StorageWriteDone, StorageWriteOffer,
        TouchAction, WebcamConfig, WebcamStart,
        calls::{
            CONTROL as CALLS_CONTROL, DIAL as CALLS_DIAL, IN_CALL as CALLS_IN_CALL, LOG as CALLS_LOG,
            SHOW as CALLS_SHOW, STATE as CALLS_STATE,
        },
        contacts::{READ as CONTACTS_READ, SHOW as CONTACTS_SHOW},
        format_mac, ipv4_broadcast, is_package_name, is_valid_storage_dir_path, is_valid_storage_id,
        is_valid_storage_name, is_valid_storage_path, magic_packet,
        mirror::{
            AUDIO as MIRROR_AUDIO, AUDIO_PLAYBACK as MIRROR_AUDIO_PLAYBACK, CAPTURE as MIRROR_CAPTURE,
            INPUT as MIRROR_INPUT, LISTEN as MIRROR_LISTEN, MAX_TEXT_BYTES as MIRROR_MAX_TEXT_BYTES,
            PCM as MIRROR_PCM, SCREEN as MIRROR_SCREEN, VIEW as MIRROR_VIEW,
            VIRTUAL_DISPLAY as MIRROR_VIRTUAL_DISPLAY,
        },
        mirror_keys, parse_mac,
        photos::{
            MAX_GET_ITEMS as PHOTO_MAX_GET_ITEMS, MAX_PAGE as PHOTO_MAX_PAGE,
            MAX_THUMB_BATCH as PHOTO_MAX_THUMB_BATCH, MAX_THUMB_BYTES as PHOTO_MAX_THUMB_BYTES,
            READ as PHOTOS_READ, SHOW as PHOTOS_SHOW,
        },
        ringer_modes,
        sms::{READ as SMS_READ, SEND as SMS_SEND, SHOW as SMS_SHOW},
        storage::{MOUNT as STORAGE_MOUNT, READ as STORAGE_READ, WRITE as STORAGE_WRITE},
        toggle_ids,
        toggles::{
            BLUETOOTH as TOGGLES_BLUETOOTH, BRIGHTNESS as TOGGLES_BRIGHTNESS, DND as TOGGLES_DND,
            FLASHLIGHT as TOGGLES_FLASHLIGHT, READ as TOGGLES_READ, RINGER as TOGGLES_RINGER,
            SHOW as TOGGLES_SHOW, VOLUME as TOGGLES_VOLUME, WIFI as TOGGLES_WIFI,
        },
        webcam::{
            ADDON_VCAM as WEBCAM_ADDON_VCAM, CAMERA_BACK as WEBCAM_CAMERA_BACK,
            CAMERA_FRONT as WEBCAM_CAMERA_FRONT, H264 as WEBCAM_H264, STREAM as WEBCAM_STREAM,
            VIRTUAL as WEBCAM_VIRTUAL,
        },
    },
    pairing::PairingUri,
};
pub use node::Node;
pub use notifications::NotificationError;
pub use remote::{ButtonAction, INPUT_INJECT, KeyMod, MouseButton, RemoteInput, SlideAction, remote_keys};
pub use storage::{FolderStorage, StorageError, StorageReadFile, TOGGLE as STORAGE_TOGGLE};
pub use transfer::{
    Direction, FileSource, OutgoingFile, RECORDER, RecordingMarker, Transfer, TransferFailure, TransferState,
    outgoing_paths, safe_file_name,
};
pub use webcam::{TOGGLE as WEBCAM_TOGGLE, WebcamSink};

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
