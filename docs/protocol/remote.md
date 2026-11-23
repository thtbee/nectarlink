# Remote input service (protocol v0)

> Status: **draft.** Builds on [protocol v0](v0.md); breaking changes are
> allowed until v1. License: CC BY 4.0.

A phone acts as a touchpad, keyboard, presentation remote, and game
controller for a paired PC.

## 1. Capability and permission

| ID | Offered by | Meaning |
|---|---|---|
| `input.inject` | desktop | Accepts pointer, keyboard, and presentation input from a paired phone |
| `input.precision` | desktop | Accepts precision motion and desktop gestures and supplies a display map (§3.2). Offered with `input.inject` |
| `input.gamepad` | desktop | Also plugs in a game controller for a phone's `pad` input (§3.1); offered with `input.inject` |

Input is governed by the `remote_input` per-device toggle on the PC, which is
**off by default**. While `remote_input` is off for a phone:

- The PC refuses every input from that phone (`DENIED` on control requests;
  datagrams are dropped without injecting anything).
- The first time a phone asks while the toggle is off, the PC prompts the user
  once to allow or decline remote input for that phone. The user can also
  change the toggle at any time in the phone's device settings on the PC.

## 2. Transport

### 2.1 QUIC datagrams (channel tag `0x01`)

Continuous, loss-tolerant motion travels in QUIC datagrams (`RFC 9221`,
[protocol v0 §4](v0.md)) prefixed with the one-byte channel tag `0x01`,
followed by at most 256 bytes of CBOR-encoded `Input`:

```
+--------+-----------------------+
|  0x01  |  CBOR Input (≤ 256 B) |
+--------+-----------------------+
```

If QUIC datagrams are unavailable on a connection, the sender opens a
bidirectional stream with header `{ svc: "remote", op: "motion", v: 1 }` and
sends length-prefixed `remote.input` envelopes with CBOR `Input` bodies.
Pointer motion without a click uses datagrams or this fallback stream.

Datagram-eligible `Input` variants:

```
{ type: "move",          dx: float32, dy: float32 }
{ type: "move_precise",  dx: float32, dy: float32 }
{ type: "move_absolute", x: float32, y: float32, seq: uint32 }
{ type: "scroll",        dx: float32, dy: float32 }
{ type: "zoom",          delta: float32 }
{ type: "laser",         on: bool, x?: float32, y?: float32 }
{ type: "pad",           pad: Pad }
```

### 2.2 Control stream requests

Discrete actions that must arrive, and permission checks when opening the
remote screen, use control-stream requests:

```
t = "remote.check"  id = n  b = { presence?: bool }
t = "ok"            re = n

t = "remote.info"   id = n  b = {}
t = "remote.info"   re = n  b = RemoteInfo

t = "remote.input"  id = n  b = Input
t = "ok"            re = n
```

**`remote.check`** verifies whether the PC currently accepts remote input from
the sender. If `remote_input` is off, the PC replies with `DENIED` and shows
its one-time prompt if it hasn't already asked for this phone. A precision
remote can report `presence: true` while open or `presence: false` when it
closes. Omission keeps the legacy permission check. Older peers can send an
empty body or omit it entirely.

**`remote.info`** reports the gesture profile and display map (§3.2). It
requires `input.precision` and the same `remote_input` permission as input.

**`remote.input`** on the control stream executes actions that can take a
reliable request. `move`, `move_precise`, and `move_absolute` without a click
are rejected with `BAD_MESSAGE`. The receiver also uses `BAD_MESSAGE` for
out-of-bounds values or unknown keys. It returns `UNSUPPORTED` when it lacks
the required capability and `DENIED` when permission is off or the sender is
rate-limited.

## 3. Input payloads

```
Input =
  | { type: "move",   dx: float32, dy: float32 }
  | { type: "button", button: Button, action: ButtonAction }
  | { type: "scroll", dx: float32, dy: float32 }
  | { type: "text",   text: text }
  | { type: "key",    key: text, mods?: [KeyMod] }
  | { type: "slide",  action: SlideAction }
  | { type: "laser",  on: bool, x?: float32, y?: float32 }
  | { type: "pad",    pad: Pad }
  | { type: "pad_off" }
  | { type: "move_precise", dx: float32, dy: float32 }
  | { type: "move_absolute", x: float32, y: float32, seq: uint32, click?: Button }
  | { type: "zoom", delta: float32 }
  | { type: "modified_button", button: Button, action: ButtonAction, mods?: [KeyMod] }
  | { type: "gesture", action: GestureAction }

Button       = "left" | "right" | "middle" | "back" | "forward"
ButtonAction = "down" | "up" | "click"
KeyMod       = "ctrl" | "alt" | "shift" | "win"
SlideAction  = "next" | "previous" | "start" | "stop" | "black"
```

- **Pointer motion (`move`):** relative displacement in logical pixels (`dx`
  right, `dy` down), each finite and within `[-4000, 4000]`. Dragging holds a
  button with `button` `down`, sends `move` datagrams, and finishes with
  `button` `up`.
- **Buttons (`button`):** mouse button `down`, `up`, or `click` (press and
  release). `back` and `forward` require `input.precision`. If a session ends
  or `remote_input` is turned off while a button is held down, the PC
  releases the held button.
- **Scroll (`scroll`):** horizontal (`dx`, positive right) and vertical (`dy`,
  positive down on Windows and Linux) in wheel notches
  (fractions allowed for smooth scrolling), each finite and within
  `[-200, 200]`.
- **Text (`text`):** 1 to 256 UTF-8 bytes of printable Unicode text (no control
  characters). Injected as Unicode characters (`KEYEVENTF_UNICODE` on Windows)
  so non-Latin scripts and emojis work regardless of keyboard layout.
- **Keys (`key`):**
  - Named keys: `enter`, `backspace`, `tab`, `escape`, `space`, `left`,
    `right`, `up`, `down`, `home`, `end`, `page_up`, `page_down`, `delete`,
    `f1` to `f12`, `win`.
  - Precision remotes can also send `super`, `insert`, `printscreen`, `pause`,
    `capslock`, `menu`, `browserback`, `browserforward`, `media_play_pause`,
    `media_next`, `media_previous`, `media_stop`, `volume_mute`, `volume_down`,
    and `volume_up`. Accepted aliases include `esc`, `pageup`, `pagedown`,
    `print`, `print_screen`, `playpause`, `next`, `prev`, `stop`, `mute`,
    `voldown`, and `volup`.
  - Modified keys: a single ASCII lowercase letter (`a` to `z`) or digit
    (`0` to `9`) when `mods` is non-empty. Use up to four distinct modifiers
    from `ctrl`, `alt`, `shift`, and `win`.
  - Named shortcuts (with empty `mods`): `copy` (`Ctrl+C`), `paste`
    (`Ctrl+V`), `cut` (`Ctrl+X`), `undo` (`Ctrl+Z`), `select_all` (`Ctrl+A`),
    `task_view` (`Win+Tab`), `lock` (`Win+L`), and `redo` (`Ctrl+Y`, requires
    `input.precision`).
- **Presentation (`slide`):** `next` (next slide), `previous` (previous
  slide), `start` (`F5`), `stop` (`Escape`), `black` (`B`).
- **Laser pointer (`laser`):** when `on` is `true`, `x` and `y` are finite
  fractions of the PC's primary screen (`0.0..=1.0`, left-to-right and
  top-to-bottom). The PC draws a bright dot in a frameless, always-on-top,
  click-through overlay window and fades it out when `on` is `false` or updates
  stop arriving.

### 3.1 Game controller (`pad`, `pad_off`)

Only to a PC that offers `input.gamepad`, under the same `remote_input`
toggle as the rest.

```
Pad = {
  buttons?: uint16,   ; XInput bits, below; 0x0800 is never set
  lt?: uint8, rt?: uint8,                     ; triggers, 0 to 255
  lx?: int16, ly?: int16, rx?: int16, ry?: int16 ; sticks, y positive is up
}
```

`buttons`: `0x0001` up, `0x0002` down, `0x0004` left, `0x0008` right,
`0x0010` start, `0x0020` back, `0x0040` left stick, `0x0080` right stick,
`0x0100` LB, `0x0200` RB, `0x0400` guide, `0x1000` A, `0x2000` B, `0x4000`
X, `0x8000` Y. A missing field is 0.

- Each `pad` carries the whole controller, so it may travel in a datagram:
  one that's lost is made up for by the next. The phone sends it on every
  change and again every 100 ms while its controller is open.
- The first `pad` goes on the control stream. The PC plugs a controller in
  for that phone (an Xbox 360 one on Windows and Linux) and answers `OK`, or
  `UNSUPPORTED` when it can't: on Windows, when the ViGEmBus driver isn't
  installed. A PC without `input.gamepad` answers `UNSUPPORTED` too.
- The PC lets go of every button and centres the sticks when no `pad` has
  come for 1 second, and unplugs the controller after 60 seconds, when the
  phone sends `pad_off` (control stream only), when the link drops, or when
  `remote_input` is turned off. Each phone gets its own controller, up to 4.

### 3.2 Precision motion, gestures, and displays

Send these actions to PCs that advertise `input.precision`.

- `move_precise` carries relative motion with the sender's gain already
  applied. Each axis uses the same units and bounds as `move`.
- `move_absolute` uses finite `x` and `y` from `0.0` to `1.0`, increasing
  right and down across the bounding rectangle of all monitors. `seq` is a
  wrapping sequence number the receiver can use to discard stale motion.
  With `click`, send the position and button together as a control request.
  The PC moves and clicks at that position as one action.
- `zoom` carries signed wheel notches in `[-200, 200]`. Values must be finite.
  Positive values zoom in.
- `modified_button` carries a reliable button action with up to four
  distinct modifiers. An omitted modifier list is empty.
- `gesture` carries a reliable desktop action. The controller uses the
  `supported_actions` list from `remote.info` to decide which to offer.

```
GestureAction =
  "switch_start" | "switch_next" | "switch_prev" | "switch_end" | "switch_cancel"
  | "overview_open" | "overview_close" | "desktop_toggle"
  | "workspace_next" | "workspace_prev" | "search" | "notifications" | "launcher"
  | "close_window" | "screenshot" | "emoji"

RemoteInfo = {
  profile: text,
  supported_actions: [GestureAction],
  monitors: [{ name: text, x: int32, y: int32, width: uint32, height: uint32, primary: bool }],
}
```

`profile` identifies the PC's active gesture profile. Its name has at most
64 UTF-8 bytes. The actions list has no duplicates. The display map contains
up to 32 monitors, each with a name of up to 128 bytes and positive width
and height. Profile and monitor names contain non-whitespace text and no
control characters. Monitor origins can be negative and share the PC's
virtual desktop coordinate space. `primary` marks the primary display.

## 4. Validation and rate limiting

- The PC validates all bounds and key names before touching the OS input APIs.
- Per-peer token buckets bound incoming rates (240 datagrams/s with burst 60;
  500 control requests/s with burst 500).
- Implementations **MUST NOT** log typed `text` (v0 §11).
