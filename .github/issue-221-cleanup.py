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
    "crates/dawn-sector/src/node/player_loadout_projection.rs",
    "};\n\nuse super::SimulationNode;",
    "};\n#[cfg(test)]\nuse dawn_wire::ItemWire;\n\nuse super::SimulationNode;",
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

replace(
    "crates/dawn-client-gdext/src/client_command_gd.rs",
    '''    #[func]
    fn market_place_order_command(
        &self,
        ship_id: i64,
        item_type: GString,
        module_id: i64,
        ship_type_id: i64,
        side: GString,
        price: i64,
        quantity: i64,
    ) -> PackedByteArray {
        let Some(item_id) = item_wire_from_legacy_fields(
            &item_type.to_string(),
            module_id,
            ship_type_id,
        ) else {
            godot_error!("ClientCommand.market_place_order_command: invalid Item identity");
            return PackedByteArray::new();
        };
        let (Ok(price), Ok(quantity)) = (u64::try_from(price), u64::try_from(quantity)) else {
            return PackedByteArray::new();
        };
        market_command_wire_bytes(MarketCommandWire::PlaceMarketOrderCommand {
            ship_id: ship_id as u64,
            item_id,
            side: side.to_string(),
            price,
            quantity,
        })
    }
''',
    '''    #[func]
    fn market_place_order_command(&self, fields: Dict) -> PackedByteArray {
        let Some(fields) = scalar_dict_to_json_object(&fields) else {
            return PackedByteArray::new();
        };
        let (
            Some(ship_id),
            Some(item_type),
            Some(module_id),
            Some(ship_type_id),
            Some(side),
            Some(price),
            Some(quantity),
        ) = (
            fields.get("ship_id").and_then(serde_json::Value::as_i64),
            fields.get("item_type").and_then(serde_json::Value::as_str),
            fields.get("module_id").and_then(serde_json::Value::as_i64),
            fields
                .get("ship_type_id")
                .and_then(serde_json::Value::as_i64),
            fields.get("side").and_then(serde_json::Value::as_str),
            fields.get("price").and_then(serde_json::Value::as_i64),
            fields.get("quantity").and_then(serde_json::Value::as_i64),
        )
        else {
            godot_error!("ClientCommand.market_place_order_command: missing or invalid field");
            return PackedByteArray::new();
        };
        let Some(item_id) = item_wire_from_legacy_fields(item_type, module_id, ship_type_id) else {
            godot_error!("ClientCommand.market_place_order_command: invalid Item identity");
            return PackedByteArray::new();
        };
        let (Ok(ship_id), Ok(price), Ok(quantity)) = (
            u64::try_from(ship_id),
            u64::try_from(price),
            u64::try_from(quantity),
        ) else {
            return PackedByteArray::new();
        };
        market_command_wire_bytes(MarketCommandWire::PlaceMarketOrderCommand {
            ship_id,
            item_id,
            side: side.to_owned(),
            price,
            quantity,
        })
    }
''',
)
replace(
    "client/scripts/connection.gd",
    '''\t_send_bytes(_cmd.market_place_order_command(
\t\tp_ship_id, p_item_type, p_module_id, p_ship_type_id,
\t\tp_side, p_price, p_quantity))''',
    '''\t_send_bytes(_cmd.market_place_order_command({
\t\t"ship_id": p_ship_id,
\t\t"item_type": p_item_type,
\t\t"module_id": p_module_id,
\t\t"ship_type_id": p_ship_type_id,
\t\t"side": p_side,
\t\t"price": p_price,
\t\t"quantity": p_quantity,
\t}))''',
)
replace(
    "client/test/client_command_gd_test.gd",
    '''\tvar bytes := _cmd.market_place_order_command(
\t\t42, "ScrapMetal", 0, 0, "Ask", 100, 3)''',
    '''\tvar bytes := _cmd.market_place_order_command({
\t\t"ship_id": 42,
\t\t"item_type": "ScrapMetal",
\t\t"module_id": 0,
\t\t"ship_type_id": 0,
\t\t"side": "Ask",
\t\t"price": 100,
\t\t"quantity": 3,
\t})''',
)
