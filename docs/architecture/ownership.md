---
scope    : Complete rules for "who manages what" — ownership, state transitions, responsibilities.
audience : AI Agent / Human Developer
update   : When Actor composition changes / when Sector management rules change.
related  : entity-model.md, event-catalog.md
---

> **Implementation status**
>
> | Section | Content | Status |
> |---|---|---|
> | §1-2 | Ship ownership, basic state transitions | Implemented |
> | §2 Sector Transit / §3 Node failure | Cross-Sector move exclusion, Raft failover | Implemented (ADR-0014) |
> | §4 Actor ownership | Data isolation between Actors | Implemented |
> | §5 ID generation | NodeId + monotonic counter | Implemented |
>
> Sector Transit must always go through Raft (CLAUDE.md FBD-006).
>
> **Scope note (2026-07-07):** this file documents *Sector*-level ownership
> (which Sector owns a Ship) and Actor data isolation. §7 now covers
> *player*-level ownership (which Player owns which Ship(s)) per ADR-0037 --
> the owned ship / active ship / docked station context split has landed
> (`ShipRegistry.owners` / `active_ship`), unblocking Phase 9B's `Assemble`.

# Ownership Rules

## 1. What ownership means

Ownership means **exactly one authority has the right to mutate a given entity's state.** Concurrent mutation by multiple authorities causes conflicts; ownership rules are encoded in code now so the same rules hold as the system distributes (ADR-0021: single ownership means CRDTs are unnecessary).

| | Single Process (current) | Distributed (future) |
|---|---|---|
| Conflicts | Impossible in-process | Occur from network latency/failures |
| Enforcement | Code convention | Physically enforced by Raft |
| Violation impact | Caught by tests | Production data inconsistency |

---

## 2. Ship ownership

### Basic rule

```
A Ship is owned by exactly one Sector at a time.
Multiple Sectors must never own the same Ship simultaneously.
```

### Ownership state transitions

```
[does not exist]
      │
      │ ShipSpawned { sector_id }
      ▼
[owned by Sector A]  ←─────────────────────────┐
      │                                        │
      │ SectorTransitRequested                  │
      ▼                                        │
[in Transit]                                   │
  ownership: remains with Sector A              │
      │                                        │
      │ SectorTransitCompleted                  │
      ▼                                        │
[owned by Sector B] ─────────────────────────────┘
      │
      │ ShipDespawned
      ▼
[does not exist]
```

While in Transit, ownership is logically still held by the origin Sector, which prevents another operation from interleaving before Transit completes.

### Operations forbidden during Transit

```
- Accepting a MoveCommand
- Starting another SectorTransit
- ShipDespawn
```

### Ownership-check responsibility

| Operation type | Responsible party | Current implementation |
|---|---|---|
| Sector-local mutation | The Sector Node itself | SimulationNode |
| Crossing Sector boundary | Consensus Layer | dawn-consensus (Raft, ADR-0014) |
| Read (reference only) | No check needed | — |

---

## 3. Sector management responsibility

### Basic rule

```
Each Sector is managed by exactly one Node.
Multiple Nodes must never manage the same Sector simultaneously.
```

### Sector → Node mapping

```
Current:  SimulationNode manages all Sectors (single process); production
          deploys (dawn-sector-node, 8D-4) split Sectors across processes
          per static TOML config.
Future:   Consensus Layer manages Sector → Node mapping dynamically.
```

`SectorId` semantics are stable across this transition regardless of which Node owns the mapping.

### Node failure handling

```
- Node crash → Raft leader re-election (ADR-0014)
- New Node takes over the failed Node's Sector
- State is rebuilt from the Event Log (snapshot + tail replay) to complete handover
```

---

## 4. Actor ownership

The Actor model lives in `dawn-actor` (ADR-0002).

### Actor-to-data mapping

| Actor | Owns | Accepts |
|---|---|---|
| `SectorSimulatorActor` | ECS World + Event Log (for its Sector) | `Tick`, `MoveCommand`, `SpawnShip`, `Transit` |

`SimulationNode` owns the Event Log directly; there is no dedicated EventStore actor.

### Inter-Actor data-sharing rules

```
Forbidden: sharing data via Arc<Mutex<T>>
Forbidden: an Actor reading another Actor's internal state directly
Allowed:   message copies via Mailbox (mpsc channel) only
```

This anticipates Actor-to-Actor communication moving over the network once physically distributed.

---

## 5. ID generation ownership

### Basic rule

```
EntityId generation is the exclusive right of the Node holding the relevant NodeId.
No coordination (locking, consensus) is required.
```

### Why no coordination is needed

```
EntityId = NodeId (8 bit) + Counter (56 bit)

As long as Counter increases monotonically within a given NodeId,
EntityId stays unique even if Counters collide across different Nodes.
```

Example:
```
Node(0), Counter(100) → EntityId: 0x00_00000000000064
Node(1), Counter(100) → EntityId: 0x01_00000000000064  ← different
```

This design requires no changes after distribution.

---

## 6. Invariants (ownership-related)

Where these duplicate CLAUDE.md INVs, only the link is listed.

| Invariant | Description | Ref |
|---|---|---|
| A Ship always belongs to exactly 1 Sector | No simultaneous ownership by multiple Sectors | §2 |
| EntityId is never reused | No same ID after Despawn | [CLAUDE.md INV-004](../../CLAUDE.md) |
| Operations on an in-Transit Ship are restricted | MoveCommand etc. rejected | §2 |
| A Sector is managed by exactly 1 Node | No simultaneous management by multiple Nodes | §3 |
| Actors never share data directly | Mailbox only | [CLAUDE.md INV-005](../../CLAUDE.md) |

---

## 7. Player-level ship ownership (ADR-0037)

`ShipRegistry` splits the "which player controls which ship" question into
three separate, independently-tracked concerns instead of one conflated
`PlayerId -> ShipId` map:

- **Owned ship** (`owners: ShipId -> PlayerId`): every ship a player owns.
  Plural -- a player may own more than one. Unchanged shape from before
  ADR-0037; this map was already correct for multiple ownership, since it's
  keyed by ship, not by player.
- **Active ship** (`active_ship: PlayerId -> ShipId`): the *one* owned ship
  currently routable for flight/steering commands (Move/Stop/Approach/Warp/
  Orbit/KeepAtRange/Jump/LockOn/Activate/Deactivate) and Undock. This was
  the field previously named `by_player`; it coincided 1:1 with `owners`
  before any player could own more than one ship.
- **Docked station context** (`docked_players: PlayerId -> StationId`):
  which station the player is currently docked at. Player-level, not
  ship-derived -- unaffected by which owned ship is active (§2 predates
  ADR-0037 and already got this right).

```
Player owns {Ship A, Ship B}, both docked at Station S, active = A

  SelectActiveShipCommand{ship_id: B}
        │
        ▼ (must own B, B != current active, B docked at same station
        │  the player is currently docked at)
  active = B  (docked_ships/docked_players for both A and B: unchanged --
               only which ship is *controllable* changed)
```

Flight/steering commands and Undock check `is_active_ship(player_id,
ship_id)` (implies `owns_ship`, since active ⊆ owned) -- a wire-level
consequence is that these commands carry no `ship_id` at all (`MoveCommand`,
`WarpCommand`, `UndockCommand`, etc.): the server always resolves the target
from `active_ship`, so there is no wire-representable way for a client to
name a ship it isn't currently flying. Station inventory-management commands
(`FitModuleCommand`/`UnfitModuleCommand`/`DockCommand`/
`BuildPackagedShipCommand`/`DisassembleShipCommand`) still carry an explicit
`ship_id` and check `owns_ship` only, since they operate on any owned ship,
not just the active one (e.g. Disassemble a spare docked ship without first
making it active).

Switching active ship is **session-local, not event-sourced**: it changes
no Ship's authoritative state (HP, fitting, position), only which ship a
connection's commands route to. Comparable to AoI delivery-control messages
(`event-catalog.md` §3.12) -- no `DomainEvent` variant exists for it. It is
still captured in `StateSnapshot` (`active_ship: BTreeMap<PlayerId, ShipId>`)
so a reconnecting player's active ship survives restart/restore, the same
tier as `docked_ships`/`docked_players`.

`ShipRegistry::remove()` only clears a player's `active_ship` entry if the
removed ship *was* that player's active ship -- removing a different owned
ship (e.g. Disassemble) must not silently clear the pointer to the ship the
player is still flying.

---

## 8. A player can reach zero owned ships while docked (resolved 2026-07-07)

`disassemble_ship_owned` checks ownership, docked-station context, unfitted,
and undamaged -- not whether this is the player's only ship. A player who
owns exactly one ship can `Disassemble` it, after which `ShipRegistry::remove()`
clears `active_ship` (it was the only owned ship), leaving the player with
zero owned ships, no active ship, still docked.

This state is structurally representable (no crash, no invariant violated),
and is no longer a dead end: `AssembleCommand` (Phase 9B-5, implemented
2026-07-07) converts a station-inventory `PackagedShip` item into a new
owned, docked ship without changing `active_ship` (ADR-0037); the player then
sends `SelectActiveShipCommand` to make it active and can `Undock` normally.
`BuildPackagedShipCommand` still requires `owns_ship(player_id, cmd.ship_id)`
(used only as an ownership-proof anchor, unrelated to the packaged ship being
built) -- a shipless player cannot use it, but this no longer matters for
recovery since Assemble only needs a `PackagedShip` already in station
inventory, not an existing ship.

A related bug surfaced and was fixed in the same change: `ClientCommandFollowup::RefreshFitting`
used to carry a `ShipId`, and `build_player_loadout_json` bailed out
immediately once that ship no longer existed in the ECS -- so a client that
disassembled its only ship never received the updated station inventory at
all (the item existed server-side, but the client never learned about it).
Fixed by keying `RefreshFitting` on `PlayerId` instead, with a new
`build_player_loadout_json_for_player` that falls back to reporting just the
docked station and station inventory when the player has no active ship.

**Disembark (added 2026-07-07):** the zero-owned-ships-while-docked state was
previously only reachable by accident (via Disassemble). `DisembarkCommand`
(no fields, no `ship_id`, resolved from the caller's active ship like
`UndockCommand`) makes it a deliberate player action: it clears `active_ship`
while docked, without disassembling the ship or touching `owns_ship` --
`docked_ships`/`docked_players` are unaffected, only which ship the caller's
commands route to. `disembark_owned` returns `Result<ShipId, StationOperationRejection>`
rather than `StationOperationOutcome` for the same reason `assemble_ship_owned`
does: the "no active ship" rejection has no real ship to report. Session-local,
not event-sourced (same tier as `SelectActiveShipCommand`) -- no `DomainEvent`
variant exists for it, and it isn't forwarded on the wire as an event. Round
trip: `Disembark` -> `SelectActiveShipCommand` (this ship, or another owned
ship docked at the same station) -> `Undock`.
