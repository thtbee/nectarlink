// SPDX-License-Identifier: MPL-2.0
//! Agent access to NectarLink: one list of actions, and one gateway every
//! agent request passes through, whichever way it arrives (MCP, the CLI or
//! the local HTTP API).
//!
//! The gateway checks, in order: agent access is on, the token, the action,
//! the client's permissions, read-only, the device and its own permission,
//! the rate limit and the user's confirmation; then it runs the action and
//! logs it. See `docs/agents/README.md`.

pub mod def;
pub mod error;
pub mod events;
pub mod gateway;
pub mod handlers;
pub mod host;
pub mod more;
pub mod paths;
pub mod ratelimit;
pub mod registry;
pub mod setup;
pub mod store;
pub mod untrusted;

pub use def::{Access, ActionDef, ConfirmRule, Grant, Risk, Scope, Toolset};
pub use error::{ActionError, Code};
pub use gateway::{Caller, Ctx, Gateway, Switches};
pub use host::{ActionHost, ConfirmRequest, Decision};
pub use store::{Activity, Client, Store};

/// Grants for the "Read only" preset: every read, except the clipboard and
/// one-time codes, which only a custom choice gives.
pub fn preset_read_only() -> std::collections::BTreeSet<Grant> {
    Scope::ALL
        .into_iter()
        .filter(|s| !matches!(s, Scope::Clipboard | Scope::NotificationsSensitive))
        .map(|s| Grant::new(s, Access::Read))
        .collect()
}

/// Grants for the "Everyday" preset: reads, plus writing notifications,
/// messages, files, media, devices, photos, app windows and the phone's
/// quick settings. Still no clipboard or codes, and nothing that changes
/// the phone's files, runs Deck tiles or powers a PC off.
pub fn preset_everyday() -> std::collections::BTreeSet<Grant> {
    let mut grants = preset_read_only();
    for s in EVERYDAY_WRITES {
        grants.insert(Grant::new(s, Access::Write));
    }
    grants
}

const EVERYDAY_WRITES: [Scope; 8] = [
    Scope::Devices,
    Scope::Notifications,
    Scope::Messages,
    Scope::Files,
    Scope::Media,
    Scope::Photos,
    Scope::Mirroring,
    Scope::PhoneSettings,
];

/// The scopes the first release had. A client saved with one of its presets
/// gets the new scopes of that preset (see [`upgrade_preset`]).
const FIRST_SCOPES: [Scope; 10] = [
    Scope::Devices,
    Scope::Notifications,
    Scope::NotificationsSensitive,
    Scope::Messages,
    Scope::Calls,
    Scope::Contacts,
    Scope::Clipboard,
    Scope::Files,
    Scope::Media,
    Scope::PcActions,
];

/// A client's grants after an update that added scopes: a client that held
/// exactly an old preset gets today's preset, so "Read only" keeps meaning
/// every read. Anything the user changed by hand is left alone (`None`).
pub fn upgrade_preset(
    grants: &std::collections::BTreeSet<Grant>,
) -> Option<std::collections::BTreeSet<Grant>> {
    let first = |set: std::collections::BTreeSet<Grant>| -> std::collections::BTreeSet<Grant> {
        set.into_iter().filter(|g| FIRST_SCOPES.contains(&g.scope)).collect()
    };
    let old_everyday = {
        let mut set = first(preset_read_only());
        for s in [Scope::Devices, Scope::Notifications, Scope::Messages, Scope::Files, Scope::Media] {
            set.insert(Grant::new(s, Access::Write));
        }
        set
    };
    if *grants == first(preset_read_only()) && *grants != preset_read_only() {
        Some(preset_read_only())
    } else if *grants == old_everyday && *grants != preset_everyday() {
        Some(preset_everyday())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_presets_grow_with_new_scopes_but_hand_picked_grants_stay() {
        let old_read: std::collections::BTreeSet<Grant> =
            preset_read_only().into_iter().filter(|g| FIRST_SCOPES.contains(&g.scope)).collect();
        assert_eq!(upgrade_preset(&old_read), Some(preset_read_only()));
        let mut old_everyday = old_read.clone();
        for s in [Scope::Devices, Scope::Notifications, Scope::Messages, Scope::Files, Scope::Media] {
            old_everyday.insert(Grant::new(s, Access::Write));
        }
        assert_eq!(upgrade_preset(&old_everyday), Some(preset_everyday()));
        assert_eq!(upgrade_preset(&preset_read_only()), None);
        assert_eq!(upgrade_preset(&preset_everyday()), None);
        let mut custom = old_read;
        custom.remove(&Grant::new(Scope::Calls, Access::Read));
        assert_eq!(upgrade_preset(&custom), None);
    }

    #[test]
    fn presets_never_include_the_clipboard_codes_or_pc_power() {
        for preset in [preset_read_only(), preset_everyday()] {
            assert!(
                preset
                    .iter()
                    .all(|g| g.scope != Scope::Clipboard && g.scope != Scope::NotificationsSensitive)
            );
            assert!(!preset.contains(&Grant::new(Scope::PcActions, Access::Write)));
            assert!(!preset.contains(&Grant::new(Scope::Storage, Access::Write)));
            assert!(!preset.contains(&Grant::new(Scope::Deck, Access::Write)));
        }
    }
}
