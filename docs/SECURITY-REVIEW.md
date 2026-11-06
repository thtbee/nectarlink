# NectarLink Security Review

**First reviewed:** 2026-10-10. **Checked against the code and corrected:** 2026-11-05, for 0.1.5.
**Scope:** every network, local IPC, elevated privilege, shared-memory and filesystem surface in `nectarlink-protocol`, `nectarlink-core`, `nectarlink-adb`, `nectarlink-desktop`, `nectarlink-vcam` and the Android app.

This is a review by the project, not an independent audit. It says what the code does and names what isn't protected. For the design, see [`docs/technical/security-model.md`](technical/security-model.md). To report a problem, see [`SECURITY.md`](../SECURITY.md).

---

## Summary

1. **Mutually authenticated sessions.** Each install has a persistent Ed25519 identity. Sessions (`ALPN = "nectarlink/0"`) run over QUIC with TLS 1.3 through `iroh`, which proves each side holds the key for its `DeviceId`. A connection whose `DeviceId` isn't in the local `peers` table is closed with `CLOSE_UNPAIRED` before anything else runs.
2. **Pairing needs a secret or a person.** Pairing (`ALPN = "nectarlink-pair/0"`) needs either a one-time 128-bit secret from a QR code or link, or a 6-digit code confirmed on both devices.
3. **Bounded input.** Frames are capped before allocation (`MAX_FRAME_LEN`, 1 MiB). Messages are validated, and every feature is gated by a per-device toggle.

### What isn't protected

- **After turning off Connect over mobile data, one idle relay connection stays until the next restart.** Nothing is published, looked up or relayed by then, and non-LAN sessions are closed. iroh 1.3 only lets go of its home relay when the endpoint closes.
- **On Linux the device key and clipboard history aren't sealed.** `identity.key` is stored with `0600` permissions and no platform protection, and the core logs that it is. Anything running as your user can read it, and so can anyone who reads the disk without full-disk encryption. A Secret Service option is under review for after 0.1.5 ([#189](https://github.com/thtbee/nectarlink/pull/189)).
- **Windows DPAPI and the macOS Keychain protect against other accounts and offline disks, not against programs running as you.** The same goes for anything else a program running as you can read.
- **Release binaries aren't code-signed (Authenticode).** Trust in downloads rests on HTTPS to GitHub and the published checksums. Android APKs are signed, and updates must carry the same signature.
- **LocalSend is an open LAN protocol.** Any device on the network can offer a file. It's off by default, and nothing is written until you accept.
- **Elevated gives NectarLink shell-level control of the phone.** That's what it's for. Its limits are in Surface 5.

---

## Surface 1: QUIC sessions, pairing and USB transport

**Code:** `core/nectarlink-core/src/{pairing,session,usb,node,store}.rs`, `core/nectarlink-adb/src/lib.rs`, `core/nectarlink-protocol/src/{pairing,frame}.rs`

**Threats:** a man in the middle on a shared network, silent pairing over USB, oversized or malformed frames, a paired device using a feature you turned off.

**Controls:**

1. **Short authentication string.** Both sides commit to 128-bit nonces first (`pair.commit` with `SHA-256(nA)`, then `pair.nonce`, then `pair.reveal`). The code is `uint32(HMAC-SHA256(nA || nB, "nectarlink-sas-v0" || idA || idB)[0..4]) mod 1,000,000` (`sas_code`). A man in the middle has a different key from the real device, so it changes `idA` or `idB`, and the commitment stops it choosing a nonce after seeing the other side's. It has a one-in-a-million chance of matching codes.
2. **One-time secret.** The QR secret is 128 random bits. It expires after 5 minutes (`PAIRING_TTL`) and is dropped after 5 failed attempts (`MAX_FAILED_ATTEMPTS`). The proof is `HMAC-SHA256(S, "nectarlink-pair-v0" || joining_id || host_id)`, checked in constant time.
3. **A link from outside the app never pairs by itself.** The phone asks first and names the PC.
4. **Consent on USB.** An unpaired phone on a USB cable starts the same 6-digit ceremony, confirmed on both devices. Android emulators don't start it on their own (`auto_pairs`). The tunnel only carries QUIC packets, so encryption and authentication don't depend on the cable. A tunnel that stops answering is closed and reopened (`TUNNEL_SILENCE_LIMIT`).
5. **ADB serials.** `is_serial_allowed` rejects malformed serials in everything that talks to `adb`, and `NECTARLINK_ADB_ONLY_SERIAL` limits a build to one device for testing.
6. **Frame cap.** `read_frame` rejects a length over 1 MiB before allocating.
7. **Per-device toggles.** See Surface 8 and the security model.
8. **Sealed data at rest.** `identity.key`, the clipboard vault (`clipboard_history.enc`, `clipboard_images/*.enc`) and queued clips, images, links and task alerts in `send_queue` (`QueueSeal`, including their titles) go through the platform `KeyProtector`: DPAPI on Windows, the login Keychain (AES-256-GCM) on macOS, Android Keystore AES-256-GCM on Android, and none on Linux (see above).

**Reported and fixed:** a private report pointed out that queued clips were plain text in SQLite and survived clearing, and that turning off Connect over mobile data left internet services running until restart. Both were real. Queued content is now sealed, older rows are sealed at startup, `secure_delete` is on, clearing removes it, and it's removed at its deadline. The switch now applies while running, with the one limit above.

---

## Surface 2: LocalSend server

**Code:** `core/nectarlink-core/src/localsend.rs`, `core/nectarlink-core/src/transfer.rs`

**Threats:** any host on the LAN can reach TCP 53317 when LocalSend is on. It can send huge headers or bodies, hostile file names, or try to upload into someone else's session.

**Controls:**

1. **Off by default.**
2. **Size caps.** `MAX_HEADER_BYTES` is 64 KiB, `MAX_JSON_BODY_BYTES` is 4 MiB, and an offer holds at most `files::MAX_FILES` (5,000) files.
3. **Consent first.** An offer waits for you to accept, for up to `APPROVAL_TIMEOUT` (60 seconds). No file bytes are written before that.
4. **Random session and per-file tokens** (`random_token`), checked before an upload is read.
5. **File names.** `safe_file_name` keeps only the last path component and replaces control characters. `storable_name` then removes what Windows would treat specially: `<>:"|?*` (a `:` would otherwise be a drive prefix or an alternate data stream), trailing dots and spaces, and reserved device names such as `CON` and `COM1`. Folder paths go through the same steps, segment by segment. Both apply to transfers from paired devices too.
6. **Staged writes.** Bytes go to a `.part` file under `<data_dir>/incoming/localsend/<session_id>/` and move to Downloads when complete. `unique_path` never overwrites an existing file. On Windows, received files are marked as coming from elsewhere (`mark_from_elsewhere`), so SmartScreen and Protected View apply.

---

## Surface 3: Local automation API

**Code:** `desktop/app/src/local_api.rs`, `docs/api.md`

**Threats:** a web page in a local browser calling `127.0.0.1`, or a local process without access to your profile.

**Controls:**

1. **Off by default** (`local_api_enabled: false`).
2. **Loopback only.** Bound to `127.0.0.1`, default port 53320.
3. **Bearer token.** Every request needs `Authorization: Bearer <token>`, a 256-bit random secret in `<data_dir>/api-token.json`, compared in constant time. A missing or wrong token gets `401`. The token is only accepted in that header. If no secure random source answers, the API stays off.
4. **Browsers are refused.** A request with an `Origin` header gets `403`, whatever its token, and no `Access-Control-Allow-Origin` header is ever sent.
5. **Bounded requests.** `MAX_REQUEST_BYTES` is 256 KiB, with a 5-second I/O timeout (`IO_TIMEOUT`).

Anything running as your user can read `api-token.json`, so this API doesn't separate programs running as you.

---

## Surface 4: Single instance and launch requests

**Code:** `desktop/app/src/launch.rs`, `desktop/app/src/send_to.rs`, `desktop/app/src/win/single_instance.rs`, `desktop/app/src/linux/single_instance.rs`

**Threats:** another user signalling the app or writing requests, and malformed request files.

**Controls:**

1. **Windows.** The mutex and wake event are `Local\Nectarlink.Desktop.<hash>` and `Local\Nectarlink.Desktop.Activate.<hash>`: per logon session and per data folder.
2. **Linux.** `flock` on `<data_dir>/instance.lock`, and a Unix socket at `<data_dir>/instance.sock`, inside your private data folder.
3. **Requests are files in the data folder,** which only you can write to. A malformed one is removed and ignored.

---

## Surface 5: Elevated helper

**Code:** `android/app/src/main/kotlin/app/nectarlink/android/elevated/{InputServer,Elevated,ShizukuShell}.kt`, `core/nectarlink-core/src/elevated_usb.rs`, `docs/protocol/elevated-usb.md`

**What it is:** `InputServer`, from NectarLink's own APK, run through `app_process` as Android's `shell` user (UID 2000), started over Wireless Debugging, USB `adb` or Shizuku.

**Threats:** another app on the phone connecting to the helper, or commands doing more than they should.

**Controls:**

1. **Loopback and a token.** Android's SELinux blocks regular apps from `shell`'s Unix sockets, so the helper listens on `127.0.0.1:<ephemeral port>`. Every connection must send a 24-byte (48 hex characters) `SecureRandom` token as its first line, compared with `MessageDigest.isEqual`, or it's closed before any command is read. The helper ends when the shell that started it goes.
2. **Fixed commands.** Lines are parsed as fixed verbs dispatched to Android APIs: finger and wheel events, keys, text, clipboard, gamepad, display power, app windows and device audio.
3. **A short list of shell commands.** The `C` verb runs only what's in `allowed`: expanding the notification shade, Wi-Fi, Bluetooth, hotspot, brightness `0..255`, and the grants NectarLink needs for itself, each named exactly. Anything else is rejected.
4. **Shizuku.** `ShizukuShell` answers only NectarLink's UID and Shizuku's, and runs only its checked grants and `InputServer`. It isn't used when Shizuku runs as root.
5. **Sensitive clips stay.** A clip marked `EXTRA_IS_SENSITIVE`, as password managers do, isn't read or sent.

**Limit:** the token is what separates other apps from shell-level input. It isn't written where other apps can read it, but anyone who already has `shell` or root on the phone can do these things anyway.

---

## Surface 6: Virtual camera shared memory

**Code:** `desktop/vcam/src/{shm,source,gpu}.rs`, `desktop/app/src/webcam.rs`, `desktop/app/src/linux/vcam.rs`

**Threats:** the camera's media source runs inside Windows' Frame Server, so bad frame headers could hurt that process, and torn reads could show half a frame.

**Controls:**

1. **Who can open it.** The mapping is `Global\NectarlinkVcamFrame_v2`, created by the service, because the Frame Server runs in another session. Its security descriptor (`GLOBAL_SDDL`) gives the system, LocalService and administrators full access, and signed-in interactive users read and write. A signed-in user on the same PC can therefore write frames to the camera. `Local\` names are used only when the media source runs in-process.
2. **Bounds.** Frames are limited to `MAX_WIDTH` × `MAX_HEIGHT` (3840 × 2160) and `MAX_FRAME_BYTES`. Control messages are at most 256 bytes.
3. **No torn reads.** Readers check a sequence counter around each copy and discard the read if it changed.

**Linux:** the camera is a `v4l2loopback` device. Any local program allowed to open that device can read or write it, as with any V4L2 camera.

---

## Surface 7: Phone storage and PC folders

**Code:** `core/nectarlink-core/src/storage.rs`, `core/nectarlink-protocol/src/messages.rs`, `android/.../storage/PhoneStorage.kt`, `desktop/app/src/storage.rs`, `desktop/app/src/linux/storage.rs`, `desktop/app/src/pc_storage.rs`

**Threats:** paths escaping the shared root, or files reaching the PC under names that misbehave.

**Controls:**

1. **Path validation.** `is_valid_storage_path` accepts relative paths only: at most 1,024 bytes and 32 segments, each at most 255 bytes, with no leading `/`, `.` or `..` segments, `\`, or control characters.
2. **Canonical paths on the phone.** `PhoneStorage` resolves paths canonically, so symlinks can't leave the storage root. It never exposes `/data`, `Android/data`, `Android/obb` or NectarLink's own folders.
3. **Names on the PC.** A phone entry whose name Windows couldn't store safely isn't surfaced as it is (`storable_name` in `storage.rs`).
4. **Linux mount.** The FUSE mount at `~/Nectarlink/<Phone name>` uses `nosuid,nodev,default_permissions` and only your user can reach it.
5. **Toggles.** Storage needs the `storage` toggle (off by default). Over a relay it also needs `remote_files` (off by default).
6. **Photos.** A paired PC with the `photos` toggle on can move photos to the phone's trash (`photos.delete`), without a prompt on the phone, when the phone has All files access. It can't delete anything for good.
7. **PC folders on the phone.** `pc_storage.rs` exposes only `Downloads`, `Documents`, `Desktop` and `Pictures`, only when **share_pc_storage** is turned on, and rejects `.` and `..` segments.

---

## Surface 8: PC capabilities a phone can reach

**Code:** `core/nectarlink-core/src/{features,mirror,remote,storage}.rs`, `desktop/app/src/{core_host,pc_mirror,pc_audio_stream,pc_notifications,pc_storage,input_share}.rs`, `desktop/app/qml/MainWindow.qml`

**Threats:** a paired phone, possibly in someone else's hands, silently watching or controlling the PC.

**Controls:**

1. **Sharing the PC's screen.** `pc_mirror` is off by default and separate from `mirroring`. When it's off, a request is denied and the PC asks (`NodeEvent::PcMirrorRequested`): **Not now**, **Allow** (this session only) or **Allow for this device**. While sharing, the PC shows a banner with **Stop sharing**.
2. **Controlling the PC.** Input into a PC needs `remote_input`, off by default, on the PC itself: `pc_mirror.rs` and `mirror.rs` check it before injecting, and `remote.rs` asks the first time. The check sits on the PC because the PC is what's being controlled. A phone doesn't apply it to itself.
3. **What the PC advertises.** `compute_capabilities` advertises `mirror.capture` only when `pc_mirror` is on for a device or sharing is active, and `mirror.input` and `input.inject` only when `remote_input` is on for a device.
4. **No silent toggles.** Starting a session doesn't change a device's toggles.
5. **Off by default and set only on the PC:** streaming PC audio (`stream_pc_audio_to_phone`), forwarding PC notifications (`forward_pc_notifications_to_phone`) and sharing PC folders (`share_pc_storage`).
6. **Phone's camera and microphone.** Starting them from a PC needs a tap on the phone, except for a PC you chose under **Start the camera and microphone without asking** (Elevated, off by default).
7. **Moving control between devices.** `input_share.rs` tracks which device the PC is controlling. Moving the cursor across a screen edge onto the phone is not built.

---

## Surface 9: Updates

**Code:** `desktop/app/src/{updater,feed,update_relay}.rs`, `desktop/installer/nectarlink.nsi`, `android/.../update/AppUpdater.kt`. Details are in the [security model](technical/security-model.md#automatic-updates-and-release-verification).

**Threats:** a tampered download, a local user abusing the SYSTEM update task, or a paired device pushing a build.

**Controls:**

1. **HTTPS, size caps and SHA-256.** A feed can only point at this repository's own release files, each with a hash that the download must match.
2. **Android** installs only over the same signature.
3. **The Windows task takes no input.** It fetches and checks everything itself, and copies the installer into a folder only SYSTEM and administrators can write before re-checking it. The worst a user can do by starting it is install the newest official build early.
4. **Hints between devices never name a file or hash.** They only start a check of the receiver's own feed.
5. **Linux** updates only with the same kind of package it was installed from.

**Limit:** see "What isn't protected": Windows binaries aren't signed.
