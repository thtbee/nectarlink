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
    calls: Mutex<Vec<(String, nectarlink_core::CallCommand)>>,
    dialed: Mutex<Vec<String>>,
    texts: Mutex<Vec<(Vec<String>, String)>>,
    mirror_asks: Mutex<Vec<String>>,
    /// What a PC got of a phone's screen.
    screen: Arc<ScreenSink>,
    /// Photos `open_photo` finds, by ID.
    photos: Mutex<std::collections::HashMap<String, std::path::PathBuf>>,
    /// Optional barrier `open_photo` waits on to simulate a slow platform call.
    photo_barrier: Mutex<Option<Arc<std::sync::Barrier>>>,
    remote_inputs: Mutex<Vec<nectarlink_core::RemoteInput>>,
    phone_toggles: Mutex<Vec<(String, nectarlink_core::PhoneToggleValue)>>,
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
    fn mirror_sink(
        &self,
        _peer: &nectarlink_core::DeviceId,
        session: u32,
    ) -> Option<Arc<dyn nectarlink_core::MirrorSink>> {
        self.mirror_asks.lock().unwrap().push(format!("sink {session}"));
        Some(self.screen.clone())
    }
    fn mirror_requested(
        &self,
        _peer: &nectarlink_core::DeviceId,
        options: &nectarlink_core::MirrorStart,
    ) -> Result<(), String> {
        let sound = if options.audio { " with sound" } else { "" };
        let app = options.app.as_deref().map(|a| format!(" {a} as {}", options.session)).unwrap_or_default();
        self.mirror_asks.lock().unwrap().push(format!("start {}{sound}{app}", options.max_size));
        Ok(())
    }
    fn mirror_stop_requested(&self, _peer: &nectarlink_core::DeviceId, session: u32) {
        self.mirror_asks.lock().unwrap().push(format!("stop {session}"));
    }
    fn mirror_keyframe_requested(&self, _peer: &nectarlink_core::DeviceId, session: u32) {
        self.mirror_asks.lock().unwrap().push(format!("keyframe {session}"));
    }
    fn mirror_resize_requested(
        &self,
        _peer: &nectarlink_core::DeviceId,
        session: u32,
        width: u32,
        height: u32,
    ) {
        self.mirror_asks.lock().unwrap().push(format!("resize {session} {width}x{height}"));
    }
    fn mirror_input(
        &self,
        _peer: &nectarlink_core::DeviceId,
        session: u32,
        input: nectarlink_core::MirrorInput,
    ) {
        self.mirror_asks.lock().unwrap().push(format!("{input:?} on {session}"));
    }
    fn phone_apps(&self) -> Result<Vec<nectarlink_core::PhoneApp>, String> {
        let app = |pkg: &str, label: &str, icon: usize| nectarlink_core::PhoneApp {
            pkg: pkg.into(),
            label: label.into(),
            icon: Some(vec![1; icon]),
        };
        Ok(vec![
            app("com.example.notes", "notes", 100),
            app("com.example.chat", "Chat", 100),
            // Not a package name: never offered.
            app("not a package", "Bad", 10),
            // Too big an icon: shown without.
            app("com.example.maps", "Maps", 9 * 1024),
        ])
    }
    fn sms_threads(&self, limit: u32) -> Result<Vec<nectarlink_core::SmsThread>, String> {
        let thread = |i: u32| nectarlink_core::SmsThread {
            id: i.to_string(),
            addresses: vec![format!("+1555010{i}")],
            names: vec![],
            snippet: format!("text {i}"),
            date: 1_790_000_000_000 - i64::from(i),
            unread: i % 2,
            photo: None,
        };
        Ok((0..5).map(thread).take(limit as usize).collect())
    }
    fn sms_messages(
        &self,
        thread: &str,
        before: Option<i64>,
        limit: u32,
    ) -> Result<Vec<nectarlink_core::SmsMessage>, String> {
        let message = |date: i64| nectarlink_core::SmsMessage {
            id: date.to_string(),
            thread: thread.to_owned(),
            address: "+15550100".into(),
            body: format!("at {date}"),
            date,
            outgoing: date % 2 == 0,
            status: None,
            parts: vec![nectarlink_core::SmsPart { id: "p1".into(), mime: "image/jpeg".into(), size: 3 }],
        };
        let newest = before.unwrap_or(100);
        Ok((0..newest).rev().take(limit as usize).map(message).collect())
    }
    fn sms_send(&self, to: &[String], body: &str) -> Result<(), String> {
        self.texts.lock().unwrap().push((to.to_vec(), body.to_owned()));
        Ok(())
    }
    fn sms_part(&self, id: &str) -> Result<(String, Vec<u8>), String> {
        if id == "p1" { Ok(("image/jpeg".into(), vec![1, 2, 3])) } else { Err("gone".into()) }
    }
    fn call_command(&self, id: &str, command: nectarlink_core::CallCommand) -> Result<(), String> {
        self.calls.lock().unwrap().push((id.to_owned(), command));
        Ok(())
    }
    fn call_log(
        &self,
        before: Option<i64>,
        limit: u32,
    ) -> Result<Vec<nectarlink_core::CallLogEntry>, String> {
        let entry = |date: i64| nectarlink_core::CallLogEntry {
            id: format!("log:{date}"),
            number: format!("+1555010{date}"),
            name: (date % 2 == 0).then(|| format!("Caller {date}")),
            direction: match date % 4 {
                0 => "incoming",
                1 => "outgoing",
                2 => "missed",
                _ => "rejected",
            }
            .into(),
            date,
            duration: if date % 2 == 0 { 42 } else { 0 },
            // Entry 3 has an oversized photo that the core should drop.
            photo: match date {
                4 => Some(vec![0xff, 0xd8]),
                3 => Some(vec![0; 20 * 1024]),
                _ => None,
            },
        };
        let newest = before.unwrap_or(5);
        Ok((0..newest).rev().take(limit as usize).map(entry).collect())
    }
    fn call_dial(&self, number: &str) -> Result<(), String> {
        self.dialed.lock().unwrap().push(number.to_owned());
        Ok(())
    }
    fn contacts(
        &self,
        query: Option<&str>,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<nectarlink_core::Contact>, String> {
        use nectarlink_core::{Contact, ContactNumber};
        let all = vec![
            Contact {
                id: "c1".into(),
                name: "Sam Rivera".into(),
                numbers: vec![ContactNumber { number: "+15550100".into(), label: Some("Mobile".into()) }],
                starred: true,
                photo: Some(vec![0xff, 0xd8]),
            },
            Contact {
                id: "c2".into(),
                name: "Alex Chen".into(),
                numbers: vec![ContactNumber { number: "+15550188".into(), label: Some("Mobile".into()) }],
                starred: true,
                // Oversized photo: should be dropped by core rather than dropping the contact.
                photo: Some(vec![0; 20 * 1024]),
            },
            Contact {
                id: "c3".into(),
                name: "Jordan Patel".into(),
                numbers: vec![ContactNumber { number: "+15550155".into(), label: Some("Work".into()) }],
                starred: false,
                photo: None,
            },
            Contact {
                id: "c4".into(),
                name: "Priya Nair".into(),
                numbers: vec![ContactNumber { number: "+15550172".into(), label: None }],
                starred: false,
                photo: None,
            },
        ];
        let q = query.map(str::to_lowercase);
        Ok(all
            .into_iter()
            .filter(|c| match &q {
                None => true,
                Some(q) => {
                    c.name.to_lowercase().contains(q)
                        || c.numbers.iter().any(|n| n.number.to_lowercase().contains(q))
                }
            })
            .skip(offset as usize)
            .take(limit as usize)
            .collect())
    }
    fn open_photo(&self, id: &str) -> Result<OutgoingFile, String> {
        let barrier = self.photo_barrier.lock().unwrap().clone();
        if let Some(barrier) = barrier {
            barrier.wait();
            barrier.wait();
        }
        let path = self.photos.lock().unwrap().get(id).cloned().ok_or("gone")?;
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        Ok(OutgoingFile { name, folder: None, source: FileSource::Path(path) })
    }
    fn photo_albums(&self) -> Result<Vec<nectarlink_core::PhotoAlbum>, String> {
        use nectarlink_core::PhotoAlbum;
        Ok(vec![
            PhotoAlbum { id: "cam".into(), name: "Camera".into(), count: 3, cover: Some("media:4".into()) },
            PhotoAlbum {
                id: "shots".into(),
                name: "Screenshots".into(),
                count: 2,
                cover: Some("media:3".into()),
            },
        ])
    }
    fn photo_list(
        &self,
        album: Option<&str>,
        before: Option<(i64, &str)>,
        limit: u32,
    ) -> Result<Vec<nectarlink_core::PhotoItem>, String> {
        use nectarlink_core::PhotoItem;
        let all = vec![
            PhotoItem {
                id: "media:4".into(),
                name: "IMG_4.jpg".into(),
                date: 4_000,
                size: 100,
                width: 1920,
                height: 1080,
                duration: None,
                album: Some("cam".into()),
            },
            PhotoItem {
                id: "media:3".into(),
                name: "Screenshot_3.png".into(),
                date: 3_000,
                size: 80,
                width: 1080,
                height: 2400,
                duration: None,
                album: Some("shots".into()),
            },
            PhotoItem {
                id: "media:2".into(),
                name: "VID_2.mp4".into(),
                // Taken the same millisecond as media:3, across a page break.
                date: 3_000,
                size: 5_000,
                width: 1920,
                height: 1080,
                duration: Some(14),
                album: Some("cam".into()),
            },
            PhotoItem {
                id: "media:1".into(),
                name: "IMG_1.jpg".into(),
                date: 1_000,
                size: 90,
                width: 1920,
                height: 1080,
                duration: None,
                album: Some("cam".into()),
            },
            PhotoItem {
                id: "media:0".into(),
                name: "Screenshot_0.png".into(),
                date: 500,
                size: 70,
                width: 1080,
                height: 2400,
                duration: None,
                album: Some("shots".into()),
            },
        ];
        Ok(all
            .into_iter()
            .filter(|i| album.is_none_or(|a| i.album.as_deref() == Some(a)))
            .filter(|i| before.is_none_or(|(date, id)| i.date < date || (i.date == date && *i.id < *id)))
            .take(limit as usize)
            .collect())
    }
    fn photo_thumbs(&self, ids: &[String]) -> Result<Vec<nectarlink_core::PhotoThumb>, String> {
        use nectarlink_core::PhotoThumb;
        Ok(ids
            .iter()
            .filter_map(|id| match id.as_str() {
                "media:4" | "media:3" | "media:2" | "media:1" | "media:0" => Some(PhotoThumb {
                    id: id.clone(),
                    data: vec![0xff, 0xd8, id.as_bytes().last().copied().unwrap()],
                }),
                "oversized" => Some(PhotoThumb {
                    id: id.clone(),
                    data: vec![0; nectarlink_core::PHOTO_MAX_THUMB_BYTES + 1],
                }),
                _ => None,
            })
            .collect())
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
    fn remote_input(
        &self,
        _peer: &nectarlink_core::DeviceId,
        input: nectarlink_core::RemoteInput,
    ) -> Result<(), String> {
        self.remote_inputs.lock().unwrap().push(input);
        Ok(())
    }
    fn set_phone_toggle(&self, id: &str, value: &nectarlink_core::PhoneToggleValue) -> Result<(), String> {
        self.phone_toggles.lock().unwrap().push((id.to_owned(), value.clone()));
        Ok(())
    }
}

#[derive(Debug, Default)]
struct ScreenSink {
    got: Mutex<Vec<String>>,
}

impl nectarlink_core::MirrorSink for ScreenSink {
    fn config(&self, config: nectarlink_core::MirrorConfig) {
        self.got.lock().unwrap().push(format!("config {}x{}", config.width, config.height));
    }
    fn packet(&self, keyframe: bool, time_us: u64, data: Vec<u8>) {
        self.got.lock().unwrap().push(format!(
            "{} {time_us} {}",
            if keyframe { "key" } else { "frame" },
            data.len()
        ));
    }
    fn ended(&self) {
        self.got.lock().unwrap().push("ended".into());
    }
    fn audio_config(&self, config: nectarlink_core::MirrorAudioConfig) {
        self.got.lock().unwrap().push(format!("sound {} Hz x{}", config.rate, config.channels));
    }
    fn audio(&self, time_us: u64, data: Vec<u8>) {
        self.got.lock().unwrap().push(format!("sound {time_us} {}", data.len()));
    }
    fn audio_ended(&self) {
        self.got.lock().unwrap().push("sound ended".into());
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
async fn second_nearby_pairing_is_refused_while_one_awaits_confirmation() {
    let (mut pc, mut phone_a, addrs) = start_nearby_setup().await;
    let pc_id = pc.node.device_id();
    let phone_b = device("Other", DeviceKind::Phone).await;
    phone_b.node.add_known_addrs(pc_id, &addrs);

    let initiator_a = phone_a.node.clone();
    let task_a = tokio::spawn(async move { initiator_a.pairing_start_nearby(pc_id).await });
    let (code_a, code_pc) = (sas_code(&mut phone_a).await, sas_code(&mut pc).await);
    assert_eq!(code_a, code_pc);

    // A second phone tries to pair while the PC is waiting for the user to
    // confirm the first phone's 6-digit code.
    let second = with_timeout("second nearby", phone_b.node.pairing_start_nearby(pc_id)).await;
    assert!(matches!(second, Err(Error::Denied)), "second ceremony is refused: {second:?}");

    // The first ceremony's decision channel is untouched and still completes.
    phone_a.node.pairing_confirm(true).unwrap();
    pc.node.pairing_confirm(true).unwrap();
    with_timeout("first nearby", task_a).await.unwrap().expect("first pairing succeeds");
    assert_eq!(pc.node.paired_devices().unwrap().len(), 1);
    assert_eq!(pc.node.paired_devices().unwrap()[0].id, phone_a.node.device_id());
    assert!(phone_b.node.paired_devices().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn too_many_failed_nearby_attempts_end_pairing_mode() {
    let (mut pc, mut phone, _) = start_nearby_setup().await;
    let pc_id = pc.node.device_id();
    for _ in 0..5 {
        let initiator = phone.node.clone();
        let task = tokio::spawn(async move { initiator.pairing_start_nearby(pc_id).await });
        sas_code(&mut phone).await;
        sas_code(&mut pc).await;
        phone.node.pairing_confirm(false).unwrap();
        let _ = with_timeout("nearby attempt", task).await.unwrap();
        wait_for(&mut pc, "attempt failed", |e| match e {
            NodeEvent::Pairing(PairingEvent::Failed(_)) => Some(()),
            _ => None,
        })
        .await;
    }
    assert!(!pc.node.is_pairing(), "pairing mode ends after 5 failed nearby attempts");
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
async fn the_phone_screen_streams_to_the_pc() {
    use nectarlink_core::{MirrorSend, PacketKind};
    let mut pc = device_with(
        "Desktop",
        DeviceKind::Desktop,
        &[nectarlink_core::MIRROR_VIEW, nectarlink_core::MIRROR_LISTEN],
    )
    .await;
    let mut phone = device_with(
        "Pixel",
        DeviceKind::Phone,
        &[
            nectarlink_core::MIRROR_CAPTURE,
            nectarlink_core::MIRROR_INPUT,
            nectarlink_core::MIRROR_AUDIO_PLAYBACK,
        ],
    )
    .await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    let options = nectarlink_core::MirrorStart {
        max_size: 1920,
        fps: 60,
        bitrate: 8_000_000,
        audio: true,
        session: 0,
        app: None,
    };
    with_timeout("start", pc.node.mirror_start(phone_id, options)).await.unwrap();
    assert_eq!(*phone.platform.mirror_asks.lock().unwrap(), ["start 1920 with sound"]);

    // The user agreed: the phone streams.
    let stream = Arc::new(with_timeout("open", phone.node.mirror_open(pc_id)).await.unwrap());
    let config =
        nectarlink_core::MirrorConfig { codec: "h264".into(), width: 1080, height: 2400, session: 0 };
    let sender = stream.clone();
    let sent = tokio::task::spawn_blocking(move || {
        [
            sender.send(PacketKind::Config, 0, config.to_cbor()),
            sender.send(PacketKind::Keyframe, 1, vec![0; 90_000]),
            sender.send(PacketKind::Frame, 16_667, vec![0; 4_000]),
        ]
    })
    .await
    .unwrap();
    assert!(sent.iter().all(|s| *s == MirrorSend::Queued), "{sent:?}");
    wait_for(&mut pc, "showing", |e| matches!(e, NodeEvent::Mirroring { on: true, .. }).then_some(())).await;
    let screen = pc.platform.screen.clone();
    let deadline = tokio::time::Instant::now() + WAIT;
    while screen.got.lock().unwrap().len() < 3 && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(*screen.got.lock().unwrap(), ["config 1080x2400", "key 1 90000", "frame 16667 4000"]);

    // And its sound, on a stream of its own.
    let sound = Arc::new(with_timeout("open sound", phone.node.mirror_open_audio(pc_id)).await.unwrap());
    let format = nectarlink_core::MirrorAudioConfig {
        codec: nectarlink_core::MIRROR_PCM.into(),
        rate: 48_000,
        channels: 2,
    };
    let sender = sound.clone();
    let sent = tokio::task::spawn_blocking(move || {
        [
            sender.send(PacketKind::Config, 0, format.to_cbor()),
            sender.send(PacketKind::Frame, 10_000, vec![0; 1_920]),
        ]
    })
    .await
    .unwrap();
    assert!(sent.iter().all(|s| *s == MirrorSend::Queued), "{sent:?}");
    let deadline = tokio::time::Instant::now() + WAIT;
    while screen.got.lock().unwrap().len() < 5 && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(screen.got.lock().unwrap()[3..], ["sound 48000 Hz x2", "sound 10000 1920"]);

    pc.node.mirror_keyframe(phone_id, 0).await;
    // The PC's mouse on the phone's screen; nonsense never leaves the PC.
    use nectarlink_core::{MirrorInput, TouchAction};
    pc.node
        .mirror_input(phone_id, 0, MirrorInput::Touch { action: TouchAction::Down, x: 0.5, y: 0.25 })
        .await
        .unwrap();
    pc.node.mirror_input(phone_id, 0, MirrorInput::Key { key: "back".into() }).await.unwrap();
    assert!(pc.node.mirror_input(phone_id, 0, MirrorInput::Key { key: "power".into() }).await.is_err());
    // The PC stops watching: the phone hears it, and its stream closes.
    pc.node.mirror_stop(phone_id, 0).await;
    wait_for(&mut pc, "stopped", |e| matches!(e, NodeEvent::Mirroring { on: false, .. }).then_some(())).await;
    let deadline = tokio::time::Instant::now() + WAIT;
    while !screen.got.lock().unwrap().iter().any(|g| g == "sound ended")
        && tokio::time::Instant::now() < deadline
    {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let got = screen.got.lock().unwrap().clone();
    assert!(got.iter().any(|g| g == "ended") && got.iter().any(|g| g == "sound ended"), "{got:?}");
    let deadline = tokio::time::Instant::now() + WAIT;
    while !stream.is_closed() && tokio::time::Instant::now() < deadline {
        let s = stream.clone();
        tokio::task::spawn_blocking(move || s.send(PacketKind::Frame, 0, vec![0])).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(stream.is_closed(), "the phone's stream closes");
    let deadline = tokio::time::Instant::now() + WAIT;
    while !sound.is_closed() && tokio::time::Instant::now() < deadline {
        let s = sound.clone();
        tokio::task::spawn_blocking(move || s.send(PacketKind::Frame, 0, vec![0])).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(sound.is_closed(), "and its sound stream");
    let asks = phone.platform.mirror_asks.lock().unwrap().clone();
    for wanted in ["keyframe 0", "Touch(Down) on 0", "Key(back) on 0", "stop 0"] {
        assert!(asks.iter().any(|a| a == wanted), "{wanted}: {asks:?}");
    }
    let shown = pc.platform.mirror_asks.lock().unwrap().clone();
    assert_eq!(shown, ["sink 0", "sink 0"], "the screen and its sound");
}

#[tokio::test(flavor = "multi_thread")]
async fn phone_apps_open_in_windows_of_their_own() {
    use nectarlink_core::{MirrorInput, MirrorSend, PacketKind};
    let mut pc = device_with("Desktop", DeviceKind::Desktop, &[nectarlink_core::MIRROR_VIEW]).await;
    let mut phone = device_with("Pixel", DeviceKind::Phone, &[nectarlink_core::MIRROR_CAPTURE]).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());
    let app = |session: u32| nectarlink_core::MirrorStart {
        max_size: 1280,
        fps: 30,
        bitrate: 4_000_000,
        audio: false,
        session,
        app: Some("com.example.chat".into()),
    };

    // Not Elevated yet: no apps, no app windows.
    assert!(pc.node.mirror_apps(phone_id).await.is_err());
    assert!(pc.node.mirror_start(phone_id, app(7)).await.is_err());
    phone
        .node
        .update_power(
            PowerLevel::Elevated,
            vec![nectarlink_core::MIRROR_CAPTURE.into(), nectarlink_core::MIRROR_VIRTUAL_DISPLAY.into()],
        )
        .await;
    let deadline = tokio::time::Instant::now() + WAIT;
    let apps = loop {
        match pc.node.mirror_apps(phone_id).await {
            Ok(apps) => break apps,
            Err(_) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await
            }
            Err(e) => panic!("no apps: {e}"),
        }
    };
    let listed: Vec<_> = apps.iter().map(|a| (a.label.as_str(), a.icon.is_some())).collect();
    assert_eq!(listed, [("Chat", true), ("Maps", false), ("notes", true)], "by name; big icons left out");

    // An app window is a session of its own; the screen's (0) can't have an app.
    assert!(pc.node.mirror_start(phone_id, app(0)).await.is_err());
    with_timeout("start", pc.node.mirror_start(phone_id, app(7))).await.unwrap();
    let stream = Arc::new(with_timeout("open", phone.node.mirror_open(pc_id)).await.unwrap());
    let config = nectarlink_core::MirrorConfig { codec: "h264".into(), width: 720, height: 1280, session: 7 };
    let sender = stream.clone();
    let sent = tokio::task::spawn_blocking(move || {
        [
            sender.send(PacketKind::Config, 0, config.to_cbor()),
            sender.send(PacketKind::Keyframe, 1, vec![0; 10]),
        ]
    })
    .await
    .unwrap();
    assert!(sent.iter().all(|s| *s == MirrorSend::Queued), "{sent:?}");
    let session = wait_for(&mut pc, "showing", |e| match e {
        NodeEvent::Mirroring { session, on: true, .. } => Some(*session),
        _ => None,
    })
    .await;
    assert_eq!(session, 7);
    assert!(matches!(pc.node.mirror_resize(phone_id, 0, 800, 600).await, Err(Error::Protocol(_))));
    assert!(matches!(pc.node.mirror_resize(phone_id, 7, 10, 600).await, Err(Error::Protocol(_))));
    pc.node.mirror_resize(phone_id, 7, 1024, 768).await.unwrap();
    pc.node.mirror_input(phone_id, 7, MirrorInput::Key { key: "back".into() }).await.unwrap();
    pc.node.mirror_keyframe(phone_id, 7).await;
    pc.node.mirror_stop(phone_id, 7).await;
    let ended = wait_for(&mut pc, "stopped", |e| match e {
        NodeEvent::Mirroring { session, on: false, .. } => Some(*session),
        _ => None,
    })
    .await;
    assert_eq!(ended, 7);
    // The phone handles some of these off the control stream, so give it a
    // moment to get to all of them.
    for wanted in
        ["start 1280 com.example.chat as 7", "resize 7 1024x768", "Key(back) on 7", "keyframe 7", "stop 7"]
    {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !phone.platform.mirror_asks.lock().unwrap().iter().any(|a| a == wanted) {
            let asks = phone.platform.mirror_asks.lock().unwrap().clone();
            assert!(std::time::Instant::now() < deadline, "{wanted}: {asks:?}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    assert_eq!(*pc.platform.mirror_asks.lock().unwrap(), ["sink 7"]);

    // The app closes on the phone: its window's stream ends at once, even
    // with no more pictures to send.
    with_timeout("start again", pc.node.mirror_start(phone_id, app(8))).await.unwrap();
    let stream = Arc::new(with_timeout("open again", phone.node.mirror_open(pc_id)).await.unwrap());
    let config = nectarlink_core::MirrorConfig { codec: "h264".into(), width: 720, height: 1280, session: 8 };
    let sender = stream.clone();
    tokio::task::spawn_blocking(move || sender.send(PacketKind::Config, 0, config.to_cbor())).await.unwrap();
    wait_for(&mut pc, "showing 8", |e| {
        matches!(e, NodeEvent::Mirroring { session: 8, on: true, .. }).then_some(())
    })
    .await;
    stream.close();
    wait_for(&mut pc, "ended 8", |e| {
        matches!(e, NodeEvent::Mirroring { session: 8, on: false, .. }).then_some(())
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_pc_reads_and_sends_texts_through_the_phone() {
    let mut pc = device_with("Desktop", DeviceKind::Desktop, &[nectarlink_core::SMS_SHOW]).await;
    let mut phone =
        device_with("Pixel", DeviceKind::Phone, &[nectarlink_core::SMS_READ, nectarlink_core::SMS_SEND])
            .await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    let threads = with_timeout("threads", pc.node.sms_threads(phone_id, 3)).await.unwrap();
    assert_eq!(threads.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["0", "1", "2"]);

    // Paging back through a conversation.
    let latest = pc.node.sms_messages(phone_id, "1".into(), None, 10).await.unwrap();
    assert_eq!((latest[0].date, latest.len()), (99, 10));
    let older = pc.node.sms_messages(phone_id, "1".into(), Some(latest[9].date), 10).await.unwrap();
    assert_eq!(older[0].date, 89);

    let (mime, data) = pc.node.sms_part(phone_id, "p1".into()).await.unwrap();
    assert_eq!((mime.as_str(), data.as_slice()), ("image/jpeg", [1u8, 2, 3].as_slice()));

    pc.node.sms_send(phone_id, vec!["+15550100".into()], "On my way".into()).await.unwrap();
    assert_eq!(
        *phone.platform.texts.lock().unwrap(),
        [(vec!["+15550100".to_owned()], "On my way".to_owned())]
    );
    assert!(matches!(pc.node.sms_send(phone_id, vec![], "x".into()).await, Err(Error::Protocol(_))));

    // The phone says what changed.
    phone.node.sms_changed(Some("1".into())).await;
    let changed = wait_for(&mut pc, "a change", |e| match e {
        NodeEvent::SmsChanged { device, thread } if *device == phone_id => Some(thread.clone()),
        _ => None,
    })
    .await;
    assert_eq!(changed.as_deref(), Some("1"));

    // The phone's user turned messages off for this PC.
    phone.node.set_device_toggle(pc_id, "messages", false).unwrap();
    assert!(matches!(pc.node.sms_threads(phone_id, 3).await, Err(Error::Denied)));
}

#[tokio::test(flavor = "multi_thread")]
async fn calls_show_on_the_pc_which_can_answer_them() {
    use nectarlink_core::{CallCommand, CallState};
    let mut pc = device_with("Desktop", DeviceKind::Desktop, &[nectarlink_core::CALLS_SHOW]).await;
    let mut phone = device_with(
        "Pixel",
        DeviceKind::Phone,
        &[nectarlink_core::CALLS_STATE, nectarlink_core::CALLS_CONTROL],
    )
    .await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());
    let ringing = CallState {
        id: "c1".into(),
        state: "ringing".into(),
        incoming: true,
        number: Some("+15550100".into()),
        name: Some("Sam".into()),
        photo: Some(vec![0xff, 0xd8]),
        missed: false,
        since: None,
        controls: None,
    };
    // The call in `state` (a PC that just connected may hear the earlier
    // one again).
    async fn next_call(pc: &mut TestDevice, phone_id: nectarlink_core::DeviceId, state: &str) -> CallState {
        wait_for(pc, state, |e| match e {
            NodeEvent::Call { device, call } if *device == phone_id && call.state == state => {
                Some(call.clone())
            }
            _ => None,
        })
        .await
    }

    phone.node.call_changed(ringing.clone()).await.unwrap();
    assert_eq!(next_call(&mut pc, phone_id, "ringing").await, ringing);
    with_timeout("answer", pc.node.call_command(phone_id, "c1".into(), CallCommand::Answer)).await.unwrap();
    assert_eq!(*phone.platform.calls.lock().unwrap(), [("c1".to_owned(), CallCommand::Answer)]);
    // Only the call in progress.
    assert!(matches!(
        pc.node.call_command(phone_id, "c0".into(), CallCommand::Decline).await,
        Err(Error::NotFound)
    ));

    let active = CallState { state: "active".into(), photo: None, ..ringing.clone() };
    phone.node.call_changed(active.clone()).await.unwrap();
    next_call(&mut pc, phone_id, "active").await;
    // An answered call doesn't ring anymore, but can be hung up.
    assert!(matches!(
        pc.node.call_command(phone_id, "c1".into(), CallCommand::Silence).await,
        Err(Error::NotFound)
    ));
    // The volume works on any phone; mute needs one that controls the call.
    pc.node.call_command(phone_id, "c1".into(), CallCommand::Volume(true)).await.unwrap();
    assert!(matches!(
        pc.node.call_command(phone_id, "c1".into(), CallCommand::Mute(true)).await,
        Err(Error::Unsupported)
    ));
    phone
        .node
        .update_power(
            PowerLevel::NotApplicable,
            vec![
                "media.control".into(),
                nectarlink_core::CALLS_STATE.into(),
                nectarlink_core::CALLS_CONTROL.into(),
                nectarlink_core::CALLS_IN_CALL.into(),
            ],
        )
        .await;
    pc.node.call_command(phone_id, "c1".into(), CallCommand::Mute(true)).await.unwrap();
    pc.node.call_command(phone_id, "c1".into(), CallCommand::Dtmf('5')).await.unwrap();
    let seen: Vec<CallCommand> = phone.platform.calls.lock().unwrap().iter().map(|(_, c)| *c).collect();
    assert_eq!(seen[1..], [CallCommand::Volume(true), CallCommand::Mute(true), CallCommand::Dtmf('5')]);
    pc.node.call_command(phone_id, "c1".into(), CallCommand::Decline).await.unwrap();

    let ended = CallState { state: "ended".into(), ..active };
    phone.node.call_changed(ended).await.unwrap();
    next_call(&mut pc, phone_id, "ended").await;
    assert!(matches!(
        pc.node.call_command(phone_id, "c1".into(), CallCommand::Decline).await,
        Err(Error::NotFound)
    ));

    // The phone's user turned calls off for this PC.
    phone.node.set_device_toggle(pc_id, "calls", false).unwrap();
    phone.node.call_changed(CallState { id: "c2".into(), ..ringing }).await.unwrap();
    assert!(matches!(
        pc.node.call_command(phone_id, "c2".into(), CallCommand::Answer).await,
        Err(Error::Denied)
    ));
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
    let interrupted =
        std::fs::metadata(dir.path().join("incoming").join(pc_id.to_string()).join(&id).join("0.part"))
            .unwrap()
            .len();
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

#[tokio::test(flavor = "multi_thread")]
async fn slow_rpc_does_not_block_control_stream() {
    let mut pc = device_with("Desktop", DeviceKind::Desktop, &[nectarlink_core::PHOTOS_SHOW]).await;
    let mut phone = device_with("Pixel", DeviceKind::Phone, &[nectarlink_core::PHOTOS_READ]).await;
    pair_qr(&mut pc, &mut phone).await;
    let phone_id = phone.node.device_id();

    let file = phone.dir.path().join("IMG_1.jpg");
    let pixels = b"jpeg-bytes";
    std::fs::write(&file, pixels).unwrap();
    phone.platform.photos.lock().unwrap().insert("media:1".into(), file);
    let photo = nectarlink_core::Photo {
        id: "media:1".into(),
        name: "IMG_1.jpg".into(),
        size: pixels.len() as u64,
        taken: 1_790_000_000,
        screenshot: true,
        thumb: vec![0xff, 0xd8, 0xff],
    };
    phone.node.photo_taken(photo).await.unwrap();
    wait_for(&mut pc, "the photo", |e| match e {
        NodeEvent::PhotoAdded { device, .. } if *device == phone_id => Some(()),
        _ => None,
    })
    .await;

    let barrier = Arc::new(std::sync::Barrier::new(2));
    *phone.platform.photo_barrier.lock().unwrap() = Some(barrier.clone());

    let pc_node = pc.node.clone();
    let get_task = tokio::spawn(async move { pc_node.fetch_photo(phone_id, "media:1".into()).await });

    // Wait until the phone's `open_photo` is blocked inside `photos.get`.
    tokio::task::spawn_blocking({
        let barrier = barrier.clone();
        move || barrier.wait()
    })
    .await
    .unwrap();

    // While `photos.get` is stuck, another control-stream RPC still completes.
    with_timeout("ring while photos.get is blocked", pc.node.ring(phone_id, true))
        .await
        .expect("ring succeeds without waiting for photos.get");

    // Release `open_photo` so `photos.get` finishes cleanly.
    tokio::task::spawn_blocking(move || barrier.wait()).await.unwrap();
    let transfer_id = with_timeout("photos.get finishes", get_task).await.unwrap().expect("photo sent");
    wait_transfer(&mut pc, &transfer_id, "photo received", saved).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn call_log_and_dial_work_and_respect_permissions_and_toggles() {
    let mut pc = device_with("Desktop", DeviceKind::Desktop, &[nectarlink_core::CALLS_SHOW]).await;
    let mut phone = device_with("Pixel", DeviceKind::Phone, &[nectarlink_core::CALLS_LOG]).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    let page1 = with_timeout("call log page 1", pc.node.call_log(phone_id, None, 3)).await.unwrap();
    assert_eq!(page1.len(), 3);
    assert_eq!((&*page1[0].id, &*page1[0].direction, page1[0].date), ("log:4", "incoming", 4));
    assert_eq!(page1[0].photo.as_deref(), Some(&[0xff, 0xd8][..]));
    // Entry 3 had an oversized photo: kept without its photo.
    assert_eq!((&*page1[1].id, &*page1[1].direction, page1[1].photo.as_ref()), ("log:3", "rejected", None));
    assert_eq!((&*page1[2].id, &*page1[2].direction), ("log:2", "missed"));

    let page2 = with_timeout("call log page 2", pc.node.call_log(phone_id, Some(2), 3)).await.unwrap();
    assert_eq!(page2.iter().map(|e| e.date).collect::<Vec<_>>(), [1, 0]);

    phone.node.call_log_changed().await;
    wait_for(&mut pc, "call log changed", |e| match e {
        NodeEvent::CallLogChanged { device } if *device == phone_id => Some(()),
        _ => None,
    })
    .await;

    // Without `call.dial` on the phone, dialing from the PC is refused as unsupported.
    assert!(matches!(pc.node.call_dial(phone_id, "+15550100".into()).await, Err(Error::Unsupported)));

    phone
        .node
        .update_power(
            PowerLevel::NotApplicable,
            vec![
                "media.control".into(),
                nectarlink_core::CALLS_LOG.into(),
                nectarlink_core::CALLS_DIAL.into(),
            ],
        )
        .await;
    wait_for(&mut pc, "call.dial unlocked", |e| match e {
        NodeEvent::Capabilities(m)
            if m.device == phone_id && m.state("calls.dial") == Some(FeatureState::Available) =>
        {
            Some(())
        }
        _ => None,
    })
    .await;

    with_timeout("dial", pc.node.call_dial(phone_id, "+15550100".into())).await.unwrap();
    assert_eq!(*phone.platform.dialed.lock().unwrap(), ["+15550100"]);

    // Turning `calls` off for this PC blocks both call log and dialing.
    phone.node.set_device_toggle(pc_id, "calls", false).unwrap();
    assert!(matches!(pc.node.call_log(phone_id, None, 3).await, Err(Error::Denied)));
    assert!(matches!(pc.node.call_dial(phone_id, "+15550100".into()).await, Err(Error::Denied)));
}

#[tokio::test(flavor = "multi_thread")]
async fn contacts_list_search_page_and_respect_permissions_and_toggles() {
    let mut pc = device_with("Desktop", DeviceKind::Desktop, &[nectarlink_core::CONTACTS_SHOW]).await;
    let mut phone = device_with("Pixel", DeviceKind::Phone, &[]).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    // Without `contacts.read` on the phone, listing contacts is unsupported.
    assert!(matches!(pc.node.contacts(phone_id, None, 0, 10).await, Err(Error::Unsupported)));

    phone
        .node
        .update_power(
            PowerLevel::NotApplicable,
            vec!["media.control".into(), nectarlink_core::CONTACTS_READ.into()],
        )
        .await;
    wait_for(&mut pc, "contacts.read unlocked", |e| match e {
        NodeEvent::Capabilities(m)
            if m.device == phone_id && m.state("contacts.read") == Some(FeatureState::Available) =>
        {
            Some(())
        }
        _ => None,
    })
    .await;

    let first_two = with_timeout("contacts page 1", pc.node.contacts(phone_id, None, 0, 2)).await.unwrap();
    assert_eq!(first_two.len(), 2);
    assert_eq!((&*first_two[0].name, first_two[0].starred), ("Sam Rivera", true));
    assert_eq!(first_two[0].photo.as_deref(), Some(&[0xff, 0xd8][..]));
    // Oversized photo on c2 was stripped rather than dropping the contact.
    assert_eq!((&*first_two[1].name, first_two[1].photo.as_ref()), ("Alex Chen", None));

    let rest = with_timeout("contacts page 2", pc.node.contacts(phone_id, None, 2, 10)).await.unwrap();
    assert_eq!(rest.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Jordan Patel", "Priya Nair"]);

    let searched = with_timeout("contacts search", pc.node.contacts(phone_id, Some("patel".into()), 0, 10))
        .await
        .unwrap();
    assert_eq!(searched.len(), 1);
    assert_eq!(searched[0].name, "Jordan Patel");

    phone.node.contacts_changed().await;
    wait_for(&mut pc, "contacts changed", |e| match e {
        NodeEvent::ContactsChanged { device } if *device == phone_id => Some(()),
        _ => None,
    })
    .await;

    // Turning `contacts` off for this PC refuses contact queries.
    phone.node.set_device_toggle(pc_id, "contacts", false).unwrap();
    assert!(matches!(pc.node.contacts(phone_id, None, 0, 10).await, Err(Error::Denied)));
}

#[tokio::test(flavor = "multi_thread")]
async fn gallery_albums_paging_thumbs_download_and_toggles() {
    let mut pc = device_with("Desktop", DeviceKind::Desktop, &[nectarlink_core::PHOTOS_SHOW]).await;
    let mut phone = device_with("Pixel", DeviceKind::Phone, &[]).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    // Without `photos.read` on the phone, gallery queries are unsupported.
    assert!(matches!(pc.node.photo_albums(phone_id).await, Err(Error::Unsupported)));
    assert!(matches!(pc.node.photo_list(phone_id, None, None, 10).await, Err(Error::Unsupported)));
    assert!(matches!(pc.node.photo_thumbs(phone_id, vec!["media:4".into()]).await, Err(Error::Unsupported)));

    phone
        .node
        .update_power(
            PowerLevel::NotApplicable,
            vec!["media.control".into(), nectarlink_core::PHOTOS_READ.into()],
        )
        .await;
    wait_for(&mut pc, "photos.read unlocked", |e| match e {
        NodeEvent::Capabilities(m)
            if m.device == phone_id && m.state("files.recent_photos") == Some(FeatureState::Available) =>
        {
            Some(())
        }
        _ => None,
    })
    .await;

    // Albums list.
    let albums = with_timeout("albums", pc.node.photo_albums(phone_id)).await.unwrap();
    assert_eq!(albums.len(), 2);
    assert_eq!((&*albums[0].id, &*albums[0].name, albums[0].count), ("cam", "Camera", 3));
    assert_eq!(albums[0].cover.as_deref(), Some("media:4"));
    assert_eq!((&*albums[1].id, &*albums[1].name, albums[1].count), ("shots", "Screenshots", 2));

    // Paging all items newest-first; an item sharing the last one's date
    // still comes on the next page.
    let page1 = with_timeout("list page 1", pc.node.photo_list(phone_id, None, None, 2)).await.unwrap();
    assert_eq!(page1.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["media:4", "media:3"]);
    let last = Some((page1[1].date, page1[1].id.clone()));
    let page2 = with_timeout("list page 2", pc.node.photo_list(phone_id, None, last, 2)).await.unwrap();
    assert_eq!(page2.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["media:2", "media:1"]);
    assert_eq!(page2[0].duration, Some(14));

    // Filtering by album.
    let shots = with_timeout("list album", pc.node.photo_list(phone_id, Some("shots".into()), None, 10))
        .await
        .unwrap();
    assert_eq!(shots.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["media:3", "media:0"]);

    // Batch thumbnails (oversized or missing ones are skipped without failing the batch).
    let thumbs = with_timeout(
        "thumbs",
        pc.node.photo_thumbs(
            phone_id,
            vec!["media:4".into(), "oversized".into(), "gone".into(), "media:2".into()],
        ),
    )
    .await
    .unwrap();
    assert_eq!(thumbs.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["media:4", "media:2"]);
    assert_eq!(thumbs[0].data, vec![0xff, 0xd8, b'4']);
    assert_eq!(thumbs[1].data, vec![0xff, 0xd8, b'2']);

    // Multi-file download via `fetch_photos`.
    let img4 = phone.dir.path().join("IMG_4.jpg");
    let vid2 = phone.dir.path().join("VID_2.mp4");
    let bytes4 = data(50_000, 11);
    let bytes2 = data(120_000, 12);
    std::fs::write(&img4, &bytes4).unwrap();
    std::fs::write(&vid2, &bytes2).unwrap();
    {
        let mut map = phone.platform.photos.lock().unwrap();
        map.insert("media:4".into(), img4);
        map.insert("media:2".into(), vid2);
    }
    let tid = with_timeout(
        "fetch_photos",
        pc.node.fetch_photos(phone_id, vec!["media:4".into(), "media:2".into()]),
    )
    .await
    .unwrap();
    let files = wait_transfer(&mut pc, &tid, "gallery files", saved).await;
    assert_eq!(files.len(), 2);
    assert_eq!(std::fs::read(&files[0]).unwrap(), bytes4);
    assert_eq!(std::fs::read(&files[1]).unwrap(), bytes2);

    // `photos.changed` notification.
    phone.node.photos_changed().await;
    wait_for(&mut pc, "photos changed", |e| match e {
        NodeEvent::PhotosChanged { device } if *device == phone_id => Some(()),
        _ => None,
    })
    .await;

    // Turning `photos` toggle off on the phone blocks albums, list, thumbs, and download.
    phone.node.set_device_toggle(pc_id, "photos", false).unwrap();
    assert!(matches!(pc.node.photo_albums(phone_id).await, Err(Error::Denied)));
    assert!(matches!(pc.node.photo_list(phone_id, None, None, 10).await, Err(Error::Denied)));
    assert!(matches!(pc.node.photo_thumbs(phone_id, vec!["media:4".into()]).await, Err(Error::Denied)));
    assert!(matches!(pc.node.fetch_photos(phone_id, vec!["media:4".into()]).await, Err(Error::Denied)));
}

#[tokio::test(flavor = "multi_thread")]
async fn remote_input_needs_toggle_and_delivers_motion_keys_and_slides() {
    use nectarlink_core::{
        ButtonAction, INPUT_INJECT, KeyMod, MouseButton, RemoteInput, SlideAction, remote_keys,
    };

    let mut pc = device("Desktop", DeviceKind::Desktop).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    // Without `input.inject` on the PC, remote input is unsupported.
    assert!(matches!(phone.node.remote_check(pc_id).await, Err(Error::Unsupported)));
    assert!(matches!(
        phone
            .node
            .remote_input(
                pc_id,
                RemoteInput::Button { button: MouseButton::Left, action: ButtonAction::Click }
            )
            .await,
        Err(Error::Unsupported)
    ));

    // The PC announces `input.inject`.
    pc.node.update_power(PowerLevel::NotApplicable, vec!["media.control".into(), INPUT_INJECT.into()]).await;
    wait_for(&mut phone, "input.remote available on phone", |e| match e {
        NodeEvent::Capabilities(m)
            if m.device == pc_id && m.state("input.remote") == Some(FeatureState::Available) =>
        {
            Some(())
        }
        _ => None,
    })
    .await;

    // On the PC, `remote_input` is off by default: the first ask is refused and
    // triggers the one-time prompt event on the PC.
    let toggles = pc.node.device_toggles(phone_id).unwrap();
    assert!(toggles.contains(&("remote_input", false)));
    assert!(matches!(phone.node.remote_check(pc_id).await, Err(Error::Denied)));
    wait_for(&mut pc, "remote input prompt", |e| match e {
        NodeEvent::RemoteInputRequested { device } if *device == phone_id => Some(()),
        _ => None,
    })
    .await;

    // While the toggle is off, discrete and datagram inputs are both refused.
    assert!(matches!(
        phone
            .node
            .remote_input(
                pc_id,
                RemoteInput::Button { button: MouseButton::Left, action: ButtonAction::Click }
            )
            .await,
        Err(Error::Denied)
    ));
    assert!(matches!(
        phone.node.remote_input(pc_id, RemoteInput::Move { dx: 10.0, dy: 20.0 }).await,
        Err(Error::Denied)
    ));
    phone.node.remote_move(pc_id, 10.0, 20.0).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(pc.platform.remote_inputs.lock().unwrap().is_empty(), "nothing injected while toggle is off");

    // The PC user enables `remote_input` for this phone.
    pc.node.set_device_toggle(phone_id, "remote_input", true).unwrap();
    with_timeout("remote_check allowed", phone.node.remote_check(pc_id)).await.unwrap();

    // Pointer motion (datagram), buttons, drag, scroll, text, keys, slides, and laser.
    phone.node.remote_move(pc_id, 18.5, -12.0).await.unwrap();
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        if pc.platform.remote_inputs.lock().unwrap().contains(&RemoteInput::Move { dx: 18.5, dy: -12.0 }) {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "timed out waiting for move datagram");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let events = vec![
        RemoteInput::Button { button: MouseButton::Left, action: ButtonAction::Click },
        RemoteInput::Button { button: MouseButton::Right, action: ButtonAction::Click },
        RemoteInput::Button { button: MouseButton::Left, action: ButtonAction::Down },
        RemoteInput::Button { button: MouseButton::Left, action: ButtonAction::Up },
        RemoteInput::Scroll { dx: 0.0, dy: -3.0 },
        RemoteInput::Text { text: "Hello Nectarlink 🐝".into() },
        RemoteInput::Key { key: remote_keys::ENTER.into(), mods: vec![] },
        RemoteInput::Key { key: "c".into(), mods: vec![KeyMod::Ctrl] },
        RemoteInput::Key { key: remote_keys::COPY.into(), mods: vec![] },
        RemoteInput::Slide { action: SlideAction::Next },
        RemoteInput::Slide { action: SlideAction::Previous },
        RemoteInput::Slide { action: SlideAction::Black },
    ];
    for ev in &events {
        with_timeout("remote_input", phone.node.remote_input(pc_id, ev.clone())).await.unwrap();
    }
    with_timeout("laser off", phone.node.remote_laser(pc_id, false, 0.0, 0.0)).await.unwrap();

    let recorded = pc.platform.remote_inputs.lock().unwrap().clone();
    for ev in &events {
        assert!(recorded.contains(ev), "missing {ev:?} in {recorded:?}");
    }
    assert!(recorded.contains(&RemoteInput::Laser { on: false, x: 0.0, y: 0.0 }));

    // Invalid inputs are rejected.
    assert!(
        phone
            .node
            .remote_input(pc_id, RemoteInput::Key { key: "unknown_key".into(), mods: vec![] })
            .await
            .is_err()
    );
    assert!(phone.node.remote_move(pc_id, 99999.0, 0.0).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn voice_recordings_arrive_with_markers_and_respect_capability_and_toggle() {
    use nectarlink_core::{RECORDER, RecordingMarker, TransferFailure};

    let mut pc = device("Desktop", DeviceKind::Desktop).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());
    let src = tempfile::tempdir().unwrap();
    let audio = data(64 * 1024 + 31, 42);
    let markers = vec![
        RecordingMarker { at_ms: 1_250, label: Some("Intro".into()) },
        RecordingMarker { at_ms: 4_800, label: None },
    ];

    // Without the `recorder` capability on the PC, a recording transfer is refused as unsupported.
    let tid = phone
        .node
        .send_recording(pc_id, outgoing(src.path(), "rec1.m4a", &audio), markers.clone())
        .await
        .unwrap();
    let failure = wait_transfer(&mut phone, &tid, "refused without capability", |t| match &t.state {
        TransferState::Failed(f) => Some(f.clone()),
        _ => None,
    })
    .await;
    assert!(
        matches!(failure, TransferFailure::Other(ref s) if s.to_ascii_lowercase().contains("unsupported")),
        "{failure:?}"
    );

    // The PC announces `recorder`.
    pc.node.update_power(PowerLevel::NotApplicable, vec![RECORDER.into()]).await;
    wait_for(&mut phone, "files.recordings available", |e| match e {
        NodeEvent::Capabilities(m)
            if m.device == pc_id && m.state("files.recordings") == Some(FeatureState::Available) =>
        {
            Some(())
        }
        _ => None,
    })
    .await;

    // The recording arrives on the PC and is reported with `recording == true` and its markers.
    let tid = phone
        .node
        .send_recording(pc_id, outgoing(src.path(), "Recording.m4a", &audio), markers.clone())
        .await
        .unwrap();
    let (received_paths, got_recording, got_markers) =
        wait_transfer(&mut pc, &tid, "recording received", |t| {
            saved(t).map(|paths| (paths, t.recording, t.markers.clone()))
        })
        .await;
    assert!(got_recording);
    assert_eq!(got_markers, markers);
    assert_eq!(received_paths.len(), 1);
    assert_eq!(std::fs::read(&received_paths[0]).unwrap(), audio);
    wait_transfer(&mut phone, &tid, "recording sent", saved).await;

    // Turning off the `recordings` toggle on the PC denies incoming recordings while regular files still work.
    pc.node.set_device_toggle(phone_id, "recordings", false).unwrap();
    let tid =
        phone.node.send_recording(pc_id, outgoing(src.path(), "rec2.m4a", &audio), Vec::new()).await.unwrap();
    let denied = wait_transfer(&mut phone, &tid, "denied by toggle", |t| match &t.state {
        TransferState::Failed(f) => Some(f.clone()),
        _ => None,
    })
    .await;
    assert_eq!(denied, TransferFailure::Denied);
}

#[tokio::test(flavor = "multi_thread")]
async fn phone_toggles_arrive_on_connect_change_and_respect_capabilities_and_toggle() {
    use nectarlink_core::{
        PhoneToggleValue, PhoneToggles, TOGGLES_BLUETOOTH, TOGGLES_BRIGHTNESS, TOGGLES_DND,
        TOGGLES_FLASHLIGHT, TOGGLES_READ, TOGGLES_RINGER, TOGGLES_SHOW, TOGGLES_VOLUME, TOGGLES_WIFI,
        ringer_modes, toggle_ids,
    };

    let mut pc = device_with("Desktop", DeviceKind::Desktop, &[TOGGLES_SHOW]).await;
    let mut phone = device_with(
        "Pixel",
        DeviceKind::Phone,
        &[TOGGLES_READ, TOGGLES_RINGER, TOGGLES_VOLUME, TOGGLES_FLASHLIGHT],
    )
    .await;

    let initial = PhoneToggles {
        dnd: false,
        ringer: ringer_modes::RING.into(),
        flashlight: Some(false),
        volume: 65,
        brightness: 40,
        wifi: true,
        bluetooth: true,
    };
    phone.node.toggles_changed(initial.clone()).await.unwrap();

    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    // Initial state arrives on connect.
    let got = wait_for(&mut pc, "initial toggles", |e| match e {
        NodeEvent::PhoneToggles { device, toggles } if *device == phone_id => Some(toggles.clone()),
        _ => None,
    })
    .await;
    assert_eq!(got, initial);

    // Volume, ringer (vibrate), and flashlight work at Basic without extra permissions.
    with_timeout(
        "set volume",
        pc.node.set_phone_toggle(phone_id, toggle_ids::VOLUME.into(), PhoneToggleValue::Level(80)),
    )
    .await
    .unwrap();
    with_timeout(
        "set ringer vibrate",
        pc.node.set_phone_toggle(
            phone_id,
            toggle_ids::RINGER.into(),
            PhoneToggleValue::Mode(ringer_modes::VIBRATE.into()),
        ),
    )
    .await
    .unwrap();
    with_timeout(
        "set flashlight on",
        pc.node.set_phone_toggle(phone_id, toggle_ids::FLASHLIGHT.into(), PhoneToggleValue::Bool(true)),
    )
    .await
    .unwrap();

    // Silent ringer and DND require `toggles.dnd`; brightness requires `toggles.brightness`;
    // Wi-Fi requires `toggles.wifi` (Elevated).
    assert!(matches!(
        pc.node
            .set_phone_toggle(
                phone_id,
                toggle_ids::RINGER.into(),
                PhoneToggleValue::Mode(ringer_modes::SILENT.into()),
            )
            .await,
        Err(Error::Unsupported)
    ));
    assert!(matches!(
        pc.node.set_phone_toggle(phone_id, toggle_ids::DND.into(), PhoneToggleValue::Bool(true)).await,
        Err(Error::Unsupported)
    ));
    assert!(matches!(
        pc.node.set_phone_toggle(phone_id, toggle_ids::BRIGHTNESS.into(), PhoneToggleValue::Level(50)).await,
        Err(Error::Unsupported)
    ));
    assert!(matches!(
        pc.node.set_phone_toggle(phone_id, toggle_ids::WIFI.into(), PhoneToggleValue::Bool(false)).await,
        Err(Error::Unsupported)
    ));

    // Invalid IDs or out-of-range values are rejected as protocol errors.
    assert!(matches!(
        pc.node.set_phone_toggle(phone_id, "unknown".into(), PhoneToggleValue::Bool(true)).await,
        Err(Error::Protocol(_))
    ));
    assert!(matches!(
        pc.node.set_phone_toggle(phone_id, toggle_ids::VOLUME.into(), PhoneToggleValue::Level(101)).await,
        Err(Error::Protocol(_))
    ));

    // Phone unlocks DND, brightness, and Elevated toggles.
    phone
        .node
        .update_power(
            PowerLevel::Elevated,
            vec![
                "media.control".into(),
                TOGGLES_READ.into(),
                TOGGLES_RINGER.into(),
                TOGGLES_VOLUME.into(),
                TOGGLES_FLASHLIGHT.into(),
                TOGGLES_DND.into(),
                TOGGLES_BRIGHTNESS.into(),
                TOGGLES_WIFI.into(),
                TOGGLES_BLUETOOTH.into(),
            ],
        )
        .await;
    wait_for(&mut pc, "toggles.wifi available", |e| match e {
        NodeEvent::Capabilities(m)
            if m.device == phone_id && m.state("toggles.wifi") == Some(FeatureState::Available) =>
        {
            Some(())
        }
        _ => None,
    })
    .await;

    with_timeout(
        "set ringer silent",
        pc.node.set_phone_toggle(
            phone_id,
            toggle_ids::RINGER.into(),
            PhoneToggleValue::Mode(ringer_modes::SILENT.into()),
        ),
    )
    .await
    .unwrap();
    with_timeout(
        "set dnd on",
        pc.node.set_phone_toggle(phone_id, toggle_ids::DND.into(), PhoneToggleValue::Bool(true)),
    )
    .await
    .unwrap();
    with_timeout(
        "set brightness",
        pc.node.set_phone_toggle(phone_id, toggle_ids::BRIGHTNESS.into(), PhoneToggleValue::Level(90)),
    )
    .await
    .unwrap();
    with_timeout(
        "set wifi off",
        pc.node.set_phone_toggle(phone_id, toggle_ids::WIFI.into(), PhoneToggleValue::Bool(false)),
    )
    .await
    .unwrap();

    let recorded = phone.platform.phone_toggles.lock().unwrap().clone();
    assert_eq!(
        recorded,
        vec![
            (toggle_ids::VOLUME.into(), PhoneToggleValue::Level(80)),
            (toggle_ids::RINGER.into(), PhoneToggleValue::Mode(ringer_modes::VIBRATE.into())),
            (toggle_ids::FLASHLIGHT.into(), PhoneToggleValue::Bool(true)),
            (toggle_ids::RINGER.into(), PhoneToggleValue::Mode(ringer_modes::SILENT.into())),
            (toggle_ids::DND.into(), PhoneToggleValue::Bool(true)),
            (toggle_ids::BRIGHTNESS.into(), PhoneToggleValue::Level(90)),
            (toggle_ids::WIFI.into(), PhoneToggleValue::Bool(false)),
        ]
    );

    // Phone pushes updated toggles state; PC receives it.
    let updated = PhoneToggles {
        dnd: true,
        ringer: ringer_modes::SILENT.into(),
        flashlight: Some(true),
        volume: 80,
        brightness: 90,
        wifi: false,
        bluetooth: true,
    };
    phone.node.toggles_changed(updated.clone()).await.unwrap();
    let got_updated = wait_for(&mut pc, "updated toggles", |e| match e {
        NodeEvent::PhoneToggles { device, toggles } if *device == phone_id && *toggles == updated => {
            Some(toggles.clone())
        }
        _ => None,
    })
    .await;
    assert_eq!(got_updated, updated);

    // Turning off the `toggles` device toggle on the phone refuses `phone.toggle.set`.
    phone.node.set_device_toggle(pc_id, "toggles", false).unwrap();
    assert!(matches!(
        pc.node.set_phone_toggle(phone_id, toggle_ids::VOLUME.into(), PhoneToggleValue::Level(50)).await,
        Err(Error::Denied)
    ));
}
