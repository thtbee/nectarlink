// SPDX-License-Identifier: MPL-2.0
//! The phase 2 actions: the phone's quick settings, photos and files, app
//! windows, power and wake for PCs, the Deck, the Timeline, LocalSend and
//! waiting for events. Same rules as `handlers`: checked arguments in, plain
//! JSON out, outside text through `untrusted`.
//!
//! Pictures go out as `_images: [{ mime, data }]` (base64); the MCP server
//! turns them into image content, HTTP and CLI callers get the JSON.

use std::{
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

use nectarlink_core::{PhoneToggleSet, PhoneToggleValue, PowerAction, TimelineKind, TimelineQuery};
use nectarlink_protocol::messages::{is_valid_storage_path, toggle_ids};
use schemars::JsonSchema;
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};

use crate::{
    def::{Access, Grant, HandlerFuture, Scope},
    error::{ActionError, Code},
    events::{self, Event},
    gateway::Ctx,
    host::UNSUPPORTED,
    untrusted::{FIELD_MAX, clean, clean_message, clean_opt},
};

/// The most picture data one answer carries, before base64.
const IMAGE_BUDGET: usize = 3 * 1024 * 1024;
/// The most text `storage.read` returns.
const TEXT_MAX: u64 = 256 * 1024;
/// The largest phone file `storage.save` copies to the PC.
const SAVE_MAX: u64 = 2 * 1024 * 1024 * 1024;
const SAVE_CHUNK: u64 = 4 * 1024 * 1024;

fn parse<T: DeserializeOwned>(args: Value) -> Result<T, ActionError> {
    serde_json::from_value(args).map_err(|e| ActionError::invalid(format!("Invalid arguments: {e}")))
}

fn ok() -> Value {
    json!({ "ok": true })
}

/// A host error, as `Unsupported` when the host can't do it at all.
fn host_err(e: String) -> ActionError {
    if e == UNSUPPORTED { ActionError::new(Code::Unsupported, e) } else { ActionError::internal(e) }
}

fn image(mime: &str, data: &[u8]) -> Value {
    json!({ "mime": mime, "data": data_encoding::BASE64.encode(data) })
}

macro_rules! handler {
    ($name:ident, $input:ty, |$ctx:ident, $a:ident| $body:expr) => {
        pub fn $name(ctx: Ctx, args: Value) -> HandlerFuture {
            Box::pin(async move {
                let $ctx = ctx;
                let $a: $input = parse(args)?;
                $body
            })
        }
    };
}

// ---- Inputs ----

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PhoneSetting {
    Dnd,
    Ringer,
    Flashlight,
    Volume,
    Brightness,
    Wifi,
    Bluetooth,
    Hotspot,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PhoneSettingArgs {
    pub device: Option<String>,
    pub setting: PhoneSetting,
    /// For dnd, flashlight, wifi, bluetooth and hotspot.
    pub on: Option<bool>,
    /// For volume and brightness, 0 to 100.
    #[schemars(range(min = 0, max = 100))]
    pub level: Option<u8>,
    /// For ringer: ring, vibrate or silent.
    pub mode: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PhotoListArgs {
    pub device: Option<String>,
    /// An album `id` from photos_albums; all photos and videos when left out.
    pub album: Option<String>,
    /// For the next page: the `date` and `id` of the last item you got.
    pub before_date: Option<i64>,
    pub before_id: Option<String>,
    #[schemars(range(min = 1, max = 100))]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PhotoIdsArgs {
    pub device: Option<String>,
    /// Item `id`s from photos_list.
    #[schemars(length(min = 1, max = 50))]
    pub ids: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ThumbArgs {
    pub device: Option<String>,
    /// Item `id`s from photos_list, 1 to 12.
    #[schemars(length(min = 1, max = 12))]
    pub ids: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StoragePathArgs {
    pub device: Option<String>,
    /// A path inside the phone's shared storage, like `Download/notes.txt`,
    /// with `/` between folders. Empty for the top.
    pub path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StorageReadArgs {
    pub device: Option<String>,
    pub path: String,
    /// Where to start, in bytes.
    pub offset: Option<u64>,
    /// How much to read, in bytes, up to 262144 for text.
    pub length: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StorageUploadArgs {
    pub device: Option<String>,
    /// An absolute path of a file on this PC.
    pub source: String,
    /// Where it goes on the phone, folder and name, like `Download/report.pdf`.
    pub path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StorageRenameArgs {
    pub device: Option<String>,
    pub from: String,
    /// The new path, which may be in another folder.
    pub to: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PowerKind {
    Lock,
    Sleep,
    Hibernate,
    Shutdown,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PowerArgs {
    /// A paired PC. Leave it out for this PC.
    pub device: Option<String>,
    pub action: PowerKind,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PcAudioArgs {
    /// A paired PC.
    pub device: Option<String>,
    /// 0 to 100.
    #[schemars(range(min = 0, max = 100))]
    pub volume: Option<u8>,
    pub muted: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MirrorOpenArgs {
    pub device: Option<String>,
    /// An app's `package` from mirror_apps. Leave it out for the phone's whole screen.
    pub app: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AppArgs {
    pub device: Option<String>,
    /// The app's `package`, from mirror_apps.
    pub app: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TileArgs {
    /// The tile's `id`, from deck_tiles.
    pub tile: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TimelineArgs {
    /// Only one device's entries.
    pub device: Option<String>,
    /// file, clip, link, photo, recording or session.
    pub kind: Option<String>,
    pub query: Option<String>,
    #[schemars(range(min = 1, max = 100))]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TransferIdArgs {
    /// The transfer's `id`, from transfers_list or a transfer.started event.
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SmsPartArgs {
    pub device: Option<String>,
    /// An attachment's `id`, from the `attachments` of sms_messages.
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EventsArgs {
    /// The `seq` of the last event you got; leave it out the first time to
    /// get only what happens from now on.
    pub since: Option<u64>,
    /// Only these kinds, like notification.posted or transfer.finished.
    pub kinds: Option<Vec<String>>,
    /// How long to wait for the first event, 0 to 50 seconds. 0 answers at once.
    #[schemars(range(min = 0, max = 50))]
    pub wait_s: Option<u64>,
    #[schemars(range(min = 1, max = 100))]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoArgs {}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeviceOnly {
    pub device: Option<String>,
}

fn limit(l: Option<u32>) -> u32 {
    l.unwrap_or(20).clamp(1, 100)
}

// ---- Phone settings ----

handler!(phone_settings_get, DeviceOnly, |ctx, _a| {
    let toggles = ctx.node()?.phone_toggles(ctx.device()?).ok_or_else(|| {
        ActionError::new(Code::Unsupported, "The phone hasn't shared its quick settings with this PC yet.")
            .user_action(
                "Make sure the phone is connected and Quick settings is on for this PC on the phone.",
            )
    })?;
    Ok(serde_json::to_value(toggles).unwrap_or_default())
});

handler!(phone_settings_set, PhoneSettingArgs, |ctx, a| {
    let need_on = || a.on.ok_or_else(|| ActionError::invalid("This setting needs `on`."));
    let (id, value) = match a.setting {
        PhoneSetting::Dnd => (toggle_ids::DND, PhoneToggleValue::Bool(need_on()?)),
        PhoneSetting::Flashlight => (toggle_ids::FLASHLIGHT, PhoneToggleValue::Bool(need_on()?)),
        PhoneSetting::Wifi => (toggle_ids::WIFI, PhoneToggleValue::Bool(need_on()?)),
        PhoneSetting::Bluetooth => (toggle_ids::BLUETOOTH, PhoneToggleValue::Bool(need_on()?)),
        PhoneSetting::Hotspot => (toggle_ids::HOTSPOT, PhoneToggleValue::Bool(need_on()?)),
        PhoneSetting::Volume | PhoneSetting::Brightness => {
            let level = a
                .level
                .filter(|l| *l <= 100)
                .ok_or_else(|| ActionError::invalid("This setting needs `level`, 0 to 100."))?;
            let id = if matches!(a.setting, PhoneSetting::Volume) {
                toggle_ids::VOLUME
            } else {
                toggle_ids::BRIGHTNESS
            };
            (id, PhoneToggleValue::Level(level))
        }
        PhoneSetting::Ringer => {
            let mode = a.mode.unwrap_or_default();
            if !matches!(mode.as_str(), "ring" | "vibrate" | "silent") {
                return Err(ActionError::invalid("ringer needs `mode`: ring, vibrate or silent."));
            }
            (toggle_ids::RINGER, PhoneToggleValue::Mode(mode))
        }
    };
    let set = PhoneToggleSet { id: id.to_owned(), value };
    if !set.is_valid() {
        return Err(ActionError::invalid("That value doesn't fit this setting."));
    }
    ctx.node()?.set_phone_toggle(ctx.device()?, set.id, set.value).await?;
    Ok(ok())
});

// ---- Photos ----

handler!(photos_albums, DeviceOnly, |ctx, _a| {
    let albums = ctx.node()?.photo_albums(ctx.device()?).await?;
    Ok(json!({ "albums": albums.iter().map(|a| json!({
        "id": a.id,
        "name": clean(&a.name, 120),
        "count": a.count,
    })).collect::<Vec<_>>(), "_untrusted": ["albums[].name"] }))
});

handler!(photos_list, PhotoListArgs, |ctx, a| {
    let before = match (a.before_date, a.before_id) {
        (Some(d), Some(id)) => Some((d, id)),
        (None, None) => None,
        _ => return Err(ActionError::invalid("Give both before_date and before_id, or neither.")),
    };
    let items = ctx.node()?.photo_list(ctx.device()?, a.album, before, limit(a.limit)).await?;
    Ok(json!({ "items": items.iter().map(|p| json!({
        "id": p.id,
        "name": clean(&p.name, 200),
        "date": p.date,
        "size": p.size,
        "width": p.width,
        "height": p.height,
        "video_s": p.duration,
    })).collect::<Vec<_>>(), "_untrusted": ["items[].name"] }))
});

handler!(photos_thumbnails, ThumbArgs, |ctx, a| {
    if a.ids.is_empty() || a.ids.len() > 12 {
        return Err(ActionError::invalid("Give 1 to 12 ids."));
    }
    let thumbs = ctx.node()?.photo_thumbs(ctx.device()?, a.ids).await?;
    let mut used = 0usize;
    let mut images = Vec::new();
    let mut ids = Vec::new();
    for t in thumbs.iter().filter(|t| !t.data.is_empty()) {
        if used + t.data.len() > IMAGE_BUDGET {
            break;
        }
        used += t.data.len();
        ids.push(t.id.clone());
        images.push(image("image/jpeg", &t.data));
    }
    Ok(json!({ "ids": ids, "_images": images }))
});

handler!(photos_save, PhotoIdsArgs, |ctx, a| {
    if a.ids.is_empty() || a.ids.len() > 50 {
        return Err(ActionError::invalid("Give 1 to 50 ids."));
    }
    let count = a.ids.len();
    let id = ctx.node()?.fetch_photos(ctx.device()?, a.ids).await?;
    Ok(json!({ "transfer_id": id, "items": count }))
});

handler!(photos_delete, PhotoIdsArgs, |ctx, a| {
    if a.ids.is_empty() || a.ids.len() > 50 {
        return Err(ActionError::invalid("Give 1 to 50 ids."));
    }
    let moved = ctx.node()?.photos_delete(ctx.device()?, a.ids).await?;
    Ok(json!({ "moved_to_trash": moved }))
});

// ---- Phone storage ----

fn phone_path(raw: &str, allow_top: bool) -> Result<String, ActionError> {
    let path = raw.trim().trim_matches('/').to_owned();
    if path.is_empty() && allow_top {
        return Ok(path);
    }
    if !is_valid_storage_path(&path) {
        return Err(ActionError::invalid(format!(
            "{raw:?} isn't a path on the phone. Use folders like Download/file.txt."
        )));
    }
    Ok(path)
}

fn name_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

handler!(storage_list, StoragePathArgs, |ctx, a| {
    let path = phone_path(&a.path, true)?;
    let entries = ctx.node()?.storage_list(ctx.device()?, path.clone()).await?;
    Ok(json!({ "path": path, "entries": entries.iter().take(1000).map(|e| json!({
        "name": clean(&e.name, 255),
        "folder": e.is_dir,
        "size": e.size,
        "modified": e.modified,
    })).collect::<Vec<_>>(), "_untrusted": ["entries[].name"] }))
});

fn image_mime(name: &str) -> Option<&'static str> {
    let ext = name.rsplit('.').next()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => return None,
    })
}

handler!(storage_read, StorageReadArgs, |ctx, a| {
    let path = phone_path(&a.path, false)?;
    let offset = a.offset.unwrap_or(0);
    if let Some(mime) = image_mime(&path) {
        let (meta, data) =
            ctx.node()?.storage_read(ctx.device()?, path.clone(), 0, Some(IMAGE_BUDGET as u64)).await?;
        if meta.size > IMAGE_BUDGET as u64 {
            return Err(ActionError::invalid(
                "That picture is too big to show. Use storage_save to copy it to the PC.",
            ));
        }
        return Ok(
            json!({ "path": path, "size": meta.size, "modified": meta.modified, "_images": [image(mime, &data)] }),
        );
    }
    let length = a.length.unwrap_or(TEXT_MAX).clamp(1, TEXT_MAX);
    let (meta, data) = ctx.node()?.storage_read(ctx.device()?, path.clone(), offset, Some(length)).await?;
    let text = match std::str::from_utf8(&data) {
        Ok(t) => t.to_owned(),
        // A cut in the middle of a character at the end is fine.
        Err(e) if e.valid_up_to() + 4 > data.len() => {
            String::from_utf8_lossy(&data[..e.valid_up_to()]).into_owned()
        }
        Err(_) => {
            return Err(ActionError::invalid("That file isn't text. Use storage_save to copy it to the PC."));
        }
    };
    let read_to = offset + data.len() as u64;
    Ok(json!({
        "path": path,
        "size": meta.size,
        "modified": meta.modified,
        "offset": offset,
        "text": clean_message(&text, TEXT_MAX as usize, ctx.show_codes),
        "more": read_to < meta.size,
        "_untrusted": ["text"],
    }))
});

/// A file name in `dir` that isn't taken: `name`, `name (2)`...
fn free_name(dir: &Path, name: &str) -> PathBuf {
    let safe: String =
        name.chars().map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { '_' } else { c }).collect();
    let safe = if safe.trim().is_empty() { "file".to_owned() } else { safe };
    let (stem, ext) = match safe.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_owned(), format!(".{e}")),
        _ => (safe.clone(), String::new()),
    };
    let mut candidate = dir.join(&safe);
    let mut n = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    candidate
}

handler!(storage_save, StoragePathArgs, |ctx, a| {
    let path = phone_path(&a.path, false)?;
    let device = ctx.device()?;
    let node = ctx.node()?;
    let dir = ctx.host.save_dir().ok_or_else(|| ActionError::new(Code::Unsupported, UNSUPPORTED))?;
    std::fs::create_dir_all(&dir).map_err(|e| ActionError::internal(e.to_string()))?;
    let target = free_name(&dir, name_of(&path));
    let partial = target.with_extension("nectarlink-part");
    let mut file = std::fs::File::create(&partial).map_err(|e| ActionError::internal(e.to_string()))?;
    let mut offset = 0u64;
    let result: Result<u64, ActionError> = async {
        loop {
            let (meta, data) = node.storage_read(device, path.clone(), offset, Some(SAVE_CHUNK)).await?;
            if meta.size > SAVE_MAX {
                return Err(ActionError::invalid("That file is too big to copy this way."));
            }
            file.write_all(&data).map_err(|e| ActionError::internal(e.to_string()))?;
            offset += data.len() as u64;
            if data.is_empty() || offset >= meta.size {
                return Ok(meta.size);
            }
        }
    }
    .await;
    drop(file);
    match result {
        Ok(size) => {
            std::fs::rename(&partial, &target).map_err(|e| ActionError::internal(e.to_string()))?;
            Ok(json!({ "saved": target.to_string_lossy(), "size": size }))
        }
        Err(e) => {
            let _ = std::fs::remove_file(&partial);
            Err(e)
        }
    }
});

handler!(storage_upload, StorageUploadArgs, |ctx, a| {
    let source = crate::paths::checked(&a.source)?;
    if !source.is_file() {
        return Err(ActionError::invalid("The source must be a file."));
    }
    let path = phone_path(&a.path, false)?;
    let done = ctx.node()?.storage_write(ctx.device()?, path.clone(), &source).await?;
    Ok(json!({ "path": path, "size": done.size }))
});

handler!(storage_mkdir, StoragePathArgs, |ctx, a| {
    let path = phone_path(&a.path, false)?;
    ctx.node()?.storage_mkdir(ctx.device()?, path).await?;
    Ok(ok())
});

handler!(storage_rename, StorageRenameArgs, |ctx, a| {
    let from = phone_path(&a.from, false)?;
    let to = phone_path(&a.to, false)?;
    ctx.node()?.storage_rename(ctx.device()?, from, to).await?;
    Ok(ok())
});

handler!(storage_delete, StoragePathArgs, |ctx, a| {
    let path = phone_path(&a.path, false)?;
    // The user already approved this exact path in NectarLink.
    ctx.node()?.storage_delete(ctx.device()?, path, true).await?;
    Ok(ok())
});

// ---- PCs ----

handler!(pc_power, PowerArgs, |ctx, a| {
    let action = match a.action {
        PowerKind::Lock => PowerAction::Lock,
        PowerKind::Sleep => PowerAction::Sleep,
        PowerKind::Hibernate => PowerAction::Hibernate,
        PowerKind::Shutdown => PowerAction::Shutdown,
    };
    match ctx.device {
        Some(device) => ctx.node()?.pc_power(device, action).await?,
        None => ctx.host.pc_power(action).map_err(host_err)?,
    }
    Ok(ok())
});

handler!(pc_wake, DeviceOnly, |ctx, _a| {
    let device = ctx.device()?;
    let node = ctx.node()?;
    if node.wake_info(device)?.is_none() {
        return Err(ActionError::new(Code::Unsupported, "NectarLink doesn't know how to wake that PC yet.")
            .user_action(
                "Connect the phone or PC to it once while it's awake, and turn on Wake-on-LAN on it.",
            ));
    }
    node.wake(device).await?;
    Ok(
        json!({ "sent": true, "note": "Wake-up packets were sent. Use devices_list to see when it connects." }),
    )
});

handler!(pc_audio_set, PcAudioArgs, |ctx, a| {
    if a.volume.is_none() && a.muted.is_none() {
        return Err(ActionError::invalid("Give volume, muted or both."));
    }
    if a.volume.is_some_and(|v| v > 100) {
        return Err(ActionError::invalid("volume is 0 to 100."));
    }
    ctx.node()?.set_pc_audio(ctx.device()?, a.volume, a.muted).await?;
    Ok(ok())
});

// ---- App windows ----

handler!(mirror_apps, DeviceOnly, |ctx, _a| {
    let apps = ctx.node()?.mirror_apps(ctx.device()?).await?;
    Ok(json!({ "apps": apps.iter().map(|a| json!({
        "package": a.pkg,
        "name": clean(&a.label, 80),
    })).collect::<Vec<_>>(), "_untrusted": ["apps[].name"] }))
});

handler!(mirror_open, MirrorOpenArgs, |ctx, a| {
    let device = ctx.device()?;
    let app = match a.app {
        Some(pkg) => {
            let apps = ctx.node()?.mirror_apps(device).await?;
            let found = apps.into_iter().find(|x| x.pkg == pkg).ok_or_else(|| {
                ActionError::new(Code::NotFound, "The phone has no app with that package. See mirror_apps.")
            })?;
            Some((found.pkg, found.label))
        }
        None => None,
    };
    ctx.host.mirror_open(device, app).map_err(host_err)?;
    Ok(json!({ "opening": true, "note": "The phone may ask its user to allow it." }))
});

handler!(mirror_windows, NoArgs, |ctx, _a| {
    let windows: Vec<Value> = ctx
        .host
        .mirror_windows()
        .into_iter()
        .filter(|w| {
            w.get("device")
                .and_then(Value::as_str)
                .and_then(|d| d.parse().ok())
                .is_none_or(|d| ctx.may_see(d, Scope::Mirroring))
        })
        .collect();
    Ok(json!({ "windows": windows, "_untrusted": ["windows[].app_name"] }))
});

handler!(mirror_close, MirrorOpenArgs, |ctx, a| {
    let closed = ctx.host.mirror_close(ctx.device()?, a.app.as_deref()).map_err(host_err)?;
    if !closed {
        return Err(ActionError::new(Code::NotFound, "No such window is open."));
    }
    Ok(ok())
});

handler!(app_uninstall, AppArgs, |ctx, a| {
    ctx.node()?.uninstall_phone_app(ctx.device()?, a.app).await?;
    Ok(json!({ "asked": true, "note": "The phone asks its user to confirm." }))
});

// ---- Deck ----

handler!(deck_tiles, NoArgs, |ctx, _a| {
    Ok(json!({ "tiles": ctx.host.deck_tiles(), "_untrusted": ["tiles[].label"] }))
});

handler!(deck_run, TileArgs, |ctx, a| {
    ctx.host.deck_run(&a.tile).map_err(host_err)?;
    Ok(ok())
});

// ---- Timeline, LocalSend, attachments ----

handler!(timeline_list, TimelineArgs, |ctx, a| {
    let kind = match a.kind.as_deref() {
        Some(k) => Some(TimelineKind::from_str_opt(k).ok_or_else(|| ActionError::invalid("Unknown kind."))?),
        None => None,
    };
    let node = ctx.node()?;
    let see_clips = ctx.holds(Grant::new(Scope::Clipboard, Access::Read));
    let wanted = limit(a.limit);
    let mut entries = Vec::new();
    let mut offset = 0;
    // Entries from devices the agent may not use are skipped, so look further.
    while entries.len() < wanted as usize && offset < 1000 {
        let page = node
            .timeline_page(&TimelineQuery {
                kind,
                device: ctx.device,
                search: a.query.clone(),
                offset,
                limit: 100,
            })
            .map_err(ActionError::from)?;
        for e in &page.entries {
            if !ctx.may_see(e.device_id, Scope::Files) || entries.len() >= wanted as usize {
                continue;
            }
            let title = if e.kind == TimelineKind::Clip && !see_clips {
                "[clipboard text hidden]".to_owned()
            } else {
                clean_message(&e.title, 300, ctx.show_codes)
            };
            entries.push(json!({
                "id": e.id,
                "kind": e.kind.as_str(),
                "device": e.device_id.to_string(),
                "device_name": clean(&e.device_name, 80),
                "incoming": e.incoming,
                "time": e.timestamp,
                "title": title,
                "size": e.size_bytes,
            }));
        }
        if !page.has_more {
            break;
        }
        offset += 100;
    }
    Ok(json!({ "entries": entries, "_untrusted": ["entries[].title", "entries[].device_name"] }))
});

handler!(transfer_accept, TransferIdArgs, |ctx, a| {
    ctx.node()?.accept_transfer(&a.id);
    Ok(ok())
});

handler!(localsend_peers, NoArgs, |ctx, _a| {
    let node = ctx.node()?;
    if !node.localsend_enabled() {
        return Ok(json!({ "peers": [], "note": "LocalSend is off in NectarLink." }));
    }
    Ok(json!({ "peers": node.localsend_peers().iter().map(|p| json!({
        "name": clean(&p.alias, 80),
        "model": clean_opt(p.device_model.as_deref(), 80),
        "type": p.device_type,
    })).collect::<Vec<_>>(), "_untrusted": ["peers[].name", "peers[].model"] }))
});

handler!(sms_attachment, SmsPartArgs, |ctx, a| {
    let (mime, data) = ctx.node()?.sms_part(ctx.device()?, a.id).await?;
    let mime = mime.to_ascii_lowercase();
    if mime.starts_with("image/")
        && matches!(mime.as_str(), "image/jpeg" | "image/png" | "image/gif" | "image/webp")
    {
        if data.len() > IMAGE_BUDGET {
            return Err(ActionError::invalid("That picture is too big to show."));
        }
        return Ok(json!({ "mime": mime, "size": data.len(), "_images": [image(&mime, &data)] }));
    }
    if mime.starts_with("text/") {
        let text = String::from_utf8_lossy(&data);
        return Ok(
            json!({ "mime": mime, "text": clean_message(&text, FIELD_MAX * 10, ctx.show_codes), "_untrusted": ["text"] }),
        );
    }
    Ok(json!({ "mime": mime, "size": data.len(), "note": "This kind of attachment can't be shown here." }))
});

// ---- Events ----

handler!(events_wait, EventsArgs, |ctx, a| {
    if let Some(kinds) = &a.kinds
        && let Some(bad) = kinds.iter().find(|k| !events::KINDS.contains(&k.as_str()))
    {
        return Err(ActionError::invalid(format!(
            "Unknown kind {bad:?}. Kinds: {}",
            events::KINDS.join(", ")
        )));
    }
    let log = ctx.events.clone();
    let since = a.since.unwrap_or_else(|| log.last_seq());
    let wait = Duration::from_secs(a.wait_s.unwrap_or(25).min(events::MAX_WAIT.as_secs()));
    let keep = |e: &Event| {
        a.kinds.as_ref().is_none_or(|k| k.iter().any(|k| k == e.kind))
            && ctx.holds(Grant::new(e.scope, Access::Read))
            && e.device.is_none_or(|d| ctx.may_see(d, e.scope))
    };
    let (found, missed) = log.wait(since, wait, limit(a.limit) as usize, keep).await;
    let next = found.last().map(|e| e.seq).unwrap_or(since);
    Ok(json!({
        "events": found.iter().map(|e| event_json(e, ctx.show_codes)).collect::<Vec<_>>(),
        "next_since": next,
        "missed_some": missed,
        "_untrusted": ["events[].data"],
    }))
});

fn event_json(e: &Event, show_codes: bool) -> Value {
    let mut data = e.data.clone();
    if let Value::Object(m) = &mut data {
        for (k, v) in m.iter_mut() {
            if let Value::String(s) = v {
                *v = Value::String(if matches!(k.as_str(), "title" | "body") {
                    clean_message(s, 500, show_codes)
                } else {
                    clean(s, 200)
                });
            }
        }
    }
    json!({ "seq": e.seq, "time": e.ts, "kind": e.kind, "device": e.device.map(|d| d.to_string()), "data": data })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phone_paths_stay_inside_shared_storage() {
        assert_eq!(phone_path("/Download/a.txt/", false).unwrap(), "Download/a.txt");
        assert_eq!(phone_path("", true).unwrap(), "");
        assert!(phone_path("", false).is_err());
        assert!(phone_path("Download/../Android/data", false).is_err());
        assert!(phone_path("Download//a", false).is_err());
    }

    #[test]
    fn saved_files_never_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let first = free_name(dir.path(), "a.txt");
        assert_eq!(first, dir.path().join("a.txt"));
        std::fs::write(&first, b"x").unwrap();
        assert_eq!(free_name(dir.path(), "a.txt"), dir.path().join("a (2).txt"));
        assert_eq!(free_name(dir.path(), "bad:name?.txt"), dir.path().join("bad_name_.txt"));
    }

    #[test]
    fn event_text_is_cleaned_and_codes_hidden() {
        let e = Event {
            seq: 3,
            ts: 1,
            kind: "notification.posted",
            scope: Scope::Notifications,
            device: None,
            data: json!({ "title": "Bank\u{202E}", "body": "Your code is 482913", "app": "x" }),
        };
        let out = event_json(&e, false);
        assert_eq!(out["data"]["title"], "Bank");
        assert!(!out["data"]["body"].as_str().unwrap().contains("482913"));
        assert!(event_json(&e, true)["data"]["body"].as_str().unwrap().contains("482913"));
    }
}
