# Remote input service (protocol v0)

> Status: **draft.** Builds on [protocol v0](v0.md); breaking changes are
> allowed until v1. License: CC BY 4.0.

A phone acts as a touchpad, keyboard, and presentation remote for a paired PC.

## 1. Capability and permission

| ID | Offered by | Meaning |
|---|---|---|
| `input.inject` | desktop | Accepts pointer, keyboard, and presentation input from a paired phone |

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
unidirectional stream with header `{ svc: "remote", op: "motion", v: 1 }` and
sends length-prefixed CBOR `Input` frames there. Pointer motion (`type =
"move"`) **MUST NOT** be sent on the control stream.

Datagram-eligible `Input` variants:

```
{ type: "move",   dx: float32, dy: float32 }
{ type: "scroll", dx: float32, dy: float32 }
{ type: "laser",  on: bool, x?: float32, y?: float32 }
```

### 2.2 Control stream requests

Discrete actions that must arrive — and permission checks when opening the
remote screen — use control-stream requests:

```
t = "remote.check"  id = n  b = {}
t = "ok"            re = n

t = "remote.input"  id = n  b = Input
t = "ok"            re = n
```

**`remote.check`** verifies whether the PC currently accepts remote input from
the sender. If `remote_input` is off, the PC replies with `DENIED` and shows
its one-time prompt if it hasn't already asked for this phone.

**`remote.input`** executes a discrete input action (`button`, `scroll`,
`text`, `key`, `slide`, or `laser`). Sending `type = "move"` in `remote.input`
is rejected with `BAD_MESSAGE`. Errors: `UNSUPPORTED` when the receiver does
not offer `input.inject`, `DENIED` when `remote_input` is off for the sender
or rate-limited, `BAD_MESSAGE` when values are out of bounds or name an
unknown key.

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

Button       = "left" | "right" | "middle"
ButtonAction = "down" | "up" | "click"
KeyMod       = "ctrl" | "alt" | "shift" | "win"
SlideAction  = "next" | "previous" | "start" | "stop" | "black"
```

- **Pointer motion (`move`):** relative displacement in logical pixels (`dx`
  right, `dy` down), each finite and within `[-4000, 4000]`. Dragging holds a
  button with `button` `down`, sends `move` datagrams, and finishes with
  `button` `up`.
- **Buttons (`button`):** `left`, `right`, or `middle` mouse button `down`,
  `up`, or `click` (press and release). If a session ends or `remote_input` is
  turned off while a button is held down, the PC releases the held button.
- **Scroll (`scroll`):** horizontal (`dx`, positive right) and vertical (`dy`,
  positive down/towards the user or wheel delta; on Windows `dy > 0` scrolls
  up / standard wheel ticks, `dx > 0` scrolls right) in wheel notches
  (fractions allowed for smooth scrolling), each finite and within
  `[-200, 200]`.
- **Text (`text`):** 1–256 UTF-8 bytes of printable Unicode text (no control
  characters). Injected as Unicode characters (`KEYEVENTF_UNICODE` on Windows)
  so non-Latin scripts and emojis work regardless of keyboard layout.
- **Keys (`key`):**
  - Named keys: `enter`, `backspace`, `tab`, `escape`, `space`, `left`,
    `right`, `up`, `down`, `home`, `end`, `page_up`, `page_down`, `delete`,
    `f1`–`f12`, `win`.
  - Modified keys: a single ASCII lowercase letter (`a`–`z`) or digit
    (`0`–`9`) when `mods` is non-empty (up to 4 modifiers from `ctrl`, `alt`,
    `shift`, `win`).
  - Named shortcuts (with empty `mods`): `copy` (`Ctrl+C`), `paste`
    (`Ctrl+V`), `cut` (`Ctrl+X`), `undo` (`Ctrl+Z`), `select_all` (`Ctrl+A`),
    `task_view` (`Win+Tab`), `lock` (`Win+L`).
- **Presentation (`slide`):** `next` (next slide), `previous` (previous
  slide), `start` (`F5`), `stop` (`Escape`), `black` (`B`).
- **Laser pointer (`laser`):** when `on` is `true`, `x` and `y` are finite
  fractions of the PC's primary screen (`0.0..=1.0`, left-to-right and
  top-to-bottom). The PC draws a bright dot in a frameless, always-on-top,
  click-through overlay window and fades it out when `on` is `false` or updates
  stop arriving.

## 4. Validation and rate limiting

- The PC validates all bounds and key names before touching the OS input APIs.
- Per-peer token buckets bound incoming rates (240 datagrams/s with burst 60;
  60 control requests/s with burst 30).
- Implementations **MUST NOT** log typed `text` (v0 §11).
