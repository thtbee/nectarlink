// SPDX-License-Identifier: MPL-2.0
//! Media control between devices (docs/protocol/media.md).
//!
//! A device that shares its players (`media.control`) keeps every allowed
//! device that shows them (`media.remote`) up to date: the full list when a
//! session starts or the user changes what that device may see, then after
//! each change. Artwork goes once per key and session. The other side turns
//! that into [`NodeEvent::MediaChanged`] and sends the user's commands back.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use iroh::endpoint::{RecvStream, SendStream};
use nectarlink_protocol::{
    DeviceId, Envelope, ErrorCode, MAX_FRAME_LEN,
    messages::{
        MediaCommand, MediaFileMeta, MediaFileRead, MediaPlayer, MediaState, MediaWatch, StreamHeader,
        WatchFile, media_actions, media_limits::PLAYERS, media_watch, types,
    },
    read_frame, write_frame,
};

use crate::{Error, Result, events::NodeEvent, node::Shared, session::Session};

/// How long a Play on phone offer stays open for the phone to take.
const WATCH_OFFER: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// The device toggle that allows media for a device.
pub(crate) const TOGGLE: &str = "media";
/// Offered by devices that show and control other devices' players.
const REMOTE: &str = "media.remote";
/// Offered by devices that share their own players.
const SHARES: &str = "media.control";

/// A command for a player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaAction {
    Play,
    Pause,
    Next,
    Previous,
    /// To the position given with it.
    Seek,
}

impl MediaAction {
    pub fn as_str(self) -> &'static str {
        match self {
            MediaAction::Play => media_actions::PLAY,
            MediaAction::Pause => media_actions::PAUSE,
            MediaAction::Next => media_actions::NEXT,
            MediaAction::Previous => media_actions::PREVIOUS,
            MediaAction::Seek => media_actions::SEEK,
        }
    }

    pub fn parse(action: &str) -> Option<MediaAction> {
        Some(match action {
            media_actions::PLAY => MediaAction::Play,
            media_actions::PAUSE => MediaAction::Pause,
            media_actions::NEXT => MediaAction::Next,
            media_actions::PREVIOUS => MediaAction::Previous,
            media_actions::SEEK => MediaAction::Seek,
            _ => return None,
        })
    }
}

/// Why a media command didn't run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaError {
    /// The player is gone.
    NotFound,
    /// The player (or this device) can't do that.
    Unsupported,
    /// Anything else; the string is for logs.
    Failed(String),
}

/// This device's players, as the app last reported them, with their
/// artwork kept apart for sending once per session.
#[derive(Default)]
pub(crate) struct Players {
    inner: Mutex<PlayersState>,
}

#[derive(Default)]
struct PlayersState {
    /// Set once the app reports players: only then does this device send.
    active: bool,
    players: Vec<MediaPlayer>,
    art: HashMap<String, Vec<u8>>,
}

impl Players {
    fn lock(&self) -> std::sync::MutexGuard<'_, PlayersState> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set(&self, players: Vec<MediaPlayer>) {
        let mut state = self.lock();
        state.active = true;
        state.players.clear();
        let mut art = HashMap::new();
        for mut player in players.into_iter().filter_map(MediaPlayer::sanitized).take(PLAYERS) {
            if let (Some(key), Some(bytes)) = (&player.art_key, player.art.take()) {
                art.insert(key.clone(), bytes);
            } else if let Some(key) = &player.art_key
                && let Some(bytes) = state.art.remove(key)
            {
                // Artwork is sent with every report; keep what's still in use.
                art.insert(key.clone(), bytes);
            }
            state.players.push(player);
        }
        state.art = art;
    }

    /// The players for one session, with new artwork that fits in the frame.
    fn for_session(&self, sent: &mut HashSet<String>) -> MediaState {
        // Leave room for metadata for every player (four 256-character text
        // fields, ID, art key, actions and CBOR overhead) and the envelope.
        let mut remaining = MAX_FRAME_LEN - 64 * 1024;
        let state = self.lock();
        let mut players = state.players.clone();
        // Prefer artwork for playing media if several players get new artwork at once.
        for playing in [true, false] {
            for player in players.iter_mut().filter(|p| p.playing == playing) {
                if let Some(key) = &player.art_key
                    && let Some(art) = state.art.get(key)
                    && art.len() <= remaining
                    && sent.insert(key.clone())
                {
                    player.art = Some(art.clone());
                    remaining -= art.len();
                }
            }
        }
        MediaState { players }
    }

    /// Returns the title of the currently playing media item on this device, if any.
    pub(crate) fn active_title(&self) -> Option<String> {
        let state = self.lock();
        state
            .players
            .iter()
            .find(|p| p.playing && p.title.as_ref().is_some_and(|t| !t.trim().is_empty()))
            .or_else(|| state.players.iter().find(|p| p.title.as_ref().is_some_and(|t| !t.trim().is_empty())))
            .and_then(|p| p.title.clone())
    }
}

impl Shared {
    fn shows_media(&self, peer: &DeviceId) -> bool {
        self.store.get_peer(peer).ok().flatten().is_some_and(|p| p.caps.contains(REMOTE))
    }

    /// Sends this device's players to one device: all of them when the
    /// user allows it, none (which clears that device) when not.
    pub(crate) async fn send_media_state(&self, session: &Arc<Session>) {
        if !self.players.lock().active || !self.shows_media(&session.peer) {
            return;
        }
        let mut sent_art = session.sent_art.lock().await;
        let state = if self.toggle_on(&session.peer, TOGGLE) {
            self.players.for_session(&mut sent_art)
        } else {
            MediaState { players: Vec::new() }
        };
        if let Ok(env) = Envelope::new(types::MEDIA_STATE, &state) {
            let _ = session.send(env).await;
        }
    }

    pub(crate) async fn media_changed(self: &Arc<Self>, players: Vec<MediaPlayer>) {
        self.players.set(players);
        let mut sends = tokio::task::JoinSet::new();
        for session in self.live_sessions() {
            if self.toggle_on(&session.peer, TOGGLE) {
                let shared = self.clone();
                sends.spawn(async move { shared.send_media_state(&session).await });
            }
        }
        while sends.join_next().await.is_some() {}
    }

    /// Brings both sides in line after the user allowed or stopped media
    /// for a device.
    pub(crate) fn media_toggled(self: &Arc<Self>, peer: DeviceId, enabled: bool) {
        let Some(session) = self.session(&peer) else {
            if !enabled {
                self.emit(NodeEvent::MediaChanged { device: peer, players: Vec::new() });
            }
            return;
        };
        let shared = self.clone();
        self.runtime.spawn(async move {
            // What this device shares: everything, or nothing.
            shared.send_media_state(&session).await;
            if enabled {
                // What the other one shares: ask for it.
                let _ = session.send(Envelope::empty(types::MEDIA_SYNC)).await;
            } else {
                shared.emit(NodeEvent::MediaChanged { device: peer, players: Vec::new() });
            }
        });
    }
}

/// Handles a `media.*` message on a session. Returns false for other types.
pub(crate) async fn handle(shared: &Arc<Shared>, session: &Arc<Session>, env: &Envelope) -> Result<bool> {
    let peer = session.peer;
    match env.t.as_str() {
        types::MEDIA_STATE => {
            if shared.toggle_on(&peer, TOGGLE) {
                let state: MediaState = env.body()?;
                let players =
                    state.players.into_iter().filter_map(MediaPlayer::sanitized).take(PLAYERS).collect();
                shared.emit(NodeEvent::MediaChanged { device: peer, players });
            }
        }
        types::MEDIA_SYNC => shared.send_media_state(session).await,
        types::MEDIA_WATCH => {
            let reply = watch_offered(shared, &peer, env.body()?).await;
            session.send(reply.reply_to(env.id)).await?;
        }
        types::MEDIA_COMMAND => {
            let command: MediaCommand = env.body()?;
            let reply = run_command(shared, &peer, command).await;
            session.send(reply.reply_to(env.id)).await?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// A Play on phone offer this PC made a phone.
#[derive(Debug, Clone)]
pub(crate) struct WatchOffer {
    player: String,
    /// When it was made, or last taken or left.
    at: std::time::Instant,
    /// The file the phone may read, for a file it plays itself.
    file: Option<std::path::PathBuf>,
}

/// Play on phone, on the PC: offers `peer` what `player` plays, streamed,
/// or with `file` (and where it had got to, `position_ms`), that video
/// file for the phone to play itself. The offer is what lets that phone
/// start mirroring with `watch` (see [`watch_allowed`]), or read the file
/// (see [`serve_file`]).
pub(crate) async fn offer_watch(
    shared: &Arc<Shared>,
    peer: DeviceId,
    player: String,
    title: Option<String>,
    file: Option<(std::path::PathBuf, u64)>,
) -> Result<()> {
    let described = match &file {
        Some((path, position)) => {
            let size = tokio::fs::metadata(path).await.map_err(|e| Error::Internal(e.to_string()))?.len();
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            Some(WatchFile { name, size, position: *position })
        }
        None => None,
    };
    let watch = MediaWatch { player, title, file: described };
    if !watch.is_valid() {
        return Err(Error::Internal("not a player to offer".into()));
    }
    if !shared.we_offer(media_watch::STREAM) {
        return Err(Error::Unsupported);
    }
    if !shared.store.get_peer(&peer)?.is_some_and(|p| p.caps.contains(media_watch::WATCH)) {
        return Err(Error::Unsupported);
    }
    if !shared.toggle_on(&peer, TOGGLE) {
        return Err(Error::Denied);
    }
    let session = shared.session(&peer).ok_or(Error::Offline)?;
    let offer = WatchOffer {
        player: watch.player.clone(),
        at: std::time::Instant::now(),
        file: file.map(|(path, _)| path),
    };
    shared.watch_offers.lock().unwrap_or_else(|e| e.into_inner()).insert(peer, offer);
    let env = Envelope::new(types::MEDIA_WATCH, &watch)?;
    let answer =
        session.request(env, crate::session::REQUEST_TIMEOUT).await.and_then(|r| Ok(r.expect(types::OK)?));
    if answer.is_err() {
        shared.watch_offers.lock().unwrap_or_else(|e| e.into_inner()).remove(&peer);
    }
    answer.map(|_| ())
}

/// Whether `peer` may watch `player`: this PC offered it, and the phone
/// takes it (or comes back to it) in time. Taking it renews the offer.
pub(crate) fn watch_allowed(shared: &Shared, peer: &DeviceId, player: &str) -> bool {
    let mut offers = shared.watch_offers.lock().unwrap_or_else(|e| e.into_inner());
    match offers.get_mut(peer) {
        Some(offer) if offer.player == player && offer.at.elapsed() < WATCH_OFFER => {
            offer.at = std::time::Instant::now();
            true
        }
        _ => false,
    }
}

/// Whether `peer` has a Play on phone offer from this PC open (so this PC
/// may open the stream to it).
pub(crate) fn watch_open(shared: &Shared, peer: &DeviceId) -> bool {
    shared
        .watch_offers
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(peer)
        .is_some_and(|offer| offer.at.elapsed() < WATCH_OFFER)
}

/// Watching stopped: the offer stays open for a while from now.
pub(crate) fn renew_watch(shared: &Shared, peer: &DeviceId) {
    if let Some(offer) = shared.watch_offers.lock().unwrap_or_else(|e| e.into_inner()).get_mut(peer) {
        offer.at = std::time::Instant::now();
    }
}

/// The file `peer` may read, while its offer is open (reading renews it).
fn offered_file(shared: &Shared, peer: &DeviceId) -> Option<std::path::PathBuf> {
    let mut offers = shared.watch_offers.lock().unwrap_or_else(|e| e.into_inner());
    let offer = offers.get_mut(peer).filter(|offer| offer.at.elapsed() < WATCH_OFFER)?;
    offer.at = std::time::Instant::now();
    offer.file.clone()
}

/// How long a `media/file` read may take to arrive or answer.
const FILE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// The PC's side of a `media/file` stream (its header already read): one
/// read of the offered file, if `peer` has one open.
pub(crate) async fn serve_file(
    shared: Arc<Shared>,
    peer: DeviceId,
    mut send: SendStream,
    mut recv: RecvStream,
) {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    let Ok(Ok(Some(bytes))) = tokio::time::timeout(FILE_TIMEOUT, read_frame(&mut recv)).await else {
        return;
    };
    let Ok(env) = Envelope::from_cbor(&bytes) else { return };
    let reply_error = |code, msg: &'static str| Envelope::error(code, msg).reply_to(env.id);
    let read = match env.expect_body::<MediaFileRead>(types::MEDIA_FILE) {
        Ok(read) => read,
        Err(_) => {
            let _ =
                write_frame(&mut send, &reply_error(ErrorCode::BadMessage, "invalid media.file").to_cbor())
                    .await;
            let _ = send.finish();
            return;
        }
    };
    let Some(path) = offered_file(&shared, &peer) else {
        let _ = write_frame(&mut send, &reply_error(ErrorCode::Denied, "no file offered").to_cbor()).await;
        let _ = send.finish();
        return;
    };
    let opened = async {
        let mut file = tokio::fs::File::open(&path).await?;
        let size = file.metadata().await?.len();
        let length = read.length.min(media_watch::MAX_READ).min(size.saturating_sub(read.offset));
        file.seek(std::io::SeekFrom::Start(read.offset.min(size))).await?;
        let mut data = vec![0u8; length as usize];
        file.read_exact(&mut data).await?;
        std::io::Result::Ok((size, data))
    }
    .await;
    let (size, data) = match opened {
        Ok(read) => read,
        Err(e) => {
            tracing::warn!(error = %e, "can't read the offered file");
            let _ =
                write_frame(&mut send, &reply_error(ErrorCode::Internal, "can't read it").to_cbor()).await;
            let _ = send.finish();
            return;
        }
    };
    let meta = MediaFileMeta { size, length: data.len() as u64 };
    let Ok(reply) = Envelope::new(types::MEDIA_FILE_META, &meta).map(|e| e.reply_to(env.id)) else { return };
    if write_frame(&mut send, &reply.to_cbor()).await.is_ok() && send.write_all(&data).await.is_ok() {
        let _ = send.finish();
    }
}

/// The phone's side: reads up to `length` bytes of the file `peer` (a PC)
/// offered, from `offset`. Returns the file's size and the bytes.
pub(crate) async fn read_file(session: &Session, offset: u64, length: u64) -> Result<(u64, Vec<u8>)> {
    let (mut send, mut recv) = session.open_bi().await?;
    let header = StreamHeader {
        svc: media_watch::SERVICE.into(),
        op: media_watch::OP_FILE.into(),
        v: media_watch::VERSION,
    };
    write_frame(&mut send, &Envelope::new(types::STREAM, &header)?.to_cbor()).await?;
    let read = MediaFileRead { offset, length: length.min(media_watch::MAX_READ) };
    write_frame(&mut send, &Envelope::new(types::MEDIA_FILE, &read)?.with_id(1).to_cbor()).await?;
    let _ = send.finish();
    let reply = match tokio::time::timeout(FILE_TIMEOUT, read_frame(&mut recv)).await {
        Ok(Ok(Some(bytes))) => Envelope::from_cbor(&bytes)?,
        Ok(Ok(None)) => return Err(Error::Offline),
        Ok(Err(e)) => return Err(e.into()),
        Err(_) => return Err(Error::Timeout),
    };
    let meta: MediaFileMeta = reply.expect_body(types::MEDIA_FILE_META)?;
    if meta.length > media_watch::MAX_READ {
        return Err(Error::Protocol("media.file sent too much".into()));
    }
    let mut data = vec![0u8; meta.length as usize];
    tokio::time::timeout(FILE_TIMEOUT, recv.read_exact(&mut data))
        .await
        .map_err(|_| Error::Timeout)?
        .map_err(|e| Error::Network(e.to_string()))?;
    Ok((meta.size, data))
}

/// The phone's side of `media.watch`: show the offer.
async fn watch_offered(shared: &Arc<Shared>, peer: &DeviceId, watch: MediaWatch) -> Envelope {
    if !shared.we_offer(media_watch::WATCH) {
        return Envelope::error(ErrorCode::Unsupported, "this device doesn't show a PC's playback");
    }
    if !shared.toggle_on(peer, TOGGLE) {
        return Envelope::error(ErrorCode::Denied, "media is off for this device");
    }
    if !watch.is_valid() {
        return Envelope::error(ErrorCode::BadMessage, "invalid watch offer");
    }
    let platform = shared.platform.clone();
    let peer = *peer;
    let MediaWatch { player, title, file } = watch;
    match tokio::task::spawn_blocking(move || {
        platform.watch_offered(&peer, &player, title.as_deref(), file.as_ref())
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()))
    {
        Ok(()) => Envelope::empty(types::OK),
        Err(reason) => {
            tracing::warn!(reason, "can't show a watch offer");
            Envelope::error(ErrorCode::Internal, "can't show it")
        }
    }
}

async fn run_command(shared: &Arc<Shared>, peer: &DeviceId, command: MediaCommand) -> Envelope {
    if !shared.we_offer(SHARES) {
        return Envelope::error(ErrorCode::Unsupported, "media control is not available");
    }
    if !shared.toggle_on(peer, TOGGLE) {
        return Envelope::error(ErrorCode::Denied, "media is off for this device");
    }
    let Some(action) = MediaAction::parse(&command.action) else {
        return Envelope::error(ErrorCode::Unsupported, "unknown media action");
    };
    if action == MediaAction::Seek && command.position.is_none() {
        return Envelope::error(ErrorCode::BadMessage, "seek needs a position");
    }
    let platform = shared.platform.clone();
    let MediaCommand { player, position, .. } = command;
    let ran = tokio::task::spawn_blocking(move || platform.media_command(&player, action, position))
        .await
        .unwrap_or_else(|e| Err(MediaError::Failed(e.to_string())));
    match ran {
        Ok(()) => Envelope::empty(types::OK),
        Err(MediaError::NotFound) => Envelope::error(ErrorCode::NotFound, "no such player"),
        Err(MediaError::Unsupported) => Envelope::error(ErrorCode::Unsupported, "the player can't do that"),
        Err(MediaError::Failed(reason)) => {
            tracing::warn!(reason, "media command failed");
            Envelope::error(ErrorCode::Internal, "failed")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_round_trip() {
        for action in [
            MediaAction::Play,
            MediaAction::Pause,
            MediaAction::Next,
            MediaAction::Previous,
            MediaAction::Seek,
        ] {
            assert_eq!(MediaAction::parse(action.as_str()), Some(action));
        }
        assert_eq!(MediaAction::parse("rewind"), None);
    }
}
