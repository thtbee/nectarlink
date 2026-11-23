// SPDX-License-Identifier: GPL-3.0-or-later
//! Watching for IP address and network route changes on Linux via a Netlink
//! `NETLINK_ROUTE` socket (with a `/proc/net/route` fallback), debounced so a
//! single network transition triggers a leading callback and coalesces trailing
//! events (such as `RTM_NEWADDR` arriving shortly after `RTM_NEWLINK`) without
//! dropping them.

use std::{
    fs,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
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
const RTMGRP_LINK: u32 = 1;
const RTMGRP_IPV4_IFADDR: u32 = 0x10;
const RTMGRP_IPV4_ROUTE: u32 = 0x40;
const RTMGRP_IPV6_IFADDR: u32 = 0x100;
const RTMGRP_IPV6_ROUTE: u32 = 0x400;

#[repr(C)]
struct SockAddrNl {
    nl_family: u16,
    nl_pad: u16,
    nl_pid: u32,
    nl_groups: u32,
}

struct StopHandle {
    flag: Arc<AtomicBool>,
    wake_fd: Option<Arc<OwnedFd>>,
}

static STOP_STATE: Mutex<Option<StopHandle>> = Mutex::new(None);

fn proc_network_snapshot() -> String {
    let route = fs::read_to_string("/proc/net/route").unwrap_or_default();
    let inet6 = fs::read_to_string("/proc/net/if_inet6").unwrap_or_default();
    let fib_trie = fs::read_to_string("/proc/net/arp").unwrap_or_default();
    format!("{route}\n---\n{inet6}\n---\n{fib_trie}")
}

fn create_wake_eventfd() -> Option<Arc<OwnedFd>> {
    // SAFETY: `eventfd` with `EFD_CLOEXEC | EFD_NONBLOCK` has no preconditions.
    let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
    if fd < 0 {
        None
    } else {
        // SAFETY: `fd` is a freshly created file descriptor owned by us.
        Some(Arc::new(unsafe { OwnedFd::from_raw_fd(fd) }))
    }
}

fn drain_socket(fd: i32, buf: &mut [u8]) {
    loop {
        // SAFETY: `buf` is a valid mutable slice and `fd` is an open socket.
        let n = unsafe { libc::recv(fd, buf.as_mut_ptr().cast(), buf.len(), libc::MSG_DONTWAIT) };
        if n <= 0 {
            break;
        }
    }
}

fn watch_netlink_loop(fd: i32, wake_fd: i32, stop_flag: &AtomicBool, on_change: &dyn Fn()) {
    let mut buf = [0u8; 4096];
    let mut last_emit: Option<Instant> = None;

    while !stop_flag.load(Ordering::Relaxed) {
        let mut pfds = [
            libc::pollfd { fd, events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: wake_fd, events: libc::POLLIN, revents: 0 },
        ];
        let nfds: libc::nfds_t = if wake_fd >= 0 { 2 } else { 1 };
        // SAFETY: `pfds` is a valid stack array of `nfds` pollfd structs.
        let rc = unsafe { libc::poll(pfds.as_mut_ptr(), nfds, 1000) };
        if stop_flag.load(Ordering::Relaxed) || (wake_fd >= 0 && (pfds[1].revents & libc::POLLIN) != 0) {
            break;
        }
        if rc <= 0 || (pfds[0].revents & libc::POLLIN) == 0 {
            continue;
        }

        // SAFETY: `buf` is a live stack buffer and `fd` is our open socket.
        let n = unsafe { libc::recv(fd, buf.as_mut_ptr().cast(), buf.len(), libc::MSG_DONTWAIT) };
        if n <= 0 || stop_flag.load(Ordering::Relaxed) {
            continue;
        }
        drain_socket(fd, &mut buf);

        if let Some(prev) = last_emit {
            let elapsed = prev.elapsed();
            if elapsed < DEBOUNCE {
                let remaining = DEBOUNCE - elapsed;
                let timeout_ms = remaining.as_millis().clamp(1, i32::MAX as u128) as i32;
                if wake_fd >= 0 {
                    let mut wake_pfd = [libc::pollfd { fd: wake_fd, events: libc::POLLIN, revents: 0 }];
                    // SAFETY: `wake_pfd` is a valid 1-element pollfd array.
                    let _ = unsafe { libc::poll(wake_pfd.as_mut_ptr(), 1, timeout_ms) };
                    if stop_flag.load(Ordering::Relaxed) || (wake_pfd[0].revents & libc::POLLIN) != 0 {
                        break;
                    }
                } else {
                    std::thread::sleep(remaining);
                    if stop_flag.load(Ordering::Relaxed) {
                        break;
                    }
                }
                drain_socket(fd, &mut buf);
            }
        }

        if stop_flag.load(Ordering::Relaxed) {
            break;
        }
        last_emit = Some(Instant::now());
        on_change();
    }
}

/// Calls `callback` whenever an IP address or route is added or removed on the PC.
/// Replacing an earlier watch stops the previous background thread.
pub fn watch(callback: impl Fn() + Send + Sync + 'static) {
    stop();
    let stop_flag = Arc::new(AtomicBool::new(false));
    let wake_fd = create_wake_eventfd();
    *STOP_STATE.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(StopHandle { flag: stop_flag.clone(), wake_fd: wake_fd.clone() });

    let _ = std::thread::Builder::new().name("net-watch".into()).spawn(move || {
        let raw_wake_fd = wake_fd.as_ref().map_or(-1, |f| f.as_raw_fd());

        // Try opening an AF_NETLINK route socket subscribed to link + IPv4/IPv6 address groups.
        // SAFETY: creating a standard non-privileged NETLINK_ROUTE socket.
        let fd = unsafe { libc::socket(AF_NETLINK, SOCK_RAW | SOCK_CLOEXEC, NETLINK_ROUTE) };
        if fd >= 0 {
            let bound = unsafe {
                let addr = SockAddrNl {
                    nl_family: AF_NETLINK as u16,
                    nl_pad: 0,
                    nl_pid: 0,
                    nl_groups: RTMGRP_LINK
                        | RTMGRP_IPV4_IFADDR
                        | RTMGRP_IPV6_IFADDR
                        | RTMGRP_IPV4_ROUTE
                        | RTMGRP_IPV6_ROUTE,
                };
                libc::bind(fd, (&raw const addr).cast(), std::mem::size_of::<SockAddrNl>() as u32) == 0
            };
            if bound {
                watch_netlink_loop(fd, raw_wake_fd, &stop_flag, &callback);
                // SAFETY: closing our own socket fd.
                unsafe {
                    let _ = libc::close(fd);
                }
                return;
            }
            // SAFETY: closing our own socket fd when bind failed.
            unsafe {
                let _ = libc::close(fd);
            }
        }

        // Fallback: poll `/proc/net/route` and `/proc/net/if_inet6`.
        let mut last_snapshot = proc_network_snapshot();
        while !stop_flag.load(Ordering::Relaxed) {
            if raw_wake_fd >= 0 {
                let mut wake_pfd = [libc::pollfd { fd: raw_wake_fd, events: libc::POLLIN, revents: 0 }];
                // SAFETY: `wake_pfd` is a valid 1-element pollfd array.
                let _ = unsafe { libc::poll(wake_pfd.as_mut_ptr(), 1, 2000) };
            } else {
                std::thread::sleep(Duration::from_secs(2));
            }
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
    if let Some(handle) = STOP_STATE.lock().unwrap_or_else(|e| e.into_inner()).take() {
        handle.flag.store(true, Ordering::Relaxed);
        if let Some(wake) = handle.wake_fd {
            let val: u64 = 1;
            // SAFETY: writing an 8-byte counter increment to a valid eventfd.
            unsafe {
                let _ = libc::write(wake.as_raw_fd(), (&raw const val).cast(), std::mem::size_of::<u64>());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn starts_and_stops_net_watcher_cleanly() {
        watch(|| {});
        stop();
    }

    #[test]
    fn trailing_event_within_debounce_window_is_coalesced_and_emitted_not_dropped() {
        let mut fds = [0i32; 2];
        // SAFETY: creating an AF_UNIX datagram socketpair for testing `watch_netlink_loop`.
        let rc = unsafe {
            libc::socketpair(libc::AF_UNIX, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0, fds.as_mut_ptr())
        };
        assert_eq!(rc, 0);
        let read_fd = fds[0];
        let write_fd = fds[1];

        let wake = create_wake_eventfd().expect("eventfd");
        let wake_raw = wake.as_raw_fd();
        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop_clone = stop_flag.clone();
        let count = Arc::new(AtomicUsize::new(0));
        let count_clone = count.clone();

        let worker = std::thread::spawn(move || {
            watch_netlink_loop(read_fd, wake_raw, &stop_clone, &move || {
                count_clone.fetch_add(1, Ordering::SeqCst);
            });
        });

        let msg1 = b"RTM_NEWLINK";
        let msg2 = b"RTM_NEWADDR";
        let msg3 = b"RTM_NEWROUTE";
        unsafe {
            let _ = libc::send(write_fd, msg1.as_ptr().cast(), msg1.len(), 0);
        }
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(count.load(Ordering::SeqCst), 1, "leading event should fire immediately");

        // Send two trailing events inside the 500ms window after `msg1`.
        unsafe {
            let _ = libc::send(write_fd, msg2.as_ptr().cast(), msg2.len(), 0);
        }
        std::thread::sleep(Duration::from_millis(50));
        unsafe {
            let _ = libc::send(write_fd, msg3.as_ptr().cast(), msg3.len(), 0);
        }

        // Wait for the remaining 500ms debounce window to complete and emit the trailing callback.
        std::thread::sleep(Duration::from_millis(450));
        assert_eq!(
            count.load(Ordering::SeqCst),
            2,
            "trailing RTM_NEWADDR/RTM_NEWROUTE within 500ms must fire after debounce instead of being dropped"
        );

        stop_flag.store(true, Ordering::SeqCst);
        let one: u64 = 1;
        unsafe {
            let _ = libc::write(wake.as_raw_fd(), (&raw const one).cast(), std::mem::size_of::<u64>());
        }
        let _ = worker.join();
        unsafe {
            let _ = libc::close(read_fd);
            let _ = libc::close(write_fd);
        }
    }
}
