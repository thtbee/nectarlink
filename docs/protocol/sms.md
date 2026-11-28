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
| `sms.mark_unread` | phone | Marks a conversation's latest received text unread in the phone's messages when a PC asks (Elevated only) |
| `sms.delete` | phone | Deletes a conversation, or some of its messages, from the phone's messages when a PC asks (Elevated only) |

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

t = "sms.mark_unread" id = n b = { thread: text, ? message: text }
t = "ok"            re = n

t = "sms.delete"    id = n   b = { thread: text, ? messages: [ text ] }   // at most 100 messages
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
  ? fresh: uint,          // Of the unread, the new ones not yet seen on the phone (with a notification); absent: all of them
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

### 2.5 Marking unread

`sms.mark_unread` asks the phone to mark a received text of a conversation
unread in its own messages: `message` (an id from `sms.messages`, `sms:` or
`mms:` and digits) if given, else the conversation's latest received text, text
or picture message. The user said marking that one is enough: the phone shows
the conversation unread, with one unread text. `thread` is a number as in §2.4,
anything else (and a `message` that isn't an id) is `BAD_MESSAGE`.

Only the Elevated helper can do it, in the same way as the shell's marking read
(§2.4): the phone's messages take writes from the default SMS app, and from the
shell once it allows itself the SMS write app-op for the one change, which it
puts back as it was. The phone reads the message back and answers `ok` only
when it reads unread, because the messages store ignores a write it doesn't
allow without saying so; any other failure is `INTERNAL`. A phone that offers
`sms.mark_unread` has the helper running (and may read texts); one that doesn't
answers `UNSUPPORTED`. A PC sends it only if the user turned on **Sync read
state with the phone**, the same setting that sends `sms.mark_read`, which is off
by default.

A messaging app such as Google Messages keeps its own state besides the phone's
messages, so its badge may not change in every app; the phone's messages
database does.

### 2.6 Deleting

`sms.delete` asks the phone to delete a whole conversation (no `messages`), or
only the messages named, from its own messages, texts and picture messages
(`sms:12`, `mms:7`, as `sms.messages` lists them; at most 100, anything else is
`BAD_MESSAGE`). It cannot be undone and is **sent only when the user chooses it
for that deletion**: a PC shows the choice each time and never sends it by
default or for something else. A list of `messages` that is empty is a whole
conversation, so a PC that names messages sends none rather than an empty list.

As with §2.5, only the Elevated helper does it, one change at a time, with
nothing but numbers in the shell's commands: a conversation is
`content delete` by its `thread_id` on both the texts and the picture messages,
and messages by their numbers and the `thread_id`, so a number of another
conversation deletes nothing. The phone reads back and answers `ok` only when
nothing it was asked to delete remains; the phone's messages drop an emptied
conversation. A failure, for instance a phone whose messages the shell cannot
write, is `INTERNAL`; `DENIED` when messages are off for the PC; `UNSUPPORTED`
from a phone that doesn't offer `sms.delete`.

A PC answers a failure in plain words and keeps what it did on its own side.

### 2.7 What a PC keeps to itself

`sms.mark_read`, `sms.mark_unread` and `sms.delete` are all the protocol has for
changing the phone's messages. What a PC keeps itself, where the phone can't or the
user didn't choose to (a delete on this PC only, or a conversation marked unread on a phone
that doesn't offer it), is by conversation (the phone's
`thread`) and message (`id`) and as of the date of the latest message at the time, so a
newer message shows again. It sends nothing to the phone for it, and drops it when the
phone is unpaired. There is no archive: Android has no API for it, so a PC doesn't offer one.
