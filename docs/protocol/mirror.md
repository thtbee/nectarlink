# Screen mirroring (protocol v0)

> Status: **draft.** Builds on [protocol v0](v0.md); breaking changes are
> allowed until v1. License: CC BY 4.0.

A PC shows a phone's screen. The phone's user agrees each time (Android
asks); the phone then streams its screen as H.264 on a stream of its own.
The PC can also use the phone with its mouse and keyboard (§4).

## 1. Capabilities

| ID | Offered by | Meaning |
|---|---|---|
| `mirror.capture` | phone | Shares its screen when its user agrees |
| `mirror.view` | PC | Shows a phone's screen |
| `mirror.input` | phone | Takes the PC's mouse and keyboard while mirrored (Assist: an accessibility service the user turns on; Elevated: injected input) |

Both devices must allow it: the `mirroring` device toggle, on by default,
on each side.

## 2. Control messages

On the control stream.

```
t = "mirror.start"     id = n   b = { max_size: uint, fps: uint, bitrate: uint }
t = "mirror.stop"      id = n
t = "mirror.keyframe"
t = "ok"               re = n
```

- `mirror.start` (PC → phone): show me your screen, its longer side at most
  `max_size` pixels, at most `fps` frames a second, aiming for `bitrate`
  bits a second. The phone answers `ok` once it has asked its user; video
  follows only if the user agrees. Errors: `DENIED` (mirroring is off for
  the PC), `UNSUPPORTED` (no `mirror.capture`).
- `mirror.stop` (either way): stop. The PC stops reading the video stream;
  the phone stops sharing and finishes it.
- `mirror.keyframe` (PC → phone): the decoder lost its place; send a
  keyframe next. Not answered.

## 3. The video stream

The phone opens a bidirectional stream with the header
`{ svc: "mirror", op: "video", v: 1 }` (v0 §4), then writes packets until
it stops. The PC writes nothing; it finishes its side, or stops reading,
to end the stream.

Each packet is:

```
u32 length (big-endian; counts everything after it)
u8  kind      // 0 config, 1 frame, 2 keyframe
u64 time      // capture time in microseconds, big-endian (only differences mean anything)
    data
```

- **config**: the stream's format, CBOR `{ codec: "h264", width: uint,
  height: uint }`. First, and again whenever it changes (the phone turned).
- **frame**: one H.264 access unit (Annex B) that depends on earlier ones.
- **keyframe**: an access unit that starts fresh (an IDR picture), with its
  SPS and PPS before it.

Packets are at most 8 MiB. Video is BT.709, limited range.

## 4. Input

```
t = "mirror.input"   b = { type: "touch", action: "down" | "move" | "up", x: float, y: float }
                   | { type: "scroll", x: float, y: float, dx: float, dy: float }
                   | { type: "key", key: text }
                   | { type: "text", text: text }
```

Sent by the PC, not answered (input is only worth it right away), and
only to phones that offer `mirror.input`. Positions are fractions of the
screen as the phone shows it now (0 at the left or top, 1 at the right or
bottom), so they hold at any size and rotation.

- `touch`: one finger. At Assist, Android plays a gesture whole, so the
  phone turns each down–up into a tap (barely moved), a long press (held
  450 ms or more) or a swipe along the way it went, when it comes up.
- `scroll`: the mouse wheel at a point, in notches (positive: down or
  right); the phone swipes the other way.
- `key`: `back`, `home`, `recents`, `notifications`, `enter`, `backspace`,
  `delete`, `left`, `right`, `up`, `down` or `tab`. Others are dropped.
- `text`: typed text (at most 4 KiB), into the focused text field.

The Windows app maps the left button to a finger, right-click to Back,
middle-click to Home and Ctrl+V to typing the PC's clipboard.

## 5. Latency over completeness

The stream is reliable, so a slow network would otherwise build up delay.
Instead, the phone keeps only a few packets waiting to go out: when that's
full it drops the picture and every one after it until a keyframe, and asks
its encoder for one. The PC likewise decodes everything it gets but shows
only the newest picture when it's behind, and asks for a keyframe when a
picture doesn't decode.

## 6. Privacy

The screen is never recorded or logged by either side. Android shows that
the screen is being shared (in the status bar), and the user can stop it
there, from the phone's notification, or from the PC.
