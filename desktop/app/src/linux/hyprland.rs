// SPDX-License-Identifier: GPL-3.0-or-later
//! Hyprland monitor layout and local desktop actions over its command socket.

#![cfg(target_os = "linux")]

use std::{
    env,
    ffi::OsStr,
    fs,
    io::{self, Read, Write},
    os::unix::{fs::FileTypeExt, net::UnixStream},
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

use serde::Deserialize;

const IPC_TIMEOUT: Duration = Duration::from_millis(500);
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_LAUNCHER_BYTES: usize = 8_192;
const SUPER_MASK: u32 = 64;
const SPACE_KEYCODE: u32 = 65; // XKB keycode (evdev KEY_SPACE + 8).

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Monitor {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub primary: bool,
}

#[derive(Debug, Deserialize)]
struct MonitorEntry {
    name: String,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale: f64,
    #[serde(default)]
    transform: u8,
    #[serde(default)]
    disabled: bool,
    #[serde(default)]
    focused: bool,
    #[serde(default)]
    primary: bool,
}

/// Returns enabled monitors in logical pixels. Hyprland does not ordinarily
/// designate a primary monitor, so use its focused monitor, then the first.
pub fn parse_monitors_json(json: &str) -> io::Result<Vec<Monitor>> {
    let entries: Vec<MonitorEntry> = serde_json::from_str(json)?;
    let mut monitors = Vec::new();
    let mut primary = None;
    let mut focused = None;
    for entry in entries.into_iter().filter(|entry| !entry.disabled) {
        let (width, height) = if matches!(entry.transform, 1 | 3 | 5 | 7) {
            (entry.height, entry.width)
        } else {
            (entry.width, entry.height)
        };
        let width = (f64::from(width) / entry.scale).round();
        let height = (f64::from(height) / entry.scale).round();
        if entry.scale <= 0.0 || !valid_dimension(width) || !valid_dimension(height) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid Hyprland monitor size"));
        }
        if entry.primary {
            primary.get_or_insert(monitors.len());
        }
        if entry.focused {
            focused.get_or_insert(monitors.len());
        }
        monitors.push(Monitor {
            name: entry.name,
            x: entry.x,
            y: entry.y,
            width: width as i32,
            height: height as i32,
            primary: false,
        });
    }
    if let Some(monitor) = monitors.get_mut(primary.or(focused).unwrap_or(0)) {
        monitor.primary = true;
    }
    Ok(monitors)
}

fn valid_dimension(value: f64) -> bool {
    (1.0..=f64::from(i32::MAX)).contains(&value)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherCommand(String);

impl LauncherCommand {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Deserialize)]
struct BindEntry {
    #[serde(default)]
    modmask: u32,
    #[serde(default)]
    key: String,
    #[serde(default)]
    keycode: u32,
    #[serde(default)]
    dispatcher: String,
    #[serde(default)]
    arg: String,
    #[serde(default)]
    submap: String,
}

/// Selects a plain Super+Space exec bind from the local compositor configuration.
pub fn parse_super_space_bind(json: &str) -> io::Result<Option<LauncherCommand>> {
    let entries: Vec<BindEntry> = serde_json::from_str(json)?;
    Ok(entries.into_iter().find_map(|bind| {
        let space = bind.key.eq_ignore_ascii_case("space")
            || bind.key.eq_ignore_ascii_case("code:65")
            || bind.keycode == SPACE_KEYCODE;
        let arg = bind.arg.trim();
        (bind.modmask == SUPER_MASK
            && space
            && bind.dispatcher == "exec"
            && (bind.submap.is_empty() || bind.submap == "reset")
            && !arg.is_empty()
            && arg.len() <= MAX_LAUNCHER_BYTES
            && !bind.arg.chars().any(char::is_control))
        .then(|| LauncherCommand(arg.to_owned()))
    }))
}

#[derive(Debug, Clone)]
pub struct Hyprland {
    socket: PathBuf,
}

impl Hyprland {
    pub fn discover() -> io::Result<Self> {
        let runtime = env::var_os("XDG_RUNTIME_DIR")
            .filter(|runtime| !runtime.is_empty())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR is unset"))?;
        let base = PathBuf::from(runtime).join("hypr");
        let signature = env::var_os("HYPRLAND_INSTANCE_SIGNATURE").filter(|signature| !signature.is_empty());
        Self::discover_in(&base, signature.as_deref())
    }

    fn discover_in(base: &Path, signature: Option<&OsStr>) -> io::Result<Self> {
        if let Some(signature) = signature {
            let Some(Component::Normal(name)) = Path::new(signature).components().next() else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "invalid Hyprland instance signature",
                ));
            };
            if name != signature {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "invalid Hyprland instance signature",
                ));
            }
            let socket = base.join(name).join(".socket.sock");
            return is_socket_path(&socket).then_some(Self { socket }).ok_or_else(socket_not_found);
        }

        let deadline = Instant::now() + IPC_TIMEOUT;
        // A user service may lack the session environment. Prefer the newest
        // instance that still answers; stale sockets can survive a crash.
        let mut candidates: Vec<_> = fs::read_dir(base)?
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let socket = entry.path().join(".socket.sock");
                let metadata = fs::symlink_metadata(&socket).ok()?;
                metadata.file_type().is_socket().then(|| (metadata.modified().ok(), socket))
            })
            .collect();
        candidates.sort_unstable_by(|left, right| right.cmp(left));
        for (_, socket) in candidates {
            remaining(deadline)?;
            let client = Self { socket };
            if client
                .request_until("j/monitors", deadline)
                .and_then(|json| parse_monitors_json(&json))
                .is_ok()
            {
                return Ok(client);
            }
        }
        remaining(deadline)?;
        Err(socket_not_found())
    }

    pub fn query_monitors(&self) -> io::Result<Vec<Monitor>> {
        parse_monitors_json(&self.request("j/monitors")?)
    }

    pub fn find_super_space_launcher(&self) -> io::Result<Option<LauncherCommand>> {
        parse_super_space_bind(&self.request("j/binds")?)
    }

    pub fn move_cursor(&self, x: i32, y: i32) -> io::Result<()> {
        self.command(&format!("dispatch movecursor {x} {y}"))
    }

    pub fn cycle_window(&self, next: bool) -> io::Result<()> {
        self.command(if next { "dispatch cyclenext" } else { "dispatch cyclenext prev" })?;
        self.command("dispatch bringactivetotop")
    }

    pub fn toggle_maximize(&self) -> io::Result<()> {
        self.command("dispatch fullscreen 1")
    }

    pub fn workspace_empty(&self) -> io::Result<()> {
        self.command("dispatch workspace empty")
    }

    pub fn workspace_previous(&self) -> io::Result<()> {
        self.command("dispatch workspace previous")
    }

    pub fn workspace_step(&self, next: bool) -> io::Result<()> {
        self.command(if next { "dispatch workspace e+1" } else { "dispatch workspace e-1" })
    }

    /// Run the command configured in the local compositor's launcher bind.
    pub fn exec_launcher(&self, command: &LauncherCommand) -> io::Result<()> {
        self.command(&format!("dispatch exec {}", command.as_str()))
    }

    pub fn set_virtual_mouse_flat_accel(&self, device_name: &str) -> io::Result<()> {
        if device_name.is_empty()
            || !device_name.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid Hyprland input device name"));
        }
        self.command(&format!("keyword device[{device_name}]:accel_profile flat"))
    }

    fn command(&self, message: &str) -> io::Result<()> {
        if self.request(message)?.trim() == "ok" {
            Ok(())
        } else {
            Err(io::Error::other("Hyprland rejected the command"))
        }
    }

    fn request(&self, message: &str) -> io::Result<String> {
        self.request_until(message, Instant::now() + IPC_TIMEOUT)
    }

    fn request_until(&self, message: &str, deadline: Instant) -> io::Result<String> {
        // Tokio connects using a nonblocking socket. std's blocking connect
        // has no deadline and can stall if the compositor's accept queue fills.
        let mut stream: UnixStream = super::non_tokio(|| {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_io().enable_time().build()?;
            let stream = runtime.block_on(async {
                tokio::time::timeout(remaining(deadline)?, tokio::net::UnixStream::connect(&self.socket))
                    .await
                    .map_err(|_| ipc_timeout())?
            })?;
            stream.into_std()
        })?;
        stream.set_nonblocking(false)?;
        let mut bytes = message.as_bytes();
        while !bytes.is_empty() {
            stream.set_write_timeout(Some(remaining(deadline)?))?;
            match stream.write(bytes) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(written) => bytes = &bytes[written..],
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(map_socket_error(err)),
            }
        }
        let mut response = Vec::new();
        let mut chunk = [0; 8192];
        loop {
            stream.set_read_timeout(Some(remaining(deadline)?))?;
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => {
                    if response.len() + read > MAX_RESPONSE_BYTES {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "Hyprland response is too large",
                        ));
                    }
                    response.extend_from_slice(&chunk[..read]);
                }
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(map_socket_error(err)),
            }
        }
        String::from_utf8(response).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

fn is_socket_path(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_socket())
}

fn socket_not_found() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "no active Hyprland command socket")
}

fn ipc_timeout() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "Hyprland IPC timed out")
}

fn remaining(deadline: Instant) -> io::Result<Duration> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() { Err(ipc_timeout()) } else { Ok(remaining) }
}

fn map_socket_error(err: io::Error) -> io::Error {
    if err.kind() == io::ErrorKind::WouldBlock { ipc_timeout() } else { err }
}

pub fn restore_notification() -> io::Result<()> {
    for (program, arg) in [("makoctl", "restore"), ("swaync-client", "-t")] {
        if super::run_cmd_timeout(program, &[arg], IPC_TIMEOUT).is_some_and(|output| output.status.success())
        {
            return Ok(());
        }
    }
    Err(io::Error::other("no notification service restored a notification"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{os::unix::net::UnixListener, thread};

    #[test]
    fn parses_scaled_and_rotated_monitors_with_negative_origins() {
        let input = r#"[
            {"name":"DP-1","x":-1536,"y":-100,"width":3840,"height":2160,"scale":2.5},
            {"name":"eDP-1","x":0,"y":0,"width":2560,"height":1600,"scale":1.25,"transform":1,"focused":true},
            {"name":"HDMI-A-1","x":1280,"y":0,"width":1920,"height":1080,"scale":1.0,"disabled":true}
        ]"#;
        let monitors = parse_monitors_json(input).unwrap();
        assert_eq!(
            monitors,
            [
                Monitor { name: "DP-1".into(), x: -1536, y: -100, width: 1536, height: 864, primary: false },
                Monitor { name: "eDP-1".into(), x: 0, y: 0, width: 1280, height: 2048, primary: true },
            ]
        );
        for transform in 0..8 {
            let input = format!(
                r#"[{{"name":"DP-1","x":-50,"y":0,"width":1920,"height":1080,"scale":1.5,
                "transform":{transform}}}]"#
            );
            let monitor = parse_monitors_json(&input).unwrap().remove(0);
            let dimensions = if transform % 2 == 0 { (1280, 720) } else { (720, 1280) };
            assert_eq!((monitor.width, monitor.height), dimensions);
            assert!(monitor.primary);
        }
        assert!(parse_monitors_json(&input.replace("2.5", "0.0")).is_err());
        assert!(parse_monitors_json("[]").unwrap().is_empty());
        assert!(parse_monitors_json("oops").is_err());
    }

    #[test]
    fn parses_only_local_unmodified_super_space_exec_binds() {
        let input = r#"[
            {"modmask":65,"key":"SPACE","dispatcher":"exec","arg":"shifted"},
            {"modmask":64,"key":"SPACE","dispatcher":"global","arg":"not-an-exec"},
            {"modmask":64,"key":"SPACE","dispatcher":"exec","submap":"resize","arg":"wrong-submap"},
            {"modmask":64,"key":"SPACE","dispatcher":"exec","arg":"line\nfeed"},
            {"modmask":64,"key":"SPACE","dispatcher":"exec","arg":"launcher --mode app"}
        ]"#;
        assert_eq!(parse_super_space_bind(input).unwrap().unwrap().as_str(), "launcher --mode app");
        let code = r#"[{"modmask":64,"key":"","keycode":65,"dispatcher":"exec","arg":"rofi -show drun"}]"#;
        assert_eq!(parse_super_space_bind(code).unwrap().unwrap().as_str(), "rofi -show drun");
        let no_match = r#"[{"modmask":64,"key":"x","keycode":53,"dispatcher":"exec","arg":"rofi"}]"#;
        assert_eq!(parse_super_space_bind(no_match).unwrap(), None);
        assert!(parse_super_space_bind("null").is_err());
    }

    #[test]
    fn finds_an_active_socket_and_dispatches_the_exact_commands() {
        let dir = tempfile::tempdir().unwrap();
        let active = dir.path().join("active");
        fs::create_dir(&active).unwrap();
        let socket = active.join(".socket.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let stale = dir.path().join("stale");
        fs::create_dir(&stale).unwrap();
        drop(UnixListener::bind(stale.join(".socket.sock")).unwrap());

        let server = thread::spawn(move || {
            for (message, reply) in [
                ("j/monitors", "[]"), // discovery probe
                ("j/monitors", "[]"),
                ("j/binds", r#"[{"modmask":64,"key":"space","dispatcher":"exec","arg":"launcher -a"}]"#),
                ("dispatch exec launcher -a", "ok"),
                ("dispatch movecursor -20 200", "ok"),
                ("dispatch cyclenext", "ok"),
                ("dispatch bringactivetotop", "ok"),
                ("dispatch cyclenext prev", "ok"),
                ("dispatch bringactivetotop", "ok"),
                ("dispatch fullscreen 1", "ok"),
                ("dispatch workspace empty", "ok"),
                ("dispatch workspace previous", "ok"),
                ("dispatch workspace e+1", "ok"),
                ("dispatch workspace e-1", "ok"),
                ("keyword device[virtual-mouse]:accel_profile flat", "ok"),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(IPC_TIMEOUT)).unwrap();
                let mut got = vec![0; message.len()];
                stream.read_exact(&mut got).unwrap();
                assert_eq!(got, message.as_bytes());
                stream.write_all(reply.as_bytes()).unwrap();
            }
        });

        let client = Hyprland::discover_in(dir.path(), None).unwrap();
        assert_eq!(client.socket, socket);
        assert_eq!(Hyprland::discover_in(dir.path(), Some(OsStr::new("active"))).unwrap().socket, socket);
        assert!(Hyprland::discover_in(dir.path(), Some(OsStr::new("../active"))).is_err());
        assert!(client.query_monitors().unwrap().is_empty());
        client.exec_launcher(&client.find_super_space_launcher().unwrap().unwrap()).unwrap();
        client.move_cursor(-20, 200).unwrap();
        client.cycle_window(true).unwrap();
        client.cycle_window(false).unwrap();
        client.toggle_maximize().unwrap();
        client.workspace_empty().unwrap();
        client.workspace_previous().unwrap();
        client.workspace_step(true).unwrap();
        client.workspace_step(false).unwrap();
        assert!(client.set_virtual_mouse_flat_accel("virtual]:other").is_err());
        client.set_virtual_mouse_flat_accel("virtual-mouse").unwrap();
        server.join().unwrap();
    }

    #[test]
    fn bounds_a_compositor_that_never_finishes_its_response() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join(".socket.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let (release, hold) = std::sync::mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.write_all(b"[").unwrap();
            let _ = hold.recv_timeout(Duration::from_secs(1));
        });
        let client = Hyprland { socket };
        let start = Instant::now();
        let result = client.request_until("j/monitors", start + Duration::from_millis(50));
        let _ = release.send(());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() < IPC_TIMEOUT);
        server.join().unwrap();
    }
}
