// SPDX-License-Identifier: GPL-3.0-or-later
//! Calls on a phone (docs/protocol/calls.md): a ringing Windows
//! notification to answer, decline or silence the call (its audio stays on
//! the phone), the call in progress (for Home's call card and a quiet
//! notification to mute or end it), and a notification for calls nobody
//! answered.

use std::{collections::HashMap, path::PathBuf, sync::Mutex};

use nectarlink_core::{CallCommand, CallState, DeviceId, Error, FeatureState, LinkState, NodeEvent};

use crate::{
    bridge::app::{describe, show_message},
    core_host,
    state::Changes,
    win::toast::{self, CALL_ANSWER, CALL_DECLINE, CALL_END, Toast},
};

/// The toast "device" for calls; their key is `<device ID> <call ID>`.
pub const TOAST_GROUP: &str = "calls";
const ACTION_SILENCE: &str = "silence";
const ACTION_MUTE: &str = "mute";
const ACTION_UNMUTE: &str = "unmute";
const ACTION_SPEAKER: &str = "speaker";
const ACTION_EARPIECE: &str = "earpiece";
/// The in-call notification's key prefix (`active <device> <call>`).
const ACTIVE: &str = "active ";

/// The ringing call shown per phone, to show it again (silenced) or take it
/// away when it stops ringing.
static RINGING: Mutex<Option<HashMap<DeviceId, CallState>>> = Mutex::new(None);

fn ringing<T>(f: impl FnOnce(&mut HashMap<DeviceId, CallState>) -> T) -> T {
    f(RINGING.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_default())
}

/// The call in progress per phone (answered, or held).
static IN_PROGRESS: Mutex<Option<HashMap<DeviceId, CallState>>> = Mutex::new(None);

fn in_progress<T>(f: impl FnOnce(&mut HashMap<DeviceId, CallState>) -> T) -> T {
    f(IN_PROGRESS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_default())
}

/// The call per phone whose in-call notification was shown.
static NOTIFIED: Mutex<Option<HashMap<DeviceId, String>>> = Mutex::new(None);

fn notified<T>(f: impl FnOnce(&mut HashMap<DeviceId, String>) -> T) -> T {
    f(NOTIFIED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_default())
}

/// A phone's call in progress, if any.
pub fn active(device: DeviceId) -> Option<CallState> {
    in_progress(|c| c.get(&device).cloned())
}

/// Whether the PC can answer and end a phone's calls.
pub fn can_control(device: DeviceId) -> bool {
    core_host::host().hub.read(|s| {
        s.matrices.get(&device).and_then(|m| m.state("calls.control")) == Some(FeatureState::Available)
    })
}

fn key(device: DeviceId, call: &str) -> String {
    format!("{device} {call}")
}

pub fn on_event(event: &NodeEvent) {
    match event {
        NodeEvent::Call { device, call } => changed(*device, call),
        // A phone that's gone can't be answered (or hung up) from here.
        NodeEvent::LinkChanged { device, link: LinkState::Offline { .. } } => {
            stop_ringing(*device);
            finish(*device);
        }
        // A call can arrive before what the phone can do with it: once the
        // PC may end it, its notification shows.
        NodeEvent::Capabilities(matrix) if matrix.state("calls.control") == Some(FeatureState::Available) => {
            if let Some(call) = active(matrix.device)
                && notified(|n| n.get(&matrix.device) != Some(&call.id))
            {
                show_in_progress(matrix.device, &call);
            }
        }
        _ => {}
    }
}

fn changed(device: DeviceId, call: &CallState) {
    if call.state == "active" {
        match in_progress(|c| c.insert(device, call.clone())) {
            Some(previous) if previous.id == call.id => {}
            previous => {
                if let Some(previous) = previous {
                    toast::remove(TOAST_GROUP, &format!("{ACTIVE}{}", key(device, &previous.id)));
                }
                if can_control(device) {
                    show_in_progress(device, call);
                }
            }
        }
        core_host::host().hub.changed(Changes::CALLS);
    } else if call.state == "ended" || in_progress(|c| c.get(&device).is_some_and(|c| c.id == call.id)) {
        finish(device);
    }
    match call.state.as_str() {
        "ringing" if call.incoming => {
            if let Some(previous) = ringing(|r| r.insert(device, call.clone()))
                && previous.id != call.id
            {
                toast::remove(TOAST_GROUP, &key(device, &previous.id));
            }
            show_ringing(device, call, false);
        }
        "ended" if call.missed => {
            stop_ringing(device);
            show_missed(device, call);
        }
        // Answered (here or on the phone) or over.
        _ => stop_ringing(device),
    }
}

/// The call in progress is over (or out of reach).
fn finish(device: DeviceId) {
    notified(|n| n.remove(&device));
    if let Some(call) = in_progress(|c| c.remove(&device)) {
        toast::remove(TOAST_GROUP, &format!("{ACTIVE}{}", key(device, &call.id)));
        core_host::host().hub.changed(Changes::CALLS);
    }
}

fn stop_ringing(device: DeviceId) {
    if let Some(call) = ringing(|r| r.remove(&device)) {
        toast::remove(TOAST_GROUP, &key(device, &call.id));
    }
}

/// Who's calling, as well as the phone knows.
fn caller(call: &CallState) -> String {
    call.name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .or_else(|| call.number.clone().filter(|n| !n.trim().is_empty()))
        .unwrap_or_else(|| "Unknown caller".into())
}

fn phone_name(device: DeviceId) -> String {
    core_host::host().hub.read(|s| s.name_of(&device)).unwrap_or_else(|| "your phone".into())
}

fn show_ringing(device: DeviceId, call: &CallState, silenced: bool) {
    let mut actions = Vec::new();
    if can_control(device) {
        actions.push((CALL_ANSWER.to_owned(), "Answer".to_owned()));
        actions.push((CALL_DECLINE.to_owned(), "Decline".to_owned()));
        if !silenced {
            actions.push((ACTION_SILENCE.to_owned(), "Silence".to_owned()));
        }
    }
    // The number under the name, when there's a name.
    let body = match (&call.name, &call.number) {
        (Some(name), Some(number)) if !name.trim().is_empty() => {
            format!("{number} · on {}", phone_name(device))
        }
        _ => format!("Calling {}", phone_name(device)),
    };
    toast::show(Toast {
        device: TOAST_GROUP.into(),
        key: key(device, &call.id),
        title: caller(call),
        body,
        attribution: "Nectarlink".into(),
        icon: call.photo.as_deref().and_then(|photo| save_photo(device, &call.id, photo)),
        image: None,
        actions,
        reply: None,
        silent: silenced,
        progress: None,
        call: true,
    });
}

/// A quiet notification for the call in progress: mute, speaker, end
/// (for a phone whose calls the PC controls).
fn show_in_progress(device: DeviceId, call: &CallState) {
    notified(|n| n.insert(device, call.id.clone()));
    let mut actions = Vec::new();
    if let Some(controls) = &call.controls {
        actions.push(if controls.muted {
            (ACTION_UNMUTE.to_owned(), "Unmute".to_owned())
        } else {
            (ACTION_MUTE.to_owned(), "Mute".to_owned())
        });
        actions.push(if controls.speaker {
            (ACTION_EARPIECE.to_owned(), "Earpiece".to_owned())
        } else {
            (ACTION_SPEAKER.to_owned(), "Speaker".to_owned())
        });
    }
    actions.push((CALL_END.to_owned(), "End".to_owned()));
    toast::show(Toast {
        device: TOAST_GROUP.into(),
        key: format!("{ACTIVE}{}", key(device, &call.id)),
        title: format!("On a call with {}", caller(call)),
        body: format!("On {}", phone_name(device)),
        attribution: "Nectarlink".into(),
        icon: None,
        image: None,
        actions,
        reply: None,
        silent: true,
        progress: None,
        call: false,
    });
}

/// Asks a phone to do something with its call; problems are shown.
pub fn command(device: DeviceId, call: String, command: CallCommand) {
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        match node.call_command(device, call, command).await {
            Ok(()) => {}
            Err(Error::NotFound) => show_message("That call is over."),
            Err(e) => show_message(describe(&e)),
        }
    });
}

fn show_missed(device: DeviceId, call: &CallState) {
    toast::show(Toast {
        device: TOAST_GROUP.into(),
        key: format!("missed {}", key(device, &call.id)),
        title: format!("Missed call from {}", caller(call)),
        body: format!("On {}", phone_name(device)),
        attribution: "Nectarlink".into(),
        icon: None,
        image: None,
        actions: Vec::new(),
        reply: None,
        silent: false,
        progress: None,
        call: false,
    });
}

/// The caller's photo as a file, for the notification.
fn save_photo(device: DeviceId, call: &str, photo: &[u8]) -> Option<PathBuf> {
    let dir = crate::notifications::images_dir();
    let path = dir.join(format!("call-{:016x}.jpg", crate::photos::fingerprint(&key(device, call))));
    match std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, photo)) {
        Ok(()) => Some(path),
        Err(e) => {
            tracing::warn!(error = %e, "can't keep a caller's photo");
            None
        }
    }
}

/// The user pressed a button on a call notification.
pub fn on_toast(key: &str, action: &str) {
    if key.starts_with("missed ") {
        return;
    }
    let (key, in_call) = match key.strip_prefix(ACTIVE) {
        Some(key) => (key, true),
        None => (key, false),
    };
    let Some((device, call)) = key.split_once(' ') else { return };
    let Ok(device) = device.parse::<DeviceId>() else { return };
    let command = match action {
        CALL_ANSWER => CallCommand::Answer,
        CALL_DECLINE | CALL_END => CallCommand::Decline,
        ACTION_SILENCE => CallCommand::Silence,
        ACTION_MUTE => CallCommand::Mute(true),
        ACTION_UNMUTE => CallCommand::Mute(false),
        ACTION_SPEAKER => CallCommand::Speaker(true),
        ACTION_EARPIECE => CallCommand::Speaker(false),
        _ => return,
    };
    if in_call {
        // A button took the notification away: it comes back (quietly) once
        // the phone has done it, with the new state, unless the call ended.
        let Some(node) = core_host::node() else { return };
        let call = call.to_owned();
        core_host::spawn(async move {
            match node.call_command(device, call.clone(), command).await {
                Ok(()) if command == CallCommand::Decline => {}
                Ok(()) => {
                    // The phone's new state comes as its own event, right after.
                    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                    if let Some(active) = active(device).filter(|c| c.id == call) {
                        show_in_progress(device, &active);
                    }
                }
                Err(Error::NotFound) => show_message("That call is over."),
                Err(e) => show_message(describe(&e)),
            }
        });
        return;
    }
    let Some(node) = core_host::node() else { return };
    let call = call.to_owned();
    core_host::spawn(async move {
        match node.call_command(device, call.clone(), command).await {
            // Quiet now, but still ringing for the caller: the buttons stay.
            Ok(()) if command == CallCommand::Silence => {
                if let Some(ringing) = ringing(|r| r.get(&device).filter(|c| c.id == call).cloned()) {
                    show_ringing(device, &ringing, true);
                }
            }
            Ok(()) => {}
            Err(Error::NotFound) => show_message("That call is over."),
            Err(e) => show_message(describe(&e)),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: Option<&str>, number: Option<&str>) -> CallState {
        CallState {
            id: "1".into(),
            state: "ringing".into(),
            incoming: true,
            number: number.map(Into::into),
            name: name.map(Into::into),
            photo: None,
            missed: false,
            since: None,
            controls: None,
        }
    }

    #[test]
    fn callers_are_named_as_well_as_known() {
        assert_eq!(caller(&call(Some("Sam"), Some("+1555"))), "Sam");
        assert_eq!(caller(&call(Some(" "), Some("+1555"))), "+1555");
        assert_eq!(caller(&call(None, None)), "Unknown caller");
    }
}
