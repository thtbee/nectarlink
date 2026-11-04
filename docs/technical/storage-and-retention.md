# Storage and Retention

NectarLink persists all relational state in an embedded SQLite database (`nectarlink.db`) managed by `core/nectarlink-core/src/store.rs`, supplemented by a `KeyProtector`-encrypted clipboard vault (`core/nectarlink-core/src/clipboard_history.rs`) and bounded desktop notification state files (`desktop/app/src/notification_store.rs`).

## Data Directory Layout

- **Windows (`%LOCALAPPDATA%\Nectarlink`)** or custom `--data-dir`
- **Android (`context.filesDir`)**

```text
<data_dir>/
├── identity.key                  Ed25519 secret key (NLK1 header + DPAPI/Keystore ciphertext)
├── nectarlink.db                 SQLite database (WAL mode, schema v8)
├── clipboard_history.enc         KeyProtector-encrypted clipboard index (NLCB\x01 header)
├── clipboard_images/
│   └── <clip_id>.enc             KeyProtector-encrypted PNG/image payloads (NLCI\x01 header)
├── incoming/
│   └── <peer_id>/<transfer_id>/  In-flight .part files for resumable transfers
├── queue_staging/
│   └── <queue_id>/               Staged copies of ephemeral Android ContentResolver FDs
├── notification-rules.json       Per-app notification mute/banner rules (desktop)
├── notification-history.json     24-hour notification history cache (desktop)
└── api-token.json                256-bit Bearer token for opt-in localhost HTTP API (desktop)
```

## SQLite Schema (`SCHEMA_VERSION = 7`)

Configured in `core/nectarlink-core/src/store.rs` with `PRAGMA journal_mode = WAL`, `PRAGMA foreign_keys = ON`, and `PRAGMA user_version = 7`. All 7 tables use SQLite `STRICT` typing:

| Table | Primary Key | Purpose | Retention & Bounding Policy |
|---|---|---|---|
| `meta` | `key TEXT` | Local device name (`device_name`) and global key-value settings | Permanent per installation |
| `peers` | `id BLOB` | Paired devices (32-byte Ed25519 `id`, `name`, `kind`, `paired_at_ms`, `last_seen_ms`, `last_addrs`) | Removed immediately upon `unpair` (`ON DELETE CASCADE` cleans child rows) |
| `peer_toggles` | `(peer, toggle)` | Per-device capability toggle overrides (`peer BLOB`, `toggle TEXT`, `enabled INTEGER`) | Cascade-deleted when parent `peers` row is removed |
| `timeline` | `id INTEGER` | Unified cross-device activity log (`ts_ms`, `peer BLOB`, `kind`, `direction`, `summary`, `payload BLOB`) | Indexed by `ts_ms DESC`; supports substring filtering (`search_timeline`) and clear actions |
| `chat_threads` | `(peer, thread_id)` | Unified `MessagingStyle` notification conversations (`app_pkg`, `app_name`, `conv_key`, `title`, `is_group`, `last_sender`, `last_text`, `last_ts_ms`, `unread_count`, `reply_notif_id`, `updated_at_ms`) | Pruned after 90 days (`CHAT_RETENTION_MS`) and capped at `MAX_CHAT_THREADS_PER_PEER = 200` |
| `chat_messages` | `id INTEGER` | Individual messages within `chat_threads` (`peer BLOB`, `thread_id`, `msg_key`, `sender`, `is_self`, `text`, `ts_ms`, `notif_id`) | Unique on `(peer, thread_id, msg_key)`; capped at `MAX_CHAT_MESSAGES_PER_THREAD = 200` and pruned with parent thread |
| `send_queue` | `id TEXT` | Persistent bidirectional FIFO outbound delivery queue (`seq`, `peer BLOB`, `kind`, `status`, `created_at_ms`, `expires_at_ms`, `retry_count`, `next_retry_ms`, `last_error`, `total_bytes`, `sent_bytes`, `transfer_id`, `summary`, `payload_json`) | Active items capped at `200` items / `10 GiB`; terminal history pruned at `100` rows (`MAX_QUEUE_HISTORY_ITEMS`) |

### Schema Migrations (`Store::migrate`)

`Store::open` inspects `PRAGMA user_version` inside a transaction and applies incremental migrations (`v1` → … → `v8`) non-destructively using `CREATE TABLE IF NOT EXISTS` and column checks before setting `PRAGMA user_version = 8`. Version 8 added `peers.install_id`: a salted hash of the other device's Android ID or Windows machine GUID, used only to replace the old entry when the same device is paired again (`cleanup_peers` merges by install ID and device kind, never by name).

## Unified Conversations (`chat_threads` & `chat_messages`)

When an Android notification uses `Notification.MessagingStyle` (such as WhatsApp, Signal, Telegram, Messages, Discord, or Slack), `NotificationReader.kt` extracts the conversation metadata, participant messages, and `RemoteInput` reply capability.

- `Store::upsert_chat_from_notification` upserts the thread header in `chat_threads` and deduplicates message turns in `chat_messages` by `(peer, thread_id, msg_key)`.
- When the user replies from the desktop **Conversations** view (`chat.send`), the outgoing reply is appended with `is_self = 1` and routed to the Android `RemoteInput` pending intent.
- Automatic maintenance (`prune_chat_tables`) enforces:
  - **Age limit**: `CHAT_RETENTION_MS = 90 * 86_400 * 1,000` (90 days).
  - **Thread cap**: `MAX_CHAT_THREADS_PER_PEER = 200` most recent threads per device.
  - **Message cap**: `MAX_CHAT_MESSAGES_PER_THREAD = 200` most recent messages per thread.

## Desktop Notification History (`desktop/app/src/notification_store.rs`)

In addition to SQLite conversation threads, the desktop app maintains fast JSON-backed stores for live toast rules and recent notifications:

- `notification-rules.json`: Stores `Rules { history: bool, apps: BTreeMap<String, AppEntry> }` where each `AppEntry` maps `<package_name>` to `AppRule` (`Show`, `Quiet`, or `Hidden`).
- `notification-history.json`: Retains up to `MAX_HISTORY = 300` history entries (alongside up to `MAX_NOTIFICATIONS = 200` active notifications in memory) and automatically prunes entries older than 24 hours (`HISTORY_RETENTION_MS = 24 * 60 * 60 * 1000`) on load and save.

## Persistent Bidirectional Send Queue (`send_queue`)

Implemented across `core/nectarlink-core/src/store.rs`, `core/nectarlink-core/src/node.rs`, and `core/nectarlink-core/src/transfer.rs`.

Whenever a user sends files, folders, links, clipboard payloads, voice recordings, handoff documents, task alerts, or camera requests while the target device is offline (or when the connection drops mid-operation), NectarLink enqueues the action into `send_queue` and delivers it automatically when a session is established.

### Queue Item Kinds and Default TTLs (`QueueKind`)

| `QueueKind` | Wire Value | Default TTL | Description |
|---|---|---|---|
| `QueueKind::Files` | `"files"` | 7 days (`604,800,000 ms`) | One or more files (`QueuePayload::Files`) |
| `QueueKind::Folder` | `"folder"` | 7 days (`604,800,000 ms`) | Directory tree with relative paths (`QueuePayload::Files { is_folder: true }`) |
| `QueueKind::HandoffDocument` | `"handoff_document"` | 7 days (`604,800,000 ms`) | Document file + reading position (`page`, `scroll_y`) |
| `QueueKind::Recording` | `"recording"` | 7 days (`604,800,000 ms`) | Voice or audio recording file |
| `QueueKind::Clipboard` | `"clipboard"` | 24 hours (`86,400,000 ms`) | Text clipboard payload (`QueuePayload::ClipboardText`) |
| `QueueKind::ClipboardImage` | `"clipboard_image"` | 24 hours (`86,400,000 ms`) | Image clipboard payload (`QueuePayload::ClipboardImage`) |
| `QueueKind::Link` | `"link"` | 24 hours (`86,400,000 ms`) | URL + optional title (`QueuePayload::Link`) |
| `QueueKind::TaskNotify` | `"task_notify"` | 24 hours (`86,400,000 ms`) | Long-running command/task completion alert (`QueuePayload::TaskNotify`) |
| `QueueKind::CameraRequest` | `"camera_request"` | 2 minutes (`120,000 ms`) | Interactive remote camera capture request (`QueuePayload::CameraRequest`) |

### Queue States (`QueueStatus`)

```text
           enqueue_item()
                 │
                 ▼
            [ waiting ] ◄─────────────── retry_send_queue_item()
             │       │                   or reset_interrupted_sending()
  TTL passed │       │ peer online &
             │       │ next_retry_ms <= now
             │       ▼
             │  [ sending ]
             │   │       │
             │   │ ok    │ transport drop / integrity error / max attempts
             │   ▼       ▼
             │ (deleted) [ failed ]
             ▼
         [ expired ]
```

- Successfully delivered items are deleted from `send_queue` (`complete_send_queue_item`) and recorded in `timeline` as completed transfers.
- Failed or expired items remain visible in the UI (`failed` / `expired`) so the user can inspect the error reason, click **Retry**, or remove them; completed/failed/expired history is capped at `MAX_QUEUE_HISTORY_ITEMS = 100`.

### Ordering, Replacement, and Capacity Guarantees

1. **Strict Per-Peer FIFO Sequencing**: Every item receives a monotonically increasing `seq` number (`idx_send_queue_peer_fifo` on `(peer, status, seq ASC)`). Items for a given peer are drained strictly in `seq` order, while queues for different peers drain independently in parallel.
2. **In-Place Clipboard Replacement**: If a peer already has a `waiting` item with `kind IN ('clipboard', 'clipboard_image')`, copying a new text or image clip replaces the existing row in place and preserves its `seq` slot instead of piling up stale clipboard states. Previously staged temporary clipboard image files are cleaned up before replacement.
3. **Capacity Limits**:
   - Maximum active (`waiting` + `sending`) items across the queue: `MAX_QUEUE_ACTIVE_ITEMS = 200`.
   - Maximum active file bytes across the queue: `MAX_QUEUE_ACTIVE_BYTES = 10 GiB` (`10 * 1024 * 1024 * 1024` bytes).
   - Maximum queued clipboard image size: `MAX_QUEUE_CLIPBOARD_IMAGE_BYTES = 50 MiB` in `store.rs` (`clip::MAX_IMAGE_BYTES = 32 MiB` on the wire).
4. **Pre-Send File Integrity Verification (`QueuedFileSpec::verify_integrity`)**: When a file is enqueued, `Store` records its `size` and last-modified timestamp (`mtime_ms`). Immediately before opening the QUIC stream, `verify_integrity` re-stats the path; if the file was deleted, truncated, or modified after queueing, the item transitions to `QueueStatus::Failed` with a descriptive error rather than streaming corrupt or unintended data.
5. **Ephemeral File Descriptor Staging (`stage_file_for_queue`)**: On Android, files shared from `content://` URIs may be backed by transient `ContentResolver` file descriptors that close when the share activity finishes. `Node` copies transient inputs into `<data_dir>/queue_staging/<queue_id>/<idx>_<sanitized_name>` (`staged: true`) and deletes the staging directory (`cleanup_staging_id`) as soon as the item completes, is canceled, or is pruned.
6. **Crash Recovery and Zero-Polling Coordinator**:
   - On startup, `Store::reset_interrupted_sending` resets any item left in `'sending'` state by a previous process crash back to `'waiting'` (`next_retry_ms = 0`).
   - The background queue coordinator (`node.rs`) never busy-polls SQLite: it computes the exact next wakeup deadline (`Store::next_queue_wake_ms`) across pending retry timers and TTL expirations, and awaits `std::future::pending()` when the queue is empty until woken by `queue_notify` (triggered on enqueue, retry, or peer session connection).

### Sealed Queue Content (`QueueSeal`)

Clipboard text and images, links and task alerts in `send_queue` are sealed with the platform `KeyProtector` before they reach SQLite: `payload_json` holds `sealed1:` and base64, `payload_blob` holds sealed bytes, and `title` and `detail` hold a plain label, with the real ones worked out when the row is read. Rows from older versions are sealed at startup, followed by `VACUUM`, and `secure_delete` is on. These kinds are deleted when they expire rather than kept as `expired`. Clearing clipboard history removes waiting clips, and clearing everything also removes waiting links and alerts. File items keep their names and paths in the clear, and stay for Retry.

## Encrypted Clipboard History (`clipboard_history.rs`)

Recent clipboard items are persisted in `<data_dir>/clipboard_history.enc` and `<data_dir>/clipboard_images/<id>.enc`:

- **At-Rest Protection (`KeyProtector`)**: Both the CBOR index (`clipboard_history.enc`, prefixed with 5-byte magic header `NLCB\x01`) and each image file (`clipboard_images/<id>.enc`, prefixed with 5-byte magic header `NLCI\x01`) are sealed directly via `KeyProtector::seal` and `KeyProtector::open` (`DpapiKeyProtector` using Windows DPAPI tied to the user logon credential on Windows, or `KeystoreKeyProtector` using AES-256-GCM inside Android Keystore on Android).
- **Atomic Writes**: Updates write to `.tmp` files (`clipboard_history.enc.tmp` and `<id>.enc.tmp`) and atomically rename them into place.
- **Retention Limits**:
  - Up to `MAX_ENTRIES = 50` entries.
  - Automatic 7-day expiration (`RETENTION_SECS = 7 * 24 * 60 * 60`).
  - Maximum single image size: `MAX_IMAGE_BYTES = 16 MiB`; maximum total image vault size: `MAX_TOTAL_IMAGE_BYTES = 128 MiB`.
- **Sensitive Content Filtering**: Clips flagged as sensitive (`ClipMeta::is_sensitive`, such as password manager entries with `ClipboardDescription` / `ExcludeClipboardContentFromMonitorProcessing`) or one-time passcodes (OTPs) are excluded from persistent history.
