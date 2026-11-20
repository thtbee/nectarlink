// SPDX-License-Identifier: MPL-2.0
//! What each action does. Every handler gets checked arguments and a context
//! with the device already chosen and allowed; it calls the node or the host
//! and returns plain JSON. Outside text goes through `untrusted`.

use std::path::PathBuf;

use nectarlink_core::{CallCommand, LinkState, MediaAction};
use schemars::JsonSchema;
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};

use crate::{
    def::HandlerFuture,
    error::ActionError,
    gateway::Ctx,
    untrusted::{FIELD_MAX, clean, clean_message, clean_opt},
};

fn parse<T: DeserializeOwned>(args: Value) -> Result<T, ActionError> {
    serde_json::from_value(args).map_err(|e| ActionError::invalid(format!("Invalid arguments: {e}")))
}

fn ok() -> Value {
    json!({ "ok": true })
}

macro_rules! handler {
    ($name:ident, $input:ty, |$ctx:ident, $a:ident| $body:expr) => {
        pub fn $name(ctx: Ctx, args: Value) -> HandlerFuture {
            Box::pin(async move {
                let $ctx = ctx;
                let $a: $input = parse(args)?;
                $body
            })
        }
    };
}

// ---- Inputs ----

/// Only picks the device.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeviceOnly {
    /// Device id, id prefix or name. Optional when only one device is paired.
    pub device: Option<String>,
}

/// No arguments.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoArgs {}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListArgs {
    pub device: Option<String>,
    /// Filter text.
    pub query: Option<String>,
    /// How many to return, 1 to 100.
    #[schemars(range(min = 1, max = 100))]
    pub limit: Option<u32>,
}

fn limit(l: Option<u32>) -> u32 {
    l.unwrap_or(20).clamp(1, 100)
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NotificationKey {
    pub device: Option<String>,
    /// The notification's `key`, from notifications_list.
    pub key: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NotificationActionArgs {
    pub device: Option<String>,
    pub key: String,
    /// The action's `id`, from the notification's `actions`.
    pub action: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NotificationReplyArgs {
    pub device: Option<String>,
    pub key: String,
    /// The reply action's `id`, from the notification's `actions` (one with `takes_text`).
    pub action: String,
    #[schemars(length(min = 1, max = 4000))]
    pub text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PhoneNotifyArgs {
    pub device: Option<String>,
    /// What the notice says.
    #[schemars(length(min = 1, max = 200))]
    pub title: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ThreadArgs {
    pub device: Option<String>,
    /// The thread's `id`, from sms_threads or chat_threads.
    pub thread: String,
    #[schemars(range(min = 1, max = 100))]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SmsSendArgs {
    pub device: Option<String>,
    /// Phone numbers, 1 to 10.
    #[schemars(length(min = 1, max = 10))]
    pub to: Vec<String>,
    #[schemars(length(min = 1, max = 1600))]
    pub body: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChatReplyArgs {
    pub device: Option<String>,
    pub thread: String,
    #[schemars(length(min = 1, max = 4000))]
    pub text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DialArgs {
    pub device: Option<String>,
    pub number: String,
    /// A SIM id from call_sims, when the phone has more than one.
    pub sim: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CallKind {
    Answer,
    Decline,
    Silence,
    Mute,
    Unmute,
    Speaker,
    Earpiece,
    Hold,
    Unhold,
    Dtmf,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CallControlArgs {
    pub device: Option<String>,
    /// The call's id, from call_state.
    pub call: String,
    pub command: CallKind,
    /// For `dtmf`: one of 0-9, * or #.
    pub digit: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QueryArgs {
    pub query: Option<String>,
    #[schemars(range(min = 1, max = 100))]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipboardSetArgs {
    #[schemars(length(min = 1, max = 100000))]
    pub text: String,
    /// Also put it on this device's clipboard.
    pub device: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HistoryPinArgs {
    /// The entry's `id`, from clipboard_history.
    pub id: String,
    pub pinned: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IdArgs {
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FilesSendArgs {
    pub device: Option<String>,
    /// Absolute paths of files or folders on this PC.
    #[schemars(length(min = 1, max = 100))]
    pub paths: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LinkArgs {
    pub device: Option<String>,
    /// An http or https address.
    pub url: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TransferAction {
    Pause,
    Resume,
    Cancel,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TransferControlArgs {
    /// The transfer's id, from transfers_list or files_send.
    pub id: String,
    pub action: TransferAction,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Play,
    Pause,
    Next,
    Previous,
    Seek,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MediaControlArgs {
    pub device: Option<String>,
    /// The player's id, from media_players.
    pub player: String,
    pub action: MediaKind,
    /// For `seek`, in milliseconds.
    pub position_ms: Option<u64>,
}

// ---- Core ----

handler!(devices_list, NoArgs, |ctx, _a| {
    let node = ctx.node()?;
    let devices = node.paired_devices()?;
    Ok(json!({ "devices": devices.iter().map(|d| {
        let (online, link) = match &d.link {
            LinkState::Online { path, .. } => (true, format!("{path:?}").to_lowercase()),
            LinkState::Connecting => (false, "connecting".to_owned()),
            LinkState::Offline { .. } => (false, "offline".to_owned()),
        };
        json!({
            "id": d.id.to_string(),
            "name": clean(&d.info.name, 80),
            "kind": format!("{:?}", d.info.kind).to_lowercase(),
            "os": clean(&d.info.os, 40),
            "online": online,
            "link": link,
        })
    }).collect::<Vec<_>>(), "_untrusted": ["devices[].name"] }))
});

handler!(device_get, DeviceOnly, |ctx, _a| {
    let node = ctx.node()?;
    let id = ctx.device()?;
    let matrix = node.capabilities(id)?;
    let toggles = node.device_toggles(id)?;
    Ok(json!({
        "id": id.to_string(),
        "features": matrix.features.iter().map(|(k, v)| (k.to_string(), json!(feature_state(v)))).collect::<serde_json::Map<_, _>>(),
        "permissions": toggles.iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<serde_json::Map<_, _>>(),
    }))
});

fn feature_state(s: &nectarlink_core::FeatureState) -> &'static str {
    use nectarlink_core::FeatureState as F;
    match s {
        F::Available => "available",
        F::Partial { .. } => "partial",
        F::Locked { .. } => "locked",
        _ => "unsupported",
    }
}

handler!(device_ring, DeviceOnly, |ctx, _a| {
    ctx.node()?.ring(ctx.device()?, true).await?;
    Ok(ok())
});

handler!(device_ring_stop, DeviceOnly, |ctx, _a| {
    ctx.node()?.ring(ctx.device()?, false).await?;
    Ok(ok())
});

// ---- Notifications ----

handler!(notifications_list, ListArgs, |ctx, a| {
    let device = ctx.device()?;
    let query = a.query.map(|q| q.to_lowercase());
    let items: Vec<Value> = ctx
        .host
        .notifications()
        .into_iter()
        .filter(|(d, _)| *d == device)
        .map(|(_, n)| n)
        .filter(|n| {
            query.as_ref().is_none_or(|q| {
                [Some(&n.app_name), n.title.as_ref(), n.text.as_ref()]
                    .into_iter()
                    .flatten()
                    .any(|f| f.to_lowercase().contains(q.as_str()))
            })
        })
        .take(limit(a.limit) as usize)
        .map(|n| notification_json(&n, ctx.show_codes))
        .collect();
    Ok(
        json!({ "notifications": items, "_untrusted": ["notifications[].title", "notifications[].body", "notifications[].app_name", "notifications[].actions[].label"] }),
    )
});

fn notification_json(n: &nectarlink_protocol::messages::Notification, show_codes: bool) -> Value {
    json!({
        "key": n.key,
        "app": n.app,
        "app_name": clean(&n.app_name, 80),
        "title": n.title.as_deref().map(|t| clean_message(t, 300, show_codes)),
        "body": n.text.as_deref().map(|t| clean_message(t, FIELD_MAX, show_codes)),
        "posted": n.when,
        "ongoing": n.ongoing,
        "actions": n.actions.iter().map(|a| json!({ "id": a.id, "label": clean(&a.title, 80), "takes_text": a.reply })).collect::<Vec<_>>(),
    })
}

handler!(notifications_history, ListArgs, |ctx, a| {
    let node = ctx.node()?;
    let items: Vec<Value> = node
        .notification_history(a.query.as_deref(), limit(a.limit) as usize)
        .iter()
        .map(|n| notification_json(n, ctx.show_codes))
        .collect();
    Ok(
        json!({ "notifications": items, "_untrusted": ["notifications[].title", "notifications[].body", "notifications[].app_name"] }),
    )
});

handler!(notification_dismiss, NotificationKey, |ctx, a| {
    ctx.node()?.dismiss_notification(ctx.device()?, a.key).await?;
    Ok(ok())
});

handler!(notification_action, NotificationActionArgs, |ctx, a| {
    ctx.node()?.run_notification_action(ctx.device()?, a.key, a.action, None).await?;
    Ok(ok())
});

handler!(notification_reply, NotificationReplyArgs, |ctx, a| {
    ctx.node()?.run_notification_action(ctx.device()?, a.key, a.action, Some(a.text)).await?;
    Ok(ok())
});

handler!(phone_notify, PhoneNotifyArgs, |ctx, a| {
    let id = format!("agent-{}", data_encoding::HEXLOWER.encode(&rand::random::<[u8; 4]>()));
    let task = nectarlink_protocol::messages::TaskNotify {
        id,
        title: a.title,
        active: false,
        elapsed_ms: 0,
        exit_code: Some(0),
    };
    ctx.node()?.task_notify(Some(ctx.device()?), task).await?;
    Ok(ok())
});

// ---- Messages ----

handler!(sms_threads, ListArgs, |ctx, a| {
    let threads = ctx.node()?.sms_threads(ctx.device()?, limit(a.limit)).await?;
    Ok(json!({ "threads": threads.iter().map(|t| json!({
        "id": t.id,
        "addresses": t.addresses,
        "names": t.names.iter().map(|n| clean(n, 80)).collect::<Vec<_>>(),
        "snippet": clean_message(&t.snippet, 300, ctx.show_codes),
        "date": t.date,
        "unread": t.unread,
    })).collect::<Vec<_>>(), "_untrusted": ["threads[].names", "threads[].snippet"] }))
});

handler!(sms_messages, ThreadArgs, |ctx, a| {
    let msgs = ctx.node()?.sms_messages(ctx.device()?, a.thread, None, limit(a.limit)).await?;
    Ok(json!({ "messages": msgs.iter().map(|m| json!({
        "id": m.id,
        "from": if m.outgoing { "me".to_owned() } else { clean(&m.address, 40) },
        "body": clean_message(&m.body, FIELD_MAX, ctx.show_codes),
        "date": m.date,
        "outgoing": m.outgoing,
        "attachments": m.parts.iter().map(|p| json!({ "id": p.id, "mime": p.mime, "size": p.size })).collect::<Vec<_>>(),
    })).collect::<Vec<_>>(), "_untrusted": ["messages[].body"] }))
});

handler!(sms_send, SmsSendArgs, |ctx, a| {
    if a.to.is_empty() || a.to.len() > 10 {
        return Err(ActionError::invalid("Give 1 to 10 phone numbers."));
    }
    for n in &a.to {
        let digits = n.chars().filter(char::is_ascii_digit).count();
        if !(3..=20).contains(&digits) || !n.chars().all(|c| c.is_ascii_digit() || " +-()".contains(c)) {
            return Err(ActionError::invalid(format!("{n:?} isn't a phone number.")));
        }
    }
    if a.body.trim().is_empty() || a.body.chars().count() > 1600 {
        return Err(ActionError::invalid("The text must be 1 to 1600 characters."));
    }
    let count = a.to.len();
    ctx.node()?.send_sms(ctx.device()?, a.to, a.body).await?;
    Ok(json!({ "sent": true, "recipients": count }))
});

handler!(chat_threads, ListArgs, |ctx, a| {
    let threads = ctx.node()?.chat_threads(ctx.device()?);
    Ok(json!({ "threads": threads.iter().take(limit(a.limit) as usize).map(|t| json!({
        "id": t.thread_id,
        "app_name": clean(&t.app_name, 80),
        "title": clean(&t.title, 200),
        "group": t.is_group,
        "snippet": clean_message(&t.snippet, 300, ctx.show_codes),
        "last_sender": clean_opt(t.last_sender.as_deref(), 80),
        "updated_ms": t.updated_ms,
        "unread": t.unread,
        "can_reply": t.active && t.reply_action_id.is_some(),
    })).collect::<Vec<_>>(), "_untrusted": ["threads[].title", "threads[].snippet", "threads[].last_sender"] }))
});

handler!(chat_messages, ThreadArgs, |ctx, a| {
    let msgs = ctx.node()?.chat_messages(ctx.device()?, &a.thread, limit(a.limit));
    Ok(json!({ "messages": msgs.iter().map(|m| json!({
        "from": if m.self_sent { Some("me".to_owned()) } else { clean_opt(m.sender.as_deref(), 80) },
        "text": clean_message(&m.text, FIELD_MAX, ctx.show_codes),
        "time_ms": m.time_ms,
    })).collect::<Vec<_>>(), "_untrusted": ["messages[].from", "messages[].text"] }))
});

handler!(chat_reply, ChatReplyArgs, |ctx, a| {
    ctx.node()?.reply_chat_thread(ctx.device()?, &a.thread, a.text).await?;
    Ok(ok())
});

// ---- Calls and contacts ----

handler!(call_state, DeviceOnly, |ctx, _a| {
    let device = ctx.device()?;
    let calls: Vec<Value> =
        ctx.host.calls().into_iter().filter(|(d, _)| *d == device).map(|(_, c)| c).collect();
    Ok(json!({ "calls": calls, "_untrusted": ["calls[].name"] }))
});

handler!(call_log, ListArgs, |ctx, a| {
    let entries = ctx.node()?.call_log(ctx.device()?, None, limit(a.limit)).await?;
    Ok(json!({ "calls": entries.iter().map(|e| json!({
        "number": clean(&e.number, 40),
        "name": clean_opt(e.name.as_deref(), 80),
        "direction": e.direction,
        "date": e.date,
        "duration_s": e.duration,
    })).collect::<Vec<_>>(), "_untrusted": ["calls[].name"] }))
});

handler!(call_sims, DeviceOnly, |ctx, _a| {
    let sims = ctx.node()?.call_sims(ctx.device()?).await?;
    Ok(serde_json::to_value(sims).unwrap_or_default())
});

handler!(call_dial, DialArgs, |ctx, a| {
    let digits = a.number.chars().filter(char::is_ascii_digit).count();
    if !(2..=20).contains(&digits) || !a.number.chars().all(|c| c.is_ascii_digit() || " +-()*#".contains(c)) {
        return Err(ActionError::invalid("That isn't a phone number."));
    }
    ctx.node()?.call_dial(ctx.device()?, a.number, a.sim).await?;
    Ok(ok())
});

handler!(call_control, CallControlArgs, |ctx, a| {
    let command = match a.command {
        CallKind::Answer => CallCommand::Answer,
        CallKind::Decline => CallCommand::Decline,
        CallKind::Silence => CallCommand::Silence,
        CallKind::Mute => CallCommand::Mute(true),
        CallKind::Unmute => CallCommand::Mute(false),
        CallKind::Speaker => CallCommand::Speaker(true),
        CallKind::Earpiece => CallCommand::Speaker(false),
        CallKind::Hold => CallCommand::Hold(true),
        CallKind::Unhold => CallCommand::Hold(false),
        CallKind::Dtmf => {
            let digit = a.digit.as_deref().and_then(|d| {
                let mut chars = d.chars();
                chars
                    .next()
                    .filter(|c| chars.next().is_none() && (c.is_ascii_digit() || *c == '*' || *c == '#'))
            });
            CallCommand::Dtmf(
                digit.ok_or_else(|| ActionError::invalid("dtmf needs one digit: 0-9, * or #."))?,
            )
        }
    };
    ctx.node()?.call_command(ctx.device()?, a.call, command).await?;
    Ok(ok())
});

handler!(contacts_search, ListArgs, |ctx, a| {
    let list = ctx.node()?.contacts(ctx.device()?, a.query, 0, limit(a.limit)).await?;
    Ok(json!({ "contacts": list.iter().map(|c| json!({
        "id": c.id,
        "name": clean(&c.name, 120),
        "numbers": c.numbers.iter().map(|n| clean(&n.number, 40)).collect::<Vec<_>>(),
        "starred": c.starred,
    })).collect::<Vec<_>>(), "_untrusted": ["contacts[].name"] }))
});

// ---- Clipboard ----

handler!(clipboard_get, NoArgs, |ctx, _a| {
    let text = ctx.host.pc_clipboard_read().map_err(ActionError::internal)?;
    Ok(json!({ "text": clean_message(&text, 20_000, ctx.show_codes), "_untrusted": ["text"] }))
});

handler!(clipboard_history, QueryArgs, |ctx, a| {
    let entries = ctx.node()?.clipboard_history(a.query.as_deref());
    Ok(json!({ "entries": entries.iter().take(limit(a.limit) as usize).map(|e| json!({
        "id": e.id,
        "text": clean_message(&e.text, FIELD_MAX, ctx.show_codes),
        "pinned": e.pinned,
        "time": e.timestamp,
    })).collect::<Vec<_>>(), "_untrusted": ["entries[].text"] }))
});

handler!(clipboard_set, ClipboardSetArgs, |ctx, a| {
    ctx.host.pc_clipboard_write(&a.text).map_err(ActionError::internal)?;
    if let Some(device) = ctx.device {
        ctx.node()?.send_clipboard(device, a.text).await?;
    }
    Ok(ok())
});

handler!(clipboard_history_pin, HistoryPinArgs, |ctx, a| {
    if !ctx.node()?.pin_clipboard_history(&a.id, a.pinned) {
        return Err(ActionError::new(crate::error::Code::NotFound, "No clipboard entry with that id."));
    }
    Ok(ok())
});

handler!(clipboard_history_delete, IdArgs, |ctx, a| {
    if !ctx.node()?.delete_clipboard_history(&a.id) {
        return Err(ActionError::new(crate::error::Code::NotFound, "No clipboard entry with that id."));
    }
    Ok(ok())
});

// ---- Files ----

handler!(files_send, FilesSendArgs, |ctx, a| {
    let paths = a.paths.iter().map(|p| crate::paths::checked(p)).collect::<Result<Vec<PathBuf>, _>>()?;
    let files = nectarlink_core::outgoing_paths(&paths).map_err(ActionError::from)?;
    if files.is_empty() {
        return Err(ActionError::invalid("Nothing to send there."));
    }
    let count = files.len();
    let id = ctx.node()?.send_files(ctx.device()?, files).await?;
    Ok(json!({ "transfer_id": id, "files": count }))
});

handler!(link_open, LinkArgs, |ctx, a| {
    let url = a.url.trim();
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://"))
        || url.len() > 2048
        || url.contains(char::is_whitespace)
    {
        return Err(ActionError::invalid("Only http and https addresses can be opened."));
    }
    ctx.node()?.open_link(ctx.device()?, url.to_owned()).await?;
    Ok(ok())
});

handler!(transfers_list, NoArgs, |ctx, _a| {
    Ok(json!({ "transfers": ctx.host.transfers(), "_untrusted": ["transfers[].name"] }))
});

handler!(transfer_control, TransferControlArgs, |ctx, a| {
    let node = ctx.node()?;
    match a.action {
        TransferAction::Pause => node.pause_transfer(&a.id),
        TransferAction::Resume => node.resume_transfer(&a.id),
        TransferAction::Cancel => node.cancel_transfer(&a.id),
    }
    Ok(ok())
});

// ---- Media ----

handler!(media_players, DeviceOnly, |ctx, _a| {
    let device = ctx.device;
    let players: Vec<Value> = ctx
        .host
        .media_players()
        .into_iter()
        .filter(|(d, _)| device.is_none() || *d == device)
        // Across devices, only those this agent may use, with media allowed.
        .filter(|(d, _)| d.is_none_or(|d| ctx.may_see(d, crate::def::Scope::Media)))
        .map(|(d, p)| {
            json!({
                "device": d.map(|d| d.to_string()),
                "player": p.id,
                "app": clean(&p.app, 80),
                "title": clean_opt(p.title.as_deref(), 200),
                "artist": clean_opt(p.artist.as_deref(), 200),
                "playing": p.playing,
                "position_ms": p.position,
                "duration_ms": p.duration,
                "actions": p.actions,
            })
        })
        .collect();
    Ok(json!({ "players": players, "_untrusted": ["players[].title", "players[].artist", "players[].app"] }))
});

handler!(media_control, MediaControlArgs, |ctx, a| {
    let action = match a.action {
        MediaKind::Play => MediaAction::Play,
        MediaKind::Pause => MediaAction::Pause,
        MediaKind::Next => MediaAction::Next,
        MediaKind::Previous => MediaAction::Previous,
        MediaKind::Seek => MediaAction::Seek,
    };
    if matches!(action, MediaAction::Seek) && a.position_ms.is_none() {
        return Err(ActionError::invalid("seek needs position_ms."));
    }
    ctx.node()?.media_command(ctx.device()?, a.player, action, a.position_ms).await?;
    Ok(ok())
});

// ---- This PC ----

handler!(pc_lock, NoArgs, |ctx, _a| {
    ctx.host.pc_lock().map_err(ActionError::internal)?;
    Ok(ok())
});
