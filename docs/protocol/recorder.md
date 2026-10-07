# Voice recorder (`recorder`)

Records a voice note on the phone and sends it to a paired PC through the
resumable `files` stream (`docs/protocol/files.md`), along with any markers
added during recording.

## 1. Capability & toggle

| Item | Value | Notes |
|---|---|---|
| Desktop capability | `recorder` | Announced in `hello.caps` / `hello.update` by PCs that save and convert voice recordings |
| Phone capability | `files.transfer` | Base capability on every build |
| Per-device toggle | `recordings` | Default `true` on both devices |
| Feature ID | `files.recordings` | Available when phone offers `files.transfer`, desktop offers `recorder`, and `recordings` toggle is on |

## 2. Wire format

Voice recordings travel on the `files` stream (`nectarlink/1/files`,
`op: "send"`, `v: 1`) as a single-file `files.offer` with `recording: true`
and optional `markers`:

```json
{
  "t": "files.offer",
  "id": 1,
  "b": {
    "id": "AbCdEfGhIjKlMnOpQrStUv",
    "files": [
      { "name": "Recording 2026-10-07 14.32.m4a", "size": 194560 }
    ],
    "recording": true,
    "markers": [
      { "at_ms": 4200 },
      { "at_ms": 18500, "label": "Action item" }
    ]
  }
}
```

### Validation rules

- When `recording` is `true`:
  - `files` must contain **exactly 1** entry, and its `folder` must be `None`.
  - `markers` may contain `0..=256` (`MAX_MARKERS`) entries.
  - Each `RecordingMarker` has `at_ms: u64` (elapsed recording milliseconds,
    excluding paused time) and optional `label: Option<String>` (1–256 UTF-8
    bytes after trimming, with no control characters).
- When `recording` is `false` (or omitted):
  - `markers` must be empty.

### Refusal codes

Before accepting a `files.offer` with `recording: true`, the receiver checks:

1. **Capability:** If the receiver does not advertise `recorder`, it replies
   with `err` (`code: "unsupported"`) and closes the stream.
2. **Toggle:** If the `recordings` per-device toggle is off for this sender,
   it replies with `err` (`code: "denied"`) and closes the stream.

### Resumability & offline queueing

Because recordings use `files.offer` and `files.accept`, an interrupted
recording transfer resumes from `files.accept.have` when the connection returns,
and a recording stopped while the PC is offline waits in `Waiting` state and
transfers automatically once the PC reconnects.

## 3. Desktop storage & conversion

When a recording transfer (`recording: true`) completes on the PC:

1. **Folder & naming:** Saved in the user's configured Recordings folder
   (default `Documents\Nectarlink Recordings`) under the name the phone gave
   it, from when recording started (so one that waited for the PC keeps its
   time), or else the time it arrived: `Recording YYYY-MM-DD HH.MM.<ext>` (or `Recording YYYY-MM-DD HH.MM (2).<ext>`
   if that name is already taken; existing files are never overwritten).
2. **Format conversion:** Converted off the UI thread with Windows Media
   Foundation to the user's chosen format (`m4a` as recorded without
   re-encoding, `mp3`, `wav`, or `flac`). If conversion fails, the original
   `.m4a` is kept in the Recordings folder.
3. **Markers file:** When `markers` is non-empty, a companion UTF-8 text file
   `<stem>.markers.txt` (for example,
   `Recording 2026-10-07 14.32.markers.txt`) is written next to the audio file,
   one marker per line (`MM:SS.mmm  <label>` or `HH:MM:SS.mmm  <label>`;
   one without a label reads `Marker <n>`).
4. **Notification:** A Windows toast `"Recording from <phone> saved"` is shown
   with **Open** and **Show in folder** actions, and the transfer entry's saved
   path is updated to the final file in the Recordings folder.
