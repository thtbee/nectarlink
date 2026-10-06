// SPDX-License-Identifier: GPL-3.0-or-later
//! Calls on a phone (docs/protocol/calls.md): a ringing Windows
//! notification to answer, decline or silence the call (its audio stays on
//! the phone), and a notification for calls nobody answered.

use std::{collections::HashMap, path::PathBuf, sync::Mutex};

use nectarlink_core::{CallCommand, CallState, DeviceId, Error, FeatureState, LinkState, NodeEvent};

use crate::{
    bridge::app::{describe, show_message},
    core_host,
    win::toast::{self, CALL_ANSWER, CALL_DECLINE, Toast},
};

/// The toast "device" for calls; their key is `<device ID> <call ID>`.
pub const TOAST_GROUP: &str = "calls";
const ACTION_SILENCE: &str = "silence";

/// The ringing call shown per phone, to show it again (silenced) or take it
/// away when it stops ringing.
static RINGING: Mutex<Option<HashMap<DeviceId, CallState>>> = Mutex::new(None);

fn ringing<T>(f: impl FnOnce(&mut HashMap<DeviceId, CallState>) -> T) -> T {
    f(RINGING.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_default())
}

fn key(device: DeviceId, call: &str) -> String {
    format!("{device} {call}")
}

pub fn on_event(event: &NodeEvent) {
    match event {
        NodeEvent::Call { device, call } => changed(*device, call),
        // A phone that's gone can't be answered from here.
        NodeEvent::LinkChanged { device, link: LinkState::Offline { .. } } => stop_ringing(*device),
        _ => {}
    }
}

fn changed(device: DeviceId, call: &CallState) {
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
    let controls = core_host::host().hub.read(|s| {
        s.matrices.get(&device).and_then(|m| m.state("calls.control")) == Some(FeatureState::Available)
    });
    let mut actions = Vec::new();
    if controls {
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
    let Some((device, call)) = key.split_once(' ') else { return };
    let Ok(device) = device.parse::<DeviceId>() else { return };
    let command = match action {
        CALL_ANSWER => CallCommand::Answer,
        CALL_DECLINE => CallCommand::Decline,
        ACTION_SILENCE => CallCommand::Silence,
        _ => return,
    };
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
        }
    }

    #[test]
    fn callers_are_named_as_well_as_known() {
        assert_eq!(caller(&call(Some("Sam"), Some("+1555"))), "Sam");
        assert_eq!(caller(&call(Some(" "), Some("+1555"))), "+1555");
        assert_eq!(caller(&call(None, None)), "Unknown caller");
    }
}
