// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS menu bar tray integration backed by Qt `QSystemTrayIcon` + `QMenu`.

use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
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
    HandoffPull,
    HandoffPush,
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

/// Whether the menu bar icon is the one-color glyph, which macOS draws white
/// or black to match the menu bar, rather than the coloured tile.
static ONE_COLOUR: AtomicBool = AtomicBool::new(false);

/// The icon files in the temporary folder: the coloured tile, and the glyph.
static COLOUR_FILE: Mutex<Option<PathBuf>> = Mutex::new(None);
static GLYPH_FILE: Mutex<Option<PathBuf>> = Mutex::new(None);

/// The glyph's size in pixels: 18 points at 2x, the height of a menu bar icon.
const GLYPH_PIXELS: usize = 36;

fn lock(file: &Mutex<Option<PathBuf>>) -> std::sync::MutexGuard<'_, Option<PathBuf>> {
    file.lock().unwrap_or_else(|e| e.into_inner())
}

/// The glyph as a black PNG: the system turns it white on a dark menu bar.
fn write_glyph_png() -> Option<PathBuf> {
    let mut bgra = crate::mark::render_glyph(GLYPH_PIXELS, [0, 0, 0]);
    for px in bgra.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
    let bitmap = crate::win::image::Bitmap { width: GLYPH_PIXELS as u32, height: GLYPH_PIXELS as u32, bgra };
    let png = crate::win::image::encode_png(&bitmap).ok()?;
    let path = std::env::temp_dir().join(format!("nectarlink-tray-glyph-{}.png", std::process::id()));
    std::fs::write(&path, png).ok()?;
    Some(path)
}

/// Puts the icon the setting asks for in the menu bar. Nothing happens until
/// the tray exists; [`Tray::create`] applies it again.
fn apply_icon() {
    let one_colour = ONE_COLOUR.load(Ordering::Acquire);
    let glyph = if one_colour {
        lock(&GLYPH_FILE).get_or_insert_with(|| write_glyph_png().unwrap_or_default()).clone()
    } else {
        PathBuf::new()
    };
    let (one_colour, path) = if glyph.as_os_str().is_empty() {
        (false, lock(&COLOUR_FILE).clone().unwrap_or_default())
    } else {
        (true, glyph)
    };
    ffi::linux_tray_set_one_colour(one_colour, "", &path.to_string_lossy());
}

/// Switches the menu bar icon between the one-color glyph and the colour one.
pub fn set_one_colour(on: bool) {
    ONE_COLOUR.store(on, Ordering::Release);
    apply_icon();
}

fn parse_tray_action(action: &str) -> Option<TrayEvent> {
    match action {
        "OPEN" => Some(TrayEvent::Open),
        "COMMAND_PALETTE" => Some(TrayEvent::CommandPalette),
        "SHELF" => Some(TrayEvent::Shelf),
        "TAKE_PHOTO" => Some(TrayEvent::TakePhoto),
        "SCAN_DOCUMENT" => Some(TrayEvent::ScanDocument),
        "FIND_PHONE" => Some(TrayEvent::FindPhone),
        "OPEN_LINK" => Some(TrayEvent::OpenLinkOnPhone),
        "PAUSE_NOTIFICATIONS" => Some(TrayEvent::ToggleNotificationsPause),
        "QUIT" => Some(TrayEvent::Quit),
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
        *lock(&COLOUR_FILE) = icon_file.clone();
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
        apply_icon();
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
        *lock(&COLOUR_FILE) = None;
        if let Some(path) = lock(&GLYPH_FILE).take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

pub fn update_hotkeys(
    _photo_spec: &str,
    _scan_spec: &str,
    _palette_spec: &str,
    _shelf_spec: &str,
    _handoff_pull_spec: &str,
    _handoff_push_spec: &str,
) {
}

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
        assert_eq!(parse_tray_action("OPEN"), Some(TrayEvent::Open));
        assert_eq!(parse_tray_action("COMMAND_PALETTE"), Some(TrayEvent::CommandPalette));
        assert_eq!(parse_tray_action("SHELF"), Some(TrayEvent::Shelf));
        assert_eq!(parse_tray_action("TAKE_PHOTO"), Some(TrayEvent::TakePhoto));
        assert_eq!(parse_tray_action("SCAN_DOCUMENT"), Some(TrayEvent::ScanDocument));
        assert_eq!(parse_tray_action("FIND_PHONE"), Some(TrayEvent::FindPhone));
        assert_eq!(parse_tray_action("OPEN_LINK"), Some(TrayEvent::OpenLinkOnPhone));
        assert_eq!(parse_tray_action("PAUSE_NOTIFICATIONS"), Some(TrayEvent::ToggleNotificationsPause));
        assert_eq!(parse_tray_action("QUIT"), Some(TrayEvent::Quit));
        assert_eq!(parse_tray_action(""), None);
        assert_eq!(parse_tray_action("unknown"), None);
    }
}
