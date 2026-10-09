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
            Ok(_) => {}
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
    }
}

/// Opens a received file with its app (or a folder in File Explorer).
pub fn open(path: &Path) {
    if path.to_str().is_some_and(|s| s.starts_with("req:")) {
        crate::bridge::app::request_activation();
        return;
    }
    if let Err(e) = std::process::Command::new("explorer").arg(path).spawn() {
        tracing::warn!(error = %e, "can't open a received file");
    }
}

/// Shows a received file or folder in File Explorer, selected.
pub fn show_in_folder(path: &Path) {
    if path.to_str().is_some_and(|s| s.starts_with("req:")) {
        crate::bridge::app::request_activation();
        return;
    }
    let mut select = std::ffi::OsString::from("/select,");
    select.push(path);
    if let Err(e) = std::process::Command::new("explorer").arg(select).spawn() {
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
            body: "Accept in Nectarlink to save to Downloads\\Nectarlink".into(),
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
    let auto_open = t.open_on_arrival
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
        attribution: "Nectarlink".into(),
        icon: None,
        image: None,
        actions: vec![(ACTION_OPEN.into(), "Open".into()), (ACTION_SHOW.into(), "Show in folder".into())],
        reply: None,
        silent: false,
        progress: None,
        call: false,
    });
}
