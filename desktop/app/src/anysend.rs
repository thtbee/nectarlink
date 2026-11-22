// SPDX-License-Identifier: GPL-3.0-or-later
//! Ephemeral browser file transfers, independent of paired devices.

mod lan;
mod transfer;

use std::{
    net::{Ipv4Addr, UdpSocket},
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use iroh::{PublicKey, RelayUrl, SecretKey, dns::DnsResolver};
use iroh_relay::{
    client::{Client, ClientBuilder},
    protos::relay::{ClientToRelayMsg, RelayToClientMsg},
    tls::{CaTlsConfig, default_provider},
};
use n0_future::{BufferedStreamExt, SinkExt, StreamExt, stream};
use tokio::sync::Mutex;
use url::Url;

pub(crate) const DEFAULT_SITE: &str = "https://anysend-nectarlink.netlify.app/";
const LIFETIME: Duration = Duration::from_secs(600);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Route {
    Lan,
    Relay,
}

pub(crate) enum Selection {
    Send(PathBuf),
    Receive(PathBuf),
}

pub(crate) enum Event {
    Ticket(Route, Url),
    Unavailable(Route, String),
    Started(Route),
    Progress(u64, u64),
    Finished(Option<PathBuf>),
    Failed(String),
    Expired,
}

type Report = Arc<dyn Fn(Event) + Send + Sync>;
type Shared = Arc<Mutex<transfer::Session>>;

pub(crate) fn site(input: &str) -> Result<Url> {
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

pub(crate) async fn run(
    selection: Selection,
    site: Url,
    interface: String,
    relays: Vec<RelayUrl>,
    report: Report,
) -> Result<()> {
    let host = SecretKey::generate();
    let guest = SecretKey::generate();
    let expires = SystemTime::now() + LIFETIME;
    let session = Arc::new(Mutex::new(
        transfer::Session::new(selection, &host, &guest, expires, report.clone()).await?,
    ));
    let ticket = |page: Url, relay: Option<&Url>| {
        ticket_url(page, expires, host.public(), &guest, relay)
    };

    let local = async {
        let result = lan_route(&interface, &session, &ticket, &report).await;
        if let Err(error) = result {
            report(Event::Unavailable(Route::Lan, error.to_string()));
        }
    };
    let global = async {
        let result = async {
            let (url, client) = connect_relay(relays, &host).await?;
            let mut relay = (*url).clone();
            if relay.domain().is_some_and(|domain| domain.ends_with('.')) {
                let domain = relay.domain().unwrap_or_default().trim_end_matches('.').to_owned();
                relay.set_host(Some(&domain)).context("Invalid relay host")?;
            }
            relay.set_scheme("wss").map_err(|()| anyhow::anyhow!("Invalid relay URL"))?;
            relay.set_path("/relay");
            report(Event::Ticket(Route::Relay, ticket(site, Some(&relay))));
            relay_loop(client, guest.public(), &session).await
        }
        .await;
        if let Err(error) = result {
            report(Event::Unavailable(Route::Relay, error.to_string()));
        }
    };
    if tokio::time::timeout(LIFETIME, async { tokio::join!(local, global) }).await.is_err()
        && !session.lock().await.finished()
    {
        report(Event::Expired);
    }
    Ok(())
}

async fn lan_route(
    interface: &str,
    session: &Shared,
    ticket: &impl Fn(Url, Option<&Url>) -> Url,
    report: &Report,
) -> Result<()> {
    let ip = lan_ip(interface).context("No IPv4 address on the selected LAN interface.")?;
    let listener =
        tokio::net::TcpListener::bind((ip, 0)).await.context("Could not start the LAN listener.")?;
    let address = listener.local_addr()?;
    let page = Url::parse(&format!("http://{address}/"))?;
    report(Event::Ticket(Route::Lan, ticket(page, None)));
    lan::serve(listener, session.clone()).await
}

fn lan_ip(selected: &str) -> Option<Ipv4Addr> {
    // Prefer the interface used for the default route, provided it is physical.
    // An explicit NectarLink choice takes priority, including a LAN without Internet.
    let route_ip = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .and_then(|socket| {
            socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9))?;
            socket.local_addr()
        })
        .ok()
        .map(|address| address.ip());
    netdev::get_interfaces()
        .into_iter()
        .filter(|interface| {
            interface.is_up()
                && !interface.is_loopback()
                && if selected.is_empty() { interface.is_physical() } else { interface.name == selected }
        })
        .flat_map(|interface| interface.ipv4.into_iter())
        .map(|network| network.addr())
        .filter(|ip| !ip.is_unspecified() && !ip.is_loopback() && !ip.is_link_local())
        .min_by_key(|ip| route_ip != Some((*ip).into()))
}

async fn connect_relay(relays: Vec<RelayUrl>, host: &SecretKey) -> Result<(RelayUrl, Client)> {
    if relays.is_empty() {
        bail!("No HTTPS relay is enabled in the mobile network settings.");
    }
    let tls = CaTlsConfig::default().client_config(default_provider())?;
    let dns = DnsResolver::new();
    let dial = |url: RelayUrl| {
        let builder =
            ClientBuilder::new(url.clone(), host.clone(), dns.clone()).tls_client_config(tls.clone());
        async move {
            let client = tokio::time::timeout(Duration::from_secs(5), builder.connect()).await.ok()?.ok()?;
            Some((url, client))
        }
    };
    let mut relays = relays.into_iter();
    if let Some(first) = relays.next() {
        if let Some(client) = dial(first).await {
            return Ok(client);
        }
    }
    let mut candidates = stream::iter(relays).map(dial).buffered_unordered(4);
    while let Some(candidate) = candidates.next().await {
        if let Some(client) = candidate {
            return Ok(client);
        }
    }
    bail!("Could not connect to an enabled mobile network relay.")
}

async fn relay_loop(mut client: Client, guest: PublicKey, session: &Shared) -> Result<()> {
    while let Some(message) = client.next().await {
        match message.map_err(|_| anyhow::anyhow!("The relay connection closed."))? {
            RelayToClientMsg::Ping(nonce) => {
                client.send(ClientToRelayMsg::Pong(nonce)).await?;
            }
            RelayToClientMsg::Datagrams { remote_endpoint_id, datagrams }
                if remote_endpoint_id == guest && datagrams.segment_size.is_none() =>
            {
                let replies = session.lock().await.handle(Route::Relay, &datagrams.contents).await;
                for reply in replies {
                    client
                        .send(ClientToRelayMsg::Datagrams { dst_endpoint_id: guest, datagrams: reply.into() })
                        .await?;
                }
            }
            _ => {}
        }
    }
    bail!("The relay connection closed.")
}

fn ticket_url(
    mut page: Url,
    expiry: SystemTime,
    peer: PublicKey,
    guest: &SecretKey,
    relay: Option<&Url>,
) -> Url {
    let milliseconds = expiry.duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
    let mut ticket = Vec::with_capacity(128);
    ticket.extend_from_slice(&milliseconds.to_be_bytes()[2..]);
    ticket.extend_from_slice(peer.as_bytes());
    ticket.extend_from_slice(&guest.to_bytes());
    if let Some(relay) = relay {
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
