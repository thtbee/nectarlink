// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS menu bar tray integration backed by Qt `QSystemTrayIcon` + `QMenu`.

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
};

use crate::bridge::native::ffi;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    Open,
    CommandPalette,
    Shelf,
    TakePhoto,
    ScanDocument,
    FindPhone,
    OpenLinkOnPhone,
    ToggleNotificationsPause,
    Quit,
    Resumed,
    AppearanceChanged,
}

#[derive(Debug, Clone)]
pub struct MenuLabels {
    pub open: String,
    pub command_palette: String,
    pub shelf: String,
    pub take_photo: String,
    pub scan_document: String,
    pub find_phone: String,
    pub open_link: String,
    pub pause_notifications: String,
    pub resume_notifications: String,
    pub quit: String,
}

pub struct Tray {
    labels: MenuLabels,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    icon_file: Option<PathBuf>,
}

impl std::fmt::Debug for Tray {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tray").finish_non_exhaustive()
    }
}

fn write_tray_icon_png() -> Option<PathBuf> {
    let rgba = crate::mark::render(64);
    let mut bgra = rgba;
    for px in bgra.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
    let bitmap = crate::win::image::Bitmap { width: 64, height: 64, bgra };
    let png = crate::win::image::encode_png(&bitmap).ok()?;
    let path = std::env::temp_dir().join(format!("nectarlink-tray-{}.png", std::process::id()));
    std::fs::write(&path, png).ok()?;
    Some(path)
}

fn parse_tray_action(action: &str) -> Option<TrayEvent> {
    match action {
        "open" => Some(TrayEvent::Open),
        "command_palette" => Some(TrayEvent::CommandPalette),
        "shelf" => Some(TrayEvent::Shelf),
        "take_photo" => Some(TrayEvent::TakePhoto),
        "scan_document" => Some(TrayEvent::ScanDocument),
        "find_phone" => Some(TrayEvent::FindPhone),
        "open_link" => Some(TrayEvent::OpenLinkOnPhone),
        "toggle_pause" => Some(TrayEvent::ToggleNotificationsPause),
        "quit" => Some(TrayEvent::Quit),
        _ => None,
    }
}

impl Tray {
    pub fn create(
        tooltip: &str,
        labels: MenuLabels,
        handler: impl Fn(TrayEvent) + Send + 'static,
    ) -> Result<Tray, String> {
        let icon_file = write_tray_icon_png();
        let icon_path = icon_file.as_ref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        ffi::linux_tray_setup(
            &icon_path,
            tooltip,
            &labels.open,
            &labels.command_palette,
            &labels.shelf,
            &labels.take_photo,
            &labels.scan_document,
            &labels.find_phone,
            &labels.open_link,
            &labels.pause_notifications,
            &labels.quit,
        );
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = thread::Builder::new()
            .name("mac-tray".into())
            .spawn(move || {
                while !worker_stop.load(Ordering::Acquire) {
                    let action = ffi::linux_tray_wait_action(250);
                    if let Some(event) = parse_tray_action(&action) {
                        handler(event);
                    }
                }
            })
            .map_err(|e| format!("failed to spawn macOS tray thread: {e}"))?;
        Ok(Tray { labels, stop, worker: Some(worker), icon_file })
    }

    pub fn set_tooltip(&self, text: &str) {
        ffi::linux_tray_set_tooltip(text);
    }

    pub fn set_notifications_paused(&self, paused: bool) {
        let label = if paused { &self.labels.resume_notifications } else { &self.labels.pause_notifications };
        ffi::linux_tray_set_pause_label(label);
    }

    pub fn set_find_phone_enabled(&self, enabled: bool) {
        ffi::linux_tray_set_find_phone_enabled(enabled);
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        ffi::linux_tray_teardown();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Some(path) = self.icon_file.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

pub fn update_hotkeys(_photo_spec: &str, _scan_spec: &str, _palette_spec: &str, _shelf_spec: &str) {}

#[allow(dead_code)]
pub fn record_foreground_window() {}

pub fn apply_no_activate_style_by_title(_title: &str) {}

pub fn focus_window(_hwnd: isize) -> bool {
    false
}

pub fn is_window_alive_and_visible(_hwnd: isize) -> bool {
    false
}

pub fn send_paste_fallback_if_not_foreground(_hwnd: isize) {}

pub fn snapshot_target_window() -> Option<(isize, String)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_actions_map_to_events() {
        assert_eq!(parse_tray_action("open"), Some(TrayEvent::Open));
        assert_eq!(parse_tray_action("command_palette"), Some(TrayEvent::CommandPalette));
        assert_eq!(parse_tray_action("shelf"), Some(TrayEvent::Shelf));
        assert_eq!(parse_tray_action("take_photo"), Some(TrayEvent::TakePhoto));
        assert_eq!(parse_tray_action("scan_document"), Some(TrayEvent::ScanDocument));
        assert_eq!(parse_tray_action("find_phone"), Some(TrayEvent::FindPhone));
        assert_eq!(parse_tray_action("open_link"), Some(TrayEvent::OpenLinkOnPhone));
        assert_eq!(parse_tray_action("toggle_pause"), Some(TrayEvent::ToggleNotificationsPause));
        assert_eq!(parse_tray_action("quit"), Some(TrayEvent::Quit));
        assert_eq!(parse_tray_action(""), None);
        assert_eq!(parse_tray_action("unknown"), None);
    }
}
