# Game-data catalog

`data/modules.toml` and `data/ship_types.toml` are the only authoritative
sources for module and hull balance values.

Every server runtime loads both files through
`dawn_sector::game_data::GameDataCatalog`. The catalog parses, validates, and
registers both categories as one startup operation. Missing files, malformed
TOML, duplicate IDs, unknown enum values, missing required IDs, and invalid
numeric ranges are fatal startup errors. A runtime must never substitute a
built-in balance catalog.

Rust retains stable IDs and simulation invariants. Compatibility functions such
as `modules::all_modules()` and `ship_types::all_ship_types()` read the same
repository TOML through `GameDataCatalog`; they do not contain fallback balance
definitions.

When adding or changing game data:

1. Edit the appropriate file under `data/`.
2. Keep every field explicit when it represents a game rule.
3. Run `cargo test --workspace`; the catalog tests load the production files.
4. Deploy both TOML files with the server binaries.
