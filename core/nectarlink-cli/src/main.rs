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
    /// Where received files and recordings are saved.
    #[arg(long, global = true)]
    downloads_dir: Option<PathBuf>,
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
    /// Show a paired phone's quick settings, or change one with `toggles <device> set <id> <value>`.
    Toggles {
        device: String,
        #[command(subcommand)]
        action: Option<TogglesArg>,
    },
    /// Stay online and print events until Ctrl+C.
    Run,
    /// Send files to a paired device and wait until they've arrived.
    Send {
        device: String,
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Send a voice recording with optional markers to a paired PC (use with
    /// --as-phone) and wait until it has arrived.
    Record {
        device: String,
        /// Duration in seconds when generating a test recording.
        #[arg(long, default_value_t = 3)]
        duration: u32,
        /// Existing audio file (.m4a) to send instead of generating one.
        #[arg(long)]
        file: Option<PathBuf>,
        /// Timestamped marker in milliseconds, optionally with a label:
        /// `<ms>` or `<ms>:<label>` (repeatable, e.g. `--marker 1200:Intro`).
        #[arg(long = "marker", value_name = "MS[:LABEL]")]
        markers: Vec<String>,
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
    /// Browse a paired phone's gallery (albums, photos, thumbnails, download),
    /// or with no arguments watch for new photos from paired phones until Ctrl+C.
    Photos {
        /// Paired phone (omit to watch for new photos from any phone).
        device: Option<String>,
        #[command(subcommand)]
        action: Option<PhotosArg>,
    },
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
        /// Resize the app window ("WxH", with --app) 2 s after the first frame.
        #[arg(long)]
        resize: Option<String>,
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
    /// Control a paired PC's mouse, keyboard or presentation (use with --as-phone).
    Remote {
        device: String,
        #[command(subcommand)]
        action: RemoteArg,
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
    /// List a paired phone's recent calls, or call a number with `call <device> <number>`.
    Call {
        device: String,
        /// Number to call (omit to list recent calls).
        number: Option<String>,
        /// Max recent calls to list when no number is given.
        #[arg(long, default_value_t = 30)]
        limit: u32,
    },
    /// List or search a paired phone's contacts.
    Contacts {
        device: String,
        /// Search by name or number.
        query: Option<String>,
        #[arg(long, default_value_t = 0)]
        offset: u32,
        #[arg(long, default_value_t = 50)]
        limit: u32,
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
enum RemoteArg {
    /// Move the PC's mouse cursor by (dx, dy) pixels.
    Move {
        #[arg(allow_hyphen_values = true)]
        dx: f32,
        #[arg(allow_hyphen_values = true)]
        dy: f32,
    },
    /// Click, press (--down) or release (--up) a mouse button.
    Click {
        #[arg(value_enum, default_value_t = MouseButtonArg::Left)]
        button: MouseButtonArg,
        /// Press and hold the button down.
        #[arg(long, conflicts_with = "up")]
        down: bool,
        /// Release the button.
        #[arg(long, conflicts_with = "down")]
        up: bool,
    },
    /// Scroll the mouse wheel by (dx, dy) notches (positive dy scrolls down).
    Scroll {
        #[arg(allow_hyphen_values = true)]
        dx: f32,
        #[arg(allow_hyphen_values = true)]
        dy: f32,
    },
    /// Type Unicode text on the PC.
    Type { text: String },
    /// Press a named key or shortcut (enter, backspace, tab, escape, space,
    /// up, down, left, right, copy, paste, undo, ...) with optional modifiers.
    Key {
        key: String,
        #[arg(long)]
        ctrl: bool,
        #[arg(long)]
        alt: bool,
        #[arg(long)]
        shift: bool,
        #[arg(long)]
        win: bool,
    },
    /// Control a presentation on the PC (next, previous, start, stop, black).
    Slide {
        #[arg(value_enum)]
        action: SlideArg,
    },
    /// Show the laser pointer at normalized (x, y) in 0..=1, or hide it with --off.
    Laser {
        #[arg(required_unless_present = "off")]
        x: Option<f32>,
        #[arg(required_unless_present = "off")]
        y: Option<f32>,
        /// Hide the laser pointer overlay.
        #[arg(long)]
        off: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum MouseButtonArg {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum SlideArg {
    Next,
    #[value(alias = "prev")]
    Previous,
    Start,
    Stop,
    Black,
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

#[derive(Debug, Subcommand)]
enum PhotosArg {
    /// List photo and video albums on the phone.
    Albums,
    /// List photos and videos on the phone (newest first).
    List {
        /// Only items in this album ID.
        #[arg(long, allow_hyphen_values = true)]
        album: Option<String>,
        /// Only items after this one (the previous page's last), given as
        /// `<date ms>:<id>`, or `<date ms>` for items older than that.
        #[arg(long, value_parser = parse_photo_cursor)]
        before: Option<(i64, String)>,
        /// Maximum number of items to list.
        #[arg(long, default_value_t = 30)]
        limit: u32,
    },
    /// Fetch thumbnails for one or more item IDs.
    Thumbs {
        #[arg(required = true)]
        ids: Vec<String>,
        /// Directory to save `<id>.jpg` files into (default: current directory).
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Download one or more full-size photos or videos into Downloads\Nectarlink.
    Get {
        #[arg(required = true)]
        ids: Vec<String>,
    },
}

/// `--before <date ms>[:<id>]` for `photos list`.
fn parse_photo_cursor(s: &str) -> std::result::Result<(i64, String), String> {
    let (date, id) = s.split_once(':').unwrap_or((s, ""));
    let date = date.parse().map_err(|_| format!("not a date in ms: {date}"))?;
    Ok((date, id.to_owned()))
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

#[derive(Debug, Subcommand)]
enum TogglesArg {
    /// Change one of the phone's quick settings (`dnd`, `ringer`, `flashlight`,
    /// `volume`, `brightness`, `wifi`, `bluetooth`).
    Set { id: String, value: String },
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
/// Active `screen` sessions: (session, pending resize (width, height), stream).
type ScreenSession = (
    u32,
    std::sync::Arc<std::sync::Mutex<Option<(u32, u32)>>>,
    std::sync::Arc<nectarlink_core::MirrorStream>,
);
static SCREEN_SESSIONS: std::sync::Mutex<Vec<ScreenSession>> = std::sync::Mutex::new(Vec::new());
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

static CONTACTS: std::sync::Mutex<Vec<nectarlink_core::Contact>> = std::sync::Mutex::new(Vec::new());
static CALL_LOG: std::sync::Mutex<Vec<nectarlink_core::CallLogEntry>> = std::sync::Mutex::new(Vec::new());

fn sample_contacts() {
    use nectarlink_core::{Contact, ContactNumber};
    let num = |number: &str, label: &str| ContactNumber {
        number: number.into(),
        label: (!label.is_empty()).then(|| label.into()),
    };
    *CONTACTS.lock().unwrap() = vec![
        Contact {
            id: "1".into(),
            name: "Sam Rivera".into(),
            numbers: vec![num("+15550100", "Mobile"), num("+15550144", "Work")],
            starred: true,
            photo: None,
        },
        Contact {
            id: "2".into(),
            name: "Alex Chen".into(),
            numbers: vec![num("+15550188", "Mobile")],
            starred: true,
            photo: None,
        },
        Contact {
            id: "3".into(),
            name: "Mom".into(),
            numbers: vec![num("+15550199", "Mobile")],
            starred: true,
            photo: None,
        },
        Contact {
            id: "4".into(),
            name: "City Bakery".into(),
            numbers: vec![num("+15550130", "Work")],
            starred: false,
            photo: None,
        },
        Contact {
            id: "5".into(),
            name: "Jordan Patel".into(),
            numbers: vec![num("+15550155", "Mobile"), num("+15550156", "Home")],
            starred: false,
            photo: None,
        },
        Contact {
            id: "6".into(),
            name: "Priya Nair".into(),
            numbers: vec![num("+15550172", "Mobile")],
            starred: false,
            photo: None,
        },
    ];
}

fn sample_call_log() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    let min = 60_000;
    let entry = |id: &str, number: &str, name: Option<&str>, direction: &str, date: i64, duration: u32| {
        nectarlink_core::CallLogEntry {
            id: id.into(),
            number: number.into(),
            name: name.map(Into::into),
            direction: direction.into(),
            date,
            duration,
            photo: None,
        }
    };
    *CALL_LOG.lock().unwrap() = vec![
        entry("call:1", "+15550100", Some("Sam Rivera"), "missed", now - 18 * min, 0),
        entry("call:2", "+15550188", Some("Alex Chen"), "incoming", now - 2 * 60 * min, 254),
        entry("call:3", "+15550199", Some("Mom"), "outgoing", now - 4 * 60 * min, 612),
        entry("call:4", "+15550123", None, "missed", now - 25 * 60 * min, 0),
        entry("call:5", "+15550155", Some("Jordan Patel"), "outgoing", now - 28 * 60 * min, 95),
        entry("call:6", "+15550190", None, "rejected", now - 30 * 60 * min, 0),
        entry("call:7", "+15550172", Some("Priya Nair"), "incoming", now - 3 * 24 * 60 * min, 180),
    ];
}

fn contact_name_for(number: &str) -> Option<String> {
    let digits: String = number.chars().filter(char::is_ascii_digit).collect();
    CONTACTS.lock().unwrap().iter().find_map(|c| {
        c.numbers
            .iter()
            .any(|n| {
                n.number == number || {
                    let nd: String = n.number.chars().filter(char::is_ascii_digit).collect();
                    !digits.is_empty() && nd == digits
                }
            })
            .then(|| c.name.clone())
    })
}

/// The call `incoming-call` pretends to have, and how to tell PCs it changed.
static CALL: std::sync::Mutex<Option<nectarlink_core::CallState>> = std::sync::Mutex::new(None);
static CALL_NODE: std::sync::OnceLock<Node> = std::sync::OnceLock::new();
/// Whether `incoming-call` offers in-call controls (without --basic).
static CALL_CONTROLS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Sample quick settings for a test phone (`demo` or `--as-phone`).
static TOGGLES: std::sync::Mutex<Option<nectarlink_core::PhoneToggles>> = std::sync::Mutex::new(None);
static TOGGLES_NODE: std::sync::OnceLock<Node> = std::sync::OnceLock::new();

fn sample_toggles() -> nectarlink_core::PhoneToggles {
    let mut lock = TOGGLES.lock().unwrap();
    lock.get_or_insert_with(|| nectarlink_core::PhoneToggles {
        dnd: false,
        ringer: "ring".into(),
        flashlight: Some(false),
        volume: 60,
        brightness: 70,
        wifi: true,
        bluetooth: true,
    })
    .clone()
}

/// The picture `photo` announced, for PCs that ask for it.
static PHOTO: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
const PHOTO_ID: &str = "cli-photo";

struct SampleGalleryItem {
    item: nectarlink_core::PhotoItem,
    thumb_jpeg: Vec<u8>,
    full_path: PathBuf,
}

static GALLERY_ALBUMS: std::sync::Mutex<Vec<nectarlink_core::PhotoAlbum>> = std::sync::Mutex::new(Vec::new());
static GALLERY_ITEMS: std::sync::Mutex<Vec<SampleGalleryItem>> = std::sync::Mutex::new(Vec::new());

/// Encodes a baseline JFIF JPEG (width and height rounded to multiples of 8)
/// with a smooth vertical gradient between `top` and `bottom` RGB and a bright
/// accent circle.
fn sample_jpeg(
    width: u16,
    height: u16,
    top: (u8, u8, u8),
    bottom: (u8, u8, u8),
    sun: (u8, u8, u8),
) -> Vec<u8> {
    let bw = (width.max(8) / 8) as usize;
    let bh = (height.max(8) / 8) as usize;
    let (w, h) = ((bw * 8) as u16, (bh * 8) as u16);

    let mut out = Vec::with_capacity(4096);
    // SOI + APP0 (JFIF 1.01)
    out.extend_from_slice(&[
        0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0x00, 0x01, 0x01, 0x00, 0x00, 0x01, 0x00,
        0x01, 0x00, 0x00,
    ]);
    // DQT: table 0, 8-bit precision, all 64 entries = 8 so quantized DC == (level - 128).
    out.extend_from_slice(&[0xFF, 0xDB, 0x00, 0x43, 0x00]);
    out.extend_from_slice(&[8u8; 64]);
    // SOF0: baseline DCT, 8-bit, 3 components (Y=1, Cb=2, Cr=3) at 1x1 sampling.
    out.extend_from_slice(&[
        0xFF,
        0xC0,
        0x00,
        0x11,
        0x08,
        (h >> 8) as u8,
        (h & 0xFF) as u8,
        (w >> 8) as u8,
        (w & 0xFF) as u8,
        0x03,
        0x01,
        0x11,
        0x00,
        0x02,
        0x11,
        0x00,
        0x03,
        0x11,
        0x00,
    ]);
    // DHT DC table 0: 12 symbols (0..=11), each 4 bits long (codes 0000..=1011).
    out.extend_from_slice(&[0xFF, 0xC4, 0x00, 0x1F, 0x00]);
    out.extend_from_slice(&[0, 0, 0, 12, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    out.extend_from_slice(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
    // DHT AC table 0: 1 symbol (0x00 = EOB), 1 bit long (code 0).
    out.extend_from_slice(&[0xFF, 0xC4, 0x00, 0x14, 0x10]);
    out.extend_from_slice(&[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    out.push(0x00);
    // SOS: 3 components, all using DC table 0 / AC table 0.
    out.extend_from_slice(&[
        0xFF, 0xDA, 0x00, 0x0C, 0x03, 0x01, 0x00, 0x02, 0x00, 0x03, 0x00, 0x00, 0x3F, 0x00,
    ]);

    let mut bit_buf = 0u32;
    let mut bit_count = 0u32;
    fn push_bits(bit_buf: &mut u32, bit_count: &mut u32, bits: u16, len: u32, out: &mut Vec<u8>) {
        *bit_buf = (*bit_buf << len) | u32::from(bits & ((1 << len) - 1));
        *bit_count += len;
        while *bit_count >= 8 {
            *bit_count -= 8;
            let byte = ((*bit_buf >> *bit_count) & 0xFF) as u8;
            out.push(byte);
            if byte == 0xFF {
                out.push(0x00);
            }
        }
    }

    let mut prev_dc = [0i32; 3];
    let sun_x = (bw as f32) * 0.68;
    let sun_y = (bh as f32) * 0.30;
    let sun_r2 = ((bh.min(bw) as f32) * 0.18).powi(2).max(1.0);

    for by in 0..bh {
        let t = (by as f32) / ((bh.max(2) - 1) as f32);
        let horizon = t > 0.62;
        for bx in 0..bw {
            let dx = (bx as f32) - sun_x;
            let dy = (by as f32) - sun_y;
            let in_sun = dx * dx + dy * dy <= sun_r2;
            let (r, g, b) = if in_sun {
                sun
            } else if horizon {
                let ht = (t - 0.62) / 0.38;
                (
                    (f32::from(bottom.0) * (1.0 - 0.25 * ht)) as u8,
                    (f32::from(bottom.1) * (1.0 - 0.25 * ht)) as u8,
                    (f32::from(bottom.2) * (1.0 - 0.25 * ht)) as u8,
                )
            } else {
                let st = t / 0.62;
                (
                    (f32::from(top.0) * (1.0 - st) + f32::from(bottom.0) * st) as u8,
                    (f32::from(top.1) * (1.0 - st) + f32::from(bottom.1) * st) as u8,
                    (f32::from(top.2) * (1.0 - st) + f32::from(bottom.2) * st) as u8,
                )
            };
            let (rf, gf, bf) = (f32::from(r), f32::from(g), f32::from(b));
            let y = (0.299 * rf + 0.587 * gf + 0.114 * bf).round().clamp(0.0, 255.0) as i32 - 128;
            let cb =
                (128.0 - 0.168736 * rf - 0.331264 * gf + 0.5 * bf).round().clamp(0.0, 255.0) as i32 - 128;
            let cr =
                (128.0 + 0.5 * rf - 0.418688 * gf - 0.081312 * bf).round().clamp(0.0, 255.0) as i32 - 128;

            for (comp, dc) in [y, cb, cr].into_iter().enumerate() {
                let diff = dc - prev_dc[comp];
                prev_dc[comp] = dc;
                if diff == 0 {
                    // DC category 0 (4 bits: 0000) + AC EOB (1 bit: 0)
                    push_bits(&mut bit_buf, &mut bit_count, 0, 5, &mut out);
                } else {
                    let abs = diff.unsigned_abs();
                    let cat = 32 - abs.leading_zeros();
                    let mag = if diff > 0 { diff as u16 } else { ((1u32 << cat) - 1 - abs) as u16 };
                    push_bits(&mut bit_buf, &mut bit_count, cat as u16, 4, &mut out);
                    push_bits(&mut bit_buf, &mut bit_count, mag, cat, &mut out);
                    // AC EOB (1 bit: 0)
                    push_bits(&mut bit_buf, &mut bit_count, 0, 1, &mut out);
                }
            }
        }
    }
    if bit_count > 0 {
        let pad = 8 - bit_count;
        push_bits(&mut bit_buf, &mut bit_count, (1u16 << pad) - 1, pad, &mut out);
    }
    // EOI
    out.extend_from_slice(&[0xFF, 0xD9]);
    out
}

fn sample_photos() {
    use nectarlink_core::{PhotoAlbum, PhotoItem};
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    let min = 60_000i64;
    let hour = 60 * min;

    let specs = [
        (
            "media:demo-1",
            "IMG_20260406_184210.jpg",
            now - 25 * min,
            1920,
            1440,
            None,
            "bucket:camera",
            (255, 140, 82),
            (180, 62, 92),
            (255, 232, 160),
        ),
        (
            "media:demo-2",
            "IMG_20260406_161504.jpg",
            now - 2 * hour,
            1920,
            1440,
            None,
            "bucket:camera",
            (72, 158, 235),
            (26, 94, 150),
            (255, 246, 210),
        ),
        (
            "media:demo-5",
            "Screenshot_20260406_142011.jpg",
            now - 3 * hour,
            1080,
            2400,
            None,
            "bucket:screenshots",
            (46, 64, 87),
            (28, 40, 56),
            (110, 212, 196),
        ),
        (
            "video:demo-1",
            "VID_20260406_123000.mp4",
            now - 4 * hour,
            1920,
            1080,
            Some(18_000),
            "bucket:camera",
            (88, 178, 128),
            (38, 102, 68),
            (242, 248, 185),
        ),
        (
            "media:demo-3",
            "IMG_20260405_191200.jpg",
            now - 26 * hour,
            1920,
            1440,
            None,
            "bucket:camera",
            (118, 92, 196),
            (218, 112, 118),
            (255, 224, 152),
        ),
        (
            "media:demo-4",
            "IMG_20260405_154022.jpg",
            now - 29 * hour,
            1920,
            1440,
            None,
            "bucket:camera",
            (64, 188, 206),
            (28, 106, 128),
            (240, 250, 255),
        ),
        (
            "media:demo-6",
            "Screenshot_20260405_110509.jpg",
            now - 32 * hour,
            1080,
            2400,
            None,
            "bucket:screenshots",
            (238, 228, 212),
            (196, 180, 158),
            (226, 118, 76),
        ),
        (
            "media:demo-7",
            "IMG_20260405_093015.jpg",
            now - 35 * hour,
            1920,
            1440,
            None,
            "bucket:trips",
            (142, 182, 214),
            (76, 118, 104),
            (255, 240, 198),
        ),
        (
            "media:demo-8",
            "IMG_20260403_174500.jpg",
            now - 74 * hour,
            1920,
            1440,
            None,
            "bucket:trips",
            (232, 146, 102),
            (164, 74, 48),
            (255, 228, 168),
        ),
        (
            "media:demo-9",
            "IMG_20260403_141130.jpg",
            now - 77 * hour,
            1920,
            1440,
            None,
            "bucket:trips",
            (94, 186, 148),
            (36, 114, 82),
            (232, 250, 200),
        ),
        (
            "media:demo-10",
            "IMG_20260403_112045.jpg",
            now - 80 * hour,
            1920,
            1440,
            None,
            "bucket:trips",
            (102, 152, 224),
            (54, 88, 156),
            (255, 236, 184),
        ),
        (
            "media:demo-11",
            "IMG_20260403_085012.jpg",
            now - 82 * hour,
            1920,
            1440,
            None,
            "bucket:camera",
            (198, 148, 108),
            (116, 74, 48),
            (250, 222, 182),
        ),
    ];

    let temp_dir = std::env::temp_dir().join("nectarlink-demo-photos");
    let _ = std::fs::create_dir_all(&temp_dir);
    let mut items = Vec::with_capacity(specs.len());
    for (id, name, date, w, h, duration, album, top, bottom, sun) in specs {
        let thumb_jpeg = sample_jpeg(240, 180, top, bottom, sun);
        let full_bytes = sample_jpeg(640, 480, top, bottom, sun);
        let size = full_bytes.len() as u64;
        let full_path = temp_dir.join(name);
        let _ = std::fs::write(&full_path, &full_bytes);
        items.push(SampleGalleryItem {
            item: PhotoItem {
                id: id.into(),
                name: name.into(),
                date,
                size,
                width: w,
                height: h,
                duration,
                album: Some(album.into()),
            },
            thumb_jpeg,
            full_path,
        });
    }
    *GALLERY_ITEMS.lock().unwrap() = items;
    *GALLERY_ALBUMS.lock().unwrap() = vec![
        PhotoAlbum {
            id: "bucket:camera".into(),
            name: "Camera".into(),
            count: 6,
            cover: Some("media:demo-1".into()),
        },
        PhotoAlbum {
            id: "bucket:screenshots".into(),
            name: "Screenshots".into(),
            count: 2,
            cover: Some("media:demo-5".into()),
        },
        PhotoAlbum {
            id: "bucket:trips".into(),
            name: "Weekend Trip".into(),
            count: 4,
            cover: Some("media:demo-7".into()),
        },
    ];
}

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
        let mut sessions = SCREEN_SESSIONS.lock().unwrap();
        sessions.retain(|(s, _, stream)| {
            if *s == session {
                stream.close();
                false
            } else {
                true
            }
        });
    }
    fn mirror_resize_requested(&self, _peer: &DeviceId, session: u32, width: u32, height: u32) {
        let (w, h) = ((width & !1).max(64), (height & !1).max(64));
        println!("The PC resized session {session} to {w}x{h}.");
        for (s, pending, _) in SCREEN_SESSIONS.lock().unwrap().iter() {
            if *s == session {
                *pending.lock().unwrap() = Some((w, h));
            }
        }
    }
    fn mirror_input(&self, _peer: &DeviceId, session: u32, input: nectarlink_core::MirrorInput) {
        println!("Input on session {session}: {input:?}");
        if session != 0 && matches!(&input, nectarlink_core::MirrorInput::Key { key } if key == "home") {
            let mut sessions = SCREEN_SESSIONS.lock().unwrap();
            sessions.retain(|(s, _, stream)| {
                if *s == session {
                    stream.close();
                    false
                } else {
                    true
                }
            });
        }
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
        let name = contact_name_for(&to[0]).unwrap_or_default();
        let mut texts = TEXTS.lock().unwrap();
        let thread = texts.iter().find(|(_, n, ..)| *n == to[0]).map(|(t, ..)| *t);
        let thread = thread.unwrap_or_else(|| texts.iter().map(|(t, ..)| *t).max().unwrap_or(0) + 1);
        texts.push((thread, to[0].clone(), name, false, body.to_owned(), now));
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
    fn call_log(
        &self,
        before: Option<i64>,
        limit: u32,
    ) -> std::result::Result<Vec<nectarlink_core::CallLogEntry>, String> {
        let mut entries: Vec<nectarlink_core::CallLogEntry> =
            CALL_LOG.lock().unwrap().iter().filter(|e| before.is_none_or(|b| e.date < b)).cloned().collect();
        entries.sort_by_key(|e| std::cmp::Reverse(e.date));
        entries.truncate(limit as usize);
        Ok(entries)
    }
    fn call_dial(&self, number: &str) -> std::result::Result<(), String> {
        println!("A PC started a call to {number}.");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let name = contact_name_for(number);
        let call = nectarlink_core::CallState {
            id: now.to_string(),
            state: "active".into(),
            incoming: false,
            number: Some(number.into()),
            name: name.clone(),
            photo: None,
            missed: false,
            since: Some(now),
            controls: CALL_CONTROLS
                .load(std::sync::atomic::Ordering::Relaxed)
                .then(|| nectarlink_core::CallControls { can_hold: true, ..Default::default() }),
        };
        *CALL.lock().unwrap() = Some(call.clone());
        CALL_LOG.lock().unwrap().push(nectarlink_core::CallLogEntry {
            id: format!("call:{now}"),
            number: number.into(),
            name,
            direction: "outgoing".into(),
            date: now,
            duration: 0,
            photo: None,
        });
        if let Some(node) = CALL_NODE.get().cloned() {
            tokio::runtime::Handle::current().spawn(async move {
                let _ = node.call_changed(call).await;
                node.call_log_changed().await;
            });
        }
        Ok(())
    }
    fn contacts(
        &self,
        query: Option<&str>,
        offset: u32,
        limit: u32,
    ) -> std::result::Result<Vec<nectarlink_core::Contact>, String> {
        let q = query.map(str::trim).filter(|q| !q.is_empty()).map(str::to_lowercase);
        let q_digits: String =
            q.as_deref().map(|s| s.chars().filter(char::is_ascii_digit).collect()).unwrap_or_default();
        let mut items: Vec<nectarlink_core::Contact> = CONTACTS
            .lock()
            .unwrap()
            .iter()
            .filter(|c| match &q {
                None => true,
                Some(q) => {
                    c.name.to_lowercase().contains(q)
                        || c.numbers.iter().any(|n| {
                            n.number.to_lowercase().contains(q)
                                || (!q_digits.is_empty()
                                    && n.number
                                        .chars()
                                        .filter(char::is_ascii_digit)
                                        .collect::<String>()
                                        .contains(&q_digits))
                        })
                }
            })
            .cloned()
            .collect();
        items.sort_by_key(|a| (!a.starred, a.name.to_lowercase()));
        Ok(items.into_iter().skip(offset as usize).take(limit as usize).collect())
    }
    fn photo_albums(&self) -> std::result::Result<Vec<nectarlink_core::PhotoAlbum>, String> {
        Ok(GALLERY_ALBUMS.lock().unwrap().clone())
    }
    fn photo_list(
        &self,
        album: Option<&str>,
        before: Option<(i64, &str)>,
        limit: u32,
    ) -> std::result::Result<Vec<nectarlink_core::PhotoItem>, String> {
        let mut items: Vec<nectarlink_core::PhotoItem> = GALLERY_ITEMS
            .lock()
            .unwrap()
            .iter()
            .filter(|entry| {
                album.is_none_or(|a| entry.item.album.as_deref() == Some(a))
                    && before.is_none_or(|(date, id)| (entry.item.date, entry.item.id.as_str()) < (date, id))
            })
            .map(|entry| entry.item.clone())
            .collect();
        items.sort_by(|a, b| (b.date, &b.id).cmp(&(a.date, &a.id)));
        items.truncate(limit as usize);
        Ok(items)
    }
    fn photo_thumbs(&self, ids: &[String]) -> std::result::Result<Vec<nectarlink_core::PhotoThumb>, String> {
        let items = GALLERY_ITEMS.lock().unwrap();
        Ok(ids
            .iter()
            .filter_map(|id| {
                let entry = items.iter().find(|e| e.item.id == *id)?;
                Some(nectarlink_core::PhotoThumb { id: id.clone(), data: entry.thumb_jpeg.clone() })
            })
            .collect())
    }
    fn open_photo(&self, id: &str) -> std::result::Result<nectarlink_core::OutgoingFile, String> {
        if id == PHOTO_ID
            && let Some(path) = PHOTO.get()
        {
            println!("A PC asked for the photo; sending it.");
            return Ok(nectarlink_core::OutgoingFile {
                name: nectarlink_core::safe_file_name(&path.to_string_lossy()),
                folder: None,
                source: nectarlink_core::FileSource::Path(path.clone()),
            });
        }
        let items = GALLERY_ITEMS.lock().unwrap();
        let entry = items.iter().find(|e| e.item.id == id).ok_or("no such photo")?;
        println!("A PC asked for gallery item {id}; sending it.");
        Ok(nectarlink_core::OutgoingFile {
            name: entry.item.name.clone(),
            folder: None,
            source: nectarlink_core::FileSource::Path(entry.full_path.clone()),
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
    fn remote_input(&self, _peer: &DeviceId, input: nectarlink_core::RemoteInput) -> Result<(), String> {
        println!("Remote input: {input:?}");
        Ok(())
    }
    fn set_phone_toggle(
        &self,
        id: &str,
        value: &nectarlink_core::PhoneToggleValue,
    ) -> std::result::Result<(), String> {
        use nectarlink_core::PhoneToggleValue as V;
        let mut t = sample_toggles();
        match (id, value) {
            ("dnd", V::Bool(on)) => {
                t.dnd = *on;
                println!("Phone DND is now {}.", if *on { "on" } else { "off" });
            }
            ("ringer", V::Mode(mode)) => {
                t.ringer = mode.clone();
                println!("Phone ringer is now {mode}.");
            }
            ("flashlight", V::Bool(on)) => {
                t.flashlight = Some(*on);
                println!("Phone flashlight is now {}.", if *on { "on" } else { "off" });
            }
            ("volume", V::Level(level)) => {
                t.volume = *level;
                println!("Phone volume is now {level}%.");
            }
            ("brightness", V::Level(level)) => {
                t.brightness = *level;
                println!("Phone brightness is now {level}%.");
            }
            ("wifi", V::Bool(on)) => {
                t.wifi = *on;
                println!("Phone Wi-Fi is now {}.", if *on { "on" } else { "off" });
            }
            ("bluetooth", V::Bool(on)) => {
                t.bluetooth = *on;
                println!("Phone Bluetooth is now {}.", if *on { "on" } else { "off" });
            }
            _ => return Err("invalid toggle".into()),
        }
        *TOGGLES.lock().unwrap() = Some(t.clone());
        if let Some(node) = TOGGLES_NODE.get().cloned() {
            tokio::runtime::Handle::current().spawn(async move {
                let _ = node.toggles_changed(t).await;
            });
        }
        Ok(())
    }
}

fn main() -> Result<()> {
    std::thread::Builder::new()
        .name("main".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread().enable_all().build()?.block_on(async {
                let cli = Cli::parse();
                tracing_subscriber::fmt()
                    .with_env_filter(
                        tracing_subscriber::EnvFilter::try_new(&cli.log).unwrap_or_else(|_| "warn".into()),
                    )
                    .with_writer(std::io::stderr)
                    .init();

                let node = Box::pin(start_node(&cli)).await?;
                let result = Box::pin(run(&cli, &node)).await;
                node.shutdown().await;
                result
            })
        })?
        .join()
        .unwrap_or_else(|_| bail!("main thread panicked"))
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
    config.downloads_dir = cli.downloads_dir.clone();
    config.port = cli.port;
    config.lan_discovery = !cli.no_lan;
    config.away_mode = cli.away;
    config.power = power;
    if !cli.as_phone {
        config.capabilities.push(nectarlink_core::RECORDER.into());
        config.capabilities.push(nectarlink_core::TOGGLES_SHOW.into());
    }
    let extra_caps = config.capabilities.clone();
    let node = Node::start(config, Arc::new(TerminalPlatform)).await.context("failed to start")?;
    if cli.as_phone {
        let _ = TOGGLES_NODE.set(node.clone());
        let _ = node.toggles_changed(sample_toggles()).await;
    }
    if !cli.offers.is_empty() {
        let mut offers = extra_caps;
        for o in &cli.offers {
            if !offers.contains(o) {
                offers.push(o.clone());
            }
        }
        node.update_power(power, offers).await;
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
        Command::Record { device, duration, file, markers } => {
            if !cli.as_phone {
                bail!("voice recordings are sent by phones: add --as-phone");
            }
            let parsed_markers: Vec<nectarlink_core::RecordingMarker> =
                markers.iter().map(|s| parse_recording_marker(s)).collect::<Result<Vec<_>>>()?;
            let id = resolve(node, device)?;
            wait_until_online(node, id).await?;
            send_recording(node, id, *duration, file.as_deref(), parsed_markers).await?;
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
        Command::Toggles { device, action: None } => {
            let mut events = node.events();
            let id = resolve(node, device)?;
            wait_until_online(node, id).await?;
            let toggles = match node.phone_toggles(id) {
                Some(t) => t,
                None => {
                    let wait = async {
                        loop {
                            if let Ok(NodeEvent::PhoneToggles { device: d, toggles }) = events.recv().await
                                && d == id
                            {
                                return toggles;
                            }
                        }
                    };
                    tokio::time::timeout(Duration::from_secs(5), wait)
                        .await
                        .context("the phone didn't report its quick settings")?
                }
            };
            print_phone_toggles(node, id, &toggles)?;
        }
        Command::Toggles { device, action: Some(TogglesArg::Set { id: toggle_id, value }) } => {
            let parsed = parse_phone_toggle_value(toggle_id, value)?;
            let id = resolve(node, device)?;
            wait_until_online(node, id).await?;
            node.set_phone_toggle(id, toggle_id.clone(), parsed)
                .await
                .with_context(|| format!("couldn't set {toggle_id}"))?;
            println!("{toggle_id} set to {value}.");
        }
        Command::Run => watch(node, false).await?,
        Command::Photos { device: None, .. } => {
            let mut offers = cli.offers.clone();
            offers.push(nectarlink_core::PHOTOS_SHOW.into());
            node.update_power(node_power(cli), offers).await;
            watch(node, true).await?;
        }
        Command::Photos { device: Some(device), action } => {
            let mut offers = cli.offers.clone();
            offers.push(nectarlink_core::PHOTOS_SHOW.into());
            node.update_power(node_power(cli), offers).await;
            let id = resolve(node, device)?;
            wait_until_online(node, id).await?;
            match action.as_ref().unwrap_or(&PhotosArg::List { album: None, before: None, limit: 30 }) {
                PhotosArg::Albums => {
                    for a in node.photo_albums(id).await.context("can't list albums")? {
                        println!(
                            "{:<24} {:>5}  {} (cover {})",
                            a.id,
                            a.count,
                            a.name,
                            a.cover.as_deref().unwrap_or("-")
                        );
                    }
                }
                PhotosArg::List { album, before, limit } => {
                    for item in node
                        .photo_list(id, album.clone(), before.clone(), *limit)
                        .await
                        .context("can't list photos")?
                    {
                        let dim = if item.width > 0 && item.height > 0 {
                            format!(" {}x{}", item.width, item.height)
                        } else {
                            String::new()
                        };
                        let dur = item
                            .duration
                            .map(|ms| format!(" video {}", clock(u64::from(ms))))
                            .unwrap_or_default();
                        let alb = item.album.map(|a| format!(" [{a}]")).unwrap_or_default();
                        println!(
                            "{:<18} {:>9} B  {}  {}{}{}{}",
                            item.id, item.size, item.date, item.name, dim, dur, alb
                        );
                    }
                }
                PhotosArg::Thumbs { ids, out } => {
                    let dir = out.clone().unwrap_or_else(|| PathBuf::from("."));
                    std::fs::create_dir_all(&dir).context("can't create output directory")?;
                    let thumbs =
                        node.photo_thumbs(id, ids.clone()).await.context("can't fetch thumbnails")?;
                    for t in thumbs {
                        let safe = t.id.replace(':', "-");
                        let path = dir.join(format!("{safe}.jpg"));
                        std::fs::write(&path, &t.data).context("can't write thumbnail")?;
                        println!("{} ({} B) -> {}", t.id, t.data.len(), path.display());
                    }
                }
                PhotosArg::Get { ids } => {
                    let mut events = node.events();
                    let transfer_id =
                        node.fetch_photos(id, ids.clone()).await.context("can't download photos")?;
                    loop {
                        match events.recv().await {
                            Ok(NodeEvent::Transfer(t)) if t.id == transfer_id => match t.state {
                                TransferState::Done { saved } => {
                                    for path in saved {
                                        println!("{}", path.display());
                                    }
                                    break;
                                }
                                TransferState::Failed(why) => bail!("download failed: {why:?}"),
                                TransferState::Cancelled => bail!("download cancelled"),
                                _ => {}
                            },
                            Ok(_) | Err(RecvError::Lagged(_)) => {}
                            Err(RecvError::Closed) => bail!("node stopped"),
                        }
                    }
                }
            }
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
        Command::Remote { device, action } => {
            use nectarlink_core::{ButtonAction, KeyMod, MouseButton, RemoteInput, SlideAction};
            if !cli.as_phone {
                bail!("remote input is sent by phones: add --as-phone");
            }
            let id = resolve(node, device)?;
            wait_until_online(node, id).await?;
            match action {
                RemoteArg::Move { dx, dy } => {
                    node.remote_check(id).await.context("the PC refused remote input")?;
                    node.remote_move(id, *dx, *dy).await.context("move not sent")?;
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
                RemoteArg::Click { button, down, up } => {
                    let button = match button {
                        MouseButtonArg::Left => MouseButton::Left,
                        MouseButtonArg::Right => MouseButton::Right,
                        MouseButtonArg::Middle => MouseButton::Middle,
                    };
                    let action = if *down {
                        ButtonAction::Down
                    } else if *up {
                        ButtonAction::Up
                    } else {
                        ButtonAction::Click
                    };
                    node.remote_input(id, RemoteInput::Button { button, action })
                        .await
                        .context("the PC refused remote input")?;
                }
                RemoteArg::Scroll { dx, dy } => {
                    node.remote_input(id, RemoteInput::Scroll { dx: *dx, dy: *dy })
                        .await
                        .context("the PC refused remote input")?;
                }
                RemoteArg::Type { text } => {
                    node.remote_input(id, RemoteInput::Text { text: text.clone() })
                        .await
                        .context("the PC refused remote input")?;
                }
                RemoteArg::Key { key, ctrl, alt, shift, win } => {
                    let mut mods = Vec::new();
                    if *ctrl {
                        mods.push(KeyMod::Ctrl);
                    }
                    if *alt {
                        mods.push(KeyMod::Alt);
                    }
                    if *shift {
                        mods.push(KeyMod::Shift);
                    }
                    if *win {
                        mods.push(KeyMod::Win);
                    }
                    node.remote_input(id, RemoteInput::Key { key: key.clone(), mods })
                        .await
                        .context("the PC refused remote input")?;
                }
                RemoteArg::Slide { action } => {
                    let action = match action {
                        SlideArg::Next => SlideAction::Next,
                        SlideArg::Previous => SlideAction::Previous,
                        SlideArg::Start => SlideAction::Start,
                        SlideArg::Stop => SlideAction::Stop,
                        SlideArg::Black => SlideAction::Black,
                    };
                    node.remote_input(id, RemoteInput::Slide { action })
                        .await
                        .context("the PC refused remote input")?;
                }
                RemoteArg::Laser { x, y, off } => {
                    if *off {
                        node.remote_laser(id, false, 0.0, 0.0)
                            .await
                            .context("the PC refused remote input")?;
                    } else {
                        let (x, y) = (x.unwrap_or(0.5), y.unwrap_or(0.5));
                        node.remote_check(id).await.context("the PC refused remote input")?;
                        node.remote_laser(id, true, x, y).await.context("laser not sent")?;
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                }
            }
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
        Command::Mirror { device, out, seconds, at, sound, app, tap, resize } => {
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
            if let Some(dim) = resize {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                let (w, h) = dim.split_once('x').context("--resize takes WxH")?;
                let (w, h): (u32, u32) = (w.trim().parse()?, h.trim().parse()?);
                node.mirror_resize(id, session, w, h).await.context("resize not sent")?;
                println!("Resized to {w}x{h}.");
            }
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
            let _ = tokio::time::timeout(left, async {
                loop {
                    if let Ok(NodeEvent::Mirroring { session: s, on: false, .. }) = events.recv().await
                        && s == session
                    {
                        break;
                    }
                }
            })
            .await;
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
                    let pending = std::sync::Arc::new(std::sync::Mutex::new(None));
                    {
                        let mut sessions = SCREEN_SESSIONS.lock().unwrap();
                        sessions.retain(|(s, _, old)| {
                            if *s == session {
                                old.close();
                                false
                            } else {
                                true
                            }
                        });
                        sessions.push((session, pending.clone(), mirror.clone()));
                    }
                    let units = units.clone();
                    std::thread::spawn(move || {
                        stream_screen(&mirror, &units, width, height, fps, session, &pending)
                    });
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
            sample_contacts();
            sample_call_log();
            let _ = TEXTS_NODE.set(node.clone());
            let _ = CALL_NODE.set(node.clone());
            CALL_CONTROLS.store(true, std::sync::atomic::Ordering::Relaxed);
            let mut offers = cli.offers.clone();
            offers.extend(
                [
                    nectarlink_core::SMS_READ,
                    nectarlink_core::SMS_SEND,
                    nectarlink_core::CALLS_STATE,
                    nectarlink_core::CALLS_CONTROL,
                    nectarlink_core::CALLS_IN_CALL,
                    nectarlink_core::CALLS_LOG,
                    nectarlink_core::CALLS_DIAL,
                    nectarlink_core::CONTACTS_READ,
                ]
                .map(str::to_owned),
            );
            node.update_power(node_power(cli), offers).await;
            println!("Sharing sample conversations, contacts, and calls with paired PCs.");
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
            sample_contacts();
            sample_call_log();
            sample_photos();
            let _ = TEXTS_NODE.set(node.clone());
            let _ = CALL_NODE.set(node.clone());
            let _ = TOGGLES_NODE.set(node.clone());
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
                    nectarlink_core::CALLS_LOG,
                    nectarlink_core::CALLS_DIAL,
                    nectarlink_core::CONTACTS_READ,
                    nectarlink_core::PHOTOS_READ,
                    nectarlink_core::TOGGLES_READ,
                    nectarlink_core::TOGGLES_RINGER,
                    nectarlink_core::TOGGLES_VOLUME,
                    nectarlink_core::TOGGLES_FLASHLIGHT,
                    nectarlink_core::TOGGLES_DND,
                    nectarlink_core::TOGGLES_BRIGHTNESS,
                ]
                .map(str::to_owned),
            );
            if node_power(cli) == PowerLevel::Elevated {
                offers.extend(
                    [nectarlink_core::TOGGLES_WIFI, nectarlink_core::TOGGLES_BLUETOOTH].map(str::to_owned),
                );
            }
            node.update_power(node_power(cli), offers).await;
            let _ = node.toggles_changed(sample_toggles()).await;
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
        Command::Call { device, number, limit } => {
            let mut offers = cli.offers.clone();
            offers.push(nectarlink_core::CALLS_SHOW.into());
            node.update_power(node_power(cli), offers).await;
            let id = resolve(node, device)?;
            wait_until_online(node, id).await?;
            match number {
                Some(number) => {
                    node.call_dial(id, number.clone()).await.context("couldn't start the call")?;
                    println!("Calling {number}…");
                }
                None => {
                    for e in node.call_log(id, None, *limit).await.context("can't read recent calls")? {
                        let arrow = match e.direction.as_str() {
                            "incoming" => "↙",
                            "outgoing" => "↗",
                            "missed" => "✕",
                            "rejected" => "⊘",
                            _ => "•",
                        };
                        let who = match &e.name {
                            Some(n) if !n.is_empty() => format!("{n} ({})", e.number),
                            _ => e.number.clone(),
                        };
                        let dur = if e.duration > 0 {
                            format!(" · {}", clock(u64::from(e.duration) * 1000))
                        } else {
                            String::new()
                        };
                        println!(
                            "{arrow} {} {who}{dur}{}",
                            e.direction,
                            if e.photo.is_some() { " (photo)" } else { "" }
                        );
                    }
                }
            }
        }
        Command::Contacts { device, query, offset, limit } => {
            let mut offers = cli.offers.clone();
            offers.push(nectarlink_core::CONTACTS_SHOW.into());
            node.update_power(node_power(cli), offers).await;
            let id = resolve(node, device)?;
            wait_until_online(node, id).await?;
            for c in node.contacts(id, query.clone(), *offset, *limit).await.context("can't list contacts")? {
                let nums: Vec<String> = c
                    .numbers
                    .iter()
                    .map(|n| match &n.label {
                        Some(l) if !l.is_empty() => format!("{} ({l})", n.number),
                        _ => n.number.clone(),
                    })
                    .collect();
                println!(
                    "{}{} · {}{}",
                    if c.starred { "★ " } else { "  " },
                    c.name,
                    nums.join(", "),
                    if c.photo.is_some() { " (photo)" } else { "" }
                );
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
                // Let the pairing close frame flush and the first session
                // connect so both sides record addresses and capabilities.
                let _ = tokio::time::timeout(Duration::from_secs(3), wait_until_online(node, d.id)).await;
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
    wait_for_outgoing_transfer(&mut events, &id).await
}

fn parse_recording_marker(s: &str) -> Result<nectarlink_core::RecordingMarker> {
    let (ms_str, label) = match s.split_once(':') {
        Some((ms, l)) => (ms, Some(l.trim()).filter(|l| !l.is_empty()).map(str::to_owned)),
        None => (s, None),
    };
    let at_ms: u64 =
        ms_str.trim().parse().with_context(|| format!("invalid marker timestamp in ms: {ms_str:?}"))?;
    Ok(nectarlink_core::RecordingMarker { at_ms, label })
}

async fn send_recording(
    node: &Node,
    device: DeviceId,
    duration_secs: u32,
    file: Option<&std::path::Path>,
    markers: Vec<nectarlink_core::RecordingMarker>,
) -> Result<()> {
    let mut events = node.events();
    let (name, path, cleanup) = match file {
        Some(p) => (nectarlink_core::safe_file_name(&p.to_string_lossy()), p.to_path_buf(), false),
        None => {
            let tmp = std::env::temp_dir().join(format!("nectarlink-recording-{}.m4a", std::process::id()));
            std::fs::write(&tmp, sample_m4a(duration_secs)).context("can't write temporary M4A recording")?;
            ("Recording.m4a".to_owned(), tmp, true)
        }
    };
    let outgoing = nectarlink_core::OutgoingFile {
        name,
        folder: None,
        source: nectarlink_core::FileSource::Path(path.clone()),
    };
    let marker_count = markers.len();
    let id = node.send_recording(device, outgoing, markers).await.context("can't send recording")?;
    let res = wait_for_outgoing_transfer(&mut events, &id).await;
    if cleanup {
        let _ = std::fs::remove_file(&path);
    }
    res?;
    if marker_count > 0 {
        println!("Sent voice recording with {marker_count} marker(s).");
    } else {
        println!("Sent voice recording.");
    }
    Ok(())
}

async fn wait_for_outgoing_transfer(
    events: &mut tokio::sync::broadcast::Receiver<NodeEvent>,
    id: &str,
) -> Result<()> {
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

/// Builds a self-contained 48 kHz mono AAC-LC `.m4a` (MP4) file of `duration_secs`.
fn sample_m4a(duration_secs: u32) -> Vec<u8> {
    // 1024-sample 48 kHz mono AAC-LC frame (ID_SCE + HCB_1 tone + ID_END).
    const AAC_FRAME: [u8; 6] = [0x01, 0x48, 0x00, 0x84, 0x21, 0x7E];
    const SAMPLE_RATE: u32 = 48_000;
    let num_frames = (duration_secs.max(1) * SAMPLE_RATE).div_ceil(1024).max(1);
    let total_samples = num_frames * 1024;

    fn mp4_box(fourcc: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let len = (8 + payload.len()) as u32;
        let mut out = Vec::with_capacity(len as usize);
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(fourcc);
        out.extend_from_slice(payload);
        out
    }

    fn full_box(fourcc: &[u8; 4], version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
        let mut payload = Vec::with_capacity(4 + body.len());
        payload.push(version);
        payload.extend_from_slice(&flags.to_be_bytes()[1..4]);
        payload.extend_from_slice(body);
        mp4_box(fourcc, &payload)
    }

    let mut ftyp_body = Vec::new();
    ftyp_body.extend_from_slice(b"M4A ");
    ftyp_body.extend_from_slice(&0u32.to_be_bytes());
    ftyp_body.extend_from_slice(b"M4A mp42isom\0\0\0\0");
    let ftyp = mp4_box(b"ftyp", &ftyp_body);

    let build_moov = |chunk_offset: u32| -> Vec<u8> {
        let mut mvhd = Vec::new();
        mvhd.extend_from_slice(&0u32.to_be_bytes()); // creation_time
        mvhd.extend_from_slice(&0u32.to_be_bytes()); // modification_time
        mvhd.extend_from_slice(&SAMPLE_RATE.to_be_bytes()); // timescale
        mvhd.extend_from_slice(&total_samples.to_be_bytes()); // duration
        mvhd.extend_from_slice(&0x0001_0000u32.to_be_bytes()); // rate 1.0
        mvhd.extend_from_slice(&0x0100u16.to_be_bytes()); // volume 1.0
        mvhd.extend_from_slice(&[0u8; 10]); // reserved
        for &m in &[0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000] {
            mvhd.extend_from_slice(&m.to_be_bytes());
        }
        mvhd.extend_from_slice(&[0u8; 24]); // pre_defined
        mvhd.extend_from_slice(&2u32.to_be_bytes()); // next_track_id
        let mvhd_box = full_box(b"mvhd", 0, 0, &mvhd);

        let mut tkhd = Vec::new();
        tkhd.extend_from_slice(&0u32.to_be_bytes()); // creation_time
        tkhd.extend_from_slice(&0u32.to_be_bytes()); // modification_time
        tkhd.extend_from_slice(&1u32.to_be_bytes()); // track_id
        tkhd.extend_from_slice(&0u32.to_be_bytes()); // reserved
        tkhd.extend_from_slice(&total_samples.to_be_bytes()); // duration
        tkhd.extend_from_slice(&[0u8; 8]); // reserved
        tkhd.extend_from_slice(&0u16.to_be_bytes()); // layer
        tkhd.extend_from_slice(&0u16.to_be_bytes()); // alternate_group
        tkhd.extend_from_slice(&0x0100u16.to_be_bytes()); // volume 1.0
        tkhd.extend_from_slice(&0u16.to_be_bytes()); // reserved
        for &m in &[0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000] {
            tkhd.extend_from_slice(&m.to_be_bytes());
        }
        tkhd.extend_from_slice(&0u32.to_be_bytes()); // width
        tkhd.extend_from_slice(&0u32.to_be_bytes()); // height
        let tkhd_box = full_box(b"tkhd", 0, 3, &tkhd);

        let mut mdhd = Vec::new();
        mdhd.extend_from_slice(&0u32.to_be_bytes()); // creation_time
        mdhd.extend_from_slice(&0u32.to_be_bytes()); // modification_time
        mdhd.extend_from_slice(&SAMPLE_RATE.to_be_bytes()); // timescale
        mdhd.extend_from_slice(&total_samples.to_be_bytes()); // duration
        mdhd.extend_from_slice(&0x55c4u16.to_be_bytes()); // "und"
        mdhd.extend_from_slice(&0u16.to_be_bytes()); // pre_defined
        let mdhd_box = full_box(b"mdhd", 0, 0, &mdhd);

        let mut hdlr = Vec::new();
        hdlr.extend_from_slice(&0u32.to_be_bytes()); // pre_defined
        hdlr.extend_from_slice(b"soun"); // handler_type
        hdlr.extend_from_slice(&[0u8; 12]); // reserved
        hdlr.extend_from_slice(b"SoundHandler\0");
        let hdlr_box = full_box(b"hdlr", 0, 0, &hdlr);

        let smhd_box = full_box(b"smhd", 0, 0, &[0u8; 4]);
        let url_box = full_box(b"url ", 0, 1, &[]);
        let mut dref = Vec::new();
        dref.extend_from_slice(&1u32.to_be_bytes());
        dref.extend_from_slice(&url_box);
        let dinf_box = mp4_box(b"dinf", &full_box(b"dref", 0, 0, &dref));

        // ESDescriptor for AAC-LC, 48000 Hz, mono (AudioSpecificConfig = [0x11, 0x88]).
        let esds_body: [u8; 27] = [
            0x03, 25, 0x00, 0x01, 0x00, // ES_Descriptor (ES_ID=1)
            0x04, 17, 0x40, 0x15, 0x00, 0x00, 0x00, // DecoderConfigDescriptor (AAC, audio)
            0x00, 0x01, 0xF4, 0x00, // maxBitrate = 128000
            0x00, 0x01, 0xF4, 0x00, // avgBitrate = 128000
            0x05, 2, 0x11, 0x88, // DecoderSpecificInfo (AAC-LC 48kHz mono)
            0x06, 1, 0x02, // SLConfigDescriptor
        ];
        let esds_box = full_box(b"esds", 0, 0, &esds_body);

        let mut mp4a = Vec::new();
        mp4a.extend_from_slice(&[0u8; 6]); // reserved
        mp4a.extend_from_slice(&1u16.to_be_bytes()); // data_reference_index
        mp4a.extend_from_slice(&[0u8; 8]); // reserved
        mp4a.extend_from_slice(&1u16.to_be_bytes()); // channelcount = 1
        mp4a.extend_from_slice(&16u16.to_be_bytes()); // samplesize = 16
        mp4a.extend_from_slice(&0u16.to_be_bytes()); // pre_defined
        mp4a.extend_from_slice(&0u16.to_be_bytes()); // reserved
        mp4a.extend_from_slice(&(SAMPLE_RATE << 16).to_be_bytes()); // samplerate 16.16
        mp4a.extend_from_slice(&esds_box);
        let mp4a_box = mp4_box(b"mp4a", &mp4a);

        let mut stsd = Vec::new();
        stsd.extend_from_slice(&1u32.to_be_bytes());
        stsd.extend_from_slice(&mp4a_box);
        let stsd_box = full_box(b"stsd", 0, 0, &stsd);

        let mut stts = Vec::new();
        stts.extend_from_slice(&1u32.to_be_bytes());
        stts.extend_from_slice(&num_frames.to_be_bytes());
        stts.extend_from_slice(&1024u32.to_be_bytes());
        let stts_box = full_box(b"stts", 0, 0, &stts);

        let mut stsc = Vec::new();
        stsc.extend_from_slice(&1u32.to_be_bytes());
        stsc.extend_from_slice(&1u32.to_be_bytes()); // first_chunk
        stsc.extend_from_slice(&num_frames.to_be_bytes()); // samples_per_chunk
        stsc.extend_from_slice(&1u32.to_be_bytes()); // sample_description_index
        let stsc_box = full_box(b"stsc", 0, 0, &stsc);

        let mut stsz = Vec::new();
        stsz.extend_from_slice(&(AAC_FRAME.len() as u32).to_be_bytes()); // constant sample_size
        stsz.extend_from_slice(&num_frames.to_be_bytes());
        let stsz_box = full_box(b"stsz", 0, 0, &stsz);

        let mut stco = Vec::new();
        stco.extend_from_slice(&1u32.to_be_bytes());
        stco.extend_from_slice(&chunk_offset.to_be_bytes());
        let stco_box = full_box(b"stco", 0, 0, &stco);

        let mut stbl_body = Vec::new();
        stbl_body.extend_from_slice(&stsd_box);
        stbl_body.extend_from_slice(&stts_box);
        stbl_body.extend_from_slice(&stsc_box);
        stbl_body.extend_from_slice(&stsz_box);
        stbl_body.extend_from_slice(&stco_box);
        let stbl_box = mp4_box(b"stbl", &stbl_body);

        let mut minf_body = Vec::new();
        minf_body.extend_from_slice(&smhd_box);
        minf_body.extend_from_slice(&dinf_box);
        minf_body.extend_from_slice(&stbl_box);
        let minf_box = mp4_box(b"minf", &minf_body);

        let mut mdia_body = Vec::new();
        mdia_body.extend_from_slice(&mdhd_box);
        mdia_body.extend_from_slice(&hdlr_box);
        mdia_body.extend_from_slice(&minf_box);
        let mdia_box = mp4_box(b"mdia", &mdia_body);

        let mut trak_body = Vec::new();
        trak_body.extend_from_slice(&tkhd_box);
        trak_body.extend_from_slice(&mdia_box);
        let trak_box = mp4_box(b"trak", &trak_body);

        let mut moov_body = Vec::new();
        moov_body.extend_from_slice(&mvhd_box);
        moov_body.extend_from_slice(&trak_box);
        mp4_box(b"moov", &moov_body)
    };

    let moov_len = build_moov(0).len();
    let chunk_offset = (ftyp.len() + moov_len + 8) as u32;
    let moov = build_moov(chunk_offset);
    let mdat_payload = AAC_FRAME.repeat(num_frames as usize);
    let mdat = mp4_box(b"mdat", &mdat_payload);

    let mut out = Vec::with_capacity(ftyp.len() + moov.len() + mdat.len());
    out.extend_from_slice(&ftyp);
    out.extend_from_slice(&moov);
    out.extend_from_slice(&mdat);
    out
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

fn format_marker_time(ms: u64) -> String {
    let total_secs = ms / 1000;
    let millis = ms % 1000;
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    format!("{mins:02}:{secs:02}.{millis:03}")
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
        NodeEvent::CallLogChanged { device } => println!("{}: call log changed", name(device)),
        NodeEvent::ContactsChanged { device } => println!("{}: contacts changed", name(device)),
        NodeEvent::PhotosChanged { device } => println!("{}: photo library changed", name(device)),
        NodeEvent::PhoneToggles { device, toggles } => {
            println!("{}: toggles {}", name(device), summarize_toggles(toggles));
        }
        NodeEvent::RemoteInputRequested { device } => println!(
            "{}: asked to control mouse and keyboard (allow with: nectarlink toggle {} remote_input on)",
            name(device),
            device.short()
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
                    if t.recording {
                        if !t.markers.is_empty() {
                            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Recording");
                            let markers_path = path.with_file_name(format!("{stem}.markers.txt"));
                            let mut text = String::new();
                            for (i, m) in t.markers.iter().enumerate() {
                                let label = m.label.as_deref().unwrap_or_default();
                                if label.is_empty() {
                                    text.push_str(&format!(
                                        "{}  Marker {}\n",
                                        format_marker_time(m.at_ms),
                                        i + 1
                                    ));
                                } else {
                                    text.push_str(&format!("{}  {label}\n", format_marker_time(m.at_ms)));
                                }
                            }
                            let _ = std::fs::write(&markers_path, text);
                        }
                        let markers_summary = if t.markers.is_empty() {
                            String::new()
                        } else {
                            let list: Vec<String> = t
                                .markers
                                .iter()
                                .enumerate()
                                .map(|(i, m)| match &m.label {
                                    Some(l) if !l.is_empty() => {
                                        format!("{} ({l})", format_marker_time(m.at_ms))
                                    }
                                    _ => format!("{} (Marker {})", format_marker_time(m.at_ms), i + 1),
                                })
                                .collect();
                            format!(" [markers: {}]", list.join(", "))
                        };
                        println!(
                            "{}: received recording {}{markers_summary}",
                            name(&t.device),
                            path.display()
                        );
                    } else {
                        println!("{}: received {}", name(&t.device), path.display());
                    }
                }
            }
            TransferState::Failed(why) => println!("{}: a transfer failed ({why:?})", name(&t.device)),
            TransferState::Cancelled => println!("{}: a transfer was cancelled", name(&t.device)),
            _ => {}
        },
        _ => {}
    }
}

fn summarize_toggles(t: &nectarlink_core::PhoneToggles) -> String {
    let yn = |b: bool| if b { "on" } else { "off" };
    let yn_opt = |b: Option<bool>| match b {
        Some(true) => "on",
        Some(false) => "off",
        None => "—",
    };
    format!(
        "dnd={} ringer={} flash={} vol={}% bright={}% wifi={} bt={}",
        yn(t.dnd),
        t.ringer,
        yn_opt(t.flashlight),
        t.volume,
        t.brightness,
        yn(t.wifi),
        yn(t.bluetooth),
    )
}

fn parse_phone_toggle_value(id: &str, raw: &str) -> Result<nectarlink_core::PhoneToggleValue> {
    use nectarlink_core::PhoneToggleValue as V;
    let s = raw.trim().to_ascii_lowercase();
    match id {
        "dnd" | "flashlight" | "wifi" | "bluetooth" => match s.as_str() {
            "on" | "true" | "1" => Ok(V::Bool(true)),
            "off" | "false" | "0" => Ok(V::Bool(false)),
            _ => bail!("{id} takes `on` or `off`"),
        },
        "ringer" => match s.as_str() {
            "ring" | "vibrate" | "silent" => Ok(V::Mode(s)),
            _ => bail!("ringer takes `ring`, `vibrate`, or `silent`"),
        },
        "volume" | "brightness" => {
            let level: u8 = s
                .trim_end_matches('%')
                .parse()
                .ok()
                .filter(|&v| v <= 100)
                .with_context(|| format!("{id} takes 0..=100"))?;
            Ok(V::Level(level))
        }
        _ => bail!(
            "unknown toggle {id:?}; expected dnd, ringer, flashlight, volume, brightness, wifi or bluetooth"
        ),
    }
}

fn print_phone_toggles(node: &Node, id: DeviceId, t: &nectarlink_core::PhoneToggles) -> Result<()> {
    let matrix = node.capabilities(id)?;
    let yn = |b: bool| if b { "on" } else { "off" }.to_owned();
    let yn_opt = |b: Option<bool>| match b {
        Some(true) => "on".to_owned(),
        Some(false) => "off".to_owned(),
        None => "—".to_owned(),
    };
    let rows = [
        ("dnd", yn(t.dnd), "toggles.dnd"),
        ("ringer", t.ringer.clone(), "toggles.ringer"),
        ("flashlight", yn_opt(t.flashlight), "toggles.flashlight"),
        ("volume", format!("{}%", t.volume), "toggles.volume"),
        ("brightness", format!("{}%", t.brightness), "toggles.brightness"),
        ("wifi", yn(t.wifi), "toggles.wifi"),
        ("bluetooth", yn(t.bluetooth), "toggles.bluetooth"),
    ];
    for (name, val, feat) in rows {
        let note = match matrix.state(feat) {
            Some(FeatureState::Available) => String::new(),
            Some(state) => format!("  ({})", describe_state(&state)),
            None => String::new(),
        };
        println!("{name:<12} {val:<10}{note}");
    }
    Ok(())
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
    pending: &std::sync::Mutex<Option<(u32, u32)>>,
) {
    use nectarlink_core::{MirrorSend, PacketKind};
    let config = nectarlink_core::MirrorConfig { codec: "h264".into(), width, height, session }.to_cbor();
    if mirror.send(PacketKind::Config, 0, config) == MirrorSend::Closed {
        return;
    }
    let keyframe = units.iter().find(|u| is_keyframe(u));
    let frame = std::time::Duration::from_secs_f64(1.0 / f64::from(fps.max(1)));
    let started = std::time::Instant::now();
    let mut dropped = 0u32;
    for (n, unit) in units.iter().cycle().enumerate() {
        let due = started + frame * n as u32;
        if let Some(wait) = due.checked_duration_since(std::time::Instant::now()) {
            std::thread::sleep(wait);
        }
        let time = started.elapsed().as_micros() as u64;
        if let Some((w, h)) = pending.lock().unwrap().take() {
            let cfg = nectarlink_core::MirrorConfig { codec: "h264".into(), width: w, height: h, session }
                .to_cbor();
            if mirror.send(PacketKind::Config, time, cfg) == MirrorSend::Closed {
                break;
            }
            if let Some(kf) = keyframe
                && mirror.send(PacketKind::Keyframe, time, kf.clone()) == MirrorSend::Closed
            {
                break;
            }
        }
        let kind = if is_keyframe(unit) { PacketKind::Keyframe } else { PacketKind::Frame };
        match mirror.send(kind, time, unit.clone()) {
            MirrorSend::Queued => {}
            MirrorSend::NeedKeyframe | MirrorSend::Dropped => dropped += 1,
            MirrorSend::Closed => break,
        }
    }
    println!("Stopped sharing the screen ({dropped} frames dropped to keep up).");
}
