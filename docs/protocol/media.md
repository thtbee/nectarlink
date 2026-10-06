# Media service (protocol v0)

> Status: **draft.** Builds on [protocol v0](v0.md); breaking changes are
> allowed until v1. License: CC BY 4.0.

What plays on one device shows on the other, which can control it: a
phone's music in the PC's app and in Windows' media flyout, a PC's in the
phone's media controls. Both directions use the same messages.

## 1. Capabilities

| ID | Offered by | Meaning |
|---|---|---|
| `media.control` | both | Shares its players and takes commands for them (a phone needs notification access to read other apps' players) |
| `media.remote` | both | Shows other devices' players and sends commands to them |

A device sends its players only to devices that offer `media.remote`, and
only while the user allows media for that device (the `media` device
toggle, on by default). It takes commands only while it offers
`media.control` and allows the sender.

## 2. Messages

All on the control stream.

```
t = "media.state"    b = { players: [Player] }          // no reply
t = "media.sync"     b = {}                             // no reply
t = "media.command"  id = n  b = { player: text, action: text, position?: uint }
t = "ok"             re = n
```

**`media.state`** carries every player the sender shares, the most
relevant first (what Windows calls the current session, or what's playing,
then the most recently active). It's sent when a session starts, after
every change, and again when the user changes what the other device may
see. An empty list means nothing plays (or the user stopped sharing); the
receiver clears what it showed. A receiver also clears a device's players
when the connection ends.

**`media.sync`** asks for a `media.state` now (the user allowed media again
on the receiving side).

**`media.command`** asks a player to `play`, `pause`, go to the `next` or
`previous` item, or `seek` to `position` (milliseconds). Errors:
`NOT_FOUND` when the player is gone, `UNSUPPORTED` when it can't do that,
`DENIED` when the user turned media off for the sender, `BAD_MESSAGE` for a
seek without a position.

```
Player = {
  id:        text,       // 1–256 bytes, stable while the player exists
  app:       text,       // user-visible app name
  title?:    text,       // each text up to 256 characters
  artist?:   text,
  album?:    text,
  playing:   bool,
  duration?: uint,       // milliseconds
  position?: uint,       // milliseconds, when sent
  actions:   [text],     // what it takes: play, pause, next, previous, seek
  art_key?:  text,       // 1–64 bytes; the same key means the same picture
  art?:      bytes,      // JPEG or PNG, up to 256 KiB
}
```

While `playing`, the position moves on in real time from when the state
arrived; senders don't send updates just because time passes.

Artwork goes once per `art_key` and session: later states carry the key
alone, and receivers keep pictures by key. A device sends at most 8
players. Receivers drop actions they don't know.

## 3. Rules

- **No echoes.** A device leaves its own stand-in for another device's
  player (Windows' flyout session, Android's media session) out of what it
  shares.
- Implementations **MUST NOT** log titles, artists or albums (v0 §11).
