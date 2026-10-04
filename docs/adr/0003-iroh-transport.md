# ADR 0003: iroh (QUIC) as the transport

- **Status:** Accepted
- **Date:** 2026-10-05

## Context

Connection reliability is the #1 complaint across Phone Link, KDE Connect and LocalSend: discovery failures, firewalls, AP isolation, same-network-only. Hand-rolling discovery, NAT traversal and multiplexing is where competitors break.

## Decision

Use iroh 1.x: devices are dialed by Ed25519 public key over QUIC, with LAN discovery, hole punching, relay fallback (over HTTPS), multipath and many streams per connection. Wrap it behind our own `Connection` abstraction.

## Consequences

- E2E encryption and mutual key authentication by default.
- Away-from-home works without port forwarding (relays opt-in, self-hostable).
- Young library (1.0 in June 2026); the abstraction keeps it swappable.
- USB path relies on iroh's custom transports (currently unstable) or a TCP-framed fallback.
