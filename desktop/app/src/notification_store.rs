// SPDX-License-Identifier: GPL-3.0-or-later
//! What the app keeps about notifications between runs: the user's rules
//! for apps (and those apps' names), whether history is kept, and the
//! history itself. Two JSON files in the data folder, written in the
//! background shortly after a change.

use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, SystemTime},
};

use serde::{Deserialize, Serialize};

use crate::{
    core_host,
    state::{AppRule, AppState, Changes, HistoryEntry, unix_ms},
};

const RULES_FILE: &str = "notification-rules.json";
const HISTORY_FILE: &str = "notification-history.json";
/// Changes come in bursts (a phone reconnecting resends everything).
const SAVE_DELAY: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Rules {
    /// Keep a day of history.
    history: bool,
    /// Per app (package name): its name, and its rule if not "show".
    apps: BTreeMap<String, AppEntry>,
}

impl Default for Rules {
    fn default() -> Self {
        Rules { history: true, apps: BTreeMap::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct AppEntry {
    name: String,
    rule: AppRule,
}

impl Default for AppEntry {
    fn default() -> Self {
        AppEntry { name: String::new(), rule: AppRule::Show }
    }
}

fn read<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> T {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            tracing::warn!(error = %e, file = %path.display(), "unreadable; starting over");
            T::default()
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => T::default(),
        Err(e) => {
            tracing::warn!(error = %e, file = %path.display(), "can't read");
            T::default()
        }
    }
}

/// Writes a whole file or nothing (a temporary file renamed over it).
fn write(path: &Path, value: &impl Serialize) -> std::io::Result<()> {
    let partial = path.with_extension("json.tmp");
    let mut file = fs::File::create(&partial)?;
    file.write_all(&serde_json::to_vec(value).map_err(std::io::Error::other)?)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&partial, path)
}

/// Loads what was kept into the state.
pub fn load(dir: &Path, state: &mut AppState) {
    let rules: Rules = read(&dir.join(RULES_FILE));
    state.history_enabled = rules.history;
    for (app, entry) in rules.apps {
        if !entry.name.is_empty() {
            state.app_names.insert(app.clone(), entry.name);
        }
        if entry.rule != AppRule::Show {
            state.app_rules.insert(app, entry.rule);
        }
    }
    if state.history_enabled {
        state.history = read(&dir.join(HISTORY_FILE));
        state.prune_history(unix_ms(SystemTime::now()));
    }
}

fn snapshot(state: &AppState) -> (Rules, Vec<HistoryEntry>) {
    let apps = state
        .app_names
        .iter()
        .map(|(app, name)| (app.clone(), AppEntry { name: name.clone(), rule: state.app_rule(app) }))
        .chain(
            state
                .app_rules
                .iter()
                .filter(|(app, _)| !state.app_names.contains_key(*app))
                .map(|(app, rule)| (app.clone(), AppEntry { name: String::new(), rule: *rule })),
        )
        .collect();
    (Rules { history: state.history_enabled, apps }, state.history.clone())
}

/// Saves after changes from now on, and lets history older than a day go
/// while the app runs.
pub fn start(dir: PathBuf) {
    static PENDING: AtomicBool = AtomicBool::new(false);
    let pruner = std::thread::Builder::new().name("prune-history".into()).spawn(|| {
        loop {
            std::thread::sleep(Duration::from_secs(600));
            core_host::host().hub.update(|s| s.prune_history(unix_ms(SystemTime::now())));
        }
    });
    if let Err(e) = pruner {
        tracing::warn!(error = %e, "history won't be trimmed while running");
    }
    core_host::host().hub.subscribe(Changes::APPS | Changes::HISTORY, move || {
        if !PENDING.swap(true, Ordering::AcqRel) {
            let dir = dir.clone();
            let spawned = std::thread::Builder::new().name("save-notifications".into()).spawn(move || {
                std::thread::sleep(SAVE_DELAY);
                PENDING.store(false, Ordering::Release);
                let (rules, history) = core_host::host().hub.read(snapshot);
                if let Err(e) = write(&dir.join(RULES_FILE), &rules) {
                    tracing::warn!(error = %e, "can't save notification rules");
                }
                let history_file = dir.join(HISTORY_FILE);
                let saved =
                    if rules.history { write(&history_file, &history) } else { remove(&history_file) };
                if let Err(e) = saved {
                    tracing::warn!(error = %e, "can't save notification history");
                }
            });
            if spawned.is_err() {
                PENDING.store(false, Ordering::Release);
            }
        }
        true
    });
}

fn remove(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use nectarlink_core::{DeviceId, Notification};

    use super::*;

    fn note(key: &str, app: &str) -> Notification {
        Notification {
            key: key.into(),
            app: app.into(),
            app_name: app.to_uppercase(),
            title: Some("Hi".into()),
            text: None,
            sub: None,
            when: 1,
            actions: Vec::new(),
            silent: false,
            icon: None,
            image: None,
            live: None,
        }
    }

    #[test]
    fn rules_and_history_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = AppState { history_enabled: true, ..AppState::default() };
        state.app_names.insert("com.chat".into(), "Chat".into());
        state.set_app_rule("com.chat", AppRule::Quiet);
        state.set_app_rule("com.game", AppRule::Hidden);
        let now = unix_ms(SystemTime::now());
        state.history.push(HistoryEntry {
            device: DeviceId([1; 32]),
            notification: note("a", "com.chat"),
            removed_at: now,
        });
        state.history.push(HistoryEntry {
            device: DeviceId([1; 32]),
            notification: note("old", "com.chat"),
            removed_at: now - crate::state::HISTORY_MS - 1,
        });
        let (rules, history) = snapshot(&state);
        write(&dir.path().join(RULES_FILE), &rules).unwrap();
        write(&dir.path().join(HISTORY_FILE), &history).unwrap();

        let mut loaded = AppState::default();
        load(dir.path(), &mut loaded);
        assert!(loaded.history_enabled);
        assert_eq!(loaded.app_rule("com.chat"), AppRule::Quiet);
        assert_eq!(loaded.app_rule("com.game"), AppRule::Hidden);
        assert_eq!(loaded.app_rule("com.other"), AppRule::Show);
        assert_eq!(loaded.app_names.get("com.chat").map(String::as_str), Some("Chat"));
        assert_eq!(loaded.history.len(), 1, "a day old is gone");
        assert_eq!(loaded.history[0].notification.key, "a");
    }

    #[test]
    fn missing_or_broken_files_start_fresh() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = AppState::default();
        load(dir.path(), &mut state);
        assert!(state.history_enabled, "on by default");
        fs::write(dir.path().join(RULES_FILE), b"{ nope").unwrap();
        let mut state = AppState::default();
        load(dir.path(), &mut state);
        assert!(state.app_rules.is_empty());
    }
}
