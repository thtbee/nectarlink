# ADR 0004: CBOR as the wire format

- **Status:** Accepted
- **Date:** 2026-10-05

## Context

Messages must be compact, forward-compatible (old and new app versions talk to each other) and implementable by third parties, without forcing a code-generation toolchain into every build.

## Decision

Encode messages as CBOR (RFC 8949) via serde. Unknown fields are ignored, unknown message types get an `Unsupported` reply. The schema is documented in `docs/protocol/`.

## Consequences

- No protoc/codegen step.
- Self-describing payloads are easy to debug.
- Schema discipline is manual: every change goes through the protocol spec and compatibility tests.
