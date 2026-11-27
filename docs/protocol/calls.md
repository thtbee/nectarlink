# Calls service (protocol v0)

> Status: **draft.** Builds on [protocol v0](v0.md); breaking changes are
> allowed until v1. License: CC BY 4.0.

A phone tells its PCs about its calls: who's calling, when it's answered,
and when it ends. A PC can answer, decline or silence a ringing call,
hang up, mute, switch to the speaker, hold, press keypad keys and change
the volume of the call in progress, browse recent calls, and ask the phone
to call a number. With `call.audio`, the call in progress can also sound
on the PC: the other party from its speakers, its microphone into the
call (§2.6).

## 1. Capabilities

| ID | Offered by | Meaning |
|---|---|---|
| `call.state` | phone | Reports its calls (needs the phone permission) |
| `call.control` | phone | Answers, declines, silences and hangs up calls, and changes their volume, when a PC asks |
| `call.incall` | phone | Also mutes, switches to the speaker, holds and presses keys on the call in progress, and reports those `controls` |
| `call.log` | phone | Lists recent calls (needs the call log permission) |
| `call.dial` | phone | Places a call (or opens the dialer with the number filled in) when a PC asks |
| `call.sims` | phone | Lists the SIMs it calls with, and places a call on the one a PC picks |
| `call.audio` | phone | Carries the call in progress's sound to a PC and the PC's microphone into the call (§2.6) |
| `call.audio.test` | phone | Plays a PC's test sound on its speaker and sends it back, with no call (§2.6.1) |
| `call.show` | PC | Shows calls, recent calls and the dialer, and wants `call.log.changed` |

A phone reports calls and answers call requests only for PCs that offer
`call.show`, and only while both devices allow it (the `calls` device
toggle, on by default, on each side).

## 2. Messages

On the control stream.

```
t = "call.state"            b = {
  id:        text,      // The phone's ID for this call: 1 to 64 bytes, the same in each of its messages
  state:     "ringing" | "active" | "ended",
  ? incoming: bool,     // Default true; false for calls the phone's user made
  ? number:  text,      // At most 256 bytes, when the phone knows it
  ? name:    text,      // The contact's name, when the number is a contact
  ? photo:   bytes,     // The contact's photo, a JPEG of at most 64 KiB, with "ringing"
  ? missed:  bool,      // With "ended": it rang and nobody answered (default false)
  ? app:     text,      // The app the call is in ("WhatsApp"), when it isn't a phone call; answered, declined and ended through the app (its sound stays on the phone)
  ? since:   int,       // With "active": when it was answered, in Unix milliseconds
  ? controls: {         // With "active", from a phone offering call.incall
      ? muted:    bool,     // The phone's microphone is muted
      ? speaker:  bool,     // The sound is on the speaker
      ? held:     bool,     // The call is on hold
      ? can_hold: bool,     // The call can be held
    },
}
t = "call.action"  id = n   b = {
  id:      text,
  action:  "answer" | "decline" | "silence"
         | "mute" | "unmute" | "speaker" | "earpiece" | "hold" | "unhold"
         | "dtmf" | "volume_up" | "volume_down",
  ? digit: text,        // With "dtmf": one of 0 to 9, * and #
}
t = "ok"           re = n

t = "call.log"          id = n   b = { ? before: int, limit: uint }          // 1 to 100
t = "call.log"          re = n   b = { entries: [ call_entry ] }             // newest first

t = "call.log.changed"           b = {}                                      // phone → PC

t = "call.dial"         id = n   b = { number: text, ? sim: text }       // sim: an id from call.sims
t = "ok"                re = n

t = "call.sims"         id = n   b = {}
t = "call.sims"         re = n   b = { sims: [ { id: text, label: text } ], ? default: text }

call_entry = {
  id:        text,
  number:    text,      // Who called, or who was called ("" when withheld)
  ? name:    text,      // The contact's name, when the number is a contact
  direction: "incoming" | "outgoing" | "missed" | "rejected",
  date:      int,       // When the call started, in Unix milliseconds
  duration:  uint,      // Seconds on the call (0 when missed or not answered)
  ? photo:   bytes,     // The contact's photo, a JPEG of at most 16 KiB
}
```

### 2.1 `call.state`

Sent when a call starts ringing (and again when the phone learns more,
such as the number), when it's answered, whenever its `controls` change,
and when it ends. Not answered. A PC that connects during a call gets its
latest `call.state` right away.

`controls` is there only while the phone can change them: a PC shows
mute, speaker, hold and the keypad only then.

A call that was declined, on the phone or from a PC, ends without
`missed`.

### 2.2 `call.action`

- `answer`: answers the ringing call; the conversation happens on the phone.
- `decline`: declines the ringing call, or hangs up the active one.
- `silence`: stops the phone ringing; the call still rings for the caller.
- `volume_up`, `volume_down`: the active call's volume (`call.control`).
- `mute`, `unmute`, `speaker`, `earpiece`, `hold`, `unhold`: as named, on
  the active call (`call.incall`). The new state comes back in
  `call.state`.
- `dtmf`: plays the keypad tone `digit` on the active call (`call.incall`).

Errors: `NOT_FOUND` when `id` isn't the call in progress, or the action
doesn't fit its state (`answer` and `silence` need it ringing; the others,
except `decline`, need it active); `DENIED` when calls are off for the PC;
`UNSUPPORTED` for an unknown action, a `dtmf` without a valid `digit`, or
when the phone doesn't offer the capability the action needs.

### 2.3 `call.log` and `call.log.changed`

`call.log` pages back through the phone's recent calls, newest first:
`before` is the oldest `date` already shown. A phone answers with fewer
items than `limit` when the answer wouldn't fit in a frame (v0 §3),
dropping the oldest.

The phone sends `call.log.changed` to connected PCs that offer `call.show`
when its call log changes (debounced). PCs then ask again for what they
show.

Errors: `DENIED` when calls are off for the PC, `UNSUPPORTED` when the
phone doesn't offer `call.log`.

### 2.4 `call.dial`

`call.dial` asks the phone to call `number` (1 to 256 bytes, not blank). With
`CALL_PHONE` permission the phone places the call directly
(`Intent.ACTION_CALL`); if that permission isn't granted, it opens the
dialer (`Intent.ACTION_DIAL`) with the number filled in. Once the call
starts, the phone reports it through `call.state`, so the PC's call card
and in-call controls work as for any call.

With `sim` (an `id` from `call.sims`, 1 to 64 bytes), a phone that offers
`call.sims` places the call on that SIM, or on its default if the SIM is
gone; phones from before `sim` ignore it. A PC sends `sim` only to a phone
that offers `call.sims`.

Errors: `DENIED` when calls are off for the PC, `UNSUPPORTED` when the
phone doesn't offer `call.dial`, `INTERNAL` when the phone couldn't start
the call.

### 2.4.1 `call.sims`

`call.sims` asks which SIMs (and calling apps, such as a SIP account) the
phone places calls with: at most 8, each with an `id` that stays the same
while the SIM is in and a `label` as Android shows it ("SIM 1 · Airtel"),
and `default` when the phone calls with one without asking. A phone set to
ask each time has no `default`, and Android would ask on the phone, where
nobody at the PC sees it, so the PC picks one: the user's choice for that
phone, else the first. With one SIM there's nothing to pick.

Errors: `DENIED` when calls are off for the PC, `UNSUPPORTED` when the
phone doesn't offer `call.sims`.

### 2.5 On Android

`call.control` needs the phone permission and the permission to answer
calls. `call.log` needs the call log permission. `call.dial` uses
`CALL_PHONE` to place the call directly, or opens the dialer when
`CALL_PHONE` isn't granted. `call.sims` needs `CALL_PHONE` and
`READ_PHONE_STATE`: it lists `TelecomManager.getCallCapablePhoneAccounts()`
and calls with `EXTRA_PHONE_ACCOUNT_HANDLE`. `call.incall` needs Android 12 or later and
the `MANAGE_ONGOING_CALLS` app-op, which lets Telecom bind NectarLink's
in-call service as a calling companion without it being the phone app.
Apps can't grant that app-op to themselves; NectarLink's Elevated mode
(Wireless debugging) grants it through the shell when it starts.

`call.audio` needs Android 13 or later and Elevated (§2.6).
`call.audio.test` needs neither: any phone with the app version that
added it offers it.

### 2.6 `call.audio.start` and `call.audio.stop`

Moves the call in progress's sound to the PC, and back:

```
t = "call.audio.start"  id = n  body = { id: "<call id>" }   // PC to phone
t = "ok"                re = n
t = "call.audio.stop"   [id = n]                             // either way
t = "ok"                re = n                               // when it had an id
```

**`call.audio.start`** asks the phone to carry call `id`'s sound. The phone
answers `ok` once it can reach the call's sound, then opens the `downlink`
stream; the PC then opens the `uplink` stream. Errors: `DENIED` when calls
or the `call_audio` device toggle (on by default) are off for the PC, `NOT_FOUND` when `id` isn't the active call (a ringing
call can't move), `UNSUPPORTED` without `call.audio` or when the phone
can't reach the call's sound after all (its audio HAL doesn't allow it).

**`call.audio.stop`** gives the sound back to the phone: from the PC, the
phone stops carrying it; from the phone (its user tapped Back to phone),
the PC stops playing and recording. Ending either stream means the same.
The call ending ends both streams.

Each direction is its own bidirectional QUIC stream, written one way with
the packets of the [webcam stream](webcam.md) §3, a `config` packet
(`{ codec: "pcm_s16le", rate: 16000, channels: 1 }`) first, then sound:

| Header | Opened by | Carries |
|---|---|---|
| `{ svc: "calls", op: "downlink", v: 1 }` | phone | The other party, for the PC's speakers |
| `{ svc: "calls", op: "uplink", v: 1 }` | PC | The PC's microphone, for the call |

A PC takes a `downlink` stream only while it asked that phone for the
call's sound; a phone takes an `uplink` stream only while it offers
`call.audio`. Any other format, or a stream nobody asked for, is reset
with code `14`, as is a stream that stops.

While the sound is on the PC, the phone shows an ongoing notification
saying so, with a **Back to phone** button.

**On Android**: only the shell user holds `CALL_AUDIO_INTERCEPTION`, so
NectarLink's Elevated helper reaches the call's sound: an `AudioRecord` on
`VOICE_DOWNLINK` and an `AudioTrack` with `USAGE_CALL_ASSISTANT`, both in
call redirection mode `PSTN` (what `AudioManager`'s
`getCallDownlinkExtractionAudioRecord` and
`getCallUplinkInjectionAudioTrack` build). That needs the phone's audio
HAL to have the telephony Rx and Tx devices
(`isPstnCallAudioInterceptable`), which is up to the phone's maker. The
phone checks it each time Elevated starts and offers `call.audio` only
when it holds.

#### 2.6.1 `call.audio.test.start` and `call.audio.test.stop`

A PC's **Test call audio** (Settings › Calls through this PC) checks the
sound path of a call with no call: the PC's microphone goes to the phone,
which plays it on its speaker and sends it straight back, and the PC plays
what returns on its call output. It reuses the packets and format of §2.6
and nothing else of it:

```
t = "call.audio.test.start"  id = n          // PC to phone, no body
t = "ok"                     re = n
t = "call.audio.test.stop"   [id = n]        // either way
t = "ok"                     re = n          // when it had an id
```

The PC asks only a phone that offers `call.audio.test`; to any other the
test runs on the PC alone (it records, then plays back). The phone answers
`DENIED` when calls or the `call_audio` toggle are off for the PC,
`UNSUPPORTED` without the capability, and `BUSY` while a call's sound is on
a PC (the test would cut into it). A call's sound always wins: a
`call.audio.start` from the PC ends its test, and a phone that gets one
ends the test too.

After `ok` the PC opens the first stream, the phone the second:

| Header | Opened by | Carries |
|---|---|---|
| `{ svc: "calls", op: "test-uplink", v: 1 }` | PC | The PC's microphone, for the phone's speaker |
| `{ svc: "calls", op: "test-downlink", v: 1 }` | phone | The same packets, sent back as they came: `config` first, then each frame with its `time_us` unchanged |

The phone opens `test-downlink` when it has read the `config` packet of
`test-uplink`, so the PC measures the round trip from its own capture time
(`time_us` counts samples from the first one) to the packet's return, and
sound that never came back shows as a jump in `time_us`. A device takes
these streams only while a test is under way (after `ok`, until
`call.audio.test.stop` or a stream ends); any other, or a different format,
is reset with code `14`, as for a call's sound. The test has its own stream
ops and stop signals, so it never reaches a call's sound, and the call's
streams (`uplink`, `downlink`) never carry a test. Nothing is stored on
either device.

Not in this service:

- **Recording calls.** NectarLink doesn't record calls; `call.audio`
  only carries the sound while the call goes on.

## 3. Privacy

Numbers, names and photos are never logged (v0 §11). A call's sound
(§2.6) is never stored on either device: it's played and recorded only
while it moves, and the phone shows that it does.
