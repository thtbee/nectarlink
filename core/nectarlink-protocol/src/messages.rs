// SPDX-License-Identifier: MPL-2.0
//! Message bodies for protocol v0 (protocol §5, §7, §9).

use serde::{Deserialize, Serialize};

use crate::{DeviceId, ErrorCode};

/// Message type names (the envelope's `t` field).
pub mod types {
    pub const HELLO: &str = "hello";
    pub const HELLO_UPDATE: &str = "hello.update";
    pub const PING: &str = "ping";
    pub const PONG: &str = "pong";
    pub const OK: &str = "ok";
    pub const ERROR: &str = "error";
    pub const STREAM: &str = "stream";
    pub const EVENT_BATTERY: &str = "event.battery";
    pub const EVENT_DEVICE: &str = "event.device";
    pub const DEVICE_RING: &str = "device.ring";

    pub const PAIR_REQUEST: &str = "pair.request";
    pub const PAIR_ACCEPT: &str = "pair.accept";
    pub const PAIR_COMMIT: &str = "pair.commit";
    pub const PAIR_NONCE: &str = "pair.nonce";
    pub const PAIR_REVEAL: &str = "pair.reveal";
    pub const PAIR_CONFIRM: &str = "pair.confirm";
    pub const PAIR_REVOKE: &str = "pair.revoke";
}

/// What kind of device this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    Phone,
    Tablet,
    Desktop,
    Laptop,
    #[serde(other)]
    Unknown,
}

/// A device's power level (see PLAN.md §4.6). Desktops report `NotApplicable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PowerLevel {
    Basic,
    Assist,
    Elevated,
    #[serde(rename = "n/a")]
    NotApplicable,
    /// Unknown values (e.g. from a newer peer) are treated as Basic.
    #[serde(other)]
    Unknown,
}

impl PowerLevel {
    /// The level to use for capability decisions.
    pub fn effective(self) -> PowerLevel {
        match self {
            PowerLevel::Unknown => PowerLevel::Basic,
            other => other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub name: String,
    pub kind: DeviceKind,
    pub os: String,
    pub os_ver: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Optional ARGB seed color for Material You sync.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub proto: u32,
    pub min: u32,
    pub app: String,
    pub device: DeviceInfo,
    #[serde(default)]
    pub caps: Vec<String>,
    pub power: PowerLevel,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caps: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power: Option<PowerLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<DeviceInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ping {
    /// Sender's monotonic clock in milliseconds.
    pub ts: u64,
}

/// Echoes the `ts` of the matching ping.
pub type Pong = Ping;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Battery {
    pub level: u8,
    pub charging: bool,
    /// "ac", "usb" or "wireless".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugged: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ring {
    pub on: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: ErrorCode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub msg: Option<String>,
}

/// First frame of every non-control stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamHeader {
    pub svc: String,
    pub op: String,
    pub v: u32,
}

/// An empty body (`{}`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Empty {}

// ---- Pairing (protocol §9) ----

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairRequest {
    pub device: DeviceInfo,
    #[serde(with = "serde_bytes")]
    pub proof: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairAccept {
    pub device: DeviceInfo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairCommit {
    pub device: DeviceInfo,
    #[serde(with = "serde_bytes")]
    pub c: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairNonce {
    pub device: DeviceInfo,
    #[serde(rename = "nB", with = "serde_bytes")]
    pub n_b: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairReveal {
    #[serde(rename = "nA", with = "serde_bytes")]
    pub n_a: Vec<u8>,
}

/// A paired device identity as stored in trust stores and exchanged in tests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerIdentity {
    pub id: DeviceId,
    pub device: DeviceInfo,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Envelope;

    fn info() -> DeviceInfo {
        DeviceInfo {
            name: "Pixel 9".into(),
            kind: DeviceKind::Phone,
            os: "android".into(),
            os_ver: "16".into(),
            model: Some("Google Pixel 9".into()),
            accent: None,
        }
    }

    #[test]
    fn hello_round_trips() {
        let hello = Hello {
            proto: 0,
            min: 0,
            app: "0.0.1".into(),
            device: info(),
            caps: vec!["core.ping".into(), "device.battery".into()],
            power: PowerLevel::Elevated,
        };
        let env = Envelope::new(types::HELLO, &hello).unwrap();
        let back: Hello = Envelope::from_cbor(&env.to_cbor()).unwrap().expect_body(types::HELLO).unwrap();
        assert_eq!(back, hello);
    }

    #[test]
    fn unknown_enum_values_are_tolerated() {
        #[derive(Serialize)]
        struct Raw<'a> {
            kind: &'a str,
            power: &'a str,
        }
        #[derive(Deserialize)]
        struct Parsed {
            kind: DeviceKind,
            power: PowerLevel,
        }
        let env = Envelope::new("x", &Raw { kind: "watch", power: "root" }).unwrap();
        let parsed: Parsed = env.body().unwrap();
        assert_eq!(parsed.kind, DeviceKind::Unknown);
        assert_eq!(parsed.power.effective(), PowerLevel::Basic);
    }

    #[test]
    fn power_level_wire_names() {
        let env = Envelope::new("x", &PowerLevel::NotApplicable).unwrap();
        assert_eq!(env.b, Some(ciborium::Value::Text("n/a".into())));
    }
}
