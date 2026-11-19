// SPDX-License-Identifier: GPL-3.0-or-later
//! The MCP server agents start over stdio (`nectarlink mcp --agent <name>`).
//! Tools are the app's actions; with `Mode::Dispatch` there are only three
//! (search, describe, run), for clients that cap how many tools they load.

use std::{borrow::Cow, sync::Arc};

use rmcp::{
    ErrorData, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
        ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool, ToolAnnotations,
    },
    service::{RequestContext, RoleServer},
};
use serde_json::{Map, Value, json};

use crate::{AppClient, INSTRUCTIONS, client::ClientError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// One tool per action.
    Tools,
    /// Three tools that reach every action.
    Dispatch,
}

/// Serves MCP on stdin and stdout until the client goes away.
pub async fn serve_stdio(
    client: Result<AppClient, ClientError>,
    mode: Mode,
    toolsets: Vec<String>,
) -> anyhow::Result<()> {
    let server = Server { client: Arc::new(client), mode, toolsets };
    let running = server.serve(rmcp::transport::stdio()).await?;
    running.waiting().await?;
    Ok(())
}

#[derive(Debug, Clone)]
struct Server {
    client: Arc<Result<AppClient, ClientError>>,
    mode: Mode,
    toolsets: Vec<String>,
}

impl Server {
    /// The app's actions for this agent; nothing when it can't be reached.
    async fn actions(&self) -> Vec<Value> {
        let Ok(client) = self.client.as_ref() else { return Vec::new() };
        let list = client.actions().await.unwrap_or_default();
        list.into_iter()
            .filter(|a| {
                self.toolsets.is_empty()
                    || a.get("toolset")
                        .and_then(Value::as_str)
                        .is_some_and(|t| self.toolsets.iter().any(|s| s == t))
            })
            .collect()
    }

    async fn run(&self, action: &str, args: Value) -> CallToolResult {
        let reply = match self.client.as_ref() {
            Ok(c) => c.call(action, args).await.unwrap_or_else(|e| e),
            Err(e) => e.to_json(),
        };
        to_result(reply)
    }
}

/// The app's answer as a tool result: the JSON as structured content and as
/// text, marked as an error when it is one.
fn to_result(reply: Value) -> CallToolResult {
    if reply.get("ok").and_then(Value::as_bool) == Some(true) {
        let result = reply.get("result").cloned().unwrap_or(Value::Null);
        let mut r = CallToolResult::success(vec![ContentBlock::text(pretty(&result))]);
        if result.is_object() {
            r.structured_content = Some(result);
        }
        r
    } else {
        let error = reply.get("error").cloned().unwrap_or(reply);
        let mut text = error.get("message").and_then(Value::as_str).unwrap_or("It didn't work.").to_owned();
        if let Some(code) = error.get("code").and_then(Value::as_str) {
            text = format!("{code}: {text}");
        }
        if let Some(what) = error.get("user_action").and_then(Value::as_str) {
            text.push_str(&format!("\nWhat the user can do: {what}"));
        }
        if let Some(details) = error.get("details") {
            text.push_str(&format!("\n{}", pretty(details)));
        }
        let mut r = CallToolResult::error(vec![ContentBlock::text(text)]);
        r.structured_content = Some(json!({ "error": error }));
        r
    }
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}

fn schema(v: Option<&Value>) -> Arc<Map<String, Value>> {
    let mut map = v.and_then(Value::as_object).cloned().unwrap_or_default();
    // Clients only need the shape; the draft marker and the title confuse some.
    map.remove("$schema");
    map.remove("title");
    map.entry("type").or_insert(json!("object"));
    // Every action that asks the user can take back a pending approval.
    if let Some(Value::Object(props)) = map.get_mut("properties") {
        props.insert(
            "confirmation_id".into(),
            json!({ "type": "string", "description": "Only to pick up a request still waiting for the user (from a CONFIRMATION_PENDING answer)." }),
        );
    }
    Arc::new(map)
}

fn tool_for(a: &Value) -> Option<Tool> {
    let name = a.get("tool")?.as_str()?.to_owned();
    let description = a.get("description").and_then(Value::as_str).unwrap_or("").to_owned();
    let mut tool = Tool::new(Cow::Owned(name), Cow::Owned(description), schema(a.get("input_schema")));
    tool.title = a.get("title").and_then(Value::as_str).map(str::to_owned);
    let flag = |k: &str| a.get(k).and_then(Value::as_bool);
    tool.annotations = Some(ToolAnnotations::from_raw(
        tool.title.clone(),
        flag("read_only"),
        flag("destructive"),
        flag("idempotent"),
        flag("open_world"),
    ));
    Some(tool)
}

fn dispatch_tools() -> Vec<Tool> {
    let search = json!({ "type": "object", "properties": {
        "query": { "type": "string", "description": "Words to look for, like 'text message' or 'notifications'." },
        "toolset": { "type": "string" } } });
    let describe = json!({ "type": "object", "required": ["id"], "properties": {
        "id": { "type": "string", "description": "An action id from actions_search, like sms.send." } } });
    let run = json!({ "type": "object", "required": ["id"], "properties": {
        "id": { "type": "string" },
        "args": { "type": "object", "description": "The action's arguments, as action_describe shows them." } } });
    let make = |name: &'static str, desc: &'static str, s: Value, read_only: bool| {
        let mut t = Tool::new(
            Cow::Borrowed(name),
            Cow::Borrowed(desc),
            Arc::new(s.as_object().cloned().unwrap_or_default()),
        );
        t.annotations = Some(ToolAnnotations::from_raw(
            None,
            Some(read_only),
            Some(!read_only),
            Some(false),
            Some(!read_only),
        ));
        t
    };
    vec![
        make(
            "actions_search",
            "Lists what you may do on the user's phone and PC through NectarLink. Start here.",
            search,
            true,
        ),
        make("action_describe", "Shows one action's arguments.", describe, true),
        make(
            "action_run",
            "Runs an action through NectarLink. Sensitive ones wait for the user to approve them.",
            run,
            false,
        ),
    ]
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("nectarlink", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let tools = match self.mode {
            Mode::Dispatch => dispatch_tools(),
            Mode::Tools => self.actions().await.iter().filter_map(tool_for).collect(),
        };
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let args = request.arguments.map(Value::Object).unwrap_or(Value::Null);
        let result = match (self.mode, request.name.as_ref()) {
            (Mode::Dispatch, "actions_search") => {
                let query = args.get("query").and_then(Value::as_str).unwrap_or("").to_lowercase();
                let toolset = args.get("toolset").and_then(Value::as_str);
                let found: Vec<Value> = self
                    .actions()
                    .await
                    .into_iter()
                    .filter(|a| toolset.is_none_or(|t| a.get("toolset").and_then(Value::as_str) == Some(t)))
                    .filter(|a| {
                        query.is_empty()
                            || ["id", "title", "description"].iter().any(|k| {
                                a.get(*k).and_then(Value::as_str).is_some_and(|s| s.to_lowercase().contains(&query))
                            })
                    })
                    .map(|a| json!({ "id": a["id"], "title": a["title"], "description": a["description"], "confirm": a["confirm"] }))
                    .collect();
                to_result(json!({ "ok": true, "result": { "actions": found } }))
            }
            (Mode::Dispatch, "action_describe") => {
                let id = args.get("id").and_then(Value::as_str).unwrap_or("");
                match self.actions().await.into_iter().find(|a| a["id"] == id || a["tool"] == id) {
                    Some(a) => to_result(json!({ "ok": true, "result": a })),
                    None => to_result(
                        json!({ "ok": false, "error": { "code": "UNKNOWN_ACTION", "message": "No such action for this agent. Use actions_search." } }),
                    ),
                }
            }
            (Mode::Dispatch, "action_run") => {
                let id = args.get("id").and_then(Value::as_str).unwrap_or("").to_owned();
                self.run(&id, args.get("args").cloned().unwrap_or(Value::Null)).await
            }
            (_, name) => self.run(name, args).await,
        };
        Ok(result.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_answers_become_tool_results() {
        let ok = to_result(json!({ "ok": true, "result": { "sent": true } }));
        assert_eq!(ok.is_error, Some(false));
        assert_eq!(ok.structured_content, Some(json!({ "sent": true })));

        let denied = to_result(json!({ "ok": false, "error": {
            "code": "CONFIRM_DENIED", "message": "The user said no.", "user_action": "Nothing to do." } }));
        assert_eq!(denied.is_error, Some(true));
        let text = format!("{:?}", denied.content);
        assert!(text.contains("CONFIRM_DENIED") && text.contains("Nothing to do."), "{text}");
    }

    #[test]
    fn tools_carry_the_actions_schema_and_hints() {
        let a = json!({ "id": "sms.send", "tool": "sms_send", "title": "Send a text message",
            "description": "Sends an SMS.", "read_only": false, "destructive": true, "idempotent": false, "open_world": true,
            "input_schema": { "$schema": "x", "title": "SmsSendArgs", "type": "object", "properties": { "body": { "type": "string" } } } });
        let t = tool_for(&a).unwrap();
        assert_eq!(t.name, "sms_send");
        assert!(!t.input_schema.contains_key("$schema"));
        assert!(t.input_schema["properties"].get("confirmation_id").is_some());
        let ann = t.annotations.unwrap();
        assert_eq!(ann.destructive_hint, Some(true));
        assert_eq!(ann.read_only_hint, Some(false));
    }

    #[test]
    fn dispatch_mode_has_three_tools() {
        let names: Vec<_> = dispatch_tools().into_iter().map(|t| t.name.into_owned()).collect();
        assert_eq!(names, ["actions_search", "action_describe", "action_run"]);
    }
}
