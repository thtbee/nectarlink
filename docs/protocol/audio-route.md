# Managed audio routing

AudioLink routes and mixes audio between paired devices. A device advertises
the [capabilities](capabilities.md) for the roles it can perform.

| Capability | Meaning |
|---|---|
| `audio.route.source.v1` | Captures and sends audio for a route |
| `audio.route.sink.v1` | Receives and mixes route audio for playback |
| `audio.route.controller.v1` | Configures routes and receives status and levels |
| `mic.status.v1` | Reports virtual microphone readiness and whether it can open setup |

## Configuration and feedback

The controller sends an `audio.route.set` request with a complete snapshot
for the recipient. The recipient replies with `audio.route.status`, carrying
the request's ID in `re`, or with a protocol error. It can also send
`audio.route.status` whenever the effective route changes. A sink can send
`audio.route.levels` as a one-way event. The controller correlates status and
levels using the route's `generation`.

```
AudioRouteSet = {
  generation: uint64,
  role: Role,
  target_device_id?: text,
  output_id?: text,
  master_gain: float32,
  master_muted: bool,
  silence_local_speaker: bool,
  include_phone_audio: bool,
  latency_preset: "lowest" | "balanced" | "stable",
  playback_policy: "pause" | "mute" | "lower" | "off",
  selective_resume: bool,
  phone_playback_active: bool,
  lanes: [{ source_id: text, gain: float32, muted: bool }],
}

AudioRouteStatus = {
  generation: uint64,
  role: Role,
  active: bool,
  local_paused: bool,
  speaker_muted_by_route: bool,
  output_id?: text,
  reason?: text,
}

AudioRouteLevels = {
  generation: uint64,
  levels: [{ source_id: text, peak: float32, active: bool }],
}

Role = "idle" | "source" | "sink" | "phone_playback"
```

`idle` stops routing for the recipient. `source` sends audio to the selected
target, `sink` mixes its enrolled lanes, and `phone_playback` plays the route
on the phone. `output_id` selects an output device. `master_gain` and each
lane's `gain` are finite linear multipliers from 0 to 2. Each `peak` is finite
and ranges from 0 to 1. The `lanes` and `levels` lists each carry up to 16
entries with unique source IDs.

When phone playback is active, the playback policy can pause, mute, or lower
PC audio. `off` leaves PC playback alone. `selective_resume` limits resume to
the playback the route paused. `silence_local_speaker` controls whether the
source also plays audio locally. `include_phone_audio` enables phone audio in
the route. The latency preset selects how much buffering the receiver uses.

Source and target IDs have at most 64 UTF-8 bytes. Output IDs have at most 256
bytes. Reasons have at most 256 bytes. Present strings contain non-whitespace
text and no control characters.

## Managed PCM

A managed PCM frame carries `generation: uint64`, `seq: uint32`,
`sample_rate: uint32`, `channels: uint8`, `frame_count: uint16`,
`source_id: text`, and `pcm: bytes`. The sequence wraps per source. Receivers
accept frames for the current generation from the paired source enrolled in
that lane.

The sender uses a QUIC datagram when the frame fits the connection's datagram
limit. Each datagram has this layout. All multi-byte integers use little-endian
byte order.

| Offset | Width | Field |
|---|---|---|
| 0 | 4 | ASCII `NLRP` |
| 4 | 8 | Generation |
| 12 | 4 | Sequence |
| 16 | 4 | Sample rate in Hz |
| 20 | 1 | Channels |
| 21 | 2 | Frame count |
| 23 | 1 | Source ID byte length |
| 24 | Source ID length | Source ID in UTF-8 |
| After source ID | `frame_count * channels * 2` | Interleaved signed PCM16 little-endian samples |

Valid frames have a rate from 8,000 to 96,000 Hz, one or two channels, a
nonzero frame count, and a source ID of 1 to 64 bytes. The PCM length must
match the frame count and channel count exactly.

When datagrams are unavailable, the sender uses an `audio.route.pcm` envelope
without a request ID. Its CBOR body uses the fields above and encodes `pcm`
as a byte string. Legacy peers continue to use `NLPA`, `audio.pcm`,
`pc.audio.set`, `mic.start`, `mic.stop`, `mic/pcm`, and `mirror/audio`.

## Virtual microphone

The PC reports virtual microphone readiness with a `mic.status` event to a
paired peer. When `can_open_setup` is true, the peer may send an empty
`mic.setup` request. The PC answers with `mic.setup` and `re` set to the
request ID. Its response has `ok: bool` and an optional `message: text`.
`ok` reports whether the PC opened its local setup UI.

```
MicStatus = {
  ready: bool,
  active: bool,
  is_default_input: bool,
  device_name?: text,
  setup_reason?: text,
  can_open_setup: bool,
}
```

The optional device name has at most 128 UTF-8 bytes. The optional setup
reason and response message each have at most 256 bytes. Present strings
contain non-whitespace text and no control characters.
