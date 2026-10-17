// SPDX-License-Identifier: GPL-3.0-or-later
//! Synthesizes mouse and keyboard input on Linux (`xdotool`, `wtype`, `ydotool`)
//! for a paired phone acting as a touchpad, keyboard, or presentation remote
//! (`docs/protocol/remote.md`), as well as reverse PC mirror touch injection.

use std::{
    process::{Command, Stdio},
    sync::Mutex,
};

use nectarlink_core::{ButtonAction, KeyMod, MouseButton, PowerAction, SlideAction, TouchAction};

/// Fractional remainders for sub-pixel pointer motion and sub-notch scrolling,
/// plus which mouse buttons are currently held down by remote input.
#[derive(Default, Debug, Clone, PartialEq)]
struct InputState {
    move_rem_x: f32,
    move_rem_y: f32,
    scroll_rem_x: f32,
    scroll_rem_y: f32,
    held_left: bool,
    held_right: bool,
    held_middle: bool,
}

impl InputState {
    fn accumulate_move(&mut self, dx: f32, dy: f32) -> (i32, i32) {
        let total_x = self.move_rem_x + dx;
        let total_y = self.move_rem_y + dy;
        let ix = total_x.round() as i32;
        let iy = total_y.round() as i32;
        self.move_rem_x = total_x - (ix as f32);
        self.move_rem_y = total_y - (iy as f32);
        (ix, iy)
    }

    fn accumulate_scroll(&mut self, dx: f32, dy: f32) -> (i32, i32) {
        let total_x = self.scroll_rem_x + dx;
        let total_y = self.scroll_rem_y + dy;
        let nx = total_x.round() as i32;
        let ny = total_y.round() as i32;
        self.scroll_rem_x = total_x - (nx as f32);
        self.scroll_rem_y = total_y - (ny as f32);
        (nx, ny)
    }

    fn record_button(&mut self, button: MouseButton, action: ButtonAction) {
        let slot = match button {
            MouseButton::Left => &mut self.held_left,
            MouseButton::Right => &mut self.held_right,
            MouseButton::Middle => &mut self.held_middle,
        };
        *slot = matches!(action, ButtonAction::Down);
    }

    fn take_held_and_reset(&mut self) -> (bool, bool, bool) {
        let held = (self.held_left, self.held_right, self.held_middle);
        self.held_left = false;
        self.held_right = false;
        self.held_middle = false;
        self.move_rem_x = 0.0;
        self.move_rem_y = 0.0;
        self.scroll_rem_x = 0.0;
        self.scroll_rem_y = 0.0;
        held
    }
}

static STATE: Mutex<InputState> = Mutex::new(InputState {
    move_rem_x: 0.0,
    move_rem_y: 0.0,
    scroll_rem_x: 0.0,
    scroll_rem_y: 0.0,
    held_left: false,
    held_right: false,
    held_middle: false,
});

fn has_display_env() -> bool {
    std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

fn is_wayland_only() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("DISPLAY").is_none()
}

fn run_quiet(program: &str, args: &[&str]) -> bool {
    if !has_display_env() {
        return false;
    }
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn display_dimensions() -> (i32, i32) {
    if let Some((x, y, w, h)) = super::work_area() {
        let total_w = (x.max(0) + w).max(1);
        let total_h = (y.max(0) + h).max(1);
        if total_w > 0 && total_h > 0 {
            return (total_w, total_h);
        }
    }
    if has_display_env()
        && let Ok(out) = Command::new("xdotool")
            .arg("getdisplaygeometry")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
        && out.status.success()
    {
        let text = String::from_utf8_lossy(&out.stdout);
        let mut parts = text.split_whitespace().filter_map(|s| s.parse::<i32>().ok());
        if let (Some(w), Some(h)) = (parts.next(), parts.next())
            && w > 0
            && h > 0
        {
            return (w, h);
        }
    }
    (1920, 1080)
}

fn normalized_to_pixels(x: f32, y: f32, width: i32, height: i32) -> (i32, i32) {
    let max_x = (width - 1).max(1) as f32;
    let max_y = (height - 1).max(1) as f32;
    let px = (x.clamp(0.0, 1.0) * max_x).round() as i32;
    let py = (y.clamp(0.0, 1.0) * max_y).round() as i32;
    (px, py)
}

/// Moves the mouse cursor relatively by `(dx, dy)` pixels, keeping sub-pixel
/// fractions for the next move.
pub fn move_relative(dx: f32, dy: f32) {
    if !dx.is_finite() || !dy.is_finite() {
        return;
    }
    let (ix, iy) = {
        let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
        s.accumulate_move(dx, dy)
    };
    if ix == 0 && iy == 0 {
        return;
    }
    let sx = ix.to_string();
    let sy = iy.to_string();
    if !run_quiet("xdotool", &["mousemove_relative", "--", &sx, &sy]) {
        let _ = run_quiet("ydotool", &["mousemove", "-x", &sx, "-y", &sy]);
    }
}

/// Moves the mouse cursor to normalized coordinates `(x, y)` on the primary
/// display (`0.0..=1.0`).
pub fn move_absolute(x: f32, y: f32) {
    if !x.is_finite() || !y.is_finite() {
        return;
    }
    let (w, h) = display_dimensions();
    let (px, py) = normalized_to_pixels(x, y, w, h);
    let sx = px.to_string();
    let sy = py.to_string();
    if !run_quiet("xdotool", &["mousemove", "--", &sx, &sy]) {
        let _ = run_quiet("ydotool", &["mousemove", "--absolute", "-x", &sx, "-y", &sy]);
    }
}

/// Injects a normalized touch event (`0.0..=1.0`) from reverse PC mirroring as
/// absolute mouse movement + left button press/release.
pub fn mirror_touch(action: TouchAction, x: f32, y: f32) {
    if !x.is_finite() || !y.is_finite() {
        return;
    }
    let (w, h) = display_dimensions();
    let (px, py) = normalized_to_pixels(x, y, w, h);
    let sx = px.to_string();
    let sy = py.to_string();

    match action {
        TouchAction::Down => {
            if let Ok(mut s) = STATE.lock() {
                s.held_left = true;
            }
            if !run_quiet("xdotool", &["mousemove", "--", &sx, &sy, "mousedown", "1"]) {
                move_absolute(x, y);
                mouse_button(MouseButton::Left, ButtonAction::Down);
            }
        }
        TouchAction::Move => {
            move_absolute(x, y);
        }
        TouchAction::Up => {
            if let Ok(mut s) = STATE.lock() {
                s.held_left = false;
            }
            if !run_quiet("xdotool", &["mousemove", "--", &sx, &sy, "mouseup", "1"]) {
                move_absolute(x, y);
                mouse_button(MouseButton::Left, ButtonAction::Up);
            }
        }
    }
}

fn xdotool_button_id(button: MouseButton) -> &'static str {
    match button {
        MouseButton::Left => "1",
        MouseButton::Middle => "2",
        MouseButton::Right => "3",
    }
}

/// Presses, releases or clicks a mouse button.
pub fn mouse_button(button: MouseButton, action: ButtonAction) {
    {
        let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
        s.record_button(button, action);
    }
    let btn = xdotool_button_id(button);
    let subcmd = match action {
        ButtonAction::Click => "click",
        ButtonAction::Down => "mousedown",
        ButtonAction::Up => "mouseup",
    };
    if !run_quiet("xdotool", &[subcmd, btn]) {
        let ydo_code = match (button, action) {
            (MouseButton::Left, ButtonAction::Click) => "0xC0",
            (MouseButton::Left, ButtonAction::Down) => "0x40",
            (MouseButton::Left, ButtonAction::Up) => "0x80",
            (MouseButton::Right, ButtonAction::Click) => "0xC1",
            (MouseButton::Right, ButtonAction::Down) => "0x41",
            (MouseButton::Right, ButtonAction::Up) => "0x81",
            (MouseButton::Middle, ButtonAction::Click) => "0xC2",
            (MouseButton::Middle, ButtonAction::Down) => "0x42",
            (MouseButton::Middle, ButtonAction::Up) => "0x82",
        };
        let _ = run_quiet("ydotool", &["click", ydo_code]);
    }
}

/// Releases any mouse buttons still held down by remote input.
pub fn release_held_buttons() {
    let (left, right, middle) = {
        let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
        s.take_held_and_reset()
    };
    if left {
        let _ = run_quiet("xdotool", &["mouseup", "1"]);
    }
    if middle {
        let _ = run_quiet("xdotool", &["mouseup", "2"]);
    }
    if right {
        let _ = run_quiet("xdotool", &["mouseup", "3"]);
    }
}

/// Scrolls the mouse wheel smoothly (`dy > 0` scrolls down, `dx > 0` scrolls right).
pub fn scroll(dx: f32, dy: f32) {
    if !dx.is_finite() || !dy.is_finite() {
        return;
    }
    let (nx, ny) = {
        let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
        s.accumulate_scroll(dx, dy)
    };
    if ny != 0 {
        // X11 button 4 = wheel up, button 5 = wheel down.
        let btn = if ny > 0 { "5" } else { "4" };
        let count = ny.unsigned_abs().to_string();
        if !run_quiet("xdotool", &["click", "--repeat", &count, "--delay", "1", btn]) {
            let wheel_y = (-ny).to_string();
            let _ = run_quiet("ydotool", &["mousemove", "--wheel", "-x", "0", "-y", &wheel_y]);
        }
    }
    if nx != 0 {
        // X11 button 6 = wheel left, button 7 = wheel right.
        let btn = if nx > 0 { "7" } else { "6" };
        let count = nx.unsigned_abs().to_string();
        if !run_quiet("xdotool", &["click", "--repeat", &count, "--delay", "1", btn]) {
            let wheel_x = nx.to_string();
            let _ = run_quiet("ydotool", &["mousemove", "--wheel", "-x", &wheel_x, "-y", "0"]);
        }
    }
}

/// Types arbitrary Unicode text (filtering carriage returns `\r`).
pub fn type_text(text: &str) {
    let cleaned: String = text.chars().filter(|&c| c != '\r').collect();
    if cleaned.is_empty() {
        return;
    }
    if is_wayland_only() && run_quiet("wtype", &["--", &cleaned]) {
        return;
    }
    if !run_quiet("xdotool", &["type", "--clearmodifiers", "--delay", "0", "--", &cleaned]) {
        let _ = run_quiet("wtype", &["--", &cleaned]);
    }
}

/// Resolves a protocol key or shortcut name into `(keysym, extra_mods)`.
pub(crate) fn resolve_key(key: &str) -> Option<(String, &'static [KeyMod])> {
    const CTRL: &[KeyMod] = &[KeyMod::Ctrl];
    const WIN: &[KeyMod] = &[KeyMod::Win];
    const NONE: &[KeyMod] = &[];

    let trimmed = key.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    let (sym, mods): (&str, &'static [KeyMod]) = match lower.as_str() {
        "copy" => ("c", CTRL),
        "paste" => ("v", CTRL),
        "cut" => ("x", CTRL),
        "undo" => ("z", CTRL),
        "redo" => ("y", CTRL),
        "select_all" => ("a", CTRL),
        "task_view" => ("Tab", WIN),
        "enter" | "return" => ("Return", NONE),
        "backspace" => ("BackSpace", NONE),
        "tab" => ("Tab", NONE),
        "escape" | "esc" => ("Escape", NONE),
        "space" => ("space", NONE),
        "delete" | "del" => ("Delete", NONE),
        "insert" | "ins" => ("Insert", NONE),
        "up" => ("Up", NONE),
        "down" => ("Down", NONE),
        "left" => ("Left", NONE),
        "right" => ("Right", NONE),
        "home" => ("Home", NONE),
        "end" => ("End", NONE),
        "page_up" | "pageup" | "pgup" | "prior" => ("Prior", NONE),
        "page_down" | "pagedown" | "pgdn" | "next" => ("Next", NONE),
        "print" | "printscreen" | "sysrq" => ("Print", NONE),
        "f1" => ("F1", NONE),
        "f2" => ("F2", NONE),
        "f3" => ("F3", NONE),
        "f4" => ("F4", NONE),
        "f5" => ("F5", NONE),
        "f6" => ("F6", NONE),
        "f7" => ("F7", NONE),
        "f8" => ("F8", NONE),
        "f9" => ("F9", NONE),
        "f10" => ("F10", NONE),
        "f11" => ("F11", NONE),
        "f12" => ("F12", NONE),
        "volume_up" | "xf86audioraisevolume" => ("XF86AudioRaiseVolume", NONE),
        "volume_down" | "xf86audiolowervolume" => ("XF86AudioLowerVolume", NONE),
        "volume_mute" | "xf86audiomute" => ("XF86AudioMute", NONE),
        "play_pause" | "xf86audioplay" => ("XF86AudioPlay", NONE),
        "next_track" | "xf86audionext" => ("XF86AudioNext", NONE),
        "prev_track" | "xf86audioprev" => ("XF86AudioPrev", NONE),
        s if s.len() == 1 => {
            let b = s.as_bytes()[0];
            if b.is_ascii_lowercase() || b.is_ascii_digit() {
                return Some((s.to_owned(), NONE));
            }
            return None;
        }
        _ => return None,
    };
    Some((sym.to_owned(), mods))
}

fn mod_name(m: KeyMod) -> &'static str {
    match m {
        KeyMod::Ctrl => "ctrl",
        KeyMod::Alt => "alt",
        KeyMod::Shift => "shift",
        KeyMod::Win => "super",
    }
}

/// Formats an `xdotool key` combo string (e.g. `"ctrl+shift+k"` or `"super+Tab"`)
/// for a protocol key and modifier list.
pub(crate) fn format_xdotool_combo(key: &str, mods: &[KeyMod]) -> Option<String> {
    let (sym, preset_mods) = resolve_key(key)?;
    let mut all_mods: Vec<KeyMod> = Vec::with_capacity(4);
    for &m in preset_mods.iter().chain(mods.iter()) {
        if !all_mods.contains(&m) {
            all_mods.push(m);
        }
    }
    if all_mods.is_empty() {
        return Some(sym);
    }
    let mut parts: Vec<&str> = all_mods.iter().map(|&m| mod_name(m)).collect();
    parts.push(&sym);
    Some(parts.join("+"))
}

/// Presses a named key or shortcut with optional modifiers.
pub fn press_key(key: &str, mods: &[KeyMod]) {
    if key.eq_ignore_ascii_case("lock") {
        let _ = crate::links::power(PowerAction::Lock);
        return;
    }
    let Some(combo) = format_xdotool_combo(key, mods) else {
        return;
    };
    if is_wayland_only()
        && let Some((sym, preset_mods)) = resolve_key(key)
    {
        let mut wtype_args: Vec<String> = Vec::new();
        for &m in preset_mods.iter().chain(mods.iter()) {
            let wm = match m {
                KeyMod::Ctrl => "ctrl",
                KeyMod::Alt => "alt",
                KeyMod::Shift => "shift",
                KeyMod::Win => "logo",
            };
            wtype_args.push("-M".into());
            wtype_args.push(wm.into());
        }
        wtype_args.push("-k".into());
        wtype_args.push(sym);
        let refs: Vec<&str> = wtype_args.iter().map(String::as_str).collect();
        if run_quiet("wtype", &refs) {
            return;
        }
    }
    let _ = run_quiet("xdotool", &["key", "--clearmodifiers", &combo]);
}

/// Executes a presentation slide action.
pub fn slide(action: SlideAction) {
    let key = match action {
        SlideAction::Next => "page_down",
        SlideAction::Previous => "page_up",
        SlideAction::Start => "f5",
        SlideAction::Stop => "escape",
        SlideAction::Black => "b",
    };
    press_key(key, &[]);
}

/// Parses idle milliseconds from `xprintidle` output (`"8801\n"`) or D-Bus
/// `GetIdletime` / `GetSessionIdleTime` output (`"(uint64 8801,)"` or `"(uint32 8,)"`).
pub(crate) fn parse_idle_output_ms(raw: &str, is_seconds: bool) -> Option<u64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(val) = trimmed.parse::<u64>() {
        return Some(if is_seconds { val.saturating_mul(1000) } else { val });
    }
    // Extract the first integer token from gdbus tuple output like "(uint64 8801,)" or "(uint32 12,)".
    for token in trimmed.split(|c: char| !c.is_ascii_digit()) {
        if !token.is_empty()
            && token != "64"
            && token != "32"
            && let Ok(val) = token.parse::<u64>()
        {
            return Some(if is_seconds { val.saturating_mul(1000) } else { val });
        }
    }
    None
}

fn query_idle_ms() -> Option<u64> {
    if !has_display_env() {
        return None;
    }
    if let Ok(out) = Command::new("xprintidle").stdin(Stdio::null()).stderr(Stdio::null()).output()
        && out.status.success()
        && let Some(ms) = parse_idle_output_ms(&String::from_utf8_lossy(&out.stdout), false)
    {
        return Some(ms);
    }
    if let Ok(out) = Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.ScreenSaver",
            "--object-path",
            "/org/freedesktop/ScreenSaver",
            "--method",
            "org.freedesktop.ScreenSaver.GetSessionIdleTime",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        && out.status.success()
    {
        // org.freedesktop.ScreenSaver.GetSessionIdleTime returns seconds (or ms on some KDE builds).
        if let Some(ms) = parse_idle_output_ms(&String::from_utf8_lossy(&out.stdout), false) {
            return Some(ms);
        }
    }
    None
}

/// Returns `true` when the user has moved the mouse or pressed a key on this PC
/// within the last `max_idle_ms` milliseconds.
pub fn is_user_active_within_ms(max_idle_ms: u32) -> bool {
    match query_idle_ms() {
        Some(idle_ms) => idle_ms <= u64::from(max_idle_ms),
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_named_keys_and_shortcuts() {
        assert_eq!(resolve_key("enter"), Some(("Return".to_owned(), &[][..])));
        assert_eq!(resolve_key("up"), Some(("Up".to_owned(), &[][..])));
        assert_eq!(resolve_key("copy"), Some(("c".to_owned(), &[KeyMod::Ctrl][..])));
        assert_eq!(resolve_key("paste"), Some(("v".to_owned(), &[KeyMod::Ctrl][..])));
        assert_eq!(resolve_key("task_view"), Some(("Tab".to_owned(), &[KeyMod::Win][..])));
        assert_eq!(resolve_key("a"), Some(("a".to_owned(), &[][..])));
        assert_eq!(resolve_key("A"), Some(("a".to_owned(), &[][..])));
        assert_eq!(resolve_key("5"), Some(("5".to_owned(), &[][..])));
        assert_eq!(resolve_key("f5"), Some(("F5".to_owned(), &[][..])));
        assert_eq!(resolve_key("play_pause"), Some(("XF86AudioPlay".to_owned(), &[][..])));
        assert_eq!(resolve_key("volume_up"), Some(("XF86AudioRaiseVolume".to_owned(), &[][..])));
        assert_eq!(resolve_key("print"), Some(("Print".to_owned(), &[][..])));
        assert!(resolve_key("unknown_key").is_none());
    }

    #[test]
    fn formats_xdotool_combos_with_preset_and_explicit_modifiers() {
        assert_eq!(format_xdotool_combo("copy", &[]), Some("ctrl+c".to_owned()));
        assert_eq!(
            format_xdotool_combo("k", &[KeyMod::Ctrl, KeyMod::Shift]),
            Some("ctrl+shift+k".to_owned())
        );
        assert_eq!(format_xdotool_combo("tab", &[KeyMod::Alt]), Some("alt+Tab".to_owned()));
        assert_eq!(format_xdotool_combo("d", &[KeyMod::Win]), Some("super+d".to_owned()));
        assert_eq!(format_xdotool_combo("f5", &[]), Some("F5".to_owned()));
    }

    #[test]
    fn accumulates_subpixel_move_and_scroll_remainders() {
        let mut s = InputState::default();
        assert_eq!(s.accumulate_move(0.4, -0.4), (0, 0));
        assert_eq!(s.accumulate_move(0.4, -0.4), (1, -1));
        assert_eq!(s.accumulate_move(0.4, -0.4), (0, 0));

        assert_eq!(s.accumulate_scroll(0.3, 0.3), (0, 0));
        assert_eq!(s.accumulate_scroll(0.3, 0.3), (1, 1));
    }

    #[test]
    fn tracks_held_mouse_buttons_and_resets_on_release() {
        let mut s = InputState::default();
        s.record_button(MouseButton::Left, ButtonAction::Down);
        s.record_button(MouseButton::Right, ButtonAction::Click);
        assert!(s.held_left);
        assert!(!s.held_right);

        let held = s.take_held_and_reset();
        assert_eq!(held, (true, false, false));
        assert!(!s.held_left);
    }

    #[test]
    fn maps_normalized_coordinates_and_parses_idle_output() {
        assert_eq!(normalized_to_pixels(0.0, 0.0, 1920, 1080), (0, 0));
        assert_eq!(normalized_to_pixels(1.0, 1.0, 1920, 1080), (1919, 1079));
        assert_eq!(normalized_to_pixels(0.5, 0.5, 1921, 1081), (960, 540));

        assert_eq!(parse_idle_output_ms("8801\n", false), Some(8801));
        assert_eq!(parse_idle_output_ms("(uint64 4250,)", false), Some(4250));
        assert_eq!(parse_idle_output_ms("12", true), Some(12_000));
    }
}
