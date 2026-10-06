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
/// saved file's path.
pub const TOAST_GROUP: &str = "files";
/// The "Show in folder" toast action.
pub const ACTION_SHOW: &str = "show";

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
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        let files = match tokio::task::spawn_blocking(move || nectarlink_core::outgoing_paths(&paths)).await {
            Ok(Ok(files)) if !files.is_empty() => files,
            Ok(Ok(_)) => return show_message("There's nothing to send in there."),
            Ok(Err(Error::TooLarge)) => return show_message("That's too many files to send at once."),
            Ok(Err(e)) => return show_message(describe(&e)),
            Err(_) => return,
        };
        match node.send_files(device, files).await {
            Ok(_) => {}
            Err(Error::Denied) => show_message("Files are turned off for that device."),
            Err(e) => show_message(describe(&e)),
        }
    });
}

pub fn cancel(id: &str) {
    if let Some(node) = core_host::node() {
        node.cancel_transfer(id);
    }
}

/// Opens a received file with its app (or a folder in File Explorer).
pub fn open(path: &Path) {
    if let Err(e) = std::process::Command::new("explorer").arg(path).spawn() {
        tracing::warn!(error = %e, "can't open a received file");
    }
}

/// Shows a received file or folder in File Explorer, selected.
pub fn show_in_folder(path: &Path) {
    let mut select = std::ffi::OsString::from("/select,");
    select.push(path);
    if let Err(e) = std::process::Command::new("explorer").arg(select).spawn() {
        tracing::warn!(error = %e, "can't show a received file");
    }
}

/// A Windows notification when files from a phone are saved.
pub fn on_event(event: &NodeEvent) {
    let NodeEvent::Transfer(t) = event else { return };
    // Photos the user asked to open or copy say nothing about being saved.
    if t.direction != Direction::Incoming || crate::photos::handles(&t.id) {
        return;
    }
    let TransferState::Done { saved } = &t.state else { return };
    let Some(first) = saved.first() else { return };
    let device = core_host::host().hub.read(|s| s.name_of(&t.device)).unwrap_or_else(|| "your phone".into());
    let title = title(t);
    toast::show(Toast {
        device: TOAST_GROUP.into(),
        key: first.to_string_lossy().into_owned(),
        title,
        body: format!("From {device}, saved in Downloads\\Nectarlink"),
        attribution: "Nectarlink".into(),
        icon: None,
        image: None,
        actions: vec![(ACTION_SHOW.into(), "Show in folder".into())],
        reply: None,
        silent: false,
        progress: None,
        call: false,
    });
}
