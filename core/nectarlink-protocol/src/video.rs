// SPDX-License-Identifier: MPL-2.0
//! Video packets on a mirroring stream (docs/protocol/mirror.md §3):
//! `u32 length (big-endian) || u8 kind || u64 time (µs, big-endian) || data`,
//! where `length` counts kind, time and data.

use tokio::io::{AsyncRead, AsyncReadExt};

use crate::ProtocolError;

/// The largest packet accepted (a keyframe at a high bitrate fits easily).
pub const MAX_VIDEO_PACKET: usize = 8 << 20;
const HEADER: usize = 1 + 8;

/// What a video packet holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PacketKind {
    /// The stream's format (a CBOR `MirrorConfig`); comes first, and again
    /// whenever it changes (the phone turned).
    Config = 0,
    /// Encoded video that depends on earlier packets.
    Frame = 1,
    /// Encoded video that starts fresh (with its parameter sets).
    Keyframe = 2,
}

impl PacketKind {
    fn from_u8(kind: u8) -> Option<PacketKind> {
        match kind {
            0 => Some(PacketKind::Config),
            1 => Some(PacketKind::Frame),
            2 => Some(PacketKind::Keyframe),
            _ => None,
        }
    }
}

/// A packet read from a stream.
#[derive(Clone, PartialEq, Eq)]
pub struct VideoPacket {
    pub kind: PacketKind,
    /// Capture time on the sender, in microseconds (only differences mean
    /// anything).
    pub time_us: u64,
    pub data: Vec<u8>,
}

impl std::fmt::Debug for VideoPacket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoPacket")
            .field("kind", &self.kind)
            .field("time_us", &self.time_us)
            .field("bytes", &self.data.len())
            .finish()
    }
}

/// Encodes a packet's header; the data follows it on the stream.
pub fn video_packet_header(
    kind: PacketKind,
    time_us: u64,
    data_len: usize,
) -> Result<[u8; 4 + HEADER], ProtocolError> {
    let len = HEADER + data_len;
    if len > MAX_VIDEO_PACKET {
        return Err(ProtocolError::FrameTooLarge(len));
    }
    let mut header = [0u8; 4 + HEADER];
    header[..4].copy_from_slice(&(len as u32).to_be_bytes());
    header[4] = kind as u8;
    header[5..].copy_from_slice(&time_us.to_be_bytes());
    Ok(header)
}

/// Reads one packet; `None` at the end of the stream.
pub async fn read_video_packet<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> Result<Option<VideoPacket>, ProtocolError> {
    let mut len = [0u8; 4];
    match reader.read_exact(&mut len).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let len = u32::from_be_bytes(len) as usize;
    if !(HEADER..=MAX_VIDEO_PACKET).contains(&len) {
        return Err(ProtocolError::FrameTooLarge(len));
    }
    let mut header = [0u8; HEADER];
    reader.read_exact(&mut header).await?;
    let kind = PacketKind::from_u8(header[0]).ok_or(ProtocolError::BadLength)?;
    let time_us = u64::from_be_bytes(header[1..].try_into().expect("8 bytes"));
    let mut data = vec![0u8; len - HEADER];
    reader.read_exact(&mut data).await?;
    Ok(Some(VideoPacket { kind, time_us, data }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn packets_round_trip() {
        let mut stream = Vec::new();
        for (kind, time, data) in
            [(PacketKind::Config, 0, b"cfg".to_vec()), (PacketKind::Keyframe, 16_667, vec![7; 70_000])]
        {
            stream.extend_from_slice(&video_packet_header(kind, time, data.len()).unwrap());
            stream.extend_from_slice(&data);
        }
        let mut reader = stream.as_slice();
        let first = read_video_packet(&mut reader).await.unwrap().unwrap();
        assert_eq!(
            (first.kind, first.time_us, first.data.as_slice()),
            (PacketKind::Config, 0, b"cfg".as_slice())
        );
        let second = read_video_packet(&mut reader).await.unwrap().unwrap();
        assert_eq!((second.kind, second.time_us, second.data.len()), (PacketKind::Keyframe, 16_667, 70_000));
        assert!(read_video_packet(&mut reader).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn bad_packets_are_rejected() {
        assert!(video_packet_header(PacketKind::Frame, 0, MAX_VIDEO_PACKET).is_err());
        // An unknown kind.
        let mut bytes = video_packet_header(PacketKind::Frame, 0, 1).unwrap().to_vec();
        bytes[4] = 9;
        bytes.push(0);
        assert!(read_video_packet(&mut bytes.as_slice()).await.is_err());
        // A length too short for the header.
        assert!(read_video_packet(&mut [0u8, 0, 0, 3, 1, 2, 3].as_slice()).await.is_err());
    }
}
