# Notifications

NectarLink mirrors your phone's notifications to your PC in real time. You can read alerts, reply inline to messages, trigger notification action buttons, track live delivery or timer progress, copy one-time verification codes with one click, and dismiss notifications on both devices at once.

<p align="center">
  <img src="images/notifications-grouped.png" alt="Phone notifications grouped by app on the PC Home screen" width="760">
</p>

## What You Need

| Requirement | Details |
| :--- | :--- |
| **Android Permission** | **Notification access** (`Settings › Permissions › Notifications › Show notifications on your PC`). On Android 13+, if the switch is grayed out, open **App info › Allow restricted settings** first. |
| **Android Power Level** | `Standard` for all regular notifications, inline replies, actions, and Live Updates. `Elevated` is required on Android 15 and newer if you want to receive notifications that Android's Enhanced Confirmation Mode marks as sensitive (such as SMS one-time verification codes). |
| **Per-Device Toggle** | **Notifications** (enabled by default in **Settings › Devices** on the PC). |
| **Network** | Local network, USB cable, or **Connect over mobile data**. |

## Step-by-Step Use

### Reading, Grouping, and Dismissing Notifications

1. Open **NectarLink** on your PC and stay on **Home**. Active notifications from your connected phone appear in the **Notifications** card, displaying each app's icon, title, body text, timestamp, and any attached photo or big picture.
2. When an app posts multiple notifications, NectarLink groups them under a collapsible app header showing the count. Click the header to expand or collapse that app's alerts, or click **Clear group** (`x`) to dismiss all notifications from that app at once.
3. Click the `x` button on any individual notification card (or dismiss the Windows popup) to clear it. Dismissals synchronize both ways: clearing a notification on your PC removes it from your phone's notification shade, and swiping it away on your phone removes it from your PC. After you dismiss a notification or group on the PC, a brief **Undo** bar lets you restore it locally if you clicked by mistake.
4. **Clear** in the `Nectar Island` panel is local to the Island: it hides those alerts there, while Home keeps every one and its history. A newer post under the same key shows on the Island again. With **Alerts** on, the Island replaces the Windows popup toast, so a phone notification is announced once.
5. Click a notification card on your PC while `Elevated` is active (on Android 11+) to open that phone app directly on your PC in its own `app window`.
6. If the phone already shows notifications but Home stays empty, click **Refresh** under **No notifications**. That asks the phone for what it shows now. If it stays empty, check the phone's notification access and set NectarLink's battery use to **Unrestricted** (see [Troubleshooting](troubleshooting.md)).

### Replying Inline and Running Notification Actions

1. When a notification supports direct reply (such as a chat message or email), click **Reply** on the notification card in the app or type directly into the Windows notification popup.
2. Type your response and press **Enter** or click **Send**. Your phone sends the reply through the original app's `RemoteInput` handler immediately.
3. Action buttons attached to the notification on your phone (such as **Mark as read**, **Archive**, **Remind me**, or **Snooze**) appear as buttons on both the Windows popup and the card in NectarLink. Click any action button to run it on the phone.

### Live Updates, Progress Bars, and Timers

Notifications that report ongoing progress or live time stay pinned at the top of your notification feed and update cleanly in place without re-popping or playing repeated alert sounds:

- **Android 16 Live Updates (`ProgressStyle`)**: Ride-share, food delivery, navigation, and flight tracker notifications display their multi-segment progress bar, current milestone point, and tracker icon live on your PC.
- **Ongoing Progress Bars**: Downloads, uploads, and system tasks update their percentage bar in place.
- **Chronometers and Countdowns**: Active kitchen timers, stopwatches, and ongoing workout timers tick every second in the feed (and in `Nectar Island` when enabled).

### One-Time Verification Codes (OTPs)

When an incoming notification or SMS contains a 4-to-8-digit one-time verification code:

<p align="center">
  <img src="images/notifications-otp.png" alt="One-time verification code detected with a Copy code button" width="760">
</p>

1. NectarLink extracts the verification code locally on your device and adds a prominent **Copy code (`123456`)** button to both the Windows notification popup and the notification card on Home.
2. If you turn on **Copy one-time codes** in **Settings › General**, NectarLink copies the verification code to your PC clipboard automatically as soon as it arrives and shows a brief confirmation banner so you can press `Ctrl+V` straight into your browser.
3. One-time verification codes are never written to your clipboard history or `Timeline`, and copying an OTP on your PC never loops it back to the phone.

### Viewing Dismissed Notification History

If you accidentally swipe away an alert on your phone or want to check a notification from earlier in the day:

1. In the **Notifications** card on PC Home, switch the filter from **Active** to **History**.
2. NectarLink displays dismissed notifications from the last day by default (up to 300 items; longer if you choose, see below), complete with search filtering by app name or text.
3. You can clear the history at any time, turn it off, or choose how long it stays (1 day to 30 days) in **Settings › Storage & Privacy**.

### Forwarding PC Notifications to Your Phone (Optional)

If you step away from your desk and want your PC's alerts (such as calendar reminders, build completions, or desktop chat messages) sent to your phone:

1. On your PC, open **Settings › Notifications** and turn on **Forward PC notifications**.
2. Grant Windows notification listener access if Windows prompts you.
3. Incoming Windows notifications are forwarded to your connected phone under the **Tasks on your PC** channel. This setting is **off by default** for privacy.

## What the Options Do

All notification settings live in **Settings › Notifications** (and **Settings › General** / **Settings › Storage & Privacy**) on the PC:

- **Follow phone Do not disturb** (`Settings › Notifications`, default: **Off**): When turned on, your PC silences popup toasts whenever your phone is in Do Not Disturb mode. Notifications still arrive quietly in the NectarLink feed so you can read them when you want.
- **Keep pop-ups paused after a restart** (`Settings › Notifications`, default: **Off**): **Pause pop-ups** on Home normally lasts until NectarLink quits. With this on, it stays paused through restarts until you press **Resume pop-ups**.
- **Notification profile on Home**: From the Phone Controls card on PC Home, you can switch your phone's notification profile between **All**, **Priority only** (Do Not Disturb), and **Silent**.
- **Forward PC notifications** (`Settings › Notifications`, default: **Off**): Mirrors Windows desktop notifications to your connected phone.
- **Quiet hours** (`Settings › Notifications`): Opens Windows **System › Notifications** settings so you can configure automatic Focus / Do Not Disturb schedules on Windows 11.
- **Per-app notification rules** (`Settings › Notifications`): Every phone app that has sent at least one notification appears in an alphabetical list with its icon and a three-way selector:
  - **Show** (default): Shows a Windows popup toast and lists the alert in the NectarLink feed.
  - **No pop-ups**: Suppresses Windows popup toasts for that app while keeping its notifications visible inside the NectarLink feed.
  - **Hide**: Completely blocks notifications from that app on your PC.
- **Copy one-time codes** (`Settings › General`, default: **Off**): Automatically copies detected verification codes from incoming notifications and SMS messages to the PC clipboard.
- **Notification history** (`Settings › Storage & Privacy`, default: **On**): Keeps dismissed phone notifications on the PC. Includes a one-click **Clear** button.
- **Keep notification history for** (`Settings › Storage & Privacy`, default: **1 day**): 1 day, 3 days, 1 week or 30 days. Up to 300 alerts a day are kept, at most 3,000.

## Data and Privacy

- **Where data travels**: Notification payloads, app icons, and inline images travel directly between your phone and PC over your end-to-end encrypted QUIC session.
- **Where data stays**: Active notifications are held in memory while current (up to 200 active notifications). If **Notification history** is enabled on the PC, dismissed notifications are stored locally in `%LOCALAPPDATA%\Nectarlink\notification-history.json` (and per-app rules in `notification-rules.json`) and cached inline images are kept in `%LOCALAPPDATA%\Nectarlink\cache\images\` for as long as you chose to keep history (a day by default; images a day longer), then automatically purged. No notification history is duplicated on the phone.
- **OTP protection**: Notifications containing one-time verification codes are excluded from clipboard history and `Timeline` storage. Neither the Android app nor the `Elevated` shell helper ever logs notification text to `logcat` or disk logs.

## Limits

- **Android 15+ Enhanced Confirmation Mode (`RECEIVE_SENSITIVE_NOTIFICATIONS`)**: Starting in Android 15, the operating system redacts the text of notifications that appear to contain two-factor authentication codes from untrusted notification listeners. To receive sensitive notification text and OTPs on Android 15 and newer, enable the `Elevated` power level once; NectarLink's shell helper automatically grants `RECEIVE_SENSITIVE_NOTIFICATIONS` to NectarLink.
- **OEM Background Restrictions**: If your phone manufacturer puts NectarLink to sleep in the background, notifications may be delayed until you wake the phone screen. Follow the per-OEM steps in [Troubleshooting](troubleshooting.md) to set NectarLink's battery usage to **Unrestricted**.
- **Chat App History**: Notification mirroring captures alerts that arrive while your phone and PC are connected (or recently active in the phone's shade). Full historical message databases for third-party encrypted chat apps (such as Signal or WhatsApp) are sandboxed by Android and cannot be read retroactively.
