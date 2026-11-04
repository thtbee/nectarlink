# Privacy and Security

NectarLink is built so that your personal data stays on your own devices under your direct control. There is no user account, no cloud database, no analytics or telemetry, and no advertising SDK. Every connection between your phone and your PC is mutually authenticated and encrypted end to end.

For a line-by-line engineering audit of every network, IPC, shared-memory, and filesystem boundary, see [`docs/SECURITY-REVIEW.md`](../SECURITY-REVIEW.md).

## 1. Device Identity Keys

When NectarLink starts for the first time on your PC or Android phone, it generates a persistent 256-bit **Ed25519** cryptographic identity keypair:

- **How devices are identified**: Every device is addressed by its 32-byte Ed25519 public key (`DeviceId`), never by an IP address, hostname, or email address.
- **How your private key is protected at rest**:
  - On **Windows**, `identity.key` is encrypted using the Windows Data Protection API (**DPAPI** via `CryptProtectData` with `CRYPTPROTECT_UI_FORBIDDEN` and entropy `b"nectarlink/identity/v1"`), tied to your Windows user account credentials (not hardware-backed).
  - On **Android**, `identity.key` is encrypted by a non-exportable **AES-256-GCM** key (`nectarlink.identity.v1`) inside the **Android Keystore**, which may be hardware-backed depending on your phone.
  - On **Linux**, `identity.key` isn't encrypted yet. It's protected by file permissions (`0600`), so only your user can read it, but anything running as you can, and so can anyone who reads the disk. Full-disk encryption covers the second case.

## 2. Cryptographic Pairing (Out-of-Band Secret and 6-Digit SAS)

An unknown device on your Wi-Fi network can never read your notifications, clipboard, or files. Devices only communicate after completing a mutual pairing ceremony (`ALPN = "nectarlink-pair/0"`):

- **QR Code Pairing**: When you display a pairing QR code on your PC, NectarLink generates a one-time 128-bit random secret (`s`, encoded in URL-safe base64) that expires after 5 minutes and is invalidated after use or after 5 failed attempts. Scanning that QR code proves your phone observed the secret out of band by computing `HMAC-SHA256(s, "nectarlink-pair-v0" || idA || idB)` over both devices' Ed25519 public keys (`idA` for the initiator and `idB` for the responder).
- **Nearby and USB 6-Digit Code Pairing**: When you pair without a camera (via **Pair nearby** or **USB auto-pair**), both devices establish a TLS 1.3 session, exchange SHA-256 commitments to 128-bit random nonces (`nA` and `nB`), and derive a 6-digit Short Authentication String (**SAS**) using **HMAC-SHA256** (`uint32_be(HMAC-SHA256(nA || nB, "nectarlink-sas-v0" || idA || idB)[0..4]) mod 1,000,000`). If an attacker on an untrusted Wi-Fi network attempted a man-in-the-middle attack, the public keys in the SAS calculation would not match on both sides. Nothing is paired until you confirm **Codes match** on both devices.

- **Pairing links opened from outside the app**: A `nectarlink://pair` link that reaches the phone from a web page, another app, or the system camera never pairs on its own. The phone asks first, naming the PC, and pairs only after you tap **Pair**.

<p align="center">
  <img src="images/pair-link-prompt.png" alt="The phone asking Pair with Bee? after a pairing link was opened" width="360">
</p>

Once paired, each device saves the peer's Ed25519 public key in its local SQLite trust store (`peers` table).

**Install ID**: Each device also shares a 32-character install ID with the devices it pairs with. It is a salted BLAKE3 hash of the Android ID on the phone, or the Windows machine GUID or Linux machine ID on the PC, so the underlying identifier is never sent. It is used for one thing: when you reset and re-pair the same device, the old entry is replaced instead of left behind. Two different devices with the same name are never merged.

## 3. End-to-End Encrypted Connections (QUIC + TLS 1.3)

Every post-pairing session (`ALPN = "nectarlink/0"`) runs over **QUIC** (via `iroh`) with **TLS 1.3** mutual authentication:

- **Mutual verification**: Both the phone and the PC verify the peer's Ed25519 signature during the TLS 1.3 handshake and immediately drop any connection whose public key is not in the local `peers` table.
- **USB tunnel encryption**: Even when connected over a USB cable (`On USB`), NectarLink runs the exact same TLS 1.3 end-to-end encrypted QUIC protocol over the local loopback bridge.
- **Strict wire bounds**: Every protocol message is length-prefixed CBOR capped at 1 MiB before memory allocation (`MAX_FRAME_LEN`), preventing malformed frames from exhausting memory.

## 4. Network Connections and Automatic Update Checks

### Local-First Device Communication

- **Local network by default**: By default, NectarLink discovers your paired devices on the local network using mDNS and connects directly over LAN IPv4/IPv6 or a USB cable (`127.0.0.1:53319` over `adb forward`). No device-to-device data leaves your local network unless you turn on **Connect over mobile data**.
- **Opt-in **Connect over mobile data** (`away_mode`, off by default on both PC and Android)**: If you turn on **Connect over mobile data** in **Settings › Connection & About** on the PC and **Settings › Devices** on Android, NectarLink maintains your connection when your phone is on LTE/5G or a different Wi-Fi network.
- **What it publishes**: While it's on, your device publishes its network addresses to `iroh`'s public address service under its public key, so your other device can find it. Your device's name isn't included.
- **Turning it off**: It takes effect right away. Publishing and looking up stop, connections that aren't on your local network or USB close, and no relay carries anything more. One idle connection to the relay stays open, carrying nothing, until NectarLink next restarts.
- **End-to-end encryption over relays**: When a direct UDP hole-punch cannot be established while **Connect over mobile data** is enabled, encrypted QUIC packets are forwarded through an `iroh` relay. Because TLS 1.3 terminates exclusively inside your phone and your PC using your paired Ed25519 keys, a relay only sees opaque encrypted UDP datagrams and can never decrypt your messages, notifications, clipboard, or files.
- **Extra protection for remote storage**: Even if you have enabled **File Explorer integration** (`storage`) on your home Wi-Fi, NectarLink blocks storage browsing over a relay connection unless you also explicitly enable **Remote PC file access** (`remote_files`, off by default) in **Settings › Devices**.

### Automatic Update Checks

To keep your installation current without an app store, installed copies check this project's GitHub repository over HTTPS. Each check sends only a `User-Agent` such as `Nectarlink/<version> (Windows)`, with no personal or device information.

- **What's fetched**: your channel's feed, `https://raw.githubusercontent.com/thtbee/nectarlink/channels/<stable|nightly>.json`, or the GitHub Releases API if no feed is published. A download is only accepted from this repository's own release files, and only if its SHA-256 matches.
- **How often on the PC**: 30 seconds after startup, then every 12 hours on Stable or every 2 hours on Nightly.
- **How often on Android**: at most once a day on Stable or every 6 hours on Nightly, in the background and when the app comes to the front, and when you tap **Check for updates**. A paired device can tell another that a build is out, which only starts the same check sooner. It can't name a file.
- **Installing on Windows**: with **Install updates automatically** on (the default), the update runs at a quiet moment, or when you click **Update**. The installer registers a Windows task, **Nectarlink Update**, that runs the update as SYSTEM so no administrator prompt is needed. It takes no input from whoever starts it, only installs an official build, and writes what it did to `update.log` in the install folder.
- **Installing on Android**: a newer APK is downloaded, checked and installed. Android only installs it over NectarLink if it carries the same signature, so only official builds can update the app.
- **Installing on Linux**: an install only takes its own kind of file, such as a `.deb` for a `.deb` install.

Turn either setting off in **Settings › Connection & About** on the PC, or **Settings › Features & Data › About** on Android.

### Network Connections NectarLink Makes

| Connection / Activity | Protocol & Destination | Default State | How to Control or Disable |
| :--- | :--- | :--- | :--- |
| **Paired device discovery & session** | mDNS (`_nectarlink._udp.local`) and direct QUIC + TLS 1.3 UDP on your local LAN | **On** (LAN only) | Unpair devices in **Settings › Devices** or quit NectarLink |
| **USB wired transport (`On USB`)** | QUIC + TLS 1.3 over loopback TCP (`127.0.0.1:5037` ADB host to `127.0.0.1:53319` on device) | **On** when USB debugging is active | Unplug the USB cable or turn off USB debugging on the phone |
| **Automatic update check (PC)** | HTTPS `GET` of the channel feed on `raw.githubusercontent.com`, or `api.github.com` (see above); downloads the update from this repository's releases when a newer one exists | **On** on installed copies | Turn off **Settings › Connection & About › Check for updates** |
| **Automatic / manual update check (Android)** | The same feed over HTTPS (see above); downloads the `.apk` from this repository's releases when a newer one exists | **On** in release builds | Turn off **Settings › Features & Data › About › Check for updates** |
| **Home radio** | HTTPS `GET` to the Radio Browser directory (`*.api.radio-browser.info`) for the station list, then the chosen station's own stream (plain HTTP) | **Off** until you press play, tune or open the radio's settings | Don't use the radio |
| **Home weather** | HTTPS `GET` to Open-Meteo (`geocoding-api.open-meteo.com` for the city you type, `api.open-meteo.com` for its weather) every 30 minutes while Home is open | **Off** until you set a city | Don't set a city |
| **Connect over mobile data** (`away_mode`) | `iroh` DNS/Pkarr lookup and end-to-end TLS 1.3 encrypted QUIC relay servers | **Off** on both PC and Android | Toggle **Settings › Connection & About › Connect over mobile data** (PC) and **Settings › Devices › Connect over mobile data** (Android) |
| **Voice typing** | Android's speech recognizer on your phone, which may send what you say to its provider (usually Google) | Only while you tap or hold **Dictate** | Don't use **Dictate** |
| **LocalSend discovery & transfer** | UDP multicast `224.0.0.167:53317` and HTTPS/HTTP TCP `53317` (local network only) | **Off** on both PC and Android | Toggle **Settings › Files & Sync › LocalSend on local network** (PC) or **Settings › Features & Data › LocalSend** (Android) |
| **Local automation API (PC)** | Loopback HTTP on `127.0.0.1:53320` (never bound to LAN interfaces) | **Off** | Toggle **Settings › Connection & About › Local automation API** |

## 5. Per-Device Feature Toggles and Explicit Consent Prompts

Pairing a device does not give it unrestricted access to everything. You control 19 individual per-device feature toggles in **Settings › Devices** (see [Settings Reference](settings.md#3-devices)), and sensitive capabilities require explicit user consent:

| Capability | Default State | Consent and Visibility Controls |
| :--- | :--- | :--- |
| **Reverse Mirroring** (`Share PC screen to phone`) | **Off** | Prompts on the PC screen (**Not now**, **Allow**, **Allow for this device**) when requested from a phone; shows a persistent **Sharing this PC's screen with `<Phone>`** top banner with a one-click **Stop sharing** button while active. |
| **Remote Input to PC** (`Remote mouse, keyboard & Deck`) | **Off** | Prompts on the PC the first time a paired phone asks to move the cursor or send keys. |
| **Deck Shell Commands** (`Run Deck shell commands`) | **Off** | Separate per-phone toggle that must be manually enabled in PC Settings before any `Deck` command tile can run. |
| **File Explorer Storage** (`File Explorer integration`) | **Off** on both PC and phone | Must be turned on per device, with an explicit prompt on the phone and support for sharing only specific folders (**Selected folders only**). |
| **Phone Screen Mirroring** (`Screen mirroring`) | **On** | Android prompts before sharing at `Standard`/`Assist`, and shows a persistent notification with a **Stop** button (or **Close all** for `app window` sessions) on the phone while active. |
| **Deleting Photos from the PC** | **On** with `photos` | Needs **All files access** on the phone. A PC you've paired can move photos to the phone's trash without a prompt on the phone, and they stay restorable there. Turn off `photos` for that PC to prevent it. |
| **Phone as Webcam, Phone as Microphone & Continuity Camera** | **On** | Starting them from the PC posts a notification on the phone requiring a physical tap on the phone before the camera opens, plus a persistent **Stop** notification and Android's green camera dot. The one exception is a PC you chose for **Start the camera and microphone without asking** on the phone (`Elevated`, off by default): its webcam and microphone requests start right away. |
| **Type on Phone from PC** | Requires `Assist` or `Elevated` | Shows a persistent **Typing from `<PC>`** banner on the phone with a one-tap **Stop typing** button. |
| **LocalSend** (`LocalSend on local network`) | **Off** | Every incoming file offer from a nearby LocalSend device requires explicit user approval (**Accept** / **Decline**) before any bytes are written. |
| **Forward PC Notifications / Stream PC Audio / Share PC Folders** | **Off** | All three PC-to-phone sharing features are off by default and can only be enabled locally on the PC. |

## 6. How the `Elevated` Shell Helper Is Secured

If you choose to enable the `Elevated` power level on Android, NectarLink starts its companion helper (`app.nectarlink.android.elevated.InputServer` via `app_process`) running as Android's `shell` user (`UID 2000`) through Wireless Debugging or USB `adb`.

To ensure no other app on your phone can abuse this helper:

1. **Loopback Binding and 192-Bit Per-Session Token Authentication**: Because Android SELinux blocks regular apps from connecting to `shell`-owned Unix domain sockets, `InputServer` binds exclusively to a loopback TCP socket (`127.0.0.1:<ephemeral_port>`) and requires a 24-byte (48-hex-character) `SecureRandom` secret token on the first line of every connection. The token is generated fresh by the NectarLink app when launching the helper and verified in constant time (`MessageDigest.isEqual`); any unauthenticated connection is closed immediately before reading a command.
2. **Fixed Commands and a Strict Allowlist**: Commands sent over the socket are fixed verbs dispatched directly to Android APIs (`IInputManager`, `IClipboard`, `/dev/uhid` and others). The one that runs a shell command, `C`, only accepts an explicit list: expanding the notification shade, Wi-Fi, Bluetooth, hotspot, brightness `0..255`, and the grants NectarLink needs for itself. Arbitrary shell commands are rejected.

## 7. Local Data Storage, Encryption, and Retention

All cached data stays inside your local profile directory (`%LOCALAPPDATA%\Nectarlink` on Windows, `~/.local/share/Nectarlink` on Linux and the app's private data folder on Android):

| Data Store | How It Is Stored | Default Retention |
| :--- | :--- | :--- |
| **Clipboard History** | Encrypted at rest in `clipboard_history.enc` (`NLCB\x01` header) and `clipboard_images/<id>.enc` (`NLCI\x01` header) directly using the platform `KeyProtector` (**Windows DPAPI** on Windows, **Android Keystore AES-256-GCM** on Android; not sealed on Linux, where only file permissions protect it). Passwords marked sensitive by password managers and one-time verification codes (OTPs) are never sent or saved. | **7 days**, up to 50 items / 128 MB |
| **Notification History (PC)** | Stored locally in `notification-history.json` (and per-app rules in `notification-rules.json`) with cached images in `cache\images\`. One-time verification codes are excluded. Not duplicated on the phone. | **24 hours**, up to 300 items |
| **Cross-Device `Timeline`** | Stored locally in `nectarlink.db`. Clipboard items in `Timeline` reference `clipboard_history.enc` by ID so clip text is never stored in plain text in SQLite. | **90 days** (configurable to `30d`, `90d`, `1y`, or up to `5,000` items) |
| **Cached Messages (PC)** | Cached in `nectarlink.db` and `cache\images\` for fast inbox loading. Temporary MMS staging files on Android are deleted within 5 minutes. | **90 days** on PC |
| **Photo Thumbnails (PC)** | Cached in `cache\photos\`. Full-resolution photos are only fetched on demand. Temporary Continuity Camera captures on Android are deleted within 24 hours. | **30 days** on PC |
| **Offline `Waiting to send` Queue** | Stored in `nectarlink.db` (`send_queue` table). Copied text and images, links and task alerts are sealed with the platform `KeyProtector`, like clipboard history (not on Linux, which has none yet). File names are stored as they are, and files are referenced by path rather than duplicated. | Camera requests: **2m**; Links/Clips/Alerts: **24h**, then removed; Files: **7d**. Clearing clipboard history removes waiting clips. **Clear everything...** also removes waiting links and alerts, and leaves files you chose to send. |
| **OBS Studio Password (PC)** | Saved in `settings.json` only after you enter it in **Settings › Shortcuts & Rules › OBS Studio**, encrypted with **Windows DPAPI** for your Windows account (on Linux, protected by file permissions only). Used only to sign in to OBS's WebSocket server on this PC (`127.0.0.1`). | Until you remove it |
| **Media Artwork Lookup** | Extracted from the local media session (Windows GSMTC, Linux MPRIS or Android `MediaSession`) and cached locally on disk. | Local session cache |

You can clear any individual store or click **Clear everything...** at any time in **Settings › Storage & Privacy** on the PC or **Settings › Features & Data › Data & storage** on Android.
