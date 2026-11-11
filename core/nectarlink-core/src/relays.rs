// SPDX-License-Identifier: MPL-2.0
//! Relay configuration and on-demand iroh protocol probes.

use std::{collections::BTreeMap, time::Duration};

use iroh::{Endpoint, RelayConfig, RelayMap, RelayUrl, defaults::prod};
use iroh_relay::{
    client::ClientBuilder,
    protos::relay::{ClientToRelayMsg, RelayToClientMsg},
};
use n0_future::{BufferedStreamExt, SinkExt, StreamExt, stream};
use serde::{Deserialize, Serialize};
use url::Url;

const MAX_CUSTOM_RELAYS: usize = 16;
const PROBE_TIMEOUT: Duration = Duration::from_secs(6);

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelayRegion {
    NorthAmericaEast,
    NorthAmericaWest,
    Europe,
    AsiaPacific,
    Custom,
}

#[derive(Debug, Clone, Serialize)]
pub struct RelayServer {
    pub url: String,
    pub provider: String,
    pub region: RelayRegion,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct HomeRelay {
    pub url: String,
    pub connected: bool,
    pub failed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RelayRoute {
    pub device: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RelaySnapshot {
    pub servers: Vec<RelayServer>,
    pub home: Vec<HomeRelay>,
    pub routes: Vec<RelayRoute>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RelayProbe {
    pub url: String,
    pub rtt_ms: Option<u32>,
    pub timed_out: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum RelayEdit {
    Add,
    Enable,
    Disable,
    Remove,
}

#[derive(Debug, thiserror::Error)]
pub enum RelayEditError {
    #[error("Enter an HTTP or HTTPS relay address without credentials, a path or a query.")]
    InvalidAddress,
    #[error("This relay is already in the list.")]
    Duplicate,
    #[error("The list can hold up to {MAX_CUSTOM_RELAYS} custom relays.")]
    Limit,
    #[error("This relay cannot be changed.")]
    NotFound,
    #[error(transparent)]
    Storage(#[from] crate::Error),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct RelaySettings {
    presets: BTreeMap<RelayUrl, bool>,
    custom: BTreeMap<RelayUrl, bool>,
}

fn presets() -> [(RelayConfig, RelayRegion, &'static str, bool); 4] {
    use RelayRegion::*;
    [
        (prod::default_na_east_relay(), NorthAmericaEast, "n0", true),
        (prod::default_na_west_relay(), NorthAmericaWest, "n0", true),
        (prod::default_eu_relay(), Europe, "n0", true),
        (prod::default_ap_relay(), AsiaPacific, "n0", true),
    ]
}

impl RelaySettings {
    fn entries(&self) -> impl Iterator<Item = (RelayConfig, RelayRegion, &str, bool)> {
        presets()
            .into_iter()
            .map(|(config, region, provider, default)| {
                let enabled = self.presets.get(&config.url).copied().unwrap_or(default);
                (config, region, provider, enabled)
            })
            .chain(
                self.custom.iter().map(|(url, enabled)| {
                    (RelayConfig::from(url.clone()), RelayRegion::Custom, "", *enabled)
                }),
            )
    }

    pub(crate) fn servers(&self) -> Vec<RelayServer> {
        self.entries()
            .map(|(config, region, provider, enabled)| RelayServer {
                url: config.url.to_string(),
                region,
                provider: provider.to_owned(),
                enabled,
            })
            .collect()
    }

    pub(crate) fn enabled_map(&self, away: bool) -> RelayMap {
        if !away {
            return RelayMap::empty();
        }
        self.entries().filter_map(|(config, _, _, enabled)| enabled.then_some(config)).collect()
    }

    pub(crate) fn edit(&mut self, input: &str, action: RelayEdit) -> Result<(), RelayEditError> {
        let url = parse_address(input)?;
        let preset = presets().into_iter().find(|(config, _, _, _)| same_address(&config.url, &url));
        match (action, preset) {
            (RelayEdit::Add, Some(_)) => Err(RelayEditError::Duplicate),
            (RelayEdit::Add, None) => {
                if self.custom.contains_key(&url) {
                    return Err(RelayEditError::Duplicate);
                }
                if self.custom.len() >= MAX_CUSTOM_RELAYS {
                    return Err(RelayEditError::Limit);
                }
                self.custom.insert(url, true);
                Ok(())
            }
            (RelayEdit::Remove, None) if self.custom.remove(&url).is_some() => Ok(()),
            (RelayEdit::Remove, _) => Err(RelayEditError::NotFound),
            (RelayEdit::Enable | RelayEdit::Disable, Some((config, _, _, _))) => {
                self.presets.insert(config.url, matches!(action, RelayEdit::Enable));
                Ok(())
            }
            (RelayEdit::Enable | RelayEdit::Disable, None) => {
                let enabled = self.custom.get_mut(&url).ok_or(RelayEditError::NotFound)?;
                *enabled = matches!(action, RelayEdit::Enable);
                Ok(())
            }
        }
    }
}

fn same_address(left: &Url, right: &Url) -> bool {
    left.scheme() == right.scheme()
        && left.port() == right.port()
        && left.host_str().map(|host| host.trim_end_matches('.')) == right.host_str()
}

fn parse_address(input: &str) -> Result<RelayUrl, RelayEditError> {
    let input = input.trim();
    if input.len() > 1024 {
        return Err(RelayEditError::InvalidAddress);
    }
    let mut url =
        if input.contains("://") { Url::parse(input) } else { Url::parse(&format!("https://{input}")) }
            .map_err(|_| RelayEditError::InvalidAddress)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(RelayEditError::InvalidAddress);
    }
    if let Some(domain) = url.domain()
        && domain.ends_with('.')
    {
        let host = domain.trim_end_matches('.').to_owned();
        url.set_host(Some(&host)).map_err(|_| RelayEditError::InvalidAddress)?;
    }
    Ok(url.into())
}

/// Run a relay handshake and time a Ping/Pong with an ephemeral endpoint identity.
pub(crate) async fn probe_servers(
    endpoint: &Endpoint,
    servers: Vec<RelayServer>,
    proxy: Option<&Url>,
) -> Vec<RelayProbe> {
    let Ok(dns) = endpoint.dns_resolver() else { return Vec::new() };
    let tls = endpoint.tls_config();
    stream::iter(servers)
        .map(|server| {
            let client = server.url.parse::<RelayUrl>().ok().map(|url| {
                let client = ClientBuilder::new(url, iroh::SecretKey::generate(), dns.clone())
                    .tls_client_config(tls.clone());
                match proxy {
                    Some(url) => client.proxy_url(url.clone()),
                    None => client,
                }
            });
            async move {
                let result = tokio::time::timeout(PROBE_TIMEOUT, async {
                    let mut client = client?.connect().await.ok()?;
                    let nonce = rand::random();
                    let start = std::time::Instant::now();
                    client.send(ClientToRelayMsg::Ping(nonce)).await.ok()?;
                    while let Some(Ok(message)) = client.next().await {
                        if matches!(message, RelayToClientMsg::Pong(reply) if reply == nonce) {
                            return Some(start.elapsed().as_millis() as u32);
                        }
                    }
                    None
                })
                .await;
                RelayProbe { url: server.url, timed_out: result.is_err(), rtt_ms: result.ok().flatten() }
            }
        })
        .buffered_unordered(8)
        .collect()
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_preferences_round_trip_and_respect_away_mode() {
        let mut settings = RelaySettings::default();
        assert_eq!(settings.enabled_map(true), prod::default_relay_map());
        assert!(matches!(
            settings.edit("use1-1.relay.n0.iroh.link", RelayEdit::Add),
            Err(RelayEditError::Duplicate)
        ));
        settings.edit("use1-1.relay.n0.iroh.link", RelayEdit::Disable).unwrap();
        settings.edit("http://192.168.0.2:8080", RelayEdit::Add).unwrap();

        let json = serde_json::to_string(&settings).unwrap();
        let mut restored: RelaySettings = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.enabled_map(true), settings.enabled_map(true));
        assert!(restored.enabled_map(false).is_empty());
        restored.edit("http://192.168.0.2:8080", RelayEdit::Remove).unwrap();
        assert_eq!(restored.enabled_map(true).len(), 3);
    }
}
