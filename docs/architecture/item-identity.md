# Item identity

`dawn_core::ItemId` is the canonical identity for every supported Item:

- `Module(ModuleId)`
- `PackagedShip(ShipTypeId)`
- `ScrapMetal`

Callers must not model an Item as an independent kind string plus several
optional or sentinel IDs. The enum variant owns its only valid identifier.

## Persistence

Station inventory and Market SQLite tables retain the existing
`item_type/module_id/ship_type_id` columns so deployed data remains readable.
Both authorities encode and validate those columns exclusively through
`ItemId::storage_columns` and `ItemId::from_storage_columns`. Invalid legacy
rows, including unrelated non-zero ID columns, never become an `ItemId`.

## Wire and client

`dawn_wire::ItemWire` is the postcard-compatible externally tagged projection
of `ItemId`. Item-bearing commands, Market snapshots, and PlayerLoadout rows
carry this variant directly. `dawn-client-core::ItemRow` stores `ItemId`, while
the Godot wrapper derives the old scalar properties only as a temporary UI
compatibility view.
