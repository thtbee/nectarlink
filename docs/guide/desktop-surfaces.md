# Desktop Surfaces and Automation

Beyond the main window, NectarLink integrates into Windows 11 with fast, low-overhead desktop surfaces and automation tools: the slide-out **`Shelf`** edge panel, **`Nectar Island`** at the top of your screen, the searchable cross-device **`Timeline`**, the global **`Command Palette`** (`Ctrl+Alt+Space`), rich **`Handoff`** for links, maps, videos, and documents, **`notify-when`** process completion alerts, a trigger-action **Rules Engine**, a local authenticated **HTTP API**, a **PowerToys Command Palette** extension, and the `nectarlink` command-line client.

<p align="center">
  <img src="images/command-palette.png" alt="Global Command Palette open over the NectarLink window on Windows 11" width="760">
</p>

## What You Need

| Feature | Requirements |
| :--- | :--- |
| **`Shelf` edge panel (`Ctrl+Alt+S`)** | Windows 11 app running (window open or in the system tray). |
| **`Nectar Island`** | Turn on **Nectar Island** in PC **Settings › General** (off by default). |
| **`Timeline`** | Enabled by default on both Windows and Android (retains up to 5,000 items or 90 days by default). |
| **`Command Palette` (`Ctrl+Alt+Space`)** | Windows 11 app running (window open or in the system tray). |
| **Rich `Handoff` (links, maps, video timestamps, documents)** | `Standard` power level. **Files** toggle enabled in **Settings › Devices**. |
| **`notify-when` task alerts & CLI (`nectarlink`)** | Windows 11 app running in the same user session. |
| **Trigger-action automations** | Configured in PC **Settings › Shortcuts & Rules › Trigger-action rules**. |
| **Local automation API (`127.0.0.1:53320`)** | Turn on **Local automation API** in PC **Settings › Connection & About** (off by default). |

## Step-by-Step Use

### 1. The `Shelf` Slide-Out Edge Panel and Flick to Send

The `Shelf` is a lightweight edge panel on your PC that gives you instant drag-and-drop access to your phone's recent items without switching windows or stealing keyboard focus from your current app:

<p align="center">
  <img src="images/shelf-panel.png" alt="The slide-out Shelf edge panel showing the latest photo, screenshot, clipboard item, and recent files" width="760">
</p>

1. Press **`Ctrl+Alt+S`** from any application (or flick a file toward the screen edge) to slide out the `Shelf`.
2. The `Shelf` displays four live slots from your connected phone:
   - **Latest photo**
   - **Latest screenshot**
   - **Current clipboard**
   - **Recent received files**
3. **Drag out to any app**: Drag any item out of the `Shelf` directly into an email, document, browser upload box, chat window, or File Explorer folder.
4. **Drop or Flick in to send**: Drag any file, folder, image, or text selection from your PC into the `Shelf` (or flick a file toward the `Shelf` screen edge) to send it straight to your phone, with a 3-second **Undo** option.
5. Press **Escape** or click outside the `Shelf` to hide it. When hidden, the `Shelf` unloads its QML surface completely so it uses zero background GPU or memory overhead.

### 2. `Nectar Island`

`Nectar Island` is an optional compact pill at the top center of your primary monitor that surfaces live phone activity without covering your workspace in notification popups:

1. Turn on **Nectar Island** in PC **Settings › General** (it is **off by default**).
2. In the **Island triggers** row right below it, choose which activities appear in the pill:
   - **Calls** (on by default): Incoming caller ID with Answer/Decline buttons and live call duration.
   - **Transfers** (on by default): Live file transfer progress bars and `Waiting to send` count.
   - **Media** (on by default): Now-playing track artwork, title, and transport controls.
   - **Alerts** (off by default): Phone notifications, one-time verification codes, active timers/Live Updates, and incoming voice recordings.
3. Hover over or click `Nectar Island` to expand its interactive controls, or let it collapse back into a minimal pill when idle. `Nectar Island` automatically hides itself when you are running a full-screen game or video, or when Windows Do Not Disturb is active.

### 3. Cross-Device `Timeline`

The `Timeline` gives you one searchable local log of everything that has moved between your phone and your PC:

<p align="center">
  <img src="images/timeline-page.png" alt="Timeline page on Windows showing files, clips, links, photos, and sessions grouped by day" width="760">
</p>

1. Click **Timeline** in the left navigation bar on your PC (or scroll to the **Timeline** card on the Android Home screen).
2. Entries are grouped by day (**Today**, **Yesterday**, and earlier dates) and include:
   - Sent and received **Files** and folders
   - Shared **Clips** (linked by ID to encrypted clipboard history so clip text is never duplicated in SQLite)
   - Opened **Links**, map locations, and video handoffs
   - Saved **Photos** and Continuity Camera scans
   - Received **Recordings** (voice notes)
   - Screen mirroring and webcam **Sessions** (with duration)
3. Filter by category (**All**, **Files**, **Clips**, **Links**, **Photos**, **Recordings**, **Sessions**), filter by paired device when you have more than one phone or PC, or type in the **Search timeline** box.
4. Each entry offers quick actions matched to its type: **Open**, **Show in folder**, **Copy**, **Send again**, or **Remove**.

### 4. Global `Command Palette` (`Ctrl+Alt+Space`)

Press **`Ctrl+Alt+Space`** from anywhere in Windows to open NectarLink's `Command Palette` over your current window:

1. Start typing to fuzzy-search every action, paired device, recent file, or setting:
   - **Mirror phone screen**
   - **Send files to phone** / **Send clipboard to phone**
   - **Take photo with phone** (`Ctrl+Alt+C`) / **Scan document with phone** (`Ctrl+Alt+D`)
   - **Type on phone with PC keyboard**
   - **Ring phone (Find my phone)**
   - **Toggle Phone as speaker**
   - **Open phone storage in File Explorer**
   - **Open Setup walkthrough** / **Open Features guide**
   - **Run Connection Doctor**
2. You can also type or paste a web URL or text directly into the `Command Palette` to send it to your phone immediately.
3. Use the **Up** and **Down** arrow keys and press **Enter** to run the highlighted command, or press **Escape** to close the palette.

### 5. Rich `Handoff` (Links, Maps, Video Timestamps, and Documents)

`Handoff` lets you start reading, watching, navigating, or editing on one device and continue immediately on the other:

- **Web links**: Share any URL from your phone's browser to your PC via the Android share sheet (or send a URL from your PC via the `Command Palette` or `nectarlink send <url>`). The link opens immediately in the default browser on the receiving device.
- **Maps and street addresses**: Share a Google Maps link or `geo:` location URI between devices to open that location directly in Maps.
- **Video links with playback timestamp**: When you share a YouTube, Vimeo, or Twitch link (or run `nectarlink send <url> --at-seconds 142`), NectarLink attaches the current playback position so the receiving device offers a **Continue watching** action that resumes the video at the exact second you left off.
- **Documents (`--open` / `Shift+Drop`)**: Hold **`Shift`** while dropping a document onto NectarLink (or run `nectarlink send report.pdf --open`) to transfer the file and open it automatically in the default viewer on the receiving device as soon as the transfer finishes.

### 6. Task Completion Alerts (`nectarlink notify-when`)

When you start a long build, test run, render, or download on your PC and want to step away with your phone:

1. In any Windows terminal (PowerShell, Command Prompt, or Windows Terminal), wrap a command or attach to an existing process ID (`PID`):
   ```powershell
   # Run a command and alert your phone when it finishes or fails
   nectarlink notify-when -- cargo test --workspace

   # Or watch an already-running process by its PID
   nectarlink notify-when 14820
   ```
2. While the command or process is running on your PC, your phone displays a live ongoing notification (**Running on `<PC>`**) with a ticking chronometer timer.
3. As soon as the process exits, your phone receives a completion alert showing whether it succeeded or failed, its exit code, and how long it took (for example, `cargo test finished / Took 1m 42s` or `cargo build failed (exit 101)`). If your phone briefly goes offline while the task finishes, the completion alert waits in the `Waiting to send` queue and arrives on reconnect.

### 7. Trigger-Action Automations (Rules Engine)

You can automate routine actions between your phone and PC without writing scripts:

1. Open PC **Settings › Shortcuts & Rules** and scroll to the **Trigger-action rules** card.
2. Click one of the quick presets (**Lock PC on disconnect** or **Pause media on ring**) or add custom rules combining a trigger and an action:
   - **Triggers**: `When phone connects`, `When phone disconnects`, `When phone rings`, `When call ends`, `When phone battery is low`, `When phone is charged`.
   - **Actions**: `Lock this PC`, `Sleep this PC`, `Pause PC media`, `Resume PC media`, `Mute PC audio`, `Unmute PC audio`, `Show notification`, `Run command`.
3. Toggle individual rules on or off or click **Remove** to delete a rule at any time.

### 8. Local Authenticated HTTP API and PowerToys Extension

For power users who use **PowerToys Command Palette**, **AutoHotkey**, **Raycast**, **Flow Launcher**, or custom PowerShell/Python scripts, NectarLink includes an opt-in local HTTP/JSON API:

1. Open PC **Settings › Connection & About** and turn on **Local automation API** (default port `53320`).
2. NectarLink binds strictly to loopback (`127.0.0.1:53320`, never exposed on LAN) and writes a random 256-bit Bearer token to `%LOCALAPPDATA%\Nectarlink\api-token.json`.
3. Every request must include `Authorization: Bearer <token>`, and any request carrying a browser `Origin` header is rejected with `403 Forbidden` to prevent cross-site browser attacks.
4. Available endpoints (see [`docs/api.md`](../api.md) for full request/response schemas and PowerShell/`curl` examples):
   - `GET /v1/devices`: List paired devices, connection status, and battery level.
   - `POST /v1/send`: Send files, URLs, or text to a phone.
   - `GET /v1/clipboard` and `POST /v1/clipboard`: Read or push clipboard text and history.
   - `POST /v1/notify`: Send a custom notification to a phone.
   - `POST /v1/ring`: Trigger Find my phone.
   - `POST /v1/play-on-phone`: Play what this PC plays on the phone.
   - `POST /v1/camera`: Trigger Continuity Camera (`"photo"` or `"scan"`).
   - `GET /v1/timeline`: Query recent cross-device `Timeline` entries.
5. A companion **PowerToys Command Palette** extension in `tools/powertoys-cmdpal/` connects to this API so you can invoke NectarLink actions directly from PowerToys.

## What the Options Do

- **Nectar Island** and **Island triggers** (`Settings › General` on PC, default: **Off**): Enables the top-of-screen activity pill and selects which triggers (`Calls`, `Transfers`, `Media`, `Alerts`) expand it.
- **Command palette / File shelf / Take photo / Scan document shortcuts** (`Settings › Shortcuts & Rules` on PC): Customizes the global Windows hotkeys (`Ctrl+Alt+Space`, `Ctrl+Alt+S`, `Ctrl+Alt+C`, `Ctrl+Alt+D`), with a **Reset** button next to each.
- **Trigger-action rules** (`Settings › Shortcuts & Rules` on PC): Manages "When X, Do Y" automation rules.
- **Cross-device timeline** (`Settings › Storage & Privacy` on PC and `Settings › Features & Data › Data & storage` on Android, default: **90d**): Sets auto-purge retention (**30d**, **90d**, **1y**, or **All** up to 5,000 items) and provides a **Clear** button.
- **Local automation API** (`Settings › Connection & About` on PC, default: **Off**, port `53320`): Starts or stops the localhost HTTP server and configures its TCP port (`1024` to `65535`).

## Data and Privacy

- **Local-only `Timeline`**: `Timeline` entries are stored exclusively in your local SQLite database (`nectarlink.db`) on each device and are automatically pruned according to your chosen retention period (**30 days**, **90 days**, **1 year**, or **5,000 items**). Clipboard entries in `Timeline` store only a pointer to your encrypted `clipboard_history.enc` file so clip text is never stored in plain text inside SQLite.
- **Zero network exposure for the Local API**: The Local automation API is off by default, binds only to `127.0.0.1`, requires a 256-bit CSPRNG Bearer token readable only by your Windows user account, and deletes `api-token.json` immediately when turned off.

## Limits

- **Global Hotkey Conflicts**: If another Windows application has already registered `Ctrl+Alt+Space`, `Ctrl+Alt+S`, `Ctrl+Alt+C`, or `Ctrl+Alt+D` as a system-wide hotkey, you can assign any non-conflicting `Ctrl`/`Alt`/`Shift`/`Win` key combination in **Settings › Shortcuts & Rules**.
- **No Windows Widgets board widget yet**: NectarLink doesn't offer a widget in the Windows 11 `Win+W` Widgets board. The taskbar **Jump List** works on all installs, and on Android the **PC quick actions** home-screen widget works on all devices.
