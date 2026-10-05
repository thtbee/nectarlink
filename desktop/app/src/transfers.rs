// SPDX-License-Identifier: GPL-3.0-or-later
//! File transfers on this PC (docs/protocol/files.md): sending what the
//! user picks or drops, opening what arrived, and a Windows notification
//! when files from a phone are saved.

use std::path::{Path, PathBuf};

use nectarlink_core::{DeviceId, Direction, Error, FileSource, NodeEvent, OutgoingFile, TransferState};

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

/// Sends files to a device; the transfer list shows how it goes.
pub fn send(device: DeviceId, paths: Vec<PathBuf>) {
    let Some(node) = core_host::node() else { return };
    let files: Vec<OutgoingFile> = paths
        .into_iter()
        .filter(|p| p.is_file())
        .map(|path| OutgoingFile {
            name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            source: FileSource::Path(path),
        })
        .collect();
    if files.is_empty() {
        show_message("Folders can't be sent yet; pick files instead.");
        return;
    }
    core_host::spawn(async move {
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

/// Opens a received file with its app.
pub fn open(path: &Path) {
    if let Err(e) = std::process::Command::new("explorer").arg(path).spawn() {
        tracing::warn!(error = %e, "can't open a received file");
    }
}

/// Shows a received file in File Explorer, selected.
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
    if t.direction != Direction::Incoming {
        return;
    }
    let TransferState::Done { saved } = &t.state else { return };
    let Some(first) = saved.first() else { return };
    let device = core_host::host().hub.read(|s| s.name_of(&t.device)).unwrap_or_else(|| "your phone".into());
    let title = match saved.len() {
        1 => first.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        n => format!("{n} files"),
    };
    toast::show(Toast {
        device: TOAST_GROUP.into(),
        key: first.to_string_lossy().into_owned(),
        title,
        body: format!("From {device}, saved in Downloads\\Nectarlink"),
        attribution: "Nectarlink".into(),
        icon: None,
        actions: vec![(ACTION_SHOW.into(), "Show in folder".into())],
        reply: None,
        silent: false,
    });
}
