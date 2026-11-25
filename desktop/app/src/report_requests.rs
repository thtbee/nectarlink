// SPDX-License-Identifier: GPL-3.0-or-later
//! A paired phone making a problem report asks for this PC's recent log
//! (`diag.ask`, docs/protocol/diag.md). The log goes only when the user
//! clicks Share on the notification; Don't share, dismissing it, or a
//! minute passing sends nothing. A log this PC asked a phone for arrives
//! here too, for the report being made.

use nectarlink_core::{DeviceId, NodeEvent, diag::send};

use crate::{core_host, i18n::tr, win::toast};

/// The notifications' group.
pub const TOAST_GROUP: &str = "nectarlink-report";
const ACTION_SHARE: &str = "share";
const ACTION_DONT: &str = "dont_share";
const ACTION_REPLY: &str = "reply";
/// A notification about a sent report is keyed by its rid.
const REPORT_KEY: &str = "report:";

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

/// Someone replied to a report this PC sent, or closed it: what they
/// wrote, when it could be read, and a field to answer in.
pub fn notice(notice: send::Notice, rid: &str, latest: Option<send::ThreadEntry>, can_reply: bool) {
    let (issue, title) = match notice {
        send::Notice::Replied { issue } => (issue, tr!("ReportRequests", "There's a reply to your report")),
        send::Notice::Closed { issue } => (issue, tr!("ReportRequests", "Your report was closed")),
    };
    let body = match latest {
        Some(entry) => format!("{}: {}", who(&entry.who), excerpt(&entry.text, 200)),
        None => tr!("ReportRequests", "Report #%1. Open it to read it.", issue),
    };
    toast::show(toast::Toast {
        device: TOAST_GROUP.into(),
        key: format!("{REPORT_KEY}{rid}"),
        title,
        body,
        attribution: "NectarLink".into(),
        icon: None,
        image: None,
        actions: Vec::new(),
        reply: can_reply.then(|| (ACTION_REPLY.into(), tr!("ReportRequests", "Reply"))),
        silent: false,
        progress: None,
        call: false,
    });
}

/// Who wrote an answer, as a notification says it.
pub fn who(who: &send::Who) -> String {
    match who {
        send::Who::Team => "NectarLink".into(),
        send::Who::You => tr!("ReportRequests", "You"),
        send::Who::OtherReporter => tr!("ReportRequests", "Someone with the same problem"),
        send::Who::Someone(login) => login.clone(),
    }
}

/// The start of `text` on one line, `max` characters at most.
fn excerpt(text: &str, max: usize) -> String {
    let line: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= max {
        return line;
    }
    let cut: String = line.chars().take(max).collect();
    format!("{}…", cut.trim_end())
}

/// A notice about a sent report was clicked: Your reports opens at it. One
/// from before (keyed by issue) opens the issue.
pub fn on_opened(key: &str) {
    if let Some(rid) = key.strip_prefix(REPORT_KEY) {
        crate::bridge::report::show_conversations(rid);
    } else if let Some(issue) = key.strip_prefix("issue:").and_then(|n| n.parse::<u64>().ok())
        && let Err(e) = crate::win::shell::open_url(&send::issue_url(issue))
    {
        tracing::warn!(error = %e, "can't open the report");
    }
}

/// The person answered in the notification itself.
pub fn on_reply(key: &str, text: &str) {
    let Some(rid) = key.strip_prefix(REPORT_KEY).map(str::to_owned) else { return };
    let text = text.to_owned();
    toast::remove(TOAST_GROUP, key);
    let _ = std::thread::Builder::new().name("report-reply".into()).spawn(move || {
        let state = crate::bridge::report::reply(&rid, &text);
        if state != "sent" {
            // Not lost: the same text, to send again from Your reports.
            toast::show(toast::Toast {
                device: TOAST_GROUP.into(),
                key: format!("{REPORT_KEY}{rid}"),
                title: tr!("ReportRequests", "Your reply wasn't sent"),
                body: match state {
                    "limited" => {
                        tr!("ReportRequests", "That's the most replies for today. Try again tomorrow.")
                    }
                    "refused" => tr!("ReportRequests", "This report can't take replies any more."),
                    _ => tr!("ReportRequests", "Check the connection, then send it again from Your reports."),
                },
                attribution: "NectarLink".into(),
                icon: None,
                image: None,
                actions: Vec::new(),
                reply: None,
                silent: true,
                progress: None,
                call: false,
            });
        }
    });
}

/// The user clicked Share or Don't share, or dismissed the notification.
pub fn on_toast(key: &str, action: Option<&str>) {
    if key.starts_with("issue:") || key.starts_with(REPORT_KEY) {
        return;
    }
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
