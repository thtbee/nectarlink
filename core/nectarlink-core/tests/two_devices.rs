// SPDX-License-Identifier: MPL-2.0
//! End-to-end tests: two real nodes talking over localhost.

use std::{
    future::Future,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use nectarlink_core::{
    Battery, DeviceInfo, DeviceKind, Error, FeatureState, LinkState, Node, NodeConfig, NodeEvent,
    Notification, NotificationAction, NotificationError, PairingEvent, PairingFailure, PairingUri,
    PlainKeyProtector, Platform, PowerLevel,
    features::{Effort, Upgrade, UpgradeAction},
};
use tempfile::TempDir;
use tokio::sync::broadcast::Receiver;

const WAIT: Duration = Duration::from_secs(20);

#[derive(Debug, Default)]
struct RecordingPlatform {
    rings: Mutex<Vec<bool>>,
    dismissed: Mutex<Vec<String>>,
    actions: Mutex<Vec<(String, String, Option<String>)>>,
}

impl Platform for RecordingPlatform {
    fn start_ringing(&self) {
        self.rings.lock().unwrap().push(true);
    }
    fn stop_ringing(&self) {
        self.rings.lock().unwrap().push(false);
    }
    fn dismiss_notification(&self, key: &str) -> Result<(), NotificationError> {
        self.dismissed.lock().unwrap().push(key.to_owned());
        Ok(())
    }
    fn run_notification_action(
        &self,
        key: &str,
        action: &str,
        reply: Option<&str>,
    ) -> Result<(), NotificationError> {
        if key == "gone" {
            return Err(NotificationError::NotFound);
        }
        self.actions.lock().unwrap().push((key.to_owned(), action.to_owned(), reply.map(str::to_owned)));
        Ok(())
    }
}

struct TestDevice {
    node: Node,
    events: Receiver<NodeEvent>,
    platform: Arc<RecordingPlatform>,
    dir: TempDir,
    name: &'static str,
}

fn info(name: &str, kind: DeviceKind) -> DeviceInfo {
    DeviceInfo {
        name: name.into(),
        kind,
        os: if kind == DeviceKind::Phone { "android" } else { "windows" }.into(),
        os_ver: "test".into(),
        model: None,
        accent: None,
    }
}

fn init_logging() {
    // Set RUST_LOG=nectarlink_core=debug to see what the nodes are doing.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .try_init();
}

async fn start_in(dir: TempDir, name: &'static str, kind: DeviceKind) -> TestDevice {
    init_logging();
    let mut config = NodeConfig::new(dir.path(), info(name, kind), "0.0.1-test");
    config.lan_discovery = false;
    config.away_mode = false;
    config.key_protector = Some(Arc::new(PlainKeyProtector));
    let platform = Arc::new(RecordingPlatform::default());
    let node = Node::start(config, platform.clone()).await.expect("node starts");
    let events = node.events();
    TestDevice { node, events, platform, dir, name }
}

async fn device(name: &'static str, kind: DeviceKind) -> TestDevice {
    start_in(tempfile::tempdir().unwrap(), name, kind).await
}

/// Waits for the first event matching `pick`, failing the test on timeout.
async fn wait_for<T>(dev: &mut TestDevice, what: &str, mut pick: impl FnMut(&NodeEvent) -> Option<T>) -> T {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, dev.events.recv()).await {
            Ok(Ok(event)) => {
                if let Some(found) = pick(&event) {
                    return found;
                }
            }
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(e)) => panic!("{}: event stream ended while waiting for {what}: {e}", dev.name),
            Err(_) => panic!("{}: timed out waiting for {what}", dev.name),
        }
    }
}

async fn wait_online(dev: &mut TestDevice, peer: nectarlink_core::DeviceId) {
    wait_for(dev, "peer online", |e| match e {
        NodeEvent::LinkChanged { device, link: LinkState::Online { .. } } if *device == peer => Some(()),
        _ => None,
    })
    .await
}

async fn with_timeout<T>(what: &str, fut: impl Future<Output = T>) -> T {
    tokio::time::timeout(WAIT, fut).await.unwrap_or_else(|_| panic!("timed out: {what}"))
}

/// Pairs `phone` with `pc` through the QR flow and waits until both are online.
async fn pair_qr(pc: &mut TestDevice, phone: &mut TestDevice) {
    let uri = pc.node.pairing_start_qr().await.unwrap();
    assert!(!uri.addrs.is_empty(), "pairing link should carry direct addresses");
    with_timeout("join", phone.node.pairing_join(&uri.to_uri())).await.expect("pairing succeeds");

    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());
    wait_for(pc, "paired", |e| match e {
        NodeEvent::Pairing(PairingEvent::Paired(d)) if d.id == phone_id => Some(()),
        _ => None,
    })
    .await;
    wait_online(phone, pc_id).await;
    wait_online(pc, phone_id).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn qr_pairing_connects_both_devices() {
    let mut pc = device("Desktop", DeviceKind::Desktop).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;
    pair_qr(&mut pc, &mut phone).await;

    assert!(!pc.node.is_pairing(), "pairing mode ends after a successful pairing");
    let pc_peers = pc.node.paired_devices().unwrap();
    assert_eq!(pc_peers.len(), 1);
    assert_eq!(pc_peers[0].id, phone.node.device_id());
    assert_eq!(pc_peers[0].info.name, "Pixel");
    assert!(matches!(pc_peers[0].link, LinkState::Online { .. }));

    let phone_peers = phone.node.paired_devices().unwrap();
    assert_eq!(phone_peers.len(), 1);
    assert_eq!(phone_peers[0].info.name, "Desktop");
}

#[tokio::test(flavor = "multi_thread")]
async fn ring_and_battery_flow_between_devices() {
    let mut pc = device("Desktop", DeviceKind::Desktop).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    with_timeout("ring", pc.node.ring(phone_id, true)).await.expect("ring succeeds");
    with_timeout("stop ring", pc.node.ring(phone_id, false)).await.expect("stop succeeds");
    assert_eq!(*phone.platform.rings.lock().unwrap(), vec![true, false]);

    let battery = Battery { level: 82, charging: true, plugged: Some("usb".into()) };
    phone.node.update_battery(battery.clone()).await;
    let received = wait_for(&mut pc, "battery", |e| match e {
        NodeEvent::Battery { device, battery } if *device == phone_id => Some(battery.clone()),
        _ => None,
    })
    .await;
    assert_eq!(received, battery);

    let mut renamed = info("Pixel 9 Pro", DeviceKind::Phone);
    renamed.accent = Some(0xFF65558F);
    phone.node.update_device_info(renamed.clone()).await;
    let info = wait_for(&mut pc, "rename", |e| match e {
        NodeEvent::PeerInfoChanged { device, info } if *device == phone_id && info.name == "Pixel 9 Pro" => {
            Some(info.clone())
        }
        _ => None,
    })
    .await;
    assert_eq!(info, renamed);
    assert_eq!(pc.node.paired_devices().unwrap()[0].info.name, "Pixel 9 Pro", "rename is persisted");
    let _ = pc_id;
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_pairing_secret_is_rejected() {
    let pc = device("Desktop", DeviceKind::Desktop).await;
    let phone = device("Pixel", DeviceKind::Phone).await;

    let mut uri = pc.node.pairing_start_qr().await.unwrap();
    uri.secret[0] ^= 0xFF; // as if an attacker guessed or altered the code
    let result = with_timeout("join", phone.node.pairing_join(&uri.to_uri())).await;
    assert!(matches!(result, Err(Error::Denied)), "got {result:?}");
    assert!(pc.node.paired_devices().unwrap().is_empty());
    assert!(phone.node.paired_devices().unwrap().is_empty());
    assert!(pc.node.is_pairing(), "a single wrong attempt doesn't end pairing mode");
}

#[tokio::test(flavor = "multi_thread")]
async fn pairing_requires_pairing_mode() {
    let pc = device("Desktop", DeviceKind::Desktop).await;
    let phone = device("Pixel", DeviceKind::Phone).await;

    // A link for the right device, but the PC isn't showing a code.
    let uri = PairingUri {
        id: pc.node.device_id(),
        secret: [1; 16],
        name: "Desktop".into(),
        addrs: pc.node.direct_addrs().await,
    };
    let result = with_timeout("join", phone.node.pairing_join(&uri.to_uri())).await;
    assert!(result.is_err(), "pairing without pairing mode must fail");
    assert!(pc.node.paired_devices().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn too_many_wrong_attempts_end_pairing_mode() {
    let mut pc = device("Desktop", DeviceKind::Desktop).await;
    let phone = device("Pixel", DeviceKind::Phone).await;
    let mut uri = pc.node.pairing_start_qr().await.unwrap();
    uri.secret[3] ^= 1;
    for _ in 0..5 {
        let _ = with_timeout("join", phone.node.pairing_join(&uri.to_uri())).await;
    }
    wait_for(&mut pc, "pairing failure", |e| match e {
        NodeEvent::Pairing(PairingEvent::Failed(PairingFailure::Rejected)) => Some(()),
        _ => None,
    })
    .await;
    assert!(!pc.node.is_pairing());
}

async fn sas_code(dev: &mut TestDevice) -> String {
    wait_for(dev, "6-digit code", |e| match e {
        NodeEvent::Pairing(PairingEvent::SasCode { code, .. }) => Some(code.clone()),
        _ => None,
    })
    .await
}

async fn start_nearby_setup() -> (TestDevice, TestDevice, Vec<SocketAddr>) {
    let pc = device("Desktop", DeviceKind::Desktop).await;
    let phone = device("Pixel", DeviceKind::Phone).await;
    // The PC shows its pairing screen; the phone picks it from the nearby list.
    pc.node.pairing_start_qr().await.unwrap();
    let addrs = pc.node.direct_addrs().await;
    phone.node.add_known_addrs(pc.node.device_id(), &addrs);
    (pc, phone, addrs)
}

#[tokio::test(flavor = "multi_thread")]
async fn nearby_pairing_with_matching_codes() {
    let (mut pc, mut phone, _) = start_nearby_setup().await;
    let pc_id = pc.node.device_id();
    let initiator = phone.node.clone();
    let task = tokio::spawn(async move { initiator.pairing_start_nearby(pc_id).await });

    let (phone_code, pc_code) = (sas_code(&mut phone).await, sas_code(&mut pc).await);
    assert_eq!(phone_code, pc_code, "both screens show the same code");
    assert_eq!(phone_code.len(), 6);

    phone.node.pairing_confirm(true).unwrap();
    pc.node.pairing_confirm(true).unwrap();
    with_timeout("nearby pairing", task).await.unwrap().expect("pairing succeeds");

    wait_online(&mut phone, pc_id).await;
    assert_eq!(pc.node.paired_devices().unwrap().len(), 1);
    assert_eq!(phone.node.paired_devices().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn nearby_pairing_fails_if_either_user_declines() {
    let (mut pc, mut phone, _) = start_nearby_setup().await;
    let pc_id = pc.node.device_id();
    let initiator = phone.node.clone();
    let task = tokio::spawn(async move { initiator.pairing_start_nearby(pc_id).await });

    sas_code(&mut phone).await;
    sas_code(&mut pc).await;
    phone.node.pairing_confirm(true).unwrap();
    pc.node.pairing_confirm(false).unwrap(); // codes didn't match on the PC

    let result = with_timeout("nearby pairing", task).await.unwrap();
    assert!(matches!(result, Err(Error::Denied)), "the phone learns the PC rejected: {result:?}");
    wait_for(&mut pc, "declined", |e| match e {
        NodeEvent::Pairing(PairingEvent::Failed(PairingFailure::Declined)) => Some(()),
        _ => None,
    })
    .await;
    assert!(pc.node.paired_devices().unwrap().is_empty());
    assert!(phone.node.paired_devices().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn unpairing_is_mirrored_on_the_other_device() {
    let mut pc = device("Desktop", DeviceKind::Desktop).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;
    pair_qr(&mut pc, &mut phone).await;
    let pc_id = pc.node.device_id();

    with_timeout("unpair", phone.node.unpair(pc_id)).await.unwrap();
    let phone_id = phone.node.device_id();
    wait_for(&mut pc, "device removed", |e| match e {
        NodeEvent::DeviceRemoved(id) if *id == phone_id => Some(()),
        _ => None,
    })
    .await;
    assert!(pc.node.paired_devices().unwrap().is_empty());
    assert!(phone.node.paired_devices().unwrap().is_empty());
    assert!(matches!(pc.node.ring(phone_id, true).await, Err(Error::NotPaired)));
}

#[tokio::test(flavor = "multi_thread")]
async fn reconnects_after_a_restart() {
    let mut pc = device("Desktop", DeviceKind::Desktop).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    let old_ports: std::collections::BTreeSet<u16> =
        phone.node.direct_addrs().await.iter().map(SocketAddr::port).collect();
    // The phone restarts (app update, reboot...).
    phone.node.shutdown().await;
    wait_for(&mut pc, "peer offline", |e| match e {
        NodeEvent::LinkChanged { device, link: LinkState::Offline { .. } } if *device == phone_id => Some(()),
        _ => None,
    })
    .await;

    let ports = |addrs: Vec<SocketAddr>| {
        addrs.iter().map(SocketAddr::port).collect::<std::collections::BTreeSet<_>>()
    };
    // A restart is a new process: the old node and its sockets are gone.
    let TestDevice { node: old_node, dir, .. } = phone;
    drop(old_node);
    let mut phone = start_in(dir, "Pixel", DeviceKind::Phone).await;
    assert_eq!(ports(phone.node.direct_addrs().await), old_ports, "the port is kept across restarts");
    assert_eq!(phone.node.device_id(), phone_id, "identity survives the restart");
    assert_eq!(phone.node.paired_devices().unwrap().len(), 1, "pairing survives the restart");

    wait_online(&mut phone, pc_id).await;
    wait_online(&mut pc, phone_id).await;
    with_timeout("ring after reconnect", pc.node.ring(phone_id, true)).await.expect("ring works again");
}

/// Waits for a capability matrix of `peer` whose `feature` is in a state
/// matching `want`.
async fn wait_feature(
    dev: &mut TestDevice,
    peer: nectarlink_core::DeviceId,
    feature: &str,
    want: impl Fn(&FeatureState) -> bool,
) -> FeatureState {
    wait_for(dev, feature, |e| match e {
        NodeEvent::Capabilities(m) if m.device == peer => m.state(feature).filter(|s| want(s)),
        _ => None,
    })
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn capabilities_follow_power_and_toggles() {
    let mut pc = device("Desktop", DeviceKind::Desktop).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    // Right after connecting, the PC knows what the phone offers.
    let matrix = pc.node.capabilities(phone_id).unwrap();
    assert_eq!(matrix.state("device.find_phone"), Some(FeatureState::Available));
    let elevated =
        Upgrade { action: UpgradeAction::RaisePower(PowerLevel::Elevated), effort: Effort::Minutes(2) };
    assert_eq!(matrix.state("clipboard.auto_phone_to_pc"), Some(FeatureState::Locked { upgrade: elevated }));
    // Both devices compute the same matrix for the pair.
    assert_eq!(phone.node.capabilities(pc_id).unwrap().features, matrix.features);

    // The phone is raised to Elevated and now offers automatic clipboard.
    phone.node.update_power(PowerLevel::Elevated, vec!["clip.read.auto".into(), "clip.write".into()]).await;
    wait_feature(&mut pc, phone_id, "clipboard.auto_phone_to_pc", |s| *s == FeatureState::Available).await;

    // The PC user turns clipboard off for this phone.
    pc.node.set_device_toggle(phone_id, "clipboard", false).unwrap();
    let state =
        wait_feature(&mut pc, phone_id, "clipboard.pc_to_phone", |s| *s != FeatureState::Available).await;
    assert_eq!(
        state,
        FeatureState::Locked {
            upgrade: Upgrade {
                action: UpgradeAction::EnableDeviceToggle("clipboard"),
                effort: Effort::Instant
            }
        }
    );
    let toggles = pc.node.device_toggles(phone_id).unwrap();
    assert!(toggles.contains(&("clipboard", false)) && toggles.contains(&("photos", true)));
    assert!(matches!(pc.node.set_device_toggle(phone_id, "no-such-toggle", true), Err(Error::Unsupported)));

    // What the phone offered is remembered while it's offline.
    phone.node.shutdown().await;
    wait_for(&mut pc, "peer offline", |e| match e {
        NodeEvent::LinkChanged { device, link: LinkState::Offline { .. } } if *device == phone_id => Some(()),
        _ => None,
    })
    .await;
    pc.node.set_device_toggle(phone_id, "clipboard", true).unwrap();
    let matrix = pc.node.capabilities(phone_id).unwrap();
    assert_eq!(matrix.state("clipboard.auto_phone_to_pc"), Some(FeatureState::Available));
}

fn note(key: &str, app: &str, title: &str, icon: bool) -> Notification {
    Notification {
        key: key.into(),
        app: app.into(),
        app_name: "Chat".into(),
        title: Some(title.into()),
        text: Some("Are you coming?".into()),
        sub: None,
        when: 1_760_000_000_000,
        actions: vec![
            NotificationAction { id: "0".into(), title: "Reply".into(), reply: true },
            NotificationAction { id: "1".into(), title: "Mark as read".into(), reply: false },
        ],
        silent: false,
        icon: icon.then(|| vec![0x89, b'P', b'N', b'G', 1, 2, 3]),
    }
}

/// A phone that mirrors notifications (notification access granted).
async fn grant_notification_access(phone: &TestDevice) {
    phone.node.update_power(PowerLevel::Basic, vec!["notify.mirror".into(), "notify.reply".into()]).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn notifications_mirror_to_the_pc_and_actions_reach_the_phone() {
    let mut pc = device("Desktop", DeviceKind::Desktop).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;
    pair_qr(&mut pc, &mut phone).await;
    let phone_id = phone.node.device_id();
    grant_notification_access(&phone).await;

    // The listener connects: everything showing goes over as a snapshot,
    // with each app's icon once.
    phone
        .node
        .notifications_reset(vec![note("a", "com.chat", "Sam", true), note("b", "com.chat", "Alex", true)])
        .await;
    let items = wait_for(&mut pc, "snapshot", |e| match e {
        NodeEvent::NotificationsReset { device, items } if *device == phone_id => Some(items.clone()),
        _ => None,
    })
    .await;
    assert_eq!(items.len(), 2);
    assert_eq!(items.iter().filter(|n| n.icon.is_some()).count(), 1, "one icon per app and session");

    // A new one: the PC already has this app's icon.
    phone.node.notification_posted(note("c", "com.chat", "Kim", true)).await;
    let posted = wait_for(&mut pc, "posted", |e| match e {
        NodeEvent::NotificationPosted { device, notification } if *device == phone_id => {
            Some(notification.clone())
        }
        _ => None,
    })
    .await;
    assert_eq!(posted.key, "c");
    assert_eq!(posted.title.as_deref(), Some("Kim"));
    assert_eq!(posted.icon, None);

    phone.node.notification_removed("a".into()).await;
    let removed = wait_for(&mut pc, "removed", |e| match e {
        NodeEvent::NotificationRemoved { device, key } if *device == phone_id => Some(key.clone()),
        _ => None,
    })
    .await;
    assert_eq!(removed, "a");

    // The PC dismisses one and replies to another.
    with_timeout("dismiss", pc.node.dismiss_notification(phone_id, "b".into())).await.expect("dismissed");
    with_timeout(
        "reply",
        pc.node.run_notification_action(phone_id, "c".into(), "0".into(), Some("Yes!".into())),
    )
    .await
    .expect("replied");
    assert_eq!(*phone.platform.dismissed.lock().unwrap(), vec!["b".to_owned()]);
    assert_eq!(*phone.platform.actions.lock().unwrap(), vec![("c".into(), "0".into(), Some("Yes!".into()))]);

    let gone =
        with_timeout("gone", pc.node.run_notification_action(phone_id, "gone".into(), "1".into(), None))
            .await;
    assert!(matches!(gone, Err(Error::NotFound)), "{gone:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn notification_toggles_are_honored_on_both_devices() {
    let mut pc = device("Desktop", DeviceKind::Desktop).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());
    grant_notification_access(&phone).await;
    phone.node.notifications_reset(vec![note("a", "com.chat", "Sam", false)]).await;
    let snapshot = |e: &NodeEvent| match e {
        NodeEvent::NotificationsReset { device, items } if *device == phone_id => Some(items.len()),
        _ => None,
    };
    assert_eq!(wait_for(&mut pc, "first snapshot", snapshot).await, 1);

    // The phone stops sharing with this PC: the PC is cleared, and actions
    // from it are refused.
    phone.node.set_device_toggle(pc_id, "notifications", false).unwrap();
    assert_eq!(wait_for(&mut pc, "cleared by the phone", snapshot).await, 0);
    let denied = with_timeout("denied", pc.node.dismiss_notification(phone_id, "a".into())).await;
    assert!(matches!(denied, Err(Error::Denied)), "{denied:?}");
    phone.node.set_device_toggle(pc_id, "notifications", true).unwrap();
    assert_eq!(wait_for(&mut pc, "shared again", snapshot).await, 1);

    // The PC hides this phone's notifications, then shows them again: it
    // asks the phone for what's showing.
    pc.node.set_device_toggle(phone_id, "notifications", false).unwrap();
    assert_eq!(wait_for(&mut pc, "hidden on the PC", snapshot).await, 0);
    pc.node.set_device_toggle(phone_id, "notifications", true).unwrap();
    assert_eq!(wait_for(&mut pc, "synced again", snapshot).await, 1);
}
