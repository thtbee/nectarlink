// SPDX-License-Identifier: GPL-3.0-or-later
//! Links and power between this PC and paired phones
//! (docs/protocol/actions.md): links a phone shares (web URLs, YouTube/video
//! links with playback timestamps, and `geo:` / map locations) open in the
//! PC's browser; a copied link, video, or address can be handed off to the
//! phone from the tray or Command Palette; a phone can lock this PC or put it
//! to sleep.

use nectarlink_core::{
    ClipKind, DeviceId, Error, FeatureState, HandoffKind, HandoffLink, LinkState, PowerAction,
    address_to_geo_uri, classify_clip, extract_handoff_link, format_video_timestamp, geo_to_maps_https,
    with_video_timestamp,
};

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

/// Opens a link a phone sent (`http://`, `https://`, or `geo:`), in the default browser.
pub fn open_here(url: &str) -> Result<(), String> {
    if let Some(maps_url) = geo_to_maps_https(url) {
        return shell::open_url(&maps_url);
    }
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

#[cfg(test)]
fn as_link(text: &str) -> Option<&str> {
    let text = text.trim();
    if text.len() > 4096 || text.chars().any(char::is_whitespace) {
        return None;
    }
    let lower = text.get(..8).map(str::to_ascii_lowercase).unwrap_or_default();
    let web = lower.starts_with("http://") || lower.starts_with("https://");
    let geo = text.get(..4).is_some_and(|p| p.eq_ignore_ascii_case("geo:")) && text.len() > 4;
    (web || geo).then_some(text)
}

/// Enriches a Handoff link with the PC's active media playback timestamp if it
/// is a video link without an explicit timestamp.
pub fn enrich_handoff_link(mut link: HandoffLink) -> HandoffLink {
    if link.kind == HandoffKind::VideoLink
        && link.timestamp_secs.is_none()
        && let Some(secs) = crate::media::local_playback_position_secs()
    {
        link.url = with_video_timestamp(&link.url, secs);
        link.timestamp_secs = Some(secs);
        link.label = format!("{} · {}", link.label, format_video_timestamp(secs));
    }
    link
}

/// Extracts a rich [`HandoffLink`] from arbitrary text (supporting `http://`/`https://`
/// links, YouTube/video links with timestamps, `geo:` URIs, map links, and street addresses).
pub fn handoff_from_text(text: &str) -> Option<HandoffLink> {
    if let Some(h) = extract_handoff_link(text) {
        return Some(enrich_handoff_link(h));
    }
    if let Some(s) = classify_clip(text)
        && s.kind == ClipKind::StreetAddress
    {
        let geo = address_to_geo_uri(text.trim());
        return extract_handoff_link(&geo);
    }
    None
}

/// Whether `text` can be handed off as a link, video with timestamp, or map location.
pub fn is_handoff_text(text: &str) -> bool {
    handoff_from_text(text).is_some()
}

/// Extracts a rich [`HandoffLink`] from the current Windows clipboard (supporting
/// `http://`/`https://` links, YouTube/video links with timestamps, `geo:` URIs,
/// map links, and copied street addresses).
pub fn handoff_from_clipboard() -> Option<HandoffLink> {
    let Clip::Text(text) = clipboard::read() else { return None };
    handoff_from_text(&text)
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

/// "Open copied link on phone" (tray / Command Palette): opens the link, video
/// with timestamp, or map location on the clipboard on a connected phone.
pub fn open_copied_link_on_phone() {
    let Clip::Text(_) = clipboard::read() else {
        return notice(
            "Copy a link or address first",
            "Copy a web link, video URL, or street address, then choose this again to open it on your phone.",
        );
    };
    let Some(handoff) = handoff_from_clipboard() else {
        return notice(
            "That's not a link or address",
            "Copy a web link (https://…), geo: URI, or street address, then choose this again.",
        );
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
    send(phone, name, handoff);
}

/// Opens `url` (or a street address / link snippet) on `phone`, showing a Windows toast notification with the outcome.
pub fn send_to(phone: DeviceId, url: String) {
    let name = core_host::host().hub.read(|s| s.name_of(&phone)).unwrap_or_else(|| "your phone".into());
    let handoff = handoff_from_text(&url).unwrap_or(HandoffLink {
        kind: HandoffKind::WebLink,
        label: url.clone(),
        url,
        timestamp_secs: None,
        map_query: None,
    });
    send(phone, name, handoff);
}

fn send(phone: DeviceId, name: String, handoff: HandoffLink) {
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        let body = match handoff.kind {
            HandoffKind::MapLocation => {
                format!("{} — tap the notification on your phone to open in Maps.", handoff.label)
            }
            HandoffKind::VideoLink if handoff.timestamp_secs.is_some() => {
                format!("{} — tap the notification on your phone to continue watching.", handoff.label)
            }
            _ => "Tap the notification on your phone to open it.".into(),
        };
        match node.open_link(phone, handoff.url).await {
            Ok(()) => notice(&format!("Sent to {name}"), &body),
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
        assert_eq!(as_link("geo:37.7749,-122.4194?q=SF"), Some("geo:37.7749,-122.4194?q=SF"));
        assert_eq!(as_link("see https://example.com"), None);
        assert_eq!(as_link("javascript:alert(1)"), None);
        assert_eq!(as_link("https://a.com b"), None);
    }
}
