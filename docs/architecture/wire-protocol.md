---
scope    : Client<->server wire format over the WebSocket connection. What an
           external (non-Godot) client would need to talk to a Dawn server.
audience : AI Agent / Human Developer
update   : Server -> client half is generated; see "Keeping this in sync".
           Client -> server half (issue #94 part 2) is still hand-maintained.
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

### Keeping this in sync

`wire-schema_doc_is_up_to_date` (a test in `protocol.rs`) fails the build if
`wire-protocol.schema.json` drifts from what `EventJson` currently produces.
After changing `EventJson` (or `PosJson`/`VelJson`), regenerate with:

```bash
cargo run -p dawn-actor --example gen_wire_schema
```

and commit the updated `wire-protocol.schema.json` alongside the code change.
This file is documentation, generated from the types -- never hand-edit it.

## Client -> server: still hand-maintained

`parse_client_command()` in `protocol.rs` parses each incoming line by
inspecting `"type"` and pulling fields out of a `serde_json::Value` by hand.
It is not yet backed by a typed, schema-derivable enum, so there is no
generated schema for this half -- see issue #94 part 2. Until that lands,
read `parse_client_command()` directly for the exact set of accepted command
messages and their fields (`MoveCommand`, `LockOnCommand`,
`ActivateModuleCommand`, `DeactivateModuleCommand`, `AttackCommand`,
`StopCommand`, `JumpCommand`, `ApproachCommand`, `WarpCommand`,
`OrbitCommand`, `KeepAtRangeCommand`, `FitModuleCommand`,
`UnfitModuleCommand`, `DockCommand`, `UndockCommand`,
`BuildPackagedShipCommand`, `DisassembleShipCommand`).

Two format quirks worth knowing if you're implementing a new client:

- `WarpCommand` accepts a legacy `{"gate_id": N}` form and the current
  `{"target": {"Gate": N}}` / `{"target": {"Body": N}}` form. Prefer the
  `target` form; the legacy form only exists for backward compatibility.
- `ApproachCommand`, `OrbitCommand`, and `KeepAtRangeCommand` select their
  target with either `gate_id` (a Jump Gate) or `target_id` (a Ship); exactly
  one must be present.

## Connection handshake

- A fresh client sends `{"type":"Hello"}`.
- A client resuming after a `Redirect` (cross-node jump) sends
  `{"type":"Hello","player_id":N,"ship_id":N}` to resume its identity on the
  new node instead of spawning fresh. See `parse_hello()` /
  `ResumeIdentity` in `protocol.rs`.
