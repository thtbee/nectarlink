// SPDX-License-Identifier: GPL-3.0-or-later
//! Network change notifications (Wi-Fi switched, cable plugged, VPN up), so
//! paired devices reconnect right away instead of after their backoff.

use std::{
    ffi::c_void,
    sync::{Mutex, OnceLock},
};

use windows::Win32::{
    Foundation::{HANDLE, NO_ERROR},
    NetworkManagement::IpHelper::{
        CancelMibChangeNotify2, MIB_IPINTERFACE_ROW, MIB_NOTIFICATION_TYPE, NotifyIpInterfaceChange,
    },
    Networking::WinSock::AF_UNSPEC,
};

type Callback = Box<dyn Fn() + Send + Sync>;

static CALLBACK: OnceLock<Callback> = OnceLock::new();
static HANDLE_: Mutex<Option<isize>> = Mutex::new(None);

unsafe extern "system" fn on_change(
    _: *const c_void,
    _: *const MIB_IPINTERFACE_ROW,
    _: MIB_NOTIFICATION_TYPE,
) {
    if let Some(callback) = CALLBACK.get() {
        callback();
    }
}

/// Calls `callback` (on a system thread) whenever a network interface
/// changes. Bursts are expected; the callback should debounce.
pub fn watch(callback: impl Fn() + Send + Sync + 'static) {
    if CALLBACK.set(Box::new(callback)).is_err() {
        return; // already watching
    }
    let mut handle = HANDLE::default();
    // SAFETY: the callback is a 'static function; the handle is stored and
    // released in `stop`.
    let status = unsafe { NotifyIpInterfaceChange(AF_UNSPEC, Some(on_change), None, false, &mut handle) };
    if status == NO_ERROR {
        *HANDLE_.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle.0 as isize);
    } else {
        tracing::warn!(?status, "can't watch for network changes");
    }
}

/// Stops the notifications (before exit).
pub fn stop() {
    if let Some(raw) = HANDLE_.lock().unwrap_or_else(|e| e.into_inner()).take() {
        // SAFETY: the handle came from NotifyIpInterfaceChange and is
        // cancelled once.
        unsafe {
            let _ = CancelMibChangeNotify2(HANDLE(raw as *mut c_void));
        }
    }
}
