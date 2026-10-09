// SPDX-License-Identifier: GPL-3.0-or-later
//! Continuity Camera coordinator for Windows: initiates "Take photo with phone"
//! and "Scan document with phone" from the tray menu, global hotkeys, or the
//! Nectarlink UI, remembers the external foreground window (`HWND`) that was
//! active when the request started, places the captured image onto the Windows
//! clipboard on arrival, and either restores focus + synthesizes `Ctrl+V` into
//! the original target window or prompts the user if that window was closed.

use std::{
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use nectarlink_core::{CameraCaptureMode, DeviceId, Error, LinkState, NodeEvent};

use crate::{core_host, state::Changes};

static REQUEST_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone)]
struct ActiveRequest {
    request_id: String,
    device: DeviceId,
    device_name: String,
    mode: CameraCaptureMode,
    target_hwnd: Option<isize>,
    target_title: String,
}

#[derive(Debug, Clone)]
struct PendingPastePrompt {
    title: String,
    body: String,
    target_name: String,
}

#[derive(Debug, Default)]
struct ContinuityState {
    active: Option<ActiveRequest>,
    prompt: Option<PendingPastePrompt>,
}

static STATE: Mutex<ContinuityState> = Mutex::new(ContinuityState { active: None, prompt: None });

/// Snapshot of Continuity Camera state exposed to QML via `AppController`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContinuityView {
    pub busy: bool,
    pub mode: String,
    pub status: String,
    pub paste_prompt_visible: bool,
    pub paste_prompt_title: String,
    pub paste_prompt_body: String,
    pub paste_target_name: String,
}

/// Pure decision for what to do after placing a captured image on the clipboard.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DeliveryDecision {
    /// The remembered target window is still alive and visible: focus it and paste.
    AutoPaste { hwnd: isize, toast: String },
    /// A target window was remembered when the request started, but is no longer
    /// alive/visible when the capture arrived. Do not blindly paste elsewhere;
    /// ask the user in Nectarlink.
    PromptMissingTarget { title: String, body: String, target_name: String },
    /// No external target window was remembered when the request started.
    ClipboardOnly { toast: String },
}

fn decide_delivery_action(
    mode: CameraCaptureMode,
    device_name: &str,
    target: Option<(isize, &str)>,
    target_still_alive: bool,
) -> DeliveryDecision {
    let (item_cap, item_lower) = match mode {
        CameraCaptureMode::Photo => ("Photo", "photo"),
        CameraCaptureMode::Scan => ("Scanned document", "scanned document"),
    };
    if let Some((hwnd, raw_title)) = target
        && hwnd != 0
    {
        let trimmed = raw_title.trim();
        let target_name =
            if trimmed.is_empty() { "your previous window".to_owned() } else { trimmed.to_owned() };
        if target_still_alive {
            return DeliveryDecision::AutoPaste {
                hwnd,
                toast: format!("{item_cap} from {device_name} pasted into {target_name}"),
            };
        }
        return DeliveryDecision::PromptMissingTarget {
            title: format!("{item_cap} ready on clipboard"),
            body: format!(
                "“{target_name}” is no longer open, so Nectarlink didn’t paste automatically. Switch to the window where you want your {item_lower} from {device_name} and click Paste into active window, or press Ctrl+V anywhere."
            ),
            target_name,
        };
    }
    DeliveryDecision::ClipboardOnly { toast: format!("{item_cap} from {device_name} copied to clipboard") }
}

fn next_request_id() -> String {
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let seq = REQUEST_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("cc-{ms}-{seq}")
}

fn notify_continuity() {
    core_host::host().hub.update(|_| Changes::CONTINUITY);
}

/// Returns a snapshot of the current Continuity Camera UI state.
pub fn view() -> ContinuityView {
    let guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let (busy, mode, status) = if let Some(active) = &guard.active {
        let mode_str = active.mode.as_str().to_owned();
        let status_str = match active.mode {
            CameraCaptureMode::Photo => format!("Take a photo on {}…", active.device_name),
            CameraCaptureMode::Scan => format!("Scan a document on {}…", active.device_name),
        };
        (true, mode_str, status_str)
    } else {
        (false, String::new(), String::new())
    };
    let (paste_prompt_visible, paste_prompt_title, paste_prompt_body, paste_target_name) =
        if let Some(prompt) = &guard.prompt {
            (true, prompt.title.clone(), prompt.body.clone(), prompt.target_name.clone())
        } else {
            (false, String::new(), String::new(), String::new())
        };
    ContinuityView {
        busy,
        mode,
        status,
        paste_prompt_visible,
        paste_prompt_title,
        paste_prompt_body,
        paste_target_name,
    }
}

/// Starts a Continuity Camera capture (`"photo"` or `"scan"`) on the active
/// paired phone, remembering the current external foreground window.
pub fn start(mode_raw: &str) {
    start_for_index(0, mode_raw);
}

/// Starts a Continuity Camera capture preferring the paired device at
/// `preferred_index` if it is online, or the first online phone otherwise.
pub fn start_for_index(preferred_index: usize, mode_raw: &str) {
    let mode = CameraCaptureMode::parse(mode_raw).unwrap_or(CameraCaptureMode::Photo);
    let target = crate::win::tray::snapshot_target_window();

    let chosen = core_host::host().hub.read(|s| {
        s.devices
            .get(preferred_index)
            .filter(|d| matches!(d.link, LinkState::Online { .. }))
            .or_else(|| s.devices.iter().find(|d| matches!(d.link, LinkState::Online { .. })))
            .map(|d| (d.id, d.info.name.clone()))
    });
    let Some((device_id, device_name)) = chosen else {
        crate::bridge::app::show_message("Connect your phone to take a photo or scan a document.");
        return;
    };

    let request_id = next_request_id();
    let (target_hwnd, target_title) = match target {
        Some((hwnd, title)) => (Some(hwnd), title),
        None => (None, String::new()),
    };

    let previous = {
        let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
        guard.prompt = None;
        guard.active.replace(ActiveRequest {
            request_id: request_id.clone(),
            device: device_id,
            device_name: device_name.clone(),
            mode,
            target_hwnd,
            target_title,
        })
    };
    notify_continuity();

    let open_msg = match mode {
        CameraCaptureMode::Photo => format!("Opening camera on {device_name}…"),
        CameraCaptureMode::Scan => format!("Opening document scanner on {device_name}…"),
    };
    crate::bridge::app::show_message(open_msg);

    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        if let Some(prev) = previous {
            let _ = node.cancel_camera_capture(prev.device, prev.request_id, Some("replaced".into())).await;
        }
        if let Err(err) = node.request_camera_capture(device_id, request_id.clone(), mode).await {
            let should_report = {
                let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
                if guard.active.as_ref().is_some_and(|a| a.request_id == request_id) {
                    guard.active = None;
                    true
                } else {
                    false
                }
            };
            if should_report {
                notify_continuity();
                let msg = match err {
                    Error::Denied => format!("Allow Photos access for {device_name} in Settings first."),
                    Error::Unsupported => format!("{device_name} doesn’t support Continuity Camera yet."),
                    Error::Offline => format!("{device_name} is not connected."),
                    other => format!("Couldn’t open camera on {device_name}: {other}"),
                };
                crate::bridge::app::show_message(msg);
            }
        }
    });
}

/// Cancels an in-flight Continuity Camera request from the PC side.
pub fn cancel() {
    let active = {
        let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
        guard.active.take()
    };
    let Some(active) = active else { return };
    notify_continuity();
    if let Some(node) = core_host::node() {
        core_host::spawn(async move {
            let _ =
                node.cancel_camera_capture(active.device, active.request_id, Some("cancelled".into())).await;
        });
    }
}

/// Confirms pasting the clipboard image when the original target window had closed.
pub fn confirm_paste() {
    {
        let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
        guard.prompt = None;
    }
    notify_continuity();

    if let Some((hwnd, title)) = crate::win::tray::snapshot_target_window() {
        spawn_focus_and_paste(hwnd);
        let label = if title.is_empty() { "active window".to_owned() } else { title };
        crate::bridge::app::show_message(format!("Pasted into {label}"));
    } else {
        crate::bridge::app::show_message("Image is on your clipboard — press Ctrl+V in your target app.");
    }
}

/// Dismisses the missing-window paste confirmation prompt, keeping the image on the clipboard.
pub fn dismiss_paste() {
    {
        let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
        guard.prompt = None;
    }
    notify_continuity();
}

fn spawn_focus_and_paste(hwnd: isize) {
    let _ = std::thread::Builder::new().name("continuity-paste".into()).spawn(move || {
        if crate::win::tray::focus_window(hwnd) {
            std::thread::sleep(Duration::from_millis(65));
            if crate::win::tray::is_window_alive_and_visible(hwnd) {
                crate::win::input::press_key("paste", &[]);
                crate::win::tray::send_paste_fallback_if_not_foreground(hwnd);
            }
        }
    });
}

/// Handles Continuity Camera events from `nectarlink_core`.
pub fn on_event(event: &NodeEvent) {
    match event {
        NodeEvent::CameraCaptureReceived { device, request_id, mode, mime, data, .. } => {
            let matched = {
                let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
                if guard.active.as_ref().is_some_and(|a| a.request_id == *request_id || a.device == *device) {
                    guard.active.take()
                } else {
                    None
                }
            };

            let device_name = matched.as_ref().map(|a| a.device_name.clone()).unwrap_or_else(|| {
                core_host::host().hub.read(|s| s.name_of(device).unwrap_or_else(|| device.short()))
            });

            if let Err(e) = crate::win::clipboard::write_image(mime, data) {
                tracing::warn!(error = %e, "can't copy continuity camera image to clipboard");
                notify_continuity();
                crate::bridge::app::show_message(format!("Couldn’t copy image to clipboard: {e}"));
                return;
            }

            let target_ref =
                matched.as_ref().and_then(|a| a.target_hwnd.map(|hwnd| (hwnd, a.target_title.as_str())));
            let alive =
                target_ref.is_some_and(|(hwnd, _)| crate::win::tray::is_window_alive_and_visible(hwnd));

            match decide_delivery_action(*mode, &device_name, target_ref, alive) {
                DeliveryDecision::AutoPaste { hwnd, toast } => {
                    {
                        let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
                        guard.prompt = None;
                    }
                    notify_continuity();
                    crate::bridge::app::show_message(toast);
                    spawn_focus_and_paste(hwnd);
                }
                DeliveryDecision::PromptMissingTarget { title, body, target_name } => {
                    {
                        let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
                        guard.prompt = Some(PendingPastePrompt { title, body, target_name });
                    }
                    notify_continuity();
                    crate::bridge::app::request_activation();
                }
                DeliveryDecision::ClipboardOnly { toast } => {
                    {
                        let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
                        guard.prompt = None;
                    }
                    notify_continuity();
                    crate::bridge::app::show_message(toast);
                }
            }
        }
        NodeEvent::CameraCaptureCancelled { device, request_id, reason } => {
            let removed = {
                let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
                if guard.active.as_ref().is_some_and(|a| {
                    a.request_id == *request_id || (request_id.is_empty() && a.device == *device)
                }) {
                    guard.active.take()
                } else {
                    None
                }
            };
            if let Some(active) = removed {
                notify_continuity();
                let msg = match reason.as_deref() {
                    Some("busy") => format!("Camera is busy on {}.", active.device_name),
                    Some("permission_denied") => {
                        format!("Camera permission was denied on {}.", active.device_name)
                    }
                    _ => format!("Camera capture cancelled on {}.", active.device_name),
                };
                crate::bridge::app::show_message(msg);
            }
        }
        NodeEvent::LinkChanged { device, link } if !matches!(link, LinkState::Online { .. }) => {
            let cleared = {
                let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
                if guard.active.as_ref().is_some_and(|a| a.device == *device) {
                    guard.active = None;
                    true
                } else {
                    false
                }
            };
            if cleared {
                notify_continuity();
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivery_decision_auto_pastes_when_target_window_is_alive() {
        let decision = decide_delivery_action(
            CameraCaptureMode::Photo,
            "Pixel 9",
            Some((0x1234, "Document.docx - Word")),
            true,
        );
        assert_eq!(
            decision,
            DeliveryDecision::AutoPaste {
                hwnd: 0x1234,
                toast: "Photo from Pixel 9 pasted into Document.docx - Word".into(),
            }
        );
    }

    #[test]
    fn delivery_decision_prompts_when_target_window_disappeared() {
        let decision = decide_delivery_action(
            CameraCaptureMode::Scan,
            "Pixel 9",
            Some((0x1234, "Notes - Notion")),
            false,
        );
        match decision {
            DeliveryDecision::PromptMissingTarget { title, body, target_name } => {
                assert_eq!(title, "Scanned document ready on clipboard");
                assert_eq!(target_name, "Notes - Notion");
                assert!(body.contains("Notes - Notion"));
                assert!(body.contains("no longer open"));
            }
            other => panic!("expected PromptMissingTarget, got {other:?}"),
        }
    }

    #[test]
    fn delivery_decision_copies_only_when_no_external_target_was_recorded() {
        let decision = decide_delivery_action(CameraCaptureMode::Photo, "Pixel 9", None, false);
        assert_eq!(
            decision,
            DeliveryDecision::ClipboardOnly { toast: "Photo from Pixel 9 copied to clipboard".into() }
        );
    }
}
