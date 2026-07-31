use dawn_client_core::ItemRow as CoreItemRow;
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
