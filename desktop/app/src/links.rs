// SPDX-License-Identifier: GPL-3.0-or-later
//! Links and power between this PC and paired phones
//! (docs/protocol/actions.md): links a phone shares open in the PC's
//! browser; a copied link can be opened on the phone from the tray; a phone
//! can lock this PC or put it to sleep.

use nectarlink_core::{DeviceId, Error, FeatureState, LinkState, PowerAction};

use crate::{
    bridge::app::describe,
    core_host,
    win::{
        clipboard::{self, Clip},
        shell,
        toast::{self, Toast},
    },
};

/// The toast "device" for link notices.
pub const TOAST_GROUP: &str = "links";

/// Opens a link a phone sent, in the default browser.
pub fn open_here(url: &str) -> Result<(), String> {
    shell::open_url(url)
}

/// Locks this PC, or puts it to sleep (a phone asked).
pub fn power(action: PowerAction) -> Result<(), String> {
    tracing::info!(action = action.as_str(), "a phone asked");
    match action {
        PowerAction::Lock => shell::lock(),
        PowerAction::Sleep => shell::sleep(),
    }
}

/// Whether text is a single web link.
fn as_link(text: &str) -> Option<&str> {
    let text = text.trim();
    let lower = text.get(..8).map(str::to_ascii_lowercase).unwrap_or_default();
    let web = lower.starts_with("http://") || lower.starts_with("https://");
    (web && text.len() <= 4096 && !text.chars().any(char::is_whitespace)).then_some(text)
}

fn notice(title: &str, body: &str) {
    toast::show(Toast {
        device: TOAST_GROUP.into(),
        key: "notice".into(),
        title: title.into(),
        body: body.into(),
        attribution: "Nectarlink".into(),
        icon: None,
        image: None,
        actions: Vec::new(),
        reply: None,
        silent: true,
        progress: None,
        call: false,
    });
}

/// "Open copied link on phone" (tray): opens the link on the clipboard on
/// a connected phone that takes links.
pub fn open_copied_link_on_phone() {
    let Clip::Text(text) = clipboard::read() else {
        return notice(
            "Copy a link first",
            "Copy a web link, then choose this again to open it on your phone.",
        );
    };
    let Some(url) = as_link(&text).map(str::to_owned) else {
        return notice("That's not a link", "Copy a web link (https://…), then choose this again.");
    };
    let phone = core_host::host().hub.read(|s| {
        s.devices
            .iter()
            .filter(|d| matches!(d.link, LinkState::Online { .. }))
            .find(|d| {
                s.matrices.get(&d.id).and_then(|m| m.state("device.links_to_phone"))
                    == Some(FeatureState::Available)
            })
            .map(|d| (d.id, d.info.name.clone()))
    });
    let Some((phone, name)) = phone else {
        return notice(
            "No phone to open it on",
            "Your phone isn't connected right now, or its app needs an update.",
        );
    };
    send(phone, name, url);
}

fn send(phone: DeviceId, name: String, url: String) {
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        match node.open_link(phone, url).await {
            Ok(()) => notice(&format!("Sent to {name}"), "Tap the notification on your phone to open it."),
            Err(Error::Unsupported) => notice("Couldn't send it", &format!("Update Nectarlink on {name}.")),
            Err(e) => notice("Couldn't send it", &describe(&e)),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copied_links_are_recognized() {
        assert_eq!(as_link("  https://example.com/a?b \n"), Some("https://example.com/a?b"));
        assert_eq!(as_link("HTTP://X.ORG"), Some("HTTP://X.ORG"));
        assert_eq!(as_link("see https://example.com"), None);
        assert_eq!(as_link("javascript:alert(1)"), None);
        assert_eq!(as_link("https://a.com b"), None);
    }
}
