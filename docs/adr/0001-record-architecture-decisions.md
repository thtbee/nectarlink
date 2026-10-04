# ADR 0001: Record architecture decisions

- **Status:** Accepted
- **Date:** 2026-10-05

## Context

We need a lightweight, durable record of why big decisions were made, so contributors don't re-litigate them and we can revisit them deliberately.

## Decision

Use Architecture Decision Records (ADRs) in `docs/adr/`, one numbered Markdown file per decision, with the sections Context, Decision, Consequences. Superseded ADRs stay in place and link to their replacement.

## Consequences

- Decisions are discoverable next to the code.
- Changing a decision means writing a new ADR, not editing history.
