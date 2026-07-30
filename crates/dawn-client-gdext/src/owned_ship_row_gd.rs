use dawn_client_core::OwnedShipRow as CoreOwnedShipRow;
use godot::prelude::*;

type Dict = Dictionary<Variant, Variant>;

const REQUIRED_KEYS: &[&str] = &[
    "ship_id",
    "ship_type_id",
    "ship_type_name",
    "docked_station_id",
    "is_active",
];

/// GDScript-facing typed view of one owned-ship roster row (ADR-0037).
/// Optional Rust fields preserve the existing Godot sentinels: `-1` for
/// absent numeric ids and an empty string for an absent type name.
#[derive(GodotClass)]
#[class(no_init, base=RefCounted)]
pub struct OwnedShipRow {
    #[var]
    ship_id: i64,
    #[var]
    ship_type_id: i64,
    #[var]
    ship_type_name: GString,
    #[var]
    docked_station_id: i64,
    #[var]
    is_active: bool,
}

impl OwnedShipRow {
    pub(crate) fn wrap(row: CoreOwnedShipRow) -> Gd<Self> {
        Gd::from_init_fn(|_base| Self {
            ship_id: row.ship_id as i64,
            ship_type_id: row.ship_type_id.map(i64::from).unwrap_or(-1),
            ship_type_name: row.ship_type_name.as_deref().unwrap_or_default().into(),
            docked_station_id: row.docked_station_id.map(i64::from).unwrap_or(-1),
            is_active: row.is_active,
        })
    }
}

#[godot_api]
impl OwnedShipRow {
    /// Test-fixture adapter matching the old Dictionary row shape.
    /// Missing keys fail loudly; explicit `null` optional fields map to
    /// the same sentinels as production Rust values.
    #[func]
    fn from_json(src: Dict) -> Variant {
        for key in REQUIRED_KEYS {
            if src.get(*key).is_none() {
                godot_error!("OwnedShipRow.from_json: invalid row, missing '{key}'");
                return Variant::nil();
            }
        }

        let get_i64 = |key: &str, default: i64| -> i64 {
            src.get(key)
                .and_then(|value| value.try_to::<i64>().ok())
                .unwrap_or(default)
        };
        let get_string = |key: &str| -> String {
            src.get(key)
                .and_then(|value| value.try_to::<GString>().ok())
                .map(|value| value.to_string())
                .unwrap_or_default()
        };
        let get_bool = |key: &str| -> bool {
            src.get(key)
                .and_then(|value| value.try_to::<bool>().ok())
                .unwrap_or(false)
        };

        let ship_type_id = get_i64("ship_type_id", -1);
        let ship_type_name = get_string("ship_type_name");
        let docked_station_id = get_i64("docked_station_id", -1);
        let row = CoreOwnedShipRow {
            ship_id: get_i64("ship_id", 0).max(0) as u64,
            ship_type_id: u32::try_from(ship_type_id).ok(),
            ship_type_name: (!ship_type_name.is_empty()).then_some(ship_type_name),
            docked_station_id: u32::try_from(docked_station_id).ok(),
            is_active: get_bool("is_active"),
        };
        Self::wrap(row).to_variant()
    }
}
