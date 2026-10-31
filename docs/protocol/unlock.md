# Unlock the PC with the phone (draft)

> Status: **proposal**, not implemented on the wire. See
> [ADR 0009](../adr/0009-unlock-with-phone.md) for the design, the threat model
> and the open questions. Builds on [protocol v0](v0.md). License: CC BY 4.0.

A phone lets a paired PC unlock its locked Windows session after the phone's
owner confirms with a strong biometric. The PC keeps the user's Windows
password only sealed under a key `K`; the phone keeps `K` only wrapped by a
non-exportable Keystore key that needs a biometric for each use.

## 1. Capabilities and toggle

| Capability | Offered by | Meaning |
|---|---|---|
| `unlock.key` | Phone | Can hold a wrapped key for a PC and release it after a strong biometric |
| `unlock.pc` | PC | Can unlock its locked session with a phone |

Both devices have an `unlock` per-device toggle, **off by default**. Every
message below is refused with `DENIED` when it's off on the receiving side, and
only accepted on a direct local path (LAN or USB), never over a relay.

## 2. Messages

On the control stream.

```
t = "unlock.enroll"   id = n   b = { pc_name: text, user_name: text, user: text, key: bytes(32) }
t = "unlock.wrapped"  re = n   b = { wrapped: bytes }
t = "unlock.request"  id = n   b = { request: bytes(16), user_name: text, wrapped: bytes }
t = "unlock.release"  re = n   b = { request: bytes(16), key: bytes(32) }
t = "unlock.forget"            b = { user?: text }
```

### 2.1 `unlock.enroll` (PC to phone)

Sent once the user has turned it on in the PC's Settings and typed their
Windows password there. `user` is the Windows security identifier, `user_name`
what the phone shows. The phone asks its owner ("Let <pc_name> unlock with
your fingerprint for <user_name>?"); after a strong biometric it wraps `key`
with its Keystore key and answers `unlock.wrapped`. Errors: `DENIED` when
declined or off, `UNSUPPORTED` without a strong biometric.

The PC seals the password under `key` (AES-256-GCM, associated data binding the
PC, the phone and `user`), keeps the sealed password and `wrapped`, and
forgets `key`.

### 2.2 `unlock.request` (PC to phone)

Sent when the user picks "Unlock with <phone>" on the lock screen. `request` is
random and single use; the phone must answer within 30 seconds. The phone
shows its biometric prompt ("Unlock <pc_name> for <user_name>?"), unwraps
`wrapped`, and answers `unlock.release` with the same `request` and `key`.
Errors: `DENIED` (declined or off), `NOT_FOUND` (the Keystore key is gone,
for example after a new fingerprint: the PC asks the user to enroll again).

The PC opens the sealed password with `key`. A key that doesn't open it counts
as a failure; five failures in 15 minutes lock the feature for 15 minutes.

### 2.3 `unlock.forget` (either way)

Ends it: the phone deletes its Keystore key for this PC (or for `user`), the PC
deletes the sealed password and the wrapped key. Sent when either side turns
the toggle off or unpairs.

## 3. Privacy and logging

`key`, `wrapped` and the password are never logged, shown or written anywhere
but where section 2 says. The phone's prompt always names the PC and the user,
so a request can't be passed off as something else.
