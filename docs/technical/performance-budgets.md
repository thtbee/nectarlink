# Performance Budgets

NectarLink treats startup latency, idle resource consumption, UI frame pacing, and bulk transfer throughput as regression-tested engineering budgets. Every metric below is taken from [`docs/PERFORMANCE.md`](../PERFORMANCE.md) and verified on hardware (`Windows 11 desktop + Samsung SM-S908E Android 14`).

## Target Budgets vs Measured Results

### Windows Desktop (`nectarlink-desktop`)

| Metric | Budget | Measured | Verification Method |
|---|---|---|---|
| Cold start to first visible frame | `< 800 ms` | **`310 ms - 424 ms`** (median `310 ms`) | `[PERF] cold_start_first_frame_ms` instrumentation in `desktop/app/src/main.rs` |
| Cold start (`--minimized` to system tray) | `< 400 ms` | **`145 ms`** | Process start to event-loop ready in tray mode |
| Idle CPU (minimized to tray, 60 s window) | `< 0.2%` | **`0.00%`** (`0 ms` CPU delta over 60 s) | Windows process CPU time counter (`Get-Process nectarlink-desktop`) |
| Idle CPU (window open, connected, 30 s) | `< 0.5%` | **`0.039%`** (`140.6 ms` CPU over 30 s) | Windows process CPU time counter |
| Idle working set (minimized to tray) | `< 50 MB` | **`14.2 MB`** (`54.0 MB` private bytes) | `EmptyWorkingSet` on `hide_to_tray()` + D3D11 resource release |
| Idle working set (window open, all pages visited) | `< 170 MB` | **`165.32 MB`** | Working set after cycling through all 14 navigation pages |
| Window resize responsiveness (20 steps) | `< 350 ms` CPU | **`296.9 ms`** CPU (`0 MB` working set leak) | Win32 `MoveWindow` 20-step sweep from `1040x680` to `1520x960` |
| Command Palette (`Ctrl+Alt+Space`) open + filter | `< 30 ms` | **`8.4 ms`** | Key press to filtered QML list layout completion |
| Phone-to-PC screen mirror glass-to-glass latency | `< 45 ms` | **`22 ms - 31 ms`** (`60 fps` 1080p H.264) | High-speed camera timestamp delta over LAN/USB |
| PC-to-phone audio streaming latency | `< 40 ms` | **`22 ms - 36 ms`** (`48 kHz` stereo) | WASAPI loopback capture → `NLPA` QUIC datagrams → Android `AudioTrack` |

### Scale Benchmarks (`nectarlink-core`)

The scale benchmarks move gigabytes of data and hold timing limits, so they are opt-in rather than part of every CI run. Measured with a release build on Windows 11 x64 (DPAPI key protection) on 2026-10-14:

```powershell
cargo test -p nectarlink-core --release --test scale_tests -- --ignored --nocapture
```

| Benchmark (`tests/scale_tests.rs`) | Limit in the test | Measured |
|---|---|---|
| `scale_4gb_file_transfer_with_pause_resume_interruption_and_blake3`: a 4 GiB file between two paired nodes, paused and resumed twice mid-stream, then BLAKE3 verified | Transfer `< 180 s` | **`16.0 s`** transfer, **`1.74 s`** BLAKE3 verify |
| `scale_10000_notifications_in_history_and_search`: 10,000 notifications ingested, a 200-item page listed and searched | List and search `< 100 ms` | **`13.8 ms`** ingest, **`0.15 ms`** list, **`2.43 ms`** search |
| `scale_5000_timeline_entries_and_200_chat_threads`: 5,000 timeline entries and 200 chat threads | Page and thread queries `< 50 ms`, search `< 100 ms` | **`0.89 ms`** page, **`4.60 ms`** search, **`0.37 ms`** threads, **`0.06 ms`** messages |
| `scale_50_large_clips_encrypted_history`: 50 clipboard items of 64 KiB each, encrypted at rest | Reopen, decrypt, list and search `< 250 ms` (release) | **`3.06 s`** to write all 50, **`182 ms`** to reopen and search |

### Android Client (`app.nectarlink.android`)

| Metric | Budget | Measured | Verification Method |
|---|---|---|---|
| Cold start (`TotalTime` via `am start -W`) | `< 1,000 ms` | **`479 ms - 750 ms`** | `adb shell am start -W -S app.nectarlink.android/.MainActivity` |
| Warm start (`TotalTime` via `am start -W`) | `< 100 ms` | **`14 ms - 30 ms`** | `adb shell am start -W app.nectarlink.android/.MainActivity` |
| Memory footprint (`TOTAL PSS`) | `< 110 MB` | **`75.66 MB - 86.48 MB`** | `adb shell dumpsys meminfo app.nectarlink.android` |
| Home tab scroll jank (`gfxinfo`) | `< 5.0%` | **`0.26%`** (`1 / 382` frames) | `adb shell dumpsys gfxinfo app.nectarlink.android` |
| Settings tab scroll jank (`gfxinfo`) | `< 5.0%` | **`4.09%`** (`11 / 269` frames, `p95 = 11 ms`) | `adb shell dumpsys gfxinfo app.nectarlink.android` |
| Idle background CPU (screen off, connected) | `< 0.5%` | **`0.0% - 0.1%`** | `adb shell top -n 1 -b` |

## Key Engineering Optimizations

The budgets above are enforced through architectural choices across the Rust core, Qt desktop client, and Android app:

### 1. Asynchronous Runtime Bootstrap (`desktop/app/src/main.rs`)

The Qt `QGuiApplication` and QML engine initialize on the main thread while `nectarlink_core::Node::start` (SQLite migration, DPAPI unsealing, `iroh` QUIC socket binding, mDNS discovery, and ADB watcher startup) runs concurrently on the background Tokio runtime. The first QML frame paints in `~310 ms` before network sockets finish binding, and the `NodeBridge` attaches via an asynchronous channel once ready.

### 2. Lazy QML Loading (`Loader` Pattern in `App.qml`)

Rather than instantiating every page at startup, `desktop/app/qml/App.qml` loads the main window only when it is shown, and `MainWindow.qml` loads pages (`MessagesPage`, `CallsPage`, `PhotosPage`, `SettingsPage` and the rest) when they are opened on demand through QML `Loader` components.

### 3. Tray Working-Set Trimming (`hide_to_tray`)

When the user closes the window to the system tray (`trim_working_set` in `desktop/app/src/bridge/app.rs`):

- QML releases cached GPU textures and offscreen buffers (`releaseResources()`).
- On Windows the process calls `SetProcessWorkingSetSize(GetCurrentProcess(), -1, -1)` (`desktop/app/cpp/app_helpers.cpp`), trimming the resident working set from `~165 MB` down to **`14.2 MB`** while keeping the background Tokio QUIC node responsive.

### 4. Event-Driven Zero-Polling Subsystems

To keep tray idle CPU at **`0.00%`**:

- **Send Queue Coordinator (`core/nectarlink-core/src/node.rs`)**: Uses `tokio::sync::Notify` (`queue_notify`) plus calculated `tokio::time::sleep_until` deadlines (`Store::next_queue_wake_ms`); when no items are waiting, the task parks indefinitely on `std::future::pending()`.
- **ADB USB Watcher (`core/nectarlink-adb` & `core/nectarlink-core/src/usb.rs`)**: Maintains a single long-lived TCP connection to `127.0.0.1:5037` using the event-driven `host:track-devices` stream instead of spawning `adb.exe` on a timer.
- **QML Animations**: Indeterminate spinners and pulse animations in QML bind `running` to item visibility and active window state so Qt's scene graph ticks `0` frames per second when minimized or on another tab.

### 5. Off-UI-Thread Android Service Startup (`MainActivity.kt` & `ConnectionService.kt`)

- `NectarlinkNode` creation, Android Keystore AES-GCM unsealing, and SQLite initialization run on `Dispatchers.IO` inside `ConnectionService`.
- Compose lists (`HomeTab`, `SettingsTab`, `SendQueueSection`) pre-compute formatted timestamps and strings outside `LazyColumn` item scopes and supply stable `key` lambdas to avoid layout recomposition jank during flings.
