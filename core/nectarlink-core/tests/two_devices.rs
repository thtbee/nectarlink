// SPDX-License-Identifier: MPL-2.0
//! End-to-end tests: two real nodes talking over localhost.

use std::{
    future::Future,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use nectarlink_core::{
    Battery, DeviceInfo, DeviceKind, Direction, Error, FeatureState, FileSource, LinkState, MediaAction,
    MediaError, MediaPlayer, Node, NodeConfig, NodeEvent, Notification, NotificationAction,
    NotificationError, OutgoingFile, PairingEvent, PairingFailure, PairingUri, PlainKeyProtector, Platform,
    PowerAction, PowerLevel, Transfer, TransferFailure, TransferState,
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
    clipboard: Mutex<Vec<String>>,
    images: Mutex<Vec<(String, Vec<u8>)>>,
    media: Mutex<Vec<(String, MediaAction, Option<u64>)>>,
    power: Mutex<Vec<PowerAction>>,
    links: Mutex<Vec<String>>,
    /// Photos `open_photo` finds, by ID.
    photos: Mutex<std::collections::HashMap<String, std::path::PathBuf>>,
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
    fn set_clipboard(&self, text: &str) -> Result<(), String> {
        self.clipboard.lock().unwrap().push(text.to_owned());
        Ok(())
    }
    fn set_clipboard_image(&self, mime: &str, bytes: &[u8]) -> Result<(), String> {
        self.images.lock().unwrap().push((mime.to_owned(), bytes.to_vec()));
        Ok(())
    }
    fn power(&self, action: PowerAction) -> Result<(), String> {
        self.power.lock().unwrap().push(action);
        Ok(())
    }
    fn open_link(&self, _from: &nectarlink_core::DeviceId, url: &str) -> Result<(), String> {
        self.links.lock().unwrap().push(url.to_owned());
        Ok(())
    }
    fn open_photo(&self, id: &str) -> Result<OutgoingFile, String> {
        let path = self.photos.lock().unwrap().get(id).cloned().ok_or("gone")?;
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        Ok(OutgoingFile { name, folder: None, source: FileSource::Path(path) })
    }
    fn media_command(
        &self,
        player: &str,
        action: MediaAction,
        position: Option<u64>,
    ) -> Result<(), MediaError> {
        if player == "gone" {
            return Err(MediaError::NotFound);
        }
        self.media.lock().unwrap().push((player.to_owned(), action, position));
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
    config.capabilities = vec!["media.control".into()];
    let platform = Arc::new(RecordingPlatform::default());
    let node = Node::start(config, platform.clone()).await.expect("node starts");
    let events = node.events();
    TestDevice { node, events, platform, dir, name }
}

async fn device(name: &'static str, kind: DeviceKind) -> TestDevice {
    start_in(tempfile::tempdir().unwrap(), name, kind).await
}

/// A device that offers more than the defaults.
async fn device_with(name: &'static str, kind: DeviceKind, offers: &[&str]) -> TestDevice {
    let dev = device(name, kind).await;
    let mut caps = vec!["media.control".to_owned()];
    caps.extend(offers.iter().map(|c| (*c).to_owned()));
    dev.node.update_power(PowerLevel::NotApplicable, caps).await;
    dev
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
    // The port is kept, unless something else took it in the meantime
    // (other tests run in parallel); then any free port is fine.
    let new_ports = ports(phone.node.direct_addrs().await);
    if new_ports != old_ports {
        let taken = old_ports.iter().any(|p| std::net::UdpSocket::bind(("0.0.0.0", *p)).is_err());
        assert!(taken, "the port is kept across restarts when it's free: {old_ports:?} -> {new_ports:?}");
    }
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
        image: None,
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
    // Snapshots are idempotent and may repeat (one on connect, one when
    // the phone's list is reset), so wait for the one with the expected size.
    let snapshot = |expected: usize| {
        move |e: &NodeEvent| match e {
            NodeEvent::NotificationsReset { device, items }
                if *device == phone_id && items.len() == expected =>
            {
                Some(())
            }
            _ => None,
        }
    };
    wait_for(&mut pc, "first snapshot", snapshot(1)).await;

    // The phone stops sharing with this PC: the PC is cleared, and actions
    // from it are refused.
    phone.node.set_device_toggle(pc_id, "notifications", false).unwrap();
    wait_for(&mut pc, "cleared by the phone", snapshot(0)).await;
    let denied = with_timeout("denied", pc.node.dismiss_notification(phone_id, "a".into())).await;
    assert!(matches!(denied, Err(Error::Denied)), "{denied:?}");
    phone.node.set_device_toggle(pc_id, "notifications", true).unwrap();
    wait_for(&mut pc, "shared again", snapshot(1)).await;

    // The PC hides this phone's notifications, then shows them again: it
    // asks the phone for what's showing.
    pc.node.set_device_toggle(phone_id, "notifications", false).unwrap();
    wait_for(&mut pc, "hidden on the PC", snapshot(0)).await;
    pc.node.set_device_toggle(phone_id, "notifications", true).unwrap();
    wait_for(&mut pc, "synced again", snapshot(1)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn clipboard_text_goes_both_ways_with_consent() {
    let mut pc = device("Desktop", DeviceKind::Desktop).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    with_timeout("pc to phone", pc.node.send_clipboard(phone_id, "from the PC".into())).await.expect("sent");
    assert_eq!(*phone.platform.clipboard.lock().unwrap(), ["from the PC"]);
    wait_for(&mut phone, "clipboard received", |e| match e {
        NodeEvent::ClipboardReceived { device } if *device == pc_id => Some(()),
        _ => None,
    })
    .await;

    with_timeout("phone to pc", phone.node.send_clipboard(pc_id, "from the phone ✓".into()))
        .await
        .expect("sent");
    assert_eq!(*pc.platform.clipboard.lock().unwrap(), ["from the phone ✓"]);

    // The receiver turned the clipboard off for the sender.
    phone.node.set_device_toggle(pc_id, "clipboard", false).unwrap();
    let denied = with_timeout("denied", pc.node.send_clipboard(phone_id, "nope".into())).await;
    assert!(matches!(denied, Err(Error::Denied)), "{denied:?}");
    // The sender turned it off: nothing leaves.
    let not_sent = with_timeout("not sent", phone.node.send_clipboard(pc_id, "nope".into())).await;
    assert!(matches!(not_sent, Err(Error::Denied)), "{not_sent:?}");
    assert_eq!(phone.platform.clipboard.lock().unwrap().len(), 1);

    let huge = "x".repeat(nectarlink_core::CLIP_MAX_BYTES + 1);
    assert!(matches!(pc.node.send_clipboard(phone_id, huge).await, Err(Error::TooLarge)));
}

#[tokio::test(flavor = "multi_thread")]
async fn clipboard_images_go_both_ways_with_consent() {
    let mut pc = device("Desktop", DeviceKind::Desktop).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    // A large screenshot, and a small photo the other way.
    let screenshot = data(9 * 1024 * 1024 + 17, 3);
    with_timeout(
        "pc to phone",
        pc.node.send_clipboard_image(phone_id, "image/png".into(), screenshot.clone()),
    )
    .await
    .expect("sent");
    assert_eq!(*phone.platform.images.lock().unwrap(), [("image/png".to_owned(), screenshot)]);
    wait_for(&mut phone, "clipboard received", |e| match e {
        NodeEvent::ClipboardReceived { device } if *device == pc_id => Some(()),
        _ => None,
    })
    .await;
    let photo = data(40_000, 4);
    with_timeout("phone to pc", phone.node.send_clipboard_image(pc_id, "image/jpeg".into(), photo.clone()))
        .await
        .expect("sent");
    assert_eq!(*pc.platform.images.lock().unwrap(), [("image/jpeg".to_owned(), photo)]);

    // Only PNG and JPEG, and not too large.
    let gif = pc.node.send_clipboard_image(phone_id, "image/gif".into(), vec![1; 10]).await;
    assert!(matches!(gif, Err(Error::Internal(_))), "{gif:?}");
    let max = usize::try_from(nectarlink_core::CLIP_MAX_IMAGE_BYTES).unwrap();
    let huge = pc.node.send_clipboard_image(phone_id, "image/png".into(), vec![0; max + 1]).await;
    assert!(matches!(huge, Err(Error::TooLarge)), "{huge:?}");

    // The receiver turned the clipboard off for the sender; the sender's
    // stream is refused and nothing lands.
    phone.node.set_device_toggle(pc_id, "clipboard", false).unwrap();
    let denied = with_timeout(
        "denied",
        pc.node.send_clipboard_image(phone_id, "image/png".into(), data(2_000_000, 5)),
    )
    .await;
    assert!(matches!(denied, Err(Error::Denied)), "{denied:?}");
    assert_eq!(phone.platform.images.lock().unwrap().len(), 1);
    // And the connection still works.
    with_timeout("still connected", pc.node.send_clipboard(phone_id, "x".into()))
        .await
        .expect_err("still off");
}

fn player(id: &str, title: &str, art: Option<(&str, Vec<u8>)>) -> MediaPlayer {
    MediaPlayer {
        id: id.into(),
        app: "Music".into(),
        title: Some(title.into()),
        artist: Some("Artist".into()),
        album: None,
        playing: true,
        duration: Some(200_000),
        position: Some(12_000),
        actions: vec!["play".into(), "pause".into(), "next".into(), "seek".into(), "bogus".into()],
        art_key: art.as_ref().map(|(key, _)| (*key).into()),
        art: art.map(|(_, bytes)| bytes),
    }
}

fn media_of(device: nectarlink_core::DeviceId) -> impl FnMut(&NodeEvent) -> Option<Vec<MediaPlayer>> {
    move |e| match e {
        NodeEvent::MediaChanged { device: d, players } if *d == device => Some(players.clone()),
        _ => None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn media_is_shared_and_controlled_with_consent() {
    let mut pc = device("Desktop", DeviceKind::Desktop).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    // The phone plays something; the PC gets it with its artwork.
    let art = data(30_000, 7);
    phone.node.media_changed(vec![player("com.music", "Song", Some(("a1", art.clone())))]).await;
    let players = wait_for(&mut pc, "media", media_of(phone_id)).await;
    assert_eq!(players.len(), 1);
    assert_eq!(players[0].title.as_deref(), Some("Song"));
    assert_eq!(players[0].art.as_deref(), Some(&art[..]));
    assert_eq!(players[0].actions, ["play", "pause", "next", "seek"], "unknown actions are dropped");

    // Artwork goes once per session; a new picture goes again.
    phone.node.media_changed(vec![player("com.music", "Song", Some(("a1", art.clone())))]).await;
    let players = wait_for(&mut pc, "same art", media_of(phone_id)).await;
    assert_eq!((players[0].art_key.as_deref(), players[0].art.as_ref()), (Some("a1"), None));
    phone.node.media_changed(vec![player("com.music", "Next song", Some(("a2", vec![9; 10])))]).await;
    // (A state sent as the session started may arrive in between.)
    let players = wait_for(&mut pc, "new art", |e| {
        media_of(phone_id)(e).filter(|p| p.first().is_some_and(|p| p.title.as_deref() == Some("Next song")))
    })
    .await;
    assert_eq!(players[0].art.as_deref(), Some(&[9; 10][..]));

    // The PC controls it.
    with_timeout("pause", pc.node.media_command(phone_id, "com.music".into(), MediaAction::Pause, None))
        .await
        .expect("paused");
    with_timeout(
        "seek",
        pc.node.media_command(phone_id, "com.music".into(), MediaAction::Seek, Some(60_000)),
    )
    .await
    .expect("sought");
    assert_eq!(
        *phone.platform.media.lock().unwrap(),
        [
            ("com.music".to_owned(), MediaAction::Pause, None),
            ("com.music".to_owned(), MediaAction::Seek, Some(60_000))
        ]
    );
    let gone = pc.node.media_command(phone_id, "gone".into(), MediaAction::Play, None).await;
    assert!(matches!(gone, Err(Error::NotFound)), "{gone:?}");
    let no_position = pc.node.media_command(phone_id, "com.music".into(), MediaAction::Seek, None).await;
    assert!(no_position.is_err(), "{no_position:?}");

    // The PC turns media off for the phone: it's cleared there at once,
    // and the phone stops taking its commands only if the phone turns it off.
    pc.node.set_device_toggle(phone_id, "media", false).unwrap();
    assert!(wait_for(&mut pc, "cleared", media_of(phone_id)).await.is_empty());
    let off = pc.node.media_command(phone_id, "com.music".into(), MediaAction::Play, None).await;
    assert!(matches!(off, Err(Error::Denied)), "{off:?}");
    // On again: the PC asks for what plays now.
    pc.node.set_device_toggle(phone_id, "media", true).unwrap();
    let players = wait_for(&mut pc, "synced", media_of(phone_id)).await;
    assert_eq!(players[0].title.as_deref(), Some("Next song"));

    // The phone turns it off for the PC: the PC is told nothing plays, and
    // commands are refused.
    phone.node.set_device_toggle(pc_id, "media", false).unwrap();
    assert!(wait_for(&mut pc, "cleared by the phone", media_of(phone_id)).await.is_empty());
    let denied = pc.node.media_command(phone_id, "com.music".into(), MediaAction::Play, None).await;
    assert!(matches!(denied, Err(Error::Denied)), "{denied:?}");

    // And the other way: the PC's own players reach the phone once the
    // phone allows media again.
    phone.node.set_device_toggle(pc_id, "media", true).unwrap();
    pc.node.media_changed(vec![player("Spotify.exe", "On the PC", None)]).await;
    let players = wait_for(&mut phone, "pc media", |e| match e {
        NodeEvent::MediaChanged { device, players } if *device == pc_id && !players.is_empty() => {
            Some(players.clone())
        }
        _ => None,
    })
    .await;
    assert_eq!(players[0].title.as_deref(), Some("On the PC"));
    with_timeout(
        "phone controls the pc",
        phone.node.media_command(pc_id, "Spotify.exe".into(), MediaAction::Next, None),
    )
    .await
    .expect("next");
    assert_eq!(pc.platform.media.lock().unwrap()[0], ("Spotify.exe".to_owned(), MediaAction::Next, None));
}

#[tokio::test(flavor = "multi_thread")]
async fn phones_lock_pcs_and_links_open_on_either_side() {
    let mut pc = device_with("Desktop", DeviceKind::Desktop, &["pc.power"]).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    with_timeout("lock", phone.node.pc_power(pc_id, PowerAction::Lock)).await.expect("locked");
    with_timeout("sleep", phone.node.pc_power(pc_id, PowerAction::Sleep)).await.expect("asleep");
    // The PC answers first and acts a moment later.
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(*pc.platform.power.lock().unwrap(), [PowerAction::Lock, PowerAction::Sleep]);

    // A phone doesn't lock or sleep on request.
    let phone_power = pc.node.pc_power(phone_id, PowerAction::Lock).await;
    assert!(matches!(phone_power, Err(Error::Unsupported)), "{phone_power:?}");

    // The PC's user turned PC actions off for the phone.
    pc.node.set_device_toggle(phone_id, "pc_actions", false).unwrap();
    let denied = phone.node.pc_power(pc_id, PowerAction::Lock).await;
    assert!(matches!(denied, Err(Error::Denied)), "{denied:?}");

    // Links both ways; only web links.
    with_timeout("to pc", phone.node.open_link(pc_id, "https://example.com/a".into())).await.expect("opened");
    with_timeout("to phone", pc.node.open_link(phone_id, "http://example.org".into())).await.expect("opened");
    assert_eq!(*pc.platform.links.lock().unwrap(), ["https://example.com/a"]);
    assert_eq!(*phone.platform.links.lock().unwrap(), ["http://example.org"]);
    let script = pc.node.open_link(phone_id, "javascript:alert(1)".into()).await;
    assert!(script.is_err());
    assert_eq!(phone.platform.links.lock().unwrap().len(), 1);
}

/// Deterministic, incompressible-looking test data.
fn data(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

fn outgoing(dir: &std::path::Path, name: &str, contents: &[u8]) -> OutgoingFile {
    let path = dir.join(name);
    std::fs::write(&path, contents).unwrap();
    OutgoingFile { name: name.into(), folder: None, source: FileSource::Path(path) }
}

/// Waits for a transfer to reach a state matching `pick`.
async fn wait_transfer<T>(
    dev: &mut TestDevice,
    id: &str,
    what: &str,
    mut pick: impl FnMut(&Transfer) -> Option<T>,
) -> T {
    wait_for(dev, what, |e| match e {
        NodeEvent::Transfer(t) if t.id == id => pick(t),
        _ => None,
    })
    .await
}

fn saved(t: &Transfer) -> Option<Vec<std::path::PathBuf>> {
    match &t.state {
        TransferState::Done { saved } => Some(saved.clone()),
        TransferState::Failed(f) => panic!("transfer failed: {f:?}"),
        TransferState::Cancelled => panic!("transfer cancelled"),
        _ => None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn files_arrive_intact_under_free_names() {
    let mut pc = device("Desktop", DeviceKind::Desktop).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;
    pair_qr(&mut pc, &mut phone).await;
    let phone_id = phone.node.device_id();
    let src = tempfile::tempdir().unwrap();
    let big = data(3 * 1024 * 1024 + 17, 1);
    let small = b"hello".to_vec();

    // The phone already has a "notes.txt".
    let downloads = phone.dir.path().join("received");
    std::fs::create_dir_all(&downloads).unwrap();
    std::fs::write(downloads.join("notes.txt"), "old").unwrap();

    let files = vec![outgoing(src.path(), "video.bin", &big), outgoing(src.path(), "notes.txt", &small)];
    let id = with_timeout("send", pc.node.send_files(phone_id, files)).await.expect("send starts");

    let received = wait_transfer(&mut phone, &id, "received", |t| {
        assert_eq!(t.direction, Direction::Incoming);
        assert_eq!(t.names, ["video.bin", "notes.txt"]);
        saved(t)
    })
    .await;
    assert_eq!(received, [downloads.join("video.bin"), downloads.join("notes (2).txt")]);
    assert_eq!(std::fs::read(&received[0]).unwrap(), big);
    assert_eq!(std::fs::read(&received[1]).unwrap(), small);
    assert_eq!(std::fs::read(downloads.join("notes.txt")).unwrap(), b"old", "existing files are kept");

    let sent = wait_transfer(&mut pc, &id, "sent", |t| {
        assert_eq!(t.direction, Direction::Outgoing);
        saved(t).map(|_| t.done)
    })
    .await;
    assert_eq!(sent, big.len() as u64 + 5);
    assert!(
        std::fs::read_dir(phone.dir.path().join("incoming")).unwrap().next().is_none(),
        "no partial files are left"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn folders_arrive_with_their_layout() {
    let mut pc = device("Desktop", DeviceKind::Desktop).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;
    pair_qr(&mut pc, &mut phone).await;
    let phone_id = phone.node.device_id();
    let src = tempfile::tempdir().unwrap();
    let trip = src.path().join("Trip");
    std::fs::create_dir_all(trip.join("Day 1")).unwrap();
    std::fs::write(trip.join("Day 1").join("beach.jpg"), b"sand").unwrap();
    std::fs::write(trip.join("notes.txt"), b"fun").unwrap();
    std::fs::write(src.path().join("ticket.pdf"), b"pdf").unwrap();

    // The phone already has a "Trip" folder: this one becomes "Trip (2)".
    let downloads = phone.dir.path().join("received");
    std::fs::create_dir_all(downloads.join("Trip")).unwrap();

    let files = nectarlink_core::outgoing_paths(&[trip, src.path().join("ticket.pdf")]).unwrap();
    let id = with_timeout("send", pc.node.send_files(phone_id, files)).await.expect("send starts");
    let received = wait_transfer(&mut phone, &id, "received", |t| {
        assert_eq!(t.names, ["Trip", "ticket.pdf"]);
        assert_eq!(t.files, 3);
        saved(t)
    })
    .await;
    let folder = downloads.join("Trip (2)");
    assert_eq!(received, [folder.clone(), downloads.join("ticket.pdf")]);
    assert_eq!(std::fs::read(folder.join("Day 1").join("beach.jpg")).unwrap(), b"sand");
    assert_eq!(std::fs::read(folder.join("notes.txt")).unwrap(), b"fun");
    assert!(
        std::fs::read_dir(downloads.join("Trip")).unwrap().next().is_none(),
        "the old folder is untouched"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn new_photos_reach_the_pc_and_come_when_asked() {
    let mut pc = device_with("Desktop", DeviceKind::Desktop, &[nectarlink_core::PHOTOS_SHOW]).await;
    let mut phone = device_with("Pixel", DeviceKind::Phone, &[nectarlink_core::PHOTOS_READ]).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());
    let shot = phone.dir.path().join("Screenshot_1.png");
    let pixels = data(300_000, 7);
    std::fs::write(&shot, &pixels).unwrap();
    phone.platform.photos.lock().unwrap().insert("media:1".into(), shot);
    let photo = nectarlink_core::Photo {
        id: "media:1".into(),
        name: "Screenshot_1.png".into(),
        size: pixels.len() as u64,
        taken: 1_790_000_000,
        screenshot: true,
        thumb: vec![0xff, 0xd8, 0xff],
    };

    phone.node.photo_taken(photo.clone()).await.unwrap();
    let got = wait_for(&mut pc, "the photo", |e| match e {
        NodeEvent::PhotoAdded { device, photo } if *device == phone_id => Some(photo.clone()),
        _ => None,
    })
    .await;
    assert_eq!(got, photo);

    // The PC asks for it: it comes as a transfer.
    let id = with_timeout("fetch", pc.node.fetch_photo(phone_id, "media:1".into())).await.unwrap();
    let saved = wait_transfer(&mut pc, &id, "the photo", saved).await;
    assert_eq!(std::fs::read(&saved[0]).unwrap(), pixels);

    // Only announced photos can be asked for.
    assert!(matches!(pc.node.fetch_photo(phone_id, "media:2".into()).await, Err(Error::NotFound)));
    // The phone's user turned photos off for this PC: nothing more comes.
    phone.node.set_device_toggle(pc_id, "photos", false).unwrap();
    assert!(matches!(pc.node.fetch_photo(phone_id, "media:1".into()).await, Err(Error::Denied)));
    // An oversized preview isn't sent.
    let big = nectarlink_core::Photo { thumb: vec![0; nectarlink_core::PHOTO_MAX_THUMB_BYTES + 1], ..photo };
    assert!(matches!(phone.node.photo_taken(big).await, Err(Error::TooLarge)));
}

#[tokio::test(flavor = "multi_thread")]
async fn files_need_the_toggle_and_can_be_cancelled() {
    let mut pc = device("Desktop", DeviceKind::Desktop).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());
    let src = tempfile::tempdir().unwrap();

    // The phone doesn't take files from this PC.
    phone.node.set_device_toggle(pc_id, "files", false).unwrap();
    let id = pc.node.send_files(phone_id, vec![outgoing(src.path(), "a.txt", b"a")]).await.unwrap();
    let state =
        wait_transfer(&mut pc, &id, "refused", |t| t.state.is_finished().then(|| t.state.clone())).await;
    assert_eq!(state, TransferState::Failed(TransferFailure::Denied));
    phone.node.set_device_toggle(pc_id, "files", true).unwrap();
    // This PC doesn't send files to the phone.
    pc.node.set_device_toggle(phone_id, "files", false).unwrap();
    assert!(matches!(
        pc.node.send_files(phone_id, vec![outgoing(src.path(), "b.txt", b"b")]).await,
        Err(Error::Denied)
    ));
    pc.node.set_device_toggle(phone_id, "files", true).unwrap();

    // Cancelled by the receiver as soon as bytes flow.
    let big = data(64 * 1024 * 1024, 2);
    let id = pc.node.send_files(phone_id, vec![outgoing(src.path(), "big.bin", &big)]).await.unwrap();
    wait_transfer(&mut phone, &id, "receiving", |t| {
        (t.state == TransferState::Running && t.done > 0).then_some(())
    })
    .await;
    phone.node.cancel_transfer(&id);
    let on_phone =
        wait_transfer(&mut phone, &id, "cancelled here", |t| t.state.is_finished().then(|| t.state.clone()))
            .await;
    let on_pc =
        wait_transfer(&mut pc, &id, "cancelled there", |t| t.state.is_finished().then(|| t.state.clone()))
            .await;
    assert_eq!((on_phone, on_pc), (TransferState::Cancelled, TransferState::Cancelled));
    assert!(!phone.dir.path().join("received").join("big.bin").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn transfers_resume_after_the_receiver_restarts() {
    let mut pc = device("Desktop", DeviceKind::Desktop).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());
    let src = tempfile::tempdir().unwrap();
    let big = data(96 * 1024 * 1024, 3);
    let id = pc.node.send_files(phone_id, vec![outgoing(src.path(), "big.bin", &big)]).await.unwrap();

    // The phone goes away part way through.
    let before = wait_transfer(&mut phone, &id, "receiving", |t| (t.done > 0).then_some(t.done)).await;
    phone.node.shutdown().await;
    let TestDevice { node, dir, .. } = phone;
    drop(node);
    let interrupted = std::fs::metadata(dir.path().join("incoming").join(&id).join("0.part")).unwrap().len();
    assert!(interrupted >= before && interrupted < big.len() as u64, "{interrupted} of {}", big.len());
    wait_transfer(&mut pc, &id, "waiting for the phone", |t| {
        (t.state == TransferState::Waiting).then_some(())
    })
    .await;

    // Back again: the transfer continues where it stopped.
    let mut phone = start_in(dir, "Pixel", DeviceKind::Phone).await;
    let resumed_at =
        wait_transfer(&mut phone, &id, "resumed", |t| (t.state == TransferState::Running).then_some(t.done))
            .await;
    assert!(resumed_at >= interrupted, "resumed at {resumed_at}, had {interrupted}");
    let received = wait_transfer(&mut phone, &id, "received", saved).await;
    assert_eq!(std::fs::read(&received[0]).unwrap(), big);
    wait_transfer(&mut pc, &id, "sent", saved).await;
    let _ = pc_id;
}
