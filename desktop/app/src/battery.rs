// SPDX-License-Identifier: GPL-3.0-or-later
//! Battery alerts: a Windows notification when a phone's battery runs low,
//! and when it's fully charged. Once each, until the battery has moved on.

use crate::i18n::tr;
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
/// A low alert comes again only after charging or climbing this far past
/// the low level.
const RECOVERY: u8 = 10;

static ENABLED: AtomicBool = AtomicBool::new(true);
static WATCHES: Mutex<Option<HashMap<DeviceId, Watch>>> = Mutex::new(None);

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Alert {
    Low(u8),
    Full(u8),
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
    fn update(&mut self, battery: &Battery, low_at: u8, full_target: u8) -> Option<Alert> {
        let power = battery.charging || battery.plugged.is_some();
        let low = !power && battery.level <= low_at;
        let full = power && battery.level >= full_target;
        if power || battery.level >= low_at.saturating_add(RECOVERY) {
            self.told_low = false;
        }
        if !power || battery.level < full_target {
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
            (!first).then_some(Alert::Full(battery.level))
        } else {
            None
        }
    }
}

pub fn on_event(event: &NodeEvent) {
    let NodeEvent::Battery { device, battery } = event else { return };
    let full_target = crate::settings::battery_full_threshold();
    let low_at = crate::settings::battery_low_threshold();
    let alert = {
        let mut watches = WATCHES.lock().unwrap_or_else(|e| e.into_inner());
        watches.get_or_insert_default().entry(*device).or_default().update(battery, low_at, full_target)
    };
    let Some(alert) = alert else { return };
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let name =
        core_host::host().hub.read(|s| s.name_of(device)).unwrap_or_else(|| tr!("Battery", "Your phone"));
    let (title, body) = match alert {
        Alert::Low(level) => {
            (tr!("Battery", "%1's battery is low", name), tr!("Battery", "%1% left. Charge it soon.", level))
        }
        Alert::Full(level) => {
            if level >= 100 {
                (tr!("Battery", "%1 is fully charged", name), tr!("Battery", "You can unplug it."))
            } else {
                (
                    tr!("Battery", "%1 has reached %2% charge", name, level),
                    tr!("Battery", "You can unplug it."),
                )
            }
        }
    };
    toast::show(Toast {
        device: TOAST_GROUP.into(),
        key: device.to_string(),
        title,
        body,
        attribution: "NectarLink".into(),
        icon: None,
        image: None,
        actions: Vec::new(),
        reply: None,
        silent: false,
        progress: None,
        call: false,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(level: u8, charging: bool) -> Battery {
        Battery {
            level,
            charging,
            plugged: None,
            full_in: None,
            cell_bars: None,
            cell_type: None,
            wifi_bars: None,
        }
    }

    #[test]
    fn low_is_told_once_until_it_recovers() {
        let mut w = Watch::default();
        assert_eq!(w.update(&at(40, false), 15, 100), None);
        assert_eq!(w.update(&at(15, false), 15, 100), Some(Alert::Low(15)));
        assert_eq!(w.update(&at(12, false), 15, 100), None);
        // Plugged in, then out again while still low: worth saying again.
        assert_eq!(w.update(&at(13, true), 15, 100), None);
        assert_eq!(w.update(&at(13, false), 15, 100), Some(Alert::Low(13)));
        // Bouncing around the line doesn't repeat it.
        assert_eq!(w.update(&at(16, false), 15, 100), None);
        assert_eq!(w.update(&at(15, false), 15, 100), None);
        assert_eq!(w.update(&at(30, false), 15, 100), None);
        assert_eq!(w.update(&at(14, false), 15, 100), Some(Alert::Low(14)));
    }

    #[test]
    fn low_follows_the_chosen_level() {
        let mut w = Watch::default();
        assert_eq!(w.update(&at(12, false), 5, 100), None);
        assert_eq!(w.update(&at(6, false), 5, 100), None);
        assert_eq!(w.update(&at(5, false), 5, 100), Some(Alert::Low(5)));
        // Recovered only 10 points above the chosen level.
        assert_eq!(w.update(&at(14, false), 5, 100), None);
        assert_eq!(w.update(&at(5, false), 5, 100), None);
        assert_eq!(w.update(&at(15, false), 5, 100), None);
        assert_eq!(w.update(&at(4, false), 5, 100), Some(Alert::Low(4)));
    }

    #[test]
    fn low_is_told_right_after_connecting() {
        assert_eq!(Watch::default().update(&at(9, false), 15, 100), Some(Alert::Low(9)));
    }

    #[test]
    fn full_is_told_when_it_gets_there() {
        let mut w = Watch::default();
        assert_eq!(w.update(&at(99, true), 15, 100), None);
        assert_eq!(w.update(&at(100, true), 15, 100), Some(Alert::Full(100)));
        assert_eq!(w.update(&at(100, true), 15, 100), None);
        assert_eq!(w.update(&at(100, false), 15, 100), None);
        assert_eq!(w.update(&at(100, true), 15, 100), Some(Alert::Full(100)));
        // Already full when it connected: not news.
        assert_eq!(Watch::default().update(&at(100, true), 15, 100), None);
    }

    #[test]
    fn full_respects_protect_battery_threshold() {
        let mut w = Watch::default();
        assert_eq!(w.update(&at(79, true), 15, 80), None);
        assert_eq!(w.update(&at(80, true), 15, 80), Some(Alert::Full(80)));
        assert_eq!(w.update(&at(80, true), 15, 80), None);
        assert_eq!(w.update(&at(80, false), 15, 80), None);
        assert_eq!(w.update(&at(80, true), 15, 80), Some(Alert::Full(80)));
    }

    #[test]
    fn full_alerts_when_holding_at_protect_threshold_while_plugged() {
        let mut w = Watch::default();
        let mut b = at(80, false);
        b.plugged = Some("ac".into());
        // First report after connect doesn't alert if already 80
        assert_eq!(w.update(&b, 15, 80), None);
        // Level dips to 79 and back up to 80 while still plugged
        b.level = 79;
        assert_eq!(w.update(&b, 15, 80), None);
        b.level = 80;
        assert_eq!(w.update(&b, 15, 80), Some(Alert::Full(80)));
    }
}
