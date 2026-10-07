# Phone toggles service (protocol v0)

> Status: **draft.** Builds on [protocol v0](v0.md); breaking changes are
> allowed until v1. License: CC BY 4.0.

A phone shares its quick settings state with paired PCs, and a PC can
change the settings the phone's Power Level and permissions allow. Settings
that cannot be changed right now are still shown read-only on the PC with
the capability matrix's reason for what unlocks them.

## 1. Capabilities

| ID | Offered by | Needs | Meaning |
|---|---|---|---|
| `toggles.read` | phone | Basic | Shares its quick settings state (`phone.toggles`) |
| `toggles.ringer` | phone | Basic | Changes the ringer mode between `ring` and `vibrate` (`silent` also needs `toggles.dnd`) |
| `toggles.volume` | phone | Basic | Changes the media volume (`0..=100`) |
| `toggles.flashlight` | phone | Basic | Turns the flashlight on or off (only on phones with a flash unit) |
| `toggles.dnd` | phone | Basic + Do Not Disturb access (or Elevated) | Turns Do Not Disturb on or off and sets the ringer to `silent` |
| `toggles.brightness` | phone | Basic + Modify system settings (or Elevated) | Changes the screen brightness (`0..=100`) |
| `toggles.wifi` | phone | Elevated | Turns Wi-Fi on or off |
| `toggles.bluetooth` | phone | Elevated | Turns Bluetooth on or off |
| `toggles.show` | desktop | — | Shows a phone's controls and sends `phone.toggle.set` requests |

A phone sends `phone.toggles` only to desktops that offer `toggles.show`,
and only while both devices allow the `toggles` device toggle (on by
default).

## 2. Messages

All on the control stream.

```
t = "phone.toggles"     b = PhoneToggles                        // no reply
t = "phone.toggle.set"  id = n  b = { id: text, value: Value }
t = "ok"                re = n
```

### 2.1 `phone.toggles`

Sent by the phone when a session starts, whenever any toggle changes on the
phone (debounced), and when the `toggles` device toggle is turned back on.

```
PhoneToggles = {
  dnd:         bool,          // Do Not Disturb is on
  ringer:      text,          // "ring" | "vibrate" | "silent"
  flashlight?: bool,          // omitted when the phone has no flash unit
  volume:      uint,          // media volume, 0..=100
  brightness:  uint,          // screen brightness, 0..=100
  wifi:        bool,          // Wi-Fi is on
  bluetooth:   bool,          // Bluetooth is on
}
```

### 2.2 `phone.toggle.set`

Asks the phone to change one toggle:

| `id` | `value` | Required phone capability |
|---|---|---|
| `"dnd"` | `bool` | `toggles.dnd` |
| `"ringer"` | `"ring"` \| `"vibrate"` \| `"silent"` | `toggles.ringer` (and `toggles.dnd` for `"silent"`) |
| `"flashlight"` | `bool` | `toggles.flashlight` |
| `"volume"` | `uint` (`0..=100`) | `toggles.volume` |
| `"brightness"` | `uint` (`0..=100`) | `toggles.brightness` |
| `"wifi"` | `bool` | `toggles.wifi` |
| `"bluetooth"` | `bool` | `toggles.bluetooth` |

Errors:
- `BAD_MESSAGE` when `id` is unknown or `value` has the wrong type or is out of range (`> 100` for `volume`/`brightness`, or an unknown ringer mode).
- `UNSUPPORTED` when the phone does not offer the capability for `id` (or `toggles.dnd` when setting `ringer` to `"silent"`, or has no flash unit for `flashlight`).
- `DENIED` when the user turned the `toggles` device toggle off for the peer.
- `INTERNAL` when the platform call fails.

## 3. Rules

- **Confirm turning Wi-Fi off.** Turning Wi-Fi off from the PC can sever the
  LAN connection between the PC and the phone; the PC UI confirms with the
  user before sending `phone.toggle.set { id: "wifi", value: false }`.
- **Debouncing.** Continuous sliders (`volume`, `brightness`) are debounced
  by the sender so dragging a slider does not flood the control stream.
