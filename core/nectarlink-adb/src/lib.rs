// SPDX-License-Identifier: MPL-2.0
//! Pure-Rust client for the local ADB server (`127.0.0.1:5037`) used to detect
//! USB-attached Android devices and set up TCP port forwarding for NectarLink's
//! USB QUIC tunnel.

use std::{
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    time::Duration,
};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tokio_util::sync::CancellationToken;

/// Default local ADB server address (`127.0.0.1:5037`).
pub const DEFAULT_ADB_ADDR: SocketAddr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 5037));

/// Per-operation timeout for ADB socket connects, commands, and replies.
#[cfg(not(test))]
const ADB_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
const ADB_TIMEOUT: Duration = Duration::from_millis(200);

async fn with_adb_timeout<F, T>(fut: F) -> Result<T, AdbError>
where
    F: std::future::Future<Output = Result<T, AdbError>>,
{
    tokio::time::timeout(ADB_TIMEOUT, fut).await.map_err(|_| {
        AdbError::Io(std::io::Error::new(std::io::ErrorKind::TimedOut, "adb request timed out"))
    })?
}

async fn connect_adb(server: SocketAddr) -> Result<TcpStream, AdbError> {
    with_adb_timeout(async { Ok(TcpStream::connect(server).await?) }).await
}

/// Returns `true` if `serial` is non-empty, made only of the characters
/// serials use, and permitted by the optional `NECTARLINK_ADB_ONLY_SERIAL`
/// filter. A USB device names its own serial, and it goes into adb commands
/// as text, so anything else (`;`, spaces, control characters) is refused.
pub fn is_serial_allowed(serial: &str) -> bool {
    if serial != serial.trim() {
        return false;
    }
    if serial.is_empty()
        || serial.len() > 128
        || !serial
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b':' | b'[' | b']'))
    {
        return false;
    }
    if serial.contains(':') || serial.contains('[') || serial.contains(']') {
        let valid_host_port = if let Some(rest) = serial.strip_prefix('[') {
            rest.split_once("]:").is_some_and(|(host, port)| {
                !host.is_empty()
                    && !host.contains(['[', ']'])
                    && !port.is_empty()
                    && port.bytes().all(|b| b.is_ascii_digit())
            })
        } else {
            serial.split_once(':').is_some_and(|(host, port)| {
                !host.is_empty()
                    && !host.contains(['[', ']'])
                    && !port.is_empty()
                    && port.bytes().all(|b| b.is_ascii_digit())
            })
        };
        if !valid_host_port {
            return false;
        }
    }
    if let Ok(only) = std::env::var("NECTARLINK_ADB_ONLY_SERIAL") {
        let only = only.trim();
        if !only.is_empty() && serial != only {
            return false;
        }
    }
    true
}

/// Whether adb reaches this device over the network rather than a cable:
/// `192.168.1.40:5555`, or `adb-XYZ._adb-tls-connect._tcp` from Wireless debugging.
/// A loopback address stays local (an emulator, or a forward on this PC).
pub fn is_network_serial(serial: &str) -> bool {
    let serial = serial.trim();
    if serial.contains("._adb-tls-connect.") || serial.contains("._adb._tcp") {
        return true;
    }
    let Some((host, port)) = serial.rsplit_once(':') else { return false };
    if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let host = host.trim_start_matches('[').trim_end_matches(']');
    match host.parse::<std::net::IpAddr>() {
        Ok(ip) => !ip.is_loopback(),
        Err(_) => !host.eq_ignore_ascii_case("localhost"),
    }
}

/// Errors returned by ADB server operations.
#[derive(Debug, thiserror::Error)]
pub enum AdbError {
    #[error("ADB I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("ADB server rejected command: {0}")]
    Remote(String),
    #[error("ADB protocol error: {0}")]
    Protocol(String),
    #[error("ADB serial {0} is blocked by safety guard")]
    BlockedSerial(String),
}

/// An Android device reported by the ADB server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdbDevice {
    pub serial: String,
    pub state: String,
    pub model: Option<String>,
    pub product: Option<String>,
    pub transport_id: Option<String>,
}

impl AdbDevice {
    /// Whether the device is online and authorized (`state == "device"`).
    pub fn is_online(&self) -> bool {
        self.state == "device"
    }
}

/// Parses the output of `host:devices-l` or `host:track-devices`, filtering out
/// any serial rejected by [`is_serial_allowed`].
pub fn parse_devices_l(input: &str) -> Vec<AdbDevice> {
    let mut devices = Vec::new();
    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("List of devices") {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let Some(serial) = parts.next() else { continue };
        let Some(state) = parts.next() else { continue };
        if !is_serial_allowed(serial) {
            continue;
        }
        let mut model = None;
        let mut product = None;
        let mut transport_id = None;
        for token in parts {
            if let Some(v) = token.strip_prefix("model:") {
                model = Some(v.to_owned());
            } else if let Some(v) = token.strip_prefix("product:") {
                product = Some(v.to_owned());
            } else if let Some(v) = token.strip_prefix("transport_id:") {
                transport_id = Some(v.to_owned());
            }
        }
        devices.push(AdbDevice {
            serial: serial.to_owned(),
            state: state.to_owned(),
            model,
            product,
            transport_id,
        });
    }
    devices
}

/// Sends a length-prefixed command to the ADB server and reads the 4-byte status.
pub async fn send_command(stream: &mut TcpStream, cmd: &str) -> Result<(), AdbError> {
    with_adb_timeout(async {
        let frame = format!("{:04x}{cmd}", cmd.len());
        stream.write_all(frame.as_bytes()).await?;
        stream.flush().await?;
        let mut status = [0u8; 4];
        stream.read_exact(&mut status).await?;
        match &status {
            b"OKAY" => Ok(()),
            b"FAIL" => {
                let reason = read_payload_inner(stream).await.unwrap_or_else(|_| "unknown failure".into());
                Err(AdbError::Remote(reason))
            }
            other => {
                Err(AdbError::Protocol(format!("unexpected ADB status: {}", String::from_utf8_lossy(other))))
            }
        }
    })
    .await
}

async fn read_payload_inner(stream: &mut TcpStream) -> Result<String, AdbError> {
    let mut len_hex = [0u8; 4];
    stream.read_exact(&mut len_hex).await?;
    read_payload_after_len(stream, &len_hex).await
}

async fn read_payload_after_len(stream: &mut TcpStream, len_hex: &[u8; 4]) -> Result<String, AdbError> {
    let len_str =
        std::str::from_utf8(len_hex).map_err(|_| AdbError::Protocol("invalid hex length".into()))?;
    let len = usize::from_str_radix(len_str, 16)
        .map_err(|_| AdbError::Protocol(format!("invalid hex length: {len_str}")))?;
    let mut buf = vec![0u8; len];
    if len > 0 {
        stream.read_exact(&mut buf).await?;
    }
    String::from_utf8(buf).map_err(|_| AdbError::Protocol("invalid UTF-8 payload".into()))
}

/// Reads a 4-hex-digit length prefix followed by the UTF-8 payload.
pub async fn read_payload(stream: &mut TcpStream) -> Result<String, AdbError> {
    with_adb_timeout(read_payload_inner(stream)).await
}

/// Lists attached ADB devices via `host:devices-l`.
pub async fn list_devices(server: SocketAddr) -> Result<Vec<AdbDevice>, AdbError> {
    let mut stream = connect_adb(server).await?;
    send_command(&mut stream, "host:devices-l").await?;
    let payload = read_payload(&mut stream).await?;
    Ok(parse_devices_l(&payload))
}

/// Sets up TCP port forwarding (`adb -s <serial> forward tcp:<local> tcp:<remote>`)
/// and returns the bound local TCP port on `127.0.0.1`.
pub async fn forward_tcp(
    server: SocketAddr,
    serial: &str,
    local_port: u16,
    remote_port: u16,
) -> Result<u16, AdbError> {
    if !is_serial_allowed(serial) {
        return Err(AdbError::BlockedSerial(serial.to_owned()));
    }
    let mut port = if local_port == 0 { free_local_port().await? } else { local_port };
    let mut attempts = 0;
    loop {
        let mut stream = connect_adb(server).await?;
        let cmd = format!("host-serial:{serial}:forward:tcp:{port};tcp:{remote_port}");
        match send_command(&mut stream, &cmd).await {
            Ok(()) => return Ok(port),
            // A port that was free a moment ago can still be taken by the time
            // adb binds it (on WSL the Windows side can hold a port the Linux
            // side just closed). Only then is another port worth trying.
            Err(AdbError::Remote(reason))
                if local_port == 0 && attempts < BIND_RETRIES && is_bind_failure(&reason) =>
            {
                attempts += 1;
                port = free_local_port().await?;
            }
            Err(e) => return Err(e),
        }
    }
}

/// How many other ports [`forward_tcp`] tries when adb can't bind the one it picked.
const BIND_RETRIES: u32 = 4;

/// A local TCP port that is free right now.
async fn free_local_port() -> Result<u16, AdbError> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    Ok(listener.local_addr()?.port())
}

/// Whether an adb `FAIL` reason means the local port couldn't be bound,
/// for example "cannot bind listener: Address already in use".
fn is_bind_failure(reason: &str) -> bool {
    let reason = reason.to_ascii_lowercase();
    reason.contains("cannot bind") || reason.contains("already in use")
}

/// Runs `command` in a shell on `serial` (`adb -s <serial> shell <command>`)
/// and returns the connection carrying its output. The command runs as long
/// as the connection stays open; dropping it ends the command.
pub async fn shell(server: SocketAddr, serial: &str, command: &str) -> Result<TcpStream, AdbError> {
    if !is_serial_allowed(serial) {
        return Err(AdbError::BlockedSerial(serial.to_owned()));
    }
    let mut stream = connect_adb(server).await?;
    send_command(&mut stream, &format!("host:transport:{serial}")).await?;
    send_command(&mut stream, &format!("shell:{command}")).await?;
    Ok(stream)
}

/// Removes a TCP port forward (`adb -s <serial> forward --remove tcp:<local>`).
pub async fn kill_forward_tcp(server: SocketAddr, serial: &str, local_port: u16) -> Result<(), AdbError> {
    if !is_serial_allowed(serial) {
        return Err(AdbError::BlockedSerial(serial.to_owned()));
    }
    let mut stream = connect_adb(server).await?;
    let cmd = format!("host-serial:{serial}:killforward:tcp:{local_port}");
    send_command(&mut stream, &cmd).await?;
    Ok(())
}

/// Streams device list changes from `host:track-devices` until `cancel` is
/// triggered or the ADB server closes the connection.
pub async fn track_devices<F>(
    server: SocketAddr,
    cancel: CancellationToken,
    mut on_change: F,
) -> Result<(), AdbError>
where
    F: FnMut(Vec<AdbDevice>),
{
    let mut stream = tokio::select! {
        _ = cancel.cancelled() => return Ok(()),
        res = connect_adb(server) => res?,
    };
    send_command(&mut stream, "host:track-devices").await?;
    loop {
        let mut len_hex = [0u8; 4];
        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            res = stream.read_exact(&mut len_hex) => { res?; }
        };
        let payload = tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            res = with_adb_timeout(read_payload_after_len(&mut stream, &len_hex)) => res?,
        };
        on_change(parse_devices_l(&payload));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_serials_are_told_from_cables_and_loopback() {
        for serial in [
            "192.168.1.40:5555",
            "[fe80::1]:5555",
            "pixel.lan:5555",
            "adb-1A2B3C4D-xYz._adb-tls-connect._tcp",
            "adb-1A2B3C4D-xYz._adb-tls-connect._tcp.",
        ] {
            assert!(is_network_serial(serial), "{serial}");
        }
        for serial in ["SYNTH_SERIAL_001", "emulator-5554", "127.0.0.1:5591", "localhost:5555", "[::1]:5555"]
        {
            assert!(!is_network_serial(serial), "{serial}");
        }
    }

    #[test]
    fn only_serials_made_of_serial_characters_are_allowed() {
        for serial in [
            "emulator-5554",
            "R5CT20ABCDE",
            "192.168.1.20:5555",
            "adb-R5CT20-abc._adb-tls-connect._tcp",
            "[fe80::1]:5555",
        ] {
            assert!(is_serial_allowed(serial), "{serial}");
        }
        for serial in [
            "",
            " emulator-5554",
            "emulator-5554 ",
            "a;b",
            "x:kill",
            "x:forward:tcp",
            "x:forward:tcp:1;tcp:2",
            "a b",
            "a
b",
            &"a".repeat(129),
        ] {
            assert!(!is_serial_allowed(serial), "{serial:?}");
        }
    }
    use std::sync::{Arc, Mutex};

    #[test]
    fn empty_serial_is_rejected_and_valid_serials_are_allowed() {
        assert!(!is_serial_allowed(""));
        assert!(!is_serial_allowed("   "));
        assert!(is_serial_allowed("emulator-5554"));
        assert!(is_serial_allowed("SYNTH_SERIAL_001"));
    }

    #[test]
    fn parse_devices_l_parses_metadata() {
        let sample = "\
List of devices attached
SYNTH_SERIAL_001       device product:dm1q model:Test_Phone device:dm1q transport_id:1
emulator-5554          device product:sdk_gphone64_x86_64 model:Pixel_9 device:emu64x transport_id:2
192.168.1.40:5555      offline transport_id:3
";
        let devices = parse_devices_l(sample);
        assert_eq!(devices.len(), 3);
        assert_eq!(
            devices[0],
            AdbDevice {
                serial: "SYNTH_SERIAL_001".into(),
                state: "device".into(),
                model: Some("Test_Phone".into()),
                product: Some("dm1q".into()),
                transport_id: Some("1".into()),
            }
        );
        assert!(devices[0].is_online());
        assert_eq!(
            devices[1],
            AdbDevice {
                serial: "emulator-5554".into(),
                state: "device".into(),
                model: Some("Pixel_9".into()),
                product: Some("sdk_gphone64_x86_64".into()),
                transport_id: Some("2".into()),
            }
        );
        assert!(devices[1].is_online());
        assert_eq!(
            devices[2],
            AdbDevice {
                serial: "192.168.1.40:5555".into(),
                state: "offline".into(),
                model: None,
                product: None,
                transport_id: Some("3".into()),
            }
        );
        assert!(!devices[2].is_online());
    }

    #[test]
    fn only_bind_failures_count_as_bind_failures() {
        assert!(is_bind_failure(
            "cannot bind listener: cannot bind to 127.0.0.1:5000: Address already in use (98)"
        ));
        assert!(is_bind_failure("Address already in use"));
        assert!(!is_bind_failure("device offline"));
        assert!(!is_bind_failure("device unauthorized."));
        assert!(!is_bind_failure("more than one device/emulator"));
    }

    /// A mock adb server that answers each `forward` with the next of
    /// `replies` (`None` = OKAY, `Some(reason)` = FAIL) and records the commands.
    async fn forward_server(replies: Vec<Option<&'static str>>) -> (SocketAddr, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let commands = Arc::new(Mutex::new(Vec::new()));
        let seen = commands.clone();
        tokio::spawn(async move {
            let mut replies = replies.into_iter();
            while let Ok((mut socket, _)) = listener.accept().await {
                let Ok(cmd) = read_payload(&mut socket).await else { continue };
                seen.lock().unwrap().push(cmd);
                match replies.next().flatten() {
                    None => {
                        let _ = socket.write_all(b"OKAY").await;
                    }
                    Some(reason) => {
                        let frame = format!("FAIL{:04x}{reason}", reason.len());
                        let _ = socket.write_all(frame.as_bytes()).await;
                    }
                }
                let _ = socket.flush().await;
            }
        });
        (addr, commands)
    }

    #[tokio::test]
    async fn forward_tries_another_port_only_when_adb_cannot_bind() {
        let (addr, commands) =
            forward_server(vec![Some("cannot bind listener: Address already in use"), None]).await;
        let port = forward_tcp(addr, "emulator-5554", 0, 53319).await.unwrap();
        let commands = commands.lock().unwrap().clone();
        assert_eq!(commands.len(), 2, "one retry after the bind failure");
        assert!(commands[1].contains(&format!("forward:tcp:{port};tcp:53319")));

        let (addr, commands) = forward_server(vec![Some("device offline")]).await;
        let err = forward_tcp(addr, "emulator-5554", 0, 53319).await.unwrap_err();
        assert!(matches!(err, AdbError::Remote(ref r) if r == "device offline"));
        assert_eq!(commands.lock().unwrap().len(), 1, "other errors are not retried");

        let (addr, commands) = forward_server(vec![Some("cannot bind listener"); 10]).await;
        assert!(forward_tcp(addr, "emulator-5554", 0, 53319).await.is_err());
        assert_eq!(commands.lock().unwrap().len(), 1 + BIND_RETRIES as usize, "retries are bounded");

        let (addr, commands) = forward_server(vec![Some("cannot bind listener")]).await;
        assert!(forward_tcp(addr, "emulator-5554", 5000, 53319).await.is_err());
        assert_eq!(commands.lock().unwrap().len(), 1, "a port the caller chose is not swapped");
    }

    #[tokio::test]
    async fn shell_picks_the_device_then_streams_the_commands_output() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let transport = read_payload(&mut socket).await.unwrap();
            socket.write_all(b"OKAY").await.unwrap();
            let shell = read_payload(&mut socket).await.unwrap();
            socket.write_all(b"OKAY").await.unwrap();
            socket.write_all(b"ready 41234\n").await.unwrap();
            (transport, shell)
        });
        let mut stream = shell(addr, "R5CT1234", "echo hi").await.unwrap();
        let mut line = String::new();
        tokio::io::AsyncBufReadExt::read_line(&mut tokio::io::BufReader::new(&mut stream), &mut line)
            .await
            .unwrap();
        assert_eq!(line, "ready 41234\n");
        let (transport, shell_cmd) = server.await.unwrap();
        assert_eq!(transport, "host:transport:R5CT1234");
        assert_eq!(shell_cmd, "shell:echo hi");
    }

    #[tokio::test]
    async fn mock_adb_server_forward_killforward_and_track_devices() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let commands = Arc::new(Mutex::new(Vec::<String>::new()));
        let commands_srv = commands.clone();

        let srv = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { break };
                let commands_srv = commands_srv.clone();
                tokio::spawn(async move {
                    let Ok(cmd) = read_payload(&mut socket).await else { return };
                    commands_srv.lock().unwrap().push(cmd.clone());
                    if cmd == "host:track-devices" {
                        let _ = socket.write_all(b"OKAY").await;
                        let update1 = "emulator-5554\tdevice\n";
                        let frame1 = format!("{:04x}{update1}", update1.len());
                        let _ = socket.write_all(frame1.as_bytes()).await;
                        let _ = socket.flush().await;
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    } else if cmd.starts_with("host-serial:") {
                        let _ = socket.write_all(b"OKAY").await;
                        let _ = socket.flush().await;
                    } else if cmd == "host:devices-l" {
                        let _ = socket.write_all(b"OKAY").await;
                        let body =
                            "List of devices attached\nemulator-5554 device model:Pixel_9 transport_id:4\n";
                        let frame = format!("{:04x}{body}", body.len());
                        let _ = socket.write_all(frame.as_bytes()).await;
                        let _ = socket.flush().await;
                    }
                });
            }
        });

        // Empty serial must be rejected before even connecting.
        let blocked_res = forward_tcp(addr, "   ", 0, 53319).await;
        assert!(matches!(blocked_res, Err(AdbError::BlockedSerial(_))));
        let blocked_kill = kill_forward_tcp(addr, "", 12345).await;
        assert!(matches!(blocked_kill, Err(AdbError::BlockedSerial(_))));

        let devs = list_devices(addr).await.unwrap();
        assert_eq!(devs.len(), 1);
        assert_eq!(devs[0].serial, "emulator-5554");

        let port = forward_tcp(addr, "emulator-5554", 0, 53319).await.unwrap();
        assert_ne!(port, 0);
        kill_forward_tcp(addr, "emulator-5554", port).await.unwrap();

        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();
        let mut tracked = Vec::new();
        track_devices(addr, cancel_clone, |list| {
            tracked = list;
            cancel.cancel();
        })
        .await
        .unwrap();

        assert_eq!(tracked.len(), 1);
        assert_eq!(tracked[0].serial, "emulator-5554");
        assert!(tracked[0].is_online());

        let recorded = commands.lock().unwrap().clone();
        assert!(recorded.iter().any(|c| c == "host:devices-l"));
        assert!(
            recorded.iter().any(|c| c == &format!("host-serial:emulator-5554:forward:tcp:{port};tcp:53319"))
        );
        assert!(recorded.iter().any(|c| c == &format!("host-serial:emulator-5554:killforward:tcp:{port}")));
        assert!(recorded.iter().any(|c| c == "host:track-devices"));

        srv.abort();
    }

    #[tokio::test]
    async fn hung_adb_server_times_out() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let srv = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
        });

        let err = list_devices(addr).await.unwrap_err();
        assert!(matches!(err, AdbError::Io(ref e) if e.kind() == std::io::ErrorKind::TimedOut));
        srv.abort();
    }
}
