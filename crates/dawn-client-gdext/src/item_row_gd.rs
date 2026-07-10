use dawn_client_core::{ItemRow as CoreItemRow, ItemType};
use godot::prelude::*;

/// `ItemType`'s wire-string name, exactly matching what the server sends
/// (`player_loadout_projection.rs::item_id_to_row_json`'s `"item_type"`
/// field), spelled out explicitly rather than relying on `Debug`.
fn item_type_str(item_type: ItemType) -> &'static str {
    match item_type {
        ItemType::Module => "Module",
        ItemType::PackagedShip => "PackagedShip",
        ItemType::ScrapMetal => "ScrapMetal",
        ItemType::Unknown => "",
    }
}

fn parse_item_type(item_type: &str) -> ItemType {
    match item_type {
        "Module" => ItemType::Module,
        "PackagedShip" => ItemType::PackagedShip,
        "ScrapMetal" => ItemType::ScrapMetal,
        _ => ItemType::Unknown,
    }
}

/// Godot `Dictionary` value type used by `ItemRow::from_json` -- matches
/// `loadout_gd::Dict`.
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

/// GDScript-facing view of one row shared by `PlayerLoadout`'s `inventory`
/// and `station_inventory` arrays (`dawn_client_core::ItemRow`). Field names
/// mirror the old `item_row.gd` so `hud_manager.gd`'s dot-access reads
/// (`item.item_type`, `item.slot`, ...) need no changes.
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
            item_type: item_type_str(row.item_type).into(),
            module_id: row.module_id as i64,
            ship_type_id: row.ship_type_id as i64,
            name: (&row.name).into(),
            kind: (&row.kind).into(),
            slot: (&row.slot).into(),
            count: row.count as i64,
        })
    }
}

#[godot_api]
impl ItemRow {
    /// Parses one item row out of a plain (non-wire-JSON) `Dictionary` --
    /// used directly by `hud_surface_test.gd`, mirroring the old
    /// `item_row.gd::from_json`. Returns `null` (after an error log) if a
    /// required key is missing, same "fail loudly, drop the row" contract as
    /// before.
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

        let row = CoreItemRow {
            item_type: parse_item_type(&get_gstring("item_type")),
            module_id: get_i64("module_id") as u32,
            ship_type_id: get_i64("ship_type_id") as u32,
            name: get_gstring("name"),
            kind: get_gstring("kind"),
            slot: get_gstring("slot"),
            count: get_i64("count") as u64,
        };
        Self::wrap(row).to_variant()
    }
}
