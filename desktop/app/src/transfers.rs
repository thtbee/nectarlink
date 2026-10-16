// SPDX-License-Identifier: GPL-3.0-or-later
//! File transfers on this PC (docs/protocol/files.md): sending what the
//! user picks or drops, opening what arrived, and a Windows notification
//! when files from a phone are saved.

use std::path::{Path, PathBuf};

use nectarlink_core::{DeviceId, Direction, Error, NodeEvent, Transfer, TransferState};

use crate::{
    bridge::app::{describe, show_message},
    core_host,
    win::toast::{self, Toast},
};

/// The toast "device" for received-file notifications; their key is the
/// saved file's path (or `req:<id>` for a pending incoming transfer request).
pub const TOAST_GROUP: &str = "files";
/// The "Open" toast action.
pub const ACTION_OPEN: &str = "open";
/// The "Show in folder" toast action.
pub const ACTION_SHOW: &str = "show";
/// The "Accept" toast action for an incoming LocalSend transfer request.
pub const ACTION_ACCEPT: &str = "accept";
/// The "Decline" toast action for an incoming LocalSend transfer request.
pub const ACTION_DECLINE: &str = "decline";

/// What a transfer is called: its file or folder's name, or how many.
pub fn title(t: &Transfer) -> String {
    match t.names.as_slice() {
        [one] => one.clone(),
        names if names.len() == t.files => format!("{} files", names.len()),
        names => format!("{} items", names.len()),
    }
}

/// Sends files and folders to a device; the transfer list shows how it goes.
pub fn send(device: DeviceId, paths: Vec<PathBuf>) {
    send_with_mode(device, paths, false);
}

/// Hands off a document to a device and asks the receiver to open it with its
/// default app once it arrives.
pub fn send_handoff(device: DeviceId, paths: Vec<PathBuf>) {
    send_with_mode(device, paths, true);
}

fn send_with_mode(device: DeviceId, paths: Vec<PathBuf>, handoff: bool) {
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        let files = match tokio::task::spawn_blocking(move || nectarlink_core::outgoing_paths(&paths)).await {
            Ok(Ok(files)) if !files.is_empty() => files,
            Ok(Ok(_)) => return show_message("There's nothing to send in there."),
            Ok(Err(Error::TooLarge)) => return show_message("That's too many files to send at once."),
            Ok(Err(e)) => return show_message(describe(&e)),
            Err(_) => return,
        };
        let res = if handoff {
            node.send_handoff_files(device, files).await
        } else {
            node.send_files(device, files).await
        };
        match res {
            Ok(transfer_id) => {
                crate::bridge::app::show_offline_queued_toast(device, Some(transfer_id));
            }
            Err(Error::Denied) => show_message("Files are turned off for that device."),
            Err(e) => show_message(describe(&e)),
        }
    });
}

/// Accepts an incoming transfer waiting in `TransferState::Requested`.
pub fn accept(id: &str) {
    if let Some(node) = core_host::node() {
        node.accept_transfer(id);
    }
}

pub fn cancel(id: &str) {
    if let Some(node) = core_host::node() {
        node.cancel_transfer(id);
        let _ = node.cancel_queue_item(id);
    }
}

/// Cancels a single queued outgoing item by ID.
pub fn cancel_queue_item(id: &str) {
    let id_owned = id.trim();
    if id_owned.is_empty() {
        return;
    }
    cancel(id_owned);
}

/// Cancels all active (`Waiting` / `Sending`) queued items for `device` (or all paired devices when `None`).
pub fn cancel_all_queued(device: Option<DeviceId>) {
    if let Some(node) = core_host::node() {
        let _ = node.cancel_all_queue_items(device);
    }
}

/// Retries a failed or expired queue item by ID.
pub fn retry_queue_item(id: &str) {
    let Some(node) = core_host::node() else { return };
    let id_trimmed = id.trim();
    if id_trimmed.is_empty() {
        return;
    }
    if let Err(e) = node.retry_queue_item(id_trimmed) {
        show_message(describe(&e));
    }
}

/// Clears all failed and expired items from the send queue (for `device`, or all peers when `None`).
pub fn clear_finished_queue(device: Option<DeviceId>) {
    if let Some(node) = core_host::node() {
        let _ = node.clear_finished_queue(device);
    }
}

/// Serializes the current `send_queue` into `(json, waiting_count, total_count, has_failed_or_expired)`.
pub fn send_queue_snapshot() -> (String, i32, i32, bool) {
    use nectarlink_core::{QueueKind, QueueStatus};

    core_host::host().hub.read(|s| {
        let mut waiting_count: i32 = 0;
        let mut has_failed_or_expired = false;
        let mut arr = Vec::with_capacity(s.send_queue.len());

        for item in &s.send_queue {
            let (status_str, reason) = match &item.status {
                QueueStatus::Waiting => {
                    waiting_count = waiting_count.saturating_add(1);
                    ("waiting", "")
                }
                QueueStatus::Sending => {
                    waiting_count = waiting_count.saturating_add(1);
                    ("sending", "")
                }
                QueueStatus::Failed { error } => {
                    has_failed_or_expired = true;
                    ("failed", error.as_str())
                }
                QueueStatus::Expired { error } => {
                    has_failed_or_expired = true;
                    ("expired", error.as_str())
                }
            };
            let icon = match item.kind {
                QueueKind::Folder => "folder",
                QueueKind::Files | QueueKind::HandoffDocument | QueueKind::Recording => "file",
                QueueKind::Clipboard | QueueKind::ClipboardImage => "clipboard",
                QueueKind::Link => "link",
                QueueKind::TaskNotify => "task",
                QueueKind::CameraRequest => "camera",
            };
            let device_name = s.name_of(&item.peer).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| {
                if item.peer_name.trim().is_empty() {
                    "your phone".to_owned()
                } else {
                    item.peer_name.clone()
                }
            });
            let progress = if item.bytes_total == 0 {
                0.0
            } else {
                (item.bytes_done as f64 / item.bytes_total as f64).clamp(0.0, 1.0)
            };
            arr.push(serde_json::json!({
                "id": item.id,
                "deviceId": item.peer.to_string(),
                "deviceName": device_name,
                "kind": item.kind.as_str(),
                "icon": icon,
                "title": item.title,
                "subtitle": item.detail,
                "status": status_str,
                "reason": reason,
                "totalBytes": item.bytes_total,
                "doneBytes": item.bytes_done,
                "progress": progress,
                "filesTotal": item.files_total,
                "filesDone": item.files_done,
                "createdAt": item.created_at,
                "expiresAt": item.expires_at,
                "attempts": item.attempts,
            }));
        }

        let total_count = i32::try_from(s.send_queue.len()).unwrap_or(i32::MAX);
        let json = serde_json::Value::Array(arr).to_string();
        (json, waiting_count, total_count, has_failed_or_expired)
    })
}

/// Pauses an active transfer (`Waiting` or `Running`).
pub fn pause(id: &str) {
    if let Some(node) = core_host::node() {
        node.pause_transfer(id);
    }
}

/// Resumes a paused transfer (`TransferState::Paused`).
pub fn resume(id: &str) {
    if let Some(node) = core_host::node() {
        node.resume_transfer(id);
    }
}

/// Opens a received file with its app (or a folder in File Explorer).
pub fn open(path: &Path) {
    if path.to_str().is_some_and(|s| s.starts_with("req:")) {
        crate::bridge::app::request_activation();
        return;
    }
    #[cfg(windows)]
    if let Err(e) = std::process::Command::new("explorer").arg(path).spawn() {
        tracing::warn!(error = %e, "can't open a received file");
    }
    #[cfg(target_os = "linux")]
    if let Err(e) = crate::win::shell::open_file(path) {
        tracing::warn!(error = %e, "can't open a received file");
    }
}

/// Shows a received file or folder in File Explorer, selected.
pub fn show_in_folder(path: &Path) {
    if path.to_str().is_some_and(|s| s.starts_with("req:")) {
        crate::bridge::app::request_activation();
        return;
    }
    #[cfg(windows)]
    {
        let mut select = std::ffi::OsString::from("/select,");
        select.push(path);
        if let Err(e) = std::process::Command::new("explorer").arg(select).spawn() {
            tracing::warn!(error = %e, "can't show a received file");
        }
    }
    #[cfg(target_os = "linux")]
    if let Err(e) = crate::win::shell::select_in_explorer(path) {
        tracing::warn!(error = %e, "can't show a received file");
    }
}

/// A Windows notification when files from a phone or LocalSend peer are requested or saved.
pub fn on_event(event: &NodeEvent) {
    let NodeEvent::Transfer(t) = event else { return };
    // Photos the user asked to open or copy say nothing about being saved.
    if t.direction != Direction::Incoming || crate::photos::handles(&t.id) {
        return;
    }
    if t.state == TransferState::Requested {
        let device = core_host::host()
            .hub
            .read(|s| s.name_of(&t.device))
            .or_else(|| {
                core_host::node()
                    .and_then(|n| n.localsend_peers().into_iter().find(|p| p.id == t.device).map(|p| p.alias))
            })
            .unwrap_or_else(|| "Nearby device".into());
        toast::show(Toast {
            device: TOAST_GROUP.into(),
            key: format!("req:{}", t.id),
            title: format!("{device} wants to send {}", title(t)),
            body: "Accept in NectarLink to save to Downloads\\Nectarlink".into(),
            attribution: "LocalSend".into(),
            icon: None,
            image: None,
            actions: vec![(ACTION_ACCEPT.into(), "Accept".into()), (ACTION_DECLINE.into(), "Decline".into())],
            reply: None,
            silent: false,
            progress: None,
            call: false,
        });
        return;
    }
    let TransferState::Done { saved } = &t.state else { return };
    if t.recording {
        return crate::recordings::on_done(t.clone());
    }
    let Some(first) = saved.first() else { return };
    if saved.len() <= 10 {
        let _ = crate::clipboard::copy_received_files_to_clipboard(saved);
    }
    let auto_open = t.open_on_arrival
        && crate::settings::open_shared_files()
        && saved.len() == 1
        && first.is_file()
        && first.file_name().and_then(|n| n.to_str()).is_some_and(nectarlink_core::is_safe_handoff_document);
    if auto_open {
        open(first);
    }
    let device = core_host::host()
        .hub
        .read(|s| s.name_of(&t.device))
        .or_else(|| {
            core_host::node()
                .and_then(|n| n.localsend_peers().into_iter().find(|p| p.id == t.device).map(|p| p.alias))
        })
        .unwrap_or_else(|| "your phone".into());
    let title = title(t);
    toast::show(Toast {
        device: TOAST_GROUP.into(),
        key: first.to_string_lossy().into_owned(),
        title,
        body: if auto_open {
            format!("From {device}, opened from Downloads\\Nectarlink")
        } else {
            format!("From {device}, saved in Downloads\\Nectarlink")
        },
        attribution: "NectarLink".into(),
        icon: None,
        image: None,
        actions: vec![(ACTION_OPEN.into(), "Open".into()), (ACTION_SHOW.into(), "Show in folder".into())],
        reply: None,
        silent: false,
        progress: None,
        call: false,
    });
}
