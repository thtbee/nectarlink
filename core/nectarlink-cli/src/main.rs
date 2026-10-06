// SPDX-License-Identifier: GPL-3.0-or-later
//! `nectarlink`: a command-line client for development, testing and scripting.
//!
//! It runs its own node with its own identity (separate from the desktop
//! app), so it can pair with phones, PCs or another CLI instance.

use std::{io::Write, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use nectarlink_core::{
    Battery, ConnectionPath, DeviceId, DeviceInfo, DeviceKind, Direction, FeatureState, LinkState,
    MediaAction, MediaError, MediaPlayer, Node, NodeConfig, NodeEvent, Notification, NotificationAction,
    NotificationError, PairedDevice, PairingEvent, Platform, PowerAction, PowerLevel, TransferState,
    features::{Effort, FEATURES, Role, UnsupportedReason, Upgrade, UpgradeAction},
};
use tokio::sync::broadcast::error::RecvError;

#[derive(Parser, Debug)]
#[command(name = "nectarlink", version, about = "Nectarlink command-line client")]
struct Cli {
    /// Where this client keeps its identity and paired devices.
    #[arg(long, global = true, env = "NECTARLINK_DATA_DIR")]
    data_dir: Option<PathBuf>,
    /// Name announced to other devices (defaults to the computer name).
    #[arg(long, global = true)]
    name: Option<String>,
    /// Fixed UDP port to listen on (default: any).
    #[arg(long, global = true, default_value_t = 0)]
    port: u16,
    /// Disable discovery on the local network (mDNS).
    #[arg(long, global = true)]
    no_lan: bool,
    /// Allow connections through relay servers when away from home.
    #[arg(long, global = true)]
    away: bool,
    /// Announce this client as an Android phone, to test a PC without one.
    #[arg(long, global = true)]
    as_phone: bool,
    /// Power level to announce with --as-phone.
    #[arg(long, global = true, value_enum, default_value_t = Power::Basic, requires = "as_phone")]
    power: Power,
    /// Battery level (0–100) to report, to test a PC without a phone.
    #[arg(long, global = true, value_parser = clap::value_parser!(u8).range(0..=100))]
    battery: Option<u8>,
    /// Report the battery as charging (with --battery).
    #[arg(long, global = true, requires = "battery")]
    charging: bool,
    /// Extra capability to announce, e.g. clip.read.auto (repeatable).
    #[arg(long = "offer", global = true, value_name = "CAPABILITY")]
    offers: Vec<String>,
    /// Log verbosity (error, warn, info, debug, trace).
    #[arg(long, global = true, default_value = "warn")]
    log: String,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Print this device's ID and addresses.
    Id,
    /// Show a pairing code and wait for a device to pair.
    Pair,
    /// Pair with a device using its pairing link.
    Join { link: String },
    /// Pair with a nearby device by comparing a 6-digit code.
    Nearby {
        /// Device ID to pair with (omit to list devices found on the network).
        device: Option<String>,
        /// Seconds to search for devices when listing.
        #[arg(long, default_value_t = 5)]
        wait: u64,
    },
    /// List paired devices and whether they're connected.
    Devices {
        /// Seconds to wait for connections before printing.
        #[arg(long, default_value_t = 3)]
        wait: u64,
    },
    /// Ring a paired device (find my device).
    Ring {
        device: String,
        /// Stop ringing instead.
        #[arg(long)]
        off: bool,
    },
    /// Tell a device where to reach this one (when discovery is blocked).
    Connect {
        device: String,
        /// Address like 192.168.1.20:41641 (repeatable).
        #[arg(required = true)]
        addrs: Vec<SocketAddr>,
    },
    /// Unpair a device.
    Unpair { device: String },
    /// Show which features work with a paired device, and how to unlock the rest.
    Caps {
        device: String,
        /// Seconds to wait for the device to connect (0: use what's known).
        #[arg(long, default_value_t = 3)]
        wait: u64,
    },
    /// Show or change what a paired device is allowed to do.
    Toggle {
        device: String,
        /// Toggle to change, e.g. "clipboard" (omit to list them).
        name: Option<String>,
        #[arg(requires = "name")]
        state: Option<OnOff>,
    },
    /// Stay online and print events until Ctrl+C.
    Run,
    /// Send files to a paired device and wait until they've arrived.
    Send {
        device: String,
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Put text, or an image with --image, on a paired device's clipboard.
    Clip {
        device: String,
        #[arg(required_unless_present = "image", conflicts_with = "image")]
        text: Option<String>,
        /// A PNG or JPEG file to put on the clipboard instead of text.
        #[arg(long)]
        image: Option<PathBuf>,
    },
    /// Dismiss a phone's notification (`run` prints the keys).
    Dismiss { device: String, key: String },
    /// Run an action of a phone's notification, or reply to it.
    Act {
        device: String,
        key: String,
        /// The action's ID (`run` prints them).
        action: String,
        /// Text to send, for a reply action.
        #[arg(long)]
        reply: Option<String>,
    },
    /// Control what plays on a paired device: play, pause, next, previous,
    /// or seek with --position.
    Media {
        device: String,
        #[arg(value_enum)]
        action: MediaCommandArg,
        /// The player (`run` prints them); default: the device's first.
        #[arg(long)]
        player: Option<String>,
        /// Where to seek to, in seconds.
        #[arg(long, required_if_eq("action", "seek"))]
        position: Option<u64>,
    },
    /// Lock a paired PC, or put it to sleep.
    Power {
        device: String,
        #[arg(value_enum)]
        action: PowerArg,
    },
    /// Open a web link on a paired device.
    Open { device: String, url: String },
    /// Show new photos from paired phones as this PC would, and fetch each
    /// one; stays online until Ctrl+C.
    Photos,
    /// Ask a paired phone for its screen and save the video (H.264, Annex B)
    /// for `seconds`; prints what arrived.
    Mirror {
        device: String,
        out: PathBuf,
        #[arg(long, default_value_t = 10)]
        seconds: u64,
        /// Where to reach the phone, when discovery can't (e.g. an emulator
        /// with a forwarded port).
        #[arg(long)]
        at: Vec<std::net::SocketAddr>,
        /// Also ask for the phone's sound, and save it here (WAV).
        #[arg(long)]
        sound: Option<PathBuf>,
        /// An app (package name) to show in a window of its own instead of
        /// the screen (Elevated phones; `apps` lists them).
        #[arg(long)]
        app: Option<String>,
        /// Taps while recording, one every 2 s from the first frame, each
        /// "x,y" in fractions of the screen (repeatable).
        #[arg(long)]
        tap: Vec<String>,
    },
    /// List the apps a paired phone can open in windows of their own.
    Apps {
        device: String,
        /// Where to reach the phone, when discovery can't.
        #[arg(long)]
        at: Vec<std::net::SocketAddr>,
    },
    /// Use a paired phone's screen: tap at x,y (fractions of the screen),
    /// swipe, scroll, press a key (back, home, recents, enter...) or type.
    Input {
        device: String,
        /// The app window's session (from `mirror --app`), or 0 for the screen.
        #[arg(long, default_value_t = 0)]
        session: u32,
        #[command(subcommand)]
        action: InputArg,
    },
    /// Act as a phone sharing its screen (use with --as-phone): when a PC
    /// asks, streams an H.264 file (Annex B, with access unit delimiters) in
    /// a loop at `fps`. Stays online.
    Screen {
        video: PathBuf,
        #[arg(long)]
        width: u32,
        #[arg(long)]
        height: u32,
        #[arg(long, default_value_t = 60)]
        fps: u32,
        /// Sound to share too, in a loop, with PCs that ask for it (a WAV
        /// file of 16-bit PCM).
        #[arg(long)]
        sound: Option<PathBuf>,
        /// Also offer a few sample apps to open in windows (as an Elevated
        /// phone would), each streaming the same video.
        #[arg(long)]
        apps: bool,
    },
    /// Act as a phone with a few sample conversations (use with --as-phone),
    /// so a PC can read them and text through this client; stays online.
    Texts,
    /// Act as a phone with sample content all at once (use with
    /// --as-phone): notifications, a song playing, conversations and,
    /// with --call, a call in progress. For screenshots and trying the PC
    /// app; stays online.
    Demo {
        /// Also a call in progress.
        #[arg(long)]
        call: bool,
        /// Artwork for the song (JPEG or PNG).
        #[arg(long)]
        art: Option<PathBuf>,
    },
    /// Act as a phone with an incoming call (use with --as-phone): PCs can
    /// answer it, then mute, use the speaker, hold, press keys or hang up,
    /// as on a phone that controls its calls. Stays online.
    IncomingCall {
        /// The caller's number.
        #[arg(default_value = "+15550144")]
        number: String,
        /// The caller's name, as in contacts.
        #[arg(long)]
        caller: Option<String>,
        /// Without the in-call controls (`call.incall`): only answer and end.
        #[arg(long)]
        basic: bool,
        /// Already answered on the phone: the call is in progress.
        #[arg(long)]
        answered: bool,
    },
    /// A paired phone's text messages: list conversations, show one, or send.
    Sms {
        device: String,
        #[command(subcommand)]
        action: SmsArg,
    },
    /// Show calls on paired phones as this PC would; with --auto, answer,
    /// decline or silence each ringing call. Stays online until Ctrl+C.
    Calls {
        #[arg(long, value_enum)]
        auto: Option<CallArg>,
        /// With the first ringing call, run these actions in order, comma
        /// separated: answer, decline, silence, mute, unmute, speaker,
        /// earpiece, hold, unhold, up, down, a keypad key (0-9, *, #), or
        /// wait:N (seconds).
        #[arg(long)]
        script: Option<String>,
    },
    /// Announce a picture as a photo just taken on this phone (use with
    /// --as-phone), then stay online to send it to PCs that ask.
    Photo {
        path: PathBuf,
        /// A JPEG preview (at most 96 KB) to show with it.
        #[arg(long)]
        preview: Option<PathBuf>,
    },
    /// Pretend to play a song and share it with paired devices, which can
    /// control it; stays online until Ctrl+C.
    Play {
        title: String,
        #[arg(long, default_value = "Test Artist")]
        artist: String,
        /// App name shown with it.
        #[arg(long, default_value = "Music")]
        app: String,
        /// Length, in seconds.
        #[arg(long, default_value_t = 215)]
        length: u64,
        /// Artwork (JPEG or PNG).
        #[arg(long)]
        art: Option<PathBuf>,
    },
    /// Show a notification on paired PCs as if this were a phone (use with
    /// --as-phone), then stay online to show what the PC does with it.
    Notify {
        title: String,
        text: String,
        /// App name shown with it.
        #[arg(long, default_value = "Messages")]
        app: String,
        /// Offer an inline reply.
        #[arg(long)]
        reply: bool,
        /// Don't alert (like a silent notification on the phone).
        #[arg(long)]
        silent: bool,
        /// A picture it shows (JPEG, at most 160 KiB).
        #[arg(long)]
        image: Option<PathBuf>,
        /// More notifications shown with it, each "App|Title|Text"
        /// (repeatable).
        #[arg(long)]
        also: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Power {
    Basic,
    Assist,
    Elevated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum PowerArg {
    Lock,
    Sleep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum MediaCommandArg {
    Play,
    Pause,
    Next,
    Previous,
    Seek,
}

impl From<MediaCommandArg> for MediaAction {
    fn from(a: MediaCommandArg) -> Self {
        match a {
            MediaCommandArg::Play => MediaAction::Play,
            MediaCommandArg::Pause => MediaAction::Pause,
            MediaCommandArg::Next => MediaAction::Next,
            MediaCommandArg::Previous => MediaAction::Previous,
            MediaCommandArg::Seek => MediaAction::Seek,
        }
    }
}

/// Media commands from paired devices, for `play`.
static MEDIA_COMMANDS: std::sync::OnceLock<tokio::sync::mpsc::UnboundedSender<(MediaAction, Option<u64>)>> =
    std::sync::OnceLock::new();

#[derive(Debug, Subcommand)]
enum InputArg {
    Tap { x: f32, y: f32 },
    Swipe { x1: f32, y1: f32, x2: f32, y2: f32 },
    Scroll { x: f32, y: f32, notches: f32 },
    Key { key: String },
    Type { text: String },
}

#[derive(Debug, Subcommand)]
enum SmsArg {
    /// The latest conversations.
    Threads {
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// A conversation's latest messages.
    Show {
        thread: String,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Send a text.
    Send { to: String, body: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum CallArg {
    Answer,
    Decline,
    Silence,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum OnOff {
    On,
    Off,
}

/// Where `mirror` saves the video, and what arrived.
static RECORDING: std::sync::OnceLock<std::sync::Arc<Recording>> = std::sync::OnceLock::new();

#[derive(Debug, Default)]
struct Recording {
    file: std::sync::Mutex<Option<std::fs::File>>,
    stats: std::sync::Mutex<RecordingStats>,
    /// The sound's format and samples, when asked for.
    sound: std::sync::Mutex<(Option<nectarlink_core::MirrorAudioConfig>, Vec<u8>)>,
}

#[derive(Debug, Default, Clone, Copy)]
struct RecordingStats {
    frames: u32,
    keyframes: u32,
    bytes: u64,
    size: Option<(u32, u32)>,
}

impl nectarlink_core::MirrorSink for Recording {
    fn config(&self, config: nectarlink_core::MirrorConfig) {
        println!("The phone streams {}x{} {}.", config.width, config.height, config.codec);
        self.stats.lock().unwrap().size = Some((config.width, config.height));
    }
    fn packet(&self, keyframe: bool, _time_us: u64, data: Vec<u8>) {
        use std::io::Write;
        let mut stats = self.stats.lock().unwrap();
        stats.frames += 1;
        stats.keyframes += u32::from(keyframe);
        stats.bytes += data.len() as u64;
        if let Some(file) = self.file.lock().unwrap().as_mut() {
            let _ = file.write_all(&data);
        }
    }
    fn ended(&self) {
        println!("The stream ended.");
    }
    fn audio_config(&self, config: nectarlink_core::MirrorAudioConfig) {
        println!("The phone streams its sound: {} Hz, {} channels.", config.rate, config.channels);
        self.sound.lock().unwrap().0 = Some(config);
    }
    fn audio(&self, _time_us: u64, data: Vec<u8>) {
        self.sound.lock().unwrap().1.extend_from_slice(&data);
    }
    fn audio_ended(&self) {
        println!("The sound ended.");
    }
}

/// A WAV file of 16-bit PCM: its format and samples.
fn read_wav(bytes: &[u8]) -> Option<(u32, u8, &[u8])> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let (mut at, mut format) = (12, None);
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let len = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().ok()?) as usize;
        let body = bytes.get(at + 8..(at + 8 + len).min(bytes.len()))?;
        match id {
            b"fmt " if body.len() >= 16 => {
                let tag = u16::from_le_bytes([body[0], body[1]]);
                let channels = u16::from_le_bytes([body[2], body[3]]);
                let rate = u32::from_le_bytes(body[4..8].try_into().ok()?);
                let bits = u16::from_le_bytes([body[14], body[15]]);
                if tag != 1 || bits != 16 || !(1..=2).contains(&channels) {
                    return None;
                }
                format = Some((rate, channels as u8));
            }
            b"data" => return format.map(|(rate, channels)| (rate, channels, body)),
            _ => {}
        }
        at += 8 + len + (len & 1);
    }
    None
}

fn write_wav(path: &std::path::Path, rate: u32, channels: u8, samples: &[u8]) -> std::io::Result<()> {
    let block = 2 * u32::from(channels);
    let mut out = Vec::with_capacity(44 + samples.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + samples.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&u16::from(channels).to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * block).to_le_bytes());
    out.extend_from_slice(&(block as u16).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(samples.len() as u32).to_le_bytes());
    out.extend_from_slice(samples);
    std::fs::write(path, out)
}

/// Streams PCM in 10 ms packets, in real time, in a loop, until the PC
/// stops listening.
fn stream_sound(sound: &nectarlink_core::MirrorStream, rate: u32, channels: u8, samples: &[u8]) {
    use nectarlink_core::{MirrorAudioConfig, MirrorSend, PacketKind};
    let config = MirrorAudioConfig { codec: nectarlink_core::MIRROR_PCM.into(), rate, channels };
    if sound.send(PacketKind::Config, 0, config.to_cbor()) == MirrorSend::Closed {
        return;
    }
    let packet = (rate / 100) as usize * config.frame_bytes();
    let started = std::time::Instant::now();
    for (sent, chunk) in (0u64..).zip(samples.chunks_exact(packet).cycle()) {
        let due = std::time::Duration::from_millis(sent * 10);
        if let Some(wait) = due.checked_sub(started.elapsed()) {
            std::thread::sleep(wait);
        }
        if sound.send(PacketKind::Frame, sent * 10_000, chunk.to_vec()) == MirrorSend::Closed {
            break;
        }
    }
    println!("Stopped sharing the sound.");
}

/// PCs that asked for the screen or an app (whether they want the sound,
/// and the session), for `screen`.
static SCREEN_ASKS: std::sync::OnceLock<tokio::sync::mpsc::UnboundedSender<(DeviceId, bool, u32)>> =
    std::sync::OnceLock::new();
/// Whether `screen` offers apps in windows.
static SCREEN_APPS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// The apps `screen --apps` offers.
const SAMPLE_APPS: &[(&str, &str)] = &[
    ("com.example.calendar", "Calendar"),
    ("com.example.camera", "Camera"),
    ("com.example.chat", "Chat"),
    ("com.example.maps", "Maps"),
    ("com.example.music", "Music"),
    ("com.example.notes", "Notes"),
    ("com.example.photos", "Photos"),
    ("com.example.settings", "Settings"),
];

/// A small, plain PNG icon in a color of its own (a rounded square would
/// need drawing; a solid one is enough to tell them apart).
fn sample_icon(i: usize) -> Vec<u8> {
    const SIZE: u32 = 48;
    let hues = [(234, 67, 53), (66, 133, 244), (52, 168, 83), (251, 188, 5), (171, 71, 188), (0, 172, 193)];
    let (r, g, b) = hues[i % hues.len()];
    // Each scanline: no filter, then its pixels.
    let line: Vec<u8> =
        std::iter::once(0).chain(std::iter::repeat_n([r, g, b], SIZE as usize).flatten()).collect();
    let raw = line.repeat(SIZE as usize);
    png(SIZE, SIZE, &raw)
}

/// An RGB PNG from filtered scanlines (stored, uncompressed).
fn png(width: u32, height: u32, raw: &[u8]) -> Vec<u8> {
    fn crc(data: &[u8]) -> u32 {
        let mut c = 0xffff_ffffu32;
        for &b in data {
            c ^= u32::from(b);
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let start = out.len();
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let sum = crc(&out[start..]);
        out.extend_from_slice(&sum.to_be_bytes());
    }
    // zlib with stored blocks, and its Adler-32.
    let mut zlib = vec![0x78, 0x01];
    for (i, block) in raw.chunks(65_535).enumerate() {
        let last = (i + 1) * 65_535 >= raw.len();
        zlib.push(u8::from(last));
        let len = block.len() as u16;
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&(!len).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in raw {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    zlib.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &header);
    chunk(&mut out, b"IDAT", &zlib);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// A sample text for `texts`: (thread, number, name, incoming, body, Unix ms).
type SampleText = (u32, String, String, bool, String, i64);
static TEXTS: std::sync::Mutex<Vec<SampleText>> = std::sync::Mutex::new(Vec::new());
/// Set by `texts`, to tell PCs when the samples changed.
static TEXTS_NODE: std::sync::OnceLock<Node> = std::sync::OnceLock::new();

fn sample_texts() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    let min = 60_000;
    let samples = [
        (1, "+15550100", "Sam Rivera", true, "Are we still on for dinner tonight?", now - 50 * min),
        (1, "+15550100", "Sam Rivera", false, "Yes! 7:30 at the usual place", now - 48 * min),
        (1, "+15550100", "Sam Rivera", true, "Perfect. I'll book a table 🍜", now - 47 * min),
        (
            2,
            "+15550123",
            "",
            true,
            "Your parcel will arrive tomorrow between 9:00 and 13:00.",
            now - 26 * 60 * min,
        ),
        (2, "+15550123", "", true, "Track it at https://example.com/track?id=42.", now - 26 * 60 * min + min),
        (4, "JD-BANK", "", true, "243928 is your one time password (OTP). Do not share it.", now - 5 * min),
        (3, "+15550188", "Alex", false, "Can you send me the photos from Saturday?", now - 3 * 24 * 60 * min),
        (3, "+15550188", "Alex", true, "Sure, uploading them now", now - 3 * 24 * 60 * min + 5 * min),
    ];
    *TEXTS.lock().unwrap() = samples
        .into_iter()
        .map(|(t, n, name, incoming, body, date)| (t, n.into(), name.into(), incoming, body.into(), date))
        .collect();
}

/// The call `incoming-call` pretends to have, and how to tell PCs it changed.
static CALL: std::sync::Mutex<Option<nectarlink_core::CallState>> = std::sync::Mutex::new(None);
static CALL_NODE: std::sync::OnceLock<Node> = std::sync::OnceLock::new();
/// Whether `incoming-call` offers in-call controls (without --basic).
static CALL_CONTROLS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The picture `photo` announced, for PCs that ask for it.
static PHOTO: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
const PHOTO_ID: &str = "cli-photo";

/// Rings by printing to the terminal (the CLI has no speaker access).
#[derive(Debug)]
struct TerminalPlatform;

impl Platform for TerminalPlatform {
    fn mirror_sink(
        &self,
        _peer: &DeviceId,
        _session: u32,
    ) -> Option<std::sync::Arc<dyn nectarlink_core::MirrorSink>> {
        RECORDING.get().map(|r| r.clone() as std::sync::Arc<dyn nectarlink_core::MirrorSink>)
    }
    fn mirror_requested(
        &self,
        peer: &DeviceId,
        options: &nectarlink_core::MirrorStart,
    ) -> std::result::Result<(), String> {
        let asks = SCREEN_ASKS.get().ok_or("not sharing a screen (run `screen`)")?;
        match &options.app {
            Some(app) if SAMPLE_APPS.iter().any(|(pkg, _)| pkg == app) => {
                println!("A PC opened {app} in a window (session {}).", options.session);
            }
            Some(_) => return Err("no such app".into()),
            None => {
                if options.audio {
                    println!("It wants the sound too.");
                }
                println!(
                    "A PC asked for the screen ({}px, {} fps); sharing it.",
                    options.max_size, options.fps
                );
            }
        }
        asks.send((*peer, options.audio, options.session)).map_err(|e| e.to_string())
    }
    fn mirror_stop_requested(&self, _peer: &DeviceId, session: u32) {
        println!("The PC stopped watching (session {session}).");
    }
    fn mirror_input(&self, _peer: &DeviceId, session: u32, input: nectarlink_core::MirrorInput) {
        println!("Input on session {session}: {input:?}");
    }
    fn phone_apps(&self) -> std::result::Result<Vec<nectarlink_core::PhoneApp>, String> {
        if !SCREEN_APPS.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("not offering apps (run `screen --apps`)".into());
        }
        Ok(SAMPLE_APPS
            .iter()
            .enumerate()
            .map(|(i, (pkg, label))| nectarlink_core::PhoneApp {
                pkg: (*pkg).into(),
                label: (*label).into(),
                icon: Some(sample_icon(i)),
            })
            .collect())
    }
    fn mirror_keyframe_requested(&self, _peer: &DeviceId, _session: u32) {
        println!("The PC asked for a keyframe.");
    }
    fn sms_threads(&self, limit: u32) -> std::result::Result<Vec<nectarlink_core::SmsThread>, String> {
        let texts = TEXTS.lock().unwrap();
        let mut threads: Vec<nectarlink_core::SmsThread> = Vec::new();
        for (thread, number, name, incoming, body, date) in texts.iter() {
            match threads.iter_mut().find(|t| t.id == thread.to_string()) {
                Some(t) if *date > t.date => {
                    t.date = *date;
                    t.snippet = body.clone();
                    t.unread = u32::from(*incoming);
                }
                Some(_) => {}
                None => threads.push(nectarlink_core::SmsThread {
                    id: thread.to_string(),
                    addresses: vec![number.clone()],
                    names: vec![name.clone()],
                    snippet: body.clone(),
                    date: *date,
                    unread: u32::from(*incoming),
                    photo: None,
                }),
            }
        }
        threads.sort_by_key(|t| std::cmp::Reverse(t.date));
        threads.truncate(limit as usize);
        Ok(threads)
    }
    fn sms_messages(
        &self,
        thread: &str,
        before: Option<i64>,
        limit: u32,
    ) -> std::result::Result<Vec<nectarlink_core::SmsMessage>, String> {
        let texts = TEXTS.lock().unwrap();
        let mut messages: Vec<nectarlink_core::SmsMessage> = texts
            .iter()
            .enumerate()
            .filter(|(_, (t, .., date))| t.to_string() == thread && before.is_none_or(|b| *date < b))
            .map(|(i, (t, number, _, incoming, body, date))| nectarlink_core::SmsMessage {
                id: format!("sms:{i}"),
                thread: t.to_string(),
                address: number.clone(),
                body: body.clone(),
                date: *date,
                outgoing: !incoming,
                status: (!incoming).then(|| "sent".into()),
                parts: Vec::new(),
            })
            .collect();
        messages.sort_by_key(|m| std::cmp::Reverse(m.date));
        messages.truncate(limit as usize);
        Ok(messages)
    }
    fn sms_send(&self, to: &[String], body: &str) -> std::result::Result<(), String> {
        println!("A PC sent a text to {}: {body}", to.join(", "));
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let mut texts = TEXTS.lock().unwrap();
        let thread = texts.iter().find(|(_, n, ..)| *n == to[0]).map(|(t, ..)| *t);
        let thread = thread.unwrap_or_else(|| texts.iter().map(|(t, ..)| *t).max().unwrap_or(0) + 1);
        texts.push((thread, to[0].clone(), String::new(), false, body.to_owned(), now));
        if let Some(node) = TEXTS_NODE.get().cloned() {
            tokio::runtime::Handle::current().spawn(async move { node.sms_changed(None).await });
        }
        Ok(())
    }
    fn call_command(
        &self,
        id: &str,
        command: nectarlink_core::CallCommand,
    ) -> std::result::Result<(), String> {
        use nectarlink_core::CallCommand as C;
        let mut current = CALL.lock().unwrap();
        let call = current.as_mut().filter(|c| c.id == id).ok_or("no such call")?;
        let controls = call.controls.get_or_insert_default();
        match command {
            C::Answer => {
                controls.can_hold = true;
                call.state = "active".into();
                call.photo = None;
                call.since = Some(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_millis() as i64),
                );
            }
            C::Decline => call.state = "ended".into(),
            C::Silence => println!("Silenced."),
            C::Mute(on) => controls.muted = on,
            C::Speaker(on) => controls.speaker = on,
            C::Hold(on) => controls.held = on,
            C::Dtmf(digit) => println!("Key {digit}."),
            C::Volume(up) => println!("Volume {}.", if up { "up" } else { "down" }),
        }
        println!("A PC asked: {command:?}.");
        // Only an answered call has controls, and only on a phone that offers them.
        if call.state != "active" || !CALL_CONTROLS.load(std::sync::atomic::Ordering::Relaxed) {
            call.controls = None;
        }
        let call = call.clone();
        if call.state == "ended" {
            *current = None;
        }
        if let Some(node) = CALL_NODE.get().cloned() {
            tokio::runtime::Handle::current().spawn(async move {
                if let Err(e) = node.call_changed(call).await {
                    println!("Couldn't tell PCs: {e}");
                }
            });
        }
        Ok(())
    }
    fn open_photo(&self, id: &str) -> std::result::Result<nectarlink_core::OutgoingFile, String> {
        let path = PHOTO.get().filter(|_| id == PHOTO_ID).ok_or("no such photo")?;
        println!("A PC asked for the photo; sending it.");
        Ok(nectarlink_core::OutgoingFile {
            name: nectarlink_core::safe_file_name(&path.to_string_lossy()),
            folder: None,
            source: nectarlink_core::FileSource::Path(path.clone()),
        })
    }
    fn start_ringing(&self) {
        println!("\x07🔔 Ringing! (stop it from the other device)");
    }
    fn stop_ringing(&self) {
        println!("🔕 Stopped ringing.");
    }
    fn set_clipboard(&self, text: &str) -> Result<(), String> {
        println!("Clipboard from a paired device: {text}");
        Ok(())
    }
    fn set_clipboard_image(&self, mime: &str, bytes: &[u8]) -> Result<(), String> {
        let extension = if mime == "image/jpeg" { "jpg" } else { "png" };
        let path = std::env::temp_dir().join(format!("nectarlink-clipboard.{extension}"));
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        println!("Clipboard image from a paired device ({} bytes): {}", bytes.len(), path.display());
        Ok(())
    }
    fn media_command(
        &self,
        player: &str,
        action: MediaAction,
        position: Option<u64>,
    ) -> Result<(), MediaError> {
        println!(
            "A paired device asked {player} to {}{}",
            action.as_str(),
            match position {
                Some(ms) => format!(" to {}", clock(ms)),
                None => String::new(),
            }
        );
        let sender = MEDIA_COMMANDS.get().ok_or(MediaError::NotFound)?;
        sender.send((action, position)).map_err(|_| MediaError::NotFound)
    }
    fn open_link(&self, _from: &DeviceId, url: &str) -> Result<(), String> {
        println!("A paired device sent a link: {url}");
        Ok(())
    }
    fn power(&self, action: PowerAction) -> Result<(), String> {
        println!("A paired device asked this computer to {} (not done: this is the CLI)", action.as_str());
        Ok(())
    }
    fn dismiss_notification(&self, key: &str) -> Result<(), NotificationError> {
        println!("The PC dismissed {key}");
        Ok(())
    }
    fn run_notification_action(
        &self,
        key: &str,
        action: &str,
        reply: Option<&str>,
    ) -> Result<(), NotificationError> {
        match reply {
            Some(text) => println!("The PC replied to {key}: {text}"),
            None => println!("The PC ran action {action} of {key}"),
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_new(&cli.log).unwrap_or_else(|_| "warn".into()))
        .with_writer(std::io::stderr)
        .init();

    let node = start_node(&cli).await?;
    let result = run(&cli, &node).await;
    node.shutdown().await;
    result
}

async fn start_node(cli: &Cli) -> Result<Node> {
    let data_dir = match &cli.data_dir {
        Some(dir) => dir.clone(),
        None => dirs::data_local_dir().context("no local data directory")?.join("Nectarlink").join("cli"),
    };
    let name = cli.name.clone().unwrap_or_else(|| {
        hostname::get().ok().and_then(|h| h.into_string().ok()).unwrap_or_else(|| "Nectarlink CLI".into())
    });
    let (kind, os, os_ver) = if cli.as_phone {
        (DeviceKind::Phone, "android".into(), "16".into())
    } else {
        (DeviceKind::Desktop, std::env::consts::OS.into(), String::new())
    };
    let device = DeviceInfo { name, kind, os, os_ver, model: None, accent: None };
    let power = node_power(cli);
    let mut config = NodeConfig::new(data_dir, device, env!("CARGO_PKG_VERSION"));
    config.port = cli.port;
    config.lan_discovery = !cli.no_lan;
    config.away_mode = cli.away;
    config.power = power;
    let node = Node::start(config, Arc::new(TerminalPlatform)).await.context("failed to start")?;
    if !cli.offers.is_empty() {
        node.update_power(power, cli.offers.clone()).await;
    }
    if let Some(level) = cli.battery {
        let plugged = cli.charging.then(|| "ac".to_owned());
        node.update_battery(Battery { level, charging: cli.charging, plugged }).await;
    }
    Ok(node)
}

fn node_power(cli: &Cli) -> PowerLevel {
    match (cli.as_phone, cli.power) {
        (false, _) => PowerLevel::NotApplicable,
        (true, Power::Basic) => PowerLevel::Basic,
        (true, Power::Assist) => PowerLevel::Assist,
        (true, Power::Elevated) => PowerLevel::Elevated,
    }
}

async fn run(cli: &Cli, node: &Node) -> Result<()> {
    match &cli.command {
        Command::Id => {
            println!("Device ID: {}", node.device_id());
            for addr in node.direct_addrs().await {
                println!("Address:   {addr}");
            }
        }
        Command::Pair => pair(node).await?,
        Command::Join { link } => {
            println!("Pairing…");
            node.pairing_join(link).await.context("pairing failed")?;
            println!("✓ Paired.");
        }
        Command::Nearby { device: None, wait } => {
            println!("Looking for devices on this network for {wait}s…");
            tokio::time::sleep(Duration::from_secs(*wait)).await;
            let found = node.discovered_devices();
            if found.is_empty() {
                println!("No devices found. Make sure the other device is showing its pairing screen.");
            }
            for d in found {
                println!("{}  {}", d.id, d.name.as_deref().unwrap_or("(unnamed)"));
            }
        }
        Command::Nearby { device: Some(device), .. } => nearby(node, parse_id(device)?).await?,
        Command::Devices { wait } => {
            tokio::time::sleep(Duration::from_secs(*wait)).await;
            let devices = node.paired_devices()?;
            if devices.is_empty() {
                println!("No paired devices. Run `nectarlink pair` to pair one.");
            }
            for d in devices {
                print_device(&d);
            }
        }
        Command::Ring { device, off } => {
            let id = resolve(node, device)?;
            wait_until_online(node, id).await?;
            node.ring(id, !off).await.context("ring failed")?;
            println!("{}", if *off { "Stopped ringing." } else { "Ringing…" });
        }
        Command::Send { device, files } => {
            let id = resolve(node, device)?;
            wait_until_online(node, id).await?;
            send_files(node, id, files).await?;
        }
        Command::Clip { device, text, image } => {
            let id = resolve(node, device)?;
            wait_until_online(node, id).await?;
            match (text, image) {
                (_, Some(path)) => {
                    let mime = match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase) {
                        Some(e) if e == "png" => "image/png",
                        Some(e) if e == "jpg" || e == "jpeg" => "image/jpeg",
                        _ => anyhow::bail!("only PNG and JPEG images can go on the clipboard"),
                    };
                    let bytes =
                        std::fs::read(path).with_context(|| format!("can't read {}", path.display()))?;
                    node.send_clipboard_image(id, mime.into(), bytes).await.context("clipboard failed")?;
                }
                (Some(text), None) => {
                    node.send_clipboard(id, text.clone()).await.context("clipboard failed")?
                }
                (None, None) => unreachable!("clap requires text or --image"),
            }
            println!("Sent.");
        }
        Command::Dismiss { device, key } => {
            let id = resolve(node, device)?;
            wait_until_online(node, id).await?;
            node.dismiss_notification(id, key.clone()).await.context("dismiss failed")?;
            println!("Dismissed.");
        }
        Command::Act { device, key, action, reply } => {
            let id = resolve(node, device)?;
            wait_until_online(node, id).await?;
            node.run_notification_action(id, key.clone(), action.clone(), reply.clone())
                .await
                .context("action failed")?;
            println!("Done.");
        }
        Command::Connect { device, addrs } => {
            let id = resolve(node, device)?;
            node.add_known_addrs(id, addrs);
            wait_until_online(node, id).await?;
            println!("✓ Connected.");
        }
        Command::Unpair { device } => {
            let id = resolve(node, device)?;
            node.unpair(id).await?;
            println!("✓ Unpaired.");
        }
        Command::Caps { device, wait } => {
            let id = resolve(node, device)?;
            if *wait > 0
                && tokio::time::timeout(Duration::from_secs(*wait), wait_until_online(node, id))
                    .await
                    .is_err()
            {
                println!("(not connected; showing what the device offered last time)");
            }
            print_capabilities(node, id)?;
        }
        Command::Toggle { device, name: None, .. } => {
            let id = resolve(node, device)?;
            for (name, on) in node.device_toggles(id)? {
                println!("{name:<16} {}", if on { "on" } else { "off" });
            }
        }
        Command::Toggle { device, name: Some(name), state } => {
            let id = resolve(node, device)?;
            let current = node.device_toggles(id)?.into_iter().find(|(n, _)| n == name).map(|(_, on)| on);
            let Some(current) = current else {
                bail!("unknown toggle {name:?}; run without a name to list them")
            };
            let on = match state {
                Some(OnOff::On) => true,
                Some(OnOff::Off) => false,
                None => !current,
            };
            node.set_device_toggle(id, name, on)?;
            println!("{name} is now {}", if on { "on" } else { "off" });
        }
        Command::Run => watch(node, false).await?,
        Command::Photos => {
            let mut offers = cli.offers.clone();
            offers.push(nectarlink_core::PHOTOS_SHOW.into());
            node.update_power(node_power(cli), offers).await;
            watch(node, true).await?;
        }
        Command::Input { device, session, action } => {
            use nectarlink_core::{MirrorInput, TouchAction};
            let mut offers = cli.offers.clone();
            offers.push(nectarlink_core::MIRROR_VIEW.into());
            node.update_power(node_power(cli), offers).await;
            let id = resolve(node, device)?;
            wait_until_online(node, id).await?;
            let touch = |action, x, y| MirrorInput::Touch { action, x, y };
            let inputs = match action {
                InputArg::Tap { x, y } => {
                    vec![touch(TouchAction::Down, *x, *y), touch(TouchAction::Up, *x, *y)]
                }
                InputArg::Swipe { x1, y1, x2, y2 } => (0..=10)
                    .map(|i| {
                        let t = i as f32 / 10.0;
                        let action = match i {
                            0 => TouchAction::Down,
                            10 => TouchAction::Up,
                            _ => TouchAction::Move,
                        };
                        touch(action, x1 + (x2 - x1) * t, y1 + (y2 - y1) * t)
                    })
                    .collect(),
                InputArg::Scroll { x, y, notches } => {
                    vec![MirrorInput::Scroll { x: *x, y: *y, dx: 0.0, dy: *notches }]
                }
                InputArg::Key { key } => vec![MirrorInput::Key { key: key.clone() }],
                InputArg::Type { text } => vec![MirrorInput::Text { text: text.clone() }],
            };
            for input in inputs {
                node.mirror_input(id, *session, input).await.context("not sent")?;
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
            // Let it leave before this client goes.
            tokio::time::sleep(Duration::from_millis(500)).await;
            println!("Sent.");
        }
        Command::Apps { device, at } => {
            let id = resolve(node, device)?;
            if !at.is_empty() {
                node.add_known_addrs(id, at);
            }
            wait_until_online(node, id).await?;
            for app in node.mirror_apps(id).await.context("no apps")? {
                println!(
                    "{:<40} {}{}",
                    app.pkg,
                    app.label,
                    app.icon.map(|i| format!(" ({} B icon)", i.len())).unwrap_or_default()
                );
            }
        }
        Command::Mirror { device, out, seconds, at, sound, app, tap } => {
            let recording = std::sync::Arc::new(Recording::default());
            *recording.file.lock().unwrap() =
                Some(std::fs::File::create(out).context("can't create the file")?);
            let _ = RECORDING.set(recording.clone());
            let mut offers = cli.offers.clone();
            offers.push(nectarlink_core::MIRROR_VIEW.into());
            if sound.is_some() {
                offers.push(nectarlink_core::MIRROR_LISTEN.into());
            }
            node.update_power(node_power(cli), offers).await;
            let id = resolve(node, device)?;
            if !at.is_empty() {
                node.add_known_addrs(id, at);
            }
            wait_until_online(node, id).await?;
            // An app window is a session of its own.
            let session = u32::from(app.is_some());
            let options = nectarlink_core::MirrorStart {
                max_size: 1920,
                fps: 60,
                bitrate: 8_000_000,
                audio: sound.is_some() && app.is_none(),
                session,
                app: app.clone(),
            };
            node.mirror_start(id, options).await.context("the phone didn't ask its user")?;
            println!(
                "Asked the phone; accept on the phone. Recording for {seconds} s after the first frame."
            );
            let mut events = node.events();
            loop {
                if let Ok(NodeEvent::Mirroring { on: true, .. }) = events.recv().await {
                    break;
                }
            }
            let started = std::time::Instant::now();
            for at in tap {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                let (x, y) = at.split_once(',').context("--tap takes x,y")?;
                let (x, y): (f32, f32) = (x.trim().parse()?, y.trim().parse()?);
                use nectarlink_core::{MirrorInput, TouchAction};
                for action in [TouchAction::Down, TouchAction::Up] {
                    node.mirror_input(id, session, MirrorInput::Touch { action, x, y })
                        .await
                        .context("not sent")?;
                }
                println!("Tapped {x},{y}.");
            }
            let left = std::time::Duration::from_secs(*seconds).saturating_sub(started.elapsed());
            tokio::time::sleep(left).await;
            node.mirror_stop(id, session).await;
            let RecordingStats { frames, keyframes, bytes, size } = *recording.stats.lock().unwrap();
            let secs = started.elapsed().as_secs_f64();
            println!(
                "{frames} frames ({keyframes} keyframes) in {secs:.1} s: {:.1} fps, {:.1} Mbit/s{}",
                f64::from(frames) / secs,
                bytes as f64 * 8.0 / secs / 1e6,
                size.map(|(w, h)| format!(", {w}x{h}")).unwrap_or_default()
            );
            if let Some(path) = sound {
                let (config, samples) = std::mem::take(&mut *recording.sound.lock().unwrap());
                match config {
                    Some(config) => {
                        let peak = samples
                            .as_chunks::<2>()
                            .0
                            .iter()
                            .map(|s| i16::from_le_bytes(*s).unsigned_abs())
                            .max()
                            .unwrap_or(0);
                        let ms = samples.len() as u64 * 1000
                            / (u64::from(config.rate) * config.frame_bytes() as u64);
                        write_wav(path, config.rate, config.channels, &samples)
                            .context("can't save the sound")?;
                        println!("Sound: {ms} ms, peak {peak} of 32767.");
                    }
                    None => println!("No sound came."),
                }
            }
        }
        Command::Screen { video, width, height, fps, sound, apps } => {
            if !cli.as_phone {
                bail!("screens are shared by phones: add --as-phone");
            }
            let stream = std::fs::read(video).context("can't read the video")?;
            let units = access_units(&stream);
            if units.is_empty() {
                bail!("no access unit delimiters in the video");
            }
            let sound = sound.as_deref().map(std::fs::read).transpose().context("can't read the sound")?;
            let sound = match &sound {
                Some(bytes) => {
                    let (rate, channels, samples) = read_wav(bytes).context("not a 16-bit PCM WAV file")?;
                    Some((rate, channels, std::sync::Arc::new(samples.to_vec())))
                }
                None => None,
            };
            let (asks, mut asked) = tokio::sync::mpsc::unbounded_channel();
            let _ = SCREEN_ASKS.set(asks);
            let mut offers = cli.offers.clone();
            offers.push(nectarlink_core::MIRROR_CAPTURE.into());
            if sound.is_some() {
                offers.push(nectarlink_core::MIRROR_AUDIO_PLAYBACK.into());
            }
            if *apps {
                SCREEN_APPS.store(true, std::sync::atomic::Ordering::Relaxed);
                offers.push(nectarlink_core::MIRROR_VIRTUAL_DISPLAY.into());
            }
            node.update_power(node_power(cli), offers).await;
            println!("Sharing a {width}x{height} screen ({} frames) with PCs that ask.", units.len());
            let streamer = node.clone();
            let (width, height, fps) = (*width, *height, *fps);
            tokio::spawn(async move {
                while let Some((pc, wants_sound, session)) = asked.recv().await {
                    let mirror = match streamer.mirror_open(pc).await {
                        Ok(mirror) => std::sync::Arc::new(mirror),
                        Err(e) => {
                            println!("Couldn't open the stream: {e}");
                            continue;
                        }
                    };
                    let units = units.clone();
                    std::thread::spawn(move || stream_screen(&mirror, &units, width, height, fps, session));
                    if let (true, Some((rate, channels, samples))) = (wants_sound, sound.clone()) {
                        match streamer.mirror_open_audio(pc).await {
                            Ok(stream) => {
                                std::thread::spawn(move || stream_sound(&stream, rate, channels, &samples));
                            }
                            Err(e) => println!("Couldn't open the sound stream: {e}"),
                        }
                    }
                }
            });
            watch(node, false).await?;
        }
        Command::Texts => {
            if !cli.as_phone {
                bail!("texts are on phones: add --as-phone");
            }
            sample_texts();
            let _ = TEXTS_NODE.set(node.clone());
            let mut offers = cli.offers.clone();
            offers.extend([nectarlink_core::SMS_READ.to_owned(), nectarlink_core::SMS_SEND.to_owned()]);
            node.update_power(node_power(cli), offers).await;
            println!("Sharing sample conversations with paired PCs.");
            watch(node, false).await?;
        }
        Command::IncomingCall { number, caller, basic, answered } => {
            if !cli.as_phone {
                bail!("calls are on phones: add --as-phone");
            }
            let _ = CALL_NODE.set(node.clone());
            let mut offers = cli.offers.clone();
            offers
                .extend([nectarlink_core::CALLS_STATE.to_owned(), nectarlink_core::CALLS_CONTROL.to_owned()]);
            CALL_CONTROLS.store(!*basic, std::sync::atomic::Ordering::Relaxed);
            if !*basic {
                offers.push(nectarlink_core::CALLS_IN_CALL.to_owned());
            }
            node.update_power(node_power(cli), offers).await;
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis() as i64;
            let call = nectarlink_core::CallState {
                id: now.to_string(),
                state: if *answered { "active" } else { "ringing" }.into(),
                incoming: true,
                number: Some(number.clone()),
                name: caller.clone(),
                photo: None,
                missed: false,
                since: answered.then_some(now),
                controls: (*answered && !*basic)
                    .then(|| nectarlink_core::CallControls { can_hold: true, ..Default::default() }),
            };
            *CALL.lock().unwrap() = Some(call.clone());
            node.call_changed(call).await.context("can't tell PCs")?;
            println!("Ringing on connected PCs (and on others when they connect).");
            watch(node, false).await?;
        }
        Command::Demo { call, art } => {
            if !cli.as_phone {
                bail!("the demo is a phone: add --as-phone");
            }
            sample_texts();
            let _ = TEXTS_NODE.set(node.clone());
            let _ = CALL_NODE.set(node.clone());
            CALL_CONTROLS.store(true, std::sync::atomic::Ordering::Relaxed);
            let mut offers = cli.offers.clone();
            offers.extend(
                [
                    "notify.mirror",
                    "notify.reply",
                    "media.control",
                    "clip.write",
                    "clip.share",
                    nectarlink_core::SMS_READ,
                    nectarlink_core::SMS_SEND,
                    nectarlink_core::CALLS_STATE,
                    nectarlink_core::CALLS_CONTROL,
                    nectarlink_core::CALLS_IN_CALL,
                ]
                .map(str::to_owned),
            );
            node.update_power(node_power(cli), offers).await;
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis() as i64;
            let samples = [
                ("Messages", "Sam Rivera", "Are we still on for dinner tonight?", true),
                ("Calendar", "Design review", "Today 15:30 · Room 4", false),
                (
                    "Gmail",
                    "Alex Chen",
                    "Photos from Saturday: here they are, plus the ones from the lake",
                    false,
                ),
            ];
            for (i, (app, title, text, reply)) in samples.into_iter().enumerate() {
                let mut actions = vec![NotificationAction {
                    id: "read".into(),
                    title: "Mark as read".into(),
                    reply: false,
                }];
                if reply {
                    actions.insert(
                        0,
                        NotificationAction { id: "reply".into(), title: "Reply".into(), reply: true },
                    );
                }
                node.notification_posted(Notification {
                    key: format!("demo|{i}"),
                    app: format!("dev.nectarlink.demo.{}", app.to_lowercase()),
                    app_name: app.into(),
                    title: Some(title.into()),
                    text: Some(text.into()),
                    sub: None,
                    when: now - 7 * 60_000 * i as i64,
                    actions,
                    silent: true,
                    icon: None,
                    image: None,
                })
                .await;
            }
            if *call {
                let call = nectarlink_core::CallState {
                    id: now.to_string(),
                    state: "active".into(),
                    incoming: true,
                    number: Some("+15550144".into()),
                    name: Some("Sam Rivera".into()),
                    photo: None,
                    missed: false,
                    since: Some(now - 83_000),
                    controls: Some(nectarlink_core::CallControls { can_hold: true, ..Default::default() }),
                };
                *CALL.lock().unwrap() = Some(call.clone());
                node.call_changed(call).await.context("can't tell PCs")?;
            }
            println!("Showing sample content to paired PCs.");
            play(node, "Golden Hour Drive", "Lumen Coast", "Music", 215, art.as_deref()).await?;
        }
        Command::Sms { device, action } => {
            let mut offers = cli.offers.clone();
            offers.push(nectarlink_core::SMS_SHOW.into());
            node.update_power(node_power(cli), offers).await;
            let id = resolve(node, device)?;
            wait_until_online(node, id).await?;
            match action {
                SmsArg::Threads { limit } => {
                    for t in node.sms_threads(id, *limit).await.context("can't list conversations")? {
                        let who: Vec<String> = t
                            .addresses
                            .iter()
                            .zip(t.names.iter().map(Some).chain(std::iter::repeat(None)))
                            .map(|(a, n)| match n.filter(|n| !n.is_empty()) {
                                Some(n) => format!("{n} ({a})"),
                                None => a.clone(),
                            })
                            .collect();
                        println!(
                            "[{}] {}{}: {}{}",
                            t.id,
                            who.join(", "),
                            if t.unread > 0 { format!(" · {} unread", t.unread) } else { String::new() },
                            t.snippet,
                            if t.photo.is_some() { " (photo)" } else { "" }
                        );
                    }
                }
                SmsArg::Show { thread, limit } => {
                    let mut messages =
                        node.sms_messages(id, thread.clone(), None, *limit).await.context("can't read it")?;
                    messages.reverse();
                    for m in messages {
                        println!(
                            "{} {}: {}{}{}",
                            if m.outgoing { "→" } else { "←" },
                            m.address,
                            m.body,
                            m.parts.iter().map(|p| format!(" [{} {}]", p.mime, p.id)).collect::<String>(),
                            m.status.map(|s| format!(" ({s})")).unwrap_or_default()
                        );
                    }
                }
                SmsArg::Send { to, body } => {
                    node.sms_send(id, vec![to.clone()], body.clone()).await.context("not sent")?;
                    println!("Sent.");
                }
            }
        }
        Command::Calls { auto, script } => {
            let mut offers = cli.offers.clone();
            offers.push(nectarlink_core::CALLS_SHOW.into());
            node.update_power(node_power(cli), offers).await;
            let command = auto.map(|a| match a {
                CallArg::Answer => nectarlink_core::CallCommand::Answer,
                CallArg::Decline => nectarlink_core::CallCommand::Decline,
                CallArg::Silence => nectarlink_core::CallCommand::Silence,
            });
            let mut events = node.events();
            let answerer = node.clone();
            let mut script = script.clone();
            tokio::spawn(async move {
                while let Ok(event) = events.recv().await {
                    if let (Some(steps), NodeEvent::Call { device, call }) = (script.as_ref(), &event)
                        && call.state == "ringing"
                        && call.incoming
                    {
                        use nectarlink_core::CallCommand as C;
                        for step in steps.split(',').map(str::trim) {
                            let command = match step {
                                "answer" => C::Answer,
                                "decline" | "end" => C::Decline,
                                "silence" => C::Silence,
                                "mute" => C::Mute(true),
                                "unmute" => C::Mute(false),
                                "speaker" => C::Speaker(true),
                                "earpiece" => C::Speaker(false),
                                "hold" => C::Hold(true),
                                "unhold" => C::Hold(false),
                                "up" => C::Volume(true),
                                "down" => C::Volume(false),
                                s if s.starts_with("wait:") => {
                                    let secs = s[5..].parse().unwrap_or(1);
                                    tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                                    continue;
                                }
                                s if s.len() == 1 => C::Dtmf(s.chars().next().unwrap_or('0')),
                                other => {
                                    println!("Unknown step {other}");
                                    continue;
                                }
                            };
                            match answerer.call_command(*device, call.id.clone(), command).await {
                                Ok(()) => println!("Did it: {command:?}."),
                                Err(e) => println!("Couldn't {command:?}: {e}"),
                            }
                        }
                        script = None;
                    }
                    if let (Some(command), NodeEvent::Call { device, call }) = (command, &event)
                        && call.state == "ringing"
                        && call.incoming
                    {
                        // Long enough to see it ring.
                        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                        match answerer.call_command(*device, call.id.clone(), command).await {
                            Ok(()) => println!("Did it: {command:?}."),
                            Err(e) => println!("Couldn't {command:?}: {e}"),
                        }
                    }
                }
            });
            watch(node, false).await?;
        }
        Command::Power { device, action } => {
            let id = resolve(node, device)?;
            wait_until_online(node, id).await?;
            let action = match action {
                PowerArg::Lock => PowerAction::Lock,
                PowerArg::Sleep => PowerAction::Sleep,
            };
            node.pc_power(id, action).await.context("the PC didn't do it")?;
            println!("Done.");
        }
        Command::Open { device, url } => {
            let id = resolve(node, device)?;
            wait_until_online(node, id).await?;
            node.open_link(id, url.clone()).await.context("the link didn't open")?;
            println!("Opened.");
        }
        Command::Media { device, action, player, position } => {
            let id = resolve(node, device)?;
            wait_until_online(node, id).await?;
            let player = match player {
                Some(player) => player.clone(),
                None => first_player(node, id).await?,
            };
            node.media_command(id, player.clone(), (*action).into(), position.map(|s| s * 1000))
                .await
                .context("media command failed")?;
            println!("Done ({player}).");
        }
        Command::Play { title, artist, app, length, art } => {
            let mut offers = cli.offers.clone();
            offers.push("media.control".into());
            node.update_power(node_power(cli), offers).await;
            play(node, title, artist, app, *length, art.as_deref()).await?;
        }
        Command::Photo { path, preview } => {
            if !cli.as_phone {
                bail!("photos come from phones: add --as-phone");
            }
            let thumb =
                preview.as_deref().map(std::fs::read).transpose().context("can't read the preview")?;
            let size = std::fs::metadata(path).context("can't read the picture")?.len();
            let _ = PHOTO.set(path.clone());
            let mut offers = cli.offers.clone();
            offers.push(nectarlink_core::PHOTOS_READ.into());
            node.update_power(node_power(cli), offers).await;
            // Only connected PCs hear of it.
            let first = node.paired_devices()?.first().map(|d| d.id).context("pair with a PC first")?;
            wait_until_online(node, first).await?;
            let name = nectarlink_core::safe_file_name(&path.to_string_lossy());
            let taken = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs() as i64;
            node.photo_taken(nectarlink_core::Photo {
                id: PHOTO_ID.into(),
                screenshot: name.to_ascii_lowercase().contains("screenshot"),
                name,
                size,
                taken,
                thumb: thumb.unwrap_or_default(),
            })
            .await
            .context("can't announce it")?;
            println!("Announced to connected PCs that show photos.");
            watch(node, false).await?;
        }
        Command::Notify { title, text, app, reply, silent, image, also } => {
            let image = image.as_deref().map(std::fs::read).transpose().context("can't read the picture")?;
            if !cli.as_phone {
                bail!("notifications come from phones: add --as-phone");
            }
            let mut offers = cli.offers.clone();
            offers.extend(["notify.mirror".to_owned(), "notify.reply".to_owned()]);
            node.update_power(node_power(cli), offers).await;
            let when = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis() as i64;
            let mut actions =
                vec![NotificationAction { id: "read".into(), title: "Mark as read".into(), reply: false }];
            if *reply {
                actions
                    .insert(0, NotificationAction { id: "reply".into(), title: "Reply".into(), reply: true });
            }
            node.notification_posted(Notification {
                key: format!("cli|{when}"),
                app: "dev.nectarlink.cli".into(),
                app_name: app.clone(),
                title: Some(title.clone()),
                text: Some(text.clone()),
                sub: None,
                when,
                actions,
                silent: *silent,
                icon: None,
                image,
            })
            .await;
            for (i, extra) in also.iter().enumerate() {
                let mut parts = extra.splitn(3, '|');
                let (Some(app), Some(title), Some(text)) = (parts.next(), parts.next(), parts.next()) else {
                    bail!("--also takes \"App|Title|Text\"");
                };
                node.notification_posted(Notification {
                    key: format!("cli|{when}|{i}"),
                    app: format!("dev.nectarlink.cli.{i}"),
                    app_name: app.into(),
                    title: Some(title.into()),
                    text: Some(text.into()),
                    sub: None,
                    when: when - 60_000 * (i as i64 + 1),
                    actions: Vec::new(),
                    silent: true,
                    icon: None,
                    image: None,
                })
                .await;
            }
            println!("Notification sent to connected PCs (and to others when they connect).");
            watch(node, false).await?;
        }
    }
    Ok(())
}

async fn pair(node: &Node) -> Result<()> {
    let mut events = node.events();
    let link = node.pairing_start_qr().await?;
    let uri = link.to_uri();
    print_qr(&uri);
    println!("\nScan this code in the Nectarlink app, or run on another machine:");
    println!("  nectarlink join \"{uri}\"\n");
    println!("Waiting for a device (pairing mode ends in 5 minutes)…");
    loop {
        match events.recv().await {
            Ok(NodeEvent::Pairing(PairingEvent::Paired(d))) => {
                println!("✓ Paired with {}.", d.info.name);
                return Ok(());
            }
            Ok(NodeEvent::Pairing(PairingEvent::SasCode { code, .. })) => {
                node.pairing_confirm(confirm_code(code).await?)?;
            }
            Ok(NodeEvent::Pairing(PairingEvent::Failed(why))) => bail!("pairing failed: {why:?}"),
            Ok(_) | Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => bail!("node stopped"),
        }
    }
}

async fn nearby(node: &Node, peer: DeviceId) -> Result<()> {
    let mut events = node.events();
    let mut task = {
        let node = node.clone();
        tokio::spawn(async move { node.pairing_start_nearby(peer).await })
    };
    loop {
        tokio::select! {
            result = &mut task => {
                result.context("pairing task failed")?.context("pairing failed")?;
                println!("✓ Paired.");
                return Ok(());
            }
            event = events.recv() => match event {
                Ok(NodeEvent::Pairing(PairingEvent::SasCode { code, .. })) => {
                    node.pairing_confirm(confirm_code(code).await?)?;
                }
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => bail!("node stopped"),
            }
        }
    }
}

/// Asks the user to compare the 6-digit code (stdin is read off the runtime).
async fn confirm_code(code: String) -> Result<bool> {
    tokio::task::spawn_blocking(move || -> Result<bool> {
        print!("Does the other device show {} {}? [y/N] ", &code[..3], &code[3..]);
        std::io::stdout().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        Ok(matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes"))
    })
    .await?
}

/// Sends files and shows progress until they've arrived.
async fn send_files(node: &Node, device: DeviceId, paths: &[PathBuf]) -> Result<()> {
    let mut events = node.events();
    let files = nectarlink_core::outgoing_paths(paths).context("can't read what to send")?;
    let id = node.send_files(device, files).await.context("can't send")?;
    loop {
        match events.recv().await {
            Ok(NodeEvent::Transfer(t)) if t.id == id => match t.state {
                TransferState::Done { .. } => {
                    println!("\rSent {} file(s), {} bytes.          ", t.files, t.total);
                    return Ok(());
                }
                TransferState::Failed(why) => bail!("transfer failed: {why:?}"),
                TransferState::Cancelled => bail!("transfer cancelled"),
                TransferState::Waiting => print!("\rWaiting for the device…          "),
                TransferState::Running => {
                    let percent = (t.done * 100).checked_div(t.total).unwrap_or(100);
                    print!("\rSending… {percent}%          ");
                }
            },
            Ok(_) | Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => bail!("node stopped"),
        }
        std::io::stdout().flush()?;
    }
}

/// "3:07".
fn clock(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

/// The first player a device reports (it reports them when connecting).
async fn first_player(node: &Node, device: DeviceId) -> Result<String> {
    let mut events = node.events();
    // Ask again in case the report came before we listened.
    node.set_device_toggle(device, "media", true)?;
    let wait = async {
        loop {
            if let Ok(NodeEvent::MediaChanged { device: d, players }) = events.recv().await
                && d == device
                && let Some(p) = players.first()
            {
                return p.id.clone();
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(10), wait).await.context("nothing is playing on that device")
}

/// A pretend player that follows the commands it gets.
async fn play(
    node: &Node,
    title: &str,
    artist: &str,
    app: &str,
    length: u64,
    art: Option<&std::path::Path>,
) -> Result<()> {
    let (sender, mut commands) = tokio::sync::mpsc::unbounded_channel();
    let _ = MEDIA_COMMANDS.set(sender);
    let art = art.map(std::fs::read).transpose().context("can't read the artwork")?;
    let art_key = art.as_ref().map(|a| format!("{:x}", a.len()));
    let length = length * 1000;
    let mut track = 1u32;
    let mut position = 0u64;
    let mut playing = true;
    let mut since = std::time::Instant::now();
    let player = |track: u32, position: u64, playing: bool| MediaPlayer {
        id: "cli.player".into(),
        app: app.into(),
        title: Some(if track == 1 { title.to_owned() } else { format!("{title} ({track})") }),
        artist: Some(artist.into()),
        album: Some("Nectarlink Test Album".into()),
        playing,
        duration: Some(length),
        position: Some(position),
        actions: ["play", "pause", "next", "previous", "seek"].map(String::from).to_vec(),
        art_key: art_key.clone(),
        art: art.clone(),
    };
    node.media_changed(vec![player(track, position, playing)]).await;
    println!("Playing \"{title}\". Paired devices can control it; Ctrl+C to stop.");
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                node.media_changed(Vec::new()).await;
                return Ok(());
            }
            Some((action, to)) = commands.recv() => {
                if playing {
                    position = (position + since.elapsed().as_millis() as u64).min(length);
                }
                since = std::time::Instant::now();
                match action {
                    MediaAction::Play => playing = true,
                    MediaAction::Pause => playing = false,
                    MediaAction::Next => { track += 1; position = 0; }
                    MediaAction::Previous => { track = track.saturating_sub(1).max(1); position = 0; }
                    MediaAction::Seek => position = to.unwrap_or(position).min(length),
                }
                node.media_changed(vec![player(track, position, playing)]).await;
            }
        }
    }
}

/// Prints events until Ctrl+C; with `fetch_photos`, also asks for each new photo.
async fn watch(node: &Node, fetch_photos: bool) -> Result<()> {
    println!("Online as {}. Press Ctrl+C to stop.", node.device_id());
    for d in node.paired_devices()? {
        print_device(&d);
    }
    let mut events = node.events();
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => return Ok(()),
            event = events.recv() => match event {
                Ok(event) => {
                    print_event(node, &event);
                    if let (true, NodeEvent::PhotoAdded { device, photo }) = (fetch_photos, &event) {
                        let (node, device, id) = (node.clone(), *device, photo.id.clone());
                        tokio::spawn(async move {
                            if let Err(e) = node.fetch_photo(device, id).await {
                                println!("Couldn't fetch it: {e}");
                            }
                        });
                    }
                }
                Err(RecvError::Lagged(n)) => println!("(missed {n} events)"),
                Err(RecvError::Closed) => return Ok(()),
            }
        }
    }
}

fn print_event(node: &Node, event: &NodeEvent) {
    let name = |id: &DeviceId| {
        node.paired_devices()
            .ok()
            .and_then(|ds| ds.into_iter().find(|d| d.id == *id).map(|d| d.info.name))
            .unwrap_or_else(|| id.short())
    };
    match event {
        NodeEvent::LinkChanged { device, link } => println!("{}: {}", name(device), describe_link(link)),
        NodeEvent::Battery { device, battery } => println!(
            "{}: battery {}%{}",
            name(device),
            battery.level,
            if battery.charging { " (charging)" } else { "" }
        ),
        NodeEvent::Ring { device, on } => {
            println!("{}: {}", name(device), if *on { "asked us to ring" } else { "stopped ringing" })
        }
        NodeEvent::DeviceAdded(d) => println!("Paired with {}", d.info.name),
        NodeEvent::DeviceRemoved(id) => println!("{} unpaired", id.short()),
        NodeEvent::PeerInfoChanged { device, info } => {
            println!("{} is now called {}", device.short(), info.name)
        }
        NodeEvent::Discovered(d) => {
            println!("Found nearby: {} ({})", d.name.as_deref().unwrap_or("unnamed"), d.id.short())
        }
        NodeEvent::Capabilities(m) => {
            let available = m.features.values().filter(|s| **s == FeatureState::Available).count();
            println!("{}: {available} of {} features available", name(&m.device), m.features.len())
        }
        NodeEvent::NotificationsReset { device, items } => {
            println!("{}: {} notifications showing", name(device), items.len())
        }
        NodeEvent::NotificationPosted { device, notification: n } => {
            println!(
                "{}: {} · {}{}  [key {}]",
                name(device),
                n.app_name,
                n.title.as_deref().or(n.text.as_deref()).unwrap_or_default(),
                n.image.as_ref().map(|i| format!(" (with a picture, {} bytes)", i.len())).unwrap_or_default(),
                n.key
            );
            for a in &n.actions {
                println!("    action {}: {}{}", a.id, a.title, if a.reply { " (reply)" } else { "" });
            }
        }
        NodeEvent::NotificationRemoved { device, .. } => {
            println!("{}: a notification went away", name(device))
        }
        NodeEvent::ClipboardReceived { device } => println!("{}: sent its clipboard", name(device)),
        NodeEvent::Call { device, call } => println!(
            "{}: {} call {} {}{}{}",
            name(device),
            if call.incoming { "incoming" } else { "outgoing" },
            call.id,
            call.state,
            match (&call.name, &call.number) {
                (Some(n), Some(num)) => format!(" from {n} ({num})"),
                (None, Some(num)) => format!(" from {num}"),
                _ => String::new(),
            },
            match (call.missed, call.photo.is_some(), call.controls) {
                (true, ..) => ", missed".to_owned(),
                (_, _, Some(c)) => format!(
                    " [muted {}, speaker {}, held {}, can hold {}]",
                    c.muted, c.speaker, c.held, c.can_hold
                ),
                (_, true, _) => ", with photo".to_owned(),
                _ => String::new(),
            },
        ),
        NodeEvent::PhotoAdded { device, photo } => println!(
            "{}: new {} {} ({} bytes, preview {} bytes)",
            name(device),
            if photo.screenshot { "screenshot" } else { "photo" },
            photo.name,
            photo.size,
            photo.thumb.len()
        ),
        NodeEvent::MediaChanged { device, players } if players.is_empty() => {
            println!("{}: nothing playing", name(device));
        }
        NodeEvent::MediaChanged { device, players } => {
            for p in players {
                println!(
                    "{}: {} {} – {} ({}, {} of {}{}) [player {}]",
                    name(device),
                    if p.playing { "▶" } else { "⏸" },
                    p.title.as_deref().unwrap_or("?"),
                    p.artist.as_deref().unwrap_or("?"),
                    p.app,
                    clock(p.position.unwrap_or(0)),
                    clock(p.duration.unwrap_or(0)),
                    if p.art.is_some() { ", with artwork" } else { "" },
                    p.id,
                );
            }
        }
        NodeEvent::Transfer(t) if t.direction == Direction::Incoming => match &t.state {
            TransferState::Done { saved } => {
                for path in saved {
                    println!("{}: received {}", name(&t.device), path.display());
                }
            }
            TransferState::Failed(why) => println!("{}: a transfer failed ({why:?})", name(&t.device)),
            TransferState::Cancelled => println!("{}: a transfer was cancelled", name(&t.device)),
            _ => {}
        },
        _ => {}
    }
}

fn print_capabilities(node: &Node, id: DeviceId) -> Result<()> {
    let matrix = node.capabilities(id)?;
    let mut group = None;
    for def in FEATURES {
        if group != Some(def.group) {
            group = Some(def.group);
            println!("\n{:?}", def.group);
        }
        let state = matrix.state(def.id).context("feature missing from the matrix")?;
        println!("  {:<30} {}", def.id, describe_state(&state));
    }
    Ok(())
}

fn describe_state(state: &FeatureState) -> String {
    match state {
        FeatureState::Available => "✓ available".into(),
        FeatureState::Partial { limit, upgrade: Some(u) } => {
            format!("◐ partial ({limit}); unlock: {}", describe_upgrade(u))
        }
        FeatureState::Partial { limit, upgrade: None } => format!("◐ partial ({limit})"),
        FeatureState::Locked { upgrade } => format!("🔒 {}", describe_upgrade(upgrade)),
        FeatureState::Unsupported { reason } => format!("✗ {}", describe_reason(reason)),
    }
}

fn describe_upgrade(upgrade: &Upgrade) -> String {
    let action = match upgrade.action {
        UpgradeAction::RaisePower(level) => format!("raise the phone to {}", power_name(level)),
        UpgradeAction::GrantPermission(p) => format!("grant the {} permission on the phone", p.as_str()),
        UpgradeAction::EnableAddon(addon) => format!("install the {addon} add-on"),
        UpgradeAction::EnablePath(ConnectionPath::Relay) => "turn on Away mode".into(),
        UpgradeAction::EnablePath(path) => format!("enable the {path:?} connection"),
        UpgradeAction::EnableDeviceToggle(t) => format!("allow {t} (nectarlink toggle <device> {t} on)"),
        UpgradeAction::UpdateApp(Role::Phone) => "update the app on the phone".into(),
        UpgradeAction::UpdateApp(Role::Desktop) => "update the app on the PC".into(),
    };
    match upgrade.effort {
        Effort::Instant => action,
        Effort::Minutes(n) => format!("{action} · ~{n} min"),
    }
}

fn describe_reason(reason: &UnsupportedReason) -> String {
    match reason {
        UnsupportedReason::DeviceKinds => "needs a phone paired with a PC".into(),
        UnsupportedReason::AndroidTooOld { needs } => format!("needs Android {needs} or newer"),
        UnsupportedReason::WindowsTooOld { needs_build } => {
            format!("needs Windows build {needs_build} or newer")
        }
        UnsupportedReason::NotOnThisDevice => "not available on this phone".into(),
    }
}

fn power_name(level: PowerLevel) -> &'static str {
    match level {
        PowerLevel::Assist => "Assist",
        PowerLevel::Elevated => "Elevated",
        _ => "Basic",
    }
}

fn print_device(d: &PairedDevice) {
    println!("{:<24} {:<10} {}  {}", d.info.name, format!("{:?}", d.info.kind), describe_link(&d.link), d.id);
}

fn describe_link(link: &LinkState) -> String {
    match link {
        LinkState::Online { path, rtt_ms } => format!("online ({path:?}, {rtt_ms} ms)"),
        LinkState::Connecting => "connecting".into(),
        LinkState::Offline { .. } => "offline".into(),
    }
}

/// Accepts a full device ID, or a unique prefix / name of a paired device.
fn resolve(node: &Node, query: &str) -> Result<DeviceId> {
    if let Ok(id) = query.parse::<DeviceId>() {
        return Ok(id);
    }
    let q = query.to_ascii_lowercase();
    let matches: Vec<_> = node
        .paired_devices()?
        .into_iter()
        .filter(|d| d.id.to_string().starts_with(&q) || d.info.name.to_ascii_lowercase() == q)
        .collect();
    match matches.as_slice() {
        [one] => Ok(one.id),
        [] => bail!("no paired device matches {query:?}"),
        _ => bail!("{query:?} matches more than one device; use more of the ID"),
    }
}

fn parse_id(s: &str) -> Result<DeviceId> {
    s.parse().with_context(|| format!("{s:?} is not a device ID"))
}

async fn wait_until_online(node: &Node, id: DeviceId) -> Result<()> {
    let mut events = node.events();
    let is_online = |node: &Node| {
        node.paired_devices()
            .map(|ds| ds.iter().any(|d| d.id == id && matches!(d.link, LinkState::Online { .. })))
            .unwrap_or(false)
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while !is_online(node) {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(_) => {}
            Err(_) => bail!("the device didn't come online within 20 seconds"),
        }
    }
    Ok(())
}

fn print_qr(data: &str) {
    match qrcode::QrCode::new(data.as_bytes()) {
        Ok(code) => {
            let image = code
                .render::<qrcode::render::unicode::Dense1x2>()
                .dark_color(qrcode::render::unicode::Dense1x2::Light)
                .light_color(qrcode::render::unicode::Dense1x2::Dark)
                .quiet_zone(true)
                .build();
            println!("{image}");
        }
        Err(e) => tracing::warn!(error = %e, "could not render QR code"),
    }
}

/// Splits an Annex B stream at its access unit delimiters (a 4-byte start
/// code, then NAL type 9).
fn access_units(stream: &[u8]) -> Vec<Vec<u8>> {
    let starts: Vec<usize> =
        (0..stream.len()).filter(|&i| stream[i..].starts_with(&[0, 0, 0, 1, 9])).collect();
    starts
        .iter()
        .enumerate()
        .map(|(n, &s)| stream[s..*starts.get(n + 1).unwrap_or(&stream.len())].to_vec())
        .collect()
}

/// Whether an access unit holds an IDR picture (NAL type 5).
fn is_keyframe(unit: &[u8]) -> bool {
    unit.windows(4).any(|w| w[..3] == [0, 0, 1] && w[3] & 0x1f == 5)
}

/// Streams `units` in a loop at `fps` until the PC stops watching.
fn stream_screen(
    mirror: &nectarlink_core::MirrorStream,
    units: &[Vec<u8>],
    width: u32,
    height: u32,
    fps: u32,
    session: u32,
) {
    use nectarlink_core::{MirrorSend, PacketKind};
    let config = nectarlink_core::MirrorConfig { codec: "h264".into(), width, height, session }.to_cbor();
    if mirror.send(PacketKind::Config, 0, config) == MirrorSend::Closed {
        return;
    }
    let frame = std::time::Duration::from_secs_f64(1.0 / f64::from(fps.max(1)));
    let started = std::time::Instant::now();
    let mut dropped = 0u32;
    for (n, unit) in units.iter().cycle().enumerate() {
        let due = started + frame * n as u32;
        if let Some(wait) = due.checked_duration_since(std::time::Instant::now()) {
            std::thread::sleep(wait);
        }
        let kind = if is_keyframe(unit) { PacketKind::Keyframe } else { PacketKind::Frame };
        let time = started.elapsed().as_micros() as u64;
        match mirror.send(kind, time, unit.clone()) {
            MirrorSend::Queued => {}
            MirrorSend::NeedKeyframe | MirrorSend::Dropped => dropped += 1,
            MirrorSend::Closed => break,
        }
    }
    println!("Stopped sharing the screen ({dropped} frames dropped to keep up).");
}
