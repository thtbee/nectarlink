// SPDX-License-Identifier: GPL-3.0-or-later
//! MCP resources: read-only views an agent can attach as context, each one a
//! read action under the hood, so the app checks them like any other call.
//! An agent only sees the ones its permissions allow.

use serde_json::{Value, json};

/// A fixed resource: its URI, a name, and the action that reads it.
#[derive(Debug)]
pub struct Fixed {
    pub uri: &'static str,
    pub name: &'static str,
    pub title: &'static str,
    pub action: &'static str,
}

/// A resource for one device: `{device}` is its id or name.
#[derive(Debug)]
pub struct PerDevice {
    pub template: &'static str,
    pub name: &'static str,
    pub title: &'static str,
    pub action: &'static str,
}

pub const FIXED: [Fixed; 3] = [
    Fixed { uri: "nectarlink://devices", name: "devices", title: "Paired devices", action: "devices.list" },
    Fixed {
        uri: "nectarlink://transfers",
        name: "transfers",
        title: "File transfers",
        action: "transfers.list",
    },
    Fixed { uri: "nectarlink://timeline", name: "timeline", title: "Timeline", action: "timeline.list" },
];

pub const PER_DEVICE: [PerDevice; 5] = [
    PerDevice {
        template: "nectarlink://device/{device}/notifications",
        name: "notifications",
        title: "A device's notifications",
        action: "notifications.list",
    },
    PerDevice {
        template: "nectarlink://device/{device}/sms",
        name: "sms",
        title: "A phone's text conversations",
        action: "sms.threads",
    },
    PerDevice {
        template: "nectarlink://device/{device}/calls",
        name: "calls",
        title: "A phone's recent calls",
        action: "call.log",
    },
    PerDevice {
        template: "nectarlink://device/{device}/photos",
        name: "photos",
        title: "A phone's latest photos",
        action: "photos.list",
    },
    PerDevice {
        template: "nectarlink://device/{device}/settings",
        name: "settings",
        title: "A phone's quick settings",
        action: "phone.settings",
    },
];

/// The action and arguments that read `uri`, if it's one of ours.
pub fn resolve(uri: &str) -> Option<(&'static str, Value)> {
    if let Some(f) = FIXED.iter().find(|f| f.uri == uri) {
        return Some((f.action, json!({})));
    }
    let rest = uri.strip_prefix("nectarlink://device/")?;
    let (device, what) = rest.split_once('/')?;
    let device = percent_decode(device)?;
    if device.is_empty() || device.len() > 200 {
        return None;
    }
    PER_DEVICE
        .iter()
        .find(|p| p.template.ends_with(&format!("/{what}")))
        .map(|p| (p.action, json!({ "device": device })))
}

/// `%20` and the like back to text; `None` when it isn't valid.
fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_map_to_read_actions() {
        assert_eq!(resolve("nectarlink://devices"), Some(("devices.list", json!({}))));
        assert_eq!(
            resolve("nectarlink://device/My%20Pixel/sms"),
            Some(("sms.threads", json!({ "device": "My Pixel" })))
        );
        assert_eq!(resolve("nectarlink://device/ab12cd/photos").unwrap().0, "photos.list");
        assert_eq!(resolve("nectarlink://device//sms"), None);
        assert_eq!(resolve("nectarlink://device/x/passwords"), None);
        assert_eq!(resolve("nectarlink://device/%zz/sms"), None);
        assert_eq!(resolve("file:///etc/passwd"), None);
    }
}
