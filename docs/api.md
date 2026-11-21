# NectarLink Local Authenticated Automation API (`docs/api.md`)

NectarLink for Windows includes an opt-in, loopback-only HTTP/JSON API (`127.0.0.1:<port>`, default port `53320`) designed for local desktop automation tools:

- **PowerToys Command Palette / PowerToys Run** (`tools/powertoys-extension`)
- **AutoHotkey / Raycast for Windows / Flow Launcher**
- **PowerShell & CLI scripts**

---

## 1. Enabling the API & Discovering the Token

The Local API is **off by default**. Enable it in **Settings → Local automation API** (or set `"localApiEnabled": true` in `%LOCALAPPDATA%\Nectarlink\desktop-settings.json`).

When enabled, NectarLink binds strictly to `127.0.0.1:<localApiPort>` (default `53320`) and writes `<data-dir>\api-token.json` (`%LOCALAPPDATA%\Nectarlink\api-token.json`):

```json
{
  "port": 53320,
  "token": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
}
```

When disabled, the listener stops immediately and `api-token.json` is removed.

---

## 2. Security Model

1. **Loopback-only binding**: The server binds to `127.0.0.1` and is never exposed to LAN interfaces.
2. **256-bit Bearer Token**: Every request must include:
   ```http
   Authorization: Bearer <64-hex-char-token>
   ```
   Tokens are compared in constant time. Missing or invalid tokens receive `401 Unauthorized` (`{"ok":false,"error":"unauthorized"}`).
3. **Browser Cross-Origin Protection**: Any HTTP request containing an `Origin` header (automatically attached by web browsers on cross-origin `fetch` / `XMLHttpRequest` calls) is rejected before routing with `403 Forbidden` (`{"ok":false,"error":"forbidden"}`).

---

## 3. Endpoints

### `GET /v1/devices`

Lists paired devices with connection state and battery status.

**Response (`200 OK`)**:
```json
{
  "ok": true,
  "devices": [
    {
      "id": "a1b2c3...",
      "name": "Pixel 9 Pro",
      "platform": "android",
      "online": true,
      "link": "lan",
      "battery": {
        "level": 84,
        "charging": false
      }
    }
  ]
}
```

---

### `POST /v1/send`

Sends files, a URL, or text to a paired phone (defaults to the first online or paired phone when `device` is omitted).

**Request Body**:
```json
{
  "device": "Pixel 9 Pro",
  "paths": ["C:\\Users\\Alice\\Pictures\\diagram.png"],
  "url": "https://nectarlink.dev",
  "text": "Meeting notes"
}
```

**Response (`200 OK`)**:
```json
{ "ok": true }
```

---

### `GET /v1/clipboard`

Returns the current PC clipboard text and up to 20 recent clipboard history entries.

**Response (`200 OK`)**:
```json
{
  "ok": true,
  "text": "Current clipboard text",
  "history": [
    {
      "id": "clip_00000022",
      "kind": "text",
      "preview": "Current clipboard text",
      "deviceName": "Pixel 9 Pro",
      "incoming": true,
      "timestamp": 1730000000,
      "pinned": false
    }
  ]
}
```

`history` holds the 20 most recent items, newest first. `kind` is `text` or
`image` (an image's `preview` is empty), `incoming` says whether it came from
`deviceName` or was sent to it, and `timestamp` is in Unix seconds.

---

### `POST /v1/clipboard`

Copies `text` to the PC clipboard and pushes it to the target phone.

**Request Body**:
```json
{
  "text": "Copied from script",
  "device": "Pixel 9 Pro"
}
```

**Response (`200 OK`)**:
```json
{ "ok": true }
```

---

### `POST /v1/notify`

Sends a task/alert notification to the paired phone.

**Request Body**:
```json
{
  "title": "Build completed",
  "body": "cargo test finished in 14s",
  "device": "Pixel 9 Pro"
}
```

**Response (`200 OK`)**:
```json
{ "ok": true }
```

---

### `POST /v1/ring`

Rings the paired phone ("Find my phone").

**Request Body**:
```json
{
  "device": "Pixel 9 Pro"
}
```

**Response (`200 OK`)**:
```json
{ "ok": true }
```

---

### `POST /v1/play-on-phone`

Play on phone: what one of this PC's players plays moves to the phone,
streamed from the PC, or for protected streaming services, opened in the
service's app while the PC pauses. The phone shows a notification to tap.

**Request Body** (optional):
```json
{
  "player": "Chrome"
}
```

Without `player`, the player that's playing (else the first one).

**Response (`200 OK`)**:
```json
{ "ok": true, "player": "Chrome" }
```

`404` with `"nothing_playing"` when no player is there, `409` with
`"no_phone"` when no phone is online.

---

### `POST /v1/camera`

Triggers Continuity Camera on the paired phone (`"photo"` or `"scan"`).

**Request Body**:
```json
{
  "mode": "scan",
  "device": "Pixel 9 Pro"
}
```

**Response (`200 OK`)**:
```json
{
  "ok": true,
  "mode": "scan"
}
```

---

### `GET /v1/timeline`

Returns recent cross-device timeline items. Accepts optional query parameters `?limit=20&q=<filter>`.

**Response (`200 OK`)**:
```json
{
  "ok": true,
  "items": [
    {
      "id": 42,
      "kind": "file",
      "direction": "incoming",
      "deviceId": "a1b2c3...",
      "timestamp": 1730000000,
      "title": "notes.txt",
      "detail": "File",
      "target": "C:\\Users\\you\\Downloads\\Nectarlink\\notes.txt",
      "sizeBytes": 5
    }
  ]
}
```

`kind` is one of `file`, `clip`, `link`, `photo`, `recording` or `session`.
`timestamp` is in Unix seconds, and `target` is the saved path (several are
separated by newlines) or the link.

### Notifications and messages

These two are off even when the API is on. Turn on **Notifications and
messages** under **Settings → Local automation API** (`"localApiPrivate": true`
in `desktop-settings.json`). Until then both answer `403`
(`{"ok":false,"error":"not_allowed"}`). Both take an optional `?device=` (an
ID, the start of one, or part of a device's name); a device that isn't paired
answers `404` (`unknown_device`).

### `GET /v1/notifications`

The phone notifications shown on this PC, newest first, with ongoing ones
(calls, timers, downloads) first as in the feed. Apps hidden in NectarLink stay
out. Optional `?limit=20` (1 to 100).

**Response (`200 OK`)**:
```json
{
  "ok": true,
  "notifications": [
    {
      "id": "0|com.whatsapp|1|null|10123",
      "deviceId": "a1b2c3...",
      "app": "com.whatsapp",
      "appName": "WhatsApp",
      "title": "Sam",
      "text": "On my way",
      "sub": null,
      "timestamp": 1730000000,
      "silent": false,
      "ongoing": false
    }
  ]
}
```

### `GET /v1/messages`

Conversations, newest first, with unread counts: SMS and MMS from each
connected Android phone (`"kind": "sms"`), and chats from notifications such as
WhatsApp or Signal kept on this PC (`"kind": "chat"`). A phone that doesn't
answer within 5 seconds leaves its SMS out.

**Response (`200 OK`)**:
```json
{
  "ok": true,
  "unreadTotal": 3,
  "threads": [
    {
      "id": "42",
      "deviceId": "a1b2c3...",
      "kind": "sms",
      "app": "",
      "appName": "SMS",
      "title": "Sam",
      "group": false,
      "lastMessage": "See you at 6",
      "unread": 2,
      "timestamp": 1730000000
    }
  ]
}
```

---

## 4. Examples

### PowerShell

```powershell
$cfg = Get-Content "$env:LOCALAPPDATA\Nectarlink\api-token.json" | ConvertFrom-Json
$headers = @{ Authorization = "Bearer $($cfg.token)" }

# List paired devices
Invoke-RestMethod -Uri "http://127.0.0.1:$($cfg.port)/v1/devices" -Headers $headers

# Ring your phone
Invoke-RestMethod -Method Post -Uri "http://127.0.0.1:$($cfg.port)/v1/ring" `
  -Headers $headers -ContentType "application/json" -Body '{}'
```

### curl

```bash
TOKEN=$(jq -r .token "$LOCALAPPDATA/Nectarlink/api-token.json")
PORT=$(jq -r .port "$LOCALAPPDATA/Nectarlink/api-token.json")

curl -s -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$PORT/v1/devices"
curl -s -X POST -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"mode":"photo"}' "http://127.0.0.1:$PORT/v1/camera"
```
