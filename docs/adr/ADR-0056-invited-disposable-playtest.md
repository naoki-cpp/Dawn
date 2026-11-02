---
id      : ADR-0056
title   : Invited disposable single-process playtest on Render
status  : accepted
date    : 2026-11-02
deciders: [human, ai-agent]
related : ADR-0048 (ResumeTicket remains a separate capability), ADR-0049 (production durability is unchanged), ADR-0051 (existing server composition)
---

# ADR-0056 - Invited disposable single-process playtest on Render

## Context

The user approved a small deployment: one Render Free web service, WSS,
individual tester invites, and disposable data. The LAN prototype's lack of
transport authentication is not appropriate for an internet endpoint.

## Decision

- Deploy `simulate --serve --cluster --ships 5 --pop-cap 8` in one service.
  Amendment (2026-11-02): host the existing three-Sector in-process Raft
  cluster so gate travel works. This replaces the initial single-Sector limit,
  not the one-service, disposable-data or trusted-tester restrictions.
- Keep one public WebSocket endpoint. In-process transit uses the existing
  consensus/handoff path and sends the destination InitialState on the same
  connection; resume resolves the ticket's current owning Sector. No public
  peer ports, extra services, or credential forwarding are needed.
- With `PORT` set, bind `0.0.0.0:PORT` and require a nonempty validated
  `DAWN_INVITE_TOKENS` JSON array. Configuration errors fail before listening.
- Render terminates TLS; the native Godot client uses WSS and sends its
  individual token in an Authorization bearer header, never a URL.
- Reject invalid/missing tokens before upgrade and Hello. Limit concurrent
  handshakes to eight across all Sectors and time-bound upgrade/Hello separately.
- Explicitly limit inbound WebSocket frames and messages to 64 KiB; binary
  client commands/Hello require no large payloads. Bound the shared pending
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
- Each Sector uses the existing InMemoryJournal. On every cloud process start,
  Market SQLite also gets a fresh temporary directory, avoiding a surviving
  ledger paired with reset Sector inventories. No recovery/durability claim
  is made for this demo. Production FileJournal semantics remain unchanged.
- Both `--serve` and `--serve --cluster` share the same fail-closed listener,
  bounded invite handshake ingress, and disposable cloud Market lifecycle.
  Co-located Raft actors do not provide machine-failure durability or failover.

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
- [x] Remote native-client connection verified for the initial single-Sector deployment.
- [x] Real-binary test covers cloud port, invitation rejection, gate travel and resume.
- [ ] Updated Render deployment and remote gate-jump playtest verified.
