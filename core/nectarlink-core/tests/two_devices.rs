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

type MmsRecord = (Vec<String>, String, Vec<nectarlink_core::SmsAttachment>);
type RichClipboardRecord = (String, Option<String>, Option<String>);

#[derive(Debug, Default)]
struct RecordingPlatform {
    rings: Mutex<Vec<bool>>,
    dismissed: Mutex<Vec<String>>,
    actions: Mutex<Vec<(String, String, Option<String>)>>,
    clipboard: Mutex<Vec<String>>,
    clipboard_rich: Mutex<Vec<RichClipboardRecord>>,
    images: Mutex<Vec<(String, Vec<u8>)>>,
    media: Mutex<Vec<(String, MediaAction, Option<u64>)>>,
    power: Mutex<Vec<PowerAction>>,
    links: Mutex<Vec<String>>,
    calls: Mutex<Vec<(String, nectarlink_core::CallCommand)>>,
    dialed: Mutex<Vec<String>>,
    texts: Mutex<Vec<(Vec<String>, String)>>,
    mms_sent: Mutex<Vec<MmsRecord>>,
    mirror_asks: Mutex<Vec<String>>,
    mirror_reject: std::sync::atomic::AtomicBool,
    watch_offers: Mutex<Vec<String>>,
    mic_asks: Mutex<Vec<String>>,
    mic: Arc<TestMicSink>,
    /// What a PC got of a phone's screen.
    screen: Arc<ScreenSink>,
    webcam_asks: Mutex<Vec<String>>,
    webcam: Arc<TestWebcamSink>,
    camera_asks: Mutex<Vec<String>>,
    /// Photos `open_photo` finds, by ID.
    photos: Mutex<std::collections::HashMap<String, std::path::PathBuf>>,
    /// Optional barrier `open_photo` waits on to simulate a slow platform call.
    photo_barrier: Mutex<Option<Arc<std::sync::Barrier>>>,
    remote_inputs: Mutex<Vec<nectarlink_core::RemoteInput>>,
    phone_toggles: Mutex<Vec<(String, nectarlink_core::PhoneToggleValue)>>,
    deck_presses: Mutex<Vec<String>>,
    pc_audio_changes: Mutex<Vec<(Option<u8>, Option<bool>)>>,
    tasks: Mutex<Vec<nectarlink_core::TaskNotify>>,
    storage: Mutex<Option<nectarlink_core::FolderStorage>>,
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
    fn set_clipboard_rich(&self, text: &str, html: Option<&str>, rtf: Option<&str>) -> Result<(), String> {
        self.clipboard.lock().unwrap().push(text.to_owned());
        self.clipboard_rich.lock().unwrap().push((
            text.to_owned(),
            html.map(str::to_owned),
            rtf.map(str::to_owned),
        ));
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
    fn task_notify(
        &self,
        _from: &nectarlink_core::DeviceId,
        task: &nectarlink_core::TaskNotify,
    ) -> Result<(), String> {
        self.tasks.lock().unwrap().push(task.clone());
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
        if self.mirror_reject.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("consent declined".into());
        }
        let sound = if options.audio { " with sound" } else { "" };
        let app = options.app.as_deref().map(|a| format!(" {a} as {}", options.session)).unwrap_or_default();
        let watch = options.watch.as_deref().map(|p| format!(" watching {p}")).unwrap_or_default();
        let only = if options.sound_only { ", sound only" } else { "" };
        self.mirror_asks.lock().unwrap().push(format!("start {}{sound}{only}{app}{watch}", options.max_size));
        Ok(())
    }
    fn watch_offered(
        &self,
        _from: &nectarlink_core::DeviceId,
        player: &str,
        title: Option<&str>,
        file: Option<&nectarlink_core::WatchFile>,
    ) -> Result<(), String> {
        let file = file.map(|f| format!(" [{} {} at {}]", f.name, f.size, f.position)).unwrap_or_default();
        self.watch_offers.lock().unwrap().push(format!("{player}: {}{file}", title.unwrap_or("")));
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
    fn mirror_power_requested(&self, _peer: &nectarlink_core::DeviceId, stay_awake: bool, screen_off: bool) {
        self.mirror_asks.lock().unwrap().push(format!("power awake={stay_awake} off={screen_off}"));
    }
    fn mirror_input(
        &self,
        _peer: &nectarlink_core::DeviceId,
        session: u32,
        input: nectarlink_core::MirrorInput,
    ) {
        self.mirror_asks.lock().unwrap().push(format!("{input:?} on {session}"));
    }
    fn mic_sink(&self, _peer: &nectarlink_core::DeviceId) -> Option<Arc<dyn nectarlink_core::MicSink>> {
        Some(self.mic.clone())
    }
    fn mic_requested(&self, _peer: &nectarlink_core::DeviceId) -> Result<(), String> {
        self.mic_asks.lock().unwrap().push("start".into());
        Ok(())
    }
    fn mic_stop_requested(&self, _peer: &nectarlink_core::DeviceId) {
        self.mic_asks.lock().unwrap().push("stop".into());
    }
    fn webcam_sink(&self, _peer: &nectarlink_core::DeviceId) -> Option<Arc<dyn nectarlink_core::WebcamSink>> {
        self.webcam_asks.lock().unwrap().push("sink".into());
        Some(self.webcam.clone())
    }
    fn webcam_requested(
        &self,
        _peer: &nectarlink_core::DeviceId,
        options: &nectarlink_core::WebcamStart,
    ) -> Result<(), String> {
        self.webcam_asks
            .lock()
            .unwrap()
            .push(format!("start {} {}x{} @ {}", options.camera, options.width, options.height, options.fps));
        Ok(())
    }
    fn webcam_stop_requested(&self, _peer: &nectarlink_core::DeviceId) {
        self.webcam_asks.lock().unwrap().push("stop".into());
    }
    fn webcam_keyframe_requested(&self, _peer: &nectarlink_core::DeviceId) {
        self.webcam_asks.lock().unwrap().push("keyframe".into());
    }
    fn camera_capture_requested(
        &self,
        _peer: &nectarlink_core::DeviceId,
        request: &nectarlink_core::CameraCaptureRequest,
    ) -> Result<(), String> {
        self.camera_asks.lock().unwrap().push(format!("request {} {:?}", request.request_id, request.mode));
        Ok(())
    }
    fn camera_capture_cancelled(&self, _peer: &nectarlink_core::DeviceId, request_id: &str) {
        self.camera_asks.lock().unwrap().push(format!("cancel {request_id}"));
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
    fn send_sms_with_attachments(
        &self,
        to: &[String],
        body: &str,
        attachments: &[nectarlink_core::SmsAttachment],
    ) -> Result<(), String> {
        if attachments.is_empty() {
            return self.sms_send(to, body);
        }
        self.mms_sent.lock().unwrap().push((to.to_vec(), body.to_owned(), attachments.to_vec()));
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
    fn deck_press(&self, _from: &nectarlink_core::DeviceId, tile: &str) -> Result<(), String> {
        self.deck_presses.lock().unwrap().push(tile.to_owned());
        Ok(())
    }
    fn set_pc_audio(
        &self,
        _from: &nectarlink_core::DeviceId,
        volume: Option<u8>,
        muted: Option<bool>,
    ) -> Result<(), String> {
        self.pc_audio_changes.lock().unwrap().push((volume, muted));
        Ok(())
    }
    fn storage_list(
        &self,
        path: &str,
    ) -> Result<Vec<nectarlink_core::StorageEntry>, nectarlink_core::StorageError> {
        let guard = self.storage.lock().unwrap();
        let storage = guard.as_ref().ok_or(nectarlink_core::StorageError::Unsupported)?;
        storage.list(path)
    }
    fn storage_open_read(
        &self,
        path: &str,
    ) -> Result<nectarlink_core::StorageReadFile, nectarlink_core::StorageError> {
        let guard = self.storage.lock().unwrap();
        let storage = guard.as_ref().ok_or(nectarlink_core::StorageError::Unsupported)?;
        storage.open_read(path)
    }
    fn storage_write(
        &self,
        path: &str,
        staged: &std::path::Path,
        modified: Option<i64>,
    ) -> Result<nectarlink_core::StorageWriteDone, nectarlink_core::StorageError> {
        let guard = self.storage.lock().unwrap();
        let storage = guard.as_ref().ok_or(nectarlink_core::StorageError::Unsupported)?;
        storage.write(path, staged, modified)
    }
    fn storage_mkdir(&self, path: &str) -> Result<(), nectarlink_core::StorageError> {
        let guard = self.storage.lock().unwrap();
        let storage = guard.as_ref().ok_or(nectarlink_core::StorageError::Unsupported)?;
        storage.mkdir(path)
    }
    fn storage_rename(&self, from: &str, to: &str) -> Result<(), nectarlink_core::StorageError> {
        let guard = self.storage.lock().unwrap();
        let storage = guard.as_ref().ok_or(nectarlink_core::StorageError::Unsupported)?;
        storage.rename(from, to)
    }
    fn storage_delete(&self, path: &str, confirmed: bool) -> Result<(), nectarlink_core::StorageError> {
        let guard = self.storage.lock().unwrap();
        let storage = guard.as_ref().ok_or(nectarlink_core::StorageError::Unsupported)?;
        storage.delete(path, confirmed)
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

#[derive(Debug, Default)]
struct TestWebcamSink {
    got: Mutex<Vec<String>>,
}

impl nectarlink_core::WebcamSink for TestWebcamSink {
    fn config(&self, config: nectarlink_core::WebcamConfig) {
        self.got
            .lock()
            .unwrap()
            .push(format!("config {} {}x{} @ {}", config.camera, config.width, config.height, config.fps));
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
}

#[derive(Debug, Default)]
struct TestMicSink {
    got: Mutex<Vec<String>>,
}

impl nectarlink_core::MicSink for TestMicSink {
    fn config(&self, config: nectarlink_core::MirrorAudioConfig) {
        self.got.lock().unwrap().push(format!("config {} Hz x{}", config.rate, config.channels));
    }
    fn audio(&self, _time_us: u64, data: Vec<u8>) {
        self.got.lock().unwrap().push(format!("audio {}", data.len()));
    }
    fn ended(&self) {
        self.got.lock().unwrap().push("ended".into());
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
        screen: None,
        install_id: None,
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

    let battery = Battery {
        level: 82,
        charging: true,
        plugged: Some("usb".into()),
        full_in: Some(28),
        cell_bars: Some(4),
        cell_type: Some("5G".into()),
        wifi_bars: Some(3),
    };
    phone.node.update_battery(battery.clone()).await;
    let received = wait_for(&mut pc, "battery", |e| match e {
        NodeEvent::Battery { device, battery } if *device == phone_id => Some(battery.clone()),
        _ => None,
    })
    .await;
    assert_eq!(received, battery);

    let mut renamed = info("Pixel 9 Pro", DeviceKind::Phone);
    renamed.accent = Some(0xFF65558F);
    renamed.screen = Some(nectarlink_core::ScreenShape {
        v: 1,
        aspect: 0.45,
        corners: Some(nectarlink_core::ScreenCorners { tl: 0.088, tr: 0.088, br: 0.088, bl: 0.088 }),
        cutouts: vec![nectarlink_core::ScreenRect { x: 0.46, y: 0.018, w: 0.08, h: 0.036 }],
        cutout_path: None,
    });
    phone.node.update_device_info(renamed.clone()).await;
    let info = wait_for(&mut pc, "rename", |e| match e {
        NodeEvent::PeerInfoChanged { device, info } if *device == phone_id && info.name == "Pixel 9 Pro" => {
            Some(info.clone())
        }
        _ => None,
    })
    .await;
    assert_eq!(info, renamed);
    let stored = &pc.node.paired_devices().unwrap()[0].info;
    assert_eq!(stored.name, "Pixel 9 Pro", "rename is persisted");
    assert_eq!(stored.screen, renamed.screen, "screen geometry is persisted");
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

/// "Stop reconnecting" (GitHub #48): the device disconnects and stays
/// disconnected, though the other side keeps dialing, across a restart too,
/// until it's turned back on.
#[tokio::test(flavor = "multi_thread")]
async fn stop_reconnecting_keeps_a_device_disconnected_until_turned_back_on() {
    let mut pc = device("Desktop", DeviceKind::Desktop).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());
    let offline = |dev: &TestDevice| {
        dev.node.paired_devices().unwrap().iter().all(|d| !matches!(d.link, LinkState::Online { .. }))
    };

    phone.node.set_reconnect(pc_id, false).unwrap();
    wait_for(&mut phone, "reconnect off", |e| match e {
        NodeEvent::ReconnectChanged { device, on: false } if *device == pc_id => Some(()),
        _ => None,
    })
    .await;
    wait_for(&mut pc, "peer offline", |e| match e {
        NodeEvent::LinkChanged { device, link: LinkState::Offline { .. } } if *device == phone_id => Some(()),
        _ => None,
    })
    .await;
    assert!(!phone.node.paired_devices().unwrap()[0].reconnect);
    assert!(pc.node.paired_devices().unwrap()[0].reconnect, "only the phone stopped");
    // The PC dials again with its backoff (1 s, then 2 s); the phone refuses.
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(offline(&pc) && offline(&phone), "still disconnected while the PC keeps dialing");

    // Kept across a restart.
    phone.node.shutdown().await;
    let TestDevice { node: old_node, dir, .. } = phone;
    drop(old_node);
    let mut phone = start_in(dir, "Pixel", DeviceKind::Phone).await;
    assert!(!phone.node.paired_devices().unwrap()[0].reconnect, "the setting survives a restart");
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert!(offline(&pc) && offline(&phone), "still disconnected after the restart");

    // Reconnect: it dials right away.
    phone.node.set_reconnect(pc_id, true).unwrap();
    wait_online(&mut phone, pc_id).await;
    wait_online(&mut pc, phone_id).await;
    assert!(phone.node.paired_devices().unwrap()[0].reconnect);
    with_timeout("ring after reconnecting", pc.node.ring(phone_id, true)).await.expect("ring works again");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_phone_stopped_and_started_again_in_the_same_process_reconnects() {
    // With its background connection off, the phone app stops the node when it goes to the
    // background and starts a new one when it's opened, in the same process, while the old
    // handle may still be around. Stopping has to let go of everything a new node binds.
    let usb_base = loop {
        let base = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
        let Some(last) = base.checked_add(nectarlink_core::USB_PORT_COUNT) else { continue };
        if (base..=last).all(|p| std::net::TcpListener::bind(("127.0.0.1", p)).is_ok()) {
            break base;
        }
    };
    let mut pc = device("Desktop", DeviceKind::Desktop).await;
    let mut phone = start_usb_device("Pixel", DeviceKind::Phone, Some(usb_base), &[]).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());
    let usb_port = phone.node.usb_port().expect("the USB listener is up");

    for round in 0..3 {
        phone.node.shutdown().await;
        wait_for(&mut pc, "the phone goes offline", |e| match e {
            NodeEvent::LinkChanged { device, link: LinkState::Offline { .. } } if *device == phone_id => {
                Some(())
            }
            _ => None,
        })
        .await;
        assert!(phone.node.paired_devices().is_ok(), "a stopped node still answers what it stores");
        assert!(matches!(phone.node.ring(pc_id, true).await, Err(Error::Offline)), "and has no connections");

        let TestDevice { node: stopped, dir, .. } = phone;
        phone = start_usb_device_in(dir, "Pixel", DeviceKind::Phone, Some(usb_base), &[]).await;
        drop(stopped);
        assert_eq!(phone.node.device_id(), phone_id, "round {round}: same identity");
        assert_eq!(phone.node.usb_port(), Some(usb_port), "round {round}: the USB port was let go");

        wait_online(&mut phone, pc_id).await;
        wait_online(&mut pc, phone_id).await;
        with_timeout("ring after starting again", pc.node.ring(phone_id, round % 2 == 0))
            .await
            .expect("connected again");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn mirror_open_waits_for_a_connection_that_is_coming_back() {
    let pc_offers = [nectarlink_core::MIRROR_VIEW, nectarlink_core::MIRROR_LISTEN];
    let mut pc = device_with("Desktop", DeviceKind::Desktop, &pc_offers).await;
    let mut phone = device_with("Pixel", DeviceKind::Phone, &[nectarlink_core::MIRROR_CAPTURE]).await;
    pair_qr(&mut pc, &mut phone).await;
    let pc_id = pc.node.device_id();

    // The PC drops off right after asking for the screen.
    pc.node.shutdown().await;
    wait_for(&mut phone, "pc offline", |e| match e {
        NodeEvent::LinkChanged { device, link: LinkState::Offline { .. } } if *device == pc_id => Some(()),
        _ => None,
    })
    .await;

    // If it stays away, opening gives up after a while instead of hanging.
    let started = std::time::Instant::now();
    let gone = with_timeout("open while away", phone.node.mirror_open(pc_id)).await;
    assert!(matches!(gone, Err(Error::Offline)), "{:?}", gone.err());
    assert!(started.elapsed() >= Duration::from_secs(5), "waits for the PC first");

    // If it comes back meanwhile, the stream opens on the new connection.
    let TestDevice { node: old_pc, dir, .. } = pc;
    drop(old_pc);
    let (opened, _pc) =
        tokio::join!(with_timeout("open while reconnecting", phone.node.mirror_open(pc_id)), async {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let pc = start_in(dir, "Desktop", DeviceKind::Desktop).await;
            let mut caps = vec!["media.control".to_owned()];
            caps.extend(pc_offers.iter().map(|c| (*c).to_owned()));
            pc.node.update_power(PowerLevel::NotApplicable, caps).await;
            pc
        });
    assert!(opened.is_ok(), "opens once the PC is back: {:?}", opened.err());
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
        live: None,
        conversation: None,
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

    // Rich text (HTML + RTF) accompanies plain text both ways.
    with_timeout(
        "rich pc to phone",
        pc.node.send_clipboard_rich(
            phone_id,
            "Hello world".into(),
            Some("<b>Hello</b> world".into()),
            Some("{\\rtf1\\ansi\\b Hello\\b0  world}".into()),
        ),
    )
    .await
    .expect("rich sent");
    assert_eq!(
        phone.platform.clipboard_rich.lock().unwrap().last(),
        Some(&(
            "Hello world".to_owned(),
            Some("<b>Hello</b> world".to_owned()),
            Some("{\\rtf1\\ansi\\b Hello\\b0  world}".to_owned()),
        )),
    );

    // Oversized rich text markup is dropped so the plain text still arrives.
    let big_rtf = "r".repeat(nectarlink_core::CLIP_MAX_BYTES);
    with_timeout(
        "oversized rtf dropped",
        pc.node.send_clipboard_rich(
            phone_id,
            "Plain survives".into(),
            Some("<i>Plain</i> survives".into()),
            Some(big_rtf),
        ),
    )
    .await
    .expect("plain + html sent");
    assert_eq!(
        phone.platform.clipboard_rich.lock().unwrap().last(),
        Some(&("Plain survives".to_owned(), Some("<i>Plain</i> survives".to_owned()), None)),
    );

    // The receiver turned the clipboard off for the sender.
    phone.node.set_device_toggle(pc_id, "clipboard", false).unwrap();
    let denied = with_timeout("denied", pc.node.send_clipboard(phone_id, "nope".into())).await;
    assert!(matches!(denied, Err(Error::Denied)), "{denied:?}");
    // The sender turned it off: nothing leaves.
    let not_sent = with_timeout("not sent", phone.node.send_clipboard(pc_id, "nope".into())).await;
    assert!(matches!(not_sent, Err(Error::Denied)), "{not_sent:?}");
    assert_eq!(phone.platform.clipboard.lock().unwrap().len(), 3);

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

#[tokio::test(flavor = "multi_thread")]
async fn clipboard_history_tracks_text_and_images_and_excludes_otps() {
    let mut pc = device("Desktop", DeviceKind::Desktop).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    // Text clip sent PC -> phone is recorded on both sides (outgoing on PC, incoming on phone).
    with_timeout("pc to phone text", pc.node.send_clipboard(phone_id, "https://nectarlink.dev/docs".into()))
        .await
        .expect("sent");
    wait_for(&mut phone, "clipboard received", |e| match e {
        NodeEvent::ClipboardReceived { device } if *device == pc_id => Some(()),
        _ => None,
    })
    .await;

    // OTP clip is never recorded in clipboard history on either side.
    with_timeout("otp clip", phone.node.send_clipboard(pc_id, "482913".into())).await.expect("sent");
    wait_for(&mut pc, "otp received", |e| match e {
        NodeEvent::ClipboardReceived { device } if *device == phone_id => Some(()),
        _ => None,
    })
    .await;

    // Image clip sent phone -> PC is recorded on both sides.
    let img = vec![0x89, b'P', b'N', b'G', 1, 2, 3, 4];
    with_timeout(
        "phone to pc image",
        phone.node.send_clipboard_image(pc_id, "image/png".into(), img.clone()),
    )
    .await
    .expect("sent");
    wait_for(&mut pc, "image received", |e| match e {
        NodeEvent::ClipboardReceived { device } if *device == phone_id => Some(()),
        _ => None,
    })
    .await;

    let pc_hist = pc.node.clipboard_history(None);
    assert_eq!(pc_hist.len(), 2, "OTP clip was excluded from PC history");
    assert_eq!(pc_hist[0].kind, nectarlink_core::ClipboardItemKind::Image);
    assert!(pc_hist[0].incoming);
    assert_eq!(pc_hist[0].device_name, "Pixel");
    assert_eq!(pc.node.clipboard_history_image(&pc_hist[0].id), Some(("image/png".to_owned(), img.clone())));
    assert_eq!(pc_hist[1].kind, nectarlink_core::ClipboardItemKind::Text);
    assert_eq!(pc_hist[1].text, "https://nectarlink.dev/docs");
    assert!(!pc_hist[1].incoming);

    let phone_hist = phone.node.clipboard_history(None);
    assert_eq!(phone_hist.len(), 2, "OTP clip was excluded from phone history");
    assert!(!phone_hist[0].incoming);
    assert!(phone_hist[1].incoming);

    // Search, pin, copy again, delete, and clear all.
    assert_eq!(pc.node.clipboard_history(Some("nectarlink")).len(), 1);
    let text_id = pc_hist[1].id.clone();
    let img_id = pc_hist[0].id.clone();
    assert!(pc.node.pin_clipboard_history(&text_id, true));
    assert_eq!(pc.node.clipboard_history(None)[0].id, text_id);

    pc.node.copy_clipboard_history(&text_id).unwrap();
    assert_eq!(
        pc.platform.clipboard.lock().unwrap().last().map(String::as_str),
        Some("https://nectarlink.dev/docs")
    );

    assert!(pc.node.delete_clipboard_history(&img_id));
    assert_eq!(pc.node.clipboard_history_image(&img_id), None);
    pc.node.clear_clipboard_history().unwrap();
    assert!(pc.node.clipboard_history(None).is_empty());
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

/// The phone's sound on its own (#94): the PC asks for the sound alone, the
/// phone gets that request, and its sound stream plays without any video. A
/// phone that doesn't offer it is never asked to share its screen instead.
#[tokio::test(flavor = "multi_thread")]
async fn the_phone_sound_plays_on_the_pc_without_its_screen() {
    use nectarlink_core::{MirrorSend, PacketKind};
    let sound_only = nectarlink_core::MirrorStart {
        max_size: 0,
        fps: 0,
        bitrate: 0,
        audio: true,
        session: 0,
        app: None,
        stay_awake: false,
        screen_off: false,
        watch: None,
        sound_only: true,
    };

    // An older phone: no mirror.sound, so the request is refused, not
    // turned into screen sharing.
    let mut pc = device_with("Desktop", DeviceKind::Desktop, &[nectarlink_core::MIRROR_LISTEN]).await;
    let mut old_phone = device_with(
        "Old phone",
        DeviceKind::Phone,
        &[nectarlink_core::MIRROR_CAPTURE, nectarlink_core::MIRROR_AUDIO_PLAYBACK],
    )
    .await;
    pair_qr(&mut pc, &mut old_phone).await;
    let refused = with_timeout(
        "start on an old phone",
        pc.node.mirror_start(old_phone.node.device_id(), sound_only.clone()),
    )
    .await;
    assert!(matches!(refused, Err(Error::Unsupported)), "{refused:?}");
    assert!(old_phone.platform.mirror_asks.lock().unwrap().is_empty(), "the old phone was never asked");

    let mut phone = device_with(
        "Pixel",
        DeviceKind::Phone,
        &[
            nectarlink_core::MIRROR_CAPTURE,
            nectarlink_core::MIRROR_AUDIO_PLAYBACK,
            nectarlink_core::MIRROR_SOUND,
        ],
    )
    .await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());
    with_timeout("start", pc.node.mirror_start(phone_id, sound_only)).await.unwrap();
    assert_eq!(*phone.platform.mirror_asks.lock().unwrap(), ["start 0 with sound, sound only"]);

    // The phone sends only its sound.
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
    let screen = pc.platform.screen.clone();
    let deadline = tokio::time::Instant::now() + WAIT;
    while screen.got.lock().unwrap().len() < 2 && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(
        *screen.got.lock().unwrap(),
        ["sound 48000 Hz x2", "sound 10000 1920"],
        "sound, and no picture"
    );

    // Turning it off: the phone hears it, and the sound stream ends.
    pc.node.mirror_stop(phone_id, 0).await;
    let deadline = tokio::time::Instant::now() + WAIT;
    while !phone.platform.mirror_asks.lock().unwrap().iter().any(|a| a == "stop 0")
        && tokio::time::Instant::now() < deadline
    {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(phone.platform.mirror_asks.lock().unwrap().iter().any(|a| a == "stop 0"));
    let deadline = tokio::time::Instant::now() + WAIT;
    while !screen.got.lock().unwrap().iter().any(|g| g == "sound ended")
        && tokio::time::Instant::now() < deadline
    {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(screen.got.lock().unwrap().iter().any(|g| g == "sound ended"));
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
        stay_awake: false,
        screen_off: false,
        watch: None,
        sound_only: false,
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
    pc.node.mirror_power(phone_id, true, true).await.unwrap();
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
    // The phone records the stop after the streams close; under load that can lag a little.
    let wanted = ["keyframe 0", "power awake=true off=true", "Touch(Down) on 0", "Key(back) on 0", "stop 0"];
    let deadline = tokio::time::Instant::now() + WAIT;
    let asks = loop {
        let asks = phone.platform.mirror_asks.lock().unwrap().clone();
        let complete = wanted.iter().all(|w| asks.iter().any(|a| a == w));
        if complete || tokio::time::Instant::now() >= deadline {
            break asks;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    };
    for wanted in wanted {
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
        stay_awake: false,
        screen_off: false,
        watch: None,
        sound_only: false,
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

    // A backup's download is saved the same way but leaves nothing in the Timeline.
    let quiet =
        with_timeout("fetch_photos_quietly", pc.node.fetch_photos_quietly(phone_id, vec!["media:4".into()]))
            .await
            .unwrap();
    let quiet_files = wait_transfer(&mut pc, &quiet, "backup file", saved).await;
    assert_eq!(std::fs::read(&quiet_files[0]).unwrap(), bytes4);
    let timeline =
        pc.node.timeline_page(&nectarlink_core::TimelineQuery { limit: 50, ..Default::default() }).unwrap();
    let refs: Vec<_> = timeline.entries.iter().filter_map(|e| e.ref_id.as_deref()).collect();
    assert!(refs.contains(&tid.as_str()), "the gallery download is in the Timeline");
    assert!(!refs.contains(&quiet.as_str()), "the backup download isn't");

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
        RemoteInput::Text { text: "Hello NectarLink 🐝".into() },
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

    // Voice typing rewrites what it typed: the phone sends a backspace for each
    // character it replaces, then the new text, each as soon as the last is
    // answered. None of it may be refused (GitHub #52).
    pc.platform.remote_inputs.lock().unwrap().clear();
    let backspace = RemoteInput::Key { key: remote_keys::BACKSPACE.into(), mods: vec![] };
    let rewrite = RemoteInput::Text { text: "नमस्ते, how are you doing today".into() };
    for _ in 0..80 {
        with_timeout("backspace", phone.node.remote_input(pc_id, backspace.clone())).await.unwrap();
    }
    with_timeout("rewritten text", phone.node.remote_input(pc_id, rewrite.clone())).await.unwrap();
    let recorded = pc.platform.remote_inputs.lock().unwrap().clone();
    let mut expected = vec![backspace; 80];
    expected.push(rewrite);
    assert_eq!(recorded, expected, "every backspace and the text arrive, in order");

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

/// A device relays Wake-on-LAN only for an address one of its paired PCs
/// gave as its own (#74 audit), never for any MAC another device names.
#[tokio::test(flavor = "multi_thread")]
async fn wake_relay_is_only_for_a_paired_pcs_own_address() {
    use nectarlink_core::{PC_WAKE, PcWakeInfo};
    let mut pc = device_with("Desktop", DeviceKind::Desktop, &[PC_WAKE]).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;
    pc.node
        .set_wake_info(PcWakeInfo {
            macs: vec!["00:00:5E:00:53:01".into()],
            broadcasts: vec!["127.255.255.255".into()],
        })
        .await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());
    wait_for(&mut phone, "wake info", |e| match e {
        NodeEvent::WakeInfoChanged { device, can_wake: true } if *device == pc_id => Some(()),
        _ => None,
    })
    .await;
    let loopback = Some(std::net::Ipv4Addr::new(127, 255, 255, 255));

    // The PC's own address: the phone relays it.
    with_timeout(
        "relay own",
        pc.node.wake_via_relay(&phone_id, [0x00, 0x00, 0x5e, 0x00, 0x53, 0x01], loopback),
    )
    .await
    .expect("a paired PC's own address is relayed");
    // Any other address: refused.
    let other = with_timeout(
        "relay other",
        pc.node.wake_via_relay(&phone_id, [0x02, 0x11, 0x22, 0x33, 0x44, 0x55], loopback),
    )
    .await;
    assert!(matches!(other, Err(Error::Denied)), "{other:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn wake_info_syncs_persists_respects_toggle_and_sends_magic_packet() {
    use nectarlink_core::{PC_WAKE, PcWakeInfo, magic_packet};
    use std::net::Ipv4Addr;

    let mut pc = device_with("Desktop", DeviceKind::Desktop, &[PC_WAKE]).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;

    pc.node
        .set_wake_info(PcWakeInfo {
            macs: vec!["00:00:5E:00:53:01".into()],
            broadcasts: vec!["192.168.1.255".into()],
        })
        .await;

    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    // Initial wake info arrives on connect and is stored on the phone.
    wait_for(&mut phone, "initial wake info", |e| match e {
        NodeEvent::WakeInfoChanged { device, can_wake: true } if *device == pc_id => Some(()),
        _ => None,
    })
    .await;
    assert_eq!(
        phone.node.wake_info(pc_id).unwrap(),
        Some(PcWakeInfo { macs: vec!["00:00:5e:00:53:01".into()], broadcasts: vec!["192.168.1.255".into()] })
    );
    assert!(phone.node.paired_devices().unwrap().iter().any(|d| d.id == pc_id && d.can_wake));

    // Adapter change on the PC updates connected phones immediately.
    pc.node
        .set_wake_info(PcWakeInfo {
            macs: vec!["00:00:5E:00:53:01".into(), "aa-bb-cc-dd-ee-ff".into()],
            broadcasts: vec!["192.168.1.255".into(), "10.0.0.255".into()],
        })
        .await;
    let phone_node = phone.node.clone();
    wait_for(&mut phone, "updated wake info", |e| match e {
        NodeEvent::WakeInfoChanged { device, can_wake: true }
            if *device == pc_id
                && phone_node
                    .wake_info(pc_id)
                    .ok()
                    .flatten()
                    .is_some_and(|w| w.macs.len() == 2 && w.broadcasts.len() == 2) =>
        {
            Some(())
        }
        _ => None,
    })
    .await;

    // Turning `pc_actions` off on the PC clears the phone's stored wake info; turning it back on restores it.
    pc.node.set_device_toggle(phone_id, "pc_actions", false).unwrap();
    wait_for(&mut phone, "wake info cleared", |e| match e {
        NodeEvent::WakeInfoChanged { device, can_wake: false } if *device == pc_id => Some(()),
        _ => None,
    })
    .await;
    assert_eq!(phone.node.wake_info(pc_id).unwrap(), None);
    assert!(matches!(phone.node.wake(pc_id).await, Err(Error::Unsupported)));

    pc.node.set_device_toggle(phone_id, "pc_actions", true).unwrap();
    wait_for(&mut phone, "wake info restored", |e| match e {
        NodeEvent::WakeInfoChanged { device, can_wake: true } if *device == pc_id => Some(()),
        _ => None,
    })
    .await;

    // The PC goes offline and the phone restarts: stored wake info survives across restarts.
    pc.node.shutdown().await;
    phone.node.shutdown().await;
    let TestDevice { node, dir, .. } = phone;
    drop(node);
    let phone = start_in(dir, "Pixel", DeviceKind::Phone).await;
    assert!(phone.node.paired_devices().unwrap().iter().any(|d| d.id == pc_id && d.can_wake));

    // Waking the offline PC sends the 102-byte magic packet over UDP.
    let listener = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    phone.node.wake_on_ports(pc_id, &[port], &[Ipv4Addr::LOCALHOST]).await.unwrap();

    let mut buf = [0u8; 256];
    let (n, _) = with_timeout("receive magic packet", listener.recv_from(&mut buf)).await.unwrap();
    assert_eq!(n, 102);
    assert_eq!(&buf[..102], &magic_packet(&[0x00, 0x00, 0x5E, 0x00, 0x53, 0x01])[..]);
    phone.node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn deck_layout_and_state_arrive_press_runs_action_and_respects_toggles_and_unknown_tile() {
    use nectarlink_core::{
        DECK_ACTIONS, DeckAction, DeckConfig, DeckLayout, DeckState, DeckTileConfig, deck_colors, deck_icons,
    };

    let mut pc = device_with("Desktop", DeviceKind::Desktop, &[DECK_ACTIONS]).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;

    let mut cfg = DeckConfig::default_deck();
    cfg.pages[0].tiles.push(DeckTileConfig {
        id: "run_build".into(),
        label: "Build".into(),
        icon: deck_icons::TERMINAL.into(),
        color: deck_colors::CORAL.into(),
        action: DeckAction::RunCommand { command: "cargo check".into() },
    });
    let expected_layout = cfg.to_wire_layout();
    pc.node.set_deck_layout(expected_layout.clone()).await.unwrap();
    let initial_state = DeckState {
        playing: true,
        volume: 60,
        muted: false,
        mic_muted: Some(false),
        output_devices: Vec::new(),
    };
    pc.node.set_deck_state(initial_state.clone()).await.unwrap();

    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    // Initial layout and live state arrive on connect.
    let got_layout = wait_for(&mut phone, "initial deck layout", |e| match e {
        NodeEvent::DeckLayout { device, layout } if *device == pc_id => Some(layout.clone()),
        _ => None,
    })
    .await;
    assert_eq!(got_layout, expected_layout);
    assert_eq!(phone.node.deck_layout(pc_id), Some(expected_layout));

    let got_state = wait_for(&mut phone, "initial deck state", |e| match e {
        NodeEvent::DeckState { device, state } if *device == pc_id && *state == initial_state => {
            Some(state.clone())
        }
        _ => None,
    })
    .await;
    assert_eq!(got_state, initial_state);
    assert_eq!(phone.node.deck_state(pc_id), Some(initial_state));

    // Both `remote_input` and `commands` are off by default on the PC.
    let toggles = pc.node.device_toggles(phone_id).unwrap();
    assert!(toggles.contains(&("remote_input", false)));
    assert!(toggles.contains(&("commands", false)));

    // Pressing any tile while `remote_input` is off is denied and emits the one-time prompt.
    assert!(matches!(phone.node.deck_press(pc_id, "play_pause".into()).await, Err(Error::Denied)));
    wait_for(&mut pc, "remote input prompt from deck", |e| match e {
        NodeEvent::RemoteInputRequested { device } if *device == phone_id => Some(()),
        _ => None,
    })
    .await;
    assert!(pc.platform.deck_presses.lock().unwrap().is_empty());

    // Enable `remote_input`: built-in tiles work, unknown tiles return NotFound,
    // and `run_command` tiles are still denied while `commands` is off.
    pc.node.set_device_toggle(phone_id, "remote_input", true).unwrap();
    with_timeout("press play_pause", phone.node.deck_press(pc_id, "play_pause".into())).await.unwrap();
    with_timeout("press mic_mute", phone.node.deck_press(pc_id, "mic_mute".into())).await.unwrap();
    assert!(matches!(phone.node.deck_press(pc_id, "no_such_tile".into()).await, Err(Error::NotFound)));
    assert!(matches!(phone.node.deck_press(pc_id, "bad tile!".into()).await, Err(Error::Protocol(_))));
    assert!(matches!(phone.node.deck_press(pc_id, "run_build".into()).await, Err(Error::Denied)));

    // Enable `commands`: the `run_build` tile now runs.
    pc.node.set_device_toggle(phone_id, "commands", true).unwrap();
    with_timeout("press run_build", phone.node.deck_press(pc_id, "run_build".into())).await.unwrap();

    assert_eq!(
        *pc.platform.deck_presses.lock().unwrap(),
        vec!["play_pause".to_owned(), "mic_mute".to_owned(), "run_build".to_owned()]
    );

    // Live state updates push to the phone.
    let updated_state = DeckState {
        playing: false,
        volume: 45,
        muted: true,
        mic_muted: Some(true),
        output_devices: Vec::new(),
    };
    pc.node.set_deck_state(updated_state.clone()).await.unwrap();
    let got_updated = wait_for(&mut phone, "updated deck state", |e| match e {
        NodeEvent::DeckState { device, state } if *device == pc_id && *state == updated_state => {
            Some(state.clone())
        }
        _ => None,
    })
    .await;
    assert_eq!(got_updated, updated_state);

    // Layout edits push to the phone; removed tiles can no longer be pressed.
    let default_only = DeckLayout::default_layout();
    pc.node.set_deck_layout(default_only.clone()).await.unwrap();
    wait_for(&mut phone, "updated deck layout", |e| match e {
        NodeEvent::DeckLayout { device, layout } if *device == pc_id && *layout == default_only => Some(()),
        _ => None,
    })
    .await;
    assert!(matches!(phone.node.deck_press(pc_id, "run_build".into()).await, Err(Error::NotFound)));
}

/// A phone browses a PC's shared folders: the toggles named `storage` (on
/// the PC: browse that phone; on the phone: let that PC browse it) don't
/// stand in the way, whichever way they are set.
#[tokio::test(flavor = "multi_thread")]
async fn a_phone_browses_a_pcs_shared_folders() {
    use nectarlink_core::{FolderStorage, STORAGE_MOUNT, STORAGE_READ, STORAGE_WRITE};

    let mut pc = device_with("Desktop", DeviceKind::Desktop, &[STORAGE_MOUNT, STORAGE_READ]).await;
    let mut phone = device_with("Pixel", DeviceKind::Phone, &[STORAGE_READ, STORAGE_WRITE]).await;
    let shared = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(shared.path().join("Downloads")).unwrap();
    std::fs::write(shared.path().join("Downloads/report.pdf"), b"pdf").unwrap();
    *pc.platform.storage.lock().unwrap() = Some(FolderStorage::new(shared.path()));

    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    for (pc_toggle, phone_toggle) in [(None, None), (Some(false), Some(false)), (Some(true), Some(true))] {
        if let Some(on) = pc_toggle {
            pc.node.set_device_toggle(phone_id, "storage", on).unwrap();
        }
        if let Some(on) = phone_toggle {
            phone.node.set_device_toggle(pc_id, "storage", on).unwrap();
        }
        let root = with_timeout("list the PC", phone.node.storage_list(pc_id, "")).await.unwrap();
        assert_eq!(root.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["Downloads"], "{pc_toggle:?}");
        let files =
            with_timeout("list Downloads", phone.node.storage_list(pc_id, "Downloads")).await.unwrap();
        assert_eq!(files.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["report.pdf"]);
    }

    // A PC that doesn't share its folders says no.
    *pc.platform.storage.lock().unwrap() = None;
    assert!(phone.node.storage_list(pc_id, "").await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn phone_storage_lists_reads_ranges_writes_resumes_mutates_and_enforces_security() {
    use nectarlink_core::{FolderStorage, STORAGE_MOUNT, STORAGE_READ, STORAGE_WRITE};

    let mut pc = device_with("Desktop", DeviceKind::Desktop, &[STORAGE_MOUNT]).await;
    let mut phone = device_with("Pixel", DeviceKind::Phone, &[STORAGE_READ, STORAGE_WRITE]).await;

    let storage_root = tempfile::tempdir().unwrap();
    let trash_root = tempfile::tempdir().unwrap();
    let outside_root = tempfile::tempdir().unwrap();
    std::fs::write(outside_root.path().join("outside.txt"), b"do not expose").unwrap();

    std::fs::create_dir_all(storage_root.path().join("Documents")).unwrap();
    std::fs::create_dir_all(storage_root.path().join("DCIM")).unwrap();
    std::fs::write(storage_root.path().join("Documents/readme.txt"), b"0123456789abcdef").unwrap();
    std::fs::write(storage_root.path().join("DCIM/photo.jpg"), b"fake-jpeg-bytes").unwrap();

    // Create a directory link inside storage_root pointing to outside_root to verify symlink/junction rejection.
    #[cfg(windows)]
    {
        let link = storage_root.path().join("escape_link");
        let _ = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(outside_root.path())
            .output();
    }
    #[cfg(unix)]
    {
        let _ = std::os::unix::fs::symlink(outside_root.path(), storage_root.path().join("escape_link"));
    }

    *phone.platform.storage.lock().unwrap() =
        Some(FolderStorage::new(storage_root.path()).with_trash_dir(trash_root.path()));

    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    // `storage` toggle is OFF by default on the phone.
    let toggles = phone.node.device_toggles(pc_id).unwrap();
    assert!(toggles.contains(&("storage", false)));

    // Requests while `storage` is off are denied and emit `StorageRequested` once on the phone.
    assert!(matches!(pc.node.storage_list(phone_id, "").await, Err(Error::Denied)));
    wait_for(&mut phone, "storage requested prompt", |e| match e {
        NodeEvent::StorageRequested { device } if *device == pc_id => Some(()),
        _ => None,
    })
    .await;
    assert!(matches!(
        pc.node.storage_read(phone_id, "Documents/readme.txt", 0, None).await,
        Err(Error::Denied)
    ));
    assert!(matches!(pc.node.storage_mkdir(phone_id, "NewFolder").await, Err(Error::Denied)));

    // Enable `storage` on the phone for this PC.
    phone.node.set_device_toggle(pc_id, "storage", true).unwrap();

    // Root listing returns DCIM and Documents (sorted), and omits `escape_link` pointing outside root.
    let root_entries = with_timeout("list root", pc.node.storage_list(phone_id, "")).await.unwrap();
    let names: Vec<&str> = root_entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["DCIM", "Documents"]);
    assert!(root_entries.iter().all(|e| e.is_dir));
    assert!(phone.node.storage_open_folders().contains(&String::new()));

    // Listing `Documents` returns `readme.txt` with its size.
    let doc_entries =
        with_timeout("list Documents", pc.node.storage_list(phone_id, "Documents")).await.unwrap();
    assert_eq!(doc_entries.len(), 1);
    assert_eq!(doc_entries[0].name, "readme.txt");
    assert_eq!(doc_entries[0].size, 16);
    assert!(!doc_entries[0].is_dir);

    // Full read and ranged read (`offset` + `length`).
    let (meta_full, bytes_full) =
        with_timeout("read full", pc.node.storage_read(phone_id, "Documents/readme.txt", 0, None))
            .await
            .unwrap();
    assert_eq!(meta_full.size, 16);
    assert_eq!(meta_full.length, 16);
    assert_eq!(bytes_full, b"0123456789abcdef");

    let (meta_range, bytes_range) =
        with_timeout("read range", pc.node.storage_read(phone_id, "Documents/readme.txt", 6, Some(5)))
            .await
            .unwrap();
    assert_eq!(meta_range.size, 16);
    assert_eq!(meta_range.length, 5);
    assert_eq!(bytes_range, b"6789a");

    let (meta_eof, bytes_eof) =
        with_timeout("read past eof", pc.node.storage_read(phone_id, "Documents/readme.txt", 100, Some(10)))
            .await
            .unwrap();
    assert_eq!(meta_eof.size, 16);
    assert_eq!(meta_eof.length, 0);
    assert!(bytes_eof.is_empty());

    // `mkdir` creates a directory and triggers `storage.changed` for watched parent `""`.
    with_timeout("mkdir Projects", pc.node.storage_mkdir(phone_id, "Projects")).await.unwrap();
    assert!(storage_root.path().join("Projects").is_dir());
    let changed_dir = wait_for(&mut pc, "storage changed on root", |e| match e {
        NodeEvent::StorageChanged { device, path } if *device == phone_id => Some(path.clone()),
        _ => None,
    })
    .await;
    assert_eq!(changed_dir, "");

    // `write` with interruption and resume using the same upload ID.
    let upload_src = tempfile::tempdir().unwrap();
    let payload = data(250 * 1024, 9);
    let local_file = upload_src.path().join("upload.bin");
    std::fs::write(&local_file, &payload).unwrap();

    // First attempt stops after 100 KiB.
    let interrupted = pc
        .node
        .storage_write_with_id(
            phone_id,
            "Documents/upload.bin",
            &local_file,
            Some("resume-upload-01".into()),
            Some(100 * 1024),
        )
        .await;
    assert!(interrupted.is_err());
    assert!(!storage_root.path().join("Documents/upload.bin").exists());

    // Wait briefly for the phone to flush the partial staging file, then resume with the same ID.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let done = with_timeout(
        "resumed write",
        pc.node.storage_write_with_id(
            phone_id,
            "Documents/upload.bin",
            &local_file,
            Some("resume-upload-01".into()),
            None,
        ),
    )
    .await
    .unwrap();
    assert_eq!(done.size, payload.len() as u64);
    assert_eq!(std::fs::read(storage_root.path().join("Documents/upload.bin")).unwrap(), payload);

    // `rename` moves the uploaded file into `Projects/renamed.bin`.
    with_timeout(
        "rename file",
        pc.node.storage_rename(phone_id, "Documents/upload.bin", "Projects/renamed.bin"),
    )
    .await
    .unwrap();
    assert!(!storage_root.path().join("Documents/upload.bin").exists());
    assert_eq!(std::fs::read(storage_root.path().join("Projects/renamed.bin")).unwrap(), payload);

    // `delete`: media file moves to trash even with `confirmed = false`; non-media requires `confirmed = true`.
    with_timeout("trash photo", pc.node.storage_delete(phone_id, "DCIM/photo.jpg", false)).await.unwrap();
    assert!(!storage_root.path().join("DCIM/photo.jpg").exists());
    assert_eq!(std::fs::read(trash_root.path().join("photo.jpg")).unwrap(), b"fake-jpeg-bytes");

    assert!(matches!(
        pc.node.storage_delete(phone_id, "Projects/renamed.bin", false).await,
        Err(Error::Denied)
    ));
    with_timeout("confirmed delete", pc.node.storage_delete(phone_id, "Projects/renamed.bin", true))
        .await
        .unwrap();
    assert!(!storage_root.path().join("Projects/renamed.bin").exists());

    // Path traversal, absolute paths, `/data`, and symlinks/junctions outside root are refused.
    for bad in ["..", "../outside.txt", "Documents/../../outside.txt", "/etc/passwd", "/data/data"] {
        assert!(pc.node.storage_list(phone_id, bad).await.is_err(), "list {bad}");
        assert!(pc.node.storage_read(phone_id, bad, 0, None).await.is_err(), "read {bad}");
        assert!(pc.node.storage_mkdir(phone_id, bad).await.is_err(), "mkdir {bad}");
        assert!(pc.node.storage_delete(phone_id, bad, true).await.is_err(), "delete {bad}");
    }
    if storage_root.path().join("escape_link").exists() {
        assert!(matches!(
            pc.node.storage_list(phone_id, "escape_link").await,
            Err(Error::Denied | Error::NotFound)
        ));
        assert!(matches!(
            pc.node.storage_read(phone_id, "escape_link/outside.txt", 0, None).await,
            Err(Error::Denied | Error::NotFound)
        ));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_phone_camera_streams_to_the_pc_as_a_webcam() {
    webcam_round_trip(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_phone_negotiates_hevc_only_with_an_advertising_receiver() {
    webcam_round_trip(true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn screen_mirroring_prefers_hevc_only_when_the_receiver_advertises_it() {
    let mut pc = device_with("Desktop", DeviceKind::Desktop, &[nectarlink_core::MIRROR_HEVC]).await;
    let mut phone = device_with("Pixel", DeviceKind::Phone, &[]).await;
    pair_qr(&mut pc, &mut phone).await;
    let pc_id = pc.node.device_id();

    assert_eq!(phone.node.mirror_codecs(pc_id).unwrap(), ["h265", "h264"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn screen_mirroring_uses_h264_for_receivers_without_hevc_support() {
    let mut pc = device_with("Desktop", DeviceKind::Desktop, &[]).await;
    let mut phone = device_with("Pixel", DeviceKind::Phone, &[]).await;
    pair_qr(&mut pc, &mut phone).await;

    assert_eq!(phone.node.mirror_codecs(pc.node.device_id()).unwrap(), ["h264"]);
}

async fn webcam_round_trip(hevc: bool) {
    use nectarlink_core::{MirrorSend, PacketKind};
    let mut caps = vec![nectarlink_core::WEBCAM_VIRTUAL];
    if hevc {
        caps.push(nectarlink_core::WEBCAM_HEVC);
    }
    let mut pc = device_with("Desktop", DeviceKind::Desktop, &caps).await;
    let mut phone = device_with("Pixel", DeviceKind::Phone, &[nectarlink_core::WEBCAM_STREAM]).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    let codecs = phone.node.webcam_codecs(pc_id).unwrap();
    assert_eq!(codecs, if hevc { vec!["h265", "h264"] } else { vec!["h264"] });

    let options = nectarlink_core::WebcamStart {
        camera: nectarlink_core::WEBCAM_CAMERA_BACK.into(),
        width: 1920,
        height: 1080,
        fps: 30,
        bitrate: 6_000_000,
    };
    with_timeout("webcam_start", pc.node.webcam_start(phone_id, options.clone())).await.unwrap();
    assert_eq!(*phone.platform.webcam_asks.lock().unwrap(), ["start back 1920x1080 @ 30"]);

    let stream = Arc::new(with_timeout("webcam_open", phone.node.webcam_open(pc_id)).await.unwrap());
    let config = nectarlink_core::WebcamConfig {
        codec: codecs[0].clone(),
        width: 1920,
        height: 1080,
        camera: nectarlink_core::WEBCAM_CAMERA_BACK.into(),
        fps: 30,
        max_height: 2160,
    };
    let sender = stream.clone();
    let sent = tokio::task::spawn_blocking(move || {
        [
            sender.send(PacketKind::Config, 0, config.to_cbor()),
            sender.send(PacketKind::Keyframe, 1, vec![0; 48_000]),
            sender.send(PacketKind::Frame, 33_333, vec![0; 3_200]),
        ]
    })
    .await
    .unwrap();
    assert!(sent.iter().all(|s| *s == MirrorSend::Queued), "{sent:?}");
    wait_for(&mut pc, "webcam on", |e| matches!(e, NodeEvent::Webcam { on: true, .. }).then_some(())).await;

    let sink = pc.platform.webcam.clone();
    let deadline = tokio::time::Instant::now() + WAIT;
    while sink.got.lock().unwrap().len() < 3 && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(*sink.got.lock().unwrap(), ["config back 1920x1080 @ 30", "key 1 48000", "frame 33333 3200"]);

    pc.node.webcam_keyframe(phone_id).await;
    pc.node.webcam_stop(phone_id).await;
    wait_for(&mut pc, "webcam off", |e| matches!(e, NodeEvent::Webcam { on: false, .. }).then_some(())).await;

    let deadline = tokio::time::Instant::now() + WAIT;
    while !stream.is_closed() && tokio::time::Instant::now() < deadline {
        let s = stream.clone();
        tokio::task::spawn_blocking(move || s.send(PacketKind::Frame, 0, vec![0])).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(stream.is_closed(), "the phone's webcam stream closes");

    for wanted in ["keyframe", "stop"] {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !phone.platform.webcam_asks.lock().unwrap().iter().any(|a| a == wanted) {
            let asks = phone.platform.webcam_asks.lock().unwrap().clone();
            assert!(std::time::Instant::now() < deadline, "{wanted}: {asks:?}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    // Turning off the webcam device toggle refuses new requests.
    phone.node.set_device_toggle(pc_id, "webcam", false).unwrap();
    let refused = pc.node.webcam_start(phone_id, options).await;
    assert!(matches!(refused, Err(Error::Denied)), "{refused:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn pc_audio_volume_mute_output_devices_and_smart_clipboard_context_chips_flow_end_to_end() {
    use nectarlink_core::{AudioOutputDevice, ClipKind, DECK_ACTIONS, DeckState, PC_AUDIO};

    let mut pc =
        device_with("Desktop", DeviceKind::Desktop, &[DECK_ACTIONS, PC_AUDIO, "clipboard.sync"]).await;
    let mut phone = device_with("Pixel", DeviceKind::Phone, &["clipboard.sync"]).await;

    let initial_state = DeckState {
        playing: false,
        volume: 75,
        muted: false,
        mic_muted: Some(false),
        output_devices: vec![
            AudioOutputDevice {
                id: "{0.0.0.00000000}.{speakers-id}".into(),
                name: "Speakers (Realtek Audio)".into(),
                is_default: true,
            },
            AudioOutputDevice {
                id: "{0.0.0.00000000}.{headphones-id}".into(),
                name: "Headphones (USB DAC)".into(),
                is_default: false,
            },
        ],
    };
    pc.node.set_deck_state(initial_state.clone()).await.unwrap();

    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    // Initial DeckState with active output devices arrives on connect.
    let got_state = wait_for(&mut phone, "initial deck state with output devices", |e| match e {
        NodeEvent::DeckState { device, state } if *device == pc_id && *state == initial_state => {
            Some(state.clone())
        }
        _ => None,
    })
    .await;
    assert_eq!(got_state.default_output_name(), Some("Speakers (Realtek Audio)"));
    assert_eq!(got_state.output_devices.len(), 2);

    // Phone sets PC volume and mute (`pc_actions` is ON by default).
    with_timeout("set pc volume 40", phone.node.set_pc_audio(pc_id, Some(40), None)).await.unwrap();
    let after_vol = wait_for(&mut phone, "deck state volume 40", |e| match e {
        NodeEvent::DeckState { device, state } if *device == pc_id && state.volume == 40 => {
            Some(state.clone())
        }
        _ => None,
    })
    .await;
    assert_eq!(after_vol.volume, 40);
    assert!(!after_vol.muted);

    with_timeout("set pc mute true", phone.node.set_pc_audio(pc_id, None, Some(true))).await.unwrap();
    let after_mute = wait_for(&mut phone, "deck state muted true", |e| match e {
        NodeEvent::DeckState { device, state } if *device == pc_id && state.muted => Some(state.clone()),
        _ => None,
    })
    .await;
    assert!(after_mute.muted);
    assert_eq!(*pc.platform.pc_audio_changes.lock().unwrap(), vec![(Some(40), None), (None, Some(true))]);

    // Out-of-range volume or empty request is rejected as a protocol error.
    assert!(matches!(phone.node.set_pc_audio(pc_id, Some(101), None).await, Err(Error::Protocol(_))));
    assert!(matches!(phone.node.set_pc_audio(pc_id, None, None).await, Err(Error::Protocol(_))));

    // When both `pc_actions` and `remote_input` are off, `set_pc_audio` is denied.
    pc.node.set_device_toggle(phone_id, "pc_actions", false).unwrap();
    assert!(matches!(phone.node.set_pc_audio(pc_id, Some(50), None).await, Err(Error::Denied)));

    // Turning `pc_actions` back on re-enables `set_pc_audio`.
    pc.node.set_device_toggle(phone_id, "pc_actions", true).unwrap();
    with_timeout("set pc audio unmute + 65", phone.node.set_pc_audio(pc_id, Some(65), Some(false)))
        .await
        .unwrap();

    // Phone as speaker needs the phone's `media` permission to start; stopping is always allowed.
    pc.node.set_device_toggle(phone_id, "media", false).unwrap();
    assert!(matches!(
        phone.node.set_pc_audio(pc_id, Some(nectarlink_core::deck::PHONE_SPEAKER_ON), None).await,
        Err(Error::Denied)
    ));
    with_timeout(
        "phone speaker off while media is off",
        phone.node.set_pc_audio(pc_id, Some(nectarlink_core::deck::PHONE_SPEAKER_OFF), None),
    )
    .await
    .unwrap();
    pc.node.set_device_toggle(phone_id, "media", true).unwrap();
    with_timeout(
        "phone speaker on",
        phone.node.set_pc_audio(pc_id, Some(nectarlink_core::deck::PHONE_SPEAKER_ON), None),
    )
    .await
    .unwrap();

    // Smart clipboard context chips: phone sends different text clips to PC,
    // and PC classifies each into at most 1 context chip (and None for OTP).
    let cases = [
        ("https://nectarlink.app/docs?ref=clip", Some((ClipKind::WebLink, "Open"))),
        (
            "1600 Amphitheatre Parkway, Mountain View, CA 94043",
            Some((ClipKind::StreetAddress, "Open in Maps")),
        ),
        ("+1 (415) 555-0199", Some((ClipKind::PhoneNumber, "Call"))),
        ("1Z999AA10123456784", Some((ClipKind::TrackingNumber, "Track"))),
        ("hello@nectarlink.app", Some((ClipKind::Email, "Email"))),
        ("Your verification code is 482910", None),
        ("Just a regular sentence with no special action.", None),
    ];

    for (text, expected) in cases {
        with_timeout("send clipboard", phone.node.send_clipboard(pc_id, text.into())).await.unwrap();
        wait_for(&mut pc, "clipboard received", |e| match e {
            NodeEvent::ClipboardReceived { device } if *device == phone_id => Some(()),
            _ => None,
        })
        .await;
        let suggestion = pc.node.last_clip_suggestion();
        match expected {
            Some((kind, label)) => {
                let (from, s) = suggestion.unwrap_or_else(|| panic!("expected chip for {text:?}"));
                assert_eq!(from, phone_id);
                assert_eq!(s.kind, kind, "wrong kind for {text:?}");
                assert_eq!(s.action_label(), label, "wrong label for {text:?}");
                assert!(!format!("{s:?}").contains(&s.target), "Debug must not leak target");
            }
            None => {
                assert!(suggestion.is_none(), "expected no chip for {text:?}, got {suggestion:?}");
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn live_notifications_and_task_notify_flow_end_to_end() {
    use nectarlink_core::{LivePoint, LiveSegment, NotificationLive, TaskNotify};

    let mut pc = device("Desktop", DeviceKind::Desktop).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;
    pair_qr(&mut pc, &mut phone).await;
    let phone_id = phone.node.device_id();
    grant_notification_access(&phone).await;

    // Post a regular notification and an older live notification: snapshot puts live first.
    let mut regular = note("n-regular", "com.chat", "Sam", false);
    regular.when = 2_000_000_000_000;
    let mut live_note = note("n-live", "com.rides", "Driver arriving", false);
    live_note.when = 1_500_000_000_000;
    live_note.live = Some(NotificationLive {
        v: 1,
        progress: Some(40),
        max: Some(100),
        indeterminate: false,
        chip: Some("4 min".into()),
        segments: vec![
            LiveSegment { length: 50, color: Some(0xFF4C_AF50) },
            LiveSegment { length: 50, color: None },
        ],
        points: vec![LivePoint { position: 50, color: Some(0xFFFF_C107) }],
        chronometer: false,
        countdown: false,
    });
    phone.node.notifications_reset(vec![regular, live_note.clone()]).await;

    let items = wait_for(&mut pc, "snapshot with live first", |e| match e {
        NodeEvent::NotificationsReset { device, items } if *device == phone_id && items.len() == 2 => {
            Some(items.clone())
        }
        _ => None,
    })
    .await;
    assert_eq!(items[0].key, "n-live");
    assert_eq!(items[0].live.as_ref().and_then(|l| l.chip.as_deref()), Some("4 min"));
    assert_eq!(items[1].key, "n-regular");

    // Updating the live notification in place delivers the updated progress and chip.
    live_note.live.as_mut().unwrap().progress = Some(85);
    live_note.live.as_mut().unwrap().chip = Some("1 min".into());
    phone.node.notification_posted(live_note).await;
    let updated = wait_for(&mut pc, "live update posted", |e| match e {
        NodeEvent::NotificationPosted { device, notification }
            if *device == phone_id && notification.key == "n-live" =>
        {
            Some(notification.clone())
        }
        _ => None,
    })
    .await;
    assert_eq!(updated.live.as_ref().and_then(|l| l.progress), Some(85));
    assert_eq!(updated.live.as_ref().and_then(|l| l.chip.as_deref()), Some("1 min"));

    // PC sends an active task notification and then a completion task notification to the phone.
    let running = TaskNotify {
        id: "task-build-1".into(),
        title: "cargo build --release".into(),
        active: true,
        elapsed_ms: 1_500,
        exit_code: None,
    };
    with_timeout("task_notify running", pc.node.task_notify(Some(phone_id), running.clone()))
        .await
        .expect("task_notify running succeeds");

    let done = TaskNotify {
        id: "task-build-1".into(),
        title: "cargo build --release".into(),
        active: false,
        elapsed_ms: 42_000,
        exit_code: Some(0),
    };
    with_timeout("task_notify done", pc.node.task_notify(None, done.clone()))
        .await
        .expect("task_notify done succeeds");

    let recorded = phone.platform.tasks.lock().unwrap().clone();
    assert_eq!(recorded, vec![running, done]);
}

#[tokio::test(flavor = "multi_thread")]
async fn material_you_accent_syncs_updates_live_and_persists_offline() {
    let mut pc = device("Desktop", DeviceKind::Desktop).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;

    // Set initial Material You seed on the phone before pairing.
    phone.node.update_accent(Some(0xFF8A_5100)).await;
    pair_qr(&mut pc, &mut phone).await;
    let phone_id = phone.node.device_id();

    let paired = pc.node.paired_devices().unwrap();
    assert_eq!(paired.len(), 1);
    assert_eq!(paired[0].info.accent, Some(0xFF8A_5100));

    // Live wallpaper change on the phone sends HelloUpdate via update_accent.
    phone.node.update_accent(Some(0xFF00_FF00)).await;
    let info1 = wait_for(&mut pc, "accent updated to green", |e| match e {
        NodeEvent::PeerInfoChanged { device, info }
            if *device == phone_id && info.accent == Some(0xFF00_FF00) =>
        {
            Some(info.clone())
        }
        _ => None,
    })
    .await;
    assert_eq!(info1.accent, Some(0xFF00_FF00));
    assert_eq!(pc.node.paired_devices().unwrap()[0].info.accent, Some(0xFF00_FF00));

    // When the phone later updates DeviceInfo with accent = None (e.g. name change),
    // the PC preserves the phone's last stored seed in SQLite and in PeerInfoChanged.
    let mut renamed = info("Pixel Pro", DeviceKind::Phone);
    renamed.accent = None;
    phone.node.update_device_info(renamed).await;
    let info2 = wait_for(&mut pc, "renamed with preserved accent", |e| match e {
        NodeEvent::PeerInfoChanged { device, info } if *device == phone_id && info.name == "Pixel Pro" => {
            Some(info.clone())
        }
        _ => None,
    })
    .await;
    assert_eq!(info2.accent, Some(0xFF00_FF00));
    assert_eq!(pc.node.paired_devices().unwrap()[0].info.accent, Some(0xFF00_FF00));
}

#[tokio::test(flavor = "multi_thread")]
async fn timeline_records_files_clips_links_photos_recordings_and_sessions_without_duplicating_clips() {
    use nectarlink_core::{
        MIRROR_CAPTURE, MIRROR_VIEW, MirrorConfig, PHOTOS_READ, PHOTOS_SHOW, PacketKind, RECORDER,
        RecordingMarker, TimelineKind, TimelineQuery, WEBCAM_STREAM, WEBCAM_VIRTUAL, WebcamConfig,
    };

    let mut pc =
        device_with("Desktop", DeviceKind::Desktop, &[PHOTOS_SHOW, RECORDER, MIRROR_VIEW, WEBCAM_VIRTUAL])
            .await;
    let mut phone =
        device_with("Pixel", DeviceKind::Phone, &[PHOTOS_READ, MIRROR_CAPTURE, WEBCAM_STREAM]).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    // 1. Outgoing & incoming file transfer.
    let file_path = phone.dir.path().join("report.pdf");
    std::fs::write(&file_path, b"quarterly report").unwrap();
    let tid = phone
        .node
        .send_files(
            pc_id,
            vec![OutgoingFile {
                name: "report.pdf".into(),
                folder: None,
                source: FileSource::Path(file_path),
            }],
        )
        .await
        .unwrap();
    let _ = wait_transfer(&mut pc, &tid, "report.pdf", saved).await;

    // 2. Text clip (stored by ref_id only, never duplicated in SQLite) + OTP clip (excluded).
    let secret_clip = "super-unique-clipboard-payload-998877";
    phone.node.send_clipboard(pc_id, secret_clip.into()).await.unwrap();
    wait_for(&mut pc, "clip received", |e| match e {
        NodeEvent::ClipboardReceived { device } if *device == phone_id => Some(()),
        _ => None,
    })
    .await;

    phone
        .node
        .send_clipboard(pc_id, "Your verification code is 482910. Do not share this OTP.".into())
        .await
        .unwrap();
    wait_for(&mut pc, "otp clip received", |e| match e {
        NodeEvent::ClipboardReceived { device } if *device == phone_id => Some(()),
        _ => None,
    })
    .await;

    // 3. Web link and map handoff opened.
    phone.node.open_link(pc_id, "https://nectarlink.app/timeline".into()).await.unwrap();
    phone.node.open_link(pc_id, "geo:37.7749,-122.4194?q=San+Francisco".into()).await.unwrap();

    // 4. Photo saved from phone.
    let photo_path = phone.dir.path().join("IMG_100.jpg");
    std::fs::write(&photo_path, b"\xff\xd8\xff\xd9").unwrap();
    phone.platform.photos.lock().unwrap().insert("media:100".into(), photo_path);
    let ptid = pc.node.fetch_photo(phone_id, "media:100".into()).await.unwrap();
    let _ = wait_transfer(&mut pc, &ptid, "IMG_100.jpg", saved).await;

    // 5. Voice recording with markers.
    let rec_path = phone.dir.path().join("Voice_01.m4a");
    std::fs::write(&rec_path, b"fake-m4a-audio").unwrap();
    let rtid = phone
        .node
        .send_recording(
            pc_id,
            OutgoingFile { name: "Voice_01.m4a".into(), folder: None, source: FileSource::Path(rec_path) },
            vec![RecordingMarker { at_ms: 1000, label: Some("Intro".into()) }],
        )
        .await
        .unwrap();
    let _ = wait_transfer(&mut pc, &rtid, "Voice_01.m4a", saved).await;

    // 6. Screen mirroring session & webcam session (start/end recorded as one line with duration).
    let mstream = phone.node.mirror_open(pc_id).await.unwrap();
    let mcfg = MirrorConfig { codec: "h264".into(), width: 1080, height: 2400, session: 0 }.to_cbor();
    let _ = tokio::task::block_in_place(|| mstream.send(PacketKind::Config, 0, mcfg));
    wait_for(&mut pc, "mirroring started", |e| match e {
        NodeEvent::Mirroring { device, on: true, .. } if *device == phone_id => Some(()),
        _ => None,
    })
    .await;
    mstream.close();
    wait_for(&mut pc, "mirroring ended", |e| match e {
        NodeEvent::Mirroring { device, on: false, .. } if *device == phone_id => Some(()),
        _ => None,
    })
    .await;

    let wstream = phone.node.webcam_open(pc_id).await.unwrap();
    let wcfg = WebcamConfig {
        codec: "h264".into(),
        width: 1280,
        height: 720,
        camera: "back".into(),
        fps: 30,
        max_height: 1080,
    }
    .to_cbor();
    let _ = tokio::task::block_in_place(|| wstream.send(PacketKind::Config, 0, wcfg));
    wait_for(&mut pc, "webcam started", |e| match e {
        NodeEvent::Webcam { device, on: true } if *device == phone_id => Some(()),
        _ => None,
    })
    .await;
    wstream.close();
    wait_for(&mut pc, "webcam ended", |e| match e {
        NodeEvent::Webcam { device, on: false } if *device == phone_id => Some(()),
        _ => None,
    })
    .await;

    // Query the PC's timeline.
    let page = pc.node.timeline_page(&TimelineQuery::default()).unwrap();
    let kinds: Vec<TimelineKind> = page.entries.iter().map(|i| i.kind).collect();
    assert!(kinds.contains(&TimelineKind::File), "expected File in {kinds:?}");
    assert!(kinds.contains(&TimelineKind::Clip), "expected Clip in {kinds:?}");
    assert!(kinds.contains(&TimelineKind::Link), "expected Link in {kinds:?}");
    let geo_item = page
        .entries
        .iter()
        .find(|i| i.kind == TimelineKind::Link && i.detail == "geo:37.7749,-122.4194?q=San+Francisco")
        .expect("geo link recorded with raw URI in detail");
    assert_eq!(geo_item.title, "San Francisco on a map");
    assert!(kinds.contains(&TimelineKind::Photo), "expected Photo in {kinds:?}");
    assert!(kinds.contains(&TimelineKind::Recording), "expected Recording in {kinds:?}");
    assert!(
        kinds.iter().filter(|&&k| k == TimelineKind::Session).count() >= 2,
        "expected Mirror and Webcam sessions in {kinds:?}"
    );
    for session_item in page.entries.iter().filter(|i| i.kind == TimelineKind::Session) {
        assert!(session_item.duration_secs >= 1, "session must record duration >= 1s");
    }

    // Verify OTP was never recorded in the timeline.
    let otp_search = pc
        .node
        .timeline_page(&TimelineQuery { search: Some("482910".into()), ..TimelineQuery::default() })
        .unwrap();
    assert!(otp_search.entries.is_empty(), "OTP must never appear in timeline");

    // Verify clip content is resolved in memory for search and display, but NEVER stored in SQLite!
    let clip_search = pc
        .node
        .timeline_page(&TimelineQuery {
            search: Some("super-unique-clipboard".into()),
            ..TimelineQuery::default()
        })
        .unwrap();
    assert_eq!(clip_search.entries.len(), 1);
    assert_eq!(clip_search.entries[0].kind, TimelineKind::Clip);
    assert_eq!(clip_search.entries[0].title, secret_clip);
    let clip_ref = clip_search.entries[0].ref_id.clone().expect("clip has ref_id");

    let sqlite_bytes = std::fs::read(pc.dir.path().join("nectarlink.db")).unwrap();
    let wal_bytes = std::fs::read(pc.dir.path().join("nectarlink.db-wal")).unwrap_or_default();
    assert!(
        !sqlite_bytes.windows(secret_clip.len()).any(|w| w == secret_clip.as_bytes())
            && !wal_bytes.windows(secret_clip.len()).any(|w| w == secret_clip.as_bytes()),
        "SQLite database must never duplicate clip content"
    );

    // Deleting the clip from clipboard history also removes its timeline entry.
    assert!(pc.node.delete_clipboard_history(&clip_ref));
    let after_del = pc
        .node
        .timeline_page(&TimelineQuery { kind: Some(TimelineKind::Clip), ..TimelineQuery::default() })
        .unwrap();
    assert!(after_del.entries.is_empty(), "deleting clip removes linked timeline entry");
}

#[tokio::test(flavor = "multi_thread")]
async fn messaging_style_notifications_populate_unified_chat_threads_and_reply() {
    use nectarlink_core::{NotificationChatMessage, NotificationConversation};

    let mut pc = device("Desktop", DeviceKind::Desktop).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;
    pair_qr(&mut pc, &mut phone).await;
    let phone_id = phone.node.device_id();
    grant_notification_access(&phone).await;

    let now_ms = 1_760_000_000_000i64;
    let mut wa_note = note("wa|maya", "com.whatsapp", "Maya Lin", true);
    wa_note.app_name = "WhatsApp".into();
    wa_note.when = now_ms - 60_000;
    wa_note.conversation = Some(NotificationConversation {
        v: 1,
        title: "Maya Lin".into(),
        group: false,
        avatar: Some(vec![0xff, 0xd8, 1, 2]),
        messages: vec![
            NotificationChatMessage {
                sender: Some("Maya Lin".into()),
                text: "Are you free for a quick sync?".into(),
                time: now_ms - 120_000,
                self_sent: false,
                avatar: None,
            },
            NotificationChatMessage {
                sender: Some("Maya Lin".into()),
                text: "Sent over the deck too".into(),
                time: now_ms - 60_000,
                self_sent: false,
                avatar: None,
            },
        ],
    });

    phone.node.notification_posted(wa_note).await;
    let changed_thread = wait_for(&mut pc, "chat thread updated from notification", |e| match e {
        NodeEvent::SmsChanged { device, thread: Some(tid) }
            if *device == phone_id && tid.starts_with("chat:") =>
        {
            Some(tid.clone())
        }
        _ => None,
    })
    .await;

    let threads = pc.node.chat_threads(phone_id);
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].thread_id, changed_thread);
    assert_eq!(threads[0].app, "com.whatsapp");
    assert_eq!(threads[0].app_name, "WhatsApp");
    assert_eq!(threads[0].title, "Maya Lin");
    assert!(!threads[0].is_group);
    assert_eq!(threads[0].unread, 2);
    assert!(threads[0].can_reply());
    assert!(threads[0].app_icon.is_some());
    assert!(threads[0].avatar.is_some());

    let msgs = pc.node.chat_messages(phone_id, &changed_thread, 20);
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].text, "Sent over the deck too");
    assert_eq!(msgs[1].text, "Are you free for a quick sync?");

    // Reply directly from the unified conversation thread.
    with_timeout(
        "reply_chat_thread",
        pc.node.reply_chat_thread(phone_id, &changed_thread, "Give me 5 minutes!".into()),
    )
    .await
    .unwrap();
    assert_eq!(
        *phone.platform.actions.lock().unwrap(),
        vec![("wa|maya".into(), "0".into(), Some("Give me 5 minutes!".into()))]
    );

    let after_reply = pc.node.chat_thread(phone_id, &changed_thread).unwrap();
    assert_eq!(after_reply.unread, 0);
    assert_eq!(after_reply.snippet, "You: Give me 5 minutes!");
    let msgs = pc.node.chat_messages(phone_id, &changed_thread, 20);
    assert_eq!(msgs.len(), 3);
    assert_eq!(msgs[0].text, "Give me 5 minutes!");
    assert!(msgs[0].self_sent);

    // Removing the notification disables inline reply while keeping local history.
    phone.node.notification_removed("wa|maya".into()).await;
    wait_for(&mut pc, "notification removed", |e| match e {
        NodeEvent::NotificationRemoved { device, key } if *device == phone_id && key == "wa|maya" => Some(()),
        _ => None,
    })
    .await;
    let after_removed = pc.node.chat_thread(phone_id, &changed_thread).unwrap();
    assert!(!after_removed.can_reply());
    assert!(matches!(
        pc.node.reply_chat_thread(phone_id, &changed_thread, "Still there?".into()).await,
        Err(Error::NotFound)
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn mms_image_attachments_send_over_sms_service() {
    use nectarlink_core::{SMS_MAX_ATTACHMENT_BYTES, SmsAttachment};

    let mut pc = device_with("Desktop", DeviceKind::Desktop, &[nectarlink_core::SMS_SHOW]).await;
    let mut phone =
        device_with("Pixel", DeviceKind::Phone, &[nectarlink_core::SMS_READ, nectarlink_core::SMS_SEND])
            .await;
    pair_qr(&mut pc, &mut phone).await;
    let phone_id = phone.node.device_id();

    let jpeg = SmsAttachment { mime: "image/jpeg".into(), data: vec![0xff, 0xd8, 1, 2, 3, 4] };
    let png = SmsAttachment { mime: "image/png".into(), data: vec![0x89, b'P', b'N', b'G', 5, 6] };

    // Send MMS with both caption and a JPEG attachment.
    with_timeout(
        "send mms with caption",
        pc.node.send_sms_with_attachments(
            phone_id,
            vec!["+15550100".into()],
            "Check this out".into(),
            vec![jpeg.clone()],
        ),
    )
    .await
    .unwrap();

    // Send image-only MMS (empty body, PNG attachment).
    with_timeout(
        "send image-only mms",
        pc.node.send_sms_with_attachments(
            phone_id,
            vec!["+15550188".into()],
            String::new(),
            vec![png.clone()],
        ),
    )
    .await
    .unwrap();

    let sent = phone.platform.mms_sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].0, vec!["+15550100".to_owned()]);
    assert_eq!(sent[0].1, "Check this out");
    assert_eq!(sent[0].2, vec![jpeg.clone()]);
    assert_eq!(sent[1].0, vec!["+15550188".to_owned()]);
    assert_eq!(sent[1].1, "");
    assert_eq!(sent[1].2, vec![png.clone()]);

    // Unsupported MIME type is rejected as a protocol error; oversized or too many attachments return TooLarge.
    let bad_mime = SmsAttachment { mime: "application/x-executable".into(), data: vec![1, 2, 3] };
    assert!(matches!(
        pc.node
            .send_sms_with_attachments(phone_id, vec!["+15550100".into()], String::new(), vec![bad_mime])
            .await,
        Err(Error::Protocol(_))
    ));
    assert!(matches!(
        pc.node
            .send_sms_with_attachments(phone_id, vec!["+15550100".into()], String::new(), vec![jpeg, png])
            .await,
        Err(Error::TooLarge)
    ));
    let oversized =
        SmsAttachment { mime: "image/jpeg".into(), data: vec![0u8; SMS_MAX_ATTACHMENT_BYTES + 1] };
    assert!(matches!(
        pc.node
            .send_sms_with_attachments(phone_id, vec!["+15550100".into()], String::new(), vec![oversized])
            .await,
        Err(Error::TooLarge)
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn continuity_camera_photo_and_scan_flow() {
    use nectarlink_core::{CameraCaptureMode, CameraCaptureResultMeta};

    let mut pc = device_with("pc", DeviceKind::Desktop, &["camera.continuity"]).await;
    let mut phone = device_with("phone", DeviceKind::Phone, &["camera.capture"]).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    assert_eq!(
        pc.node.capabilities(phone_id).unwrap().state("camera.continuity"),
        Some(FeatureState::Available)
    );

    // 1. Request a photo capture from PC.
    with_timeout(
        "request photo capture",
        pc.node.request_camera_capture(phone_id, "req-photo-1".into(), CameraCaptureMode::Photo),
    )
    .await
    .unwrap();

    let (req_peer, req_id, req_mode) = wait_for(&mut phone, "photo capture requested", |e| match e {
        NodeEvent::CameraCaptureRequested { device, request_id, mode } => {
            Some((*device, request_id.clone(), *mode))
        }
        _ => None,
    })
    .await;
    assert_eq!(req_peer, pc_id);
    assert_eq!(req_id, "req-photo-1");
    assert_eq!(req_mode, CameraCaptureMode::Photo);

    let fake_jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3, 4, 0xFF, 0xD9];
    let photo_meta = CameraCaptureResultMeta {
        request_id: "req-photo-1".into(),
        mode: CameraCaptureMode::Photo,
        file_name: "IMG_20260409.jpg".into(),
        mime: "image/jpeg".into(),
        size: fake_jpeg.len() as u64,
        width: 1920,
        height: 1080,
    };
    with_timeout(
        "send photo capture result",
        phone.node.send_camera_capture_result(pc_id, photo_meta, fake_jpeg.clone()),
    )
    .await
    .unwrap();

    let (recv_peer, recv_id, recv_mode, recv_name, recv_mime, recv_w, recv_h, recv_data, saved_path) =
        wait_for(&mut pc, "photo capture received", |e| match e {
            NodeEvent::CameraCaptureReceived {
                device,
                request_id,
                mode,
                file_name,
                mime,
                width,
                height,
                data,
                saved_path,
            } => Some((
                *device,
                request_id.clone(),
                *mode,
                file_name.clone(),
                mime.clone(),
                *width,
                *height,
                data.clone(),
                saved_path.clone(),
            )),
            _ => None,
        })
        .await;
    assert_eq!(recv_peer, phone_id);
    assert_eq!(recv_id, "req-photo-1");
    assert_eq!(recv_mode, CameraCaptureMode::Photo);
    assert_eq!(recv_name, "IMG_20260409.jpg");
    assert_eq!(recv_mime, "image/jpeg");
    assert_eq!(recv_w, 1920);
    assert_eq!(recv_h, 1080);
    assert_eq!(recv_data, fake_jpeg);
    let path = saved_path.expect("saved to downloads_dir");
    assert_eq!(std::fs::read(&path).unwrap(), fake_jpeg);

    // 2. Request a document scan from PC.
    with_timeout(
        "request scan capture",
        pc.node.request_camera_capture(phone_id, "req-scan-2".into(), CameraCaptureMode::Scan),
    )
    .await
    .unwrap();

    let (_, scan_id, scan_mode) = wait_for(&mut phone, "scan capture requested", |e| match e {
        NodeEvent::CameraCaptureRequested { device, request_id, mode } => {
            Some((*device, request_id.clone(), *mode))
        }
        _ => None,
    })
    .await;
    assert_eq!(scan_id, "req-scan-2");
    assert_eq!(scan_mode, CameraCaptureMode::Scan);

    let fake_scan = vec![0xFF, 0xD8, 0xFF, 0xDB, 9, 8, 7, 6, 5, 0xFF, 0xD9];
    let scan_meta = CameraCaptureResultMeta {
        request_id: "req-scan-2".into(),
        mode: CameraCaptureMode::Scan,
        // A drive-prefixed name must still land in the downloads folder.
        file_name: "C:Scan_20260409.jpg".into(),
        mime: "image/jpeg".into(),
        size: fake_scan.len() as u64,
        width: 1654,
        height: 2339,
    };
    with_timeout(
        "send scan capture result",
        phone.node.send_camera_capture_result(pc_id, scan_meta, fake_scan.clone()),
    )
    .await
    .unwrap();

    let (_, got_scan_id, got_scan_mode, got_scan_name, _, got_w, got_h, got_scan_data, got_scan_path) =
        wait_for(&mut pc, "scan capture received", |e| match e {
            NodeEvent::CameraCaptureReceived {
                device,
                request_id,
                mode,
                file_name,
                mime,
                width,
                height,
                data,
                saved_path,
            } => Some((
                *device,
                request_id.clone(),
                *mode,
                file_name.clone(),
                mime.clone(),
                *width,
                *height,
                data.clone(),
                saved_path.clone(),
            )),
            _ => None,
        })
        .await;
    assert_eq!(got_scan_id, "req-scan-2");
    assert_eq!(got_scan_mode, CameraCaptureMode::Scan);
    assert_eq!(got_scan_name, "C_Scan_20260409.jpg");
    assert_eq!(got_w, 1654);
    assert_eq!(got_h, 2339);
    assert_eq!(got_scan_data, fake_scan);
    let got_scan_path = got_scan_path.unwrap();
    assert!(got_scan_path.exists());
    assert_eq!(got_scan_path.parent(), path.parent());
    assert_eq!(got_scan_path.file_name().unwrap(), "C_Scan_20260409.jpg");

    pc.node.shutdown().await;
    phone.node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn continuity_camera_cancel_propagates() {
    use nectarlink_core::CameraCaptureMode;

    let mut pc = device_with("pc", DeviceKind::Desktop, &["camera.continuity"]).await;
    let mut phone = device_with("phone", DeviceKind::Phone, &["camera.capture"]).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    // Phone cancels an active request -> PC receives CameraCaptureCancelled.
    with_timeout(
        "request capture 1",
        pc.node.request_camera_capture(phone_id, "req-cancel-1".into(), CameraCaptureMode::Photo),
    )
    .await
    .unwrap();
    wait_for(&mut phone, "capture 1 requested", |e| match e {
        NodeEvent::CameraCaptureRequested { request_id, .. } if request_id == "req-cancel-1" => Some(()),
        _ => None,
    })
    .await;

    with_timeout(
        "phone cancels capture 1",
        phone.node.cancel_camera_capture(pc_id, "req-cancel-1".into(), Some("user dismissed".into())),
    )
    .await
    .unwrap();

    let (from, id, reason) = wait_for(&mut pc, "pc sees cancel 1", |e| match e {
        NodeEvent::CameraCaptureCancelled { device, request_id, reason } => {
            Some((*device, request_id.clone(), reason.clone()))
        }
        _ => None,
    })
    .await;
    assert_eq!(from, phone_id);
    assert_eq!(id, "req-cancel-1");
    assert_eq!(reason.as_deref(), Some("user dismissed"));

    // PC cancels an active request -> Phone receives CameraCaptureCancelled.
    with_timeout(
        "request capture 2",
        pc.node.request_camera_capture(phone_id, "req-cancel-2".into(), CameraCaptureMode::Scan),
    )
    .await
    .unwrap();
    wait_for(&mut phone, "capture 2 requested", |e| match e {
        NodeEvent::CameraCaptureRequested { request_id, .. } if request_id == "req-cancel-2" => Some(()),
        _ => None,
    })
    .await;

    with_timeout(
        "pc cancels capture 2",
        pc.node.cancel_camera_capture(phone_id, "req-cancel-2".into(), Some("cancelled on PC".into())),
    )
    .await
    .unwrap();

    let (from2, id2, reason2) = wait_for(&mut phone, "phone sees cancel 2", |e| match e {
        NodeEvent::CameraCaptureCancelled { device, request_id, reason } => {
            Some((*device, request_id.clone(), reason.clone()))
        }
        _ => None,
    })
    .await;
    assert_eq!(from2, pc_id);
    assert_eq!(id2, "req-cancel-2");
    assert_eq!(reason2.as_deref(), Some("cancelled on PC"));

    pc.node.shutdown().await;
    phone.node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn continuity_camera_respects_photos_toggle() {
    use nectarlink_core::CameraCaptureMode;

    let mut pc = device_with("pc", DeviceKind::Desktop, &["camera.continuity"]).await;
    let mut phone = device_with("phone", DeviceKind::Phone, &["camera.capture"]).await;
    pair_qr(&mut pc, &mut phone).await;
    let (_pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    // Turning off photos toggle on PC blocks outgoing request_camera_capture with Error::Denied.
    pc.node.set_device_toggle(phone_id, "photos", false).unwrap();
    let res =
        pc.node.request_camera_capture(phone_id, "req-denied-pc".into(), CameraCaptureMode::Photo).await;
    assert!(matches!(res, Err(Error::Denied)));

    // Re-enable on PC, but disable photos toggle on phone -> phone replies with cancel.
    pc.node.set_device_toggle(phone_id, "photos", true).unwrap();
    phone.node.set_device_toggle(pc.node.device_id(), "photos", false).unwrap();
    with_timeout(
        "request with phone photos toggle off",
        pc.node.request_camera_capture(phone_id, "req-denied-phone".into(), CameraCaptureMode::Photo),
    )
    .await
    .unwrap();

    let (from, id, reason) = wait_for(&mut pc, "phone cancels because photos off", |e| match e {
        NodeEvent::CameraCaptureCancelled { device, request_id, reason } => {
            Some((*device, request_id.clone(), reason.clone()))
        }
        _ => None,
    })
    .await;
    assert_eq!(from, phone_id);
    assert_eq!(id, "req-denied-phone");
    assert!(reason.unwrap_or_default().contains("photos"));

    pc.node.shutdown().await;
    phone.node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn data_retention_counts_and_clear_all_local_data_preserves_pairing_and_settings() {
    use nectarlink_core::{
        Notification, NotificationAction, NotificationChatMessage, NotificationConversation,
    };

    let mut pc = device_with("pc", DeviceKind::Desktop, &["notify.show", "clip.write"]).await;
    let mut phone = device_with("phone", DeviceKind::Phone, &["notify.mirror", "notify.reply"]).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    // Configure non-default retention and a device toggle on PC.
    pc.node
        .set_timeline_retention(nectarlink_core::timeline::TimelineRetention {
            max_days: 30,
            max_entries: 5000,
        })
        .unwrap();
    pc.node.set_device_toggle(phone_id, "clipboard", true).unwrap();

    // Send a clipboard item from phone -> PC.
    with_timeout("send clip", phone.node.send_clipboard(pc_id, "Secret recipe notes".into())).await.unwrap();
    wait_for(&mut pc, "clip history changed", |e| match e {
        NodeEvent::ClipboardHistoryChanged => Some(()),
        _ => None,
    })
    .await;

    // Post a MessagingStyle chat notification from phone -> PC.
    let note = Notification {
        key: "sig|sam".into(),
        app: "org.thoughtcrime.securesms".into(),
        app_name: "Signal".into(),
        title: Some("Sam Rivera".into()),
        text: Some("Dinner at 7?".into()),
        sub: None,
        when: 1_760_000_000_000,
        actions: vec![NotificationAction { id: "reply".into(), title: "Reply".into(), reply: true }],
        silent: false,
        icon: None,
        image: None,
        live: None,
        conversation: Some(NotificationConversation {
            v: 1,
            title: "Sam Rivera".into(),
            group: false,
            avatar: None,
            messages: vec![NotificationChatMessage {
                sender: Some("Sam Rivera".into()),
                text: "Dinner at 7?".into(),
                time: 1_760_000_000_000,
                self_sent: false,
                avatar: None,
            }],
        }),
    };
    phone.node.notification_posted(note).await;
    wait_for(&mut pc, "chat sms changed", |e| match e {
        NodeEvent::SmsChanged { device, .. } if *device == phone_id => Some(()),
        _ => None,
    })
    .await;

    let counts = pc.node.data_retention_counts();
    assert!(counts.clipboard_items >= 1);
    assert!(counts.timeline_items >= 1);
    assert_eq!(counts.timeline_retention_days, 30);
    assert_eq!(counts.chat_threads, 1);
    assert_eq!(counts.chat_messages, 1);

    // Clear everything on PC: all history and caches become 0, while pairing and settings remain intact.
    pc.node.clear_all_local_data().unwrap();
    let after = pc.node.data_retention_counts();
    assert_eq!(after.clipboard_items, 0);
    assert_eq!(after.timeline_items, 0);
    assert_eq!(after.chat_threads, 0);
    assert_eq!(after.chat_messages, 0);
    assert_eq!(after.received_file_records, 0);
    assert_eq!(after.timeline_retention_days, 30, "timeline retention setting must be preserved");
    assert_eq!(pc.node.paired_devices().unwrap().len(), 1, "paired devices must never be deleted");
    assert!(
        pc.node.device_toggles(phone_id).unwrap().into_iter().any(|(k, v)| k == "clipboard" && v),
        "per-device toggles must be preserved"
    );

    pc.node.shutdown().await;
    phone.node.shutdown().await;
}

fn free_tcp_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local addr")
        .port()
}

async fn start_localsend_in(dir: TempDir, name: &'static str, kind: DeviceKind, port: u16) -> TestDevice {
    init_logging();
    let mut config = NodeConfig::new(dir.path(), info(name, kind), "0.0.1-test");
    config.lan_discovery = false;
    config.away_mode = false;
    config.localsend_port = port;
    config.key_protector = Some(Arc::new(PlainKeyProtector));
    config.capabilities = vec!["media.control".into()];
    let platform = Arc::new(RecordingPlatform::default());
    let node = Node::start(config, platform.clone()).await.expect("node starts");
    let events = node.events();
    TestDevice { node, events, platform, dir, name }
}

#[tokio::test]
async fn localsend_is_off_by_default_and_persists_setting() {
    let dir = tempfile::tempdir().unwrap();
    let port1 = free_tcp_port();
    let mut dev = start_localsend_in(dir, "PC", DeviceKind::Desktop, port1).await;

    assert!(!dev.node.localsend_enabled());
    assert!(!dev.node.localsend_receiving());
    assert!(dev.node.localsend_peers().is_empty());

    dev.node.set_localsend_enabled(true).unwrap();
    wait_for(&mut dev, "localsend enabled event", |e| match e {
        NodeEvent::LocalSendChanged => Some(()),
        _ => None,
    })
    .await;
    assert!(dev.node.localsend_enabled());
    // Wait up to 2s for the background TCP listener to bind and set receiving=true.
    for _ in 0..40 {
        if dev.node.localsend_receiving() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(dev.node.localsend_receiving());

    let dir = dev.dir;
    dev.node.shutdown().await;

    let port2 = free_tcp_port();
    let dev2 = start_localsend_in(dir, "PC", DeviceKind::Desktop, port2).await;
    assert!(dev2.node.localsend_enabled(), "enabled state must persist across restarts");
    for _ in 0..40 {
        if dev2.node.localsend_receiving() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(dev2.node.localsend_receiving(), "server must start automatically when persisted enabled");
    dev2.node.shutdown().await;
}

/// Apps turn LocalSend on from their UI thread, outside the node's runtime.
#[tokio::test(flavor = "multi_thread")]
async fn localsend_turns_on_from_a_thread_outside_the_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let mut dev = start_localsend_in(dir, "PC", DeviceKind::Desktop, free_tcp_port()).await;
    let node = dev.node.clone();
    std::thread::spawn(move || {
        node.set_localsend_enabled(true).unwrap();
        node.refresh_localsend();
    })
    .join()
    .expect("turning LocalSend on outside the runtime doesn't panic");
    for _ in 0..40 {
        if dev.node.localsend_receiving() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(dev.node.localsend_receiving());
    dev.node.set_localsend_enabled(false).unwrap();
    wait_for(&mut dev, "localsend changed event", |e| match e {
        NodeEvent::LocalSendChanged => Some(()),
        _ => None,
    })
    .await;
    dev.node.shutdown().await;
}

#[tokio::test]
async fn localsend_coexists_when_port_is_already_bound() {
    let occupied = std::net::TcpListener::bind(("0.0.0.0", 0)).expect("bind occupied port");
    let port = occupied.local_addr().unwrap().port();

    let dir = tempfile::tempdir().unwrap();
    let dev = start_localsend_in(dir, "PC", DeviceKind::Desktop, port).await;
    dev.node.set_localsend_enabled(true).unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;

    assert!(dev.node.localsend_enabled());
    assert!(
        !dev.node.localsend_receiving(),
        "when port is occupied by standalone LocalSend, NectarLink must leave receiving to it without failing"
    );

    dev.node.shutdown().await;
    drop(occupied);
}

#[tokio::test]
async fn localsend_discovers_peer_requires_accept_and_transfers_files_over_https() {
    let sender_port = free_tcp_port();
    let receiver_port = free_tcp_port();
    let mut sender =
        start_localsend_in(tempfile::tempdir().unwrap(), "Phone Sender", DeviceKind::Phone, sender_port)
            .await;
    let mut receiver =
        start_localsend_in(tempfile::tempdir().unwrap(), "PC Receiver", DeviceKind::Desktop, receiver_port)
            .await;

    sender.node.set_localsend_enabled(true).unwrap();
    receiver.node.set_localsend_enabled(true).unwrap();
    for _ in 0..40 {
        if receiver.node.localsend_receiving() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(receiver.node.localsend_receiving());

    let peer = sender
        .node
        .probe_localsend_peer("127.0.0.1", receiver_port)
        .await
        .expect("probe receiver over HTTPS");
    assert_eq!(peer.alias, "PC Receiver");
    assert_eq!(peer.port, receiver_port);
    assert_eq!(peer.protocol, "https");
    assert_eq!(peer.device_type, "desktop");
    assert!(sender.node.localsend_peers().iter().any(|p| p.id == peer.id));

    let src_dir = sender.dir.path().join("src_files");
    std::fs::create_dir_all(src_dir.join("sub")).unwrap();
    let file_a = src_dir.join("notes.txt");
    let file_b = src_dir.join("sub").join("hello.txt");
    std::fs::write(&file_a, b"LocalSend interop over HTTPS!").unwrap();
    std::fs::write(&file_b, b"Nested folder payload").unwrap();

    let out_id = sender
        .node
        .send_files(
            peer.id,
            vec![
                OutgoingFile { name: "notes.txt".into(), folder: None, source: FileSource::Path(file_a) },
                OutgoingFile {
                    name: "hello.txt".into(),
                    folder: Some("sub".into()),
                    source: FileSource::Path(file_b),
                },
            ],
        )
        .await
        .expect("start LocalSend upload");

    let req: Transfer = wait_for(&mut receiver, "incoming LocalSend transfer request", |e| match e {
        NodeEvent::Transfer(t)
            if t.direction == Direction::Incoming && t.state == TransferState::Requested =>
        {
            Some(t.clone())
        }
        _ => None,
    })
    .await;
    assert_eq!(req.files, 2);
    assert!(
        !receiver.dir.path().join("received").join("notes.txt").exists(),
        "unpaired LocalSend transfer must wait for user acceptance before writing files"
    );

    receiver.node.accept_transfer(&req.id);

    wait_for(&mut sender, "sender LocalSend transfer done", |e| match e {
        NodeEvent::Transfer(t) if t.id == out_id && matches!(t.state, TransferState::Done { .. }) => Some(()),
        _ => None,
    })
    .await;
    wait_for(&mut receiver, "receiver LocalSend transfer done", |e| match e {
        NodeEvent::Transfer(t) if t.id == req.id && matches!(t.state, TransferState::Done { .. }) => Some(()),
        _ => None,
    })
    .await;

    let dl_a = receiver.dir.path().join("received").join("notes.txt");
    let dl_b = receiver.dir.path().join("received").join("sub").join("hello.txt");
    assert_eq!(std::fs::read(&dl_a).unwrap(), b"LocalSend interop over HTTPS!");
    assert_eq!(std::fs::read(&dl_b).unwrap(), b"Nested folder payload");

    assert!(
        !sender
            .node
            .timeline_page(&nectarlink_core::timeline::TimelineQuery::default())
            .unwrap()
            .entries
            .is_empty()
    );
    assert!(
        !receiver
            .node
            .timeline_page(&nectarlink_core::timeline::TimelineQuery::default())
            .unwrap()
            .entries
            .is_empty()
    );

    sender.node.shutdown().await;
    receiver.node.shutdown().await;
}

/// One HTTP/1.1 request over plain TCP to a LocalSend server, as a LocalSend
/// sender would send it; returns the status and the body.
async fn localsend_request(port: u16, method: &str, target: &str, body: &[u8]) -> (u16, Vec<u8>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut tcp = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let head = format!(
        "{method} {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    tcp.write_all(head.as_bytes()).await.unwrap();
    tcp.write_all(body).await.unwrap();
    let mut out = Vec::new();
    let _ = tcp.read_to_end(&mut out).await;
    let text = String::from_utf8_lossy(&out);
    let status = text.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let split = out.windows(4).position(|w| w == b"\r\n\r\n").map_or(out.len(), |i| i + 4);
    (status, out[split..].to_vec())
}

fn localsend_offer(files: &[(&str, &str, &str, u64)]) -> Vec<u8> {
    let files: serde_json::Map<String, serde_json::Value> = files
        .iter()
        .map(|(key, id, name, size)| {
            (
                (*key).to_owned(),
                serde_json::json!({ "id": id, "fileName": name, "size": size, "fileType": "text/plain" }),
            )
        })
        .collect();
    serde_json::to_vec(&serde_json::json!({
        "info": { "alias": "Raw sender", "version": "2.1", "deviceType": "mobile", "fingerprint": "raw-sender-fp", "port": 1, "protocol": "http" },
        "files": files,
    }))
    .unwrap()
}

fn part_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(part_files(&path));
        } else if path.extension().is_some_and(|e| e == "part") {
            found.push(path);
        }
    }
    found
}

/// A sender that offers duplicate file ids is refused; one that sends one of
/// two files and cancels leaves that file, a timeline entry saying it only
/// partly arrived, no staged parts, and no session (#72 audit).
#[tokio::test]
async fn localsend_refuses_duplicate_ids_and_records_a_cancelled_partial_delivery() {
    let port = free_tcp_port();
    let mut receiver =
        start_localsend_in(tempfile::tempdir().unwrap(), "PC Receiver", DeviceKind::Desktop, port).await;
    receiver.node.set_localsend_enabled(true).unwrap();
    for _ in 0..40 {
        if receiver.node.localsend_receiving() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(receiver.node.localsend_receiving());

    let (status, _) = localsend_request(
        port,
        "POST",
        "/api/localsend/v2/prepare-upload",
        &localsend_offer(&[("a", "same", "a.txt", 5), ("b", "same", "b.txt", 5)]),
    )
    .await;
    assert_eq!(status, 400, "two files with one id could never both arrive");

    let offer = localsend_offer(&[("f1", "f1", "first.txt", 5), ("f2", "f2", "second.txt", 6)]);
    let prepare = tokio::spawn(async move {
        localsend_request(port, "POST", "/api/localsend/v2/prepare-upload", &offer).await
    });
    let req: Transfer = wait_for(&mut receiver, "offer", |e| match e {
        NodeEvent::Transfer(t) if t.state == TransferState::Requested => Some(t.clone()),
        _ => None,
    })
    .await;
    receiver.node.accept_transfer(&req.id);
    let (status, body) = with_timeout("prepare-upload", prepare).await.unwrap();
    assert_eq!(status, 200);
    let answer: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let session = answer["sessionId"].as_str().unwrap().to_owned();
    let token = answer["files"]["f1"].as_str().unwrap().to_owned();

    let (status, _) = localsend_request(
        port,
        "POST",
        &format!("/api/localsend/v2/upload?sessionId={session}&fileId=f1&token={token}"),
        b"hello",
    )
    .await;
    assert_eq!(status, 200);
    let (status, _) = localsend_request(
        port,
        "POST",
        &format!("/api/localsend/v2/upload?sessionId={session}&fileId=f1&token={token}"),
        b"again",
    )
    .await;
    assert_eq!(status, 403, "a file that arrived isn't uploaded twice");

    let (status, _) =
        localsend_request(port, "POST", &format!("/api/localsend/v2/cancel?sessionId={session}"), b"").await;
    assert_eq!(status, 200);
    wait_for(&mut receiver, "cancelled", |e| match e {
        NodeEvent::Transfer(t) if t.id == req.id && t.state == TransferState::Cancelled => Some(()),
        _ => None,
    })
    .await;

    let saved = receiver.dir.path().join("received").join("first.txt");
    assert_eq!(std::fs::read(&saved).unwrap(), b"hello", "what arrived is kept");
    let entries =
        receiver.node.timeline_page(&nectarlink_core::timeline::TimelineQuery::default()).unwrap().entries;
    assert!(
        entries.iter().any(|e| e.detail.contains("1 of 2 files") && e.target.contains("first.txt")),
        "{:?}",
        entries.iter().map(|e| &e.detail).collect::<Vec<_>>()
    );
    assert!(part_files(receiver.dir.path()).is_empty(), "no staged parts are left");
    let (status, _) = localsend_request(
        port,
        "POST",
        &format!("/api/localsend/v2/upload?sessionId={session}&fileId=f2&token=x"),
        b"second",
    )
    .await;
    assert_eq!(status, 404, "the session is gone");

    receiver.node.shutdown().await;
}

#[tokio::test]
async fn localsend_declining_transfer_returns_denied_to_sender() {
    let sender_port = free_tcp_port();
    let receiver_port = free_tcp_port();
    let mut sender =
        start_localsend_in(tempfile::tempdir().unwrap(), "Phone Sender", DeviceKind::Phone, sender_port)
            .await;
    let mut receiver =
        start_localsend_in(tempfile::tempdir().unwrap(), "PC Receiver", DeviceKind::Desktop, receiver_port)
            .await;

    sender.node.set_localsend_enabled(true).unwrap();
    receiver.node.set_localsend_enabled(true).unwrap();
    for _ in 0..40 {
        if receiver.node.localsend_receiving() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let peer = sender.node.probe_localsend_peer("127.0.0.1", receiver_port).await.unwrap();

    let file_a = sender.dir.path().join("reject_me.txt");
    std::fs::write(&file_a, b"do not accept").unwrap();

    let out_id = sender
        .node
        .send_files(
            peer.id,
            vec![OutgoingFile {
                name: "reject_me.txt".into(),
                folder: None,
                source: FileSource::Path(file_a),
            }],
        )
        .await
        .unwrap();

    let req: Transfer = wait_for(&mut receiver, "incoming LocalSend transfer request", |e| match e {
        NodeEvent::Transfer(t)
            if t.direction == Direction::Incoming && t.state == TransferState::Requested =>
        {
            Some(t.clone())
        }
        _ => None,
    })
    .await;

    receiver.node.cancel_transfer(&req.id);

    wait_for(&mut sender, "sender sees Denied failure", |e| match e {
        NodeEvent::Transfer(t)
            if t.id == out_id && t.state == TransferState::Failed(TransferFailure::Denied) =>
        {
            Some(())
        }
        _ => None,
    })
    .await;

    assert!(!receiver.dir.path().join("received").join("reject_me.txt").exists());

    sender.node.shutdown().await;
    receiver.node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn transfer_blake3_integrity_verified_and_mismatch_keeps_partial() {
    let mut pc = device("Desktop", DeviceKind::Desktop).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());
    let src = tempfile::tempdir().unwrap();

    // 1. A transfer whose bytes are altered on the sender after the offer hash is computed
    //    must fail with TransferFailure::Integrity on both sides and keep the .part file.
    let payload = data(48 * 1024 * 1024, 42);
    let src_path = src.path().join("corrupt_me.bin");
    std::fs::write(&src_path, &payload).unwrap();

    let id = pc
        .node
        .send_files(
            phone_id,
            vec![OutgoingFile {
                name: "corrupt_me.bin".into(),
                folder: None,
                source: FileSource::Path(src_path.clone()),
            }],
        )
        .await
        .unwrap();

    // Paused as soon as it runs: the offer and its hash are sent by then, and
    // waiting for the first progress report lets an optimized build finish
    // all 48 MB before it comes.
    wait_transfer(&mut pc, &id, "sending before pause", |t| {
        (t.state == TransferState::Running).then_some(())
    })
    .await;
    pc.node.pause_transfer(&id);
    wait_transfer(&mut pc, &id, "sender paused", |t| (t.state == TransferState::Paused).then_some(())).await;
    wait_transfer(&mut phone, &id, "receiver paused", |t| (t.state == TransferState::Paused).then_some(()))
        .await;

    // Flip the last byte of the source file while paused so the remaining stream bytes
    // no longer match the BLAKE3 digest in the offer.
    let mut corrupted = payload.clone();
    let last = corrupted.len() - 1;
    corrupted[last] ^= 0xFF;
    std::fs::write(&src_path, &corrupted).unwrap();

    pc.node.resume_transfer(&id);

    let phone_state = wait_transfer(&mut phone, &id, "receiver integrity failure", |t| {
        t.state.is_finished().then(|| t.state.clone())
    })
    .await;
    let pc_state = wait_transfer(&mut pc, &id, "sender integrity failure", |t| {
        t.state.is_finished().then(|| t.state.clone())
    })
    .await;
    assert_eq!(phone_state, TransferState::Failed(TransferFailure::Integrity));
    assert_eq!(pc_state, TransferState::Failed(TransferFailure::Integrity));

    // The partial file is kept in incoming/<pc_id>/<id>/0.part and NOT moved to received/.
    let part_path = phone.dir.path().join("incoming").join(pc_id.to_string()).join(&id).join("0.part");
    assert!(part_path.exists(), "partial .part file must be kept on integrity failure");
    assert!(!phone.dir.path().join("received").join("corrupt_me.bin").exists());

    // 2. Restore the valid bytes and send again: normal BLAKE3 verification succeeds.
    std::fs::write(&src_path, &payload).unwrap();
    let id2 = pc
        .node
        .send_files(
            phone_id,
            vec![OutgoingFile {
                name: "corrupt_me.bin".into(),
                folder: None,
                source: FileSource::Path(src_path),
            }],
        )
        .await
        .unwrap();
    let received = wait_transfer(&mut phone, &id2, "valid transfer received", saved).await;
    assert_eq!(std::fs::read(&received[0]).unwrap(), payload);
    wait_transfer(&mut pc, &id2, "valid transfer sent", saved).await;

    pc.node.shutdown().await;
    phone.node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn transfer_manual_pause_and_resume_both_sides() {
    let mut pc = device("Desktop", DeviceKind::Desktop).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());
    let src = tempfile::tempdir().unwrap();

    let big = data(64 * 1024 * 1024, 77);
    let id = pc.node.send_files(phone_id, vec![outgoing(src.path(), "movie.bin", &big)]).await.unwrap();

    // 1. Pause from the sender (PC) as soon as it runs: an optimized build can
    //    send all 64 MB before the first progress report.
    wait_transfer(&mut phone, &id, "receiving initial", |t| {
        (t.state == TransferState::Running).then_some(())
    })
    .await;
    pc.node.pause_transfer(&id);

    wait_transfer(&mut pc, &id, "pc paused", |t| (t.state == TransferState::Paused).then_some(())).await;
    wait_transfer(&mut phone, &id, "phone paused", |t| (t.state == TransferState::Paused).then_some(()))
        .await;

    let part_path = phone.dir.path().join("incoming").join(pc_id.to_string()).join(&id).join("0.part");
    assert!(part_path.exists(), "partial file kept while paused");

    // 2. Resume from the sender (PC), then pause from the receiver (phone), then resume from the receiver.
    pc.node.resume_transfer(&id);
    wait_transfer(&mut phone, &id, "phone running after sender resume", |t| {
        (t.state == TransferState::Running).then_some(())
    })
    .await;

    phone.node.pause_transfer(&id);
    wait_transfer(&mut phone, &id, "phone paused by receiver", |t| {
        (t.state == TransferState::Paused).then_some(())
    })
    .await;
    wait_transfer(&mut pc, &id, "pc paused by receiver", |t| {
        (t.state == TransferState::Paused).then_some(())
    })
    .await;

    phone.node.resume_transfer(&id);
    let received = wait_transfer(&mut phone, &id, "received after receiver resume", saved).await;
    assert_eq!(std::fs::read(&received[0]).unwrap(), big);
    wait_transfer(&mut pc, &id, "sent after receiver resume", saved).await;

    // 3. Pause a transfer and cancel it while paused from the receiver: partial files are cleaned up
    //    and both sides transition to Cancelled.
    let id2 =
        pc.node.send_files(phone_id, vec![outgoing(src.path(), "cancel_paused.bin", &big)]).await.unwrap();
    wait_transfer(&mut phone, &id2, "receiving second", |t| {
        (t.state == TransferState::Running).then_some(())
    })
    .await;
    phone.node.pause_transfer(&id2);
    wait_transfer(&mut phone, &id2, "phone paused second", |t| {
        (t.state == TransferState::Paused).then_some(())
    })
    .await;
    wait_transfer(&mut pc, &id2, "pc paused second", |t| (t.state == TransferState::Paused).then_some(()))
        .await;

    phone.node.cancel_transfer(&id2);
    let phone_end = wait_transfer(&mut phone, &id2, "phone cancelled from pause", |t| {
        t.state.is_finished().then(|| t.state.clone())
    })
    .await;
    let pc_end = wait_transfer(&mut pc, &id2, "pc cancelled from pause", |t| {
        t.state.is_finished().then(|| t.state.clone())
    })
    .await;
    assert_eq!((phone_end, pc_end), (TransferState::Cancelled, TransferState::Cancelled));
    let part_dir2 = phone.dir.path().join("incoming").join(pc_id.to_string()).join(&id2);
    assert!(!part_dir2.exists(), "cancelled paused transfer removes incoming directory");

    pc.node.shutdown().await;
    phone.node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn battery_signal_strength_and_peer_diagnostics_flow() {
    let mut pc = device("Desktop", DeviceKind::Desktop).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;
    pair_qr(&mut pc, &mut phone).await;
    let (_pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    // 1. Signal strength fields in Battery flow intact and clamp/sanitize on the wire.
    let signal_battery = Battery {
        level: 64,
        charging: false,
        plugged: None,
        full_in: None,
        cell_bars: Some(4),
        cell_type: Some("5G".into()),
        wifi_bars: Some(3),
    };
    phone.node.update_battery(signal_battery.clone()).await;
    let received = wait_for(&mut pc, "battery with signal", |e| match e {
        NodeEvent::Battery { device, battery } if *device == phone_id => Some(battery.clone()),
        _ => None,
    })
    .await;
    assert_eq!(received, signal_battery);

    // 2. PeerDiagnostics starts empty when mDNS is disabled in test harness, and records
    //    LAN sightings and dial failures accurately.
    let initial = pc.node.peer_diagnostics(&phone_id);
    assert_eq!(initial.mdns_seen_ago_secs, None);
    assert!(initial.mdns_addrs.is_empty());
    assert_eq!(initial.dial_failures, 0);

    let lan_addr: SocketAddr = "192.168.1.55:4242".parse().unwrap();
    pc.node.record_lan_sighting(phone_id, vec![lan_addr]);
    pc.node.record_dial_failure(phone_id);
    pc.node.record_dial_failure(phone_id);

    let diag = pc.node.peer_diagnostics(&phone_id);
    assert!(diag.mdns_seen_ago_secs.is_some_and(|s| s < 5));
    assert_eq!(diag.mdns_addrs, vec![lan_addr]);
    assert_eq!(diag.dial_failures, 2);

    // 3. Unpairing clears LAN sighting and dial failure diagnostics for the peer.
    with_timeout("unpair", pc.node.unpair(phone_id)).await.unwrap();
    let cleared = pc.node.peer_diagnostics(&phone_id);
    assert_eq!(cleared.mdns_seen_ago_secs, None);
    assert!(cleared.mdns_addrs.is_empty());
    assert_eq!(cleared.dial_failures, 0);

    pc.node.shutdown().await;
    phone.node.shutdown().await;
}

async fn start_usb_device(
    name: &'static str,
    kind: DeviceKind,
    usb_port: Option<u16>,
    offers: &[&str],
) -> TestDevice {
    start_usb_device_in(tempfile::tempdir().unwrap(), name, kind, usb_port, offers).await
}

async fn start_usb_device_in(
    dir: TempDir,
    name: &'static str,
    kind: DeviceKind,
    usb_port: Option<u16>,
    offers: &[&str],
) -> TestDevice {
    init_logging();
    let mut config = NodeConfig::new(dir.path(), info(name, kind), "0.0.1-test");
    config.lan_discovery = false;
    config.away_mode = false;
    config.usb_port = usb_port;
    config.adb_server = None;
    config.key_protector = Some(Arc::new(PlainKeyProtector));
    let mut caps = vec!["media.control".to_owned()];
    caps.extend(offers.iter().map(|c| (*c).to_owned()));
    config.capabilities = caps;
    let platform = Arc::new(RecordingPlatform::default());
    let node = Node::start(config, platform.clone()).await.expect("usb test node starts");
    let events = node.events();
    TestDevice { node, events, platform, dir, name }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_profiles_on_one_phone_each_get_their_own_usb_port() {
    // Android profiles (personal, work, a cloned app) share one loopback: both apps ask
    // for the same port, and the second takes the next one rather than a random port the
    // PC never tries. Each port then reaches its own profile.
    let base = loop {
        let base = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
        let Some(last) = base.checked_add(nectarlink_core::USB_PORT_COUNT) else { continue };
        if (base..=last).all(|p| std::net::TcpListener::bind(("127.0.0.1", p)).is_ok()) {
            break base;
        }
    };
    let personal = start_usb_device("Pixel", DeviceKind::Phone, Some(base), &[]).await;
    let work = start_usb_device("Pixel (Work)", DeviceKind::Phone, Some(base), &[]).await;
    let pc = start_usb_device("Desktop", DeviceKind::Desktop, None, &[]).await;

    // Other tests run alongside and take ports too, so which ones are free can vary; what
    // matters is that both stay in the range the PC tries, and apart.
    let range = base..base + nectarlink_core::USB_PORT_COUNT;
    let personal_port = personal.node.usb_port().expect("personal profile listens");
    let work_port = work.node.usb_port().expect("work profile listens");
    assert!(
        range.contains(&personal_port) && range.contains(&work_port),
        "{personal_port}, {work_port} in {range:?}"
    );
    assert_ne!(personal_port, work_port);

    let at = |port: u16| -> SocketAddr { ("127.0.0.1".parse::<std::net::IpAddr>().unwrap(), port).into() };
    let reached =
        pc.node.connect_usb_tunnel(at(personal_port)).await.expect("tunnel to the personal profile");
    assert_eq!(reached, personal.node.device_id());
    let reached = pc.node.connect_usb_tunnel(at(work_port)).await.expect("tunnel to the work profile");
    assert_eq!(reached, work.node.device_id());

    pc.node.shutdown().await;
    work.node.shutdown().await;
    personal.node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn usb_auto_pair_requires_consent_on_both_sides_and_connects_over_usb() {
    use nectarlink_core::ConnectionPath;

    let mut phone = start_usb_device("Pixel USB", DeviceKind::Phone, Some(0), &[]).await;
    let mut pc = start_usb_device("Desktop USB", DeviceKind::Desktop, None, &[]).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    let usb_port = phone.node.usb_port().expect("phone bound ephemeral USB TCP listener");
    let tunnel_addr: SocketAddr = ("127.0.0.1".parse::<std::net::IpAddr>().unwrap(), usb_port).into();

    pc.node.connect_usb_tunnel(tunnel_addr).await.expect("connect USB tunnel");

    // 1. Both PC and Phone emit NodeEvent::Discovered for each other.
    let mut pc_saw_discovered = false;
    let mut pc_code: Option<String> = None;
    while !pc_saw_discovered || pc_code.is_none() {
        let ev = wait_for(&mut pc, "pc discovered + sas code", |e| Some(e.clone())).await;
        match ev {
            NodeEvent::Discovered(d) if d.id == phone_id => pc_saw_discovered = true,
            NodeEvent::Pairing(PairingEvent::SasCode { peer, code }) if peer == phone_id => {
                pc_code = Some(code);
            }
            _ => {}
        }
    }

    let mut phone_saw_discovered = false;
    let mut phone_code: Option<String> = None;
    while !phone_saw_discovered || phone_code.is_none() {
        let ev = wait_for(&mut phone, "phone discovered + sas code", |e| Some(e.clone())).await;
        match ev {
            NodeEvent::Discovered(d) if d.id == pc_id => phone_saw_discovered = true,
            NodeEvent::Pairing(PairingEvent::SasCode { peer, code }) if peer == pc_id => {
                phone_code = Some(code);
            }
            _ => {}
        }
    }

    // 2. Both emitted matching 6-digit SAS codes without calling pairing_start_qr or pairing_start_nearby.
    let code_a = pc_code.unwrap();
    let code_b = phone_code.unwrap();
    assert_eq!(code_a.len(), 6);
    assert_eq!(code_a, code_b);

    // 3. Before user confirmation on both sides, neither device is paired.
    assert!(pc.node.paired_devices().unwrap().is_empty());
    assert!(phone.node.paired_devices().unwrap().is_empty());

    // 4. Declining on one side aborts pairing and keeps paired_devices() empty on both sides.
    phone.node.pairing_confirm(false).unwrap();
    let phone_fail = wait_for(&mut phone, "phone pairing declined", |e| match e {
        NodeEvent::Pairing(PairingEvent::Failed(r)) => Some(r.clone()),
        _ => None,
    })
    .await;
    let pc_fail = wait_for(&mut pc, "pc pairing rejected", |e| match e {
        NodeEvent::Pairing(PairingEvent::Failed(r)) => Some(r.clone()),
        _ => None,
    })
    .await;
    assert_eq!(phone_fail, PairingFailure::Declined);
    assert_eq!(pc_fail, PairingFailure::Rejected);
    assert!(pc.node.paired_devices().unwrap().is_empty());
    assert!(phone.node.paired_devices().unwrap().is_empty());

    // 5. Re-triggering over USB (via refresh) starts a fresh SAS prompt; confirming on both sides pairs
    //    and connects with ConnectionPath::Usb.
    pc.node.refresh().await;
    let code_pc_2 = wait_for(&mut pc, "pc second sas code", |e| match e {
        NodeEvent::Pairing(PairingEvent::SasCode { peer, code }) if *peer == phone_id => Some(code.clone()),
        _ => None,
    })
    .await;
    let code_phone_2 = wait_for(&mut phone, "phone second sas code", |e| match e {
        NodeEvent::Pairing(PairingEvent::SasCode { peer, code }) if *peer == pc_id => Some(code.clone()),
        _ => None,
    })
    .await;
    assert_eq!(code_pc_2, code_phone_2);

    pc.node.pairing_confirm(true).unwrap();
    phone.node.pairing_confirm(true).unwrap();

    wait_for(&mut pc, "pc paired over usb", |e| match e {
        NodeEvent::Pairing(PairingEvent::Paired(d)) if d.id == phone_id => Some(()),
        _ => None,
    })
    .await;
    wait_for(&mut phone, "phone paired over usb", |e| match e {
        NodeEvent::Pairing(PairingEvent::Paired(d)) if d.id == pc_id => Some(()),
        _ => None,
    })
    .await;

    wait_for(&mut pc, "pc online over usb", |e| match e {
        NodeEvent::LinkChanged { device, link: LinkState::Online { path: ConnectionPath::Usb, .. } }
            if *device == phone_id =>
        {
            Some(())
        }
        _ => None,
    })
    .await;
    wait_for(&mut phone, "phone online over usb", |e| match e {
        NodeEvent::LinkChanged { device, link: LinkState::Online { path: ConnectionPath::Usb, .. } }
            if *device == pc_id =>
        {
            Some(())
        }
        _ => None,
    })
    .await;

    // 6. Plugged in, the link stays up: the phone lets the PC dial through the cable instead
    //    of racing it, so neither side sees its connection replaced and dropped.
    let settle = tokio::time::Instant::now() + std::time::Duration::from_secs(8);
    let mut drops = 0;
    while tokio::time::Instant::now() < settle {
        tokio::select! {
            ev = pc.events.recv() => if let Ok(NodeEvent::LinkChanged { device, link: LinkState::Offline { .. } }) = ev
                && device == phone_id { drops += 1 },
            ev = phone.events.recv() => if let Ok(NodeEvent::LinkChanged { device, link: LinkState::Offline { .. } }) = ev
                && device == pc_id { drops += 1 },
            () = tokio::time::sleep_until(settle) => {}
        }
    }
    assert_eq!(drops, 0, "the USB link must not drop while plugged in");
    let pc_paired = pc.node.paired_devices().unwrap();
    assert!(
        matches!(pc_paired[0].link, LinkState::Online { path: ConnectionPath::Usb, .. }),
        "still connected over USB after settling"
    );

    pc.node.shutdown().await;
    phone.node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn usb_transport_carries_features_when_wifi_unavailable() {
    use nectarlink_core::{ConnectionPath, MirrorSend, PacketKind};

    let mut phone = start_usb_device(
        "Pixel USB",
        DeviceKind::Phone,
        Some(0),
        &["clip.write", nectarlink_core::MIRROR_CAPTURE],
    )
    .await;
    let mut pc = start_usb_device(
        "Desktop USB",
        DeviceKind::Desktop,
        None,
        &["clip.write", nectarlink_core::MIRROR_VIEW],
    )
    .await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    let usb_port = phone.node.usb_port().expect("phone USB TCP listener");
    let tunnel_addr: SocketAddr = ("127.0.0.1".parse::<std::net::IpAddr>().unwrap(), usb_port).into();
    pc.node.connect_usb_tunnel(tunnel_addr).await.expect("connect USB tunnel");

    let code_pc = wait_for(&mut pc, "pc sas code", |e| match e {
        NodeEvent::Pairing(PairingEvent::SasCode { peer, code }) if *peer == phone_id => Some(code.clone()),
        _ => None,
    })
    .await;
    let code_phone = wait_for(&mut phone, "phone sas code", |e| match e {
        NodeEvent::Pairing(PairingEvent::SasCode { peer, code }) if *peer == pc_id => Some(code.clone()),
        _ => None,
    })
    .await;
    assert_eq!(code_pc, code_phone);

    pc.node.pairing_confirm(true).unwrap();
    phone.node.pairing_confirm(true).unwrap();

    wait_for(&mut pc, "pc online over usb", |e| match e {
        NodeEvent::LinkChanged { device, link: LinkState::Online { path: ConnectionPath::Usb, .. } }
            if *device == phone_id =>
        {
            Some(())
        }
        _ => None,
    })
    .await;
    wait_for(&mut phone, "phone online over usb", |e| match e {
        NodeEvent::LinkChanged { device, link: LinkState::Online { path: ConnectionPath::Usb, .. } }
            if *device == pc_id =>
        {
            Some(())
        }
        _ => None,
    })
    .await;

    // 4. paired_devices() reports LinkState::Online { path: ConnectionPath::Usb, .. }.
    let pc_paired = pc.node.paired_devices().unwrap();
    assert_eq!(pc_paired.len(), 1);
    assert!(matches!(pc_paired[0].link, LinkState::Online { path: ConnectionPath::Usb, .. }));

    // 1. Clipboard sync over USB tunnel.
    with_timeout("send clipboard over USB", pc.node.send_clipboard(phone_id, "Clipboard over USB!".into()))
        .await
        .unwrap();
    wait_for(&mut phone, "phone clipboard history changed", |e| match e {
        NodeEvent::ClipboardHistoryChanged => Some(()),
        _ => None,
    })
    .await;
    assert_eq!(*phone.platform.clipboard.lock().unwrap(), ["Clipboard over USB!"]);

    // 2. File transfer over USB tunnel.
    let src = tempfile::tempdir().unwrap();
    let payload = b"USB cable file payload 1234567890".repeat(512);
    let transfer_id = with_timeout(
        "send file over USB",
        pc.node.send_files(phone_id, vec![outgoing(src.path(), "usb_file.bin", &payload)]),
    )
    .await
    .unwrap();
    let received_paths = wait_transfer(&mut phone, &transfer_id, "phone received usb_file.bin", saved).await;
    assert_eq!(std::fs::read(&received_paths[0]).unwrap(), payload);
    wait_transfer(&mut pc, &transfer_id, "pc sent usb_file.bin", saved).await;

    // 3. Screen mirroring over USB tunnel.
    let options = nectarlink_core::MirrorStart {
        max_size: 1280,
        fps: 60,
        bitrate: 6_000_000,
        audio: false,
        session: 0,
        app: None,
        stay_awake: false,
        screen_off: false,
        watch: None,
        sound_only: false,
    };
    with_timeout("start mirror over USB", pc.node.mirror_start(phone_id, options)).await.unwrap();
    let stream = Arc::new(with_timeout("open mirror over USB", phone.node.mirror_open(pc_id)).await.unwrap());
    let mirror_cfg =
        nectarlink_core::MirrorConfig { codec: "h264".into(), width: 1080, height: 2400, session: 0 };
    let sender = stream.clone();
    let sent = tokio::task::spawn_blocking(move || {
        [
            sender.send(PacketKind::Config, 0, mirror_cfg.to_cbor()),
            sender.send(PacketKind::Keyframe, 42, vec![7u8; 16_384]),
        ]
    })
    .await
    .unwrap();
    assert!(sent.iter().all(|s| *s == MirrorSend::Queued), "{sent:?}");

    wait_for(&mut pc, "pc mirroring active over USB", |e| {
        matches!(e, NodeEvent::Mirroring { on: true, .. }).then_some(())
    })
    .await;

    let screen = pc.platform.screen.clone();
    let deadline = tokio::time::Instant::now() + WAIT;
    while screen.got.lock().unwrap().len() < 2 && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(*screen.got.lock().unwrap(), ["config 1080x2400", "key 42 16384"]);

    pc.node.mirror_stop(phone_id, 0).await;
    pc.node.shutdown().await;
    phone.node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn reverse_mirror_requires_pc_consent_and_remote_input_toggle() {
    use nectarlink_core::{
        MIRROR_CAPTURE, MIRROR_INPUT, MIRROR_SCREEN, MirrorConfig, MirrorInput, MirrorSend, MirrorStart,
        PC_MIRROR_TOGGLE, PacketKind, TouchAction,
    };
    use std::sync::atomic::Ordering;

    let mut pc = device_with("Desktop", DeviceKind::Desktop, &[MIRROR_CAPTURE, MIRROR_INPUT]).await;
    let mut phone = device_with("Pixel", DeviceKind::Phone, &[nectarlink_core::MIRROR_VIEW]).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    let options = MirrorStart {
        max_size: 1280,
        fps: 30,
        bitrate: 4_000_000,
        audio: false,
        session: MIRROR_SCREEN,
        app: None,
        stay_awake: false,
        screen_off: false,
        watch: None,
        sound_only: false,
    };

    // (a) A paired phone calling `mirror_start` against a PC is denied by default
    // (`pc_mirror` is false by default), and emits `NodeEvent::PcMirrorRequested` on the PC.
    let denied_default = phone.node.mirror_start(pc_id, options.clone()).await;
    assert!(
        matches!(denied_default, Err(Error::Denied)),
        "expected Denied by default, got {denied_default:?}"
    );
    let prompt_opts = wait_for(&mut pc, "pc mirror consent prompt event", |e| match e {
        NodeEvent::PcMirrorRequested { device, options } if *device == phone_id => Some(options.clone()),
        _ => None,
    })
    .await;
    assert_eq!(prompt_opts.max_size, 1280);
    assert!(pc.platform.mirror_asks.lock().unwrap().is_empty());

    // Opening a PC mirror stream from the PC while `pc_mirror` is off is also denied.
    assert!(matches!(pc.node.mirror_open(phone_id).await, Err(Error::Denied)));

    // (b) Even after enabling `pc_mirror` on the PC, if the platform consent hook
    // (`Platform::mirror_requested`) rejects, `mirror_start` is denied.
    pc.node.set_device_toggle(phone_id, PC_MIRROR_TOGGLE, true).unwrap();
    pc.platform.mirror_reject.store(true, Ordering::SeqCst);
    let declined = phone.node.mirror_start(pc_id, options.clone()).await;
    assert!(
        matches!(declined, Err(Error::Denied)),
        "expected Denied when consent hook rejects, got {declined:?}"
    );
    assert!(pc.platform.mirror_asks.lock().unwrap().is_empty());

    // Once both `pc_mirror` is enabled on the PC AND the platform consent hook approves,
    // `mirror_start` succeeds and the PC streams frames to the phone.
    pc.platform.mirror_reject.store(false, Ordering::SeqCst);
    with_timeout("mirror_start after toggle + consent", phone.node.mirror_start(pc_id, options.clone()))
        .await
        .unwrap();
    assert_eq!(*pc.platform.mirror_asks.lock().unwrap(), ["start 1280"]);

    let stream = Arc::new(with_timeout("pc mirror_open", pc.node.mirror_open(phone_id)).await.unwrap());
    let cfg = MirrorConfig { codec: "h264".into(), width: 1280, height: 720, session: MIRROR_SCREEN };
    let sender = stream.clone();
    let sent = tokio::task::spawn_blocking(move || {
        [
            sender.send(PacketKind::Config, 0, cfg.to_cbor()),
            sender.send(PacketKind::Keyframe, 100, vec![9u8; 4096]),
        ]
    })
    .await
    .unwrap();
    assert!(sent.iter().all(|s| *s == MirrorSend::Queued), "{sent:?}");
    wait_for(&mut phone, "phone mirroring on", |e| {
        matches!(e, NodeEvent::Mirroring { on: true, .. }).then_some(())
    })
    .await;

    // (c) `mirror_input` against a PC is refused unless `remote_input` is also on
    // on both sides.
    let touch = MirrorInput::Touch { action: TouchAction::Down, x: 0.5, y: 0.5 };
    let input_denied_both_off = phone.node.mirror_input(pc_id, MIRROR_SCREEN, touch.clone()).await;
    assert!(
        matches!(input_denied_both_off, Err(Error::Denied)),
        "expected Denied when remote_input is off on phone, got {input_denied_both_off:?}"
    );
    phone.node.set_device_toggle(pc_id, "remote_input", true).unwrap();
    phone.node.mirror_input(pc_id, MIRROR_SCREEN, touch.clone()).await.unwrap();
    wait_for(&mut pc, "pc remote input prompt event", |e| match e {
        NodeEvent::RemoteInputRequested { device } if *device == phone_id => Some(()),
        _ => None,
    })
    .await;
    assert_eq!(
        *pc.platform.mirror_asks.lock().unwrap(),
        ["start 1280"],
        "PC must refuse to inject mirror_input while remote_input is off on PC"
    );

    // Once the PC also enables `remote_input`, `mirror_input` succeeds.
    pc.node.set_device_toggle(phone_id, "remote_input", true).unwrap();
    with_timeout("mirror_input with remote_input on", phone.node.mirror_input(pc_id, MIRROR_SCREEN, touch))
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + WAIT;
    while pc.platform.mirror_asks.lock().unwrap().len() < 2 && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(pc.platform.mirror_asks.lock().unwrap().len(), 2);

    phone.node.mirror_stop(pc_id, MIRROR_SCREEN).await;
    pc.node.shutdown().await;
    phone.node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn queue_offline_fifo_clipboard_replacement_and_drain_on_reconnect() {
    use nectarlink_core::{QueueKind, QueueStatus};

    let mut pc = device("Desktop", DeviceKind::Desktop).await;
    let mut phone = device_with("Pixel", DeviceKind::Phone, &["clip.write", "link.open"]).await;
    pair_qr(&mut pc, &mut phone).await;
    let phone_id = phone.node.device_id();

    let phone_dir = phone.dir;
    phone.node.shutdown().await;
    wait_for(&mut pc, "phone offline", |e| match e {
        NodeEvent::LinkChanged { device, link: LinkState::Offline { .. } } if *device == phone_id => Some(()),
        _ => None,
    })
    .await;

    let src = tempfile::tempdir().unwrap();
    let file_id = pc
        .node
        .send_files(phone_id, vec![outgoing(src.path(), "alpha.txt", b"queued-alpha-payload")])
        .await
        .unwrap();
    pc.node.send_clipboard(phone_id, "clip-v1".into()).await.unwrap();
    pc.node.open_link(phone_id, "https://example.com/queued".into()).await.unwrap();
    pc.node.send_clipboard(phone_id, "clip-v2".into()).await.unwrap();

    let queued = pc.node.send_queue(Some(phone_id)).unwrap();
    assert_eq!(queued.len(), 3, "second clipboard send must replace the waiting first clipboard item");
    assert_eq!(queued[0].id, file_id);
    assert_eq!(queued[0].kind, QueueKind::Files);
    assert_eq!(queued[0].status, QueueStatus::Waiting);
    assert_eq!(queued[1].kind, QueueKind::Link);
    assert_eq!(queued[1].status, QueueStatus::Waiting);
    assert_eq!(queued[2].kind, QueueKind::Clipboard);
    assert_eq!(queued[2].title, "clip-v2");
    assert_eq!(queued[2].status, QueueStatus::Waiting);
    assert!(queued[0].seq < queued[1].seq && queued[1].seq < queued[2].seq);

    let mut phone = start_in(phone_dir, "Pixel", DeviceKind::Phone).await;
    phone
        .node
        .update_power(
            PowerLevel::NotApplicable,
            vec!["media.control".into(), "clip.write".into(), "link.open".into()],
        )
        .await;

    let received = wait_transfer(&mut phone, &file_id, "queued file delivered on reconnect", saved).await;
    assert_eq!(std::fs::read(&received[0]).unwrap(), b"queued-alpha-payload");

    let deadline = tokio::time::Instant::now() + WAIT;
    while (phone.platform.clipboard.lock().unwrap().is_empty()
        || phone.platform.links.lock().unwrap().is_empty()
        || !pc.node.send_queue(Some(phone_id)).unwrap().is_empty())
        && tokio::time::Instant::now() < deadline
    {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    assert_eq!(*phone.platform.clipboard.lock().unwrap(), ["clip-v2"]);
    assert_eq!(*phone.platform.links.lock().unwrap(), ["https://example.com/queued"]);
    assert!(pc.node.send_queue(Some(phone_id)).unwrap().is_empty());

    pc.node.shutdown().await;
    phone.node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn queue_survives_node_restart_and_delivers_on_reconnect() {
    use nectarlink_core::QueueStatus;

    let mut pc = device("Desktop", DeviceKind::Desktop).await;
    let mut phone = device_with("Pixel", DeviceKind::Phone, &["clip.write"]).await;
    pair_qr(&mut pc, &mut phone).await;
    let phone_id = phone.node.device_id();

    let phone_dir = phone.dir;
    phone.node.shutdown().await;
    wait_for(&mut pc, "phone offline", |e| match e {
        NodeEvent::LinkChanged { device, link: LinkState::Offline { .. } } if *device == phone_id => Some(()),
        _ => None,
    })
    .await;

    let src = tempfile::tempdir().unwrap();
    let file_id = pc
        .node
        .send_files(phone_id, vec![outgoing(src.path(), "persisted.txt", b"survives-node-restart")])
        .await
        .unwrap();
    pc.node.send_clipboard(phone_id, "persisted-clip".into()).await.unwrap();
    assert_eq!(pc.node.send_queue(Some(phone_id)).unwrap().len(), 2);

    let pc_dir = pc.dir;
    pc.node.shutdown().await;

    let pc = start_in(pc_dir, "Desktop", DeviceKind::Desktop).await;
    let recovered = pc.node.send_queue(Some(phone_id)).unwrap();
    assert_eq!(recovered.len(), 2);
    assert!(recovered.iter().all(|i| i.status == QueueStatus::Waiting));

    let mut phone = start_in(phone_dir, "Pixel", DeviceKind::Phone).await;
    phone
        .node
        .update_power(PowerLevel::NotApplicable, vec!["media.control".into(), "clip.write".into()])
        .await;
    pc.node.add_known_addrs(phone_id, &phone.node.direct_addrs().await);
    phone.node.add_known_addrs(pc.node.device_id(), &pc.node.direct_addrs().await);

    let received = wait_transfer(&mut phone, &file_id, "persisted file delivered", saved).await;
    assert_eq!(std::fs::read(&received[0]).unwrap(), b"survives-node-restart");

    let deadline = tokio::time::Instant::now() + WAIT;
    while (phone.platform.clipboard.lock().unwrap().is_empty()
        || !pc.node.send_queue(Some(phone_id)).unwrap().is_empty())
        && tokio::time::Instant::now() < deadline
    {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(*phone.platform.clipboard.lock().unwrap(), ["persisted-clip"]);
    assert!(pc.node.send_queue(None).unwrap().is_empty());

    pc.node.shutdown().await;
    phone.node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn queue_source_file_deleted_or_modified_fails_cleanly_and_retry_works() {
    use nectarlink_core::QueueStatus;

    let mut pc = device("Desktop", DeviceKind::Desktop).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;
    pair_qr(&mut pc, &mut phone).await;
    let phone_id = phone.node.device_id();

    let phone_dir = phone.dir;
    phone.node.shutdown().await;
    wait_for(&mut pc, "phone offline", |e| match e {
        NodeEvent::LinkChanged { device, link: LinkState::Offline { .. } } if *device == phone_id => Some(()),
        _ => None,
    })
    .await;

    let src = tempfile::tempdir().unwrap();
    let missing_path = src.path().join("missing.txt");
    let modified_path = src.path().join("modified.txt");
    let valid_path = src.path().join("valid.txt");
    std::fs::write(&missing_path, b"will-be-deleted").unwrap();
    std::fs::write(&modified_path, b"original-bytes").unwrap();
    std::fs::write(&valid_path, b"good-bytes").unwrap();

    let missing_id = pc
        .node
        .send_files(
            phone_id,
            vec![OutgoingFile {
                name: "missing.txt".into(),
                folder: None,
                source: FileSource::Path(missing_path.clone()),
            }],
        )
        .await
        .unwrap();
    let modified_id = pc
        .node
        .send_files(
            phone_id,
            vec![OutgoingFile {
                name: "modified.txt".into(),
                folder: None,
                source: FileSource::Path(modified_path.clone()),
            }],
        )
        .await
        .unwrap();
    let valid_id = pc
        .node
        .send_files(
            phone_id,
            vec![OutgoingFile {
                name: "valid.txt".into(),
                folder: None,
                source: FileSource::Path(valid_path),
            }],
        )
        .await
        .unwrap();

    // Mutate/remove the first two source files while queued.
    std::fs::remove_file(&missing_path).unwrap();
    std::fs::write(&modified_path, b"modified-after-queuing-different-length").unwrap();

    let mut phone = start_in(phone_dir, "Pixel", DeviceKind::Phone).await;
    let received =
        wait_transfer(&mut phone, &valid_id, "valid file still delivered past failed items", saved).await;
    assert_eq!(std::fs::read(&received[0]).unwrap(), b"good-bytes");

    let deadline = tokio::time::Instant::now() + WAIT;
    while pc.node.send_queue(Some(phone_id)).unwrap().len() != 2 && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let queue = pc.node.send_queue(Some(phone_id)).unwrap();
    assert_eq!(queue.len(), 2);
    let missing_item = queue.iter().find(|i| i.id == missing_id).unwrap();
    assert!(matches!(
        &missing_item.status,
        QueueStatus::Failed { error } if error.contains("File was deleted before sending: missing.txt")
    ));
    let modified_item = queue.iter().find(|i| i.id == modified_id).unwrap();
    assert!(matches!(
        &modified_item.status,
        QueueStatus::Failed { error } if error.contains("File was modified before sending: modified.txt")
    ));

    // Clearing finished items removes both failed entries.
    assert_eq!(pc.node.clear_finished_queue_items(Some(phone_id)).unwrap(), 2);
    assert!(pc.node.send_queue(Some(phone_id)).unwrap().is_empty());

    // Also verify retry_queue_item when a toggle caused a failure while queued:
    let phone_dir2 = phone.dir;
    phone.node.shutdown().await;
    wait_for(&mut pc, "phone offline 2", |e| match e {
        NodeEvent::LinkChanged { device, link: LinkState::Offline { .. } } if *device == phone_id => Some(()),
        _ => None,
    })
    .await;

    let retry_id = pc
        .node
        .send_files(phone_id, vec![outgoing(src.path(), "retry.txt", b"retry-success")])
        .await
        .unwrap();
    pc.node.set_device_toggle(phone_id, "files", false).unwrap();

    let mut phone = start_in(phone_dir2, "Pixel", DeviceKind::Phone).await;
    wait_online(&mut pc, phone_id).await;

    let deadline = tokio::time::Instant::now() + WAIT;
    while !pc
        .node
        .send_queue(Some(phone_id))
        .unwrap()
        .iter()
        .any(|i| i.id == retry_id && matches!(i.status, QueueStatus::Failed { .. }))
        && tokio::time::Instant::now() < deadline
    {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    pc.node.set_device_toggle(phone_id, "files", true).unwrap();
    assert!(pc.node.retry_queue_item(&retry_id).unwrap());

    let retried_paths = wait_transfer(&mut phone, &retry_id, "retried file delivered", saved).await;
    assert_eq!(std::fs::read(&retried_paths[0]).unwrap(), b"retry-success");

    let deadline = tokio::time::Instant::now() + WAIT;
    while !pc.node.send_queue(Some(phone_id)).unwrap().is_empty() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(pc.node.send_queue(Some(phone_id)).unwrap().is_empty());

    pc.node.shutdown().await;
    phone.node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn queue_cancel_waiting_and_mid_transfer_cleans_up_and_unpair_drops_queue() {
    let mut pc = device("Desktop", DeviceKind::Desktop).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;
    pair_qr(&mut pc, &mut phone).await;
    let phone_id = phone.node.device_id();

    phone.node.shutdown().await;
    wait_for(&mut pc, "phone offline", |e| match e {
        NodeEvent::LinkChanged { device, link: LinkState::Offline { .. } } if *device == phone_id => Some(()),
        _ => None,
    })
    .await;

    let src = tempfile::tempdir().unwrap();
    let fd_src_path = src.path().join("fd_source.bin");
    std::fs::write(&fd_src_path, b"staged-descriptor-bytes").unwrap();
    let fd_handle = std::fs::File::open(&fd_src_path).unwrap();

    let staged_id = pc
        .node
        .send_files(
            phone_id,
            vec![OutgoingFile {
                name: "staged.bin".into(),
                folder: None,
                source: FileSource::File(fd_handle),
            }],
        )
        .await
        .unwrap();

    let staging_dir = pc.dir.path().join("queue_staging").join(&staged_id);
    assert!(staging_dir.exists(), "FileSource::File must be staged in queue_staging/<id>");

    assert!(pc.node.cancel_queue_item(&staged_id).unwrap());
    assert!(pc.node.send_queue(Some(phone_id)).unwrap().is_empty());
    assert!(!staging_dir.exists(), "cancelling a queued item must delete its staged files");

    // Enqueue 2 items (one staged file + one clipboard clip) and unpair: both are dropped and
    // QueueDroppedOnUnpair is emitted with dropped_count == 2.
    let fd_handle2 = std::fs::File::open(&fd_src_path).unwrap();
    let staged_id2 = pc
        .node
        .send_files(
            phone_id,
            vec![OutgoingFile {
                name: "staged2.bin".into(),
                folder: None,
                source: FileSource::File(fd_handle2),
            }],
        )
        .await
        .unwrap();
    pc.node.send_clipboard(phone_id, "dropped-on-unpair".into()).await.unwrap();
    let staging_dir2 = pc.dir.path().join("queue_staging").join(&staged_id2);
    assert!(staging_dir2.exists());
    assert_eq!(pc.node.send_queue(Some(phone_id)).unwrap().len(), 2);

    pc.node.unpair(phone_id).await.unwrap();
    let dropped_count = wait_for(&mut pc, "QueueDroppedOnUnpair event", |e| match e {
        NodeEvent::QueueDroppedOnUnpair { peer, dropped_count, .. } if *peer == phone_id => {
            Some(*dropped_count)
        }
        _ => None,
    })
    .await;
    assert_eq!(dropped_count, 2);
    assert!(pc.node.send_queue(None).unwrap().is_empty());
    assert!(!staging_dir2.exists(), "unpairing must clean up staged files for dropped queue items");

    pc.node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn queue_offline_no_busy_loop_timeline_only_on_delivery_and_clear_history_preserves_queue() {
    use nectarlink_core::{TimelineKind, TimelineQuery};

    let mut pc = device("Desktop", DeviceKind::Desktop).await;
    let mut phone = device_with("Pixel", DeviceKind::Phone, &["clip.write", "link.open"]).await;
    pair_qr(&mut pc, &mut phone).await;
    let phone_id = phone.node.device_id();

    let phone_dir = phone.dir;
    phone.node.shutdown().await;
    wait_for(&mut pc, "phone offline", |e| match e {
        NodeEvent::LinkChanged { device, link: LinkState::Offline { .. } } if *device == phone_id => Some(()),
        _ => None,
    })
    .await;

    // Enqueue a staged file, a clipboard item, and a link while phone is offline.
    let src = tempfile::tempdir().unwrap();
    let fd_src_path = src.path().join("staged_deliver.txt");
    std::fs::write(&fd_src_path, b"staged-for-delivery").unwrap();
    let fd_handle = std::fs::File::open(&fd_src_path).unwrap();
    let file_id = pc
        .node
        .send_files(
            phone_id,
            vec![OutgoingFile {
                name: "staged_deliver.txt".into(),
                folder: None,
                source: FileSource::File(fd_handle),
            }],
        )
        .await
        .unwrap();
    pc.node.send_clipboard(phone_id, "offline-clip-text".into()).await.unwrap();
    pc.node.open_link(phone_id, "https://example.com/offline-link".into()).await.unwrap();

    let staging_dir = pc.dir.path().join("queue_staging").join(&file_id);
    assert!(staging_dir.exists());

    // 1. Timeline must NOT record any Sent/Opened entries while items are still waiting in the queue.
    let tl_before = pc
        .node
        .timeline_page(&TimelineQuery { device: Some(phone_id), limit: 50, ..Default::default() })
        .unwrap();
    assert_eq!(
        tl_before.total, 0,
        "timeline must not record queued items before they are actually delivered"
    );

    // 2. Setting a past next_retry_ms on a waiting item for an offline peer must NOT cause a 50ms busy loop.
    assert!(pc.node.debug_set_queue_retry_ms(&file_id, 1, 1_000).unwrap());
    tokio::time::sleep(Duration::from_millis(60)).await;
    let ticks_start = pc.node.queue_coordinator_ticks();
    tokio::time::sleep(Duration::from_millis(450)).await;
    let ticks_elapsed = pc.node.queue_coordinator_ticks().saturating_sub(ticks_start);
    assert!(
        ticks_elapsed <= 1,
        "queue coordinator must stay idle while peer is offline, got {ticks_elapsed} ticks in 450ms"
    );

    // 3. Clearing local history/cache must preserve send_queue and staged files.
    pc.node.clear_all_local_data().unwrap();
    assert_eq!(pc.node.send_queue(Some(phone_id)).unwrap().len(), 3);
    assert!(staging_dir.exists(), "clear_all_local_data must not delete queue_staging");

    // 4. Bring phone online: all 3 items are delivered, staged files are deleted, and timeline records all 3.
    let mut phone = start_in(phone_dir, "Pixel", DeviceKind::Phone).await;
    phone
        .node
        .update_power(
            PowerLevel::NotApplicable,
            vec!["media.control".into(), "clip.write".into(), "link.open".into()],
        )
        .await;

    let received = wait_transfer(&mut phone, &file_id, "staged file delivered", saved).await;
    assert_eq!(std::fs::read(&received[0]).unwrap(), b"staged-for-delivery");

    let deadline = tokio::time::Instant::now() + WAIT;
    while (phone.platform.clipboard.lock().unwrap().is_empty()
        || phone.platform.links.lock().unwrap().is_empty()
        || !pc.node.send_queue(Some(phone_id)).unwrap().is_empty())
        && tokio::time::Instant::now() < deadline
    {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    assert!(!staging_dir.exists(), "staged directory must be deleted once file delivery completes");
    let tl_after = pc
        .node
        .timeline_page(&TimelineQuery { device: Some(phone_id), limit: 50, ..Default::default() })
        .unwrap();
    assert_eq!(tl_after.total, 3, "timeline must record File, Clip, and Link once delivered");
    assert!(tl_after.entries.iter().any(|e| e.kind == TimelineKind::File && !e.incoming));
    assert!(tl_after.entries.iter().any(|e| e.kind == TimelineKind::Clip && !e.incoming));
    assert!(tl_after.entries.iter().any(|e| e.kind == TimelineKind::Link && !e.incoming));

    pc.node.shutdown().await;
    phone.node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn pc_audio_reaches_the_phone_over_both_paths() {
    use nectarlink_core::{DECK_ACTIONS, PC_AUDIO};

    let mut pc = device_with("Desktop", DeviceKind::Desktop, &[DECK_ACTIONS, PC_AUDIO]).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;
    pair_qr(&mut pc, &mut phone).await;
    let phone_id = phone.node.device_id();

    // "Phone speaker" can be turned on from either device, so the phone plays whatever its
    // paired PC streams: small frames go as datagrams, larger ones on a stream.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut heard = 0;
    while heard == 0 && tokio::time::Instant::now() < deadline {
        let _ = pc.node.send_pc_audio_pcm(&phone_id, 48_000, 2, vec![1u8; 480]).await;
        let _ = pc.node.send_pc_audio_pcm(&phone_id, 48_000, 2, vec![2u8; 4000]).await;
        while let Ok(Ok(ev)) = tokio::time::timeout(Duration::from_millis(40), phone.events.recv()).await {
            if matches!(ev, NodeEvent::PcAudioFrame { .. }) {
                heard += 1;
            }
        }
    }
    assert!(heard > 0, "the phone never surfaced the PC's audio");

    pc.node.shutdown().await;
    phone.node.shutdown().await;
}

/// Play on phone for a video file: the phone reads only the file the PC
/// offered, in ranges, while the offer is open.
#[tokio::test(flavor = "multi_thread")]
async fn play_on_phone_offers_a_video_file_the_phone_reads_in_ranges() {
    use nectarlink_core::{MEDIA_STREAM, MEDIA_WATCH, MEDIA_WATCH_MAX_READ};

    let mut pc = device_with("Desktop", DeviceKind::Desktop, &[MEDIA_STREAM]).await;
    let mut phone =
        device_with("Pixel", DeviceKind::Phone, &[MEDIA_WATCH, nectarlink_core::MIRROR_VIEW]).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    // Nothing offered yet: nothing to read.
    let early = phone.node.watch_file_read(pc_id, 0, 1024).await;
    assert!(matches!(early, Err(Error::Denied)), "{early:?}");

    let dir = tempfile::tempdir().unwrap();
    let movie = dir.path().join("Holiday.mp4");
    let bytes: Vec<u8> = (0..(MEDIA_WATCH_MAX_READ as usize + 300_000)).map(|i| (i % 251) as u8).collect();
    std::fs::write(&movie, &bytes).unwrap();
    with_timeout(
        "offer the file",
        pc.node.offer_watch_file(phone_id, "vlc.exe".into(), Some("Holiday".into()), movie.clone(), 61_500),
    )
    .await
    .unwrap();
    let offered = format!("vlc.exe: Holiday [Holiday.mp4 {} at 61500]", bytes.len());
    assert_eq!(*phone.platform.watch_offers.lock().unwrap(), [offered]);

    // Ranges come back as they are on the PC, capped at one read's worth.
    let (size, head) =
        with_timeout("read the start", phone.node.watch_file_read(pc_id, 0, 1000)).await.unwrap();
    assert_eq!((size, head.as_slice()), (bytes.len() as u64, &bytes[..1000]));
    let (_, big) = with_timeout("read a lot", phone.node.watch_file_read(pc_id, 10, u64::MAX)).await.unwrap();
    assert_eq!(big.len() as u64, MEDIA_WATCH_MAX_READ);
    assert_eq!(big.as_slice(), &bytes[10..10 + MEDIA_WATCH_MAX_READ as usize]);
    let tail_at = bytes.len() as u64 - 100;
    let (_, tail) =
        with_timeout("read the end", phone.node.watch_file_read(pc_id, tail_at, 4096)).await.unwrap();
    assert_eq!(tail.as_slice(), &bytes[tail_at as usize..]);
    let (_, past) =
        with_timeout("read past it", phone.node.watch_file_read(pc_id, bytes.len() as u64 + 5, 10))
            .await
            .unwrap();
    assert!(past.is_empty());

    // A streamed offer replaces it: the file can't be read any more.
    with_timeout("offer a stream", pc.node.offer_watch(phone_id, "Chrome".into(), None)).await.unwrap();
    let after = phone.node.watch_file_read(pc_id, 0, 10).await;
    assert!(matches!(after, Err(Error::Denied)), "{after:?}");

    pc.node.shutdown().await;
    phone.node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn play_on_phone_lets_the_phone_watch_only_what_the_pc_offered() {
    use nectarlink_core::{MEDIA_STREAM, MEDIA_WATCH, MIRROR_SCREEN, MirrorStart};

    let mut pc = device_with("Desktop", DeviceKind::Desktop, &[MEDIA_STREAM]).await;
    let mut phone =
        device_with("Pixel", DeviceKind::Phone, &[MEDIA_WATCH, nectarlink_core::MIRROR_VIEW]).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());
    let watch = |player: &str| MirrorStart {
        max_size: 1280,
        fps: 30,
        bitrate: 4_000_000,
        audio: true,
        session: MIRROR_SCREEN,
        app: None,
        stay_awake: false,
        screen_off: false,
        watch: Some(player.into()),
        sound_only: false,
    };

    // Not offered: the phone can't watch, and the PC isn't asked to share its screen.
    let early = phone.node.mirror_start(pc_id, watch("Chrome")).await;
    assert!(matches!(early, Err(Error::Denied)), "{early:?}");
    assert!(pc.platform.mirror_asks.lock().unwrap().is_empty());

    with_timeout("offer", pc.node.offer_watch(phone_id, "Chrome".into(), Some("A video".into())))
        .await
        .unwrap();
    assert_eq!(*phone.platform.watch_offers.lock().unwrap(), ["Chrome: A video"]);

    // Another player than the one offered: no.
    let other = phone.node.mirror_start(pc_id, watch("Spotify")).await;
    assert!(matches!(other, Err(Error::Denied)), "{other:?}");

    // The one offered: the PC streams it, with its sound.
    with_timeout("watch", phone.node.mirror_start(pc_id, watch("Chrome"))).await.unwrap();
    assert_eq!(*pc.platform.mirror_asks.lock().unwrap(), ["start 1280 with sound watching Chrome"]);
    // The PC may open the stream to it, though screen sharing is off for the phone.
    let stream = with_timeout("open", pc.node.mirror_open(phone_id)).await.unwrap();
    drop(stream);

    // Stopped and started again (the phone rotated): still allowed.
    with_timeout("stop", phone.node.mirror_stop(pc_id, MIRROR_SCREEN)).await;
    with_timeout("watch again", phone.node.mirror_start(pc_id, watch("Chrome"))).await.unwrap();

    // The screen itself stays off limits.
    let screen = MirrorStart { watch: None, audio: false, ..watch("Chrome") };
    let denied = phone.node.mirror_start(pc_id, screen).await;
    assert!(matches!(denied, Err(Error::Denied)), "{denied:?}");

    // With media off for the phone, the PC can't offer.
    pc.node.set_device_toggle(phone_id, "media", false).unwrap();
    let off = pc.node.offer_watch(phone_id, "Chrome".into(), None).await;
    assert!(matches!(off, Err(Error::Denied)), "{off:?}");

    pc.node.shutdown().await;
    phone.node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn play_on_phone_needs_a_phone_that_can_watch() {
    let mut pc = device_with("Desktop", DeviceKind::Desktop, &[nectarlink_core::MEDIA_STREAM]).await;
    let mut phone = device_with("Pixel", DeviceKind::Phone, &[]).await;
    pair_qr(&mut pc, &mut phone).await;
    let offer = pc.node.offer_watch(phone.node.device_id(), "Chrome".into(), None).await;
    assert!(matches!(offer, Err(Error::Unsupported)), "{offer:?}");
    assert!(phone.platform.watch_offers.lock().unwrap().is_empty());
    pc.node.shutdown().await;
    phone.node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_phone_streams_its_microphone_when_its_user_agrees() {
    use nectarlink_core::{MIC_LISTEN, MIC_STREAM, MirrorAudioConfig, MirrorSend, PacketKind};

    let mut pc = device_with("Desktop", DeviceKind::Desktop, &[MIC_LISTEN]).await;
    let mut phone = device_with("Pixel", DeviceKind::Phone, &[MIC_STREAM]).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    // The PC asks; the phone asks its user (the platform), nothing streams yet.
    with_timeout("mic_start", pc.node.mic_start(phone_id)).await.unwrap();
    assert_eq!(*phone.platform.mic_asks.lock().unwrap(), ["start"]);

    // The user agreed: the phone streams 48 kHz mono.
    let stream = Arc::new(with_timeout("mic_open", phone.node.mic_open(pc_id)).await.unwrap());
    let config = MirrorAudioConfig { codec: nectarlink_core::MIRROR_PCM.into(), rate: 48_000, channels: 1 };
    let sender = stream.clone();
    let sent = tokio::task::spawn_blocking(move || {
        [
            sender.send(PacketKind::Config, 0, config.to_cbor()),
            sender.send(PacketKind::Frame, 10_000, vec![0u8; 960]),
            sender.send(PacketKind::Frame, 20_000, vec![1u8; 960]),
        ]
    })
    .await
    .unwrap();
    assert!(sent.iter().all(|s| *s == MirrorSend::Queued), "{sent:?}");
    wait_for(&mut pc, "mic on", |e| matches!(e, NodeEvent::Mic { on: true, .. }).then_some(())).await;
    let deadline = std::time::Instant::now() + WAIT;
    while pc.platform.mic.got.lock().unwrap().len() < 3 {
        assert!(std::time::Instant::now() < deadline, "{:?}", pc.platform.mic.got.lock().unwrap());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(pc.platform.mic.got.lock().unwrap()[..3], ["config 48000 Hz x1", "audio 960", "audio 960"]);

    // The PC stops listening: the phone is told to stop recording.
    pc.node.mic_stop(phone_id).await;
    wait_for(&mut pc, "mic off", |e| matches!(e, NodeEvent::Mic { on: false, .. }).then_some(())).await;
    let deadline = std::time::Instant::now() + WAIT;
    while !phone.platform.mic_asks.lock().unwrap().contains(&"stop".to_owned()) {
        assert!(std::time::Instant::now() < deadline, "the phone wasn't told to stop");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // With the microphone off for the PC on the phone, the PC's ask is refused.
    phone.node.set_device_toggle(pc_id, "mic", false).unwrap();
    let denied = pc.node.mic_start(phone_id).await;
    assert!(matches!(denied, Err(Error::Denied)), "{denied:?}");
    assert_eq!(phone.platform.mic_asks.lock().unwrap().iter().filter(|a| *a == "start").count(), 1);

    pc.node.shutdown().await;
    phone.node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_phone_can_start_its_microphone_for_a_pc_itself() {
    use nectarlink_core::{MIC_LISTEN, MIC_STREAM, MirrorAudioConfig, MirrorSend, PacketKind};

    let mut pc = device_with("Desktop", DeviceKind::Desktop, &[MIC_LISTEN]).await;
    let mut phone = device_with("Pixel", DeviceKind::Phone, &[MIC_STREAM]).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    // Started on the phone, as the webcam can be: the PC never asked.
    let stream = Arc::new(with_timeout("mic_open", phone.node.mic_open(pc_id)).await.unwrap());
    let config = MirrorAudioConfig { codec: nectarlink_core::MIRROR_PCM.into(), rate: 48_000, channels: 1 };
    let sender = stream.clone();
    let sent = tokio::task::spawn_blocking(move || sender.send(PacketKind::Config, 0, config.to_cbor()))
        .await
        .unwrap();
    assert_eq!(sent, MirrorSend::Queued);
    let device = wait_for(&mut pc, "mic on", |e| match e {
        NodeEvent::Mic { device, on: true } => Some(*device),
        _ => None,
    })
    .await;
    assert_eq!(device, phone_id);
    assert!(phone.platform.mic_asks.lock().unwrap().is_empty(), "nobody was asked");

    pc.node.shutdown().await;
    phone.node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pc_hears_when_the_phone_declines_its_microphone_request() {
    use nectarlink_core::{MIC_LISTEN, MIC_STREAM};

    let mut pc = device_with("Desktop", DeviceKind::Desktop, &[MIC_LISTEN]).await;
    let mut phone = device_with("Pixel", DeviceKind::Phone, &[MIC_STREAM]).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());

    with_timeout("mic_start", pc.node.mic_start(phone_id)).await.unwrap();
    // The user taps Not now: no sound ever came, and the PC stops waiting.
    phone.node.mic_stop(pc_id).await;
    let device = wait_for(&mut pc, "mic off", |e| match e {
        NodeEvent::Mic { device, on: false } => Some(*device),
        _ => None,
    })
    .await;
    assert_eq!(device, phone_id);

    pc.node.shutdown().await;
    phone.node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pc_without_mic_listen_refuses_a_phones_microphone() {
    use nectarlink_core::{MIC_STREAM, MirrorAudioConfig, PacketKind};

    let mut pc = device_with("Desktop", DeviceKind::Desktop, &[]).await;
    let mut phone = device_with("Pixel", DeviceKind::Phone, &[MIC_STREAM]).await;
    pair_qr(&mut pc, &mut phone).await;
    let stream = Arc::new(with_timeout("mic_open", phone.node.mic_open(pc.node.device_id())).await.unwrap());
    let config = MirrorAudioConfig { codec: nectarlink_core::MIRROR_PCM.into(), rate: 48_000, channels: 1 };
    let sender = stream.clone();
    tokio::task::spawn_blocking(move || sender.send(PacketKind::Config, 0, config.to_cbor())).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(pc.platform.mic.got.lock().unwrap().is_empty());
    pc.node.shutdown().await;
    phone.node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_phones_notifications_show_on_a_paired_tablet_that_shows_them() {
    let mut tablet = device_with("Tab", DeviceKind::Tablet, &[nectarlink_core::NOTIFY_SHOW]).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;
    pair_qr(&mut tablet, &mut phone).await;
    let phone_id = phone.node.device_id();
    grant_notification_access(&phone).await;

    phone.node.notifications_reset(vec![note("a", "com.chat", "Sam", true)]).await;
    let items = wait_for(&mut tablet, "snapshot", |e| match e {
        NodeEvent::NotificationsReset { device, items } if *device == phone_id => Some(items.clone()),
        _ => None,
    })
    .await;
    assert_eq!(items.len(), 1);

    phone.node.notification_posted(note("b", "com.chat", "Kim", true)).await;
    let posted = wait_for(&mut tablet, "posted", |e| match e {
        NodeEvent::NotificationPosted { device, notification } if *device == phone_id => {
            Some(notification.key.clone())
        }
        _ => None,
    })
    .await;
    assert_eq!(posted, "b");

    // Dismissed on the tablet: gone on the phone too.
    with_timeout("dismiss", tablet.node.dismiss_notification(phone_id, "b".into())).await.unwrap();

    tablet.node.shutdown().await;
    phone.node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_phone_keeps_its_notifications_from_a_phone_that_doesnt_show_them() {
    let mut other = device("Other phone", DeviceKind::Phone).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;
    pair_qr(&mut other, &mut phone).await;
    grant_notification_access(&phone).await;
    phone.node.notification_posted(note("a", "com.chat", "Sam", true)).await;
    let got = tokio::time::timeout(Duration::from_millis(800), async {
        loop {
            match other.events.recv().await {
                Ok(NodeEvent::NotificationPosted { .. } | NodeEvent::NotificationsReset { .. }) => {
                    return true;
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => return false,
            }
        }
    })
    .await;
    assert!(!matches!(got, Ok(true)), "notifications went to a phone that doesn't show them");
    other.node.shutdown().await;
    phone.node.shutdown().await;
}

#[tokio::test]
async fn usb_upgrade_reconnect_and_realtime_pcm_do_not_starve_control_rpcs() {
    use nectarlink_core::ConnectionPath;

    let mut phone = start_usb_device("Pixel", DeviceKind::Phone, Some(0), &["find.ring"]).await;
    let mut pc = start_usb_device("Desktop", DeviceKind::Desktop, None, &["find.ring"]).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());
    assert!(matches!(
        pc.node.paired_devices().unwrap()[0].link,
        LinkState::Online { path: ConnectionPath::Lan, .. }
    ));

    // Attach USB tunnel while a LAN session is already live: both sides upgrade to ConnectionPath::Usb.
    let usb_port = phone.node.usb_port().expect("phone USB TCP listener");
    let tunnel_addr: SocketAddr = ("127.0.0.1".parse::<std::net::IpAddr>().unwrap(), usb_port).into();
    pc.node.connect_usb_tunnel(tunnel_addr).await.expect("connect USB tunnel");

    wait_for(&mut pc, "pc upgrades to usb", |e| match e {
        NodeEvent::LinkChanged { device, link: LinkState::Online { path: ConnectionPath::Usb, .. } }
            if *device == phone_id =>
        {
            Some(())
        }
        _ => None,
    })
    .await;
    wait_for(&mut phone, "phone upgrades to usb", |e| match e {
        NodeEvent::LinkChanged { device, link: LinkState::Online { path: ConnectionPath::Usb, .. } }
            if *device == pc_id =>
        {
            Some(())
        }
        _ => None,
    })
    .await;

    // Enqueue a burst of 512 >960-byte PCM frames (2x OUTBOX_CAPACITY, going through
    // try_send_realtime) before the writer task runs and verify that excess frames past
    // half outbox capacity are dropped while control RPCs (ring) still complete promptly.
    const BURST_FRAMES: usize = 512;
    let payload = vec![0u8; 1_000];
    for _ in 0..BURST_FRAMES {
        pc.node
            .send_pc_audio_pcm(&phone_id, 48_000, 2, payload.clone())
            .await
            .expect("realtime PCM enqueue returns Ok even when dropping frames");
    }
    with_timeout("ring during realtime pcm burst", pc.node.ring(phone_id, true))
        .await
        .expect("control RPC succeeds during realtime burst");
    assert_eq!(*phone.platform.rings.lock().unwrap(), [true]);

    let mut received_pcm = 0usize;
    while let Ok(ev) = phone.events.try_recv() {
        if matches!(ev, NodeEvent::PcAudioFrame { .. }) {
            received_pcm += 1;
        }
    }
    assert!(
        (1..=128).contains(&received_pcm),
        "expected excess realtime frames to be dropped past half outbox capacity (128), got {received_pcm}"
    );

    // Shut down the phone (which closes the USB TCP tunnel) and restart it over LAN.
    phone.node.shutdown().await;
    wait_for(&mut pc, "phone offline before restart", |e| match e {
        NodeEvent::LinkChanged { device, link: LinkState::Offline { .. } } if *device == phone_id => Some(()),
        _ => None,
    })
    .await;

    let TestDevice { node: old_phone, dir, .. } = phone;
    drop(old_phone);
    let mut phone = start_in(dir, "Pixel", DeviceKind::Phone).await;
    wait_online(&mut phone, pc_id).await;
    wait_online(&mut pc, phone_id).await;
    with_timeout("ring after restart", pc.node.ring(phone_id, false))
        .await
        .expect("ring works after restart");

    pc.node.shutdown().await;
    phone.node.shutdown().await;
}

/// Throughput of one large file between two devices in this process, over
/// loopback. Not a check: run it to measure (`--ignored --nocapture`, in
/// release), with NECTARLINK_BENCH_MB for the size (default 1024).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "benchmark"]
async fn bench_file_throughput() {
    use std::io::Write;
    let mb: usize = std::env::var("NECTARLINK_BENCH_MB").ok().and_then(|v| v.parse().ok()).unwrap_or(1024);
    let mut pc = device("Desktop", DeviceKind::Desktop).await;
    let mut phone = device("Pixel", DeviceKind::Phone).await;
    pair_qr(&mut pc, &mut phone).await;
    let phone_id = phone.node.device_id();
    let src = tempfile::tempdir().unwrap();
    let path = src.path().join("bench.bin");
    {
        let block = data(8 * 1024 * 1024, 7);
        let mut f = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
        for _ in 0..mb / 8 {
            f.write_all(&block).unwrap();
        }
    }
    let files = vec![OutgoingFile { name: "bench.bin".into(), folder: None, source: FileSource::Path(path) }];
    let started = std::time::Instant::now();
    let id = pc.node.send_files(phone_id, files).await.expect("send starts");
    let first_byte =
        wait_transfer(&mut phone, &id, "first bytes", |t| (t.done > 0).then(|| started.elapsed())).await;
    wait_transfer(&mut phone, &id, "received", saved).await;
    let total = started.elapsed();
    let moving = total - first_byte;
    println!(
        "bench: {mb} MB in {:.2} s = {:.0} MB/s overall; first byte after {:.2} s, then {:.0} MB/s",
        total.as_secs_f64(),
        mb as f64 / total.as_secs_f64(),
        first_byte.as_secs_f64(),
        mb as f64 / moving.as_secs_f64()
    );
    pc.node.shutdown().await;
    phone.node.shutdown().await;
}

/// A phone whose app reports `install_id` (the same on this phone after a reinstall).
async fn phone_with_install_id(name: &'static str, install_id: &str) -> TestDevice {
    init_logging();
    let dir = tempfile::tempdir().unwrap();
    let mut phone_info = info(name, DeviceKind::Phone);
    phone_info.install_id = Some(install_id.into());
    let mut config = NodeConfig::new(dir.path(), phone_info, "0.0.1-test");
    config.lan_discovery = false;
    config.away_mode = false;
    config.key_protector = Some(Arc::new(PlainKeyProtector));
    config.capabilities = vec!["media.control".into()];
    let platform = Arc::new(RecordingPlatform::default());
    let node = Node::start(config, platform.clone()).await.expect("node starts");
    let events = node.events();
    TestDevice { node, events, platform, dir, name }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reinstalled_phone_replaces_its_old_pairing() {
    const INSTALL: &str = "0123456789abcdef0123456789abcdef";
    let mut pc = device("pc", DeviceKind::Desktop).await;
    let mut old = phone_with_install_id("phone", INSTALL).await;
    pair_qr(&mut pc, &mut old).await;
    let old_id = old.node.device_id();
    old.node.shutdown().await;
    wait_for(&mut pc, "old phone offline", |e| match e {
        NodeEvent::LinkChanged { device, link: LinkState::Offline { .. } } if *device == old_id => Some(()),
        _ => None,
    })
    .await;

    // Reinstalled: a new identity, the same install ID. Pairing it replaces the old entry.
    let mut new = phone_with_install_id("phone", INSTALL).await;
    let mut pc_events = pc.node.events();
    pair_qr(&mut pc, &mut new).await;
    let new_id = new.node.device_id();
    let mut removed = false;
    while let Ok(event) = pc_events.try_recv() {
        removed |= matches!(event, NodeEvent::DeviceRemoved(device) if device == old_id);
    }
    assert!(removed, "the old entry is announced as removed");
    let ids: Vec<nectarlink_core::DeviceId> =
        pc.node.paired_devices().unwrap().into_iter().map(|d| d.id).collect();
    assert_eq!(ids, vec![new_id]);

    pc.node.shutdown().await;
    new.node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn one_time_screen_consent_ends_with_the_pairing() {
    use nectarlink_core::{MIRROR_CAPTURE, MIRROR_INPUT, MIRROR_SCREEN, MirrorStart};

    let mut pc = device_with("Desktop", DeviceKind::Desktop, &[MIRROR_CAPTURE, MIRROR_INPUT]).await;
    let mut phone = device_with("Pixel", DeviceKind::Phone, &[nectarlink_core::MIRROR_VIEW]).await;
    pair_qr(&mut pc, &mut phone).await;
    let (pc_id, phone_id) = (pc.node.device_id(), phone.node.device_id());
    let options = MirrorStart {
        max_size: 1280,
        fps: 30,
        bitrate: 4_000_000,
        audio: false,
        session: MIRROR_SCREEN,
        app: None,
        stay_awake: false,
        screen_off: false,
        watch: None,
        sound_only: false,
    };

    // "Allow once" lets the phone view the PC's screen during this connection
    // (granted here before the phone starts, so nothing else clears it).
    pc.node.set_pc_mirror_session_allowed(phone_id, true);

    // After unpairing and pairing again, the phone has to be allowed again.
    with_timeout("unpair", pc.node.unpair(phone_id)).await.unwrap();
    wait_for(&mut phone, "phone forgets the PC", |e| match e {
        NodeEvent::DeviceRemoved(device) if *device == pc_id => Some(()),
        _ => None,
    })
    .await;
    pair_qr(&mut pc, &mut phone).await;
    let again = phone.node.mirror_start(pc_id, options).await;
    assert!(matches!(again, Err(Error::Denied)), "expected Denied after re-pairing, got {again:?}");

    pc.node.shutdown().await;
    phone.node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn notification_history_keeps_the_newest_thousand() {
    let phone = device("phone", DeviceKind::Phone).await;
    for i in 0..1005i64 {
        phone
            .node
            .notification_posted(Notification {
                key: format!("app|{i:04}"),
                app: "org.example.chat".into(),
                app_name: "Chat".into(),
                title: Some(format!("Message {i}")),
                text: None,
                sub: None,
                when: 1_760_000_000_000 + i * 1000,
                silent: false,
                actions: Vec::new(),
                icon: None,
                image: None,
                live: None,
                conversation: None,
            })
            .await;
    }
    let history = phone.node.notification_history(None, 5000);
    assert_eq!(history.len(), 1000);
    assert!(history.iter().all(|n| n.key.as_str() >= "app|0005"), "the oldest five are gone");
    phone.node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn elevated_over_usb_is_refused_without_a_usb_connection() {
    let mut pc = device("pc", DeviceKind::Desktop).await;
    let mut phone = device("phone", DeviceKind::Phone).await;
    pair_qr(&mut pc, &mut phone).await;
    let pc_id = pc.node.device_id();
    let apk = "/data/app/~~a==/app.nectarlink-b==/base.apk".to_owned();
    let token = "0123456789abcdef0123456789abcdef".to_owned();

    // This PC doesn't watch adb, so it can't start anything over USB.
    let refused = phone.node.elevated_usb_start(pc_id, apk.clone(), token.clone(), Vec::new()).await;
    assert!(matches!(refused, Err(Error::Unsupported)), "{refused:?}");
    // A request for anything but the app's own helper never leaves the phone.
    let bad = phone.node.elevated_usb_start(pc_id, apk, token, vec!["reboot".into()]).await;
    assert!(matches!(bad, Err(Error::Protocol(_))), "{bad:?}");

    pc.node.shutdown().await;
    phone.node.shutdown().await;
}
