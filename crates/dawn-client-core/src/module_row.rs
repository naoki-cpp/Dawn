use serde::Deserialize;

/// Wire-format mirror of `dawn_core::fitting::ModuleKind`'s `Debug` string
/// representation (`crates/dawn-sector/src/node/player_loadout_projection.rs`
/// serializes `kind` via `format!("{:?}", slot.def.kind)`). `Unknown` absorbs
/// any variant added server-side before this enum catches up, instead of
/// failing the whole payload to parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum ModuleKind {
    Weapon,
    ShieldBooster,
    ArmorRepairer,
    Propulsion,
    Sensor,
    Rig,
    Tackle,
    RemoteShieldBooster,
    RemoteArmorRepairer,
    #[serde(other)]
    Unknown,
}

/// Wire-format mirror of `dawn_core::fitting::StatDelta`. Carries every field
/// the server sends (`player_loadout_projection.rs::build_player_loadout_json`),
/// not just the ones a given caller happens to read today.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct StatDelta {
    #[serde(default)]
    pub weapon_damage_add: f64,
    #[serde(default)]
    pub weapon_range_add: f64,
    #[serde(default)]
    pub falloff_range_add: f64,
    #[serde(default)]
    pub tracking_speed_add: f64,
    #[serde(default = "default_speed_multiplier")]
    pub speed_multiplier: f64,
    #[serde(default)]
    pub mass_add: f64,
    #[serde(default)]
    pub max_shield_add: f64,
    #[serde(default)]
    pub max_armor_add: f64,
    #[serde(default)]
    pub max_hull_add: f64,
    #[serde(default)]
    pub tackle_range_add: f64,
    #[serde(default)]
    pub repair_range_add: f64,
}

fn default_speed_multiplier() -> f64 {
    1.0
}

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
    pub stat_delta: StatDelta,

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
    fn module_kind_falls_back_to_unknown_for_an_unrecognized_variant_name() {
        let kind: ModuleKind = serde_json::from_str("\"SomeFutureModuleKind\"").unwrap();
        assert_eq!(kind, ModuleKind::Unknown);
    }

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
                "tackle_range_add": 0.0,
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
