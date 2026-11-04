# Contributing

This guide summarizes the repository conventions, module header requirements, protocol change rules, and pre-submission checks for contributing to NectarLink. See also the root [`CONTRIBUTING.md`](../../CONTRIBUTING.md) and [`LICENSE`](../../LICENSE).

## License

NectarLink uses three licenses across the repository (see [`LICENSE`](../../LICENSE)):
- **Core libraries** (`core/nectarlink-protocol`, `core/nectarlink-core`, `core/nectarlink-ffi`): [`MPL-2.0`](../../LICENSES/MPL-2.0.txt)
- **Applications and CLI** (`desktop/app`, `desktop/vcam`, `android`, `cli`): [`GPL-3.0-or-later`](../../LICENSES/GPL-3.0-or-later.txt) with an [`app-store exception`](../../LICENSES/APP-STORE-EXCEPTION.md)
- **Protocol specification and documentation** (`docs/`): [`CC BY 4.0`](../../LICENSES/CC-BY-4.0.txt)

Contributions are accepted under the [`Developer Certificate of Origin`](../../DCO.txt).

## SPDX Header Policy

Every Rust (`.rs`), Kotlin (`.kt`), C++ (`.cpp`, `.h`), and QML (`.qml`) source file must start with the appropriate `SPDX-License-Identifier` comment:

```rust
// SPDX-License-Identifier: MPL-2.0
```

or for application and CLI files:

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
```

- Verified automatically by [`scripts/check-headers.ps1`](../../scripts/check-headers.ps1) and [`scripts/check.ps1`](../../scripts/check.ps1).
- Never log message text, phone numbers, contact names, file paths, or clipboard contents (see [`docs/protocol/v0.md`](../protocol/v0.md) section 11).

## Code Style and Static Analysis

### Rust

- **Formatting**: Format all Rust code with `cargo fmt --all`. Configuration lives in [`rustfmt.toml`](../../rustfmt.toml).
- **Lints**: Workspace-wide lints are configured in the root [`Cargo.toml`](../../Cargo.toml) (`unsafe_op_in_unsafe_fn = "warn"`, `dbg_macro = "warn"`, `todo = "warn"`). All code must pass `cargo clippy --workspace --all-targets -- -D warnings` with zero warnings.
- **Error Handling**:
  - Use typed `thiserror` enums (`CoreError`, `FrameError`, `PairError`) across `nectarlink-protocol` and `nectarlink-core` public APIs.
  - Never unwrap fallible network, CBOR, file I/O, or SQLite operations on untrusted peer inputs.
- **`unsafe` Blocks**:
  - Avoid `unsafe` in `nectarlink-protocol` and `nectarlink-core`.
  - Where Win32/COM/FFI calls require `unsafe` (such as `DpapiKeyProtector`, `desktop/vcam` COM interfaces, or `desktop/app` Win32/WASAPI/DXGI interop), keep the `unsafe` block minimal and document the pointer/lifetime invariants.

### Kotlin (Android)

- Follow standard Kotlin conventions (`kotlin.code.style=official` in `android/gradle.properties`).
- Run blocking disk, SQLite, or FFI calls on `Dispatchers.IO`, never on the main UI thread.
- Keep `Compose` list items free of per-frame allocations (such as `SimpleDateFormat` creation inside `LazyColumn` item blocks) to preserve the `< 5%` scroll jank budget.

### QML (Windows Desktop)

- Read colors, spacing, radii, and typography exclusively from the `Theme` singleton (`desktop/app/qml/Theme.qml`, built on the generated `Tokens.qml`) rather than hardcoding hex literals.
- Gate continuous animations (`NumberAnimation`, `RotationAnimator`) on item visibility and active window state so minimized tray mode consumes `0.00%` CPU.

## Adding or Modifying a Capability

When introducing a new cross-device capability or extending an existing wire message:

1. **Specification First (`docs/protocol/`)**: Document the capability ID, stream framing, CBOR message schemas, and error handling in [`docs/protocol/`](../protocol/v0.md) and register the capability in [`docs/protocol/capabilities.md`](../protocol/capabilities.md).
2. **Protocol Constants & Types (`core/nectarlink-protocol`)**:
   - Add the capability's name next to its message types in `src/messages.rs`, and list it in [`docs/protocol/capabilities.md`](../protocol/capabilities.md).
   - Define the serde-serializable structs in `src/messages.rs` using `#[serde(default)]` / `#[serde(skip_serializing_if = "Option::is_none")]` so wire changes remain backward-compatible.
3. **Authorization Mapping (`core/nectarlink-core/src/features.rs`)**:
   - Gate it on an existing or new toggle in `DEVICE_TOGGLES` (`Shared::toggle_on` in the handler), and add it to the matrix in `compute`.
   - Decide whether the feature is default-on or high-sensitivity opt-in (`default_enabled: false`), and whether it must be restricted over `ConnectionPath::Relay` (**Connect over mobile data** / `away_mode`).
4. **Core & FFI Plumbing (`core/nectarlink-core`, `core/nectarlink-ffi`)**:
   - Implement the handler in `nectarlink-core` and enforce `self.capabilities.is_allowed(&peer, cap::...)` on both inbound and outbound paths.
   - If Android needs the API, expose it in `core/nectarlink-ffi/src/lib.rs` and run `android\gradlew.bat generateUniffiBindings` (or `cargo run -p nectarlink-ffi --bin uniffi-bindgen`).
5. **Two-Device Integration Test (`core/nectarlink-core/tests/two_devices.rs`)**:
   - Add an end-to-end test spinning up two in-process `Node` instances over QUIC to verify capability negotiation, happy-path execution, and toggle denial behavior.

## Database Migrations (`core/nectarlink-core/src/store.rs`)

If a change alters the SQLite schema:

1. Increment `SCHEMA_VERSION` in `core/nectarlink-core/src/store.rs`.
2. Add an idempotent migration step in `Store::migrate` (using `ALTER TABLE` or `CREATE TABLE IF NOT EXISTS`) so existing user databases upgrade without data loss.
3. Add a unit test in `store.rs` verifying both fresh database creation and migration from the previous version.

## Pre-Submission Checklist

Before opening a pull request or merging a change, run the full verification script from the repository root:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\check.ps1
```

Verify that:
- `cargo fmt --all -- --check` produces no diff.
- `cargo clippy --workspace --all-targets -- -D warnings` exits cleanly.
- `cargo test --workspace` passes all unit and integration tests.
- `scripts/check-headers.ps1` reports `OK`.
- `cargo xtask tokens --check` confirms generated design token files are in sync.
- `android/gradlew.bat compileDebugKotlin` succeeds when Android or FFI code is touched.
