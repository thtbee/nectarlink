# ADR 0008: Platforms and distribution

- **Status:** Accepted
- **Date:** 2026-10-05

## Context

Focus beats breadth. Windows 10 is out of support, and store policies (Play) restrict SMS, accessibility and file access.

## Decision

Windows 11 only (x64 + ARM64) and Android 8+. Distribute GitHub-first: GitHub Releases + winget on Windows, GitHub Releases + Obtainium (later IzzyOnDroid) on Android. Stores and macOS come later.

## Consequences

- Free to use Win11 APIs (Mica, virtual camera, Win11 context menu).
- No Play policy constraints in the `full` flavor; a `play` flavor seam is kept.
- Android developer verification must be completed before global enforcement (2027).
