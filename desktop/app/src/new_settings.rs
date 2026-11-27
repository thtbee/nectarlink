// SPDX-License-Identifier: GPL-3.0-or-later
//! New settings (#656): the options a recent release added, each with a
//! **New** pill on its row and on its section in the Settings sidebar until
//! the user has looked at that section.
//!
//! A key goes in [`NEW`] with the release that adds the setting, and comes
//! out a release or two later. A new install has seen them all; an update
//! shows the ones not looked at yet.

use serde::Serialize;

/// The settings that are new, as (key, section ID in the Settings page).
/// A key with the section's own ID marks a whole new section.
pub const NEW: &[(&str, &str)] = &[
    ("toolbar-beside", "general"),
    ("mark-read-on-phone", "general"),
    ("beside-this-pc", "devices"),
    ("unlock-pc", "devices"),
    ("frame-rate", "camera_audio"),
    ("keep-popups-paused", "notifications"),
    ("daily-count", "storage"),
    ("notification-history", "storage"),
    ("bluetooth-link", "connection"),
    ("usb-debugging", "connection"),
    ("api-notifications", "connection"),
    ("updates", "updates"),
];

/// Kept in the seen list once the first start has decided what's new, so a
/// later start with nothing seen yet isn't taken for a new install.
pub const DECIDED: &str = "*";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Unseen {
    pub key: &'static str,
    pub section: &'static str,
}

/// The new settings not looked at yet.
pub fn unseen(seen: &[String]) -> Vec<Unseen> {
    NEW.iter()
        .filter(|(key, _)| !seen.iter().any(|s| s == key))
        .map(|&(key, section)| Unseen { key, section })
        .collect()
}

/// [`unseen`] as JSON for the Settings page: `[{ key, section }]`.
pub fn json(seen: &[String]) -> String {
    serde_json::to_string(&unseen(seen)).unwrap_or_else(|_| "[]".into())
}

/// `seen` with every new setting in `section` added (the user looked at it).
/// Keys no longer in [`NEW`] are dropped, so the list doesn't grow forever.
pub fn after_visiting(seen: &[String], section: &str) -> Vec<String> {
    std::iter::once(DECIDED.to_owned())
        .chain(
            NEW.iter()
                .filter(|(key, in_section)| *in_section == section || seen.iter().any(|s| s == key))
                .map(|(key, _)| (*key).to_owned()),
        )
        .collect()
}

/// The seen list at a start where nothing was decided yet: everything on a
/// new install, nothing after an update (its new settings get their pill).
pub fn first_start(seen: &[String], updated: bool) -> Option<Vec<String>> {
    if !seen.is_empty() {
        return None;
    }
    let mut decided = vec![DECIDED.to_owned()];
    if !updated {
        decided.extend(NEW.iter().map(|(key, _)| (*key).to_owned()));
    }
    Some(decided)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_unique_and_name_real_sections() {
        const SECTIONS: &[&str] = &[
            "appearance",
            "general",
            "devices",
            "install_phone",
            "camera_audio",
            "notifications",
            "files_sync",
            "shortcuts",
            "agents",
            "storage",
            "connection",
            "updates",
            "support",
            "report",
        ];
        let mut keys: Vec<_> = NEW.iter().map(|(k, _)| *k).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), NEW.len(), "a key is listed twice");
        for (key, section) in NEW {
            assert!(SECTIONS.contains(section), "{key} is in an unknown section {section}");
        }
    }

    #[test]
    fn a_section_looked_at_stops_showing_its_new_settings() {
        assert_eq!(unseen(&[]).len(), NEW.len());
        let seen = after_visiting(&[], "devices");
        let left = unseen(&seen);
        assert!(left.iter().all(|u| u.section != "devices"));
        assert!(left.iter().any(|u| u.section == "general"));
        // Looking at another section keeps the first one seen.
        let seen = after_visiting(&seen, "general");
        assert!(unseen(&seen).iter().all(|u| u.section != "devices" && u.section != "general"));
    }

    #[test]
    fn keys_that_left_the_list_are_forgotten() {
        let seen = after_visiting(&["gone-setting".to_owned(), "frame-rate".to_owned()], "devices");
        assert!(!seen.contains(&"gone-setting".to_owned()));
        assert!(seen.contains(&"frame-rate".to_owned()));
        assert!(seen.contains(&DECIDED.to_owned()), "never empty again");
    }

    #[test]
    fn a_new_install_has_seen_them_all_and_an_update_none() {
        let fresh = first_start(&[], false).unwrap();
        assert!(unseen(&fresh).is_empty());
        assert_eq!(json(&fresh), "[]");
        let updated = first_start(&[], true).unwrap();
        assert_eq!(unseen(&updated).len(), NEW.len());
        // A later start, updated or not, leaves it alone.
        assert_eq!(first_start(&updated, false), None);
        assert_eq!(first_start(&updated, true), None);
    }
}
