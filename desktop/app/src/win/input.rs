// SPDX-License-Identifier: GPL-3.0-or-later
//! Synthesizes mouse and keyboard input on Windows (`SendInput`) for a paired
//! phone acting as a touchpad, keyboard or presentation remote
//! (`docs/protocol/remote.md`).

use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use nectarlink_core::{ButtonAction, KeyMod, MouseButton, PowerAction, SlideAction, TouchAction};
use windows::Win32::{
    Foundation::{LPARAM, WPARAM},
    UI::{
        Input::{
            Ime::ImmGetDefaultIMEWnd,
            KeyboardAndMouse::{
                GetKeyState, GetKeyboardLayout, HKL, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE,
                KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
                MAPVK_VK_TO_VSC, MOUSE_EVENT_FLAGS, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL,
                MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
                MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEINPUT,
                MapVirtualKeyExW, SendInput, ToUnicodeEx, VIRTUAL_KEY, VK_A, VK_B, VK_BACK, VK_C, VK_CAPITAL,
                VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_F5, VK_HOME, VK_INSERT, VK_LEFT,
                VK_LWIN, VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE, VK_MEDIA_PREV_TRACK, VK_MENU, VK_NEXT,
                VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP, VK_V, VK_VOLUME_DOWN,
                VK_VOLUME_MUTE, VK_VOLUME_UP, VK_X, VK_Y, VK_Z, VkKeyScanExW,
            },
        },
        WindowsAndMessaging::{
            GetForegroundWindow, GetWindowThreadProcessId, SMTO_ABORTIFHUNG, SendMessageTimeoutW,
            WHEEL_DELTA, WM_IME_CONTROL,
        },
    },
};

/// Fractional remainders for sub-pixel pointer motion and sub-notch scrolling,
/// plus which mouse buttons are currently held down by remote input.
#[derive(Default)]
struct InputState {
    move_rem_x: f32,
    move_rem_y: f32,
    scroll_rem_x: f32,
    scroll_rem_y: f32,
    held_left: bool,
    held_right: bool,
    held_middle: bool,
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

fn send_inputs(inputs: &[INPUT]) {
    if inputs.is_empty() {
        return;
    }
    // SAFETY: `inputs` is a valid slice of `INPUT` structs.
    unsafe {
        let _ = SendInput(inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

fn mouse_input(dx: i32, dy: i32, mouse_data: i32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT { dx, dy, mouseData: mouse_data as u32, dwFlags: flags, time: 0, dwExtraInfo: 0 },
        },
    }
}

fn vk_input(vk: VIRTUAL_KEY, extended: bool, up: bool) -> INPUT {
    let mut flags = KEYBD_EVENT_FLAGS(0);
    if extended {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    }
}

/// A key of the keyboard layout, with its scan code, as a real key press sends it.
fn layout_key_input(vk: VIRTUAL_KEY, scan: u16, up: bool) -> INPUT {
    let flags = if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) };
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 },
        },
    }
}

fn unicode_input(unit: u16, up: bool) -> INPUT {
    let mut flags = KEYEVENTF_UNICODE;
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: VIRTUAL_KEY(0), wScan: unit, dwFlags: flags, time: 0, dwExtraInfo: 0 },
        },
    }
}

/// Moves the mouse cursor relatively by `(dx, dy)` pixels, keeping fractions
/// for the next move. Windows applies the user's pointer speed (and "enhance
/// pointer precision") to relative motion itself.
pub fn move_relative(dx: f32, dy: f32) {
    if !dx.is_finite() || !dy.is_finite() {
        return;
    }
    let (ix, iy) = {
        let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let total_x = s.move_rem_x + dx;
        let total_y = s.move_rem_y + dy;
        let ix = total_x.round() as i32;
        let iy = total_y.round() as i32;
        s.move_rem_x = total_x - (ix as f32);
        s.move_rem_y = total_y - (iy as f32);
        (ix, iy)
    };
    if ix != 0 || iy != 0 {
        send_inputs(&[mouse_input(ix, iy, 0, MOUSEEVENTF_MOVE)]);
    }
}

/// Moves the mouse cursor to normalized coordinates `(x, y)` on the primary
/// display (`0.0..=1.0`).
pub fn move_absolute(x: f32, y: f32) {
    if !x.is_finite() || !y.is_finite() {
        return;
    }
    let ax = (x.clamp(0.0, 1.0) * 65535.0).round() as i32;
    let ay = (y.clamp(0.0, 1.0) * 65535.0).round() as i32;
    send_inputs(&[mouse_input(ax, ay, 0, MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE)]);
}

/// Injects a normalized touch event (`0.0..=1.0`) from reverse PC mirroring as
/// absolute mouse movement + left button press/release.
pub fn mirror_touch(action: TouchAction, x: f32, y: f32) {
    if !x.is_finite() || !y.is_finite() {
        return;
    }
    let ax = (x.clamp(0.0, 1.0) * 65535.0).round() as i32;
    let ay = (y.clamp(0.0, 1.0) * 65535.0).round() as i32;
    let flags = match action {
        TouchAction::Down => {
            if let Ok(mut s) = STATE.lock() {
                s.held_left = true;
            }
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_LEFTDOWN
        }
        TouchAction::Move => MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE,
        TouchAction::Up => {
            if let Ok(mut s) = STATE.lock() {
                s.held_left = false;
            }
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_LEFTUP
        }
    };
    send_inputs(&[mouse_input(ax, ay, 0, flags)]);
}

/// Presses, releases or clicks a mouse button.
pub fn mouse_button(button: MouseButton, action: ButtonAction) {
    let (down_flag, up_flag) = match button {
        MouseButton::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
        MouseButton::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
        MouseButton::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
    };
    {
        let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let slot = match button {
            MouseButton::Left => &mut s.held_left,
            MouseButton::Right => &mut s.held_right,
            MouseButton::Middle => &mut s.held_middle,
        };
        *slot = matches!(action, ButtonAction::Down);
    }
    match action {
        ButtonAction::Down => send_inputs(&[mouse_input(0, 0, 0, down_flag)]),
        ButtonAction::Up => send_inputs(&[mouse_input(0, 0, 0, up_flag)]),
        ButtonAction::Click => {
            send_inputs(&[mouse_input(0, 0, 0, down_flag), mouse_input(0, 0, 0, up_flag)]);
        }
    }
}

/// Releases any mouse buttons still held down by remote input.
pub fn release_held_buttons() {
    let (left, right, middle) = {
        let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let held = (s.held_left, s.held_right, s.held_middle);
        s.held_left = false;
        s.held_right = false;
        s.held_middle = false;
        s.move_rem_x = 0.0;
        s.move_rem_y = 0.0;
        s.scroll_rem_x = 0.0;
        s.scroll_rem_y = 0.0;
        held
    };
    let mut inputs = Vec::new();
    if left {
        inputs.push(mouse_input(0, 0, 0, MOUSEEVENTF_LEFTUP));
    }
    if right {
        inputs.push(mouse_input(0, 0, 0, MOUSEEVENTF_RIGHTUP));
    }
    if middle {
        inputs.push(mouse_input(0, 0, 0, MOUSEEVENTF_MIDDLEUP));
    }
    send_inputs(&inputs);
}

/// Scrolls the mouse wheel smoothly (`dy` > 0 scrolls down, `dx` > 0 scrolls right,
/// in wheel notches of `WHEEL_DELTA = 120`).
pub fn scroll(dx: f32, dy: f32) {
    if !dx.is_finite() || !dy.is_finite() {
        return;
    }
    let delta = WHEEL_DELTA as f32;
    let (wx, wy) = {
        let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
        // Protocol: positive dy = scroll down = negative Win32 wheel delta.
        // Protocol: positive dx = scroll right = positive Win32 horizontal wheel delta.
        let total_x = s.scroll_rem_x + dx * delta;
        let total_y = s.scroll_rem_y - dy * delta;
        let wx = total_x.round() as i32;
        let wy = total_y.round() as i32;
        s.scroll_rem_x = total_x - (wx as f32);
        s.scroll_rem_y = total_y - (wy as f32);
        (wx, wy)
    };
    let mut inputs = Vec::with_capacity(2);
    if wy != 0 {
        inputs.push(mouse_input(0, 0, wy, MOUSEEVENTF_WHEEL));
    }
    if wx != 0 {
        inputs.push(mouse_input(0, 0, wx, MOUSEEVENTF_HWHEEL));
    }
    send_inputs(&inputs);
}

/// Types arbitrary Unicode text via `KEYEVENTF_UNICODE` (including surrogate
/// pairs for non-BMP characters like emoji).
pub fn type_text(text: &str) {
    let layout = typing_layout();
    // One character at a time: an app that reads a packet key late (Notepad on
    // Windows 11) can turn a batch into its last letter ("great" as "ttttt").
    for keystroke in keystrokes(text, layout) {
        wait_for_last_keystroke(BETWEEN_CHARACTERS);
        send_inputs(&keystroke);
    }
}

/// The keyboard layout of the window being typed into, and whether Caps Lock is on;
/// `None` when an input method there is on (it would turn keys into its own text).
fn typing_layout() -> Option<(HKL, bool)> {
    // SAFETY: plain Win32 queries; a missing window gives a null handle, which they accept.
    unsafe {
        let window = GetForegroundWindow();
        let thread = GetWindowThreadProcessId(window, None);
        let layout = GetKeyboardLayout(thread);
        let ime = ImmGetDefaultIMEWnd(window);
        if !ime.is_invalid() {
            // IMC_GETOPENSTATUS. No answer counts as on: the old way is always safe.
            let mut open = 0usize;
            let answered = SendMessageTimeoutW(
                ime,
                WM_IME_CONTROL,
                WPARAM(5),
                LPARAM(0),
                SMTO_ABORTIFHUNG,
                50,
                Some(&mut open),
            );
            if answered.0 == 0 || open != 0 {
                return None;
            }
        }
        let caps = GetKeyState(i32::from(VK_CAPITAL.0)) & 1 != 0;
        Some((layout, caps))
    }
}

/// The key and Shift that type `ch` on `layout`, checked by asking the layout what
/// that press gives: only plain and Shift presses, never a dead key or AltGr.
fn layout_key(ch: char, layout: HKL, caps: bool) -> Option<(VIRTUAL_KEY, u16, bool)> {
    if ch.is_control() {
        return None;
    }
    let mut buf = [0u16; 2];
    let [unit] = ch.encode_utf16(&mut buf) else { return None };
    let unit = *unit;
    // SAFETY: plain Win32 queries about a keyboard layout.
    unsafe {
        let found = VkKeyScanExW(unit, layout);
        if found == -1 {
            return None;
        }
        let [vk, mods] = (found as u16).to_le_bytes();
        if mods & !1 != 0 {
            return None;
        }
        let scan = MapVirtualKeyExW(u32::from(vk), MAPVK_VK_TO_VSC, Some(layout)) as u16;
        let gives = |shift: bool| {
            let mut state = [0u8; 256];
            if shift {
                state[usize::from(VK_SHIFT.0)] = 0x80;
            }
            if caps {
                state[usize::from(VK_CAPITAL.0)] = 0x01;
            }
            let mut out = [0u16; 4];
            // 4: leave the layout's dead-key state alone.
            ToUnicodeEx(u32::from(vk), u32::from(scan), &state, &mut out, 4, Some(layout)) == 1
                && out[0] == unit
        };
        // Caps Lock turns Shift around for letters, so try the other one too.
        let shift = mods & 1 != 0;
        if gives(shift) {
            Some((VIRTUAL_KEY(u16::from(vk)), scan, shift))
        } else if gives(!shift) {
            Some((VIRTUAL_KEY(u16::from(vk)), scan, !shift))
        } else {
            None
        }
    }
}

static NEXT_KEYSTROKE: Mutex<Option<Instant>> = Mutex::new(None);

/// Waits until the app has had its time since the last character or key press,
/// and books `gap` for this one. Voice typing rewrites a phrase with Backspaces
/// and then the new text, and a letter sent right after a Backspace could get
/// lost ("working" as "rking").
fn wait_for_last_keystroke(gap: Duration) {
    let mut next = NEXT_KEYSTROKE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(left) = next.and_then(|at| at.checked_duration_since(Instant::now())) {
        std::thread::sleep(left);
    }
    *next = Some(Instant::now() + gap);
}

/// How long the app gets to read one typed character before the next. In
/// Notepad on Windows 11, up to 6 ms still garbled words and 12 ms dropped a
/// letter; 16 ms was clean. It can't grow much: see the test below.
const BETWEEN_CHARACTERS: Duration = Duration::from_millis(16);
/// A key press takes the app longer than a character: Notepad on Windows 11 still
/// lost a letter now and then after 16 ms, here after Backspace or Enter.
const AFTER_KEY: Duration = Duration::from_millis(32);

/// The input events of each character in `text`, press and release together.
/// What the layout has a key for is typed with that key: Notepad on Windows 11
/// could still turn fast dictation into the wrong letters ("or not" as "tttttt")
/// when every letter came as a Unicode packet. The rest still comes as one.
fn keystrokes(text: &str, layout: Option<(HKL, bool)>) -> Vec<Vec<INPUT>> {
    let mut keystrokes = Vec::with_capacity(text.len());
    for ch in text.chars() {
        if ch == '\r' {
            continue;
        }
        if ch == '\n' {
            keystrokes.push(vec![vk_input(VK_RETURN, false, false), vk_input(VK_RETURN, false, true)]);
            continue;
        }
        if let Some((vk, scan, shift)) = layout.and_then(|(layout, caps)| layout_key(ch, layout, caps)) {
            let mut keystroke = Vec::with_capacity(4);
            if shift {
                keystroke.push(vk_input(VK_SHIFT, false, false));
            }
            keystroke.push(layout_key_input(vk, scan, false));
            keystroke.push(layout_key_input(vk, scan, true));
            if shift {
                keystroke.push(vk_input(VK_SHIFT, false, true));
            }
            keystrokes.push(keystroke);
            continue;
        }
        let mut buf = [0u16; 2];
        let units = ch.encode_utf16(&mut buf);
        let mut keystroke = Vec::with_capacity(units.len() * 2);
        for &unit in units.iter() {
            keystroke.push(unicode_input(unit, false));
        }
        for &unit in units.iter().rev() {
            keystroke.push(unicode_input(unit, true));
        }
        keystrokes.push(keystroke);
    }
    keystrokes
}

/// Resolves a protocol key or shortcut name into `(VIRTUAL_KEY, is_extended, extra_mods)`.
fn resolve_key(key: &str) -> Option<(VIRTUAL_KEY, bool, &'static [KeyMod])> {
    const CTRL: &[KeyMod] = &[KeyMod::Ctrl];
    const WIN: &[KeyMod] = &[KeyMod::Win];
    const NONE: &[KeyMod] = &[];

    Some(match key {
        "copy" => (VK_C, false, CTRL),
        "paste" => (VK_V, false, CTRL),
        "cut" => (VK_X, false, CTRL),
        "undo" => (VK_Z, false, CTRL),
        "redo" => (VK_Y, false, CTRL),
        "select_all" => (VK_A, false, CTRL),
        "task_view" => (VK_TAB, false, WIN),
        "enter" => (VK_RETURN, false, NONE),
        "backspace" => (VK_BACK, false, NONE),
        "tab" => (VK_TAB, false, NONE),
        "escape" => (VK_ESCAPE, false, NONE),
        "space" => (VK_SPACE, false, NONE),
        "delete" => (VK_DELETE, true, NONE),
        "insert" => (VK_INSERT, true, NONE),
        "up" => (VK_UP, true, NONE),
        "down" => (VK_DOWN, true, NONE),
        "left" => (VK_LEFT, true, NONE),
        "right" => (VK_RIGHT, true, NONE),
        "home" => (VK_HOME, true, NONE),
        "end" => (VK_END, true, NONE),
        "page_up" => (VK_PRIOR, true, NONE),
        "page_down" => (VK_NEXT, true, NONE),
        "f1" => (VK_F1, false, NONE),
        "f2" => (VIRTUAL_KEY(VK_F1.0 + 1), false, NONE),
        "f3" => (VIRTUAL_KEY(VK_F1.0 + 2), false, NONE),
        "f4" => (VIRTUAL_KEY(VK_F1.0 + 3), false, NONE),
        "f5" => (VK_F5, false, NONE),
        "f6" => (VIRTUAL_KEY(VK_F1.0 + 5), false, NONE),
        "f7" => (VIRTUAL_KEY(VK_F1.0 + 6), false, NONE),
        "f8" => (VIRTUAL_KEY(VK_F1.0 + 7), false, NONE),
        "f9" => (VIRTUAL_KEY(VK_F1.0 + 8), false, NONE),
        "f10" => (VIRTUAL_KEY(VK_F1.0 + 9), false, NONE),
        "f11" => (VIRTUAL_KEY(VK_F1.0 + 10), false, NONE),
        "f12" => (VIRTUAL_KEY(VK_F1.0 + 11), false, NONE),
        "volume_up" => (VK_VOLUME_UP, true, NONE),
        "volume_down" => (VK_VOLUME_DOWN, true, NONE),
        "volume_mute" => (VK_VOLUME_MUTE, true, NONE),
        "play_pause" => (VK_MEDIA_PLAY_PAUSE, true, NONE),
        "next_track" => (VK_MEDIA_NEXT_TRACK, true, NONE),
        "prev_track" => (VK_MEDIA_PREV_TRACK, true, NONE),
        s if s.len() == 1 => {
            let b = s.as_bytes()[0];
            if b.is_ascii_lowercase() {
                (VIRTUAL_KEY(u16::from(b - b'a' + b'A')), false, NONE)
            } else if b.is_ascii_digit() {
                (VIRTUAL_KEY(u16::from(b)), false, NONE)
            } else {
                return None;
            }
        }
        _ => return None,
    })
}

fn mod_vk(m: KeyMod) -> (VIRTUAL_KEY, bool) {
    match m {
        KeyMod::Ctrl => (VK_CONTROL, false),
        KeyMod::Alt => (VK_MENU, false),
        KeyMod::Shift => (VK_SHIFT, false),
        KeyMod::Win => (VK_LWIN, true),
    }
}

/// Presses a named key or shortcut with optional modifiers.
pub fn press_key(key: &str, mods: &[KeyMod]) {
    if key == "lock" {
        let _ = crate::links::power(PowerAction::Lock);
        return;
    }
    let Some((vk, extended, preset_mods)) = resolve_key(key) else {
        return;
    };
    let mut all_mods: Vec<KeyMod> = Vec::with_capacity(4);
    for &m in preset_mods.iter().chain(mods.iter()) {
        if !all_mods.contains(&m) {
            all_mods.push(m);
        }
    }
    let mut inputs = Vec::with_capacity(all_mods.len() * 2 + 2);
    for &m in &all_mods {
        let (mvk, mext) = mod_vk(m);
        inputs.push(vk_input(mvk, mext, false));
    }
    inputs.push(vk_input(vk, extended, false));
    inputs.push(vk_input(vk, extended, true));
    for &m in all_mods.iter().rev() {
        let (mvk, mext) = mod_vk(m);
        inputs.push(vk_input(mvk, mext, true));
    }
    wait_for_last_keystroke(AFTER_KEY);
    send_inputs(&inputs);
}

/// Executes a presentation slide action.
pub fn slide(action: SlideAction) {
    let (vk, extended) = match action {
        SlideAction::Next => (VK_NEXT, true),
        SlideAction::Previous => (VK_PRIOR, true),
        SlideAction::Start => (VK_F5, false),
        SlideAction::Stop => (VK_ESCAPE, false),
        SlideAction::Black => (VK_B, false),
    };
    send_inputs(&[vk_input(vk, extended, false), vk_input(vk, extended, true)]);
}

#[repr(C)]
struct LastInputInfo {
    cb_size: u32,
    dw_time: u32,
}

unsafe extern "system" {
    fn GetLastInputInfo(plii: *mut LastInputInfo) -> i32;
    fn GetTickCount() -> u32;
}

/// Returns `true` when the user has moved the mouse or pressed a key on this PC
/// within the last `max_idle_ms` milliseconds.
pub fn is_user_active_within_ms(max_idle_ms: u32) -> bool {
    let mut info = LastInputInfo { cb_size: std::mem::size_of::<LastInputInfo>() as u32, dw_time: 0 };
    let ok = unsafe { GetLastInputInfo(&mut info) };
    if ok == 0 {
        return false;
    }
    let now = unsafe { GetTickCount() };
    now.wrapping_sub(info.dw_time) <= max_idle_ms
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_keystroke_waits_out_the_last_ones_gap() {
        wait_for_last_keystroke(Duration::from_millis(40));
        let start = Instant::now();
        wait_for_last_keystroke(Duration::ZERO);
        assert!(start.elapsed() >= Duration::from_millis(30), "{:?}", start.elapsed());
    }

    #[test]
    fn the_longest_text_is_typed_before_the_phone_gives_up() {
        // The phone sends at most 256 bytes at a time and waits 5 s for the answer.
        assert!(BETWEEN_CHARACTERS * 256 < Duration::from_millis(4_500));
    }

    #[test]
    fn text_is_typed_one_character_at_a_time() {
        let sizes = |text: &str| keystrokes(text, None).iter().map(Vec::len).collect::<Vec<_>>();
        assert_eq!(sizes("great"), [2, 2, 2, 2, 2]);
        // An emoji is two UTF-16 units, kept together; a line break is Enter.
        assert_eq!(sizes("a\u{1F600}"), [2, 4]);
        assert_eq!(sizes("a\r\nb"), [2, 2, 2]);
        assert!(keystrokes("", None).is_empty());
    }

    /// This thread's keyboard layout: US English on CI, any Latin layout on a PC.
    fn latin_layout() -> HKL {
        // SAFETY: a plain Win32 query.
        let layout = unsafe { GetKeyboardLayout(0) };
        assert!(layout_key('a', layout, false).is_some(), "these tests need a Latin keyboard layout");
        layout
    }

    fn key_of(input: &INPUT) -> (u16, bool, bool) {
        // SAFETY: every keystroke built here is a keyboard input.
        let ki = unsafe { input.Anonymous.ki };
        (ki.wVk.0, ki.dwFlags.contains(KEYEVENTF_UNICODE), ki.dwFlags.contains(KEYEVENTF_KEYUP))
    }

    #[test]
    fn what_the_layout_has_a_key_for_is_typed_with_that_key() {
        let latin = latin_layout();
        let key = |ch, caps| layout_key(ch, latin, caps).map(|(vk, _, shift)| (vk, shift));
        assert_eq!(key('a', false), Some((VK_A, false)));
        assert_eq!(key('A', false), Some((VK_A, true)));
        assert_eq!(key(' ', false), Some((VK_SPACE, false)));
        // Caps Lock on: a capital without Shift, a small letter with it.
        assert_eq!(key('A', true), Some((VK_A, false)));
        assert_eq!(key('a', true), Some((VK_A, true)));
        assert!(layout_key('a', latin, false).is_some_and(|(_, scan, _)| scan == 0x1E));
        // Not on the layout, or not a key at all: the Unicode way.
        for ch in ['я', 'म', '\u{1F600}', '\t'] {
            assert_eq!(layout_key(ch, latin, false), None, "{ch:?}");
        }
    }

    #[test]
    fn a_mixed_sentence_uses_keys_where_it_can() {
        let strokes = keystrokes("Hi я", Some((latin_layout(), false)));
        let keys: Vec<Vec<(u16, bool, bool)>> =
            strokes.iter().map(|k| k.iter().map(key_of).collect()).collect();
        let shift = VK_SHIFT.0;
        let (h, i, space) = (u16::from(b'H'), u16::from(b'I'), VK_SPACE.0);
        assert_eq!(
            keys[0],
            [(shift, false, false), (h, false, false), (h, false, true), (shift, false, true)]
        );
        assert_eq!(keys[1], [(i, false, false), (i, false, true)]);
        assert_eq!(keys[2], [(space, false, false), (space, false, true)]);
        assert_eq!(keys[3], [(0, true, false), (0, true, true)]);
        // No layout (an input method is on): every letter the Unicode way, as before.
        assert!(keystrokes("Hi", None).iter().flatten().all(|input| key_of(input).1));
    }

    /// Voice typing's pattern in Notepad, as fast as the phone sends it: words, then
    /// Backspaces and the rewritten phrase. Needs a desktop, so not on CI; run with
    /// `--ignored` while Notepad has focus on an empty file, and compare the lines.
    ///
    /// `NL_UNPACED=old` or `=new` sends every letter and Backspace at once with no
    /// gaps, the old Unicode way or with the layout's keys, to compare the two.
    #[test]
    #[ignore = "types into the focused window"]
    fn rewrite_in_notepad() {
        // `old` or `new`, optionally with a gap in milliseconds: `new:4`.
        let unpaced = std::env::var("NL_UNPACED").ok();
        let gap = unpaced
            .as_deref()
            .and_then(|mode| mode.split_once(':'))
            .and_then(|(_, ms)| ms.parse().ok())
            .map_or(Duration::ZERO, Duration::from_millis);
        let typing = |text: &str| match unpaced.as_deref() {
            Some(mode) => {
                let layout = if mode.starts_with("new") { typing_layout() } else { None };
                for keystroke in keystrokes(text, layout) {
                    send_inputs(&keystroke);
                    std::thread::sleep(gap);
                }
            }
            None => type_text(text),
        };
        let backspaces = |count: usize| {
            for _ in 0..count {
                match unpaced {
                    Some(_) => {
                        send_inputs(&[vk_input(VK_BACK, false, false), vk_input(VK_BACK, false, true)]);
                        std::thread::sleep(gap);
                    }
                    None => press_key("backspace", &[]),
                }
            }
        };
        for _ in 0..40 {
            assert!(focused_title().contains("nl-type-test"), "focus moved: {}", focused_title());
            typing("trying to check whether it");
            backspaces(2);
            typing("that's w");
            backspaces(1);
            typing("working or");
            backspaces(3);
            typing(" or not Great");
            backspaces(5);
            typing("great but again\n");
            std::thread::sleep(Duration::from_millis(150));
        }
        std::thread::sleep(Duration::from_millis(500));
        assert!(focused_title().contains("nl-type-test"), "focus moved: {}", focused_title());
        press_key("s", &[KeyMod::Ctrl]);
    }

    fn focused_title() -> String {
        use windows::Win32::UI::WindowsAndMessaging::GetWindowTextW;
        let mut title = [0u16; 256];
        // SAFETY: a plain Win32 query into a local buffer.
        let len = unsafe { GetWindowTextW(GetForegroundWindow(), &mut title) };
        String::from_utf16_lossy(&title[..len.max(0) as usize])
    }

    #[test]
    fn resolves_named_keys_and_shortcuts() {
        assert_eq!(resolve_key("enter"), Some((VK_RETURN, false, &[][..])));
        assert_eq!(resolve_key("up"), Some((VK_UP, true, &[][..])));
        assert_eq!(resolve_key("copy"), Some((VK_C, false, &[KeyMod::Ctrl][..])));
        assert_eq!(resolve_key("paste"), Some((VK_V, false, &[KeyMod::Ctrl][..])));
        assert_eq!(resolve_key("task_view"), Some((VK_TAB, false, &[KeyMod::Win][..])));
        assert_eq!(resolve_key("a"), Some((VK_A, false, &[][..])));
        assert_eq!(resolve_key("f5"), Some((VK_F5, false, &[][..])));
        assert!(resolve_key("unknown_key").is_none());
    }
}
