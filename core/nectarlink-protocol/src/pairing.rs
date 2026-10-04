// SPDX-License-Identifier: MPL-2.0
//! Pairing primitives (protocol §9): the QR pairing URI, the pairing proof,
//! and the numeric-comparison commitment and short authentication string.

use std::net::SocketAddr;

use data_encoding::BASE64URL_NOPAD;
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};
use url::Url;

use crate::DeviceId;

type HmacSha256 = Hmac<Sha256>;

/// Length of the one-time pairing secret in the QR code (128 bits).
pub const SECRET_LEN: usize = 16;
/// Length of the nonces used in the nearby (numeric comparison) flow.
pub const NONCE_LEN: usize = 16;

const PROOF_LABEL: &[u8] = b"nectarlink-pair-v0";
const SAS_LABEL: &[u8] = b"nectarlink-sas-v0";

/// Contents of the pairing QR code shown by the PC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingUri {
    pub id: DeviceId,
    pub secret: [u8; SECRET_LEN],
    pub name: String,
    pub addrs: Vec<SocketAddr>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PairingUriError {
    #[error("not a Nectarlink pairing link")]
    NotPairing,
    #[error("pairing link version {0} is not supported; please update Nectarlink")]
    UnsupportedVersion(String),
    #[error("pairing link is missing {0}")]
    Missing(&'static str),
    #[error("pairing link has an invalid {0}")]
    Invalid(&'static str),
}

impl PairingUri {
    /// `nectarlink://pair?v=0&id=…&s=…&n=…[&a=ip:port,…]`
    pub fn to_uri(&self) -> String {
        let mut url = Url::parse("nectarlink://pair").expect("static URL");
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("v", "0");
            q.append_pair("id", &self.id.to_string());
            q.append_pair("s", &BASE64URL_NOPAD.encode(&self.secret));
            q.append_pair("n", &self.name);
            if !self.addrs.is_empty() {
                let addrs: Vec<String> = self.addrs.iter().map(ToString::to_string).collect();
                q.append_pair("a", &addrs.join(","));
            }
        }
        url.into()
    }

    pub fn parse(uri: &str) -> Result<Self, PairingUriError> {
        let url = Url::parse(uri.trim()).map_err(|_| PairingUriError::NotPairing)?;
        if url.scheme() != "nectarlink" || url.host_str() != Some("pair") {
            return Err(PairingUriError::NotPairing);
        }
        let get = |key: &str| url.query_pairs().find(|(k, _)| k == key).map(|(_, v)| v.into_owned());

        let version = get("v").ok_or(PairingUriError::Missing("version"))?;
        if version != "0" {
            return Err(PairingUriError::UnsupportedVersion(version));
        }
        let id = get("id")
            .ok_or(PairingUriError::Missing("device ID"))?
            .parse()
            .map_err(|_| PairingUriError::Invalid("device ID"))?;
        let secret: [u8; SECRET_LEN] = BASE64URL_NOPAD
            .decode(get("s").ok_or(PairingUriError::Missing("secret"))?.as_bytes())
            .ok()
            .and_then(|b| b.try_into().ok())
            .ok_or(PairingUriError::Invalid("secret"))?;
        let name = get("n").unwrap_or_default();
        let addrs = match get("a") {
            None => Vec::new(),
            Some(list) => list
                .split(',')
                .filter(|s| !s.is_empty())
                .map(|s| s.parse().map_err(|_| PairingUriError::Invalid("address")))
                .collect::<Result<_, _>>()?,
        };
        Ok(PairingUri { id, secret, name, addrs })
    }
}

fn proof_mac(secret: &[u8; SECRET_LEN], joining: &DeviceId, host: &DeviceId) -> HmacSha256 {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(PROOF_LABEL);
    mac.update(joining.as_bytes());
    mac.update(host.as_bytes());
    mac
}

/// Proof that the joining device (phone) scanned the host's (PC's) QR code:
/// `HMAC-SHA256(S, "nectarlink-pair-v0" || joining_id || host_id)`.
pub fn pair_proof(secret: &[u8; SECRET_LEN], joining: &DeviceId, host: &DeviceId) -> [u8; 32] {
    proof_mac(secret, joining, host).finalize().into_bytes().into()
}

/// Verifies a pairing proof in constant time.
pub fn verify_pair_proof(
    secret: &[u8; SECRET_LEN],
    joining: &DeviceId,
    host: &DeviceId,
    proof: &[u8],
) -> bool {
    proof_mac(secret, joining, host).verify_slice(proof).is_ok()
}

/// Commitment to the initiator's nonce: `SHA-256(nA)`.
pub fn commitment(n_a: &[u8]) -> [u8; 32] {
    Sha256::digest(n_a).into()
}

/// Checks a revealed nonce against its commitment.
pub fn verify_commitment(commitment_bytes: &[u8], n_a: &[u8]) -> bool {
    let expected = commitment(n_a);
    // Not secret, but compare without early exit anyway.
    commitment_bytes.len() == expected.len()
        && commitment_bytes.iter().zip(expected).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

/// The 6-digit short authentication string both screens show:
/// `uint32(HMAC-SHA256(nA || nB, "nectarlink-sas-v0" || idA || idB)[0..4]) mod 1,000,000`,
/// where A is the initiator and B the responder.
pub fn sas_code(n_a: &[u8], n_b: &[u8], id_a: &DeviceId, id_b: &DeviceId) -> String {
    let mut key = Vec::with_capacity(n_a.len() + n_b.len());
    key.extend_from_slice(n_a);
    key.extend_from_slice(n_b);
    let mut mac = HmacSha256::new_from_slice(&key).expect("HMAC accepts any key length");
    mac.update(SAS_LABEL);
    mac.update(id_a.as_bytes());
    mac.update(id_b.as_bytes());
    let digest = mac.finalize().into_bytes();
    let value = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) % 1_000_000;
    format!("{value:06}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(b: u8) -> DeviceId {
        DeviceId([b; 32])
    }

    #[test]
    fn uri_round_trips() {
        let uri = PairingUri {
            id: id(3),
            secret: [9; SECRET_LEN],
            name: "Bee's Desktop".into(),
            addrs: vec!["192.168.1.20:41641".parse().unwrap(), "[fe80::1]:41641".parse().unwrap()],
        };
        let text = uri.to_uri();
        assert!(text.starts_with("nectarlink://pair?v=0&id="));
        assert_eq!(PairingUri::parse(&text).unwrap(), uri);
    }

    #[test]
    fn uri_without_addresses() {
        let uri = PairingUri { id: id(1), secret: [0; SECRET_LEN], name: String::new(), addrs: vec![] };
        assert_eq!(PairingUri::parse(&uri.to_uri()).unwrap(), uri);
    }

    #[test]
    fn uri_errors() {
        assert_eq!(PairingUri::parse("https://example.com"), Err(PairingUriError::NotPairing));
        assert_eq!(
            PairingUri::parse("nectarlink://pair?v=9&id=x&s=y"),
            Err(PairingUriError::UnsupportedVersion("9".into()))
        );
        assert_eq!(
            PairingUri::parse("nectarlink://pair?v=0&s=AAAA"),
            Err(PairingUriError::Missing("device ID"))
        );
        let good =
            PairingUri { id: id(1), secret: [0; SECRET_LEN], name: "x".into(), addrs: vec![] }.to_uri();
        let bad_secret = good.replace("s=AAAAAAAAAAAAAAAAAAAAAA", "s=AAAA");
        assert_eq!(PairingUri::parse(&bad_secret), Err(PairingUriError::Invalid("secret")));
    }

    #[test]
    fn proof_binds_secret_and_both_ids() {
        let secret = [5u8; SECRET_LEN];
        let proof = pair_proof(&secret, &id(1), &id(2));
        assert!(verify_pair_proof(&secret, &id(1), &id(2), &proof));
        assert!(!verify_pair_proof(&[6u8; SECRET_LEN], &id(1), &id(2), &proof), "wrong secret");
        assert!(!verify_pair_proof(&secret, &id(9), &id(2), &proof), "wrong joining device");
        assert!(!verify_pair_proof(&secret, &id(1), &id(9), &proof), "wrong host");
        assert!(!verify_pair_proof(&secret, &id(1), &id(2), &proof[..31]), "truncated");
    }

    #[test]
    fn commitment_checks() {
        let n_a = [1u8; NONCE_LEN];
        let c = commitment(&n_a);
        assert!(verify_commitment(&c, &n_a));
        assert!(!verify_commitment(&c, &[2u8; NONCE_LEN]));
        assert!(!verify_commitment(&c[..10], &n_a));
    }

    #[test]
    fn sas_is_six_digits_and_order_sensitive() {
        let (n_a, n_b) = ([1u8; NONCE_LEN], [2u8; NONCE_LEN]);
        let code = sas_code(&n_a, &n_b, &id(1), &id(2));
        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|c| c.is_ascii_digit()));
        assert_eq!(code, sas_code(&n_a, &n_b, &id(1), &id(2)), "deterministic");
        // A man in the middle presents a different identity on one side.
        assert_ne!(code, sas_code(&n_a, &n_b, &id(1), &id(3)));
        assert_ne!(code, sas_code(&n_b, &n_a, &id(1), &id(2)));
    }
}
