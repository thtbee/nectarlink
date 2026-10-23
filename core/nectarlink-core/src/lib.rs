// SPDX-License-Identifier: MPL-2.0
//! The NectarLink engine: device identity, pairing, sessions with paired
//! devices, and storage. Shared by the Windows and Android apps.
//!
//! The entry point is [`Node`]. Commands are async methods; everything that
//! changes is reported as a [`NodeEvent`]. See
//! `docs/architecture/core-api.md` for the design.

mod actions;
mod calls;
pub mod clip_kind;
mod clipboard;
pub mod clipboard_history;
mod config;
mod contacts;
pub mod continuity_camera;
pub mod deck;
mod error;
mod events;
pub mod features;
pub mod handoff;
mod identity;
pub mod localsend;
mod media;
pub mod mic;
mod mirror;
mod node;
mod notifications;
pub mod otp;
mod pairing;
pub mod palette;
mod photos;
pub mod play_on_phone;
pub mod remote;
mod session;
mod sms;
pub mod storage;
mod store;
pub mod timeline;
pub mod toggles;
mod transfer;
pub mod unlock;
pub mod usb;
pub mod webcam;

pub use actions::{PC_WAKE, PowerAction};
pub use calls::CallCommand;
pub use clip_kind::{
    ClipKind, ClipSuggestion, classify_clip, maps_web_url, percent_encode, tracking_search_url,
};
pub use clipboard_history::{
    ClipboardHistoryEntry, ClipboardItemKind, MAX_CLIPBOARD_AGE_SECS, MAX_CLIPBOARD_HISTORY,
};
pub use config::{NodeConfig, NoopPlatform, Platform};
pub use deck::{
    COMMANDS_TOGGLE as DECK_COMMANDS_TOGGLE, DECK_ACTIONS, DeckAction, DeckConfig, DeckLayout, DeckPage,
    DeckPageConfig, DeckPress, DeckState, DeckTile, DeckTileConfig, INPUT_TOGGLE as DECK_INPUT_TOGGLE,
    MAX_DECK_ID_BYTES, MAX_DECK_LABEL_BYTES, MAX_DECK_PAGES, MAX_DECK_TILES_PER_PAGE, PC_AUDIO, deck_colors,
    deck_icons, deck_kinds, format_shortcut, is_valid_app_path, is_valid_command, is_valid_deck_id,
    is_valid_http_url, is_valid_shortcut, is_valid_snippet,
};
pub use error::{Error, Result, Side};
pub use events::{
    ConnectionPath, DiscoveredDevice, LinkState, NodeEvent, PairedDevice, PairingEvent, PairingFailure,
};
pub use features::{CapabilityMatrix, FeatureState};
pub use handoff::{
    HandoffKind, HandoffLink, address_to_geo_uri, extract_handoff_link, format_video_timestamp,
    geo_to_maps_https, is_safe_handoff_document, parse_video_timestamp, timeline_link_title_and_detail,
    with_video_timestamp,
};
pub use identity::{KeyProtector, PlainKeyProtector, default_protector, install_id_from};
pub use localsend::{
    LOCALSEND_DEFAULT_PORT, LOCALSEND_MULTICAST_ADDR, LOCALSEND_PROTOCOL_VERSION, LocalSendPeer,
    peer_device_id as localsend_peer_device_id,
};
pub use media::{MediaAction, MediaError};
pub use mic::{MicSink, TOGGLE as MIC_TOGGLE};
pub use mirror::{MirrorSend, MirrorSink, MirrorStream, PC_MIRROR_TOGGLE};
pub use nectarlink_protocol::messages::clip::{
    IMAGE_TYPES as CLIP_IMAGE_TYPES, MAX_IMAGE_BYTES as CLIP_MAX_IMAGE_BYTES,
};
pub use nectarlink_protocol::{
    DeviceId, PacketKind,
    messages::{
        AudioOutputDevice, Battery, CLIP_MAX_BYTES, CallControls, CallLogEntry, CallState,
        CameraCaptureCancel, CameraCaptureMode, CameraCaptureOk, CameraCaptureRequest,
        CameraCaptureResultMeta, Contact, ContactNumber, DeviceInfo, DeviceKind, LivePoint, LiveSegment,
        MediaPlayer, MirrorAudioConfig, MirrorConfig, MirrorInput, MirrorPower, MirrorResize, MirrorStart,
        Notification, NotificationAction, NotificationChatMessage, NotificationConversation,
        NotificationLive, PcAudioSet, PcWakeInfo, PhoneApp, PhoneToggleSet, PhoneToggleValue, PhoneToggles,
        PhotoAlbum, PhotoItem, PhotoNew as Photo, PhotoThumb, PowerLevel, ScreenCorners, ScreenRect,
        ScreenShape, SmsAttachment, SmsMessage, SmsPart, SmsThread, StorageChanged, StorageDelete,
        StorageEntries, StorageEntry, StorageList, StorageMkdir, StorageRead, StorageReadMeta, StorageRename,
        StorageWriteAccept, StorageWriteDone, StorageWriteOffer, TaskNotify, TouchAction, WatchFile,
        WebcamConfig, WebcamStart,
        calls::{
            CONTROL as CALLS_CONTROL, DIAL as CALLS_DIAL, IN_CALL as CALLS_IN_CALL, LOG as CALLS_LOG,
            SHOW as CALLS_SHOW, STATE as CALLS_STATE,
        },
        contacts::{READ as CONTACTS_READ, SHOW as CONTACTS_SHOW},
        continuity_camera::{
            CAPTURE as CAMERA_CAPTURE, MAX_IMAGE_BYTES as CAMERA_CAPTURE_MAX_IMAGE_BYTES,
            RECEIVE as CAMERA_CONTINUITY,
        },
        format_mac, ipv4_broadcast, is_package_name, is_valid_storage_dir_path, is_valid_storage_id,
        is_valid_storage_name, is_valid_storage_path, magic_packet,
        media_watch::{MAX_READ as MEDIA_WATCH_MAX_READ, STREAM as MEDIA_STREAM, WATCH as MEDIA_WATCH},
        mic::{LISTEN as MIC_LISTEN, STREAM as MIC_STREAM},
        mirror::{
            AUDIO as MIRROR_AUDIO, AUDIO_PLAYBACK as MIRROR_AUDIO_PLAYBACK, AV1 as MIRROR_AV1,
            CAPTURE as MIRROR_CAPTURE, H264 as MIRROR_H264, H265 as MIRROR_H265, INPUT as MIRROR_INPUT,
            LISTEN as MIRROR_LISTEN, MAX_TEXT_BYTES as MIRROR_MAX_TEXT_BYTES, PCM as MIRROR_PCM,
            SCREEN as MIRROR_SCREEN, VIEW as MIRROR_VIEW, VIRTUAL_DISPLAY as MIRROR_VIRTUAL_DISPLAY,
        },
        mirror_keys, notify_limits, parse_mac,
        photos::{
            MAX_GET_ITEMS as PHOTO_MAX_GET_ITEMS, MAX_PAGE as PHOTO_MAX_PAGE,
            MAX_THUMB_BATCH as PHOTO_MAX_THUMB_BATCH, MAX_THUMB_BYTES as PHOTO_MAX_THUMB_BYTES,
            READ as PHOTOS_READ, SHOW as PHOTOS_SHOW,
        },
        ringer_modes, screen_limits,
        sms::{
            MAX_ATTACHMENT_BYTES as SMS_MAX_ATTACHMENT_BYTES, MAX_ATTACHMENTS as SMS_MAX_ATTACHMENTS,
            READ as SMS_READ, SEND as SMS_SEND, SHOW as SMS_SHOW,
        },
        storage::{MOUNT as STORAGE_MOUNT, READ as STORAGE_READ, WRITE as STORAGE_WRITE},
        task_notify, toggle_ids,
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
pub use node::{DataRetentionCounts, Node, PeerDiagnostics};
pub use notifications::{NotificationError, SHOW as NOTIFY_SHOW};
pub use palette::{
    ACHROMATIC_SEED_RGB, MAX_SAMPLE_SIDE, Palette as ThemePaletteMap, RolePalette, ThemeScheme,
    achromatic_scheme, downsample_rgb, downsample_rgba, extract_artwork_scheme_rgb,
    extract_artwork_scheme_rgba, scheme_from_seed_rgb, scheme_json, seed_from_grid, seed_from_pixels,
};
pub use play_on_phone::PlayOnPhone;
pub use remote::{ButtonAction, INPUT_INJECT, KeyMod, MouseButton, RemoteInput, SlideAction, remote_keys};
pub use storage::{FolderStorage, StorageError, StorageReadFile, TOGGLE as STORAGE_TOGGLE};
pub use store::{
    CHAT_RETENTION_SECS, ChatMessageRecord, ChatThreadRecord, MAX_CHAT_MESSAGES_PER_THREAD,
    MAX_CHAT_THREADS_PER_PEER, MAX_QUEUE_ACTIVE_BYTES, MAX_QUEUE_ACTIVE_ITEMS,
    MAX_QUEUE_CLIPBOARD_IMAGE_BYTES, QueueItem, QueueKind, QueuePayload, QueueStatus, QueuedFileSpec,
    chat_thread_id, file_mtime_ms,
};
pub use timeline::{
    DEFAULT_TIMELINE_MAX_DAYS, DEFAULT_TIMELINE_MAX_ENTRIES, DEFAULT_TIMELINE_PAGE_LIMIT,
    MAX_TIMELINE_PAGE_LIMIT, SessionKind, TimelineEntry, TimelineKind, TimelinePage, TimelineQuery,
    TimelineRetention, format_clip_display,
};
pub use transfer::{
    Direction, FileSource, OutgoingFile, RECORDER, RecordingMarker, Transfer, TransferFailure, TransferState,
    outgoing_paths, safe_file_name,
};
pub use usb::{DEFAULT_ADB_ADDR, DEFAULT_USB_PORT, USB_PORT_COUNT};
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

pub(crate) fn now_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

/// On Android: lets the network code read the phone's own DNS servers
/// (through Java), instead of falling back to public ones that some
/// networks block. Call once, before any [`Node`] starts.
///
/// # Safety
///
/// `java_vm` must be the process's `JavaVM` and `context` a JNI global
/// reference to the application `Context`; both must stay valid until the
/// process exits.
#[cfg(target_os = "android")]
#[allow(unsafe_code)]
pub unsafe fn install_android_context(java_vm: *mut std::ffi::c_void, context: *mut std::ffi::c_void) {
    // SAFETY: as this function's contract says.
    unsafe { iroh::dns::install_android_jni_context(java_vm, context) }
}
