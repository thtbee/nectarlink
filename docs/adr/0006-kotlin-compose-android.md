# ADR 0006: Kotlin + Jetpack Compose for the Android app

- **Status:** Accepted
- **Date:** 2026-10-05

## Context

Nearly every feature depends on deep Android APIs (notification listener, telephony, Companion Device Manager, MediaProjection, CameraX, QS tiles, widgets).

## Decision

Write the Android app in Kotlin with Jetpack Compose and Material 3 Expressive, calling the Rust core through UniFFI.

## Consequences

- First-class access to every system API, best battery behavior.
- Two UI codebases (QML and Compose), unified by a shared design language rather than shared code.
