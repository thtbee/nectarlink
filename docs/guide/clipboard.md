# Clipboard

NectarLink shares your clipboard between your PC and your Android phone. Copy plain text, rich HTML text, an image, or a file on one device and paste it on the other, with smart action chips for links and addresses, an optional encrypted 50-item clipboard history, and automatic filtering for passwords and one-time verification codes.

<p align="center">
  <img src="images/clipboard-autopaste.png" alt="Clipboard sync and smart context chips in NectarLink" width="760">
</p>

## What You Need

| Requirement | Details |
| :--- | :--- |
| **Android Permissions** | No special permission is needed for PC-to-phone clipboard sync or manual phone-to-PC sending. |
| **Android Power Level** | `Standard` for automatic PC-to-phone sync and one-tap phone-to-PC sending (via Quick Settings tile, share sheet, widget, or app button). `Elevated` enables **automatic background phone-to-PC clipboard sync** whenever you copy text in any Android app. |
| **Per-Device Toggle** | **Clipboard** (enabled by default in **Settings › Devices** on the PC). |
| **Network** | Local network, USB cable, or **Connect over mobile data**. If the target device is offline, your latest clipboard send waits in the `Waiting to send` queue (replacing any older queued clip for that device) and arrives automatically on reconnect. |

## Step-by-Step Use

### Copying on Your PC to Paste on Your Phone

1. On your PC, copy any plain text, formatted HTML text, or image (`Ctrl+C`, right-click **Copy**, or `Win+Shift+S` screen snip).
2. With **Sync clipboard** turned on in **Settings › General** (the default), NectarLink sends the copied content to your connected phone, and to other PCs paired with this one, automatically in the background.
3. On your phone, tap **Paste** in any text field or image editor.
4. You can also send your current PC clipboard manually at any time by clicking **Send clipboard** on the PC Home screen or running **Send clipboard to phone** in the `Command Palette` (`Ctrl+Alt+Space`).

### Copying on Your Phone to Paste on Your PC

Android 10 and newer block background apps from reading the system clipboard unless they run with shell privileges. NectarLink gives you two ways to send your phone's clipboard to your PC:

- **Automatic in the background (with `Elevated`)**: Once you enable `Elevated` (via Wireless Debugging in **Settings › Permissions › Control from your PC** or **Settings › Features & Data › Clipboard › Background clipboard access**), NectarLink's local shell helper listens for clipboard changes and sends copied text to your PC automatically as soon as you tap **Copy** in any app. Copied images go with the one-tap send below.
- **One-tap manual send (at `Standard` or `Assist`)**: Without `Elevated`, you can send what you copied on your phone using any of four quick surfaces:
  1. **Quick Settings tile**: Pull down your phone's notification shade and tap the **Send clipboard** tile.
  2. **Android share sheet**: Select text or an image in any app, tap **Share**, and choose **Send to PC** (or tap your PC's Direct Share icon).
  3. **Home screen widget**: Tap **Send clip** on the NectarLink **PC quick actions** widget on your Android home screen.
  4. **Inside the NectarLink app**: Tap **Send clipboard** on the Android Home screen (or in the persistent connection notification).

### Copying and Pasting Files (`Ctrl+C` / `Ctrl+V`)

NectarLink bridges Windows file clipboard drops (`CF_HDROP`) with file transfers:

- **PC to phone**: If you copy one or more files in Windows File Explorer (`Ctrl+C`) and click **Send clipboard** in NectarLink, those files are transferred straight to your phone.
- **Phone to PC**: When **Copy received files** is enabled in **Settings › General** (the default), small files and photos you send from your phone to your PC are saved to `Downloads\Nectarlink` and automatically placed onto your Windows clipboard as file drops so you can press `Ctrl+V` straight into an email, chat window, document, or folder.

### Smart Context Chips

When a text clip arrives from a paired device and matches a recognizable pattern, NectarLink displays a context chip on the confirmation banner so you can act on it with one click or tap:

| Detected Content | Context Chip Action |
| :--- | :--- |
| **Web URL (`https://...`)** | **Open**: Opens the link in your default web browser. |
| **Street Address or Coordinates** | **Open in Maps**: Opens the address in your default maps app. |
| **Phone Number** | **Call**: Opens the number in the NectarLink dialer (on PC) or phone dialer (on Android). |
| **Parcel Tracking Number** (UPS, FedEx, USPS, DHL) | **Track**: Opens the carrier tracking page for that package. |
| **Email Address** | **Email**: Starts a new email to that address in your default mail client. |

You can turn these suggestions off at any time in **Settings › General › Suggest clipboard actions** on the PC or **Settings › Features & Data › Clipboard › Suggest actions for copied text** on Android.

### Using Clipboard History

Both the Windows app and the Android app keep an optional encrypted local **Clipboard history** of the last 50 text and image clips shared between your devices:

1. Open **Clipboard history** from the Home screen (or `Timeline` / `Shelf` on PC).
2. Search clips by text, filter by source device, click **Copy** to place any earlier clip back on your active clipboard, or click **Pin** so an important snippet is never pushed out when new clips arrive.
3. Click **Delete** on an individual item or **Clear history** to wipe all stored clips.

## What the Options Do

### On Windows

- **Sync clipboard** (`Settings › General`, default: **On**): Automatically sends text and images copied on the PC to connected phones and paired PCs.
- **Copy one-time codes** (`Settings › General`, default: **Off**): Automatically copies verification codes detected in phone notifications and SMS messages to the PC clipboard.
- **Copy received files** (`Settings › General`, default: **On**): Places newly received files onto the Windows file clipboard (`CF_HDROP`) so they are ready to paste with `Ctrl+V`.
- **Suggest clipboard actions** (`Settings › General`, default: **On**): Shows context chips (**Open**, **Open in Maps**, **Call**, **Track**, **Email**) when matching text arrives.
- **Clipboard history** (`Settings › Storage & Privacy`, default: **On**): Stores up to 50 synced clips (kept for 7 days, up to 128 MB) in an encrypted local file, with a one-click **Clear** button.

### On Android

- **Background clipboard access** (`Settings › Features & Data › Clipboard`): Guides you through enabling Wireless Debugging (`Elevated`) so phone-to-PC clipboard sync runs automatically in the background.
- **Suggest actions for copied text** (`Settings › Features & Data › Clipboard`, default: **On**): Shows context chips when text arrives from your PC.
- **Clipboard history** (`Settings › Features & Data › Data & storage`, default: **On**): Toggles the encrypted 50-item local history on your phone and lets you clear it.

## Data and Privacy

- **Automatic exclusion of passwords and one-time codes**:
  - Any clipboard item marked private or sensitive by a password manager (`ExcludeClipboardContentFromMonitorProcessing` / `CloudClipboard` format flags on Windows, or `ClipDescription.EXTRA_IS_SENSITIVE` on Android 13+) is ignored by default: it isn't sent, and it's **never** saved in clipboard history. Turn on **Sync copied passwords** (off by default) to send it anyway; it then leaves the other device's clipboard after a minute and still isn't kept in history.
  - One-time verification codes (OTPs) are never written to clipboard history or `Timeline`, and an OTP auto-copied on your PC is never echoed back to your phone.
- **At-rest encryption**: When **Clipboard history** is enabled, your saved clips (`clipboard_history.enc` with `NLCB\x01` header) and clipboard images (`clipboard_images/<id>.enc` with `NLCI\x01` header) are encrypted directly at rest using the platform `KeyProtector`: **Windows DPAPI** (`CryptProtectData`, tied to your Windows user account) on the PC and the **Android Keystore** (`AES-256-GCM`, which may be hardware-backed) on Android.
- **Retention**: Unpinned clipboard history entries expire automatically after **7 days** (capped at 50 items and 128 MB total). In `Timeline`, clipboard entries reference the encrypted history item by ID so clip content is never stored in plain text inside SQLite.

## Limits

- **Android Background Clipboard Restriction without `Elevated`**: On Android 10 and newer, only the foreground app or the active input method (keyboard) can read the system clipboard. Unless `Elevated` is active, phone-to-PC clipboard sharing requires tapping the Quick Settings tile, share sheet, widget, or in-app button.
- **Clip Size Caps**: Individual text clips are capped at 256 KiB on the wire (`clip::MAX_TEXT_BYTES`) and clipboard images are capped at 32 MiB (`clip::MAX_IMAGE_BYTES`) during live sync (16 MiB per image in local history) so an accidental huge copy never stalls your connection. For larger images or files, use [Files and Transfers](files-and-transfers.md).
