# Photos service (protocol v0)

> Status: **draft.** Builds on [protocol v0](v0.md) and the
> [files service](files.md); breaking changes are allowed until v1.
> License: CC BY 4.0.

A phone tells its PCs about each photo or screenshot it takes, with a small
preview, so the PC can offer it right away, and lets a PC browse its albums,
photos and videos with thumbnails fetched on demand. Full files come only when
the PC asks, as an ordinary files transfer.

## 1. Capabilities

| ID | Offered by | Meaning |
|---|---|---|
| `photos.read` | phone | Announces new photos, lists albums and items, serves thumbnails, and sends full files on request (needs the phone's photos permission) |
| `photos.show` | PC | Shows announced photos and the phone's gallery |

A phone announces photos and library changes only to PCs that offer
`photos.show`, and only while both devices allow it (the `photos` device
toggle, on by default, on each side).

## 2. Messages

On the control stream.

```
t = "photos.new"                     b = {
  id:         text,      // The phone's ID for it: 1–64 printable ASCII characters
  name:       text,      // A file name, as in files.offer
  size:       uint,      // Bytes
  taken:      int,       // Unix seconds
  screenshot: bool,      // Optional, false when missing
  thumb:      bytes,     // A JPEG preview: at most 96 KiB, at most 512 pixels a side
}
t = "photos.albums"      id = n      b = {}
t = "photos.albums.list" re = n      b = { albums: [ PhotoAlbum, ... ] }
t = "photos.list"        id = n      b = { album?: text, before?: int, before_id?: text, limit: uint }
t = "photos.items"       re = n      b = { items: [ PhotoItem, ... ] }
t = "photos.thumbs"      id = n      b = { ids: [ text, ... ] }
t = "photos.thumbs.list" re = n      b = { thumbs: [ PhotoThumb, ... ] }
t = "photos.get"         id = n      b = { id?: text, ids?: [ text, ... ] }
t = "photos.sending"     re = n      b = { transfer: text }
t = "photos.changed"                 b = {}
```

```
PhotoAlbum = {
  id:     text,          // Album bucket ID on the phone
  name:   text,          // Display name, e.g. "Camera", "Screenshots"
  count:  uint,          // Number of photos and videos in the album
  cover?: text,          // Newest item's ID, for its thumbnail
}

PhotoItem = {
  id:        text,       // Item ID on the phone (1–64 printable ASCII characters)
  name:      text,       // File name, sanitized as in files.offer
  date:      int,        // When it was taken (Unix ms), or else when it was saved
  size:      uint,       // Full file size in bytes
  width:     uint,       // Pixels (0 when unknown)
  height:    uint,       // Pixels (0 when unknown)
  duration?: uint,       // Video duration in milliseconds (absent for photos)
  album?:    text,       // Album bucket ID
}

PhotoThumb = {
  id:   text,            // Item ID
  data: bytes,           // A JPEG thumbnail: at most 96 KiB, ~256 pixels a side
}
```

### 2.1 `photos.new`

Sent once per new photo, to the PCs connected at the time; a PC that
connects later doesn't hear about it. It isn't answered. Phones **SHOULD**
leave out bursts (a camera's burst mode, a downloaded album) beyond the
latest few. A PC drops announcements that break the rules above.

### 2.2 `photos.albums`

Lists the phone's photo and video albums (MediaStore buckets), with each
album's item count and newest item ID (`cover`).

### 2.3 `photos.list`

Lists photos and videos on the phone (or in `album` when given), newest
first by `date`, then in an order of the phone's own that never changes
(Android: by media ID). To get the next page, the PC sends the last item of
the one before as `before` (its `date`) and `before_id` (its `id`); the
phone answers with the items after it in that order, so items sharing a
date are neither skipped nor repeated. With `before` alone, only items
older than `before` come. `limit` is capped at 200, and the phone trims the
page if needed so the frame fits within `MAX_FRAME_LEN`; fewer items than
`limit` means there are no more.

### 2.4 `photos.thumbs`

Fetches small JPEG thumbnails for up to 24 item IDs in one batch. Items
whose thumbnail cannot be generated are omitted from `thumbs` rather than
failing the batch. The PC caches thumbnails on disk and only asks for
items currently visible in the grid.

### 2.5 `photos.get`

Asks for one (`id`) or several (`ids`, up to 200) photos or videos. The
phone starts sending them as a files transfer ([files](files.md) §2) and
answers with that transfer's `id`; the PC finds the file(s) there when the
transfer is done.

Errors: `NOT_FOUND` when a requested item is gone; `DENIED` when photos or
files are off for the PC.

### 2.6 `photos.changed`

Sent (debounced) when the phone's photo or video library changes so
connected PCs can refresh their album and item lists. It isn't answered.

## 3. Privacy

- Nothing is sent before the user gives the phone app access to photos and
  videos (including Android 14's "Select photos" access, where only the
  chosen items are visible), and only new photos taken from then on are
  announced with `photos.new`.
- Previews, thumbnails and photos are never logged; names neither (v0 §11).

