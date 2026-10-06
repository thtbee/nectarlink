// SPDX-License-Identifier: GPL-3.0-or-later
//! `nectarlink`: a command-line client for development, testing and scripting.
//!
//! It runs its own node with its own identity (separate from the desktop
//! app), so it can pair with phones, PCs or another CLI instance.

use std::{io::Write, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use nectarlink_core::{
    Battery, ConnectionPath, DeviceId, DeviceInfo, DeviceKind, Direction, FeatureState, FileSource,
    LinkState, MediaAction, MediaError, MediaPlayer, Node, NodeConfig, NodeEvent, Notification,
    NotificationAction, NotificationError, OutgoingFile, PairedDevice, PairingEvent, Platform, PowerAction,
    PowerLevel, TransferState,
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
    /// Show a notification on paired PCs as if this were a phone (use with
    /// --as-phone), then stay online to show what the PC does with it.
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

#[derive(Debug, Clone, Copy, ValueEnum)]
enum OnOff {
    On,
    Off,
}

/// Rings by printing to the terminal (the CLI has no speaker access).
#[derive(Debug)]
struct TerminalPlatform;

impl Platform for TerminalPlatform {
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
        Command::Run => watch(node).await?,
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
        Command::Notify { title, text, app, reply, silent } => {
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
            })
            .await;
            println!("Notification sent to connected PCs (and to others when they connect).");
            watch(node).await?;
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
    let files = paths
        .iter()
        .map(|path| OutgoingFile {
            name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            source: FileSource::Path(path.clone()),
        })
        .collect();
    let id = node.send_files(device, files).await.context("can't send")?;
    loop {
        match events.recv().await {
            Ok(NodeEvent::Transfer(t)) if t.id == id => match t.state {
                TransferState::Done { .. } => {
                    println!("\rSent {} file(s), {} bytes.          ", t.names.len(), t.total);
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

async fn watch(node: &Node) -> Result<()> {
    println!("Online as {}. Press Ctrl+C to stop.", node.device_id());
    for d in node.paired_devices()? {
        print_device(&d);
    }
    let mut events = node.events();
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => return Ok(()),
            event = events.recv() => match event {
                Ok(event) => print_event(node, &event),
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
                "{}: {} · {}  [key {}]",
                name(device),
                n.app_name,
                n.title.as_deref().or(n.text.as_deref()).unwrap_or_default(),
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
