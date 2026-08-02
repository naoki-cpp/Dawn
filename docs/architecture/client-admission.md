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

Begin checks the population cap before allocating identity. Below the cap it
allocates a `PlayerId`, spawns the player Ship at the runtime-provided position,
and builds `InitialState`/`PlayerLoadout` from that Ship's observer-scoped AoI.

- **Commit:** verifies that the Ship still exists, then makes the session
  eligible for promotion.
- **Abort:** removes the freshly-spawned Ship and its ownership/AoI presence.
- **Missing observer while beginning:** removes the fresh Ship before returning
  the refusal.

A consumed ID or event-log history is not reused; INV-004 still applies.

## Resume admission (ADR-0007)

Resume names an exact `(PlayerId, ShipId)`. A missing Ship is refused and never
falls back to fresh spawn. Begin validates the Ship and builds the observer-
scoped `InitialState`, but leaves the ownership-dependent `PlayerLoadout` out
of the pre-commit handoff.

- **Commit:** calls `resume_player_ship`, establishing active/owned and docked
  player context only after the socket handoff succeeded. The Sector then builds
  and sends the complete `PlayerLoadout` before publishing the session to command
  routing or AoI delivery.
- **Abort:** does nothing to authoritative state. The resumed Ship predates the
  connection attempt and must never be removed as handshake cleanup.

The handoff marks the observer as a player Ship even when ownership was absent
from a restored snapshot; authoritative ownership is still deferred until
commit.

## Cluster routing

`player_sector` and `ship_player` are runtime routing indexes, not admission
authority. Cluster mode inserts both entries only after
`ClientAdmissionAttempt::commit` succeeds. Failed or disconnected attempts
therefore expose neither routing entry. Sector Transit continues to use the
ADR-0014 Raft-owned transit path; client admission cannot move a Ship between
Sectors or bypass transit ownership.
