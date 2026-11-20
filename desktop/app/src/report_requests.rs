// SPDX-License-Identifier: GPL-3.0-or-later
//! A paired phone making a problem report asks for this PC's recent log
//! (`diag.ask`, docs/protocol/diag.md). The log goes only when the user
//! clicks Share on the notification; Don't share, dismissing it, or a
//! minute passing sends nothing. A log this PC asked a phone for arrives
//! here too, for the report being made.

use nectarlink_core::{DeviceId, NodeEvent};

use crate::{core_host, i18n::tr, win::toast};

/// The notifications' group.
pub const TOAST_GROUP: &str = "nectarlink-report";
const ACTION_SHARE: &str = "share";
const ACTION_DONT: &str = "dont_share";

pub fn on_event(event: &NodeEvent) {
    match event {
        NodeEvent::DiagLogAsked { device } => ask(*device),
        NodeEvent::DiagLog { device, nonce, lines } => {
            crate::bridge::report::peer_log_arrived(*device, *nonce, lines.clone());
        }
        _ => {}
    }
}

fn ask(device: DeviceId) {
    let name = core_host::host()
        .hub
        .read(|s| s.devices.iter().find(|d| d.id == device).map(|d| d.info.name.clone()))
        .unwrap_or_default();
    let from = if name.is_empty() { tr!("ReportRequests", "Your phone") } else { name };
    toast::show(toast::Toast {
        device: TOAST_GROUP.into(),
        key: device.to_string(),
        title: tr!("ReportRequests", "%1 asks for this PC's log", from),
        body: tr!(
            "ReportRequests",
            "For a problem report you're making. The log has no names, messages, files, numbers or addresses. You see the whole report before it's sent."
        ),
        attribution: "NectarLink".into(),
        icon: None,
        image: None,
        actions: vec![
            (ACTION_SHARE.into(), tr!("ReportRequests", "Share")),
            (ACTION_DONT.into(), tr!("ReportRequests", "Don't share")),
        ],
        reply: None,
        silent: false,
        progress: None,
        call: false,
    });
}

/// The user clicked Share or Don't share, or dismissed the notification.
pub fn on_toast(key: &str, action: Option<&str>) {
    let Ok(device) = key.parse::<DeviceId>() else { return };
    let share = action == Some(ACTION_SHARE);
    toast::remove(TOAST_GROUP, key);
    core_host::spawn(async move {
        if let Some(node) = core_host::node()
            && let Err(e) = node.diag_log_answer(device, share).await
        {
            tracing::info!(error = %e, "couldn't answer the log request");
        }
    });
}
