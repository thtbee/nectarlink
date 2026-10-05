# Clipboard service (protocol v0)

> Status: **draft.** Builds on [protocol v0](v0.md); breaking changes are
> allowed until v1. License: CC BY 4.0.

Copied text moves between paired devices: a PC sends what the user copies
(automatically, unless turned off), and a phone sends its clipboard when
the user asks (Android only lets the app in front read the clipboard;
reading it in the background needs the Elevated power level).

Images, rich text and files on the clipboard are not part of this version;
they will travel over bulk streams with file transfer.

## 1. Capabilities

| ID | Offered by | Meaning |
|---|---|---|
| `clip.write` | phone | Accepts `clip.set` |
| `clip.share` | phone | Sends its clipboard when the user asks |
| `clip.read.auto` | phone | Sends clipboard changes on its own (Elevated) |

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

## 3. Rules

- **Consent.** Each device sends and accepts only with the `clipboard`
  toggle on for the other device.
- **Sensitive content is never sent.** On Windows: clips marked
  `ExcludeClipboardContentFromMonitorProcessing`, or with
  `CanIncludeInClipboardHistory` set to 0 (what password managers use). On
  Android: clips marked `ClipDescription.EXTRA_IS_SENSITIVE`.
- **No echoes.** A device does not send text it just received with
  `clip.set`.
- Implementations **MUST NOT** log clipboard content (v0 §11).
