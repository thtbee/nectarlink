# Mirroring and App Windows

NectarLink streams your phone's screen to your PC in a dedicated window at up to 60 frames per second with hardware video encoding (H.264, H.265, or AV1) and live audio forwarding. Depending on your Android power level, you can view your screen, control it with your PC mouse and keyboard, open individual Android apps in their own desktop windows (`app window`), launch a full desktop layout (DeX mode), or reverse-mirror your PC's screen to your phone.

<p align="center">
  <img src="images/mirroring-window.png" alt="Phone screen mirrored in a window on Windows 11" width="760">
</p>

## What You Need

| Feature | Requirements |
| :--- | :--- |
| **View phone screen and audio on PC (`mirror`)** | `Standard` power level. **Screen mirroring** toggle enabled in PC **Settings › Devices** (on by default). At `Standard` or `Assist`, Android displays a system **Start recording or casting** prompt on your phone each time mirroring starts. With `Elevated`, NectarLink grants `PROJECT_MEDIA` automatically so mirroring starts without a phone prompt. |
| **Tap, swipe, scroll, and type from PC** | `Assist` (Android Accessibility Service enabled in **Settings › Permissions › Control from your PC**) or `Elevated` (Wireless Debugging). `Elevated` adds real-time touch dragging, XInput gamepad passthrough, and stylus pressure/tilt. |
| **Turn off physical phone screen while mirroring** | `Elevated` power level. |
| **Phone apps in individual PC windows (`app window`)** | `Elevated` power level on Android 11 or newer. |
| **Reverse mirroring (PC screen on phone)** | Per-device **Share PC screen to phone** (`pc_mirror`) toggle in PC **Settings › Devices** (off by default), or one-time approval on the PC's consent dialog when requested from the phone. Controlling the PC from the reverse-mirror view also requires **Remote mouse, keyboard & Deck** (`remote_input`, off by default). |

## Step-by-Step Use

### Mirroring Your Phone Screen to Your PC

1. On your PC, click **Mirror screen** on the **Home** screen (or press `Ctrl+Alt+Space` and choose **Mirror phone screen**). You can also start mirroring from your phone by tapping **Mirror to PC** on the Android Home screen.
2. If your phone is at `Standard` or `Assist`, tap the notification on your phone and approve Android's screen-sharing dialog. If your phone is at `Elevated`, the `mirror` window opens immediately on your PC without a prompt on the phone.
3. Your phone's screen streams at up to 1920 pixels on the long edge and 60 fps. NectarLink automatically negotiates the best hardware codec supported by both devices (**AV1**, **H.265/HEVC**, or **H.264/AVC**) and adapts when you rotate your phone between portrait and landscape.
4. On Android 10 and newer, your phone's media and game audio plays through your PC's speakers automatically during mirroring. Click the speaker icon in the `mirror` window toolbar at any time to mute or unmute the phone's audio on your PC.

### Controlling Your Phone from the Mirror Window

Once `Assist` or `Elevated` is enabled on your phone:

- **Left-click and drag**: Taps or swipes the screen. At `Elevated`, pointer movement streams continuously in real time so you can drag icons, scrub video timelines, or draw without stutter.
- **Mouse wheel / two-finger trackpad scroll**: Scrolls vertically or horizontally under the cursor.
- **Right-click**: Triggers Android **Back**.
- **Middle-click**: Triggers Android **Home**.
- **Keyboard typing and shortcuts**: Type directly into any focused Android field. At `Elevated` (when `/dev/uhid` is permitted by your phone's kernel/SELinux policy), NectarLink registers a virtual gamepad so connected Xbox/XInput controllers work natively inside Android apps and games.
- **Clipboard and file drop inside the mirror**: Pressing `Ctrl+C` or `Ctrl+V` while the `mirror` window is focused syncs your clipboard, and dragging files from Windows File Explorer onto the `mirror` window sends them straight to `Download/Nectarlink` on your phone.
- **Stylus / Pen**: Windows Ink pen pressure, tilt, and barrel buttons are forwarded to the phone screen at `Elevated`.

On Xiaomi, Redmi and POCO phones, `Elevated` control also needs **USB debugging (Security settings)** turned on in **Developer options**.

### Mirror Toolbar and Keyboard Shortcuts

The `mirror` window header includes quick controls and keyboard shortcuts:

| Action | Shortcut | What It Does |
| :--- | :--- | :--- |
| **Copy screenshot** | `Ctrl+Shift+C` | Captures the current mirrored frame directly onto the Windows clipboard as a PNG image. |
| **Save screenshot** | `Ctrl+S` | Saves a PNG screenshot of the mirrored screen to `Downloads\Nectarlink`. |
| **Record video (MP4)** | `Ctrl+R` | Records the live mirror stream and phone audio to an `.mp4` file in `Downloads\Nectarlink` using lossless H.264 stream passthrough (zero video re-encoding overhead on your PC). Press `Ctrl+R` again to stop and save. |
| **Keep phone awake** | Toolbar icon | Prevents your phone's screen timeout from locking the device while mirroring is active. |
| **Turn phone screen off** | Toolbar icon (`Elevated`) | Blanks your phone's physical OLED/LCD display while the `mirror` window continues running on your PC, and turns the physical display back on automatically when mirroring stops. |
| **Lock phone** | Toolbar icon | Locks the phone, as its power button would. Shows when you can control the phone. |
| **Play the sound on the phone / here** | Toolbar icon | Stops sending the phone's sound to the PC so the phone plays it again, or brings it back to the PC. While it's on the phone, new mirroring leaves it there. |

### Opening Phone Apps in Their Own PC Windows (`app window`)

With `Elevated` on Android 11 or newer, you can run individual phone apps in separate Windows 11 windows without mirroring your whole phone screen:

<p align="center">
  <img src="images/app-window.png" alt="An Android app running in its own independent window on Windows 11" width="760">
</p>

1. Click **Apps** in the PC sidebar to see every app on your phone. You can also click **Open an app** on **Home**, or any phone notification in the feed, Notification history, or a Windows notification popup.
2. **Apps** shows a **Recent** row (apps you opened from the PC and apps that recently posted notifications) above your phone's full app list. Type to search, and press Enter to open the first match. Apps already open on the PC are marked with a dot.

<p align="center">
  <img src="images/apps-page.png" alt="The Apps page on the PC listing the phone's apps" width="760">
</p>

3. Click any app. NectarLink creates an isolated virtual display on your phone and opens that app on your PC in its own `app window` with its own Windows taskbar icon.
4. The app runs independently beside whatever is shown on your phone's physical screen, and typing from your PC does not pop up the on-screen soft keyboard on your phone.
5. Resize the `app window` on your PC and the Android app re-lays out to fill the window's new shape when you let go of the edge; while you hold it, the window shows the app blurred under its icon. Hold `Ctrl` and turn the mouse wheel to make the app draw bigger or smaller (50% to 200%). For an app that doesn't take odd shapes well, turn off **Flex mode** in the window's toolbar: the app restarts at your phone's resolution and the window keeps that shape. Flex mode and zoom are remembered for each app. NectarLink remembers each app's window size and screen position for the next time you open it. You can open multiple `app window` instances side by side in tabs or separate windows.
6. The phone's sound comes to your PC with its app windows, and the phone itself goes quiet. It's the phone's whole sound, so with several app windows open you hear all of them. The speaker icon in any `app window` toolbar mutes it on the PC, and the sound stops when you close the last `app window`.

### Desktop Display Mode (DeX / Desktop Mode)

On phones that support a desktop launcher layout on secondary displays (such as Samsung DeX or Android 14+ Desktop Mode), you can launch a 16:9 desktop workspace in a window on your PC from the mirroring menu (`Elevated`).

### Reverse Mirroring: Viewing Your PC Screen on Your Phone

You can also view (and optionally control) your PC's desktop from your phone:

1. On your phone, open **NectarLink** and tap **View PC screen** under **Screen, Camera & Input** (or open the **Control PC** screen and switch to the screen view).
2. **Explicit PC Consent**: Because a PC screen is sensitive, reverse mirroring is **off by default** (**Share PC screen to phone** in **Settings › Devices**). If it is not already enabled for your phone, your PC displays a modal consent prompt asking whether to **Not now**, **Allow** (for this session only, cleared automatically on disconnect), or **Allow for this device** (enables **Share PC screen to phone** for that phone).
3. **Sharing indicator**: Throughout any active reverse-mirroring session, the PC window's title bar shows a live pill (**Sharing screen with `<Phone>`**) with a **Stop** button.
4. **Input Gating**: Viewing the PC screen from your phone is read-only unless you have also enabled **Remote mouse, keyboard & Deck** (`remote_input`) for that phone on your PC.
5. **Primary display**: The phone sees the PC's primary display. On a PC with several monitors, the others are not shared. To share a different monitor, make it the primary display in Windows **Settings › System › Display**.
6. **Slow connections**: On a slow link (mobile data through the relay), the PC lowers the video bitrate until the picture flows smoothly, and raises it again once the link recovers.

## What the Options Do

- **Mirror quality**: On a direct connection, **Smoother** lowers both screen mirroring and app windows to at most 1280 pixels and 4 Mbps. Reopen an app window after changing it. This reduces encoding, network and decoding work; it can help responsiveness when several apps are open.
- **Screen mirroring** (`Settings › Devices` on PC, default: **On**): Allows your PC and phone to start phone-to-PC screen mirroring and `app window` sessions.
- **Share PC screen to phone** (`Settings › Devices` on PC, default: **Off**): Allows the selected phone to view this PC's screen without showing the one-time consent dialog each time.
- **Remote mouse, keyboard & Deck** (`Settings › Devices` on PC, default: **Off**): Allows the selected phone to send touch/mouse clicks and keystrokes to this PC (including during reverse mirroring).
- **Use your PC's mouse and keyboard** (`Settings › Permissions › Control from your PC` on Android): Opens Android Accessibility settings to enable or disable `Assist` mode.
- **Elevated** (`Settings › Permissions › Control from your PC` on Android): Guides you through setting up or turning off `Elevated` mode.

## Data and Privacy

- **Live streaming only**: Video frames and audio samples are encoded in hardware, transmitted directly over your end-to-end encrypted QUIC connection, and decoded into GPU textures in memory. Nothing is recorded to disk unless you explicitly press **Save screenshot** (`Ctrl+S`) or **Record video** (`Ctrl+R`).
- **Session log in `Timeline`**: When a mirroring session ends, `Timeline` records a single summary entry noting the device name and session duration (without storing any video or screenshots).
- **Consent and visibility**: Phone-to-PC mirroring shows a persistent **Sharing your screen with `<PC>`** notification on your phone with a **Stop** button (and an **app is open in a window on a PC** notification with **Close all** when using `app window` streaming). PC-to-phone reverse mirroring requires PC consent and shows a persistent **Stop sharing** banner on the PC.

## Limits

- **`FLAG_SECURE` Protected Content**: Android apps that set the `FLAG_SECURE` window flag (such as banking apps, password managers, and DRM-protected video streaming apps like Netflix) render black inside `Standard` and `Assist` `MediaProjection` streams by Android OS design.
- **Audio Capture Rules and `REMOTE_SUBMIX`**: Standard Android 10+ `AudioPlaybackCapture` captures apps that allow playback capture (`USAGE_MEDIA`, `USAGE_GAME`). Apps that opt out of playback capture in their manifest cannot be captured at `Standard` or `Assist`. At `Elevated`, NectarLink attempts internal audio capture via `REMOTE_SUBMIX` (`UID 2000`), which works on AOSP/Pixel builds but may be blocked by custom SELinux policies on some OEM ROMs (falling back automatically to standard playback capture).
- **UHID Kernel/SELinux Access**: The virtual gamepad at `Elevated` opens `/dev/uhid` as `shell` (`UID 2000`). On OEM ROMs whose SELinux policy denies `shell` access to `/dev/uhid`, NectarLink falls back automatically to standard Android `InputManager` key and motion injection.
