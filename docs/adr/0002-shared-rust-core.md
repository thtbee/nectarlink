# ADR 0002: Shared Rust core for both apps

- **Status:** Accepted
- **Date:** 2026-10-05

## Context

Competitors that implement the protocol twice (e.g. KDE Connect's separate desktop and Android code) suffer interop bugs. We need one implementation of networking, pairing, transfers and storage, that is fast, memory-safe for parsing untrusted input, and portable to Windows, Android and later macOS.

## Decision

All protocol, pairing, session, transfer and storage logic lives in Rust crates under `core/`. The Windows app links them directly; the Android app uses them via UniFFI bindings.

## Consequences

- One source of truth; interop bugs between our own apps are impossible by construction.
- Android APK grows by ~5–8 MB per ABI (ship arm64 primarily).
- Contributors need Rust; Rust compile times are slower.
