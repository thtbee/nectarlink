// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS activity detection and synthetic input stubs.

use nectarlink_core::{
    ButtonAction, GestureAction, KeyMod, MouseButton, RemoteInfo, SlideAction, TouchAction,
};

pub fn move_relative(_dx: f32, _dy: f32) {}
pub fn move_precise(_dx: f32, _dy: f32) {}
#[allow(dead_code)]
pub fn move_absolute(_x: f32, _y: f32) {}
pub fn move_virtual_desktop(_x: f32, _y: f32, _click: Option<MouseButton>) {}
#[allow(dead_code)]
pub fn mirror_touch(_action: TouchAction, _x: f32, _y: f32) {}
pub fn mouse_button(_button: MouseButton, _action: ButtonAction) {}
pub fn modified_button(_button: MouseButton, _action: ButtonAction, _mods: &[KeyMod]) {}
pub fn hold_mods(_mods: &[KeyMod], _down: bool) {}
pub fn release_held_buttons() {}
pub fn scroll(_dx: f32, _dy: f32) {}
pub fn zoom(_delta: f32) {}
pub fn type_text(_text: &str) {}
pub fn press_key(_key: &str, _mods: &[KeyMod]) {}
pub fn slide(_action: SlideAction) {}
pub fn gesture(_action: GestureAction) {}
pub fn remote_info() -> RemoteInfo {
    RemoteInfo { profile: "macos".to_owned(), supported_actions: Vec::new(), monitors: Vec::new() }
}
// This read-only CoreGraphics function accepts value arguments and has no
// pointer or thread preconditions. It does not request Accessibility access.
#[allow(unsafe_code)]
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    safe fn CGEventSourceSecondsSinceLastEventType(state_id: i32, event_type: u32) -> f64;
}

/// Whether the session received keyboard, mouse or trackpad input recently.
pub fn is_user_active_within_ms(max_idle_ms: u32) -> bool {
    // kCGEventSourceStateCombinedSessionState and kCGAnyInputEventType.
    let idle_secs = CGEventSourceSecondsSinceLastEventType(0, u32::MAX);
    active_within_ms(idle_secs, max_idle_ms)
}

fn active_within_ms(idle_secs: f64, max_idle_ms: u32) -> bool {
    !idle_secs.is_finite() || idle_secs < 0.0 || idle_secs * 1000.0 <= f64::from(max_idle_ms)
}

#[cfg(test)]
mod tests {
    use super::active_within_ms;

    #[test]
    fn activity_window_includes_its_boundary() {
        assert!(active_within_ms(0.0, 0));
        assert!(active_within_ms(29.999, 30_000));
        assert!(active_within_ms(30.0, 30_000));
        assert!(!active_within_ms(30.001, 30_000));
    }

    #[test]
    fn invalid_idle_readings_count_as_activity() {
        for idle in [-1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(active_within_ms(idle, 0));
        }
    }
}
