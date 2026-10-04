# Spike S1: Qt Quick + Rust proof

Throwaway code. What we keep is the knowledge below.

Run (needs Qt 6.12 on PATH and `QMAKE` set):
`cargo run --release -p s1-qt-rust` · `-- --autotest` (self-driving measurement) · `-- --mica`

## Results (2026-10-05, this machine, 144 Hz display, release build)

| Criterion | Budget | Result | |
|---|---|---|---|
| First frame | < 500 ms | ~400 ms | ✅ |
| RAM, main window open | < 120 MB | ~88 MB idle, ~104 MB under 10k events/s | ✅ |
| Smoothness under load | 120 fps | 144 fps avg, 143 min (display max) at 10,000 core events/s + hero transition | ✅ |
| Core→UI events | 10k/s without stutter | ✅ with coalescing (see below) | ✅ |
| Bloom design fidelity | matches mockups | Home, notification cards, Deck, shared-element conversation transition, real blur (MultiEffect) | ✅ |
| Mica | works | ⏳ DWM calls succeed and window alpha works, but the backdrop shows white: needs Qt-side presentation work | follow-up |
| Video in scene graph (zero copy) | works | ⏳ not started | follow-up (with S2) |
| ARM64 build | CI builds | ⏳ not tried | follow-up |
| Tray-only RAM < 60 MB | | ⏳ not measured (no tray mode yet) | follow-up |

## Lessons (apply to the real `nectarlink-qt`)

1. **Never queue one closure per core event.** It ballooned to 5.7 GB at 10k/s.
   Use a coalescing pump: latest state in a shared slot, at most one queued
   flush, non-mergeable events batched and bounded (`UiPump` in `src/bridge.rs`).
2. **MSVC needs `/utf-8`** or non-ASCII text in qmlcachegen output is garbled
   (set repo-wide in `.cargo/config.toml`).
3. **`#[auto_cxx_name]`** on `extern "RustQt"` blocks gives QML camelCase names.
4. **QML can't have properties named `onX`**: Material token `onSurface` becomes
   `surfaceContent` in QML (token generator must map this).
5. `font.pixelSize` must be an integer.
6. Windows sends Qt logs to the debugger; set `QT_FORCE_STDERR_LOGGING=1` when diagnosing.

## Verdict

**Gate passed for the UI stack** (looks, smoothness, memory, startup, event throughput).
Mica, the zero-copy video item, ARM64 and tray-mode memory remain to verify.
