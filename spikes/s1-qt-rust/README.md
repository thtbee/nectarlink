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
| Mica | works | Works on D3D11 with no cost: 144 fps under load, same memory (see lesson 7) | ✅ |
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
7. **Mica on Qt Quick** needs four things: `QQuickWindow::setDefaultAlphaBuffer(true)`
   and `QT_QPA_DISABLE_REDIRECTION_SURFACE=1` before the app starts (without
   it D3D swap chains don't use DirectComposition and transparent pixels are
   white), a transparent root `Window`, then `DwmExtendFrameIntoClientArea`
   + `DWMWA_SYSTEMBACKDROP_TYPE`. OpenGL also works but costs ~100 MB more;
   stay on D3D11.
8. **Pin `Qt.styleHints.colorScheme` to the app theme.** Qt otherwise follows
   the system, so a light theme on a dark-mode PC gets a dark frame and dark
   Mica tint.
9. **Drop the engine and app before `process::exit`**, or Qt's render and
   vsync threads are torn down mid-flight and log warnings at exit.

## Verdict

**Gate passed for the UI stack** (looks, smoothness, memory, startup, event throughput).
The zero-copy video item, ARM64 and tray-mode memory remain to verify. The
real app also needs a custom title bar so Mica runs edge to edge (the system
caption is drawn in the accent color here).
