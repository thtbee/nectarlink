// SPDX-License-Identifier: MPL-2.0
//! The local find-device alarm and the alarms we've started on other devices.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use nectarlink_protocol::{
    DeviceId, Envelope,
    messages::{Ring, RingStopped, types},
};

use crate::{NodeEvent, node::Shared, session::Session};

const RING_DURATION: Duration = Duration::from_secs(60);

#[derive(Default)]
pub(crate) struct Rings {
    incoming: Mutex<Option<Incoming>>,
    outgoing: Mutex<HashMap<DeviceId, u64>>,
}

#[derive(Clone, Copy)]
pub(crate) struct Incoming {
    pub peer: DeviceId,
    token: Option<u64>,
    generation: u64,
}

impl Shared {
    pub(crate) fn start_remote_ring(self: &Arc<Self>, peer: DeviceId) -> u64 {
        let token = rand::random();
        {
            let mut outgoing = self.rings.outgoing.lock().unwrap_or_else(|e| e.into_inner());
            outgoing.insert(peer, token);
            self.emit(NodeEvent::RemoteRing { device: peer, on: true });
        }
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            tokio::time::sleep(RING_DURATION).await;
            if let Some(shared) = weak.upgrade() {
                shared.clear_remote_ring(peer, Some(token));
            }
        });
        token
    }

    pub(crate) fn remote_ring_token(&self, peer: DeviceId) -> Option<u64> {
        self.rings.outgoing.lock().unwrap_or_else(|e| e.into_inner()).get(&peer).copied()
    }

    pub(crate) fn clear_remote_ring(&self, peer: DeviceId, token: Option<u64>) {
        let mut outgoing = self.rings.outgoing.lock().unwrap_or_else(|e| e.into_inner());
        if outgoing.get(&peer).is_some_and(|current| token.is_none_or(|token| token == *current)) {
            outgoing.remove(&peer);
            self.emit(NodeEvent::RemoteRing { device: peer, on: false });
        }
    }

    pub(crate) async fn receive_ring(self: &Arc<Self>, session: &Arc<Session>, ring: Ring) {
        let peer = session.peer;
        if !ring.on {
            self.stop_local_ring_if(move |active| {
                active.peer == peer && ring.token.is_none_or(|token| Some(token) == active.token)
            })
            .await;
            return;
        }
        let shared = self.clone();
        let session = session.clone();
        let next = Incoming { peer, token: ring.token, generation: rand::random() };
        let Ok(Some(previous)) = tokio::task::spawn_blocking(move || {
            let mut incoming = shared.rings.incoming.lock().unwrap_or_else(|e| e.into_inner());
            if !session.is_alive() {
                return None;
            }
            let previous = incoming.replace(next);
            if previous.is_some() {
                shared.platform.stop_ringing();
            }
            shared.platform.start_ringing();
            shared.emit(NodeEvent::Ring { device: peer, on: true });
            // The control stream can close while waiting for the blocking worker.
            let weak = Arc::downgrade(&shared);
            shared.runtime.spawn(async move {
                tokio::time::sleep(RING_DURATION).await;
                if let Some(shared) = weak.upgrade() {
                    shared.stop_local_ring_if(move |active| active.generation == next.generation).await;
                }
            });
            Some(previous)
        })
        .await
        else {
            return;
        };
        if let Some(previous) = previous {
            self.report_ring_stopped(previous).await;
        }
    }

    pub(crate) async fn stop_local_ring_if(
        self: &Arc<Self>,
        matches: impl FnOnce(Incoming) -> bool + Send + 'static,
    ) {
        let shared = self.clone();
        let stopped = tokio::task::spawn_blocking(move || {
            let mut incoming = shared.rings.incoming.lock().unwrap_or_else(|e| e.into_inner());
            if !incoming.is_some_and(matches) {
                return None;
            }
            let stopped = incoming.take()?;
            shared.platform.stop_ringing();
            shared.emit(NodeEvent::Ring { device: stopped.peer, on: false });
            Some(stopped)
        })
        .await;
        if let Ok(Some(stopped)) = stopped {
            self.report_ring_stopped(stopped).await;
        }
    }

    async fn report_ring_stopped(&self, stopped: Incoming) {
        if let Some(token) = stopped.token
            && let Some(session) = self.session(&stopped.peer)
            && let Ok(env) = Envelope::new(types::DEVICE_RING_STOPPED, &RingStopped { token })
        {
            let _ = session.send(env).await;
        }
    }
}
