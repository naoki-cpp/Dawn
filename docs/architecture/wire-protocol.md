---
scope    : Client<->server wire format over the WebSocket connection. What an
           external (non-Godot) client would need to talk to a Dawn server.
audience : AI Agent / Human Developer
update   : Both halves are generated from the types; see "Keeping this in sync".
related  : ADR-0005 (ClientConnection), docs/architecture/event-catalog.md,
           crates/dawn-actor/src/protocol.rs
---

# Wire Protocol

Transport is a single WebSocket connection per client. Every message in
either direction is one newline-delimited JSON object tagged by a `"type"`
field (`{"type": "ShipSpawned", ...}`).

## Server -> client: generated from `EventJson`

The full, authoritative list of messages the server can send, with every
field and its JSON type, is generated straight from the Rust source and
checked in at
[`wire-protocol.schema.json`](./wire-protocol.schema.json) (JSON Schema,
draft-07). It is produced by `dawn_actor::protocol::event_json_schema()`,
which reflects the `EventJson` enum in
[`crates/dawn-actor/src/protocol.rs`](../../crates/dawn-actor/src/protocol.rs).

Read `wire-protocol.schema.json` for the exact contract. In summary, the
`"type"` values are: `ShipSpawned`, `VelocityChanged`, `ShipDespawned`,
`ShipDocked`, `ShipUndocked`, `DamageTaken`, `RepairApplied`,
`ShipDestroyed`, `TargetLocked`, `LockLost`, `ModuleActivated`,
`ModuleDeactivated`, `JumpGateUsed`, `StarSystemChanged`, and `Redirect`
(server-initiated reconnect to a different node on cross-node jump, see
ADR-0026 / multi-node clusters).

Every event carries `tick: u64` except `Redirect`, which is a transport
control message rather than a domain fact.

Not every `DomainEvent` reaches the wire -- `domain_event_to_json()` returns
`None` for internal bookkeeping events (`ShipFitted`, `WeaponFired`,
`TackleApplied`, `TackleReleased`, the `SectorTransit*` family,
`AnchorRebased`, `PackagedShipBuilt`, `ShipDisassembled`). See
`docs/architecture/event-catalog.md` for what those events mean server-side.

## Client -> server: generated from `ClientCommandJson`

The full list of messages a client can send, with every field and its JSON
type, is generated the same way and checked in at
[`wire-protocol-commands.schema.json`](./wire-protocol-commands.schema.json).
It is produced by `dawn_actor::protocol::client_command_json_schema()`,
which reflects the `ClientCommandJson` enum in `protocol.rs`.

The `"type"` values are: `MoveCommand`, `LockOnCommand`,
`ActivateModuleCommand`, `DeactivateModuleCommand`, `AttackCommand`,
`StopCommand`, `JumpCommand`, `ApproachCommand`, `WarpCommand`,
`OrbitCommand`, `KeepAtRangeCommand`, `FitModuleCommand`,
`UnfitModuleCommand`, `DockCommand`, `UndockCommand`,
`BuildPackagedShipCommand`, `DisassembleShipCommand`,
`SelectActiveShipCommand`.

**ADR-0037 (owned ship / active ship split):** `MoveCommand`, `LockOnCommand`,
`ActivateModuleCommand`, `DeactivateModuleCommand`, `StopCommand`,
`JumpCommand`, `ApproachCommand`, `WarpCommand`, `OrbitCommand`,
`KeepAtRangeCommand`, `DockCommand`, and `UndockCommand` carry no `ship_id`
field at all -- the server always resolves them against the caller's active
ship, so there is no wire-representable way to name a ship the player isn't
currently flying. `FitModuleCommand`, `UnfitModuleCommand`,
`BuildPackagedShipCommand`, and `DisassembleShipCommand` still carry an
explicit `ship_id`, since they may target any owned docked ship, not just the
active one. `SelectActiveShipCommand { ship_id }` is the only way to change
which owned ship is active (station-local switch only for now). See
`docs/architecture/ownership.md` §7.

`ClientCommandJson` mirrors the wire format exactly, including two
backward-compatible quirks it does not itself resolve (that validation
happens in `parse_client_command()`, same as before this enum existed):

- `WarpCommand` accepts a legacy `{"gate_id": N}` form and the current
  `{"target": {"Gate": N}}` / `{"target": {"Body": N}}` form. `target` wins
  if both are present; prefer it for new clients.
- `ApproachCommand`, `OrbitCommand`, and `KeepAtRangeCommand` select their
  target with either `gate_id` (a Jump Gate) or `target_id` (a Ship);
  `gate_id` wins if both are present, and the command is rejected
  (`parse_client_command` returns `None`) if neither is present.

`ActivateModuleCommand`'s `target_ship_id` is only required for targeted
module kinds (Weapon/Tackle, ADR-0035); the server validates that
requirement, not the wire schema.

## Keeping this in sync

`wire_schema_doc_is_up_to_date` (a test in `protocol.rs`) fails the build if
either checked-in schema file drifts from what `EventJson` /
`ClientCommandJson` currently produce. After changing either enum (or a type
either references -- `PosJson`, `VelJson`, `WarpTargetJson`), regenerate with:

```bash
cargo run -p dawn-actor --example gen_wire_schema
```

and commit both updated `.schema.json` files alongside the code change.
These files are documentation, generated from the types -- never hand-edit
them.

## Connection handshake

- A fresh client sends `{"type":"Hello"}`.
- A client resuming after a `Redirect` (cross-node jump) sends
  `{"type":"Hello","player_id":N,"ship_id":N}` to resume its identity on the
  new node instead of spawning fresh. See `parse_hello()` /
  `ResumeIdentity` in `protocol.rs`.
