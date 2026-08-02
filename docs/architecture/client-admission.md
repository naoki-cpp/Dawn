---
scope    : Client connection admission lifecycle across production and simulation runtimes.
audience : AI Agent / Human Developer
update   : When handshake, resume, ownership, or session-promotion behavior changes.
related  : ownership.md, ADR-0007, ADR-0014
---

# Client Admission

## Single lifecycle owner

`dawn-sector::client_admission` is the only owner of authoritative client
admission mutation and rollback. Production `dawn-sector-node`, single-Sector
`dawn-simulation --serve`, and clustered `dawn-simulation --serve --cluster`
are socket adapters only: they wait for `Hello`, pass fresh/resume intent into
`SimulationNode::begin_client_admission`, complete the WebSocket handoff, and
promote a session only after the returned attempt commits.

Runtime code must not independently allocate a `PlayerId`, spawn/adopt a Ship,
build an observer-scoped handoff, or decide which Ship to remove after failure.
Those rules belong to `ClientAdmissionAttempt`.

## State machine

```text
socket Hello
    |
    v
begin_client_admission(intent, AoI cell size)
    |
    +-- refusal --------------------------> drop socket; no session
    |
    v
ClientAdmissionAttempt
    |
    +-- WebSocket handoff succeeds ------> commit(node) --> promote session
    |
    +-- error / disconnect --------------> abort(node)  --> drop socket
```

The asynchronous socket task carries the attempt token with its completion
result. The owning tick thread performs `commit` or `abort`, keeping all
`SimulationNode` mutation on the Sector thread.

## Fresh admission

Begin checks the population cap, reserves `PlayerId`/`ShipId`, and counts the
reservation against the cap. The consumed `PlayerId`/`ShipId` watermark is appended
durably before any frame can be sent. Begin materializes the Ship only inside the
call to build observer-scoped `InitialState`/`PlayerLoadout`, then removes that
preview before returning. The in-flight reservation is non-durable and snapshots
never include it; the allocation watermark survives through snapshot or event replay.

- **Commit:** materializes the reserved Ship, appends its spawn/fitting events,
  and credits the starter packaged Ship in durable Station inventory.
- **Abort:** releases only the in-memory reservation; no authoritative mutation
  exists to roll back.
- **Process loss before resolution:** loses the non-durable reservation and
  cannot resurrect a Ship, while the durable watermark prevents either ID
  from being issued to a later client.
- **Missing observer while beginning:** removes the temporary preview and
  releases the reservation before returning the refusal.

A consumed ID or event-log history is not reused; INV-004 still applies.

## Resume admission (ADR-0007)

Resume names an exact `(PlayerId, ShipId)`. A missing Ship is refused and never
falls back to fresh spawn. Begin first acquires a non-durable Ship-level resume
reservation; a concurrent attempt for the same Ship is refused until the first
attempt commits or aborts. Begin then validates the Ship and builds the observer-
scoped `InitialState`, but leaves the ownership-dependent `PlayerLoadout` out
of the pre-commit handoff.

- **Begin/handoff:** projects a complete `PlayerLoadout` from the exact
  `(PlayerId, ShipId)` and persisted dock state without changing ownership. The
  socket task await-sends it with `Welcome` and `InitialState`; any failed frame
  fails the handshake.
- **Commit:** calls `resume_player_ship`, establishing active/owned and docked
  player context only after every handshake frame succeeded, then publishes the
  session to command routing or AoI delivery.
- **Abort:** does nothing to authoritative state. The resumed Ship predates the
  connection attempt and must never be removed as handshake cleanup.

The handoff marks the observer as a player Ship even when ownership was absent
from a restored snapshot; authoritative ownership is still deferred until
commit.

## Cluster routing

`player_sector` and `ship_player` are runtime routing indexes, not admission
authority. Fresh admission begins in Sector 0; resume locates the exact Ship
across all cluster Sectors and carries that Sector index through asynchronous
handoff completion. A missing Ship or a duplicate `ShipId` visible in more than
one Sector is refused rather than choosing an ambiguous owner. Cluster mode
inserts both routing entries only after `ClientAdmissionAttempt::commit`
succeeds in that same Sector. Failed or disconnected attempts therefore expose
neither routing entry. Sector Transit continues to use the ADR-0014 Raft-owned
transit path; client admission cannot move a Ship between Sectors or bypass
transit ownership.
