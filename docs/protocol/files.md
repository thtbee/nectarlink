# Files service (protocol v0)

> Status: **draft.** Builds on [protocol v0](v0.md); breaking changes are
> allowed until v1. License: CC BY 4.0.

Sends files and folders between paired devices, in either direction, with
progress, cancellation and resuming after a dropped connection.

## 1. Capability

| ID | Offered by | Meaning |
|---|---|---|
| `files.transfer` | both | Sends and receives files |

## 2. A transfer

Each transfer runs on its own bidirectional stream (v0 §4), so it never
holds up the control stream. The sender opens it and writes the stream
header, then an offer:

```
t = "stream"        b = { svc: "files", op: "send", v: 1 }
t = "files.offer"   b = {
  id:    text,             // Transfer ID: 16–64 bytes of [A-Za-z0-9_-], the same on every retry
  files: [ {
    name:    text,         // The file's name
    size:    uint,
    ? folder: text,        // Only for files in a sent folder (see below)
  } ],                     // 1–5,000 files, in order
}
```

`name` is a file name only (no path), 1–255 bytes, without `/`, `\`, NUL
or control characters, and not `.` or `..`.

**Folders.** A sent folder is its files, each with `folder`: where the file
is, as names joined by `/`, starting with the sent folder's own (`Trip`,
`Trip/Day 1`). Each name follows the rules for `name`; there are at most 32
of them, in at most 1,024 bytes. A folder's files are listed together.
Empty folders aren't sent.

Receivers **MUST** reject an offer that breaks these rules with
`BAD_MESSAGE`, and still pick their own safe names for writing (see §4). The
whole offer is one frame (v0 §3), which limits how many files with long
names fit in it.

The receiver answers on the same stream:

```
t = "files.accept"  b = { have: [uint] }   // bytes already received, per file
```

or `error` (`DENIED`: the user turned files off for this device; `BUSY`:
not enough space or too many transfers) and closes the stream.

The sender then writes the files' bytes, raw and back to back: for each
file, from `have[i]` to its `size`. A file whose `have` equals its `size`
contributes nothing. The sender then finishes its side of the stream.

When the receiver has every byte, it stores the files and answers:

```
t = "files.done"
```

The transfer has succeeded once the sender reads `files.done`. Anything
else (a reset, a closed connection, a missing `done`) means it didn't.

## 3. Cancelling and resuming

- Either side cancels by resetting the stream (QUIC `RESET_STREAM` or
  `STOP_SENDING`, application code 10, "cancelled").
- A transfer interrupted any other way (the connection dropped) is resumed
  by offering it again with the **same `id`** when the devices reconnect.
  The receiver keeps partly received files for a transfer for at least an
  hour and reports their length in `have`. A sender **MUST** use a new `id`
  if any file changed in between.

## 4. Receiving safely

- Files are written to a private folder first and moved to where the user
  finds them (the Downloads folder) only once complete.
- A name that already exists there gets a number: `photo (2).jpg`. A sent
  folder gets a new folder of its own the same way (`Trip (2)`); nothing
  is ever added to a folder that was already there.
- Names the receiving system can't store (on Windows: `CON`, `NUL`, `?`,
  trailing dots…) are changed to ones it can.
- A device accepts files only from paired devices with the `files` toggle
  on. A receiver that runs out of space resets the stream.
- Implementations **MUST NOT** log file names (v0 §11).
