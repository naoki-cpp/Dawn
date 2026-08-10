---
adr       : ADR-0050
title     : Shared Versioned Peer Transport
status    : Proposed
date      : 2026-08-10
deciders  : Dawn maintainers
tags      : transport, replication, raft, durability, networking
---

# ADR-0050: Shared Versioned Peer Transport

## Context

Raft, event-log replication, catch-up, and snapshot transfer had separate TCP
listeners, framing, reconnect loops, queue policies, and configuration. That
made identity validation and operational limits inconsistent. A snapshot-sized
write could also compete with Raft traffic for the same physical resources.

Issue #280 requires one peer transport subsystem while preserving control/bulk
isolation and leaving domain semantics in their owning crates.

## Decision

Add `dawn-peer-transport` as a low-level DAG dependency. It owns:

- a versioned handshake carrying node identity, sector identity, channel, and
  capability bits;
- deterministic simultaneous-dial avoidance: the lower node ID dials and the
  higher node accepts;
- independent control and bulk TCP listeners, queues, reconnect loops, and
  frame limits;
- bounded `try_send` backpressure, malformed-frame rejection, and metrics;
- opaque metadata/payload frames. Domain crates remain responsible for
  serialization, authorization, fencing, staged durability, and application.

The first implementation is plaintext LAN transport. Identity matching is
configuration validation, not cryptographic authentication. TLS can be added
below this API without changing domain messages.

## Options considered

1. One interleaved TCP stream: rejected because a bounded frame still lets
   bulk work delay control traffic and complicates fairness.
2. One manager with separate control/bulk TCP channels: chosen. It preserves
   physical isolation without forcing unrelated domain protocols into one
   envelope.
3. Multiple typed channels per domain: rejected because lifecycle and
   handshake logic would be duplicated again.
4. QUIC: deferred until a milestone needs encrypted multiplexing or WAN
   migration; it would change deployment and testing assumptions now.

## Consequences

Raft and recovery adapters share the same connection lifecycle and observability
while remaining independently bounded. Recovery ranges, snapshots, and
repository payloads use the bulk channel; Raft, durability staging/receipts,
and catch-up requests use the control channel. A disconnected bulk channel
cannot block control sends. The transport does not make remote durability
receipts authoritative: callers must preserve owner epoch/term, fencing,
transition identity/position, range, and content hash and reject stale
evidence.

Issue #280's transport migration is complete. Deployment-specific fault
benchmarks and RTO selection remain follow-up work; runtime quorum activation
continues to be owned by #278 rather than this transport crate.
