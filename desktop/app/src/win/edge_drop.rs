// SPDX-License-Identifier: GPL-3.0-or-later
//! Screen-edge file drop trigger and `IDropTarget` on Windows.
//!
//! When enabled in Settings and at least one paired device is connected, a
//! tiny invisible trigger strip at the chosen screen edge stays completely
//! click-through (`WS_EX_TRANSPARENT`) until the user drags from inside a
//! window toward that outer screen edge. Windows OLE `DoDragDrop` then enters
//! our `IDropTarget` with `CF_HDROP`, sliding out `EdgeDropWindow.qml` so
//! dropping sends the files straight to the active phone without stealing focus.

use std::{
    cell::Cell,
    ffi::OsString,
    os::windows::ffi::OsStringExt,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicU8, Ordering},
    },
};

use windows::{
    Win32::{
        Foundation::{
            COLORREF, DRAGDROP_E_INVALIDHWND, HWND, LPARAM, LRESULT, POINT, POINTL, RECT, S_OK, WPARAM,
        },
        Graphics::Gdi::{
            GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromPoint,
        },
        System::{
            Com::{DVASPECT_CONTENT, FORMATETC, IDataObject, ReleaseStgMedium, STGMEDIUM, TYMED_HGLOBAL},
            LibraryLoader::GetModuleHandleW,
            Memory::{GlobalLock, GlobalUnlock},
            Ole::{
                CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE, IDropTarget, IDropTarget_Impl,
                OleInitialize, OleUninitialize, RegisterDragDrop, RevokeDragDrop,
            },
            SystemServices::{MK_SHIFT, MODIFIERKEYS_FLAGS},
            Threading::{GetCurrentProcessId, GetCurrentThreadId},
        },
        UI::{
            Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON, VK_SHIFT},
            Shell::{DragQueryFileW, HDROP},
            WindowsAndMessaging::{
                CW_USEDEFAULT, CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow, GW_HWNDNEXT,
                GWL_EXSTYLE, GetMessageW, GetTopWindow, GetWindow, GetWindowLongW, GetWindowRect,
                GetWindowTextW, GetWindowThreadProcessId, HHOOK, HWND_TOPMOST, KillTimer, LWA_ALPHA, MSG,
                MSLLHOOKSTRUCT, PostMessageW, PostThreadMessageW, RegisterClassExW, SW_SHOWNOACTIVATE,
                SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
                SetLayeredWindowAttributes, SetTimer, SetWindowLongW, SetWindowPos, SetWindowsHookExW,
                ShowWindow, UnhookWindowsHookEx, WH_MOUSE_LL, WM_APP, WM_LBUTTONDOWN, WM_LBUTTONUP,
                WM_MOUSEMOVE, WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED,
                WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
            },
        },
    },
    core::{PCWSTR, Ref, w},
};

const WM_APP_BTN_DOWN: u32 = WM_APP + 1;
const WM_APP_ARM: u32 = WM_APP + 2;
const WM_APP_DISARM: u32 = WM_APP + 3;
const WM_APP_SIDE_CHANGED: u32 = WM_APP + 4;
const TIMER_WATCHDOG: usize = 1;

const STRIP_WIDTH_PX: i32 = 4;
const ARM_MARGIN_PX: i32 = 8;
const SCROLLBAR_GUARD_PX: i32 = 16;
const DRAG_THRESHOLD_PX: i32 = 16;
const PILL_CAPTURE_W_PX: i32 = 310;
const PILL_CAPTURE_H_PX: i32 = 76;

/// Which outer edge of the monitor hosts the file drop pill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub fn parse(raw: &str) -> Self {
        if raw.trim().eq_ignore_ascii_case("left") { Self::Left } else { Self::Right }
    }

    fn as_u8(self) -> u8 {
        match self {
            Self::Left => 0,
            Self::Right => 1,
        }
    }

    fn from_u8(v: u8) -> Self {
        if v == 0 { Self::Left } else { Self::Right }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

static ENABLED_IN_SETTINGS: AtomicBool = AtomicBool::new(false);
static PHONE_CONNECTED: AtomicBool = AtomicBool::new(false);
static SIDE_U8: AtomicU8 = AtomicU8::new(1);

static THREAD_ID: Mutex<Option<u32>> = Mutex::new(None);
static TRIGGER_HWND: AtomicIsize = AtomicIsize::new(0);

// Hook -> worker thread shared atomics (read with relaxed ordering in hook).
static DRAG_BTN_DOWN: AtomicBool = AtomicBool::new(false);
static DOWN_X: AtomicI32 = AtomicI32::new(0);
static DOWN_Y: AtomicI32 = AtomicI32::new(0);
static STARTED_AT_EDGE: AtomicBool = AtomicBool::new(false);
static ARMED: AtomicBool = AtomicBool::new(false);
static IN_DROP_SESSION: AtomicBool = AtomicBool::new(false);
static HAS_OUTER_EDGE: AtomicBool = AtomicBool::new(false);
static EDGE_X: AtomicI32 = AtomicI32::new(0);
static EDGE_MIN_Y: AtomicI32 = AtomicI32::new(0);
static EDGE_MAX_Y: AtomicI32 = AtomicI32::new(0);
static MON_LEFT: AtomicI32 = AtomicI32::new(0);
static MON_RIGHT: AtomicI32 = AtomicI32::new(0);
static MON_TOP: AtomicI32 = AtomicI32::new(0);
static MON_BOTTOM: AtomicI32 = AtomicI32::new(0);

static ACTIVE_WORK: Mutex<Option<(i32, i32, i32, i32)>> = Mutex::new(None);
static ACTIVE_CURSOR_Y: AtomicI32 = AtomicI32::new(0);
static ACTIVE_SIDE: AtomicU8 = AtomicU8::new(1);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn current_side() -> Side {
    Side::from_u8(SIDE_U8.load(Ordering::Relaxed))
}

/// Updates settings state (`enabled`, `"left"` or `"right"`) and starts or
/// stops the background edge-drop hook accordingly.
pub fn configure(enabled: bool, side: &str) {
    let parsed_side = Side::parse(side);
    SIDE_U8.store(parsed_side.as_u8(), Ordering::Relaxed);
    ENABLED_IN_SETTINGS.store(enabled, Ordering::Relaxed);
    if enabled {
        PHONE_CONNECTED.store(crate::bridge::app::has_online_phone(), Ordering::Relaxed);
    }
    sync_running();
    let hwnd_raw = TRIGGER_HWND.load(Ordering::Acquire);
    if hwnd_raw != 0 {
        // SAFETY: `hwnd_raw` is cleared before DestroyWindow on the worker thread.
        let _ = unsafe {
            PostMessageW(Some(HWND(hwnd_raw as *mut _)), WM_APP_SIDE_CHANGED, WPARAM(0), LPARAM(0))
        };
    }
}

/// Re-evaluates whether an online phone is connected on link/device events.
pub fn on_event(event: &nectarlink_core::NodeEvent) {
    use nectarlink_core::NodeEvent;
    if matches!(event, NodeEvent::LinkChanged { .. } | NodeEvent::DeviceRemoved(_))
        && ENABLED_IN_SETTINGS.load(Ordering::Relaxed)
    {
        PHONE_CONNECTED.store(crate::bridge::app::has_online_phone(), Ordering::Relaxed);
        sync_running();
    }
}

/// Stops the edge-drop hook and destroys its trigger window on app shutdown.
pub fn stop() {
    ENABLED_IN_SETTINGS.store(false, Ordering::Relaxed);
    PHONE_CONNECTED.store(false, Ordering::Relaxed);
    sync_running();
}

fn sync_running() {
    let want = ENABLED_IN_SETTINGS.load(Ordering::Relaxed) && PHONE_CONNECTED.load(Ordering::Relaxed);
    let mut tid = lock(&THREAD_ID);
    match (want, *tid) {
        (true, None) => {
            *tid = start_worker();
        }
        (false, Some(id)) => {
            *tid = None;
            reset_hook_atomics();
            // SAFETY: posting WM_QUIT to our own worker thread ends its GetMessageW loop cleanly.
            let _ = unsafe { PostThreadMessageW(id, WM_QUIT, WPARAM(0), LPARAM(0)) };
        }
        _ => {}
    }
}

fn reset_hook_atomics() {
    DRAG_BTN_DOWN.store(false, Ordering::Relaxed);
    STARTED_AT_EDGE.store(false, Ordering::Relaxed);
    ARMED.store(false, Ordering::Relaxed);
    IN_DROP_SESSION.store(false, Ordering::Relaxed);
}

fn start_worker() -> Option<u32> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new().name("edge-drop".into()).spawn(move || worker_main(tx)).ok()?;
    rx.recv_timeout(std::time::Duration::from_secs(5)).ok().flatten()
}

fn worker_main(ready: std::sync::mpsc::SyncSender<Option<u32>>) {
    // SAFETY: STA COM initialization required for `RegisterDragDrop` on this thread.
    let ole_ok = unsafe { OleInitialize(None) }.is_ok();
    if !ole_ok {
        let _ = ready.send(None);
        return;
    }

    let hwnd = create_trigger_window();
    if hwnd.0.is_null() {
        // SAFETY: matches successful `OleInitialize` above.
        unsafe { OleUninitialize() };
        let _ = ready.send(None);
        return;
    }

    let target: IDropTarget = EdgeDropTarget { hwnd, has_files: Cell::new(false) }.into();
    // SAFETY: `hwnd` is a live window owned by this STA thread and `target` implements `IDropTarget`.
    if unsafe { RegisterDragDrop(hwnd, &target) }.is_err() {
        // SAFETY: `hwnd` is valid and owned by this thread.
        let _ = unsafe { DestroyWindow(hwnd) };
        // SAFETY: matches successful `OleInitialize` above.
        unsafe { OleUninitialize() };
        let _ = ready.send(None);
        return;
    }

    // SAFETY: `mouse_hook` is a valid low-level hook proc for this thread's message loop.
    let hook = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), None, 0) }.ok();
    if hook.is_none() {
        let _ = unsafe { RevokeDragDrop(hwnd) };
        let _ = unsafe { DestroyWindow(hwnd) };
        unsafe { OleUninitialize() };
        let _ = ready.send(None);
        return;
    }

    TRIGGER_HWND.store(hwnd.0 as isize, Ordering::Release);
    // SAFETY: no preconditions.
    let tid = unsafe { GetCurrentThreadId() };
    let _ = ready.send(Some(tid));

    let mut msg = MSG::default();
    // SAFETY: standard Win32 message loop for this thread's window and hook.
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        match msg.message {
            WM_APP_BTN_DOWN => {
                let pt = POINT { x: msg.wParam.0 as i32, y: msg.lParam.0 as i32 };
                update_monitor_for_point(pt, true);
            }
            WM_APP_ARM => {
                let y = msg.lParam.0 as i32;
                arm_trigger(hwnd, y);
            }
            WM_APP_DISARM => {
                disarm_trigger(hwnd);
            }
            WM_APP_SIDE_CHANGED => {
                if !IN_DROP_SESSION.load(Ordering::Acquire) {
                    disarm_trigger(hwnd);
                }
            }
            _ => {}
        }
        // SAFETY: `msg` came from `GetMessageW`.
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::TranslateMessage(&msg);
            windows::Win32::UI::WindowsAndMessaging::DispatchMessageW(&msg);
        }
    }

    TRIGGER_HWND.store(0, Ordering::Release);
    reset_hook_atomics();
    if let Some(h) = hook {
        // SAFETY: `h` was returned by `SetWindowsHookExW`.
        let _ = unsafe { UnhookWindowsHookEx(h) };
    }
    // SAFETY: balances `RegisterDragDrop` on the same thread/window.
    let _ = unsafe { RevokeDragDrop(hwnd) };
    // SAFETY: `hwnd` is owned by this thread.
    let _ = unsafe { DestroyWindow(hwnd) };
    // SAFETY: balances `OleInitialize` above.
    unsafe { OleUninitialize() };
}

fn create_trigger_window() -> HWND {
    // SAFETY: `GetModuleHandleW(None)` returns the current executable's HINSTANCE.
    let hinstance = unsafe { GetModuleHandleW(None) }.unwrap_or_default();
    let class_name = w!("NectarLinkEdgeDropTrigger");
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(trigger_wnd_proc),
        hInstance: hinstance.into(),
        lpszClassName: class_name,
        ..Default::default()
    };
    // SAFETY: valid WNDCLASSEXW; repeated registrations safely fail/return existing atom.
    unsafe { RegisterClassExW(&wc) };

    let ex = WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT;
    // SAFETY: creates a top-level tool window owned by this thread.
    let hwnd = unsafe {
        CreateWindowExW(
            ex,
            class_name,
            w!("NectarLinkEdgeDropTrigger"),
            WS_POPUP,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            STRIP_WIDTH_PX,
            100,
            None,
            None,
            Some(hinstance.into()),
            None,
        )
    }
    .unwrap_or_default();

    if !hwnd.0.is_null() {
        // SAFETY: `hwnd` has `WS_EX_LAYERED`; alpha 1/255 keeps it invisible while hit-testable when transparent flag is off.
        let _ = unsafe { SetLayeredWindowAttributes(hwnd, COLORREF(0), 1, LWA_ALPHA) };
        // SAFETY: shows without stealing focus.
        let _ = unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
    }
    hwnd
}

unsafe extern "system" fn trigger_wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_TIMER && wparam.0 == TIMER_WATCHDOG {
        // If no mouse button is held anymore and we aren't mid-drop, restore click-through.
        // SAFETY: `GetAsyncKeyState` has no preconditions.
        let ldown = (unsafe { GetAsyncKeyState(i32::from(VK_LBUTTON.0)) }) < 0;
        // SAFETY: `GetAsyncKeyState` has no preconditions.
        let rdown = (unsafe { GetAsyncKeyState(i32::from(VK_RBUTTON.0)) }) < 0;
        if !ldown && !rdown && !IN_DROP_SESSION.load(Ordering::Acquire) {
            DRAG_BTN_DOWN.store(false, Ordering::Relaxed);
            ARMED.store(false, Ordering::Relaxed);
            disarm_trigger(hwnd);
        }
        return LRESULT(0);
    }
    // SAFETY: forwards unhandled messages to `DefWindowProcW`.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let msg = wparam.0 as u32;
        // SAFETY: `lparam` points to an `MSLLHOOKSTRUCT` when `code == HC_ACTION` (>= 0).
        let ms = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        let pt = ms.pt;
        match msg {
            WM_LBUTTONDOWN | WM_RBUTTONDOWN => {
                DOWN_X.store(pt.x, Ordering::Relaxed);
                DOWN_Y.store(pt.y, Ordering::Relaxed);
                DRAG_BTN_DOWN.store(true, Ordering::Relaxed);
                let hwnd_raw = TRIGGER_HWND.load(Ordering::Acquire);
                if hwnd_raw != 0 {
                    // SAFETY: `hwnd_raw` is valid while worker runs.
                    let _ = unsafe {
                        PostMessageW(
                            Some(HWND(hwnd_raw as *mut _)),
                            WM_APP_BTN_DOWN,
                            WPARAM(pt.x as usize),
                            LPARAM(pt.y as isize),
                        )
                    };
                }
            }
            WM_LBUTTONUP | WM_RBUTTONUP => {
                DRAG_BTN_DOWN.store(false, Ordering::Relaxed);
                STARTED_AT_EDGE.store(false, Ordering::Relaxed);
                if ARMED.swap(false, Ordering::AcqRel) && !IN_DROP_SESSION.load(Ordering::Acquire) {
                    let hwnd_raw = TRIGGER_HWND.load(Ordering::Acquire);
                    if hwnd_raw != 0 {
                        // SAFETY: `hwnd_raw` is valid while worker runs.
                        let _ = unsafe {
                            PostMessageW(Some(HWND(hwnd_raw as *mut _)), WM_APP_DISARM, WPARAM(0), LPARAM(0))
                        };
                    }
                }
            }
            WM_MOUSEMOVE => {
                if DRAG_BTN_DOWN.load(Ordering::Relaxed) && !STARTED_AT_EDGE.load(Ordering::Relaxed) {
                    let dx = (pt.x - DOWN_X.load(Ordering::Relaxed)).abs();
                    let dy = (pt.y - DOWN_Y.load(Ordering::Relaxed)).abs();
                    if dx >= DRAG_THRESHOLD_PX || dy >= DRAG_THRESHOLD_PX {
                        let left = MON_LEFT.load(Ordering::Relaxed);
                        let right = MON_RIGHT.load(Ordering::Relaxed);
                        let top = MON_TOP.load(Ordering::Relaxed);
                        let bottom = MON_BOTTOM.load(Ordering::Relaxed);
                        let hwnd_raw = TRIGGER_HWND.load(Ordering::Acquire);
                        if pt.x < left || pt.x >= right || pt.y < top || pt.y >= bottom {
                            if hwnd_raw != 0 {
                                // Crossed onto another monitor mid-drag; refresh cached monitor bounds.
                                // SAFETY: `hwnd_raw` is valid while worker runs.
                                let _ = unsafe {
                                    PostMessageW(
                                        Some(HWND(hwnd_raw as *mut _)),
                                        WM_APP_BTN_DOWN,
                                        WPARAM(pt.x as usize),
                                        LPARAM(pt.y as isize),
                                    )
                                };
                            }
                        } else if HAS_OUTER_EDGE.load(Ordering::Relaxed) {
                            let edge_x = EDGE_X.load(Ordering::Relaxed);
                            let min_y = EDGE_MIN_Y.load(Ordering::Relaxed);
                            let max_y = EDGE_MAX_Y.load(Ordering::Relaxed);
                            let in_zone =
                                is_in_arm_zone(current_side(), pt.x, pt.y, edge_x, left, right, min_y, max_y);
                            if in_zone && !ARMED.swap(true, Ordering::AcqRel) {
                                if hwnd_raw != 0 {
                                    // SAFETY: `hwnd_raw` is valid while worker runs.
                                    let _ = unsafe {
                                        PostMessageW(
                                            Some(HWND(hwnd_raw as *mut _)),
                                            WM_APP_ARM,
                                            WPARAM(pt.x as usize),
                                            LPARAM(pt.y as isize),
                                        )
                                    };
                                }
                            } else if !in_zone
                                && !IN_DROP_SESSION.load(Ordering::Acquire)
                                && ARMED.swap(false, Ordering::AcqRel)
                                && hwnd_raw != 0
                            {
                                // SAFETY: `hwnd_raw` is valid while worker runs.
                                let _ = unsafe {
                                    PostMessageW(
                                        Some(HWND(hwnd_raw as *mut _)),
                                        WM_APP_DISARM,
                                        WPARAM(0),
                                        LPARAM(0),
                                    )
                                };
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    // SAFETY: always pass mouse events to the next hook in the chain.
    unsafe { CallNextHookEx(None::<HHOOK>, code, wparam, lparam) }
}

#[allow(clippy::too_many_arguments)]
fn is_in_arm_zone(
    side: Side,
    x: i32,
    y: i32,
    edge_x: i32,
    mon_left: i32,
    mon_right: i32,
    work_top: i32,
    work_bottom: i32,
) -> bool {
    if y < work_top || y >= work_bottom {
        return false;
    }
    match side {
        Side::Right => x >= edge_x - ARM_MARGIN_PX && x <= mon_right,
        Side::Left => x >= mon_left && x <= edge_x + ARM_MARGIN_PX,
    }
}

fn update_monitor_for_point(pt: POINT, check_edge_start: bool) {
    // SAFETY: `MonitorFromPoint` returns a valid HMONITOR or null.
    let hmon = unsafe { MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST) };
    if hmon.0.is_null() {
        HAS_OUTER_EDGE.store(false, Ordering::Relaxed);
        return;
    }
    let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    // SAFETY: `info.cbSize` is initialized.
    if !unsafe { GetMonitorInfoW(hmon, &mut info) }.as_bool() {
        HAS_OUTER_EDGE.store(false, Ordering::Relaxed);
        return;
    }
    let mon = info.rcMonitor;
    let work = info.rcWork;
    let side = current_side();
    let edge_x = match side {
        Side::Right => work.right,
        Side::Left => work.left,
    };
    let probe_pt = match side {
        Side::Right => POINT { x: mon.right + 4, y: pt.y.clamp(mon.top, mon.bottom - 1) },
        Side::Left => POINT { x: mon.left - 4, y: pt.y.clamp(mon.top, mon.bottom - 1) },
    };
    // Only treat this monitor edge as an outer drop edge if there is no neighbor monitor across it.
    // SAFETY: `MonitorFromPoint` with `MONITOR_DEFAULTTONULL` returns null when no monitor exists at `probe_pt`.
    let outer = unsafe { MonitorFromPoint(probe_pt, MONITOR_DEFAULTTONULL) }.0.is_null();

    MON_LEFT.store(mon.left, Ordering::Relaxed);
    MON_RIGHT.store(mon.right, Ordering::Relaxed);
    MON_TOP.store(mon.top, Ordering::Relaxed);
    MON_BOTTOM.store(mon.bottom, Ordering::Relaxed);
    EDGE_X.store(edge_x, Ordering::Relaxed);
    EDGE_MIN_Y.store(work.top, Ordering::Relaxed);
    EDGE_MAX_Y.store(work.bottom, Ordering::Relaxed);
    HAS_OUTER_EDGE.store(outer, Ordering::Relaxed);

    if check_edge_start {
        let started_near = match side {
            Side::Right => pt.x >= edge_x - SCROLLBAR_GUARD_PX,
            Side::Left => pt.x <= edge_x + SCROLLBAR_GUARD_PX,
        };
        STARTED_AT_EDGE.store(started_near, Ordering::Relaxed);
    }
}

fn set_transparent(hwnd: HWND, transparent: bool) {
    // SAFETY: `hwnd` is owned by this thread.
    let ex = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) } as u32;
    let wanted = if transparent { ex | WS_EX_TRANSPARENT.0 } else { ex & !WS_EX_TRANSPARENT.0 };
    if wanted != ex {
        // SAFETY: `hwnd` is owned by this thread.
        unsafe { SetWindowLongW(hwnd, GWL_EXSTYLE, wanted as i32) };
    }
}

fn arm_trigger(hwnd: HWND, _cursor_y: i32) {
    let top = EDGE_MIN_Y.load(Ordering::Relaxed);
    let bottom = EDGE_MAX_Y.load(Ordering::Relaxed);
    let edge_x = EDGE_X.load(Ordering::Relaxed);
    let h = (bottom - top).max(100);
    let x = match current_side() {
        Side::Right => edge_x - STRIP_WIDTH_PX,
        Side::Left => edge_x,
    };
    set_transparent(hwnd, false);
    // SAFETY: `hwnd` is owned by this thread.
    let _ = unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            x,
            top,
            STRIP_WIDTH_PX,
            h,
            SWP_NOACTIVATE | SWP_FRAMECHANGED | SWP_SHOWWINDOW,
        )
    };
    // SAFETY: `hwnd` is owned by this thread; watchdog prevents stuck non-transparent state.
    let _ = unsafe { SetTimer(Some(hwnd), TIMER_WATCHDOG, 250, None) };
}

fn disarm_trigger(hwnd: HWND) {
    // SAFETY: `hwnd` is owned by this thread.
    let _ = unsafe { KillTimer(Some(hwnd), TIMER_WATCHDOG) };
    set_transparent(hwnd, true);
    let top = EDGE_MIN_Y.load(Ordering::Relaxed);
    let bottom = EDGE_MAX_Y.load(Ordering::Relaxed);
    let edge_x = EDGE_X.load(Ordering::Relaxed);
    let h = (bottom - top).max(100);
    let x = match current_side() {
        Side::Right => edge_x - STRIP_WIDTH_PX,
        Side::Left => edge_x,
    };
    // SAFETY: `hwnd` is owned by this thread.
    let _ = unsafe {
        SetWindowPos(hwnd, Some(HWND_TOPMOST), x, top, STRIP_WIDTH_PX, h, SWP_NOACTIVATE | SWP_FRAMECHANGED)
    };
}

fn expand_trigger_for_pill(hwnd: HWND, cursor_y: i32) -> ((i32, i32, i32, i32), Side) {
    let pt = POINT {
        x: EDGE_X.load(Ordering::Relaxed) + if current_side() == Side::Right { -1 } else { 1 },
        y: cursor_y,
    };
    // SAFETY: `MonitorFromPoint` returns a valid HMONITOR or null.
    let hmon = unsafe { MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    let (work_left, work_top, work_right, work_bottom) =
        if !hmon.0.is_null() && unsafe { GetMonitorInfoW(hmon, &mut info) }.as_bool() {
            (info.rcWork.left, info.rcWork.top, info.rcWork.right, info.rcWork.bottom)
        } else {
            (
                MON_LEFT.load(Ordering::Relaxed),
                EDGE_MIN_Y.load(Ordering::Relaxed),
                MON_RIGHT.load(Ordering::Relaxed),
                EDGE_MAX_Y.load(Ordering::Relaxed),
            )
        };

    let side = current_side();
    let (x, y, w, h) = pill_capture_rect(side, (work_left, work_top, work_right, work_bottom), cursor_y);
    // SAFETY: `hwnd` is owned by this thread.
    let _ = unsafe {
        SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE | SWP_FRAMECHANGED | SWP_SHOWWINDOW)
    };
    ((work_left, work_top, work_right, work_bottom), side)
}

fn pill_capture_rect(side: Side, work: (i32, i32, i32, i32), cursor_y: i32) -> (i32, i32, i32, i32) {
    let (wl, wt, wr, wb) = work;
    let w = PILL_CAPTURE_W_PX.min((wr - wl).max(64));
    let h = PILL_CAPTURE_H_PX.min((wb - wt).max(48));
    let x = match side {
        Side::Right => wr - w,
        Side::Left => wl,
    };
    let y = (cursor_y - h / 2).clamp(wt, (wb - h).max(wt));
    (x, y, w, h)
}

fn has_cf_hdrop(pdataobj: Ref<'_, IDataObject>) -> bool {
    let Some(obj) = pdataobj.as_ref() else {
        return false;
    };
    let fmt = FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    // SAFETY: `fmt` is a valid stack `FORMATETC` for `CF_HDROP`.
    unsafe { obj.QueryGetData(&fmt) == S_OK }
}

fn extract_dropped_paths(pdataobj: Ref<'_, IDataObject>) -> Vec<PathBuf> {
    let Some(obj) = pdataobj.as_ref() else {
        return Vec::new();
    };
    let fmt = FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    // SAFETY: `GetData` fills `medium` which we release via `ReleaseStgMedium` below.
    let Ok(mut medium): windows::core::Result<STGMEDIUM> = (unsafe { obj.GetData(&fmt) }) else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    if medium.tymed == TYMED_HGLOBAL.0 as u32 {
        // SAFETY: `medium.u.hGlobal` is valid while `medium.tymed == TYMED_HGLOBAL`.
        let hglobal = unsafe { medium.u.hGlobal };
        // SAFETY: `GlobalLock` locks the HGLOBAL for reading `DROPFILES`.
        let ptr = unsafe { GlobalLock(hglobal) };
        if !ptr.is_null() {
            let hdrop = HDROP(ptr);
            // SAFETY: `hdrop` points to locked `DROPFILES` memory.
            let count = unsafe { DragQueryFileW(hdrop, 0xFFFF_FFFF, None) };
            for i in 0..count {
                // SAFETY: queries buffer length in UTF-16 code units.
                let len = unsafe { DragQueryFileW(hdrop, i, None) };
                if len == 0 {
                    continue;
                }
                let mut buf = vec![0u16; (len as usize) + 1];
                // SAFETY: `buf` has space for `len + 1` UTF-16 code units including NUL.
                let copied = unsafe { DragQueryFileW(hdrop, i, Some(&mut buf)) };
                if copied > 0 {
                    let pb = PathBuf::from(OsString::from_wide(&buf[..copied as usize]));
                    if pb.exists() {
                        paths.push(pb);
                    }
                }
            }
            // SAFETY: pairs with `GlobalLock` above.
            let _ = unsafe { GlobalUnlock(hglobal) };
        }
    }
    // SAFETY: releases the `STGMEDIUM` returned by `IDataObject::GetData`.
    unsafe { ReleaseStgMedium(&mut medium) };
    paths
}

fn shift_held(grfkeystate: MODIFIERKEYS_FLAGS) -> bool {
    if (grfkeystate.0 & MK_SHIFT.0) != 0 {
        return true;
    }
    // SAFETY: no preconditions.
    (unsafe { GetAsyncKeyState(i32::from(VK_SHIFT.0)) }) < 0
}

#[windows::core::implement(IDropTarget)]
struct EdgeDropTarget {
    hwnd: HWND,
    has_files: Cell<bool>,
}

impl IDropTarget_Impl for EdgeDropTarget_Impl {
    fn DragEnter(
        &self,
        pdataobj: Ref<'_, IDataObject>,
        grfkeystate: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        pdweffect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        if pdweffect.is_null() {
            return Err(DRAGDROP_E_INVALIDHWND.into());
        }
        let ok = ENABLED_IN_SETTINGS.load(Ordering::Relaxed)
            && crate::bridge::app::has_online_phone()
            && has_cf_hdrop(pdataobj);
        self.has_files.set(ok);
        if ok {
            IN_DROP_SESSION.store(true, Ordering::Release);
            // SAFETY: caller (`ole32!DoDragDrop`) passes a valid writable `pdweffect`.
            unsafe { *pdweffect = DROPEFFECT_COPY };
            let (work, side) = expand_trigger_for_pill(self.hwnd, pt.y);
            *lock(&ACTIVE_WORK) = Some(work);
            ACTIVE_CURSOR_Y.store(pt.y, Ordering::Relaxed);
            ACTIVE_SIDE.store(side.as_u8(), Ordering::Relaxed);
            crate::bridge::app::on_edge_drop_enter(shift_held(grfkeystate));
        } else {
            // SAFETY: caller passes a valid writable `pdweffect`.
            unsafe { *pdweffect = DROPEFFECT_NONE };
        }
        Ok(())
    }

    fn DragOver(
        &self,
        grfkeystate: MODIFIERKEYS_FLAGS,
        _pt: &POINTL,
        pdweffect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        if !pdweffect.is_null() {
            let eff = if self.has_files.get() { DROPEFFECT_COPY } else { DROPEFFECT_NONE };
            // SAFETY: caller passes a valid writable `pdweffect`.
            unsafe { *pdweffect = eff };
        }
        if self.has_files.get() {
            crate::bridge::app::on_edge_drop_shift(shift_held(grfkeystate));
        }
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        if self.has_files.replace(false) {
            IN_DROP_SESSION.store(false, Ordering::Release);
            ARMED.store(false, Ordering::Release);
            disarm_trigger(self.hwnd);
            crate::bridge::app::on_edge_drop_leave();
        }
        Ok(())
    }

    fn Drop(
        &self,
        pdataobj: Ref<'_, IDataObject>,
        grfkeystate: MODIFIERKEYS_FLAGS,
        _pt: &POINTL,
        pdweffect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        IN_DROP_SESSION.store(false, Ordering::Release);
        ARMED.store(false, Ordering::Release);
        DRAG_BTN_DOWN.store(false, Ordering::Release);
        STARTED_AT_EDGE.store(false, Ordering::Release);
        disarm_trigger(self.hwnd);

        let had_files = self.has_files.replace(false);
        if !had_files {
            if !pdweffect.is_null() {
                // SAFETY: caller passes a valid writable `pdweffect`.
                unsafe { *pdweffect = DROPEFFECT_NONE };
            }
            return Ok(());
        }

        let paths = extract_dropped_paths(pdataobj);
        if !pdweffect.is_null() {
            let eff = if paths.is_empty() { DROPEFFECT_NONE } else { DROPEFFECT_COPY };
            // SAFETY: caller passes a valid writable `pdweffect`.
            unsafe { *pdweffect = eff };
        }
        if paths.is_empty() {
            crate::bridge::app::on_edge_drop_leave();
        } else {
            crate::bridge::app::on_edge_drop_files(paths, shift_held(grfkeystate));
        }
        Ok(())
    }
}

/// Applies non-activating topmost tool-window styles to the QML pill window
/// (`title`) and aligns it flush against the active monitor work area at the
/// drag Y position.
pub fn position_pill_window(title: &str) {
    let Some(work) = *lock(&ACTIVE_WORK) else {
        crate::win::tray::apply_no_activate_style_by_title(title);
        return;
    };
    let cursor_y = ACTIVE_CURSOR_Y.load(Ordering::Relaxed);
    let side = Side::from_u8(ACTIVE_SIDE.load(Ordering::Relaxed));

    // SAFETY: enumerates top-level windows belonging to this process to position the pill window.
    unsafe {
        let self_pid = GetCurrentProcessId();
        let mut cur = match GetTopWindow(None) {
            Ok(h) => h,
            Err(_) => return,
        };
        for _ in 0..256 {
            let mut pid: u32 = 0;
            GetWindowThreadProcessId(cur, Some(&mut pid));
            if pid == self_pid && window_title(cur) == title {
                let ex = GetWindowLongW(cur, GWL_EXSTYLE) as u32;
                let wanted = ex | WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0;
                let _ = SetWindowLongW(cur, GWL_EXSTYLE, wanted as i32);

                let mut rect = RECT::default();
                let _ = GetWindowRect(cur, &mut rect);
                let (x, y) = pill_window_origin(
                    side,
                    work,
                    (rect.right - rect.left).max(200),
                    (rect.bottom - rect.top).max(48),
                    cursor_y,
                );
                let _ = SetWindowPos(cur, Some(HWND_TOPMOST), x, y, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE);
                // Re-read size in case moving onto a monitor with another DPI triggered WM_DPICHANGED.
                let mut rect2 = RECT::default();
                if GetWindowRect(cur, &mut rect2).is_ok()
                    && ((rect2.right - rect2.left) != (rect.right - rect.left)
                        || (rect2.bottom - rect2.top) != (rect.bottom - rect.top))
                {
                    let (x2, y2) = pill_window_origin(
                        side,
                        work,
                        rect2.right - rect2.left,
                        rect2.bottom - rect2.top,
                        cursor_y,
                    );
                    let _ = SetWindowPos(cur, Some(HWND_TOPMOST), x2, y2, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE);
                }
                break;
            }
            match GetWindow(cur, GW_HWNDNEXT) {
                Ok(next) if !next.0.is_null() => cur = next,
                _ => break,
            }
        }
    }
}

fn pill_window_origin(
    side: Side,
    work: (i32, i32, i32, i32),
    win_w: i32,
    win_h: i32,
    cursor_y: i32,
) -> (i32, i32) {
    let (wl, wt, wr, wb) = work;
    let margin = 4;
    let x = match side {
        Side::Right => wr - win_w - margin,
        Side::Left => wl + margin,
    };
    let y = (cursor_y - win_h / 2).clamp(wt + margin, (wb - win_h - margin).max(wt + margin));
    (x, y)
}

fn window_title(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    // SAFETY: `buf` is a live mutable UTF-16 buffer.
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
    if len <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..len as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_parse_defaults_to_right() {
        assert_eq!(Side::parse("left"), Side::Left);
        assert_eq!(Side::parse(" LEFT "), Side::Left);
        assert_eq!(Side::parse("right"), Side::Right);
        assert_eq!(Side::parse(""), Side::Right);
        assert_eq!(Side::parse("top"), Side::Right);
    }

    #[test]
    fn arm_zone_checks_edge_and_work_area_y() {
        assert!(is_in_arm_zone(Side::Right, 1915, 500, 1920, 0, 1920, 0, 1032));
        assert!(!is_in_arm_zone(Side::Right, 1880, 500, 1920, 0, 1920, 0, 1032));
        assert!(!is_in_arm_zone(Side::Right, 1918, 1050, 1920, 0, 1920, 0, 1032));
        assert!(is_in_arm_zone(Side::Left, 4, 500, 0, 0, 1920, 0, 1032));
        assert!(!is_in_arm_zone(Side::Left, 25, 500, 0, 0, 1920, 0, 1032));
    }

    #[test]
    fn pill_origin_clamps_inside_work_area() {
        let work = (0, 0, 1920, 1032);
        assert_eq!(pill_window_origin(Side::Right, work, 296, 64, 500), (1620, 468));
        assert_eq!(pill_window_origin(Side::Left, work, 296, 64, 10), (4, 4));
        assert_eq!(pill_window_origin(Side::Right, work, 296, 64, 1030), (1620, 964));
    }
}
