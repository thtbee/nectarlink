# Clipboard service (protocol v0)

> Status: **draft.** Builds on [protocol v0](v0.md); breaking changes are
> allowed until v1. License: CC BY 4.0.

Copied text and images move between paired devices: a PC sends what the
user copies (automatically, unless turned off), and a phone sends its
clipboard when the user asks (Android only lets the app in front read the
clipboard; reading it in the background needs the Elevated power level).

Rich text and files on the clipboard are not part of this version.

## 1. Capabilities

| ID | Offered by | Meaning |
|---|---|---|
| `clip.write` | phone | Accepts `clip.set` |
| `clip.share` | phone | Sends its clipboard when the user asks |
| `clip.read.auto` | phone | Sends clipboard changes on its own (Elevated) |
| `clip.image` | both | Accepts images (§3) |

A PC always accepts `clip.set` from a phone the user allows.

## 2. Messages

```
t = "clip.set"   id = n   b = { text: text }
t = "ok"         re = n
```

**`clip.set`** puts `text` on the receiver's clipboard, replacing what's
there. `text` is plain text of 1 byte to 512 KiB (UTF-8); senders **MUST NOT**
send larger text and receivers reply `BAD_MESSAGE` to it.

Errors: `DENIED` when the user turned the clipboard off for the sending
device (the `clipboard` device toggle), `INTERNAL` when the clipboard
couldn't be written.

## 3. Images

An image travels on its own bidirectional stream (v0 §4), so a large
screenshot never holds up the control stream. Only devices that offer
`clip.image` are sent images. The sender opens the stream and writes:

```
t = "stream"      b = { svc: "clip", op: "image", v: 1 }
t = "clip.image"  b = { mime: text, size: uint }
```

followed by exactly `size` bytes of the image, then finishes its side.
`mime` is `image/png` or `image/jpeg`; senders convert other formats (a
copied bitmap on Windows, a WebP on Android) to PNG. `size` is 1 byte to
32 MiB.

The receiver puts the image on its clipboard and answers on the same
stream with `ok`, or with `error`: `BAD_MESSAGE` for a type or size out of
bounds, `DENIED` as for `clip.set`, `INTERNAL` when the clipboard couldn't
be written. A receiver that refuses early stops reading (QUIC
`STOP_SENDING`) and still sends its answer. The whole exchange times out
after 60 seconds.

Receivers offer the image the way their platform's apps paste it best:
Windows as a bitmap with alpha (`CF_DIBV5`) plus the `PNG` format when a
PNG arrived; Android as a content URI from the app's own provider.

## 4. Rules

- **Consent.** Each device sends and accepts only with the `clipboard`
  toggle on for the other device.
- **Sensitive content is never sent.** On Windows: clips marked
  `ExcludeClipboardContentFromMonitorProcessing`, or with
  `CanIncludeInClipboardHistory` set to 0 (what password managers use). On
  Android: clips marked `ClipDescription.EXTRA_IS_SENSITIVE`.
- **No echoes.** A device does not send text it just received with
  `clip.set`.
- Implementations **MUST NOT** log clipboard content (v0 §11).
