# Actions service (protocol v0)

> Status: **draft.** Builds on [protocol v0](v0.md); breaking changes are
> allowed until v1. License: CC BY 4.0.

One-shot requests from one paired device to another: a phone locks its PC,
puts it to sleep, or wakes it over the local network with Wake-on-LAN, and
either device opens a web link on the other.

## 1. Capabilities

| ID | Offered by | Meaning |
|---|---|---|
| `pc.power` | PC | Locks or sleeps when a paired phone asks |
| `pc.wake` | PC | Shares its network adapter MAC and subnet broadcast addresses (`pc.wake_info`) so a paired phone can wake it |
| `link.open` | both | Opens web links a paired device sends |

Senders check the other device's capabilities first and don't send a
request it doesn't offer.

## 2. Messages

All on the control stream. `pc.power` and `link.open` are requests answered
with `ok` or `error`; `pc.wake_info` is a one-way event (no `id`, no reply).

```
t = "pc.power"      id = n   b = { action: "lock" | "sleep" }
t = "link.open"     id = n   b = { url: text }
t = "ok"            re = n
t = "pc.wake_info"           b = { macs: [text, ...], broadcasts?: [text, ...] }
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

### 2.3 `pc.wake_info` (Wake-on-LAN)

Sent by a PC to a paired phone on session startup and whenever the PC's
network adapters or IPv4 addresses change:

- `macs`: up to 8 6-byte hardware addresses of the PC's physical Ethernet
  and Wi-Fi adapters that are currently up, formatted as lowercase
  `"aa:bb:cc:dd:ee:ff"`, with wired Ethernet listed first. All-zero
  (`00:00:00:00:00:00`) and broadcast (`ff:ff:ff:ff:ff:ff`) addresses are
  invalid.
- `broadcasts` *(optional)*: up to 16 directed IPv4 subnet broadcast
  addresses (e.g. `"192.168.1.255"`) computed from those adapters' current
  IPv4 addresses and prefix lengths (`ip | !mask` for prefix lengths
  `1..=30`).

When the user turns the `pc_actions` device toggle off for a phone, the PC
sends `pc.wake_info` with an empty `macs` list so the phone clears any
stored wake info for that PC.

The phone persists the latest `pc.wake_info` per PC in its store so it
survives restarts. To wake an offline PC, the phone sends the standard
102-byte Wake-on-LAN magic packet (`6 × 0xFF` followed by the 6-byte MAC
repeated 16 times) over UDP to ports `9` and `7`, addressed to each stored
subnet broadcast address and `255.255.255.255`, repeated a few times over
~2 seconds.

## 3. Rules

- Implementations **MUST NOT** log links or hardware/IP addresses in
  `pc.wake_info` debug output (v0 §11).

