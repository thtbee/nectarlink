// SPDX-License-Identifier: GPL-3.0-or-later
//! The PC's mouse and keyboard on a phone beside it (#107), on Windows.
//!
//! Low-level mouse and keyboard hooks run on a thread of their own while a
//! phone is set beside the PC. Until the mouse is pushed against the edge
//! facing the phone they only look. Once it's over there they hold the
//! cursor still (in the middle of that screen, so Windows never clips a move
//! towards the edge to nothing) and pass what the mouse and keyboard do to the
//! phone instead: moves as counts for the phone's own pointer
//! (`pointer.move`), keys as `mirror.input`. The mouse comes back when the
//! phone's pointer is pushed against its edge facing the PC, on Ctrl+Alt+Esc,
//! or as soon as the phone stops answering. `pointer_flow` has the arithmetic.
//!
//! The hook procedures only read two atomics and queue an event: Windows
//! drops a low-level hook that's slow to answer.

use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicI32, Ordering},
    mpsc,
};

use nectarlink_core::{DeviceId, MirrorInput, PointerEdge, PointerMove, PointerScreen};
use windows::Win32::{
    Foundation::{LPARAM, LRESULT, POINT, WPARAM},
    Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint},
    System::Threading::GetCurrentThreadId,
    UI::{
        Input::KeyboardAndMouse::{GetKeyState, GetKeyboardLayout, ToUnicodeEx},
        WindowsAndMessaging::{
            CallNextHookEx, GetCursorPos, GetMessageW, GetSystemMetrics, HC_ACTION, KBDLLHOOKSTRUCT, MSG,
            MSLLHOOKSTRUCT, PostThreadMessageW, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
            SM_YVIRTUALSCREEN, SetCursorPos, SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL,
            WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP,
            WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
            WM_XBUTTONDOWN, WM_XBUTTONUP,
        },
    },
};

use crate::pointer_flow::{self, Bounds, EdgeWatch, KeyAction, OnPhone, Side, vk};

/// What the hooks saw, and what came back from the phone.
#[derive(Debug)]
enum Event {
    /// The PC's mouse moved (while it's on the PC).
    Moved(i32, i32),
    /// The mouse moved by this much while on the phone (the cursor stays put).
    Delta(i32, i32),
    /// Bits: 1 left, 2 right, 4 middle.
    Button(u8, bool),
    Wheel(i8),
    /// The mouse's back button: Back on the phone.
    BackButton,
    Key {
        code: u16,
        scan: u16,
        down: bool,
    },
    Entered(Result<PointerScreen, String>),
    /// Where the phone's pointer really is (`None`: it didn't say).
    Where(Option<nectarlink_core::PointerAt>),
    /// The phone stopped taking it (offline, or a send failed).
    Lost,
    Stop,
}

/// The mouse is on the phone: the hooks hold the cursor and swallow input.
static ON_PHONE: AtomicBool = AtomicBool::new(false);
/// Where the cursor is held while on the phone: the middle of the screen it
/// left, so Windows doesn't clip a move towards the edge to nothing.
static PARK_X: AtomicI32 = AtomicI32::new(0);
static PARK_Y: AtomicI32 = AtomicI32::new(0);
/// Where it left the PC's screen, to come back there.
static EDGE_X: AtomicI32 = AtomicI32::new(0);
static EDGE_Y: AtomicI32 = AtomicI32::new(0);
static EVENTS: Mutex<Option<mpsc::Sender<Event>>> = Mutex::new(None);
/// Which phone is beside the PC, and on which side.
static TARGET: Mutex<Option<(DeviceId, Side)>> = Mutex::new(None);
/// The hook thread's ID while it runs, to stop it.
static HOOKS: Mutex<Option<u32>> = Mutex::new(None);

fn queue(event: Event) {
    if let Some(tx) = EVENTS.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let _ = tx.send(event);
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Sets which phone sits beside the PC, and on which side, or none: starts
/// or stops watching the mouse. Setting what's already set does nothing.
pub fn configure(target: Option<(DeviceId, Side)>) {
    let mut current = lock(&TARGET);
    if *current == target {
        return;
    }
    *current = target;
    // The flow for the old phone or side ends, giving the mouse back.
    queue(Event::Stop);
    let mut hooks = lock(&HOOKS);
    match target {
        None => {
            *lock(&EVENTS) = None;
            if let Some(thread) = hooks.take() {
                // SAFETY: posting WM_QUIT to the hook thread's own queue ends its loop.
                let _ = unsafe { PostThreadMessageW(thread, WM_QUIT, WPARAM(0), LPARAM(0)) };
            }
            tracing::debug!("not watching the mouse for a phone");
        }
        Some((device, side)) => {
            let (tx, rx) = mpsc::channel();
            *lock(&EVENTS) = Some(tx);
            std::thread::Builder::new()
                .name("pointer-flow".into())
                .spawn(move || Flow::new(device, side).run(rx))
                .ok();
            if hooks.is_none() {
                *hooks = start_hooks();
            }
            tracing::debug!(phone = %device.short(), ?side, hooks = hooks.is_some(), "watching the mouse for a phone");
        }
    }
}

/// Starts the hook thread; its ID once the hooks are in.
fn start_hooks() -> Option<u32> {
    let (ready, started) = mpsc::sync_channel(1);
    std::thread::Builder::new().name("pointer-hooks".into()).spawn(move || hook_thread(ready)).ok()?;
    started.recv_timeout(std::time::Duration::from_secs(5)).ok().flatten()
}

/// Core events: the phone going away gives the mouse back.
pub fn on_event(event: &nectarlink_core::NodeEvent) {
    use nectarlink_core::{LinkState, NodeEvent};
    if let NodeEvent::LinkChanged { device, link } = event
        && !matches!(link, LinkState::Online { .. })
        && lock(&TARGET).is_some_and(|(d, _)| d == *device)
        && ON_PHONE.load(Ordering::Acquire)
    {
        queue(Event::Lost);
    }
}

fn hook_thread(ready: mpsc::SyncSender<Option<u32>>) {
    // SAFETY: plain thread-wide low-level hooks with procedures that live
    // for the whole program; removed before the thread ends.
    let hooks = unsafe {
        (
            SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0),
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), None, 0),
        )
    };
    let (Ok(mouse), Ok(keyboard)) = hooks else {
        tracing::warn!("can't watch the mouse for a phone beside the PC");
        let _ = ready.send(None);
        if let Ok(h) = hooks.0 {
            // SAFETY: unhooking the hook just set.
            let _ = unsafe { UnhookWindowsHookEx(h) };
        }
        if let Ok(h) = hooks.1 {
            // SAFETY: as above.
            let _ = unsafe { UnhookWindowsHookEx(h) };
        }
        return;
    };
    // SAFETY: no preconditions.
    let _ = ready.send(Some(unsafe { GetCurrentThreadId() }));
    let mut msg = MSG::default();
    // SAFETY: the message loop low-level hooks need on their thread.
    unsafe { while GetMessageW(&mut msg, None, 0, 0).as_bool() {} }
    ON_PHONE.store(false, Ordering::Release);
    // SAFETY: unhooking what this thread set.
    unsafe {
        let _ = UnhookWindowsHookEx(mouse);
        let _ = UnhookWindowsHookEx(keyboard);
    }
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: for HC_ACTION, lparam points at an MSLLHOOKSTRUCT.
        let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        let msg = wparam.0 as u32;
        if ON_PHONE.load(Ordering::Acquire) {
            match msg {
                WM_MOUSEMOVE => {
                    let (px, py) = (PARK_X.load(Ordering::Relaxed), PARK_Y.load(Ordering::Relaxed));
                    queue(Event::Delta(info.pt.x - px, info.pt.y - py));
                }
                WM_LBUTTONDOWN => queue(Event::Button(1, true)),
                WM_LBUTTONUP => queue(Event::Button(1, false)),
                WM_RBUTTONDOWN => queue(Event::Button(2, true)),
                WM_RBUTTONUP => queue(Event::Button(2, false)),
                WM_MBUTTONDOWN => queue(Event::Button(4, true)),
                WM_MBUTTONUP => queue(Event::Button(4, false)),
                WM_MOUSEWHEEL => {
                    let notches = ((info.mouseData >> 16) as i16) / 120;
                    if notches != 0 {
                        queue(Event::Wheel(notches.clamp(-127, 127) as i8));
                    }
                }
                WM_XBUTTONDOWN if (info.mouseData >> 16) & 1 != 0 => queue(Event::BackButton),
                WM_XBUTTONUP | WM_XBUTTONDOWN => {}
                _ => {}
            }
            // Swallowed: the cursor stays put and nothing on the PC sees it.
            return LRESULT(1);
        }
        if msg == WM_MOUSEMOVE {
            queue(Event::Moved(info.pt.x, info.pt.y));
        }
    }
    // SAFETY: passing the event on, as every hook must.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && ON_PHONE.load(Ordering::Acquire) {
        // SAFETY: for HC_ACTION, lparam points at a KBDLLHOOKSTRUCT.
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
        let up = matches!(wparam.0 as u32, WM_KEYUP | WM_SYSKEYUP);
        if down || up {
            queue(Event::Key { code: info.vkCode as u16, scan: info.scanCode as u16, down });
            return LRESULT(1);
        }
    }
    // SAFETY: passing the event on.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// The PC's screens together.
fn desktop() -> Bounds {
    // SAFETY: GetSystemMetrics has no preconditions.
    let (x, y, w, h) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };
    Bounds { left: x, top: y, right: x + w, bottom: y + h }
}

/// The screen at (`x`, `y`).
fn screen_at(x: i32, y: i32) -> Bounds {
    // SAFETY: MonitorFromPoint always returns a monitor with this flag;
    // GetMonitorInfoW fills a struct sized for it.
    unsafe {
        let monitor = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let mut info =
            MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if GetMonitorInfoW(monitor, &mut info).as_bool() {
            let r = info.rcMonitor;
            Bounds { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
        } else {
            desktop()
        }
    }
}

/// What goes to the phone, in order.
enum Out {
    Move(PointerMove),
    Input(MirrorInput),
    Leave,
}

/// Sends to the phone in the order things happened, merging moves that
/// queued up while a send was slow. A failed send gives the mouse back.
fn sender(device: DeviceId) -> tokio::sync::mpsc::UnboundedSender<Out> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Out>();
    crate::core_host::spawn(async move {
        let Some(node) = crate::core_host::node() else { return queue(Event::Lost) };
        let mut held: Option<Out> = None;
        loop {
            let next = match held.take() {
                Some(out) => out,
                None => match rx.recv().await {
                    Some(out) => out,
                    None => return,
                },
            };
            let result = match next {
                Out::Move(mut moved) => {
                    while let Ok(more) = rx.try_recv() {
                        match more {
                            Out::Move(m)
                                if m.buttons == moved.buttons && m.wheel == 0 && moved.wheel == 0 =>
                            {
                                moved.dx = (moved.dx + m.dx)
                                    .clamp(-PointerMove::MAX_COUNTS, PointerMove::MAX_COUNTS);
                                moved.dy = (moved.dy + m.dy)
                                    .clamp(-PointerMove::MAX_COUNTS, PointerMove::MAX_COUNTS);
                            }
                            other => {
                                held = Some(other);
                                break;
                            }
                        }
                    }
                    node.pointer_move(device, moved).await
                }
                Out::Input(input) => node.mirror_input(device, nectarlink_core::MIRROR_SCREEN, input).await,
                Out::Leave => {
                    let _ = node.pointer_leave(device).await;
                    return;
                }
            };
            if let Err(e) = result {
                tracing::info!(error = %e, "the phone stopped taking the PC's mouse");
                return queue(Event::Lost);
            }
        }
    });
    tx
}

/// Modifier keys held, as the hooks saw them (they're swallowed, so Windows'
/// own key state doesn't change while the mouse is on the phone).
#[derive(Default)]
struct Mods {
    shift: bool,
    ctrl: bool,
    alt: bool,
    caps: bool,
}

impl Mods {
    fn key(&mut self, code: u16, down: bool) {
        match code {
            vk::SHIFT | vk::LSHIFT | vk::RSHIFT => self.shift = down,
            vk::CONTROL | vk::LCONTROL | vk::RCONTROL => self.ctrl = down,
            vk::MENU | vk::LMENU | vk::RMENU => self.alt = down,
            vk::CAPITAL if down => self.caps = !self.caps,
            _ => {}
        }
    }

    /// What the key types on the PC's keyboard layout, with these held.
    fn text(&self, code: u16, scan: u16) -> Option<String> {
        let mut state = [0u8; 256];
        let set = |state: &mut [u8; 256], k: u16| state[k as usize] = 0x80;
        if self.shift {
            set(&mut state, vk::SHIFT);
        }
        if self.ctrl {
            set(&mut state, vk::CONTROL);
        }
        if self.alt {
            set(&mut state, vk::MENU);
        }
        if self.caps {
            state[vk::CAPITAL as usize] = 0x01;
        }
        let mut buf = [0u16; 8];
        // SAFETY: a key state of 256 bytes and a buffer for the result; flag 4
        // leaves the layout's dead-key state alone.
        let n = unsafe {
            let layout = GetKeyboardLayout(0);
            ToUnicodeEx(code as u32, scan as u32, &state, &mut buf, 4, Some(layout))
        };
        if n <= 0 {
            return None;
        }
        let text = String::from_utf16_lossy(&buf[..n as usize]);
        (!text.chars().any(char::is_control)).then_some(text)
    }
}

enum State {
    Watching(EdgeWatch),
    /// Asked the phone to show its pointer; the reply's on its way.
    Entering {
        at: (i32, i32),
        y: f32,
    },
    OnPhone {
        track: OnPhone,
        out: tokio::sync::mpsc::UnboundedSender<Out>,
        buttons: u8,
        asked_where: bool,
    },
}

struct Flow {
    device: DeviceId,
    side: Side,
    state: State,
    mods: Mods,
}

impl Flow {
    fn new(device: DeviceId, side: Side) -> Self {
        Flow { device, side, state: State::Watching(EdgeWatch::new(side)), mods: Mods::default() }
    }

    fn run(mut self, rx: mpsc::Receiver<Event>) {
        while let Ok(event) = rx.recv() {
            if matches!(event, Event::Stop) {
                self.come_back(None);
                return;
            }
            self.handle(event);
        }
        self.come_back(None);
    }

    fn handle(&mut self, event: Event) {
        match (&mut self.state, event) {
            (State::Watching(watch), Event::Moved(x, y)) => {
                if watch.moved(x, y, desktop()) {
                    tracing::debug!(x, y, "the mouse pushed against the edge facing the phone");
                    self.go_over(x, y);
                }
            }
            (State::Entering { at, y }, Event::Entered(Ok(screen))) => {
                let ((x, py), fraction) = (*at, *y);
                // The hook sees where a push would have gone, past the edge:
                // it comes back to the edge itself.
                let desk = desktop();
                let x = x.clamp(desk.left, desk.right - 1);
                let py = py.clamp(desk.top, desk.bottom - 1);
                EDGE_X.store(x, Ordering::Relaxed);
                EDGE_Y.store(py, Ordering::Relaxed);
                let monitor = screen_at(x, py);
                let (cx, cy) = ((monitor.left + monitor.right) / 2, (monitor.top + monitor.bottom) / 2);
                PARK_X.store(cx, Ordering::Relaxed);
                PARK_Y.store(cy, Ordering::Relaxed);
                // SAFETY: SetCursorPos has no preconditions.
                let _ = unsafe { SetCursorPos(cx, cy) };
                // SAFETY: GetKeyState has no preconditions.
                let caps = unsafe { GetKeyState(vk::CAPITAL as i32) } & 1 != 0;
                self.mods = Mods { caps, ..Mods::default() };
                let edge: PointerEdge = self.side.phone_edge();
                let out = sender(self.device);
                self.state = State::OnPhone {
                    track: OnPhone::new(screen, edge, fraction),
                    out,
                    buttons: 0,
                    asked_where: false,
                };
                ON_PHONE.store(true, Ordering::Release);
                crate::input_share::enter_remote(self.device);
                tracing::info!("the PC's mouse is on the phone");
            }
            (State::Entering { .. }, Event::Entered(Err(reason))) => {
                tracing::info!(reason, "the phone can't take the PC's mouse");
                let mut watch = EdgeWatch::new(self.side);
                // Not again on every push: once back from the edge.
                watch.disarm();
                self.state = State::Watching(watch);
            }
            (State::OnPhone { track, out, buttons, asked_where }, Event::Delta(dx, dy)) => {
                if dx == 0 && dy == 0 {
                    return;
                }
                let step = track.moved(dx, dy);
                if step.dx != 0 || step.dy != 0 {
                    let _ = out.send(Out::Move(PointerMove {
                        dx: step.dx,
                        dy: step.dy,
                        buttons: *buttons,
                        wheel: 0,
                    }));
                }
                // At the edge facing the PC and pushing on: is the pointer
                // really there? (Once at a time.)
                if step.pushing_out && *buttons == 0 && !*asked_where {
                    *asked_where = true;
                    let device = self.device;
                    crate::core_host::spawn(async move {
                        let at = match crate::core_host::node() {
                            Some(node) => node.pointer_where(device).await.ok(),
                            None => None,
                        };
                        queue(Event::Where(at));
                    });
                }
            }
            (State::OnPhone { track, asked_where, .. }, Event::Where(at)) => {
                *asked_where = false;
                match at {
                    Some(at) if track.at_exit(at) => {
                        track.resync(at);
                        let fraction = track.height_fraction();
                        self.come_back(Some(fraction));
                    }
                    Some(at) => track.resync(at),
                    // The phone can't say: trust the count.
                    None => {
                        let fraction = track.height_fraction();
                        self.come_back(Some(fraction));
                    }
                }
            }
            (State::OnPhone { out, buttons, .. }, Event::Button(bit, down)) => {
                *buttons = if down { *buttons | bit } else { *buttons & !bit };
                let _ = out.send(Out::Move(PointerMove { dx: 0, dy: 0, buttons: *buttons, wheel: 0 }));
            }
            (State::OnPhone { out, buttons, .. }, Event::Wheel(notches)) => {
                let _ = out.send(Out::Move(PointerMove { dx: 0, dy: 0, buttons: *buttons, wheel: notches }));
            }
            (State::OnPhone { out, .. }, Event::BackButton) => {
                let _ =
                    out.send(Out::Input(MirrorInput::Key { key: nectarlink_core::mirror_keys::BACK.into() }));
            }
            (State::OnPhone { .. }, Event::Key { code, scan, down }) => self.key(code, scan, down),
            (State::OnPhone { .. }, Event::Lost) => self.come_back(None),
            _ => {}
        }
    }

    fn key(&mut self, code: u16, scan: u16, down: bool) {
        self.mods.key(code, down);
        if !down {
            return;
        }
        let State::OnPhone { out, .. } = &self.state else { return };
        let input = match pointer_flow::key_action(code, self.mods.ctrl, self.mods.alt) {
            KeyAction::Key(key) => Some(MirrorInput::Key { key: key.into() }),
            KeyAction::Text => self.mods.text(code, scan).map(|text| MirrorInput::Text { text }),
            KeyAction::Paste => match super::clipboard::read_for_paste() {
                super::clipboard::Clip::Text { text, .. }
                    if !text.is_empty() && text.len() <= nectarlink_core::MIRROR_MAX_TEXT_BYTES =>
                {
                    Some(MirrorInput::Text { text })
                }
                _ => None,
            },
            KeyAction::Leave => {
                let fraction = match &self.state {
                    State::OnPhone { track, .. } => track.height_fraction(),
                    _ => 0.5,
                };
                return self.come_back(Some(fraction));
            }
            KeyAction::Ignore => None,
        };
        if let Some(input) = input {
            let _ = out.send(Out::Input(input));
        }
    }

    /// Asks the phone to show its pointer, from the PC's mouse at (`x`, `y`).
    fn go_over(&mut self, x: i32, y: i32) {
        let screen = screen_at(x, y);
        let fraction = pointer_flow::fraction(y, screen.top, screen.bottom);
        self.state = State::Entering { at: (x, y), y: fraction };
        let (device, edge) = (self.device, self.side.phone_edge());
        crate::core_host::spawn(async move {
            let result = match crate::core_host::node() {
                Some(node) => node.pointer_enter(device, edge, fraction).await.map_err(|e| e.to_string()),
                None => Err("not started".into()),
            };
            queue(Event::Entered(result));
        });
    }

    /// The mouse comes back to the PC, at the edge facing the phone,
    /// `fraction` of the way down (or where it was held).
    fn come_back(&mut self, fraction: Option<f32>) {
        let was_on_phone = ON_PHONE.swap(false, Ordering::AcqRel);
        if let State::OnPhone { out, buttons, .. } = &self.state {
            // Nothing stays held on the phone.
            if *buttons != 0 {
                let _ = out.send(Out::Move(PointerMove { dx: 0, dy: 0, buttons: 0, wheel: 0 }));
            }
            let _ = out.send(Out::Leave);
        }
        if was_on_phone {
            let (x, y) = (EDGE_X.load(Ordering::Relaxed), EDGE_Y.load(Ordering::Relaxed));
            let y = match fraction {
                Some(f) => {
                    let screen = screen_at(x, y);
                    pointer_flow::at_fraction(f, screen.top, screen.bottom)
                }
                None => y,
            };
            // SAFETY: SetCursorPos has no preconditions.
            let _ = unsafe { SetCursorPos(x, y) };
            crate::input_share::leave_remote();
            tracing::info!("the PC's mouse is back");
        }
        let mut watch = EdgeWatch::new(self.side);
        watch.disarm();
        self.state = State::Watching(watch);
    }
}

/// Where the cursor is now (for tests and the first push).
#[allow(dead_code)]
pub fn cursor() -> Option<(i32, i32)> {
    let mut p = POINT::default();
    // SAFETY: GetCursorPos writes the point.
    unsafe { GetCursorPos(&mut p) }.ok().map(|_| (p.x, p.y))
}
