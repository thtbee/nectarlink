// SPDX-License-Identifier: MPL-2.0
use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Error codes carried in `error` messages (protocol §8).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    Unsupported,
    Unpaired,
    VersionTooOld,
    FrameTooLarge,
    BadMessage,
    Denied,
    Busy,
    Internal,
    /// A code this implementation doesn't know yet (forward compatibility).
    Other(String),
}

impl ErrorCode {
    pub fn as_str(&self) -> &str {
        match self {
            ErrorCode::Unsupported => "UNSUPPORTED",
            ErrorCode::Unpaired => "UNPAIRED",
            ErrorCode::VersionTooOld => "VERSION_TOO_OLD",
            ErrorCode::FrameTooLarge => "FRAME_TOO_LARGE",
            ErrorCode::BadMessage => "BAD_MESSAGE",
            ErrorCode::Denied => "DENIED",
            ErrorCode::Busy => "BUSY",
            ErrorCode::Internal => "INTERNAL",
            ErrorCode::Other(code) => code,
        }
    }

    /// Numeric QUIC application close code for this error.
    pub fn close_code(&self) -> u32 {
        match self {
            ErrorCode::Unsupported => 1,
            ErrorCode::Unpaired => 2,
            ErrorCode::VersionTooOld => 3,
            ErrorCode::FrameTooLarge => 4,
            ErrorCode::BadMessage => 5,
            ErrorCode::Denied => 6,
            ErrorCode::Busy => 7,
            ErrorCode::Internal | ErrorCode::Other(_) => 8,
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ErrorCode {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "UNSUPPORTED" => ErrorCode::Unsupported,
            "UNPAIRED" => ErrorCode::Unpaired,
            "VERSION_TOO_OLD" => ErrorCode::VersionTooOld,
            "FRAME_TOO_LARGE" => ErrorCode::FrameTooLarge,
            "BAD_MESSAGE" => ErrorCode::BadMessage,
            "DENIED" => ErrorCode::Denied,
            "BUSY" => ErrorCode::Busy,
            "INTERNAL" => ErrorCode::Internal,
            other => ErrorCode::Other(other.to_owned()),
        })
    }
}

impl Serialize for ErrorCode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ErrorCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(s.parse().expect("infallible"))
    }
}

/// Errors produced while encoding, decoding or framing messages.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("frame of {0} bytes exceeds the {max} byte limit", max = crate::MAX_FRAME_LEN)]
    FrameTooLarge(usize),
    #[error("malformed frame length prefix")]
    BadLength,
    #[error("malformed message: {0}")]
    BadMessage(String),
    #[error("unexpected message type {got:?}, expected {expected:?}")]
    UnexpectedType { expected: &'static str, got: String },
    #[error("peer returned error {code}: {msg}")]
    Remote { code: ErrorCode, msg: String },
    #[error("stream closed")]
    Closed,
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
}

impl ProtocolError {
    /// The error code to report to the peer for this local failure.
    pub fn code(&self) -> ErrorCode {
        match self {
            ProtocolError::FrameTooLarge(_) => ErrorCode::FrameTooLarge,
            ProtocolError::BadLength
            | ProtocolError::BadMessage(_)
            | ProtocolError::UnexpectedType { .. } => ErrorCode::BadMessage,
            ProtocolError::Remote { code, .. } => code.clone(),
            ProtocolError::Closed | ProtocolError::Io(_) => ErrorCode::Internal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip_and_tolerate_unknown_values() {
        for code in ["UNSUPPORTED", "UNPAIRED", "DENIED", "VERSION_TOO_OLD"] {
            assert_eq!(code.parse::<ErrorCode>().unwrap().as_str(), code);
        }
        assert_eq!("FUTURE_CODE".parse::<ErrorCode>().unwrap(), ErrorCode::Other("FUTURE_CODE".into()));
    }
}
