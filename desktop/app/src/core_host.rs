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
use tokio::{runtime::Runtime, sync::broadcast::error::RecvError};

use crate::state::{Changes, CoreStatus, Hub};

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub struct CoreHost {
    runtime: Runtime,
    node: OnceLock<Node>,
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
    let host = CoreHost { runtime, node: OnceLock::new(), hub: Hub::new(), data_dir: data_dir.clone() };
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
    let config = NodeConfig::new(&data_dir, this_device(), env!("CARGO_PKG_VERSION"));
    let node = match Node::start(config, platform).await {
        Ok(node) => node,
        Err(e) => {
            tracing::error!(error = %e, "core failed to start");
            host.hub.update(|s| {
                s.status = Some(CoreStatus::Failed(e.to_string()));
                Changes::STATUS
            });
            return;
        }
    };
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
    let status = CoreStatus::Ready { device_id: node.device_id(), name: this_device().name };
    let _ = host.node.set(node);
    host.hub.update(|s| {
        let mut changes = s.set_devices(devices);
        for m in matrices {
            changes |= s.set_matrix(m);
        }
        s.status = Some(status);
        changes |= Changes::STATUS | Changes::CAPABILITIES;
        changes
    });

    loop {
        match events.recv().await {
            Ok(event) => {
                crate::notifications::apply(&event);
                crate::notifications::update_toasts(&event);
            }
            Err(RecvError::Lagged(missed)) => {
                // Resynchronize what can be re-read; transient events are lost.
                tracing::warn!(missed, "UI fell behind core events; resyncing");
                let devices =
                    host.node.get().map(|n| n.paired_devices().unwrap_or_default()).unwrap_or_default();
                host.hub.update(|s| s.set_devices(devices));
            }
            Err(RecvError::Closed) => return,
        }
    }
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
