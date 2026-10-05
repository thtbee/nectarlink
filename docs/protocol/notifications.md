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

### 2.1 App icons

The icon of an app is sent with the **first** notification of that app in a
session (in `notify.posted` or `notify.snapshot`) and omitted afterwards.
Receivers keep the icons they got for the session's lifetime and may cache
them longer, keyed by package name.

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
