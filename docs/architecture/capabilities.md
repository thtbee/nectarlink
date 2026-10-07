# Capability matrix

The capability matrix decides, for every feature and every paired device,
whether the feature is **Available**, **Partial**, **Locked** or
**Unsupported**, and what the user can do about it. Both apps render their UI
from the matrix, computed by `nectarlink-core` (`features.rs`), so the phone
and PC agree about what works.

## Inputs

| Input | Source |
|---|---|
| Capabilities of both devices | Each device's `hello.caps` / `hello.update` (protocol §6, registry in `docs/protocol/capabilities.md`); the peer's are stored, so the matrix also works while it is offline |
| Phone power level | The phone's `hello.power` |
| OS versions | `DeviceInfo.os_ver`: Android release (`"16"`), Windows build (`"10.0.26200"`) |
| Enabled connection paths | LAN always; relay when Away mode is on |
| Per-device toggles | The user's switches for this paired device (`DEVICE_TOGGLES`) |
| Feature registry | Static definitions compiled into the core (`FEATURES`) |

The matrix is recomputed whenever an input changes and emitted as
`NodeEvent::Capabilities` only when it actually changed. `Node::capabilities`
returns it on demand.

## Feature definitions

Each feature is declared once, in Rust. Requirements name the **phone** and
the **desktop**, never "local" and "peer": the same definitions evaluated on
either device give the same answer.

```rust
pub struct FeatureDef {
    pub id: &'static str,              // e.g. "clipboard.auto_phone_to_pc"
    pub group: FeatureGroup,           // Notifications, Clipboard, Files, Mirroring…
    /// Every requirement must hold for the feature to be Available.
    pub requires: &'static [Requirement],
    /// If `requires` fails, this weaker set may still give a Partial state.
    pub partial: Option<PartialDef>,
}

pub enum Requirement {
    /// The device in `role` (Phone or Desktop) announces capability `id`.
    /// `unlock` says how a missing capability is obtained.
    Cap { role: Role, id: &'static str, unlock: Unlock },
    PowerAtLeast(PowerLevel),          // phone: Basic < Assist < Elevated
    AndroidAtLeast(u32),               // phone's Android release
    WindowsBuildAtLeast(u32),          // desktop's Windows build
    Path(ConnectionPath),              // e.g. Relay (Away mode)
    DeviceToggle(&'static str),        // per-device user permission
}

pub enum Unlock {
    Power(PowerLevel),                 // the phone offers it from this level on
    Permission(Permission),            // … once this Android permission is granted
    Addon(&'static str),               // the desktop offers it once the add-on is installed
    UpdateApp,                         // every current app offers it: that app is too old
}

pub struct PartialDef {
    pub requires: &'static [Requirement],
    pub limit: &'static str,           // translation key, e.g. "clipboard.limit.manual"
}
```

## States

```rust
pub enum FeatureState {
    Available,
    /// Works, with a limitation shown inline, plus how to remove it (if possible).
    Partial { limit: &'static str, upgrade: Option<Upgrade> },
    /// Doesn't work yet, but the user can unlock it.
    Locked { upgrade: Upgrade },
    /// Can't work on this device pair; explained, not actionable.
    Unsupported { reason: UnsupportedReason },
}

pub struct Upgrade { pub action: UpgradeAction, pub effort: Effort }   // Effort: Instant | Minutes(n)

pub enum UpgradeAction {
    RaisePower(PowerLevel),            // open "Choose your power" at that level   (Assist ~1 min, Elevated ~2 min)
    GrantPermission(Permission),       // e.g. notification access                 (instant)
    EnableAddon(&'static str),         // e.g. install the virtual camera          (~1 min)
    EnablePath(ConnectionPath),        // e.g. turn on Away mode                   (instant)
    EnableDeviceToggle(&'static str),  // flip a per-device permission             (instant)
    UpdateApp(Role),                   // that device's app is too old             (~2 min)
}

pub enum UnsupportedReason {
    DeviceKinds,                       // the pair isn't one phone + one desktop
    AndroidTooOld { needs: u32 },
    WindowsTooOld { needs_build: u32 },
    NotOnThisDevice,                   // right power level, still not offered (hardware/OEM)
}
```

### How each requirement is checked

- `Cap`: met if announced. Otherwise, by `unlock`: `Power(level)` gives
  `RaisePower(level)` if the phone is below that level, and
  `NotOnThisDevice` if it already has it; the others give the matching
  upgrade.
- `AndroidAtLeast` / `WindowsBuildAtLeast`: an older OS can't be fixed, so the
  requirement is impossible. An unparseable version doesn't block anything.
- `Path`, `DeviceToggle`, `PowerAtLeast`: give the matching upgrade.

### How a state is chosen

1. All `requires` hold: **Available**.
2. Else, if a `partial` set holds: **Partial**. Part of the feature works, so
   this wins even when the rest never will; `upgrade` is the easiest unmet
   requirement of the full set, or none if one of them is impossible.
3. Else, if any requirement is impossible: **Unsupported**, with the first
   reason.
4. Else: **Locked**, with the lowest-effort upgrade among the unmet
   requirements (declaration order breaks ties).

A pair that isn't one phone (or tablet) and one desktop (or laptop) has every
feature Unsupported (`DeviceKinds`). An `Unknown` device kind falls back to
the OS (`android` → phone, `windows`/`macos` → desktop).

## Per-device toggles

`notifications`, `messages`, `calls`, `contacts`, `clipboard`, `files`,
`media`, `photos`, `recordings`, `toggles`, `pc_actions`, `mirroring` (on by
default) and `remote_input`, `commands`, `remote_files` (off by default). Toggles are this
device's policy for that peer and are stored locally, so a feature the PC
user switched off is Locked on the PC while the phone may still show it as
Available. Everything else in the matrix is identical on both devices.

## Examples (phone at Basic, all Basic permissions granted)

| Feature | Requires | Partial | State |
|---|---|---|---|
| `clipboard.pc_to_phone` | phone `clip.write`, toggle `clipboard` | – | Available |
| `clipboard.auto_phone_to_pc` | phone `clip.read.auto` (Elevated), toggle | phone `clip.share` → "Manual: tap to send" | Partial, upgrade → Elevated · ~2 min |
| `notifications.sensitive` | phone `notify.sensitive` (Elevated), toggle | – | Locked → Elevated · ~2 min |
| `mirroring.control` | phone `mirror.capture`, `mirror.input` (Assist) | – | Locked → Assist · ~1 min |
| `mirroring.app_windows` | phone `mirror.virtual_display` (Elevated), Elevated, Android ≥ 11 | – | Locked → Elevated |
| `camera.webcam` | phone `camera.stream` (camera permission), desktop `addon.vcam`, Windows build ≥ 22000 | – | Locked → install virtual camera · ~1 min |

The full registry is `FEATURES` in `core/nectarlink-core/src/features.rs`.

## UI contract

- Both UIs receive `CapabilityMatrix { device, features: Map<FeatureId, FeatureState> }`.
- **Available:** normal control.
- **Partial:** normal control plus an inline hint (`limit`) and, if present, an
  "Unlock" affordance.
- **Locked:** greyed control with a chip showing the level and effort (e.g.
  "Elevated · ~2 min"). Tapping it opens the setup sheet for `upgrade.action`,
  and on success returns to the feature.
- **Unsupported:** hidden in compact views, shown with the reason in settings.
