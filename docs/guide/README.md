# NectarLink User Guide

NectarLink connects your Android phone and your PC directly over your local network. It handles notifications, messages, calls, clipboard sharing, file transfers, photo browsing, screen mirroring, camera streaming, audio routing, and cross-device input in one open-source application.

<p align="center">
  <img src="images/home-bloom.png" alt="NectarLink for Windows home screen in the Bloom theme" width="760">
</p>

## Why This Guide Is Organized This Way

Documentation for NectarLink is split into two parts so you can find answers quickly:

1. **User Guide (`docs/guide/`)**: Organized by everyday tasks and feature areas. Each page explains what a feature does, why it is useful, what permissions or settings it requires, how to use it step by step, where your data is stored, and what platform limits apply. This layout matches the navigation inside the Windows and Android apps and can be rendered directly on GitHub or published as a static documentation site.
2. **Technical Reference ([`docs/technical/`](../technical/README.md))**: Written for developers, security auditors, and contributors. It covers internal crate architecture, the wire protocol, FFI bridges, database schemas, performance budgets, and build instructions.

Separating task-oriented workflows from implementation internals keeps everyday instructions concise while giving engineers full access to system specifications.

## Table of Contents

### Setup and Foundations

- [Getting Started](getting-started.md): System requirements, installing on Windows 11 and Android, the first-run walkthrough, pairing methods (QR code, 6-digit nearby code, USB auto-pair), and choosing an Android power level (`Standard`, `Assist`, or `Elevated`).

### Everyday Features

- [Notifications](notifications.md): Live notification feed, Windows popups, inline replies, action buttons, Live Updates and timers, one-time verification code detection, per-app mute rules, notification profiles, and forwarding PC notifications to your phone.
- [Messages](messages.md): SMS and MMS conversations, Unified Conversations from chat apps (`MessagingStyle` and `RemoteInput`), image attachments, contact search, and one-time codes.
- [Calls](calls.md): Incoming call banners on your PC, answering or declining calls, rejecting with an SMS reply, placing calls from the PC dialer or contacts, call history, `Elevated` in-call controls, and why voice audio remains on the phone.
- [Clipboard](clipboard.md): Sharing plain text, HTML, images, and copied files between PC and phone, Quick Settings tile and share sheet sending, `Elevated` automatic background sync, smart context chips, encrypted 50-item history, and automatic exclusion of passwords and one-time codes.
- [Files and Transfers](files-and-transfers.md): Sending files and folders both ways, drag and drop, File Explorer `Send to` integration, pause and resume, BLAKE3 integrity verification, the persistent send queue and `Waiting to send` panel, mounting phone storage in File Explorer, browsing PC files from your phone, folder sync, and LocalSend compatibility.
- [Photos](photos.md): Browsing the phone camera roll, screenshots, and albums on your PC, full-resolution inspection, dragging photos directly into File Explorer or desktop apps, and new photo alerts.

### Screen, Camera, Audio, and Input

- [Mirroring and App Windows](mirroring.md): Mirroring your phone screen at up to 60 fps (H.264, H.265, AV1), forwarding phone audio, taking screenshots and MP4 recordings, keeping the phone awake or turning its physical screen off, remote touch and keyboard input (`Assist` and `Elevated`), opening individual phone apps in their own `app window`, DeX desktop mode, and reverse mirroring your PC screen to your phone.
- [Cameras and Audio](cameras-and-audio.md): Using your phone as a Windows webcam (`NectarLink Webcam` virtual camera add-on), lens and zoom controls, Continuity Camera for capturing photos and scanning documents with corner crop and black-and-white enhancement, using your phone as a low-latency wireless speaker for PC audio, and recording voice notes with timestamped markers.
- [Input and Control](input-and-control.md): Controlling your PC from your phone (touchpad, gyroscope air mouse, keyboard with modifier bar, voice dictation, presentation remote with laser pointer), typing on your phone from your PC keyboard, the `Deck` macro pad, phone system controls on PC Home, Desk Mode stand display, and Lock, Sleep, and Wake-on-LAN actions.
- [Desktop Surfaces and Automation](desktop-surfaces.md): The slide-out `Shelf` edge panel, Flick to send gestures, `Nectar Island` top-of-screen activity pill, the searchable cross-device `Timeline`, the global `Command Palette` (`Ctrl+Alt+Space`), rich `Handoff` for links, maps, videos, and documents, `notify-when` task completion alerts, trigger-action automations, the local HTTP API, and the command-line client.

### Configuration, Privacy, and Support

- [Settings Reference](settings.md): Every setting in the PC app (all 9 categories) and the Android app explained one by one.
- [Privacy and Security](privacy-and-security.md): Plain-language overview of Ed25519 device identity keys, cryptographic pairing, QUIC TLS 1.3 end-to-end encryption, local-first networking, automatic update checks, opt-in **Connect over mobile data**, per-device permissions, consent prompts, and local data retention.
- [Troubleshooting](troubleshooting.md): Running the `Connection Doctor`, resolving firewall, public network, VPN, and Wi-Fi AP isolation issues, configuring per-OEM Android battery settings, and fixing `Elevated` Wireless Debugging, USB transport, audio, and virtual camera issues.
- [Frequently Asked Questions (FAQ)](faq.md): Direct answers to common questions about accounts, networks, battery usage, multiple devices, and platform support.
- [Known Limitations](known-limitations.md): Honest summary of operating system and hardware constraints across Windows 11 and Android.

## Glossary

NectarLink uses consistent terms across the PC app, the Android app, and this guide:

| Term | Definition |
| :--- | :--- |
| `pair` | The one-time cryptographic ceremony that links a phone and a PC. Pairing exchanges Ed25519 public keys and verifies a shared secret via a QR code token or a 6-digit confirmation code. |
| `paired device` | A phone or PC that has completed pairing and is stored in your local trust store. Only a `paired device` can open an encrypted NectarLink session with your device. |
| `Standard` | The default Android power level. It uses ordinary Android runtime permissions and notification listener access, requiring no accessibility service and no developer options. |
| `Assist` | The second Android power level. It adds NectarLink's Android Accessibility Service (**Use your PC's mouse and keyboard** under **Control from your PC**) so your PC mouse and keyboard can tap, swipe, scroll, and type during screen mirroring without developer setup. |
| `Elevated` | The third Android power level (the **Elevated** card under **Control from your PC**). It uses Android's built-in Wireless Debugging (or USB `adb`, Shizuku or root) to run a local shell helper (`UID 2000`, no root). It enables real-time touch dragging, gamepad input, `app window` streaming, automatic background phone-to-PC clipboard sync, sensitive notification OTP detection on Android 15+, Wi-Fi and Bluetooth toggles, and in-call mute/hold/speaker controls. |
| `mirror` | A live video and audio stream of your phone's primary screen inside a dedicated window on your PC (or, with **Share PC screen to phone**, your PC's screen on your phone). |
| `app window` | A single Android application streamed to its own independent window on your PC using an Android virtual display (`Elevated` on Android 11+), running alongside whatever is on the phone's physical screen. |
| `Deck` | A customizable button grid on your phone that acts as a macro pad and live audio controller for your PC. |
| `Shelf` (**File shelf**) | A slide-out edge panel on your PC (`Ctrl+Alt+S`) that holds your phone's latest photo, latest screenshot, current clipboard item, and recent files for quick drag-and-drop sharing without stealing window focus. |
| `Timeline` | A searchable local history of everything shared between your devices, including files, clipboard items, links, saved photos, voice recordings, and mirroring or webcam sessions. |
| `island` (**Nectar Island**) | An optional compact pill at the top center of your PC monitor that displays live phone calls, file transfer progress, media playback, timers, and alerts. |
| `Waiting to send` | A persistent queue panel on Home (both PC and Android) that holds files, clipboard items, links, and task alerts sent while the target device was offline, delivering them automatically in order as soon as the device reconnects. |
| `Command Palette` | A keyboard-driven launcher on your PC (`Ctrl+Alt+Space`) for running any NectarLink action, sending text or files, starting mirroring, or jumping to settings from any application. |
| `Connection Doctor` | A diagnostic tool in the PC app (**Settings › Connection & About › Connection Doctor**) and Android app that checks firewall rules, network profile type, VPN routing, Wi-Fi AP isolation, and Android background battery restrictions. |
| **Connect over mobile data** | An opt-in setting (**Settings › Connection & About › Connect over mobile data** on the PC and **Settings › Devices › Connect over mobile data** on Android, off by default on both) that allows paired devices to stay connected across different networks (such as LTE/5G and home broadband) using end-to-end encrypted relay packets when a direct LAN path is unavailable. |
| `Handoff` | Sending an active web link, map location (`geo:` URI), video link with its current playback timestamp, or document to a paired device so it opens immediately in the default application on the receiving side. |
