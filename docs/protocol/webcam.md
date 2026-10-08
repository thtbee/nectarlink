# Phone as webcam (v1)

Uses a paired phone's camera as a webcam on a PC. Control messages travel on
the session's control stream ([protocol v0](v0.md) §4); encoded video frames
travel on a dedicated bidirectional QUIC stream (`{ svc: "webcam", op: "video", v: 1 }`).

## 1. Capabilities and toggle

| Capability | Offered by | Meaning |
|---|---|---|
| `camera.stream` | Phone | Streams a camera for webcam use (when `CAMERA` permission is granted) |
| `camera.virtual` | PC | Receives and decodes a phone's webcam stream |
| `addon.vcam` | PC | The Windows virtual camera COM add-on is registered in `HKLM`, so apps (Teams, Meet, Zoom, Windows Camera) see `"Nectarlink - <Phone name>"` |

Both devices have a `webcam` per-device toggle (on by default). When turned off
for a peer, `webcam.start` is refused with `denied` (`4`), incoming `webcam`
streams are reset with code `12`, and any active webcam stream with that peer
is stopped immediately.

A phone **never** starts streaming its camera silently: streaming starts only
when the user starts it on the phone's Webcam screen or explicitly accepts a
PC's request, and an ongoing `camera` foreground service notification with a
**Stop** button is shown the entire time.

## 2. Starting and stopping

### `webcam.start` → `webcam.ok` (PC → phone, request)

Asks the phone to stream its camera (or updates camera/resolution while
already streaming):

```
{
  t: "webcam.start",
  id: uint,
  camera: "back" | "front",   // default: "back"
  width: uint,                // 320..=3840 (e.g. 1280, 1920, 3840)
  height: uint,               // 240..=2160 (e.g. 720, 1080, 2160)
  fps: uint,                  // 1..=60 (default: 30)
  bitrate: uint               // 100_000..=50_000_000 bits/s (default: 6_000_000)
}
```

The phone answers `{ t: "webcam.ok", re: id }` once it has shown the user the
request (or applied the updated options to an already-active stream), or an
`error` (`denied` when the `webcam` toggle is off on either side, `unsupported`
when `camera.stream` is not offered, `bad_message` when fields are out of
bounds).

The user may also start the webcam directly from the phone's Webcam screen
without a prior `webcam.start` from the PC; in that case the phone opens the
`webcam/video` stream directly to the selected PC.

### `webcam.stop` (either direction, notification)

```
{ t: "webcam.stop" }
```

Stops the webcam stream. Either side may also stop by closing or resetting the
`webcam/video` stream with application error code `12` (`webcam::STOPPED`).

### `webcam.keyframe` (PC → phone, notification)

```
{ t: "webcam.keyframe" }
```

Asks the phone's hardware H.264 encoder for an immediate IDR keyframe
(`PARAMETER_KEY_REQUEST_SYNC_FRAME`), sent when the PC decoder starts mid-stream
or after a dropped packet.

## 3. Video stream

The phone opens a bidirectional stream to the PC and writes the stream header:

```
{ t: "stream", svc: "webcam", op: "video", v: 1 }
```

Followed by a sequence of framed video packets (`core/nectarlink-protocol/src/video.rs`):

```
[kind: u8][timestamp_us: u64 big-endian][length: u32 big-endian][payload: length bytes]
```

| `kind` | Name | Payload |
|---|---|---|
| `0` | `Config` | CBOR `WebcamConfig` (`{ codec: "h264", width: uint, height: uint, camera: "back" \| "front", fps: uint }`), sent at the start of the stream and whenever resolution or camera changes |
| `1` | `Frame` | Annex B H.264 non-IDR access unit |
| `2` | `Keyframe` | Annex B H.264 IDR access unit (preceded by SPS and PPS NAL units) |

The phone's sender bounds its outgoing queue to a few frames (`QUEUE_FRAMES = 4`):
if the network falls behind, queued frames are dropped and the encoder is asked
for a fresh keyframe rather than building up latency.
