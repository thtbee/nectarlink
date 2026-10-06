# Calls service (protocol v0)

> Status: **draft.** Builds on [protocol v0](v0.md); breaking changes are
> allowed until v1. License: CC BY 4.0.

A phone tells its PCs about its calls: who's calling, when it's answered,
and when it ends. A PC can answer, decline or silence a ringing call. The
call's audio stays on the phone; this service carries no sound.

## 1. Capabilities

| ID | Offered by | Meaning |
|---|---|---|
| `call.state` | phone | Reports its calls (needs the phone permission) |
| `call.control` | phone | Answers, declines and silences calls when a PC asks |
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
}
t = "call.action"  id = n   b = { id: text, action: "answer" | "decline" | "silence" }
t = "ok"           re = n
```

### 2.1 `call.state`

Sent when a call starts ringing (and again when the phone learns more,
such as the number), when it's answered, and when it ends. Not answered.
A PC that connects during a call gets its latest `call.state` right away.

A call that was declined, on the phone or from a PC, ends without
`missed`.

### 2.2 `call.action`

- `answer`: answers the ringing call; the conversation happens on the phone.
- `decline`: declines the ringing call, or hangs up the active one.
- `silence`: stops the phone ringing; the call still rings for the caller.

Errors: `NOT_FOUND` when `id` isn't the call in progress (or, for `answer`
and `silence`, it no longer rings); `DENIED` when calls are off for the
PC; `UNSUPPORTED` when the phone doesn't offer `call.control`.

## 3. Privacy

Numbers, names and photos are never logged (v0 §11).
