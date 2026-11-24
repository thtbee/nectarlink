# ADR 0009: Unlock the PC with the phone

- **Status:** Accepted (2026-11-18)
- **Date:** 2026-10-19, decisions 2026-11-18

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
  1. The PC first seals the password to the machine key (below), so that
     only SYSTEM can open it. Then it makes a random 256-bit key `K` and
     encrypts that sealed password with AES-256-GCM under `K`. Only that
     ciphertext is kept on the PC, bound (as associated data) to this PC,
     this phone and this Windows user.
  2. The PC sends `K` to the phone over the paired session. The phone wraps it
     with an AES-GCM key held in Android Keystore (StrongBox when there is
     one) that requires strong biometrics for every use and is invalidated
     when a new fingerprint is enrolled. The phone returns the wrapped `K`
     and keeps nothing else.
  3. The PC stores the wrapped `K` beside the ciphertext and forgets `K`.
- **Unlock**: the lock screen offers "Unlock with <phone>". The PC sends a
  request (single use, 30-second expiry, naming the PC and the user) with the
  wrapped `K`. The phone shows a biometric prompt naming the PC and the user;
  on success Keystore unwraps `K` and the phone returns it. The PC opens the
  ciphertext (the GCM tag proves `K` is right) and hands the sealed password
  to the credential provider, which opens it as SYSTEM inside LogonUI and
  wipes it once serialized. The app never holds the password itself after
  enrolling.

### The machine key

Turning the feature on (the one administrator prompt) makes an RSA-3072 key
in the machine's key store, not exportable, that only SYSTEM and
administrators may use, and publishes its public half under the provider's
registry key. The app seals the password to it with RSA-OAEP (SHA-256), in
chunks whose OAEP label names the Windows user and the chunk's place, so a
blob made for one user, or with its chunks reordered, doesn't open. Only the
credential provider, running as SYSTEM, opens it. Turning the feature off
deletes the key. So malware running as the user that gets `K` (by asking the
phone's owner, or from NectarLink's memory) still has only a blob it can't
open.

What each side holds:

| Holder | Has | Alone, it gets |
|---|---|---|
| PC disk | password ciphertext, wrapped `K` | nothing: `K` never touches the PC's disk, and the wrapping key never leaves the phone |
| The user's processes (malware running as the user included) | the above, and `K` after an approved unlock | the password sealed to the machine key, which only SYSTEM can open |
| Phone | the non-exportable wrapping key | nothing: it never sees the password |
| Network | an encrypted, mutually authenticated session (ADR 0003) | nothing |

### Guards

- **Only on the local network**, never over relays or mobile data, and only
  with a live session to the enrolled phone: the USB cable, or a direct path
  whose address is on this network: private or link-local, or one the other
  device announced over mDNS here that is also on one of this device's own
  subnets. Tailnet and CGNAT (`100.64.0.0/10`) addresses don't count, since
  they reach a phone on mobile data too, and neither does an address announced
  here once but reached later from elsewhere. Away mode doesn't widen it, and a
  session with no selected path is refused. Both sides check before the phone
  asks its owner, and the PC checks again before opening the password.
  Optionally, BLE proximity (the beacon already in `ble_beacon`).
- **Rate limit:** 5 failed attempts in 15 minutes locks the feature for 15
  minutes, and the tile says so. A failed attempt is a key that doesn't open
  the password, an answer from the wrong device, or a prompt the phone's owner
  declined or didn't answer, so the lock screen can't keep asking. The phone
  keeps its own count per PC too, so a PC that skips its limit still can't
  keep prompting. The prompt names the PC as the phone knows it, not as the
  request says.
- **Revocable:** unpairing, turning the feature off, changing the Windows
  password (which makes the ciphertext useless; the PC asks to re-enroll),
  or a new fingerprint on the phone (the Keystore key is invalidated) all end
  it. An unlock already waiting on the phone ends too: the PC opens the
  password only if the same enrollment is still there. When Windows refuses
  the password the tile handed it (wrong password, not a locked account), the
  tile tells NectarLink over the pipe, which forgets that user's enrollment and
  asks to set it up again, so an old password isn't tried again and again under
  an account lockout policy. A forget message that doesn't parse, or names an
  invalid user, is ignored rather than read as "everyone".
- **Unlock only:** since Windows 10 the lock screen mostly asks providers for
  `CPUS_LOGON` (sign-in and unlock were merged; `CPUS_UNLOCK_WORKSTATION` only in
  some setups), so the provider takes both but offers its tile only to the account
  already signed in to this session (WTS session user), and only while that
  user's NectarLink serves the pipe. At the first sign-in after a restart nobody
  is signed in, so there's no tile; the app also answers only while the session
  is locked. A restart always asks for the real password once: when Windows
  signs in by itself after an update and locks the session ("use my sign-in
  info to finish setting up"), NectarLink answers only once the session has
  been unlocked some other way since it started.
- **The pipe:** only SYSTEM may open it, and OWNER RIGHTS keeps the user who
  owns it from rewriting that. The tile talks only to a server that is the
  NectarLink next to its DLL, running as the session's account in that session,
  since pipe names are machine-wide and another account could otherwise serve
  this session's pipe.
- **Off by default**, behind a clear explanation.
- **Asked for, never by itself:** picking the tile doesn't ask the phone; the
  arrow does. Windows remembers the last sign-in option, so otherwise any key
  pressed at the lock screen would make the phone ask its owner.
- **Starting on the phone:** the paired and enrolled phone can request an unlock
  while the existing Windows session is locked. The PC keeps one intent for
  ten seconds, tied to the requesting phone. The sign-in provider can consume
  it once and start the same biometric challenge for that phone.
- **Only for accounts set up with a phone:** NectarLink serves the pipe only
  once its user has set up a phone, and the tile shows only while the pipe is
  served, so other accounts on a PC where the provider is registered never see
  it.

### Where the pieces run

- **Core (`nectarlink-core`, Rust):** the protocol, the vault (encrypting,
  decrypting, binding) and the request rules above. Pure code, fully tested.
  The first part of this is in `core/nectarlink-core/src/unlock.rs`.
- **Phone (Kotlin):** the Keystore key, the biometric prompt with a
  `CryptoObject`, and the protocol handler.
- **Credential provider (`desktop/credprov`, Rust):** a small COM DLL in
  LogonUI. It knows nothing about phones: it asks the user's NectarLink, over
  a named pipe whose ACL admits only that user's NectarLink, for "the
  credential for user X", opens the sealed password it gets with the machine
  key, and serializes it. Kept small so it's easy
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
| Malware running as the user on the PC | Out of scope for any unlock feature (it already has the session), but it must not learn the password: the app keeps it only sealed to a machine key only SYSTEM can use, so even `K` (from an approved prompt, or from memory) yields nothing it can open. It can still make the phone ask, which the phone's own per-PC limit caps. |
| Bugs in the credential provider locking the user out | It only adds a tile; the normal password tile stays. Tested in a VM with recovery snapshots before any real machine. |
| Brute force via the lock screen | Rate limit and lockout above |

## Decisions (2026-11-18)

1. **Store the password, split as above.** Windows Hello's companion device
   framework is ruled out: every `SecondaryAuthenticationFactor` API is marked
   deprecated ("might not work on all platforms"), and it's gated: Microsoft
   must approve each app first (cdfonboard@microsoft.com), or calls fail. The
   one shipping open-source tool for this, PulseUnlock (GPL-3.0, formerly PC
   Bio Unlock), uses the same split: AES-256-GCM on the PC, the key only on the
   phone, a one-time challenge per unlock.
2. **PIN-only and passwordless Microsoft accounts** are told it isn't
   available for them. Settings explains what it needs (a password; and with
   "only allow Windows Hello sign-in for Microsoft accounts" on, Windows hides
   other sign-in options).
3. **The credential provider is written in Rust** (`windows` crate,
   `#[implement]` of `ICredentialProvider` and `ICredentialProviderCredential`),
   like the rest of the PC app: no C++ inside LogonUI. Every exported entry
   point catches panics.
4. **The pipe admits only SYSTEM** (LogonUI's account), never the user: malware
   running as the user must not be able to ask for the password. The app also
   checks the caller is LogonUI in its own session and that the session is
   locked before it asks the phone.
5. **The DLL lives where only administrators can write** (the install folder
   under Program Files, or System32): a DLL that SYSTEM loads from a
   user-writable folder would let any user process become SYSTEM. Enabling the
   feature copies and registers it with one administrator prompt; turning it
   off removes it.
6. **It ships in builds**, behind the `unlock` toggle (off by default), and the
   PC doesn't offer `unlock.pc` until the credential provider is installed.
   Code signing stays open.

## Implementation (2026-11-18)

- Core: `unlock.rs` (vault, request rules, both sides' messages), the
  `unlock_vault` table, platform hooks `unlock_wrap` / `unlock_unwrap` /
  `unlock_forget`, a two-device test covering every rule.
- Phone: `unlock/UnlockKeys.kt` (Keystore key per PC and user, StrongBox when
  present, per-use strong biometric, invalidated by a new fingerprint) and
  `unlock/UnlockPrompts.kt` (BiometricPrompt with a CryptoObject).
- Credential provider: `desktop/credprov` (Rust; only for the account signed in to the locked session; the
  pipe client checks the server is the NectarLink next to the DLL and connects
  at identification level; 40 s cap; panics caught). Its tile carries a picture
  (Windows 10 and later list a provider under "Sign-in options" by it), is never
  the default (the user picks it), and asks the phone as soon as it's picked,
  once per lock screen; its arrow asks again. What LogonUI asked and what it answered go to
  the Application event log (source "NectarLink Unlock"), never names, SIDs or
  passwords.
- PC app: `win/unlock.rs` (registration, the SYSTEM-only pipe answering
  LogonUI in this session while it's locked, enrolling with a Windows password
  check). The app runs as the user and can't open LogonUI's process, so it
  checks the caller from what it can see: the client's token (read while
  impersonating it at identification level) is SYSTEM, it's in this session,
  and its program is `logonui.exe`. The PC offers `unlock.pc` only while the provider is registered.

## Test plan before release

1. A spare Windows 11 machine or VM with a local account and with a Microsoft
   account: register, enroll, lock (Win+L), unlock with the phone; decline on
   the phone; phone away; wrong key (tampered vault row); five failures; change
   the Windows password; add a fingerprint on the phone; uninstall (the tile
   goes).
2. Confirm the password tile and PIN keep working throughout, and that a
   crash in the provider (forced in a debug build) doesn't block sign-in.
3. Process Monitor / handle checks: the password isn't written anywhere; the
   pipe refuses a non-SYSTEM client and a client outside LogonUI.

## Consequences

- A security-critical component that needs its own tests and a VM test plan.
- More install surface: a DLL registered machine-wide when enabled.
- The core and protocol parts can be built and tested now, independently of
  the credential provider.
