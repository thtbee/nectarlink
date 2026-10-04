# Capability matrix

The capability matrix decides, for every feature and every paired device,
whether the feature is **Available**, **Partial**, **Locked** or
**Unsupported**, and what the user can do about it. Both apps render their UI
from the same matrix, computed by `nectarlink-core`, so the phone and PC can
never disagree about what works.

## Inputs

| Input | Source |
|---|---|
| Peer capabilities | The peer's `hello.caps` (protocol §6) |
| Local capabilities | This device's own state (permissions, add-ons, OS version) |
| Power level | The phone's `hello.power` |
| Connection paths | Which paths are active (LAN, USB, relay) |
| Per-device permissions | The user's toggles for this paired device |
| Feature registry | Static definitions compiled into the core (below) |

The matrix is recomputed whenever any input changes and is pushed to the UI
as an event.

## Feature definitions

Each feature is declared once, in Rust, in `nectarlink-core::features`:

```rust
pub struct FeatureDef {
    pub id: FeatureId,                 // e.g. "clipboard.auto_phone_to_pc"
    pub group: FeatureGroup,           // Notifications, Clipboard, Files, Mirroring…
    /// Every requirement must hold for the feature to be Available.
    pub requires: &'static [Requirement],
    /// If `requires` fails, this weaker set may still give a Partial state.
    pub partial: Option<PartialDef>,
}

pub enum Requirement {
    PeerCap(&'static str),             // peer offers a capability ID
    LocalCap(&'static str),            // this device offers a capability ID
    PowerAtLeast(PowerLevel),          // Basic < Assist < Elevated
    AndroidApiAtLeast(u32),
    WindowsBuildAtLeast(u32),
    Path(ConnectionPath),              // e.g. Usb, Relay
    DeviceToggle(&'static str),        // per-device user permission
    Addon(&'static str),               // e.g. "vcam", "explorer", "hfp"
}

pub struct PartialDef {
    pub requires: &'static [Requirement],
    pub limit: &'static str,           // translation key, e.g. "clip.partial.manual"
}
```

## States

```rust
pub enum FeatureState {
    Available,
    /// Works, with a limitation shown inline, plus how to remove it.
    Partial { limit: MessageKey, upgrade: Option<Upgrade> },
    /// Doesn't work yet, but the user can unlock it.
    Locked { upgrade: Upgrade },
    /// Can't work on this device combination; explained, not actionable.
    Unsupported { reason: MessageKey },
}

pub struct Upgrade {
    pub action: UpgradeAction,         // which setup sheet to open
    pub effort: Effort,                // e.g. Minutes(2), Instant
}

pub enum UpgradeAction {
    RaisePower(PowerLevel),            // open "Choose your power" at that level
    GrantPermission(Permission),       // e.g. notification access
    EnableAddon(&'static str),         // e.g. install the virtual camera
    EnablePath(ConnectionPath),        // e.g. turn on Away mode
    EnableDeviceToggle(&'static str),  // flip a per-device permission
    UpdateApp(DeviceSide),             // peer or local app too old
}
```

### How a state is chosen

1. If any requirement is impossible on this device pair (OS version too old,
   hardware missing), the state is **Unsupported**.
2. Else, if all `requires` hold: **Available**.
3. Else, if a `partial` set holds: **Partial**, with an `upgrade` pointing at
   the first unmet requirement of the full set.
4. Else: **Locked**, with an `upgrade` for the first unmet requirement. When
   several requirements are unmet, the one with the lowest effort is offered
   first.

## Examples

| Feature | Requires | Partial | Typical state at Basic |
|---|---|---|---|
| `clipboard.pc_to_phone` | `PeerCap("clip.write")` | – | Available |
| `clipboard.auto_phone_to_pc` | `PeerCap("clip.read.auto")` (needs Elevated) | `PeerCap("clip.share")` → "Manual: tap to send" | Partial, upgrade → Elevated · ~2 min |
| `notifications.sensitive` | `PeerCap("notify.sensitive")` | – | Locked, upgrade → Elevated · ~2 min |
| `mirroring.app_windows` | `PeerCap("mirror.virtual_display")`, `PowerAtLeast(Elevated)` | – | Locked |
| `mirroring.control` | `PeerCap("mirror.input")` | – | Locked → Assist · ~1 min (or Elevated) |
| `camera.webcam` | `PeerCap("camera.stream")`, `Addon("vcam")`, `WindowsBuildAtLeast(22000)` | – | Locked → install virtual camera |

## UI contract

- Both UIs receive `CapabilityMatrix { device_id, features: Map<FeatureId, FeatureState> }`.
- **Available:** normal control.
- **Partial:** normal control plus an inline hint (`limit`) and, if present, an
  "Unlock" affordance.
- **Locked:** greyed control with a chip showing the level and effort (e.g.
  "Elevated · ~2 min"). Tapping it opens the setup sheet for `upgrade.action`,
  and on success returns to the feature.
- **Unsupported:** hidden in compact views, shown with the reason in settings.
