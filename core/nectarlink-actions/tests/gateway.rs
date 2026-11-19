// SPDX-License-Identifier: MPL-2.0
//! The gateway's checks, with a fake host and actions that need no node.

use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
    time::Duration,
};

use nectarlink_actions::{
    Access, ActionDef, ActionHost, Caller, Code, ConfirmRequest, ConfirmRule, Decision, Gateway, Grant, Risk,
    Scope, Store, Switches, Toolset,
    def::{RateClass, Target},
    host::BoxFuture,
};
use nectarlink_core::DeviceId;
use nectarlink_protocol::messages::{MediaPlayer, Notification};
use serde_json::{Value, json};

#[derive(Debug, Default)]
struct FakeHost {
    answers: Mutex<Vec<Decision>>,
    asked: Mutex<Vec<ConfirmRequest>>,
    /// Never answers (the user walked away).
    silent: bool,
}

impl ActionHost for FakeHost {
    fn confirm(&self, request: ConfirmRequest, _expires_s: u64) -> BoxFuture<Decision> {
        self.asked.lock().unwrap().push(request);
        if self.silent {
            return Box::pin(std::future::pending());
        }
        let answer = self.answers.lock().unwrap().pop().unwrap_or(Decision::Deny);
        Box::pin(async move { answer })
    }
    fn notifications(&self) -> Vec<(DeviceId, Notification)> {
        Vec::new()
    }
    fn media_players(&self) -> Vec<(Option<DeviceId>, MediaPlayer)> {
        Vec::new()
    }
    fn pc_clipboard_read(&self) -> Result<String, String> {
        Ok(String::new())
    }
    fn pc_clipboard_write(&self, _text: &str) -> Result<(), String> {
        Ok(())
    }
    fn pc_lock(&self) -> Result<(), String> {
        Ok(())
    }
    fn transfers(&self) -> Vec<Value> {
        Vec::new()
    }
    fn calls(&self) -> Vec<(DeviceId, Value)> {
        Vec::new()
    }
}

fn echo(_ctx: nectarlink_actions::Ctx, args: Value) -> nectarlink_actions::def::HandlerFuture {
    Box::pin(async move { Ok(json!({ "echo": args })) })
}

fn schema() -> Value {
    json!({ "type": "object" })
}

const fn def(
    id: &'static str,
    access: Access,
    confirm: ConfirmRule,
    rate: RateClass,
    tainted: bool,
) -> ActionDef {
    ActionDef {
        id,
        title: "Test action",
        description: "Test.",
        toolset: Toolset::Messages,
        scope: Scope::Messages,
        access,
        risk: Risk::Low,
        confirm,
        rate,
        target: Target::None,
        tainted,
        idempotent: false,
        open_world: false,
        input: schema,
        output: schema,
        run: echo,
    }
}

static TEST_ACTIONS: &[ActionDef] = &[
    def("t.read", Access::Read, ConfirmRule::Never, RateClass::Read, true),
    def("t.write", Access::Write, ConfirmRule::Never, RateClass::Write, false),
    def("t.first", Access::Write, ConfirmRule::FirstUse, RateClass::Write, false),
    def("t.always", Access::Write, ConfirmRule::Always, RateClass::Write, false),
    def("t.sensitive", Access::Write, ConfirmRule::Never, RateClass::Sensitive, false),
];

struct Setup {
    gw: Gateway,
    host: Arc<FakeHost>,
    caller: Caller,
    client_id: String,
}

fn setup(host: FakeHost, grants: &[(Scope, Access)]) -> Setup {
    let store = Arc::new(Store::in_memory().unwrap());
    let grants: BTreeSet<Grant> = grants.iter().map(|(s, a)| Grant::new(*s, *a)).collect();
    let (client, token) = store.create_client("test-agent", "Test agent", "cli", &grants).unwrap();
    let host = Arc::new(host);
    let gw = Gateway::with_actions(host.clone(), store, TEST_ACTIONS);
    gw.set_switches(Switches { enabled: true, read_only: false });
    Setup { gw, host, caller: Caller { token, channel: "cli".into() }, client_id: client.id }
}

async fn code(s: &Setup, action: &str, args: Value) -> Code {
    s.gw.invoke(&s.caller, action, args).await.expect_err("should fail").code
}

#[tokio::test]
async fn nothing_works_while_agent_access_is_off_or_with_a_wrong_token() {
    let s = setup(FakeHost::default(), &[(Scope::Messages, Access::Write)]);
    s.gw.set_switches(Switches { enabled: false, read_only: false });
    assert_eq!(code(&s, "t.read", json!({})).await, Code::AgentAccessOff);
    s.gw.set_switches(Switches { enabled: true, read_only: false });
    let stranger = Caller { token: "0".repeat(64), channel: "http".into() };
    assert_eq!(s.gw.invoke(&stranger, "t.read", json!({})).await.unwrap_err().code, Code::Unauthenticated);
    assert_eq!(code(&s, "t.nope", json!({})).await, Code::UnknownAction);
    assert!(s.gw.invoke(&s.caller, "t.read", json!({})).await.is_ok());
    // Tool names work as well as ids.
    assert!(s.gw.invoke(&s.caller, "t_read", json!({})).await.is_ok());
}

#[tokio::test]
async fn scopes_and_read_only_stop_writes() {
    let s = setup(FakeHost::default(), &[(Scope::Messages, Access::Read)]);
    assert!(s.gw.invoke(&s.caller, "t.read", json!({})).await.is_ok());
    let err = s.gw.invoke(&s.caller, "t.write", json!({})).await.unwrap_err();
    assert_eq!(err.code, Code::ScopeDenied);
    assert!(err.user_action.is_some());

    let s = setup(FakeHost::default(), &[(Scope::Messages, Access::Write)]);
    s.gw.set_switches(Switches { enabled: true, read_only: true });
    assert_eq!(code(&s, "t.write", json!({})).await, Code::ReadOnly);
    assert!(s.gw.invoke(&s.caller, "t.read", json!({})).await.is_ok(), "reads still work");
    // Read-only also hides writes from the list.
    let client = s.gw.store().client(&s.client_id).unwrap().unwrap();
    assert!(s.gw.visible(&client).iter().all(|a| a.access == Access::Read));
}

#[tokio::test]
async fn always_asks_every_time_and_a_no_is_final() {
    let host = FakeHost::default();
    host.answers.lock().unwrap().extend([Decision::Deny, Decision::Allow]);
    let s = setup(host, &[(Scope::Messages, Access::Write)]);
    // Answers are popped from the end: Allow first.
    let out = s.gw.invoke(&s.caller, "t.always", json!({ "body": "hi" })).await.unwrap();
    assert_eq!(out["echo"]["body"], "hi");
    assert_eq!(code(&s, "t.always", json!({ "body": "hi" })).await, Code::ConfirmDenied);
    let asked = s.host.asked.lock().unwrap();
    assert_eq!(asked.len(), 2, "asked both times");
    assert_eq!(
        asked[0].details,
        vec![("body".to_owned(), "hi".to_owned())],
        "the window shows the arguments"
    );
    assert!(!asked[0].can_remember);
}

#[tokio::test]
async fn first_use_can_be_remembered() {
    let host = FakeHost::default();
    host.answers.lock().unwrap().push(Decision::AllowAlways);
    let s = setup(host, &[(Scope::Messages, Access::Write)]);
    s.gw.invoke(&s.caller, "t.first", json!({})).await.unwrap();
    s.gw.invoke(&s.caller, "t.first", json!({})).await.unwrap();
    assert_eq!(s.host.asked.lock().unwrap().len(), 1, "not asked again");
    assert!(s.host.asked.lock().unwrap()[0].can_remember);
}

#[tokio::test]
async fn three_denials_pause_the_agent() {
    let s = setup(FakeHost::default(), &[(Scope::Messages, Access::Write)]);
    for _ in 0..3 {
        assert_eq!(code(&s, "t.always", json!({})).await, Code::ConfirmDenied);
    }
    assert_eq!(code(&s, "t.read", json!({})).await, Code::ClientPaused);
}

#[tokio::test(start_paused = true)]
async fn an_unanswered_request_can_be_picked_up_only_with_the_same_arguments() {
    let host = FakeHost { silent: true, ..Default::default() };
    let s = setup(host, &[(Scope::Messages, Access::Write)]);
    let err = s.gw.invoke(&s.caller, "t.always", json!({ "body": "hi" })).await.unwrap_err();
    assert_eq!(err.code, Code::ConfirmationPending);
    let id = err.details.unwrap()["confirmation_id"].as_str().unwrap().to_owned();
    // Different arguments under the same approval are refused.
    let err =
        s.gw.invoke(&s.caller, "t.always", json!({ "body": "other", "confirmation_id": id }))
            .await
            .unwrap_err();
    assert_eq!(err.code, Code::InvalidArguments);
    // The same ones wait again, and it's still pending (nobody answered).
    let err =
        s.gw.invoke(&s.caller, "t.always", json!({ "body": "hi", "confirmation_id": id })).await.unwrap_err();
    assert_eq!(err.code, Code::ConfirmationPending);
    assert_eq!(s.host.asked.lock().unwrap().len(), 1, "asked once");
    let _ = Duration::ZERO;
}

#[tokio::test]
async fn sensitive_calls_are_rate_limited() {
    let s = setup(FakeHost::default(), &[(Scope::Messages, Access::Write)]);
    for _ in 0..5 {
        s.gw.invoke(&s.caller, "t.sensitive", json!({})).await.unwrap();
    }
    let err = s.gw.invoke(&s.caller, "t.sensitive", json!({})).await.unwrap_err();
    assert_eq!(err.code, Code::RateLimited);
    assert!(err.details.unwrap()["retry_after_ms"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn outside_text_is_flagged_and_everything_is_logged_without_content() {
    let s = setup(FakeHost::default(), &[(Scope::Messages, Access::Read)]);
    let out = s.gw.invoke(&s.caller, "t.read", json!({ "body": "a private message" })).await.unwrap();
    assert!(out["_notice"].as_str().unwrap().contains("data"));
    let _ = s.gw.invoke(&s.caller, "t.write", json!({})).await;
    let log = s.gw.store().activity(None, 10).unwrap();
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].decision, "scope_denied");
    assert_eq!(log[1].decision, "ok");
    assert_eq!(log[1].summary, "body=17 chars");
    assert!(!log.iter().any(|a| a.summary.contains("private")));
}
