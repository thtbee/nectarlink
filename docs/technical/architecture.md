# Architecture

NectarLink is built around a shared Rust core library (`nectarlink-core`) that implements peer discovery, cryptographic identity, QUIC session management, capability negotiation, persistent storage, and media/file framing. Native platform shells on Windows (`desktop/app`) and Android (`android/app`) bind directly to this core and provide OS integration and UI rendering.

## System Topology

```text
+-------------------------------------------------------------------------------+
| Windows or Linux PC                                                           |
|                                                                               |
|  +-------------------------------------------------------------------------+  |
|  | desktop/app (nectarlink-desktop)                                        |  |
|  |  +---------------------------+   +-----------------------------------+  |  |
|  |  | Qt 6.12 QML UI            |   | Windows OS Integrations           |  |  |
|  |  | (desktop/app/qml/*.qml)   |   | WASAPI, Media Foundation, WinRT,  |  |  |
|  |  +-------------+-------------+   | DXGI Desktop Duplication, SendInput|  |  |
|  |                | cxx-qt 0.10     +-----------------+-----------------+  |  |
|  |  +-------------v-----------------------------------v-----------------+  |  |
|  |  | Rust Bridge & Services (desktop/app/src/bridge/, services)        |  |  |
|  |  +-------------+-----------------------------------+-----------------+  |  |
|  |                |                                   | Shared Memory      |  |
|  |                |                                   | NLVC v2            |  |
|  |                |                     +-------------v-----------------+  |  |
|  |                |                     | desktop/vcam                  |  |  |
|  |                |                     | (nectarlink_vcam.dll IMFMedia)|  |  |
|  |                |                     +-------------------------------+  |  |
|  |  +-------------v-----------------------------------------------------+  |  |
|  |  | core/nectarlink-core (incl. adb.rs & usb.rs) + nectarlink-protocol|  |  |
|  |  +-------------+-------------------+-------------------+-------------+  |  |
|  +----------------|-------------------|-------------------|----------------+  |
+-------------------|-------------------|-------------------|-------------------+
                    | LAN QUIC          | USB ADB TCP       | Relay QUIC
                    | (UDP + mDNS)      | (127.0.0.1:53319) | (away_mode)
+-------------------v-------------------v-------------------v-------------------+
| Android Device                                                                |
|                                                                               |
|  +-------------------------------------------------------------------------+  |
|  | android/app (app.nectarlink.android)                                    |  |
|  |  +-------------------------------------------------------------------+  |  |
|  |  | core/nectarlink-ffi (UniFFI 0.32 → libnectarlink_ffi.so)          |  |  |
|  |  | embeds core/nectarlink-core + core/nectarlink-protocol            |  |  |
|  |  +-------------+-----------------------------------+-----------------+  |  |
|  |                | Kotlin Callbacks / Events         | Loopback TCP      |  |
|  |  +-------------v-----------------+   +-------------v-----------------+  |  |
|  |  | Jetpack Compose UI &          |   | Elevated Helper (app_process) |  |  |
|  |  | ConnectionService (Foreground)|   | InputServer / AppDisplay      |  |  |
|  |  | NotificationListener, Camera2 |   | (UID 2000 shell via ADB TLS)  |  |  |
|  |  +-------------------------------+   +-------------------------------+  |  |
|  +-------------------------------------------------------------------------+  |
+-------------------------------------------------------------------------------+
```

## Workspace Crates and Components

### 1. `core/nectarlink-protocol`

Pure wire-protocol crate with no runtime IO or platform dependencies beyond `tokio::io::{AsyncRead, AsyncWrite}` for stream framing.

- **Envelope and Framing (`src/frame.rs`, `src/messages.rs`)**: Implements unsigned LEB128 length-prefixed CBOR `Envelope` encoding (`ciborium`), with a 1 MiB frame ceiling (`MAX_FRAME_LEN = 1,048,576` bytes).
- **Capabilities (`src/messages.rs`)**: Each service's capability names (`files.transfer`, `sms.read`, `mirror.capture` and so on) live with its message types. [`docs/protocol/capabilities.md`](../protocol/capabilities.md) lists them all.
- **Pairing Primitives (`src/pairing.rs`)**: Defines the `nectarlink://pair` URI parser/serializer, `ALPN_PAIR` (`"nectarlink-pair/0"`), `ALPN_SESSION` (`"nectarlink/0"`), QR HMAC proof calculation, and 6-digit Short Authentication String (SAS) derivation.
- **Video Packetizer (`src/video.rs`)**: Encodes and decodes real-time video packets (`VideoKind::Config`, `VideoKind::Frame`, `VideoKind::Keyframe`) with microsecond timestamps and an 8 MiB packet limit (`MAX_VIDEO_PACKET`).

### 2. `core/nectarlink-core`

The central networking, USB/ADB transport, persistence, and feature runtime.

- **Node and Session Engine (`src/node.rs`, `src/session.rs`)**: Manages the `iroh::Endpoint`, accepts incoming `ALPN_PAIR` and `ALPN_SESSION` connections, runs the `hello` / `hello.update` capability handshake, maintains per-peer connection state, and multiplexes control messages, bulk streams, and QUIC datagrams.
- **Zero-Shell-Out ADB & USB Tunnel (`core/nectarlink-adb`, `src/usb.rs`)**: Speaks the ADB host protocol directly over TCP (`127.0.0.1:5037`) to track attached USB Android devices via `host:track-devices` without spawning `adb.exe` subprocesses per poll, configures port forwards (`host-serial:<serial>:forward:tcp:0;tcp:53319`), and opens a TCP-carried UDP tunnel (`NLQU` header) so `iroh` QUIC datagrams traverse the USB cable with full TLS 1.3 encryption and congestion control.
- **Persistence (`src/store.rs`)**: Embedded SQLite database (`nectarlink.db`, schema version 7) storing paired devices, per-device capability toggles, key-value settings, unified timeline entries, `MessagingStyle` chat threads/messages, and the persistent FIFO send queue (`send_queue`).
- **Resumable Transfers (`src/transfer.rs`)**: Implements `files.v0` multi-file transfers with BLAKE3 integrity verification (`blake3`), `.part` byte-offset resume, pause/resume/cancel semantics, and automatic queueing when a peer is offline or mid-disconnect.
- **Perceptual Color Engine (`src/palette.rs`)**: Extracts Material You HCT + OKLab 15-role light and dark color palettes (`ThemeScheme`) from album cover art.
- **Encrypted Clipboard Vault (`src/clipboard_history.rs`)**: Stores recent text and image clips in encrypted files (`clipboard_history.enc` with `NLCB\x01` header and `clipboard_images/<id>.enc` with `NLCI\x01` header) sealed directly with the platform `KeyProtector` (Windows DPAPI or Android Keystore AES-256-GCM).
- **Interoperability (`src/localsend.rs`, `src/handoff.rs`)**: Implements LocalSend v2 HTTP/HTTPS discovery and file transfer compatibility (`UDP/TCP 53317`) and cross-device reading/document continuation (`handoff`).

### 3. `core/nectarlink-ffi`

UniFFI `0.32` bridge (`src/lib.rs`) compiled via `cargo-ndk` into `libnectarlink_ffi.so` for `arm64-v8a` and `x86_64`.

- Wraps `nectarlink_core::Node` in an FFI-safe `NectarlinkNode` handle.
- Marshals async Rust events into Kotlin `EventCallback` interfaces (`NodeEvent`) and exposes synchronous/asynchronous methods for pairing, messaging, file transfers, send queue inspection, media sessions, and stream creation.
- Accepts a Kotlin-implemented `ForeignKeyProtector` so Android's `KeystoreKeyProtector` (AES-256-GCM inside `AndroidKeyStore`, hardware-backed when supported by the device) seals `identity.key` and `clipboard_history.enc` inside Rust.

### 4. `cli` (`nectarlink-cli`)

Command-line binary (`nectarlink`) in top-level `cli/` for headless operation, scripted integration testing, `notify-when` task completion alerts, and diagnostic commands (`id`, `pair`, `qr-host`, `send`, `status`).

### 5. `desktop/app` (`nectarlink-desktop`)

The desktop client for Windows and Linux, combining a Rust service host with a Qt 6.12 Quick/QML user interface bridged via `cxx-qt` `0.10`.

- **Qt Bridges (`desktop/app/src/bridge/`)**: One bridge per area (`app.rs`, `devices.rs`, `messages.rs`, `calls.rs`, `photos.rs`, `media.rs`, `mirror.rs`, `transfers.rs`, `prefs.rs` and others) exposes state and actions to QML. Design tokens come from `qml/Tokens.qml` and `qml/Theme.qml`.
- **Platform Modules (`desktop/app/src/win/`, `desktop/app/src/linux/`)**: The same interface on each system: audio, clipboard, input, tray, video, storage integration and so on. `crate::win` resolves to the module for the system being built.
- **Shared Desktop Modules (`desktop/app/src/`)**:
  - `mirror.rs`: Phone screen mirroring and app windows, with hardware video decoding.
  - `pc_audio_stream.rs`, `phone_sound.rs` and `phone_mic.rs`: PC sound to the phone, the phone's sound on the PC, and the phone as a microphone.
  - `pc_mirror.rs` and `remote.rs`: Sharing the PC's screen with a phone (DXGI Desktop Duplication on Windows, the screen capture portal or X11 on Linux), and input from a phone (`SendInput` on Windows; `/dev/uinput`, the portal or XTest on Linux).
  - `media.rs`, `win/smtc.rs` and `win/media_sessions.rs`: Now Playing metadata and transport controls, in both directions (SMTC on Windows, MPRIS on Linux).
  - `win/ble_beacon.rs`: Bluetooth LE beacon for waking the phone's connection.
  - `local_api.rs`: Opt-in localhost HTTP/SSE server (`127.0.0.1:53320`) for CLI, PowerToys, Raycast, and Stream Deck integrations.
  - `win/share_target.rs` and `send_to.rs`: Windows Share Target activation, and files sent from File Explorer's or a Linux file manager's **Send to**.
  - `setup.rs` (with `bridge/setup.rs` and `qml/setup/`): The installer's and uninstaller's window. The NSIS installer (`desktop/installer/nectarlink.nsi`) unpacks the app and starts it with `--setup`; the uninstall entry starts the installed app with `--remove`. The window runs the NSIS installer or uninstaller silently with administrator rights and shows its progress.
  - `updater.rs` and `feed.rs`: Update checks against the channel feed (see the [security model](security-model.md#automatic-updates-and-release-verification)), SHA-256 verified downloads, and the installer or package for the kind of install.

### 6. `desktop/vcam` (`nectarlink-vcam`)

A standalone Windows Media Foundation virtual camera COM DLL (`nectarlink_vcam.dll`) registered via `IMFVirtualCamera` or DirectShow/Media Foundation class factories.

- Communicates with `nectarlink-desktop` across a lock-free sequence-numbered shared memory region (`Global\NectarlinkVcamFrame_v2`, `Local\NectarlinkVcamFrame_v2`, or `Local\NectarlinkVirtualCamera_v2` defined in `desktop/vcam/src/shm.rs`).
- Supports `RGB32` (`PIXEL_FORMAT_RGB32 = 0`) and `NV12` (`PIXEL_FORMAT_NV12 = 1`) frames up to `3840x2160` (`MAX_WIDTH = 3840`, `MAX_HEIGHT = 2160`, `MAX_FRAME_BYTES = 33,177,600`) with a 64-byte `SharedHeader` (signature `0x4E4C5643` / `"NLVC"`, `VERSION = 1`).
- Writers update payload bytes between odd and even sequence numbers (`seq`), allowing camera consumers (Zoom, Teams, Meet, OBS) to read frames without cross-process mutexes.

### 7. `android/app` (`app.nectarlink.android`)

The Android application written in Kotlin with Jetpack Compose (`compileSdk = 37`, `targetSdk = 37`, `minSdk = 26`).

- **Foreground Service (`service/ConnectionService.kt`)**: Hosts the `NectarlinkNode` singleton, maintains wake/Wi-Fi locks as needed, and coordinates the feature handlers, one package per area (`sms`, `calls`, `contacts`, `media`, `photos`, `storage`, `recorder`, `toggles`, `webcam`, `mirror`).
- **Notification Listener (`notifications/NotificationListener.kt`)**: Implements `NotificationListenerService` to stream active notifications, `MessagingStyle` conversation threads, inline replies, and action invocations.
- **Elevated Helper (`elevated/Elevated.kt`, `elevated/InputServer.kt`, `elevated/AppDisplay.kt`, `elevated/LoopbackAdb.kt`, `elevated/ShizukuShell.kt`)**: An optional privileged helper process launched via `app_process` (`app.nectarlink.android.elevated.InputServer <token>`) under `shell` (`UID 2000` via on-device Wireless Debugging TLS pairing or USB ADB). It binds a loopback TCP socket (`127.0.0.1:<ephemeral_port>`) and requires a 24-byte (48-hex-char) per-session `SecureRandom` token on the first line (verified in constant time with `MessageDigest.isEqual`), injects touch/key/scroll events through `android.hardware.input.IInputManager` and `/dev/uhid`, and creates virtual displays (`DisplayManager.createVirtualDisplay` in `AppDisplay.kt`) for per-app window streaming (`mirror.apps.v0`).

## Discovery and Transport Architecture

NectarLink uses `iroh` (`1.3`) to run a single QUIC + TLS 1.3 endpoint per device. Every paired connection is classified into one of three `ConnectionPath` variants (`core/nectarlink-core/src/node.rs`):

| Path | Discovery Mechanism | Transport Path | Latency & Bandwidth Profile |
|---|---|---|---|
| `ConnectionPath::Lan` | mDNS (`iroh-mdns-address-lookup`, service name `"nectarlink"`; on PCs on every LAN interface, through a patched copy in `third_party/`) + cached direct socket addresses from `peers.last_addrs` | Direct UDP QUIC between LAN IPv4/IPv6 sockets | Lowest latency, full Wi-Fi / Ethernet throughput |
| `ConnectionPath::Usb` | `core/nectarlink-adb` device tracker (`127.0.0.1:5037`) forwarding to `127.0.0.1:53319` (`DEFAULT_USB_PORT`) | QUIC UDP datagrams framed over TCP (`NLQU` header in `src/usb.rs`) via `adb forward` | Zero Wi-Fi dependency, deterministic wired latency |
| `ConnectionPath::Relay` | Opt-in **Connect over mobile data** (`away_mode = true`) enabling `n0_dns` Pkarr lookup and `RelayMode::Default` | End-to-end TLS 1.3 encrypted QUIC relayed via `iroh` DERP/relay servers | Works across cellular and different networks; subject to relay capability guards |

### QUIC Transport Parameters

Configured in `core/nectarlink-core/src/node.rs`:

- `KEEP_ALIVE_INTERVAL`: `1,000 ms` (1 second)
- `IDLE_TIMEOUT`: `8s` (detects network drops and triggers path failover within seconds)
- `STREAM_WINDOW`: `16 MiB` per stream
- `CONNECTION_WINDOW`: `32 MiB` per connection

### Automatic Path Failover and Deduping

When both USB and Wi-Fi are available, or when a phone transitions between Wi-Fi and a **Connect over mobile data** (`away_mode`) relay path, both peers may attempt simultaneous connections. `Node` resolves concurrent dials deterministically and updates the active session handle in place so upper-layer features (`Timeline`, `SendQueue`, `Media`, `Notifications`) continue operating against the peer's `DeviceId`.

## Perceptual OKLab Color Engine

Implemented in `core/nectarlink-core/src/palette.rs`, the color engine derives cohesive light and dark UI palettes (`ThemeScheme`) from album artwork (`extract_theme_from_encoded_image`, `extract_theme_from_rgba`).

```text
Encoded Image / RGBA Buffer
  │
  ├─► 1. Border Trim (trim_borders: strips up to 22% uniform/black/white letterbox bars)
  │
  ├─► 2. Area-Box Downsample (downsample_rgba: max 112x112 = 12,544 pixels)
  │
  ├─► 3. Alpha & Near-Black Filter (ignores alpha < 128 and CIE L* < 1.5)
  │
  ├─► 4. Celebi Quantization in CIE L*a*b* (QuantizerCelebi, up to 128 clusters)
  │
  ├─► 5. Achromatic Check & 360-deg Hue Neighborhood Scoring
  │      (weights population^0.38 * chroma * mid-tone lightness; penalizes skin-tone bands)
  │
  ├─► 6. OKLab Centroid Seed Calculation (sRGB → linear RGB → LMS → OKLab mean)
  │
  └─► 7. HCT TonalPalette Generation + WCAG AA/AAA Contrast Enforcement
         Produces 15-role dark & light RolePalette (cached in 64-entry LRU)
```

### Pipeline Stages

1. **Letterbox and Border Trimming (`trim_borders`)**: Scans inward from all four edges (up to 22% of width/height) and removes rows/columns that are near-black, near-white, or low-variance solid frames so letterboxed video thumbnails do not skew the dominant color.
2. **Area-Box Downsampling (`downsample_rgba` / `downsample_rgb`)**: Averages pixel blocks into at most `112x112` (`MAX_SAMPLE_SIDE = 112`) pixels, suppressing compression noise while preserving proportional color area.
3. **Celebi Clustering (`QuantizerCelebi`)**: Quantizes filtered pixels in CIE L\*a\*b\* space into at most 128 clusters (`QUANTIZE_COLORS = 128`).
4. **Monochrome Detection (`is_mostly_achromatic`)**: If at least 92% of non-dark pixels have HCT chroma `< 8.0` and no vibrant accent exists, the engine selects the neutral zinc seed (`ACHROMATIC_SEED_RGB = 0x5E5E62`) to avoid inventing false color casts on black-and-white covers.
5. **Hue Family Scoring (`select_perceptual_seed`)**: Evaluates each candidate cluster across a 15-degree Gaussian hue neighborhood, weighting `population_ratio^0.38`, HCT chroma peak (`48.0`), mid-tone lightness preference, and a skin-tone de-bias factor (`35..52` deg hue with moderate chroma) so background/graphic art beats portrait skin tones.
6. **OKLab Centroid (`oklab_centroid_rgb`)**: Computes the population-weighted mean of all clusters within 25 degrees of the winning hue in perceptual OKLab space (`srgb_to_oklab` → `oklab_to_srgb`), preventing muddy RGB averaging.
7. **Tonal Palette and Contrast Enforcement (`scheme_from_seed`, `enforce_contrast`)**: Builds six HCT `TonalPalette` ramps (`primary`, `secondary`, `tertiary`, `neutral`, `neutral_variant`, `error`) and derives 15 semantic tokens (`surface`, `surface_container`, `surface_bright`, `primary`, `on_primary`, `primary_container`, `on_primary_container`, `secondary`, `on_secondary`, `secondary_container`, `tertiary`, `on_tertiary`, `on_surface`, `on_surface_variant`, `outline`) with guaranteed WCAG contrast ratios (`>= 4.5:1` for primary/surface text, `>= 3.0:1` for muted text) and a 64-entry FNV-1a hashed LRU cache (`CACHE_CAPACITY = 64`).
