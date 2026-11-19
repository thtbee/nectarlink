# NectarLink Technical Documentation

This directory contains engineering documentation for contributors, security reviewers, and systems engineers working on NectarLink. It covers the multi-crate Rust workspace, the Windows Qt 6 + `cxx-qt` desktop application, the Android Jetpack Compose + UniFFI client, the QUIC + TLS 1.3 protocol, the SQLite persistence layer, and the performance verification suite.

For end-user setup and feature guides, see [docs/guide/README.md](../guide/README.md).

## Contents

| Document | Scope |
|---|---|
| [Architecture](architecture.md) | Crate layout, runtime topology, discovery and transport (`Lan`, `Usb`, `Relay`), Android elevated helper, Windows Media Foundation virtual camera, and perceptual OKLab color engine |
| [Security Model](security-model.md) | Ed25519 device identity, OS key protection (Windows DPAPI tied to user account and Android Keystore AES-256-GCM), QR and 6-digit pairing protocols, `CapabilityMatrix`, consent gates, update checks, and localhost HTTP API isolation |
| [Protocol Overview](protocol-overview.md) | `nectarlink/0` wire framing (LEB128 + CBOR), QUIC stream taxonomy, control handshake, resumable BLAKE3 file transfers, real-time media framing, and index of the 18 specification files in [`docs/protocol/`](../protocol/v0.md) |
| [Storage and Retention](storage-and-retention.md) | SQLite schema v8 (`core/nectarlink-core/src/store.rs`), timeline and `MessagingStyle` conversation caches, persistent FIFO send queue (`send_queue`), and `KeyProtector`-encrypted clipboard history |
| [Performance Budgets](performance-budgets.md) | Startup, memory, CPU, transport, and latency targets from [`docs/PERFORMANCE.md`](../PERFORMANCE.md), measured benchmarks, and key root-cause engineering optimizations |
| [Building and Testing](building-and-testing.md) | Pinned toolchains (`Rust 1.97+`, `Qt 6.12`, `Android compileSdk 37 / minSdk 26`, `JDK 17+`), `cargo xtask` automation, PowerShell verification scripts, and test suites |
| [Releasing](releasing.md) | Versioning, the tag-driven release workflow, Android and Windows signing, and how both apps update themselves |
| [Contributing](contributing.md) | Coding conventions, licensing (`MPL-2.0`, `GPL-3.0-or-later`, `CC BY 4.0`), module header policy, protocol evolution rules, and pre-submission checks |

## Repository Map

```text
nectarlink/
├── Cargo.toml                  Workspace manifest and shared dependency versions
├── rust-toolchain.toml         Exact Rust toolchain (1.100.0; MSRV 1.97, 2024 edition); builds add their targets
├── core/
│   ├── nectarlink-protocol/    Wire framing, CBOR envelopes, capability constants, SAS derivation
│   ├── nectarlink-core/        QUIC node, ADB/USB tunnel, session engine, SQLite store, send queue, OKLab palette
│   └── nectarlink-ffi/         UniFFI 0.32 bindings exposing nectarlink-core to Kotlin
├── cli/                        Headless CLI (nectarlink-cli) for pairing, file transfer, notify-when, and testing
├── desktop/
│   ├── app/                    Windows desktop app (Rust + cxx-qt 0.10 bridges + Qt 6.12 QML)
│   ├── vcam/                   Windows Media Foundation virtual camera COM DLL (nectarlink_vcam.dll)
│   └── installer/              NSIS installer script (nectarlink.nsi) and Windows Sparse Package manifest (AppxManifest.xml)
├── android/
│   └── app/                    Android client (Kotlin, Jetpack Compose, foreground service, elevated helper)
├── tools/
│   └── xtask/                  Design token codegen and third-party license notice generator
├── scripts/                    PowerShell check, header verification, and packaging scripts
└── docs/
    ├── guide/                  User-facing setup and feature documentation
    ├── technical/              Engineering architecture, security, storage, and build reference
    ├── protocol/               Normative wire protocol specifications (18 documents)
    ├── adr/                    Architecture decision records
    ├── PERFORMANCE.md          Measured performance report and verification commands
    └── SECURITY-REVIEW.md      Security audit findings and hardening verification
```

## Related Specification Directories

- [`docs/protocol/`](../protocol/v0.md): Normative wire-level specifications for `nectarlink/0` and `nectarlink-pair/0`, plus per-capability message schemas.
- [`docs/adr/`](../adr/README.md): Architecture decision records covering the shared Rust core (`0002`), `iroh` QUIC transport (`0003`), CBOR wire framing (`0004`), the Windows Qt Quick UI stack (`0005`), Kotlin + Jetpack Compose on Android (`0006`), licensing (`0007`), and platform distribution (`0008`).
- [`docs/architecture/`](../architecture/capabilities.md): Capability matrix reference and core Rust API design notes.
- [`docs/design/`](../design/README.md): Visual design system tokens, typography, and motion guidelines.
