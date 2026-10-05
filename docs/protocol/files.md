# Files service (protocol v0)

> Status: **draft.** Builds on [protocol v0](v0.md); breaking changes are
> allowed until v1. License: CC BY 4.0.

Sends files between paired devices, in either direction, with progress,
cancellation and resuming after a dropped connection.

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
  files: [ { name: text, size: uint } ],   // 1–1,000 files, in order
}
```

`name` is a file name only (no path), 1–255 bytes, without `/`, `\`, NUL
or control characters, and not `.` or `..`. Receivers **MUST** reject an
offer that breaks these rules with `BAD_MESSAGE`, and still pick their own
safe name for writing (see §4).

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
- A name that already exists there gets a number: `photo (2).jpg`.
- A device accepts files only from paired devices with the `files` toggle
  on. A receiver that runs out of space resets the stream.
- Implementations **MUST NOT** log file names (v0 §11).
