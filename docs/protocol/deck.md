# Deck service (protocol v0)

> Status: **draft.** Builds on [protocol v0](v0.md); breaking changes are
> allowed until v1. License: CC BY 4.0.

The phone acts as a macro pad for a paired PC, showing pages of large tiles
that trigger PC actions and reflect live PC state (playing/paused media,
speaker volume and mute, microphone mute).

The PC owns the Deck layout and action definitions: the phone receives only
page and tile display metadata (`id`, `label`, `icon`, `color`, `kind`) and
triggers actions by sending `deck.press { tile }` with the tile ID. The phone
never sends commands, file paths, or URLs.

## 1. Capability and permissions

| ID | Offered by | Meaning |
|---|---|---|
| `deck.actions` | desktop | Shares its Deck layout (`deck.layout`) and live state (`deck.state`) and accepts `deck.press` requests (`input.deck`) |

- **Per-device toggles on the PC:**
  - `remote_input` (**off by default**): required for all `deck.press` requests. When a paired phone presses a tile (or calls `remote.check`) while `remote_input` is off, the PC refuses with `DENIED` and emits a one-time consent prompt (`RemoteInputRequested`) for that phone.
  - `commands` (**off by default**): additionally required for tiles whose `kind` is `"run_command"`. Even when `remote_input` is on, pressing a `"run_command"` tile while `commands` is off for that phone is refused with `DENIED`.

## 2. Messages

All on the control stream.

```
t = "deck.layout"  b = DeckLayout                // PC → phone, no reply
t = "deck.state"   b = DeckState                 // PC → phone, no reply
t = "deck.press"   id = n  b = { tile: text }    // phone → PC
t = "ok"           re = n
```

### 2.1 `deck.layout`

Sent by a PC that offers `deck.actions` when a session starts and whenever the
user edits the Deck on the PC.

```
DeckLayout = {
  pages: [DeckPage],        // 1..=8 pages
}

DeckPage = {
  id:    text,              // 1..=64 ASCII alphanumeric / '_' / '-' / '.', unique per layout
  name:  text,              // 1..=64 UTF-8 bytes, no control chars
  tiles: [DeckTile],        // 0..=24 tiles
}

DeckTile = {
  id:    text,              // 1..=64 ASCII alphanumeric / '_' / '-' / '.', unique across the layout
  label: text,              // 1..=64 UTF-8 bytes, no control chars
  icon:  text,              // from the fixed icon set (§2.4)
  color: text,              // from the fixed color set (§2.4)
  kind:  text,              // action kind (§2.4); never contains paths, URLs, or commands
}
```

### 2.2 `deck.state`

Sent by a PC that offers `deck.actions` when a session starts and whenever
the PC's media playback state, speaker volume/mute, or microphone mute state
changes.

```
DeckState = {
  playing:    bool,         // PC media is currently playing (`media_play_pause` tiles)
  volume:     uint,         // PC speaker volume, 0..=100 (`volume_*` tiles)
  muted:      bool,         // PC speaker is muted (`volume_*` tiles)
  mic_muted?: bool,         // default PC microphone is muted (`mic_mute` tiles); omitted when no capture device exists
}
```

### 2.3 `deck.press`

Asks the PC to run the action bound to `tile`.

```
DeckPress = {
  tile: text,               // 1..=64 ASCII alphanumeric / '_' / '-' / '.'
}
```

Errors:
- `BAD_MESSAGE` when `tile` is empty, longer than 64 bytes, or contains disallowed characters.
- `UNSUPPORTED` when the PC does not offer `deck.actions`.
- `DENIED` when `remote_input` is off for the phone, or when the tile's `kind` is `"run_command"` and `commands` is off for the phone.
- `NOT_FOUND` when no tile with ID `tile` exists in the PC's current Deck layout.
- `INTERNAL` when the platform action fails on the PC.

### 2.4 Action kinds, icons, and colors

- **Action kinds (`kind`):**
  - Media: `"media_play_pause"`, `"media_next"`, `"media_previous"`
  - Volume: `"volume_up"`, `"volume_down"`, `"volume_mute"`
  - Microphone: `"mic_mute"`
  - System & windows: `"lock_pc"`, `"show_desktop"`, `"switch_window"`, `"screenshot"`
  - Parameterized on the PC (parameters stay on the PC and are never sent over the wire):
    - `"shortcut"` (custom keyboard shortcut: key + modifiers)
    - `"open_url"` (`http` or `https` URL)
    - `"type_text"` (text snippet typed into the focused field)
    - `"launch_app"` (`.exe` or `.lnk` picked on the PC, launched via `ShellExecuteW` without arguments)
    - `"run_command"` (command written on the PC; requires the per-device `commands` toggle)
- **Icons (`icon`):** `"play"`, `"pause"`, `"skip_next"`, `"skip_previous"`, `"volume_up"`, `"volume_down"`, `"volume_off"`, `"mic"`, `"mic_off"`, `"lock"`, `"desktop"`, `"switch_window"`, `"screenshot"`, `"shortcut"`, `"globe"`, `"text"`, `"app"`, `"terminal"`, `"sparkle"`, `"star"`.
- **Colors (`color`):** `"amber"`, `"coral"`, `"red"`, `"teal"`, `"green"`, `"blue"`, `"violet"`, `"slate"`.
