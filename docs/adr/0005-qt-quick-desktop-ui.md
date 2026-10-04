# ADR 0005: Qt Quick (QML) + Rust for the Windows app

- **Status:** Accepted
- **Date:** 2026-10-05

## Context

The desktop app runs all day, is video-heavy (mirroring, one window per phone app) and needs a very high visual bar. The maintainer ruled out web stacks (RAM) and Flutter. We scored Tauri+React (pure and hybrid), Qt Quick + Rust and Slint (see PLAN.md §5.1).

## Decision

Build the desktop UI in Qt Quick (QML) with all logic in Rust, bridged by cxx-qt through a single crate (`nectarlink-qt`). Mirrored video is decoded with Media Foundation into D3D11 textures shown directly in Qt's scene graph.

## Consequences

- Visual ceiling equal to web (blur, shaders, fluid motion) at a fraction of the RAM.
- Heavier build: Qt toolchain, a little C++, pre-1.0 cxx-qt.
- Rules: QML is presentation only; use only LGPL Qt modules; track the latest Qt 6 minor release.
- Fallback if the Phase 0 proof fails: web UI (Tauri) with native Rust video windows.
