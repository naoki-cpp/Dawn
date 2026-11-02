---
id      : ADR-0056
title   : Invited disposable single-node playtest on Render
status  : accepted
date    : 2026-11-02
deciders: [human, ai-agent]
related : ADR-0048 (ResumeTicket remains a separate capability), ADR-0049 (production durability is unchanged), ADR-0051 (existing server composition)
---

# ADR-0056 - Invited disposable single-node playtest on Render

## Context

The user approved a small deployment: one Render Free web service, WSS,
individual tester invites, and disposable data. The LAN prototype's lack of
transport authentication is not appropriate for an internet endpoint.

## Decision

- Deploy only `simulate --serve --ships 5 --pop-cap 8`, not a Sector cluster.
- With `PORT` set, bind `0.0.0.0:PORT` and require a nonempty validated
  `DAWN_INVITE_TOKENS` JSON array. Configuration errors fail before listening.
- Render terminates TLS; the native Godot client uses WSS and sends its
  individual token in an Authorization bearer header, never a URL.
- Reject invalid/missing tokens before upgrade and Hello. Limit concurrent
  single-node handshakes to eight and time-bound upgrade/Hello separately.
- Explicitly limit inbound WebSocket frames and messages to 64 KiB; binary
  client commands/Hello require no large payloads. Bound the pending single-node
  handshake queue to eight and omit per-denial public logs.
- Tokens are 32 cryptographically random bytes encoded as 64 hex characters;
  accept at most 16 distinct tokens. Redact secrets and compare with `subtle`.
- An invite only grants entry; it is not an account or PlayerId. ResumeTicket
  remains the proof for resuming a session. Token rotation requires restart
  and does not preserve identities or state across restart.
- Pin the client credential to its configured endpoint; do not forward it on
  a redirect. Native desktop clients only; browser exports cannot set headers.
- A native connection dialog accepts the endpoint and masked invite code;
  environment variables only prefill it. Do not persist the code. Keep it in
  memory for reconnects and clear it on cancel/timeout. Wait for Welcome before
  enabling gameplay; initial/reconnect attempts time out after 90 seconds.
- The Sector uses the existing InMemoryJournal. On every cloud process start,
  Market SQLite also gets a fresh temporary directory, avoiding a surviving
  ledger paired with reset Sector inventories. No recovery/durability claim
  is made for this demo. Production FileJournal semantics remain unchanged.

Separate login, Redis, multiple services and persistent databases are deferred:
they add no value to a small disposable test. A single shared key was rejected
because individually issued tokens can be removed separately on restart.
This is not an internet-hardened production deployment: invites can be shared,
there is no account identity, and existing output/admission queues and the
in-memory journal still need production resource management.

## Implementation checklist

- [x] Fail-closed port/invite configuration and redacted token type.
- [x] HTTP rejection before Hello, with real socket tests.
- [x] Native client endpoint/token configuration and redirect pinning tests.
- [x] Free-plan Blueprint with no persistent services and manual deployment.
- [x] Deployment and credential instructions document data loss and limitations.
- [ ] Actual Render deployment and remote native-client playtest verified.
