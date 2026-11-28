# Messages service (protocol v0)

> Status: **draft.** Builds on [protocol v0](v0.md); breaking changes are
> allowed until v1. License: CC BY 4.0.

A PC reads a phone's text messages (SMS and MMS) and sends texts through
it. The phone keeps the messages; a PC asks for what it shows and keeps it
only in memory. New-message alerts come from the phone's own notifications
([notifications](notifications.md)), not from this service.

## 1. Capabilities

| ID | Offered by | Meaning |
|---|---|---|
| `sms.read` | phone | Lists conversations and messages (needs the SMS permission) |
| `sms.send` | phone | Sends texts when a PC asks |
| `sms.show` | PC | Shows messages, and wants `sms.changed` |
| `sms.sims` | phone | Lists the SIMs it texts from, and sends on the one a PC picks |
| `sms.mark_read` | phone | Marks a conversation's received texts read in the phone's messages when a PC asks (Elevated only) |

Both devices must allow it: the `messages` device toggle, on by default,
on each side.

## 2. Messages

On the control stream. Each request is answered with the matching answer
or `error`.

```
t = "sms.threads"   id = n   b = { limit: uint }                          // 1 to 100
t = "sms.threads"   re = n   b = { threads: [ thread ] }                  // newest first

t = "sms.messages"  id = n   b = { thread: text, ? before: int, limit: uint }
t = "sms.messages"  re = n   b = { messages: [ message ] }               // newest first

t = "sms.send"      id = n   b = { to: [ text ], ? body: text, ? attachments: [ attachment ], ? sim: text }
t = "ok"            re = n

t = "sms.part"      id = n   b = { id: text }
t = "sms.part"      re = n   b = { mime: text, data: bytes }

t = "sms.sims"      id = n   b = { sims: [] }
t = "sms.sims"      re = n   b = { sims: [ { id: text, label: text } ], ? default: text }

t = "sms.mark_read" id = n   b = { thread: text }
t = "ok"            re = n

t = "sms.changed"            b = { ? thread: text }                       // phone → PC

attachment = {
  mime: "image/jpeg" | "image/png" | "image/gif" | "image/webp"
      | "audio/aac" | "audio/mp4" | "audio/mpeg" | "audio/ogg"
      | "audio/opus" | "audio/amr" | "audio/3gpp" | "audio/wav" | "audio/x-m4a",
  data: bytes,            // 1 B to 900 KiB
}

thread = {
  id: text,
  addresses: [ text ],    // The other people's numbers; more than one is a group
  ? names: [ text ],      // Their contact names, in the same order ("" when not a contact)
  ? snippet: text,        // The latest message's text, or a description like "Picture"
  date: int,              // The latest message, in Unix milliseconds
  ? unread: uint,
  ? photo: bytes,         // One-person conversations: the contact's photo, a JPEG of at most 16 KiB
}

message = {
  id: text,
  thread: text,
  ? address: text,        // Who sent it (received), or the recipient (sent)
  ? body: text,
  date: int,              // Unix milliseconds
  ? outgoing: bool,       // Sent from the phone
  ? status: "sent" | "pending" | "failed",
  ? parts: [ { id: text, mime: text, ? size: uint } ],   // Pictures and other attachments
  ? sim: text,           // The SIM it came in on or went out from (an id from sms.sims)
}
```

### 2.1 Reading

`sms.messages` pages back through a conversation: `before` is the oldest
`date` already shown. A phone answers with fewer items than `limit` when
the answer wouldn't fit in a frame (v0 §3), dropping the oldest.

`sms.part` fetches an attachment; parts larger than 900 KiB are refused
with `BUSY`.

### 2.2 Sending

`sms.send` sends `body` (at most 8 KiB) and/or `attachments` (at most 1 media
attachment, image or voice/audio, each at most 900 KiB) to `to` (1 to 20
numbers). When `attachments` is empty, `body` must not be blank and is sent as
SMS; when one or more attachments are included, the phone sends the message as
MMS (`sendMultimediaMessage`).
Errors: `DENIED` when messages are off for the PC, `UNSUPPORTED` when the phone
doesn't offer `sms.send`, `INTERNAL` when it couldn't send.

With `sim`, a phone that offers `sms.sims` sends on that SIM; without it, or
on a phone that doesn't, the phone's default SIM sends it.

### 2.3 SIMs

`sms.sims` lists the SIMs a dual-SIM phone texts from, in slot order, with
the label its settings show ("SIM 1 · Airtel") and the one it uses without
asking, if any. The body has the same shape as `call.sims`
([calls](calls.md)), at most the same number of SIMs, but its ids are the
phone's SIM subscriptions, not its calling accounts: an id from one list
doesn't mean anything to the other. A PC shows a choice only with two or
more, and by default sends on the SIM the conversation's newest message used.
Phones that don't offer `sms.sims` answer `UNSUPPORTED`.

### 2.4 Marking read

`sms.mark_read` asks the phone to mark the received texts of a conversation
read in its own messages, so the phone's unread count and notification match
what was read on the PC. `thread` is the phone's number for the conversation:
1 to 18 digits, nothing else (anything else is `BAD_MESSAGE`).
Android lets only the default SMS app change the messages store, and a
messaging app keeps its own state besides, so a phone presses the app's own
**Mark as read** button on the conversation's notification (found by its
title, a contact name or a number of the conversation; an app without one has
the notification dismissed). With its Elevated helper running it also marks
the texts read in the phone's messages (the shell may write once it allows
itself the SMS write app-op for the one change, then restores the mode it read first). A phone offers
`sms.mark_read` with notification access or a running helper; one that
doesn't answers `UNSUPPORTED`. The PC sends it only if the user turned on
**Mark texts read on the phone**, which is off by default.

### 2.5 Changes

The phone sends `sms.changed` to PCs that offer `sms.show` when its
messages change (a text arrived or was sent), with the conversation when
it knows it. PCs then ask again for what they show.

## 3. Limits of this version

- Texts are sent to one number at a time; replying in a group needs MMS
  and isn't part of this version.
- Phones can't mark messages as read for other apps: Android lets only the
  default SMS app change the messages store.
- Android 15 and later keep texts with one-time codes from other apps for
  a few hours; such conversations look empty in the meantime.

## 4. Privacy

Numbers, names, text and pictures are never logged (v0 §11).
