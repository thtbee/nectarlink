// SPDX-License-Identifier: GPL-3.0-or-later
//! Talks to the running app's agent gateway on `127.0.0.1`. Finds the port in
//! `<app data>/agents/server.json` and the agent's token in
//! `<app data>/agents/<agent>.token`, or `NECTARLINK_AGENT_TOKEN`.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

/// Longest a call may take, a confirmation included.
const CALL_TIMEOUT: Duration = Duration::from_secs(65);
const MAX_RESPONSE: usize = 8 * 1024 * 1024;

#[derive(Debug)]
pub enum ClientError {
    /// The app isn't running, or agent access is off (no server file).
    AppNotRunning,
    /// No token for this agent.
    UnknownAgent(String),
    Io(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::AppNotRunning => write!(
                f,
                "NectarLink isn't running on this PC, or AI agent access is off (Settings > AI agents)."
            ),
            ClientError::UnknownAgent(a) => write!(
                f,
                "There's no agent named {a:?} in NectarLink. Add it in NectarLink: Settings > AI agents."
            ),
            ClientError::Io(e) => write!(f, "Couldn't reach NectarLink: {e}"),
        }
    }
}

impl std::error::Error for ClientError {}

impl ClientError {
    /// The same shape the gateway answers with, so agents see one kind of error.
    pub fn to_json(&self) -> Value {
        let code = match self {
            ClientError::AppNotRunning => "APP_NOT_RUNNING",
            ClientError::UnknownAgent(_) => "UNAUTHENTICATED",
            ClientError::Io(_) => "APP_NOT_RUNNING",
        };
        json!({ "ok": false, "error": { "code": code, "message": self.to_string(), "retryable": matches!(self, ClientError::AppNotRunning | ClientError::Io(_)) } })
    }
}

/// The desktop app's data folder: `NECTARLINK_APP_DIR`, else the default.
pub fn app_dir() -> Option<PathBuf> {
    std::env::var_os("NECTARLINK_APP_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::data_local_dir().map(|d| d.join("Nectarlink")))
}

#[derive(Debug, Clone)]
pub struct AppClient {
    port: u16,
    token: String,
    /// `mcp-stdio` or `cli`, for the activity log.
    channel: String,
}

impl AppClient {
    /// Finds the app and the agent's token.
    pub fn connect(app_dir: &Path, agent: &str, channel: &str) -> Result<AppClient, ClientError> {
        let dir = app_dir.join("agents");
        let server: Value = std::fs::read_to_string(dir.join("server.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .ok_or(ClientError::AppNotRunning)?;
        let port = server
            .get("port")
            .and_then(Value::as_u64)
            .and_then(|p| u16::try_from(p).ok())
            .ok_or(ClientError::AppNotRunning)?;
        let token = match std::env::var("NECTARLINK_AGENT_TOKEN") {
            Ok(t) if !t.trim().is_empty() => t.trim().to_owned(),
            _ => {
                let valid =
                    !agent.is_empty() && agent.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
                if !valid {
                    return Err(ClientError::UnknownAgent(agent.to_owned()));
                }
                std::fs::read_to_string(dir.join(format!("{agent}.token")))
                    .map(|t| t.trim().to_owned())
                    .map_err(|_| ClientError::UnknownAgent(agent.to_owned()))?
            }
        };
        Ok(AppClient { port, token, channel: channel.to_owned() })
    }

    /// For tests: a client for a given port and token.
    pub fn with(port: u16, token: &str, channel: &str) -> AppClient {
        AppClient { port, token: token.to_owned(), channel: channel.to_owned() }
    }

    /// The actions this agent may use, as the app describes them.
    pub async fn actions(&self) -> Result<Vec<Value>, Value> {
        let reply = self.request("GET", "/v2/actions", None).await?;
        match reply.get("result").and_then(|r| r.get("actions")).and_then(Value::as_array) {
            Some(list) => Ok(list.clone()),
            None => Err(reply),
        }
    }

    pub async fn status(&self) -> Result<Value, Value> {
        self.request("GET", "/v2/status", None).await
    }

    /// Runs an action. `Ok` is the app's whole answer, `{ ok, result }` or
    /// `{ ok: false, error }`; `Err` means the app couldn't be reached.
    pub async fn call(&self, action: &str, args: Value) -> Result<Value, Value> {
        let safe =
            !action.is_empty() && action.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_');
        if !safe {
            return Ok(
                json!({ "ok": false, "error": { "code": "UNKNOWN_ACTION", "message": "That isn't an action name.", "retryable": false } }),
            );
        }
        self.request("POST", &format!("/v2/actions/{action}"), Some(args)).await
    }

    async fn request(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value, Value> {
        let run = async {
            let mut stream = TcpStream::connect(("127.0.0.1", self.port)).await?;
            let body = body.map(|b| b.to_string()).unwrap_or_default();
            let head = format!(
                "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\nX-Nectarlink-Channel: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                self.port,
                self.token,
                self.channel,
                body.len()
            );
            stream.write_all(head.as_bytes()).await?;
            stream.write_all(body.as_bytes()).await?;
            stream.flush().await?;
            let mut buf = Vec::new();
            let mut chunk = [0u8; 16 * 1024];
            loop {
                let n = stream.read(&mut chunk).await?;
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > MAX_RESPONSE {
                    return Err(std::io::Error::other("answer too large"));
                }
            }
            Ok::<_, std::io::Error>(buf)
        };
        let raw = match tokio::time::timeout(CALL_TIMEOUT, run).await {
            Ok(Ok(raw)) => raw,
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
                return Err(ClientError::AppNotRunning.to_json());
            }
            Ok(Err(e)) => return Err(ClientError::Io(e.to_string()).to_json()),
            Err(_) => return Err(ClientError::Io("no answer in time".into()).to_json()),
        };
        let split = raw.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4).unwrap_or(raw.len());
        serde_json::from_slice::<Value>(&raw[split..])
            .map_err(|_| ClientError::Io("the answer wasn't JSON".into()).to_json())
    }
}
