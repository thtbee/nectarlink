// SPDX-License-Identifier: GPL-3.0-or-later
//! Linux platform integration (`desktop/app/src/linux/`), matching the public
//! API of `desktop/app/src/win/`.

#![allow(dead_code, unsafe_code)]

pub mod audio;
pub mod audio_out;
pub mod autostart;
pub mod ble_beacon;
pub mod browser;
pub mod bt_hfp;
pub mod clipboard;
pub mod doctor;
pub mod elevated;
pub mod h264;
pub mod http;
pub mod icon;
pub mod image;
pub mod input;
pub mod jumplist;
pub mod keymap;
pub mod media_sessions;
pub mod mp4;
pub mod net;
pub mod share_target;
pub mod shell;
pub mod shortcut;
pub mod single_instance;
pub mod smtc;
pub mod sound;
pub mod storage;
pub mod toast;
pub mod tray;
pub mod vcam;
#[cfg(linux_ffmpeg_video)]
pub(crate) mod video_decoder;
#[cfg(linux_ffmpeg_video)]
pub(crate) mod video_encoder;
pub mod vmic;
pub mod wallpaper;
pub mod widget;
pub mod wifi;
pub mod window_capture;
pub mod wol;

use std::{fs, io::Read, process::Command};

/// How long a signalled quit may take before the process ends anyway, so
/// logging out never waits on the app.
const SIGNAL_QUIT_GRACE: std::time::Duration = std::time::Duration::from_secs(15);

/// Quits the app cleanly on SIGTERM (logout, shutdown, `kill`), SIGINT and
/// SIGHUP, the way Quit does, so phone storage is unmounted and the phone
/// sees the PC leave. A second signal, or a quit that takes longer than
/// [`SIGNAL_QUIT_GRACE`], ends the process at once.
///
/// The handler only writes a byte to a pipe (all a signal handler may safely
/// do); a thread reads it and asks the app to quit. The signal mask is left
/// alone, so processes the app starts still get their signals.
pub fn quit_on_signals(quit: fn()) {
    use std::sync::atomic::{AtomicI32, Ordering};
    static WRITE_END: AtomicI32 = AtomicI32::new(-1);

    extern "C" fn on_signal(_: libc::c_int) {
        let fd = WRITE_END.load(Ordering::Relaxed);
        if fd >= 0 {
            let byte = 1u8;
            // SAFETY: write(2) is async-signal-safe; the pipe stays open for the process's life.
            let _ = unsafe { libc::write(fd, (&raw const byte).cast(), 1) };
        }
    }

    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: pipe2 fills two descriptors on success.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return;
    }
    let (read_end, write_end) = (fds[0], fds[1]);
    let spawned = std::thread::Builder::new().name("signals".into()).spawn(move || {
        let mut signals = 0;
        loop {
            let mut byte = 0u8;
            // SAFETY: reads one byte into a local buffer from our own pipe.
            let n = unsafe { libc::read(read_end, (&raw mut byte).cast(), 1) };
            if n < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            if n != 1 {
                return;
            }
            signals += 1;
            if signals > 1 {
                crate::storage::shutdown();
                std::process::exit(130);
            }
            tracing::info!("quitting on a signal");
            quit();
            let _ = std::thread::Builder::new().name("signal-quit-grace".into()).spawn(|| {
                std::thread::sleep(SIGNAL_QUIT_GRACE);
                tracing::warn!("quit took too long; ending");
                crate::storage::shutdown();
                std::process::exit(0);
            });
        }
    });
    if spawned.is_err() {
        return;
    }
    WRITE_END.store(write_end, Ordering::Relaxed);
    for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        // SAFETY: installs a handler that only calls write(2); the struct is
        // zeroed, then the handler, flags and an empty mask are set.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
            action.sa_flags = libc::SA_RESTART;
            libc::sigemptyset(&mut action.sa_mask);
            libc::sigaction(signal, &action, std::ptr::null_mut());
        }
    }
}

/// A stable machine identifier from `/etc/machine-id` or `/var/lib/dbus/machine-id`.
pub fn machine_guid() -> Option<String> {
    for path in ["/etc/machine-id", "/var/lib/dbus/machine-id"] {
        if let Ok(contents) = fs::read_to_string(path) {
            let trimmed = contents.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_owned());
            }
        }
    }
    None
}

/// Human-readable Linux distribution and kernel version (e.g. `"Ubuntu 24.04 LTS (6.8.0)"`).
pub fn os_version() -> String {
    let distro = fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|text| parse_os_release_pretty_name(&text))
        .unwrap_or_else(|| "Linux".to_owned());
    let kernel = fs::read_to_string("/proc/sys/kernel/osrelease")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    match kernel {
        Some(k) => format!("{distro} ({k})"),
        None => distro,
    }
}

fn parse_os_release_pretty_name(contents: &str) -> Option<String> {
    for line in contents.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("PRETTY_NAME=") {
            let unquoted = rest.trim().trim_matches('"').trim_matches('\'').trim();
            if !unquoted.is_empty() {
                return Some(unquoted.to_owned());
            }
        }
    }
    None
}

/// Returns a synthetic build number (`22621`) so Windows 11 feature gates in shared UI stay enabled.
pub fn os_build_number() -> u32 {
    22621
}

/// Whether this PC is a laptop (battery present in `/sys/class/power_supply` or mobile DMI chassis).
pub fn is_laptop() -> bool {
    if let Ok(entries) = fs::read_dir("/sys/class/power_supply") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("BAT") {
                return true;
            }
            if let Ok(kind) = fs::read_to_string(entry.path().join("type"))
                && kind.trim().eq_ignore_ascii_case("Battery")
            {
                return true;
            }
        }
    }
    if let Ok(chassis) = fs::read_to_string("/sys/class/dmi/id/chassis_type") {
        let c = chassis.trim();
        // 8 = Portable, 9 = Laptop, 10 = Notebook, 14 = Sub Notebook,
        // 30 = Tablet, 31 = Convertible, 32 = Detachable
        if matches!(c, "8" | "9" | "10" | "14" | "30" | "31" | "32") {
            return true;
        }
    }
    false
}

/// Whether power-saver mode is active (`/sys/firmware/acpi/platform_profile` or `powerprofilesctl`).
/// Whether this PC runs on its battery right now: it has one, and no mains
/// supply says it's online.
pub fn on_battery() -> bool {
    let Ok(entries) = fs::read_dir("/sys/class/power_supply") else { return false };
    let mut battery = false;
    for entry in entries.flatten() {
        let kind = fs::read_to_string(entry.path().join("type")).unwrap_or_default();
        match kind.trim() {
            "Mains" | "USB" => {
                if fs::read_to_string(entry.path().join("online")).is_ok_and(|o| o.trim() == "1") {
                    return false;
                }
            }
            "Battery" => battery = true,
            _ => {}
        }
    }
    battery
}

pub fn power_saver() -> bool {
    if let Ok(profile) = fs::read_to_string("/sys/firmware/acpi/platform_profile")
        && profile.trim().eq_ignore_ascii_case("low-power")
    {
        return true;
    }
    if let Ok(output) = Command::new("powerprofilesctl").arg("get").output()
        && output.status.success()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        if text.trim().eq_ignore_ascii_case("power-saver") {
            return true;
        }
    }
    false
}

/// Parses GTK_THEME, gsettings color-scheme, and kdeglobals ColorScheme to decide dark mode.
pub(crate) fn parse_system_dark_sources(
    gtk_theme: Option<&str>,
    gsettings_color_scheme: Option<&str>,
    kdeglobals: Option<&str>,
) -> bool {
    if let Some(gtk) = gtk_theme {
        let lower = gtk.to_ascii_lowercase();
        if lower.contains("dark") {
            return true;
        }
        if lower.contains("light") {
            return false;
        }
    }
    if let Some(gs) = gsettings_color_scheme {
        let text = gs.to_ascii_lowercase();
        if text.contains("prefer-dark") {
            return true;
        }
        if text.contains("prefer-light") || text.contains("default") {
            return false;
        }
    }
    if let Some(kde) = kdeglobals {
        for line in kde.lines() {
            if let Some(scheme) = line.trim().strip_prefix("ColorScheme=") {
                return scheme.to_ascii_lowercase().contains("dark");
            }
        }
    }
    true
}

/// Runs `f` outside any active Tokio runtime context so `zbus::blocking` calls
/// (which use `zbus`'s internal Tokio `Runtime::block_on` when compiled with
/// the `tokio` feature) never panic with "Cannot start a runtime from within a
/// runtime" when invoked from a `nectarlink-core` worker thread.
pub(crate) fn non_tokio<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    if tokio::runtime::Handle::try_current().is_ok() {
        std::thread::scope(|s| match s.spawn(f).join() {
            Ok(v) => v,
            Err(payload) => std::panic::resume_unwind(payload),
        })
    } else {
        f()
    }
}

/// Returns `true` if `bus_name` currently has an active owner on `conn` without
/// triggering a 25-second D-Bus service activation timeout.
pub(crate) fn dbus_name_has_owner(conn: &zbus::blocking::Connection, bus_name: &str) -> bool {
    let Ok(reply) = conn.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "NameHasOwner",
        &(bus_name,),
    ) else {
        return false;
    };
    reply.body().deserialize::<bool>().unwrap_or(false)
}

/// Runs a short-lived helper command with `PR_SET_PDEATHSIG(SIGTERM)` and a
/// strict wall-clock timeout, killing and reaping the child if it does not
/// finish within `timeout`.
pub(crate) fn run_cmd_timeout(
    bin: &str,
    args: &[&str],
    timeout: std::time::Duration,
) -> Option<std::process::Output> {
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;

    let mut cmd = Command::new(bin);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    // SAFETY: `prctl(PR_SET_PDEATHSIG, SIGTERM)` is async-signal-safe.
    unsafe {
        cmd.pre_exec(|| {
            let _ = libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
    let mut child = cmd.spawn().ok()?;
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    let out_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut s) = stdout_pipe.take() {
            let _ = s.read_to_end(&mut buf);
        }
        buf
    });
    let err_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut s) = stderr_pipe.take() {
            let _ = s.read_to_end(&mut buf);
        }
        buf
    });

    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stdout = out_handle.join().unwrap_or_default();
                let stderr = err_handle.join().unwrap_or_default();
                return Some(std::process::Output { status, stdout, stderr });
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(15));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// Queries the XDG Desktop Portal (`org.freedesktop.portal.Settings`) for an
/// `org.freedesktop.appearance` `u32` setting (`color-scheme` or `reduced-motion`).
fn read_portal_appearance_u32(key: &str) -> Option<u32> {
    non_tokio(|| {
        let conn = zbus::blocking::Connection::session().ok()?;
        if !dbus_name_has_owner(&conn, "org.freedesktop.portal.Desktop") {
            return None;
        }
        let msg = conn
            .call_method(
                Some("org.freedesktop.portal.Desktop"),
                "/org/freedesktop/portal/desktop",
                Some("org.freedesktop.portal.Settings"),
                "ReadOne",
                &("org.freedesktop.appearance", key),
            )
            .ok()?;
        let body = msg.body();
        if let Ok(val) = body.deserialize::<zbus::zvariant::OwnedValue>() {
            if let Ok(n) = u32::try_from(&val) {
                return Some(n);
            }
            if let zbus::zvariant::Value::Value(inner) = &*val
                && let Ok(n) = u32::try_from(&**inner)
            {
                return Some(n);
            }
        }
        None
    })
}

/// Whether the desktop prefers a dark color scheme.
pub fn system_dark() -> bool {
    // XDG Desktop Portal `color-scheme`: 1 = prefer dark, 2 = prefer light, 0 = no preference.
    match read_portal_appearance_u32("color-scheme") {
        Some(1) => return true,
        Some(2) => return false,
        _ => {}
    }
    let gtk_theme = std::env::var("GTK_THEME").ok();
    let gsettings = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "color-scheme"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
    let kdeglobals = dirs::config_dir().and_then(|c| fs::read_to_string(c.join("kdeglobals")).ok());
    parse_system_dark_sources(gtk_theme.as_deref(), gsettings.as_deref(), kdeglobals.as_deref())
}

/// Whether desktop animations are turned off.
pub fn reduce_motion() -> bool {
    // XDG Desktop Portal `reduced-motion`: 1 = reduced motion enabled.
    if read_portal_appearance_u32("reduced-motion") == Some(1) {
        return true;
    }
    if let Ok(output) =
        Command::new("gsettings").args(["get", "org.gnome.desktop.interface", "enable-animations"]).output()
        && output.status.success()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        if text.trim() == "false" {
            return true;
        }
    }
    false
}

/// Returns the usable desktop work area `(x, y, width, height)` in physical pixels.
pub fn work_area() -> Option<(i32, i32, i32, i32)> {
    let (mut x, mut y, mut w, mut h) = (0, 0, 0, 0);
    if crate::bridge::native::ffi::linux_work_area(&mut x, &mut y, &mut w, &mut h) && w > 0 && h > 0 {
        return Some((x, y, w, h));
    }
    if crate::bridge::native::ffi::linux_screen_geometry(&mut x, &mut y, &mut w, &mut h) && w > 0 && h > 0 {
        return Some((x, y, w, h));
    }
    if std::env::var_os("DISPLAY").is_some()
        && let Ok(output) = Command::new("xprop").args(["-root", "_NET_WORKAREA"]).output()
        && output.status.success()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        if let Some((_, rhs)) = text.split_once('=') {
            let nums: Vec<i32> =
                rhs.split(',').filter_map(|p| p.trim().parse::<i32>().ok()).take(4).collect();
            if let [rx, ry, rw, rh] = nums.as_slice()
                && *rw > 0
                && *rh > 0
            {
                return Some((*rx, *ry, *rw, *rh));
            }
        }
    }
    if cfg!(test) { Some((0, 0, 1920, 1080)) } else { None }
}

/// Fills `buf` with cryptographically strong random bytes from `/dev/urandom`.
pub fn secure_random(buf: &mut [u8]) -> bool {
    if buf.is_empty() {
        return true;
    }
    let Ok(mut file) = fs::File::open("/dev/urandom") else {
        return false;
    };
    file.read_exact(buf).is_ok()
}

/// Runs `f` on the current thread (no-op COM wrapper on Linux).
pub fn with_com<T>(f: impl FnOnce() -> T) -> T {
    f()
}

/// Milliseconds from UTC to local time at `at_ms` (Unix epoch ms).
/// East of UTC is positive.
pub fn local_offset_ms(at_ms: i64) -> i64 {
    let secs = at_ms.div_euclid(1_000) as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: `localtime_r` is thread-safe and writes into caller-provided `tm`.
    let res = unsafe { libc::localtime_r(&secs, &mut tm) };
    if res.is_null() {
        return 0;
    }
    (tm.tm_gmtoff as i64).saturating_mul(1_000)
}

/// Bytes a normal user can still write to the drive holding `path`, or
/// `None` if `path` doesn't exist yet.
pub fn free_space(path: &std::path::Path) -> Option<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c_path` is a valid NUL-terminated string and `stat` is valid writable memory.
    if unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) } != 0 {
        return None;
    }
    let block_size = if stat.f_frsize > 0 { stat.f_frsize as u64 } else { stat.f_bsize as u64 };
    Some((stat.f_bavail as u64).saturating_mul(block_size))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_os_info_without_panicking() {
        assert!(!os_version().is_empty());
        assert_eq!(os_build_number(), 22621);
        let _ = machine_guid();
        let _ = is_laptop();
        let _ = power_saver();
        let _ = system_dark();
        let _ = reduce_motion();
        assert!(work_area().is_some());
    }

    #[test]
    fn parses_os_release_pretty_name() {
        let sample = "NAME=\"Ubuntu\"\nPRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\nID=ubuntu\n";
        assert_eq!(parse_os_release_pretty_name(sample).as_deref(), Some("Ubuntu 24.04.1 LTS"));
    }

    #[test]
    fn parses_system_dark_across_gtk_gnome_and_kde_sources() {
        assert!(parse_system_dark_sources(Some("Adwaita-dark"), Some("'prefer-light'"), None));
        assert!(!parse_system_dark_sources(Some("Adwaita-light"), Some("'prefer-dark'"), None));
        assert!(parse_system_dark_sources(None, Some("'prefer-dark'\n"), None));
        assert!(!parse_system_dark_sources(None, Some("'prefer-light'\n"), None));
        assert!(!parse_system_dark_sources(None, Some("'default'\n"), None));
        assert!(parse_system_dark_sources(None, None, Some("[General]\nColorScheme=BreezeDark\n")));
        assert!(!parse_system_dark_sources(None, None, Some("[General]\nColorScheme=BreezeLight\n")));
        assert!(parse_system_dark_sources(None, None, None));
    }

    #[test]
    fn fills_secure_random_bytes() {
        let mut buf = [0u8; 32];
        assert!(secure_random(&mut buf));
        assert!(buf.iter().any(|&b| b != 0));
    }

    #[test]
    fn runs_closure_with_com() {
        assert_eq!(with_com(|| 42), 42);
    }

    #[test]
    fn local_offset_ms_returns_reasonable_timezone_offset() {
        let offset = local_offset_ms(1_700_000_000_000);
        assert!((-14 * 3_600_000..=14 * 3_600_000).contains(&offset));
    }

    #[test]
    fn free_space_queries_existing_directory() {
        let temp = std::env::temp_dir();
        assert!(free_space(&temp).is_some());
        assert!(free_space(std::path::Path::new("/nonexistent/nectarlink/path/xyz")).is_none());
    }
}
