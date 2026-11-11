// SPDX-License-Identifier: MPL-2.0
//! Source-bound LAN sockets and a private relay connector for Linux.

use std::{
    io,
    net::{SocketAddr, SocketAddrV6},
    time::Duration,
};

use data_encoding::{BASE64, BASE64URL};
use iroh::endpoint::{BindOpts, Builder};
use serde::Serialize;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpSocket, TcpStream},
    sync::watch,
    task::{AbortHandle, JoinSet},
};
use url::Url;

#[derive(Debug, Serialize)]
pub struct NetworkInterface {
    pub name: String,
    pub direct: bool,
}

/// Lists available interfaces by name and marks connected physical links.
pub fn network_interfaces() -> Vec<NetworkInterface> {
    let mut interfaces: Vec<_> = netdev::get_interfaces()
        .into_iter()
        .filter(|nic| nic.is_up() && !nic.is_loopback() && (!nic.ipv4.is_empty() || !nic.ipv6.is_empty()))
        .map(|nic| {
            let physical = !std::path::Path::new("/sys/devices/virtual/net").join(&nic.name).exists()
                && (predictable_physical_name(&nic.name) || nic.is_physical());
            let connected = nic.is_running()
                && (nic.ipv4.iter().any(|net| !net.addr().is_link_local())
                    || nic.ipv6.iter().any(|net| !net.addr().is_unicast_link_local()));
            NetworkInterface { name: nic.name, direct: physical && connected }
        })
        .collect();
    interfaces.sort_unstable_by(|left, right| left.name.cmp(&right.name));
    interfaces
}

fn predictable_physical_name(name: &str) -> bool {
    let [a, b, suffix @ ..] = name.as_bytes() else { return false };
    if !matches!((*a, *b), (b'e', b'n') | (b'w', b'l' | b'w') | (b'i', b'b')) {
        return false;
    }
    match suffix {
        [b'x', mac @ ..] => mac.len() == 12 && mac.iter().all(u8::is_ascii_hexdigit),
        [b'o' | b'd' | b'p' | b's' | b'v' | b'X' | b'b' | b'c' | b'a' | b'P', b'0'..=b'9', rest @ ..] => {
            rest.iter().all(u8::is_ascii_alphanumeric)
        }
        _ => false,
    }
}

pub(crate) fn bind_lan_interface(builder: Builder, name: &str, ipv6: bool) -> crate::Result<Builder> {
    let Some(nic) = netdev::get_interfaces().into_iter().find(|nic| nic.name == name && nic.is_up()) else {
        return Ok(builder);
    };
    let mut builder = builder;
    for net in nic.ipv4 {
        builder = bind_subnet(builder, SocketAddr::new(net.addr().into(), 0), net.prefix_len())?;
    }
    if ipv6 {
        for net in nic.ipv6 {
            let ip = net.addr();
            let scope = if ip.is_unicast_link_local() { nic.index } else { 0 };
            let addr = SocketAddrV6::new(ip, 0, 0, scope).into();
            builder = bind_subnet(builder, addr, net.prefix_len())?;
        }
    }
    Ok(builder)
}

fn bind_subnet(builder: Builder, address: SocketAddr, prefix: u8) -> crate::Result<Builder> {
    if prefix == 0 || address.ip().is_unspecified() {
        return Ok(builder);
    }
    builder
        .bind_addr_with_opts(
            address,
            BindOpts::default().set_prefix_len(prefix).set_is_default_route(false).set_is_required(false),
        )
        .map_err(crate::error::net)
}

/// iroh exposes CONNECT as its dial extension, but does not expose its TCP socket.
/// Authenticate this loopback connector and leave relay TLS and iroh framing intact.
pub(crate) struct RelayConnector {
    url: Url,
    interface: watch::Sender<String>,
    task: AbortHandle,
}

impl RelayConnector {
    pub(crate) async fn start(interface: String) -> io::Result<Self> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let password = format!("{:032x}", rand::random::<u128>());
        let credentials = format!("nectarlink:{password}");
        let auth = [
            format!("Basic {}", BASE64.encode(credentials.as_bytes())),
            format!("Basic {}", BASE64URL.encode(credentials.as_bytes())),
        ];
        let url = Url::parse(&format!("http://{credentials}@{}/", listener.local_addr()?))
            .map_err(io::Error::other)?;
        let (interface, route) = watch::channel(interface);
        let task = tokio::spawn(async move {
            let mut streams = JoinSet::new();
            loop {
                tokio::select! {
                    Some(_) = streams.join_next(), if !streams.is_empty() => {}
                    accepted = listener.accept(), if streams.len() < 64 => {
                        let Ok((stream, _)) = accepted else { break };
                        let mut route = route.clone();
                        let name = route.borrow_and_update().clone();
                        let auth = auth.clone();
                        streams.spawn(async move {
                            tokio::select! {
                                _ = route.changed() => {}
                                _ = tunnel(stream, &auth, &name) => {}
                            }
                        });
                    }
                }
            }
        })
        .abort_handle();
        Ok(Self { url, interface, task })
    }

    pub(crate) fn url(&self) -> &Url {
        &self.url
    }

    pub(crate) fn close(&self) {
        self.task.abort();
    }

    pub(crate) fn select(&self, name: &str) {
        self.interface.send_if_modified(|current| {
            if current.as_str() == name {
                return false;
            }
            name.clone_into(current);
            true
        });
    }
}

pub(crate) fn validate_interface(name: &str) -> io::Result<()> {
    if name.is_empty() {
        return Ok(());
    }
    if !network_interfaces().iter().any(|interface| interface.name == name) {
        return Err(io::Error::new(io::ErrorKind::NotFound, "network interface unavailable"));
    }
    TcpSocket::new_v4()?.bind_device(Some(name.as_bytes()))
}

impl Drop for RelayConnector {
    fn drop(&mut self) {
        self.close();
    }
}

async fn tunnel(stream: TcpStream, auth: &[String; 2], interface: &str) -> io::Result<()> {
    stream.set_nodelay(true)?;
    let mut downstream = BufReader::new(stream);
    let (mut upstream, is_tunnel) = tokio::time::timeout(Duration::from_secs(4), async {
        let mut headers = Vec::new();
        let mut limited = (&mut downstream).take(4096);
        loop {
            if limited.read_until(b'\n', &mut headers).await? == 0 {
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
            }
            if headers.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let headers = std::str::from_utf8(&headers).map_err(io::Error::other)?;
        let (request, headers) = headers.split_once("\r\n").ok_or(io::ErrorKind::InvalidData)?;
        let (method, request) = request.split_once(' ').ok_or(io::ErrorKind::InvalidData)?;
        let (target, _) = request
            .split_once(' ')
            .filter(|(_, version)| *version == "HTTP/1.1" || *version == "HTTP/1.0")
            .ok_or(io::ErrorKind::InvalidData)?;
        if !headers.lines().filter_map(|line| line.split_once(':')).any(|(name, value)| {
            name.eq_ignore_ascii_case("Proxy-Authorization")
                && auth.iter().any(|expected| expected == value.trim())
        }) {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
        match method {
            "CONNECT" => Ok((connect(target, interface).await?, true)),
            "GET" => {
                // iroh uses an absolute-form GET to probe plain HTTP relays.
                let url = Url::parse(target).map_err(io::Error::other)?;
                if url.scheme() != "http" || url.path() != "/ping" {
                    return Err(io::Error::from(io::ErrorKind::InvalidData));
                }
                let host = url.host_str().ok_or(io::ErrorKind::InvalidData)?;
                let port = url.port_or_known_default().ok_or(io::ErrorKind::InvalidData)?;
                let authority = format!("{host}:{port}");
                let mut stream = connect(&authority, interface).await?;
                let request = format!("GET /ping HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n");
                stream.write_all(request.as_bytes()).await?;
                Ok((stream, false))
            }
            _ => Err(io::Error::from(io::ErrorKind::InvalidData)),
        }
    })
    .await??;
    upstream.set_nodelay(true)?;
    if is_tunnel {
        downstream.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await?;
        tokio::io::copy_bidirectional(&mut downstream, &mut upstream).await?;
    } else {
        tokio::time::timeout(Duration::from_secs(4), tokio::io::copy(&mut upstream, &mut downstream))
            .await??;
    }
    Ok(())
}

async fn connect(target: &str, interface: &str) -> io::Result<TcpStream> {
    if interface.is_empty() {
        return TcpStream::connect(target).await;
    }
    let mut last_error = io::Error::from(io::ErrorKind::AddrNotAvailable);
    for address in tokio::net::lookup_host(target).await? {
        let socket = if address.is_ipv4() { TcpSocket::new_v4()? } else { TcpSocket::new_v6()? };
        socket.bind_device(Some(interface.as_bytes()))?;
        match tokio::time::timeout(Duration::from_millis(1300), socket.connect(address)).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(error)) => last_error = error,
            Err(error) => last_error = error.into(),
        }
    }
    Err(last_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_predictable_physical_names() {
        for name in ["enp4s0", "wlp2s0", "eno1", "enx001122aabbcc", "wwP1p0s20u1", "ibp3s0"] {
            assert!(predictable_physical_name(name), "{name}");
        }
        for name in ["tun0", "tailscale0", "docker0", "veth42", "enp4s0.42", "wlan0"] {
            assert!(!predictable_physical_name(name), "{name}");
        }
    }

    #[tokio::test]
    async fn relay_connector_forwards_and_retires_old_streams() {
        let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let connector = RelayConnector::start(String::new()).await.unwrap();
        let url = connector.url();
        let proxy_addr = format!("127.0.0.1:{}", url.port().unwrap());
        let mut client = TcpStream::connect(proxy_addr).await.unwrap();
        let credentials = format!("{}:{}", url.username(), url.password().unwrap());
        let auth = BASE64URL.encode(credentials.as_bytes());
        let request = format!(
            "CONNECT {} HTTP/1.1\r\nProxy-Authorization: Basic {auth}\r\n\r\n",
            target.local_addr().unwrap(),
        );
        client.write_all(request.as_bytes()).await.unwrap();

        tokio::time::timeout(Duration::from_secs(2), async {
            let (mut upstream, _) = target.accept().await.unwrap();
            let mut response = [0; b"HTTP/1.1 200 Connection Established\r\n\r\n".len()];
            client.read_exact(&mut response).await.unwrap();
            assert_eq!(&response, b"HTTP/1.1 200 Connection Established\r\n\r\n");
            client.write_all(b"ping").await.unwrap();
            let mut payload = [0; 4];
            upstream.read_exact(&mut payload).await.unwrap();
            assert_eq!(&payload, b"ping");

            // Changing the route must close old TCP tunnels before the next check.
            connector.interface.send_replace("other-interface".to_owned());
            assert_eq!(client.read(&mut payload).await.unwrap(), 0);
        })
        .await
        .unwrap();
    }
}
