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
    #[error("too large to send")]
    TooLarge,
    #[error("{0}")]
    QueueFull(String),
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
                ErrorCode::Denied | ErrorCode::Busy => Error::Denied,
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

/// Like [`net`], with every cause after it ("Service 'mdns' error: Cannot
/// bind to IPv4 or IPv6"): the top error alone often says too little (#466).
pub(crate) fn net_chain(err: &(dyn std::error::Error + 'static)) -> Error {
    let mut text = err.to_string();
    let mut cause = err.source();
    while let Some(next) = cause {
        let more = next.to_string();
        if !text.contains(&more) {
            text.push_str(": ");
            text.push_str(&more);
        }
        cause = next.source();
    }
    Error::Network(text)
}

#[cfg(test)]
mod chain_tests {
    use super::*;

    #[derive(Debug)]
    struct Outer(std::io::Error);
    impl std::fmt::Display for Outer {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("Service 'mdns' error")
        }
    }
    impl std::error::Error for Outer {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }

    #[test]
    fn a_network_error_keeps_its_causes() {
        let err = Outer(std::io::Error::other("Cannot bind to IPv4 or IPv6"));
        let Error::Network(text) = net_chain(&err) else { panic!("not a network error") };
        assert_eq!(text, "Service 'mdns' error: Cannot bind to IPv4 or IPv6");
    }
}
