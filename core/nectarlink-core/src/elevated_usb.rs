// SPDX-License-Identifier: MPL-2.0
//! Elevated over USB (`docs/protocol/elevated-usb.md`).
//!
//! Elevated runs a small helper on the phone as Android's shell user. The
//! phone normally starts it through its own wireless debugging; a phone
//! plugged into a PC with USB debugging allowed can instead ask the PC to
//! start it, the way scrcpy starts its server. The PC only runs the app's own
//! helper and the app's own grants (checked in [`ElevatedUsbStart::is_valid`]),
//! and only on the phone that asked, over that phone's own USB connection.
//! The helper runs as long as the PC holds its adb shell: until the phone
//! asks again, the cable comes out, or the node stops.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use nectarlink_protocol::{
    DeviceId, Envelope, ErrorCode,
    messages::{ElevatedUsbReady, ElevatedUsbStart, elevated, types},
};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

use crate::{
    error::{Error, Result},
    node::Shared,
    session::Session,
};

/// How long one grant may take.
const GRANT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long the helper may take to say it's listening.
const READY_TIMEOUT: Duration = Duration::from_secs(15);

/// The phone's side: asks `session`'s PC to start this phone's helper and
/// returns the port it listens on.
pub(crate) async fn start(session: &Session, request: ElevatedUsbStart) -> Result<u16> {
    if !request.is_valid() {
        return Err(Error::Protocol("invalid Elevated over USB request".into()));
    }
    let env = Envelope::new(types::ELEVATED_USB_START, &request)?;
    let reply = session.request(env, READY_TIMEOUT + GRANT_TIMEOUT * elevated::MAX_GRANTS as u32).await?;
    let ready: ElevatedUsbReady = reply.expect_body(types::ELEVATED_USB_READY)?;
    Ok(ready.port)
}

/// The PC's side of `elevated.usb.start`.
pub(crate) async fn handle(shared: &Arc<Shared>, session: &Arc<Session>, env: &Envelope) -> Result<bool> {
    if env.t != types::ELEVATED_USB_START {
        return Ok(false);
    }
    let peer = session.peer;
    let request: ElevatedUsbStart = env.body()?;
    let usb = shared
        .usb_serials
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&peer)
        .cloned()
        .filter(|(_, _, port)| session.uses_usb_bridge_port(*port));
    let refusal = if !request.is_valid() {
        Some(Envelope::error(ErrorCode::BadMessage, "invalid Elevated over USB request"))
    } else if !shared.we_offer(elevated::USB) {
        Some(Envelope::error(ErrorCode::Unsupported, "this PC doesn't start Elevated over USB"))
    } else if usb.is_none() {
        Some(Envelope::error(ErrorCode::Unsupported, "this phone isn't connected to this PC over USB"))
    } else {
        None
    };
    if let Some(refusal) = refusal {
        session.send(refusal.reply_to(env.id)).await?;
        return Ok(true);
    }
    let Some((serial, adb_addr, bridge_port)) = usb else { return Ok(true) };
    // Grants and the helper take seconds: answer from a task of its own.
    let (shared, session, id) = (shared.clone(), session.clone(), env.id);
    tokio::spawn(async move {
        let reply = match launch(&shared, peer, adb_addr, &serial, bridge_port, &request).await {
            Ok(port) => Envelope::new(types::ELEVATED_USB_READY, &ElevatedUsbReady { port })
                .unwrap_or_else(|_| Envelope::error(ErrorCode::Internal, "can't answer")),
            Err(reason) => {
                tracing::info!(peer = %peer.short(), reason, "couldn't start Elevated over USB");
                Envelope::error(ErrorCode::Internal, reason)
            }
        };
        let _ = session.send(reply.reply_to(id)).await;
    });
    Ok(true)
}

/// Runs the grants, then starts the helper and keeps its shell open in a task
/// stored for `peer`. Returns the helper's port.
async fn launch(
    shared: &Arc<Shared>,
    peer: DeviceId,
    adb_addr: SocketAddr,
    serial: &str,
    bridge_port: u16,
    request: &ElevatedUsbStart,
) -> std::result::Result<u16, String> {
    for grant in &request.grants {
        run_to_end(adb_addr, serial, grant).await?;
    }
    let command = helper_command(&request.apk, &request.token);
    let stream = nectarlink_adb::shell(adb_addr, serial, &command).await.map_err(|e| e.to_string())?;
    let mut output = BufReader::new(stream);
    let mut first = String::new();
    tokio::time::timeout(READY_TIMEOUT, output.read_line(&mut first))
        .await
        .map_err(|_| "the helper didn't start in time".to_owned())?
        .map_err(|e| e.to_string())?;
    let port = parse_ready(&first).ok_or_else(|| format!("the helper said {:?}", first.trim()))?;
    // Holding the shell keeps the helper running; its output is only drained.
    let keep = tokio::spawn(async move {
        let mut sink = [0u8; 1024];
        while matches!(output.read(&mut sink).await, Ok(n) if n > 0) {}
    });
    let still_active = {
        let serials = shared.usb_serials.lock().unwrap_or_else(|e| e.into_inner());
        !shared.cancel.is_cancelled()
            && serials.get(&peer).is_some_and(|(s, a, p)| s == serial && *a == adb_addr && *p == bridge_port)
    };
    if !still_active {
        keep.abort();
        return Err("the USB connection ended".into());
    }
    if let Some(old) = shared.usb_helpers.lock().unwrap_or_else(|e| e.into_inner()).insert(peer, keep) {
        old.abort();
    }
    Ok(port)
}

/// Runs a short command and waits for it to finish.
async fn run_to_end(adb_addr: SocketAddr, serial: &str, command: &str) -> std::result::Result<(), String> {
    let mut stream = nectarlink_adb::shell(adb_addr, serial, command).await.map_err(|e| e.to_string())?;
    let mut ignored = Vec::new();
    tokio::time::timeout(GRANT_TIMEOUT, stream.read_to_end(&mut ignored))
        .await
        .map_err(|_| format!("{command:?} didn't finish"))?
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// The shell command that starts the helper (both parts already checked).
fn helper_command(apk: &str, token: &str) -> String {
    format!("CLASSPATH={apk} app_process /system/bin {} {token}", elevated::HELPER_CLASS)
}

/// The port from the helper's first line, `ready <port>`.
fn parse_ready(line: &str) -> Option<u16> {
    line.trim().strip_prefix("ready ")?.parse().ok().filter(|p| *p > 0)
}

/// Stops every helper started over USB (the node is stopping).
pub(crate) fn stop_all(shared: &Shared) {
    for (_, helper) in shared.usb_helpers.lock().unwrap_or_else(|e| e.into_inner()).drain() {
        helper.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_helper_command_and_its_answer() {
        assert_eq!(
            helper_command("/data/app/~~a==/app.nectarlink-b==/base.apk", "00ff"),
            "CLASSPATH=/data/app/~~a==/app.nectarlink-b==/base.apk app_process /system/bin \
             app.nectarlink.android.elevated.InputServer 00ff"
        );
        assert_eq!(parse_ready("ready 41234\n"), Some(41234));
        assert_eq!(parse_ready("ready 0"), None);
        assert_eq!(parse_ready("can't listen: denied"), None);
        assert_eq!(parse_ready(""), None);
    }
}
