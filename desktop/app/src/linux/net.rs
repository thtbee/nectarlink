// SPDX-License-Identifier: GPL-3.0-or-later
//! Watching for IP address and network route changes on Linux via a Netlink
//! `NETLINK_ROUTE` socket (with a `/proc/net/route` fallback), debounced so a
//! single network transition triggers one callback.

use std::{
    fs,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const DEBOUNCE: Duration = Duration::from_millis(500);

const AF_NETLINK: i32 = 16;
const SOCK_RAW: i32 = 3;
const SOCK_CLOEXEC: i32 = 0o2000000;
const NETLINK_ROUTE: i32 = 0;
const SOL_SOCKET: i32 = 1;
const SO_RCVTIMEO: i32 = 20;
const RTMGRP_LINK: u32 = 1;
const RTMGRP_IPV4_IFADDR: u32 = 0x10;
const RTMGRP_IPV4_ROUTE: u32 = 0x40;
const RTMGRP_IPV6_IFADDR: u32 = 0x100;

#[repr(C)]
struct SockAddrNl {
    nl_family: u16,
    nl_pad: u16,
    nl_pid: u32,
    nl_groups: u32,
}

#[repr(C)]
struct Timeval {
    tv_sec: i64,
    tv_usec: i64,
}

unsafe extern "C" {
    fn socket(domain: i32, ty: i32, protocol: i32) -> i32;
    fn setsockopt(fd: i32, level: i32, optname: i32, optval: *const std::ffi::c_void, optlen: u32) -> i32;
    fn bind(fd: i32, addr: *const std::ffi::c_void, addrlen: u32) -> i32;
    fn recv(fd: i32, buf: *mut std::ffi::c_void, len: usize, flags: i32) -> isize;
    fn close(fd: i32) -> i32;
}

static STOP_FLAG: Mutex<Option<Arc<AtomicBool>>> = Mutex::new(None);

fn proc_network_snapshot() -> String {
    let route = fs::read_to_string("/proc/net/route").unwrap_or_default();
    let inet6 = fs::read_to_string("/proc/net/if_inet6").unwrap_or_default();
    let fib_trie = fs::read_to_string("/proc/net/arp").unwrap_or_default();
    format!("{route}\n---\n{inet6}\n---\n{fib_trie}")
}

/// Calls `callback` whenever an IP address or route is added or removed on the PC.
/// Replacing an earlier watch stops the previous background thread.
pub fn watch(callback: impl Fn() + Send + Sync + 'static) {
    stop();
    let stop_flag = Arc::new(AtomicBool::new(false));
    *STOP_FLAG.lock().unwrap_or_else(|e| e.into_inner()) = Some(stop_flag.clone());

    let _ = std::thread::Builder::new().name("net-watch".into()).spawn(move || {
        // Try opening an AF_NETLINK route socket subscribed to link + IPv4/IPv6 address groups.
        // SAFETY: creating a standard non-privileged NETLINK_ROUTE socket with a receive timeout.
        let fd = unsafe { socket(AF_NETLINK, SOCK_RAW | SOCK_CLOEXEC, NETLINK_ROUTE) };
        if fd >= 0 {
            let bound = unsafe {
                let addr = SockAddrNl {
                    nl_family: AF_NETLINK as u16,
                    nl_pad: 0,
                    nl_pid: 0,
                    nl_groups: RTMGRP_LINK | RTMGRP_IPV4_IFADDR | RTMGRP_IPV6_IFADDR | RTMGRP_IPV4_ROUTE,
                };
                let tv = Timeval { tv_sec: 1, tv_usec: 0 };
                let _ = setsockopt(
                    fd,
                    SOL_SOCKET,
                    SO_RCVTIMEO,
                    (&raw const tv).cast(),
                    std::mem::size_of::<Timeval>() as u32,
                );
                bind(fd, (&raw const addr).cast(), std::mem::size_of::<SockAddrNl>() as u32) == 0
            };
            if bound {
                let mut buf = [0u8; 4096];
                let mut last_fired: Option<Instant> = None;
                while !stop_flag.load(Ordering::Relaxed) {
                    // SAFETY: `buf` is a live stack buffer and `fd` is our open socket.
                    let n = unsafe { recv(fd, buf.as_mut_ptr().cast(), buf.len(), 0) };
                    if n > 0 && !stop_flag.load(Ordering::Relaxed) {
                        let now = Instant::now();
                        if last_fired.is_none_or(|prev| now.duration_since(prev) >= DEBOUNCE) {
                            last_fired = Some(now);
                            callback();
                        }
                    }
                }
                // SAFETY: closing our own socket fd.
                unsafe {
                    let _ = close(fd);
                }
                return;
            }
            // SAFETY: closing our own socket fd when bind failed.
            unsafe {
                let _ = close(fd);
            }
        }

        // Fallback: poll `/proc/net/route` and `/proc/net/if_inet6`.
        let mut last_snapshot = proc_network_snapshot();
        while !stop_flag.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_secs(2));
            if stop_flag.load(Ordering::Relaxed) {
                break;
            }
            let current = proc_network_snapshot();
            if current != last_snapshot {
                last_snapshot = current;
                callback();
            }
        }
    });
}

/// Alias for [`watch`].
pub fn watch_address_changes(on_change: impl Fn() + Send + Sync + 'static) {
    watch(on_change);
}

/// Stops watching for network address changes.
pub fn stop() {
    if let Some(flag) = STOP_FLAG.lock().unwrap_or_else(|e| e.into_inner()).take() {
        flag.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_and_stops_net_watcher_cleanly() {
        watch(|| {});
        stop();
    }
}
