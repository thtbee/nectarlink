use std::process::Command;

use nectarlink_core::{ButtonAction, KeyMod, MouseButton, SlideAction, TouchAction};

pub fn move_relative(dx: f32, dy: f32) {
    let x = dx.round() as i32;
    let y = dy.round() as i32;
    if x == 0 && y == 0 {
        return;
    }
    let _ = Command::new("xdotool")
        .args(["mousemove_relative", "--", &x.to_string(), &y.to_string()])
        .status();
}

pub fn move_absolute(_x: f32, _y: f32) {}

pub fn mirror_touch(_action: TouchAction, _x: f32, _y: f32) {}

pub fn mouse_button(button: MouseButton, action: ButtonAction) {
    let btn = match button {
        MouseButton::Left => "1",
        MouseButton::Middle => "2",
        MouseButton::Right => "3",
    };
    let subcmd = match action {
        ButtonAction::Click => "click",
        ButtonAction::Down => "mousedown",
        ButtonAction::Up => "mouseup",
    };
    let _ = Command::new("xdotool").args([subcmd, btn]).status();
}

pub fn release_held_buttons() {}

pub fn scroll(_dx: f32, _dy: f32) {}

pub fn type_text(text: &str) {
    if text.is_empty() {
        return;
    }
    let _ = Command::new("xdotool")
        .args(["type", "--clearmodifiers", "--", text])
        .status();
}

pub fn press_key(key: &str, _mods: &[KeyMod]) {
    if key.is_empty() {
        return;
    }
    let _ = Command::new("xdotool").args(["key", key]).status();
}

pub fn slide(action: SlideAction) {
    let key = match action {
        SlideAction::Next => "Next",
        SlideAction::Previous => "Prior",
        SlideAction::Start => "F5",
        SlideAction::Stop => "Escape",
        SlideAction::Black => "b",
    };
    press_key(key, &[]);
}

pub fn is_user_active_within_ms(_max_idle_ms: u32) -> bool {
    true
}
