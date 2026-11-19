// SPDX-License-Identifier: MPL-2.0
//! The gateway: every agent request, from any channel, passes through
//! [`Gateway::invoke`] and nothing else reaches the node for an agent.
//!
//! In order: agent access on, the token, the action, the scope, read-only,
//! the device and its permission, the rate limit, the user's confirmation,
//! then the handler, and an activity row whatever happened.

use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, Mutex, RwLock},
    time::{Duration, Instant},
};

use nectarlink_core::{DeviceId, Node};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::watch;

use crate::{
    def::{Access, ActionDef, ConfirmRule, Grant, Scope, Target},
    error::{ActionError, Code},
    events::EventLog,
    host::{ActionHost, ConfirmRequest, Decision},
    ratelimit::RateLimiter,
    registry,
    store::{Activity, Client, Store, now},
    untrusted,
};

/// How long a call waits for the user before answering "pending".
pub const CONFIRM_WAIT: Duration = Duration::from_secs(45);
/// How long a confirmation stays open in the window.
pub const CONFIRM_EXPIRES_S: u64 = 120;
/// How long a handler may run.
const RUN_TIMEOUT: Duration = Duration::from_secs(60);
/// Denials in this window pause the client.
const DENIAL_WINDOW: Duration = Duration::from_secs(600);
const DENIALS_TO_PAUSE: usize = 3;

/// Who is calling, and over what.
#[derive(Debug, Clone)]
pub struct Caller {
    pub token: String,
    /// `mcp-stdio`, `cli` or `http`.
    pub channel: String,
}

/// What a handler gets.
#[derive(Debug, Clone)]
pub struct Ctx {
    node: Option<Arc<Node>>,
    pub host: Arc<dyn ActionHost>,
    /// The device the call is for, when the action takes one.
    pub device: Option<DeviceId>,
    /// The client may see one-time codes and sensitive notifications.
    pub show_codes: bool,
    /// What the client may do, for actions that cover several areas.
    pub grants: BTreeSet<Grant>,
    /// The devices the client may use, `None` for all.
    pub devices: Option<BTreeSet<String>>,
    pub events: Arc<EventLog>,
}

impl Ctx {
    /// For tests and other hosts: a context with no node, every device and
    /// the given grants.
    pub fn for_test(host: Arc<dyn ActionHost>, device: Option<DeviceId>, grants: BTreeSet<Grant>) -> Ctx {
        Ctx {
            node: None,
            host,
            device,
            show_codes: false,
            grants,
            devices: None,
            events: Arc::new(EventLog::new()),
        }
    }

    /// Whether the client holds `grant` (write includes read).
    pub fn holds(&self, grant: Grant) -> bool {
        self.grants
            .iter()
            .any(|g| g.scope == grant.scope && (g.access == grant.access || g.access == Access::Write))
    }

    /// Whether something from `device` may be shown to this client for
    /// `scope`: the client may use the device, and the device's own
    /// permission for it is on. For actions that list across devices.
    pub fn may_see(&self, device: DeviceId, scope: Scope) -> bool {
        if self.devices.as_ref().is_some_and(|set| !set.contains(&device.to_string())) {
            return false;
        }
        let (Some(node), Some(toggle)) = (self.node.as_ref(), scope.device_toggle()) else { return true };
        match node.device_toggles(device) {
            Ok(toggles) => !toggles.iter().any(|(name, on)| *name == toggle && !*on),
            Err(_) => false,
        }
    }

    pub fn node(&self) -> Result<&Arc<Node>, ActionError> {
        self.node.as_ref().ok_or_else(|| ActionError::new(Code::AppNotReady, "NectarLink is still starting."))
    }

    pub fn device(&self) -> Result<DeviceId, ActionError> {
        self.device.ok_or_else(|| ActionError::invalid("This action needs a device."))
    }
}

/// Settings the user changes in the app.
#[derive(Debug, Clone, Copy, Default)]
pub struct Switches {
    /// Agent access as a whole.
    pub enabled: bool,
    /// Every agent is read-only.
    pub read_only: bool,
}

#[derive(Debug)]
struct Pending {
    client_id: String,
    action: &'static str,
    args_hash: String,
    decision: watch::Receiver<Option<Decision>>,
}

#[derive(Debug)]
pub struct Gateway {
    node: RwLock<Option<Arc<Node>>>,
    host: Arc<dyn ActionHost>,
    store: Arc<Store>,
    switches: RwLock<Switches>,
    limits: Mutex<RateLimiter>,
    pending: Mutex<HashMap<String, Pending>>,
    denials: Mutex<HashMap<String, Vec<Instant>>>,
    actions: &'static [ActionDef],
    events: Arc<EventLog>,
}

impl Gateway {
    pub fn new(host: Arc<dyn ActionHost>, store: Arc<Store>) -> Gateway {
        Gateway::with_actions(host, store, registry::all())
    }

    /// With a given action list (tests).
    pub fn with_actions(
        host: Arc<dyn ActionHost>,
        store: Arc<Store>,
        actions: &'static [ActionDef],
    ) -> Gateway {
        Gateway {
            node: RwLock::new(None),
            host,
            store,
            switches: RwLock::new(Switches::default()),
            limits: Mutex::new(RateLimiter::default()),
            pending: Mutex::new(HashMap::new()),
            denials: Mutex::new(HashMap::new()),
            actions,
            events: Arc::new(EventLog::new()),
        }
    }

    /// The node, once the app has started it.
    pub fn set_node(&self, node: Option<Arc<Node>>) {
        *self.node.write().unwrap_or_else(|e| e.into_inner()) = node;
    }

    pub fn set_switches(&self, switches: Switches) {
        *self.switches.write().unwrap_or_else(|e| e.into_inner()) = switches;
        if !switches.enabled {
            lock(&self.pending).clear();
        }
    }

    pub fn switches(&self) -> Switches {
        *self.switches.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    /// Where the app feeds core events for `events.wait`.
    pub fn events(&self) -> &Arc<EventLog> {
        &self.events
    }

    /// The actions a client may see: its scopes, not hidden by read-only.
    pub fn visible(&self, client: &Client) -> Vec<&'static ActionDef> {
        let read_only = self.switches().read_only || client.read_only;
        self.actions
            .iter()
            .filter(|a| client.holds(a.grant()))
            .filter(|a| !(read_only && a.access == Access::Write))
            .filter(|a| client.toolsets.is_empty() || client.toolsets.contains(&a.toolset))
            .collect()
    }

    /// The client a token belongs to, if agent access is on and it's not paused.
    pub fn authenticate(&self, token: &str) -> Result<Client, ActionError> {
        if !self.switches().enabled {
            return Err(access_off());
        }
        let client =
            self.store.client_by_token(token).map_err(|e| ActionError::internal(e.to_string()))?.ok_or_else(
                || {
                    ActionError::new(Code::Unauthenticated, "This agent isn't known to NectarLink.")
                        .user_action("Add the agent in NectarLink: Settings > AI agents.")
                },
            )?;
        if client.paused {
            return Err(ActionError::new(Code::ClientPaused, "This agent is paused in NectarLink.")
                .user_action("Resume it in NectarLink: Settings > AI agents."));
        }
        Ok(client)
    }

    /// Runs one action for a caller. The result is the action's output, or
    /// an error every channel turns into the same JSON.
    pub async fn invoke(&self, caller: &Caller, action_id: &str, args: Value) -> Result<Value, ActionError> {
        let started = Instant::now();
        let client = self.authenticate(&caller.token)?;
        let mut args = match args {
            Value::Null => Value::Object(Map::new()),
            Value::Object(m) => Value::Object(m),
            _ => return Err(ActionError::invalid("Arguments must be a JSON object.")),
        };
        let confirmation_id = args
            .as_object_mut()
            .and_then(|m| m.remove("confirmation_id"))
            .and_then(|v| v.as_str().map(str::to_owned));
        let Some(def) = self.actions.iter().find(|a| a.id == action_id || a.tool_name() == action_id) else {
            let err =
                ActionError::new(Code::UnknownAction, format!("There's no action named {action_id:?}."));
            let record = Record {
                client: &client,
                channel: &caller.channel,
                action: untrusted::clean(action_id, 60),
                device: None,
                summary: String::new(),
                confirm: "none",
                started,
            };
            let outcome = Err(err);
            self.audit(&record, &outcome);
            return outcome;
        };
        let mut record = Record {
            client: &client,
            channel: &caller.channel,
            action: def.id.to_owned(),
            device: None,
            summary: summarize(&args),
            confirm: "none",
            started,
        };
        let outcome = self.run(&client, def, args, confirmation_id, &mut record).await;
        self.audit(&record, &outcome);
        self.store.touch(&client.id);
        outcome
    }

    async fn run(
        &self,
        client: &Client,
        def: &'static ActionDef,
        args: Value,
        confirmation_id: Option<String>,
        record: &mut Record<'_>,
    ) -> Result<Value, ActionError> {
        if !client.holds(def.grant()) {
            return Err(ActionError::new(
                Code::ScopeDenied,
                format!("This agent doesn't have the {} permission.", def.grant().to_text()),
            )
            .user_action(format!(
                "In NectarLink, Settings > AI agents, give this agent {} if you want it to do this.",
                def.grant().to_text()
            ))
            .details(json!({ "scope": def.grant().to_text() })));
        }
        if def.access == Access::Write && (self.switches().read_only || client.read_only) {
            return Err(ActionError::new(Code::ReadOnly, "Agents are read-only in NectarLink right now.")
                .user_action("Turn off read-only in NectarLink: Settings > AI agents."));
        }
        let node = self.node.read().unwrap_or_else(|e| e.into_inner()).clone();
        let mut device_name = None;
        let named = args.get("device").and_then(Value::as_str).is_some_and(|d| !d.trim().is_empty());
        let device = match def.target {
            Target::None => None,
            Target::OptionalDevice if !named => None,
            Target::Device | Target::OptionalDevice => {
                let node = node
                    .as_ref()
                    .ok_or_else(|| ActionError::new(Code::AppNotReady, "NectarLink is still starting."))?;
                let wanted = args.get("device").and_then(Value::as_str).map(str::to_owned);
                let (id, name) = resolve_device(node, client, wanted.as_deref())?;
                record.device = Some(name.clone());
                device_name = Some(name);
                check_toggle(node, id, def.scope)?;
                Some(id)
            }
        };
        let class = def.rate;
        if let Err(wait) = lock(&self.limits).check(&client.id, class, Instant::now()) {
            return Err(ActionError::new(
                Code::RateLimited,
                "Too many requests from this agent. Wait a little.",
            )
            .details(json!({ "retry_after_ms": wait.as_millis() as u64 })));
        }
        record.confirm = self.confirm(client, def, &args, device_name.as_deref(), confirmation_id).await?;
        let ctx = Ctx {
            node,
            host: self.host.clone(),
            device,
            show_codes: client.holds(Grant::new(Scope::NotificationsSensitive, Access::Read)),
            grants: client.grants.clone(),
            devices: client.devices.clone(),
            events: self.events.clone(),
        };
        let out = tokio::time::timeout(RUN_TIMEOUT, (def.run)(ctx, args))
            .await
            .map_err(|_| ActionError::new(Code::Timeout, "It took too long and was stopped."))??;
        Ok(wrap(def, out))
    }

    /// Asks the user when the rule says so. Returns how it was confirmed.
    async fn confirm(
        &self,
        client: &Client,
        def: &'static ActionDef,
        args: &Value,
        device: Option<&str>,
        confirmation_id: Option<String>,
    ) -> Result<&'static str, ActionError> {
        match def.confirm {
            ConfirmRule::Never => return Ok("none"),
            ConfirmRule::FirstUse if client.remembered.contains(def.id) => return Ok("remembered"),
            _ => {}
        }
        let hash = args_hash(def.id, args);
        let mut decision = match confirmation_id {
            Some(id) => {
                let pending = lock(&self.pending);
                let p = pending.get(&id).ok_or_else(|| {
                    ActionError::new(
                        Code::NotFound,
                        "That confirmation is unknown or has expired. Ask again.",
                    )
                })?;
                if p.client_id != client.id || p.action != def.id || p.args_hash != hash {
                    return Err(ActionError::invalid(
                        "That confirmation was for a different request. Send the same arguments, or ask again without it.",
                    ));
                }
                (id, p.decision.clone())
            }
            None => {
                let id = format!("cf_{}", data_encoding::HEXLOWER.encode(&rand::random::<[u8; 8]>()));
                let request = ConfirmRequest {
                    id: id.clone(),
                    client: client.display_name.clone(),
                    action: def.id.to_owned(),
                    title: def.title.to_owned(),
                    device: device.map(str::to_owned),
                    details: details(args),
                    can_remember: def.confirm == ConfirmRule::FirstUse,
                };
                let (tx, rx) = watch::channel(None);
                let answer = self.host.confirm(request, CONFIRM_EXPIRES_S);
                tokio::spawn(async move {
                    let _ = tx.send(Some(answer.await));
                });
                lock(&self.pending).insert(
                    id.clone(),
                    Pending {
                        client_id: client.id.clone(),
                        action: def.id,
                        args_hash: hash,
                        decision: rx.clone(),
                    },
                );
                (id, rx)
            }
        };
        let answered = tokio::time::timeout(CONFIRM_WAIT, decision.1.wait_for(Option::is_some)).await;
        let value = match answered {
            Ok(Ok(v)) => *v,
            Ok(Err(_)) => Some(Decision::Expired),
            Err(_) => None,
        };
        let id = decision.0;
        match value {
            None => Err(ActionError::new(
                Code::ConfirmationPending,
                "Waiting for the user to approve this in NectarLink. Tell them to look at the NectarLink window.",
            )
            .details(json!({ "confirmation_id": id, "expires_in_s": CONFIRM_EXPIRES_S }))),
            Some(answer) => {
                lock(&self.pending).remove(&id);
                match answer {
                    Decision::Allow => Ok("pc"),
                    Decision::AllowAlways => {
                        if def.confirm == ConfirmRule::FirstUse
                            && let Ok(Some(mut c)) = self.store.client(&client.id)
                        {
                            c.remembered.insert(def.id.to_owned());
                            let _ = self.store.update_client(&c);
                        }
                        Ok("pc")
                    }
                    Decision::Deny => {
                        self.denied(client);
                        Err(ActionError::new(
                            Code::ConfirmDenied,
                            "The user said no. Don't try again or rephrase it; ask the user what they want instead.",
                        ))
                    }
                    Decision::Expired => Err(ActionError::new(
                        Code::ConfirmTimeout,
                        "Nobody approved it in time, so it didn't happen.",
                    )),
                }
            }
        }
    }

    /// Counts a denial; three in ten minutes pause the client.
    fn denied(&self, client: &Client) {
        let now = Instant::now();
        let mut denials = lock(&self.denials);
        let list = denials.entry(client.id.clone()).or_default();
        list.retain(|t| now.duration_since(*t) < DENIAL_WINDOW);
        list.push(now);
        if list.len() >= DENIALS_TO_PAUSE {
            list.clear();
            if let Ok(Some(mut c)) = self.store.client(&client.id) {
                c.paused = true;
                let _ = self.store.update_client(&c);
                tracing::warn!(client = %c.label, "agent paused after repeated denials");
            }
        }
    }

    fn audit(&self, r: &Record<'_>, outcome: &Result<Value, ActionError>) {
        let (decision, code) = match outcome {
            Ok(_) => ("ok".to_owned(), None),
            Err(e) => (decision_of(e.code).to_owned(), Some(e.code.as_str())),
        };
        let row = Activity {
            id: 0,
            ts: now(),
            client_id: r.client.id.clone(),
            client_name: r.client.display_name.clone(),
            channel: r.channel.to_owned(),
            action: r.action.clone(),
            device: r.device.clone(),
            summary: r.summary.clone(),
            decision,
            confirm: r.confirm.to_owned(),
            error_code: code,
            duration_ms: r.started.elapsed().as_millis() as i64,
        };
        if let Err(e) = self.store.log(&row) {
            tracing::warn!("agent activity not saved: {e}");
        }
    }
}

struct Record<'a> {
    client: &'a Client,
    channel: &'a str,
    action: String,
    device: Option<String>,
    summary: String,
    confirm: &'static str,
    started: Instant,
}

fn access_off() -> ActionError {
    ActionError::new(Code::AgentAccessOff, "AI agent access is turned off in NectarLink.")
        .user_action("Turn it on in NectarLink: Settings > AI agents.")
}

fn decision_of(code: Code) -> &'static str {
    match code {
        Code::ScopeDenied => "scope_denied",
        Code::ReadOnly => "read_only",
        Code::Denied => "toggle_denied",
        Code::DeviceNotAllowed => "device_denied",
        Code::RateLimited => "rate_limited",
        Code::ConfirmDenied => "confirm_denied",
        Code::ConfirmTimeout => "confirm_timeout",
        Code::ConfirmationPending => "confirm_pending",
        _ => "error",
    }
}

/// The paired device a call means: by id, id prefix or name, among those
/// the client may use. With none named, the only one there is.
fn resolve_device(
    node: &Node,
    client: &Client,
    wanted: Option<&str>,
) -> Result<(DeviceId, String), ActionError> {
    let devices = node.paired_devices().map_err(ActionError::from)?;
    let allowed: Vec<_> = devices
        .into_iter()
        .filter(|d| client.devices.as_ref().is_none_or(|set| set.contains(&d.id.to_string())))
        .collect();
    let pick = match wanted {
        Some(w) if !w.is_empty() => {
            let lower = w.to_lowercase();
            let matches: Vec<_> = allowed
                .iter()
                .filter(|d| {
                    let id = d.id.to_string();
                    id == w || (w.len() >= 6 && id.starts_with(w)) || d.info.name.to_lowercase() == lower
                })
                .collect();
            match matches.as_slice() {
                [one] => (*one).clone(),
                [] => {
                    return Err(ActionError::new(
                        Code::DeviceNotFound,
                        format!("No device this agent may use matches {w:?}."),
                    )
                    .details(json!({ "devices": names(&allowed) })));
                }
                _ => {
                    return Err(ActionError::new(
                        Code::DeviceAmbiguous,
                        "More than one device matches. Pass its id.",
                    )
                    .details(json!({ "devices": names(&allowed) })));
                }
            }
        }
        _ => match allowed.as_slice() {
            [one] => one.clone(),
            [] => {
                return Err(ActionError::new(
                    Code::DeviceNotAllowed,
                    "This agent may not use any paired device.",
                ));
            }
            _ => {
                return Err(ActionError::new(
                    Code::DeviceAmbiguous,
                    "Several devices are paired. Say which one with `device`.",
                )
                .details(json!({ "devices": names(&allowed) })));
            }
        },
    };
    Ok((pick.id, pick.info.name))
}

fn names(devices: &[nectarlink_core::PairedDevice]) -> Vec<Value> {
    devices
        .iter()
        .map(|d| json!({ "id": d.id.to_string(), "name": untrusted::clean(&d.info.name, 80) }))
        .collect()
}

/// A scope never beats a device's own permission.
fn check_toggle(node: &Node, device: DeviceId, scope: Scope) -> Result<(), ActionError> {
    let Some(toggle) = scope.device_toggle() else { return Ok(()) };
    let toggles = node.device_toggles(device).map_err(ActionError::from)?;
    if toggles.iter().any(|(name, on)| *name == toggle && !*on) {
        return Err(ActionError::new(
            Code::Denied,
            format!("{toggle} is turned off for this device in NectarLink."),
        )
        .user_action(format!("Turn on {toggle} for this device in NectarLink's device permissions.")));
    }
    Ok(())
}

/// Adds the untrusted notice to outputs that carry outside text.
fn wrap(def: &ActionDef, mut out: Value) -> Value {
    if def.tainted
        && let Value::Object(m) = &mut out
    {
        m.insert("_notice".into(), Value::String(untrusted::NOTICE.into()));
    }
    out
}

/// A stable hash of the action and its arguments, so an approval covers
/// exactly what the user saw.
fn args_hash(action: &str, args: &Value) -> String {
    let mut h = Sha256::new();
    h.update(action.as_bytes());
    h.update(b"\0");
    h.update(canonical(args).as_bytes());
    data_encoding::HEXLOWER.encode(&h.finalize())
}

/// JSON with object keys sorted.
fn canonical(v: &Value) -> String {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<_> = m.keys().collect();
            keys.sort();
            let inner: Vec<String> = keys
                .iter()
                .map(|k| format!("{}:{}", Value::String((*k).clone()), canonical(&m[*k])))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
        Value::Array(a) => format!("[{}]", a.iter().map(canonical).collect::<Vec<_>>().join(",")),
        other => other.to_string(),
    }
}

/// The arguments as label and value pairs for the confirmation window, in full.
fn details(args: &Value) -> Vec<(String, String)> {
    let Value::Object(m) = args else { return Vec::new() };
    m.iter()
        .filter(|(k, _)| k.as_str() != "device")
        .map(|(k, v)| {
            let value = match v {
                Value::String(s) => untrusted::clean(s, 20_000),
                Value::Array(a) => a
                    .iter()
                    .map(|x| x.as_str().map(|s| untrusted::clean(s, 2_000)).unwrap_or_else(|| x.to_string()))
                    .collect::<Vec<_>>()
                    .join("\n"),
                other => other.to_string(),
            };
            (k.replace('_', " "), value)
        })
        .collect()
}

/// Who and what for the activity log, never message content.
fn summarize(args: &Value) -> String {
    const KEEP: [&str; 8] = ["to", "number", "paths", "url", "action", "app", "command", "kind"];
    let Value::Object(m) = args else { return String::new() };
    m.iter()
        .filter(|(k, _)| k.as_str() != "device")
        .map(|(k, v)| {
            let shown = match v {
                Value::String(s) if KEEP.contains(&k.as_str()) => untrusted::clean(s, 80),
                Value::String(s) => format!("{} chars", s.chars().count()),
                Value::Array(a) if KEEP.contains(&k.as_str()) => a
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|s| untrusted::clean(s, 80))
                    .collect::<Vec<_>>()
                    .join(" "),
                Value::Array(a) => format!("{} items", a.len()),
                Value::Object(_) => "{…}".into(),
                other => other.to_string(),
            };
            format!("{k}={shown}")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_args_hash_ignores_key_order_but_not_values() {
        let a = json!({ "to": ["1"], "body": "hi" });
        let b = json!({ "body": "hi", "to": ["1"] });
        let c = json!({ "body": "hi!", "to": ["1"] });
        assert_eq!(args_hash("sms.send", &a), args_hash("sms.send", &b));
        assert_ne!(args_hash("sms.send", &a), args_hash("sms.send", &c));
        assert_ne!(args_hash("sms.send", &a), args_hash("chat.reply", &a));
    }

    #[test]
    fn the_log_keeps_who_and_what_but_not_the_message() {
        let s = summarize(&json!({ "to": ["+15550100"], "body": "secret plans", "device": "Pixel" }));
        assert!(s.contains("to=+15550100"), "{s}");
        assert!(s.contains("body=12 chars"), "{s}");
        assert!(!s.contains("secret"), "{s}");
        assert!(!s.contains("Pixel"), "{s}");
    }

    #[test]
    fn the_confirmation_shows_everything_but_the_device() {
        let d = details(&json!({ "body": "Running late", "to": ["+1", "+2"], "device": "x" }));
        assert!(d.contains(&("body".into(), "Running late".into())));
        assert!(d.contains(&("to".into(), "+1\n+2".into())));
        assert!(d.iter().all(|(k, _)| k != "device"));
    }
}
