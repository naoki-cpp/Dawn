# Baseline security review — 2026-07-10

The initial full review that this skill's diff-based process (Step 0)
measures against. If `docs/architecture/security-review.md` doesn't exist,
seed it from this file (copy the findings, set the front-matter date, then
maintain the doc — this reference stays frozen as the historical baseline).

## Verified clean

### A03 SQL injection — `crates/dawn-sector/src/node/station_inventory_db.rs`

All 5 SQL operations parameterized via `params![]` with positional
placeholders (`?1..?6`); no `format!`/concatenation builds SQL text:

- `get_all` (~line 90): SELECT
- `credit` (~line 127): INSERT ... ON CONFLICT upsert
- `try_debit` (~lines 156/182/196): SELECT / DELETE / UPDATE

Table/column names never derive from variable input: `item_id_to_columns`
(~line 25) is a closed `match` over the `ItemId` enum. The client's
`item_type` string is validated by a closed `match` in
`protocol/client_command.rs` (~line 365) and converted to `ItemId` before
the DB layer ever sees it.

### A03 non-SQL injection

No client string is used to build a file path, shell command, or format
string. String fields (`slot`, `item_type`, `direction`) all go through
closed-enum matches (`parse_slot_kind` ~line 390); unmatched values are
rejected, never interpolated. Log lines in `ws_server.rs` interpolate only
numeric/socket-derived values.

### A01 access control — 8 handlers spot-checked

- `fit_module_owned` / `unfit_module_owned` (`inventory.rs`): `owns_ship`
  first, plus docked-state check.
- `build_packaged_ship_owned` (`station_materialization.rs`): `owns_ship` +
  `can_use_station`.
- `disassemble_ship_owned`: `owns_ship` + docked-station match on **both**
  player and ship.
- `dock_owned` / `transfer_to_station_owned` / `assemble_ship_owned`:
  dispatched via `dispatch_station_command` (`command_station.rs`), which
  resolves Dock/Undock ship IDs from the player's own `active_ship` map —
  the client-supplied ID isn't trusted at all there.

### A04 command-layer allocation

`ClientCommandJson` is scalar-only (u32/u64/String/Option) — no `Vec<T>`,
no client-supplied count driving loops or allocation.

### A08 data integrity

Commands carry IDs/intents only. Costs are server-side constants
(`SCRAP_METAL_COST_PER_PACKAGED_SHIP`). Movement commands express targets;
tick systems own resulting state.

### A06 vulnerable components

`cargo audit` + `cargo deny` run in CI on every PR.

## Open findings

### SEC-1 (low, defer-with-trigger candidate): implicit WebSocket size limits

`crates/dawn-actor/src/ws_server.rs` (~line 245): `accept_async(stream)` is
called without an explicit `WebSocketConfig`, so `max_message_size` /
`max_frame_size` come from tokio-tungstenite's library defaults (64 MiB /
16 MiB at the pinned version) rather than a documented, chosen bound.
Not exploitable beyond memory pressure on a LAN; the fix is one
`accept_async_with_config` call. Related soft concern: `text.lines()`
processes every line in a frame with no per-connection rate limit —
bounded by the frame cap, fine for LAN.

### SEC-2 (high, exploitable): Hello resume grants ship ownership with no credential check

`crates/dawn-actor/src/protocol/hello_resume.rs` → `client_admission.rs`
(`select_handshake_identity`) → `spawner_logic.rs` (`adopt_player_ship`):
resume accepts a bare client-claimed `player_id`/`ship_id` pair. The only
check is that `ship_id` exists in the sector; `adopt_player_ship`
unconditionally overwrites `ships.owners`. Any client naming another
player's (sequentially-generated, wire-visible) `ship_id` takes over that
ship, and every downstream `owns_ship` check then trusts the hijacked
mapping. This is a consequence of the documented no-auth decision but is
distinct from it — it's not "we accept the risk of no auth," it's "the
resume path actively grants an unearned capability" — and stays a live
finding independent of when/whether auth gets added generally.

### SEC-3 (medium, exploitable): `transfer_to_station_owned` skips the ship-side dock check

`crates/dawn-sector/src/node/inventory.rs` (`transfer_to_station_owned`):
checks `owns_ship` and the player's own `can_use_station`, but never
verifies the *ship itself* is docked at the target station — unlike the
sibling `disassemble_ship_owned`, which checks both sides. Lets a player
teleport cargo from a ship docked elsewhere (or in open space) into station
inventory, bypassing the logistics constraint the docked-check exists to
enforce.

### SEC-4 (medium, exploitable): unbounded per-connection command queue

`crates/dawn-actor/src/ws_server.rs` (command receive task, `mpsc::unbounded_channel`)
and the per-tick drain in `dawn-sector-node/src/runtime.rs`: nothing bounds
how many commands a connection can queue before the server drains them, and
a single session's backlog is fully drained in one tick — a fast client can
grow server memory and monopolize tick time. Frame-size limits (SEC-1) don't
help here; this is queue *depth*, not message *size*.

### SEC-5 (medium, exploitable): unvalidated floating-point input reaches shared simulation state

`crates/dawn-actor/src/protocol/client_command.rs` (`PosJson`, movement/
orbit/range command fields, all `f32`/`f64`): no `is_finite()` check before
these values are converted into `Position`/`Velocity` and applied. A client
sending NaN or Infinity corrupts shared simulation state (poisoned physics,
navigation that never resolves) and — because the corrupted value can be
evented — the corruption survives replay permanently.
