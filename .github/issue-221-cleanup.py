from pathlib import Path


def replace(path: str, old: str, new: str) -> None:
    file = Path(path)
    text = file.read_text()
    if old not in text:
        raise SystemExit(f"{path}: expected text not found: {old!r}")
    file.write_text(text.replace(old, new, 1))


replace(
    "crates/dawn-sector/src/node/player_loadout_projection.rs",
    "ItemRowWire, ItemWire, ModuleRowWire",
    "ItemRowWire, ModuleRowWire",
)
replace(
    "crates/dawn-sector/src/node/station_inventory_db.rs",
    "use dawn_core::{ItemId, ModuleId, PlayerId, ShipTypeId, StationId};",
    "use dawn_core::{ItemId, PlayerId, StationId};\n#[cfg(test)]\nuse dawn_core::{ModuleId, ShipTypeId};",
)
replace(
    "crates/dawn-market/src/order_book.rs",
    "use dawn_core::{\n    CreditItemCommand, EntityId, ItemId, ModuleId, PlayerId, RemoveItemCommand, ReturnItemCommand,\n    ShipId, ShipTypeId,\n};",
    "use dawn_core::{\n    CreditItemCommand, EntityId, ItemId, PlayerId, RemoveItemCommand, ReturnItemCommand, ShipId,\n};\n#[cfg(test)]\nuse dawn_core::{ModuleId, ShipTypeId};",
)
