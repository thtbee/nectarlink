# Deck service (protocol v0)

> Status: **draft.** Builds on [protocol v0](v0.md); breaking changes are
> allowed until v1. License: CC BY 4.0.

The phone acts as a macro pad for a paired PC, showing pages of large tiles
that trigger PC actions and reflect live PC state (playing/paused media,
speaker volume and mute, microphone mute).

The PC owns the Deck layout and action definitions: the phone receives only
page and tile display metadata (`id`, `label`, `icon`, `color`, `kind`) and
triggers actions by sending `deck.press { tile }` with the tile ID. The phone
never sends commands, file paths, or URLs.

## 1. Capability and permissions

| ID | Offered by | Meaning |
|---|---|---|
| `deck.actions` | desktop | Shares its Deck layout (`deck.layout`) and live state (`deck.state`) and accepts `deck.press` requests (`input.deck`) |
| `pc.audio` | desktop | Reports active audio output devices in `deck.state` and accepts `pc.audio.set` requests (`media.pc_audio`) |

- **Per-device toggles on the PC:**
  - `remote_input` (**off by default**): required for all `deck.press` requests (and accepted alongside `pc_actions` for `pc.audio.set`). When a paired phone presses a tile or calls `remote.check` while `remote_input` is off, the PC refuses with `DENIED` and emits a one-time consent prompt (`RemoteInputRequested`) for that phone.
  - `commands` (**off by default**): additionally required for tiles whose `kind` is `"run_command"`. Even when `remote_input` is on, pressing a `"run_command"` tile while `commands` is off for that phone is refused with `DENIED`.

## 2. Messages

Control messages run on the control stream; low-latency PC speaker audio frames
use QUIC datagrams (`NLPA`) with `audio.pcm` on the control stream as a
fallback for larger frames.

```
t = "deck.layout"   b = DeckLayout                              // PC → phone, no reply
t = "deck.state"    b = DeckState                               // PC → phone, no reply
t = "deck.press"    id = n  b = { tile: text }                  // phone → PC
t = "pc.audio.set"  id = n  b = { volume?: uint, muted?: bool } // phone → PC
t = "audio.pcm"     b = AudioPcmFrame                           // PC → phone, no reply
t = "ok"            re = n
```

### 2.1 `deck.layout`

Sent by a PC that offers `deck.actions` when a session starts and whenever the
user edits the Deck on the PC.

```
DeckLayout = {
  pages: [DeckPage],        // 1..=8 pages
}

DeckPage = {
  id:    text,              // 1..=64 ASCII alphanumeric / '_' / '-' / '.', unique per layout
  name:  text,              // 1..=64 UTF-8 bytes, no control chars
  tiles: [DeckTile],        // 0..=24 tiles
}

DeckTile = {
  id:    text,              // 1..=64 ASCII alphanumeric / '_' / '-' / '.', unique across the layout
  label: text,              // 1..=64 UTF-8 bytes, no control chars
  icon:  text,              // from the fixed icon set (§2.5)
  color: text,              // from the fixed color set (§2.5)
  kind:  text,              // action kind (§2.5); never contains paths, URLs, or commands
}
```

### 2.2 `deck.state`

Sent by a PC that offers `deck.actions` or `pc.audio` when a session starts and
whenever the PC's media playback state, speaker volume/mute, microphone mute
state, or active audio output devices change.

```
DeckState = {
  playing:         bool,                 // PC media is currently playing (`media_play_pause` tiles)
  volume:          uint,                 // PC master speaker volume, 0..=100 (`volume_*` tiles and phone audio slider)
  muted:           bool,                 // PC speaker is muted (`volume_*` tiles and phone audio bar)
  mic_muted?:      bool,                 // default PC microphone is muted (`mic_mute` tiles); omitted when no capture device exists
  output_devices?: [AudioOutputDevice],  // active PC playback endpoints (0..=32)
}

AudioOutputDevice = {
  id:         text,                      // 1..=256 UTF-8 bytes, no control chars
  name:       text,                      // 1..=128 UTF-8 bytes, friendly endpoint name
  is_default: bool,                      // true for the current default multimedia render endpoint
}
```

### 2.3 `deck.press`

Asks the PC to run the action bound to `tile`.

```
DeckPress = {
  tile: text,               // 1..=64 ASCII alphanumeric / '_' / '-' / '.'
}
```

Errors:
- `BAD_MESSAGE` when `tile` is empty, longer than 64 bytes, or contains disallowed characters.
- `UNSUPPORTED` when the PC does not offer `deck.actions`.
- `DENIED` when `remote_input` is off for the phone, or when the tile's `kind` is `"run_command"` and `commands` is off for the phone.
- `NOT_FOUND` when no tile with ID `tile` exists in the PC's current Deck layout.
- `INTERNAL` when the platform action fails on the PC.

### 2.4 `pc.audio.set` and `audio.pcm`

Sets the PC's master speaker volume (`0..=100`), mute state, and/or default
audio output, then pushes the updated `deck.state`. Sentinel volume values
`200` (`PHONE_SPEAKER_OFF`) and `201` (`PHONE_SPEAKER_ON`, with `muted`
omitted) stop or start streaming the PC's loopback audio to the phone
("Use phone as speaker").

```
PcAudioSet = {
  volume?: uint,            // 0..=100, or 200 (stop phone speaker) / 201 (start phone speaker)
  muted?:  bool,
  output?: text,            // an `id` from DeckState.output_devices: make it the default output
}

AudioPcmFrame = {
  sample_rate: uint,        // 8000..=96000 Hz
  channels:    uint,        // 1..=2
  pcm:         bytes,       // signed 16-bit little-endian interleaved PCM (<= 64 KiB, multiple of 2 * channels)
}
```

At least one of `volume`, `muted`, or `output` must be present. `output` follows
the `AudioOutputDevice.id` rules (1..=256 UTF-8 bytes, no control characters).
Windows sets it as the default for apps and system sounds (console and
multimedia roles), Linux with `wpctl set-default` or `pactl set-default-sink`.
A PC that doesn't know `output` sees an empty request and answers `BAD_MESSAGE`.

While phone speaker streaming is active, small PCM frames (`<= 960` bytes) are
sent as QUIC datagrams (`[b"NLPA": 4B][sample_rate: u32 LE][channels: u8][0: u8][pcm]`),
and larger frames fall back to `audio.pcm` on the control stream. The phone
accepts incoming `audio.pcm` and `NLPA` frames only when its `media` toggle is
enabled for the PC.

Errors for `pc.audio.set`:
- `BAD_MESSAGE` when `volume`, `muted`, and `output` are all omitted, `volume` is not in `0..=100 | 200 | 201`, or `output` is invalid.
- `UNSUPPORTED` when the PC does not offer `pc.audio` or `deck.actions`.
- `DENIED` when both `pc_actions` and `remote_input` are off for the phone (for volume/mute), or when `media` is off for `volume = 201`.
- `INTERNAL` when the audio endpoint update fails on the PC.

### 2.5 Action kinds, icons, and colors

- **Action kinds (`kind`):**
  - Media: `"media_play_pause"`, `"media_next"`, `"media_previous"`
  - Volume: `"volume_up"`, `"volume_down"`, `"volume_mute"`
  - Microphone: `"mic_mute"`
  - System & windows: `"lock_pc"`, `"show_desktop"`, `"switch_window"`, `"screenshot"`
  - Parameterized on the PC (parameters stay on the PC and are never sent over the wire):
    - `"shortcut"` (custom keyboard shortcut: key + modifiers)
    - `"open_url"` (`http` or `https` URL)
    - `"type_text"` (text snippet typed into the focused field)
    - `"launch_app"` (`.exe` or `.lnk` picked on the PC, launched via `ShellExecuteW` without arguments)
    - `"run_command"` (command written on the PC; requires the per-device `commands` toggle)
- **Icons (`icon`):** `"play"`, `"pause"`, `"skip_next"`, `"skip_previous"`, `"volume_up"`, `"volume_down"`, `"volume_off"`, `"mic"`, `"mic_off"`, `"lock"`, `"desktop"`, `"switch_window"`, `"screenshot"`, `"shortcut"`, `"globe"`, `"text"`, `"app"`, `"terminal"`, `"sparkle"`, `"star"`.
- **Colors (`color`):** `"amber"`, `"coral"`, `"red"`, `"teal"`, `"green"`, `"blue"`, `"violet"`, `"slate"`.
