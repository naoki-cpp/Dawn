---
scope    : Definition of the "things" that exist in the World. Schema spec for types, fields, and identifiers
audience : AI Agent / Human Developer
update   : When a type or field definition changes
related  : event-catalog.md, ownership.md, ../adr/ADR-0044-absolute-f64-coordinate-authority.md
---

# Entity Model

## 1. Identifier Design

### EntityId

A unique identifier shared by all entities.

```
layout: [NodeId: upper 8 bit | Counter: lower 56 bit]
type  : u64 newtype
```

**Generation rules:**
- `NodeId` identifies the Node that issued the ID
- `Counter` increases monotonically within a given `NodeId`
- IDs from different `NodeId`s are unique even if `Counter` matches
- Once issued, an ID is **never reused** (INV-004) — reuse would make a replayed Event Log show a despawned Ship spawning again.

### ShipId

A newtype over `EntityId`, marking an entity as a Ship at the type level.

```rust
// type definition sketch (actual impl: dawn-core/src/entity.rs)
struct ShipId(EntityId);
```

Keeps `ShipId` distinct from other domain identifiers such as `StationId`.

### NodeId

```
type  : u8 newtype
range : 0-255 (max 256 Nodes)
today : configured logical identifier in both in-process and distributed deployments
```

### SectorId

```
type  : u8 newtype
range : 0-255
today : fixed count, fixed assignment
```

---

## 2. Value Objects

### Position

3D anchor-relative coordinate in metres. Static orbital positions are authored
in AU in `data/galaxy.toml` and converted to metres by the loader.

| Field | Type | Description |
|---|---|---|
| `x` | `f64` | east-west |
| `y` | `f64` | up-down |
| `z` | `f64` | north-south |

**Current representation:** ship ECS state is an `f64` offset relative to `AnchorComp` (ADR-0029).
Static navigation definitions, authoritative spawn/transit events, snapshots, and server
wire payloads use `dawn_core::AbsolutePosition` for sector-frame f64 coordinates. Ship ECS
state uses `PositionComp` (an anchor-relative f64 offset) plus `AnchorComp`; absolute
consumers must go through `AnchorTable`/`SimulationNode::ship_absolute`.

### Velocity

Displacement vector per Tick, in distance-units / Tick.

| Field | Type | Description |
|---|---|---|
| `dx` | `f64` | displacement along X |
| `dy` | `f64` | displacement along Y |
| `dz` | `f64` | displacement along Z |

`Velocity::ZERO` represents zero speed. A change to zero can emit
`VelocityChanged`; a ship that remains at zero emits no repeated change.

### SectorBounds

Axis-aligned bounding box (AABB) describing a Sector's spatial extent.

| Field | Type | Description |
|---|---|---|
| `min` | `Position` | min corner (origin side) |
| `max` | `Position` | max corner |

**Default:** `SectorBounds::centered(DEFAULT_HALF)` — a cube centered at the origin, 1,400,000 per side (DEFAULT_HALF = 700,000).
**Boundary crossing:** the Tick loop does not enforce bounds — space is infinite. `SectorBounds` is used only to generate spawn positions.

### Tick

Logical time counter. See [tick-model.md](./tick-model.md) for details.

```
type   : u64 newtype
initial: Tick::ZERO (= 0)
nature : monotonically increasing, unrelated to wall-clock time
```

---

## 3. Entity: Ship

The only entity kind in the current MVP.

### ECS Component List (ADR-0024)

`SimWorld::spawn_ship()` always creates a Ship with every component below; a partially-equipped Ship entity must never be spawned.

| Component | Description |
|---|---|
| `ShipIdComp` | maps the hecs Entity to the domain `ShipId` |
| `PositionComp` | current world coordinate, relative to `AnchorComp`'s anchor |
| `AnchorComp` | which `AnchorId` `PositionComp` is relative to (defaults to the Sector origin/star; rebased on Warp arrival, ADR-0029) |
| `VelocityComp` | displacement per Tick |
| `ThrustComp` | thrust direction / braking state (updated by MoveCommand / StopCommand) |
| `ShipStatsComp` | aggregated stats (base_stats + Σmodule.delta, updated by apply_fitting()) |
| `FittingComp` | equipment slots (High / Mid / Low / Rig lists of `FittedSlot`) |
| `HullComp` | 3-layer HP (Shield / Armor / Hull) |
| `WeaponComp` | weapon cycle state |
| `LockComp` | lock-on state (`LockState` per target) |
| `IsNpcComp` | NPC marker (removed from player ships after spawn) |
| `TransitComp` | Sector Transit state (`None` / `InTransit`, ADR-0014) |

Conditionally attached components:

| Component | Condition |
|---|---|
| `CapacitorComp` | player ships and bot ships (subject to Capacitor management) |
| `IsBotComp` | bot ships (target marker for `process_bots()`) |
| `ApproachComp` | approaching a Ship / Gate / Body / Station (semi-auto approach; removed by Move/Stop; ADR-0015) |
| `OrbitComp` | orbiting a target at a set radius (mutually exclusive with Approach/KeepAtRange; ADR-0031) |
| `KeepAtRangeComp` | holding distance from a target (mutually exclusive with Approach/Orbit; ADR-0031) |
| `WarpComp` | warping (align -> warping two-phase, intra-Sector short-range Fold; ADR-0022) |
| `TackledComp` | under Tackle (`tacklers: Vec<ShipId>`; blocks Warp/jump; ADR-0024) |
| `InventoryComp` | unfitted item stacks the pilot owns (player ships only; `BTreeMap<ItemId, u64>` — Module / PackagedShip / ScrapMetal; changed by Fit/Unfit and by combat drops; ADR-0032, generalized to `ItemId` by ADR-0034) |

### Not Yet on Ship (out of MVP scope)

Planned for future phases, not present today:

```
Name  (ship name)   <- UI / Social Context
```

(Cargo hold is no longer future work -- `InventoryComp` already serves that
role as of ADR-0034's `ItemId` generalization; see the conditionally-attached
table above.)

Ownership (`PlayerId`) is not an ECS Component; it is tracked by the
`SimulationNode` composition root in `PlayerState.owners` (`ShipId -> PlayerId`).
The owner map is authoritative routing state and is included in ADR-0049
recovery, while the ECS world remains responsible for per-Ship simulation
components.

### Ship Template (data-driven)

Each ship class's base performance is data, not code: `ShipTypeDefinition` loaded from TOML.

```
ShipTypeDefinition (immutable, data)   ShipInstance (mutable, ECS)
─────────────────────────────────      ──────────────────────────
id          : ShipTypeId               ship_id       : ShipId
name        : "Magpie"                 (ship_type_ids: ShipId -> ShipTypeId
class       : ShipClass                 is managed by SimulationNode)
slot_layout : SlotLayout               position      : Position
base_stats  : ShipBaseStats            velocity      : Velocity
                                        HullComp / CapacitorComp ...
```

**Current implementation:**
- Loaded together with module definitions through `GameDataCatalog`
- `data/ship_types.toml` is the only ship-balance authority
- Missing or invalid required game data is a fatal startup error; no Rust fallback exists
- Definitions are immutable; balance changes mean editing TOML + restarting the server (no rebuild)
- `ShipTypeId` is defined in `dawn-core` and included in the `ShipSpawned` event
- `warp_speed_au_per_sec: f64` is the positive, finite hull warp speed in AU/s
  (Magpie: 5). Warp resolves it through `ShipTypeId`, not a Magpie-specific
  branch or the fitted sub-warp `max_speed`. The 0.75 AU/s fallback is only for
  unregistered simulation hulls. This speed also sets the exponential ramp
  rates; deceleration uses one third of the numerical AU/s rate, capped at 2/s
  (ADR-0022).

### Coordinate policy (ADR-0044, accepted)

Do not introduce new code that treats `Position` as both an absolute Sector coordinate and an
anchor-relative offset. `PositionComp` is the local f64 representation governed by
ADR-0029. New authoritative coordinates use `AbsolutePosition`; conversion is performed only
at the anchor boundary. Client-authored movement targets use the typed f64 `Position` inside
`ClientRequest`; the wire layer does not maintain a parallel position wrapper.

---

## 4. Entity: Node

`NodeId` is the configured logical identity and ID-issuance domain. Development
and in-process cluster modes may host several logical Nodes in one process;
production `dawn-server --bin sector-node` runs as a physical process with a
configured Node/Sector assignment.

| Attribute | Current | Future |
|---|---|---|
| Identity | Configured `NodeId` and static Sector assignment | Dynamic Sector placement and membership |
| Communication | In-process messages within a process; shared versioned TCP peer transport between processes (ADR-0050). Client boundary is WebSocket with postcard binary messages (ADR-0007, ADR-0042) | TLS / QUIC transport |
| Failure | Raft peer/leader failure handling and checkpoint-plus-recovery-tail reconstruction are implemented baseline behavior | Dynamic placement and deployment-specific recovery SLOs |

`NodeId`'s role (unit of ID issuance) is independent of whether a Node shares a
process with other Nodes.

---

## 5. Entity: Sector

The unit by which Ships are spatially partitioned.

| Attribute | Description |
|---|---|
| `SectorId` | Sector identifier |
| `SectorBounds` | Sector's spatial extent (AABB) |
| owning Node | the logical Node responsible for this Sector (see [ownership.md](./ownership.md)) |

**Current constraints:**
- Sector count is fixed (MVP: 3)
- Sector spawn-placement bounds are fixed (1,400,000 units per side = `2 * DEFAULT_HALF`, centered at origin)
- Dynamic split/merge not implemented

---

## 6. Type Backward-Compatibility Rules

`dawn-core` type changes ripple through every crate, so handle them carefully.

```
allowed  : adding a field (as Option<T> only, must not break existing code)
allowed  : changing an authoritative coordinate field to `AbsolutePosition` under ADR-0044
forbidden: removing a field without an explicit migration plan
forbidden: renaming a field (changes the serialization key)
forbidden: unwrapping a newtype
```

Any coordinate type change still requires an ADR or an update to ADR-0044 and synchronized
event, snapshot, wire-schema, and documentation changes.
