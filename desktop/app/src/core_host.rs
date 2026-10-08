// SPDX-License-Identifier: GPL-3.0-or-later
//! Runs `nectarlink-core` beside Qt: a small Tokio runtime owns the [`Node`],
//! core events are folded into the [`Hub`], and Qt objects issue commands
//! through [`spawn`]. The UI starts before the node, which comes up in the
//! background (startup is never blocked on the network).

use std::{
    future::Future,
    path::PathBuf,
    sync::{Arc, OnceLock},
    time::Duration,
};

use nectarlink_core::{DeviceInfo, DeviceKind, LinkState, Node, NodeConfig, Platform};
use tokio::{
    runtime::Runtime,
    sync::{Notify, broadcast::error::RecvError},
};

use crate::state::{Changes, CoreStatus, Hub};

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub struct CoreHost {
    runtime: Runtime,
    node: OnceLock<Node>,
    /// Signaled once the node is up, or has failed to start.
    started: Notify,
    pub hub: Arc<Hub>,
    pub data_dir: PathBuf,
}

static HOST: OnceLock<CoreHost> = OnceLock::new();

/// The running host. Panics if called before [`start`].
pub fn host() -> &'static CoreHost {
    HOST.get().expect("core host is started in main before Qt")
}

/// The node, once it has started.
pub fn node() -> Option<Node> {
    host().node.get().cloned()
}

/// The node once it has started, or `None` if it couldn't.
pub async fn wait_for_node() -> Option<Node> {
    let host = host();
    loop {
        // Created before checking, so a signal in between isn't missed.
        let started = host.started.notified();
        if let Some(node) = node() {
            return Some(node);
        }
        if host.hub.read(|s| matches!(s.status, Some(CoreStatus::Failed(_)))) {
            return None;
        }
        started.await;
    }
}

/// Runs a command on the core's runtime.
pub fn spawn(fut: impl Future<Output = ()> + Send + 'static) {
    host().runtime.spawn(fut);
}

/// Creates the runtime and starts the node in the background.
pub fn start(data_dir: PathBuf, platform: Arc<dyn Platform>) -> std::io::Result<&'static CoreHost> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("nectarlink-core")
        .enable_all()
        .build()?;
    let host = CoreHost {
        runtime,
        node: OnceLock::new(),
        started: Notify::new(),
        hub: Hub::new(),
        data_dir: data_dir.clone(),
    };
    if HOST.set(host).is_err() {
        panic!("core host started twice");
    }
    let host = self::host();
    host.hub.update(|s| {
        s.status = Some(CoreStatus::Starting);
        Changes::STATUS
    });
    host.runtime.spawn(run(data_dir, platform));
    Ok(host)
}

async fn run(data_dir: PathBuf, platform: Arc<dyn Platform>) {
    let host = host();
    let wake = crate::win::wol::query_wake_status();
    let mut config = NodeConfig::new(&data_dir, this_device(), env!("CARGO_PKG_VERSION"));
    config.downloads_dir = Some(downloads_dir());
    // This PC's own players, for phones (crate::win::media_sessions).
    config.capabilities = vec![
        "media.control".into(),
        "pc.power".into(),
        nectarlink_core::PC_WAKE.into(),
        nectarlink_core::PHOTOS_SHOW.into(),
        nectarlink_core::CALLS_SHOW.into(),
        nectarlink_core::CONTACTS_SHOW.into(),
        nectarlink_core::SMS_SHOW.into(),
        nectarlink_core::MIRROR_VIEW.into(),
        nectarlink_core::MIRROR_LISTEN.into(),
        nectarlink_core::INPUT_INJECT.into(),
        nectarlink_core::DECK_ACTIONS.into(),
        nectarlink_core::PC_AUDIO.into(),
        nectarlink_core::RECORDER.into(),
        nectarlink_core::TOGGLES_SHOW.into(),
        nectarlink_core::STORAGE_MOUNT.into(),
        nectarlink_core::WEBCAM_VIRTUAL.into(),
    ];
    if crate::win::vcam::is_registered() {
        config.capabilities.push(nectarlink_core::WEBCAM_ADDON_VCAM.into());
    }
    let node = match Node::start(config, platform).await {
        Ok(node) => node,
        Err(e) => {
            tracing::error!(error = %e, "core failed to start");
            host.hub.update(|s| {
                s.status = Some(CoreStatus::Failed(e.to_string()));
                Changes::STATUS
            });
            host.started.notify_waiters();
            return;
        }
    };
    node.set_wake_info(wake.info.clone()).await;
    // Subscribe before reading the initial state so no event is missed.
    let mut events = node.events();
    let devices = node.paired_devices().unwrap_or_else(|e| {
        tracing::warn!(error = %e, "can't list paired devices");
        Vec::new()
    });
    // A device that never connected hasn't told us what it can do; its
    // features stay "unknown" (not "locked") until its first hello.
    let matrices: Vec<_> = devices
        .iter()
        .filter(|d| !matches!(d.link, LinkState::Offline { last_seen: None }))
        .filter_map(|d| node.capabilities(d.id).ok())
        .collect();
    let toggles: Vec<_> =
        devices.iter().filter_map(|d| node.phone_toggles(d.id).map(|t| (d.id, t))).collect();
    let status = CoreStatus::Ready { device_id: node.device_id(), name: this_device().name };
    crate::clipboard::apply_history_setting(&node);
    let _ = host.node.set(node);
    host.hub.update(|s| {
        let mut changes = s.set_devices(devices);
        for m in matrices {
            changes |= s.set_matrix(m);
        }
        for (id, t) in toggles {
            changes |= s.set_toggles(id, Some(t));
        }
        s.status = Some(status);
        s.wake = wake;
        changes |= Changes::STATUS | Changes::CAPABILITIES | Changes::CLIPBOARD;
        changes
    });
    host.started.notify_waiters();

    loop {
        match events.recv().await {
            Ok(event) => {
                crate::notifications::apply(&event);
                crate::media::on_event(&event);
                crate::notifications::update_toasts(&event);
                crate::clipboard::on_event(&event);
                crate::photos::on_event(&event);
                crate::calls::on_event(&event);
                crate::messages::on_event(&event);
                crate::mirror::on_event(&event);
                crate::remote::on_event(&event);
                crate::transfers::on_event(&event);
                crate::send_to::on_event(&event);
                crate::battery::on_event(&event);
                crate::storage::on_event(&event);
                crate::webcam::on_event(&event);
            }
            Err(RecvError::Lagged(missed)) => {
                // Resynchronize what can be re-read; transient events are lost.
                tracing::warn!(missed, "UI fell behind core events; resyncing");
                let devices =
                    host.node.get().map(|n| n.paired_devices().unwrap_or_default()).unwrap_or_default();
                let toggles: Vec<_> = host
                    .node
                    .get()
                    .map(|n| {
                        devices.iter().filter_map(|d| n.phone_toggles(d.id).map(|t| (d.id, t))).collect()
                    })
                    .unwrap_or_default();
                host.hub.update(|s| {
                    let mut changes = s.set_devices(devices);
                    for (id, t) in toggles {
                        changes |= s.set_toggles(id, Some(t));
                    }
                    changes
                });
            }
            Err(RecvError::Closed) => return,
        }
    }
}

/// Where files from phones go: `Downloads\Nectarlink`.
pub fn downloads_dir() -> PathBuf {
    dirs::download_dir().unwrap_or_else(|| host().data_dir.join("received")).join("Nectarlink")
}

/// How this PC presents itself to phones.
fn this_device() -> DeviceInfo {
    let name = hostname::get().ok().and_then(|h| h.into_string().ok()).unwrap_or_else(|| "Windows PC".into());
    DeviceInfo {
        name,
        kind: if crate::win::is_laptop() { DeviceKind::Laptop } else { DeviceKind::Desktop },
        os: "windows".into(),
        os_ver: crate::win::os_version(),
        model: None,
        accent: None,
    }
}

/// Re-reads physical network adapters and their Wake-on-LAN settings, updates
/// the UI state, and sends updated `pc.wake_info` to connected phones if changed.
pub async fn refresh_wake() {
    let wake = crate::win::wol::query_wake_status();
    let info = wake.info.clone();
    host().hub.update(|s| s.set_wake(wake));
    if let Some(node) = node() {
        node.set_wake_info(info).await;
    }
}

/// Stops the node (telling connected devices) and the runtime.
pub fn shutdown() {
    let Some(host) = HOST.get() else { return };
    if let Some(node) = host.node.get().cloned() {
        let done =
            host.runtime.block_on(async { tokio::time::timeout(SHUTDOWN_TIMEOUT, node.shutdown()).await });
        if done.is_err() {
            tracing::warn!("core shutdown timed out");
        }
    }
}
