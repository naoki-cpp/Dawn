# Game-data catalog

`data/modules.toml` and `data/ship_types.toml` are the only authoritative
sources for module and hull balance values.

Every server runtime loads both files through
`dawn_sector::game_data::GameDataCatalog`. The catalog parses, validates, and
registers both categories as one startup operation. Missing files, malformed
TOML, duplicate IDs, unknown enum values, missing required IDs, and invalid
numeric ranges are fatal startup errors. A runtime must never substitute a
built-in balance catalog.

`GameDataCatalog` is the only interface for loading module and ship-type
definitions. The stable identifiers in `dawn_sector::modules` and
`dawn_sector::ship_types` remain available to domain code, but those modules do
not expose definition catalogs. There are no category-only loaders, fallback
vectors, compatibility accessors, or deprecated aliases.

Production startup resolves `data/` only relative to the process working
directory. It never searches the compile-time source checkout. Tests either
construct a `GameDataCatalog` with explicit paths or, inside `dawn-sector`, use
the test-only repository catalog fixture. The fixture is crate-private, exists
only to remove repeated test setup, and is not compiled into the production
crate interface. Test callers consume the fixture's catalog slices directly
rather than reconstructing category-specific definition vectors.

Snapshot recovery uses definitions from the same catalog that configured the
node: callers pass `catalog.modules()` and `catalog.ship_types()` to
`SimulationNode::restore_from`.

When adding or changing game data:

1. Edit the appropriate file under `data/`.
2. Keep every field explicit when it represents a game rule.
3. Run `cargo test --workspace`; the catalog tests load the production files.
4. Deploy both TOML files with the server binaries.
