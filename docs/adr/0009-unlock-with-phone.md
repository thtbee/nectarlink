# ADR 0009: Unlock the PC with the phone

- **Status:** Proposed (needs the owner's decision and an external security review before any of it ships)
- **Date:** 2026-10-19

## Context

People want to unlock their locked PC by touching the fingerprint sensor on
their phone, the way Phone Link's "Dynamic Lock" locks it. `docs/PLAN.md`
§3.10 lists it as a dedicated, post-1.0 workstream because it puts code in the
most sensitive place on Windows: the sign-in screen.

Two questions decide the design:

1. **What does Windows accept at the lock screen?** Only a credential: a
   password, a smart-card certificate, or Windows Hello. A third-party app
   can't simply tell Windows "unlock".
2. **Where does that credential live, and who can get it?** Whatever we keep
   must be useless to someone who steals the PC's disk, malware running as the
   user, and anyone without the phone and its owner's fingerprint.

### Ways Windows allows it

| Approach | How | Fit |
|---|---|---|
| Custom credential provider + stored password | An `ICredentialProvider` DLL in LogonUI returns `KERB_INTERACTIVE_UNLOCK_LOGON` with the user's password | Works for local and Microsoft accounts that have a password. Most third-party "unlock with phone" tools do this. We must store the password, so how it's stored decides the security. |
| Windows Hello companion device framework (`SecondaryAuthenticationFactor`) | Microsoft's own API for companion devices | Microsoft lists it as deprecated; to verify on current Windows 11 before ruling it out entirely. Would avoid storing a password. |
| Certificate / virtual smart card logon | A credential provider presenting a certificate | Needs a domain and a certificate authority; not for home PCs. |

## Decision (proposed)

A credential provider that returns the user's password, where the password
can only be recovered with the phone and the user's fingerprint, and only
to unlock a session that's already signed in (not the first sign-in after a
restart).

### Split key

- **Enrollment** (the user turns it on, signed in, in Settings, and types
  their Windows password once):
  1. The PC makes a random 256-bit key `K` and encrypts the password with
     AES-256-GCM under `K`. Only that ciphertext is kept on the PC, bound
     (as associated data) to this PC, this phone and this Windows user.
  2. The PC sends `K` to the phone over the paired session. The phone wraps it
     with an AES-GCM key held in Android Keystore (StrongBox when there is
     one) that requires strong biometrics for every use and is invalidated
     when a new fingerprint is enrolled. The phone returns the wrapped `K`
     and keeps nothing else.
  3. The PC stores the wrapped `K` beside the ciphertext and forgets `K`.
- **Unlock**: the lock screen offers "Unlock with <phone>". The PC sends a
  request (single use, 30-second expiry, naming the PC and the user) with the
  wrapped `K`. The phone shows a biometric prompt naming the PC and the user;
  on success Keystore unwraps `K` and the phone returns it. The PC decrypts
  the password (the GCM tag proves `K` is right), hands it to LogonUI, and
  wipes `K` and the password from memory.

What each side holds:

| Holder | Has | Alone, it gets |
|---|---|---|
| PC disk | password ciphertext, wrapped `K` | nothing: `K` never touches the PC's disk, and the wrapping key never leaves the phone |
| Phone | the non-exportable wrapping key | nothing: it never sees the password |
| Network | an encrypted, mutually authenticated session (ADR 0003) | nothing |

### Guards

- **Only on the local network**, never over relays or mobile data, and only
  with a live session to the enrolled phone. Optionally, BLE proximity (the
  beacon already in `ble_beacon`).
- **Rate limit:** 5 failed attempts in 15 minutes locks the feature for 15
  minutes and notifies the user on both devices.
- **Revocable:** unpairing, turning the feature off, changing the Windows
  password (which makes the ciphertext useless; the PC asks to re-enroll),
  or a new fingerprint on the phone (the Keystore key is invalidated) all end
  it.
- **Unlock only:** the credential provider answers `CPUS_UNLOCK_WORKSTATION`,
  not `CPUS_LOGON`. Unlocking needs NectarLink running in the locked session,
  which also means a restart always asks for the real password once.
- **Off by default**, behind a clear explanation, and absent from builds until
  the review is done.

### Where the pieces run

- **Core (`nectarlink-core`, Rust):** the protocol, the vault (encrypting,
  decrypting, binding) and the request rules above. Pure code, fully tested.
  The first part of this is in `core/nectarlink-core/src/unlock.rs`.
- **Phone (Kotlin):** the Keystore key, the biometric prompt with a
  `CryptoObject`, and the protocol handler.
- **Credential provider (`desktop/credprov`, C++):** a small COM DLL in
  LogonUI. It knows nothing about phones: it asks the user's NectarLink, over
  a named pipe whose ACL admits only that user's NectarLink, for "the
  credential for user X", and serializes the answer. Kept small so it's easy
  to review.
- **Installation:** registering a credential provider is a machine-wide
  change (HKLM, administrator). The installer would do it only when the user
  turns the feature on, and remove it when it's off.

## Threat model

| Threat | Mitigation |
|---|---|
| Stolen or copied PC disk | Ciphertext without `K`; `K` is only on the phone, wrapped by a non-exportable key |
| Stolen phone | Needs the owner's biometric; a new fingerprint invalidates the key |
| Someone at the locked PC with a stolen, unlocked phone | Still needs the biometric for each unlock |
| Replay of an old response | Single-use request IDs, a 30-second expiry, and the session's own encryption |
| A different paired device answering | Responses only from the enrolled phone's identity, bound into the ciphertext's associated data |
| Malware running as the user on the PC | Out of scope for any unlock feature (it already has the session), but it must not learn the password: `K` and the password exist only in memory, briefly, inside NectarLink; the pipe to the credential provider admits only NectarLink. Residual risk: malware that reads NectarLink's memory at that moment. To be reviewed. |
| Bugs in the credential provider locking the user out | It only adds a tile; the normal password tile stays. Tested in a VM with recovery snapshots before any real machine. |
| Brute force via the lock screen | Rate limit and lockout above |

## Open questions for the owner

1. Is storing the password (split as above) acceptable, or should we first
   confirm whether Windows Hello's companion device API still works on
   Windows 11 24H2 and later?
2. PIN-only and passwordless Microsoft accounts have no usable password; we
   would tell those users it isn't available. Acceptable?
3. Who does the external review, and does the credential provider need code
   signing first (SignPath)?

## Consequences

- A security-critical component that needs its own tests, a VM test plan, and
  an external review before release.
- More install surface: a DLL registered machine-wide when enabled.
- The core and protocol parts can be built and tested now, independently of
  the credential provider.
