// SPDX-License-Identifier: GPL-3.0-or-later
//! `Pairing`: drives the pairing screen. Shows a QR code that refreshes
//! itself until a phone pairs, lists nearby devices for code pairing, and
//! handles the 6-digit code check.

use std::{
    pin::Pin,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QHash, QHashPair_QString_QVariant, QList, QString, QVariant};
use nectarlink_core::{DeviceId, PairingFailure};

use crate::{
    core_host, qr,
    state::{Changes, PairingView},
};

/// Matches the core's pairing window (PAIRING_TTL).
const QR_LIFETIME: Duration = Duration::from_secs(5 * 60);

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qlist.h");
        type QList_QVariant = cxx_qt_lib::QList<cxx_qt_lib::QVariant>;
    }

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        /// "idle", "starting", "hosting", "connecting", "comparing",
        /// "confirmed", "paired" or "failed".
        #[qproperty(QString, state)]
        /// The QR code as an SVG path over a `qrSize`×`qrSize` grid.
        #[qproperty(QString, qr_path)]
        #[qproperty(i32, qr_size)]
        /// When the QR code expires (ms since the epoch); it refreshes itself.
        #[qproperty(f64, expires_at)]
        /// The 6-digit code to compare, and the device it's shared with.
        #[qproperty(QString, code)]
        #[qproperty(QString, peer_name)]
        /// Why pairing failed: "rejected", "declined", "expired",
        /// "unreachable" or "other".
        #[qproperty(QString, failure)]
        /// Devices nearby in pairing mode: `[{ id, name }]`.
        #[qproperty(QList_QVariant, nearby)]
        type Pairing = super::PairingRust;

        /// Shows a QR code (and accepts nearby pairing) until cancelled.
        #[qinvokable]
        fn start(self: Pin<&mut Pairing>);
        /// Pairs with a nearby device that shows its pairing screen.
        #[qinvokable]
        fn pair_nearby(self: Pin<&mut Pairing>, device: &QString);
        /// Answers the code comparison.
        #[qinvokable]
        fn confirm(self: Pin<&mut Pairing>, codes_match: bool);
        /// Leaves pairing mode.
        #[qinvokable]
        fn cancel(self: Pin<&mut Pairing>);
    }

    impl cxx_qt::Threading for Pairing {}
    impl cxx_qt::Initialize for Pairing {}
}

#[derive(Default)]
pub struct PairingRust {
    state: QString,
    qr_path: QString,
    qr_size: i32,
    expires_at: f64,
    code: QString,
    peer_name: QString,
    failure: QString,
    nearby: QList<QVariant>,
    /// The link currently shown, to re-encode only when it changes.
    shown_uri: String,
}

impl cxx_qt::Initialize for qobject::Pairing {
    fn initialize(mut self: Pin<&mut Self>) {
        self.as_mut().set_state(QString::from("idle"));
        super::subscribe(
            self.qt_thread(),
            Changes::PAIRING | Changes::DISCOVERED | Changes::DEVICES,
            Self::refresh,
        );
    }
}

fn hub() -> &'static crate::state::Hub {
    &core_host::host().hub
}

/// Asks the core for a fresh pairing link and shows it.
fn host_qr() {
    hub().update(|s| s.set_pairing(PairingView::Starting));
    let Some(node) = core_host::node() else {
        hub().update(|s| s.set_pairing(PairingView::Failed(PairingFailure::Other("not ready".into()))));
        return;
    };
    core_host::spawn(async move {
        let next = match node.pairing_start_qr().await {
            Ok(uri) => {
                PairingView::Hosting { uri: uri.to_uri(), expires_at: SystemTime::now() + QR_LIFETIME }
            }
            Err(e) => {
                tracing::warn!(error = %e, "can't start pairing");
                PairingView::Failed(PairingFailure::Other(e.to_string()))
            }
        };
        // Don't resurrect a screen the user already left.
        hub()
            .update(|s| if s.pairing == PairingView::Starting { s.set_pairing(next) } else { Changes::NONE });
    });
}

impl qobject::Pairing {
    fn refresh(mut self: Pin<&mut Self>) {
        let (view, nearby, names) = hub().read(|s| {
            let peer = match &s.pairing {
                PairingView::Connecting { peer }
                | PairingView::Comparing { peer, .. }
                | PairingView::Confirmed { peer, .. } => Some(*peer),
                _ => None,
            };
            let nearby: Vec<(DeviceId, String)> =
                s.discovered.iter().map(|d| (d.id, d.name.clone().unwrap_or_else(|| d.id.short()))).collect();
            (s.pairing.clone(), nearby, peer.and_then(|p| s.name_of(&p)))
        });

        // An expired QR code is replaced right away; the user never sees
        // "expired" while the pairing screen is open.
        if view == PairingView::Failed(PairingFailure::Expired) && self.state == QString::from("hosting") {
            host_qr();
            return;
        }

        let state = match &view {
            PairingView::Idle => "idle",
            PairingView::Starting => "starting",
            PairingView::Hosting { .. } => "hosting",
            PairingView::Connecting { .. } => "connecting",
            PairingView::Comparing { .. } => "comparing",
            PairingView::Confirmed { .. } => "confirmed",
            PairingView::Paired { .. } => "paired",
            PairingView::Failed(_) => "failed",
        };
        match &view {
            PairingView::Hosting { uri, expires_at } => {
                if self.shown_uri != *uri {
                    match qr::encode(uri) {
                        Ok(code) => {
                            self.as_mut().set_qr_path(QString::from(&code.path));
                            self.as_mut().set_qr_size(code.size as i32);
                        }
                        Err(e) => tracing::error!(error = %e, "can't encode the pairing link"),
                    }
                    self.as_mut().rust_mut().shown_uri = uri.clone();
                }
                let ms = expires_at.duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as f64;
                self.as_mut().set_expires_at(ms);
            }
            PairingView::Comparing { code, .. } | PairingView::Confirmed { code, .. } => {
                self.as_mut().set_code(QString::from(code));
            }
            PairingView::Paired { name, .. } => self.as_mut().set_peer_name(QString::from(name)),
            PairingView::Failed(failure) => {
                let failure = match failure {
                    PairingFailure::Rejected => "rejected",
                    PairingFailure::Declined => "declined",
                    PairingFailure::Expired => "expired",
                    PairingFailure::Unreachable => "unreachable",
                    PairingFailure::Other(_) => "other",
                };
                self.as_mut().set_failure(QString::from(failure));
            }
            _ => {}
        }
        if let Some(name) = names {
            self.as_mut().set_peer_name(QString::from(&name));
        }

        let mut list = QList::<QVariant>::default();
        for (id, name) in nearby {
            let mut item = QHash::<QHashPair_QString_QVariant>::default();
            item.insert(QString::from("id"), QVariant::from(&QString::from(&id.to_string())));
            item.insert(QString::from("name"), QVariant::from(&QString::from(&name)));
            list.append(QVariant::from(&item));
        }
        self.as_mut().set_nearby(list);
        // Last, so QML reacting to the state sees the details already set.
        self.as_mut().set_state(QString::from(state));
    }

    pub fn start(self: Pin<&mut Self>) {
        host_qr();
    }

    pub fn pair_nearby(self: Pin<&mut Self>, device: &QString) {
        let (Some(peer), Some(node)) = (super::parse_device(device), core_host::node()) else { return };
        hub().update(|s| s.set_pairing(PairingView::Connecting { peer }));
        core_host::spawn(async move {
            // Success and failure arrive as pairing events.
            if let Err(e) = node.pairing_start_nearby(peer).await {
                tracing::info!(error = %e, "nearby pairing ended");
                hub().update(|s| match s.pairing {
                    PairingView::Connecting { .. } => {
                        s.set_pairing(PairingView::Failed(PairingFailure::Unreachable))
                    }
                    _ => Changes::NONE,
                });
            }
        });
    }

    pub fn confirm(self: Pin<&mut Self>, codes_match: bool) {
        let Some(node) = core_host::node() else { return };
        let comparing = hub().read(|s| match &s.pairing {
            PairingView::Comparing { peer, code } => Some((*peer, code.clone())),
            _ => None,
        });
        let Some((peer, code)) = comparing else { return };
        if let Err(e) = node.pairing_confirm(codes_match) {
            tracing::warn!(error = %e, "can't answer the code check");
        }
        let next = if codes_match {
            PairingView::Confirmed { peer, code }
        } else {
            PairingView::Failed(PairingFailure::Declined)
        };
        hub().update(|s| s.set_pairing(next));
    }

    pub fn cancel(self: Pin<&mut Self>) {
        if let Some(node) = core_host::node() {
            node.pairing_cancel();
        }
        hub().update(|s| s.set_pairing(PairingView::Idle));
    }
}
