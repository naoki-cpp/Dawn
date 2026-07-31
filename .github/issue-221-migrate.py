from pathlib import Path


def replace(path: str, old: str, new: str, count: int = 1) -> None:
    file = Path(path)
    text = file.read_text()
    found = text.count(old)
    if found < count:
        raise SystemExit(f"{path}: expected at least {count} occurrence(s), found {found}: {old[:120]!r}")
    file.write_text(text.replace(old, new, count))


def append_before_last_brace(path: str, block: str) -> None:
    file = Path(path)
    text = file.read_text()
    pos = text.rfind("\n}")
    if pos < 0:
        raise SystemExit(f"{path}: final brace not found")
    file.write_text(text[:pos] + block + text[pos:])


# dawn-core exports the storage adapter alongside ItemId.
replace(
    "crates/dawn-core/src/lib.rs",
    "pub use item::ItemId;",
    "pub use item::{ItemId, ItemIdentityError, ItemStorageColumns};",
)

# dawn-wire owns one variant-preserving adapter for every Item-bearing message.
replace(
    "crates/dawn-wire/src/lib.rs",
    "mod initial_state;\nmod market;",
    "mod initial_state;\nmod item;\nmod market;",
)
replace(
    "crates/dawn-wire/src/lib.rs",
    "pub use initial_state::{\n    AbsPosWire, BuildableShipTypeWire, CelestialBodyWire, InitialStateWire, JumpGateWire,\n    ShipStateWire, StationWire, SystemWire,\n};\n",
    "pub use initial_state::{\n    AbsPosWire, BuildableShipTypeWire, CelestialBodyWire, InitialStateWire, JumpGateWire,\n    ShipStateWire, StationWire, SystemWire,\n};\npub use item::{ItemWire, ItemWireError};\n",
)

replace(
    "crates/dawn-wire/src/player_loadout.rs",
    "use dawn_core::{ModuleKind, StatDelta};",
    "use crate::ItemWire;\nuse dawn_core::{ModuleKind, StatDelta};",
)
replace(
    "crates/dawn-wire/src/player_loadout.rs",
    "/// One row of `PlayerLoadout`'s `inventory`/`station_inventory` arrays. The\n/// one shape every `ItemId` variant (Module/PackagedShip/ScrapMetal)\n/// projects into -- unused fields for a given variant are `0`/`\"\"`.\n#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]\npub struct ItemRowWire {\n    pub item_type: String,\n    pub module_id: u32,\n    pub ship_type_id: u32,\n    pub name: String,\n",
    "/// One row of `PlayerLoadout`'s `inventory`/`station_inventory` arrays.\n/// `item_id` preserves the Item variant and carries only its owned ID.\n#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]\npub struct ItemRowWire {\n    pub item_id: ItemWire,\n    pub name: String,\n",
)

Path("crates/dawn-wire/src/market.rs").write_text(r'''use crate::ItemWire;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A client request handled by the Market authority, outside the Sector
/// command stream (ADR-0034).
#[derive(Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum MarketCommandWire {
    /// Request the caller's Currency balance and currently open orders.
    RefreshMarketCommand {},
    /// Place a limit Bid or Ask for one item stack.
    PlaceMarketOrderCommand {
        ship_id: u64,
        item_id: ItemWire,
        side: String,
        price: u64,
        quantity: u64,
    },
    /// Cancel one of the caller's own open orders.
    CancelMarketOrderCommand { order_id: u64 },
}

/// One open order shown by the Market UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MarketOrderWire {
    pub order_id: u64,
    pub item_id: ItemWire,
    pub side: String,
    pub price: u64,
    pub quantity: u64,
    pub is_own: bool,
}

/// The server-owned Market state rendered by the client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MarketSnapshotWire {
    pub balance: u64,
    pub orders: Vec<MarketOrderWire>,
    pub notice: String,
}

/// Render the Market request wire schema as JSON Schema.
pub fn market_command_wire_json_schema() -> schemars::Schema {
    schemars::schema_for!(MarketCommandWire)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn place_order_round_trips_every_item_variant_through_json() {
        for item_id in [
            ItemWire::Module { module_id: 3 },
            ItemWire::PackagedShip { ship_type_id: 7 },
            ItemWire::ScrapMetal,
        ] {
            let command = MarketCommandWire::PlaceMarketOrderCommand {
                ship_id: 42,
                item_id,
                side: "Ask".to_owned(),
                price: 100,
                quantity: 3,
            };

            let json = serde_json::to_string(&command).expect("serialize");
            let decoded: MarketCommandWire = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(decoded, command);
        }
    }

    #[test]
    fn snapshot_preserves_item_variant_and_order_ownership_for_the_client() {
        let snapshot = MarketSnapshotWire {
            balance: 500,
            orders: vec![MarketOrderWire {
                order_id: 7,
                item_id: ItemWire::Module { module_id: 12 },
                side: "Bid".to_owned(),
                price: 25,
                quantity: 2,
                is_own: true,
            }],
            notice: "Order placed".to_owned(),
        };

        let bytes = postcard::to_stdvec(&snapshot).expect("encode");
        let decoded: MarketSnapshotWire = postcard::from_bytes(&bytes).expect("decode");
        assert_eq!(decoded, snapshot);
    }
}
''')

# Sector transfer commands use the same typed Item wire adapter.
replace(
    "crates/dawn-wire/src/client_command.rs",
    "use schemars::JsonSchema;",
    "use crate::ItemWire;\nuse schemars::JsonSchema;",
)
replace(
    "crates/dawn-wire/src/client_command.rs",
    "    /// Move the entire stack of an item between a docked ship's own cargo\n    /// and the caller's station inventory (ADR-0034 9B), in the direction\n    /// `direction` says (`\"ToStation\"` or `\"ToShip\"`). `item_type` is one of\n    /// `\"Module\"`, `\"PackagedShip\"`, `\"ScrapMetal\"` (matching `ItemRow`'s\n    /// wire shape) with `module_id`/`ship_type_id` populated only for the\n    /// variant that uses them (`0` otherwise).\n    TransferToStationCommand {\n        ship_id: u64,\n        station_id: u32,\n        item_type: String,\n        module_id: u32,\n        ship_type_id: u32,\n        direction: String,\n    },",
    "    /// Move the entire stack of an item between a docked ship's own cargo\n    /// and the caller's station inventory (ADR-0034 9B). `item_id` is a\n    /// variant-preserving identity, so unrelated ID combinations cannot be\n    /// represented on the wire.\n    TransferToStationCommand {\n        ship_id: u64,\n        station_id: u32,\n        item_id: ItemWire,\n        direction: String,\n    },",
)
replace(
    "crates/dawn-wire/src/client_command.rs",
    "        ClientCommandWire::TransferToStationCommand {\n            ship_id,\n            station_id,\n            item_type,\n            module_id,\n            ship_type_id,\n            direction,\n        } => {\n            let item_id = match item_type.as_str() {\n                \"Module\" => dawn_core::ItemId::Module(ModuleId(module_id)),\n                \"PackagedShip\" => {\n                    dawn_core::ItemId::PackagedShip(dawn_core::ShipTypeId(ship_type_id))\n                }\n                \"ScrapMetal\" => dawn_core::ItemId::ScrapMetal,\n                _ => return None,\n            };",
    "        ClientCommandWire::TransferToStationCommand {\n            ship_id,\n            station_id,\n            item_id,\n            direction,\n        } => {\n            let item_id = dawn_core::ItemId::try_from(item_id).ok()?;",
)
for old, new in [
    ('{\"TransferToStationCommand\":{\"ship_id\":42,\"station_id\":2,\"item_type\":\"ScrapMetal\",\"module_id\":0,\"ship_type_id\":0,\"direction\":\"ToStation\"}}',
     '{\"TransferToStationCommand\":{\"ship_id\":42,\"station_id\":2,\"item_id\":\"ScrapMetal\",\"direction\":\"ToStation\"}}'),
    ('{\"TransferToStationCommand\":{\"ship_id\":42,\"station_id\":2,\"item_type\":\"Module\",\"module_id\":7,\"ship_type_id\":0,\"direction\":\"ToStation\"}}',
     '{\"TransferToStationCommand\":{\"ship_id\":42,\"station_id\":2,\"item_id\":{\"Module\":{\"module_id\":7}},\"direction\":\"ToStation\"}}'),
    ('{\"TransferToStationCommand\":{\"ship_id\":42,\"station_id\":2,\"item_type\":\"ScrapMetal\",\"module_id\":0,\"ship_type_id\":0,\"direction\":\"ToShip\"}}',
     '{\"TransferToStationCommand\":{\"ship_id\":42,\"station_id\":2,\"item_id\":\"ScrapMetal\",\"direction\":\"ToShip\"}}'),
    ('{\"TransferToStationCommand\":{\"ship_id\":42,\"station_id\":2,\"item_type\":\"Bogus\",\"module_id\":0,\"ship_type_id\":0,\"direction\":\"ToStation\"}}',
     '{\"TransferToStationCommand\":{\"ship_id\":42,\"station_id\":2,\"item_id\":{\"Module\":{\"module_id\":0}},\"direction\":\"ToStation\"}}'),
    ('{\"TransferToStationCommand\":{\"ship_id\":42,\"station_id\":2,\"item_type\":\"ScrapMetal\",\"module_id\":0,\"ship_type_id\":0,\"direction\":\"Bogus\"}}',
     '{\"TransferToStationCommand\":{\"ship_id\":42,\"station_id\":2,\"item_id\":\"ScrapMetal\",\"direction\":\"Bogus\"}}'),
]:
    replace("crates/dawn-wire/src/client_command.rs", old, new)
replace(
    "crates/dawn-wire/src/client_command.rs",
    "fn transfer_to_station_command_json_with_unknown_item_type_fails_to_parse()",
    "fn transfer_to_station_command_json_with_invalid_item_identity_fails_to_convert()",
)

# Both SQLite authorities delegate the legacy columns to dawn-core.
replace(
    "crates/dawn-sector/src/node/station_inventory_db.rs",
    "//! Column encoding mirrors the flat `item_type`/`module_id`/`ship_type_id`\n//! shape `serialization.rs::item_id_to_row_json` already uses for the wire\n//! format, rather than inventing a new one -- easier to eyeball with a\n//! sqlite3 CLI, and one fewer encoding to keep in sync.\n",
    "//! The existing flat SQLite columns are preserved for on-disk compatibility,\n//! but their meaning is owned by `dawn_core::ItemId` rather than duplicated\n//! here.\n",
)
replace(
    "crates/dawn-sector/src/node/station_inventory_db.rs",
    "/// One row's flat encoding: `(item_type, module_id, ship_type_id)`.\n/// `module_id`/`ship_type_id` are `0` for whichever doesn't apply, matching\n/// `item_id_to_row_json`'s convention.\nfn item_id_to_columns(item_id: ItemId) -> (&'static str, u32, u32) {\n    match item_id {\n        ItemId::Module(module_id) => (\"Module\", module_id.0, 0),\n        ItemId::PackagedShip(ship_type_id) => (\"PackagedShip\", 0, ship_type_id.0),\n        ItemId::ScrapMetal => (\"ScrapMetal\", 0, 0),\n    }\n}\n\nfn columns_to_item_id(item_type: &str, module_id: u32, ship_type_id: u32) -> Option<ItemId> {\n    match item_type {\n        \"Module\" => Some(ItemId::Module(ModuleId(module_id))),\n        \"PackagedShip\" => Some(ItemId::PackagedShip(ShipTypeId(ship_type_id))),\n        \"ScrapMetal\" => Some(ItemId::ScrapMetal),\n        _ => None,\n    }\n}\n",
    "fn item_id_to_columns(item_id: ItemId) -> (&'static str, u32, u32) {\n    item_id.storage_columns().into_tuple()\n}\n\nfn columns_to_item_id(item_type: &str, module_id: u32, ship_type_id: u32) -> Option<ItemId> {\n    ItemId::from_storage_columns(item_type, module_id, ship_type_id).ok()\n}\n",
)
replace(
    "crates/dawn-sector/src/node/station_inventory_db.rs",
    "        db.credit(PlayerId(1), StationId(7), ItemId::Module(ModuleId(3)), 2);\n\n        let inv = db.get_all(PlayerId(1), StationId(7));\n        assert_eq!(inv.get(&ItemId::ScrapMetal), Some(&5));\n        assert_eq!(inv.get(&ItemId::Module(ModuleId(3))), Some(&2));",
    "        db.credit(PlayerId(1), StationId(7), ItemId::Module(ModuleId(3)), 2);\n        db.credit(\n            PlayerId(1),\n            StationId(7),\n            ItemId::PackagedShip(ShipTypeId(7)),\n            1,\n        );\n\n        let inv = db.get_all(PlayerId(1), StationId(7));\n        assert_eq!(inv.get(&ItemId::ScrapMetal), Some(&5));\n        assert_eq!(inv.get(&ItemId::Module(ModuleId(3))), Some(&2));\n        assert_eq!(inv.get(&ItemId::PackagedShip(ShipTypeId(7))), Some(&1));",
)

replace(
    "crates/dawn-market/src/order_book.rs",
    "//! Column encoding for `ItemId` mirrors the flat `item_type`/`module_id`/\n//! `ship_type_id` shape `dawn-sector`'s `station_inventory_db.rs` already\n//! uses -- easier to eyeball with a sqlite3 CLI, and one fewer encoding to\n//! keep in sync. This module can't reuse that code directly (`dawn-market`\n//! must not depend on `dawn-sector`, ADR-0034 §4), so the encoding is\n//! duplicated here.\n",
    "//! The existing flat SQLite columns remain unchanged for compatibility,\n//! while their encoding and validation are shared through `dawn_core::ItemId`.\n//! The Market therefore stays independent of `dawn-sector` without carrying\n//! a second Item mapping.\n",
)
replace(
    "crates/dawn-market/src/order_book.rs",
    "fn item_id_to_columns(item_id: ItemId) -> (&'static str, u32, u32) {\n    match item_id {\n        ItemId::Module(module_id) => (\"Module\", module_id.0, 0),\n        ItemId::PackagedShip(ship_type_id) => (\"PackagedShip\", 0, ship_type_id.0),\n        ItemId::ScrapMetal => (\"ScrapMetal\", 0, 0),\n    }\n}\n\nfn columns_to_item_id(item_type: &str, module_id: u32, ship_type_id: u32) -> Option<ItemId> {\n    match item_type {\n        \"Module\" => Some(ItemId::Module(ModuleId(module_id))),\n        \"PackagedShip\" => Some(ItemId::PackagedShip(ShipTypeId(ship_type_id))),\n        \"ScrapMetal\" => Some(ItemId::ScrapMetal),\n        _ => None,\n    }\n}\n",
    "fn item_id_to_columns(item_id: ItemId) -> (&'static str, u32, u32) {\n    item_id.storage_columns().into_tuple()\n}\n\nfn columns_to_item_id(item_type: &str, module_id: u32, ship_type_id: u32) -> Option<ItemId> {\n    ItemId::from_storage_columns(item_type, module_id, ship_type_id).ok()\n}\n",
)
append_before_last_brace(
    "crates/dawn-market/src/order_book.rs",
    r'''

    #[test]
    fn every_item_variant_round_trips_through_market_persistence() {
        let mut db = MarketDb::open_in_memory().unwrap();
        let player = PlayerId(1);
        let ship = ShipId(EntityId::from_raw(1));
        let expected = vec![
            ItemId::Module(ModuleId(3)),
            ItemId::PackagedShip(ShipTypeId(7)),
            ItemId::ScrapMetal,
        ];

        for item_id in &expected {
            db.place_order(player, ship, *item_id, OrderSide::Ask, 10, 1)
                .unwrap()
                .unwrap();
        }

        let actual: Vec<ItemId> = db
            .open_orders_for(player)
            .unwrap()
            .into_iter()
            .map(|order| order.item_id)
            .collect();
        assert_eq!(actual, expected);
    }
''',
)

# Loadout projection now emits a typed identity and no sentinel IDs.
replace(
    "crates/dawn-sector/src/node/player_loadout_projection.rs",
    "    ItemRowWire, ModuleRowWire, OwnedShipRowWire, PlayerLoadoutWire, SlotCapacityWire,\n",
    "    ItemRowWire, ItemWire, ModuleRowWire, OwnedShipRowWire, PlayerLoadoutWire,\n    SlotCapacityWire,\n",
)
replace(
    "crates/dawn-sector/src/node/player_loadout_projection.rs",
    "    /// The one seam every `ItemRowWire` (ship cargo, station inventory) goes\n    /// through. `0`/`\"\"` fill the fields a given `ItemId` variant doesn't\n    /// use. `None` if the registry backing `item_id` no longer has a\n",
    "    /// The one seam every `ItemRowWire` (ship cargo, station inventory) goes\n    /// through. The Item variant remains typed; only presentation metadata is\n    /// added here. `None` if the registry backing `item_id` no longer has a\n",
)
replace(
    "crates/dawn-sector/src/node/player_loadout_projection.rs",
    "                self.module_registry.get(&module_id).map(|def| ItemRowWire {\n                    item_type: \"Module\".to_string(),\n                    module_id: def.id.0,\n                    ship_type_id: 0,\n                    name: def.name.clone(),",
    "                self.module_registry.get(&module_id).map(|def| ItemRowWire {\n                    item_id: item_id.into(),\n                    name: def.name.clone(),",
)
replace(
    "crates/dawn-sector/src/node/player_loadout_projection.rs",
    "                    .map(|def| ItemRowWire {\n                        item_type: \"PackagedShip\".to_string(),\n                        module_id: 0,\n                        ship_type_id: def.id.0,\n                        name: def.name.clone(),",
    "                    .map(|def| ItemRowWire {\n                        item_id: item_id.into(),\n                        name: def.name.clone(),",
)
replace(
    "crates/dawn-sector/src/node/player_loadout_projection.rs",
    "            ItemId::ScrapMetal => Some(ItemRowWire {\n                item_type: \"ScrapMetal\".to_string(),\n                module_id: 0,\n                ship_type_id: 0,\n                name: \"Scrap Metal\".to_string(),",
    "            ItemId::ScrapMetal => Some(ItemRowWire {\n                item_id: item_id.into(),\n                name: \"Scrap Metal\".to_string(),",
)
replace(
    "crates/dawn-sector/src/node/player_loadout_projection.rs",
    '.find(|row| row.item_type == "ScrapMetal")',
    ".find(|row| row.item_id == ItemWire::ScrapMetal)",
    2,
)
replace(
    "crates/dawn-sector/src/node/player_loadout_projection.rs",
    'assert!(rows.iter().any(|r| r.item_type == "Module"));\n        assert!(rows.iter().any(|r| r.item_type == "ScrapMetal"));\n        assert!(rows.iter().any(|r| r.item_type == "PackagedShip"));',
    'assert!(rows\n            .iter()\n            .any(|r| matches!(r.item_id, ItemWire::Module { .. })));\n        assert!(rows.iter().any(|r| r.item_id == ItemWire::ScrapMetal));\n        assert!(rows\n            .iter()\n            .any(|r| matches!(r.item_id, ItemWire::PackagedShip { .. })));',
)

# Market request parsing and snapshots share ItemWire.
replace(
    "crates/dawn-simulation/src/serve/market.rs",
    "use dawn_wire::{MarketCommandWire, MarketOrderWire, MarketSnapshotWire};",
    "use dawn_wire::{ItemWire, MarketCommandWire, MarketOrderWire, MarketSnapshotWire};",
)
for _ in range(2):
    replace(
        "crates/dawn-simulation/src/serve/market.rs",
        "                item_type,\n                module_id,\n                ship_type_id,",
        "                item_id,",
    )
    replace(
        "crates/dawn-simulation/src/serve/market.rs",
        "                &item_type,\n                module_id,\n                ship_type_id,",
        "                item_id,",
    )
replace(
    "crates/dawn-simulation/src/serve/market.rs",
    "fn parse_order(\n    raw_ship_id: u64,\n    item_type: &str,\n    module_id: u32,\n    ship_type_id: u32,\n    side: &str,",
    "fn parse_order(\n    raw_ship_id: u64,\n    item_id: ItemWire,\n    side: &str,",
)
replace(
    "crates/dawn-simulation/src/serve/market.rs",
    "    let item_id = match item_type {\n        \"Module\" => ItemId::Module(dawn_core::ModuleId(module_id)),\n        \"PackagedShip\" => ItemId::PackagedShip(dawn_core::ShipTypeId(ship_type_id)),\n        \"ScrapMetal\" => ItemId::ScrapMetal,\n        _ => return None,\n    };",
    "    let item_id = ItemId::try_from(item_id).ok()?;",
)
replace(
    "crates/dawn-simulation/src/serve/market.rs",
    "    let (item_type, module_id, ship_type_id) = match order.item_id {\n        ItemId::Module(module_id) => (\"Module\", module_id.0, 0),\n        ItemId::PackagedShip(ship_type_id) => (\"PackagedShip\", 0, ship_type_id.0),\n        ItemId::ScrapMetal => (\"ScrapMetal\", 0, 0),\n    };\n    let order_id = u64::try_from(order.order_id.0).ok()?;\n    Some(MarketOrderWire {\n        order_id,\n        item_type: item_type.to_owned(),\n        module_id,\n        ship_type_id,",
    "    let order_id = u64::try_from(order.order_id.0).ok()?;\n    Some(MarketOrderWire {\n        order_id,\n        item_id: order.item_id.into(),",
)
replace(
    "crates/dawn-simulation/src/serve/market.rs",
    'assert!(parse_order(1, "ScrapMetal", 0, 0, "Ask", 0, 1).is_none());\n        assert!(parse_order(1, "ScrapMetal", 0, 0, "Ask", 1, 0).is_none());\n        assert!(parse_order(1, "ScrapMetal", 0, 0, "Ask", u64::MAX, 2).is_none());\n        assert!(parse_order(1, "Unknown", 0, 0, "Ask", 1, 1).is_none());',
    'assert!(parse_order(1, ItemWire::ScrapMetal, "Ask", 0, 1).is_none());\n        assert!(parse_order(1, ItemWire::ScrapMetal, "Ask", 1, 0).is_none());\n        assert!(parse_order(1, ItemWire::ScrapMetal, "Ask", u64::MAX, 2).is_none());\n        assert!(parse_order(\n            1,\n            ItemWire::Module { module_id: 0 },\n            "Ask",\n            1,\n            1\n        )\n        .is_none());',
)

# The client core exposes ItemId directly while retaining strict legacy JSON
# fixture compatibility for apply_payload tests.
Path("crates/dawn-client-core/src/item_row.rs").write_text(r'''use dawn_core::ItemId;
use serde::{de::Error as _, Deserialize, Deserializer};

/// One row shared by `PlayerLoadout`'s ship and station inventory arrays.
///
/// `item_id` is the same canonical variant used by the server domain. Display
/// metadata remains separate and cannot alter the Item identity.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemRow {
    pub item_id: ItemId,
    pub name: String,
    pub kind: String,
    pub slot: String,
    pub count: u64,
}

impl ItemRow {
    /// Compatibility projections for the existing Godot UI. Rust callers do
    /// not need string tags or unused-ID sentinels.
    pub const fn item_type(&self) -> &'static str {
        self.item_id.storage_columns().item_type()
    }

    pub const fn module_id(&self) -> u32 {
        self.item_id.storage_columns().module_id()
    }

    pub const fn ship_type_id(&self) -> u32 {
        self.item_id.storage_columns().ship_type_id()
    }
}

#[derive(Deserialize)]
struct ItemRowRepr {
    #[serde(default)]
    item_id: Option<ItemId>,
    #[serde(default)]
    item_type: Option<String>,
    #[serde(default)]
    module_id: u32,
    #[serde(default)]
    ship_type_id: u32,
    name: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    slot: String,
    count: u64,
}

impl<'de> Deserialize<'de> for ItemRow {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let row = ItemRowRepr::deserialize(deserializer)?;
        let item_id = match (row.item_id, row.item_type.as_deref()) {
            (Some(item_id), None) if row.module_id == 0 && row.ship_type_id == 0 => item_id,
            (None, Some(item_type)) => ItemId::from_storage_columns(
                item_type,
                row.module_id,
                row.ship_type_id,
            )
            .map_err(D::Error::custom)?,
            _ => {
                return Err(D::Error::custom(
                    "item row must contain exactly one canonical or legacy identity",
                ));
            }
        };

        Ok(Self {
            item_id,
            name: row.name,
            kind: row.kind,
            slot: row.slot,
            count: row.count,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{ModuleId, ShipTypeId};

    #[test]
    fn every_item_variant_parses_with_canonical_identity() {
        for (json, expected) in [
            (
                r#"{"item_id":{"Module":3},"name":"Railgun","kind":"Weapon","slot":"High","count":2}"#,
                ItemId::Module(ModuleId(3)),
            ),
            (
                r#"{"item_id":{"PackagedShip":7},"name":"Magpie","kind":"","slot":"","count":1}"#,
                ItemId::PackagedShip(ShipTypeId(7)),
            ),
            (
                r#"{"item_id":"ScrapMetal","name":"Scrap Metal","kind":"","slot":"","count":4}"#,
                ItemId::ScrapMetal,
            ),
        ] {
            let row: ItemRow = serde_json::from_str(json).unwrap();
            assert_eq!(row.item_id, expected);
        }
    }

    #[test]
    fn legacy_scrap_metal_row_still_parses() {
        let json = r#"{
            "item_type": "ScrapMetal",
            "module_id": 0,
            "ship_type_id": 0,
            "name": "Scrap Metal",
            "kind": "",
            "slot": "",
            "count": 3
        }"#;
        let row: ItemRow = serde_json::from_str(json).unwrap();
        assert_eq!(row.item_id, ItemId::ScrapMetal);
        assert_eq!(row.count, 3);
    }

    #[test]
    fn invalid_legacy_identity_is_rejected() {
        let json = r#"{
            "item_type": "Module",
            "module_id": 3,
            "ship_type_id": 7,
            "name": "Impossible",
            "count": 1
        }"#;
        assert!(serde_json::from_str::<ItemRow>(json).is_err());
    }
}
''')
replace(
    "crates/dawn-client-core/src/lib.rs",
    "pub use item_row::{ItemRow, ItemType};",
    "pub use item_row::ItemRow;",
)

# Wire -> client conversion keeps the canonical Item variant.
replace(
    "crates/dawn-client-gdext/src/loadout_gd.rs",
    "    dawn_client_core::ItemRow {\n        item_type: crate::item_row_gd::parse_item_type(&row.item_type),\n        module_id: row.module_id,\n        ship_type_id: row.ship_type_id,\n        name: row.name,",
    "    dawn_client_core::ItemRow {\n        item_id: dawn_core::ItemId::try_from(row.item_id)\n            .expect(\"server emitted an invalid Item wire identity\"),\n        name: row.name,",
)

Path("crates/dawn-client-gdext/src/item_row_gd.rs").write_text(r'''use dawn_client_core::ItemRow as CoreItemRow;
use dawn_core::ItemId;
use godot::prelude::*;

fn item_type_str(item_id: ItemId) -> &'static str {
    item_id.storage_columns().item_type()
}

/// Godot `Dictionary` value type used by `ItemRow::from_json`.
type Dict = Dictionary<Variant, Variant>;

const REQUIRED_KEYS: &[&str] = &[
    "item_type",
    "module_id",
    "ship_type_id",
    "name",
    "kind",
    "slot",
    "count",
];

/// GDScript-facing compatibility view of one typed client Item row.
///
/// Rust state keeps a canonical `ItemId`. The existing scalar properties are
/// derived read projections so current HUD code can migrate independently;
/// they are never used to reconstruct identity on the wire.
#[derive(GodotClass)]
#[class(no_init, base=RefCounted)]
pub struct ItemRow {
    #[var]
    item_type: GString,
    #[var]
    module_id: i64,
    #[var]
    ship_type_id: i64,
    #[var]
    name: GString,
    #[var]
    kind: GString,
    #[var]
    slot: GString,
    #[var]
    count: i64,
}

impl ItemRow {
    pub(crate) fn wrap(row: CoreItemRow) -> Gd<Self> {
        Gd::from_init_fn(|_base| Self {
            item_type: item_type_str(row.item_id).into(),
            module_id: row.module_id() as i64,
            ship_type_id: row.ship_type_id() as i64,
            name: (&row.name).into(),
            kind: (&row.kind).into(),
            slot: (&row.slot).into(),
            count: row.count as i64,
        })
    }
}

#[godot_api]
impl ItemRow {
    /// Parses the legacy plain Dictionary fixture used by GdUnit tests. The
    /// three identity fields are validated together before a typed ItemId is
    /// created; contradictory combinations are rejected.
    #[func]
    fn from_json(src: Dict) -> Variant {
        for key in REQUIRED_KEYS {
            if src.get(*key).is_none() {
                godot_error!("ItemRow.from_json: invalid item row, missing '{key}'");
                return Variant::nil();
            }
        }

        let get_gstring = |key: &str| -> String {
            src.get(key)
                .and_then(|v| v.try_to::<GString>().ok())
                .map(|s| s.to_string())
                .unwrap_or_default()
        };
        let get_i64 = |key: &str| -> i64 {
            src.get(key)
                .and_then(|v| v.try_to::<i64>().ok())
                .unwrap_or(0)
        };

        let item_type = get_gstring("item_type");
        let Ok(item_id) = ItemId::from_storage_columns(
            &item_type,
            get_i64("module_id") as u32,
            get_i64("ship_type_id") as u32,
        ) else {
            godot_error!("ItemRow.from_json: invalid Item identity");
            return Variant::nil();
        };

        let row = CoreItemRow {
            item_id,
            name: get_gstring("name"),
            kind: get_gstring("kind"),
            slot: get_gstring("slot"),
            count: get_i64("count") as u64,
        };
        Self::wrap(row).to_variant()
    }
}
''')

# GDExtension adapters accept the legacy UI arguments only at the edge, then
# immediately build typed ItemWire values.
replace(
    "crates/dawn-client-gdext/src/client_command_gd.rs",
    "    ClientCommandWire, ClientMessage, HelloMessage, MarketCommandWire, NavigationTargetWire,\n    PosWire, ResumeIdentity, WarpTargetWire,\n",
    "    ClientCommandWire, ClientMessage, HelloMessage, ItemWire, MarketCommandWire,\n    NavigationTargetWire, PosWire, ResumeIdentity, WarpTargetWire,\n",
)
replace(
    "crates/dawn-client-gdext/src/client_command_gd.rs",
    "fn non_negative_or_none(value: i64) -> Option<u64> {\n    if value >= 0 {\n        Some(value as u64)\n    } else {\n        None\n    }\n}\n",
    "fn non_negative_or_none(value: i64) -> Option<u64> {\n    if value >= 0 {\n        Some(value as u64)\n    } else {\n        None\n    }\n}\n\nfn item_wire_from_legacy_fields(\n    item_type: &str,\n    module_id: i64,\n    ship_type_id: i64,\n) -> Option<ItemWire> {\n    dawn_core::ItemId::from_storage_columns(\n        item_type,\n        u32::try_from(module_id).ok()?,\n        u32::try_from(ship_type_id).ok()?,\n    )\n    .ok()\n    .map(Into::into)\n}\n",
)
replace(
    "crates/dawn-client-gdext/src/client_command_gd.rs",
    "    /// Move the entire stack of an item out of a docked ship's own cargo\n    /// into the caller's station inventory (ADR-0034 9B). `item_type` is\n    /// one of `\"Module\"`/`\"PackagedShip\"`/`\"ScrapMetal\"` (matches\n    /// `ItemRow.item_type`); `module_id`/`ship_type_id` are only meaningful\n    /// for the matching variant (`0` otherwise).",
    "    /// Compatibility adapter for the existing Godot inventory surface.\n    /// The legacy scalar arguments are validated and collapsed into one\n    /// `ItemWire` before any bytes are encoded.",
)
# Insert a dedicated Market command builder before hello_command.
replace(
    "crates/dawn-client-gdext/src/client_command_gd.rs",
    "    /// Build the `ClientMessage::Hello` binary frame `connection.gd` sends",
    "    #[func]\n    fn market_place_order_command(\n        &self,\n        ship_id: i64,\n        item_type: GString,\n        module_id: i64,\n        ship_type_id: i64,\n        side: GString,\n        price: i64,\n        quantity: i64,\n    ) -> PackedByteArray {\n        let Some(item_id) = item_wire_from_legacy_fields(\n            &item_type.to_string(),\n            module_id,\n            ship_type_id,\n        ) else {\n            godot_error!(\"ClientCommand.market_place_order_command: invalid Item identity\");\n            return PackedByteArray::new();\n        };\n        let (Ok(price), Ok(quantity)) = (u64::try_from(price), u64::try_from(quantity)) else {\n            return PackedByteArray::new();\n        };\n        market_command_wire_bytes(MarketCommandWire::PlaceMarketOrderCommand {\n            ship_id: ship_id as u64,\n            item_id,\n            side: side.to_string(),\n            price,\n            quantity,\n        })\n    }\n\n    /// Build the `ClientMessage::Hello` binary frame `connection.gd` sends",
)
replace(
    "crates/dawn-client-gdext/src/client_command_gd.rs",
    "        command_wire_bytes(ClientCommandWire::TransferToStationCommand {\n            ship_id: ship_id as u64,\n            station_id: station_id as u32,\n            item_type: item_type.to_string(),\n            module_id: module_id as u32,\n            ship_type_id: ship_type_id as u32,\n            direction: direction.to_string(),\n        })",
    "        let Some(item_id) = item_wire_from_legacy_fields(\n            &item_type.to_string(),\n            module_id,\n            ship_type_id,\n        ) else {\n            godot_error!(\"ClientCommand.transfer_command: invalid Item identity\");\n            return PackedByteArray::new();\n        };\n        command_wire_bytes(ClientCommandWire::TransferToStationCommand {\n            ship_id: ship_id as u64,\n            station_id: station_id as u32,\n            item_id,\n            direction: direction.to_string(),\n        })",
)

# Connection delegates Market item construction to the typed GDExtension seam.
replace(
    "client/scripts/connection.gd",
    "\t_send_bytes(_cmd.market_build(\"PlaceMarketOrderCommand\", {\n\t\t\"ship_id\": p_ship_id,\n\t\t\"item_type\": p_item_type,\n\t\t\"module_id\": p_module_id,\n\t\t\"ship_type_id\": p_ship_type_id,\n\t\t\"side\": p_side,\n\t\t\"price\": p_price,\n\t\t\"quantity\": p_quantity,\n\t}))",
    "\t_send_bytes(_cmd.market_place_order_command(\n\t\tp_ship_id, p_item_type, p_module_id, p_ship_type_id,\n\t\tp_side, p_price, p_quantity))",
)

# Market snapshots carry the same externally-tagged identity as commands.
replace(
    "client/scripts/market_surface.gd",
    "func _order_item_name(order: Dictionary) -> String:\n\tmatch order.get(\"item_type\", \"\") as String:\n\t\t\"Module\": return \"Module #%d\" % (order.get(\"module_id\", 0) as int)\n\t\t\"PackagedShip\": return \"Ship #%d\" % (order.get(\"ship_type_id\", 0) as int)\n\t\t\"ScrapMetal\": return \"Scrap Metal\"\n\treturn \"Unknown item\"",
    "func _order_item_name(order: Dictionary) -> String:\n\tvar item_id: Variant = order.get(\"item_id\", null)\n\tif item_id is String and item_id as String == \"ScrapMetal\":\n\t\treturn \"Scrap Metal\"\n\tif item_id is Dictionary:\n\t\tvar tagged := item_id as Dictionary\n\t\tif tagged.has(\"Module\"):\n\t\t\tvar module: Dictionary = tagged[\"Module\"] as Dictionary\n\t\t\treturn \"Module #%d\" % (module.get(\"module_id\", 0) as int)\n\t\tif tagged.has(\"PackagedShip\"):\n\t\t\tvar ship: Dictionary = tagged[\"PackagedShip\"] as Dictionary\n\t\t\treturn \"Ship #%d\" % (ship.get(\"ship_type_id\", 0) as int)\n\treturn \"Unknown item\"",
)

# GdUnit contract expectations for the new tagged identity.
replace(
    "client/test/client_command_gd_test.gd",
    'assert_str(d["item_type"]).is_equal("ScrapMetal")',
    'assert_str(d["item_id"]).is_equal("ScrapMetal")',
)
replace(
    "client/test/client_command_gd_test.gd",
    'assert_int(int(d["module_id"])).is_equal(5)',
    'var item_id: Dictionary = d["item_id"]\n\tvar module: Dictionary = item_id["Module"]\n\tassert_int(int(module["module_id"])).is_equal(5)',
)
replace(
    "client/test/client_command_gd_test.gd",
    "func test_market_build_uses_the_market_envelope_and_preserves_order_fields() -> void:\n\tvar bytes := _cmd.market_build(\"PlaceMarketOrderCommand\", {\n\t\t\"ship_id\": 42,\n\t\t\"item_type\": \"ScrapMetal\",\n\t\t\"module_id\": 0,\n\t\t\"ship_type_id\": 0,\n\t\t\"side\": \"Ask\",\n\t\t\"price\": 100,\n\t\t\"quantity\": 3,\n\t})",
    "func test_market_command_preserves_the_typed_item_identity() -> void:\n\tvar bytes := _cmd.market_place_order_command(\n\t\t42, \"ScrapMetal\", 0, 0, \"Ask\", 100, 3)",
)
replace(
    "client/test/client_command_gd_test.gd",
    "\tassert_int(int(d[\"quantity\"])).is_equal(3)\n",
    "\tassert_int(int(d[\"quantity\"])).is_equal(3)\n\tassert_str(d[\"item_id\"]).is_equal(\"ScrapMetal\")\n",
    1,
)

# Architecture note for the new deep module.
Path("docs/architecture/item-identity.md").write_text(r'''# Item identity

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
''')

# Update the primary wire documentation's old flattening description.
wire_doc = Path("docs/architecture/wire-protocol.md")
text = wire_doc.read_text()
text = text.replace(
    "`item_type` plus `module_id` / `ship_type_id` sentinel fields",
    "the externally tagged `ItemWire` variant",
)
text = text.replace(
    "item_type / module_id / ship_type_id",
    "item_id (`ItemWire`)",
)
wire_doc.write_text(text)
