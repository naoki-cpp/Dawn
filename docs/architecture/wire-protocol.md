---
scope    : Client<->server wire format over the WebSocket connection. What an
           external (non-Godot) client would need to talk to a Dawn server.
audience : AI Agent / Human Developer
update   : Both halves are generated from the types; see "Keeping this in sync".
related  : ADR-0005 (ClientConnection), ADR-0041 (dawn-wire), ADR-0042
           (postcard binary envelope), docs/architecture/event-catalog.md,
           crates/dawn-wire/src/lib.rs
---

# Wire Protocol

Transport is a single WebSocket connection per client. Since ADR-0042 (all
stages complete), every message -- `Hello`/`Welcome`/`Redirect`/`DomainEvent`/
`ClientCommand`/`InitialState`/`PlayerLoadout`/`AoiEnter`/`AoiLeave`/
`PositionSnap`/`MotionCorrection` -- travels as a **binary** frame, postcard-encoded via the
`ClientMessage`/`ServerMessage` envelope in `dawn-wire` (one WebSocket frame
always carries exactly one message; no length-prefix framing is needed on
top). There is no more ad-hoc JSON text frame path.

Market requests and snapshots use the same binary envelope but remain a
separate message family: `ClientMessage::Market(MarketCommandWire)` and
`ServerMessage::MarketSnapshot(MarketSnapshotWire)`. They do not enter the
Sector `ClientCommandWire` stream. This preserves the ADR-0034 boundary where
the Market owns order matching and Currency, while `dawn-simulation` applies
only the one-sided cargo bridge commands to the owning `SimulationNode`.

`ServerMessage::MotionCorrection` (ADR-0043) is owner-only normal-flight
authority for the local Rust predictor. It carries the ship's absolute
position, current velocity, and server tick. Warp arrival continues to use
`PositionSnap`; remote ships do not receive this owner-only correction.

The field-level shape of `EventWire`/`ClientCommandWire` below is still
generated from the Rust types and still useful as the schema-of-record for
what a message's fields mean -- but the **outer JSON shape shown in this
doc's schema files no longer matches literally what's on the wire** for
these two types: postcard cannot deserialize an internally tagged enum, so
`EventWire`/`ClientCommandWire` are externally tagged (`{"VariantName":
{...fields}}`), not `{"type": "VariantName", ...}`. An external
(non-Godot) client talking to a Dawn server needs to speak postcard, not
raw JSON, for the messages listed above.

## Server -> client: generated from `EventWire`

The full, authoritative list of messages the server can send, with every
field and its JSON type, is generated straight from the Rust source and
checked in at
[`wire-protocol.schema.json`](./wire-protocol.schema.json) (JSON Schema,
draft-07). It is produced by `dawn_actor::protocol::event_wire_json_schema()`,
which reflects the `EventWire` enum in
[`crates/dawn-wire/src/server_event.rs`](../../crates/dawn-wire/src/server_event.rs)
(re-exported from `dawn_actor::protocol`, ADR-0041/ADR-0042).

Read `wire-protocol.schema.json` for the exact contract. In summary, the
variant names are: `ShipSpawned`, `VelocityChanged`, `ShipDespawned`,
`ShipDocked`, `ShipUndocked`, `ShipAssembled`, `DamageTaken`, `RepairApplied`,
`ShipDestroyed`, `TargetLocked`, `LockLost`, `ModuleActivated`,
`ModuleDeactivated`, `JumpGateUsed`, `StarSystemChanged`. (A server-initiated
reconnect to a different node on cross-node jump is `ServerMessage::Redirect`,
a struct variant of the outer envelope, not an `EventWire` variant --
see ADR-0026 / multi-node clusters.)

`ShipAssembled` (Phase 9B-5, ADR-0034/ADR-0037) reports a new live docked
ship created from a station-inventory `PackagedShip` item: `ship_id`,
`station_id`, `ship_type_id`, `tick`. It does not imply the ship became the
caller's `active_ship` -- send `SelectActiveShipCommand` to fly it.

Every `EventWire` variant carries `tick: u64`.

Not every `DomainEvent` reaches the wire -- `domain_event_to_event_wire()`
returns `None` for internal bookkeeping events (`ShipFitted`, `WeaponFired`,
`TackleApplied`, `TackleReleased`, the `SectorTransit*` family,
`AnchorRebased`, `PackagedShipBuilt`, `ShipDisassembled`). See
`docs/architecture/event-catalog.md` for what those events mean server-side.

## Client -> server: generated from `ClientCommandWire`

The full list of messages a client can send, with every field and its JSON
type, is generated the same way and checked in at
[`wire-protocol-commands.schema.json`](./wire-protocol-commands.schema.json).
It is produced by `dawn_actor::protocol::client_command_wire_json_schema()`,
which reflects the `ClientCommandWire` enum in `crates/dawn-wire/src/client_command.rs` (re-exported from `dawn_actor::protocol`).

The variant names are: `MoveCommand`, `LockOnCommand`,
`ActivateModuleCommand`, `DeactivateModuleCommand`, `AttackCommand`,
`StopCommand`, `JumpCommand`, `ApproachCommand`, `WarpCommand`,
`OrbitCommand`, `KeepAtRangeCommand`, `FitModuleCommand`,
`UnfitModuleCommand`, `ReorderFittedModuleCommand`, `DockCommand`,
`UndockCommand`, `BuildPackagedShipCommand`, `DisassembleShipCommand`,
`SelectActiveShipCommand`, `AssembleCommand`, `DisembarkCommand`,
`TransferToStationCommand`.

`AssembleCommand { station_id, ship_type_id }` (Phase 9B-5) carries no
`ship_id` -- the ship doesn't exist yet; its ID is reported back via the
resulting `ShipAssembled` event. Rejected if the caller isn't docked at
`station_id`, `ship_type_id` is unknown, or the station inventory has no
matching `PackagedShip`.

`DisembarkCommand {}` (ADR-0037) clears the caller's active ship while
docked, without disassembling it or changing ownership -- the ship stays
owned and docked, only which ship the caller's commands route to changes.
Session-local, not event-sourced (same tier as `SelectActiveShipCommand`), so
there is no resulting domain event on the wire. Rejected if the caller has no
active ship, or the active ship isn't currently docked. See
`docs/architecture/ownership.md` §8.

`TransferToStationCommand { ship_id, station_id, item_type, module_id,
ship_type_id, direction }` (ADR-0034 9B) moves the entire stack of one item
between a docked ship's own cargo (`InventoryComp`) and the caller's station
inventory -- whole-stack only, no partial-count transfer. `direction` is
`"ToStation"` or `"ToShip"`. `item_type` is one of `"Module"`,
`"PackagedShip"`, `"ScrapMetal"` (same wire shape as `ItemRow`);
`module_id`/`ship_type_id` are populated only for the matching variant (`0`
otherwise). Carries an explicit `ship_id` like `FitModuleCommand` (it may
target any owned docked ship, not just the active one). Rejected if the
caller doesn't own `ship_id`, isn't docked at `station_id`, or the source
side has none of the named item. No resulting domain event -- silent
station-inventory credit/debit, same tier as
`BuildPackagedShipCommand`/`DisassembleShipCommand`.

`ReorderFittedModuleCommand { ship_id, slot, from_index, to_index }`
(ADR-0032's 2026-07-08 amendment) reorders two fitted modules within the
same slot kind -- persisted, not cosmetic, since iteration order assigns
weapon hotkey F-numbers. Rejected if the caller doesn't own `ship_id`, the
ship isn't docked, or either index is out of bounds for `slot`'s current
module count. Reuses `ShipFitted` (no new event type).

**ADR-0037 (owned ship / active ship split):** `MoveCommand`, `LockOnCommand`,
`ActivateModuleCommand`, `DeactivateModuleCommand`, `StopCommand`,
`JumpCommand`, `ApproachCommand`, `WarpCommand`, `OrbitCommand`,
`KeepAtRangeCommand`, `DockCommand`, `UndockCommand`, and `DisembarkCommand`
carry no `ship_id` field at all -- the server always resolves them against
the caller's active ship, so there is no wire-representable way to name a
ship the player isn't currently flying. `FitModuleCommand`,
`UnfitModuleCommand`, `ReorderFittedModuleCommand`, `BuildPackagedShipCommand`,
and `DisassembleShipCommand` still carry an explicit `ship_id`, since they
may target any owned docked ship, not just the active one.
`SelectActiveShipCommand { ship_id }` is the only way to change which owned
ship is active (station-local switch only for now). See
`docs/architecture/ownership.md` §7.

`ClientCommandWire` mirrors the wire format exactly, including two
backward-compatible quirks it does not itself resolve (that validation
happens in `client_command_from_wire()`):

- `WarpCommand` accepts a legacy `{"gate_id": N}` form and the current
  `{"target": {"Gate": N}}` / `{"target": {"Body": N}}` form. `target` wins
  if both are present; prefer it for new clients.
- `ApproachCommand`, `OrbitCommand`, and `KeepAtRangeCommand` select their
  target with either `gate_id` (a Jump Gate) or `target_id` (a Ship);
  `gate_id` wins if both are present, and the command is rejected
  (`client_command_from_wire` returns `None`) if neither is present.

`ActivateModuleCommand`'s `target_ship_id` is only required for targeted
module kinds (Weapon/Tackle, ADR-0035); the server validates that
requirement, not the wire schema.

## Market requests and snapshots

The Market request schema is generated separately at
[`wire-protocol-market.schema.json`](./wire-protocol-market.schema.json) by
`dawn_actor::protocol::market_command_wire_json_schema()`. It contains
`RefreshMarketCommand`, `PlaceMarketOrderCommand`, and
`CancelMarketOrderCommand`. `PlaceMarketOrderCommand` carries an explicit
`ship_id` because an Ask removes cargo from that owned ship and a Bid names the
ship that receives a filled item. `price` and `quantity` must be positive and
their product must fit in `u64`; the runtime rejects invalid input before
calling `dawn-market`.

`MarketSnapshotWire` contains the caller's Currency `balance`, a bounded list
of open orders (maximum 200), and a short server `notice`. Each order includes
`is_own`, which is calculated server-side and must not be trusted from client
input. A client may submit an order or cancel only through the Market family;
Sector does not parse these variants.

## Keeping this in sync

`wire_schema_doc_is_up_to_date` (a test in `dawn-actor/src/protocol/mod.rs`) fails the build if
any checked-in schema file drifts from what `EventWire`, `ClientCommandWire`,
or `MarketCommandWire` currently produce. After changing any of those enums
(or a type they reference), regenerate with:

```bash
cargo run -p dawn-actor --example gen_wire_schema
```

and commit the updated `.schema.json` files alongside the code change.
These files are documentation, generated from the types -- never hand-edit
them.

## Connection handshake

- A fresh client sends `ClientMessage::Hello(HelloMessage { resume: None })`,
  postcard-encoded as a binary frame (ADR-0042).
- A client resuming after a `Redirect` (cross-node jump) sends
  `ClientMessage::Hello(HelloMessage { resume: Some(ResumeIdentity {
  player_id, ship_id }) })` to resume its identity on the new node instead of
  spawning fresh.
- The server replies with `ServerMessage::Welcome { player_id, ship_id }`,
  then `ServerMessage::InitialState` (+ optional `ServerMessage::PlayerLoadout`),
  all binary.
