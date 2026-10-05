# Phase 0: Foundations — execution plan

Goal: prove the risky parts early and build the skeleton every later feature
plugs into. Phase 0 ends when the exit criteria at the bottom are met.

## 1. Workstreams and order

```
 W1 Design ───────────► directions picked ──► hi-fi mockups ─────────────┐
 W2 Repo & CI ──► scaffold ──► CI green                                  │
 W3 Core ─────────────► identity ► pairing ► session ► events ──┐        │
 S1 Qt + Rust proof (the gate) ──────────────────────► pass? ───┼──► W4 Desktop shell
 S2–S7 other spikes (parallel, timeboxed) ──────────────────────┘    W5 Android shell
```

1. **W2 Repo & CI** and **W1 Design directions** start together.
2. **S1 Qt + Rust proof** starts right after the scaffold, because it decides
   the desktop stack.
3. **W3 Core** proceeds in parallel; it doesn't depend on the UI choice.
4. **W4/W5 shells** start once S1 passes and a design direction is picked.
5. **S2–S7** run alongside, each timeboxed. A failed spike changes the plan,
   it doesn't block Phase 0.

## 2. Repository scaffold (W2)

```
nectarlink/
  Cargo.toml                 Rust workspace (core + desktop crates)
  rust-toolchain.toml        pinned stable toolchain
  core/
    nectarlink-protocol/     message types, framing, CBOR codec       (MPL-2.0)
    nectarlink-core/         Node, identity, pairing, sessions, store  (MPL-2.0)
    nectarlink-ffi/          UniFFI bindings                           (MPL-2.0)
    nectarlink-cli/          dev/debug CLI: pair, ping, status         (GPL)
  desktop/
    app/                     the desktop app, one crate               (GPL)
      src/bridge/            cxx-qt bridge: the only Rust↔Qt code
      src/win/               Windows integration (tray, power, sound)
      cpp/                   C++ for Qt hooks (window chrome)
      qml/                   design system + screens
  tools/xtask/               code generation (theme tokens) and checks (GPL)
  android/                   Gradle project (app module)               (GPL)
  spikes/                    throwaway experiments, one folder each
  docs/
  scripts/                   build helpers (PowerShell + bash)
```

**CI (GitHub Actions, runs once we push; until then the same scripts run
locally):**

| Job | Runs on | Does |
|---|---|---|
| `core` | windows-latest, ubuntu-latest | `cargo fmt --check`, `clippy -D warnings`, `cargo test`, fuzz smoke test |
| `desktop` | windows-latest | Install Qt 6.12 (aqtinstall, cached), build x64 + ARM64, run QML tests |
| `android` | ubuntu-latest | `cargo ndk` build of `nectarlink-ffi`, Gradle build, unit tests |
| `licenses` | ubuntu-latest | SPDX header check, dependency license audit (`cargo deny`) |

Local equivalent: `scripts/check.ps1` runs the same steps on this machine.

## 3. Core milestones (W3)

| # | Milestone | Done when |
|---|---|---|
| C1 | Protocol crate | Framing + envelope encode/decode with unit tests and a fuzz target; rejects oversized frames |
| C2 | Identity & store | Device key created/loaded and encrypted at rest; SQLite trust store with migrations |
| C3 | Pairing | QR flow and nearby (SAS) flow per protocol §9, with tests including a simulated man-in-the-middle that must fail |
| C4 | Sessions | Paired devices connect over iroh, `hello` exchange, version negotiation, `ping`/`pong`, reconnect with backoff |
| C5 | Events & capabilities | Battery and device events, capability matrix computed and emitted |
| C6 | CLI | `nectarlink-cli pair`, `status`, `ping`, `ring` work between two machines (or two processes) |

## 4. Spikes

Each spike gets a folder in `spikes/`, a short README with results, and a
verdict. Spike code is throwaway; what we keep is the knowledge.

| # | Spike | Question | Pass criteria | If it fails |
|---|---|---|---|---|
| **S1** | **Qt + Rust proof** (gate) | Can Qt Quick + cxx-qt deliver the design at our budgets? | Home screen, notification card, Deck grid, one hero transition and a live video window built from the mockups; 120 fps on a mid-range laptop; RAM < 60 MB tray / < 120 MB window; cold start < 500 ms; D3D11 video in the scene graph without copies; Mica works; 10k core→UI events/s without UI stutter; CI builds x64 + ARM64 | Fall back to web UI (Tauri) with native Rust video windows (ADR 0005) |
| S2 | Mirror latency | Phone encoder → QUIC → MF decode → screen | ≤ 50 ms glass-to-glass at 1080p60 on LAN, measured with a high-speed camera or on-screen timestamps | Tune encoder/transport; dedicated swapchain for video |
| S3 | Bluetooth calling | Can a third-party app act as an HFP hands-free device on Windows 11 24H2/25H2? | Answer a real call and hear audio on the PC, with package identity | Calls stay "alert + control", audio on the phone |
| S4 | iroh on Android | Battery cost and reconnect behavior | < 2 %/day idle while connected; reconnect < 3 s after Wi-Fi switch, screen off, Doze | Tune keep-alives; lean on CDM wake |
| S5 | USB transport | iroh custom transport (or TCP tunnel) over ADB | Session runs over USB with Wi-Fi off; ≥ 30 MB/s file throughput | TCP-framed tunnel fallback |
| S6 | Toasts & identity | Sparse package identity + toasts with inline reply from an unpackaged installer | Toast with image, buttons and a reply box; reply text reaches the app, also when it was not running | Simpler toasts; reply opens the app |
| S7 | Elevated self-pairing | Phone-only wireless-debugging pairing and re-arm after reboot | Pair from a notification on Android 11–16 (Pixel, Samsung, Xiaomi); helper process starts as `shell`; re-arm works where the OEM allows | Rely on PC-side re-arm and Shizuku |

## 5. Design (W1)

1. Three visual directions (`docs/design/directions/`): pick one, or mix.
2. Design tokens: color (light and dark), type scale, spacing, radii,
   elevation, motion curves (springs).
3. Hi-fi mockups of Home, Onboarding, Choose-your-power, Notifications,
   Messages, Mirror window and Deck, for both apps.
4. The ~30-component inventory for QML and Compose.

## 6. Exit criteria

- [ ] Phone and PC pair by QR in under 60 seconds.
- [ ] They show each other live (name, battery) and survive PC sleep/wake and
      Wi-Fi changes.
- [ ] S1 passed (or the fallback was chosen and documented as an ADR).
- [ ] Every other spike has a written verdict.
- [ ] Design direction chosen, tokens and core mockups approved.
- [ ] `scripts/check.ps1` is green locally.
