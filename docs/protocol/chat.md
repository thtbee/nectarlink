# Chat service (protocol v0)

> Status: **draft.** Builds on [protocol v0](v0.md); breaking changes are
> allowed until v1. License: CC BY 4.0.

Two paired devices keep a chat: short notes typed on one show on the other.
Each device keeps its own copy of the chat on its timeline, as an entry of
kind `message`, so history, search and retention work as they do for
everything else the two share.

All messages travel on the **control stream** (v0 §4), in order, so an edit
or a delete never overtakes the note it changes.

## 1. Capabilities

| ID | Offered by | Meaning |
|---|---|---|
| `chat.messages` | any device of this version | Takes `chat.message`, `chat.edit` and `chat.delete` |

A device sends notes only to a peer that offers `chat.messages`; an app that
doesn't know these messages ignores them (v0 §3.1), so the note would be
lost.

## 2. Notes

```
ChatMessage = {
  id:   text,   // The writer's ID for the note: 1 to 64 ASCII letters, digits and "-"
  text: text,   // Not blank, at most 8192 characters (Unicode scalar values)
}
```

The writer picks the ID (NectarLink uses 32 random hex digits). IDs are
scoped to the chat between the two devices: the same ID in another chat is
another note.

## 3. Messages

### `chat.message` (request, either way)

Body: `ChatMessage`. Asks the peer to add the note to the chat. The peer
answers `ok` once it's kept, or `error` with `BAD_MESSAGE` when the note
isn't valid. The writer keeps its copy only after the `ok`, so a note is on
both devices or neither.

A note with an ID the chat already has is answered `ok` and not added again:
it was sent again after a lost answer.

### `chat.edit` (event, either way)

Body: `{ id: text, text: text }`, the same limits as `ChatMessage`. The new
text of a note the sender wrote. A device changes only a note that the peer
sent in the chat with that peer; anything else is ignored.

### `chat.delete` (event, either way)

Body: `{ id: text }`. Deletes a note in the chat, whichever device wrote it.
An ID the chat doesn't have is ignored.

Edits and deletes are sent only while the peer is connected and aren't
queued: an app should change or delete its own copy at once, and say that a
peer that isn't connected keeps its copy.

## 4. Privacy

Notes stay on the two devices, on their timelines, like the other entries
there. They travel only over the paired devices' encrypted connection, never
through a server in the clear.
