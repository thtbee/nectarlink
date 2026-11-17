// SPDX-License-Identifier: GPL-3.0-or-later
//! The PC's mouse over the edge of its screen and onto a phone beside it
//! (#107): when it's pushed against the edge facing the phone, and where the
//! phone's own pointer is while the mouse moves it.
//!
//! The phone shows a pointer of its own, moved by a virtual mouse (Elevated),
//! with pointer acceleration off, so each count moves it the same distance
//! (`PointerScreen::scale`, measured by the phone). The PC follows the pointer
//! from that and asks the phone where it really is only when it seems to have
//! reached the edge facing the PC, so moving it costs no round trips. This
//! file is the arithmetic; `win::pointer_hooks` takes the mouse and keyboard.

use nectarlink_core::{PointerAt, PointerEdge, PointerScreen};

/// Which side of the PC the phone sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub fn parse(s: &str) -> Option<Side> {
        match s {
            "left" => Some(Side::Left),
            "right" => Some(Side::Right),
            _ => None,
        }
    }

    /// The phone's edge facing the PC, where the mouse comes in and goes out.
    pub fn phone_edge(self) -> PointerEdge {
        match self {
            Side::Left => PointerEdge::Right,
            Side::Right => PointerEdge::Left,
        }
    }
}

/// The PC's screens together, in pixels: `right` and `bottom` are one past
/// the last pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// Moves pressed against the edge, in a row, before the mouse goes over: a
/// deliberate push, not a pass along the edge to a corner.
const PUSHES: u8 = 3;
/// How far a move may go up or down and still count as a push.
const PUSH_DRIFT: i32 = 3;
/// How far back from the edge the mouse has to go, after coming back from the
/// phone, before a push sends it over again.
const REARM: i32 = 24;

/// Watches the PC's mouse for a push against the edge facing the phone.
#[derive(Debug)]
pub struct EdgeWatch {
    side: Side,
    pushes: u8,
    last: Option<(i32, i32)>,
    armed: bool,
}

impl EdgeWatch {
    pub fn new(side: Side) -> Self {
        EdgeWatch { side, pushes: 0, last: None, armed: true }
    }

    /// The mouse came back from the phone at the edge: it has to leave the
    /// edge before it can go over again.
    pub fn disarm(&mut self) {
        self.armed = false;
        self.pushes = 0;
        self.last = None;
    }

    /// The PC's mouse moved to (`x`, `y`). True when this move finishes a
    /// push against the edge facing the phone.
    pub fn moved(&mut self, x: i32, y: i32, bounds: Bounds) -> bool {
        let from_edge = match self.side {
            Side::Left => x - bounds.left,
            Side::Right => bounds.right - 1 - x,
        };
        if !self.armed {
            if from_edge >= REARM {
                self.armed = true;
            }
            self.last = Some((x, y));
            return false;
        }
        let at_edge = from_edge <= 0;
        let steady = self.last.is_some_and(|(lx, ly)| lx == x && (ly - y).abs() <= PUSH_DRIFT);
        self.pushes = match (at_edge, steady) {
            (true, true) => self.pushes.saturating_add(1),
            (true, false) => 1,
            (false, _) => 0,
        };
        self.last = Some((x, y));
        if self.pushes >= PUSHES {
            self.pushes = 0;
            return true;
        }
        false
    }
}

/// `y` as a fraction of the way down from `top` to `bottom` (one past the last pixel).
pub fn fraction(y: i32, top: i32, bottom: i32) -> f32 {
    let span = (bottom - top - 1).max(1) as f32;
    ((y - top) as f32 / span).clamp(0.0, 1.0)
}

/// The pixel `fraction` of the way down from `top` to `bottom`.
pub fn at_fraction(fraction: f32, top: i32, bottom: i32) -> i32 {
    let span = (bottom - top - 1).max(0) as f32;
    top + (fraction.clamp(0.0, 1.0) * span).round() as i32
}

/// What a move of the PC's mouse does on the phone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    /// Counts to send.
    pub dx: i32,
    pub dy: i32,
    /// The pointer is at the edge facing the PC and the mouse pushes on:
    /// time to ask the phone where it really is, and come back if it's there.
    pub pushing_out: bool,
}

/// The phone's pointer, as the PC follows it.
#[derive(Debug, Clone)]
pub struct OnPhone {
    screen: PointerScreen,
    edge: PointerEdge,
    x: f32,
    y: f32,
    carry_x: f32,
    carry_y: f32,
}

impl OnPhone {
    /// The pointer came in at `edge`, `y` (0 to 1) of the way down.
    pub fn new(screen: PointerScreen, edge: PointerEdge, y: f32) -> Self {
        let width = screen.width.max(1) as f32;
        let height = screen.height.max(1) as f32;
        let x = match edge {
            PointerEdge::Left => 0.0,
            PointerEdge::Right => width - 1.0,
        };
        OnPhone { screen, edge, x, y: y.clamp(0.0, 1.0) * (height - 1.0), carry_x: 0.0, carry_y: 0.0 }
    }

    /// The PC's mouse moved `dx`, `dy` of its pixels. The phone's pointer
    /// moves as many of the phone's, in counts of `scale` pixels; what doesn't
    /// make a whole count is kept for the next move.
    pub fn moved(&mut self, dx: i32, dy: i32) -> Step {
        let scale = self.screen.scale.max(0.05);
        let want_x = dx as f32 / scale + self.carry_x;
        let want_y = dy as f32 / scale + self.carry_y;
        let counts_x = want_x.trunc();
        let counts_y = want_y.trunc();
        self.carry_x = want_x - counts_x;
        self.carry_y = want_y - counts_y;
        let (max_x, max_y) = (self.screen.width.max(1) as f32 - 1.0, self.screen.height.max(1) as f32 - 1.0);
        self.x = (self.x + counts_x * scale).clamp(0.0, max_x);
        self.y = (self.y + counts_y * scale).clamp(0.0, max_y);
        let pushing_out = match self.edge {
            PointerEdge::Left => dx < 0 && self.x <= 0.0,
            PointerEdge::Right => dx > 0 && self.x >= max_x,
        };
        Step { dx: counts_x as i32, dy: counts_y as i32, pushing_out }
    }

    /// Whether the phone's pointer, really at `at`, is at the edge facing the PC.
    pub fn at_exit(&self, at: PointerAt) -> bool {
        let max_x = self.screen.width.max(1) as f32 - 1.0;
        match self.edge {
            PointerEdge::Left => at.x <= 1.0,
            PointerEdge::Right => at.x >= max_x - 1.0,
        }
    }

    /// Where the phone says its pointer really is (Android moved it on its
    /// own, or acceleration was on).
    pub fn resync(&mut self, at: PointerAt) {
        let (max_x, max_y) = (self.screen.width.max(1) as f32 - 1.0, self.screen.height.max(1) as f32 - 1.0);
        self.x = at.x.clamp(0.0, max_x);
        self.y = at.y.clamp(0.0, max_y);
    }

    /// How far down the phone's screen the pointer is (0 to 1), to bring the
    /// PC's mouse back at the same height.
    pub fn height_fraction(&self) -> f32 {
        let max_y = (self.screen.height.max(1) as f32 - 1.0).max(1.0);
        (self.y / max_y).clamp(0.0, 1.0)
    }
}

/// What a key pressed on the PC does while its mouse is on the phone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    /// One of `mirror_keys`.
    Key(&'static str),
    /// Whatever text the key types with the modifiers held (the PC's
    /// keyboard layout says what).
    Text,
    /// Ctrl+V: types what's on the PC's clipboard.
    Paste,
    /// Ctrl+Alt+Esc: the mouse comes back to the PC, wherever it is.
    Leave,
    /// Nothing on the phone.
    Ignore,
}

/// Windows virtual-key codes this needs (as numbers, so this file builds and
/// tests everywhere).
pub mod vk {
    pub const BACK: u16 = 0x08;
    pub const TAB: u16 = 0x09;
    pub const RETURN: u16 = 0x0D;
    pub const SHIFT: u16 = 0x10;
    pub const CONTROL: u16 = 0x11;
    pub const MENU: u16 = 0x12;
    pub const CAPITAL: u16 = 0x14;
    pub const ESCAPE: u16 = 0x1B;
    pub const LEFT: u16 = 0x25;
    pub const UP: u16 = 0x26;
    pub const RIGHT: u16 = 0x27;
    pub const DOWN: u16 = 0x28;
    pub const DELETE: u16 = 0x2E;
    pub const A: u16 = 0x41;
    pub const C: u16 = 0x43;
    pub const V: u16 = 0x56;
    pub const X: u16 = 0x58;
    pub const LWIN: u16 = 0x5B;
    pub const RWIN: u16 = 0x5C;
    pub const LSHIFT: u16 = 0xA0;
    pub const RSHIFT: u16 = 0xA1;
    pub const LCONTROL: u16 = 0xA2;
    pub const RCONTROL: u16 = 0xA3;
    pub const LMENU: u16 = 0xA4;
    pub const RMENU: u16 = 0xA5;

    pub fn is_modifier(code: u16) -> bool {
        matches!(
            code,
            SHIFT | CONTROL | MENU | LSHIFT | RSHIFT | LCONTROL | RCONTROL | LMENU | RMENU | CAPITAL
        )
    }
}

/// What the key `code`, pressed with Ctrl and/or Alt held, does on the phone.
/// Ctrl and Alt together are AltGr on many layouts, so they still type.
pub fn key_action(code: u16, ctrl: bool, alt: bool) -> KeyAction {
    use nectarlink_core::mirror_keys as keys;
    if vk::is_modifier(code) {
        return KeyAction::Ignore;
    }
    if code == vk::ESCAPE && ctrl && alt {
        return KeyAction::Leave;
    }
    let plain = match code {
        vk::ESCAPE => Some(keys::BACK),
        vk::RETURN => Some(keys::ENTER),
        vk::BACK => Some(keys::BACKSPACE),
        vk::DELETE => Some(keys::DELETE),
        vk::LEFT => Some(keys::LEFT),
        vk::RIGHT => Some(keys::RIGHT),
        vk::UP => Some(keys::UP),
        vk::DOWN => Some(keys::DOWN),
        vk::TAB => Some(keys::TAB),
        vk::LWIN | vk::RWIN => Some(keys::HOME),
        _ => None,
    };
    if let Some(key) = plain {
        return if alt && !ctrl { KeyAction::Ignore } else { KeyAction::Key(key) };
    }
    match (ctrl, alt) {
        (true, false) => match code {
            vk::C => KeyAction::Key(keys::COPY),
            vk::X => KeyAction::Key(keys::CUT),
            vk::A => KeyAction::Key(keys::SELECT_ALL),
            vk::V => KeyAction::Paste,
            _ => KeyAction::Ignore,
        },
        // Alt shortcuts are the PC's, not text.
        (false, true) => KeyAction::Ignore,
        _ => KeyAction::Text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_do_on_the_phone_what_they_do_in_a_mirror_window() {
        use nectarlink_core::mirror_keys as keys;
        assert_eq!(key_action(vk::RETURN, false, false), KeyAction::Key(keys::ENTER));
        assert_eq!(key_action(vk::ESCAPE, false, false), KeyAction::Key(keys::BACK));
        assert_eq!(key_action(vk::LWIN, false, false), KeyAction::Key(keys::HOME));
        assert_eq!(key_action(vk::C, true, false), KeyAction::Key(keys::COPY));
        assert_eq!(key_action(vk::V, true, false), KeyAction::Paste);
        assert_eq!(key_action(0x51, true, false), KeyAction::Ignore, "Ctrl+Q isn't text");
        assert_eq!(key_action(0x51, false, false), KeyAction::Text);
        assert_eq!(key_action(0x51, true, true), KeyAction::Text, "AltGr types");
        assert_eq!(key_action(0x51, false, true), KeyAction::Ignore, "Alt+Q is a shortcut");
        assert_eq!(key_action(vk::LSHIFT, false, false), KeyAction::Ignore);
        assert_eq!(key_action(vk::ESCAPE, true, true), KeyAction::Leave, "the way back, always");
    }

    const SCREEN: Bounds = Bounds { left: -1920, top: 0, right: 2560, bottom: 1440 };

    #[test]
    fn a_push_against_the_edge_goes_over_and_a_pass_along_it_does_not() {
        let mut watch = EdgeWatch::new(Side::Right);
        // Moving towards the edge isn't a push yet.
        assert!(!watch.moved(2500, 700, SCREEN));
        assert!(!watch.moved(2559, 700, SCREEN));
        assert!(!watch.moved(2559, 701, SCREEN));
        assert!(watch.moved(2559, 701, SCREEN), "three moves pressed against it");

        // Sliding down the edge to a corner isn't.
        let mut watch = EdgeWatch::new(Side::Right);
        for y in (0..400).step_by(20) {
            assert!(!watch.moved(2559, y, SCREEN));
        }
        // The left edge is the left of the leftmost screen.
        let mut watch = EdgeWatch::new(Side::Left);
        assert!(!watch.moved(0, 300, SCREEN), "the main screen's left edge isn't the PC's");
        assert!(!watch.moved(-1920, 300, SCREEN));
        assert!(!watch.moved(-1920, 300, SCREEN));
        assert!(watch.moved(-1920, 300, SCREEN));
    }

    #[test]
    fn back_from_the_phone_it_has_to_leave_the_edge_before_going_over_again() {
        let mut watch = EdgeWatch::new(Side::Right);
        watch.disarm();
        for _ in 0..10 {
            assert!(!watch.moved(2559, 700, SCREEN));
        }
        assert!(!watch.moved(2559 - REARM, 700, SCREEN), "far enough back");
        assert!(!watch.moved(2559, 700, SCREEN));
        assert!(!watch.moved(2559, 700, SCREEN));
        assert!(watch.moved(2559, 700, SCREEN));
    }

    #[test]
    fn heights_map_between_screens() {
        assert_eq!(fraction(0, 0, 1440), 0.0);
        assert_eq!(fraction(1439, 0, 1440), 1.0);
        assert_eq!(at_fraction(0.5, 0, 1001), 500);
        assert_eq!(at_fraction(fraction(720, 0, 1441), 0, 1441), 720);
        assert_eq!(at_fraction(2.0, 100, 200), 199, "never past the screen");
    }

    fn phone() -> PointerScreen {
        PointerScreen { width: 1080, height: 2400, scale: 2.0 }
    }

    #[test]
    fn the_phones_pointer_moves_the_pcs_pixels_in_whole_counts() {
        let mut on = OnPhone::new(phone(), PointerEdge::Left, 0.5);
        assert_eq!(on.height_fraction(), 0.5);
        // 3 pixels at 2 a count: 1 count now, the half kept.
        assert_eq!(on.moved(3, 0), Step { dx: 1, dy: 0, pushing_out: false });
        assert_eq!(on.moved(1, 0), Step { dx: 1, dy: 0, pushing_out: false });
        assert_eq!(on.moved(0, -5), Step { dx: 0, dy: -2, pushing_out: false });
        // Far down stays on the screen.
        on.moved(0, 100_000);
        assert_eq!(on.height_fraction(), 1.0);
    }

    #[test]
    fn it_comes_back_only_from_the_edge_facing_the_pc() {
        // Phone on the PC's right: the mouse comes in, and goes back out, at its left edge.
        let mut on = OnPhone::new(phone(), Side::Right.phone_edge(), 0.2);
        assert!(!on.moved(40, 0).pushing_out, "into the phone");
        assert!(!on.moved(-20, 0).pushing_out, "not at the edge yet");
        assert!(on.moved(-40, 0).pushing_out, "at the edge, pushing towards the PC");
        assert!(!on.moved(0, 10).pushing_out, "only while it pushes");
        // The far edge is just an edge.
        on.moved(10_000, 0);
        assert!(!on.moved(10, 0).pushing_out);

        // Android says the pointer is elsewhere: believe it.
        assert!(!on.at_exit(PointerAt { x: 300.0, y: 10.0 }));
        on.resync(PointerAt { x: 300.0, y: 1200.0 });
        assert!(!on.moved(-2, 0).pushing_out);
        assert!(on.at_exit(PointerAt { x: 0.0, y: 1200.0 }));

        let mut left = OnPhone::new(phone(), Side::Left.phone_edge(), 0.0);
        assert!(left.moved(10, 0).pushing_out, "comes in at the right edge, which faces the PC");
        assert!(left.at_exit(PointerAt { x: 1079.0, y: 0.0 }));
    }
}
