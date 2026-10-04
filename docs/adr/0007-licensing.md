# ADR 0007: Licensing

- **Status:** Accepted
- **Date:** 2026-10-05

## Context

We want forks of the apps to stay open, the protocol and core to be easy to adopt, and a future App Store release to be possible.

## Decision

Apps: GPL-3.0-or-later with an app-store additional permission. Core libraries: MPL-2.0. Docs and protocol spec: CC BY 4.0. Name and logo: trademark policy. Contributions: DCO sign-off.

## Consequences

- The app-store permission must exist from the first commit to cover all contributions.
- GPL-only Qt modules must be avoided (see ADR 0005).
- The exception text needs legal review before the first public release.
