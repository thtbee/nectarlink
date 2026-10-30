# Settings Reference

This page explains every setting in the Windows 11 app and the Android app one by one, including its exact in-app label, default value, and what changing it does.

<p align="center">
  <img src="images/settings-appearance.png" alt="Categorized two-pane Settings screen in NectarLink for Windows" width="760">
</p>

## Windows 11 App Settings

On Windows, **Settings** uses a two-pane layout with 9 categories in the left navigation rail (or a scrollable top pill bar when the window is narrower than 680 pixels). Preferences are stored atomically in `%LOCALAPPDATA%\Nectarlink\desktop-settings.json`, while per-device permissions live in your local SQLite database (`nectarlink.db`).

---

### 1. Appearance

Controls the visual theme, light/dark mode, accent palette, OLED black surfaces, and Windows 11 Mica backdrop.

| Setting | Options / Default | What It Does |
| :--- | :--- | :--- |
| **Theme** | `Hive`, `Bloom` (default), `Glass`, `Graphite` | Selects the visual style. **Hive** is honey on charcoal (dark) or warm wax (light), with a living honeycomb behind the whole window that grows from the logo, breathes, and sends soft pulses of light; its colors are fixed, so the color options below are hidden. **Bloom** uses soft, tinted Material You surfaces. **Glass** renders translucent cards over the Windows backdrop, each with a lit glass rim: a specular edge from the top left, a softer reflection at the bottom right, and a faint inner glow. **Graphite** uses high-contrast monochrome ink-on-paper surfaces. |
| **Mode** | `System` (default), `Light`, `Dark` | Follows Windows 11's system light/dark mode or locks NectarLink to Light or Dark. |
| **Color** *(shown in Bloom and Glass)* | **Wallpaper** (default), **Honey**, **Lavender**, **Ocean**, **Sage**, **Black**, **Media artwork**, or **Phone color pills** | Picks the accent seed color. **Wallpaper** extracts colors from your desktop wallpaper. **Media artwork** dynamically adapts the app's palette to the album art of whatever song is currently playing. **Phone color pills** sync live with each paired phone's Material You wallpaper palette. |
| **Pure black (AMOLED)** | **Off** (default) | Shown inline as an **AMOLED** pill when the **Black** color swatch is selected in Bloom/Glass, or as a dedicated row in **Graphite Dark**. Replaces dark gray backgrounds with pure `#000000` black for OLED monitors. |
| **Animated comb** *(Hive)* | **On** (default) | Lets Hive's honeycomb breathe and pulse. It also holds still while animations are off in Windows, while energy saver is on, and while the window isn't in front. Shown in place of **Mica backdrop**, which Hive doesn't use. |
| **Mica backdrop** | **On** (default) | Uses Windows 11's native Desktop Window Manager (DWM) Mica material to subtly tint the window background with your desktop wallpaper. |

---

### 2. General

Controls startup behavior, the system tray, `Nectar Island`, battery notifications, and clipboard automation.

| Setting | Default | What It Does |
| :--- | :--- | :--- |
| **Start with Windows** | **On** for installed copies | Launches NectarLink quietly in the Windows system tray when you sign in to your PC. |
| **Stay in system tray** | **On** | Keeps NectarLink running in the system tray (unloading the UI window to ~5 to 10 MB working set) when you close the main window, so your phone stays connected. When turned off, closing the window quits the app completely. |
| **Nectar Island** | **Off** | Shows a compact pill at the top center of your primary monitor during active phone events. Automatically hides during full-screen games/videos or Windows Do Not Disturb. |
| **Island triggers** *(shown when Nectar Island is On)* | **Calls**: On, **Transfers**: On, **Media**: On, **Alerts**: Off | Toggles which phone events expand `Nectar Island`: incoming/active **Calls**, active file **Transfers**, now-playing **Media**, and phone notification **Alerts** (including OTPs, timers, and recordings). |
| **Battery alerts** | **On** | Shows a desktop notification when a connected phone's battery drops low or reaches a full charge. |
| **Sync clipboard to phone** | **On** | Automatically sends text and images copied on this PC (`Ctrl+C`) to connected phones. |
| **Clipboard follows control** | **On** | Sends the PC clipboard to the phone when you start controlling it: the mouse moves into its mirror window, or a remote control session starts. |
| **Copy one-time codes** | **Off** | Automatically copies 4-to-8-digit verification codes detected in incoming phone notifications and SMS messages onto your PC clipboard. |
| **Copy received files** | **On** | Automatically places small incoming files received from your phone onto the Windows clipboard (`CF_HDROP`) so you can press `Ctrl+V` to paste them anywhere. |
| **Suggest clipboard actions** | **On** | Offers one-click context chips (**Open**, **Open in Maps**, **Call**, **Track**, **Email**) when a web link, street address, phone number, parcel tracking number, or email address arrives from your phone's clipboard. |

---

### 3. Devices

Lists your paired phones (with a device selector pill bar at the top when more than one phone is paired), a **Pair a new device** button, an **Unpair** button, and 18 per-device feature permission toggles grouped into three sections.

<p align="center">
  <img src="images/settings-devices.png" alt="Per-device feature permission toggles in Settings › Devices on Windows" width="760">
</p>

#### Group A: Calls, messages & notifications

| Per-Device Toggle | Default | What It Controls for the Selected Phone |
| :--- | :--- | :--- |
| **Notifications** (`notifications`) | **On** | Receiving phone notifications on the PC, inline replies, action buttons, and optional PC-to-phone notification forwarding. |
| **Messages** (`messages`) | **On** | Syncing SMS/MMS conversations and Unified Conversations, and sending text or MMS replies from the PC. |
| **Calls** (`calls`) | **On** | Incoming call banners, caller ID, recent call history, dialing from the PC, and in-call controls. |
| **Contacts** (`contacts`) | **On** | Syncing phone contacts and avatars for Calls and Messages. |

#### Group B: Files, photos & clipboard

| Per-Device Toggle | Default | What It Controls for the Selected Phone |
| :--- | :--- | :--- |
| **Clipboard** (`clipboard`) | **On** | Sharing copied text, HTML, images, and files between this PC and the selected phone. |
| **Files** (`files`) | **On** | Sending and receiving files, folders, and `Handoff` links/documents. |
| **Photos** (`photos`) | **On** | Browsing the phone's camera roll, screenshots, and albums in the **Photos** page and receiving new photo alerts. |
| **Voice recordings** (`recordings`) | **On** | Receiving voice notes recorded on the phone. |
| **File Explorer integration** (`storage`) | **Off** | Mounting the phone's storage under `%USERPROFILE%\Nectarlink\<Phone name>` via Windows Cloud Files (includes an **Open** button when enabled), and allowing the phone to browse shared PC folders when **Your PC in your pocket** is also turned on. |
| **Remote PC file access** (`remote_files`) | **Off** | Allowing storage browsing when the phone and PC are connected over mobile data / relay (**Connect over mobile data**) rather than your local Wi-Fi network. |

#### Group C: Screen, camera & control

| Per-Device Toggle | Default | What It Controls for the Selected Phone |
| :--- | :--- | :--- |
| **Media controls** (`media`) | **On** | Showing now-playing media and transport controls in both directions, and streaming PC audio via **Phone as speaker**. |
| **Phone controls** (`toggles`) | **On** | Viewing and changing the phone's volume, ringer mode, Do Not Disturb, flashlight, brightness, Wi-Fi, and Bluetooth from PC Home. |
| **Screen mirroring** (`mirroring`) | **On** | Mirroring the phone's screen and opening phone apps in windows (`app window`) on this PC. |
| **Share PC screen to phone** (`pc_mirror`) | **Off** | Allowing this phone to view this PC's screen (reverse mirroring) without showing a one-time consent dialog on every session. |
| **Webcam** (`webcam`) | **On** | Using this phone's camera as a webcam and triggering Continuity Camera photo/document captures. |
| **Lock, sleep & wake PC** (`pc_actions`) | **On** | Allowing this phone to lock or sleep this PC and receive its MAC addresses for Wake-on-LAN. |
| **Remote mouse, keyboard & Deck** (`remote_input`) | **Off** | Allowing this phone to move the PC cursor, type keystrokes, use presentation controls, and press standard `Deck` tiles. |
| **Run Deck shell commands** (`commands`) | **Off** | Allowing this phone to trigger `Deck` tiles that execute custom shell commands on this PC. Only enable for a phone you trust. |

---

### 4. Camera & Audio

Configures **Phone as speaker**, call media pausing, the **NectarLink Webcam** virtual camera add-on, and live camera stream settings.

| Setting | Options / Default | What It Does |
| :--- | :--- | :--- |
| **Stream PC audio to phone** | **Off** (default) | Streams real-time Windows system audio (uncompressed 16-bit stereo) to your connected phone so you can listen through your phone's speakers or headphones. |
| **Pause media during calls** | **On** (default) | Automatically pauses media playing on this PC when your phone rings or is in an active call, and resumes when the call ends. |
| **Virtual camera add-on** | **Set up** / **Remove** | Registers or unregisters `nectarlink_vcam.dll` with Windows 11 Media Foundation (requires a one-time Administrator prompt) so **NectarLink Webcam** appears in Zoom, Teams, Meet, and OBS. |
| **Phone** *(shown when more than one phone is paired)* | First paired phone | Picks which paired phone provides the webcam video feed. |
| **Camera** | `Back` (default), `Front` | Selects the phone's rear or front camera lens. |
| **Mirror image** | **Off** (default) | Flips the webcam video horizontally. |
| **Resolution** | `720p` (default), `1080p`, `4K` | Sets the vertical resolution (`720`, `1080`, or `2160`) of the webcam video stream at 30 fps. |
| **Start webcam / Stop webcam & Preview** | Live 16:9 preview | Starts or stops the webcam stream and displays a live preview directly inside Settings. |

---

### 5. Notifications

Configures Do Not Disturb synchronization, PC-to-phone notification forwarding, Windows Quiet Hours, and per-app notification rules.

| Setting | Options / Default | What It Does |
| :--- | :--- | :--- |
| **Follow phone Do not disturb** | **Off** (default) | Mutes Windows notification popups on this PC whenever your phone is in Do Not Disturb mode (notifications still wait quietly in the NectarLink feed). |
| **Forward PC notifications** | **Off** (default) | Forwards Windows desktop notifications to your connected phone while you are away from your desk. |
| **Quiet hours** | **Set a schedule** button | Opens Windows 11 **System › Notifications** settings to configure automatic Focus / Do Not Disturb schedules. |
| **Per-app notification rules** | `Show` (default), `No pop-ups`, `Hide` | For every phone app that has posted a notification: **Show** displays both Windows popups and feed cards; **No pop-ups** keeps alerts in the NectarLink feed without desktop popups; **Hide** blocks that app's notifications completely. |

---

### 6. Files & Sync

Configures File Explorer integration, PC folder sharing, LocalSend, voice recording output, and automatic folder synchronization.

| Setting | Options / Default | What It Does |
| :--- | :--- | :--- |
| **File Explorer Send to** | **On** (default) | Adds your paired phones to the right-click **Send to** menu in Windows File Explorer. |
| **Your PC in your pocket** | **Off** (default) | Shares your PC's `Downloads`, `Documents`, `Desktop`, `Pictures`, and `NectarLink Shared` folders with paired phones that have **File Explorer integration** enabled. |
| **LocalSend on local network** | **Off** (default) | Enables the built-in LocalSend v2.1 client and server (UDP/TCP port `53317`) to send and receive files with nearby LocalSend devices on your Wi-Fi network without pairing. |
| **Open files shared from the phone** | **Off** (default) | When you share a single photo or document from the phone, opens it in its default Windows app as soon as it arrives. When off, the file is saved to `Downloads\Nectarlink` without opening anything. |
| **Voice recordings** | Format: `M4A` (default), `MP3`, `WAV`, `FLAC`; Folder: **Choose folder** / **Reset** | Sets where incoming voice notes from your phone are saved (`Documents\Nectarlink Recordings` or `Downloads\Nectarlink` by default) and automatically converts them to MP3, WAV, or FLAC if selected. |
| **Synced folders** | **Sync now**, **Add DCIM to Pictures**, per-rule toggle & **Remove** | Configures one-way (**One-way (phone to PC)**) or two-way (**Two-way sync**, keeping conflict copies) automatic folder synchronization rules with BLAKE3 integrity verification. |

---

### 7. Shortcuts & Rules

Configures global Windows keyboard shortcuts and trigger-action automation rules.

| Setting | Default | What It Does |
| :--- | :--- | :--- |
| **Command palette** | `Ctrl+Alt+Space` | Global Windows shortcut to open the `Command Palette` from any app. Click the box and press a new key combination to change it, or click **Reset**. |
| **File shelf** | `Ctrl+Alt+S` | Global Windows shortcut to slide out the `Shelf` edge panel. |
| **Take photo** | `Ctrl+Alt+C` | Global Windows shortcut to trigger Continuity Camera in Photo mode on your phone. |
| **Scan document** | `Ctrl+Alt+D` | Global Windows shortcut to trigger Continuity Camera in Document Scan mode on your phone. |
| **Trigger-action rules** | Empty by default (includes quick buttons for **Lock PC on disconnect** and **Pause media on ring**) | Runs automated actions on your PC (`Lock this PC`, `Sleep this PC`, `Pause PC media`, `Resume PC media`, `Mute PC audio`, `Unmute PC audio`, `Show notification`, `Run command`) when a phone event occurs (`When phone connects`, `When phone disconnects`, `When phone rings`, `When call ends`, `When phone battery is low`, `When phone is charged`). |
| **OBS Studio** | Port `4455`, no password | Where Deck tiles with `obs:` commands (`obs:record`, `obs:stream`, `obs:scene:<name>`) reach OBS's WebSocket server on this PC. Enter the password from OBS's **Tools › WebSocket Server Settings** and click **Save**; it is stored encrypted with Windows DPAPI. **Test connection** checks the port and password. |

<p align="center">
  <img src="images/settings-obs.png" alt="Shortcuts and Rules settings with the OBS Studio card" width="760">
</p>

---

### 8. Storage & Privacy

Displays live item counts and disk usage for every local history store and cache, with individual **Clear** buttons and a master **Clear everything** action.

<p align="center">
  <img src="images/settings-storage.png" alt="Storage and Privacy settings on Windows showing retention periods and Clear buttons" width="760">
</p>

| Setting | Retention / Default | What It Does |
| :--- | :--- | :--- |
| **Clipboard history** | **On** (default) / up to 50 clips / 128 MB / kept 7 days | Toggles the Windows DPAPI encrypted local clipboard history (`clipboard_history.enc` and `clipboard_images/<id>.enc`) and provides a **Clear** button. |
| **Notification history** | **On** (default) / up to 300 alerts / kept 24 hours | Toggles 24-hour retention of dismissed phone notifications (`notification-history.json`) and cached notification images on this PC, with a **Clear** button. |
| **Cross-device timeline** | `30d`, `90d` (default), `1y`, `All` (max 5,000 items) | Sets how long `Timeline` activity entries are kept before auto-purging, with **Open** and **Clear** buttons. |
| **Cached messages** | Kept 90 days | Shows the count and disk size of cached SMS and Unified Conversation threads and MMS attachments, with a **Clear** button. |
| **Photo thumbnails** | Kept 30 days | Shows the count and disk size of cached gallery thumbnails in `%LOCALAPPDATA%\Nectarlink\cache\photos\`, with a **Clear** button. |
| **Transfer history** | Kept with `Timeline` | Shows the number of logged file transfer records and lets you **Clear** the log (never deletes saved files in `Downloads`). |
| **Clear all local history & caches** | **Clear everything...** button | Wipes clipboard history, `Timeline`, notification history, cached messages, photo thumbnails, and transfer history on this PC in one step while keeping your paired devices, settings, and downloaded files intact. |

---

### 9. Connection & About

Provides network diagnostics, **Connect over mobile data**, Wake-on-LAN readiness, the local HTTP automation API, version info, logs, and updates.

<p align="center">
  <img src="images/settings-connection.png" alt="Connection and About settings on Windows" width="760">
</p>

| Setting | Default | What It Does |
| :--- | :--- | :--- |
| **Connection Doctor** | **Check** button | Runs a live check of Windows Firewall rules, network profile type (Public vs Private), active VPN interfaces, and suspected Wi-Fi AP isolation, and fixes firewall or network profile issues with one click. |
| **Connect over mobile data** | **Off** | Allows this PC and paired phones to stay connected over LTE/5G or across separate networks via end-to-end encrypted relay packets when direct LAN/USB is unavailable. |
| **Wake from your phone** | Read-only status indicator (`Ready`, `Off in Windows`, `Check settings`, `No adapter`) | Inspects your active Windows network adapter and tells you whether Wake on LAN is enabled in Device Manager so your phone can wake this PC from sleep. |
| **Local automation API** | **Off** (default port `53320`) | Starts the loopback-only HTTP/JSON server (`127.0.0.1:<port>`) for PowerToys, AutoHotkey, and scripts, and writes the 256-bit Bearer token to `%LOCALAPPDATA%\Nectarlink\api-token.json`. |
| **NectarLink `<version>`** | **Check for updates** and **Open logs** buttons | Shows the app version and this PC's device name, checks GitHub Releases for SHA-256 verified updates, or opens the local diagnostic logs folder. |
| **Check for updates** | **On** (shown on installed copies) | Checks GitHub Releases over HTTPS (`https://api.github.com/repos/alejandrokundrah/nectarlink/releases/latest`) 30 seconds after startup and every 24 hours. |
| **Install updates automatically** | **On** | Installs a newer release by itself when nobody has used the PC for 10 minutes and nothing is in progress (no mirroring, app window, transfer, call, screen sharing or webcam). No administrator prompt: the installer sets up a task that does it. Turn it off to install from the notification or **Check for updates** when you choose. |
| **Setup walkthrough & Features guide** | Action buttons / Command Palette entries | Reopens the first-run setup walkthrough or the interactive in-app Features Guide at any time. |

---

## Android App Settings

On Android, tap **Settings** in the floating bottom dock. You can view all settings at once (**All**) or filter by category chip at the top (**Appearance**, **Devices**, **Permissions**, **Features & Data**).

<p align="center">
  <img src="images/android-settings.png" alt="Settings screen in NectarLink for Android" width="760">
</p>

### 1. Appearance

| Setting | Options / Default | What It Does |
| :--- | :--- | :--- |
| **Theme** | `Hive`, `Graphite` (default), `Bloom`, `Glass` | Selects the visual style on Android, matching the Windows themes. **Hive** draws the same honeycomb behind every screen, with fixed honey colors (no wallpaper or artwork colors). In **Glass** on Android 13 and newer, the floating Home and Settings dock is Liquid Glass: content scrolling beneath it bends at the rim. |
| **Mode** | `System` (default), `Light`, `Dark` | Follows Android's system dark theme or locks the app to Light or Dark. |
| **Animated comb** *(Hive)* | **On** (default) | Lets Hive's honeycomb breathe and pulse. It holds still with **Remove animations** or battery saver on, and while the app isn't in front. |
| **Pure black (AMOLED)** | **Off** (default) | Shown in Graphite Dark or as an inline **AMOLED** pill when the **Black** seed swatch is selected. Uses true `#000000` black surfaces for OLED phone screens. |
| **Media artwork color** *(Bloom and Glass)* | **Off** (default) | Tints the app to match the artwork of what is playing on the phone or a paired PC. Applies only while this is on. With nothing playing, the app uses the wallpaper or preset colors. |
| **Colors from wallpaper** *(Android 12+)* | **On** (default) | Uses your wallpaper's colors and updates when the wallpaper changes. On Samsung phones with One UI's **Color palette** off, NectarLink reads the colors from the wallpaper itself. Paired PCs that select **My phone's colors** use the same seed. |
| **Color swatches** | **Honey**, **Lavender**, **Ocean**, **Sage**, **Black** | Selects a preset accent color when **Media artwork color** and **Colors from wallpaper** are off. |

### 2. PCs (`Devices`)

| Setting | Default | What It Does |
| :--- | :--- | :--- |
| **Paired PC list & Unpair** | Per paired PC | Shows each paired PC's name and live connection status (`Connected` or `Not connected`) with an **Unpair** button. |
| **Browse this phone's storage in File Explorer** | **Off** (per PC) | Allows that specific PC to mount this phone's shared storage (or selected folders) inside Windows File Explorer. |
| **Pair another PC** | Button | Opens the pairing screen to scan a QR code or pair nearby with another PC. |
| **Connect over mobile data** | **Off** | Allows this phone to stay connected to paired PCs over LTE/5G or across different networks when away from Wi-Fi. |

### 3. Permissions

Each permission row shows its live state (**On** / **Allowed** vs **Off**) and a direct button (**Allow** or **Manage**) that opens the exact Android system prompt or settings screen:

| Section & Setting | Power Level / Purpose |
| :--- | :--- |
| **Notifications › Show notifications on your PC** | Grants Android Notification Listener access so phone notifications, inline replies, and actions work on your PC. Includes an **App info** helper on Android 13+ if **Allow restricted settings** is needed. |
| **Control from your PC › Use your PC's mouse and keyboard** | Enables NectarLink's Android Accessibility Service (`Assist` power level) so you can tap, swipe, and type on your phone from your PC during screen mirroring or via **Type on phone**. |
| **Control from your PC › Real touch and keys (Wireless debugging)** | Sets up or manages the `Elevated` power level via Android's built-in Wireless Debugging (real-time touch dragging, UHID keyboard/gamepad, `app window` streaming, background clipboard sync, Android 15+ sensitive OTPs, in-call mute/hold/speaker, and Wi-Fi/Bluetooth toggles). |
| **Control from your PC › Do Not Disturb and silent mode** | Grants Notification Policy Access so your PC can toggle Do Not Disturb and set the ringer to silent. |
| **Control from your PC › Screen brightness** | Grants Modify system settings (`WRITE_SETTINGS`) so your PC can adjust your phone's screen brightness slider. |
| **Background & battery › Stay reachable in the background** | Requests Android's unrestricted battery usage exemption (`REQUEST_IGNORE_BATTERY_OPTIMIZATIONS`) and displays step-by-step instructions tailored to your phone's manufacturer (Samsung, Xiaomi, OnePlus, Oppo/Realme, Vivo, Huawei, Pixel) with a **Battery settings** button. |
| **Background & battery › Wake when PC is nearby (Companion Device)** | Associates your paired PC with Android's `CompanionDeviceManager` so Android wakes NectarLink automatically when your PC's Bluetooth LE beacon is in range. |
| **Calls, contacts and messages › Calls on your PC** | Grants phone state, call log, and call control permissions. |
| **Calls, contacts and messages › Contacts on your PC** | Grants contact read access so names and avatars appear in Calls and Messages on your PC. |
| **Calls, contacts and messages › Text from your PC** | Grants SMS permissions to read and send texts on your PC. |
| **Photos and videos › Photos and videos on your PC** | Grants media access (`Allow all` or `Select photos`) so you can browse your camera roll and get new screenshot alerts on your PC. |
| **Photos and videos › All files access / Selected folders only** | Configures whether File Explorer storage mounting and folder sync can access your whole shared storage (`MANAGE_EXTERNAL_STORAGE`) or only specific folders you pick with Android's folder picker (`Add folder` / `Remove`). |

### 4. Features & Data (and About)

| Section & Setting | Options / Default | What It Does |
| :--- | :--- | :--- |
| **Clipboard › Background clipboard access** | **Allow** / **Manage** | Opens the `Elevated` Wireless Debugging setup so clipboard changes on your phone sync to your PC automatically in the background. |
| **Clipboard › Suggest actions for copied text** | **On** (default) | Shows context chips (**Open**, **Open in Maps**, **Call**, **Track**, **Email**) when text arrives from your PC. |
| **Control your PC › Touchpad speed** | `0.5x` to `2.5x` (`1.0x` default) | Adjusts the pointer sensitivity of the remote touchpad when controlling your PC from your phone. |
| **LocalSend › LocalSend** | **Off** (default) | Enables sending and receiving files with nearby LocalSend devices on your Wi-Fi network (always prompts before saving incoming files). |
| **Data & storage › Notification history** | Informational summary | Explains that notifications are mirrored live to paired PCs and not stored a second time on the phone. |
| **Data & storage › Clipboard history** | **On** (default) + **Clear** button | Toggles the Android Keystore AES-256-GCM encrypted 50-item (7-day / 128 MB) clipboard history on this phone and lets you clear it. |
| **Data & storage › Timeline** | `30d`, `90d` (default), `1y`, `All` + **Clear** button | Sets how long cross-device `Timeline` entries are kept on this phone (up to 5,000 items) and lets you clear them. |
| **Data & storage › Message cache** | **Clear** button | Clears temporary MMS staging files (automatically cleaned up after 5 minutes). |
| **Data & storage › Photo thumbnails & camera cache** | **Clear** button | Clears temporary Continuity Camera captures (automatically cleaned up after 24 hours). |
| **Data & storage › Received-file history** | **Clear** button | Clears the transfer log without deleting any saved files in `Download/Nectarlink`. |
| **Data & storage › Clear everything...** | Confirmation dialog | Deletes all clipboard history, `Timeline`, transfer records, and temporary caches on this phone while keeping your paired PCs and settings. |
| **About › NectarLink `<version>` & Check for updates** | **Check for updates** button | Shows the installed version and checks GitHub Releases (`https://api.github.com/repos/alejandrokundrah/nectarlink/releases/latest`) for SHA-256 verified APK updates. |
| **About › Install updates automatically** | **On** | Checks once a day in the background and installs a newer release by itself. Android 12 and later install it without asking once NectarLink has installed an update before; otherwise a notification asks for one tap. Needs **Install unknown apps** allowed for NectarLink. |

<p align="center">
  <img src="images/android-update.png" alt="About in the Android app with Install updates automatically" width="360">
</p>
