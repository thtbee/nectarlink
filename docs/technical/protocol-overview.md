# Protocol Overview

The NectarLink wire protocol (`nectarlink/0`) runs over a single mutually authenticated QUIC + TLS 1.3 connection per peer (`iroh` `1.3`). It multiplexes control messages, request/response RPCs, bulk file streams, real-time media streams, and unreliable low-latency datagrams without head-of-line blocking between independent features.

## Specification Index (`docs/protocol/`)

All 18 normative specification files live in [`docs/protocol/`](../protocol/v0.md):

| Specification | Capability IDs | Summary |
|---|---|---|
| [`v0.md`](../protocol/v0.md) | `msg.v0` (core) | Transport parameters, ALPNs (`nectarlink/0`, `nectarlink-pair/0`), LEB128 + CBOR envelope framing, `hello` / `hello.update` handshake, pairing flows, error codes |
| [`capabilities.md`](../protocol/capabilities.md) | All capabilities | Registry of capability namespaces, stream types, and version negotiation rules |
| [`actions.md`](../protocol/actions.md) | `actions.v0`, `commands.v0` | Find My Phone/PC ring, lock screen, shutdown/sleep/restart, open URL, user-defined command execution, and task completion notifications |
| [`calls.md`](../protocol/calls.md) | `calls.v0` | Live call state events (`ringing`, `active`, `ended`), answer/hangup/decline controls, and call log synchronization |
| [`clipboard.md`](../protocol/clipboard.md) | `clip.v0` | Text clipboard sync (`clip.set` up to 256 KiB), bulk image clipboard streams (`clip.image` up to 32 MiB with BLAKE3 verification), and loop suppression |
| [`contacts.md`](../protocol/contacts.md) | `contacts.v0` | Phone contact synchronization, avatar thumbnails, and phone-number lookup for SMS and call caller-ID resolution |
| [`deck.md`](../protocol/deck.md) | `deck.v0` | Programmable macro deck layout sync, live system telemetry (`deck.stats`), and button press execution |
| [`files.md`](../protocol/files.md) | `files.v0` | Multi-file and folder offers (`files.offer`), BLAKE3 digests, byte-offset `.part` resume (`files.accept { have }`), pause (`files.pause`), resume (`files.resume`), and cancel |
| [`media.md`](../protocol/media.md) | `media.v0` | Bidirectional Now Playing metadata (`media.state`), cover art thumbnails, playback position, and transport commands (`play`, `pause`, `next`, `prev`, `seek`, `volume`) |
| [`mirror.md`](../protocol/mirror.md) | `mirror.v0`, `mirror.pc.v0`, `mirror.apps.v0`, `mirror.audio.v0` | Phone screen mirroring, reverse PC-to-phone mirroring, per-app virtual display streaming, H.264/H.265/AV1 packet framing, touch/key injection, and 48 kHz audio streaming |
| [`notifications.md`](../protocol/notifications.md) | `msg.v0` | Active notification sync (`notif.post`, `notif.dismiss`), inline replies (`notif.reply`), action buttons (`notif.action`), and `MessagingStyle` conversation threads (`chat.threads`, `chat.messages`, `chat.send`) |
| [`photos.md`](../protocol/photos.md) | `photos.v0` | Recent camera roll index (`photos.list`), on-demand thumbnails (`photos.thumb`), full-resolution pulls (`photos.full`), and remote camera capture (`camera.request` / `camera.result`) |
| [`recorder.md`](../protocol/recorder.md) | `recorder.v0` | Remote voice/audio recording controls (`recorder.start`, `recorder.stop`, `recorder.status`) and automatic file handoff |
| [`remote.md`](../protocol/remote.md) | `remote.v0`, `remote.input.v0` | Media/presentation remote actions and low-latency trackpad/keyboard input synthesis over QUIC datagrams and control envelopes |
| [`sms.md`](../protocol/sms.md) | `sms.v0` | SMS/MMS thread list (`sms.threads`), message history (`sms.messages`), outbound sending (`sms.send`), SIM selection, and incoming message events (`sms.received`) |
| [`storage.md`](../protocol/storage.md) | `storage.v0` | Remote directory listing (`storage.list`), file download (`storage.read`), upload (`storage.write`), rename/move (`storage.rename`), mkdir (`storage.mkdir`), and delete (`storage.delete`) |
| [`toggles.md`](../protocol/toggles.md) | `toggles.v0` | Live phone status telemetry (battery, Wi-Fi, Bluetooth, DND, ringer mode, flashlight, hotspot, cellular signal) and quick-toggle actions |
| [`webcam.md`](../protocol/webcam.md) | `webcam.v0` | Phone-as-webcam video streaming into the Windows Media Foundation virtual camera (`nectarlink_vcam.dll`) with camera switching, resolution/FPS selection, and torch control |

## Wire Framing (`core/nectarlink-protocol/src/frame.rs`)

All framed streams (the session control stream and per-service RPC headers) use unsigned LEB128 length-prefixed CBOR (`RFC 8949`):

```text
+-------------------------------+-----------------------------------------------+
| length: uleb128 (1..4 bytes)  | payload: CBOR-encoded Envelope (length bytes) |
+-------------------------------+-----------------------------------------------+
```

- **Maximum Frame Size**: `MAX_FRAME_LEN = 1,048,576` bytes (1 MiB). Any frame advertising a length above `1 MiB` or exceeding 4 LEB128 header bytes fails immediately with `FrameError::FrameTooLarge` (`CLOSE_PROTOCOL_ERROR = 2`).
- **Envelope Schema (`src/messages.rs`)**:

```cbor
{
  "t":  text,          ; message type, e.g. "hello", "ping", "notif.post", "files.offer"
  ? "id": uint,        ; correlation ID assigned by sender for request/response RPCs
  ? "re": uint,        ; correlation ID of the request this envelope replies to
  ? "b":  any          ; message-specific CBOR map/struct body
}
```

Unknown fields in `b` are ignored by serde deserializers so minor revisions can add optional fields without breaking compatibility.

## QUIC Stream and Datagram Taxonomy

A `nectarlink/0` session uses four distinct QUIC transport primitives:

| Primitive | Direction | Framing | Typical Consumers |
|---|---|---|---|
| **Control Stream** | First bidirectional stream opened by dialer | Continuous stream of LEB128 + CBOR `Envelope` frames | `hello`, `hello.update`, `ping`/`pong`, `notif.*`, `sms.*`, `calls.*`, `media.*`, `toggles.*`, `actions.*`, `clip.set` |
| **Service / RPC Streams** | Short-lived bidirectional stream per operation | Starts with `StreamHeader { svc, op, v }` CBOR frame, followed by service-specific request/response frames | `photos.thumb`, `photos.full`, `storage.list`, `contacts.sync` |
| **Bulk Data Streams** | Bidirectional or unidirectional stream per transfer | Initial CBOR header (`files.offer`, `clip.image`, `storage.read`, `mirror.start`) followed by raw unframed bytes or media packets until EOF | `files.v0` payloads, `clip.image` bytes, `storage.read`/`write` streams, `camera.result` |
| **QUIC Datagrams** | Unreliable, unordered datagrams | 1-byte or 4-byte signature prefix + compact binary payload | Trackpad pointer deltas (`remote.input.v0`), fast scroll events, and `NLPA` PCM audio frames (`<= 960` bytes) |

## Control Stream Lifecycle and Capability Negotiation

1. **Handshake (`hello`)**: Immediately after opening the control stream, both peers send an `Envelope { t: "hello", id: 1, b: Hello }` containing:
   - `v`: Protocol version (`0`).
   - `id`: Sender's `DeviceId`.
   - `name`: User-configured device name.
   - `kind`: `DeviceKind` (`pc` or `phone`).
   - `caps`: List of currently enabled capability identifiers (for example `["msg.v0", "clip.v0", "files.v0", ...]`).
2. **Active Set Computation**: The session's active capabilities equal the set intersection of both peers' advertised `caps`, filtered through the local `CapabilityMatrix` for that peer.
3. **Live Capability Updates (`hello.update`)**: When a user toggles a feature in **Device Settings** or grants a runtime permission on Android, the device sends `Envelope { t: "hello.update", b: HelloUpdate { caps } }` on the existing control stream without dropping the QUIC session.
4. **Liveness (`ping` / `pong`)**: Peers exchange `ping { ts }` and `pong { ts }` envelopes every 15 s (`HEARTBEAT_INTERVAL`) to measure application-layer round-trip time (`rtt_ms`) and catch a peer that stopped answering. A link that dropped is QUIC's to notice: a keep-alive every 5 s and a 15 s idle timeout (`KEEP_ALIVE_INTERVAL`, `IDLE_TIMEOUT`, matching iroh's per-path limits), so an offline device shows within about 15 s. Shorter intervals kept phones from sleeping. mDNS asks the LAN who's there every 30 s, and every 700 ms only while a pairing screen shows nearby devices (`Node::set_discovery_fast`); answers go out at once either way. On PCs, mDNS runs on every LAN interface rather than only the one Windows or Linux picks for multicast, often a Hyper-V, WSL or VPN adapter; phones keep to their default network, so mobile data stays asleep. A pairing code lists the address of the default route first.

## Resumable File Transfers (`files.v0`)

Implemented in `core/nectarlink-core/src/transfer.rs` and specified in [`docs/protocol/files.md`](../protocol/files.md).

```text
Sender                                                  Receiver
------                                                  --------
Opens bidirectional stream
Sends StreamHeader { svc: "files", op: "offer", v: 0 }
Sends files.offer {
  id: 42,
  files: [{ idx: 0, name: "video.mp4", size: 104857600, sha256: "<64-hex BLAKE3>" }],
  total: 104857600
}
                                                        Checks <data_dir>/incoming/<peer>/42/0.part
                                                        Finds existing 31,457,280 bytes on disk
                  ◄─── files.accept { id: 42, have: [31457280] } ───
Seeks source file to offset 31,457,280
Streams remaining 73,400,320 raw bytes ──────────────►  Appends to 0.part
                                                        Computes full-file BLAKE3 digest
                                                        Verifies digest == offer.files[0].sha256
                                                        Atomically renames 0.part → Downloads/video.mp4
                  ◄─── files.done { id: 42, ok: true } ─────────────
```

### Integrity, Pause/Resume, and Offline Queueing

- **BLAKE3 Verification**: The `sha256` field in `FileSpec` carries a 64-hex-character BLAKE3 digest computed over the complete file. On resume (`have[idx] > 0`), the receiver first hashes the existing `.part` prefix with `blake3::Hasher` and continues hashing incoming bytes so the full file is verified end-to-end before moving into the user's Downloads folder.
- **Zero-Copy Streaming Buffer**: Bulk streaming uses a 256 KiB chunk buffer (`STREAM_CHUNK_SIZE = 262,144`) over a 16 MiB QUIC stream window, achieving `> 200 MB/s` over local loopback/LAN while updating progress callbacks at most every `50 ms` (`PROGRESS_THROTTLE`).
- **Pause, Resume, and Cancel**:
  - Either side can pause an active transfer by closing the QUIC stream with code `files::PAUSED` (`11`) or sending `files.pause`. Partial `.part` files remain intact on disk.
  - Resuming re-runs `files.offer` with the same `transfer_id`; the receiver reports `have[idx]` from the `.part` file sizes and transmission continues from the exact byte offset.
  - Canceling (`files.cancel` or stream reset `files::CANCELLED = 10`) deletes the staging directory `<data_dir>/incoming/<peer>/<id>/`.
- **Automatic Send Queue Fallback**: If `send_files` is called while the target peer is offline, or if the QUIC transport drops mid-transfer, the transfer is persisted into the SQLite `send_queue` table and automatically resumed when the peer reconnects.

## Real-Time Media Framing (`mirror.v0` & `webcam.v0`)

Implemented in `core/nectarlink-protocol/src/video.rs` and specified in [`docs/protocol/mirror.md`](../protocol/mirror.md) and [`docs/protocol/webcam.md`](../protocol/webcam.md).

### Video Stream Packet Format

After the initial CBOR negotiation (`mirror.start` / `mirror.ready` or `webcam.start` / `webcam.ready`), the bulk media stream switches to a compact 13-byte binary packet header:

```text
+--------------------+---------------+------------------------+-----------------------+
| length: u32 BE     | kind: u8      | time_us: u64 BE        | data: [u8; length-9]  |
| (4 bytes)          | (1 byte)      | (8 bytes)              | (codec payload)       |
+--------------------+---------------+------------------------+-----------------------+
```

- `length`: Total bytes following the 4-byte length field (`1 + 8 + data.len()`), capped at `MAX_VIDEO_PACKET = 8 MiB` (`8,388,608` bytes).
- `kind`:
  - `0` (`VideoKind::Config`): Codec configuration record (SPS/PPS for H.264, VPS/SPS/PPS for H.265, or AV1 sequence header). Cached by the receiver and re-fed upon decoder reset.
  - `1` (`VideoKind::Frame`): Inter-predicted P/B frame.
  - `2` (`VideoKind::Keyframe`): IDR / instantaneous decoder refresh frame.
- `time_us`: Capture presentation timestamp in microseconds.
- **Dynamic Bitrate and Keyframe Recovery**: If the decoder detects packet loss or surface recreation, the receiver sends `mirror.keyframe` or `webcam.control` on the control channel to request an immediate IDR frame without restarting the stream.

### Audio Streaming (`mirror.audio.v0`)

- **Datagram Fast Path (`NLPA`)**: For lowest latency on LAN/USB, 10 ms stereo 48 kHz 16-bit PCM frames (`1920` bytes split into two `<= 960`-byte chunks) are sent as QUIC datagrams prefixed with header tag `b"NLPA"`, sequence `u16`, and timestamp `u64`.
- **Stream Fallback**: When datagrams are unavailable or over constrained paths, audio packets flow over a unidirectional QUIC stream with a `StreamHeader { svc: "mirror.audio", op: "stream", v: 0 }` header followed by length-prefixed `pcm_s16le` or Opus frames.
