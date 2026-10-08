// SPDX-License-Identifier: MPL-2.0
//! One-shot actions on a paired device (docs/protocol/actions.md): lock or
//! sleep a PC, and open a link on the other device.

use std::{
    net::{Ipv4Addr, SocketAddrV4},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use nectarlink_protocol::{
    DeviceId, Envelope, ErrorCode,
    messages::{
        LinkOpen, PcPower, PcWakeInfo, TaskNotify, magic_packet, parse_mac, task_notify, types, wake,
    },
};

use crate::{Error, NodeEvent, Result, node::Shared, session::Session};

/// The device toggle that lets a phone lock, sleep or wake this PC.
pub(crate) const POWER_TOGGLE: &str = "pc_actions";
/// Offered by PCs that lock and sleep on request.
pub(crate) const POWER: &str = "pc.power";
/// Offered by PCs that share their Wake-on-LAN adapter addresses.
pub const PC_WAKE: &str = wake::CAP;
/// Offered by devices that open links sent to them.
pub(crate) const LINKS: &str = "link.open";
/// Offered by devices that show live and completed task notifications from a peer.
pub(crate) const TASK_NOTIFY: &str = types::TASK_NOTIFY;

/// Active long-running tasks on this device so newly connected peers receive
/// the ongoing chronometer state immediately.
#[derive(Debug, Default)]
pub(crate) struct ActiveTasks(Mutex<Vec<(Instant, TaskNotify, Option<DeviceId>)>>);

impl ActiveTasks {
    fn update(&self, peer: Option<DeviceId>, task: &TaskNotify) {
        let mut guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        guard.retain(|(_, existing, _)| existing.id != task.id);
        if task.active {
            if guard.len() >= task_notify::MAX_ACTIVE {
                guard.remove(0);
            }
            guard.push((Instant::now(), task.clone(), peer));
        }
    }

    fn snapshot_for(&self, peer: &DeviceId) -> Vec<TaskNotify> {
        let guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .iter()
            .filter(|(_, _, target)| target.is_none_or(|id| &id == peer))
            .map(|(at, task, _)| {
                let mut t = task.clone();
                t.elapsed_ms = t.elapsed_ms.saturating_add(at.elapsed().as_millis() as u64);
                t
            })
            .collect()
    }
}

/// Sends any currently active tasks on this PC to a newly connected peer.
pub(crate) async fn send_active_tasks(shared: &Arc<Shared>, session: &Session) {
    if !shared.offers(&session.peer, TASK_NOTIFY).unwrap_or(false) {
        return;
    }
    for task in shared.tasks.snapshot_for(&session.peer) {
        if let Ok(env) = Envelope::new(types::TASK_NOTIFY, &task) {
            let _ = session.send(env).await;
        }
    }
}

/// This PC's current Wake-on-LAN adapter addresses, sent to paired phones on
/// connect and whenever network adapters change.
#[derive(Debug, Default)]
pub(crate) struct CurrentWakeInfo(Mutex<Option<PcWakeInfo>>);

impl CurrentWakeInfo {
    pub(crate) fn set(&self, info: PcWakeInfo) -> PcWakeInfo {
        let clean = info.sanitized();
        *self.0.lock().unwrap() = Some(clean.clone());
        clean
    }

    pub(crate) fn get(&self) -> Option<PcWakeInfo> {
        self.0.lock().unwrap().clone()
    }
}

/// Sends this PC's `pc.wake_info` to a connected phone session. When the user
/// turned `pc_actions` off for that phone, sends an empty `PcWakeInfo` so the
/// phone clears any stored wake addresses for this PC.
pub(crate) async fn send_wake_info(shared: &Arc<Shared>, session: &Session) {
    let Some(info) = shared.wake_info.get() else {
        return;
    };
    let payload = if shared.toggle_on(&session.peer, POWER_TOGGLE) { info } else { PcWakeInfo::default() };
    if let Ok(env) = Envelope::new(types::PC_WAKE_INFO, &payload) {
        let _ = session.send(env).await;
    }
}

/// Sends Wake-on-LAN magic packets (`6 × 0xFF` + 16 × MAC) over UDP to the
/// stored subnet broadcasts and `255.255.255.255` on `ports`, repeated `bursts`
/// times separated by `interval`.
pub(crate) async fn wake(
    shared: &Arc<Shared>,
    peer: &DeviceId,
    ports: &[u16],
    extra_targets: &[Ipv4Addr],
    bursts: usize,
    interval: Duration,
) -> Result<()> {
    let record = shared.store.get_peer(peer)?.ok_or(Error::NotPaired)?;
    let Some(info) = record.wake_info.filter(|w| !w.macs.is_empty()) else {
        return Err(Error::Unsupported);
    };
    let packets: Vec<[u8; wake::MAGIC_PACKET_BYTES]> =
        info.macs.iter().filter_map(|m| parse_mac(m)).map(|m| magic_packet(&m)).collect();
    if packets.is_empty() {
        return Err(Error::Unsupported);
    }

    let mut targets: Vec<Ipv4Addr> = Vec::new();
    for b in &info.broadcasts {
        if let Ok(ip) = b.parse::<Ipv4Addr>()
            && !targets.contains(&ip)
        {
            targets.push(ip);
        }
    }
    if !targets.contains(&Ipv4Addr::BROADCAST) {
        targets.push(Ipv4Addr::BROADCAST);
    }
    for &ip in extra_targets {
        if !targets.contains(&ip) {
            targets.push(ip);
        }
    }

    let sock = tokio::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .await
        .map_err(|e| Error::Internal(format!("can't bind Wake-on-LAN UDP socket: {e}")))?;
    let _ = sock.set_broadcast(true);

    shared.nudge_reconnect();
    let bursts = bursts.max(1);
    for burst in 0..bursts {
        if burst > 0 {
            tokio::time::sleep(interval).await;
        }
        for pkt in &packets {
            for &ip in &targets {
                for &port in ports {
                    if let Err(e) = sock.send_to(pkt, SocketAddrV4::new(ip, port)).await {
                        tracing::debug!(%ip, port, error = %e, "Wake-on-LAN UDP send failed");
                    }
                }
            }
        }
    }
    shared.nudge_reconnect();
    Ok(())
}

/// What a phone can ask a PC to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerAction {
    Lock,
    Sleep,
}

impl PowerAction {
    pub fn as_str(self) -> &'static str {
        match self {
            PowerAction::Lock => "lock",
            PowerAction::Sleep => "sleep",
        }
    }

    pub fn parse(action: &str) -> Option<PowerAction> {
        match action {
            "lock" => Some(PowerAction::Lock),
            "sleep" => Some(PowerAction::Sleep),
            _ => None,
        }
    }
}

impl Shared {
    fn offers(&self, peer: &DeviceId, capability: &str) -> Result<bool> {
        Ok(self.store.get_peer(peer)?.is_some_and(|p| p.caps.contains(capability)))
    }
}

/// Checks a link the protocol allows: http or https, not too long.
pub(crate) fn valid_link(url: &str) -> bool {
    let lower = url.get(..8).map(str::to_ascii_lowercase).unwrap_or_default();
    (lower.starts_with("http://") || lower.starts_with("https://"))
        && url.len() <= nectarlink_protocol::messages::LINK_MAX_BYTES
        && !url.chars().any(|c| c.is_control() || c.is_whitespace())
        && url.split("://").nth(1).is_some_and(|rest| !rest.is_empty())
}

pub(crate) async fn pc_power(shared: &Arc<Shared>, session: &Session, action: PowerAction) -> Result<()> {
    if !shared.offers(&session.peer, POWER)? {
        return Err(Error::Unsupported);
    }
    let env = Envelope::new(types::PC_POWER, &PcPower { action: action.as_str().into() })?;
    session.request(env, crate::session::REQUEST_TIMEOUT).await?.expect(types::OK)?;
    Ok(())
}

fn link_host(url: &str) -> String {
    url.split("://").nth(1).unwrap_or(url).split(['/', '?', '#']).next().unwrap_or("").to_owned()
}

pub(crate) async fn open_link(shared: &Arc<Shared>, session: &Session, url: String) -> Result<()> {
    if !valid_link(&url) {
        return Err(Error::Internal("not a web link".into()));
    }
    if !shared.offers(&session.peer, LINKS)? {
        return Err(Error::Unsupported);
    }
    let env = Envelope::new(types::LINK_OPEN, &LinkOpen { url: url.clone() })?;
    session.request(env, crate::session::REQUEST_TIMEOUT).await?.expect(types::OK)?;
    let _ = shared.record_timeline(crate::timeline::NewTimelineEntry {
        kind: crate::TimelineKind::Link,
        device_id: session.peer,
        device_name: shared.peer_name(&session.peer),
        incoming: false,
        timestamp: crate::now_unix(),
        title: url.clone(),
        detail: link_host(&url),
        target: url,
        size_bytes: 0,
        duration_secs: 0,
        ref_id: None,
    });
    Ok(())
}

pub(crate) async fn task_notify(
    shared: &Arc<Shared>,
    peer: Option<DeviceId>,
    task: TaskNotify,
) -> Result<()> {
    let Some(clean) = task.sanitized() else {
        return Err(Error::Internal("invalid task notification".into()));
    };
    shared.tasks.update(peer, &clean);

    if let Some(id) = peer {
        if shared.store.get_peer(&id)?.is_none() {
            return Err(Error::NotPaired);
        }
        let Some(session) = shared.session(&id) else {
            return if clean.active { Ok(()) } else { Err(Error::Offline) };
        };
        if !shared.offers(&session.peer, TASK_NOTIFY)? {
            return Err(Error::Unsupported);
        }
        let env = Envelope::new(types::TASK_NOTIFY, &clean)?;
        session.request(env, crate::session::REQUEST_TIMEOUT).await?.expect(types::OK)?;
        return Ok(());
    }

    let targets: Vec<Arc<Session>> = shared
        .live_sessions()
        .into_iter()
        .filter(|s| shared.offers(&s.peer, TASK_NOTIFY).unwrap_or(false))
        .collect();
    if targets.is_empty() {
        return if clean.active { Ok(()) } else { Err(Error::Offline) };
    }
    let mut last_err = None;
    let mut any_ok = false;
    for session in targets {
        let env = Envelope::new(types::TASK_NOTIFY, &clean)?;
        match session
            .request(env, crate::session::REQUEST_TIMEOUT)
            .await
            .and_then(|r| Ok(r.expect(types::OK)?))
        {
            Ok(()) => any_ok = true,
            Err(e) => last_err = Some(e),
        }
    }
    if any_ok { Ok(()) } else { Err(last_err.unwrap_or(Error::Offline)) }
}

/// Handles `pc.power`, `pc.wake_info`, `link.open`, and `task.notify`. Returns false for other types.
pub(crate) async fn handle(shared: &Arc<Shared>, session: &Arc<Session>, env: &Envelope) -> Result<bool> {
    let peer = session.peer;
    let reply = match env.t.as_str() {
        types::PC_WAKE_INFO => {
            let info: PcWakeInfo = env.body()?;
            if !info.is_valid() {
                tracing::debug!(%peer, "ignoring invalid pc.wake_info");
                return Ok(true);
            }
            let info = info.sanitized();
            let stored = (!info.macs.is_empty()).then_some(info);
            let can_wake = stored.is_some();
            if let Err(e) = shared.store.update_wake_info(&peer, stored.as_ref()) {
                tracing::warn!(%peer, error = %e, "couldn't store wake info");
            }
            shared.emit(NodeEvent::WakeInfoChanged { device: peer, can_wake });
            return Ok(true);
        }
        types::PC_POWER => {
            let PcPower { action } = env.body()?;
            if !shared.local_capabilities().iter().any(|c| c == POWER) {
                Envelope::error(ErrorCode::Unsupported, "this device doesn't lock or sleep on request")
            } else if !shared.toggle_on(&peer, POWER_TOGGLE) {
                Envelope::error(ErrorCode::Denied, "PC actions are off for this device")
            } else {
                match PowerAction::parse(&action) {
                    None => Envelope::error(ErrorCode::Unsupported, "unknown action"),
                    Some(action) => {
                        let platform = shared.platform.clone();
                        // Sleeping cuts the connection: answer first.
                        let reply = Envelope::empty(types::OK).reply_to(env.id);
                        session.send(reply).await?;
                        tokio::spawn(async move {
                            // Give the answer time to leave before sleep cuts the network.
                            if action == PowerAction::Sleep {
                                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                            }
                            if let Err(reason) = tokio::task::spawn_blocking(move || platform.power(action))
                                .await
                                .unwrap_or_else(|e| Err(e.to_string()))
                            {
                                tracing::warn!(reason, "power action failed");
                            }
                        });
                        return Ok(true);
                    }
                }
            }
        }
        types::LINK_OPEN => {
            let LinkOpen { url } = env.body()?;
            if !valid_link(&url) {
                Envelope::error(ErrorCode::BadMessage, "not a web link")
            } else {
                let platform = shared.platform.clone();
                let url_for_open = url.clone();
                match tokio::task::spawn_blocking(move || platform.open_link(&peer, &url_for_open))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
                {
                    Ok(()) => {
                        let _ = shared.record_timeline(crate::timeline::NewTimelineEntry {
                            kind: crate::TimelineKind::Link,
                            device_id: peer,
                            device_name: shared.peer_name(&peer),
                            incoming: true,
                            timestamp: crate::now_unix(),
                            title: url.clone(),
                            detail: link_host(&url),
                            target: url,
                            size_bytes: 0,
                            duration_secs: 0,
                            ref_id: None,
                        });
                        Envelope::empty(types::OK)
                    }
                    Err(reason) => {
                        tracing::warn!(reason, "can't open a link");
                        Envelope::error(ErrorCode::Internal, "can't open it")
                    }
                }
            }
        }
        types::TASK_NOTIFY => {
            let task: TaskNotify = env.body()?;
            let reply = match task.sanitized() {
                None => Envelope::error(ErrorCode::BadMessage, "invalid task notification"),
                Some(task) => {
                    let platform = shared.platform.clone();
                    match tokio::task::spawn_blocking(move || platform.task_notify(&peer, &task))
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()))
                    {
                        Ok(()) => Envelope::empty(types::OK),
                        Err(reason) => {
                            tracing::warn!(reason, "can't show task notification");
                            Envelope::error(ErrorCode::Internal, "can't show task notification")
                        }
                    }
                }
            };
            if env.id.is_some() {
                session.send(reply.reply_to(env.id)).await?;
            }
            return Ok(true);
        }
        _ => return Ok(false),
    };
    session.send(reply.reply_to(env.id)).await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_links_go() {
        assert!(valid_link("https://example.com/a?b=c#d"));
        assert!(valid_link("HTTP://EXAMPLE.COM"));
        for bad in
            ["javascript:alert(1)", "file:///C:/x", "https://", "https://a b", "ms-settings:", "ftp://x"]
        {
            assert!(!valid_link(bad), "{bad}");
        }
        assert!(!valid_link(&format!("https://a.com/{}", "x".repeat(8192))));
    }

    #[test]
    fn power_actions_round_trip() {
        for action in [PowerAction::Lock, PowerAction::Sleep] {
            assert_eq!(PowerAction::parse(action.as_str()), Some(action));
        }
        assert_eq!(PowerAction::parse("shutdown"), None);
    }
}
