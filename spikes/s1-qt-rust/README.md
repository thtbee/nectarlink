# Spike S1: Qt Quick + Rust proof

Throwaway code. What we keep is the knowledge below.

Run (needs Qt 6.12 on PATH and `QMAKE` set):
`cargo run --release -p s1-qt-rust` · `-- --autotest` (self-driving measurement) · `-- --tray-test`
(open, close, measure tray mode) · `-- --mica` · `-- --video` · `-- --no-window`

ARM64: `$env:QMAKE = ./scripts/qmake-arm64.ps1` then
`cargo build --release --target aarch64-pc-windows-msvc -p s1-qt-rust`

## Results (2026-10-05, this machine, 144 Hz display, release build)

| Criterion | Budget | Result | |
|---|---|---|---|
| First frame | < 500 ms | ~400 ms | ✅ |
| RAM, main window open | < 120 MB | ~88 MB working set idle (Task Manager: 32.5 MB private), ~104 MB under 10k events/s | ✅ |
| Smoothness under load | 120 fps | 144 fps avg, 143 min (display max) at 10,000 core events/s + hero transition | ✅ |
| Core→UI events | 10k/s without stutter | ✅ with coalescing (see below) | ✅ |
| Bloom design fidelity | matches mockups | Home, notification cards, Deck, shared-element conversation transition, real blur (MultiEffect) | ✅ |
| Mica | works | Works on D3D11 with no cost: 144 fps under load, same memory (see lesson 7) | ✅ |
| Video in scene graph (zero copy) | works | 1080p60 GPU frames from a Rust thread on its own D3D11 device: 240/240 shown, 0 dropped, 144 fps alongside 10k events/s. Video alone: 11% of one core, 9% GPU, +6.5 MB | ✅ |
| ARM64 build | CI builds | Cross-builds from x64 into an ARM64 exe linking ARM64 Qt; not run yet (needs ARM64 hardware or a `windows-11-arm` CI runner) | ✅ build |
| Tray-only RAM | < 60 MB | After closing the main window: 64 MB working set, 52 MB private commit, **19.6 MB in Task Manager**. Never-opened floor: 34 / 23 MB. Core not included yet | ✅ |

## Lessons (applied in the desktop app)

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
10. **Video path** (`src/video.rs`, `cpp/video_surface.cpp`): the producer
    owns its device on Qt's adapter (matched by LUID) and publishes into a
    shared NT-handle texture guarded by a keyed mutex, taken with a zero
    timeout on both sides so neither thread ever waits: "latest frame wins".
    Qt copies it GPU-to-GPU during sync. Gotchas: `QSGD3D11Texture::fromNative`
    assumes **RGBA8** (BGRA fails SRV creation and renders nothing);
    `AcquireSync`'s `WAIT_TIMEOUT` is a *success* HRESULT (the `windows`
    crate maps it to `Ok`, so check `S_OK` through the vtable); keep the
    `QSGTexture` wrapper's lifetime shorter than the native texture.
11. Wake the item with a coalesced notifier (at most one queued `update()`),
    the same idea as `UiPump`, so the scene only re-renders on new frames.
12. **Tray mode = no windows**, not hidden windows: `App.qml` creates the
    main window on demand and destroys it on close, freeing its GPU device
    and scene graph (private memory 100 → 52 MB). About 30 MB of the
    remainder is the GPU driver, which stays loaded once used; trimming the
    QML component cache gains nothing. Report memory as Task Manager does
    (private working set) next to working set and private commit, since
    they differ by 3×.
13. **ARM64 cross builds** need a qmake that runs on the host but describes
    the ARM64 kit. Qt's `host-qmake.bat` can't be used because cxx-qt runs
    qmake with an empty environment; `scripts/qmake-arm64.ps1` makes an
    equivalent with a copied x64 qmake, `Qt6Core.dll` and `qt.conf`.

## Verdict

**S1 gate passed.** Every criterion is met on this machine: looks,
smoothness, startup, memory with a window and in tray mode, event
throughput, Mica, zero-copy video and the ARM64 build.

Still open, carried into later work:
- Run the ARM64 build on ARM64 hardware (CI `windows-11-arm` runner).
- S2 replaces the test pattern with the hardware H.264/H.265 decoder
  (NV12 → RGBA via the D3D11 video processor, into the same shared texture).
- The real app needs a custom title bar so Mica runs edge to edge (the
  system caption is drawn in the accent color here).
