# Actions service (protocol v0)

> Status: **draft.** Builds on [protocol v0](v0.md); breaking changes are
> allowed until v1. License: CC BY 4.0.

One-shot actions on a paired device: a phone locks or sleeps a PC, and
either device sends a web link to open on the other.

## 1. Capabilities

| ID | Offered by | Meaning |
|---|---|---|
| `pc.power` | PCs | Locks or sleeps on request |
| `link.open` | both | Opens web links sent to it |

A device sends these requests only to peers that announce the capability.

## 2. Messages

All on the control stream.

```
t = "pc.power"   id = n   b = { action: "lock" | "sleep" }
t = "link.open"  id = n   b = { url: text }
t = "ok"         re = n
```

**`pc.power`** asks a PC to lock or sleep. The PC answers `ok` first, then
acts (sleep after a short moment, so the answer leaves before the network
goes down). Shutting down is deliberately not offered (too easy to trigger
by accident from a pocket).

Errors: `UNSUPPORTED` when the device doesn't lock or sleep on request, or
for an unknown action; `DENIED` when the user turned "PC actions" off for the
phone (the `pc_actions` device toggle, on by default).

**`link.open`** asks the receiver to open `url`. Only http and https links
are allowed, at most 4096 bytes, without whitespace or control characters;
receivers **MUST** check and answer `BAD_MESSAGE` otherwise.

A PC opens the link in the default browser; a phone shows a notification the
user taps to open it (Android doesn't let apps in the background open
screens).

Errors: `BAD_MESSAGE` as above; `INTERNAL` when it couldn't be opened.

## 3. Rules

- Implementations **MUST NOT** log links (v0 §11).
