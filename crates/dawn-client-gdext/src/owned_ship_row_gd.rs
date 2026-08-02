use dawn_client_core::OwnedShipRow as CoreOwnedShipRow;
use godot::prelude::*;

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
            ship_id: i64::try_from(row.ship_id)
                .expect("PlayerLoadout range validation covers owned ship IDs"),
            ship_type_id: row.ship_type_id.map(i64::from).unwrap_or(-1),
            ship_type_name: row.ship_type_name.as_deref().unwrap_or_default().into(),
            docked_station_id: row.docked_station_id.map(i64::from).unwrap_or(-1),
            is_active: row.is_active,
        })
    }

    pub(crate) fn inner_clone(&self) -> CoreOwnedShipRow {
        CoreOwnedShipRow {
            ship_id: u64::try_from(self.ship_id).expect("OwnedShipRow stores a validated ship ID"),
            ship_type_id: u32::try_from(self.ship_type_id).ok(),
            ship_type_name: (!self.ship_type_name.is_empty())
                .then(|| self.ship_type_name.to_string()),
            docked_station_id: u32::try_from(self.docked_station_id).ok(),
            is_active: self.is_active,
        }
    }
}

#[godot_api]
impl OwnedShipRow {
    /// Debug-only typed fixture for GdUnit. Negative optional IDs and an empty
    /// type name represent absent values at the Godot boundary.
    #[cfg(debug_assertions)]
    #[func]
    fn test_fixture(
        ship_id: i64,
        ship_type_id: i64,
        ship_type_name: GString,
        docked_station_id: i64,
        is_active: bool,
    ) -> Variant {
        let Ok(ship_id) = u64::try_from(ship_id) else {
            return Variant::nil();
        };
        Self::wrap(CoreOwnedShipRow {
            ship_id,
            ship_type_id: u32::try_from(ship_type_id).ok(),
            ship_type_name: (!ship_type_name.is_empty()).then(|| ship_type_name.to_string()),
            docked_station_id: u32::try_from(docked_station_id).ok(),
            is_active,
        })
        .to_variant()
    }
}
