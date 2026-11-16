// SPDX-License-Identifier: GPL-3.0-or-later
//! One XDG portal session for sharing a monitor and, when enabled, controlling it.

use std::{
    collections::HashMap,
    future::poll_fn,
    os::fd::OwnedFd,
    pin::Pin,
    task::{Context as TaskContext, Poll, Waker},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use zbus::{
    Connection, MatchRule, MessageStream,
    export::futures_core::Stream,
    message::Type,
    zvariant::{OwnedObjectPath, OwnedValue, Value},
};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const DESKTOP: &str = "/org/freedesktop/portal/desktop";
const SCREEN_CAST: &str = "org.freedesktop.portal.ScreenCast";
const REMOTE_DESKTOP: &str = "org.freedesktop.portal.RemoteDesktop";
const REQUEST: &str = "org.freedesktop.portal.Request";
const SESSION: &str = "org.freedesktop.portal.Session";
const INPUT_DEVICES: u32 = 3; // Keyboard and pointer.
const TIMEOUT: Duration = Duration::from_secs(20);
const CLOSE_TIMEOUT: Duration = Duration::from_millis(500);

type Options<'a> = HashMap<&'static str, Value<'a>>;
type Response = HashMap<String, OwnedValue>;

pub(crate) struct ScreenPortal {
    conn: Connection,
    path: OwnedObjectPath,
    closed: MessageStream,
    pub(crate) remote: OwnedFd,
    pub(crate) node_id: u32,
    pub(crate) size: (u32, u32),
    pub(crate) input: bool,
}

impl ScreenPortal {
    pub(crate) async fn open(
        input: bool,
        token_dir: &std::path::Path,
        cancelled: impl Fn() -> bool,
    ) -> Result<Self> {
        // Restore tokens only work once. A replacement waits for the cancelled
        // request to release its session before trying to use the token again.
        static NEGOTIATING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

        let timeout = tokio::time::sleep(TIMEOUT);
        tokio::pin!(timeout);

        let _negotiating = tokio::select! {
            biased;
            () = wait_for_cancel(&cancelled) => bail!("Wayland screen sharing was cancelled"),
            () = &mut timeout => bail!("Timed out waiting for the Wayland screen sharing portal"),
            guard = NEGOTIATING.lock() => guard,
        };
        let mut pending = None;
        let result = tokio::select! {
            biased;
            () = wait_for_cancel(&cancelled) => Err(anyhow::anyhow!("Wayland screen sharing was cancelled")),
            () = &mut timeout => Err(anyhow::anyhow!("Timed out waiting for the Wayland screen sharing portal")),
            result = async {
                pending.insert(PortalRequest::connect().await?).start(input, token_dir).await
            } => result,
        };
        if let Some(pending) = pending
            && result.is_err()
        {
            tracing::debug!(method = pending.method, "closing unfinished Wayland portal request");
            pending.close().await;
        }
        result
    }

    pub(crate) fn is_closed(&mut self) -> bool {
        !matches!(
            Pin::new(&mut self.closed).poll_next(&mut TaskContext::from_waker(Waker::noop())),
            Poll::Pending
        )
    }

    pub(crate) fn input_portal(&self) -> super::input::RemoteDesktopPortal {
        super::input::RemoteDesktopPortal::for_screencast(
            self.conn.clone().into(),
            self.path.clone(),
            self.node_id,
            self.size,
        )
    }
}

impl Drop for ScreenPortal {
    fn drop(&mut self) {
        let conn = self.conn.clone();
        let path = self.path.clone();
        crate::core_host::spawn(async move { close(&conn, SESSION, &path).await });
    }
}

struct PortalRequest {
    conn: Connection,
    responses: MessageStream,
    sender: String,
    session: Option<OwnedObjectPath>,
    request: Option<OwnedObjectPath>,
    method: &'static str,
}

impl PortalRequest {
    async fn connect() -> Result<Self> {
        let conn = zbus::connection::Builder::session()?
            .method_timeout(Duration::from_secs(3))
            .build()
            .await
            .context("connect to the Wayland screen sharing portal")?;
        let sender = conn.unique_name().context("missing D-Bus client name")?[1..].replace('.', "_");
        // Listen before issuing any portal call, including when a restore token
        // lets the portal answer immediately.
        let rule = MatchRule::builder()
            .msg_type(Type::Signal)
            .sender(PORTAL)?
            .interface(REQUEST)?
            .member("Response")?
            .build();
        let responses = MessageStream::for_match_rule(rule, &conn, None).await?;
        Ok(Self { conn, responses, sender, session: None, request: None, method: "" })
    }

    async fn start(&mut self, input: bool, token_dir: &std::path::Path) -> Result<ScreenPortal> {
        let input = input && self.input_available().await;
        let interface = if input { REMOTE_DESKTOP } else { SCREEN_CAST };
        let token_path =
            token_dir.join(if input { "remote-desktop-restore-token" } else { "screencast-restore-token" });
        let restore = read_restore_token(&token_path);
        let mut persist = Options::from([("persist_mode", Value::from(2u32))]);
        if let Some(token) = restore.as_deref() {
            persist.insert("restore_token", Value::from(token));
        }

        // The predicted paths also let cancellation close a request whose
        // method call has been sent but whose handle has yet to arrive.
        self.session = Some(self.path("session", "nectarlink_screen")?);
        let results = self
            .request(interface, "CreateSession", "create", |mut opts| {
                opts.insert("session_handle_token", Value::from("nectarlink_screen"));
                (opts,)
            })
            .await?;
        // session_handle was originally declared as a string by mistake.
        let handle = results.get("session_handle").context("missing portal session handle")?;
        let path = <&str>::try_from(handle)
            .or_else(|_| <&zbus::zvariant::ObjectPath<'_>>::try_from(handle).map(|p| p.as_str()))?;
        let path = OwnedObjectPath::try_from(path)?;
        self.session = Some(path.clone());

        let rule = MatchRule::builder()
            .msg_type(Type::Signal)
            .sender(PORTAL)?
            .interface(SESSION)?
            .member("Closed")?
            .path(path.as_str())?
            .build();
        let closed = MessageStream::for_match_rule(rule, &self.conn, None).await?;
        let mut sources = Options::from([
            ("types", Value::from(1u32)),       // Monitor.
            ("cursor_mode", Value::from(2u32)), // Embedded.
            ("multiple", Value::from(false)),
        ]);
        if input {
            persist.insert("types", Value::from(INPUT_DEVICES));
            self.request(REMOTE_DESKTOP, "SelectDevices", "input", |mut opts| {
                opts.extend(persist);
                (&path, opts)
            })
            .await?;
        } else {
            sources.extend(persist);
        }
        self.request(SCREEN_CAST, "SelectSources", "sources", |mut opts| {
            opts.extend(sources);
            (&path, opts)
        })
        .await?;
        let results = self.request(interface, "Start", "start", |opts| (&path, "", opts)).await?;
        if let Some(token) = results.get("restore_token").and_then(|v| <&str>::try_from(v).ok())
            && let Err(error) = save_restore_token(&token_path, token)
        {
            tracing::warn!(%error, "can't save the Wayland screen sharing permission");
        }
        let input = input
            && results.get("devices").and_then(|v| u32::try_from(v).ok()).unwrap_or(0) & INPUT_DEVICES
                == INPUT_DEVICES;
        let streams = results
            .into_iter()
            .find_map(|(key, value)| (key == "streams").then_some(value))
            .context("missing PipeWire streams in the portal response")?;
        let streams: Vec<(u32, Response)> = streams.try_into()?;
        let (node_id, props) =
            streams.into_iter().next().context("the portal returned no screen to share")?;
        let size = props
            .get("size")
            .and_then(|v| <(i32, i32)>::try_from(v.try_clone().ok()?).ok())
            .filter(|&(w, h)| w > 0 && h > 0)
            .map(|(w, h)| (w as u32, h as u32))
            .unwrap_or((1920, 1080));

        self.method = "OpenPipeWireRemote";
        let reply = self
            .conn
            .call_method(
                Some(PORTAL),
                DESKTOP,
                Some(SCREEN_CAST),
                "OpenPipeWireRemote",
                &(&path, Options::new()),
            )
            .await
            .context("open the portal's PipeWire connection")?;
        let remote: zbus::zvariant::OwnedFd = reply.body().deserialize()?;
        Ok(ScreenPortal {
            conn: self.conn.clone(),
            path,
            closed,
            remote: remote.into(),
            node_id,
            size,
            input,
        })
    }

    async fn input_available(&self) -> bool {
        let reply = self
            .conn
            .call_method(
                Some(PORTAL),
                DESKTOP,
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &(REMOTE_DESKTOP, "AvailableDeviceTypes"),
            )
            .await;
        reply
            .ok()
            .and_then(|reply| reply.body().deserialize::<OwnedValue>().ok())
            .and_then(|value| u32::try_from(value).ok())
            .is_some_and(|types| types & INPUT_DEVICES == INPUT_DEVICES)
    }

    fn path(&self, kind: &str, token: &str) -> Result<OwnedObjectPath> {
        Ok(OwnedObjectPath::try_from(format!("{DESKTOP}/{kind}/{}/{token}", self.sender))?)
    }

    async fn request<'a, B>(
        &mut self,
        interface: &str,
        method: &'static str,
        token: &'a str,
        body: impl FnOnce(Options<'a>) -> B,
    ) -> Result<Response>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        self.method = method;
        self.request = Some(self.path("request", token)?);
        let opts = Options::from([("handle_token", Value::from(token))]);
        let reply = self
            .conn
            .call_method(Some(PORTAL), DESKTOP, Some(interface), method, &body(opts))
            .await
            .with_context(|| format!("Wayland portal {method}"))?;
        let path: OwnedObjectPath = reply.body().deserialize()?;
        self.request = Some(path.clone());
        while let Some(message) = poll_fn(|cx| Pin::new(&mut self.responses).poll_next(cx)).await {
            let message = message?;
            if message.header().path().map(|p| p.as_str()) != Some(path.as_str()) {
                continue;
            }
            self.request = None;
            let (code, result): (u32, Response) = message.body().deserialize()?;
            return match code {
                0 => Ok(result),
                1 => bail!("Wayland portal {method} was cancelled"),
                _ => bail!("Wayland portal {method} failed (response {code})"),
            };
        }
        bail!("The Wayland screen sharing portal disconnected")
    }

    async fn close(self) {
        if let Some(path) = self.request.as_ref() {
            close(&self.conn, REQUEST, path).await;
        }
        if let Some(path) = self.session.as_ref() {
            close(&self.conn, SESSION, path).await;
        }
    }
}

async fn wait_for_cancel(cancelled: impl Fn() -> bool) {
    while !cancelled() {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn close(conn: &Connection, interface: &str, path: &OwnedObjectPath) {
    let _ = tokio::time::timeout(
        CLOSE_TIMEOUT,
        conn.call_method(Some(PORTAL), path.as_str(), Some(interface), "Close", &()),
    )
    .await;
}

pub(super) fn read_restore_token(path: &std::path::Path) -> Option<String> {
    let token = std::fs::read_to_string(path).ok()?;
    let token = token.trim();
    (!token.is_empty() && token.len() <= 256).then(|| token.to_owned())
}

pub(super) fn save_restore_token(path: &std::path::Path, token: &str) -> std::io::Result<()> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let tmp = path.with_extension("tmp");
    let mut file =
        std::fs::OpenOptions::new().create(true).write(true).truncate(true).mode(0o600).open(&tmp)?;
    file.write_all(token.as_bytes())?;
    drop(file);
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::{read_restore_token, save_restore_token};

    #[test]
    fn the_screen_choice_survives_a_restart_privately() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token");
        assert_eq!(read_restore_token(&path), None);
        save_restore_token(&path, "first").unwrap();
        assert_eq!(read_restore_token(&path).as_deref(), Some("first"));
        save_restore_token(&path, "second").unwrap();
        assert_eq!(read_restore_token(&path).as_deref(), Some("second"));
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        std::fs::write(&path, "  ").unwrap();
        assert_eq!(read_restore_token(&path), None);
    }
}
