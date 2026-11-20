# Problem reports (v1)

> Status: **draft.** Builds on [protocol v0](v0.md); breaking changes are
> allowed until v1. License: CC BY 4.0.

A device making a problem report (**Report a problem** on either app) asks
each connected paired device for its part, so one report covers both ends.
Messages travel on the session's control stream ([protocol v0](v0.md) §4).

## 1. Capability

| Capability | Offered by | Meaning |
|---|---|---|
| `diag.report` | Both | Answers `diag.ask` |

A device without it isn't asked; the report then holds only what it shared
when pairing.

## 2. Asking

```
t = "diag.ask"   id = n   { log, just_now, nonce }
t = "diag.part"  re = n   { facts, checks, health, crashes, log }
t = "diag.log"            { ask, declined?, lines }      // later, only with log
```

**`diag.ask`** asks for the device's part. `just_now` asks for the log of
the last few minutes (plus warnings and errors from the last day) rather
than the newest lines. `nonce` is a random number echoed in `diag.log`.

**`diag.part`** comes back at once, with no one asked:

| Field | Holds |
|---|---|
| `facts` | `app_version`, `channel`, `install`, `os`, `arch`, `hardware` (a phone's maker and model, a PC's graphics), `extra` (`[name, value]` from fixed lists: battery saver, standby bucket) |
| `checks` | `[check, outcome]`: notification access, battery restrictions, permissions, how often Android ended the app in the background |
| `health` | Up to 14 days of feature outcome counts |
| `crashes` | Up to 5 crash records: signature, kind, where in NectarLink's code, the fixed part of the message, version, time |
| `log` | `not_asked`, `asking` (its user is being asked) or `busy` (asked less than a minute ago, so not asked again) |

None of it holds names, messages, files, numbers or addresses.

**`diag.log`** follows `asking`, only after the user of the device that was
asked taps **Share** (or **Don't share**: `declined`, no lines). A device
asks its user at most once a minute per asking device, and the request
lapses after a minute. A `diag.log` whose `ask` doesn't match a request in
flight to that device is dropped. The lines are the same private activity
log the device keeps for itself ([privacy](../guide/privacy-and-security.md)).

## 3. Limits

A receiver keeps at most 32 checks, facts and hardware lines, 14 days of
health, 5 crashes, and 200 characters per value; at most 400 log lines and
24 KiB of log, newest kept, one line each. It treats every value as text
from the other device: folders and addresses in it are hidden as in its
own log.

Errors: `UNSUPPORTED` when the device doesn't answer for reports.
