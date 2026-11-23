// SPDX-License-Identifier: GPL-3.0-or-later
//! Dispatches remote touchpad, keyboard and presentation input from a paired
//! phone to `win::input` and updates the laser pointer overlay state.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use nectarlink_core::{
    ButtonAction, DeviceId, GestureAction, KeyMod, LinkState, MouseButton, NodeEvent, RemoteInfo, RemoteInput,
};

use crate::{core_host, state::Changes, win};

/// When the laser pointer last heard from its phone (ms since the Unix
/// epoch; 0 when it's off). The phone repeats it every second while held,
/// so a dot that stops hearing (the phone left) is hidden.
static LASER_SEEN: AtomicU64 = AtomicU64::new(0);
const LASER_TIMEOUT: Duration = Duration::from_secs(4);
/// Transient pause toggled from the tray; resets to `false` on app restart.
static PAUSED: AtomicBool = AtomicBool::new(false);
/// Peers currently reporting active remote presence (`remote.check` `presence: true`).
static PRESENCE: OnceLock<Mutex<HashSet<DeviceId>>> = OnceLock::new();

fn presence() -> &'static Mutex<HashSet<DeviceId>> {
    PRESENCE.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Who holds each OS-level button, modifier and the Alt-Tab switcher. Two
/// phones can hold the same button: the OS sees one press, and the release
/// waits for the last of them, so one phone leaving can't drop another's drag.
#[derive(Debug, Default)]
struct Holds {
    buttons: HashMap<MouseButton, HashSet<DeviceId>>,
    mods: BTreeMap<KeyMod, HashSet<DeviceId>>,
    switcher: Option<DeviceId>,
}

/// What became free when a phone let go of everything.
#[derive(Debug, Default, PartialEq, Eq)]
struct Freed {
    buttons: Vec<MouseButton>,
    mods: Vec<KeyMod>,
    switcher: bool,
}

impl Holds {
    /// Whether the OS should see the press: only for the first holder.
    fn button_down(&mut self, peer: DeviceId, button: MouseButton) -> bool {
        let owners = self.buttons.entry(button).or_default();
        let first = owners.is_empty();
        owners.insert(peer);
        first
    }

    /// Whether the OS should see the release: when the last holder lets go,
    /// or when nobody holds it (a stray release can't hurt).
    fn button_up(&mut self, peer: DeviceId, button: MouseButton) -> bool {
        let Some(owners) = self.buttons.get_mut(&button) else { return true };
        owners.remove(&peer);
        if owners.is_empty() {
            self.buttons.remove(&button);
            true
        } else {
            false
        }
    }

    /// The modifiers nobody held yet, which the OS should see pressed.
    fn mods_down(&mut self, peer: DeviceId, mods: &[KeyMod]) -> Vec<KeyMod> {
        let mut fresh = Vec::new();
        for &m in mods {
            let owners = self.mods.entry(m).or_default();
            if owners.is_empty() && !fresh.contains(&m) {
                fresh.push(m);
            }
            owners.insert(peer);
        }
        fresh
    }

    /// The modifiers `peer` was the last to hold, which the OS should release.
    fn mods_up(&mut self, peer: DeviceId, mods: &[KeyMod]) -> Vec<KeyMod> {
        let mut freed = Vec::new();
        for &m in mods {
            if let Some(owners) = self.mods.get_mut(&m) {
                owners.remove(&peer);
                if owners.is_empty() {
                    self.mods.remove(&m);
                    freed.push(m);
                }
            }
        }
        freed
    }

    /// Whether `peer` may drive the switcher: one phone owns it from its
    /// start until it ends or is cancelled.
    fn switch(&mut self, peer: DeviceId, action: GestureAction) -> bool {
        match action {
            GestureAction::SwitchStart | GestureAction::SwitchNext | GestureAction::SwitchPrev => {
                if self.switcher.is_some_and(|owner| owner != peer) {
                    return false;
                }
                self.switcher = Some(peer);
                true
            }
            GestureAction::SwitchEnd | GestureAction::SwitchCancel => match self.switcher {
                Some(owner) if owner != peer => false,
                _ => {
                    self.switcher = None;
                    true
                }
            },
            _ => true,
        }
    }

    /// Drops everything `peer` held and says what nobody holds any more.
    fn release_peer(&mut self, peer: DeviceId) -> Freed {
        let mut freed = Freed::default();
        self.buttons.retain(|button, owners| {
            if owners.remove(&peer) && owners.is_empty() {
                freed.buttons.push(*button);
            }
            !owners.is_empty()
        });
        self.mods.retain(|m, owners| {
            if owners.remove(&peer) && owners.is_empty() {
                freed.mods.push(*m);
            }
            !owners.is_empty()
        });
        if self.switcher == Some(peer) {
            self.switcher = None;
            freed.switcher = true;
        }
        freed
    }

    fn is_empty(&self) -> bool {
        self.buttons.is_empty() && self.mods.is_empty() && self.switcher.is_none()
    }
}

static HOLDS: OnceLock<Mutex<Holds>> = OnceLock::new();

fn holds() -> std::sync::MutexGuard<'static, Holds> {
    HOLDS.get_or_init(Mutex::default).lock().unwrap_or_else(|e| e.into_inner())
}

pub fn is_paused() -> bool {
    PAUSED.load(Ordering::Acquire)
}

pub fn set_paused(paused: bool) {
    let prev = PAUSED.swap(paused, Ordering::AcqRel);
    if paused {
        stop_all();
    }
    if prev != paused {
        core_host::host().hub.changed(Changes::REMOTE);
    }
}

pub fn active_peer_count() -> usize {
    presence().lock().unwrap_or_else(|e| e.into_inner()).len()
}

pub fn info() -> RemoteInfo {
    win::input::remote_info()
}

pub fn release_for(peer: DeviceId) {
    let _ = crate::input_share::leave_remote();
    let (freed, nobody_left) = {
        let mut holds = holds();
        let freed = holds.release_peer(peer);
        (freed, holds.is_empty())
    };
    if nobody_left {
        // Nothing held by another phone: release everything still down.
        if freed.switcher {
            win::input::gesture(GestureAction::SwitchCancel);
        }
        win::input::release_held_buttons();
    } else {
        // Another phone still holds something: release only what was this one's.
        for button in freed.buttons {
            win::input::mouse_button(button, ButtonAction::Up);
        }
        win::input::hold_mods(&freed.mods, false);
        if freed.switcher {
            win::input::gesture(GestureAction::SwitchCancel);
        }
    }
    crate::gamepad::off(peer);
    if presence().lock().unwrap_or_else(|e| e.into_inner()).remove(&peer) {
        core_host::host().hub.changed(Changes::REMOTE);
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn hide_laser() {
    LASER_SEEN.store(0, Ordering::SeqCst);
    core_host::host().hub.update(|s| if s.laser.take().is_some() { Changes::REMOTE } else { Changes::NONE });
}

pub fn handle_input(peer: &DeviceId, input: RemoteInput) {
    if is_paused() {
        return;
    }
    let _ = crate::input_share::enter_remote(*peer);
    match input {
        RemoteInput::Move { dx, dy } => win::input::move_relative(dx, dy),
        RemoteInput::MovePrecise { dx, dy } => win::input::move_precise(dx, dy),
        RemoteInput::MoveAbsolute { x, y, click, .. } => win::input::move_virtual_desktop(x, y, click),
        RemoteInput::Button { button, action } => held_button(*peer, button, action, &[]),
        RemoteInput::ModifiedButton { button, action, mods } => held_button(*peer, button, action, &mods),
        RemoteInput::Scroll { dx, dy } => win::input::scroll(dx, dy),
        RemoteInput::Zoom { delta } => win::input::zoom(delta),
        RemoteInput::Text { text } => win::input::type_text(&text),
        RemoteInput::Key { key, mods } => win::input::press_key(&key, &mods),
        RemoteInput::Slide { action } => win::input::slide(action),
        RemoteInput::Gesture { action } => {
            if holds().switch(*peer, action) {
                win::input::gesture(action);
            }
        }
        RemoteInput::Laser { on: true, x, y } => {
            let x = x.clamp(0.0, 1.0);
            let y = y.clamp(0.0, 1.0);
            let peer = *peer;
            core_host::host().hub.update(|s| {
                let next = Some((peer, x, y));
                if s.laser == next {
                    return Changes::NONE;
                }
                s.laser = next;
                Changes::REMOTE
            });
            // One watchdog while it's shown.
            if LASER_SEEN.swap(now_ms(), Ordering::SeqCst) == 0 {
                core_host::spawn(async {
                    loop {
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        let seen = LASER_SEEN.load(Ordering::SeqCst);
                        if seen == 0 {
                            return;
                        }
                        if now_ms().saturating_sub(seen) > LASER_TIMEOUT.as_millis() as u64 {
                            hide_laser();
                            return;
                        }
                    }
                });
            }
        }
        RemoteInput::Laser { on: false, .. } => hide_laser(),
        // These go to `gamepad` through the platform's `remote_pad`.
        RemoteInput::Pad { .. } | RemoteInput::PadOff => {}
    }
}

/// A button press or release from `peer`, maybe with modifiers held around
/// it. A click is self-contained; a press or release goes to the OS only for
/// the first or last phone holding that button or modifier.
fn held_button(peer: DeviceId, button: MouseButton, action: ButtonAction, mods: &[KeyMod]) {
    match action {
        ButtonAction::Click => win::input::modified_button(button, action, mods),
        ButtonAction::Down => {
            let (fresh_mods, press) = {
                let mut holds = holds();
                (holds.mods_down(peer, mods), holds.button_down(peer, button))
            };
            win::input::hold_mods(&fresh_mods, true);
            if press {
                win::input::mouse_button(button, ButtonAction::Down);
            }
        }
        ButtonAction::Up => {
            let (release, freed_mods) = {
                let mut holds = holds();
                (holds.button_up(peer, button), holds.mods_up(peer, mods))
            };
            if release {
                win::input::mouse_button(button, ButtonAction::Up);
            }
            win::input::hold_mods(&freed_mods, false);
        }
    }
}

/// Clears any active laser pointer and releases held mouse buttons (e.g. when
/// `remote_input` is turned off).
pub fn stop_all() {
    *holds() = Holds::default();
    let _ = crate::input_share::leave_remote();
    win::input::release_held_buttons();
    crate::gamepad::stop_all();
    hide_laser();
}

pub fn on_event(event: &NodeEvent) {
    match event {
        NodeEvent::LinkChanged { link: LinkState::Offline { .. }, device, .. } => {
            release_for(*device);
        }
        NodeEvent::DeviceRemoved(device) => {
            release_for(*device);
        }
        NodeEvent::RemotePresence { device, present } => {
            let changed = {
                let mut set = presence().lock().unwrap_or_else(|e| e.into_inner());
                if *present { set.insert(*device) } else { set.remove(device) }
            };
            if changed {
                core_host::host().hub.changed(Changes::REMOTE);
            }
        }
        NodeEvent::RemoteInputRequested { .. } => {
            core_host::host().hub.update(|s| s.apply(event));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(n: u8) -> DeviceId {
        DeviceId([n; 32])
    }

    #[test]
    fn a_shared_button_is_released_by_its_last_holder() {
        let mut holds = Holds::default();
        assert!(holds.button_down(peer(1), MouseButton::Left));
        assert!(!holds.button_down(peer(2), MouseButton::Left), "already down for the OS");
        assert!(!holds.button_up(peer(1), MouseButton::Left), "phone 2 still drags");
        assert!(holds.button_up(peer(2), MouseButton::Left));
        assert!(holds.button_up(peer(3), MouseButton::Left), "a stray release passes through");
        assert!(holds.is_empty());
    }

    #[test]
    fn modifiers_are_pressed_once_and_released_by_the_last_holder() {
        let mut holds = Holds::default();
        assert_eq!(holds.mods_down(peer(1), &[KeyMod::Ctrl, KeyMod::Shift]), [KeyMod::Ctrl, KeyMod::Shift]);
        assert_eq!(holds.mods_down(peer(2), &[KeyMod::Ctrl, KeyMod::Alt]), [KeyMod::Alt]);
        assert_eq!(holds.mods_up(peer(1), &[KeyMod::Ctrl, KeyMod::Shift]), [KeyMod::Shift]);
        assert_eq!(holds.mods_up(peer(2), &[KeyMod::Ctrl, KeyMod::Alt]), [KeyMod::Ctrl, KeyMod::Alt]);
        assert!(holds.is_empty());
    }

    #[test]
    fn one_phone_owns_the_switcher_until_it_ends() {
        let mut holds = Holds::default();
        assert!(holds.switch(peer(1), GestureAction::SwitchStart));
        assert!(!holds.switch(peer(2), GestureAction::SwitchNext));
        assert!(!holds.switch(peer(2), GestureAction::SwitchCancel));
        assert!(holds.switch(peer(1), GestureAction::SwitchPrev));
        assert!(holds.switch(peer(1), GestureAction::SwitchEnd));
        assert!(holds.switch(peer(2), GestureAction::SwitchStart));
        assert!(holds.switch(peer(1), GestureAction::Search), "other gestures aren't owned");
    }

    #[test]
    fn a_leaving_phone_frees_only_what_nobody_else_holds() {
        let mut holds = Holds::default();
        holds.button_down(peer(1), MouseButton::Left);
        holds.button_down(peer(2), MouseButton::Left);
        holds.button_down(peer(1), MouseButton::Right);
        holds.mods_down(peer(1), &[KeyMod::Ctrl]);
        holds.switch(peer(1), GestureAction::SwitchStart);
        let freed = holds.release_peer(peer(1));
        assert_eq!(
            freed,
            Freed { buttons: vec![MouseButton::Right], mods: vec![KeyMod::Ctrl], switcher: true }
        );
        assert!(!holds.is_empty(), "phone 2 still holds Left");
        assert_eq!(holds.release_peer(peer(2)).buttons, [MouseButton::Left]);
        assert!(holds.is_empty());
    }
}
