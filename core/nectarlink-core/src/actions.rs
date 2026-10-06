// SPDX-License-Identifier: MPL-2.0
//! One-shot actions on a paired device (docs/protocol/actions.md): lock or
//! sleep a PC, and open a link on the other device.

use std::sync::Arc;

use nectarlink_protocol::{
    DeviceId, Envelope, ErrorCode,
    messages::{LinkOpen, PcPower, types},
};

use crate::{Error, Result, node::Shared, session::Session};

/// The device toggle that lets a phone lock or sleep this PC.
pub(crate) const POWER_TOGGLE: &str = "pc_actions";
/// Offered by PCs that lock and sleep on request.
pub(crate) const POWER: &str = "pc.power";
/// Offered by devices that open links sent to them.
pub(crate) const LINKS: &str = "link.open";

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

pub(crate) async fn open_link(shared: &Arc<Shared>, session: &Session, url: String) -> Result<()> {
    if !valid_link(&url) {
        return Err(Error::Internal("not a web link".into()));
    }
    if !shared.offers(&session.peer, LINKS)? {
        return Err(Error::Unsupported);
    }
    let env = Envelope::new(types::LINK_OPEN, &LinkOpen { url })?;
    session.request(env, crate::session::REQUEST_TIMEOUT).await?.expect(types::OK)?;
    Ok(())
}

/// Handles `pc.power` and `link.open`. Returns false for other types.
pub(crate) async fn handle(shared: &Arc<Shared>, session: &Arc<Session>, env: &Envelope) -> Result<bool> {
    let peer = session.peer;
    let reply = match env.t.as_str() {
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
                match tokio::task::spawn_blocking(move || platform.open_link(&peer, &url))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
                {
                    Ok(()) => Envelope::empty(types::OK),
                    Err(reason) => {
                        tracing::warn!(reason, "can't open a link");
                        Envelope::error(ErrorCode::Internal, "can't open it")
                    }
                }
            }
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
