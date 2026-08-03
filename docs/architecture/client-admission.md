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

Begin appends `ClientAdmissionIdentityReserved`, permanently consuming the
`PlayerId`/`ShipId`, then uses a temporary in-memory Ship to construct the
observer-scoped handoff. The preview is removed before begin returns. Therefore
an in-flight attempt has one durable allocation-watermark event but no durable
Ship, fitting, ownership, AoI, or Station inventory.

Commit materializes the starter state and appends exactly one
`ClientAdmissionCommitted` event containing the Ship creation, fitting/cargo
snapshot, ownership identity, and starter Station grant description. The
Station grant is applied through a SQLite ledger keyed by `ShipId`; the ledger
marker and inventory upsert share one SQLite transaction. If the process dies
after the event append but before the SQLite write, snapshot+tail replay and
`open_station_inventory_db` reconciliation apply the missing grant exactly once.
No checkpoint can cover a partially-returned commit because commit runs
synchronously on the owning Sector thread.

Abort releases only the live capacity reservation. The watermark remains and
IDs are never reused (INV-004).
An exact retry of a prepared fresh identity is still a fresh population claim:
it rechecks and atomically claims capacity against Ships admitted while the
original handshake was disconnected. It cannot use a stale prepared row to
exceed the Sector cap.

## Resume admission

Resume names an exact `(PlayerId, ShipId)` and never falls back to fresh spawn.
Begin reserves both sides of the identity: no other in-flight attempt may use
the same Ship or Player. Existing ownership is compare-and-set compatible only
when absent after restoration or already equal to the exact reconnect identity;
a different owner or a different active Ship is refused.

Ownership changes only after every handshake frame has been await-sent. Abort
releases the reservation without touching the pre-existing Ship. A successful
reconnect for the same exact identity replaces any older runtime session and
its routing/AoI publication, so only one command source remains live.

## Cluster routing

Fresh admission starts in Sector 0. Resume locates the exact authoritative
Sector and carries that index through asynchronous completion. `player_sector`
and `ship_player` are replaced only after commit and are kept one-to-one with
the published session. Admission cannot move a Ship between Sectors or bypass
the ADR-0014 Transit pipeline.
