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
> **Scope note:** this file documents *Sector*-level ownership (which Sector
> owns a Ship) and Actor data isolation. §7 covers *player*-level ownership
> (which Player owns which Ship(s)) per ADR-0037.

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

## 8. A docked player with no active ship cannot Undock

Flight/steering commands and Undock require `is_active_ship`. A player can
be docked with no active ship -- via `DisassembleShipCommand` (destroys the
only owned ship) or `DisembarkCommand` (clears `active_ship` without
destroying anything) -- and simply has nothing to fly until:

- `AssembleCommand` turns a station-inventory `PackagedShip` item into a new
  owned, docked ship (does not set it active), or
- `SelectActiveShipCommand` makes an owned, docked ship active.

Both `AssembleCommand` and `DisembarkCommand` are session-local, not
event-sourced (same tier as `SelectActiveShipCommand`) -- no `DomainEvent`
exists for either. Both return `Result<ShipId, StationOperationRejection>`
rather than `StationOperationOutcome`, since a rejection may have no real
`ship_id` to report.

`PlayerLoadout` carries `active_ship_id: Option<u64>` (`null` when shipless)
and `owned_ships: [{ship_id, ship_type_id, ship_type_name,
docked_station_id, is_active}]`, so the client can render a shipless docked
player and a full ship roster. The inventory panel has four columns --
FITTED, SHIP CARGO, STATION, SHIPS -- kept strictly separate.

`TransferToStationCommand { ship_id, station_id, item_id }` moves the
entire stack of one item (`Module` or `ScrapMetal`) from a docked ship's
cargo into the caller's station inventory; whole-stack only, no partial
transfer. Client trigger: right-click a SHIP CARGO row.

**Known gap:** `StateSnapshot` does not persist `ShipRegistry.owners`/
`active_ship`.
