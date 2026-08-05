# Game-data catalog

`data/modules.toml` and `data/ship_types.toml` are the only authoritative
sources for module and hull balance values.

Every server composition root loads both files through
`dawn_sector::game_data::GameDataCatalog` before constructing a Sector. The
catalog parses and validates both categories as one startup operation. Missing
files, malformed TOML, duplicate IDs, unknown enum values, missing required
IDs, and invalid numeric ranges are fatal startup errors. A runtime must never
substitute a built-in or partial balance catalog.

## Construction invariant

A production `SimulationNode` cannot be created or restored without an
`Arc<GameDataCatalog>`. The catalog is therefore part of the Sector's required
construction state, alongside topology and the event store. A node is never
observable with empty registries and cannot be repaired later through
post-construction registration. Actor runtimes group topology and catalog in a
single Sector-construction configuration instead of growing positional APIs.

The catalog owns immutable module and ship-type definitions for the lifetime of
the process. Sector instances share its indexed definition maps through `Arc`
rather than cloning and independently mutating registries. Production and test
code have no `register_module`, `register_ship_type`, or `register_into` path.

Snapshot recovery receives the same catalog selected by the runtime:

```text
SimulationNode::restore_from(store, snapshot, galaxy, catalog)
```

This makes the definitions used for live execution and replay an explicit,
single dependency. Recovery cannot silently select a different process-global
catalog.

## Deterministic observation

`GameDataCatalog::modules()` and `GameDataCatalog::ship_types()` always expose
definitions in ascending stable-ID order, independent of file or caller input
order. Indexed lookups by `ModuleId` and `ShipTypeId` resolve the same immutable
definitions. Tests reverse the complete input vectors and verify that iteration
order and lookup results remain identical. Sector construction reuses these
prebuilt indexes; it does not rebuild an order-dependent registry per node.

`GameDataCatalog` is the only interface for loading module and ship-type
definitions. The stable identifiers in `dawn_sector::modules` and
`dawn_sector::ship_types` remain available to domain code, but those modules do
not expose definition catalogs. There are no category-only loaders, fallback
vectors, compatibility accessors, deprecated aliases, or process-global
catalog singleton.

Production startup resolves `data/` only relative to the process working
directory. It never searches the compile-time source checkout. External tests
and harnesses load a complete catalog through explicit paths. Inside
`dawn-sector`, crate-unit tests use one crate-private complete validated fixture;
they do not mutate definitions after constructing the node. Simulation,
client-binding, and production-node runtime tests use complete fixtures and
pass them through the same construction paths as production code. Workspace
CI checks these paths through all-target Clippy, full Rust tests, Windows
compilation, and the Godot integration suite.

When adding or changing game data:

1. Edit the appropriate file under `data/`.
2. Keep every field explicit when it represents a game rule.
3. Run `cargo test --workspace`; the catalog tests load the production files.
4. Deploy both TOML files with the server binaries.
