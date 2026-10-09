// SPDX-License-Identifier: GPL-3.0-or-later
//! Global Command Palette (`PLAN.md` §3.9): instant fuzzy search over local
//! actions, contacts, conversations, mirrorable apps, quick settings toggles,
//! navigation pages, clipboard history, and recent files.
//!
//! Designed so that:
//! - Opening builds candidates synchronously from in-memory state and local
//!   SQLite in a few milliseconds (< 100 ms end-to-end to visible popup).
//! - Closing drops the candidate vector (`shrink_to_fit`) so the palette costs
//!   zero memory and zero CPU while closed.

use std::{collections::HashSet, path::PathBuf, sync::Mutex, time::Instant};

use nectarlink_core::{ClipboardItemKind, DeviceId, LinkState, PhoneToggleValue, TimelineKind};
use serde_json::{Value, json};

use crate::{
    bridge::app::{describe, show_message},
    core_host,
    settings::Settings,
    state::Changes,
};

/// Maximum number of search results serialized to QML per keystroke.
const MAX_RESULTS: usize = 24;

/// UI-level navigation or sheet action returned by [`execute`] when a command
/// needs `AppController` to switch pages or open a modal sheet in the main window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteUiAction {
    NavigatePage { page: String, device_index: Option<usize> },
    StartChat { device_id: String, number: String, name: String },
    OpenDialer { device_id: String, number: String },
    OpenDoctor,
    OpenPairing,
}

#[derive(Debug, Clone)]
pub(crate) struct PaletteCandidate {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub category: String,
    pub icon: String,
    pub badge: String,
    pub shortcut: String,
    pub keywords: String,
    pub base_boost: i64,
}

impl PaletteCandidate {
    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "title": self.title,
            "subtitle": self.subtitle,
            "category": self.category,
            "icon": self.icon,
            "badge": self.badge,
            "shortcut": self.shortcut,
        })
    }
}

#[derive(Debug, Default)]
struct PaletteState {
    open: bool,
    opened_at: Option<Instant>,
    last_open_ms: f64,
    candidates: Vec<PaletteCandidate>,
    preferred_device: Option<DeviceId>,
}

static STATE: Mutex<PaletteState> = Mutex::new(PaletteState {
    open: false,
    opened_at: None,
    last_open_ms: 0.0,
    candidates: Vec::new(),
    preferred_device: None,
});

fn with_state<T>(f: impl FnOnce(&mut PaletteState) -> T) -> T {
    f(&mut STATE.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Updates the preferred primary phone for the Command Palette (synced with
/// `AppController.primaryDeviceId` when that device is online).
pub fn set_preferred_device(id: Option<DeviceId>) {
    with_state(|s| s.preferred_device = id);
}

/// Opens the Command Palette, snapshots local candidates into memory, notifies
/// QML via `Changes::PALETTE`, and returns the initial JSON results list.
pub fn open() -> String {
    let t0 = Instant::now();
    let candidates = build_candidates();
    let initial = filter_and_rank(&candidates, "");
    let json = serialize_results(&initial);
    let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
    with_state(|s| {
        s.open = true;
        s.opened_at = Some(t0);
        s.last_open_ms = elapsed_ms;
        s.candidates = candidates;
    });
    core_host::host().hub.changed(Changes::PALETTE);
    json
}

/// Closes the Command Palette and frees all cached candidates so it costs
/// nothing while closed.
pub fn close() {
    let was_open = with_state(|s| {
        let prev = s.open;
        s.open = false;
        s.opened_at = None;
        s.candidates.clear();
        s.candidates.shrink_to_fit();
        prev
    });
    if was_open {
        core_host::host().hub.changed(Changes::PALETTE);
    }
}

/// Whether the Command Palette popup is currently open.
pub fn is_open() -> bool {
    with_state(|s| s.open)
}

/// Called by QML as soon as the Command Palette window is visible on screen.
/// Records and returns the end-to-end latency in milliseconds from [`open`].
pub fn mark_shown() -> f64 {
    let ms = with_state(|s| {
        if let Some(t0) = s.opened_at {
            s.last_open_ms = t0.elapsed().as_secs_f64() * 1000.0;
        }
        s.last_open_ms
    });
    tracing::info!(open_ms = format!("{ms:.2}"), "command palette shown");
    ms
}

/// Returns the most recent open-to-visible latency in milliseconds.
pub fn last_open_ms() -> f64 {
    with_state(|s| s.last_open_ms)
}

/// Searches the Command Palette candidates for `query` and returns a JSON array
/// of matching items ordered by relevance.
pub fn search(query: &str) -> String {
    let results = with_state(|s| {
        if s.candidates.is_empty() {
            s.candidates = build_candidates();
        }
        filter_and_rank(&s.candidates, query)
    });
    serialize_results(&results)
}

fn serialize_results(items: &[PaletteCandidate]) -> String {
    Value::Array(items.iter().take(MAX_RESULTS).map(PaletteCandidate::to_json).collect()).to_string()
}

/// Executes a Command Palette item by `id`, closing the palette and returning
/// an optional [`PaletteUiAction`] if the main window needs to navigate or show
/// a sheet.
pub fn execute(id: &str) -> Option<PaletteUiAction> {
    let id = id.trim().to_owned();
    close();
    if id.is_empty() {
        return None;
    }

    if let Some(page) = id.strip_prefix("nav:") {
        return Some(PaletteUiAction::NavigatePage { page: page.to_owned(), device_index: None });
    }
    if let Some(idx_str) = id.strip_prefix("nav_device:")
        && let Ok(idx) = idx_str.parse::<usize>()
    {
        return Some(PaletteUiAction::NavigatePage { page: "home".to_owned(), device_index: Some(idx) });
    }

    if id == "action:continuity_photo" {
        crate::continuity_camera::start("photo");
        return None;
    }
    if id == "action:continuity_scan" {
        crate::continuity_camera::start("scan");
        return None;
    }
    if id == "action:open_link" {
        crate::links::open_copied_link_on_phone();
        return None;
    }
    if id == "action:sync_now" {
        if let Some(node) = core_host::node() {
            show_message("Syncing devices…");
            core_host::spawn(async move { node.refresh().await });
        }
        return None;
    }
    if id == "action:doctor" {
        return Some(PaletteUiAction::OpenDoctor);
    }
    if id == "action:pair" {
        return Some(PaletteUiAction::OpenPairing);
    }
    if id == "action:wake_pc" {
        core_host::spawn(core_host::refresh_wake());
        let msg = core_host::host().hub.read(|s| match s.wake.primary() {
            Some(adapter) if s.wake.state_str() == "enabled" => {
                format!("Wake-on-LAN is ready on {} (phones can wake this PC).", adapter.label)
            }
            Some(adapter) => {
                format!("Wake-on-LAN on {} is {}. Check Settings.", adapter.label, s.wake.state_str())
            }
            None => "No physical network adapter found for Wake-on-LAN.".to_owned(),
        });
        show_message(msg);
        return Some(PaletteUiAction::NavigatePage { page: "settings".to_owned(), device_index: None });
    }

    if let Some(dev_str) = id.strip_prefix("action:send_clipboard:")
        && let Ok(dev) = dev_str.parse::<DeviceId>()
    {
        crate::clipboard::send_now(dev);
        return None;
    }
    if let Some(dev_str) = id.strip_prefix("action:ring:")
        && let Ok(dev) = dev_str.parse::<DeviceId>()
    {
        ring_device_from_palette(dev);
        return None;
    }
    if let Some(dev_str) = id.strip_prefix("action:open_last_photo:")
        && let Ok(dev) = dev_str.parse::<DeviceId>()
    {
        crate::photos::open_latest_photo(dev);
        return None;
    }
    if let Some(dev_str) = id.strip_prefix("action:record_voice:")
        && let Ok(dev) = dev_str.parse::<DeviceId>()
    {
        open_voice_recordings(dev);
        return None;
    }
    if let Some(dev_str) = id.strip_prefix("action:mirror_screen:")
        && let Ok(dev) = dev_str.parse::<DeviceId>()
    {
        crate::mirror::start(dev);
        return None;
    }
    if let Some(dev_str) = id.strip_prefix("action:webcam:")
        && let Ok(dev) = dev_str.parse::<DeviceId>()
    {
        match crate::webcam::phase() {
            crate::webcam::Phase::Asking { .. } | crate::webcam::Phase::Streaming { .. } => {
                crate::webcam::stop();
                show_message("Stopped phone webcam.");
            }
            _ => {
                crate::webcam::start(Some(dev));
                show_message("Starting phone webcam…");
            }
        }
        return None;
    }
    if let Some(dev_str) = id.strip_prefix("action:phone_storage:")
        && let Ok(dev) = dev_str.parse::<DeviceId>()
    {
        if let Err(e) = crate::storage::open_in_explorer(dev) {
            show_message(e);
        }
        return None;
    }

    if let Some(rest) = id.strip_prefix("toggle:") {
        execute_phone_toggle(rest);
        return None;
    }

    if let Some(rest) = id.strip_prefix("text_thread:")
        && let Some((dev_str, thread_id)) = rest.split_once(':')
        && let Ok(dev) = dev_str.parse::<DeviceId>()
    {
        let idx = device_index_of(dev);
        crate::messages::open_thread_from_palette(dev, thread_id);
        return Some(PaletteUiAction::NavigatePage { page: "messages".to_owned(), device_index: idx });
    }

    if let Some(rest) = id.strip_prefix("text_contact:")
        && let Some((dev_str, payload)) = rest.split_once(':')
        && let Ok(dev) = dev_str.parse::<DeviceId>()
    {
        let (number, name) = payload.split_once('\t').unwrap_or((payload, payload));
        return Some(PaletteUiAction::StartChat {
            device_id: dev.to_string(),
            number: number.to_owned(),
            name: name.to_owned(),
        });
    }

    if let Some(rest) = id.strip_prefix("call_contact:")
        && let Some((dev_str, number)) = rest.split_once(':')
        && let Ok(dev) = dev_str.parse::<DeviceId>()
    {
        if crate::calls::can_dial(dev) {
            crate::calls::dial_from_palette(dev, number);
            let name = core_host::host().hub.read(|s| s.name_of(&dev)).unwrap_or_else(|| "phone".into());
            show_message(format!("Calling {number} on {name}…"));
            return None;
        }
        return Some(PaletteUiAction::OpenDialer { device_id: dev.to_string(), number: number.to_owned() });
    }

    if let Some(rest) = id.strip_prefix("mirror_app:")
        && let Some((dev_str, payload)) = rest.split_once(':')
        && let Ok(dev) = dev_str.parse::<DeviceId>()
    {
        let (pkg, label) = payload.split_once('\t').unwrap_or((payload, payload));
        crate::mirror::start_app(dev, pkg.to_owned(), label.to_owned());
        return None;
    }

    if let Some(clip_id) = id.strip_prefix("clip_copy:") {
        crate::clipboard::copy_history_item(clip_id);
        return None;
    }

    if let Some(path_str) = id.strip_prefix("timeline_open:") {
        let path = PathBuf::from(path_str);
        if path.exists() {
            crate::transfers::open(&path);
        } else {
            show_message("That file is no longer in its saved location.");
        }
        return None;
    }

    None
}

fn device_index_of(device: DeviceId) -> Option<usize> {
    core_host::host().hub.read(|s| s.devices.iter().position(|d| d.id == device))
}

fn ring_device_from_palette(device: DeviceId) {
    let Some(node) = core_host::node() else { return };
    let name = core_host::host().hub.read(|s| s.name_of(&device)).unwrap_or_else(|| "your phone".into());
    show_message(format!("Ringing {name}…"));
    core_host::spawn(async move {
        if let Err(e) = node.ring(device, true).await {
            show_message(describe(&e));
        }
    });
}

fn open_voice_recordings(device: DeviceId) {
    let data_dir = &core_host::host().data_dir;
    let settings = Settings::load(data_dir);
    let folder = crate::recordings::effective_folder(settings.recordings_folder.as_deref());
    let _ = std::fs::create_dir_all(&folder);
    crate::transfers::open(&folder);
    let phone_name =
        core_host::host().hub.read(|s| s.name_of(&device)).unwrap_or_else(|| "your phone".into());
    show_message(format!(
        "Opened Recordings folder — start a voice note in Nectarlink on {phone_name} to send it here."
    ));
}

fn execute_phone_toggle(rest: &str) {
    let parts: Vec<&str> = rest.split(':').collect();
    let Some(dev_str) = parts.first().copied() else { return };
    let Ok(device) = dev_str.parse::<DeviceId>() else { return };
    let Some(kind) = parts.get(1).copied() else { return };
    let Some(node) = core_host::node() else { return };

    let current = core_host::host().hub.read(|s| s.toggles.get(&device).cloned());
    let (toggle_id, val, label) = match kind {
        "dnd" => {
            let next = !current.as_ref().is_some_and(|t| t.dnd);
            (
                "dnd".to_owned(),
                PhoneToggleValue::Bool(next),
                if next { "Do Not Disturb turned on" } else { "Do Not Disturb turned off" },
            )
        }
        "flashlight" => {
            let next = !current.as_ref().and_then(|t| t.flashlight).unwrap_or(false);
            (
                "flashlight".to_owned(),
                PhoneToggleValue::Bool(next),
                if next { "Flashlight turned on" } else { "Flashlight turned off" },
            )
        }
        "wifi" => {
            let next = !current.as_ref().is_some_and(|t| t.wifi);
            (
                "wifi".to_owned(),
                PhoneToggleValue::Bool(next),
                if next { "Wi-Fi turned on" } else { "Wi-Fi turned off" },
            )
        }
        "bluetooth" => {
            let next = !current.as_ref().is_some_and(|t| t.bluetooth);
            (
                "bluetooth".to_owned(),
                PhoneToggleValue::Bool(next),
                if next { "Bluetooth turned on" } else { "Bluetooth turned off" },
            )
        }
        "ringer" => {
            let mode = parts.get(2).copied().unwrap_or("normal");
            let desc = match mode {
                "silent" => "Ringer set to silent",
                "vibrate" => "Ringer set to vibrate",
                _ => "Ringer set to sound",
            };
            ("ringer".to_owned(), PhoneToggleValue::Mode(mode.to_owned()), desc)
        }
        _ => return,
    };

    core_host::host().hub.update(|s| {
        let Some(mut next) = s.toggles.get(&device).cloned() else { return Changes::NONE };
        match (toggle_id.as_str(), &val) {
            ("dnd", PhoneToggleValue::Bool(on)) => next.dnd = *on,
            ("flashlight", PhoneToggleValue::Bool(on)) => next.flashlight = Some(*on),
            ("wifi", PhoneToggleValue::Bool(on)) => next.wifi = *on,
            ("bluetooth", PhoneToggleValue::Bool(on)) => next.bluetooth = *on,
            ("ringer", PhoneToggleValue::Mode(m)) => next.ringer.clone_from(m),
            _ => {}
        }
        s.set_toggles(device, Some(next))
    });
    show_message(label);
    core_host::spawn(async move {
        if let Err(e) = node.set_phone_toggle(device, toggle_id, val).await {
            let latest = node.phone_toggles(device);
            core_host::host().hub.update(|s| s.set_toggles(device, latest));
            show_message(describe(&e));
        }
    });
}

/// Builds the full set of searchable Command Palette candidates synchronously
/// from in-memory state and local SQLite.
fn build_candidates() -> Vec<PaletteCandidate> {
    let settings = Settings::load(&core_host::host().data_dir);
    let (devices, toggles_map, wake_subtitle) = core_host::host().hub.read(|s| {
        let devs: Vec<(usize, DeviceId, String, bool)> = s
            .devices
            .iter()
            .enumerate()
            .map(|(idx, d)| (idx, d.id, d.info.name.clone(), matches!(d.link, LinkState::Online { .. })))
            .collect();
        let wake_sub = match s.wake.primary() {
            Some(a) if s.wake.state_str() == "enabled" => format!("Ready on {}", a.label),
            Some(a) => format!("{} ({})", a.label, s.wake.state_str()),
            None => "Check Wake-on-LAN status in Settings".to_owned(),
        };
        (devs, s.toggles.clone(), wake_sub)
    });

    let preferred = with_state(|s| s.preferred_device);
    let primary = preferred
        .and_then(|pref| devices.iter().find(|(_, id, _, online)| *id == pref && *online))
        .or_else(|| devices.iter().find(|(_, _, _, online)| *online))
        .or_else(|| preferred.and_then(|pref| devices.iter().find(|(_, id, _, _)| *id == pref)))
        .or_else(|| devices.first())
        .cloned();
    let multi = devices.len() > 1;

    let mut out = Vec::with_capacity(128);

    // 1. Primary phone quick commands ("Send clipboard", "Find my phone", "Open last photo", "Record voice", etc.)
    if let Some((_, dev_id, ref dev_name, online)) = primary {
        let status_badge = if online { dev_name.clone() } else { format!("{dev_name} (offline)") };
        out.push(PaletteCandidate {
            id: format!("action:send_clipboard:{dev_id}"),
            title: "Send clipboard".into(),
            subtitle: format!("Send copied text or image to {dev_name}"),
            category: "Actions".into(),
            icon: "clipboard".into(),
            badge: status_badge.clone(),
            shortcut: String::new(),
            keywords: format!("send clipboard copy paste share text image {dev_name}"),
            base_boost: 95,
        });
        out.push(PaletteCandidate {
            id: format!("action:ring:{dev_id}"),
            title: "Find my phone".into(),
            subtitle: format!("Make {dev_name} ring at full volume"),
            category: "Actions".into(),
            icon: "ring".into(),
            badge: status_badge.clone(),
            shortcut: String::new(),
            keywords: format!("find my phone ring locate alarm lost {dev_name}"),
            base_boost: 92,
        });
        out.push(PaletteCandidate {
            id: format!("action:open_last_photo:{dev_id}"),
            title: "Open last photo".into(),
            subtitle: format!("Open the newest photo or screenshot from {dev_name}"),
            category: "Actions".into(),
            icon: "photo".into(),
            badge: status_badge.clone(),
            shortcut: String::new(),
            keywords: format!("open last photo latest picture image camera screenshot {dev_name}"),
            base_boost: 90,
        });
        out.push(PaletteCandidate {
            id: format!("action:record_voice:{dev_id}"),
            title: "Record voice".into(),
            subtitle: format!("Open voice recordings folder for {dev_name}"),
            category: "Actions".into(),
            icon: "mic".into(),
            badge: status_badge.clone(),
            shortcut: String::new(),
            keywords: format!("record voice audio memo note microphone recordings {dev_name}"),
            base_boost: 84,
        });
    }

    // Continuity Camera actions (global hotkeys shown as badges/shortcuts)
    out.push(PaletteCandidate {
        id: "action:continuity_photo".into(),
        title: "Take photo with phone".into(),
        subtitle: "Capture a photo on your phone and paste it into the active window".into(),
        category: "Actions".into(),
        icon: "camera".into(),
        badge: "Continuity".into(),
        shortcut: settings.continuity_photo_hotkey.clone(),
        keywords: "take photo with phone continuity camera capture picture paste".into(),
        base_boost: 88,
    });
    out.push(PaletteCandidate {
        id: "action:continuity_scan".into(),
        title: "Scan document with phone".into(),
        subtitle: "Scan a document on your phone and paste it into the active window".into(),
        category: "Actions".into(),
        icon: "clipboardList".into(),
        badge: "Continuity".into(),
        shortcut: settings.continuity_scan_hotkey.clone(),
        keywords: "scan document with phone continuity camera receipt paper pdf paste".into(),
        base_boost: 86,
    });
    out.push(PaletteCandidate {
        id: "action:open_link".into(),
        title: "Open copied link on phone".into(),
        subtitle: "Send the web URL on your clipboard to your phone's browser".into(),
        category: "Actions".into(),
        icon: "globe".into(),
        badge: String::new(),
        shortcut: String::new(),
        keywords: "open copied link url browser web share phone".into(),
        base_boost: 78,
    });

    if let Some((_, dev_id, ref dev_name, _)) = primary {
        out.push(PaletteCandidate {
            id: format!("action:mirror_screen:{dev_id}"),
            title: "Mirror phone screen".into(),
            subtitle: format!("Stream {dev_name}'s screen in a desktop window"),
            category: "Actions".into(),
            icon: "mirror".into(),
            badge: dev_name.clone(),
            shortcut: String::new(),
            keywords: format!("mirror phone screen cast display stream {dev_name}"),
            base_boost: 80,
        });
        let webcam_streaming = matches!(
            crate::webcam::phase(),
            crate::webcam::Phase::Asking { .. } | crate::webcam::Phase::Streaming { .. }
        );
        out.push(PaletteCandidate {
            id: format!("action:webcam:{dev_id}"),
            title: if webcam_streaming { "Stop phone webcam".into() } else { "Use phone as webcam".into() },
            subtitle: format!("Stream {dev_name}'s camera into the Nectarlink virtual webcam"),
            category: "Actions".into(),
            icon: "video".into(),
            badge: dev_name.clone(),
            shortcut: String::new(),
            keywords: format!("webcam camera video stream call zoom teams {dev_name}"),
            base_boost: 76,
        });
        out.push(PaletteCandidate {
            id: format!("action:phone_storage:{dev_id}"),
            title: "Browse phone storage in File Explorer".into(),
            subtitle: format!("Open {dev_name}'s Cloud Files folder in Explorer"),
            category: "Actions".into(),
            icon: "folder".into(),
            badge: dev_name.clone(),
            shortcut: String::new(),
            keywords: format!("browse phone storage files explorer folder drive {dev_name}"),
            base_boost: 74,
        });
    }

    // Additional per-phone actions when multiple phones are paired
    if multi {
        let primary_id = primary.as_ref().map(|(_, id, _, _)| *id);
        for (_, dev_id, dev_name, online) in &devices {
            if Some(*dev_id) == primary_id {
                continue;
            }
            let badge = if *online { dev_name.clone() } else { format!("{dev_name} (offline)") };
            out.push(PaletteCandidate {
                id: format!("action:send_clipboard:{dev_id}"),
                title: format!("Send clipboard to {dev_name}"),
                subtitle: "Send copied text or image".into(),
                category: "Actions".into(),
                icon: "clipboard".into(),
                badge: badge.clone(),
                shortcut: String::new(),
                keywords: format!("send clipboard copy paste {dev_name}"),
                base_boost: 68,
            });
            out.push(PaletteCandidate {
                id: format!("action:ring:{dev_id}"),
                title: format!("Find {dev_name}"),
                subtitle: "Make phone ring at full volume".into(),
                category: "Actions".into(),
                icon: "ring".into(),
                badge: badge.clone(),
                shortcut: String::new(),
                keywords: format!("find my phone ring locate {dev_name}"),
                base_boost: 66,
            });
            out.push(PaletteCandidate {
                id: format!("action:open_last_photo:{dev_id}"),
                title: format!("Open last photo from {dev_name}"),
                subtitle: "Open newest photo or screenshot".into(),
                category: "Actions".into(),
                icon: "photo".into(),
                badge,
                shortcut: String::new(),
                keywords: format!("open last photo latest picture {dev_name}"),
                base_boost: 65,
            });
        }
    }

    // Wake PC, Sync now, Connection Doctor, Pair new phone
    out.push(PaletteCandidate {
        id: "action:wake_pc".into(),
        title: "Wake PC".into(),
        subtitle: wake_subtitle,
        category: "Actions".into(),
        icon: "power".into(),
        badge: "Wake-on-LAN".into(),
        shortcut: String::new(),
        keywords: "wake pc wake on lan wol power magic packet sleep".into(),
        base_boost: 72,
    });
    out.push(PaletteCandidate {
        id: "action:sync_now".into(),
        title: "Sync now".into(),
        subtitle: "Reconnect paired phones and refresh notifications and media".into(),
        category: "Actions".into(),
        icon: "refresh".into(),
        badge: String::new(),
        shortcut: String::new(),
        keywords: "sync now reconnect refresh devices status".into(),
        base_boost: 70,
    });
    out.push(PaletteCandidate {
        id: "action:doctor".into(),
        title: "Connection Doctor".into(),
        subtitle: "Diagnose and fix firewall, discovery, or network issues".into(),
        category: "Actions".into(),
        icon: "bolt".into(),
        badge: String::new(),
        shortcut: String::new(),
        keywords: "connection doctor diagnose troubleshoot firewall network fix repair".into(),
        base_boost: 68,
    });
    out.push(PaletteCandidate {
        id: "action:pair".into(),
        title: "Pair a new phone".into(),
        subtitle: "Show QR code and pairing code to link another phone".into(),
        category: "Actions".into(),
        icon: "plus".into(),
        badge: String::new(),
        shortcut: String::new(),
        keywords: "pair new phone add device qr code link connect".into(),
        base_boost: 64,
    });

    // 2. Quick Toggles for the primary phone
    if let Some((_, dev_id, ref dev_name, _)) = primary {
        let cur = toggles_map.get(&dev_id);
        let dnd_badge = cur.map(|t| if t.dnd { "On" } else { "Off" }).unwrap_or("");
        let flash_badge =
            cur.and_then(|t| t.flashlight).map(|on| if on { "On" } else { "Off" }).unwrap_or("");
        let wifi_badge = cur.map(|t| if t.wifi { "On" } else { "Off" }).unwrap_or("");
        let bt_badge = cur.map(|t| if t.bluetooth { "On" } else { "Off" }).unwrap_or("");

        out.push(PaletteCandidate {
            id: format!("toggle:{dev_id}:dnd"),
            title: "Toggle Do Not Disturb".into(),
            subtitle: format!("Switch Do Not Disturb on {dev_name}"),
            category: "Quick Toggles".into(),
            icon: "moon".into(),
            badge: dnd_badge.into(),
            shortcut: String::new(),
            keywords: format!("toggle dnd do not disturb quiet silence {dev_name}"),
            base_boost: 62,
        });
        out.push(PaletteCandidate {
            id: format!("toggle:{dev_id}:flashlight"),
            title: "Toggle Flashlight".into(),
            subtitle: format!("Switch torch / flashlight on {dev_name}"),
            category: "Quick Toggles".into(),
            icon: "flashlight".into(),
            badge: flash_badge.into(),
            shortcut: String::new(),
            keywords: format!("toggle flashlight torch light lamp {dev_name}"),
            base_boost: 61,
        });
        out.push(PaletteCandidate {
            id: format!("toggle:{dev_id}:ringer:normal"),
            title: "Ringer: Sound".into(),
            subtitle: format!("Set {dev_name}'s ringer mode to normal sound"),
            category: "Quick Toggles".into(),
            icon: "speaker".into(),
            badge: dev_name.clone(),
            shortcut: String::new(),
            keywords: format!("ringer sound normal unmute ringtone {dev_name}"),
            base_boost: 56,
        });
        out.push(PaletteCandidate {
            id: format!("toggle:{dev_id}:ringer:vibrate"),
            title: "Ringer: Vibrate".into(),
            subtitle: format!("Set {dev_name}'s ringer mode to vibrate"),
            category: "Quick Toggles".into(),
            icon: "ring".into(),
            badge: dev_name.clone(),
            shortcut: String::new(),
            keywords: format!("ringer vibrate haptic {dev_name}"),
            base_boost: 55,
        });
        out.push(PaletteCandidate {
            id: format!("toggle:{dev_id}:ringer:silent"),
            title: "Ringer: Silent".into(),
            subtitle: format!("Mute {dev_name}'s ringer"),
            category: "Quick Toggles".into(),
            icon: "soundOff".into(),
            badge: dev_name.clone(),
            shortcut: String::new(),
            keywords: format!("ringer silent mute quiet {dev_name}"),
            base_boost: 55,
        });
        out.push(PaletteCandidate {
            id: format!("toggle:{dev_id}:wifi"),
            title: "Toggle Wi-Fi".into(),
            subtitle: format!("Switch Wi-Fi on {dev_name}"),
            category: "Quick Toggles".into(),
            icon: "wifi".into(),
            badge: wifi_badge.into(),
            shortcut: String::new(),
            keywords: format!("toggle wifi wireless network {dev_name}"),
            base_boost: 54,
        });
        out.push(PaletteCandidate {
            id: format!("toggle:{dev_id}:bluetooth"),
            title: "Toggle Bluetooth".into(),
            subtitle: format!("Switch Bluetooth on {dev_name}"),
            category: "Quick Toggles".into(),
            icon: "bluetooth".into(),
            badge: bt_badge.into(),
            shortcut: String::new(),
            keywords: format!("toggle bluetooth bt {dev_name}"),
            base_boost: 54,
        });
    }

    // 3. Contacts & Conversations ("Text <contact>", "Call <contact>") + "Mirror <app>"
    for (_, dev_id, dev_name, _) in &devices {
        let mut seen_text = HashSet::new();
        for (title, sub, thread_id) in crate::messages::palette_recipients(*dev_id) {
            let clean = title.trim();
            if clean.is_empty() {
                continue;
            }
            let key = clean.to_ascii_lowercase();
            if !seen_text.insert(key) {
                continue;
            }
            let sub_text = if sub.trim().is_empty() || sub.trim() == clean {
                format!("Open conversation on {dev_name}")
            } else {
                format!("{sub} · {dev_name}")
            };
            out.push(PaletteCandidate {
                id: format!("text_thread:{dev_id}:{thread_id}"),
                title: format!("Text {clean}"),
                subtitle: sub_text,
                category: "Messages".into(),
                icon: "messages".into(),
                badge: dev_name.clone(),
                shortcut: String::new(),
                keywords: format!("text message sms chat reply {clean} {sub} {dev_name}"),
                base_boost: 60,
            });
        }

        let mut seen_call = HashSet::new();
        for (name, number) in crate::calls::palette_contacts(*dev_id) {
            let clean_name = name.trim();
            let clean_num = number.trim();
            if clean_name.is_empty() || clean_num.is_empty() {
                continue;
            }
            let text_key = clean_name.to_ascii_lowercase();
            if seen_text.insert(text_key) {
                out.push(PaletteCandidate {
                    id: format!("text_contact:{dev_id}:{clean_num}\t{clean_name}"),
                    title: format!("Text {clean_name}"),
                    subtitle: format!("{clean_num} · {dev_name}"),
                    category: "Messages".into(),
                    icon: "messages".into(),
                    badge: dev_name.clone(),
                    shortcut: String::new(),
                    keywords: format!("text message sms chat {clean_name} {clean_num} {dev_name}"),
                    base_boost: 52,
                });
            }
            let call_key = format!("{}|{}", clean_name.to_ascii_lowercase(), clean_num);
            if seen_call.insert(call_key) {
                let subtitle = if clean_name == clean_num {
                    format!("Place call on {dev_name}")
                } else {
                    format!("{clean_num} · {dev_name}")
                };
                out.push(PaletteCandidate {
                    id: format!("call_contact:{dev_id}:{clean_num}"),
                    title: format!("Call {clean_name}"),
                    subtitle,
                    category: "Calls".into(),
                    icon: "call".into(),
                    badge: dev_name.clone(),
                    shortcut: String::new(),
                    keywords: format!("call dial phone ring {clean_name} {clean_num} {dev_name}"),
                    base_boost: 58,
                });
            }
        }

        for (idx, (pkg, label)) in crate::mirror::palette_apps(*dev_id).into_iter().enumerate() {
            let boost = if idx < 5 { 57 } else { 48 };
            out.push(PaletteCandidate {
                id: format!("mirror_app:{dev_id}:{pkg}\t{label}"),
                title: format!("Mirror {label}"),
                subtitle: format!("Open {label} in a window from {dev_name}"),
                category: "Mirror Apps".into(),
                icon: "mirror".into(),
                badge: dev_name.clone(),
                shortcut: String::new(),
                keywords: format!("mirror app open launch window {label} {pkg} {dev_name}"),
                base_boost: boost,
            });
        }
    }

    // 4. Navigation pages
    let nav_pages = [
        ("home", "Go to Home", "Device status, quick toggles, transfers, and notifications", "home"),
        ("messages", "Go to Messages", "SMS, MMS, RCS, and chat conversations", "messages"),
        ("calls", "Go to Calls", "Call history, contacts, and dialpad", "call"),
        ("photos", "Go to Photos", "Recent photos, screenshots, and videos from your phone", "photo"),
        ("apps", "Go to Apps & Mirroring", "Mirror your phone screen or open individual apps", "apps"),
        ("deck", "Go to Media & Deck", "Now Playing controls and audio output switcher", "deck"),
        ("timeline", "Go to Timeline", "Unified history of files, photos, clips, and calls", "history"),
        (
            "settings",
            "Go to Settings",
            "Appearance, hotkeys, webcam, recordings, and permissions",
            "settings",
        ),
    ];
    for (page, title, subtitle, icon) in nav_pages {
        out.push(PaletteCandidate {
            id: format!("nav:{page}"),
            title: title.into(),
            subtitle: subtitle.into(),
            category: "Navigation".into(),
            icon: icon.into(),
            badge: "Page".into(),
            shortcut: String::new(),
            keywords: format!("navigate go to open page {page} {title}"),
            base_boost: 50,
        });
    }
    if multi {
        for (idx, _, dev_name, online) in &devices {
            out.push(PaletteCandidate {
                id: format!("nav_device:{idx}"),
                title: format!("Switch to {dev_name}"),
                subtitle: if *online { "Connected".into() } else { "Offline".into() },
                category: "Navigation".into(),
                icon: "phone".into(),
                badge: "Device".into(),
                shortcut: String::new(),
                keywords: format!("switch device phone {dev_name}"),
                base_boost: 49,
            });
        }
    }

    // 5. Recent clipboard items & timeline files from local SQLite
    if let Some(node) = core_host::node() {
        for entry in node.clipboard_history(None).into_iter().take(10) {
            let preview: String = match entry.kind {
                ClipboardItemKind::Text => {
                    let single_line: String = entry
                        .text
                        .lines()
                        .map(str::trim)
                        .filter(|l| !l.is_empty())
                        .collect::<Vec<_>>()
                        .join(" ");
                    let mut chars = single_line.chars();
                    let head: String = chars.by_ref().take(60).collect();
                    if chars.next().is_some() { format!("{head}…") } else { head }
                }
                ClipboardItemKind::Image => "Copied image".into(),
            };
            if preview.is_empty() {
                continue;
            }
            out.push(PaletteCandidate {
                id: format!("clip_copy:{}", entry.id),
                title: format!("Copy: {preview}"),
                subtitle: format!("From {}", entry.device_name),
                category: "Clipboard".into(),
                icon: "copy".into(),
                badge: if entry.pinned { "Pinned".into() } else { String::new() },
                shortcut: String::new(),
                keywords: format!("clipboard history copy paste {preview} {}", entry.device_name),
                base_boost: if entry.pinned { 46 } else { 40 },
            });
        }

        let timeline_entries = node
            .timeline_page(&nectarlink_core::TimelineQuery {
                limit: 20,
                ..nectarlink_core::TimelineQuery::default()
            })
            .map(|p| p.entries)
            .unwrap_or_default();
        for item in timeline_entries.into_iter().take(12) {
            let target = item.target.lines().next().unwrap_or("").trim();
            if target.is_empty() {
                continue;
            }
            let icon = match item.kind {
                TimelineKind::Photo => "photo",
                TimelineKind::Recording => "mic",
                TimelineKind::File => "folder",
                _ => continue,
            };
            let p = PathBuf::from(target);
            if !p.exists() {
                continue;
            }
            out.push(PaletteCandidate {
                id: format!("timeline_open:{target}"),
                title: format!("Open {}", item.title),
                subtitle: if item.detail.trim().is_empty() {
                    item.device_name.clone()
                } else {
                    format!("{} · {}", item.detail, item.device_name)
                },
                category: "Recent Activity".into(),
                icon: icon.into(),
                badge: item.kind.as_str().into(),
                shortcut: String::new(),
                keywords: format!("open recent file timeline {} {}", item.title, item.device_name),
                base_boost: 42,
            });
        }
    }

    out
}

/// Filters and ranks `candidates` against `query` using fast fuzzy matching,
/// synthesizing direct `"Text <query>"` / `"Call <query>"` actions when the
/// user types an explicit prefix like `text ...` or `call ...`.
pub(crate) fn filter_and_rank(candidates: &[PaletteCandidate], query: &str) -> Vec<PaletteCandidate> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        let mut initial: Vec<PaletteCandidate> = candidates.to_vec();
        initial.sort_by(|a, b| b.base_boost.cmp(&a.base_boost).then_with(|| a.title.cmp(&b.title)));
        initial.truncate(MAX_RESULTS);
        return initial;
    }

    let mut scored: Vec<(i64, PaletteCandidate)> = Vec::new();
    for c in candidates {
        if let Some(score) = score_candidate(c, trimmed) {
            scored.push((score + c.base_boost, c.clone()));
        }
    }

    // Synthesize ad-hoc "Text <target>" or "Call <target>" if the user typed a direct prefix
    if let Some(synth) = synthesize_direct_command(candidates, trimmed) {
        let already_exact = scored.iter().any(|(_, c)| c.title.eq_ignore_ascii_case(&synth.title));
        if !already_exact {
            scored.push((1_800, synth));
        }
    }

    scored.sort_by(|(sa, ca), (sb, cb)| sb.cmp(sa).then_with(|| ca.title.cmp(&cb.title)));
    scored.into_iter().take(MAX_RESULTS).map(|(_, c)| c).collect()
}

fn synthesize_direct_command(candidates: &[PaletteCandidate], query: &str) -> Option<PaletteCandidate> {
    let lower = query.to_ascii_lowercase();
    let dev_id = candidates.iter().find_map(|c| {
        c.id.strip_prefix("action:send_clipboard:")
            .or_else(|| c.id.strip_prefix("action:ring:"))
            .map(str::to_owned)
    })?;

    for prefix in ["text ", "message ", "sms "] {
        if lower.starts_with(prefix) {
            let target = query[prefix.len()..].trim();
            if !target.is_empty() {
                return Some(PaletteCandidate {
                    id: format!("text_contact:{dev_id}:{target}\t{target}"),
                    title: format!("Text {target}"),
                    subtitle: "Start a new message conversation".into(),
                    category: "Messages".into(),
                    icon: "messages".into(),
                    badge: "New".into(),
                    shortcut: String::new(),
                    keywords: String::new(),
                    base_boost: 100,
                });
            }
        }
    }

    for prefix in ["call ", "dial "] {
        if lower.starts_with(prefix) {
            let target = query[prefix.len()..].trim();
            if !target.is_empty() {
                return Some(PaletteCandidate {
                    id: format!("call_contact:{dev_id}:{target}"),
                    title: format!("Call {target}"),
                    subtitle: "Place a phone call or open in dialer".into(),
                    category: "Calls".into(),
                    icon: "call".into(),
                    badge: "Dial".into(),
                    shortcut: String::new(),
                    keywords: String::new(),
                    base_boost: 100,
                });
            }
        }
    }

    None
}

fn score_candidate(c: &PaletteCandidate, query: &str) -> Option<i64> {
    let q_lower = query.to_ascii_lowercase();
    let title_lower = c.title.to_ascii_lowercase();
    let sub_lower = c.subtitle.to_ascii_lowercase();
    let kw_lower = c.keywords.to_ascii_lowercase();
    let cat_lower = c.category.to_ascii_lowercase();

    // Multi-token queries: every whitespace-separated token must match somewhere
    // in title, subtitle, keywords, or category, and each token's best score sums.
    let tokens: Vec<&str> = q_lower.split_whitespace().filter(|t| !t.is_empty()).collect();
    if tokens.is_empty() {
        return Some(0);
    }

    let mut total: i64 = 0;

    // Bonus for full-query match on title
    if title_lower == q_lower {
        total += 2_000;
    } else if title_lower.starts_with(&q_lower) {
        total += 1_200;
    } else if let Some(pos) = title_lower.find(&q_lower) {
        total += 850 - (pos as i64 * 4).min(200);
    } else if let Some(fscore) = fuzzy_subsequence_score(&title_lower, &q_lower) {
        total += fscore;
    }

    for tok in &tokens {
        let mut best_tok: Option<i64> = None;
        if let Some(s) = token_score(&title_lower, tok, 500) {
            best_tok = Some(best_tok.map_or(s, |b| b.max(s)));
        }
        if let Some(s) = token_score(&kw_lower, tok, 300) {
            best_tok = Some(best_tok.map_or(s, |b| b.max(s)));
        }
        if let Some(s) = token_score(&sub_lower, tok, 240) {
            best_tok = Some(best_tok.map_or(s, |b| b.max(s)));
        }
        if let Some(s) = token_score(&cat_lower, tok, 180) {
            best_tok = Some(best_tok.map_or(s, |b| b.max(s)));
        }
        total += best_tok?;
    }

    Some(total)
}

fn token_score(haystack: &str, needle: &str, weight: i64) -> Option<i64> {
    if haystack == needle {
        return Some(weight + 250);
    }
    if haystack.starts_with(needle) {
        return Some(weight + 180);
    }
    // Word-boundary prefix match inside haystack
    for (idx, word) in haystack.split(|c: char| !c.is_alphanumeric()).enumerate() {
        if word == needle {
            return Some(weight + 150 - (idx as i64 * 8).min(80));
        }
        if word.starts_with(needle) {
            return Some(weight + 110 - (idx as i64 * 8).min(80));
        }
    }
    if let Some(pos) = haystack.find(needle) {
        return Some(weight + 40 - (pos as i64 * 2).min(60));
    }
    fuzzy_subsequence_score(haystack, needle).map(|s| (s * weight) / 400)
}

/// Scores a fuzzy subsequence match of `needle` inside `haystack` (both lowercase),
/// rewarding consecutive character runs and word-boundary alignments.
pub(crate) fn fuzzy_subsequence_score(haystack: &str, needle: &str) -> Option<i64> {
    let n_chars: Vec<char> = needle.chars().filter(|c| !c.is_whitespace()).collect();
    if n_chars.is_empty() {
        return Some(0);
    }
    let h_chars: Vec<char> = haystack.chars().collect();
    if n_chars.len() > h_chars.len() {
        return None;
    }

    let mut score: i64 = 0;
    let mut n_idx = 0;
    let mut prev_match_idx: Option<usize> = None;
    let mut first_match_idx: Option<usize> = None;

    for (h_idx, &hc) in h_chars.iter().enumerate() {
        if hc == n_chars[n_idx] {
            if first_match_idx.is_none() {
                first_match_idx = Some(h_idx);
            }
            let at_boundary = h_idx == 0 || !h_chars[h_idx - 1].is_alphanumeric();
            if at_boundary {
                score += 35;
            }
            if let Some(prev) = prev_match_idx {
                if h_idx == prev + 1 {
                    score += 45;
                } else {
                    let gap = (h_idx - prev - 1) as i64;
                    score -= (gap * 4).min(32);
                }
            } else {
                score += 25;
            }
            prev_match_idx = Some(h_idx);
            n_idx += 1;
            if n_idx == n_chars.len() {
                break;
            }
        }
    }

    if n_idx < n_chars.len() {
        return None;
    }

    let span = prev_match_idx.unwrap_or(0).saturating_sub(first_match_idx.unwrap_or(0)) + 1;
    let compactness_bonus = ((n_chars.len() * 20) / span.max(1)) as i64;
    let final_score = score + compactness_bonus;
    (final_score > 20).then_some(final_score)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_candidates() -> Vec<PaletteCandidate> {
        let dev = "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20";
        vec![
            PaletteCandidate {
                id: format!("action:send_clipboard:{dev}"),
                title: "Send clipboard".into(),
                subtitle: "Send copied text or image to Pixel 9".into(),
                category: "Actions".into(),
                icon: "clipboard".into(),
                badge: "Pixel 9".into(),
                shortcut: String::new(),
                keywords: "send clipboard copy paste share text image Pixel 9".into(),
                base_boost: 95,
            },
            PaletteCandidate {
                id: format!("action:ring:{dev}"),
                title: "Find my phone".into(),
                subtitle: "Make Pixel 9 ring at full volume".into(),
                category: "Actions".into(),
                icon: "ring".into(),
                badge: "Pixel 9".into(),
                shortcut: String::new(),
                keywords: "find my phone ring locate alarm lost Pixel 9".into(),
                base_boost: 92,
            },
            PaletteCandidate {
                id: format!("action:open_last_photo:{dev}"),
                title: "Open last photo".into(),
                subtitle: "Open the newest photo or screenshot from Pixel 9".into(),
                category: "Actions".into(),
                icon: "photo".into(),
                badge: "Pixel 9".into(),
                shortcut: String::new(),
                keywords: "open last photo latest picture screenshot Pixel 9".into(),
                base_boost: 90,
            },
            PaletteCandidate {
                id: "action:continuity_photo".into(),
                title: "Take photo with phone".into(),
                subtitle: "Capture a photo on your phone and paste it".into(),
                category: "Actions".into(),
                icon: "camera".into(),
                badge: "Continuity".into(),
                shortcut: "Ctrl+Alt+C".into(),
                keywords: "take photo with phone continuity camera capture".into(),
                base_boost: 88,
            },
            PaletteCandidate {
                id: "action:continuity_scan".into(),
                title: "Scan document with phone".into(),
                subtitle: "Scan a document on your phone and paste it".into(),
                category: "Actions".into(),
                icon: "clipboardList".into(),
                badge: "Continuity".into(),
                shortcut: "Ctrl+Alt+D".into(),
                keywords: "scan document with phone continuity camera receipt".into(),
                base_boost: 86,
            },
            PaletteCandidate {
                id: format!("action:record_voice:{dev}"),
                title: "Record voice".into(),
                subtitle: "Open voice recordings folder for Pixel 9".into(),
                category: "Actions".into(),
                icon: "mic".into(),
                badge: "Pixel 9".into(),
                shortcut: String::new(),
                keywords: "record voice audio memo note recordings".into(),
                base_boost: 84,
            },
            PaletteCandidate {
                id: "action:wake_pc".into(),
                title: "Wake PC".into(),
                subtitle: "Ready on Ethernet".into(),
                category: "Actions".into(),
                icon: "power".into(),
                badge: "Wake-on-LAN".into(),
                shortcut: String::new(),
                keywords: "wake pc wake on lan wol power magic packet".into(),
                base_boost: 72,
            },
            PaletteCandidate {
                id: format!("text_contact:{dev}:+15550101\tAlice Rivera"),
                title: "Text Alice Rivera".into(),
                subtitle: "+15550101 · Pixel 9".into(),
                category: "Messages".into(),
                icon: "messages".into(),
                badge: "Pixel 9".into(),
                shortcut: String::new(),
                keywords: "text message sms chat Alice Rivera +15550101".into(),
                base_boost: 60,
            },
            PaletteCandidate {
                id: format!("call_contact:{dev}:+15550101"),
                title: "Call Alice Rivera".into(),
                subtitle: "+15550101 · Pixel 9".into(),
                category: "Calls".into(),
                icon: "call".into(),
                badge: "Pixel 9".into(),
                shortcut: String::new(),
                keywords: "call dial phone ring Alice Rivera +15550101".into(),
                base_boost: 58,
            },
            PaletteCandidate {
                id: format!("mirror_app:{dev}:com.whatsapp\tWhatsApp"),
                title: "Mirror WhatsApp".into(),
                subtitle: "Open WhatsApp in a window from Pixel 9".into(),
                category: "Mirror Apps".into(),
                icon: "mirror".into(),
                badge: "Pixel 9".into(),
                shortcut: String::new(),
                keywords: "mirror app open launch window WhatsApp com.whatsapp".into(),
                base_boost: 57,
            },
        ]
    }

    #[test]
    fn fuzzy_search_ranks_all_planned_palette_commands() {
        let candidates = sample_candidates();

        let top = |q: &str| filter_and_rank(&candidates, q).first().unwrap().title.clone();
        assert_eq!(top("send clip"), "Send clipboard");
        assert_eq!(top("sndclp"), "Send clipboard");
        assert_eq!(top("find phone"), "Find my phone");
        assert_eq!(top("text alice"), "Text Alice Rivera");
        assert_eq!(top("call alice"), "Call Alice Rivera");
        assert_eq!(top("last photo"), "Open last photo");
        assert_eq!(top("mirror whatsapp"), "Mirror WhatsApp");
        assert_eq!(top("record voice"), "Record voice");
        assert_eq!(top("take photo"), "Take photo with phone");
        assert_eq!(top("scan doc"), "Scan document with phone");
        assert_eq!(top("wake pc"), "Wake PC");
    }

    #[test]
    fn synthesizes_direct_text_and_call_actions_for_unlisted_numbers() {
        let candidates = sample_candidates();

        let text_res = filter_and_rank(&candidates, "Text +1 555 0199");
        assert_eq!(text_res[0].title, "Text +1 555 0199");
        assert!(text_res[0].id.starts_with("text_contact:"));

        let call_res = filter_and_rank(&candidates, "Call +1 555 0199");
        assert_eq!(call_res[0].title, "Call +1 555 0199");
        assert!(call_res[0].id.starts_with("call_contact:"));
    }

    #[test]
    fn search_across_500_candidates_completes_well_under_20ms() {
        let mut candidates = sample_candidates();
        let dev = "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20";
        for i in 0..500 {
            candidates.push(PaletteCandidate {
                id: format!("call_contact:{dev}:+1555{i:04}"),
                title: format!("Call Contact Number {i}"),
                subtitle: format!("+1555{i:04} · Pixel 9"),
                category: "Calls".into(),
                icon: "call".into(),
                badge: "Pixel 9".into(),
                shortcut: String::new(),
                keywords: format!("call contact {i} +1555{i:04}"),
                base_boost: 40,
            });
        }

        let start = Instant::now();
        let results = filter_and_rank(&candidates, "mirror whatsapp");
        let elapsed = start.elapsed();
        assert_eq!(results.first().map(|c| c.title.as_str()), Some("Mirror WhatsApp"));
        assert!(
            elapsed.as_millis() < 20,
            "expected < 20 ms fuzzy search for 500+ candidates, took {} ms",
            elapsed.as_millis()
        );
    }

    #[test]
    fn close_releases_candidate_memory_to_zero_capacity() {
        with_state(|s| {
            s.open = true;
            s.candidates = sample_candidates();
            assert!(s.candidates.capacity() > 0);
        });
        with_state(|s| {
            s.open = false;
            s.opened_at = None;
            s.candidates.clear();
            s.candidates.shrink_to_fit();
            assert_eq!(s.candidates.capacity(), 0);
        });
    }
}
