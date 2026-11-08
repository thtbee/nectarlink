// SPDX-License-Identifier: GPL-3.0-or-later
//! The phone's sound on this PC without a mirror window
//! (`MirrorStart::sound_only`, docs/protocol/mirror.md): what the phone plays
//! comes out of the PC's speakers or headphones. Opening the phone's mirror
//! window takes over, since that brings the sound too.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use nectarlink_core::{
    DeviceId, Error, FeatureState, LinkState, MIRROR_SCREEN, MirrorAudioConfig, MirrorConfig, MirrorSink,
    MirrorStart,
};

use crate::{
    bridge::app::show_message,
    core_host,
    i18n::tr,
    state::{AppState, Changes},
    win::audio_out::Player,
};

/// How long the phone's prompt may go unanswered.
const ASK_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Asked; the phone may be showing its prompt.
    Asking,
    /// Its sound is playing here.
    Playing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Session {
    device: DeviceId,
    request: u64,
    phase: Phase,
}

/// The one phone whose sound plays here, if any.
static ACTIVE: Mutex<Option<Session>> = Mutex::new(None);
static NEXT_REQUEST: AtomicU64 = AtomicU64::new(0);
/// Serialize remote stop/start commands, including rapid source changes.
static REMOTE: tokio::sync::Mutex<Option<Session>> = tokio::sync::Mutex::const_new(None);
/// Whether it plays for the phone's app windows (it stops with the last
/// one), rather than because it was turned on here.
static FOR_APPS: AtomicBool = AtomicBool::new(false);

fn active() -> Option<Session> {
    *ACTIVE.lock().unwrap_or_else(|e| e.into_inner())
}

fn changed() {
    // Home's switch, and app windows' sound button.
    core_host::host().hub.changed(Changes::MEDIA | Changes::MIRROR);
}

/// A callback from an old stream must not change a newer request, even
/// when the user has switched away from and back to the same phone.
fn replace_current(active: &mut Option<Session>, expected: Session, next: Option<Session>) -> bool {
    if active.is_some_and(|s| s.device == expected.device && s.request == expected.request) {
        *active = next;
        true
    } else {
        false
    }
}

fn update_current(expected: Session, next: Option<Session>) -> bool {
    let replaced = replace_current(&mut ACTIVE.lock().unwrap_or_else(|e| e.into_inner()), expected, next);
    if replaced {
        changed();
    }
    replaced
}

fn is_current(session: Session) -> bool {
    active().is_some_and(|s| s.device == session.device && s.request == session.request)
}

fn take_current(active: &mut Option<Session>, expected: Session, phase: Option<Phase>) -> Option<Session> {
    active.filter(|s| {
        s.device == expected.device
            && s.request == expected.request
            && phase.is_none_or(|phase| s.phase == phase)
    })?;
    active.take()
}

fn clear_current(session: Session, phase: Option<Phase>) -> bool {
    let cleared =
        take_current(&mut ACTIVE.lock().unwrap_or_else(|e| e.into_inner()), session, phase).is_some();
    if cleared {
        changed();
    }
    cleared
}

/// The selected phone, if it is online and can share its sound alone.
fn phone(device: DeviceId) -> Option<(DeviceId, String)> {
    core_host::host().hub.read(|s| sharing_phone(s, device))
}

fn sharing_phone(s: &AppState, device: DeviceId) -> Option<(DeviceId, String)> {
    let phone = s.devices.iter().find(|d| d.id == device)?;
    (matches!(phone.link, LinkState::Online { .. })
        && s.matrices.get(&device).and_then(|m| m.state("mirroring.sound")) == Some(FeatureState::Available))
    .then(|| (device, phone.info.name.clone()))
}

/// Whether `device`'s sound is playing here without its screen's window.
pub fn playing(device: &DeviceId) -> bool {
    state_for(active(), device) == "playing"
}

fn state_for(session: Option<Session>, device: &DeviceId) -> &'static str {
    match session.filter(|s| s.device == *device) {
        None => "off",
        Some(Session { phase: Phase::Asking, .. }) => "asking",
        Some(Session { phase: Phase::Playing, .. }) => "playing",
    }
}

/// "off", "asking" (waiting for this phone) or "playing".
pub fn state(device: &DeviceId) -> &'static str {
    state_for(active(), device)
}

/// Starts or stops playing the selected phone's sound here.
pub fn set_on(device: DeviceId, on: bool) {
    if !on {
        stop_for_window(device);
        return;
    }
    if let Some((device, name)) = phone(device) {
        start(device, name, false);
    }
}

/// `device`'s app windows opened (`open`) or all closed (#121): its sound
/// plays here while they're open. Sound turned on here stays as it is.
pub fn for_app_windows(device: DeviceId, open: bool) {
    match active() {
        None if open => {
            let Some((_, name)) = phone(device) else { return };
            start(device, name, true);
        }
        Some(session) if session.device == device && !open && FOR_APPS.load(Ordering::Relaxed) => {
            stop(session)
        }
        _ => {}
    }
}

/// Asks `device` for its sound. `quiet`: for app windows, which work without
/// it, so a phone that can't share it only says so in the log.
fn start(device: DeviceId, name: String, quiet: bool) {
    let Some(node) = core_host::node() else { return };
    let session = {
        let mut active = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(current) = *active {
            // App windows must not replace sound explicitly turned on here.
            if quiet {
                return;
            }
            FOR_APPS.store(false, Ordering::Relaxed);
            if current.device == device {
                return;
            }
        }
        FOR_APPS.store(quiet, Ordering::Relaxed);
        let session =
            Session { device, request: NEXT_REQUEST.fetch_add(1, Ordering::Relaxed), phase: Phase::Asking };
        *active = Some(session);
        session
    };
    changed();
    core_host::spawn(async move {
        let mut remote = REMOTE.lock().await;
        if !is_current(session) {
            return;
        }
        if let Some(previous) = remote.take() {
            node.mirror_stop(previous.device, MIRROR_SCREEN).await;
        }
        if !is_current(session) {
            return;
        }
        *remote = Some(session);
        let options = MirrorStart {
            max_size: 0,
            fps: 0,
            bitrate: 0,
            audio: true,
            session: MIRROR_SCREEN,
            app: None,
            stay_awake: false,
            screen_off: false,
            watch: None,
            sound_only: true,
        };
        if let Err(e) = node.mirror_start(device, options).await {
            *remote = None;
            if !update_current(session, None) {
                return;
            }
            if quiet {
                tracing::info!(error = %e, "the phone's sound didn't come with its app windows");
                return;
            }
            show_message(match e {
                Error::Denied => tr!("PhoneSound", "Mirroring is turned off for this phone."),
                Error::Unsupported => {
                    tr!("PhoneSound", "Update NectarLink on %1 to play its sound here.", name)
                }
                Error::Offline | Error::NotPaired => tr!("PhoneSound", "The phone isn't connected."),
                // The phone couldn't show its request (notifications off, say).
                e => {
                    tracing::warn!(error = %e, "the phone couldn't share its sound");
                    tr!(
                        "PhoneSound",
                        "%1 couldn't ask to share its sound. Check that NectarLink can show notifications on the phone.",
                        name
                    )
                }
            });
            return;
        }
        drop(remote);
        // Nobody answered the phone's prompt: stop waiting.
        tokio::time::sleep(ASK_TIMEOUT).await;
        if clear_current(session, Some(Phase::Asking)) {
            stop_remote(session);
        }
    });
}

/// Stops this request and tells its phone.
fn stop(session: Session) {
    if clear_current(session, None) {
        stop_remote(session);
    }
}

fn stop_remote(session: Session) {
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        let mut remote = REMOTE.lock().await;
        if remote.is_some_and(|s| s.request == session.request) {
            *remote = None;
            node.mirror_stop(session.device, MIRROR_SCREEN).await;
        }
    });
}

/// The mirror window is about to ask `device` for its screen, which brings
/// the sound too: the phone shares one or the other, so this one ends first.
pub fn stop_for_window(device: DeviceId) {
    if let Some(session) = active().filter(|s| s.device == device) {
        stop(session);
    }
}

/// The sink for `device`'s sound stream while it plays here without a window.
pub fn sink(device: &DeviceId) -> Option<Arc<dyn MirrorSink>> {
    active()
        .filter(|s| s.device == *device)
        .map(|session| Arc::new(Sink { session, player: Mutex::new(None) }) as Arc<dyn MirrorSink>)
}

/// Plays the sound stream; there's no picture with it.
struct Sink {
    session: Session,
    player: Mutex<Option<Player>>,
}

impl MirrorSink for Sink {
    fn config(&self, _config: MirrorConfig) {}

    fn packet(&self, _keyframe: bool, _time_us: u64, _data: Vec<u8>) {}

    fn ended(&self) {}

    fn audio_config(&self, config: MirrorAudioConfig) {
        if !is_current(self.session) {
            return;
        }
        // A new format: a new player (the old one ends when dropped).
        let player = Player::start(config.rate, u16::from(config.channels))
            .inspect_err(|e| tracing::warn!(error = %e, "can't play the phone's sound"))
            .ok();
        let playing = player.is_some();
        *self.player.lock().unwrap_or_else(|e| e.into_inner()) = player;
        if playing {
            update_current(self.session, Some(Session { phase: Phase::Playing, ..self.session }));
        }
    }

    fn audio(&self, _time_us: u64, data: Vec<u8>) {
        // The mirror window's mute is the phone's sound on this PC.
        if !is_current(self.session) || crate::mirror::muted() {
            return;
        }
        if let Some(player) = self.player.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            player.push(data);
        }
    }

    fn audio_ended(&self) {
        self.player.lock().unwrap_or_else(|e| e.into_inner()).take();
        // An empty Elevated stream can be followed by the phone's consent
        // prompt. Keep waiting for that fallback until sound plays or times out.
        if clear_current(self.session, Some(Phase::Playing)) {
            let session = self.session;
            core_host::spawn(async move {
                let mut remote = REMOTE.lock().await;
                if remote.is_some_and(|s| s.request == session.request) {
                    *remote = None;
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIRST: DeviceId = DeviceId([1; 32]);
    const SECOND: DeviceId = DeviceId([2; 32]);

    #[test]
    fn ended_stream_clears_only_its_own_playing_request() {
        let request = Session { device: SECOND, request: 1, phase: Phase::Asking };
        let playing = Session { phase: Phase::Playing, ..request };
        let mut active = Some(playing);
        assert_eq!(take_current(&mut active, request, None), Some(playing));
        assert_eq!(take_current(&mut active, request, None), None);
        // A locally stopped stream has already been cleared.
        assert_eq!(take_current(&mut None, request, None), None);
    }

    #[test]
    fn timeouts_and_old_streams_cannot_end_playing_or_newer_requests() {
        let old = Session { device: SECOND, request: 1, phase: Phase::Asking };
        let playing = Session { phase: Phase::Playing, ..old };
        let mut active = Some(playing);
        assert_eq!(take_current(&mut active, old, Some(Phase::Asking)), None);
        assert_eq!(active, Some(playing));
        let new = Session { request: 2, ..old };
        active = Some(new);
        assert_eq!(take_current(&mut active, old, None), None);
        assert_eq!(active, Some(new));
        assert_eq!(take_current(&mut active, new, Some(Phase::Asking)), Some(new));
    }

    #[test]
    fn empty_elevated_stream_keeps_waiting_for_phone_consent() {
        let request = Session { device: SECOND, request: 1, phase: Phase::Asking };
        let mut active = Some(request);
        assert_eq!(take_current(&mut active, request, Some(Phase::Playing)), None);
        assert_eq!(active, Some(request));
        // The fallback capture can start on the same request.
        let playing = Session { phase: Phase::Playing, ..request };
        assert!(replace_current(&mut active, request, Some(playing)));
        assert_eq!(state_for(active, &SECOND), "playing");
        assert_eq!(take_current(&mut active, request, Some(Phase::Playing)), Some(playing));
    }

    #[test]
    fn requests_target_the_selected_phone_without_falling_back_to_the_first() {
        use crate::state::DeviceView;
        use nectarlink_core::{CapabilityMatrix, ConnectionPath, DeviceInfo, DeviceKind, PowerLevel};

        let mut state = AppState::default();
        for (id, name) in [(FIRST, "Pixel 7a"), (SECOND, "V2443A")] {
            state.devices.push(DeviceView {
                id,
                info: DeviceInfo {
                    name: name.into(),
                    kind: DeviceKind::Phone,
                    os: "android".into(),
                    os_ver: "15".into(),
                    model: None,
                    accent: None,
                    screen: None,
                    install_id: None,
                },
                paired_at: 0,
                link: LinkState::Online { path: ConnectionPath::Lan, rtt_ms: 4 },
                battery: None,
                power: PowerLevel::Basic,
                reconnect: true,
            });
            state.matrices.insert(
                id,
                CapabilityMatrix {
                    device: id,
                    features: [("mirroring.sound", FeatureState::Available)].into(),
                },
            );
        }
        assert_eq!(sharing_phone(&state, SECOND), Some((SECOND, "V2443A".into())));
        state.devices[1].link = LinkState::Offline { last_seen: None };
        assert_eq!(sharing_phone(&state, SECOND), None);
        state.devices[1].link = LinkState::Online { path: ConnectionPath::Lan, rtt_ms: 4 };
        state.matrices.remove(&SECOND);
        assert_eq!(sharing_phone(&state, SECOND), None);
        assert_eq!(sharing_phone(&state, FIRST), Some((FIRST, "Pixel 7a".into())));
    }

    #[test]
    fn each_phone_shows_only_its_own_sound_state() {
        for phase in [Phase::Asking, Phase::Playing] {
            let first = Session { device: FIRST, request: 0, phase };
            let second = Session { device: SECOND, request: 1, phase };
            assert_eq!(state_for(Some(first), &SECOND), "off");
            assert_eq!(state_for(Some(second), &FIRST), "off");
            let expected = if phase == Phase::Asking { "asking" } else { "playing" };
            assert_eq!(state_for(Some(second), &SECOND), expected);
        }
        assert_eq!(state_for(None, &SECOND), "off");
    }

    #[test]
    fn old_phone_callbacks_and_stop_do_not_clear_the_selected_phone() {
        let first = Session { device: FIRST, request: 0, phase: Phase::Playing };
        let second = Session { device: SECOND, request: 1, phase: Phase::Asking };
        let mut active = Some(second);
        assert!(!replace_current(&mut active, first, None));
        assert!(!replace_current(&mut active, first, Some(first)));
        assert_eq!(active, Some(second));
        let playing = Session { phase: Phase::Playing, ..second };
        assert!(replace_current(&mut active, second, Some(playing)));
        assert_eq!(state_for(active, &SECOND), "playing");
        assert!(replace_current(&mut active, second, None));
        assert_eq!(active, None);
    }

    #[test]
    fn switching_back_does_not_accept_callbacks_from_the_old_request() {
        let old = Session { device: FIRST, request: 0, phase: Phase::Asking };
        let new = Session { request: 2, ..old };
        let mut active = Some(new);
        // A late error, end-of-stream, or timeout from the earlier request.
        assert!(!replace_current(&mut active, old, None));
        // A late audio format must not mark the new request as playing.
        assert!(!replace_current(&mut active, old, Some(Session { phase: Phase::Playing, ..old })));
        assert_eq!(active, Some(new));
    }
}
