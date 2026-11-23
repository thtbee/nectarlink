// SPDX-License-Identifier: GPL-3.0-or-later
//! Reads the local Tailscale inventory without depending on the daemon's IPC protocol.

use std::{
    borrow::Borrow,
    collections::{BTreeMap, BTreeSet},
    env,
    io::{self, Read},
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Deserializer};

const STATUS_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_STATUS_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TailscaleNode {
    #[serde(rename = "ID")]
    pub id: Option<String>,
    pub host_name: Option<String>,
    /// A MagicDNS name, including the trailing dot when present in status.
    #[serde(rename = "DNSName")]
    pub dns_name: Option<String>,
    #[serde(rename = "TailscaleIPs", default, deserialize_with = "null_as_default")]
    pub tailscale_ips: Vec<IpAddr>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub online: bool,
    pub cur_addr: Option<String>,
    #[serde(rename = "OS")]
    pub os: Option<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
pub struct TailscaleInventory {
    #[serde(rename = "Self")]
    pub local: Option<TailscaleNode>,
    /// Keyed by the node public key from the CLI's status response.
    #[serde(rename = "Peer", default, deserialize_with = "null_as_default")]
    pub peers: BTreeMap<String, TailscaleNode>,
}

fn null_as_default<'de, D: Deserializer<'de>, T: Deserialize<'de> + Default>(
    deserializer: D,
) -> Result<T, D::Error> {
    Ok(Option::deserialize(deserializer)?.unwrap_or_default())
}

pub fn parse_status_json(json: &str) -> serde_json::Result<TailscaleInventory> {
    serde_json::from_str(json)
}

pub fn is_tailscale_ipv4(ip: Ipv4Addr) -> bool {
    let [first, second, ..] = ip.octets();
    first == 100 && second & 0xc0 == 64
}

pub fn is_tailscale_ipv6(ip: Ipv6Addr) -> bool {
    matches!(ip.segments(), [0xfd7a, 0x115c, 0xa1e0, ..])
}

pub fn is_tailscale_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_tailscale_ipv4(ip),
        IpAddr::V6(ip) => is_tailscale_ipv6(ip),
    }
}

/// Intersects interface addresses with the local node's reported tailnet IPs.
/// Prefix membership alone also matches carrier CGNAT interfaces.
pub fn correlate_local_tailscale_ips(
    snapshot: &TailscaleInventory,
    local_ips: impl IntoIterator<Item = impl Borrow<IpAddr>>,
) -> BTreeSet<IpAddr> {
    let Some(local) = &snapshot.local else { return BTreeSet::new() };
    local_ips
        .into_iter()
        .map(|ip| *ip.borrow())
        .filter(|ip| is_tailscale_ip(*ip) && local.tailscale_ips.contains(ip))
        .collect()
}

pub fn find_tailscale_binary() -> Option<PathBuf> {
    let name = if cfg!(windows) { "tailscale.exe" } else { "tailscale" };
    let paths = env::var_os("PATH").unwrap_or_default();
    if let Some(binary) = env::split_paths(&paths)
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| dir.join(name))
        .find(|path| executable(path))
    {
        return Some(binary);
    }

    #[cfg(windows)]
    {
        ["ProgramFiles", "ProgramFiles(x86)"]
            .into_iter()
            .filter_map(env::var_os)
            .filter(|dir| !dir.is_empty())
            .map(|dir| PathBuf::from(dir).join("Tailscale").join(name))
            .find(|path| executable(path))
    }
    #[cfg(not(windows))]
    {
        let binary = PathBuf::from("/usr/bin/tailscale");
        executable(&binary).then_some(binary)
    }
}

fn executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else { return false };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}

/// Runs `tailscale status --json` with a two-second deadline.
pub fn discover() -> io::Result<TailscaleInventory> {
    let binary = find_tailscale_binary()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Tailscale CLI not found"))?;
    let mut command = Command::new(binary);
    command.args(["status", "--json"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let bytes = read_stdout(&mut command, STATUS_TIMEOUT)?;
    let json = std::str::from_utf8(&bytes).map_err(io::Error::other)?;
    parse_status_json(json).map_err(io::Error::other)
}

fn read_stdout(command: &mut Command, timeout: Duration) -> io::Result<Vec<u8>> {
    let deadline = Instant::now() + timeout;
    let mut child = command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
    let stdout = child.stdout.take().expect("child stdout was piped");
    let (send, receive) = mpsc::sync_channel(1);
    let reader = thread::Builder::new().spawn(move || {
        let mut result = Vec::new();
        let read = stdout.take(MAX_STATUS_BYTES + 1).read_to_end(&mut result);
        let _ = send.send(read.map(|_| result));
    });

    let result = reader.and_then(|_| {
        loop {
            let remaining = deadline.checked_duration_since(Instant::now()).ok_or_else(timed_out)?;
            if let Some(exit) = child.try_wait()? {
                if !exit.success() {
                    return Err(io::Error::other("Tailscale status exited unsuccessfully"));
                }
                break;
            }
            thread::sleep(remaining.min(Duration::from_millis(10)));
        }

        // A child can exit while another process still holds the inherited pipe.
        // Waiting on the reader with the same deadline bounds that case as well.
        let remaining = deadline.saturating_duration_since(Instant::now());
        let bytes = receive.recv_timeout(remaining).map_err(|error| match error {
            mpsc::RecvTimeoutError::Timeout => timed_out(),
            mpsc::RecvTimeoutError::Disconnected => io::Error::other("Tailscale status reader stopped"),
        })??;
        if bytes.len() as u64 > MAX_STATUS_BYTES {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Tailscale status output is too large"));
        }
        Ok(bytes)
    });

    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

fn timed_out() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "Tailscale status timed out")
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = r#"{
      "Version": "1.80.3",
      "BackendState": "Running",
      "Self": {
        "ID": "n-local", "HostName": "local-pc", "DNSName": "local-pc.example.ts.net.",
        "TailscaleIPs": ["100.65.1.2", "fd7a:115c:a1e0::1"], "Online": true, "OS": "linux"
      },
      "Peer": {
        "nodekey:test1": {
          "ID": "n-phone", "HostName": "phone", "DNSName": "phone.example.ts.net.",
          "TailscaleIPs": ["100.82.1.3", "fd7a:115c:a1e0::2"], "Online": true,
          "CurAddr": "[2001:db8::3]:41641", "OS": "android"
        },
        "nodekey:test2": {
          "ID": "n-offline", "HostName": "offline", "DNSName": "offline.example.ts.net.",
          "TailscaleIPs": ["100.99.2.1"], "Online": false, "CurAddr": "", "OS": "windows"
        },
        "nodekey:test3": {"TailscaleIPs": null}
      }
    }"#;

    #[test]
    fn parses_local_node_and_peers_including_offline_and_missing_fields() {
        let inventory = parse_status_json(STATUS).unwrap();
        let local = inventory.local.unwrap();
        assert_eq!(local.id.as_deref(), Some("n-local"));
        assert_eq!(local.dns_name.as_deref(), Some("local-pc.example.ts.net."));
        assert!(local.online);
        assert_eq!(
            local.tailscale_ips,
            ["100.65.1.2".parse::<IpAddr>().unwrap(), "fd7a:115c:a1e0::1".parse().unwrap()]
        );
        let phone = &inventory.peers["nodekey:test1"];
        assert_eq!(phone.host_name.as_deref(), Some("phone"));
        assert_eq!(phone.cur_addr.as_deref(), Some("[2001:db8::3]:41641"));
        assert_eq!(phone.os.as_deref(), Some("android"));
        assert!(!inventory.peers["nodekey:test2"].online);
        assert_eq!(inventory.peers["nodekey:test3"], TailscaleNode::default());
        assert_eq!(
            parse_status_json(r#"{"Self": null, "Peer": null}"#).unwrap(),
            TailscaleInventory::default()
        );
        assert!(parse_status_json("broken").is_err());
    }

    #[test]
    fn recognizes_prefix_edges_but_correlates_only_the_local_node() {
        for ip in ["100.64.0.0", "100.127.255.255", "fd7a:115c:a1e0:ffff::"] {
            assert!(is_tailscale_ip(ip.parse().unwrap()));
        }
        for ip in ["100.63.255.255", "100.128.0.0", "fd7a:115c:a1e1::", "fd00::1"] {
            assert!(!is_tailscale_ip(ip.parse().unwrap()));
        }
        let inventory = parse_status_json(STATUS).unwrap();
        let local: Vec<IpAddr> =
            ["100.65.1.2", "fd7a:115c:a1e0::1", "100.82.1.3", "100.100.9.9", "192.168.1.2", "100.65.1.2"]
                .map(|ip| ip.parse().unwrap())
                .into();
        assert_eq!(correlate_local_tailscale_ips(&inventory, &local), BTreeSet::from([local[0], local[1]]));
        assert!(correlate_local_tailscale_ips(&TailscaleInventory::default(), local).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn process_timeout_stops_a_stalled_cli() {
        let mut command = Command::new("sh");
        command.args(["-c", "while :; do :; done"]);
        let start = Instant::now();
        assert_eq!(
            read_stdout(&mut command, Duration::from_millis(40)).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}
