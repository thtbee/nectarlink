// SPDX-License-Identifier: MPL-2.0
//! Phone quick settings on a PC (`docs/protocol/toggles.md`).
//!
//! A phone that offers `toggles.read` sends its current quick settings
//! (`phone.toggles`) when a session starts and after every change. A PC that
//! offers `toggles.show` displays them and sends `phone.toggle.set` for the
//! toggles the phone's Power Level and permissions allow.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

pub use nectarlink_protocol::messages::{
    PhoneToggleSet, PhoneToggleValue, PhoneToggles, ringer_modes, toggle_ids,
    toggles::{
        BLUETOOTH as TOGGLES_BLUETOOTH, BRIGHTNESS as TOGGLES_BRIGHTNESS, DND as TOGGLES_DND,
        FLASHLIGHT as TOGGLES_FLASHLIGHT, READ as TOGGLES_READ, RINGER as TOGGLES_RINGER,
        SHOW as TOGGLES_SHOW, VOLUME as TOGGLES_VOLUME, WIFI as TOGGLES_WIFI,
    },
};
use nectarlink_protocol::{DeviceId, Envelope, ErrorCode, messages::types};

use crate::{
    Error, Result,
    events::NodeEvent,
    node::Shared,
    session::{REQUEST_TIMEOUT, Session},
};

/// The device toggle that allows viewing and changing phone toggles.
pub(crate) const TOGGLE: &str = "toggles";

/// This phone's latest quick settings state (sent to newly connected PCs)
/// and the latest state received from each connected phone.
#[derive(Default)]
pub(crate) struct Current {
    local: Mutex<Option<PhoneToggles>>,
    peers: Mutex<HashMap<DeviceId, PhoneToggles>>,
}

impl Current {
    pub fn get(&self) -> Option<PhoneToggles> {
        self.local.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn set(&self, state: PhoneToggles) {
        *self.local.lock().unwrap_or_else(|e| e.into_inner()) = Some(state);
    }

    pub fn peer(&self, peer: &DeviceId) -> Option<PhoneToggles> {
        self.peers.lock().unwrap_or_else(|e| e.into_inner()).get(peer).cloned()
    }

    fn set_peer(&self, peer: DeviceId, state: PhoneToggles) {
        self.peers.lock().unwrap_or_else(|e| e.into_inner()).insert(peer, state);
    }

    pub fn remove_peer(&self, peer: &DeviceId) {
        self.peers.lock().unwrap_or_else(|e| e.into_inner()).remove(peer);
    }
}

/// The phone capability required to change toggle `id`.
pub fn required_capability(id: &str) -> Option<&'static str> {
    Some(match id {
        toggle_ids::DND => TOGGLES_DND,
        toggle_ids::RINGER => TOGGLES_RINGER,
        toggle_ids::FLASHLIGHT => TOGGLES_FLASHLIGHT,
        toggle_ids::VOLUME => TOGGLES_VOLUME,
        toggle_ids::BRIGHTNESS => TOGGLES_BRIGHTNESS,
        toggle_ids::WIFI => TOGGLES_WIFI,
        toggle_ids::BLUETOOTH => TOGGLES_BLUETOOTH,
        _ => return None,
    })
}

impl Shared {
    fn shows_toggles(&self, peer: &DeviceId) -> bool {
        self.store.get_peer(peer).ok().flatten().is_some_and(|p| p.caps.contains(TOGGLES_SHOW))
    }

    /// Sends this phone's current toggles to `session` if it shows them and
    /// the `toggles` device toggle is on.
    pub(crate) async fn send_toggles_state(&self, session: &Arc<Session>) {
        if !self.shows_toggles(&session.peer) || !self.toggle_on(&session.peer, TOGGLE) {
            return;
        }
        let Some(state) = self.toggles.get() else { return };
        if let Ok(env) = Envelope::new(types::PHONE_TOGGLES, &state) {
            let _ = session.send(env).await;
        }
    }

    /// Updates this phone's toggles state and sends it to connected PCs.
    pub(crate) async fn toggles_changed(&self, state: PhoneToggles) -> Result<()> {
        let state = state.sanitized().ok_or_else(|| Error::Protocol("invalid phone toggles".into()))?;
        self.toggles.set(state.clone());
        let env = Envelope::new(types::PHONE_TOGGLES, &state)?;
        for session in self.live_sessions() {
            if self.shows_toggles(&session.peer) && self.toggle_on(&session.peer, TOGGLE) {
                let _ = session.send(env.clone()).await;
            }
        }
        Ok(())
    }

    /// Brings a connected peer up to date when the `toggles` device toggle is
    /// flipped on this device.
    pub(crate) fn toggles_toggled(self: &Arc<Self>, peer: DeviceId, enabled: bool) {
        if !enabled {
            self.toggles.remove_peer(&peer);
            return;
        }
        if self.toggles.get().is_none() {
            return;
        }
        if let Some(session) = self.session(&peer) {
            let shared = self.clone();
            self.runtime.spawn(async move {
                shared.send_toggles_state(&session).await;
            });
        }
    }
}

/// Asks a paired phone to change one toggle (`phone.toggle.set`).
pub(crate) async fn set(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    id: String,
    value: PhoneToggleValue,
) -> Result<()> {
    let req = PhoneToggleSet { id, value };
    if !req.is_valid() {
        return Err(Error::Protocol("invalid phone toggle or value".into()));
    }
    if !shared.toggle_on(&session.peer, TOGGLE) {
        return Err(Error::Denied);
    }
    let Some(cap) = required_capability(&req.id) else {
        return Err(Error::Protocol("unknown toggle".into()));
    };
    let peer_caps = shared.store.get_peer(&session.peer)?.map(|p| p.caps).unwrap_or_default();
    let silent_needs_dnd = req.id == toggle_ids::RINGER
        && matches!(&req.value, PhoneToggleValue::Mode(m) if m == ringer_modes::SILENT);
    if !peer_caps.contains(cap) || (silent_needs_dnd && !peer_caps.contains(TOGGLES_DND)) {
        return Err(Error::Unsupported);
    }
    let env = Envelope::new(types::PHONE_TOGGLE_SET, &req)?;
    session.request(env, REQUEST_TIMEOUT).await?.expect(types::OK)?;
    Ok(())
}

/// Handles `phone.toggles` and `phone.toggle.set`.
pub(crate) async fn handle(shared: &Arc<Shared>, session: &Arc<Session>, env: &Envelope) -> Result<bool> {
    let peer = session.peer;
    match env.t.as_str() {
        types::PHONE_TOGGLES => {
            if shared.toggle_on(&peer, TOGGLE) {
                let state: PhoneToggles = env.body()?;
                if let Some(toggles) = state.sanitized() {
                    shared.toggles.set_peer(peer, toggles.clone());
                    shared.emit(NodeEvent::PhoneToggles { device: peer, toggles });
                }
            }
        }
        types::PHONE_TOGGLE_SET => {
            let req: PhoneToggleSet = env.body()?;
            let reply = run_set(shared, &peer, req).await;
            session.send(reply.reply_to(env.id)).await?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

async fn run_set(shared: &Arc<Shared>, peer: &DeviceId, req: PhoneToggleSet) -> Envelope {
    if !req.is_valid() {
        return Envelope::error(ErrorCode::BadMessage, "invalid toggle or value");
    }
    if !shared.toggle_on(peer, TOGGLE) {
        return Envelope::error(ErrorCode::Denied, "toggles are off for this device");
    }
    let Some(cap) = required_capability(&req.id) else {
        return Envelope::error(ErrorCode::BadMessage, "unknown toggle");
    };
    let caps = shared.local_capabilities();
    let silent_needs_dnd = req.id == toggle_ids::RINGER
        && matches!(&req.value, PhoneToggleValue::Mode(m) if m == ringer_modes::SILENT);
    let no_flash =
        req.id == toggle_ids::FLASHLIGHT && shared.toggles.get().is_some_and(|t| t.flashlight.is_none());
    if !caps.iter().any(|c| c == cap)
        || (silent_needs_dnd && !caps.iter().any(|c| c == TOGGLES_DND))
        || no_flash
    {
        return Envelope::error(ErrorCode::Unsupported, "this toggle is not available");
    }
    let platform = shared.platform.clone();
    let PhoneToggleSet { id, value } = req;
    let ran = tokio::task::spawn_blocking(move || platform.set_phone_toggle(&id, &value))
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
    match ran {
        Ok(()) => Envelope::empty(types::OK),
        Err(reason) => {
            tracing::warn!(reason, "phone toggle failed");
            Envelope::error(ErrorCode::Internal, "failed")
        }
    }
}
