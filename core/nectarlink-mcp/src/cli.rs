// SPDX-License-Identifier: GPL-3.0-or-later
//! `mcp`, `do` and `actions`: an AI agent's
//! way into the running NectarLink app. These never start a node of their
//! own; the app checks every call (docs/agents/README.md).

use anyhow::Result;
use crate::{AppClient, Mode, client::app_dir};
use serde_json::{Value, json};

/// Exit codes for `nectarlink do`, so scripts can tell what happened.
fn exit_code(reply: &Value) -> i32 {
    if reply.get("ok").and_then(Value::as_bool) == Some(true) {
        return 0;
    }
    match reply.pointer("/error/code").and_then(Value::as_str).unwrap_or("") {
        "APP_NOT_RUNNING" | "AGENT_ACCESS_OFF" | "APP_NOT_READY" => 3,
        "UNAUTHENTICATED" | "CLIENT_PAUSED" | "SCOPE_DENIED" | "READ_ONLY" | "DENIED"
        | "DEVICE_NOT_ALLOWED" => 4,
        "CONFIRM_DENIED" | "CONFIRM_TIMEOUT" | "CONFIRMATION_PENDING" => 5,
        "RATE_LIMITED" => 6,
        "DEVICE_OFFLINE" | "UNSUPPORTED" => 7,
        "INVALID_ARGUMENTS" | "UNKNOWN_ACTION" | "DEVICE_AMBIGUOUS" | "DEVICE_NOT_FOUND" => 2,
        _ => 1,
    }
}

fn connect(agent: &str, channel: &str) -> Result<AppClient, Value> {
    let dir = app_dir().ok_or_else(|| json!({ "ok": false, "error": { "code": "APP_NOT_RUNNING", "message": "No data folder for NectarLink on this PC." } }))?;
    AppClient::connect(&dir, agent, channel).map_err(|e| e.to_json())
}

/// `nectarlink mcp`: serves MCP on stdin and stdout.
pub async fn mcp(agent: &str, dispatch: bool, toolsets: &[String]) -> Result<i32> {
    let dir = app_dir();
    let client = match dir {
        Some(d) => AppClient::connect(&d, agent, "mcp-stdio"),
        None => Err(crate::ClientError::AppNotRunning),
    };
    let mode = if dispatch { Mode::Dispatch } else { Mode::Tools };
    crate::serve_stdio(client, mode, toolsets.to_vec()).await?;
    Ok(0)
}

/// `nectarlink do <action>`: runs one action and prints the JSON answer.
pub async fn run(
    agent: &str,
    action: &str,
    args: Option<&str>,
    device: Option<&str>,
    pretty: bool,
) -> Result<i32> {
    let reply = match connect(agent, "cli") {
        Err(e) => e,
        Ok(client) => {
            let mut args = match args {
                Some(text) => match serde_json::from_str::<Value>(text) {
                    Ok(v @ Value::Object(_)) => v,
                    _ => {
                        let e = json!({ "ok": false, "error": { "code": "INVALID_ARGUMENTS", "message": "--args must be a JSON object." } });
                        print(&e, pretty);
                        return Ok(2);
                    }
                },
                None => json!({}),
            };
            if let (Some(d), Some(map)) = (device, args.as_object_mut()) {
                map.insert("device".into(), json!(d));
            }
            client.call(action, args).await.unwrap_or_else(|e| e)
        }
    };
    print(&reply, pretty);
    Ok(exit_code(&reply))
}

/// `nectarlink actions`: what this agent may do.
pub async fn list(agent: &str, pretty: bool) -> Result<i32> {
    let reply = match connect(agent, "cli") {
        Err(e) => e,
        Ok(client) => match client.actions().await {
            Ok(list) => json!({ "ok": true, "result": { "actions": list.iter().map(|a| json!({
                "id": a["id"], "title": a["title"], "description": a["description"], "confirm": a["confirm"],
                "input_schema": a["input_schema"],
            })).collect::<Vec<_>>() } }),
            Err(e) => e,
        },
    };
    print(&reply, pretty);
    Ok(exit_code(&reply))
}

fn print(v: &Value, pretty: bool) {
    let text = if pretty { serde_json::to_string_pretty(v) } else { serde_json::to_string(v) };
    println!("{}", text.unwrap_or_default());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_say_what_happened() {
        assert_eq!(exit_code(&json!({ "ok": true, "result": {} })), 0);
        assert_eq!(exit_code(&json!({ "ok": false, "error": { "code": "CONFIRM_DENIED" } })), 5);
        assert_eq!(exit_code(&json!({ "ok": false, "error": { "code": "SCOPE_DENIED" } })), 4);
        assert_eq!(exit_code(&json!({ "ok": false, "error": { "code": "APP_NOT_RUNNING" } })), 3);
        assert_eq!(exit_code(&json!({ "ok": false, "error": { "code": "WHATEVER" } })), 1);
    }
}
