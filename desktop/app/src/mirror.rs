// SPDX-License-Identifier: GPL-3.0-or-later
//! A phone's screen on this PC (docs/protocol/mirror.md): asks the phone,
//! decodes its video on a thread of its own and hands each picture to the
//! mirror window's `VideoView`. Only the newest picture is shown; when the
//! decoder falls behind or loses its place, the phone is asked for a
//! keyframe.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc::{Receiver, SyncSender, TrySendError, sync_channel},
    },
    time::{Duration, Instant},
};

use nectarlink_core::{DeviceId, Error, MirrorConfig, MirrorSink, MirrorStart, NodeEvent};

use crate::{
    bridge::{app::describe, native::ffi},
    core_host,
    state::Changes,
    win::h264::{Decoder, to_bgrx},
};

/// What the PC asks for: the phone scales its screen to fit.
const OPTIONS: MirrorStart = MirrorStart { max_size: 1920, fps: 60, bitrate: 8_000_000 };
/// Packets waiting for the decoder, at most (about two seconds).
const BACKLOG: usize = 120;
/// How often decode times are logged.
const STATS_EVERY: Duration = Duration::from_secs(10);

/// Where mirroring of a phone stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// Waiting for the phone's user to agree.
    Asking,
    Showing,
    /// It ended; the reason for the window, if any.
    Ended(Option<String>),
}

#[derive(Debug, Default)]
struct State {
    phases: HashMap<DeviceId, Phase>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn state<T>(f: impl FnOnce(&mut State) -> T) -> T {
    f(STATE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_default())
}

fn set_phase(device: DeviceId, phase: Phase) {
    state(|s| s.phases.insert(device, phase));
    core_host::host().hub.changed(Changes::MIRROR);
}

/// Where mirroring of `device` stands (`None`: not asked).
pub fn phase(device: &DeviceId) -> Option<Phase> {
    state(|s| s.phases.get(device).cloned())
}

/// Asks the phone for its screen.
pub fn start(device: DeviceId) {
    set_phase(device, Phase::Asking);
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        if let Err(e) = node.mirror_start(device, OPTIONS).await {
            let reason = match e {
                Error::Denied => "Mirroring is turned off for this phone.".to_owned(),
                Error::Unsupported => {
                    "This phone's app doesn't share its screen yet. Update Nectarlink on the phone.".into()
                }
                Error::Offline | Error::NotPaired => "The phone isn't connected.".into(),
                e => describe(&e),
            };
            set_phase(device, Phase::Ended(Some(reason)));
        }
    });
}

/// Stops showing the phone's screen (closing the window).
pub fn stop(device: DeviceId) {
    state(|s| s.phases.remove(&device));
    ffi::video_clear(&device.to_string());
    core_host::host().hub.changed(Changes::MIRROR);
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move { node.mirror_stop(device).await });
}

pub fn on_event(event: &NodeEvent) {
    match event {
        NodeEvent::Mirroring { device, on: true } => set_phase(*device, Phase::Showing),
        // Closed here: nothing to say. Stopped on the phone: say so.
        NodeEvent::Mirroring { device, on: false } if phase(device).is_some() => {
            set_phase(*device, Phase::Ended(None));
        }
        _ => {}
    }
}

/// Where the core puts a phone's video: only for phones this PC asked.
pub fn sink(device: &DeviceId) -> Option<Arc<dyn MirrorSink>> {
    matches!(phase(device), Some(Phase::Asking | Phase::Showing))
        .then(|| Arc::new(Sink::start(*device)) as Arc<dyn MirrorSink>)
}

enum Item {
    Config(MirrorConfig),
    Packet { keyframe: bool, time_us: u64, data: Vec<u8> },
    Ended,
}

/// Hands the core's packets to the decoder thread.
struct Sink {
    device: DeviceId,
    queue: SyncSender<Item>,
    /// Packets waiting for the decoder.
    waiting: Arc<AtomicUsize>,
}

impl Sink {
    fn start(device: DeviceId) -> Sink {
        let (queue, items) = sync_channel(BACKLOG);
        let waiting = Arc::new(AtomicUsize::new(0));
        let counter = waiting.clone();
        let started = std::thread::Builder::new()
            .name("mirror-decoder".into())
            .spawn(move || decode(device, &items, &counter));
        if let Err(e) = started {
            tracing::error!(error = %e, "can't start the video decoder");
        }
        Sink { device, queue, waiting }
    }

    fn keyframe_please(&self) {
        let device = self.device;
        if let Some(node) = core_host::node() {
            core_host::spawn(async move { node.mirror_keyframe(device).await });
        }
    }
}

impl MirrorSink for Sink {
    fn config(&self, config: MirrorConfig) {
        // The format matters: wait for room rather than drop it.
        let _ = self.queue.send(Item::Config(config));
    }

    fn packet(&self, keyframe: bool, time_us: u64, data: Vec<u8>) {
        match self.queue.try_send(Item::Packet { keyframe, time_us, data }) {
            Ok(()) => {
                self.waiting.fetch_add(1, Ordering::AcqRel);
            }
            Err(TrySendError::Disconnected(_)) => {}
            // The decoder is behind; it starts over at the next keyframe.
            Err(TrySendError::Full(_)) => self.keyframe_please(),
        }
    }

    fn ended(&self) {
        let _ = self.queue.send(Item::Ended);
    }
}

/// The decoder thread: decodes everything (later pictures depend on
/// earlier ones), shows the newest picture of each batch.
fn decode(device: DeviceId, items: &Receiver<Item>, waiting: &AtomicUsize) {
    let stream = device.to_string();
    let mut decoder = match Decoder::new() {
        Ok(decoder) => decoder,
        Err(e) => {
            tracing::error!(error = %e, "no H.264 decoder");
            set_phase(device, Phase::Ended(Some("This PC can't decode the phone's video.".into())));
            return;
        }
    };
    let keyframe_please = || {
        if let Some(node) = core_host::node() {
            core_host::spawn(async move { node.mirror_keyframe(device).await });
        }
    };
    let mut size = (0, 0);
    // After a gap, pictures are garbage until the next keyframe.
    let mut need_keyframe = true;
    let mut stats = Stats::new();
    while let Ok(item) = items.recv() {
        match item {
            Item::Config(config) => {
                size = (config.width, config.height);
                need_keyframe = true;
            }
            Item::Packet { keyframe, time_us, data } => {
                let behind = waiting.fetch_sub(1, Ordering::AcqRel) > 1;
                if need_keyframe && !keyframe {
                    continue;
                }
                need_keyframe = false;
                let started = Instant::now();
                let pictures = match decoder.decode(&data, time_us) {
                    Ok(pictures) => pictures,
                    Err(e) => {
                        tracing::debug!(error = %e, "a video packet didn't decode");
                        decoder.flush();
                        need_keyframe = true;
                        keyframe_please();
                        continue;
                    }
                };
                let decoded = started.elapsed();
                // Behind: skip showing this one, the next is already here.
                if let (Some(picture), false) = (pictures.last(), behind) {
                    let converted = Instant::now();
                    let (w, h) = if size.0 > 0 { size } else { (picture.width, picture.height) };
                    let bgrx = to_bgrx(picture, w, h);
                    ffi::video_frame(&stream, w.min(picture.width), h.min(picture.height), &bgrx);
                    stats.shown(decoded, converted.elapsed());
                } else {
                    stats.skipped();
                }
            }
            Item::Ended => break,
        }
    }
    ffi::video_clear(&stream);
}

/// Decode and conversion times, logged now and then.
struct Stats {
    since: Instant,
    shown: u32,
    skipped: u32,
    decode: Duration,
    convert: Duration,
}

impl Stats {
    fn new() -> Stats {
        Stats { since: Instant::now(), shown: 0, skipped: 0, decode: Duration::ZERO, convert: Duration::ZERO }
    }

    fn shown(&mut self, decode: Duration, convert: Duration) {
        self.shown += 1;
        self.decode += decode;
        self.convert += convert;
        self.maybe_log();
    }

    fn skipped(&mut self) {
        self.skipped += 1;
        self.maybe_log();
    }

    fn maybe_log(&mut self) {
        let elapsed = self.since.elapsed();
        if elapsed < STATS_EVERY || self.shown == 0 {
            return;
        }
        tracing::info!(
            fps = format!("{:.1}", f64::from(self.shown) / elapsed.as_secs_f64()),
            skipped = self.skipped,
            decode_ms = format!("{:.1}", self.decode.as_secs_f64() * 1000.0 / f64::from(self.shown)),
            convert_ms = format!("{:.1}", self.convert.as_secs_f64() * 1000.0 / f64::from(self.shown)),
            "mirroring"
        );
        *self = Stats::new();
    }
}
