// SPDX-License-Identifier: MPL-2.0
use nectarlink_protocol::{ErrorCode, ProtocolError, pairing::PairingUriError};

/// Which side of a connection an issue is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Local,
    Remote,
}

/// Errors returned by the core. Variants are chosen so the UI can act on
/// them; details are for logs and never contain personal content.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("device is not paired")]
    NotPaired,
    #[error("device is offline")]
    Offline,
    #[error("the operation was denied")]
    Denied,
    #[error("declined on this device")]
    Declined,
    #[error("the operation timed out")]
    Timeout,
    #[error("the {side:?} app is too old for this connection")]
    VersionTooOld { side: Side },
    #[error("not supported by the other device")]
    Unsupported,
    #[error("it no longer exists")]
    NotFound,
    #[error("no pairing in progress")]
    NotPairing,
    #[error("invalid pairing link: {0}")]
    InvalidPairingLink(#[from] PairingUriError),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("internal error: {0}")]
    Internal(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl From<ProtocolError> for Error {
    fn from(err: ProtocolError) -> Self {
        match err {
            ProtocolError::Remote { code, msg } => match code {
                ErrorCode::Denied => Error::Denied,
                ErrorCode::Unpaired => Error::NotPaired,
                ErrorCode::Unsupported => Error::Unsupported,
                ErrorCode::NotFound => Error::NotFound,
                ErrorCode::VersionTooOld => Error::VersionTooOld { side: Side::Local },
                other => Error::Protocol(format!("{other}: {msg}")),
            },
            ProtocolError::Io(e) => Error::Network(e.to_string()),
            ProtocolError::Closed => Error::Offline,
            other => Error::Protocol(other.to_string()),
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(err: rusqlite::Error) -> Self {
        Error::Storage(err.to_string())
    }
}

/// Converts any network-layer error into [`Error::Network`].
pub(crate) fn net(err: impl std::fmt::Display) -> Error {
    Error::Network(err.to_string())
}
