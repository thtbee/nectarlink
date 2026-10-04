// SPDX-License-Identifier: MPL-2.0
use std::{fmt, str::FromStr, sync::LazyLock};

use data_encoding::{BitOrder, Encoding, Specification};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// z-base-32 (human-oriented base32), as used for Device IDs in the UI and in
/// pairing URIs.
static ZBASE32: LazyLock<Encoding> = LazyLock::new(|| {
    let mut spec = Specification::new();
    spec.symbols.push_str("ybndrfg8ejkmcpqxot1uwisza345h769");
    spec.bit_order = BitOrder::MostSignificantFirst;
    spec.encoding().expect("valid z-base-32 specification")
});

/// A device's identity: its Ed25519 public key (32 bytes).
///
/// Equal to the iroh endpoint ID of the device. Displayed in lowercase
/// z-base-32 (52 characters).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeviceId(pub [u8; 32]);

impl DeviceId {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// A short form for logs and debugging, e.g. `ybndrfg8`.
    pub fn short(&self) -> String {
        self.to_string()[..8].to_owned()
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&ZBASE32.encode(&self.0))
    }
}

impl fmt::Debug for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DeviceId({})", self.short())
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("invalid device ID")]
pub struct ParseDeviceIdError;

impl FromStr for DeviceId {
    type Err = ParseDeviceIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes =
            ZBASE32.decode(s.trim().to_ascii_lowercase().as_bytes()).map_err(|_| ParseDeviceIdError)?;
        let bytes: [u8; 32] = bytes.try_into().map_err(|_| ParseDeviceIdError)?;
        Ok(DeviceId(bytes))
    }
}

impl Serialize for DeviceId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serde_bytes::Bytes::new(&self.0).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for DeviceId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let bytes = serde_bytes::ByteBuf::deserialize(deserializer)?;
        let bytes: [u8; 32] = bytes
            .into_vec()
            .try_into()
            .map_err(|_| serde::de::Error::custom("device ID must be 32 bytes"))?;
        Ok(DeviceId(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_round_trips() {
        let id = DeviceId([7u8; 32]);
        let text = id.to_string();
        assert_eq!(text.len(), 52);
        assert_eq!(text.parse::<DeviceId>().unwrap(), id);
        assert_eq!(text.to_uppercase().parse::<DeviceId>().unwrap(), id);
    }

    #[test]
    fn zbase32_matches_reference_vector() {
        // From the z-base-32 specification: 0xF0 0xBF 0xC7 -> "6n9hq".
        assert_eq!(ZBASE32.encode(&[0xF0, 0xBF, 0xC7]), "6n9hq");
    }

    #[test]
    fn rejects_bad_input() {
        assert!("not-an-id".parse::<DeviceId>().is_err());
        assert!(ZBASE32.encode(&[1u8; 16]).parse::<DeviceId>().is_err());
    }
}
