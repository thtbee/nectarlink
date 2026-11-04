// SPDX-License-Identifier: MPL-2.0
//! Length-prefixed framing: `unsigned LEB128 length || payload` (protocol §3).

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::ProtocolError;

/// Maximum payload length of a frame on control and RPC streams (1 MiB).
pub const MAX_FRAME_LEN: usize = 1 << 20;

/// The longest LEB128 prefix we accept: 4 bytes encodes up to 2^28 - 1.
const MAX_PREFIX_LEN: usize = 4;

fn encode_len(mut len: usize, out: &mut Vec<u8>) {
    loop {
        let byte = (len & 0x7f) as u8;
        len >>= 7;
        if len == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// Encodes one frame into a new buffer.
pub fn encode_frame(payload: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    if payload.len() > MAX_FRAME_LEN {
        return Err(ProtocolError::FrameTooLarge(payload.len()));
    }
    let mut out = Vec::with_capacity(payload.len() + MAX_PREFIX_LEN);
    encode_len(payload.len(), &mut out);
    out.extend_from_slice(payload);
    Ok(out)
}

/// Decodes one frame from the start of `buf`.
///
/// Returns `Ok(None)` if `buf` doesn't yet contain a complete frame, or
/// `Ok(Some((payload, consumed)))` with the number of bytes consumed.
pub fn decode_frame(buf: &[u8]) -> Result<Option<(&[u8], usize)>, ProtocolError> {
    let mut len = 0usize;
    for (i, &byte) in buf.iter().enumerate().take(MAX_PREFIX_LEN) {
        len |= ((byte & 0x7f) as usize) << (7 * i);
        if byte & 0x80 == 0 {
            if len > MAX_FRAME_LEN {
                return Err(ProtocolError::FrameTooLarge(len));
            }
            let start = i + 1;
            let end = start + len;
            return Ok((buf.len() >= end).then(|| (&buf[start..end], end)));
        }
    }
    if buf.len() >= MAX_PREFIX_LEN { Err(ProtocolError::BadLength) } else { Ok(None) }
}

/// Writes one frame to an async writer.
pub async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, payload: &[u8]) -> Result<(), ProtocolError> {
    if payload.len() > MAX_FRAME_LEN {
        return Err(ProtocolError::FrameTooLarge(payload.len()));
    }
    let mut prefix = [0u8; MAX_PREFIX_LEN];
    let mut prefix_len = 0;
    let mut rem = payload.len();
    loop {
        let byte = (rem & 0x7f) as u8;
        rem >>= 7;
        if rem == 0 {
            prefix[prefix_len] = byte;
            prefix_len += 1;
            break;
        }
        prefix[prefix_len] = byte | 0x80;
        prefix_len += 1;
    }
    writer.write_all(&prefix[..prefix_len]).await?;
    writer.write_all(payload).await?;
    Ok(())
}

/// Reads one frame from an async reader.
///
/// Returns `Ok(None)` on a clean end of stream before a new frame starts.
pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Option<Vec<u8>>, ProtocolError> {
    let mut len = 0usize;
    for i in 0..MAX_PREFIX_LEN {
        let byte = match reader.read_u8().await {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return if i == 0 { Ok(None) } else { Err(ProtocolError::Closed) };
            }
            Err(e) => return Err(e.into()),
        };
        len |= ((byte & 0x7f) as usize) << (7 * i);
        if byte & 0x80 == 0 {
            if len > MAX_FRAME_LEN {
                return Err(ProtocolError::FrameTooLarge(len));
            }
            let mut payload = vec![0u8; len];
            reader.read_exact(&mut payload).await.map_err(|e| {
                if e.kind() == std::io::ErrorKind::UnexpectedEof { ProtocolError::Closed } else { e.into() }
            })?;
            return Ok(Some(payload));
        }
    }
    Err(ProtocolError::BadLength)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_prefix_boundaries() {
        for (len, prefix) in [(0usize, 1usize), (127, 1), (128, 2), (16_383, 2), (16_384, 3)] {
            let frame = encode_frame(&vec![0xAB; len]).unwrap();
            assert_eq!(frame.len(), len + prefix, "len {len}");
            let (payload, used) = decode_frame(&frame).unwrap().unwrap();
            assert_eq!(payload.len(), len);
            assert_eq!(used, frame.len());
        }
    }

    #[test]
    fn rejects_oversized_frames() {
        assert!(matches!(encode_frame(&vec![0; MAX_FRAME_LEN + 1]), Err(ProtocolError::FrameTooLarge(_))));
        // Prefix announcing MAX_FRAME_LEN + 1 bytes.
        let mut prefix = Vec::new();
        encode_len(MAX_FRAME_LEN + 1, &mut prefix);
        assert!(matches!(decode_frame(&prefix), Err(ProtocolError::FrameTooLarge(_))));
    }

    #[test]
    fn partial_and_malformed_input() {
        let frame = encode_frame(b"hello").unwrap();
        assert!(decode_frame(&frame[..3]).unwrap().is_none());
        assert!(decode_frame(&[]).unwrap().is_none());
        assert!(matches!(decode_frame(&[0x80, 0x80, 0x80, 0x80]), Err(ProtocolError::BadLength)));
    }

    #[tokio::test]
    async fn async_round_trip() {
        let mut buf = Vec::new();
        write_frame(&mut buf, b"one").await.unwrap();
        write_frame(&mut buf, b"").await.unwrap();
        write_frame(&mut buf, &[9u8; 300]).await.unwrap();

        let mut reader = buf.as_slice();
        assert_eq!(read_frame(&mut reader).await.unwrap().unwrap(), b"one");
        assert_eq!(read_frame(&mut reader).await.unwrap().unwrap(), b"");
        assert_eq!(read_frame(&mut reader).await.unwrap().unwrap(), vec![9u8; 300]);
        assert!(read_frame(&mut reader).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn truncated_stream_is_an_error() {
        let frame = encode_frame(b"truncated").unwrap();
        let mut reader = &frame[..5];
        assert!(matches!(read_frame(&mut reader).await, Err(ProtocolError::Closed)));
    }

    #[tokio::test]
    async fn fuzz_frame_decode_arbitrary_bytes_10k() {
        let mut seed = 0x517C_C1B7_2722_0A95u64;
        let mut next_u64 = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };

        let mut buf = Vec::with_capacity(512);
        for i in 0..10_000usize {
            let len = (next_u64() as usize) % 256;
            buf.clear();
            for _ in 0..len {
                buf.push((next_u64() & 0xFF) as u8);
            }
            // Exercise boundary LEB128 prefixes (0, 127, 128, 16383, MAX_FRAME_LEN, MAX_FRAME_LEN + 1, 5-byte varints)
            match i % 7 {
                0 if !buf.is_empty() => buf[0] = (buf.len().saturating_sub(1).min(127)) as u8,
                1 if buf.len() >= 4 => {
                    buf[0] = 0x80;
                    buf[1] = 0x80;
                    buf[2] = 0x80;
                    buf[3] = 0x80;
                }
                2 => {
                    buf.clear();
                    encode_len(MAX_FRAME_LEN + (i % 16), &mut buf);
                }
                _ => {}
            }

            if let Ok(Some((payload, consumed))) = decode_frame(&buf) {
                assert!(payload.len() <= MAX_FRAME_LEN);
                assert!(consumed <= buf.len());
                assert!(consumed > payload.len());
                let reencoded = encode_frame(payload).unwrap();
                let (decoded_again, re_consumed) = decode_frame(&reencoded).unwrap().unwrap();
                assert_eq!(decoded_again, payload);
                assert_eq!(re_consumed, reencoded.len());
            }

            // Also fuzz async read_frame on small prefixes / arbitrary slices where declared length <= buf.len()
            // so we don't allocate 1 MiB 10,000 times.
            if buf.first().is_none_or(|&b| b < 0x80) {
                let mut slice = buf.as_slice();
                if let Ok(Some(async_payload)) = read_frame(&mut slice).await {
                    assert!(async_payload.len() < 128);
                }
            }
        }
    }
}
