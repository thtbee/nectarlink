# Security Model

NectarLink uses an end-to-end encrypted, mutually authenticated peer-to-peer security model with no central user accounts and no cloud storage. Every connection is authenticated at the transport layer by static Ed25519 device identities and authorized at the application layer by per-device capability toggles and interactive runtime consent gates.

For the full audit record and vulnerability remediation history, see [`docs/SECURITY-REVIEW.md`](../SECURITY-REVIEW.md).

## Threat Model

| Adversary / Environment | Assumed Capabilities | Mitigations |
|---|---|---|
| Untrusted LAN / Public Wi-Fi | Can eavesdrop, replay, inject, or MITM UDP/TCP packets; can broadcast spoofed mDNS records | QUIC + TLS 1.3 (`iroh`) with mutual Ed25519 certificate verification; pairing requires either an out-of-band 128-bit QR secret (`HMAC-SHA256`) or a commitment-bound 6-digit SAS confirmed on both screens |
| Compromised Relay Server (**Connect over mobile data** / `away_mode`) | Can observe outer UDP packet sizes/timing and drop traffic | End-to-end TLS 1.3 termination between peer `DeviceId` keys; relay never possesses session keys; high-risk capabilities (`storage` without `remote_files`) are blocked on `ConnectionPath::Relay` |
| Malicious Web Page on Local PC | Can send cross-origin `fetch` / `XMLHttpRequest` / `WebSocket` requests to `127.0.0.1:53320` | Local HTTP API is disabled by default; requires a 256-bit Bearer token in `Authorization` header (constant-time verified) and rejects any request carrying an `Origin` header or `Sec-Fetch-Mode: cors` / `no-cors` |
| Unprivileged Local App on Android | Can attempt to connect to local ports or read app storage | Elevated helper (`InputServer`) binds `127.0.0.1:<ephemeral_port>` and requires a 192-bit (24-byte / 48-hex-char) per-session `SecureRandom` token on the first line, verified in constant time (`MessageDigest.isEqual`) before accepting any commands |
| Offline Disk Inspection | Read access to `%LOCALAPPDATA%\Nectarlink` or Android app data directory | Private identity key (`identity.key`) and clipboard vault (`clipboard_history.enc`, `clipboard_images/*.enc`) are sealed using Windows DPAPI (`CryptProtectData`, tied to the Windows user logon credential) or Android Keystore AES-256-GCM (hardware-backed when supported by the device) |
| Paired Device Exceeding Intended Access | Attempts to invoke disabled capabilities, traverse outside shared paths, or start silent screen/input capture | Inbound dispatch enforces `CapabilityMatrix::is_allowed` per peer, canonicalizes file paths, and gates sensitive sessions (`pc_mirror`, `remote_input`, first-time `storage`, `webcam`) behind explicit user confirmation |

## Cryptographic Device Identity

Implemented in `core/nectarlink-core/src/identity.rs`.

- Each installation generates a 32-byte Ed25519 secret key (`iroh::SecretKey`) using the OS CSPRNG (`rand::rng()`).
- The corresponding 32-byte Ed25519 public key (`iroh::EndpointId`) serves as the canonical `DeviceId` (rendered in logs and URIs as z-base-32, with a 16-character `short()` prefix for UI display).
- On disk, `identity.key` is stored with a 5-byte header:
  - Bytes `0..4`: Signature `NLK1` (`b"NLK1"`).
  - Byte `4`: `KeyProtector` ID (`0` = `PlainKeyProtector` for headless CLI/tests, `1` = OS key protector).
  - Bytes `5..`: Sealed ciphertext.
- **Windows OS Account Protection (`DpapiKeyProtector`)**: Calls Win32 `CryptProtectData` and `CryptUnprotectData` (`CRYPTPROTECT_UI_FORBIDDEN`) bound to the current Windows user logon credential with domain separation entropy `b"nectarlink/identity/v1"`.
- **Android Keystore Protection (`KeystoreKeyProtector.kt`)**: Generates a non-exportable 256-bit AES-GCM key (`alias = "nectarlink.identity.v1"`) inside `AndroidKeyStore` (`PURPOSE_ENCRYPT | PURPOSE_DECRYPT`, `BLOCK_MODE_GCM`, `ENCRYPTION_PADDING_NONE`, which may be hardware-backed on devices with StrongBox or TEE) and seals secrets as `12-byte IV || ciphertext || 128-bit auth tag`.
- On Windows and Android, file permissions are restricted to the owner immediately upon creation. Legacy raw 32-byte key files are transparently migrated into `NLK1` sealed format on first load.

## Pairing Protocols (`ALPN_PAIR = "nectarlink-pair/0"`)

Implemented in `core/nectarlink-protocol/src/pairing.rs` and `core/nectarlink-core/src/pairing.rs`. Pairing runs over a dedicated QUIC ALPN (`nectarlink-pair/0`) and enforces a 5-minute expiration (`PAIRING_TTL = 300s`) and a maximum of 5 failed attempts (`MAX_FAILED_ATTEMPTS = 5`) before invalidating the pairing session.

### 1. QR Code Pairing Flow

```text
Host (displays QR)                                      Joiner (scans QR)
------------------                                      -----------------
Generates 128-bit secret S (16 bytes)
Encodes nectarlink://pair?v=0&id=<host_id>&s=<base64url_S>&n=<name>&a=<ip:port>

                  ◄─── QUIC + TLS 1.3 (ALPN: nectarlink-pair/0) ───
                  ◄─── pair.request { id, name, kind, proof } ─────
                       where proof = HMAC-SHA256(S,
                         "nectarlink-pair-v0" || joiner_id || host_id)

Verifies TLS peer == joiner_id
Verifies HMAC proof in constant time
                  ──── pair.accept { id, name, kind } ────────────►
                  ◄─── pair.done ──────────────────────────────────
Persists peer to SQLite                                 Persists peer to SQLite
Closes with CLOSE_PAIRED (0)
```

- Because the 128-bit secret `S` is transferred optically via the QR code (`s` encoded in URL-safe base64 without padding) and the HMAC binds both `joiner_id` and `host_id` verified against the TLS 1.3 peer certificates, an active network attacker cannot forge `pair.request` or substitute their own `DeviceId`.

### 2. Nearby Discovery and 6-Digit Code Flow

When pairing via mDNS discovery or manual IP without scanning a QR code, both devices execute a commitment-based Short Authentication String (SAS) ceremony:

1. **Initiation**: Joiner sends `pair.request` with `proof: None`.
2. **Commitment Exchange**:
   - Joiner (initiator `A`) generates a 128-bit random nonce `nA` and sends `pair.commit { commit: SHA-256(nA) }`.
   - Host (responder `B`) generates a 128-bit random nonce `nB` and replies with `pair.nonce { nonce: nB }`.
   - Joiner reveals `pair.reveal { nonce: nA }`.
   - Host verifies `SHA-256(nA) == commit` in constant time (`subtle::ConstantTimeEq`).
3. **SAS Calculation**: Both sides compute a 6-digit numeric code:
   - `SAS = (u32_be(HMAC-SHA256(nA || nB, "nectarlink-sas-v0" || idA || idB)[0..4]) & 0x7fff_ffff) mod 1,000,000` (where `idA` is the initiator's 32-byte Ed25519 public key and `idB` is the responder's 32-byte Ed25519 public key; `derive_sas_bound` also supports binding the TLS 1.3 session exporter `EXPORTER-nectarlink-pair-v0`).
4. **Two-Sided User Confirmation**: Both devices display the 6-digit code and require explicit user approval (`pair.confirm { accept: true }`). Neither side writes the peer to SQLite until `pair.done` completes the two-phase commit.

## Session Authentication (`ALPN_SESSION = "nectarlink/0"`)

Every operational session connects over `ALPN_SESSION` (`"nectarlink/0"`):

1. During the QUIC + TLS 1.3 handshake, `iroh` cryptographically verifies that the remote peer holds the private key for its claimed `EndpointId` (`DeviceId`).
2. Immediately upon connection acceptance (`node.rs`), the node looks up the remote `DeviceId` in the SQLite `peers` table (`store.is_paired(&peer_id)`).
3. If the `DeviceId` is not present in `peers`, the connection is immediately terminated with application close code `CLOSE_UNPAIRED` (`1`) before any control stream or capability handler is invoked.

## Authorization and the `CapabilityMatrix`

Implemented in `core/nectarlink-core/src/features.rs`. Every paired device has an independent set of 18 feature toggles stored in the `peer_toggles` SQLite table (`DEVICE_TOGGLES`).

### Default-On vs Opt-In Toggles

| Category | Toggle Keys | Default State |
|---|---|---|
| Standard Continuity (13) | `notifications`, `messages`, `calls`, `contacts`, `clipboard`, `files`, `media`, `photos`, `recordings`, `toggles`, `pc_actions`, `mirroring`, `webcam` | Enabled (`true`) upon pairing |
| High-Sensitivity / Opt-In (5) | `storage`, `pc_mirror`, `remote_input`, `commands`, `remote_files` | Disabled (`false`) upon pairing; must be explicitly enabled by the user |

### Inbound Enforcement

`CapabilityMatrix::is_allowed(peer, capability)` maps every incoming wire capability (`cap::SMS`, `cap::STORAGE`, `cap::MIRROR_PC`, `cap::REMOTE_INPUT`, `cap::COMMANDS`, etc.) to its governing toggle key (`feature_for_capability`) and checks the peer's persisted policy before executing any handler. Outbound operations similarly call `check_toggle` before opening streams.

### Relay Restrictions (**Connect over mobile data** / `away_mode`)

When a session is connected over `ConnectionPath::Relay` (**Connect over mobile data** / `away_mode`), `Node` applies additional transport-aware authorization:

- Remote file system browsing and downloads (`storage.v0`) are blocked on relay connections unless the user has enabled **both** the `storage` toggle and the opt-in `remote_files` toggle for that peer.
- Relay discovery and relay servers remain completely disabled (`RelayMode::Disabled`) unless the user explicitly turns on **Connect over mobile data** (`away_mode`).

## Runtime Consent Gates

Even when a capability toggle is enabled, sensitive operations require interactive user confirmation:

1. **Reverse PC Screen Mirroring (`mirror.pc.v0`)**: When a phone requests to view the PC screen, the desktop app emits `NodeEvent::PcMirrorRequested` and blocks streaming until the user approves the prompt (`set_pc_mirror_session_allowed`). While active, the desktop displays a persistent top banner with a one-click **Stop Sharing** button (`stop_pc_mirror`).
2. **Remote Input Control (`remote.input.v0`)**: Synthesizing mouse and keyboard input on the PC requires both the opt-in `remote_input` toggle and interactive session approval (`NodeEvent::RemoteInputRequested`).
3. **Phone Storage Browser (`storage.v0`)**: First-time remote browsing of phone shared storage triggers an on-phone confirmation prompt (`storage_prompted` state) in addition to Android's system `MANAGE_EXTERNAL_STORAGE` permission.
4. **Phone-as-Webcam (`webcam.v0`)**: Starting a camera stream from the PC notifies the phone and allows the phone user to stop or flip the camera at any time.
5. **LocalSend Incoming Files (`localsend.v2`)**: Unauthenticated LAN LocalSend peers never bypass pairing; every incoming LocalSend offer enters `TransferState::Requested` and waits for explicit user acceptance before writing files to the Downloads directory.

## File System and Path Traversal Protections

- **Incoming Transfers (`core/nectarlink-core/src/transfer.rs`)**: `sanitize_relative_path` strips leading slashes, drive prefixes, `.` components, and rejects any path containing `..` (`Component::ParentDir`), confining all writes to `<download_dir>` and `<data_dir>/incoming/<peer>/<transfer_id>/`.
- **Storage Browser (`android/app/.../StorageHelper.kt`)**: Resolves every requested path via `File(root, rel).canonicalFile` and verifies that `canonical.path` starts with the canonical `/storage/emulated/0` root prefix, blocking symlink escapes into `/data/data/app.nectarlink.android`.

## Automatic Updates and Release Verification

Implemented in `desktop/app/src/updater.rs` (`win/http.rs`), `desktop/installer/nectarlink.nsi` (the task's definition) and `android/app/src/main/kotlin/app/nectarlink/android/update/AppUpdater.kt`.

- **Endpoint**: Both apps query `https://api.github.com/repos/alejandrokundrah/nectarlink/releases/latest` over HTTPS with the OS TLS stack (`WinHTTP`, `HttpURLConnection`). Only `https://` URLs are fetched, redirects stay on HTTPS, and responses are size-capped while they are read. No device identifiers or telemetry are sent.
- **What is accepted**: a published release (not a draft or pre-release) whose version is newer than the running one, with an installer named `Nectarlink-*-x64-setup.exe` (Windows, a plain file name) or `*-android.apk`, and a `SHA256SUMS.txt` that lists it. The download is rejected unless its SHA-256 matches. Android additionally refuses an APK not signed with the app's own key.
- **Windows, installing without a prompt**: the app is installed per machine, so installing needs administrator rights. The installer registers the scheduled task **Nectarlink Update**, which runs `nectarlink-desktop.exe --update-service` as SYSTEM. Its security descriptor gives SYSTEM and Administrators full control and authenticated users read and execute only, so a signed-in user can start it but not change what it runs. The task takes no arguments or input from the user: it fetches the latest release itself and applies the checks above. When the user clicks **Update**, the app first downloads the installer itself (resumable, with progress) into `%ProgramData%\Nectarlink\updates` and checks it against the sums. The task copies that file into a new randomly named folder in `C:\Windows\SystemTemp` (writable only by SYSTEM and administrators) and checks the copy against the sums it fetched itself, so a file planted in the shared folder can only ever be the official installer. When there is no matching file, it downloads the installer into that folder itself. It then runs the installer with `/S` and logs to `update.log` in the install folder. The worst a user can do by starting it is install the latest official release early.
- **Windows, when**: with **Install updates automatically** on, the app starts the task once nobody has used the PC for 10 minutes and nothing would be interrupted (mirroring, an app window, a transfer, a call, screen sharing, the webcam). It first starts a hidden relauncher, which reopens the app once the new version's `DisplayVersion` is in the uninstall key and the installer has exited, then quits. Without the task (copies installed before it existed) **Update** runs the installer with the usual UAC prompt.
- **Android**: with **Install updates automatically** on, the connection service checks in the background at most once a day and installs through `PackageInstaller` with `USER_ACTION_NOT_REQUIRED` (Android 12+, `UPDATE_PACKAGES_WITHOUT_USER_ACTION`). Android skips the prompt only when NectarLink installed the current version itself; otherwise the user confirms from a notification.
- **Opt-out**: **Check for updates** and **Install updates automatically** in Settings on both apps. With checks off, nothing is fetched.
- **Not yet**: release binaries are not code-signed (Authenticode) yet, so trust rests on HTTPS to GitHub and the release's checksums.

## Localhost HTTP API Isolation

Implemented in `desktop/app/src/local_api.rs`.

- **Default State**: Disabled (`local_api_enabled: false`).
- **Network Binding**: Binds strictly to `127.0.0.1:53320` (`DEFAULT_API_PORT`), never `0.0.0.0`.
- **Token Authentication**: Generates a 32-byte (64-hex-char) random token from the system CSPRNG (WinRT `CryptographicBuffer`, falling back to CNG `BCryptGenRandom`; the API stays off if neither answers) stored in `<data_dir>/api-token.json`. Every request must supply `Authorization: Bearer <token>` or `?token=<token>`, checked using a constant-time byte comparison (`constant_time_eq`).
- **Browser Drive-By Defense**: `has_browser_origin` inspects HTTP headers before routing and returns `403 Forbidden` if either `Origin` is present or `Sec-Fetch-Mode` is `cors` or `no-cors`, preventing websites opened in a local browser from probing or invoking the API.
