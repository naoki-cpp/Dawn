---
scope    : Client connection admission lifecycle across production and simulation runtimes.
audience : AI Agent / Human Developer
update   : When handshake, resume, ownership, or session-promotion behavior changes.
related  : ownership.md, ADR-0007, ADR-0014
---

# Client Admission

## Single lifecycle owner

`dawn-sector::client_admission` owns authoritative begin/commit/abort behavior.
Production, single-Sector simulation, and clustered simulation are socket
adapters: they pass intent, await the handoff, resolve the attempt on the Sector
thread, and publish only a committed session.

## State machine

```text
Hello -> begin -> await Welcome/InitialState/PlayerLoadout
                    | success -> commit -> publish/replace session
                    | failure -> abort  -> drop socket
```

## Fresh admission

Begin durably reserves the `PlayerId`/`ShipId`, resume ticket, prepared spawn
row, and allocator watermark through the explicit Admission/Identity
repositories before exposing `Welcome`. It then uses a temporary in-memory Ship
to construct the observer-scoped handoff. The preview is removed before begin
returns. Therefore an in-flight attempt has a durable protocol reservation but
no durable Ship, fitting, ownership, AoI, or Station inventory.

Commit materializes the starter state and appends exactly one
`ClientAdmissionCommitted` event containing the Ship creation, fitting/cargo
snapshot, ownership identity, and starter Station grant description. The grant
is finalized through a `SectorTransaction`: its marker, Station upsert,
consumed identities, ownership binding, and prepared-row cleanup share one
SQLite transaction. Repeating the same stable admission identity is a no-op; a
different grant payload or owner binding is rejected rather than silently
overwritten. If the process dies after the world transition but before
repository finalization, runtime retries the same identity and never allocates
a replacement Player/Ship. Reopening the repository only rebuilds identity
watermarks; it does not replay the starter Station grant, because that item may
already have been consumed after the original commit.
No checkpoint can cover a partially-returned commit because commit runs
synchronously on the owning Sector thread.

Abort releases only the live capacity reservation. The watermark remains and
IDs are never reused (INV-004).
An exact retry of a prepared fresh identity is still a fresh population claim:
it rechecks and atomically claims capacity against Ships admitted while the
original handshake was disconnected. It cannot use a stale prepared row to
exceed the Sector cap.

## Resume admission

Resume uses a server-issued `ResumeTicket`, never a client-authored
`(PlayerId, ShipId)` pair. The ticket is bound to the exact player, ship,
destination Sector, and Transit or fresh-admission attempt that issued it. It
uses durable ticket rotation for one-time consumption. Ticket expiry is not
 implemented yet and remains a production prerequisite. During a resume, the next
ticket is staged durably before `Welcome` is sent. The committed ticket remains
valid until a successful admission promotes the staged ticket, so a failed
handshake can retry without guessing whether `Welcome` was received. Transit
copies both ticket states to its destination before the old Sector is removed.

The Sector admission module resolves and validates the ticket before building
the handoff. It then reserves both sides of the resolved identity: no other
in-flight attempt may use the same Ship or Player. A Ship's presence in the
ECS is not evidence of player ownership, and NPC ships never have a resume
ticket.

Ownership changes only after every handshake frame has been await-sent. Abort
releases the reservation without touching the pre-existing Ship. A successful
reconnect for the same exact identity replaces any older runtime session and
its routing/AoI publication, so only one command source remains live.

## Cluster routing

Fresh admission starts in Sector 0. Resume routes by the authoritative ticket
binding, not by a client-provided ShipId. `player_sector` and `ship_player` are
replaced only after commit and are kept one-to-one with the published session.
Admission cannot move a Ship between Sectors or bypass the ADR-0014 Transit
pipeline.
