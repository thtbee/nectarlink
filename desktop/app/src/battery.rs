// SPDX-License-Identifier: GPL-3.0-or-later
//! Battery alerts: a Windows notification when a phone's battery runs low,
//! and when it's fully charged. Once each, until the battery has moved on.

use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use nectarlink_core::{Battery, DeviceId, NodeEvent};

use crate::{
    core_host,
    win::toast::{self, Toast},
};

/// The toast "device" for battery alerts; their key is the phone's ID.
pub const TOAST_GROUP: &str = "battery";
/// At or under this (and not charging): low.
const LOW: u8 = 15;
/// A low alert comes again only after charging or climbing past this.
const RECOVERED: u8 = 25;

static ENABLED: AtomicBool = AtomicBool::new(true);
static WATCHES: Mutex<Option<HashMap<DeviceId, Watch>>> = Mutex::new(None);

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Alert {
    Low(u8),
    Full,
}

/// What a phone has been told about already.
#[derive(Debug, Default)]
struct Watch {
    told_low: bool,
    told_full: bool,
    /// The first report after connecting only sets the scene: a phone that
    /// was already low or full when it connected isn't news.
    seen: bool,
}

impl Watch {
    fn update(&mut self, battery: &Battery) -> Option<Alert> {
        let low = !battery.charging && battery.level <= LOW;
        let full = battery.charging && battery.level >= 100;
        if battery.charging || battery.level >= RECOVERED {
            self.told_low = false;
        }
        if !battery.charging || battery.level < 100 {
            self.told_full = false;
        }
        let first = !self.seen;
        self.seen = true;
        if low && !self.told_low {
            self.told_low = true;
            // Low is worth saying even right after connecting.
            Some(Alert::Low(battery.level))
        } else if full && !self.told_full {
            self.told_full = true;
            (!first).then_some(Alert::Full)
        } else {
            None
        }
    }
}

pub fn on_event(event: &NodeEvent) {
    let NodeEvent::Battery { device, battery } = event else { return };
    let alert = {
        let mut watches = WATCHES.lock().unwrap_or_else(|e| e.into_inner());
        watches.get_or_insert_default().entry(*device).or_default().update(battery)
    };
    let Some(alert) = alert else { return };
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let name = core_host::host().hub.read(|s| s.name_of(device)).unwrap_or_else(|| "Your phone".into());
    let (title, body) = match alert {
        Alert::Low(level) => (format!("{name}'s battery is low"), format!("{level}% left. Charge it soon.")),
        Alert::Full => (format!("{name} is fully charged"), "You can unplug it.".to_owned()),
    };
    toast::show(Toast {
        device: TOAST_GROUP.into(),
        key: device.to_string(),
        title,
        body,
        attribution: "Nectarlink".into(),
        icon: None,
        image: None,
        actions: Vec::new(),
        reply: None,
        silent: false,
        progress: None,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(level: u8, charging: bool) -> Battery {
        Battery { level, charging, plugged: None }
    }

    #[test]
    fn low_is_told_once_until_it_recovers() {
        let mut w = Watch::default();
        assert_eq!(w.update(&at(40, false)), None);
        assert_eq!(w.update(&at(15, false)), Some(Alert::Low(15)));
        assert_eq!(w.update(&at(12, false)), None);
        // Plugged in, then out again while still low: worth saying again.
        assert_eq!(w.update(&at(13, true)), None);
        assert_eq!(w.update(&at(13, false)), Some(Alert::Low(13)));
        // Bouncing around the line doesn't repeat it.
        assert_eq!(w.update(&at(16, false)), None);
        assert_eq!(w.update(&at(15, false)), None);
        assert_eq!(w.update(&at(30, false)), None);
        assert_eq!(w.update(&at(14, false)), Some(Alert::Low(14)));
    }

    #[test]
    fn low_is_told_right_after_connecting() {
        assert_eq!(Watch::default().update(&at(9, false)), Some(Alert::Low(9)));
    }

    #[test]
    fn full_is_told_when_it_gets_there() {
        let mut w = Watch::default();
        assert_eq!(w.update(&at(99, true)), None);
        assert_eq!(w.update(&at(100, true)), Some(Alert::Full));
        assert_eq!(w.update(&at(100, true)), None);
        assert_eq!(w.update(&at(100, false)), None);
        assert_eq!(w.update(&at(100, true)), Some(Alert::Full));
        // Already full when it connected: not news.
        assert_eq!(Watch::default().update(&at(100, true)), None);
    }
}
