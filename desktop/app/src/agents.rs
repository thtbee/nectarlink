// SPDX-License-Identifier: GPL-3.0-or-later
//! AI agents (docs/agents/README.md): hosts the gateway from
//! `nectarlink-actions`, serves it on `127.0.0.1` to `nectarlink mcp` and
//! `nectarlink do`, keeps each agent's token file, and holds the
//! confirmations waiting for the user.
//!
//! Off by default. Nothing listens and no token works until the user turns
//! agent access on in Settings.

use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

use nectarlink_actions::{
    ActionDef, ActionError, ActionHost, Caller, Client, Code, ConfirmRequest, Decision, Gateway, Grant,
    Store, Switches, host::BoxFuture, store::label_ok,
};
use nectarlink_core::{DeviceId, MediaPlayer, NodeEvent, Notification, PowerAction};
use serde_json::{Value, json};
use tokio::{net::TcpListener, sync::oneshot};

use crate::{
    core_host,
    local_api::{HttpRequest, bearer_token, read_http_request, write_json_response},
    settings::Settings,
    state::Changes,
};

/// Where agents find NectarLink on this PC, next to `local_api`'s 53320.
pub const DEFAULT_PORT: u16 = 53321;
/// How long reading a request may take.
const READ_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a call may take, confirmation included.
const CALL_TIMEOUT: Duration = Duration::from_secs(60);

struct Agents {
    gateway: Arc<Gateway>,
    server: Mutex<Option<Server>>,
    pending: Arc<Mutex<Vec<Pending>>>,
}

struct Server {
    port: u16,
    _stop: oneshot::Sender<()>,
}

/// A confirmation on screen, waiting for the user.
struct Pending {
    request: ConfirmRequest,
    answer: oneshot::Sender<Decision>,
}

static AGENTS: OnceLock<Option<Agents>> = OnceLock::new();

fn agents() -> Option<&'static Agents> {
    AGENTS
        .get_or_init(|| {
            let dir = agents_dir(&core_host::host().data_dir);
            if let Err(e) = make_private_dir(&dir) {
                tracing::warn!("can't make the agents folder: {e}");
            }
            let store = match Store::open(&dir.join("agents.db")) {
                Ok(s) => Arc::new(s),
                Err(e) => {
                    tracing::warn!("agent access unavailable: {e}");
                    return None;
                }
            };
            upgrade_presets(&store);
            let pending = Arc::new(Mutex::new(Vec::new()));
            let host = Arc::new(DesktopHost { pending: pending.clone() });
            Some(Agents { gateway: Arc::new(Gateway::new(host, store)), server: Mutex::new(None), pending })
        })
        .as_ref()
}

/// Agents saved with a preset from before new permissions existed get the
/// preset as it is now, so "Read only" still means every read.
fn upgrade_presets(store: &Store) {
    for mut c in store.clients().unwrap_or_default() {
        if let Some(grants) = nectarlink_actions::upgrade_preset(&c.grants) {
            c.grants = grants;
            if let Err(e) = store.update_client(&c) {
                tracing::warn!("can't update agent {}: {e}", c.label);
            }
        }
    }
}

/// Keeps what agents may follow with `events.wait`, while agent access is on.
pub fn on_event(event: &NodeEvent) {
    if let Some(a) = agents().filter(|a| a.gateway.switches().enabled) {
        a.gateway.events().push(event);
    }
}

/// `<data_dir>/agents`: the database, token files and `server.json`.
pub fn agents_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("agents")
}

fn make_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub fn init(settings: &Settings) {
    apply(settings);
}

pub fn sync_from_settings() {
    apply(&Settings::load(&core_host::host().data_dir));
}

pub fn stop() {
    if let Some(a) = agents() {
        *lock(&a.server) = None;
        deny_all(a);
    }
    let _ = std::fs::remove_file(agents_dir(&core_host::host().data_dir).join("server.json"));
}

fn apply(settings: &Settings) {
    let Some(a) = agents() else { return };
    a.gateway.set_switches(Switches {
        enabled: settings.agent_access_enabled,
        read_only: settings.agent_read_only,
    });
    if !settings.agent_access_enabled {
        stop();
        return;
    }
    if lock(&a.server).is_some() {
        return;
    }
    let gateway = a.gateway.clone();
    let dir = agents_dir(&core_host::host().data_dir);
    core_host::spawn(async move {
        match serve(DEFAULT_PORT, gateway).await {
            Ok((addr, server)) => {
                if let Some(a) = agents() {
                    *lock(&a.server) = Some(server);
                }
                if let Err(e) = write_server_file(&dir, addr.port()) {
                    tracing::warn!("can't write agents/server.json: {e}");
                }
                tracing::info!(%addr, "agent access listening");
            }
            Err(e) => tracing::warn!(error = %e, "can't start agent access"),
        }
    });
}

fn write_server_file(dir: &Path, port: u16) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = crate::settings::create_private_file(&dir.join("server.json"))?;
    f.write_all(json!({ "port": port, "pid": std::process::id() }).to_string().as_bytes())
}

// ---- The loopback server ----

async fn serve(port: u16, gateway: Arc<Gateway>) -> std::io::Result<(SocketAddr, Server)> {
    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    let addr = listener.local_addr()?;
    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut stop_rx => break,
                accepted = listener.accept() => {
                    let Ok((mut stream, _)) = accepted else { continue };
                    let gateway = gateway.clone();
                    let port = addr.port();
                    tokio::spawn(async move {
                        let Ok(Ok(Some(req))) = tokio::time::timeout(READ_TIMEOUT, read_http_request(&mut stream)).await else {
                            return;
                        };
                        let (status, reason, body) = match tokio::time::timeout(CALL_TIMEOUT, handle(&gateway, port, req)).await {
                            Ok(r) => r,
                            Err(_) => (504, "Gateway Timeout", ActionError::new(Code::Timeout, "It took too long.").to_json()),
                        };
                        let _ = write_json_response(&mut stream, status, reason, &body).await;
                    });
                }
            }
        }
    });
    Ok((addr, Server { port: addr.port(), _stop: stop_tx }))
}

async fn handle(gateway: &Gateway, port: u16, req: HttpRequest) -> (u16, &'static str, Value) {
    // Browsers send Origin; agents and scripts don't. A web page must never get in.
    if req.headers.contains_key("origin") {
        return (
            403,
            "Forbidden",
            ActionError::new(Code::Denied, "Requests from web pages aren't allowed.").to_json(),
        );
    }
    // DNS rebinding: the Host must be this machine and this port.
    let host_ok = req
        .headers
        .get("host")
        .is_some_and(|h| h == &format!("127.0.0.1:{port}") || h == &format!("localhost:{port}"));
    if !host_ok {
        return (403, "Forbidden", ActionError::new(Code::Denied, "Wrong Host header.").to_json());
    }
    let token =
        req.headers.get("authorization").map(String::as_str).and_then(bearer_token).unwrap_or("").to_owned();
    // Right after start the node may still be coming up: wait a little for it.
    let node = match core_host::node() {
        Some(n) => Some(n),
        None => {
            tokio::time::timeout(Duration::from_secs(10), core_host::wait_for_node()).await.ok().flatten()
        }
    };
    gateway.set_node(node.map(Arc::new));
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/v2/status") => match gateway.authenticate(&token) {
            Ok(c) => (200, "OK", json!({ "ok": true, "result": status(gateway, &c) })),
            Err(e) => error_response(e),
        },
        ("GET", "/v2/actions") => match gateway.authenticate(&token) {
            Ok(c) => {
                let list: Vec<Value> = gateway.visible(&c).into_iter().map(describe).collect();
                (200, "OK", json!({ "ok": true, "result": { "actions": list } }))
            }
            Err(e) => error_response(e),
        },
        ("GET", path) if path.starts_with("/v2/actions/") => match gateway.authenticate(&token) {
            Ok(c) => {
                let id = &path["/v2/actions/".len()..];
                match gateway.visible(&c).into_iter().find(|a| a.id == id || a.tool_name() == id) {
                    Some(a) => (200, "OK", json!({ "ok": true, "result": describe(a) })),
                    None => error_response(ActionError::new(
                        Code::UnknownAction,
                        "No such action for this agent.",
                    )),
                }
            }
            Err(e) => error_response(e),
        },
        // Long polling for events: `?since=12&wait=25&kinds=sms.changed,call.changed`.
        ("GET", "/v2/events") => {
            let mut args = json!({});
            for (key, name) in [("since", "since"), ("wait", "wait_s"), ("limit", "limit")] {
                if let Some(v) = req.query.get(key) {
                    match v.parse::<u64>() {
                        Ok(n) => args[name] = json!(n),
                        Err(_) => {
                            return error_response(ActionError::invalid(format!("{key} must be a number.")));
                        }
                    }
                }
            }
            if let Some(kinds) = req.query.get("kinds").filter(|k| !k.is_empty()) {
                args["kinds"] = json!(kinds.split(',').map(str::trim).collect::<Vec<_>>());
            }
            match gateway.invoke(&Caller { token, channel: "http".into() }, "events.wait", args).await {
                Ok(result) => (200, "OK", json!({ "ok": true, "result": result })),
                Err(e) => error_response(e),
            }
        }
        ("POST", path) if path.starts_with("/v2/actions/") => {
            let id = path["/v2/actions/".len()..].to_owned();
            let args = if req.body.is_empty() {
                Value::Null
            } else {
                match serde_json::from_slice::<Value>(&req.body) {
                    Ok(v) => v,
                    Err(_) => return error_response(ActionError::invalid("The body isn't JSON.")),
                }
            };
            let channel = req.headers.get("x-nectarlink-channel").map(String::as_str).unwrap_or("http");
            let channel = if matches!(channel, "mcp-stdio" | "cli") { channel } else { "http" }.to_owned();
            match gateway.invoke(&Caller { token, channel }, &id, args).await {
                Ok(result) => (200, "OK", json!({ "ok": true, "result": result })),
                Err(e) => error_response(e),
            }
        }
        _ => (404, "Not Found", ActionError::new(Code::UnknownAction, "Unknown path.").to_json()),
    }
}

fn error_response(e: ActionError) -> (u16, &'static str, Value) {
    let (status, reason) = match e.code {
        Code::Unauthenticated => (401, "Unauthorized"),
        Code::AgentAccessOff
        | Code::ClientPaused
        | Code::ScopeDenied
        | Code::ReadOnly
        | Code::DeviceNotAllowed
        | Code::Denied
        | Code::ConfirmDenied => (403, "Forbidden"),
        Code::UnknownAction | Code::NotFound | Code::DeviceNotFound => (404, "Not Found"),
        Code::DeviceOffline | Code::DeviceAmbiguous | Code::Unsupported | Code::AppNotReady => {
            (409, "Conflict")
        }
        Code::InvalidArguments => (422, "Unprocessable Entity"),
        Code::RateLimited => (429, "Too Many Requests"),
        Code::ConfirmationPending => (202, "Accepted"),
        Code::Timeout | Code::ConfirmTimeout => (504, "Gateway Timeout"),
        Code::AuditUnavailable | Code::Internal => (500, "Internal Server Error"),
    };
    (status, reason, e.to_json())
}

fn status(gateway: &Gateway, c: &Client) -> Value {
    let switches = gateway.switches();
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "agent": c.label,
        "name": c.display_name,
        "grants": c.grants_text(),
        "read_only": switches.read_only || c.read_only,
        "app_ready": core_host::node().is_some(),
    })
}

/// An action as agents see it: enough to call it well.
pub fn describe(a: &ActionDef) -> Value {
    json!({
        "id": a.id,
        "tool": a.tool_name(),
        "title": a.title,
        "description": a.description,
        "toolset": a.toolset.as_str(),
        "scope": a.grant().to_text(),
        "confirm": a.confirm.as_str(),
        "read_only": a.read_only_hint(),
        "destructive": a.destructive_hint(),
        "idempotent": a.idempotent,
        "open_world": a.open_world,
        "input_schema": (a.input)(),
    })
}

// ---- The host: what only the app knows ----

#[derive(Debug)]
struct DesktopHost {
    pending: Arc<Mutex<Vec<Pending>>>,
}

impl std::fmt::Debug for Pending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pending").field("request", &self.request.id).finish()
    }
}

impl ActionHost for DesktopHost {
    fn confirm(&self, request: ConfirmRequest, expires_s: u64) -> BoxFuture<Decision> {
        let (tx, rx) = oneshot::channel();
        let id = request.id.clone();
        let title = request.title.clone();
        let client = request.client.clone();
        lock(&self.pending).push(Pending { request, answer: tx });
        core_host::host().hub.changed(Changes::REMOTE);
        notify_user(&client, &title);
        let pending = self.pending.clone();
        Box::pin(async move {
            let answer = tokio::time::timeout(Duration::from_secs(expires_s), rx).await;
            let decision = match answer {
                Ok(Ok(d)) => d,
                _ => Decision::Expired,
            };
            let before = lock(&pending).len();
            lock(&pending).retain(|p| p.request.id != id);
            if lock(&pending).len() != before {
                core_host::host().hub.changed(Changes::REMOTE);
            }
            decision
        })
    }

    fn notifications(&self) -> Vec<(DeviceId, Notification)> {
        core_host::host()
            .hub
            .read(|s| s.notifications.iter().map(|n| (n.device, n.notification.clone())).collect())
    }

    fn media_players(&self) -> Vec<(Option<DeviceId>, MediaPlayer)> {
        core_host::host().hub.read(|s| s.media.iter().map(|p| (Some(p.device), p.player.clone())).collect())
    }

    fn pc_clipboard_read(&self) -> Result<String, String> {
        Ok(crate::local_api::read_pc_clipboard())
    }

    fn pc_clipboard_write(&self, text: &str) -> Result<(), String> {
        crate::local_api::write_pc_clipboard(text)
    }

    fn pc_lock(&self) -> Result<(), String> {
        crate::win::shell::lock()
    }

    fn transfers(&self) -> Vec<Value> {
        core_host::host().hub.read(|s| {
            s.transfers
                .iter()
                .map(|t| {
                    let t = &t.transfer;
                    json!({
                        "id": t.id,
                        "device": t.device.to_string(),
                        "direction": format!("{:?}", t.direction).to_lowercase(),
                        "names": t.names.iter().take(20).map(|n| nectarlink_actions::untrusted::clean(n, 200)).collect::<Vec<_>>(),
                        "files": t.files,
                        "bytes_total": t.total,
                        "bytes_done": t.done,
                        "state": format!("{:?}", t.state).split(['(', ' ', '{']).next().unwrap_or("").to_lowercase(),
                    })
                })
                .collect()
        })
    }

    fn save_dir(&self) -> Option<PathBuf> {
        Some(core_host::downloads_dir())
    }

    fn pc_power(&self, action: PowerAction) -> Result<(), String> {
        match action {
            PowerAction::Lock => crate::win::shell::lock(),
            PowerAction::Sleep => crate::win::shell::sleep(),
            PowerAction::Hibernate => crate::win::shell::hibernate(),
            PowerAction::Shutdown => crate::win::shell::shutdown(),
        }
    }

    fn mirror_open(&self, device: DeviceId, app: Option<(String, String)>) -> Result<(), String> {
        match app {
            Some((pkg, label)) => crate::mirror::start_app(device, pkg, label),
            None => crate::mirror::start(device),
        }
        Ok(())
    }

    fn mirror_windows(&self) -> Vec<Value> {
        crate::mirror::windows()
            .into_iter()
            .filter(|(_, shown)| !matches!(shown.phase, crate::mirror::Phase::Ended(_)))
            .map(|(w, shown)| {
                json!({
                    "device": w.device.to_string(),
                    "app": shown.pkg,
                    "app_name": shown.app.as_deref().map(|a| nectarlink_actions::untrusted::clean(a, 80)),
                    "showing": matches!(shown.phase, crate::mirror::Phase::Showing),
                })
            })
            .collect()
    }

    fn mirror_close(&self, device: DeviceId, app: Option<&str>) -> Result<bool, String> {
        let open = crate::mirror::windows().into_iter().find(|(w, shown)| {
            w.device == device
                && shown.pkg.as_deref() == app
                && !matches!(shown.phase, crate::mirror::Phase::Ended(_))
        });
        match open {
            Some((window, _)) => {
                crate::mirror::stop(window);
                Ok(true)
            }
            None => Ok(false),
        }
    }

    fn deck_tiles(&self) -> Vec<Value> {
        crate::deck::config()
            .pages
            .iter()
            .flat_map(|page| page.tiles.iter().map(move |t| (page, t)))
            .map(|(page, t)| {
                let action = serde_json::to_value(&t.action).unwrap_or_default();
                let kind = action.get("kind").and_then(Value::as_str).unwrap_or("").to_owned();
                // Where a tile goes is shown; what it types or runs stays private.
                let target = match kind.as_str() {
                    "open_url" => action.get("url").cloned(),
                    "launch_app" => action.get("path").cloned(),
                    "shortcut" => Some(json!({ "key": action.get("key"), "mods": action.get("mods") })),
                    _ => None,
                };
                json!({
                    "id": t.id,
                    "page": nectarlink_actions::untrusted::clean(&page.name, 80),
                    "label": nectarlink_actions::untrusted::clean(&t.label, 80),
                    "does": kind,
                    "target": target,
                })
            })
            .collect()
    }

    fn deck_run(&self, tile: &str) -> Result<(), String> {
        if !crate::deck::config().pages.iter().any(|p| p.tiles.iter().any(|t| t.id == tile)) {
            return Err("There's no Deck tile with that id. See deck_tiles.".into());
        }
        crate::deck::execute_tile(tile)
    }

    fn calls(&self) -> Vec<(DeviceId, Value)> {
        crate::calls::all()
            .into_iter()
            .map(|(d, c, ringing)| {
                (
                    d,
                    json!({
                        "id": c.id,
                        "state": c.state,
                        "incoming": c.incoming,
                        "ringing": ringing,
                        "number": c.number.as_deref().map(|n| nectarlink_actions::untrusted::clean(n, 40)),
                        "name": c.name.as_deref().map(|n| nectarlink_actions::untrusted::clean(n, 80)),
                    }),
                )
            })
            .collect()
    }
}

/// A toast, so the user notices a request while the window is hidden.
fn notify_user(client: &str, title: &str) {
    crate::win::toast::show(crate::win::toast::Toast {
        device: String::new(),
        key: "agent-confirm".into(),
        title: crate::i18n::tr!("Agents", "%1 is asking", client),
        body: crate::i18n::tr!("Agents", "%1. Open NectarLink to allow or deny it.", title),
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

// ---- For Settings and the confirmation window ----

/// The confirmation on screen, if any.
pub fn current_confirmation() -> Option<ConfirmRequest> {
    agents().and_then(|a| lock(&a.pending).first().map(|p| p.request.clone()))
}

/// The user's answer to confirmation `id`.
pub fn answer(id: &str, decision: Decision) {
    let Some(a) = agents() else { return };
    let taken = {
        let mut pending = lock(&a.pending);
        pending.iter().position(|p| p.request.id == id).map(|i| pending.remove(i))
    };
    if let Some(p) = taken {
        let _ = p.answer.send(decision);
    }
    core_host::host().hub.changed(Changes::REMOTE);
}

fn deny_all(a: &Agents) {
    let all: Vec<Pending> = lock(&a.pending).drain(..).collect();
    for p in all {
        let _ = p.answer.send(Decision::Deny);
    }
}

pub fn clients() -> Vec<Client> {
    agents().and_then(|a| a.gateway.store().clients().ok()).unwrap_or_default()
}

/// Adds an agent with `grants` and writes its token file. Returns the client.
pub fn add_client(
    label: &str,
    display_name: &str,
    kind: &str,
    grants: &std::collections::BTreeSet<Grant>,
) -> Result<Client, String> {
    let a = agents().ok_or("agent access isn't available")?;
    if !label_ok(label) {
        return Err("invalid name".into());
    }
    let (client, token) =
        a.gateway.store().create_client(label, display_name, kind, grants).map_err(|e| e.to_string())?;
    if let Err(e) = write_token_file(label, &token) {
        let _ = a.gateway.store().delete_client(&client.id);
        return Err(e.to_string());
    }
    core_host::host().hub.changed(Changes::PREFS);
    Ok(client)
}

fn write_token_file(label: &str, token: &str) -> std::io::Result<()> {
    use std::io::Write;
    let dir = agents_dir(&core_host::host().data_dir);
    make_private_dir(&dir)?;
    let mut f = crate::settings::create_private_file(&dir.join(format!("{label}.token")))?;
    f.write_all(token.as_bytes())
}

pub fn remove_client(id: &str) {
    let Some(a) = agents() else { return };
    if let Ok(Some(c)) = a.gateway.store().client(id) {
        let _ = a.gateway.store().delete_client(id);
        let _ =
            std::fs::remove_file(agents_dir(&core_host::host().data_dir).join(format!("{}.token", c.label)));
    }
    core_host::host().hub.changed(Changes::PREFS);
}

/// Changes a client's settings with `f`.
pub fn update_client(id: &str, f: impl FnOnce(&mut Client)) {
    let Some(a) = agents() else { return };
    if let Ok(Some(mut c)) = a.gateway.store().client(id) {
        f(&mut c);
        let _ = a.gateway.store().update_client(&c);
    }
    core_host::host().hub.changed(Changes::PREFS);
}

pub fn activity(limit: usize) -> Vec<nectarlink_actions::Activity> {
    agents().and_then(|a| a.gateway.store().activity(None, limit).ok()).unwrap_or_default()
}

pub fn clear_activity() {
    if let Some(a) = agents() {
        let _ = a.gateway.store().clear_activity();
    }
    core_host::host().hub.changed(Changes::PREFS);
}

/// The port agents connect to, while agent access is on.
pub fn port() -> Option<u16> {
    agents().and_then(|a| lock(&a.server).as_ref().map(|s| s.port))
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ---- One-click setup ----

/// How the last "Add to <app>" went: the app, and the file it went into or
/// why it didn't.
static SETUP: Mutex<Option<(String, Result<String, String>)>> = Mutex::new(None);

pub fn setup_status() -> Option<(String, Result<String, String>)> {
    lock(&SETUP).clone()
}

pub fn clear_setup_status() {
    *lock(&SETUP) = None;
    core_host::host().hub.changed(Changes::PREFS);
}

/// Whether NectarLink can add itself to `kind` on this PC.
pub fn can_set_up(kind: &str) -> bool {
    if kind == "claude-code" {
        return claude_command().is_some();
    }
    let Some(dirs) = nectarlink_actions::setup::Dirs::of_this_system() else { return false };
    nectarlink_actions::setup::app(kind).is_some_and(|a| a.present(&dirs))
}

/// Adds agent `label` to the app `kind`, in the background; the result shows
/// up in [`setup_status`].
pub fn set_up(kind: &str, label: &str, exe: &str) {
    if !label_ok(label) {
        return;
    }
    *lock(&SETUP) = None;
    let (kind, label, exe) = (kind.to_owned(), label.to_owned(), exe.to_owned());
    std::thread::spawn(move || {
        let result = set_up_now(&kind, &label, &exe);
        if let Err(e) = &result {
            tracing::warn!(kind, "agent setup failed: {e}");
        }
        *lock(&SETUP) = Some((kind, result));
        core_host::host().hub.changed(Changes::PREFS);
    });
}

fn set_up_now(kind: &str, label: &str, exe: &str) -> Result<String, String> {
    use nectarlink_actions::setup::{self, SetupError};
    if kind == "claude-code" {
        return claude_code_add(label, exe);
    }
    let dirs = setup::Dirs::of_this_system()
        .ok_or_else(|| crate::i18n::tr!("Agents", "Your user folders couldn't be found."))?;
    let app =
        setup::app(kind).ok_or_else(|| crate::i18n::tr!("Agents", "This app has to be set up by hand."))?;
    match app.install(&dirs, exe, label) {
        Ok(path) => Ok(path.to_string_lossy().into_owned()),
        Err(SetupError::NotInstalled) => {
            Err(crate::i18n::tr!("Agents", "The app doesn't seem to be installed on this PC."))
        }
        Err(SetupError::NotPlainJson) => Err(crate::i18n::tr!(
            "Agents",
            "Its settings file has comments or something NectarLink can't safely change. Paste the lines below into it instead."
        )),
        Err(SetupError::Io(e)) => Err(e),
    }
}

/// The `claude` command, if Claude Code is installed.
fn claude_command() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) { &["claude.exe", "claude.cmd"] } else { &["claude"] };
    let mut dirs: Vec<PathBuf> =
        std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    // Claude Code's own installer puts it here, which a GUI app's PATH may lack.
    if let Some(home) = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }) {
        dirs.push(PathBuf::from(home).join(".local").join("bin"));
    }
    dirs.iter().flat_map(|d| names.iter().map(move |n| d.join(n))).find(|p| p.is_file())
}

fn claude_code_add(label: &str, exe: &str) -> Result<String, String> {
    let claude = claude_command()
        .ok_or_else(|| crate::i18n::tr!("Agents", "Claude Code isn't installed on this PC."))?;
    let run = |args: &[String]| -> std::io::Result<std::process::Output> {
        let mut cmd = std::process::Command::new(&claude);
        cmd.args(args).stdin(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        cmd.output()
    };
    // An older NectarLink entry would make `add` fail.
    let remove: Vec<String> =
        ["mcp", "remove", "--scope", "user", nectarlink_actions::setup::SERVER].map(str::to_owned).to_vec();
    let _ = run(&remove);
    let out = run(&nectarlink_actions::setup::claude_code_args(exe, label)).map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok("Claude Code".to_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err.lines().find(|l| !l.trim().is_empty()).unwrap_or("claude mcp add failed").trim().to_owned())
    }
}
