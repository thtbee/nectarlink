// SPDX-License-Identifier: GPL-3.0-or-later
//! Automations ("When X, Do Y", §3.11 P2): lightweight event-driven rules
//! persisted in `settings.json` (`automation_rules`).

use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

use nectarlink_core::{DeviceId, LinkState, NodeEvent};
use serde::{Deserialize, Serialize};

use crate::{deck, settings, win};

/// Supported automation trigger identifiers.
pub const TRIGGER_PHONE_CONNECTED: &str = "phone_connected";
pub const TRIGGER_PHONE_DISCONNECTED: &str = "phone_disconnected";
#[allow(dead_code)]
pub const TRIGGER_PHONE_WALKED_AWAY: &str = "phone_walked_away";
pub const TRIGGER_CALL_RINGING: &str = "call_ringing";
pub const TRIGGER_CALL_ENDED: &str = "call_ended";
pub const TRIGGER_BATTERY_LOW: &str = "battery_low";
pub const TRIGGER_CHARGING_STARTED: &str = "charging_started";

/// Supported automation action kind identifiers.
pub const ACTION_MUTE_PC: &str = "mute_pc";
pub const ACTION_UNMUTE_PC: &str = "unmute_pc";
pub const ACTION_PAUSE_PC_MEDIA: &str = "pause_pc_media";
pub const ACTION_LOCK_PC: &str = "lock_pc";
pub const ACTION_RUN_COMMAND: &str = "run_command";
pub const ACTION_DECK_TILE: &str = "deck_tile";

/// User-configured "When X, Do Y" automation rule persisted in `settings.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationRule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    /// `"phone_connected"` | `"phone_disconnected"` | `"call_ringing"` |
    /// `"call_ended"` | `"battery_low"` | `"charging_started"`
    pub trigger: String,
    /// `"mute_pc"` | `"unmute_pc"` | `"pause_pc_media"` | `"lock_pc"` |
    /// `"run_command"` | `"deck_tile"`
    pub action_kind: String,
    /// Optional command string (`"run_command"`) or Deck tile ID (`"deck_tile"`).
    #[serde(default)]
    pub action_arg: String,
}

static PREV_BATTERY: OnceLock<Mutex<HashMap<DeviceId, (u8, bool)>>> = OnceLock::new();

fn battery_history() -> &'static Mutex<HashMap<DeviceId, (u8, bool)>> {
    PREV_BATTERY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Pure evaluation helper returning all enabled [`AutomationRule`]s matching `trigger`.
pub fn matching_rules<'a>(rules: &'a [AutomationRule], trigger: &str) -> Vec<&'a AutomationRule> {
    rules.iter().filter(|r| r.enabled && r.trigger == trigger).collect()
}

fn execute_rule(rule: &AutomationRule) {
    let res = match rule.action_kind.as_str() {
        ACTION_MUTE_PC => win::audio::set_speaker_mute(true),
        ACTION_UNMUTE_PC => win::audio::set_speaker_mute(false),
        ACTION_PAUSE_PC_MEDIA => {
            win::media_sessions::pause_for_call();
            Ok(())
        }
        ACTION_LOCK_PC => win::shell::lock(),
        ACTION_RUN_COMMAND => win::shell::run_command(&rule.action_arg),
        ACTION_DECK_TILE => deck::execute_tile(&rule.action_arg),
        other => Err(format!("unknown automation action_kind: {other}")),
    };
    if let Err(e) = res {
        tracing::warn!(rule = %rule.name, trigger = %rule.trigger, error = %e, "automation action failed");
    } else {
        let _ = crate::pc_notifications::forward_to_phones(
            "NectarLink Automations",
            &rule.name,
            &format!("Triggered by {}", rule.trigger),
        );
    }
}

/// Evaluates all enabled rules in `settings::automation_rules()` matching `trigger`
/// and runs their configured actions.
pub fn fire_trigger(trigger: &str) {
    let rules = settings::automation_rules();
    for rule in matching_rules(&rules, trigger) {
        execute_rule(rule);
    }
}

/// Hooks into the main `NodeEvent` loop to fire triggers on device link, call,
/// and battery transitions.
pub fn on_event(event: &NodeEvent) {
    match event {
        NodeEvent::LinkChanged { link: LinkState::Online { .. }, .. } => {
            fire_trigger(TRIGGER_PHONE_CONNECTED);
        }
        NodeEvent::LinkChanged { link: LinkState::Offline { .. }, .. } => {
            fire_trigger(TRIGGER_PHONE_DISCONNECTED);
        }
        NodeEvent::Call { call, .. } => match call.state.as_str() {
            "ringing" => fire_trigger(TRIGGER_CALL_RINGING),
            "ended" => fire_trigger(TRIGGER_CALL_ENDED),
            _ => {}
        },
        NodeEvent::Battery { device, battery } => {
            let prev = battery_history()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(*device, (battery.level, battery.charging));
            let was_low = prev.is_some_and(|(lvl, chg)| lvl <= 20 && !chg);
            let was_charging = prev.is_some_and(|(_, chg)| chg);
            if battery.level <= 20 && !battery.charging && !was_low {
                fire_trigger(TRIGGER_BATTERY_LOW);
            }
            if battery.charging && !was_charging {
                fire_trigger(TRIGGER_CHARGING_STARTED);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_rules_filters_by_trigger_and_enabled_flag() {
        let rules = vec![
            AutomationRule {
                id: "r1".into(),
                name: "Mute on call".into(),
                enabled: true,
                trigger: TRIGGER_CALL_RINGING.into(),
                action_kind: ACTION_MUTE_PC.into(),
                action_arg: String::new(),
            },
            AutomationRule {
                id: "r2".into(),
                name: "Disabled rule".into(),
                enabled: false,
                trigger: TRIGGER_CALL_RINGING.into(),
                action_kind: ACTION_PAUSE_PC_MEDIA.into(),
                action_arg: String::new(),
            },
            AutomationRule {
                id: "r3".into(),
                name: "Lock on disconnect".into(),
                enabled: true,
                trigger: TRIGGER_PHONE_DISCONNECTED.into(),
                action_kind: ACTION_LOCK_PC.into(),
                action_arg: String::new(),
            },
        ];

        let matched = matching_rules(&rules, TRIGGER_CALL_RINGING);
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].id, "r1");

        let disconnect = matching_rules(&rules, TRIGGER_PHONE_DISCONNECTED);
        assert_eq!(disconnect.len(), 1);
        assert_eq!(disconnect[0].id, "r3");

        assert!(matching_rules(&rules, TRIGGER_BATTERY_LOW).is_empty());
    }

    #[test]
    fn automation_rule_round_trips_json() {
        let rule = AutomationRule {
            id: "auto_1".into(),
            name: "Run backup when charging".into(),
            enabled: true,
            trigger: TRIGGER_CHARGING_STARTED.into(),
            action_kind: ACTION_DECK_TILE.into(),
            action_arg: "tile_1".into(),
        };
        let json = serde_json::to_string(&rule).unwrap();
        let decoded: AutomationRule = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, rule);
    }
}
