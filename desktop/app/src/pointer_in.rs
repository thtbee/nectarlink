// SPDX-License-Identifier: GPL-3.0-or-later
//! Another PC's mouse on this one (D432): it comes in at the edge facing that
//! PC, moves this PC's cursor across the main screen, clicks and scrolls, and
//! goes back when it's pushed against that edge again (the other PC decides,
//! from where this one says the cursor is).
//!
//! The cursor is placed where it should be rather than nudged, so this PC's
//! pointer speed and acceleration don't change the distances the other PC
//! counts. Its keys come as remote input (`remote.input`), like a phone's
//! keyboard.

use std::sync::Mutex;

use nectarlink_core::{
    ButtonAction, DeviceId, MouseButton, PointerAt, PointerEdge, PointerMove, PointerScreen,
};

use crate::win;

/// The other PC's mouse while it's here.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Here {
    from: DeviceId,
    width: i32,
    height: i32,
    x: f32,
    y: f32,
    /// Bits: 1 left, 2 right, 4 middle.
    buttons: u8,
}

static HERE: Mutex<Option<Here>> = Mutex::new(None);

fn here() -> std::sync::MutexGuard<'static, Option<Here>> {
    HERE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Where `(x, y)` lands on a `width` by `height` screen, as `move_absolute` takes it.
fn fraction(x: f32, y: f32, width: i32, height: i32) -> (f32, f32) {
    let span = |n: i32| ((n - 1).max(1)) as f32;
    ((x / span(width)).clamp(0.0, 1.0), (y / span(height)).clamp(0.0, 1.0))
}

/// The other PC's mouse comes in at `edge`, `y` of the way down.
pub fn enter(from: &DeviceId, edge: PointerEdge, y: f32) -> Result<PointerScreen, String> {
    let (width, height) = win::input::main_screen_size().ok_or("this PC can't place its cursor")?;
    if width <= 0 || height <= 0 {
        return Err("no screen".into());
    }
    let x = match edge {
        PointerEdge::Left => 0.0,
        PointerEdge::Right => (width - 1) as f32,
    };
    let y = y.clamp(0.0, 1.0) * (height - 1) as f32;
    // Anything still held for a PC that left without saying goes first.
    leave_now();
    *here() = Some(Here { from: *from, width, height, x, y, buttons: 0 });
    let (fx, fy) = fraction(x, y, width, height);
    win::input::move_to_main(fx, fy);
    Ok(PointerScreen { width: width as u32, height: height as u32, scale: 1.0 })
}

/// The other PC's mouse moved, pressed or let go of a button, or scrolled.
pub fn moved(from: &DeviceId, moved: PointerMove) {
    let (place, presses, wheel) = {
        let mut guard = here();
        let Some(state) = guard.as_mut().filter(|s| s.from == *from) else { return };
        let mut place = None;
        if moved.dx != 0 || moved.dy != 0 {
            state.x = (state.x + moved.dx as f32).clamp(0.0, (state.width - 1) as f32);
            state.y = (state.y + moved.dy as f32).clamp(0.0, (state.height - 1) as f32);
            place = Some(fraction(state.x, state.y, state.width, state.height));
        }
        let changed = state.buttons ^ moved.buttons;
        state.buttons = moved.buttons;
        let presses: Vec<(MouseButton, ButtonAction)> =
            [(1, MouseButton::Left), (2, MouseButton::Right), (4, MouseButton::Middle)]
                .into_iter()
                .filter(|(bit, _)| changed & bit != 0)
                .map(|(bit, button)| {
                    (button, if moved.buttons & bit != 0 { ButtonAction::Down } else { ButtonAction::Up })
                })
                .collect();
        (place, presses, moved.wheel)
    };
    // Moved first, so a press lands where the cursor is now.
    if let Some((fx, fy)) = place {
        win::input::move_to_main(fx, fy);
    }
    for (button, action) in presses {
        win::input::mouse_button(button, action);
    }
    if wheel != 0 {
        // A notch forward on the wheel scrolls up, which `scroll` takes as negative.
        win::input::scroll(0.0, -f32::from(wheel));
    }
}

/// Where the other PC's mouse is, in pixels of the screen it was given.
pub fn at(from: &DeviceId) -> Option<PointerAt> {
    here().filter(|s| s.from == *from).map(|s| PointerAt { x: s.x, y: s.y })
}

/// The other PC's mouse went back: nothing stays held.
pub fn leave(from: &DeviceId) {
    if here().is_some_and(|s| s.from == *from) {
        leave_now();
    }
}

fn leave_now() {
    let held = here().take().map(|s| s.buttons).unwrap_or(0);
    if held != 0 {
        win::input::release_held_buttons();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_land_on_the_screen() {
        assert_eq!(fraction(0.0, 0.0, 1920, 1080), (0.0, 0.0));
        assert_eq!(fraction(1919.0, 1079.0, 1920, 1080), (1.0, 1.0));
        assert_eq!(fraction(5000.0, -3.0, 1920, 1080), (1.0, 0.0));
        let (x, _) = fraction(959.5, 0.0, 1920, 1080);
        assert!((x - 0.5).abs() < 0.001);
    }
}
