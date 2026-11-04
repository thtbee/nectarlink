# Phone as webcam (v1)

Uses a paired phone's camera as a webcam on a PC. Control messages travel on
the session's control stream ([protocol v0](v0.md) §4); encoded video frames
travel on a dedicated bidirectional QUIC stream (`{ svc: "webcam", op: "video", v: 1 }`).

## 1. Capabilities and toggle

| Capability | Offered by | Meaning |
|---|---|---|
| `camera.stream` | Phone | Streams a camera for webcam use (when `CAMERA` permission is granted) |
| `camera.virtual` | PC | Receives and decodes a phone's webcam stream |
| `camera.virtual.hevc` | PC | Also accepts H.265/HEVC webcam video; absence means H.264 only |
| `addon.vcam` | PC | The Windows virtual camera COM add-on is registered in `HKLM`, so apps (Teams, Meet, Zoom, Windows Camera) see `"NectarLink - <Phone name>"` |

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

Asks the phone's hardware video encoder for an immediate keyframe
(`PARAMETER_KEY_REQUEST_SYNC_FRAME`), sent when the PC decoder starts mid-stream
or after a dropped packet.

## 3. Video stream

The phone opens a bidirectional stream to the PC and writes the stream header:

```
{ t: "stream", svc: "webcam", op: "video", v: 1 }
```

Followed by a sequence of framed video packets (`core/nectarlink-protocol/src/video.rs`):

```
[length: u32 big-endian][kind: u8][timestamp_us: u64 big-endian][payload: length bytes]
```

| `kind` | Name | Payload |
|---|---|---|
| `0` | `Config` | CBOR `WebcamConfig` (`{ codec: "h264", width: uint, height: uint, camera: "back" \| "front", fps: uint, max_height: uint }`), sent at the start of the stream and whenever resolution or camera changes. `max_height` is the highest resolution the camera in use can stream, by its short side: `2160` when it can do 4K, else `1080`. The PC offers 4K only when it is `2160`; when it is absent (phones before 0.1.5) it counts as `2160`. |
| `1` | `Frame` | Annex B H.264 non-IDR access unit |
| `2` | `Keyframe` | Annex B H.264 IDR access unit (preceded by SPS and PPS NAL units) |

The phone's sender bounds its outgoing queue to a few frames (`QUEUE_FRAMES = 4`):
if the network falls behind, queued frames are dropped and the encoder is asked
for a fresh keyframe rather than building up latency.

### Optional HEVC negotiation

An updated phone prefers `h265` only when the connected receiver advertises
`camera.virtual.hevc` and a hardware HEVC Main-profile surface encoder supports
the requested dimensions and frame rate. Otherwise it uses `h264`. Windows
advertises HEVC only when its installed Media Foundation decoder can initialize
with a GPU device manager. The phone falls back to H.264 if the HEVC encoder
cannot start, or reconfigures the existing stream to H.264 after a HEVC drain error.

For HEVC, `Config.codec` is `h265`, `Frame` and `Keyframe` contain Annex B HEVC
access units, and keyframes carry VPS, SPS and PPS. Each access unit starts with
an HEVC AUD (NAL type 35), instead of the H.264 AUD (type 9). The receiver switches
decoders when a Config changes codec. HEVC configs are rejected by receivers that
do not advertise the HEVC capability, including later Config packets in a stream.
Older phone and desktop releases continue using H.264 without changing version 1
framing or the existing start-request fields.

The initial HEVC implementation retains the same resolution, frame rate and target
bitrate as H.264. It establishes codec compatibility; it does not itself claim a
bandwidth or latency reduction. A lower HEVC target bitrate needs separate quality
and latency measurements.
