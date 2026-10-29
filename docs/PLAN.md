# NectarLink: Product & Engineering Plan

> **NectarLink** connects Android phones and Windows PCs. It's one open-source app that replaces Phone Link, KDE Connect, scrcpy, LocalSend and Intel Unison.
> *Bees carry nectar back to the hive. NectarLink carries everything between your phone and your PC.*
> Status: **in active development (Qt Quick + Rust on Windows 11, Kotlin + Jetpack Compose on Android, shared Rust core).** Phase 1 and most of Phases 2 to 3 work end to end; see §3 for per-feature status. Last updated 2026-10-09.

---

## 0. Decisions at a glance

| Question | Decision | One-line reason |
|---|---|---|
| Name | **NectarLink** (done) | Clean name (no conflicts found), warm, on the bee theme, says "connection". See §10.1 |
| Shared engine | **Rust core** (`nectarlink-core`) used by both apps | Both ends run the same protocol code, which avoids the cross-implementation bugs KDE Connect has |
| Transport | **iroh 1.x** (QUIC, devices dialed by public key) | E2E-encrypted by default, LAN discovery, hole punching, relay fallback and multipath in one library (1.0 June 2026, now 1.3) |
| Wire format | **CBOR (serde)** over QUIC streams, plus a versioned spec in `docs/protocol` | Self-describing and forward-compatible, with no codegen toolchain. Third parties can still implement it |
| Windows app | **Qt Quick (QML) for the UI + Rust for everything else** (bridged by cxx-qt), `windows-rs` for OS integration, mirrored video drawn inside the UI with no copies. **No WebView, no web stack** (done) | Visual ceiling equal to web (real blur, shaders, Caelestia-level motion) at a fraction of the RAM, best fit for video and always-on overlays. Gated by a Phase 0 "Qt + Rust proof" (§5.1) |
| Android app | **Kotlin + Jetpack Compose + Material 3 Expressive**, core via **UniFFI** | Deep OS integration needs native Kotlin, and Compose/M3 Expressive is the best-looking, smoothest Android UI stack |
| Mirroring | **Two engines:** *Elevated* (scrcpy-server, Apache-2.0) and *Standard* (MediaProjection). PC side decodes in hardware with Media Foundation and renders with D3D11 | Best quality and features when privileged access exists, and it still works when it doesn't |
| Superpowers without root | **Power Levels** (Basic → Assist → Elevated). A Root level is parked for later. Elevated mode runs NectarLink's *own* code as the `shell` user, so there's **no extra app to install**. Several setup paths, and the UI greys out features by level | Unlocks background clipboard, sensitive notifications, consent-free mirroring, app windows and toggles on **any** Android phone |
| Accounts / cloud | **None.** Local-first. Optional "away from home" mode via relays (self-hostable) | Privacy is the #1 reason people leave Phone Link |
| Platforms | **Windows 11 only** (x64 + native ARM64). Android 8+. **macOS desktop = future scope** (Qt Quick, the Rust core and the `Platform`/`VideoSink` layers make it a port, not a rewrite). iPhone = later | Win11-only frees us to use Mica, the virtual camera API and the Win11 context menu |
| Distribution | **GitHub-first**: GitHub Releases + winget (Windows), GitHub Releases + Obtainium + later IzzyOnDroid (Android). No Play account for now | Free, fast to ship, no Play policy limits on SMS/Accessibility/files |
| License | (done) Apps **GPL-3.0-or-later** (+ app-store exception), core crates **MPL-2.0**, protocol spec & docs **CC BY 4.0**, name/logo trademark-protected, **DCO** for contributions | See §10.2 |
| Crash reports | **Opt-in only**, with a preview of exactly what gets sent | Privacy is a selling point |

---

## 1. Research: what exists and what people say

### 1.1 The landscape

| App | Strengths | Weaknesses / complaints |
|---|---|---|
| **Microsoft Phone Link** | Built into Windows 11. Calls via Bluetooth, SMS, photos, notifications, Start menu phone flyout, "Lock PC" from phone, Cross-Device Resume, "Expanded screen" app mirroring (Jan 2026) | Needs a Microsoft account. App mirroring and copy-paste are **gated to Samsung/HONOR/OPPO/ASUS/vivo/Xiaomi**. Frequent disconnects ("offline", drops after PC sleep). **Sensitive notifications (OTPs) blocked** on Android 15+ for Play Store installs, and in 2026 Microsoft announced it blocks them outright, which made users angry. Closed source |
| **KDE Connect** | Feature-rich, FOSS, cross-platform (officially on Windows via Microsoft Store and installer, v26.08.1 Sept 2026), run commands, presentation remote, true bidirectional clipboard | Windows is a secondary platform with rougher edges than Linux. Discovery and firewall trouble (UDP broadcast on 1714 to 1764, rules hard to add). Same-network only. "Constant connectivity issues", dated UI/UX. No virtual display. Several features are one-directional |
| **Sefirah** (closest competitor) | WinUI 3, nice Fluent UI, ~3k stars, actively developed (v3.1, Aug 2026). Notifications, clipboard (Shizuku/ADB), media, files, **Android storage in Explorer**, SMS, **Bluetooth calling (v3.0)**, scrcpy mirroring, Play Sound | scrcpy must be **downloaded and configured manually**. Windows-only UI tech. No SMS attachments. Open issues ask for **LAN-not-just-Wi-Fi, mobile data / away use, virtual touchpad, contacts tab, keyboard shortcuts, custom phone toggles, auto-select device, battery-saving controls**. Bugs: tray icon missing, stuck loading, SMS sync lag, mirror not responding. Breaking releases (v3.0 needed a reinstall due to expired certs). Some fork/source turbulence in Oct 2026 |
| **scrcpy** (4.0, May 2026) | Best-in-class mirroring: low latency, audio, camera, **virtual displays (resizable in 4.0)**, UHID keyboard/mouse, OTG | CLI-first, needs USB debugging / ADB knowledge, no GUI, no pairing UX, no other features |
| **LocalSend** | Fast, cross-platform (incl. iOS/macOS), no account, open REST protocol v2 | Files only (no clipboard or notification sync). Discovery fails on AP isolation / VPN / Windows firewall / "Public" network profile |
| **Intel Unison** (dead) | Loved for files, photos, SMS and calls for **both Android and iOS**, and tablet as second screen | Discontinued June 2025, download removed Jan 2026. Its users are now homeless |
| **Tandem** (SamyogKarki/tandem, new open-source project) | Windows app for Android phones: screen mirroring and control over Wi-Fi (via Android wireless debugging, QR pairing), file drag and drop, shared clipboard, notifications and calls on the PC. Nothing to install on the phone | Windows-only, needs Android 11+ and wireless debugging, same Wi-Fi. A young project, but shows the same "one app instead of scrcpy + KDE Connect" idea. Another reason to avoid the name "Tandem" |
| **Quick Share (Google)** | Fast file sharing, official, now on Windows | Files only, Google-controlled |
| **AirDroid / Pushbullet / GlideX** | Remote management, "unify control" across devices (GlideX) | Freemium, cloud accounts, ads, privacy concerns |

### 1.2 What users complain about (ranked by how often it comes up)

1. **"It disconnects / can't find my device."** This covers firewalls, the Windows "Public" network profile, AP isolation, VPNs, PC sleep, and Android battery killers (OnePlus, Xiaomi and others). *This is the #1 problem across every app in the category.*
2. **Features locked to certain phones** (Phone Link's OEM gating).
3. **Lost OTP / 2FA notifications** after Android 15.
4. **Accounts and cloud** (Phone Link's Microsoft account).
5. **Juggling several apps**: scrcpy for mirroring, LocalSend for files, KDE Connect for everything else.
6. **Setup friction**: ADB, Shizuku, manual scrcpy paths, firewall rules, restricted-settings toggles.
7. **Only works on the same Wi-Fi.** People want Ethernet PCs, USB, and mobile data / away use.
8. **Ugly or confusing UI** (KDE Connect on Windows).
9. **Wishlist:** phone apps as separate PC windows, phone as webcam, phone as touchpad/keyboard, contacts, keyboard shortcuts, phone toggles (Wi-Fi/BT/DND/hotspot) from the PC, Explorer integration, tablet as second screen, mouse flowing across devices.

### 1.3 Hard platform constraints (these shape the architecture)

| Constraint | Impact | Our answer |
|---|---|---|
| Android 10+: no background clipboard read | Auto clipboard sync phone→PC is impossible for normal apps | Elevated mode reads the clipboard. Fallbacks: QS tile, share target, notification button, optional NectarLink keyboard (IME) |
| Android 15+: sensitive notifications redacted for untrusted listeners | OTPs and 2FA vanish | Elevated mode grants NectarLink `RECEIVE_SENSITIVE_NOTIFICATIONS` via appops (one time, persists). Explained clearly in onboarding |
| Android 14+: MediaProjection needs consent **every session** | Standard mirroring shows a dialog each time | Elevated engine (scrcpy-server as shell) has no prompts |
| Android 13+: "restricted settings" for sideloaded apps | Notification-listener and accessibility toggles are greyed out for non-store installs | Onboarding detects it and walks through "Allow restricted settings". Obtainium installs reduce the friction |
| Android developer verification (enforced from 30 Sep 2026 in BR/ID/SG/TH, global 2027) | Unverified sideloads get blocked, and that includes GitHub APKs | Register for verification (non-Play path) before global enforcement. One signing key for every channel from day one |
| Google Play policy: SMS/Call Log, Accessibility, All-files access | Rejection risk *if/when* we go to Play | GitHub-first ships only the **`full`** flavor. The code keeps a `play` flavor seam so a Play build is cheap later |
| No 3rd-party call-audio capture on Android | Can't stream call audio over IP | Bluetooth **HFP** (PC acts as headset). The Windows `PhoneLineTransportDevice` API broke for unpackaged apps on 22H2+, yet Sefirah v3 ships Bluetooth calling, so it's feasible (likely needs package identity). **Spike early** |
| Windows 11 virtual camera API (`MFCreateVirtualCamera`, build 22000+) | Phone-as-webcam needs no kernel driver | COM media-source DLL. Proven by open-source samples (VCamSample, BestCam) |
| Virtual monitor on Windows needs an IddCx driver | Tablet-as-second-screen needs a signed driver | Later phase. Evaluate an MIT-licensed signed IddCx driver |
| Unlock-PC-with-phone needs a custom Windows **credential provider** | Security-critical code running in the logon UI | Dedicated design + external security review before shipping (§3.11) |


### 1.4 What we take from KDE Connect (inspiration only, no connection to it)
NectarLink does **not** pair with or talk to KDE Connect. We study what it does well and build our own version:
- **Per-device plugin toggles**: every feature can be switched on/off per paired device. This becomes our per-device feature permissions.
- **Run commands**: user-defined PC commands triggered from the phone. Our custom commands / Deck actions, off by default.
- **Presentation remote**: slide control plus a pointer. Our presentation remote with laser pointer.
- **Remote input**: phone as touchpad and keyboard. Our touchpad / air mouse.
- **Find my phone + ping**: simple and loved. Kept.
- **Battery and connectivity reports**: phone battery and signal on the PC. Kept.
- **Bidirectional media control**: both devices control each other's players. Kept, plus the Windows media flyout.
- **Browse phone storage from the PC**: KDE Connect uses SFTP; we use native Explorer integration.
- **Pairing with explicit confirmation on both sides**: kept, with our QR / 6-digit code flow.

What we deliberately do differently: discovery and connectivity (their #1 weakness), the Windows UI, and setup friction.

---

## 2. Product principles

1. **It just works.** Connection reliability beats every feature, and setup takes under 60 seconds. When something breaks, the app tells you *why* and fixes it with one click.
2. **Every Android phone is first-class.** No OEM gating, and nothing is held back as a Samsung-only feature.
3. **Local-first and private.** No account, no telemetry by default, E2E encryption always, open protocol.
4. **One app, not five.** Mirroring, files, clipboard, notifications, messages, calls and webcam live in one place, with one pairing.
5. **Beautiful, fast, light.** Native GPU rendering, 60 to 120 fps, spring physics, fast start, and a tray app that stays light all day.
6. **Graceful power tiers.** Everything works with standard permissions. Elevated adds superpowers. The UI always shows which tier you're on and what the next tier unlocks.
7. **Keyboard-first on PC, thumb-first on phone.**

---

## 3. Feature set

Tiers: **P0** = needed for the first public release, **P1** = 1.0, **P2** = after 1.0.
Each priority is followed by the power level a feature needs: Basic, Assist or Elevated (see §4.6). (done) marks what is built.

### 3.1 Connection & setup
| Feature | Tier |
|---|---|
| Pair by **QR code** (PC shows QR → phone scans), by **tap a nearby device + 6-digit code match**, or **plug in USB** (auto-pair) (done) (USB auto-pair verified against Android emulator via ADB server; physical USB cable on a real phone not yet verified) | P0 |
| Auto-reconnect everywhere: PC wake from sleep, Wi-Fi changes, phone reboot, app updates (done) | P0 |
| Works over **Wi-Fi, Ethernet-to-Wi-Fi, USB**, and **away from home** (hole punching + relay, opt-in, self-hostable) (done) (USB transport verified on Android emulator; physical USB cable on a real phone not yet verified) | P0 (LAN/USB), P1 (away) |
| **Connection Doctor**: detects firewall, Public network profile, AP isolation, VPN, battery optimization, OEM killers. One-click fixes plus per-OEM guides (done) | P0 |
| Companion Device Manager association + PC BLE beacon, so the phone app is woken when the PC is nearby: implemented (`win/ble_beacon.rs` + `CompanionDeviceHelper.kt`), **not verified end-to-end on real BLE hardware** (Android emulator lacks a BLE radio) | P1 |
| Bluetooth fallback link for low-bandwidth features (notifications, SMS, calls) when there's no shared network: deferred (requires physical Bluetooth RFCOMM radio on both endpoints; Android emulator lacks a BT controller) | P2 |
| Multiple phones per PC, multiple PCs per phone, per-device feature permissions (done) | P0 (model), P1 (UI) |

### 3.2 Notifications
| Feature | Tier |
|---|---|
| Mirror notifications as native Windows toasts with **app icon, big images, actions, inline reply** (done) | P0 Basic |
| Dismiss sync (clear on one side → cleared on the other), per-app allow/block, quiet hours (done) | P0 Basic |
| Notification feed in the app: history, search, grouping by app/conversation (done) | P0 Basic |
| **Sensitive notifications / OTPs** (Android 15+ `RECEIVE_SENSITIVE_NOTIFICATIONS` app-op granted by Elevated helper) (done) | P0 Elevated |
| **Smart OTP**: detect codes and offer a "Copy code" toast; auto-copy optional (done) | P1 |
| **Live Updates** (Android 16 progress notifications: rides, deliveries, timers) shown live on PC (done) | P1 |
| DND sync: quiet PC notifications when phone is in DND, plus toggle phone DND from PC (Windows has no public API to read or set Focus/DND) (done) | P1 Basic/Elevated |
| PC notifications → phone (optional) (done) | P2 |

### 3.3 Messages (SMS/MMS + chat apps)
| Feature | Tier |
|---|---|
| SMS/MMS threads, send, receive, search, **attachments (MMS images)** (done) | P0 Basic |
| **Unified Conversations**: RCS, WhatsApp, Telegram, Signal and others built from MessagingStyle notifications, with reply via RemoteInput. One inbox on PC (done) | P1 Basic |
| Contacts (photos, search, favorites), start a conversation or call from PC (done) | P1 Basic |
| Drag an image from the desktop into a thread (or paste from the PC clipboard) to send it as MMS (done) | P1 |

### 3.4 Calls
| Feature | Tier |
|---|---|
| Incoming call alert on PC with caller photo; answer / decline / silence (audio stays on phone) (done) | P0 Basic |
| Call in progress on PC: timer, hang up, volume; with Elevated (Android 12+), mute, speaker, hold and keypad through a calling-companion `InCallService` (done) | P0 Basic |
| Call log, dialer, call from contacts (done) | P1 Basic |
| **Call audio on PC via Bluetooth HFP** (PC mic/speakers): **not possible with public Windows APIs (findings in `docs/research/hfp.md`)** | P1 (after spike) |
| Auto-pause PC media during calls (done) | P1 |

Not planned: **recording calls.** Android keeps call audio (`VOICE_CALL`
capture) to privileged apps, so a third-party app can't record calls on
current Android.

### 3.5 Clipboard
| Feature | Tier |
|---|---|
| Text + images + rich text, bidirectional, auto (done) | P0 (PC→phone Basic, phone→PC auto Elevated) |
| Fallbacks without Elevated: "Send clipboard" QS tile, share target, notification button (done) | P0 Basic |
| Clipboard history (both devices, local, encrypted), pin items (done) | P1 |
| **Smart clipboard (context chips)**: copy an address → "Open in Maps"; a phone number → "Call"; a tracking code → "Track"; a link → "Open on phone"; an email → "Email" (OTPs have "Copy code" on notifications and texts) (done) | P1 |
| Respect password managers' "exclude from clipboard history" flags on both sides; never sync sensitive clips (done) | P0 |
| Copy files on PC → paste on phone (and the reverse) (done) | P2 |

### 3.6 Files & photos
| Feature | Tier |
|---|---|
| Send files/folders both ways: drag-drop, share sheet, Explorer **"Send to phone"** context menu, Windows share sheet: drag-drop, file/folder pickers, Android share sheet, Send-to menu, Explorer context menu, and `--share` CLI target ship for unpackaged installs (done); system-wide Windows Share sheet (`windows.shareTarget`) **requires signed MSIX/sparse package identity or Windows Developer Mode** | P0 |
| Fast, resumable, multi-stream transfers with progress, pause/resume, and integrity hashing (done) (single QUIC stream per transfer with 16 MiB stream window and 8 × 1 MiB read-ahead saturates the shared QUIC congestion window; BLAKE3 integrity hashing and manual pause/resume from either side) | P0 |
| **Flick to send**: flick a photo/file upward on the phone and it lands on the PC. Drag a file to the screen edge on the PC and it flies to the phone. Physics-based animation on both ends (done) | P1 |
| **LocalSend protocol interop**: send to and receive from any LocalSend device (iOS, Mac, Linux). We implement its open protocol in Rust; we don't reuse its (Dart) code (done) | P1 |
| **Coexistence**: runs side by side with KDE Connect (ports 1714 to 1764) and LocalSend (53317) on the same PC without conflicts. If LocalSend's port is taken, NectarLink still sends but leaves receiving to the LocalSend app (done) | P0 |
| **Phone storage in Explorer** (Cloud Files API: placeholders, on-demand download, native sync icons) (done) | P1 |
| **Photos**: gallery with thumbnails, recent screenshots/photos pop up on PC, drag out to the desktop (done) | P0 (recent), P1 (gallery) |
| Folder sync (Syncthing-style two-way folders, e.g. Camera → PC) (done) | P2 |

### 3.7 Screen mirroring & phone apps on PC
| Feature | Tier |
|---|---|
| Mirror and control the full phone screen. **Hardware decode**, ≤50 ms on LAN, 1080p60+, H.264/H.265/AV1 (done) (1080p60 H.264/H.265/AV1 hardware encode/decode, Standard Basic view, Assist Assist control, and prompt-free Elevated Elevated capture & input) | P0 (Standard Basic view / Assist control, Elevated Elevated) |
| **Built-in**. Nothing to download and no scrcpy path to configure (done) | P0 |
| **Audio forwarding** (phone audio plays on PC) (done) (Basic `AudioPlaybackCaptureConfiguration` on Android 10+ plus Elevated Elevated internal capture via shell helper where permitted by OEM/SELinux policy) | P1 Elevated (or Basic via playback capture) |
| **App windows**: launch a phone app as its own resizable PC window with taskbar icon (virtual display, flex resize). App list and pinning (done) | P1 Elevated |
| Keyboard (physical layout via UHID), mouse, scroll, gamepad, IME text, copy/paste inside the mirror, drag files in to send (done) (UHID keyboard/gamepad requires kernel/SELinux `/dev/uhid` access for `shell` UID, with automatic `input keyevent` fallback) | P1 |
| Screen off while mirroring, keep awake, record, screenshot (done) | P1 |
| Recent apps / notifications open directly into an app window (done) | P1 |

### 3.8 Media & audio
| Feature | Tier |
|---|---|
| Control phone media from PC. Phone media appears **in Windows' own media flyout** with artwork (done) | P0 Basic |
| Control PC media from phone (media notification with artwork, seek, volume) (done) | P0 Basic |
| PC volume and active output device indicator from phone (done) (Windows has no public API to switch the default audio output device) | P1 |
| **Voice Recorder → PC**: record with the phone's (better) mic, hit send, and the file lands on the PC in your chosen format (M4A/MP3/WAV/FLAC) and folder, with timestamped markers (done) (optional live stream while recording, Opus, and noise suppression not started) | P1 Basic |
| **Phone as speaker / headphones for PC audio** (real-time low-latency WASAPI loopback → QUIC → Android `AudioTrack` `PERFORMANCE_MODE_LOW_LATENCY`, with one-tap toggle on both PC Home and Android Home) (done) | P1/P2 Basic |
| Phone as live microphone for PC apps: deferred (Windows has no user-mode virtual microphone API equivalent to `IMFVirtualCamera`; requires an WHQL-signed kernel WDM/WaveRT audio driver) | P2 (needs a virtual audio driver, evaluate) |

### 3.9 Camera
| Feature | Tier |
|---|---|
| **Phone as webcam** (Windows 11 virtual camera, shows up in Teams/Zoom/OBS/Camera app). Front/back, zoom, torch, mirror, 720p/1080p/4K, upright sensor-rotation-corrected aspect-ratio preview & live controls (done) (720p/1080p verified live on emulator; 4K `2160p` pipeline implemented and unit-tested in `vcam/src/shm.rs` but **not verified on a physical 4K phone camera sensor**; 10-bit HDR intentionally omitted because Windows inbox `CLSID_CMSH264DecoderMFT` and the virtual-camera shared-memory pipeline operate in 8-bit SDR Rec.709) | P1 Basic (Camera2) |
| **Continuity Camera**: from the PC (Home, tray, Command Palette, or global hotkey) → "Take photo / Scan document with phone" → the phone opens the camera (with document edge detection and perspective correction) → the result is placed on the clipboard and pasted at the cursor (done) | P1 Basic |

### 3.10 Remote input & control
| Feature | Tier |
|---|---|
| Phone as **touchpad + keyboard** for PC, presentation remote (with laser pointer) (done) | P1 Basic |
| **Air mouse**: point the phone and its gyroscope moves the PC cursor, for presentations and couch use (done) | P1 Basic |
| **Voice typing into PC**: hold a button on the phone, speak, and the text (from the phone's own speech engine) is typed into the PC's focused field (done) | P1 Basic |
| Type on the phone with the PC keyboard (without mirroring) (done) | P1 Elevated |
| **Deck**: the phone becomes a Stream-Deck-style macro pad for the PC with live tiles: mic mute, speaker volume/mute, media, app launchers, shortcuts, scripts/commands, window switching, multiple pages, haptic feedback (done) (dedicated OBS WebSocket tile not started) | P1 Basic |
| **Flow**: move the PC mouse off the screen edge onto the phone/tablet and back, with clipboard following: not built yet | P2 Elevated |
| Phone toggles from PC: Wi-Fi, BT, DND, ringer, flashlight, volume, brightness (done) (the hotspot has no shell command to switch it; see Instant Hotspot) | P1 (partly Elevated) |
| PC actions from phone: lock, sleep, mute, custom commands/scripts (done) (shutdown intentionally omitted) | P0 (lock/sleep), P1 (custom, off by default) |
| Wake-on-LAN (done) | P1 |
| **Unlock PC with phone fingerprint**: the PC lock screen offers "Unlock with phone" → the phone shows a biometric prompt → a signed challenge unlocks Windows. Built as a Windows credential provider. Keys live in Android Keystore (StrongBox where available) and are bound to biometrics. Works only when paired and nearby (BLE proximity + LAN). Rate-limited, revocable, and **external security review before release**: deferred (dedicated post-1.0 workstream requiring a system-wide `Winlogon` `ICredentialProvider` DLL and external security review) | P2 (dedicated workstream) |

### 3.11 Ambient & signature experiences (what makes NectarLink special)
| Feature | Tier |
|---|---|
| **Nectar Island** (optional extra, off by default (`nectar_island: false`), with per-trigger toggles when enabled): a small, animated pill at the top of the PC screen showing live phone activity (call, timer, music, transfer, Live Update, OTP, voice recording). Expands on hover. A nice touch, not a pillar (done) | P2 |
| **Shelf**: slide-out edge panel with the latest photo, screenshot, clipboard and recent files. Drag anything in to send (done) | P1 |
| **Command Palette** (global hotkey): "send clipboard", "find my phone", "text Mom", "open last photo", "mirror Spotify", "record voice" (done) | P1 |
| **Material You sync**: the PC app takes its accent and theme from the phone's wallpaper colors, so both apps look like one (done) | P1 |
| **Handoff**: send a link, map location, document or YouTube timestamp to open on the other device (done) | P0 (links), P1 (rich) |
| **"Ping me when it's done"**: `nectarlink notify-when <pid \| -- command...>` watches a running process or command on the PC, shows a live chronometer notification on the phone while it runs, and alerts the phone when it finishes or fails (done) | P1 |
| **Timeline**: one searchable history of everything that moved between devices (files, clips, links, photos, recordings), with "send again" (done) | P1 |
| **Your PC in your pocket**: from anywhere (Away mode), browse and grab PC files, see what the PC is doing (Ping-me tasks, downloads), wake it (relayed Wake-on-LAN through another NectarLink device left at home), and later view/control the PC screen (reverse mirroring) (done) (PC file browsing, Ping-me status, relayed Wake-on-LAN, and reverse PC-to-phone screen mirroring implemented; reverse mirroring is off by default (`pc_mirror` toggle), prompts the PC user for consent on every session with a persistent stop banner, and requires `remote_input` for mouse/keyboard control) | P2 (files/status in P1 if Away mode lands early) |
| **Proximity lock**: lock the PC when the phone walks away (BLE RSSI): deferred (requires continuous BLE RSSI hardware ranging; Android emulator lacks a BLE radio) | P2 |
| **Automations**: triggers (connected, battery low, call started) → actions (done) | P2 |
| Find my phone (ring even on silent), battery & signal in tray, low-battery alerts, measured phone screen preview & Home summary (done) | P0 |

### 3.12 Platform & ecosystem
| Feature | Tier |
|---|---|
| Tray app, start on login, jump list, global hotkeys, Windows Widgets board widget: tray app, start on login, taskbar jump list, and global hotkeys ship for unpackaged installs (done); Windows Widgets board provider **not built yet: it needs the Windows App SDK and package identity (#85)** | P0/P1 |
| Android Quick Settings tiles, home-screen widgets (Glance), share target, Direct Share to PC (done) | P0/P1 |
| **CLI** (`nectarlink send file.pdf`, `nectarlink clip`, `nectarlink notify`, `nectarlink notify-when`) and local authenticated API for scripts and PowerToys Command Palette (done) | P1 |
| Tablet as second screen (IddCx virtual monitor): deferred (requires an WHQL-signed UMDF/IddCx indirect display driver) | P2 |
| macOS desktop build: deferred (future cross-platform scope) | P2 (future scope) |
| Plugin/extension API over the open protocol: **out of scope for v1 / not started (local HTTP/WS API and CLI shipped instead)** | P2 |
| Full i18n, accessibility (screen readers, keyboard nav, contrast), reduced motion (done) | P1 |

### 3.13 Rejected for now (parked, can be revived later)
| Idea | Why parked |
|---|---|
| **Root power level** (Magisk / KernelSU / APatch: Elevated features that survive reboots without re-arming) | Removed by the maintainer to keep setup simple; can be added later as an extra Elevated setup path |
| **Smart notification routing** (phone silent while you're active on PC, buzz when idle) | Not prioritized by the maintainer. Depends on presence heuristics that are easy to get wrong |
| **Desk Mode** (phone goes quiet + ambient dock screen when charging at desk, PC locks when you leave) | Not prioritized. Overlaps partly with Proximity lock |
| **Instant Hotspot** (one click: phone hotspot on + PC joins) | Not prioritized. Needs Elevated plus per-OEM hotspot quirks |
| **"Catch me up"** (on-device AI summary of missed notifications) | Not prioritized. Depends on local-AI availability (Copilot+ PCs / phone models) |

---

## 4. Architecture

### 4.1 Big picture

```
┌──────────────────────── Windows PC (one native process) ──┐        ┌───────────────────────── Android phone ─────────────────────────┐
│  Qt Quick UI (QML screens, NectarLink design system)          │        │  Compose UI (Material 3 Expressive)                              │
│   main window · Island · Shelf · Deck editor · palette   │        │   home · devices · onboarding · Deck · tiles · widgets           │
│   mirror/app windows: decoded D3D11 video inside the UI   │        │        ▲  Kotlin facades (Flows)                                  │
│        ▲  cxx-qt bridge (view-models, in-process)             │        │  ┌─────┴───────────────────────────────────────────────────┐    │
│  ┌─────┴──────────────────────────────────────────────┐   │        │  │ Android services (Kotlin)                               │    │
│  │ nectarlink-desktop (Rust)                              │   │        │  │  NotificationListener · SMS/Calls/Contacts · MediaSession│   │
│  │  nectarlink-win: toasts+reply · Cloud Files · GSMTC/   │   │        │  │  CameraX · AudioRecord · MediaProjection · CDM · FGS     │    │
│  │  SMTC · clipboard · share · context menu · BLE ·   │   │        │  └─────┬───────────────────────────────────────────────────┘    │
│  │  MF decode/D3D11 · vcam (COM DLL) · HFP · cred.    │   │        │        │ UniFFI                                                   │
│  │  provider · firewall · tray · hotkeys              │   │        │  ┌─────┴──────────────────────────────────────────────────┐     │
│  └─────┬──────────────────────────────────────────────┘   │  QUIC  │  │ nectarlink-core (same crate, compiled for arm64/x86_64)     │     │
│  ┌─────┴──────────────────────────────────────────────┐   │◄──────►│  └─────────────────────────────────────────────────────────┘     │
│  │ nectarlink-core (Rust, shared)                         │   │  iroh  │  ┌─────────────────────────────────────────────────────────┐     │
│  │  identity · pairing · sessions · capabilities      │   │        │  │ Elevated mode (NectarLink's own code as shell uid)          │     │
│  │  services · transfer engine · store (SQLite)       │   │        │  │  clipboard · appops · screen capture · input injection  │     │
│  └────────────────────────────────────────────────────┘   │        │  │  virtual displays · toggles · scrcpy-server engine      │     │
│  nectarlink-adb (built-in ADB client) ─ USB / wireless dbg ───┼────────┼─►│                                                         │     │
└────────────────────────────────────────────────────────────┘        └──┴─────────────────────────────────────────────────────────┴─────┘
```

The desktop is **one native process**: Rust owns the core, networking and OS integration, and Qt Quick draws the UI. Heavy windows (main, mirror, app windows) are created on demand and destroyed when closed. The always-on part (core + tray + toasts + Island) stays small.

### 4.2 Repository layout (monorepo)
```
nectarlink/
  core/                 Rust workspace
    nectarlink-core/        engine: identity, pairing, session, capabilities, services, transfer, store
    nectarlink-protocol/    message types + versioning (CBOR); spec generated into docs/protocol
    nectarlink-localsend/   LocalSend v2 interop (HTTP(S) server + multicast discovery)
    nectarlink-adb/         pure-Rust ADB client (USB + wireless pairing via SPAKE2/TLS)
    nectarlink-ffi/         UniFFI bindings for Kotlin
    nectarlink-cli/         command-line client
  desktop/
    app/                one crate (cxx-qt builds the QML module, bridge and binary together):
      src/              Rust host: startup, tray, window lifecycle, wiring to core
      src/bridge/       the ONLY Rust↔Qt bridge: cxx-qt view-models exposed to QML
      qml/              NectarLink design system (tokens, ~30 components) + screens. Presentation only
    platform/           `Platform` trait: nectarlink-win (windows-rs) now, nectarlink-mac later
    video/              `VideoSink`: MF decode + D3D11 texture → Qt Quick scene-graph item (small C++)
                        now; VideoToolbox + Metal later
    vcam/               virtual camera media source (COM DLL)
    credprov/           credential provider for "Unlock with phone" (P2, isolated, audited)
  android/
    app/                Compose app (flavor `full` now; `play` seam kept)
    elevated/           privileged-mode entry point (dex launched with app_process)
    mirror/             encoders, scrcpy-server engine integration
  relay/                deployable iroh relay config (self-hosting docs)
  docs/                 protocol spec, design system, ADRs, contributor guide
```

### 4.3 Identity, pairing, security
- **Identity:** each install has an Ed25519 key (the iroh endpoint ID). Private keys live in DPAPI-protected storage on Windows and Android Keystore-wrapped storage on the phone.
- **Trust = pinned public key.** No CA, no server. Unpaired peers can only reach the pairing handshake.
- **Pairing flows**
  - *QR:* the PC shows a QR with `{endpoint id, LAN addresses, 128-bit one-time secret, name}`. The phone scans, dials, and proves the secret. This gives mutual authentication with no codes to compare.
  - *Nearby:* pick the device from a discovered list, then both screens show a 6-digit SAS derived from the handshake transcript. Tap "matches".
  - *USB:* if USB debugging is on, the PC exchanges keys over ADB automatically.
- **Encryption:** QUIC + TLS 1.3 (raw public keys), always. Relays only forward ciphertext.
- **Per-device permissions:** each paired device has feature toggles. Dangerous ones (run commands, remote input into PC, file browse, PC unlock) are **off by default**.
- **Local API/CLI:** a named pipe with a per-user token. No open localhost TCP port.
- **At-rest & data retention (done):** clipboard history is sealed on disk with `KeyProtector` (DPAPI on Windows, Keystore AES-GCM on Android), capped at 50 items, and auto-purges unpinned clips older than 7 days (pinned clips stay; OTPs are never stored). Timeline defaults to 90 days (configurable: 30 / 90 / 365 days or Forever) and 5,000 entries. Notification history on PC is capped at 300 dismissed items and 24 hours. `MessagingStyle` conversation cache retains up to 200 threads per peer (200 messages/thread) for 90 days. Photo thumbnail cache on PC keeps up to 500 thumbnails for 30 days. Both apps expose per-category clear buttons and a single **"Clear everything"** action in *Settings → Data & storage* that wipes all local history and caches while keeping device identity, paired devices, and feature toggles intact.
- Security audit before 1.0. Fuzz the protocol decoder and pairing handshake. The credential provider gets its own threat model and review.

### 4.4 Connectivity: "it just works"
Order of attempts (iroh races paths and keeps the best one; multipath lets two paths coexist):
1. **LAN direct:** mDNS address lookup + last-known addresses + a tiny UDP beacon fallback for networks that filter mDNS.
2. **USB:** when ADB is available, a forwarded path (iroh custom transport, currently `unstable-custom-transports`; fallback is a TCP-framed tunnel). It also works with no network at all.
3. **Away:** hole punching, then relay. **Opt-in** ("Reach my phone away from home"). Default relays at first, with a one-click self-hosted relay option. A Nectarlink-operated relay pool is possible later if donations allow.
4. **Bluetooth** (P2): low-bandwidth link for notifications, SMS and calls.

**Staying connected:**
- Android: foreground service (`connectedDevice`), CDM association (exempts us from many background limits and lets the OS wake us on BLE presence), battery-optimization exemption request, and OEM-specific guidance (dontkillmyapp data).
- Windows: resume on `PBT_APMRESUMEAUTOMATIC`, network-change listener, BLE advertising so the phone's CDM sees the PC.
- Heartbeats with adaptive interval, instant reconnect on network change, exponential backoff, and a visible "connection health" indicator.

**Connection Doctor** checks and fixes: firewall rule (the installer adds it, Doctor repairs it), network profile Public→Private prompt, AP isolation (detected when the relay path works but the LAN path doesn't), VPN split tunnelling, phone battery mode, restricted settings, and Power Level status.

### 4.5 Protocol
- One QUIC connection per device pair.
- **Control stream:** `Hello` (versions, capabilities, power level, device info), then the event bus (battery, notification posted/removed, clipboard, media state…).
- **Per-operation streams:** each RPC, file transfer, thumbnail fetch, voice recording or mirror session opens its own bidirectional stream with a typed header. There's no head-of-line blocking, so a 4 GB transfer never delays a notification.
- **Datagrams:** pointer/touchpad/air-mouse motion and other latency-critical, loss-tolerant input.
- **Versioning:** capability flags, unknown fields ignored, unknown message types answered with `Unsupported`. A newer phone always works with an older PC (minus new features).
- **Transfers:** chunked, content-hashed (BLAKE3), resumable by offset, parallel streams for many small files, zero-copy reads where possible.

### 4.6 Power Levels (one APK, no extra apps)
Everything ships inside the single NectarLink APK. "Elevated" is **not a separate app**: it's a mode where NectarLink launches a copy of its *own* code (via `app_process`) with `shell`-user privileges. That process talks back to the app over a local socket with strict caller checks (our UID + signing certificate). Shizuku is only **one optional path** for people who already have it.

| Level | How you get it | Effort | Lasts | Adds (on top of the level above) |
|---|---|---|---|---|
| **Basic Basic** | Install + grant normal permissions | ~1 min | Forever | Notifications (non-sensitive), SMS/MMS, calls, contacts, files, photos, media both ways, PC→phone clipboard, webcam, Continuity Camera, voice recorder, voice typing, touchpad/air mouse/Deck, standard mirroring (consent prompt each session, view-only) |
| **Assist Assist** | Turn on NectarLink's Accessibility service (one toggle, plus the "restricted settings" step for sideloaded installs) | ~1 min | Forever | Control the phone during standard mirroring, global actions (back/home/recents/lock) |
| **Elevated Elevated** | Any one of the setup paths below | 1 to 3 min | Until phone reboot (auto re-arms) | **Auto** phone→PC clipboard, OTP/sensitive notifications, prompt-free mirroring with full control, phone audio to PC, **app windows**, type into the phone with the PC keyboard, Wi-Fi/BT/DND toggles from PC, install APKs from PC, self-whitelist from battery killers |

**Setup paths to Elevated** (NectarLink recommends the best one by detecting Android version, USB state and Shizuku):
1. **USB cable:** guided "enable USB debugging" (animated, per-OEM steps), then plug in. The PC app has a **built-in ADB client**, so there's nothing else to install. This also turns on the USB connection path.
2. **Wireless, using the PC's QR** (Android 11+): the PC shows an ADB-pairing QR, and the phone scans it in *Developer options → Wireless debugging → Pair with QR code*. No cable.
3. **Phone only** (Android 11+): NectarLink opens Wireless debugging, the user types the pairing code into a NectarLink notification, and NectarLink pairs with itself. No PC needed.
4. **Shizuku / Sui:** one tap if already installed. Never required.

**Re-arming after a reboot (keeping it painless):**
- If the PC is paired for wireless debugging, the PC silently re-arms the phone the next time both are on the same network.
- On Android 13+, NectarLink grants itself `WRITE_SECURE_SETTINGS` during the first Elevated session. After that it can switch wireless debugging back on by itself on trusted Wi-Fi, on devices where the OEM allows it.
- Otherwise it shows a single quiet notification: "Tap to restore superpowers".

**Capability-driven UI** (the same rules on both apps):
- Every feature declares its requirements: power level, Android version, Windows add-on, permission, connection path.
- The core computes a **capability matrix per paired device**, and both UIs render from it, so they never disagree.
- Each feature has one of four states:
  - **Available**
  - **Partial:** works with an inline limit, e.g. "Manual: tap to send · Auto needs Elevated"
  - **Locked:** greyed, with a chip showing the level and effort, e.g. "Elevated · ~2 min"
  - **Unsupported:** explained, e.g. "Needs Android 11+"
- Tapping anything locked opens *that exact* setup sheet, then drops you back into the feature, now working.
- **Onboarding: "Choose your power"** (after pairing). Cards for Basic / Assist / Elevated, each showing what it unlocks, what you need, how long it takes, how to undo it, and a plain-language "what this access means" explainer. A smart default is pre-selected, and "Decide later" is always available.
- **Settings → Power:** current level, how it was obtained, re-arm status, and a one-tap **Revoke**.
- **Settings → Connections** (separate axis): Wi-Fi/LAN, USB, Away (relay), Bluetooth. Each shows live status, or a greyed state with "Set up".
- **PC add-ons** use the same pattern on Windows: Virtual camera, Explorer integration, Bluetooth calling, Firewall rule, Unlock with phone. Each is opt-in with an explanation, and asks for admin only when it truly needs it.

### 4.7 Mirroring pipeline
- **Phone:** MediaCodec hardware encoder (H.265 → H.264 fallback, AV1 where hardware supports it), adaptive bitrate from QUIC congestion signals, intra-refresh to avoid big keyframes. The Elevated engine reuses scrcpy-server's capture/virtual-display/UHID code (Apache-2.0, credited), carried over **our** QUIC stream instead of an ADB socket.
- **PC (native, no WebView):** Media Foundation hardware decoder (DXVA) → D3D11 texture → wrapped directly as a texture in Qt Quick's scene graph (Qt renders with D3D11 on Windows, so both share the GPU device). Zero copies from decoder to screen. Controls, rounded app windows, overlays and transitions are ordinary QML on top of live video. If the scene-graph path ever adds latency, the fallback is a dedicated low-latency swapchain for the video surface. On Mac later: VideoToolbox → Metal behind the same `VideoSink`.
- **Input:** keyboard via UHID (correct layouts, shortcuts), mouse via UHID/inject, touch emulation, gamepad. Motion events go over datagrams.

### 4.8 Windows integration map
| Need | API |
|---|---|
| Toasts with images, actions, inline reply | Windows App Notifications + COM activator (AUMID) |
| Package identity (share target, Win11 context menu, HFP, toasts) | **Sparse package ("packaged with external location")** on top of the normal installer |
| Phone storage in Explorer | Cloud Files API (`cfapi`) sync root with placeholders |
| PC media → phone | `GlobalSystemMediaTransportControlsSessionManager` |
| Phone media → Windows media flyout | `SystemMediaTransportControls` |
| Clipboard | Clipboard listener, honoring `ExcludeClipboardContentFromMonitorProcessing` |
| Webcam | `MFCreateVirtualCamera` + custom media source |
| Calls | Bluetooth HFP via `PhoneLineTransportDevice` (spike) |
| Mirror video | Media Foundation decode → D3D11 texture shown in the Qt Quick scene graph |
| Voice typing / Deck / air mouse into PC | `SendInput` (Unicode text + mouse), virtual-key macros |
| Unlock with phone | Credential provider (`ICredentialProvider`) |
| "Ping me when done" | Process wait handles, BITS/download folder watchers, CLI hooks |
| Proximity / wake | `BluetoothLEAdvertisementPublisher` / watcher |
| Window materials | Mica / Mica Alt / Acrylic via `DwmSetWindowAttribute` (DWMWA_SYSTEMBACKDROP_TYPE) |
| Firewall, startup, tray, jump list, hotkeys | installer custom action, Run key / StartupTask, Shell_NotifyIcon, `ITaskbarList`, `RegisterHotKey` |
| Power events | `RegisterPowerSettingNotification`, network list manager |

### 4.9 Android integration map
| Need | API |
|---|---|
| Stay alive | FGS `connectedDevice`, CompanionDeviceManager + presence observing, battery exemption |
| Notifications | `NotificationListenerService` (+ RemoteInput replies, actions, MessagingStyle parsing) |
| SMS/MMS, call log, contacts | Telephony & Contacts providers |
| Calls | `TelephonyCallback`, `TelecomManager.acceptRingingCall/endCall`; in-call controls through an `InCallService` bound as a calling companion (`MANAGE_ONGOING_CALLS`, granted by Elevated) |
| Media | `MediaSessionManager.getActiveSessions`, our own `MediaSession` for PC media |
| Voice recorder | `AudioRecord` / MediaCodec (Opus/AAC), with format conversion in the Rust core on PC for MP3/WAV/FLAC |
| Voice typing | `SpeechRecognizer` (on-device where available) |
| Air mouse | Rotation-vector / gyroscope sensors |
| Unlock PC | `BiometricPrompt` + Keystore keys (StrongBox) bound to user authentication |
| Photos/files | MediaStore, SAF, `MANAGE_EXTERNAL_STORAGE` |
| Camera / Continuity Camera | CameraX → MediaCodec, ML Kit document scanner (or open-source edge detection) |
| Mirror (standard) | MediaProjection, AccessibilityService input |
| Surfaces | QS tiles, Glance widgets, share target, Direct Share, App Shortcuts |

### 4.10 Performance & quality budgets
| Metric | Target |
|---|---|
| PC cold start → window | < 500 ms (QML pre-compiled at build time) |
| PC RAM, tray + Island / main window open | **< 60 MB / < 120 MB** (+5 to 15 MB per extra window) |
| PC idle CPU | ~0 % |
| Phone battery while connected and idle | < 2 % per day |
| Notification latency (LAN) | < 300 ms |
| Mirror glass-to-glass (LAN, 1080p60) | < 50 ms |
| File transfer | ≥ 80 % of link speed |
| Reconnect after PC wake | < 3 s |
| UI | 60 fps minimum, 120 fps on capable displays, no frame > 8 ms of UI work during animations |

---

## 5. Tech stack: reasoning and trade-offs

What we optimize for, in order:
1. Reliability of the connection
2. Design ceiling and smoothness
3. **Low resource use** (the maintainer's hard requirement: no RAM-hungry web stack)
4. Depth of OS integration on both sides
5. One shared engine
6. A path to macOS later

### 5.1 Desktop UI framework: decision record

**Decision: Qt Quick (QML) for the UI, Rust for everything else.** Fallback: web UI (Tauri) with native Rust video windows.

#### Final three, scored (1 to 5, weighted for NectarLink; RAM/size/start figures are estimates to verify in Phase 0)
| Criterion | Weight | A. Web (Tauri 2 + React) | **B. Qt Quick + Rust** (done) | C. Slint |
|---|---|---|---|---|
| Visual ceiling (blur, glass, effects, type) | 15 | 5 | **5** (real blur, shaders, particles) | 3 (no in-app blur/shaders yet) |
| Smoothness under load | 10 | 4 (busy JavaScript can stutter) | **5** (animations on a render thread) | 4 |
| RAM / idle CPU | 10 | 2 (~120 to 200 MB with a window, +30 to 60 MB per window) | **4** (~50 to 60 MB tray + Island, ~80 to 120 MB with a window) | 5 (~15 to 60 MB) |
| Startup + install size | 5 | 4 (~8 to 15 MB, 0.5 to 1 s) | **3** (~35 to 60 MB, 0.3 to 0.6 s) | 5 |
| Mirroring & app windows | 15 | 3 (extra frame copy, a web process per window) | **5** (video inside the UI, no copies, cheap windows) | 3 (separate surface beside the UI) |
| Always-on windows (Island, Shelf) | 5 | 2 | **5** | 5 |
| Windows integration & native feel | 5 | 4 | **4** | 4 |
| Accessibility | 5 | 5 | **4** | 3 |
| Development speed (incl. how accurately Claude writes it) | 10 | 5 | **3** | 3 |
| Build / CI simplicity | 5 | 4 | **2** | 5 |
| Ecosystem & contributors | 5 | 5 | **3** | 2 |
| Maturity / long-term risk | 5 | 4 | **3** (cxx-qt is pre-1.0) | 3 |
| Fit with the Rust core | 3 | 4 | **3** | 5 |
| Mac later | 2 | 4 | **4** | 4 |
| **Weighted total / 100** | | **≈ 78** | **≈ 81** | **≈ 74** |

**Why B wins (narrowly):** NectarLink is *always running*, *video-heavy* (mirroring plus one window per phone app) and has *always-on overlays* (Island, Shelf). Those are exactly the web stack's weakest points and Qt's strongest, and Qt gives up nothing on looks. If development speed were weighted above ~20 %, web would win, which is why it is the fallback.

**Rejected outright:** Flutter (maintainer's call; multi-window still experimental), Electron (RAM), WinUI 3 (Windows-only), Compose Multiplatform (JVM RAM), Avalonia (C# bridge), GPUI (development slowed in 2026), iced/egui (not built for this).

#### Known issues with B, and the rules that contain them
| Issue | Rule / mitigation |
|---|---|
| Three languages in the UI layer (QML, Rust, a little C++) | **QML is presentation only**: screens, layout and animation. No business logic in QML/JavaScript. C++ is limited to the video item and a few window tweaks |
| cxx-qt is pre-1.0 (0.10.x, KDAB) | Pin the version, upgrade on purpose. **One bridge module (`desktop/app/src/bridge`)** is the only place Rust touches Qt |
| Heavier build (Qt install, C++ glue, separate ARM64 build) | Install Qt via aqtinstall, cached in CI. Windows ARM64 is a supported Qt platform (MSVC 2022). Scripts make it one command locally |
| Two threading worlds (Rust async runtime vs Qt's screen thread) | **One pattern everywhere** for handing core events to the screen thread (cxx-qt's thread queue). No ad-hoc cross-thread calls |
| Bigger installer (+30 to 50 MB) | Ship only the Qt modules we use. `windeployqt` with an explicit module list |
| Startup cost of QML | Pre-compile QML to bytecode at build time (`qmlcachegen`, LGPL) |
| Mica on a Qt window | Direct DWM calls on the native window handle, plus a transparent QML root |
| **Qt licensing pitfalls** | Use only LGPL modules (Core, Gui, Qml, Quick, Quick Controls, Quick Effects/MultiEffect, Quick Shapes, Svg, Multimedia). **Avoid GPL-only modules:** Qt Qml Compiler (`qmlsc`), Lottie, Quick Timeline, Graphs, Quick 3D, Virtual Keyboard, Canvas Painter. They'd conflict with the future app-store exception. Dynamic linking satisfies LGPL relinking |
| Qt LTS patch releases become **commercial-only** after their first period | Track the **latest Qt 6 minor release** (currently 6.12), pinned per NectarLink release, instead of relying on LTS patches |
| Qt's long-term openness | Guaranteed by the KDE Free Qt Foundation agreement: if open-source releases ever stop, Qt becomes BSD-licensed |

#### Risk gate (Phase 0): "Qt + Rust proof"
Build Home, a notification card, the Deck grid, one hero transition and a live mirrored-video window from the approved mockups.
- **Pass:** it matches the mockups, holds 120 fps on a mid-range laptop, meets the §4.10 RAM and startup budgets, shows video inside the UI without copies, Mica works, core→UI updates survive a stress test, and CI builds x64 + ARM64 installers.
- **Fail:** switch to **A: web UI (Tauri) with native Rust video windows** (mirroring and app windows as pure D3D11 windows, which removes web's worst weakness). The core, protocol, Windows integrations and video path don't change.

### 5.2 Android
| Option | Verdict |
|---|---|
| **Kotlin + Jetpack Compose + Material 3 Expressive** (done) | Native (no web). Every system API is first-class: notification listener, telephony, Companion Device Manager, MediaProjection, CameraX, Glance widgets, Quick Settings tiles. M3 Expressive gives shape morphing and spring motion out of the box. Best performance and battery |
| Flutter / React Native | Rejected. Every key feature would be a hand-written native plugin anyway. You pay for two stacks and get worse background-service behavior |
| Compose Multiplatform | Only pays off if the desktop were also Kotlin (rejected: JVM RAM) |

### 5.3 Shared core language
| Option | Pros | Cons | Verdict |
|---|---|---|---|
| **Rust** (done) | Fast, no GC pauses (matters for video and transfers), memory-safe network parsing, runs on Windows/Android/macOS/iOS, iroh is native Rust | Slower compile times, steeper learning curve, UniFFI layer on Android, +5 to 8 MB per ABI in the APK (we ship arm64 mainly) | **Chosen** |
| Go (gomobile) | Simple | gomobile is stagnant, GC, large binaries, weak FFI story | Rejected |
| Kotlin Multiplatform | Native on Android | Desktop isn't Kotlin | Rejected |
| C++ | Universal | Memory-safety risk in exactly the code that parses untrusted network input | Rejected |
| No shared core (two implementations) | Simpler builds | KDE Connect's lesson: two implementations drift and cause interop bugs | Rejected |

### 5.4 Transport
| Option | Pros | Cons | Verdict |
|---|---|---|---|
| **iroh (QUIC)** (done) | Dial by key, E2E by default, LAN discovery, ~90 % hole-punch success, relay fallback over HTTPS (works even where UDP is blocked), multipath, many streams per connection. MIT/Apache, stable 1.x | Young (1.0 in June 2026). Away mode depends on relays | **Chosen**, behind our own `Connection` trait so it's swappable |
| Hand-rolled TCP + TLS (KDE Connect, Sefirah) | Full control | We'd re-implement discovery, NAT traversal and multiplexing, which is exactly where competitors break | Rejected |
| HTTP/REST (LocalSend) | Simple, debuggable | One request per action, no push, no NAT traversal | Implemented **only** for LocalSend interop |
| WebRTC | NAT traversal + media pipeline | Huge dependency (libwebrtc), needs a signaling server, hard to debug | Rejected |
| libp2p | Feature-rich | Heavier and more complex than we need | Rejected |

### 5.5 Smaller choices
| Choice | Picked | Alternative | Why |
|---|---|---|---|
| Wire format | CBOR (serde) | Protobuf | No codegen toolchain, self-describing, forward-compatible. A versioned spec still lets others implement it |
| Local storage | SQLite (in core) | Room / per-platform DBs | One schema, one migration path, shared by both apps |
| Hashing | BLAKE3 | SHA-256 | Much faster for large files, built for streaming and verification |
| Desktop rendering | Qt Quick scene graph via RHI (Direct3D 11 on Windows, Metal on Mac) | OpenGL / software | Native GPU APIs, and our decoded video shares the same D3D11 device |
| Audio formats (voice recorder) | Opus/AAC on phone, transcode on PC in Rust (Symphonia decode + encoders) | Transcode on phone | Saves phone battery, PC is faster |
| Fonts | Bundled (Segoe UI Variable on Windows + one display face) | Downloaded at runtime | Works offline, no third-party requests |
| Mirroring engine | scrcpy-server (Elevated) + MediaProjection (Basic) | Write our own from scratch | scrcpy is the best in the world at this, and Apache-2.0. We add what it lacks (UX, pairing, transport) |

---

## 6. Design direction

**Feel:** calm, premium, alive. Think "Apple Continuity polish with Material You warmth". It should never look like a settings dump.

**Shared language across both apps**
- Same iconography, the same names for features, and the same power-level badges (mapped to the theme's own palette, no traffic-light colors).
- **Dynamic color:** default warm honey/amber accent (on brand for NectarLink). Once paired, both apps can adopt the phone's Material You palette.
- Motion: spring-based (no linear easing), choreographed transitions (a notification card grows into the conversation view), honoring reduced motion.

**Desktop (Qt Quick)**
- Mica backdrop, layered surfaces, real in-app blur and glass (MultiEffect), subtle shader accents, Segoe UI Variable for body text plus a characterful display face for headings.
- Layout: slim left rail (device switcher at top) → **Home** with a live "phone hero" (wallpaper, battery, signal, now playing, quick toggles), feature surfaces (Messages, Notifications, Photos, Files, Apps, Calls, Deck), and settings that read like explanations, not checkboxes.
- Keyboard-first: Ctrl+K palette in-app plus a global hotkey. Every action is reachable by keyboard.
- Ambient UI: Nectar Island, Shelf and tray flyout, so most interactions never open the main window.

**Phone (Compose)**
- Material 3 Expressive. A big tactile card for the connected PC (online state, laptop battery, quick actions: send clipboard, record voice, Deck, lock PC, mirror, touchpad).
- Onboarding is one live checklist where each permission shows its real state, why it's needed, and one tap to grant. Then "Choose your power".
- Predictive back, haptics, edge-to-edge, and adaptive layouts for tablets and foldables.

**Setup target (< 60 s):** install PC app → it shows a QR → install phone app → scan → checklist → done. No firewall dialogs (the installer handles it) and no ADB knowledge needed.

Themes are decided (Bloom default + Graphite; see §10.3). Phase 0 produces the design tokens + hi-fi mockups of Home, Onboarding, Choose-your-power, Messages, Mirror window, Island and Deck, before any UI code.

---

## 7. Distribution

**GitHub-first.** No Play account and no Microsoft Store for now.

- **Windows 11 only** (Windows 10 is dropped), x64 + **native ARM64** (Snapdragon laptops are a big Phone Link audience).
  - **Installer:** NSIS or WiX installer on GitHub Releases, which adds the firewall rule and registers the sparse-package identity.
  - **winget:** manifest pointing at the GitHub release (free).
  - **Auto-update:** signed update manifests, with delta updates where possible.
  - **Code signing:** see §10.6. Early alphas may ship unsigned, with an honest SmartScreen note.
- **Android:** the `full` flavor only.
  - **GitHub Releases APK** with a built-in updater that checks GitHub releases and installs through `PackageInstaller`.
  - **Obtainium**-friendly release naming. Recommended in docs, since session-based installs also reduce "restricted settings" friction.
  - **IzzyOnDroid** repo once builds are reproducible.
  - One signing key forever, enrolled in developer verification before global enforcement.
- **Later:** Microsoft Store (MSIX), Google Play (`play` flavor), Mac (needs an Apple Developer account for notarization).
- **Process:** reproducible builds where feasible, public CI (GitHub Actions is free for public repos, Windows and macOS runners included), signed release artifacts and readable changelogs.
- **Community:** build in public via the maintainer's existing X audience, run a tester program from that audience (no in-house test-device budget), and a public roadmap + changelog.

---

## 8. Roadmap

Each phase ends with a usable, releasable product. Phase sizes are relative; actual pace depends on contributors.

### Phase 0: Foundations
- Monorepo, CI, coding standards, ADRs, contribution guide, DCO, license files, trademark policy.
- Design system + 2 to 3 visual directions + mockups for both apps (sign-off before UI code).
- `nectarlink-core`: identity, iroh endpoint, pairing (QR + SAS), session/hello/capabilities, event bus, SQLite store.
- Desktop shell (tray, window, Mica, theming, onboarding with QR); Android shell (FGS, CDM, onboarding checklist, QR scanner).
- **Spikes (de-risk early):**
  - **Qt + Rust proof (§5.1 risk gate)**
  - Native mirror latency (MF + D3D11)
  - HFP calling on Win11 24H2/25H2
  - iroh on Android (battery + reconnect behavior)
  - iroh USB custom transport
  - Sparse package identity + toast inline reply
  - Phone-only wireless-debugging self-pairing and automatic re-arm (Android 11 to 16; Pixel/Samsung/Xiaomi)
- **Exit:** pair phone and PC by QR in under 60 s, see each other live (battery, name), survive PC sleep and Wi-Fi changes, and the Qt + Rust proof passed (or the web-UI fallback chosen).

### Phase 1: Daily Driver (public alpha, v0.1 to 0.3)
- Notifications (images, actions, replies, dismiss sync, filters, history).
- Clipboard (text/images; PC→phone auto; phone→PC via tile/share).
- Files both ways (drag-drop, share sheet, Explorer "Send to phone"), resumable transfers.
- Media control both ways (incl. Windows media flyout), battery/status, find my phone, lock/sleep PC, link handoff.
- Connection Doctor v1, auto-start, auto-update, crash-free reconnect.
- **Exit:** a KDE Connect / Phone Link-basics user can switch fully. Reliability green for 7 days across the tester group's phones (Pixel, Samsung, Xiaomi, OnePlus at minimum).

### Phase 2: Superpowers (beta, v0.4 to 0.6)
- **Power Levels** with all setup paths (USB, PC QR, phone-only self-pairing, Shizuku) + capability-driven greyed UI + re-arm → auto clipboard, OTP notifications.
- **Mirroring** (Standard + Elevated engines), audio, keyboard/mouse.
- **App windows** (virtual displays, taskbar icons).
- SMS/MMS + contacts. Calls (alerts, control, log; HFP audio if the spike passed).
- Recent photos + gallery. Phone storage in Explorer.
- Touchpad/keyboard/presentation remote, **air mouse**, **voice typing**, **voice recorder → PC**.
- **Exit:** replaces scrcpy and Sefirah for typical users, with no manual tooling.

### Phase 3: Magic (v0.7 to 0.9)
- Shelf, Command Palette, Material You sync, Smart OTP, Live Updates, **Timeline**.
- **Deck**, **Ping me when it's done**, **Continuity Camera**, **Smart clipboard chips**, **Flick to send**.
- Phone as webcam. Unified Conversations (chat apps).
- **Away mode** (relay, self-host guide) + first slice of **PC in your pocket** (files + status), CDM/BLE wake, USB transport.
- LocalSend interop, CLI + local API, phone toggles from PC, custom commands, Wake-on-LAN.
- **Exit:** feature superset of Phone Link + KDE Connect + LocalSend + scrcpy for Android↔Windows.

### Phase 4: 1.0 and beyond
- Security audit, fuzzing, accessibility audit, i18n (community translations), docs site.
- **1.0 release.**
- After 1.0:
  - **Unlock PC with phone fingerprint** (own workstream + external review)
  - PC in your pocket: reverse mirroring
  - Flow (cross-device mouse)
  - Nectar Island (optional), proximity lock, automations, folder sync
  - Tablet as second screen, phone as mic/speaker
  - Bluetooth fallback link
  - **macOS desktop**, plugin API

### 8.5 Pre-Release User Feedback & Polish Queue (Ranked & Reasoned)

| Rank | Item | Fit in Plan & Rationale | Execution Plan |
|---|---|---|---|
| **1 (P0, Core State & Device Identity)** | **Surface connected devices first, and deduplicate re-paired phones** | Fits **§3.1 Connection & setup**. Sorting `devices` by `paired_at` ascending left old offline test pairings (`Pixel 9` from an earlier `--as-phone` CLI demo) and stale pre-reinstall pairings ahead of the actively connected phone. Connected devices must always be surfaced first and auto-selected, and re-pairing the same physical phone (`name`, `kind`, `os`, `model`) must replace the stale offline record instead of duplicating tabs and Settings sections. | 1. Sort `state.devices` with `Online` devices first, then `last_seen` descending, then `paired_at` descending; re-sort on `LinkChanged` and auto-switch `currentDevice` when a device connects.<br>2. Deduplicate peers with identical `(name, kind, os, model)` in `store.rs` / `node.rs` (`complete_pairing` and startup migration) and prune synthetic CLI demo peers (`name = 'Pixel 9' AND model IS NULL`) when real phones exist. |
| **2 (P1, Camera & Video)** | **Fix Phone-as-Webcam 90° rotation, aspect-ratio stretching, and live camera controls** | Fits **§3.9 Camera (P1)**. In `GlSurfacePipe.kt`, `SurfaceTexture.getTransformMatrix(stMatrix)` already incorporates the Camera2 sensor buffer transform (`NATIVE_WINDOW_SET_TRANSFORM`), so multiplying by an extra manual `rotationDegrees` matrix caused a 90° counter-clockwise rotation and aspect-ratio distortion. Users also need immediate access to Front/Back camera, 720p/1080p/4K resolution, Mirror, Torch, and Zoom. | 1. Fix `GlSurfacePipe.kt` and `CameraEncoder.kt` so `stMatrix` is used without double-rotating and portrait/landscape dimensions match the upright frame.<br>2. Ensure `VideoView` in `SettingsPage.qml` and `HomePage.qml` preserves source aspect ratio (letterboxed/pillarboxed, never stretched) and expose full camera controls. |
| **3 (P1/P2, Media & Audio)** | **First-class real-time "Phone as PC Speaker" on PC Home & Android Home** | Fits **§3.8 Media & audio**. While the WASAPI loopback → QUIC `audio.pcm` → Android `AudioTrack` pipeline was added in §3.8, burying it inside Settings made it hard to reach. Users with weak laptop/PC speakers want a one-tap real-time toggle on both PC Home and Phone Home with ultra-low latency (~10 to 20 ms) and zero drift. | 1. Add bidirectional `audio.pc_speaker` control message so either the phone or PC can toggle streaming.<br>2. Add a prominent **"Phone as speaker"** quick action tile/toggle on `HomePage.qml` and Android `HomeScreen.kt`.<br>3. Tune `pc_audio_stream.rs` (5 ms WASAPI loopback cadence) and `PcSpeakerPlayer.kt` (drift-free low-latency `AudioTrack`) so audio stays locked to PC video playback. |
| **4 (P0, Desktop UI Architecture)** | **Full redesign of `SettingsPage.qml` into a 2-pane categorized layout + `ListRow.qml` layout fix** | Fits **§6 Design direction ("settings that read like explanations, not an endless dump")**. `SettingsPage.qml` grew into 18 vertically stacked cards (~3,000 lines) with endless scrolling, and `ListRow.qml` anchored `texts.right` to `trailingSlot.left`, crushing the `Color` row title into `"C..."`. | 1. Fix `ListRow.qml` so `texts` always retains a readable minimum width and wide trailing controls wrap cleanly.<br>2. Redesign `SettingsPage.qml` into a professional two-pane layout with a left category sidebar (**Appearance**, **General**, **Devices**, **Notifications**, **Camera & Audio**, **Files & Sync**, **Shortcuts & Automations**, **Storage & Privacy**, **Connection & About**) and a single-device selector inside **Devices**. |
| **5 (P0, Desktop Home Polish)** | **Declutter `HomePage.qml` & fix offline `LockChip` state** | Fits **§6 Design direction**. Showing an empty `LocalSend` card when 0 peers are on the LAN, stacking separate full-width cards for `Continuity Camera` and `Type on phone`, and showing a lock and `UPDATE APP` on `ActionTile` when a device is simply offline created visual clutter. | 1. Hide `LocalSendCard` on Home when `localsendPeers.length === 0`.<br>2. Consolidate `Continuity Camera` and `Type on phone` into a clean, compact row.<br>3. Suppress `LockChip` (`Update app`) on `ActionTile` when `!home.online`. |
| **6 (P0, Release Hardening)** | **Complete Section 3 Release Hardening ** | Fits **Phase 4 Release Gate**. After UI and feature changes land, run full live verification, R8 release APK test, `docs/PERFORMANCE.md`, and the 2-hour soak test. | Execute remaining Section 3 items (3.8 R8 live test, 3.9 states verification, 3.1 3.7 `docs/PERFORMANCE.md`, 3.4 2-hour soak test). |

### 8.6 Feature Completion & Expansion Specification (Sections 1 to 4)

#### Section 1: Partially Complete Core Features (High-Impact Completion)

| # | Feature | Current State | Architecture & Implementation Spec |
|---|---|---|---|
| **1.1** | **Prompt-Free Elevated Full-Screen Mirroring & Modern Codecs (`H.265` / `AV1`)** (§3.7) | Full-screen mirroring (`session == 0`) always invokes `MirrorConsentActivity` (`MediaProjection`), causing Android 14+'s per-session capture prompt even when Elevated (`shell` UID 2000) is active. Codec is hardcoded to H.264 (`h264`). | 1. **Prompt-Free Elevated Capture**: Extend `InputServer.kt` (`app_process` running as `com.android.shell`, UID 2000) and `AppDisplay.kt` with a physical-screen capture mode (`M <width> <height> <dpi> <bitrate> <fps> <codec>`). Grant `PROJECT_MEDIA` appop and create a mirroring surface via `ShellContext` `DisplayManager.createVirtualDisplay(..., VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR)` with fallback to `SurfaceControl` display projection (`createDisplay` + `setDisplaySurface` + `setDisplayProjection` + `setDisplayLayerStack`). When `Elevated.running` is true, `MirrorService.startElevated` streams the physical display immediately with **zero `MediaProjection` consent dialog**, falling back to `MirrorRequests.show` only if Elevated is inactive.<br>2. **H.265 (HEVC) & AV1 Codec Support**: Add `mirror::H265` (`"h265"`) and `mirror::AV1` (`"av1"`) to `nectarlink-protocol` (`MirrorStart.codec`, `MirrorConfig.codec`). Update Windows `Decoder` (`desktop/app/src/win/h264.rs`) to instantiate Media Foundation's HEVC (`MFVideoFormat_HEVC`) and AV1 (`MFVideoFormat_AV1`) hardware/inbox MFT decoders via `MFTEnumEx` alongside `CLSID_MSH264DecoderMFT`. Update Android `ScreenEncoder.kt` and `AppDisplay.kt` to configure `MediaFormat.MIMETYPE_VIDEO_HEVC` / `MIMETYPE_VIDEO_AV1` when requested and supported by hardware, with automatic fallback to `MIMETYPE_VIDEO_AVC`. |
| **1.2** | **Reverse Mirroring: "Your PC in Your Pocket"** (§3.11) | Phone can browse PC files, view CLI tasks, and send touchpad/keyboard input, but cannot view and interact with the live PC desktop screen. | 1. **Windows Desktop Capture & H.264 Hardware Encode (`desktop/app/src/pc_mirror.rs`)**: Implement `Platform::mirror_requested`, `mirror_stop_requested`, `mirror_keyframe_requested`, and `mirror_input` on `DesktopPlatform`. Capture the primary monitor at up to 60 fps using DXGI Desktop Duplication (`IDXGIOutput1::DuplicateOutput`) with GDI `BitBlt` fallback, convert BGRA→NV12, encode via Windows Media Foundation H.264 Encoder MFT (`CLSID_CMSH264EncoderMFT` / `MFTEnumEx`, `MF_LOW_LATENCY = 1`), and stream over `Node::mirror_open`. Map incoming `MirrorInput` (`Touch`, `Scroll`, `Key`, `Text`) to absolute cursor positioning, mouse clicks/drags, wheel events, and keystrokes via `SendInput` when `remote_input` is allowed.<br>2. **Core & FFI Symmetry (`core/nectarlink-core`, `core/nectarlink-ffi`)**: Advertise `mirror.capture` on Desktop and `mirror.view` on Android. Bridge `Platform::mirror_sink` in `nectarlink-ffi` to emit `Event::PcMirrorConfig`, `Event::PcMirrorFrame`, and `Event::PcMirrorEnded`, and expose `startPcMirror`, `stopPcMirror`, `requestPcMirrorKeyframe`, and `sendPcMirrorInput` on `CoreNode`.<br>3. **Android Live PC Viewer (`PcMirrorScreen.kt`)**: Hardware `MediaCodec` H.264/H.265 decoder rendering to a Compose `SurfaceView` with touch-to-click, drag, two-finger scroll, pinch-to-zoom/pan, right-click toggle, and keyboard bar, accessible from `HomeScreen.kt` and `RemoteScreen.kt`. |
| **1.3** | **Voice Recorder Live Streaming, Opus & Noise Suppression** (§3.8) | Phone records M4A and transcodes finished files on PC to M4A/MP3/WAV/FLAC with markers, but lacks on-device noise suppression, Opus encoding, and live real-time audio monitoring on PC while recording. | 1. Attach Android hardware `NoiseSuppressor` and `AutomaticGainControl` to `RecorderService`'s `AudioRecord` session when enabled.<br>2. Add live PCM streaming (`Core.sendAudioPcm` / `audio.pcm`) option while recording so the PC can monitor or route the phone mic in real time.<br>3. Add `.opus` / Ogg Opus output support alongside M4A/MP3/WAV/FLAC. |
| **1.4** | **Deck: OBS Studio WebSocket Tile** (§3.10) | Deck supports system audio, media, shortcuts, apps, scripts, and windows, but has no native OBS Studio integration. | Implement native OBS WebSocket v5 (`ws://127.0.0.1:4455`) client in `desktop/app/src/deck.rs` supporting `obs_stream_toggle`, `obs_record_toggle`, `obs_replay_save`, `obs_mic_mute`, and `obs_scene:<name>` tiles without requiring external plugins. |

#### Section 2: Deferred P2 OS & Hardware Integration Features

| # | Feature | Target & Constraints | Architecture & Implementation Spec |
|---|---|---|---|
| **2.1** | **Proximity Lock (BLE RSSI Ranging)** (§3.11) | Lock Windows automatically when the paired phone walks away. | Continuous BLE advertisement RSSI smoothing (EMA filter + hysteresis threshold, e.g. `< -82 dBm` for `> 12 s` or link loss while on battery) in `desktop/app/src/win/ble_beacon.rs` + Android BLE scanner, triggering `LockWorkStation()` when enabled in Settings. |
| **2.2** | **Unlock PC with Phone Fingerprint (`desktop/credprov`)** (§3.10) | Unlock Windows lock screen via phone `BiometricPrompt`. | Isolated Windows `ICredentialProvider` DLL (`desktop/credprov`) communicating over a local named pipe with the NectarLink service, issuing a 256-bit nonce signed by an Android Keystore `StrongBox` Ed25519/ECDSA key bound to `BiometricPrompt`. Requires external security review before enabling by default. |
| **2.3** | **Bluetooth RFCOMM Fallback Link** (§3.1) | Keep notifications, SMS, and call alerts alive when no Wi-Fi/LAN or USB is available. | Classic Bluetooth RFCOMM socket transport (`Windows.Devices.Bluetooth.Rfcomm` ↔ Android `BluetoothAdapter.listenUsingRfcommWithServiceRecord`) multiplexed into `nectarlink-core` as a low-bandwidth control path. |
| **2.4** | **Tablet / Phone as Second Monitor (`IddCx`)** (§3.12) | Extend the Windows desktop onto an Android tablet/phone as a virtual display. | Pair an open-source signed UMDF2 `IddCx` indirect display driver on Windows with the Reverse Mirroring pipeline (§1.2) capturing the virtual display output at 60 to 120 fps. |
| **2.5** | **Phone as Live PC Microphone** (§3.8) | Route the phone's microphone into Windows apps (Discord, Zoom, OBS). | Stream low-latency 48 kHz mono/stereo PCM from the phone's `AudioRecord` (with `NoiseSuppressor`) to the PC (`audio.pcm` bidirectional stream) and play into a virtual audio endpoint or default communications loopback. |

#### Section 3: Parked "Magic" Experiences (Revivable Extensions)

| # | Feature | Value Proposition | Architecture & Implementation Spec |
|---|---|---|---|
| **3.1** | **Root Power Level (`Magisk` / `KernelSU` / `APatch`)** (§3.13) | Zero-touch Elevated mode that survives phone reboots without Wireless Debugging re-arming. | In `Elevated.kt`, check `su -c id` before falling back to Wireless Debugging (`adbd`); if root is granted, launch `CLASSPATH=$apk app_process /system/bin app.nectarlink.android.elevated.InputServer --attach` directly via `su` on boot (`BOOT_COMPLETED`). |
| **3.2** | **Instant Hotspot** (§3.13) | One click on PC turns on the phone's Wi-Fi hotspot and connects the PC. | Trigger `cmd wifi start-softap` / `TetheringManager` via the Elevated helper (`InputServer.kt`), read the soft-AP SSID/passphrase, and connect Windows via `WlanConnect` / `netsh wlan`. |
| **3.3** | **Smart Notification Routing** (§3.13) | Silence duplicate phone buzzes while actively using the PC keyboard/mouse. | Track `GetLastInputInfo` on Windows; while active (`idle < 30 s`), signal the phone (`presence.pc_active`) so `NotificationListener` suppresses redundant phone vibration/sound for mirrored notifications. |
| **3.4** | **Desk Mode** (§3.13) | Ambient clock, media, and live status dock screen on the phone when charging at the desk. | Full-screen Compose ambient dock activity (`FLAG_KEEP_SCREEN_ON`, OLED burn-in pixel shift) showing time, PC media controls, and live PC task progress while charging and connected to the PC. |

#### Section 4: Daily-Use UX & Polish Opportunities

| # | Feature | Scope | Architecture & Implementation Spec |
|---|---|---|---|
| **4.1** | **Notification & Message Richness** | Desktop (`HomePage.qml`, `MessagesPage.qml`, `NotificationItem.qml`) | Collapsible per-app notification stacks with smooth height animations, inline image lightbox preview on click, and voice-note audio attachment playback/sending in `MessagesPage.qml`. |
| **4.2** | **Android Material 3 Expressive Polish** | Android (`MainActivity.kt`, `HomeScreen.kt`, `PcWidgetProvider.kt`) | Shared-axis / spring transitions between tabs, predictive-back gesture animations on sub-screens, and interactive Glance home-screen widgets with live PC battery and one-tap actions. |

---

## 9. Risks & mitigations
| Risk | Mitigation |
|---|---|
| OEM battery killers break reliability | CDM association, FGS, BLE wake, per-OEM Doctor guides, opt-in local diagnostics export for testers |
| **Qt + Rust build or bridge proves painful** (cxx-qt pre-1.0, C++ glue, CI) | Phase 0 proof with explicit pass criteria. Single bridge crate, pinned versions. Fallback: web UI (Tauri) + native Rust video windows, with an unchanged core |
| Qt licensing / LTS changes | Only LGPL modules, track latest Qt 6 minor releases, KDE Free Qt Foundation guarantee |
| Developer verification limits sideloading | Verify identity before global enforcement. One signing key across channels |
| HFP API locked down by Microsoft | Early spike. Fallback: call control with audio on phone (still useful) |
| Credential provider ("Unlock with phone") is security-critical | Post-1.0, isolated module, threat model, external review, off by default, easy revoke |
| iroh relay cost/availability for away mode | Opt-in, self-host option, documented. Donation-funded relays later |
| Scope explosion | Strict phase exits. P2 and rejected features wait. Every feature needs a design + reliability check |
| Elevated mode seen as scary/abused | Transparent UI, open-source code, strict caller verification, revoke anytime |
| Upstream scrcpy changes | Pin a vendored version. Contribute fixes upstream |
| High-visibility launch (large X audience) exposes rough edges | Private tester waves first, public alpha only after Phase 1 exit criteria |

---

## 10. Decisions & open questions

### 10.1 Name: **NectarLink** (done)
- Repo: `github.com/thtbee/nectarlink` (private until most things work locally; no pushing before that).
- Checks done: no conflicting products found on the web; `nectarlink.app`, `nectarlink.dev`, `getnectarlink.com` and the GitHub name were free (2026-10-05).
- **Still to do before any announcement:** official trademark searches (USPTO, TMview, IP India), register the domain, X handle, and reserve `nectar-link` as a redirect.
- History: "Hive" (folder codename) and "Waggle" were dropped. Hive collides with Centrica's smart-home brand; WAGGLE is a **registered, active US trademark** (Nimble Wireless, class 9, reg. 9 May 2023) that covers "software for transmission of voice, data, images, audio, and video between devices". Dozens of other candidates were checked and rejected (Honeylink, Tandem, Skein, Wingbeat, Murmur and others).

### 10.2 License (done) | Part | License | Why |
|---|---|---|
| Desktop + Android apps | **GPL-3.0-or-later**, plus an *additional permission* (GPLv3 §7) allowing distribution via app stores with conflicting terms | Anyone who ships a modified app must publish the source, which stops closed, ad-stuffed rebrands. It's compatible with code from Sefirah (GPL-3.0) and KDE Connect (GPL-2.0-or-later) if we ever borrow, and with scrcpy (Apache-2.0) and Qt (LGPL-3.0, dynamically linked; GPL-only Qt modules avoided, see §5.1). The app-store permission keeps a future App Store (iPhone/Mac) release legally clean, and it only works if it's there from the very first commit |
| Core crates (`nectarlink-core`, protocol, adb, localsend) | **MPL-2.0** | File-level copyleft: improvements to *our files* must stay open, but anyone can link the core. That grows the ecosystem without letting someone fork it closed |
| Protocol spec + docs | **CC BY 4.0** | Anyone can implement the protocol |
| Name + logo | **Trademark policy** (like Firefox/Signal) | Forks are welcome but must rebrand |
| Contributions | **DCO** sign-off (no CLA) | Contributor-friendly, with a clear provenance trail |

### 10.3 Design direction (done)
**Bloom** (default; refined Material You, minimal and soothing, light/dark, wallpaper-seeded color) plus a built-in **Graphite** theme (paper/slate, ink, hairlines, mono annotations; from github.com/thtbee/graphite). Directions A and B rejected. See `docs/design/README.md` and `docs/design/themes/index.html`.

### 10.4 Windows (done)
**Windows 11 only**, x64 + ARM64.

### 10.5 Crash reporting (done) (opt-in)
- Nothing is sent unless you say yes. After a crash, NectarLink asks once and shows the exact redacted report (no notification text, message content, file names or keys).
- **v1 uses no server at all:** "Report on GitHub" opens a prefilled issue, and you attach the redacted bundle yourself.
- If volume grows: an opt-in, self-hosted GlitchTip (Sentry-compatible) server.
- Tooling: Rust minidumps (`crash-handler`) on desktop, ACRA with a consent dialog on Android.

### 10.6 Budget (done) (GitHub-only, no Play account, no test phones)
| Item | Cost | Notes |
|---|---|---|
| Domain (site + docs on GitHub Pages) | ~$10 to 20 / yr | Hosting is free |
| CI | $0 | GitHub Actions is free for public repos |
| Windows code signing | **$0** (SignPath Foundation, free for eligible OSS) · or ~€30 to 100 / yr (Certum open-source cert) · or ~$10 / mo (Azure Artifact Signing, where individuals are eligible) | Unsigned is OK for early alphas, but SmartScreen will warn. Reputation builds over time |
| Android signing | $0 | Our own keystore. Back it up in two places |
| Android developer verification | Likely a small one-time fee (unconfirmed) | Needed before global enforcement (2027) for GitHub APKs to install normally |
| Relays for Away mode | $0 in alpha (public iroh relays) → ~$10 to 20 / mo for 2 small VPS regions | ~95 % of traffic goes direct, so relay bandwidth stays small. Self-hosting is documented |
| Test devices | $0 | Covered by the maintainer's tester community |
| Later: Apple Developer (Mac notarization) | $99 / yr | Only when the Mac build ships |
| **Total, year 1** | **Bare minimum ≈ $10 to 50 · Comfortable ≈ $250 to 450** | Funded via GitHub Sponsors / Open Collective once public |

### 10.7 Future scope (recorded)
- **macOS desktop:** the maintainer has a MacBook. A port, not a rewrite: the Rust core, the cxx-qt bridge and ~90 % of the QML screens are shared (Qt Quick renders with Metal on Mac). Mac-specific work sits behind `Platform`/`VideoSink`:

  | Feature | Windows | Mac |
  |---|---|---|
  | Mirrored video in the UI | Media Foundation + D3D11 | VideoToolbox + Metal (small Mac-only C++/Obj-C++ piece) |
  | Notifications | Windows toasts | UserNotifications (needs a signed app) |
  | Window glass | Mica | macOS vibrancy |
  | Tray | System tray | Menu-bar icon |
  | Nectar Island | Top-of-screen pill | Around the MacBook notch |
  | Phone storage in file manager | Explorer (Cloud Files) | Finder (File Provider extension) |
  | "Send to phone" right-click | Explorer context menu | Finder Share / Sync extension |
  | Phone as webcam | Windows 11 virtual camera | Camera Extension (approved once by the user) |
  | Unlock computer with phone | Credential provider | **Not portable** (no public equivalent) |
  | Call audio over Bluetooth | HFP (if spike passes) | **Unclear / likely not** (deprecated APIs); calls = alerts + controls |

  Needs the Apple Developer account ($99/yr) for signing and notarization, and universal builds (Apple Silicon + Intel) on GitHub's macOS runners.
- **iPhone:** later. Expect KDE Connect-style iOS limits (no notification mirroring, no SMS access).

---

## 11. Pre-flight checklist

### Must resolve before writing code
1. **Name clearance.** Trademark search for "NectarLink" in software classes (9 and 42) in the US, EU and India, then secure the domain, GitHub org and X handle. Do this *before* teasing the name to the X audience, because renaming after a public announcement is costly.
2. **Signing-key custody.** The Android signing key is NectarLink's permanent identity: lose it and every user has to uninstall to update, and developer verification is tied to it. Generate it once, keep two offline backups, and store it as a CI secret. The same applies to the auto-update signing key.
3. **Dev environment (done) (done 2026-10-04)**:
   - Installed: Qt 6.12.0 (MSVC x64 + ARM64) in `C:\Qt`, Android NDK 30.0.16248370 + CMake 4.1.2, Android command-line tools, Rust targets (Android ×4, Windows ARM64), `cargo-ndk` 4.1.2.
   - Already present: Android platform 37, build-tools 36, platform-tools, Rust 1.99, MSVC Build Tools 2022, Node 24, Python 3.14.
   - Environment variables set: `JAVA_HOME` (Android Studio's JDK 25), `ANDROID_HOME`, `ANDROID_NDK_HOME`, `QMAKE`, plus PATH entries for the JDK, Qt, platform-tools and the Android CLI.
   - (done) MSVC ARM64 compiler (installed by the maintainer).
4. **Repo files from the first commit.** License texts (including the app-store exception), DCO, SECURITY.md with private vulnerability reporting, CODE_OF_CONDUCT, CONTRIBUTING, trademark policy, issue templates.

### Decide during Phase 0
5. **Installer scope.** Recommended: **per-machine** (one UAC prompt at install), which lets the installer add the firewall rule and so avoids the #1 complaint in this category. Per-user installs need no admin, but Windows then shows an "allow network access" prompt that users often cancel.
6. **Antivirus false positives.** Input injection, a built-in ADB client, listening sockets and (later) a credential provider look suspicious to antivirus heuristics, especially in unsigned builds. Sign early (SignPath), submit builds to Microsoft's false-positive portal, and never use executable packers.
7. **Version skew.** Testers will run mismatched phone and PC versions. CI tests the current release against the previous two in both directions.
8. **Android target-SDK rules.** Foreground-service type requirements for `connectedDevice`, background-start limits, edge-to-edge enforcement, and Android 14+'s read-only rule for dynamically loaded code (Elevated mode loads its code straight from the installed APK, which satisfies it).
9. **Testing strategy.**
   - Protocol conformance and fuzzing in CI.
   - Network chaos tests (packet loss, Wi-Fi switching, sleep/wake).
   - Qt Quick Test and Compose UI tests.
   - Battery soak tests on testers' phones.
   - An opt-in diagnostics bundle, since there's no telemetry.
10. **Island etiquette.** Auto-hide during fullscreen games, videos and presentations and when Windows is in Do Not Disturb. Handle multiple monitors.
11. **Data retention defaults (done):** Timeline (90 days default, configurable 30 / 90 / 365 days or Forever, and 5,000 entries cap, auto-purged + Clear (done)), clipboard history (50 entries, 7-day auto-purge for unpinned items while pinned stay, encrypted at rest with `KeyProtector`, OTPs excluded + Clear (done)), notification history (24 hours, 300 dismissed items cap + Clear (done)), `MessagingStyle` message cache (90 days, 200 threads/peer, 200 messages/thread + Clear (done)), photo thumbnail cache (30 days, 500 thumbnails cap + Clear (done)), received file transfer records (Clear transfer history without deleting files on disk (done)), and one **"Clear everything"** button (`clear_all_local_data`) in *Settings → Data & storage* on both PC and Android that wipes all local history and caches while preserving device identity, paired devices, and per-device feature toggles (done).
12. **Accessibility and translations from day one.** All strings translatable (`qsTr`, Android resources), RTL layouts, screen-reader labels, keyboard focus order. Retrofitting is expensive.
13. **Asset licenses.** The display font under OFL, an icon set under a permissive license (e.g. Material Symbols, Apache-2.0). No Microsoft or Google logos; competitor names only in plain comparison wording.
14. **Realistic pacing.** Claude writes most of the code, but every phase needs maintainer review and tester feedback. Estimate phase durations after Phase 0 from real velocity, not before.

### Keep in mind
- Publish a short privacy page even though nothing is collected. Testers and press will ask.
- Gate public announcements behind phase exits, and run a tester sign-up from the X audience.

---

## 12. Next steps

| # | Step | Owner | Output |
|---|---|---|---|
| 1 | Name clearance + grab domain, GitHub org, X handle | Maintainer | "NectarLink" cleared by official searches |
| 2 | Phase 0 detailed spec: protocol v0 (Hello, pairing, capabilities), `nectarlink-core` public API, capability-matrix schema, repo scaffolding plan, ADRs, spike test plans with pass criteria | Claude | `docs/` specs |
| 3 | Design foundations + 2 to 3 visual directions (both apps) | Claude → maintainer picks | Chosen direction, then hi-fi mockups |
| 4 | Dev environment setup (Qt 6.12, JDK, Android SDK/NDK, Rust targets) | Together | One-command build prerequisites |
| 5 | Create the repo (private until Phase 0 exit), license files, DCO, policies | Together | Repo skeleton |
| 6 | Generate signing keys with offline backups | Maintainer (guided) | Keys stored safely |
| 7 | Start Phase 0 coding: **Qt + Rust proof first** (it's the gate), core + pairing and the other spikes in parallel | Claude | Phase 0 exit criteria met |
| 8 | Tester sign-up (devices, Android versions, OEMs) | Maintainer | Tester list for Phase 1 |

Later: SignPath application (needs a public repo with history), Android developer verification (before 2027 global enforcement), relay servers (Phase 3), privacy page, phase duration estimates after Phase 0.

---

## Sources
- Sefirah: [Windows repo](https://github.com/shrimqy/Sefirah), [Android repo](https://github.com/shrimqy/Sefirah-Android), [releases](https://github.com/shrimqy/Sefirah/releases), [issues](https://github.com/shrimqy/Sefirah/issues), [Sefirah-old fork](https://github.com/AlexbeatsZ/Sefirah-old), [Android Police review](https://www.androidpolice.com/forget-phone-link-this-ad-free-open-source-app-beats-it-in-every-category/)
- Phone Link: [disconnect reports](https://learn.microsoft.com/en-us/answers/questions/5516264/problem-with-phone-link-constantly-disconnecting), [sensitive notifications complaint](https://learn.microsoft.com/en-us/answers/questions/5758796/microsoft-just-broke-phone-link-in-the-name-of-saf), [Android 15 change](https://www.windowscentral.com/software-apps/windows-11/microsoft-warns-that-android-15-will-make-windows-phone-link-worse), [expanded screen OEM list](https://www.windowslatest.com/2026/01/07/windows-11s-almost-full-screen-android-apps-mirroring-now-available-for-everyone-via-phone-link-app-with-supported-phones/), [recent features](https://www.makeuseof.com/windows-11-phone-link-is-better-than-ever/)
- KDE Connect: [Windows discovery/firewall guide](https://windowsforum.com/news/kde-connect-on-windows-pair-android-or-iphone-enable-remote-control-and-fix-discovery.446592/), [feature requests](https://discuss.kde.org/t/features-request-for-kde-connect/40288), [XDA alternatives + comments](https://www.xda-developers.com/forget-phone-link-use-these-5-apps-instead/)
- LocalSend: [protocol](https://github.com/localsend/protocol), [VPN discovery bug](https://github.com/localsend/localsend/issues/1598), [Windows discovery issues](https://windowsforum.com/news/fix-localsend-not-detecting-devices-on-windows-troubleshooting-guide.354562/)
- Intel Unison: [gHacks](https://www.ghacks.net/2025/07/04/intel-unison-is-dead-here-are-some-alternatives-you-can-switch-to/), [Wikipedia](https://en.wikipedia.org/wiki/Intel_Unison)
- scrcpy: [4.0 release notes](https://ubuntuhandbook.org/index.php/2026/05/scrcpy-4-0-resizable-virtual-display/), [3.3 UHID on virtual display](https://ubuntuhandbook.org/index.php/2025/06/scrcpy-3-3-added-uhid-mouse-to-android-virtual-display/)
- iroh: [1.0 coverage](https://www.techtimes.com/articles/318490/20260616/peer-peer-library-iroh-10-ships-dial-devices-key-not-ip-address.htm), [roadmap](https://www.iroh.computer/roadmap), [0.96 QUIC multipath](https://www.iroh.computer/blog/iroh-0-96-0-the-quic-multipaths-to-1-0)
- UI frameworks: [Qt licensing (GPL-only modules)](https://doc.qt.io/qt-6/licensing.html), [Qt supported platforms](https://doc.qt.io/qt-6/supported-platforms.html), [Qt for Windows on ARM](https://www.qt.io/blog/qt-for-windows-on-arm), [Commercial-only LTS patches](https://www.qt.io/blog/commercial-lts-qt-6.8.9-released), [CXX-Qt changelog](https://github.com/KDAB/cxx-qt/blob/main/CHANGELOG.md), [Caelestia shell (QML)](https://github.com/caelestia-dots/shell), [Slint 1.18](https://slint.dev/blog/slint-1.18-released), [Slint 1.16](https://slint.dev/blog/slint-1.16-released), [Slint changelog](https://github.com/slint-ui/slint/blob/master/CHANGELOG.md), [Slint backdrop-filter request](https://github.com/slint-ui/slint/issues/13502), [Flutter desktop → Canonical](https://www.omgubuntu.co.uk/2026/05/flutter-desktop-canonical-maintained), [Flutter multi-window status](https://startdebugging.net/2026/08/how-to-enable-multi-window-support-in-a-flutter-desktop-app/), [GPUI slowdown discussion](https://news.ycombinator.com/item?id=47003569)
- Android platform: [sensitive notifications / ADB appops](https://www.androidauthority.com/android-15-two-factor-authentication-codes-3492585/), [Android 14 MediaProjection consent](https://developer.android.com/about/versions/14/behavior-changes-14), [background clipboard block](https://www.xda-developers.com/android-q-blocks-background-clipboard-access/), [developer verification timeline](https://www.androidauthority.com/android-sideloading-changes-timeline-3679204/), [F-Droid 2.0](https://pinggy.io/blog/f_droid_2_0_android_developer_verification/)
- Windows platform: [PhoneLineTransportDevice](https://learn.microsoft.com/en-us/uwp/api/windows.applicationmodel.calls.phonelinetransportdevice), [22H2 breakage](https://github.com/BestOwl/MyPhone/issues/26), [virtual camera sample](https://github.com/smourier/VCamSample), [BestCam](https://github.com/OneLimeStudio/BestCam)
- Name check: [getwaggle.app (desktop app)](https://www.getwaggle.app/download), [Waggle pet app](https://play.google.com/store/apps/details?id=com.nimble.petsafety&hl=en_US), [nectarlink-sensor](https://github.com/nectarlink-sensor)
- Quick Share: [Windows update](https://www.androidauthority.com/quick-shares-windows-app-now-looks-a-little-more-like-an-android-app-3661953/)
