// SPDX-License-Identifier: MPL-2.0
//! What an app needs again after it fell behind the node's events.
//!
//! Events go out on a bounded channel; an app that falls behind loses the
//! oldest. Most of what it shows can be read back (devices, the send queue),
//! but some state only ever arrives as an event: how a transfer stands, a
//! ring, a pairing. The node keeps the latest of those ([`LatestState`]), and
//! each app keeps what it was given ([`Resync`]); after a lag it gets the
//! ones it missed, and nothing it already had (so nothing opens twice).

use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

use nectarlink_protocol::DeviceId;

use crate::{
    events::{NodeEvent, PairingEvent},
    transfer::Transfer,
};

/// Finished transfers remembered, at most: the oldest are forgotten first.
const FINISHED_KEPT: usize = 64;
/// A code to compare older than this has timed out; it isn't shown again.
const SAS_FRESH: Duration = Duration::from_secs(120);

/// The latest of each event that carries state the app can't read back.
#[derive(Debug, Default)]
pub(crate) struct LatestState {
    transfers: HashMap<String, Transfer>,
    finished: VecDeque<String>,
    rings: HashMap<DeviceId, bool>,
    remote_rings: HashMap<DeviceId, bool>,
    pairing: Option<(PairingEvent, Instant)>,
}

impl LatestState {
    /// Notes `event` as it goes out.
    pub(crate) fn note(&mut self, event: &NodeEvent) {
        match event {
            NodeEvent::Transfer(t) => {
                if t.state.is_finished() && !self.finished.contains(&t.id) {
                    self.finished.push_back(t.id.clone());
                    while self.finished.len() > FINISHED_KEPT {
                        if let Some(old) = self.finished.pop_front() {
                            self.transfers.remove(&old);
                        }
                    }
                }
                self.transfers.insert(t.id.clone(), t.clone());
            }
            NodeEvent::Ring { device, on } => {
                self.rings.insert(*device, *on);
            }
            NodeEvent::RemoteRing { device, on } => {
                self.remote_rings.insert(*device, *on);
            }
            NodeEvent::Pairing(p) => self.pairing = Some((p.clone(), Instant::now())),
            NodeEvent::DeviceRemoved(device) => {
                self.rings.remove(device);
                self.remote_rings.remove(device);
            }
            _ => {}
        }
    }

    /// The latest of each, as events.
    pub(crate) fn snapshot(&self) -> Vec<NodeEvent> {
        let mut out: Vec<NodeEvent> = self.transfers.values().cloned().map(NodeEvent::Transfer).collect();
        out.extend(self.rings.iter().map(|(device, on)| NodeEvent::Ring { device: *device, on: *on }));
        out.extend(
            self.remote_rings.iter().map(|(device, on)| NodeEvent::RemoteRing { device: *device, on: *on }),
        );
        if let Some((p, at)) = &self.pairing
            && (!matches!(p, PairingEvent::SasCode { .. }) || at.elapsed() < SAS_FRESH)
        {
            out.push(NodeEvent::Pairing(p.clone()));
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    Transfer(String),
    Ring(DeviceId),
    RemoteRing(DeviceId),
    Pairing,
}

fn key(event: &NodeEvent) -> Option<Key> {
    Some(match event {
        NodeEvent::Transfer(t) => Key::Transfer(t.id.clone()),
        NodeEvent::Ring { device, .. } => Key::Ring(*device),
        NodeEvent::RemoteRing { device, .. } => Key::RemoteRing(*device),
        NodeEvent::Pairing(_) => Key::Pairing,
        _ => return None,
    })
}

/// What one app was given of that state. Feed it every event the app gets
/// ([`Resync::note`]); after a lag, [`Resync::missed`] says what to give it.
#[derive(Debug, Default)]
pub struct Resync {
    given: HashMap<Key, NodeEvent>,
}

impl Resync {
    pub fn new() -> Self {
        Self::default()
    }

    /// Notes an event the app got.
    pub fn note(&mut self, event: &NodeEvent) {
        if let Some(k) = key(event) {
            self.given.insert(k, event.clone());
        }
    }

    /// Of `latest` (from [`crate::Node::latest_state`]), the events the app
    /// didn't get, noted as given now. What `latest` no longer has is forgotten.
    pub fn missed(&mut self, latest: Vec<NodeEvent>) -> Vec<NodeEvent> {
        let mut missed = Vec::new();
        let mut kept = HashMap::with_capacity(latest.len());
        for event in latest {
            let Some(k) = key(&event) else { continue };
            if self.given.get(&k) != Some(&event) {
                missed.push(event.clone());
            }
            kept.insert(k, event);
        }
        self.given = kept;
        missed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer::{Direction, TransferState};

    fn transfer(id: &str, state: TransferState) -> NodeEvent {
        NodeEvent::Transfer(Transfer {
            id: id.into(),
            device: DeviceId([1; 32]),
            direction: Direction::Incoming,
            names: vec!["a.txt".into()],
            files: 1,
            total: 10,
            done: 0,
            state,
            recording: false,
            markers: Vec::new(),
            open_on_arrival: false,
        })
    }

    #[test]
    fn only_what_was_missed_comes_again() {
        let mut latest = LatestState::default();
        let mut app = Resync::new();
        for event in [
            transfer("seen", TransferState::Done { saved: Vec::new() }),
            transfer("missed", TransferState::Running),
            NodeEvent::Ring { device: DeviceId([2; 32]), on: true },
        ] {
            latest.note(&event);
            app.note(&event);
        }
        // Lost while the app was behind:
        for event in [
            transfer("missed", TransferState::Cancelled),
            NodeEvent::Ring { device: DeviceId([2; 32]), on: false },
        ] {
            latest.note(&event);
        }
        let missed = app.missed(latest.snapshot());
        assert_eq!(missed.len(), 2, "{missed:?}");
        assert!(missed.contains(&transfer("missed", TransferState::Cancelled)));
        assert!(missed.contains(&NodeEvent::Ring { device: DeviceId([2; 32]), on: false }));
        assert!(app.missed(latest.snapshot()).is_empty(), "given once");
    }

    #[test]
    fn finished_transfers_are_forgotten_oldest_first() {
        let mut latest = LatestState::default();
        for n in 0..(FINISHED_KEPT + 10) {
            latest.note(&transfer(&n.to_string(), TransferState::Cancelled));
        }
        latest.note(&transfer("live", TransferState::Running));
        let ids: Vec<String> = latest
            .snapshot()
            .into_iter()
            .filter_map(|e| match e {
                NodeEvent::Transfer(t) => Some(t.id),
                _ => None,
            })
            .collect();
        assert_eq!(ids.len(), FINISHED_KEPT + 1);
        assert!(ids.contains(&"live".to_owned()));
        assert!(!ids.contains(&"0".to_owned()));
    }

    #[test]
    fn a_stale_code_isnt_shown_again() {
        let mut latest = LatestState::default();
        latest.note(&NodeEvent::Pairing(PairingEvent::SasCode {
            peer: DeviceId([3; 32]),
            code: "123456".into(),
        }));
        assert_eq!(latest.snapshot().len(), 1);
        if let Some((_, at)) = latest.pairing.as_mut() {
            *at = at.checked_sub(SAS_FRESH).expect("the clock goes back far enough");
        }
        assert!(latest.snapshot().is_empty());
    }
}
