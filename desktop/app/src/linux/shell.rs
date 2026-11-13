// SPDX-License-Identifier: GPL-3.0-or-later
//! Opening URLs, files, folders, and power actions (lock, sleep) on Linux via
//! XDG Desktop Portals, `org.freedesktop.FileManager1`, `xdg-open`, `loginctl`, and `systemctl`.

use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

const ALLOWED_URL_SCHEMES: &[&str] = &["http", "https", "mailto", "tel", "sms", "mms", "geo", "nectarlink"];

/// Spawns `cmd` with null stdio and reaps the child in a background thread so
/// terminated subprocesses never remain as `<defunct>` zombies.
fn spawn_and_reap(mut cmd: Command) -> std::io::Result<()> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    let _ = std::thread::Builder::new().name("child-reaper".into()).spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

fn spawn_detached(program: &str, args: &[&str]) -> Result<(), String> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    spawn_and_reap(cmd).map_err(|e| format!("{program}: {e}"))
}

/// Validates that `url` uses an allowed scheme (`http`, `https`, `mailto`, `tel`,
/// `sms`, `mms`, `geo`, `nectarlink`), does not begin with `-`, and contains no
/// control or whitespace characters.
fn validate_url(url: &str) -> Result<&str, String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err("empty URL".into());
    }
    if trimmed.starts_with('-') {
        return Err("URL must not start with '-'".into());
    }
    if trimmed.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("URL contains invalid whitespace or control characters".into());
    }
    let Some((scheme, rest)) = trimmed.split_once(':') else {
        return Err("missing URL scheme".into());
    };
    let scheme_lower = scheme.to_ascii_lowercase();
    if !ALLOWED_URL_SCHEMES.contains(&scheme_lower.as_str()) {
        return Err(format!("unsupported URL scheme: {scheme}"));
    }
    if rest.is_empty() {
        return Err("empty URL target".into());
    }
    if (scheme_lower == "http" || scheme_lower == "https") && (!rest.starts_with("//") || rest.len() <= 2) {
        return Err("http/https URL must include a host".into());
    }
    Ok(trimmed)
}

/// Opens a validated URL (`http`, `https`, `mailto`, `tel`, `sms`, `mms`, `geo`,
/// `nectarlink`) via `org.freedesktop.portal.OpenURI` over D-Bus first so the
/// URL is not exposed in process command-line arguments, falling back to
/// `gio open` / `xdg-open`.
pub fn open_url(url: &str) -> Result<(), String> {
    let trimmed = url.trim();
    let validated = validate_url(trimmed)?;

    if !cfg!(test)
        && super::non_tokio(|| {
            let Ok(conn) = zbus::blocking::Connection::session() else {
                return false;
            };
            if !super::dbus_name_has_owner(&conn, "org.freedesktop.portal.Desktop") {
                return false;
            }
            let options: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::new();
            conn.call_method(
                Some("org.freedesktop.portal.Desktop"),
                "/org/freedesktop/portal/desktop",
                Some("org.freedesktop.portal.OpenURI"),
                "OpenURI",
                &("", validated, &options),
            )
            .is_ok()
        })
    {
        return Ok(());
    }

    if Command::new("gio")
        .args(["open", "--", validated])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
    {
        return Ok(());
    }

    spawn_detached("xdg-open", &[validated])
}

/// Opens the default mail client for an email address (`mailto:<email>`).
pub fn open_mailto(email: &str) -> Result<(), String> {
    let trimmed = email.trim();
    if trimmed.is_empty()
        || trimmed.starts_with('-')
        || !trimmed.contains('@')
        || trimmed.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err("invalid email address".into());
    }
    let uri = if trimmed.get(..7).is_some_and(|p| p.eq_ignore_ascii_case("mailto:")) {
        trimmed.to_owned()
    } else {
        format!("mailto:{trimmed}")
    };
    open_url(&uri)
}

/// Opens a street address in OpenStreetMap in the default browser.
pub fn open_maps(address: &str) -> Result<(), String> {
    let trimmed = address.trim();
    if trimmed.is_empty() {
        return Err("empty address".into());
    }
    open_url(&nectarlink_core::maps_web_url(trimmed))
}

/// Checks whether a custom URI scheme has a registered handler via `xdg-mime`.
pub fn is_url_scheme_registered(scheme: &str) -> bool {
    let scheme = scheme.trim().trim_end_matches(':');
    if scheme.is_empty() || !scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return false;
    }
    let mime = format!("x-scheme-handler/{scheme}");
    Command::new("xdg-mime")
        .args(["query", "default", &mime])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .is_some_and(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
}

/// Percent-encodes a filesystem path for use in `file://` URIs and FreeDesktop
/// `.trashinfo` `Path=` entries (preserving `/` and unreserved URI bytes).
pub(crate) fn percent_encode_path(path: &Path) -> String {
    #[cfg(unix)]
    let bytes: &[u8] = std::os::unix::ffi::OsStrExt::as_bytes(path.as_os_str());
    #[cfg(not(unix))]
    let owned = path.to_string_lossy().into_owned().into_bytes();
    #[cfg(not(unix))]
    let bytes: &[u8] = &owned;

    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(b as char);
            }
            _ => {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    out
}

/// Formats `path` as a properly percent-encoded `file://` URI.
pub(crate) fn format_file_uri(path: &Path) -> String {
    let encoded = percent_encode_path(path);
    if encoded.starts_with('/') { format!("file://{encoded}") } else { format!("file:///{encoded}") }
}

/// Opens a file with the user's default application via
/// `org.freedesktop.portal.OpenURI.OpenFile` (passing a file descriptor so the
/// path is not exposed on a subprocess command line), falling back to `xdg-open`.
pub fn open_file(path: &Path) -> Result<(), String> {
    let path_str = path.to_string_lossy();
    if path_str.is_empty() || path_str.starts_with('-') {
        return Err("invalid file path".into());
    }

    if !cfg!(test)
        && let Ok(file) = fs::File::open(path)
        && super::non_tokio(|| {
            let Ok(conn) = zbus::blocking::Connection::session() else {
                return false;
            };
            if !super::dbus_name_has_owner(&conn, "org.freedesktop.portal.Desktop") {
                return false;
            }
            let options: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::new();
            conn.call_method(
                Some("org.freedesktop.portal.Desktop"),
                "/org/freedesktop/portal/desktop",
                Some("org.freedesktop.portal.OpenURI"),
                "OpenFile",
                &("", zbus::zvariant::Fd::from(&file), &options),
            )
            .is_ok()
        })
    {
        return Ok(());
    }

    spawn_detached("xdg-open", &[&path_str])
}

/// Opens a directory in the file manager via `org.freedesktop.FileManager1.ShowFolders`
/// or `org.freedesktop.portal.OpenURI.OpenDirectory` first, falling back to `xdg-open`.
pub fn open_folder(path: &Path) -> Result<(), String> {
    let path_str = path.to_string_lossy();
    if path_str.is_empty() || path_str.starts_with('-') {
        return Err("invalid folder path".into());
    }

    let abs = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let folder_uri = format_file_uri(&abs);

    if !cfg!(test)
        && super::non_tokio(|| {
            let Ok(conn) = zbus::blocking::Connection::session() else {
                return false;
            };
            if super::dbus_name_has_owner(&conn, "org.freedesktop.FileManager1")
                && conn
                    .call_method(
                        Some("org.freedesktop.FileManager1"),
                        "/org/freedesktop/FileManager1",
                        Some("org.freedesktop.FileManager1"),
                        "ShowFolders",
                        &(vec![folder_uri.as_str()], ""),
                    )
                    .is_ok()
            {
                return true;
            }

            if super::dbus_name_has_owner(&conn, "org.freedesktop.portal.Desktop")
                && let Ok(dir_file) = fs::File::open(&abs)
            {
                let options: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::new();
                if conn
                    .call_method(
                        Some("org.freedesktop.portal.Desktop"),
                        "/org/freedesktop/portal/desktop",
                        Some("org.freedesktop.portal.OpenURI"),
                        "OpenDirectory",
                        &("", zbus::zvariant::Fd::from(&dir_file), &options),
                    )
                    .is_ok()
                {
                    return true;
                }
            }
            false
        })
    {
        return Ok(());
    }

    spawn_detached("xdg-open", &[&path_str])
}

/// Reveals and selects `path` in the file manager via `org.freedesktop.FileManager1.ShowItems`,
/// falling back to opening the parent directory with [`open_folder`].
pub fn select_in_explorer(path: &Path) -> Result<(), String> {
    let abs = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let file_uri = format_file_uri(&abs);
    if !cfg!(test)
        && super::non_tokio(|| {
            let Ok(conn) = zbus::blocking::Connection::session() else {
                return false;
            };
            super::dbus_name_has_owner(&conn, "org.freedesktop.FileManager1")
                && conn
                    .call_method(
                        Some("org.freedesktop.FileManager1"),
                        "/org/freedesktop/FileManager1",
                        Some("org.freedesktop.FileManager1"),
                        "ShowItems",
                        &(vec![file_uri.as_str()], ""),
                    )
                    .is_ok()
        })
    {
        return Ok(());
    }
    let folder = abs.parent().unwrap_or(&abs);
    open_folder(folder)
}

/// Runs `program` with `arguments` (split on spaces, `"` groups).
pub fn run(program: &Path, arguments: &str) -> Result<(), String> {
    let args = split_args(arguments);
    let mut cmd = Command::new(program);
    cmd.args(&args);
    spawn_and_reap(cmd).map_err(|e| e.to_string())
}

fn split_args(arguments: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for ch in arguments.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            c if c.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Locks the current user session (`loginctl lock-session` or `xdg-screensaver lock`).
pub fn lock() -> Result<(), String> {
    if Command::new("loginctl")
        .arg("lock-session")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
    {
        return Ok(());
    }
    spawn_detached("xdg-screensaver", &["lock"])
}

/// Alias for [`lock`].
pub fn lock_workstation() -> Result<(), String> {
    lock()
}

/// Suspends the system (`systemctl suspend`).
pub fn sleep() -> Result<(), String> {
    spawn_detached("systemctl", &["suspend"])
}

/// Alias for [`sleep`].
pub fn sleep_system() -> Result<(), String> {
    sleep()
}

/// Hibernates the system (`systemctl hibernate`, #355).
pub fn hibernate() -> Result<(), String> {
    spawn_detached("systemctl", &["hibernate"])
}

/// Whether logind says the system can hibernate (swap big enough, allowed).
pub fn can_hibernate() -> bool {
    super::non_tokio(|| {
        let ask = || -> zbus::Result<String> {
            let conn = zbus::blocking::Connection::system()?;
            let manager = zbus::blocking::Proxy::new(
                &conn,
                "org.freedesktop.login1",
                "/org/freedesktop/login1",
                "org.freedesktop.login1.Manager",
            )?;
            manager.call("CanHibernate", &())
        };
        ask().is_ok_and(|answer| answer == "yes")
    })
}

fn decode_file_uri(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let path_part =
        rest.strip_prefix("localhost/").map(|s| format!("/{s}")).unwrap_or_else(|| rest.to_owned());
    let bytes = path_part.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16)?;
            let lo = (bytes[i + 2] as char).to_digit(16)?;
            out.push(((hi << 4) | lo) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
    }
    #[cfg(not(unix))]
    {
        String::from_utf8(out).ok().map(PathBuf::from)
    }
}

fn screenshot_via_portal(save_path: &Path) -> bool {
    let Ok(conn) = zbus::blocking::Connection::session() else {
        return false;
    };
    let Ok(rule) = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.freedesktop.portal.Request")
        .and_then(|b| b.member("Response"))
        .map(|b| b.build())
    else {
        return false;
    };
    let Ok(iter) = zbus::blocking::MessageIterator::for_match_rule(rule, &conn, None) else {
        return false;
    };

    let mut options: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::new();
    options.insert("interactive", zbus::zvariant::Value::Bool(false));

    let Ok(reply) = conn.call_method(
        Some("org.freedesktop.portal.Desktop"),
        "/org/freedesktop/portal/desktop",
        Some("org.freedesktop.portal.Screenshot"),
        "Screenshot",
        &("", &options),
    ) else {
        return false;
    };

    let expected_handle =
        reply.body().deserialize::<zbus::zvariant::OwnedObjectPath>().ok().map(|p| p.to_string());

    for msg in iter.take(16) {
        let Ok(msg) = msg else {
            break;
        };
        if let Some(ref handle) = expected_handle
            && msg.header().path().map(|p| p.as_str()) != Some(handle.as_str())
        {
            continue;
        }
        if let Ok((code, results)) =
            msg.body().deserialize::<(u32, HashMap<String, zbus::zvariant::OwnedValue>)>()
        {
            if code != 0 {
                return false;
            }
            if let Some(uri_val) = results.get("uri")
                && let Ok(uri_str) = <&str>::try_from(uri_val)
                && let Some(src_path) = decode_file_uri(uri_str)
                && src_path.is_file()
            {
                if src_path == save_path {
                    return true;
                }
                if fs::copy(&src_path, save_path).is_ok() {
                    let _ = fs::remove_file(&src_path);
                    return true;
                }
            }
            return false;
        }
    }
    false
}

/// Captures a screenshot to `save_path` using `org.freedesktop.portal.Screenshot`,
/// `grim`, `gnome-screenshot`, `spectacle`, or ImageMagick `import`, and verifies
/// that `save_path` exists and is non-empty.
pub fn take_screenshot_to(save_path: &Path) -> Result<(), String> {
    if let Some(parent) = save_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let check_saved = || save_path.exists() && save_path.metadata().is_ok_and(|m| m.len() > 0);

    if screenshot_via_portal(save_path) && check_saved() {
        return Ok(());
    }

    let dest = save_path.to_string_lossy();
    let cli_candidates: &[(&str, &[&str])] = &[
        ("grim", &[&dest]),
        ("gnome-screenshot", &["-f", &dest]),
        ("spectacle", &["-b", "-n", "-o", &dest]),
        ("import", &["-window", "root", &dest]),
    ];

    for (prog, args) in cli_candidates {
        if Command::new(prog)
            .args(*args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
            && check_saved()
        {
            return Ok(());
        }
    }

    Err("failed to capture screenshot".into())
}

/// Captures a screenshot to the user's Pictures directory (or temporary directory),
/// falling back to interactive screenshot utilities or the `Print` key.
pub fn take_screenshot() -> Result<(), String> {
    let dir = dirs::picture_dir().or_else(dirs::home_dir).unwrap_or_else(std::env::temp_dir);
    let stamp = format_deletion_date(SystemTime::now()).replace(':', "-");
    let save_path = dir.join(format!("Screenshot_{stamp}.png"));
    if take_screenshot_to(&save_path).is_ok() {
        return Ok(());
    }

    let candidates: &[(&str, &[&str])] = &[
        ("spectacle", &[]),
        ("gnome-screenshot", &["-i"]),
        ("flameshot", &["gui"]),
        ("xfce4-screenshooter", &["-r"]),
    ];
    for (prog, args) in candidates {
        if spawn_detached(prog, args).is_ok() {
            return Ok(());
        }
    }
    super::input::press_key("print", &[]);
    Ok(())
}

/// Strips FreeDesktop `.desktop` `Exec=` field codes (`%f`, `%F`, `%u`, `%U`,
/// `%i`, `%c`, `%k`, etc.) even when embedded in an argument (e.g. `--url=%u`),
/// and unescapes `%%` to `%`.
fn strip_desktop_field_codes(token: &str) -> String {
    let mut out = String::with_capacity(token.len());
    let mut chars = token.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '%' {
            match chars.peek().copied() {
                Some('%') => {
                    chars.next();
                    out.push('%');
                }
                Some('f' | 'F' | 'u' | 'U' | 'i' | 'c' | 'k' | 'd' | 'D' | 'n' | 'N' | 'v' | 'm') => {
                    chars.next();
                }
                Some(_) | None => {
                    out.push('%');
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// Parses the `[Desktop Entry]` `Exec=` command from a `.desktop` file and strips
/// freedesktop field codes (`%f`, `%F`, `%u`, `%U`, `%i`, `%c`, `%k`, `%%`).
pub(crate) fn parse_desktop_exec(contents: &str) -> Option<(String, Vec<String>)> {
    let mut in_desktop_entry = false;
    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_desktop_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_desktop_entry {
            continue;
        }
        if let Some(rest) = line.strip_prefix("Exec=") {
            let tokens = split_args(rest.trim());
            let mut filtered: Vec<String> = Vec::new();
            for tok in tokens {
                let cleaned = strip_desktop_field_codes(&tok);
                if !cleaned.is_empty() {
                    filtered.push(cleaned);
                }
            }
            let mut iter = filtered.into_iter();
            let program = iter.next()?;
            let args: Vec<String> = iter.collect();
            return Some((program, args));
        }
    }
    None
}

/// Searches standard XDG application directories (`~/.local/share/applications`,
/// `/usr/local/share/applications`, `/usr/share/applications`, `/var/lib/flatpak/exports/share/applications`)
/// for a `.desktop` file by filename.
fn find_desktop_file(desktop_name: &str) -> Option<PathBuf> {
    let mut dirs_to_check = Vec::new();
    if let Some(data_local) = dirs::data_local_dir() {
        dirs_to_check.push(data_local.join("applications"));
    }
    dirs_to_check.push(PathBuf::from("/usr/local/share/applications"));
    dirs_to_check.push(PathBuf::from("/usr/share/applications"));
    dirs_to_check.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));
    for dir in dirs_to_check {
        let candidate = dir.join(desktop_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Launches an application, `.desktop` launcher, `.AppImage`, or executable binary.
pub fn launch_app(path: &Path) -> Result<(), String> {
    let raw = path.to_string_lossy();
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("empty application path".into());
    }

    let lower = trimmed.to_ascii_lowercase();
    if lower.ends_with(".desktop") {
        // Resolve the .desktop file path if it exists directly or in XDG applications directories.
        let resolved = if path.is_file() { Some(path.to_path_buf()) } else { find_desktop_file(trimmed) };

        if let Some(ref desktop_path) = resolved {
            // 1. Try `gio launch <path.desktop>` and check its exit status.
            if Command::new("gio")
                .args(["launch", &desktop_path.to_string_lossy()])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
            {
                return Ok(());
            }

            // 2. Parse `[Desktop Entry]` `Exec=` directly and spawn the target binary.
            if let Ok(contents) = fs::read_to_string(desktop_path)
                && let Some((prog, args)) = parse_desktop_exec(&contents)
            {
                let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
                if spawn_detached(&prog, &arg_refs).is_ok() {
                    return Ok(());
                }
            }
        }

        // 3. Try `gtk-launch <desktop-id>` using the file stem / filename.
        if let Some(file_name) = path.file_name().and_then(|n| n.to_str())
            && spawn_detached("gtk-launch", &[file_name]).is_ok()
        {
            return Ok(());
        }
        return Err(format!("could not launch desktop entry {trimmed}"));
    }

    if path.is_file() && spawn_and_reap(Command::new(path)).is_ok() {
        return Ok(());
    }

    open_file(path)
}

/// Alias for [`launch_app`].
pub fn launch_executable(path: &Path, args: &[&str]) -> Result<(), String> {
    let mut cmd = Command::new(path);
    cmd.args(args);
    spawn_and_reap(cmd).map_err(|e| e.to_string())
}

/// Runs a shell command (`sh -c <command>`) detached in the background.
pub fn run_command(command: &str) -> Result<(), String> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Err("empty command".into());
    }
    spawn_detached("sh", &["-c", trimmed])
}

/// Formats `now` as local `YYYY-MM-DDTHH:MM:SS` via `libc::localtime_r` as
/// required by the FreeDesktop.org Trash 1.0 specification.
pub(crate) fn format_deletion_date(now: SystemTime) -> String {
    let secs = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let time_t = secs as libc::time_t;
    // SAFETY: `libc::tm` is a plain C struct of integers and a pointer; zero-initialization
    // is valid before passing `&mut tm` to `localtime_r`.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: `time_t` and `tm` are valid stack pointers; `localtime_r` is thread-safe.
    let res = unsafe { libc::localtime_r(&time_t, &mut tm) };
    if res.is_null() {
        return "1970-01-01T00:00:00".to_owned();
    }
    let year = tm.tm_year + 1900;
    let month = (tm.tm_mon + 1).clamp(1, 12);
    let day = tm.tm_mday.clamp(1, 31);
    let hour = tm.tm_hour.clamp(0, 23);
    let min = tm.tm_min.clamp(0, 59);
    let sec = tm.tm_sec.clamp(0, 60);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}")
}

/// Formats the contents of a FreeDesktop.org `.trashinfo` file with a
/// percent-encoded `Path=` and `DeletionDate=`.
pub(crate) fn format_trashinfo(original_path: &Path, deletion_date: &str) -> String {
    format!("[Trash Info]\nPath={}\nDeletionDate={}\n", percent_encode_path(original_path), deletion_date)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if file_type.is_symlink() {
            let link_target = fs::read_link(&src_path)?;
            #[cfg(unix)]
            std::os::unix::fs::symlink(&link_target, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

fn move_across_filesystems(src: &Path, dst: &Path) -> std::io::Result<()> {
    let meta = fs::symlink_metadata(src)?;
    if meta.is_dir() {
        if let Err(e) = copy_dir_recursive(src, dst) {
            let _ = fs::remove_dir_all(dst);
            return Err(e);
        }
        fs::remove_dir_all(src)
    } else if meta.file_type().is_symlink() {
        let link_target = fs::read_link(src)?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(&link_target, dst)?;
        if let Err(e) = fs::remove_file(src) {
            let _ = fs::remove_file(dst);
            return Err(e);
        }
        Ok(())
    } else {
        if let Err(e) = fs::copy(src, dst) {
            let _ = fs::remove_file(dst);
            return Err(e);
        }
        if let Err(e) = fs::remove_file(src) {
            let _ = fs::remove_file(dst);
            return Err(e);
        }
        Ok(())
    }
}

/// Moves `path` to the FreeDesktop Trash (`~/.local/share/Trash/files/` + `.trashinfo`).
pub fn recycle_file(path: &Path) -> Result<(), String> {
    let data_dir = dirs::data_local_dir().ok_or_else(|| "no XDG_DATA_HOME directory".to_owned())?;
    recycle_file_in(&data_dir.join("Trash"), path)
}

fn recycle_file_in(trash_dir: &Path, path: &Path) -> Result<(), String> {
    if path.symlink_metadata().is_err() {
        return Err(format!("{} does not exist", path.display()));
    }
    let abs = match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => {
            let canon_parent = fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
            canon_parent.join(name)
        }
        _ => fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()),
    };

    let files_dir = trash_dir.join("files");
    let info_dir = trash_dir.join("info");
    fs::create_dir_all(&files_dir).map_err(|e| e.to_string())?;
    fs::create_dir_all(&info_dir).map_err(|e| e.to_string())?;

    let base_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("trashed_item");
    let mut chosen_name = base_name.to_owned();
    let mut counter = 2u32;

    let (dest_file, dest_info, mut info_file) = loop {
        let candidate_file = files_dir.join(&chosen_name);
        let candidate_info = info_dir.join(format!("{chosen_name}.trashinfo"));
        if candidate_file.symlink_metadata().is_ok() {
            chosen_name = format!("{base_name}.{counter}");
            counter += 1;
            continue;
        }

        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }

        match opts.open(&candidate_info) {
            Ok(f) => break (candidate_file, candidate_info, f),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                chosen_name = format!("{base_name}.{counter}");
                counter += 1;
            }
            Err(e) => return Err(e.to_string()),
        }
    };

    let deletion_date = format_deletion_date(SystemTime::now());
    let info_contents = format_trashinfo(&abs, &deletion_date);
    if let Err(e) = info_file.write_all(info_contents.as_bytes()).and_then(|()| info_file.flush()) {
        drop(info_file);
        let _ = fs::remove_file(&dest_info);
        return Err(e.to_string());
    }
    drop(info_file);

    if let Err(e) = fs::rename(path, &dest_file) {
        if e.raw_os_error() == Some(libc::EXDEV) {
            if let Err(copy_err) = move_across_filesystems(path, &dest_file) {
                let _ = fs::remove_file(&dest_info);
                return Err(copy_err.to_string());
            }
        } else {
            let _ = fs::remove_file(&dest_info);
            return Err(e.to_string());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_non_web_urls() {
        assert!(open_url("file:///etc/passwd").is_err());
        assert!(open_url("javascript:alert(1)").is_err());
        assert!(open_url("-http://example.com").is_err());
        assert!(open_url("http://").is_err());
    }

    #[test]
    fn validates_mailto_and_maps() {
        assert!(open_mailto("").is_err());
        assert!(open_mailto("not-an-email").is_err());
        assert!(open_mailto("-user@example.com").is_err());
        assert!(open_maps("   ").is_err());
    }

    #[test]
    fn formats_file_uri_with_percent_encoding() {
        assert_eq!(
            format_file_uri(Path::new("/home/user/My Documents/report #1 (final).pdf")),
            "file:///home/user/My%20Documents/report%20%231%20%28final%29.pdf"
        );
        assert_eq!(format_file_uri(Path::new("/tmp/100%_done?.txt")), "file:///tmp/100%25_done%3F.txt");
    }

    #[test]
    fn moves_file_to_freedesktop_trash() {
        let dir = tempfile::tempdir().unwrap();
        let trash = dir.path().join("Trash");
        let file = dir.path().join("my note #1.txt");
        fs::write(&file, b"hello").unwrap();

        recycle_file_in(&trash, &file).unwrap();
        assert!(!file.exists());
        assert!(trash.join("files").join("my note #1.txt").is_file());
        let info_path = trash.join("info").join("my note #1.txt.trashinfo");
        assert!(info_path.is_file());

        let info_text = fs::read_to_string(&info_path).unwrap();
        assert!(info_text.starts_with("[Trash Info]\nPath="), "{info_text}");
        assert!(info_text.contains("my%20note%20%231.txt\n"), "{info_text}");
        assert!(!info_text.contains("DeletionDate=1970-01-01"), "{info_text}");
        assert!(info_text.contains("DeletionDate=20"), "{info_text}");

        let formatted =
            format_trashinfo(Path::new("/home/alice/My Files/test 100%.txt"), "2026-10-22T23:05:12");
        assert_eq!(
            formatted,
            "[Trash Info]\nPath=/home/alice/My%20Files/test%20100%25.txt\nDeletionDate=2026-10-22T23:05:12\n"
        );
    }

    #[test]
    fn parses_desktop_entry_exec_and_strips_field_codes() {
        let desktop = "\
[Desktop Entry]
Type=Application
Name=Firefox Web Browser
Exec=/usr/lib/firefox/firefox --new-window --url=%u --scale=100%% %F
Icon=firefox

[Desktop Action new-private-window]
Name=Open a New Private Window
Exec=/usr/lib/firefox/firefox --private-window %u
";
        let (prog, args) = parse_desktop_exec(desktop).expect("should parse Exec=");
        assert_eq!(prog, "/usr/lib/firefox/firefox");
        assert_eq!(args, vec!["--new-window", "--url=", "--scale=100%"]);
    }

    #[test]
    fn launch_app_executes_desktop_file() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("launched.txt");
        let desktop_path = dir.path().join("test-app.desktop");
        let desktop_contents = format!(
            "[Desktop Entry]\nType=Application\nName=TestApp\nExec=/bin/sh -c \"echo ok > '{}'\" %U\n",
            marker.display()
        );
        fs::write(&desktop_path, desktop_contents).unwrap();
        assert!(launch_app(&desktop_path).is_ok());
    }
}
