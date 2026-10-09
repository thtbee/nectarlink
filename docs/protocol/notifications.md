# Notifications service (protocol v0)

> Status: **draft.** Builds on [protocol v0](v0.md); breaking changes are
> allowed until v1. License: CC BY 4.0.

A phone mirrors its notifications to paired PCs, which show them (as toasts
and in a feed) and can dismiss them, reply to them and run their actions.

All messages travel on the **control stream** (v0 §4). Notifications are
small, latency matters more than throughput, and their order must be kept:
a `notify.removed` must never overtake the `notify.posted` it removes.

## 1. Capabilities

| ID | Offered by | Meaning |
|---|---|---|
| `notify.mirror` | phone | Sends `notify.*` events. Offered once the user granted notification access |
| `notify.live` | phone | Includes `live` metadata on ongoing progress, timer, and Android 16 Live Update notifications |
| `notify.reply` | phone | Handles `notify.dismiss` and `notify.action` |
| `notify.sensitive` | phone | Includes notifications Android hides from apps as sensitive (one-time codes) |

A PC that doesn't offer anything still receives these events; a peer that
doesn't know them ignores them (v0 §3.1).

## 2. The notification

```
Notification = {
  key:      text,        // The phone's ID for it (Android: StatusBarNotification key), 1–256 bytes
  app:      text,        // Package name, e.g. "com.whatsapp"
  app_name: text,        // User-visible app name, e.g. "WhatsApp"
  title:    text?,       // e.g. the sender
  text:     text?,       // The body, expanded if the app provides more
  sub:      text?,       // Secondary line, e.g. a conversation or account name
  when:     int,         // When it was posted, Unix milliseconds
  actions:  [Action],    // In display order, at most 5
  silent:   bool,        // Arrived without sound or pop-up on the phone; don't alert either
  icon:     bytes?,      // The app's icon, PNG, at most 64 KiB (see §2.1)
  image:        bytes?,          // The picture it shows (a photo in a message, a big picture), JPEG, at most 160 KiB
  live:         Live?,           // Ongoing progress, timer, or Android 16 Live Update metadata (see §2.2)
  conversation: Conversation?,   // Notification.MessagingStyle conversation metadata (see §2.3)
}

Action = {
  id:    text,           // Opaque to the PC, 1–64 bytes
  title: text,           // Button label, e.g. "Mark as read"
  reply: bool,           // Takes text (an inline reply field)
}
```

Text fields are plain text. Senders **MUST** truncate `title` and `sub` to 256
characters, `text` to 4,096 and action titles to 64; receivers **MUST**
enforce the same limits. A notification with neither `title` nor `text` is
not sent.

`image` is sent with every post of a notification that shows a picture,
scaled so its longest side is at most 512 pixels. Receivers drop an image
over the limit. A snapshot carries pictures only while they fit its size
budget; the rest arrive without one.

### 2.1 App icons

The icon of an app is sent with the **first** notification of that app in a
session (in `notify.posted` or `notify.snapshot`) and omitted afterwards.
Receivers keep the icons they got for the session's lifetime and may cache
them longer, keyed by package name.

### 2.2 Live Updates (`live`)

Ongoing progress bars, timers, and Android 16 Live Updates (rides, deliveries,
navigation, active builds) carry an optional `live` map:

```
Live = {
  v:             uint,         // Schema version (currently 1)
  progress:      uint?,        // Current progress, 0..=max
  max:           uint?,        // Maximum progress (> 0 when determinate)
  indeterminate: bool?,        // True when progress is indeterminate
  chip:          text?,        // Short status pill text (Notification.getShortCriticalText), at most 32 chars
  segments:      [Segment]?,   // Android 16 ProgressStyle segments, at most 16
  points:        [Point]?,     // Android 16 ProgressStyle milestone points, at most 16
  chronometer:   bool?,        // Live elapsed or countdown timer relative to Notification.when
  countdown:     bool?,        // True when the chronometer counts down toward Notification.when
}

Segment = {
  length: uint,                // Relative segment length (> 0)
  color:  uint?,               // Optional 24-bit RGB / 32-bit ARGB color
}

Point = {
  position: uint,              // Position along 0..=max
  color:    uint?,             // Optional 24-bit RGB / 32-bit ARGB color
}
```

Senders coalesce rapid `notify.posted` updates for the same `key` to at most
one update per second while always delivering the final update (`progress >= max`
or `notify.removed`). Receivers pin live notifications above regular
notifications in the feed and update both the feed card and the native toast in
place without re-alerting.

### 2.3 Conversation notifications (`conversation`)

Chat notifications built with `Notification.MessagingStyle` (RCS, WhatsApp,
Telegram, Signal, and other messaging apps) carry an optional `conversation`
map so the PC can present them in a unified Messages inbox alongside SMS:

```
Conversation = {
  v:        uint,            // Schema version (currently 1)
  title:    text,            // Group title or 1:1 contact name, at most 256 chars
  group:    bool?,           // True when MessagingStyle.isGroupConversation is true
  avatar:   bytes?,          // Conversation or contact avatar (JPEG/PNG), at most 16 KiB
  messages: [ChatMessage],   // Chronological (oldest first), at most 25
}

ChatMessage = {
  sender:    text?,          // Display name (omitted when self_sent is true), at most 128 chars
  text:      text,           // Message body, at most 4,096 chars
  time:      int,            // Unix milliseconds
  self_sent: bool?,          // True when sent by the phone's user
  avatar:    bytes?,         // Optional sender avatar (JPEG/PNG), at most 16 KiB
}
```

To avoid duplicating SMS threads that the SMS provider already supplies, a
phone with `sms.read` omits `conversation` when a notification from the default
SMS app matches a message already present in the system SMS/MMS provider.
Replies from the unified inbox use `notify.action` (§4) with the notification's
`reply: true` action (`RemoteInput`).

## 3. Events (phone → PC)

```
t = "notify.snapshot"   b = { items: [Notification] }
t = "notify.posted"     b = Notification
t = "notify.removed"    b = { key: text }
```

- **`notify.snapshot`** replaces everything the PC holds for this phone. The
  phone sends it right after a session is established, when the user turns
  notifications for that PC on or off (an empty list when off), and when its
  notification listener (re)connects. Items are newest first, at most 100.
  The PC **MUST NOT** alert for snapshot items.
- **`notify.posted`** adds a notification, or replaces the one with the same
  `key` (an update, such as a new message in the same conversation). The PC
  alerts unless `silent` is set.
- **`notify.removed`** removes it: the user or the app dismissed it on the
  phone. The PC removes its toast too.

## 4. Requests (PC → phone)

```
t = "notify.sync"                                    // one-way
t = "notify.dismiss"   id = n   b = { key: text }
t = "notify.action"    id = n   b = { key: text, action: text, reply: text? }
t = "ok"               re = n
```

- **`notify.sync`** asks for a fresh `notify.snapshot`, for example after the
  user turned notifications from this phone back on. A phone that doesn't
  mirror notifications ignores it.
- **`notify.dismiss`** dismisses the notification on the phone. Dismissing
  one that's already gone succeeds.
- **`notify.action`** runs an action. For a reply action, `reply` holds the
  text (at most 4,096 characters). The phone answers `ok` once the action ran,
  without waiting for its effect (the message being delivered, for example).

Errors: `DENIED` when the user turned notifications off for this PC,
`NOT_FOUND` when the notification or action no longer exists, `UNSUPPORTED`
when the phone doesn't offer `notify.reply`.

## 5. Privacy

- Notifications are sent only to PCs the user allows (the `notifications`
  device toggle on the phone) and shown only from phones the user allows (the
  same toggle on the PC).
- Apps that mark a notification as local-only (Android `FLAG_LOCAL_ONLY`) are
  respected: such notifications are never sent.
- Implementations **MUST NOT** log notification content (v0 §11).
