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
| `media.watch` | phone | Shows a PC's playback streamed to it (Play on phone, §4) |
| `media.stream` | PC | Streams a player's picture and sound on request (§4) |

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
If the artwork would push the snapshot over the frame limit, the sender
keeps all player metadata and defers excess artwork until another snapshot.

**`media.sync`** asks for a `media.state` now (the user allowed media again
on the receiving side).

**`media.command`** asks a player to `play`, `pause`, go to the `next` or
`previous` item, or `seek` to `position` (milliseconds). Errors:
`NOT_FOUND` when the player is gone, `UNSUPPORTED` when it can't do that,
`DENIED` when the user turned media off for the sender, `BAD_MESSAGE` for a
seek without a position.

```
Player = {
  id:        text,       // 1 to 256 bytes, stable while the player exists
  app:       text,       // user-visible app name
  title?:    text,       // each text up to 256 characters
  artist?:   text,
  album?:    text,
  playing:   bool,
  duration?: uint,       // milliseconds
  position?: uint,       // milliseconds, when sent
  actions:   [text],     // what it takes: play, pause, next, previous, seek
  art_key?:  text,       // 1 to 64 bytes; the same key means the same picture
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

## 4. Play on phone

A PC can move what one of its players plays to a phone. How depends on
what plays, and the PC decides:

- **Streaming services that protect their video** (Netflix, Prime Video,
  JioHotstar, Disney+ and the like) come out black when captured. The PC
  sends the page's address with `link.open` ([actions.md](actions.md)),
  which Android hands to the service's app, and pauses its own player.
- **A video file** a desktop player plays (VLC, Media Player and the like)
  is offered as a file: the phone reads it from the PC and plays it
  itself, from where the PC had got to, at its full quality.
- **Everything else** is streamed from the PC as it plays, picture and
  sound, so whatever the PC does to it carries over (a browser's ad
  blocker on YouTube, for one). The PC keeps playing.

Streaming starts with an offer from the PC:

```
t = "media.watch"  id = n  b = { player: text, title?: text, file?: WatchFile }   // PC to phone
t = "ok"           re = n

WatchFile = { name: text, size: uint, position: uint }
```

`player` is the PC's ID for the player, as in `media.state`; `title` is
what it plays, up to 256 characters, to show with the offer. The phone
shows the offer (Android doesn't let an app open a screen from the
background, so usually as a notification) and answers `ok`. Errors:
`UNSUPPORTED` when the phone doesn't offer `media.watch`, `DENIED` when
media is off for the PC, `BAD_MESSAGE` for an empty or too long `player`
or `title`.

To watch, the phone sends `mirror.start` with `watch` set to that
player, `session` 0 and `audio` true ([mirror.md](mirror.md) §2). The PC
allows it only for a player it offered that phone, while the offer is
open: for 10 minutes after the offer or after watching last stopped, so
the phone can come back to it, and for as long as the connection lasts.
It then streams the part of its screen the player's picture shows in (a
browser's video, else its page, else its window) on the mirroring video
stream, and its sound as Phone speaker does. Input from the phone is
ignored while it watches. Anything else in that `mirror.start` is as for
the PC's screen; without an offer the answer is `DENIED`, and the PC
doesn't ask its user to share the screen.

### Playing a file

With `file`, the phone plays the file itself instead of watching a
stream. `name` is the file's name (1 to 256 characters, no `/` or `\`),
for the phone's player to tell the format by; `size` its size in bytes
(more than 0); `position` where the PC had got to, in milliseconds. A
PC offers a file only in a format phones play (MP4, MKV, WebM, MOV, 3GP,
MPEG-TS), and streams anything else. Once the phone plays it, it pauses
the PC's player with `media.command`.

The phone reads the file on bidirectional streams of their own, one read
each:

```
stream header  { svc: "media", op: "file", v: 1 }
t = "media.file"       id = 1  b = { offset: uint, length: uint }   // phone to PC
t = "media.file.meta"  re = 1  b = { size: uint, length: uint }     // PC to phone
<length bytes>
```

The PC answers with at most 4 MiB (`length` is cut to that, and to the
end of the file; past the end it is 0), then the bytes, and finishes the
stream. It serves only the file of its open offer to that phone (the
same 10 minutes as for watching, renewed by every read), so a phone can't
read anything else on the PC; without one the answer is `DENIED`.
A newer offer, streamed or not, replaces it.

