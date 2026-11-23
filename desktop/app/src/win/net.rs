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
        CancelMibChangeNotify2, MIB_IPFORWARD_ROW2, MIB_IPINTERFACE_ROW, MIB_NOTIFICATION_TYPE,
        MIB_UNICASTIPADDRESS_ROW, NotifyIpInterfaceChange, NotifyRouteChange2, NotifyUnicastIpAddressChange,
    },
    Networking::WinSock::AF_UNSPEC,
};

type Callback = Box<dyn Fn() + Send + Sync>;

static CALLBACK: OnceLock<Callback> = OnceLock::new();
static HANDLES: Mutex<Vec<isize>> = Mutex::new(Vec::new());

fn fire_change() {
    if let Some(callback) = CALLBACK.get() {
        callback();
    }
}

unsafe extern "system" fn on_interface_change(
    _: *const c_void,
    _: *const MIB_IPINTERFACE_ROW,
    _: MIB_NOTIFICATION_TYPE,
) {
    fire_change();
}

unsafe extern "system" fn on_unicast_change(
    _: *const c_void,
    _: *const MIB_UNICASTIPADDRESS_ROW,
    _: MIB_NOTIFICATION_TYPE,
) {
    fire_change();
}

unsafe extern "system" fn on_route_change(
    _: *const c_void,
    _: *const MIB_IPFORWARD_ROW2,
    _: MIB_NOTIFICATION_TYPE,
) {
    fire_change();
}

/// Calls `callback` (on a system thread) whenever a network interface, unicast
/// address, or route changes (including Tailscale / VPN transitions). Bursts
/// are expected; the callback should debounce.
pub fn watch(callback: impl Fn() + Send + Sync + 'static) {
    if CALLBACK.set(Box::new(callback)).is_err() {
        return; // already watching
    }
    let mut handles = Vec::with_capacity(3);
    // SAFETY: the callbacks are 'static functions; each returned handle is
    // stored and released once in `stop`.
    unsafe {
        let mut h = HANDLE::default();
        let status = NotifyIpInterfaceChange(AF_UNSPEC, Some(on_interface_change), None, false, &mut h);
        if status == NO_ERROR {
            handles.push(h.0 as isize);
        } else {
            tracing::warn!(?status, "can't watch for interface changes");
        }

        let mut h = HANDLE::default();
        let status = NotifyUnicastIpAddressChange(AF_UNSPEC, Some(on_unicast_change), None, false, &mut h);
        if status == NO_ERROR {
            handles.push(h.0 as isize);
        } else {
            tracing::debug!(?status, "can't watch for unicast address changes");
        }

        let mut h = HANDLE::default();
        let status = NotifyRouteChange2(AF_UNSPEC, Some(on_route_change), std::ptr::null(), false, &mut h);
        if status == NO_ERROR {
            handles.push(h.0 as isize);
        } else {
            tracing::debug!(?status, "can't watch for route changes");
        }
    }
    *HANDLES.lock().unwrap_or_else(|e| e.into_inner()) = handles;
}

/// Stops the notifications (before exit).
pub fn stop() {
    let handles = std::mem::take(&mut *HANDLES.lock().unwrap_or_else(|e| e.into_inner()));
    for raw in handles {
        // SAFETY: each handle came from Notify*Change2 and is cancelled once.
        unsafe {
            let _ = CancelMibChangeNotify2(HANDLE(raw as *mut c_void));
        }
    }
}
