// SPDX-License-Identifier: GPL-3.0-or-later
//! New builds, passed between paired devices (`event.update`). The PC is
//! plugged in and checks the feeds on a schedule; when it sees a build, it
//! tells its phones, so they needn't check on their own. A phone that heard
//! first tells the PC the same way. Only a hint: whoever hears it checks the
//! channel's feed itself, and installs nothing a feed doesn't list.

use std::{collections::HashMap, sync::Mutex};

use nectarlink_core::NodeEvent;

use crate::{
    core_host,
    version::{self, Channel},
};

/// The newest version seen per channel, so each is passed on once.
static SEEN: Mutex<Option<HashMap<Channel, String>>> = Mutex::new(None);

/// A feed showed `version` on `channel`: tell paired devices, once per version.
pub fn seen(channel: Channel, version: &str) {
    {
        let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
        let seen = seen.get_or_insert_default();
        if seen.get(&channel).is_some_and(|known| !version::is_newer(version, known)) {
            return;
        }
        seen.insert(channel, version.to_owned());
    }
    let Some(node) = core_host::node() else { return };
    let (channel, version) = (channel.as_str().to_owned(), version.to_owned());
    core_host::spawn(async move { node.announce_update(channel, version).await });
}

/// A paired device heard of a build: check now if it's for this install's
/// channel and newer than what runs here (the updater spaces checks out).
pub fn on_event(event: &NodeEvent) {
    let NodeEvent::UpdateHint { device, channel, version } = event else { return };
    let Some(channel) = Channel::parse(channel) else { return };
    if wanted(crate::updater::channel(), channel, version, env!("CARGO_PKG_VERSION")) {
        tracing::info!(from = %device.short(), channel = channel.as_str(), version, "heard of a new build");
        crate::updater::check_soon();
    }
}

/// Whether a hint of `version` on `heard` is worth a check for an install
/// following `follows` that runs `current`: a nightly install takes releases
/// too, a stable one only releases.
fn wanted(follows: Channel, heard: Channel, version: &str, current: &str) -> bool {
    (heard == Channel::Stable || follows == Channel::Nightly) && version::is_newer(version, current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hints_count_for_their_channel_and_newer_builds_only() {
        // A stable install wants new releases, not nightlies.
        assert!(wanted(Channel::Stable, Channel::Stable, "0.1.5", "0.1.4"));
        assert!(!wanted(Channel::Stable, Channel::Nightly, "0.1.5-nightly.3", "0.1.4"));
        // A nightly install wants both.
        assert!(wanted(Channel::Nightly, Channel::Nightly, "0.1.5-nightly.3", "0.1.5-nightly.2"));
        assert!(wanted(Channel::Nightly, Channel::Stable, "0.1.5", "0.1.5-nightly.9"));
        // Nothing older, the same, or not a version.
        assert!(!wanted(Channel::Nightly, Channel::Nightly, "0.1.5-nightly.2", "0.1.5-nightly.2"));
        assert!(!wanted(Channel::Stable, Channel::Stable, "0.1.3", "0.1.4"));
        assert!(!wanted(Channel::Stable, Channel::Stable, "soon", "0.1.4"));
    }
}
