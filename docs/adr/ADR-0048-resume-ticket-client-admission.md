---
id      : ADR-0048
title   : Server-issued ResumeTicket for client admission
status  : accepted
date    : 2026-08-03
deciders: [human, ai-agent]
related : ADR-0007 (multiplayer session handshake), ADR-0014 (Sector Transit), ADR-0042 (binary wire protocol)
---

# ADR-0048 - Server-issued ResumeTicket for client admission

## Context

The current resume handshake lets a client send `PlayerId` and `ShipId` in
`Hello`. Those values identify an object, but they do not prove that the
client is allowed to resume it. A materialized NPC ship has a valid `ShipId`
but no player owner, so an untrusted client can currently ask to resume that
ship and acquire ownership during admission.

This also makes recovery ambiguous. A missing live owner can mean either an
NPC or a player ship restored from an older state representation. A runtime
marker such as `IsNpcComp` is not a durable identity authority. Transactional
begin/commit/abort processing (PR #260) protects state changes during an
asynchronous handshake, but it cannot turn client-authored IDs into proof of
identity.

## Decision

Resume admission is authorized by a server-issued `ResumeTicket`, not by a
client-authored `(PlayerId, ShipId)` pair.

### Ticket semantics

`ResumeTicket` is an opaque, unguessable, server-issued bearer value. Its
authoritative record is bound to:

- the `PlayerId` and `ShipId` being resumed;
- the destination `SectorId`;
- the Sector Transit or fresh-admission attempt that issued it;
- an expiry or validity window; and
- a one-time consumption state.

The ticket is issued only after the server has durably reserved the identity.
For a resume, the next ticket is also durably staged before `Welcome` is sent.
The committed ticket remains valid until the admission commit promotes the
staged ticket and clears the previous one. A failed handshake releases only the
in-flight admission reservation; the committed and one staged ticket remain
retryable, so a client can safely retry whether or not it received `Welcome`.
Only one staged ticket is retained per owned Ship.

### Wire contract

- A fresh client sends `Hello { resume: None }`.
- A client reconnecting after a fresh-admission interruption or a cross-Sector
  Redirect sends `Hello { resume: Some(ResumeTicket) }`.
- `Welcome` and `Redirect` may expose the server-issued ticket needed for the
  next connection. `PlayerId` and `ShipId` may remain in server-to-client
  messages as session data, but they are never accepted as resume authority.
- The old `ResumeIdentity { player_id, ship_id }` Hello shape is removed. This
  is a pre-release protocol change; no compatibility bridge is required.

### Admission contract

The Sector-owned admission module keeps the small public interface:

```rust
enum ClientAdmissionIntent {
    Fresh { spawn_position: Position },
    Resume { ticket: ResumeTicket },
}
```

The module resolves the ticket to the authoritative player and ship before it
builds the handoff. Runtime adapters do not parse IDs, select a Sector from a
client-provided ship, adopt arbitrary ships, or publish routing state. NPCs
never receive resume tickets and therefore cannot enter the player admission
path.

The durable player-ship ownership record remains the source of truth for
which ships can be resumed. Admission does not infer ownership from the
presence of a Ship entity, its ship type, or an ECS marker.

### Transit integration

The authoritative Transit commit carries or durably references the ticket for
the destination Sector. The destination validates the ticket against the
committed handoff before building the client payload. Redirect delivery is a
notification of that already-authorized handoff, not an authorization step.

## Alternatives considered

### Keep raw IDs and require an existing owner

This prevents the immediate NPC-claim bug but leaves the client-controlled
identity model intact. It also cannot distinguish a legitimate legacy player
ship with missing ownership from an NPC without adding another authority.

### Reject only `IsNpcComp` ships

This relies on an ECS runtime marker that is not the durable ownership record
and can be lost or reconstructed incorrectly during snapshot restore. It is a
useful invariant check, but not an admission credential.

### Put the full identity in a signed client token

A signed token can be an implementation of the ticket record, but the
protocol still needs one-time use, destination binding, expiry, and replay
handling. The decision therefore specifies ticket semantics first and leaves
the storage or signing implementation behind the Sector admission interface.

## Consequences

The connection, wire, client binding, Transit, and three runtime adapters must
change together. The benefit is that identity resolution, ownership, and
session publication each have one authority and the same admission rules hold
in production, single-Sector, and clustered modes.

Authentication of a human account and transport security remain separate
concerns. A ResumeTicket is a reconnect capability, not a complete account
authentication system.

## Implementation checklist

- [ ] Add an opaque `ResumeTicket` type to `dawn-wire`.
- [ ] Replace `ResumeIdentity` in `HelloMessage` and remove raw-ID resume input.
- [ ] Carry the ticket in `Welcome` and `Redirect` where a retry is possible.
- [ ] Persist or replicate ticket binding through fresh admission and Transit.
- [ ] Change `ClientAdmissionIntent::Resume` to accept only a ticket.
- [ ] Resolve and consume tickets inside `dawn-sector::client_admission`.
- [ ] Make production, single, and clustered adapters pass ticket intent only.
- [ ] Store and resend the ticket in the Godot connection layer.
- [ ] Add tests for NPC rejection, wrong destination, wrong owner, expiry,
  replay, failed-handshake retry, and clustered redirect recovery.
- [ ] Update ADR-0007, client-admission documentation, wire documentation, and
  generated wire schemas.
