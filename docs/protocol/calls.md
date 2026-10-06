# Calls service (protocol v0)

> Status: **draft.** Builds on [protocol v0](v0.md); breaking changes are
> allowed until v1. License: CC BY 4.0.

A phone tells its PCs about its calls: who's calling, when it's answered,
and when it ends. A PC can answer, decline or silence a ringing call, and
hang up, mute, switch to the speaker, hold, press keypad keys and change
the volume of the call in progress. The call's audio stays on the phone;
this service carries no sound.

## 1. Capabilities

| ID | Offered by | Meaning |
|---|---|---|
| `call.state` | phone | Reports its calls (needs the phone permission) |
| `call.control` | phone | Answers, declines, silences and hangs up calls, and changes their volume, when a PC asks |
| `call.incall` | phone | Also mutes, switches to the speaker, holds and presses keys on the call in progress, and reports those `controls` |
| `call.show` | PC | Shows calls |

A phone reports calls only to PCs that offer `call.show`, and only while
both devices allow it (the `calls` device toggle, on by default, on each
side).

## 2. Messages

On the control stream.

```
t = "call.state"            b = {
  id:        text,      // The phone's ID for this call: 1–64 bytes, the same in each of its messages
  state:     "ringing" | "active" | "ended",
  ? incoming: bool,     // Default true; false for calls the phone's user made
  ? number:  text,      // At most 256 bytes, when the phone knows it
  ? name:    text,      // The contact's name, when the number is a contact
  ? photo:   bytes,     // The contact's photo, a JPEG of at most 64 KiB, with "ringing"
  ? missed:  bool,      // With "ended": it rang and nobody answered (default false)
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
  ? digit: text,        // With "dtmf": one of 0–9, * and #
}
t = "ok"           re = n
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

### 2.3 On Android

`call.control` needs the phone permission and the permission to answer
calls. `call.incall` needs Android 12 or later and the
`MANAGE_ONGOING_CALLS` app-op, which lets Telecom bind Nectarlink's
in-call service as a calling companion without it being the phone app.
Apps can't grant that app-op to themselves; Nectarlink's Elevated mode
(Wireless debugging) grants it through the shell when it starts.

Not in this service:

- **Recording calls.** Android doesn't let apps other than the phone's
  own record call audio (capturing the voice call stream needs a
  privileged permission), so Nectarlink doesn't offer it.
- **Talking through the PC.** Carrying the call's audio to the PC's
  microphone and speakers is planned as a Bluetooth hands-free (HFP)
  link from the PC to the phone, as for a headset; it isn't part of this
  service.

## 3. Privacy

Numbers, names and photos are never logged (v0 §11).
