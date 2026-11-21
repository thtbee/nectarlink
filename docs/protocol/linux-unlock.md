# Linux session unlock v1

A Linux desktop advertises `unlock.linux.pc`, and an Android phone with
biometric signing support advertises `unlock.linux.key`. Enrollment and signing
require an authenticated paired session, the per-device `unlock` toggle on both
sides, and a selected direct LAN or USB path. An inactive USB path, an internet
address, a tailnet, and a relay do not qualify. Peers using the Windows password
vault continue to use [its existing wire messages](unlock.md).

## Requests and replies

Requests use the normal envelope ID and reply correlation. The fields below
are CBOR values in the body. Device IDs are 32-byte strings. UIDs must be
nonzero.

| Message | Direction | Body and response |
| --- | --- | --- |
| `unlock.linux.v1.enroll` | PC → phone | `pc`, `phone`, `uid` (unsigned 32-bit), and `user_name` (1–128 UTF-8 bytes without control characters). The phone displays the PC's paired name and confirms the account with its owner. |
| `unlock.linux.v1.enrolled` | phone → PC | `public_key` is a 65-byte uncompressed P-256 point (0x04, 32-byte X, 32-byte Y). The local administrator decides whether to trust it. |
| `unlock.linux.v1.sign` | PC → phone | `pc`, `phone`, `uid`, and `nonce` (32-byte string). The phone uses the account label it confirmed during enrollment and asks for a fresh biometric. |
| `unlock.linux.v1.signed` | phone → PC | `signature` is the DER-encoded ECDSA P-256/SHA-256 signature. |
| `unlock.linux.v1.cancel` | PC → phone | Unsigned 64-bit request ID of the pending enrollment or signing attempt on this same session. This one-way message has no envelope ID or reply. |
| `unlock.linux.v1.forget` | PC → phone | `{"user": {"uid": …}}` removes one account key, or the CBOR text `"all"` removes all the authenticated PC's keys. The phone replies with `ok` after the platform processes removal. |

Invalid requests, unsupported capabilities, missing keys, and a denied
biometric return an error. The phone compares both device IDs to the session's
authenticated PC and its own identity before opening a prompt. The signing
request carries no display labels. The phone keeps one Linux biometric prompt
active at a time and backs off after repeated refusals or timeouts. Both sides
recheck consent, pairing, capabilities, and path after a successful response.
They discard a pending attempt if the connection leaves the selected local
path for a nonlocal route or loses its path event history. The PC also cancels
when its local caller disconnects or the request times out. The phone closes
its biometric prompt and discards the staged key, leaving any previous
enrollment intact. A cancel cannot affect requests on a different session or
remove an existing key.
Turning the feature off or unpairing clears the phone's keys when the phone
receives the change. Administrator removal revokes the local PAM enrollment
even when the phone is offline.

## Signed transcript

Concatenate these fields in order, without a length prefix or CBOR encoding.

1. ASCII `nectarlink/linux-unlock/v1` followed by one NUL byte.
2. The PC ID as 32 raw bytes.
3. The phone ID as 32 raw bytes.
4. The UID as four unsigned big-endian bytes.
5. The nonce as 32 raw bytes.

The PAM verifier owns the random nonce, the 30-second monotonic deadline,
and the enrolled public key. It verifies the signature against the exact
challenge and checks that the enrollment is still current before accepting
the response for that transaction.
