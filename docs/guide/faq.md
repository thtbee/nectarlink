# Frequently Asked Questions (FAQ)

## General and Accounts

### Do I need to create an account or sign in to use NectarLink?

No. NectarLink has no user accounts, no sign-in screens, and no cloud servers. Each device generates its own cryptographic Ed25519 identity key locally on first launch, and you link your devices directly by scanning a QR code or comparing a 6-digit code.

### Is NectarLink free and open source?

Yes. The Windows and Android applications and the CLI are licensed under [GPL-3.0-or-later](../../LICENSES/GPL-3.0-or-later.txt) (with an app-store distribution exception), the shared Rust core crates are licensed under [MPL-2.0](../../LICENSES/MPL-2.0.txt), and the protocol specifications and documentation are licensed under [CC BY 4.0](../../LICENSES/CC-BY-4.0.txt). There are no paid tiers, subscriptions, or advertisements.

### Does NectarLink only work with Samsung or Pixel phones?

No. NectarLink works with any Android phone or tablet running Android 8.0 (API 26) or newer, regardless of manufacturer (Samsung, Google Pixel, Xiaomi, OnePlus, Motorola, Sony, Nothing, Asus, Oppo, Realme, Vivo, Huawei, and others).

## Networking and Connectivity

### Do my phone and PC have to be on the same Wi-Fi network?

By default, yes: NectarLink connects directly over your local network (Wi-Fi or Ethernet) or over a USB data cable (`On USB`). If you want your phone and PC to stay connected when you leave home on mobile data (LTE/5G) or use a separate Wi-Fi network, turn on **Connect over mobile data** in **Settings › Connection & About** on your PC and **Settings › Devices** on your phone.

### Does my data pass through a server when I use **Connect over mobile data**?

When **Connect over mobile data** is turned on and a direct peer-to-peer UDP path cannot be established between your networks, encrypted QUIC packets are forwarded through an `iroh` relay. Because the TLS 1.3 session is encrypted end to end between your phone's key and your PC's key, the relay only forwards opaque encrypted packets and can never read your notifications, messages, clipboard, or files. When your devices are on the same local network or connected by USB, all device-to-device traffic goes straight between them without leaving your local network (the only internet request NectarLink makes by default on installed copies is a periodic HTTPS check to GitHub Releases for app updates, which can be turned off on the PC in **Settings › Connection & About › Automatic updates**).

### Can I use NectarLink without Wi-Fi at all, using only a USB cable?

Yes. USB needs Android's `adb` on the PC: install Google's platform-tools so `adb` is on your `PATH`. When NectarLink starts and no adb server is running, it starts one, so a restart needs nothing from you. Then enable **USB debugging** in your phone's Developer options and plug your phone into your PC with a USB data cable. NectarLink automatically detects the USB connection, lets you pair with a 6-digit confirmation code, and runs your entire encrypted session (`On USB`) over the cable.

### What happens if I send a file or copy a link while my other device is asleep or offline?

It is not lost. NectarLink places files, folders, clipboard sends, `Handoff` links, and `notify-when` task alerts into a persistent `Waiting to send` queue stored in SQLite. You can see and cancel queued items on the Home screen of either app, and they are delivered automatically in order as soon as the other device reconnects.

### Can I pair multiple phones with one PC, or one phone with multiple PCs?

Yes. You can pair two or more phones (for example, a personal phone and a work phone, or a phone and an Android tablet) with the same PC, and you can pair your phone with both a desktop PC and a laptop. Connected devices appear first in the device switcher on Home, and **Settings › Devices** lets you configure feature permissions independently for each paired device.

## Power Levels and Permissions

### Do I need to root my phone to use NectarLink?

No. Every everyday feature works at the default `Standard` power level using normal Android permissions, and advanced features (`Elevated`) use Android's own Wireless Debugging, which runs a small helper as the shell user (`UID 2000`).

If your phone is already rooted (Magisk, KernelSU or APatch), you can choose **Use root** when setting up Elevated instead. NectarLink then starts the same helper through root, still as the shell user with the same permissions, so it needs no pairing and comes back by itself after a restart or away from Wi-Fi. NectarLink never asks for root unless you choose this.

If you already use [Shizuku](https://shizuku.rikka.app/), you can choose **Use Shizuku** instead (it shows when Shizuku's app is installed). Shizuku asks once whether NectarLink may use it; after that Elevated starts through Shizuku whenever it runs, with no pairing of its own, and keeps working away from Wi-Fi. The helper is the same and runs as the shell user. If Shizuku runs with root, NectarLink doesn't use it: start Shizuku through Wireless debugging, or choose **Use root**. After a restart, start Shizuku again, then open NectarLink once: Shizuku only tells apps it's running when they come to the front, and Elevated starts as soon as it hears.

### Why does Android 13+ gray out the Notification access or Accessibility switch ("Restricted settings")?

Android 13 and newer block sideloaded APKs (apps installed outside an app store) from turning on Notification Listener or Accessibility services until you confirm in **App info**. Open **Android Settings › Apps › NectarLink**, tap the three-dot menu in the top-right corner, tap **Allow restricted settings**, verify with your fingerprint or PIN, and then return to NectarLink to turn the switch on.

### Do I have to re-pair Wireless Debugging (`Elevated`) every time I reboot my phone?

No. You only enter the 6-digit Wireless Debugging pairing code once during initial setup. After that, whenever your phone reboots or reconnects to Wi-Fi, NectarLink automatically turns Wireless Debugging back on and starts the `Elevated` helper in the background by itself.

### Can I use NectarLink if I do not want to grant SMS, Call, or Photo permissions?

Yes. Every Android permission is optional, and NectarLink's capability matrix adapts the user interface to show only what you have enabled. For example, if you only want clipboard sync, file transfers, and screen mirroring, you can skip SMS, Calls, Contacts, and Photos permissions completely.

## Battery, Memory, and Performance

### How much battery does NectarLink use on my phone?

When connected and idle in the background, NectarLink maintains a single lightweight QUIC connection with event-driven wakeups and zero polling loops, using less than 1 percent battery per hour in continuous connected soak tests. High-power features such as screen mirroring (`mirror`) and camera streaming (**Phone as webcam**) use hardware video encoders only while actively streaming and stop immediately when you close the window.

### Does NectarLink work on Linux or Windows 7?

Linux, yes, from 0.1.5: Ubuntu 24.04 or newer, Debian 13, Fedora 40 or newer and other distributions from 2024 on. See [Getting Started](getting-started.md) and what's different in [Known Limitations](known-limitations.md#on-linux). Windows needs Windows 10 version 2004 or newer, so Windows 7 and 8 aren't supported.

### How much RAM and CPU does the Windows app use in the background?

When started minimized in the Windows system tray, NectarLink uses approximately **27 MB** of private commit memory (~5 to 10 MB active working set) and **0.0 percent** idle CPU. When you close the main window (`Stay in system tray`), NectarLink unloads the QML window tree and image caches to keep background memory usage minimal while staying ready to pop up notifications or open the `Shelf` and `Command Palette` in under 50 milliseconds.

## Privacy and Data Management

### Does NectarLink send my clipboard when I copy a password from my password manager?

No. Password managers on both Windows (such as 1Password, Bitwarden, and KeePassXC) and Android 13+ mark copied passwords with system privacy flags (`ExcludeClipboardContentFromMonitorProcessing` on Windows and `ClipDescription.EXTRA_IS_SENSITIVE` on Android). NectarLink checks those flags before reading or sending the clipboard and skips any sensitive item. One-time verification codes (OTPs) are also excluded from clipboard history and `Timeline`.

### How do I delete my local history or unpair a device?

- To wipe your local history and caches while keeping your paired devices and settings, open **Settings › Storage & Privacy** on Windows (or **Settings › Features & Data › Data & storage** on Android) and click **Clear everything...** (or click **Clear** next to any individual category such as **Clipboard history**, **Notification history**, **Cross-device timeline**, **Cached messages**, or **Photo thumbnails**).
- To unpair a device, open **Settings › Devices** on either app and click **Unpair**. Both devices remove each other from their trust store and drop any queued items for that peer.
