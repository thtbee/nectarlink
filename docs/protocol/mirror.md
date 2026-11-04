# Screen mirroring (protocol v0)

> Status: **draft.** Builds on [protocol v0](v0.md); breaking changes are
> allowed until v1. License: CC BY 4.0.

A PC shows a phone's screen. The phone's user agrees each time (Android
asks); the phone then streams its screen as H.264 or, when both sides support
it, H.265 on a stream of its own,
and its sound, when the PC asks, on another (§5). The PC can also use the
phone with its mouse and keyboard (§4), and open the phone's apps in
windows of their own (§6).

## 1. Capabilities

| ID | Offered by | Meaning |
|---|---|---|
| `mirror.capture` | phone | Shares its screen when its user agrees |
| `mirror.view` | PC | Shows a phone's screen |
| `mirror.decode.hevc` | PC or phone | Decodes H.265 screen-mirroring streams |
| `mirror.pace` | PC or phone | Reports video packets as they arrive, and paces what it sends by those reports (§3) |
| `mirror.input` | phone | Takes the PC's mouse and keyboard while mirrored (Assist: an accessibility service the user turns on; Elevated: injected input) |
| `mirror.audio.playback` | phone | Shares the sound of apps that allow it while mirrored (Android 10+) |
| `mirror.audio` | phone | Shares all its sound while mirrored (planned, Elevated) |
| `mirror.sound` | phone | Shares its sound on its own, without the screen (`sound_only`, §5) |
| `mirror.virtual_display` | phone | Runs apps on displays of their own, shown in windows on the PC (Elevated, Android 11+) |
| `mirror.listen` | PC | Plays a mirrored phone's sound |

Both devices must allow it: the `mirroring` device toggle, on by default,
on each side.

## 2. Control messages

On the control stream.

```
t = "mirror.start"     id = n   b = { max_size: uint, fps: uint, bitrate: uint, ? audio: bool,
                                     ? session: uint, ? app: text,
                                     ? stay_awake: bool, ? screen_off: bool, ? watch: text,
                                     ? sound_only: bool }
t = "mirror.stop"      id = n   b = { ? session: uint }
t = "mirror.power"              b = { ? stay_awake: bool, ? screen_off: bool }
t = "mirror.keyframe"           b = { ? session: uint }
t = "mirror.resize"             b = { session: uint, width: uint, height: uint }
t = "mirror.apps"      id = n
t = "mirror.apps"      re = n   b = { apps: [{ pkg: text, label: text, ? icon: bytes }] }
t = "ok"               re = n
```

Each mirroring is a **session** the PC numbers: 0 (the default) is the
phone's screen; any other is an app window (§6). A missing `session`
means 0, so PCs and phones that predate sessions work as before.

- `mirror.start` (PC → phone): show me your screen, its longer side at most
  `max_size` pixels, at most `fps` frames a second, aiming for `bitrate`
  bits a second, and with `audio` (default false), its sound too. For the
  phone's screen (`session` 0), `stay_awake` keeps the phone's screen from
  timing out while mirrored, and `screen_off` (Elevated) turns the phone's
  physical display panel off while the mirror keeps running. The
  phone answers `ok` once it has asked its user; video follows only if the
  user agrees, and sound only if the phone can (it may share the screen
  without it). Errors: `DENIED` (mirroring is off for the PC),
  `UNSUPPORTED` (no `mirror.capture`).
- `sound_only` (PC → phone, `session` 0 with `audio`, never with `app`,
  `watch`, `stay_awake` or `screen_off`): only the phone's sound, no video
  stream; the PC plays it without a window. Sent only to a phone that offers
  `mirror.sound`; one that doesn't answers `UNSUPPORTED`, so an older phone
  is never asked for its screen instead. `mirror.stop` (session 0) ends it.
- `watch` (phone → PC, `session` 0 only, never with `app`): Play on phone.
  Show what this PC player plays instead of the PC's screen, after the PC
  offered it with `media.watch` ([media.md](media.md) §4).
- `mirror.stop` (either way): stop. The PC stops reading the video stream;
  the phone stops sharing, restores normal display power, and finishes it.
- `mirror.sound` (PC → phone, `on`): stop sending the phone's sound to the PC
  so the phone plays it again, or start again. A phone from before this
  message answers `UNSUPPORTED`.
- `mirror.power` (PC → phone): update `stay_awake` and `screen_off` while
  mirroring the phone's screen (`session` 0). Not answered.
- `mirror.keyframe` (PC → phone): the decoder lost its place; send a
  keyframe next. Not answered.
- `mirror.resize` (PC → phone): resize an app window's display (`session != 0`)
  to `width × height` pixels. Not answered; the phone sends a new `config`
  packet on the video stream once its encoder restarts at the new size.

## 3. The video stream

The phone opens a bidirectional stream with the header
`{ svc: "mirror", op: "video", v: 1 }` (v0 §4), then writes packets until
it stops. The PC finishes its side, or stops reading, to end the stream.
It writes nothing else, unless both devices offer `mirror.pace`: then it
writes one byte (any value) per packet it has read, config packets included,
and the phone keeps its video from getting more than 250 ms ahead of those
reports beyond a round trip. A device from before `mirror.pace` must not be
sent reports: it reads at most 64 bytes back, then takes the stream as ended.
A PC sharing its screen with a phone does the same, the other way round.

Each packet is:

```
u32 length (big-endian; counts everything after it)
u8  kind      // 0 config, 1 frame, 2 keyframe
u64 time      // capture time in microseconds, big-endian (only differences mean anything)
    data
```

- **config**: the stream's format, CBOR `{ codec: "h264" | "h265", width: uint,
  height: uint, ? session: uint, ? app_name: text }`. First, and again whenever
  it changes (the phone turned).
- **frame**: one H.264 or H.265 access unit (Annex B) that depends on earlier ones.
- **keyframe**: an access unit that starts fresh (an IDR/IRAP picture), with
  the codec's parameter sets before it (SPS/PPS for H.264; VPS/SPS/PPS for H.265).

Packets are at most 8 MiB. Video is BT.709, limited range.

The PC can also send its screen to the phone using the same packet format.
It uses H.265 when the phone advertises `mirror.decode.hevc` and the PC's
HEVC encoder starts successfully; otherwise it uses H.264. If the codec changes
during a stream, the sender sends a new config packet.

## 4. Input

```
t = "mirror.input"   b = { type: "touch", action: "down" | "move" | "up", x: float, y: float }
                   | { type: "scroll", x: float, y: float, dx: float, dy: float }
                   | { type: "key", key: text }
                   | { type: "text", text: text }
                   | { type: "gamepad", buttons: uint, lx: int, ly: int, rx: int, ry: int, lt: uint, rt: uint }
```

Sent by the PC, not answered (input is only worth it right away), and
only to phones that offer `mirror.input`. Positions are fractions of the
screen as the phone shows it now (0 at the left or top, 1 at the right or
bottom), so they hold at any size and rotation.

- `touch`: one finger. At Elevated, the phone injects each down, move and
  up as it comes, so dragging is live. At Assist, Android plays a gesture
  whole, so the phone turns each down and up into a tap (barely moved), a long
  press (held 450 ms or more) or a swipe along the way it went, when it
  comes up.
- `scroll`: the mouse wheel at a point, in notches (positive: down or
  right); the phone swipes the other way.
- `key`: `back`, `home`, `recents`, `notifications`, `lock`, `enter`, `backspace`,
  `delete`, `left`, `right`, `up`, `down`, `tab`, `copy`, `cut`,
  `select_all`, `send`, `keyboard_on` or `keyboard_off`. Others are dropped (so
  phones from before `copy`, `cut` and `select_all` ignore them). `copy`,
  `cut` and `select_all` act on the focused text field. `send` presses the focused field's action through the
  accessibility service, or is a plain `enter` without it. `keyboard_on` and `keyboard_off` mark
  when a PC starts or stops typing on the phone (`session` 0) without
  opening a video stream; the phone shows a banner with a button to stop it
  (which sends `mirror.stop` for `session` 0 back to the PC).
- `text`: typed text (at most 4 KiB), into the focused text field. When the
  accessibility service (`InputService`) is running, non-ASCII / Unicode
  text is inserted directly into the focused editable node; at Elevated
  without the accessibility service, ASCII/Latin text is typed as key events
  and non-ASCII text is pasted via the phone's clipboard (`KEYCODE_PASTE`).
- `gamepad`: controller state forwarded from the PC's connected gamepad
  (e.g. XInput). `buttons` is a 16-bit XInput-compatible button bitmask
  (D-pad up/down/left/right, Start, Back, L3, R3, LB, RB, Guide, A, B, X, Y);
  `lx`, `ly`, `rx`, `ry` are signed 16-bit thumbstick axes (`-32768..=32767`);
  `lt` and `rt` are unsigned 8-bit trigger axes (`0..=255`). At Elevated,
  the phone injects matching `KeyEvent` (`SOURCE_GAMEPAD`) and `MotionEvent`
  (`SOURCE_JOYSTICK`) inputs on the target display.

The Windows app maps the left button to a finger, right-click to Back,
middle-click to Home, Ctrl+C to syncing the phone's clipboard back to the PC,
and Ctrl+V to syncing the PC's clipboard (`clip.set`) and pasting/typing into
the phone. Dragging and dropping files onto the mirror canvas sends them to
the phone via the `files` service (`files.offer`). The app also offers a
**Type on phone** bar on Home (and in the Command Palette) to type on the
phone with the PC keyboard without mirroring its screen.

## 5. Sound

When the PC asked for it, the phone opens a second stream with the header
`{ svc: "mirror", op: "audio", v: 1 }`, with packets as in §3:

- **config**: the sound's format, CBOR `{ codec: "pcm_s16le", rate: uint,
  channels: uint }`: 16-bit little-endian PCM, channels interleaved, 8 to
  96 kHz, mono or stereo. First, and again if it changes.
- **frame**: a few milliseconds of sound (Android sends 10 ms of 48 kHz
  stereo, 1,920 bytes), whole sample frames only.

A PC that doesn't offer `mirror.listen` resets the stream. The stream ends
with the video: when either side stops mirroring, or the connection drops.
With `sound_only` there's no video: the sound stream is all there is, and
it ends when either side stops or the connection drops.

On Android, the sound is what apps play (media, games, and apps that don't
say otherwise), captured with the screen share's consent and the
permission to record audio, which the phone asks for the first time a PC
wants the sound. Apps that don't allow playback capture, and calls, aren't
heard. Sharing all sound (`mirror.audio`) is planned for Elevated.

While mirroring, the phone's own speaker keeps playing with playback
capture, and the PC can mute its copy. Through Elevated the sound comes
from the remote submix, which takes it off the speaker. With only the sound
(`sound_only`), the phone turns its media volume to 0 for playback capture
and gives it back afterwards: a loopback capture is fixed volume, so the PC
still gets it at full level.

## 6. App windows

With `mirror.virtual_display`, a PC can open one of the phone's apps in a
window of its own, beside whatever the phone's screen shows:

- `mirror.apps` (PC → phone) lists the apps that can open: launchable ones,
  by name, each with its package name and a small PNG icon (at most 8 KiB;
  at most 500 apps, and icons may be left out to keep the answer small).
  Errors: `DENIED`, `UNSUPPORTED` (no `mirror.virtual_display`).
- `mirror.start` with `app` (a package name) and a `session` other than 0
  runs that app on a display of its own and streams it like the screen,
  with no prompt (Elevated was the user's consent). Its video stream's
  config packets carry `session` (the config's CBOR gains `? session:
  uint`). Sound isn't sent for app windows. `BAD_MESSAGE` when `app` and
  `session` don't go together.
- `mirror.input` gains `? session: uint`: input for an app window goes to
  its display. App windows take input whenever the phone offers
  `mirror.virtual_display`; the screen still needs `mirror.input`.
- `mirror.resize` with `session`, `width` and `height` resizes that window's
  virtual display (clamped to even dimensions within encoder limits, keeping
  density sensible) and restarts its H.264 encoder, sending a new `config`
  packet and keyframe so the app re-lays out at the new aspect ratio.
- `mirror.stop` and `mirror.keyframe` with a `session` are about that
  window; stopping one closes the app's display (the app closes with it).
  When the app closes its last activity on the phone, the phone finishes
  that session's video stream.

On Android, the Elevated helper (running as the shell user) makes a
virtual display for each window with its display IME policy set to hide the
soft keyboard, attaches an H.264 encoder drawing from it, and starts the app
there; the phone shows a notification while any app window is open, with a
button to close them all.

## 7. Latency over completeness

The stream is reliable, so a slow network would otherwise build up delay.
Instead, the phone keeps only a few packets waiting to go out: when that's
full it drops the picture and every one after it until a keyframe, and asks
its encoder for one. The PC likewise decodes everything it gets but shows
only the newest picture when it's behind, and asks for a keyframe when a
picture doesn't decode.

That queue fills only once QUIC stops taking data, and QUIC takes many
seconds of video on a slow link. So with `mirror.pace` (§3) the phone also
stops writing when it gets too far ahead of the PC's reports, until all that's
on its way has arrived; that wait also measures what the link carries, which
sets the encoder's bitrate. A frame with a keyframe queued behind it is
skipped.

Sound works the same way without keyframes: a packet that doesn't fit is
dropped on its own. The PC plays from a short buffer (about 40 ms),
plays silence and refills it when the network stalls, and drops the oldest
sound when more than 200 ms piles up, so sound stays in step with the
picture.

## 8. Privacy

The screen and sound are never recorded or logged by either side. Android shows that
the screen is being shared (in the status bar), and the user can stop it
there, from the phone's notification, or from the PC.
