# Actions service (protocol v0)

> Status: **draft.** Builds on [protocol v0](v0.md); breaking changes are
> allowed until v1. License: CC BY 4.0.

One-shot requests from one paired device to another: a phone locks its PC
or puts it to sleep, and either device opens a web link on the other.

## 1. Capabilities

| ID | Offered by | Meaning |
|---|---|---|
| `pc.power` | PC | Locks or sleeps when a paired phone asks |
| `link.open` | both | Opens web links a paired device sends |

Senders check the other device's capabilities first and don't send a
request it doesn't offer.

## 2. Messages

All on the control stream, each answered with `ok` or `error`.

```
t = "pc.power"   id = n   b = { action: "lock" | "sleep" }
t = "link.open"  id = n   b = { url: text }
t = "ok"         re = n
```

### 2.1 `pc.power`

Asks the PC to lock (show the sign-in screen) or to sleep. The PC answers
`ok` before it acts, and waits a moment before sleeping so the answer
leaves before its network goes down.

Errors: `DENIED` when the user turned PC actions off for the phone (the
`pc_actions` device toggle, on by default); `UNSUPPORTED` when the device
doesn't do this, or for an action it doesn't know.

Shutting down and restarting are deliberately not part of this version:
they're too easy to set off by accident, and lose unsaved work.

### 2.2 `link.open`

Asks the receiver to open `url`, which **MUST** be an `http` or `https`
link of at most 4,096 bytes with no whitespace or control characters.
Receivers **MUST** check this themselves and answer `BAD_MESSAGE` to
anything else; no other kind of link is ever opened.

A PC opens the link in its default browser. A phone shows it in a
notification the user taps to open it, since Android doesn't let apps in
the background open screens. `INTERNAL` means it couldn't be opened or
shown.

## 3. Rules

- Implementations **MUST NOT** log links (v0 §11).
