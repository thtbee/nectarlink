// SPDX-License-Identifier: GPL-3.0-or-later
//! Phone Status desktop widget snapshot and Adaptive Card provider (`--widget-provider`)
//! on Linux, matching `desktop/app/src/win/widget.rs`.

use std::{
    fs,
    io::{self, BufRead},
    path::{Path, PathBuf},
    process::ExitCode,
};

use nectarlink_core::LinkState;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{core_host, launch::Request, win::single_instance};

/// CLI flag launched for the widget provider mode.
pub const ARG: &str = "--widget-provider";

/// Widget definition ID in the widget manifest.
pub const WIDGET_ID: &str = "NectarlinkPhoneStatus";

/// Snapshot of the primary phone's live status for the widget card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WidgetState {
    pub connected: bool,
    pub phone_name: String,
    pub battery_level: Option<u8>,
    pub battery_charging: bool,
    pub last_clip_preview: String,
}

/// Path to `widget-state.json` inside `data_dir`.
pub fn state_snapshot_path(data_dir: &Path) -> PathBuf {
    data_dir.join("widget-state.json")
}

/// Writes `state` to `<data_dir>/widget-state.json`.
pub fn write_snapshot_file(data_dir: &Path, state: &WidgetState) -> io::Result<()> {
    let path = state_snapshot_path(data_dir);
    let json = serde_json::to_vec_pretty(state).map_err(io::Error::other)?;
    fs::write(path, json)
}

/// Reads `<data_dir>/widget-state.json`, or returns [`WidgetState::default`] if missing/corrupt.
pub fn read_snapshot_file(data_dir: &Path) -> WidgetState {
    let path = state_snapshot_path(data_dir);
    fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<WidgetState>(&bytes).ok())
        .unwrap_or_default()
}

/// Captures the current primary phone state from [`core_host`] and writes `widget-state.json`.
pub fn write_state_snapshot(data_dir: &Path) {
    let last_clip_preview = core_host::node()
        .and_then(|n| n.clipboard_history(None).into_iter().next())
        .map(|c| truncate_preview(&c.text, 120))
        .unwrap_or_default();
    let state = core_host::host().hub.read(|s| {
        let primary = s
            .devices
            .iter()
            .find(|d| matches!(d.link, LinkState::Online { .. }))
            .or_else(|| s.devices.first());
        match primary {
            Some(dev) => WidgetState {
                connected: matches!(dev.link, LinkState::Online { .. }),
                phone_name: dev.info.name.clone(),
                battery_level: dev.battery.as_ref().map(|b| b.level),
                battery_charging: dev.battery.as_ref().is_some_and(|b| b.charging),
                last_clip_preview,
            },
            None => WidgetState { last_clip_preview, ..WidgetState::default() },
        }
    });
    if let Err(e) = write_snapshot_file(data_dir, &state) {
        tracing::debug!(error = %e, "could not write widget-state.json");
    }
}

fn truncate_preview(text: &str, max_chars: usize) -> String {
    let single_line: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if single_line.chars().count() <= max_chars {
        single_line
    } else {
        let mut s: String = single_line.chars().take(max_chars.saturating_sub(1)).collect();
        s.push('…');
        s
    }
}

/// Builds the Adaptive Card v1.5 JSON payload for `state`.
pub fn render_adaptive_card(state: &WidgetState) -> String {
    let status_text = if state.phone_name.is_empty() {
        "No phone paired".to_owned()
    } else if state.connected {
        format!("{} · Connected", state.phone_name)
    } else {
        format!("{} · Offline", state.phone_name)
    };

    let battery_text = match state.battery_level {
        Some(pct) if state.battery_charging => format!("Battery: {pct}% (Charging)"),
        Some(pct) => format!("Battery: {pct}%"),
        None => "Battery: —".to_owned(),
    };

    let clip_text = if state.last_clip_preview.trim().is_empty() {
        "No recent clipboard item".to_owned()
    } else {
        state.last_clip_preview.clone()
    };

    let card: Value = json!({
        "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
        "type": "AdaptiveCard",
        "version": "1.5",
        "body": [
            {
                "type": "TextBlock",
                "text": status_text,
                "weight": "Bolder",
                "size": "Medium",
                "wrap": true
            },
            {
                "type": "TextBlock",
                "text": battery_text,
                "spacing": "Small",
                "isSubtle": true,
                "wrap": true
            },
            {
                "type": "TextBlock",
                "text": "Latest Clipboard",
                "weight": "Bolder",
                "spacing": "Medium",
                "size": "Small"
            },
            {
                "type": "TextBlock",
                "text": clip_text,
                "spacing": "None",
                "isSubtle": true,
                "maxLines": 2,
                "wrap": true
            }
        ],
        "actions": [
            {
                "type": "Action.Execute",
                "title": "Send File",
                "verb": "send_file"
            },
            {
                "type": "Action.Execute",
                "title": "Find My Phone",
                "verb": "find_phone"
            },
            {
                "type": "Action.Execute",
                "title": "Mirror Screen",
                "verb": "mirror_screen"
            }
        ]
    });
    card.to_string()
}

/// Dispatches an Adaptive Card `Action.Execute` verb (`"send_file"`, `"find_phone"`, `"mirror_screen"`, `"open"`)
/// to the running NectarLink instance (or launches NectarLink with `--task <verb>` if not running).
pub fn handle_action_verb(verb: &str, data_dir: Option<&Path>) -> Result<(), String> {
    let action = match verb.trim() {
        "send_file" => "send_file",
        "find_phone" => "find_phone",
        "mirror_screen" => "mirror_screen",
        "open" | "" => "open",
        other => return Err(format!("unknown widget action verb: {other}")),
    };

    if let Some(dir) = data_dir {
        let req = if action == "open" { Request::Show } else { Request::Task { action: action.to_owned() } };
        if crate::launch::queue(dir, &req).is_ok() {
            single_instance::wake(dir);
            return Ok(());
        }
    }

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = std::process::Command::new(exe);
    if let Some(dir) = data_dir {
        cmd.arg("--data-dir").arg(dir);
    }
    if action != "open" {
        cmd.arg(crate::win::jumplist::ARG).arg(action);
    }
    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Runs the `--widget-provider` out-of-process host loop.
pub fn run_provider(data_dir: &Path) -> ExitCode {
    let state = read_snapshot_file(data_dir);
    let card = render_adaptive_card(&state);
    tracing::info!(widget_id = WIDGET_ID, "widget provider started");
    println!("{card}");

    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.eq_ignore_ascii_case("quit") || trimmed.eq_ignore_ascii_case("exit") {
            break;
        }
        if trimmed.eq_ignore_ascii_case("refresh") {
            let refreshed = read_snapshot_file(data_dir);
            println!("{}", render_adaptive_card(&refreshed));
            continue;
        }
        if let Some(verb) = trimmed.strip_prefix("verb:") {
            let _ = handle_action_verb(verb, Some(data_dir));
            let refreshed = read_snapshot_file(data_dir);
            println!("{}", render_adaptive_card(&refreshed));
        }
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_valid_adaptive_card_for_connected_and_empty_states() {
        let empty = WidgetState::default();
        let empty_json: Value = serde_json::from_str(&render_adaptive_card(&empty)).unwrap();
        assert_eq!(empty_json["type"], "AdaptiveCard");
        assert_eq!(empty_json["version"], "1.5");
        assert_eq!(empty_json["body"][0]["text"], "No phone paired");
        assert_eq!(empty_json["actions"].as_array().unwrap().len(), 3);

        let connected = WidgetState {
            connected: true,
            phone_name: "Pixel 9 Pro".into(),
            battery_level: Some(84),
            battery_charging: true,
            last_clip_preview: "https://nectarlink.dev/docs".into(),
        };
        let card_str = render_adaptive_card(&connected);
        let parsed: Value = serde_json::from_str(&card_str).unwrap();
        assert_eq!(parsed["body"][0]["text"], "Pixel 9 Pro · Connected");
        assert_eq!(parsed["body"][1]["text"], "Battery: 84% (Charging)");
        assert_eq!(parsed["body"][3]["text"], "https://nectarlink.dev/docs");
    }

    #[test]
    fn snapshot_file_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let state = WidgetState {
            connected: true,
            phone_name: "Galaxy S24".into(),
            battery_level: Some(62),
            battery_charging: false,
            last_clip_preview: "Copied text".into(),
        };
        write_snapshot_file(dir.path(), &state).unwrap();
        let loaded = read_snapshot_file(dir.path());
        assert_eq!(loaded, state);
    }
}
