# Photos service (protocol v0)

> Status: **draft.** Builds on [protocol v0](v0.md) and the
> [files service](files.md); breaking changes are allowed until v1.
> License: CC BY 4.0.

A phone tells its PCs about each photo or screenshot it takes, with a small
preview, so the PC can offer it right away. The photo itself comes only
when the PC asks, as an ordinary files transfer.

## 1. Capabilities

| ID | Offered by | Meaning |
|---|---|---|
| `photos.read` | phone | Announces new photos and sends them on request (needs the phone's photos permission) |
| `photos.show` | PC | Shows announced photos |

A phone announces photos only to PCs that offer `photos.show`, and only
while both devices allow it (the `photos` device toggle, on by default, on
each side).

## 2. Messages

On the control stream.

```
t = "photos.new"                b = {
  id:         text,      // The phone's ID for it: 1–64 printable ASCII characters
  name:       text,      // A file name, as in files.offer
  size:       uint,      // Bytes
  taken:      int,       // Unix seconds
  screenshot: bool,      // Optional, false when missing
  thumb:      bytes,     // A JPEG preview: at most 96 KiB, at most 512 pixels a side
}
t = "photos.get"     id = n     b = { id: text }
t = "photos.sending" re = n     b = { transfer: text }
```

### 2.1 `photos.new`

Sent once per new photo, to the PCs connected at the time; a PC that
connects later doesn't hear about it. It isn't answered. Phones **SHOULD**
leave out bursts (a camera's burst mode, a downloaded album) beyond the
latest few. A PC drops announcements that break the rules above.

### 2.2 `photos.get`

Asks for an announced photo. The phone starts sending it as a files
transfer ([files](files.md) §2) and answers with that transfer's `id`; the
PC finds the file there when the transfer is done.

A phone keeps at least the 50 latest announced IDs and sends only photos
it announced. Errors: `NOT_FOUND` when the ID isn't one of them or the
photo is gone; `DENIED` when photos or files are off for the PC.

## 3. Privacy

- Nothing is sent before the user gives the phone app access to photos,
  and only photos taken from then on are announced.
- Previews and photos are never logged; names neither (v0 §11).
