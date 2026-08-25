use serde::Deserialize;

use dawn_core::ModuleKind;

/// One row of `PlayerLoadout`'s `modules` array (a fitted module slot).
/// Mirrors the shape `player_loadout_projection.rs::build_player_loadout_json`
/// serializes for each `FittedSlot`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ModuleRow {
    pub slot: String,
    pub index: u32,
    pub module_id: u32,
    pub name: String,
    pub kind: ModuleKind,
    pub is_active: bool,
    pub is_active_module: bool,
    pub cap_cost_per_cycle: f64,
    pub cycle_time_ticks: u32,
    /// `dawn_core::StatDelta` directly (not a client-side copy): the wire
    /// already carries this exact type unchanged (`ModuleRowWire.stat_delta`,
    /// `dawn-protocol`), so a client-side mirror only risked silently dropping
    /// fields as `dawn_core::StatDelta` grew -- it previously did, missing
    /// `weapon_cooldown_add`/`lock_time_add`/`max_locks_add`/`cap_max_add`/
    /// `cap_recharge_add`/`repair_amount`.
    pub stat_delta: dawn_core::StatDelta,

    /// Client-local runtime state, never read from the wire. Mutated by
    /// [`crate::PlayerLoadoutMsg::simulate_capacitor_ticks`] and activation
    /// toggling, mirroring the old `player_loadout.gd`'s `cycle_remaining`/
    /// `forced_reason` fields.
    #[serde(skip)]
    pub cycle_remaining: u32,
    #[serde(skip)]
    pub forced_reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_row_parses_the_wire_shape() {
        let json = r#"{
            "slot": "High",
            "index": 0,
            "module_id": 7,
            "name": "Pulse Laser",
            "kind": "Weapon",
            "is_active": true,
            "is_active_module": true,
            "cap_cost_per_cycle": 2.5,
            "cycle_time_ticks": 10,
            "stat_delta": {
                "weapon_damage_add": 10.0,
                "weapon_range_add": 5000.0,
                "falloff_range_add": 1000.0,
                "tracking_speed_add": 0.0,
                "speed_multiplier": 1.0,
                "mass_add": 0.0,
                "max_shield_add": 0.0,
                "max_armor_add": 0.0,
                "max_hull_add": 0.0,
                "weapon_cooldown_add": 0,
                "lock_time_add": 0,
                "max_locks_add": 0,
                "cap_max_add": 0.0,
                "cap_recharge_add": 0.0,
                "tackle_range_add": 0.0,
                "repair_amount": 0.0,
                "repair_range_add": 0.0
            }
        }"#;
        let row: ModuleRow = serde_json::from_str(json).unwrap();
        assert_eq!(row.module_id, 7);
        assert_eq!(row.kind, ModuleKind::Weapon);
        assert_eq!(row.stat_delta.weapon_range_add, 5000.0);
        // Client-only runtime fields are never read from the wire.
        assert_eq!(row.cycle_remaining, 0);
        assert_eq!(row.forced_reason, "");
    }
}
