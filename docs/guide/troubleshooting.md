# Troubleshooting

If your phone and PC cannot find each other, disconnect when your phone screen turns off, or have trouble with a specific feature, follow the steps on this page.

## 1. Run the `Connection Doctor` First

NectarLink includes a built-in diagnostic tool on Windows that automatically detects the most common network blockers and fixes what it can.

1. On your PC, open **Settings › Connection & About**.
2. Next to **Connection Doctor**, click **Check**.
3. The `Connection Doctor` inspects four things and shows plain-language results with action buttons:
   - **Windows or Linux Firewall**: Checks whether `nectarlink-desktop` is allowed to receive incoming UDP (QUIC) and mDNS packets (Windows Firewall on Windows, or `ufw` / `firewalld` on Linux). If a firewall rule is missing or blocked, click **Fix** to register the rule (requires Administrator / Polkit approval).
   - **Network Profile and Local Discovery (`Public` vs. `Private` on Windows, `avahi-daemon` on Linux)**: Windows 11 often marks home Wi-Fi networks as **Public** by default, which blocks local device discovery and incoming connections. If your home Wi-Fi is set to **Public**, switch it to **Private** in **Windows Settings › Network & internet › Wi-Fi › `<Your Network>` properties › Network profile type**. On Linux, Connection Doctor also checks that `avahi-daemon` (mDNS), `fusermount3` (phone storage mount), and PipeWire / PulseAudio are available.
   - **Active VPN Interfaces**: Full-tunnel corporate or privacy VPNs often route local subnet traffic (`192.168.x.x` / `10.x.x.x`) into the VPN tunnel so your PC and phone cannot reach each other directly. Enable **Split tunneling** (or "Allow LAN traffic") in your VPN client's settings, or connect your phone over a USB cable (`On USB`), or turn on **Connect over mobile data** on both devices.
   - **Suspected Wi-Fi AP Isolation (Guest / Campus Wi-Fi)**: If your PC can hear your phone's multicast mDNS broadcasts (seen within the last 2 minutes) but every direct QUIC connection attempt times out, your Wi-Fi router has **AP Isolation** (Client Isolation) turned on. AP Isolation is common on hotel, campus, cafe, and "Guest" Wi-Fi networks. To connect on an isolated Wi-Fi network, either switch both devices to your main home Wi-Fi SSID, plug in a USB cable, or turn on **Connect over mobile data** on both devices.

## 2. Phone Disconnects When Its Screen Is Off (Per-OEM Battery Guides)

If NectarLink stays connected while the Android app is open on screen, but drops its connection a few minutes after you lock your phone (causing inline replies, notifications, or file transfers from your PC to wait or time out), Android or your phone manufacturer's battery manager is putting NectarLink to sleep.

### Step 1: Allow Standard Android Background Access

1. On your phone, open **NectarLink › Settings › Permissions › Background & battery**.
2. Next to **Stay reachable in the background**, tap **Allow** and confirm Android's prompt to let NectarLink run without battery restrictions (`Unrestricted`).

### Step 2: Apply Your Phone Brand's Extra Battery Setting

Several Android manufacturers add their own proprietary background task killers on top of standard Android. NectarLink detects your phone's manufacturer and shows the matching guide right inside **Settings › Permissions › Background & battery** (tap **Battery settings** there to jump directly to the system screen):

| Phone Brand | Required Settings |
| :--- | :--- |
| **Samsung (One UI)** | 1. Open **Android Settings › Apps › NectarLink › Battery** and select **Unrestricted**.<br>2. Open **Android Settings › Device care › Battery › Background usage limits** and make sure NectarLink is not in **Sleeping apps** or **Deep sleeping apps** (add it to **Never auto sleeping apps**). |
| **Google Pixel** | Open **Android Settings › Apps › NectarLink › App battery usage**, tap **Allow background usage** (the row text itself, not just the toggle), and select **Unrestricted**. |
| **Xiaomi / Redmi / POCO (HyperOS / MIUI)** | 1. Open **Android Settings › Apps › Manage apps › NectarLink** and turn **Autostart** (or **Background autostart**) **On**.<br>2. Tap **Battery saver** and select **No restrictions**. |
| **OnePlus (OxygenOS)** | Open **Android Settings › Apps › App management › NectarLink › Battery usage** and turn on both **Allow background activity** and **Allow auto-launch**. |
| **Oppo / Realme (ColorOS / Realme UI)** | Open **Android Settings › Apps › App management › NectarLink › Battery usage** and turn on both **Allow background activity** and **Allow auto-launch**. |
| **Vivo / iQOO (Funtouch OS / OriginOS)** | Open **Android Settings › Apps › NectarLink › Battery**, select **Unrestricted** (or **High background power usage**), and enable **Autostart** in Background power consumption management. |
| **Huawei / Honor (EMUI)** | Open **Phone Manager (or System Optimizer) › App launch**, turn **Off** **Manage automatically** for NectarLink, and keep all three toggles (**Auto-launch**, **Secondary launch**, and **Run in background**) turned **On**. |

## 3. Android Sideloading: Play Protect Blocks and Android 13+ "Restricted Settings"

### Google Play Protect Blocks the APK Without an "Install anyway" Button

When you download `Nectarlink-<version>-android.apk` in a mobile browser, Google Play Protect may show a prompt:

- **If "More details" is shown**: Tap **More details › Install anyway**.
- **If there is no "Install anyway" button (only "OK")**: Google Play Protect's **Enhanced Fraud Protection**, on in 185 markets including India, automatically blocks browser-sideloaded APKs that declare `READ_SMS`, `BIND_NOTIFICATION_LISTENER_SERVICE`, or `BIND_ACCESSIBILITY_SERVICE` in `AndroidManifest.xml`. To install:
  1. **Option A (On phone)**: Open **Google Play Store**, tap your **profile icon** (top right) › **Play Protect** › **Settings** (gear icon), and temporarily toggle off **Scan apps with Play Protect** (choose **Pause**). Install `Nectarlink-<version>-android.apk`, then turn **Scan apps with Play Protect** back on.
  2. **Option B (Over USB from PC)**: Connect your phone with USB debugging enabled and run `adb install -r Nectarlink-<version>-android.apk`.

### Android 13+ "Restricted Settings" (Grayed-Out Notification or Accessibility Switch)

On Android 13 and newer, if you install an APK downloaded from a browser or GitHub Releases, Android disables the **Notification access** and **Accessibility** switches until you explicitly allow restricted settings for that app:

1. Tap **Allow** on the **Show notifications on your PC** or **Use your PC's mouse and keyboard** row in NectarLink so Android sees that you tried to turn it on once.
2. Return to NectarLink's **Settings › Permissions** and tap **App info** (or open **Android Settings › Apps › NectarLink**).
3. Tap the **three-dot menu** in the top-right corner of the App info screen and tap **Allow restricted settings**.
4. Authenticate with your fingerprint, face, or PIN.
5. Go back to NectarLink and tap **Allow** again. The switch can now be turned on normally.

## 4. Troubleshooting `Elevated` (Wireless Debugging)

### The 6-Digit Pairing Code Notification Does Not Appear or Fails

- **Keep the code dialog open on screen**: Android invalidates the 6-digit Wireless Debugging pairing code the moment you navigate away from the **Pair device with pairing code** dialog. Keep that dialog visible on screen, pull down your notification shade over it, tap **Enter code** on NectarLink's notification, and type the 6 digits there.
- **Make sure Wi-Fi is connected**: Android's built-in Wireless Debugging only runs while connected to a Wi-Fi network.
- **If the phone says "The phone forgot NectarLink's pairing"**: If you tapped **Forget all** under Paired devices in Android's Wireless Debugging settings, open NectarLink's **Settings › Permissions › Control from your PC** and tap **Set up** to pair Wireless Debugging again.

### `Elevated` Stops After Turning Developer Options Off

- If you turn the master **Developer options** switch off in Android Settings, Android stops Wireless Debugging and terminates the `Elevated` shell helper. Keep **Developer options** turned on if you use `Elevated` features. (When you reboot your phone or reconnect to Wi-Fi with Developer options on, NectarLink automatically re-enables Wireless Debugging and restarts `Elevated` by itself.)

### A Banking App Won't Open

Many banking apps refuse to run while **Developer options**, USB or Wireless Debugging, or an accessibility service is on, all of which `Elevated` and Assist use. NectarLink can switch them off while those apps are open:

1. Open **Settings › Permissions › Control from your PC › Pause for banking apps** and tap **Choose apps**.
2. Pick your banking and payment apps. NectarLink needs `Elevated` set up once, and usage access (tap **Allow** if it asks), to see which app is open.
3. Open a banking app as usual. NectarLink turns off Developer options, debugging and its accessibility service while it's open, and turns back on exactly what was on when you leave. A notification shows while it's paused, with **Turn back on**.

If an app checks before NectarLink notices it, tap **Add shortcut** next to it and open it from that home screen shortcut: NectarLink pauses first, then opens the app.

Some banking apps also refuse to run on a phone that was ever rooted (on Samsung phones, a tripped Knox flag). NectarLink can't change that.

## 5. Troubleshooting USB Auto-Pair and `On USB` Transport

If plugging your phone into your PC with a USB cable does not trigger USB auto-pair or switch your connection to `On USB`:

1. **Check the USB cable**: Use a USB cable that carries data (some charging-only cables lack data wires).
2. **Enable USB debugging on the phone**: Open **Android Settings › Developer options** and turn on **USB debugging**.
3. **Approve the RSA key prompt on the phone**: Wake your phone screen when plugging it into the PC, check **Always allow from this computer**, and tap **Allow** on Android's **Allow USB debugging?** dialog.
4. **Check adb in the Connection Doctor**: The PC reaches a phone on a cable through Android's `adb`. The Doctor says whether it's running, with a **Start adb** button when it isn't, and whether it's missing. NectarLink finds `adb` from Android Studio's SDK folder, `ANDROID_HOME`, `ANDROID_SDK_ROOT` or your `PATH`; if there's none, install Google's platform-tools.

## 6. Troubleshooting Screen Mirroring and Audio

### Mirroring Is View-Only (Mouse Clicks Do Nothing)

- At the `Standard` power level, Android does not allow apps to inject touch or key events into other apps. To control your phone with your PC mouse and keyboard during mirroring, enable either **`Assist`** (**Settings › Permissions › Control from your PC › Use your PC's mouse and keyboard**) or **`Elevated`** (the **Elevated** card there) on your phone.
- **On Xiaomi, Redmi and POCO phones with `Elevated`:** if the picture works but clicks and typing do nothing, and no banner asks you to turn on control, open **Developer options** and turn on **USB debugging (Security settings)**. The phone blocks simulated input without it.

### Mirror Window Shows a Black Screen

- If the whole mirror window stays black only while a specific app (such as a banking app, password manager, or DRM video player) is open on your phone, that app sets Android's `FLAG_SECURE` window flag to block screen capture. Switching back to your phone's home screen restores the image immediately.

### No Sound on PC During Screen Mirroring

- Make sure your phone is running **Android 10 or newer** (earlier Android versions do not support internal audio capture).
- Check that the speaker icon in the `mirror` window toolbar on your PC is not muted, and that your phone's media volume is turned up.
- Note that some Android apps explicitly block third-party `AudioPlaybackCapture` in their manifest (see [Known Limitations](known-limitations.md)).

## 7. Troubleshooting Phone as Webcam (`NectarLink Webcam`)

### `NectarLink Webcam` Does Not Appear in Zoom, Teams, Meet, or OBS

1. Open NectarLink on your PC and go to **Settings › Camera & Audio**.
2. Check the **Virtual camera add-on** row. If it shows **Set up** instead of **Ready**, click **Set up** and approve the Windows Administrator prompt.
3. Restart Zoom, Teams, your web browser, or OBS so the application refreshes its list of Windows Media Foundation video capture devices.

### Clicking "Start Webcam" on the PC Waits on "Asking..."

- By design, a PC cannot turn on your phone's camera silently. Look at your phone and tap the **`<PC>` wants to use your camera** notification (or open NectarLink on your phone and tap **Webcam › Start webcam**).

## 8. Inspecting Diagnostic Logs

If you need to report a bug, open **Settings › Connection & About** on your PC and click **Open logs** (next to the NectarLink version row) to open the local log directory in File Explorer. NectarLink never logs notification text, SMS message bodies, or clipboard contents to its log files.
