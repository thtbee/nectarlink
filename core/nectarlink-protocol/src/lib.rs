// SPDX-License-Identifier: MPL-2.0
//! The Nectarlink wire protocol (version 0).
//!
//! This crate is pure protocol logic with no networking: device IDs, message
//! types, the CBOR envelope, length-prefixed framing and the pairing
//! primitives. It is specified in `docs/protocol/v0.md`.

mod envelope;
mod error;
mod frame;
mod id;
pub mod messages;
pub mod pairing;
mod video;

pub use envelope::Envelope;
pub use error::{ErrorCode, ProtocolError};
pub use frame::{MAX_FRAME_LEN, decode_frame, encode_frame, read_frame, write_frame};
pub use id::{DeviceId, ParseDeviceIdError};
pub use video::{MAX_VIDEO_PACKET, PacketKind, VideoPacket, read_video_packet, video_packet_header};

/// ALPN for normal sessions between paired devices.
pub const ALPN_SESSION: &[u8] = b"nectarlink/0";
/// ALPN for the pairing ceremony.
pub const ALPN_PAIR: &[u8] = b"nectarlink-pair/0";

/// Protocol version spoken by this implementation.
pub const PROTOCOL_VERSION: u32 = 0;
/// Oldest protocol version this implementation still accepts.
pub const MIN_PROTOCOL_VERSION: u32 = 0;

/// Negotiates the protocol version for a session.
///
/// Returns the version both sides will speak, or `None` if either side's
/// minimum is above what the other speaks.
pub fn negotiate_version(local: (u32, u32), remote: (u32, u32)) -> Option<u32> {
    let (local_proto, local_min) = local;
    let (remote_proto, remote_min) = remote;
    let version = local_proto.min(remote_proto);
    (version >= local_min && version >= remote_min).then_some(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiates_lowest_common_version() {
        assert_eq!(negotiate_version((3, 1), (2, 0)), Some(2));
        assert_eq!(negotiate_version((0, 0), (0, 0)), Some(0));
    }

    #[test]
    fn rejects_versions_below_minimum() {
        // Local speaks 5 but needs at least 4; remote only speaks 3.
        assert_eq!(negotiate_version((5, 4), (3, 0)), None);
        // Remote requires at least 2; local only speaks 1.
        assert_eq!(negotiate_version((1, 0), (4, 2)), None);
    }
}
