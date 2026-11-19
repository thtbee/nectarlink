// SPDX-License-Identifier: GPL-3.0-or-later
//! `Agents`: QML singleton for Settings > AI agents (crate::agents): the
//! connected agents, their activity, the confirmation waiting for the user,
//! and the setup snippet for each kind of client.

use std::pin::Pin;

use cxx_qt::Threading;
use cxx_qt_lib::QString;
use nectarlink_actions::{Decision, Grant};
use serde_json::json;

use crate::state::Changes;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        /// The agents, as JSON: `[{ id, label, name, kind, preset, grants, readOnly, paused, lastUsed }]`.
        #[qproperty(QString, clients)]
        /// Recent activity, newest first, as JSON.
        #[qproperty(QString, activity)]
        /// The confirmation waiting for the user, as JSON, or "".
        #[qproperty(QString, confirmation)]
        /// The port agents connect to, or 0 while agent access is off.
        #[qproperty(i32, port)]
        /// How the last "Add to" went, as JSON `{ kind, ok, message }`, or "".
        #[qproperty(QString, setup_status, cxx_name = "setupStatus")]
        type Agents = super::AgentsRust;
    }

    impl cxx_qt::Threading for Agents {}
    impl cxx_qt::Initialize for Agents {}

    unsafe extern "RustQt" {
        #[qinvokable]
        fn refresh(self: Pin<&mut Agents>);
        /// Adds an agent; `preset` is "read_only" or "everyday". Returns "" or why it failed.
        #[qinvokable]
        #[cxx_name = "addAgent"]
        fn add_agent(
            self: &Agents,
            label: &QString,
            name: &QString,
            kind: &QString,
            preset: &QString,
        ) -> QString;
        #[qinvokable]
        fn remove(self: &Agents, id: &QString);
        #[qinvokable]
        #[cxx_name = "setPaused"]
        fn set_paused(self: &Agents, id: &QString, paused: bool);
        #[qinvokable]
        #[cxx_name = "setReadOnly"]
        fn set_read_only(self: &Agents, id: &QString, read_only: bool);
        /// Turns one permission ("messages:write") on or off for an agent.
        #[qinvokable]
        #[cxx_name = "setGrant"]
        fn set_grant(self: &Agents, id: &QString, grant: &QString, on: bool);
        /// "allow", "always" or "deny".
        #[qinvokable]
        fn answer(self: &Agents, id: &QString, decision: &QString);
        #[qinvokable]
        #[cxx_name = "clearActivity"]
        fn clear_activity(self: &Agents);
        /// The configuration to paste into `client` (claude-code, cursor, vscode, codex, gemini, zed, other).
        #[qinvokable]
        fn snippet(self: &Agents, label: &QString, client: &QString) -> QString;
        /// Whether NectarLink can add itself to the app `client` here.
        #[qinvokable]
        #[cxx_name = "canSetUp"]
        fn can_set_up(self: &Agents, client: &QString) -> bool;
        /// Adds agent `label` to the app `client`; `setupStatus` says how it went.
        #[qinvokable]
        #[cxx_name = "setUp"]
        fn set_up(self: &Agents, label: &QString, client: &QString);
        #[qinvokable]
        #[cxx_name = "clearSetupStatus"]
        fn clear_setup_status(self: &Agents);
        /// Sets what an agent may do in one area: "off", "read" or "write".
        #[qinvokable]
        #[cxx_name = "setAccess"]
        fn set_access(self: &Agents, id: &QString, scope: &QString, level: &QString);
    }
}

#[derive(Default)]
pub struct AgentsRust {
    clients: QString,
    activity: QString,
    confirmation: QString,
    port: i32,
    setup_status: QString,
}

impl cxx_qt::Initialize for qobject::Agents {
    fn initialize(self: Pin<&mut Self>) {
        super::subscribe(self.qt_thread(), Changes::REMOTE | Changes::PREFS, Self::refresh);
    }
}

fn preset_of(grants: &std::collections::BTreeSet<Grant>) -> &'static str {
    if *grants == nectarlink_actions::preset_read_only() {
        "read_only"
    } else if *grants == nectarlink_actions::preset_everyday() {
        "everyday"
    } else {
        "custom"
    }
}

impl qobject::Agents {
    pub fn refresh(mut self: Pin<&mut Self>) {
        let clients: Vec<_> = crate::agents::clients()
            .into_iter()
            .map(|c| {
                json!({
                    "id": c.id,
                    "label": c.label,
                    "name": c.display_name,
                    "kind": c.kind,
                    "preset": preset_of(&c.grants),
                    "grants": c.grants_text(),
                    "readOnly": c.read_only,
                    "paused": c.paused,
                    "lastUsed": c.last_used.unwrap_or(0),
                })
            })
            .collect();
        let activity: Vec<_> = crate::agents::activity(100)
            .into_iter()
            .map(|a| {
                json!({
                    "ts": a.ts, "client": a.client_name, "action": a.action, "device": a.device,
                    "summary": a.summary, "decision": a.decision, "confirm": a.confirm,
                })
            })
            .collect();
        let confirmation = crate::agents::current_confirmation()
            .map(|r| {
                json!({
                    "id": r.id, "client": r.client, "action": r.action, "title": r.title, "device": r.device,
                    "details": r.details.iter().map(|(k, v)| json!({ "label": k, "value": v })).collect::<Vec<_>>(),
                    "canRemember": r.can_remember,
                })
                .to_string()
            })
            .unwrap_or_default();
        self.as_mut().set_clients(QString::from(&serde_json::Value::Array(clients).to_string()));
        self.as_mut().set_activity(QString::from(&serde_json::Value::Array(activity).to_string()));
        self.as_mut().set_confirmation(QString::from(&confirmation));
        self.as_mut().set_port(i32::from(crate::agents::port().unwrap_or(0)));
        let status = crate::agents::setup_status()
            .map(|(kind, r)| {
                let (ok, message) = match r {
                    Ok(path) => (true, path),
                    Err(e) => (false, e),
                };
                json!({ "kind": kind, "ok": ok, "message": message }).to_string()
            })
            .unwrap_or_default();
        self.as_mut().set_setup_status(QString::from(&status));
    }

    pub fn add_agent(&self, label: &QString, name: &QString, kind: &QString, preset: &QString) -> QString {
        let grants = match String::from(preset).as_str() {
            "everyday" => nectarlink_actions::preset_everyday(),
            _ => nectarlink_actions::preset_read_only(),
        };
        match crate::agents::add_client(
            &String::from(label),
            &String::from(name),
            &String::from(kind),
            &grants,
        ) {
            Ok(_) => QString::default(),
            Err(e) => QString::from(&e),
        }
    }

    pub fn remove(&self, id: &QString) {
        crate::agents::remove_client(&String::from(id));
    }

    pub fn set_paused(&self, id: &QString, paused: bool) {
        crate::agents::update_client(&String::from(id), |c| c.paused = paused);
    }

    pub fn set_read_only(&self, id: &QString, read_only: bool) {
        crate::agents::update_client(&String::from(id), |c| c.read_only = read_only);
    }

    pub fn set_grant(&self, id: &QString, grant: &QString, on: bool) {
        let Some(g) = Grant::parse(&String::from(grant)) else { return };
        crate::agents::update_client(&String::from(id), |c| {
            if on {
                c.grants.insert(g);
            } else {
                c.grants.remove(&g);
                // Taking write away keeps read.
                if g.access == nectarlink_actions::Access::Write {
                    c.grants.insert(Grant::new(g.scope, nectarlink_actions::Access::Read));
                }
            }
        });
    }

    pub fn answer(&self, id: &QString, decision: &QString) {
        let d = match String::from(decision).as_str() {
            "allow" => Decision::Allow,
            "always" => Decision::AllowAlways,
            _ => Decision::Deny,
        };
        crate::agents::answer(&String::from(id), d);
    }

    pub fn clear_activity(&self) {
        crate::agents::clear_activity();
    }

    pub fn snippet(&self, label: &QString, client: &QString) -> QString {
        let exe = cli_path();
        QString::from(&snippet_for(&String::from(client), &exe, &String::from(label)))
    }
}

impl qobject::Agents {
    pub fn can_set_up(&self, client: &QString) -> bool {
        crate::agents::can_set_up(&String::from(client))
    }

    pub fn set_up(&self, label: &QString, client: &QString) {
        crate::agents::set_up(&String::from(client), &String::from(label), &cli_path());
    }

    pub fn clear_setup_status(&self) {
        crate::agents::clear_setup_status();
    }

    pub fn set_access(&self, id: &QString, scope: &QString, level: &QString) {
        let Some(scope) = nectarlink_actions::Scope::parse(&String::from(scope)) else { return };
        let level = String::from(level);
        crate::agents::update_client(&String::from(id), |c| {
            c.grants.retain(|g| g.scope != scope);
            match level.as_str() {
                "read" => {
                    c.grants.insert(Grant::new(scope, nectarlink_actions::Access::Read));
                }
                "write" => {
                    c.grants.insert(Grant::new(scope, nectarlink_actions::Access::Write));
                }
                _ => {}
            }
        });
    }
}

/// The `nectarlink-agent` command next to the app.
fn cli_path() -> String {
    let name = if cfg!(windows) { "nectarlink-agent.exe" } else { "nectarlink-agent" };
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(name)))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.to_owned())
}

/// What to paste into each client. Only the command and the agent's name:
/// the token stays in NectarLink's own folder.
pub fn snippet_for(client: &str, exe: &str, label: &str) -> String {
    let args = json!(["mcp", "--agent", label]);
    match client {
        "claude-code" => format!("claude mcp add --scope user nectarlink -- \"{exe}\" mcp --agent {label}"),
        "codex" => format!(
            "[mcp_servers.nectarlink]\ncommand = '{exe}'\nargs = [\"mcp\", \"--agent\", \"{label}\"]\ntool_timeout_sec = 120\n"
        ),
        "gemini" => format!("gemini mcp add -s user nectarlink \"{exe}\" mcp --agent {label}"),
        "vscode" => serde_json::to_string_pretty(
            &json!({ "servers": { "nectarlink": { "type": "stdio", "command": exe, "args": args } } }),
        )
        .unwrap_or_default(),
        "zed" => serde_json::to_string_pretty(
            &json!({ "context_servers": { "nectarlink": { "command": exe, "args": args } } }),
        )
        .unwrap_or_default(),
        "cli" => format!("\"{exe}\" do devices.list --agent {label}"),
        // Cursor, Claude Desktop, Antigravity, Windsurf and most others.
        _ => serde_json::to_string_pretty(
            &json!({ "mcpServers": { "nectarlink": { "command": exe, "args": args } } }),
        )
        .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippets_name_the_agent_and_never_hold_a_token() {
        for client in ["claude-code", "codex", "gemini", "vscode", "zed", "cursor", "cli"] {
            let s = snippet_for(client, r"C:\Program Files\NectarLink\nectarlink-agent.exe", "claude-code");
            assert!(s.contains("claude-code"), "{client}: {s}");
            assert!(!s.to_lowercase().contains("token"), "{client}: {s}");
        }
        let json: serde_json::Value =
            serde_json::from_str(&snippet_for("cursor", "/usr/bin/nectarlink-agent", "cursor")).unwrap();
        assert_eq!(json["mcpServers"]["nectarlink"]["args"][2], "cursor");
    }

    #[test]
    fn every_action_that_asks_has_a_translated_title() {
        let sheet = include_str!("../../qml/components/AgentConfirmSheet.qml");
        for a in nectarlink_actions::registry::all() {
            if a.confirm == nectarlink_actions::ConfirmRule::Never {
                continue;
            }
            let case = format!("case \"{}\": return qsTr(\"{}\")", a.id, a.title);
            assert!(sheet.contains(&case), "AgentConfirmSheet.qml needs: {case}");
        }
    }

    #[test]
    fn every_app_with_a_settings_file_can_be_picked() {
        let section = include_str!("../../qml/components/AgentsSection.qml");
        for app in nectarlink_actions::setup::APPS {
            assert!(
                section.contains(&format!("value: \"{}\"", app.kind)),
                "{} missing from Settings",
                app.kind
            );
        }
    }
}
