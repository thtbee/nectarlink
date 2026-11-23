// SPDX-License-Identifier: GPL-3.0-or-later
//! Ephemeral browser file transfers, independent of paired devices.

mod lan;
mod protocol;
mod transfer;

use std::{
    collections::HashSet,
    net::{Ipv4Addr, SocketAddrV4},
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use iroh::{PublicKey, RelayUrl, SecretKey, dns::DnsResolver};
use iroh_relay::{
    client::{Client, ClientBuilder},
    protos::relay::{ClientToRelayMsg, RelayToClientMsg, Status},
    tls::{CaTlsConfig, default_provider},
};
use n0_future::{BufferedStreamExt, SinkExt, StreamExt, stream};
use nectarlink_core::{FileSource, relays::RelaySnapshot};
use tokio::{net::TcpListener, sync::mpsc, time::Instant};
use url::Url;

pub const DEFAULT_SITE: &str = "https://anysend-nectarlink.netlify.app/";
const LIFETIME: Duration = Duration::from_secs(600);
pub const MAX_GUESTS: i32 = 100;
pub const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Lan,
    Relay,
}

impl Route {
    fn code(self) -> u8 {
        match self {
            Self::Lan => 0,
            Self::Relay => 1,
        }
    }

    fn chunk(self) -> usize {
        match self {
            Self::Lan => 256 * 1024,
            Self::Relay => 60 * 1024,
        }
    }

    fn file_limit(self) -> Option<u64> {
        match self {
            Self::Lan => None,
            Self::Relay => Some(MAX_FILE_BYTES),
        }
    }

    fn supports_size(self, size: u64) -> bool {
        let protocol_limit = u64::from(protocol::COMPLETE_INDEX - 1) * self.chunk() as u64;
        size <= self.file_limit().unwrap_or(protocol_limit)
    }
}

#[derive(Debug)]
pub enum Selection {
    Send { source: FileSource, name: Option<String> },
    Receive(PathBuf),
}

#[derive(Debug)]
pub enum Event {
    Ticket(Route, Url),
    Unavailable(Route, String),
    Started(Route),
    Guests { connected: usize, completed: usize, failed: usize },
    Progress(u64, u64),
    Received(PathBuf),
    Finished(Option<PathBuf>),
    Failed(String),
    Expired,
}

type Report = Arc<dyn Fn(Event) + Send + Sync>;
type Shared = Arc<transfer::Session>;

#[derive(Debug, Default)]
pub struct Network {
    pub lan_ip: Option<Ipv4Addr>,
    pub relay_interface: String,
    pub relays: Vec<RelayUrl>,
}

pub fn relay_candidates(snapshot: Option<RelaySnapshot>) -> Vec<RelayUrl> {
    let Some(snapshot) = snapshot else { return Vec::new() };
    let mut servers: Vec<_> = snapshot.servers.into_iter().filter(|server| server.enabled).collect();
    servers.sort_by_key(|server| {
        let home = snapshot.home.iter().any(|entry| entry.connected && entry.url == server.url);
        (!home, !server.provider.is_empty())
    });
    servers
        .into_iter()
        .filter_map(|server| server.url.parse::<RelayUrl>().ok())
        .filter(|url| url.scheme() == "https")
        .collect()
}

pub fn site(input: &str) -> Result<Url> {
    let input = input.trim();
    let url = if input.is_empty() {
        Url::parse(DEFAULT_SITE)?
    } else if input.contains("://") {
        Url::parse(input)?
    } else {
        Url::parse(&format!("https://{input}"))?
    };
    if url.scheme() != "https"
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("Enter an HTTPS page without a query or fragment.");
    }
    Ok(url)
}

pub async fn run(
    selection: Selection,
    site: Url,
    network: Network,
    limit: i32,
    report: Report,
) -> Result<()> {
    if !(1..=MAX_GUESTS).contains(&limit) {
        bail!("Choose between 1 and 100 browsers.");
    }
    let host = SecretKey::generate();
    let ticket = SecretKey::generate();
    let expires = SystemTime::now() + LIFETIME;
    let deadline = Instant::now() + LIFETIME;
    let session = Arc::new(
        transfer::Session::new(
            selection,
            host.clone(),
            ticket.clone(),
            deadline,
            limit as usize,
            report.clone(),
        )
        .await?,
    );
    let listener = match bind_lan(network.lan_ip).await {
        Ok(listener) => Some(listener),
        Err(error) => {
            report(Event::Unavailable(Route::Lan, error.to_string()));
            None
        }
    };
    let local_address = listener.as_ref().map(|(address, _)| *address);
    let site_origin = site.origin().ascii_serialization();
    let local = async {
        let Some((address, listener)) = listener else { return };
        let page = Url::parse(&format!("http://{address}/")).expect("IPv4 HTTP URL");
        report(Event::Ticket(Route::Lan, ticket_url(page, expires, host.public(), &ticket, None, &[])));
        if let Err(error) = lan::serve(listener, session.clone(), site_origin).await {
            report(Event::Unavailable(Route::Lan, error.to_string()));
        }
    };
    let global = async {
        let result = async {
            #[cfg(target_os = "linux")]
            let connector = if network.relay_interface.is_empty() {
                None
            } else {
                Some(
                    nectarlink_core::direct_network::RelayConnector::start(network.relay_interface)
                        .await
                        .context("Could not route the relay through the selected interface.")?,
                )
            };
            #[cfg(target_os = "linux")]
            let proxy = connector.as_ref().map(|connector| connector.url());
            #[cfg(not(target_os = "linux"))]
            let proxy = None;
            let mut seen = HashSet::new();
            let relays: Vec<_> = network
                .relays
                .into_iter()
                .filter_map(|relay| {
                    let browser_url = browser_relay_url(&relay).ok()?;
                    seen.insert(browser_url.clone()).then_some((relay, browser_url))
                })
                .collect();
            if relays.is_empty() {
                bail!("No HTTPS relay is enabled in the mobile network settings.");
            }
            let tls = CaTlsConfig::default().client_config(default_provider())?;
            let dns = DnsResolver::new();
            let (connected, mut ready) = mpsc::channel(1);
            let mut connections = tokio::task::JoinSet::new();
            for (url, _) in &relays {
                let mut builder =
                    ClientBuilder::new(url.clone(), host.clone(), dns.clone()).tls_client_config(tls.clone());
                if let Some(proxy) = proxy {
                    builder = builder.proxy_url(proxy.clone());
                }
                let url = url.clone();
                let session = session.clone();
                let connected = connected.clone();
                connections.spawn(async move { maintain_relay(&url, builder, &session, &connected).await });
            }
            drop(connected);
            let first = match tokio::time::timeout(Duration::from_secs(16), ready.recv()).await {
                Ok(first) => first,
                Err(_) => {
                    report(Event::Unavailable(
                        Route::Relay,
                        "Could not connect to an enabled mobile network relay. Retrying.".into(),
                    ));
                    ready.recv().await
                }
            };
            first.context("Could not start an AnySend relay connection.")?;
            drop(ready);
            let urls: Vec<_> = relays.into_iter().map(|(_, url)| url).collect();
            report(Event::Ticket(
                Route::Relay,
                ticket_url(
                    site,
                    expires,
                    host.public(),
                    &ticket,
                    local_address.filter(|addr| addr.ip().is_private()),
                    &urls,
                ),
            ));
            while let Some(result) = connections.join_next().await {
                result?;
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(error) = result {
            report(Event::Unavailable(Route::Relay, error.to_string()));
        }
    };
    if tokio::time::timeout_at(deadline, async { tokio::join!(local, global) }).await.is_err() {
        report(Event::Expired);
    }
    Ok(())
}

async fn bind_lan(ip: Option<Ipv4Addr>) -> Result<(SocketAddrV4, TcpListener)> {
    let ip = ip.context("No IPv4 address on a LAN interface.")?;
    let listener = TcpListener::bind((ip, 0)).await.context("Could not start the LAN listener.")?;
    let address = SocketAddrV4::new(ip, listener.local_addr()?.port());
    Ok((address, listener))
}

fn browser_relay_url(url: &RelayUrl) -> Result<Url> {
    if url.scheme() != "https" {
        bail!("AnySend Global requires an HTTPS relay.");
    }
    let mut relay = (**url).clone();
    if let Some(domain) = relay.domain().filter(|domain| domain.ends_with('.')) {
        let host = domain.trim_end_matches('.').to_owned();
        relay.set_host(Some(&host)).context("Invalid relay host")?;
    }
    relay.set_scheme("wss").map_err(|()| anyhow::anyhow!("Invalid relay URL"))?;
    relay.set_path("/relay");
    Ok(relay)
}

async fn maintain_relay(
    url: &RelayUrl,
    builder: ClientBuilder,
    session: &Shared,
    connected: &mpsc::Sender<()>,
) {
    let mut delay = Duration::from_secs(1);
    loop {
        match tokio::time::timeout(Duration::from_secs(15), builder.connect()).await {
            Ok(Ok(client)) => {
                let _ = connected.try_send(());
                delay = Duration::from_secs(1);
                if let Err(error) = relay_loop(client, session).await {
                    tracing::debug!(relay = %url, %error, "AnySend relay disconnected");
                }
            }
            Ok(Err(error)) => tracing::debug!(relay = %url, %error, "AnySend relay connection failed"),
            Err(_) => tracing::debug!(relay = %url, "AnySend relay connection timed out"),
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(15));
    }
}

async fn relay_loop(client: Client, session: &Shared) -> Result<()> {
    let (stream, mut sink) = client.split();
    let (send, mut receive) = mpsc::channel::<Vec<ClientToRelayMsg>>(16);
    let reader = async move {
        // Stop receiving when the socket fails, but finish file operations already in flight.
        let incoming = stream::unfold((stream, send.clone()), |(mut stream, stopped)| async {
            tokio::select! {
                biased;
                _ = stopped.closed() => None,
                message = tokio::time::timeout(Duration::from_secs(45), stream.next()) => {
                    match message {
                        Ok(Some(Ok(message))) => Some((message, (stream, stopped))),
                        _ => None,
                    }
                }
            }
        });
        let mut pending = incoming
            .boxed()
            .map(|message| async move {
                match message {
                    RelayToClientMsg::Ping(nonce) => vec![ClientToRelayMsg::Pong(nonce)],
                    RelayToClientMsg::Status(Status::RateLimited) => {
                        tracing::debug!("AnySend relay is throttling this connection");
                        Vec::new()
                    }
                    RelayToClientMsg::Datagrams { remote_endpoint_id, datagrams }
                        if datagrams.segment_size.is_none() =>
                    {
                        session
                            .handle(Route::Relay, remote_endpoint_id, &datagrams.contents)
                            .await
                            .into_iter()
                            .map(|reply| ClientToRelayMsg::Datagrams {
                                dst_endpoint_id: remote_endpoint_id,
                                datagrams: reply.into(),
                            })
                            .collect()
                    }
                    _ => Vec::new(),
                }
            })
            .buffered_unordered(16);
        while let Some(replies) = pending.next().await {
            if !replies.is_empty() {
                let _ = send.send(replies).await;
            }
        }
    };
    let writer = async move {
        while let Some(replies) = receive.recv().await {
            tokio::time::timeout(Duration::from_secs(45), async {
                for reply in replies {
                    sink.feed(reply).await?;
                }
                sink.flush().await
            })
            .await??;
        }
        Ok(())
    };
    let ((), result) = tokio::join!(reader, writer);
    result
}

fn ticket_url(
    mut page: Url,
    expiry: SystemTime,
    peer: PublicKey,
    ticket_key: &SecretKey,
    lan: Option<SocketAddrV4>,
    relays: &[Url],
) -> Url {
    let milliseconds = expiry.duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
    let mut ticket = Vec::with_capacity(128);
    ticket.push(if relays.len() > 1 { 3 } else { 2 });
    ticket.extend_from_slice(&milliseconds.to_be_bytes()[2..]);
    ticket.extend_from_slice(peer.as_bytes());
    ticket.extend_from_slice(&ticket_key.to_bytes());
    let (ip, port) = lan.map_or(([0; 4], 0), |address| (address.ip().octets(), address.port()));
    ticket.extend_from_slice(&ip);
    ticket.extend_from_slice(&port.to_be_bytes());
    for (index, relay) in relays.iter().enumerate() {
        if index != 0 {
            ticket.push(b'\n');
        }
        ticket.extend_from_slice(relay.as_str().as_bytes());
    }
    page.set_fragment(Some(&base43(&ticket)));
    page
}

/// Same integer format as the browser: leading hexadecimal 1, then the bytes.
fn base43(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ$*+-./:";
    let mut digits = vec![1u16];
    for byte in bytes {
        let mut carry = u16::from(*byte);
        for digit in &mut digits {
            let value = *digit * 256 + carry;
            *digit = value % 43;
            carry = value / 43;
        }
        while carry > 0 {
            digits.push(carry % 43);
            carry /= 43;
        }
    }
    digits.iter().rev().map(|digit| char::from(ALPHABET[usize::from(*digit)])).collect()
}
