use serde::Deserialize;

use crate::{ItemRow, ModuleKind, ModuleRow};

/// One row of `PlayerLoadout`'s `owned_ships` array (ADR-0037's full owned-ship
/// roster, active or not). Mirrors
/// `player_loadout_projection.rs::owned_ships_json`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OwnedShipRow {
    pub ship_id: u64,
    pub ship_type_id: Option<u32>,
    pub ship_type_name: Option<String>,
    pub docked_station_id: Option<u32>,
    pub is_active: bool,
}

/// Per-slot-kind fitted module capacity, keyed by slot name
/// (`"High"`/`"Mid"`/`"Low"`/`"Rig"`). Mirrors the `slot_capacity` object in
/// `player_loadout_projection.rs::build_player_loadout_json`.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct SlotCapacity {
    #[serde(rename = "High", default)]
    pub high: u32,
    #[serde(rename = "Mid", default)]
    pub mid: u32,
    #[serde(rename = "Low", default)]
    pub low: u32,
    #[serde(rename = "Rig", default)]
    pub rig: u32,
}

/// Client-side deep module for the server's `PlayerLoadout` wire message
/// (ADR-0039). Owns the richer client concept behind it: fitted modules,
/// ship/station inventory, dock context, and capacitor-cycle runtime
/// simulation. Godot-independent port of the old `player_loadout.gd`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PlayerLoadoutMsg {
    pub tick: u64,
    pub modules: Vec<ModuleRow>,
    pub inventory: Vec<ItemRow>,
    pub station_inventory: Vec<ItemRow>,
    pub docked_station_id: Option<u32>,
    pub docked_station_name: Option<String>,
    pub slot_capacity: SlotCapacity,
    pub active_ship_id: Option<u64>,
    pub owned_ships: Vec<OwnedShipRow>,
}

/// What activating a given module index would do (mirrors the old
/// `player_loadout.gd::toggle_at`'s return dictionary).
#[derive(Debug, Clone, PartialEq)]
pub struct ActivationIntent {
    pub module_id: u32,
    pub slot: String,
    pub kind: ModuleKind,
    pub is_active: bool,
    pub requires_target: bool,
    pub effective_range: Option<f64>,
}

/// Which range family a [`ModuleKind`] belongs to, for aggregating
/// [`StatDelta`](crate::StatDelta) range bonuses across all active modules of
/// that family. `None` for kinds with no range concept (e.g. `Propulsion`).
fn range_family(kind: ModuleKind) -> Option<RangeFamily> {
    match kind {
        ModuleKind::Weapon => Some(RangeFamily::Weapon),
        ModuleKind::Tackle => Some(RangeFamily::Tackle),
        ModuleKind::RemoteShieldBooster | ModuleKind::RemoteArmorRepairer => {
            Some(RangeFamily::Repair)
        }
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RangeFamily {
    Weapon,
    Tackle,
    Repair,
}

impl PlayerLoadoutMsg {
    pub fn is_docked(&self) -> bool {
        self.docked_station_id.is_some()
    }

    /// Sum of active weapons' optimal + falloff range (units).
    pub fn weapon_ranges(&self) -> (f64, f64) {
        let mut optimal = 0.0;
        let mut falloff = 0.0;
        for row in &self.modules {
            if !row.is_active || row.kind != ModuleKind::Weapon {
                continue;
            }
            optimal += row.stat_delta.weapon_range_add;
            falloff += row.stat_delta.falloff_range_add;
        }
        (optimal, falloff)
    }

    /// Total effective range for `kind`'s range family if module `module_id`
    /// were active alongside every other currently-active module sharing
    /// that family. `None` if `kind` has no range concept.
    pub fn effective_range_for_activation(&self, kind: ModuleKind, module_id: u32) -> Option<f64> {
        let family = range_family(kind)?;
        let mut total = 0.0;
        for row in &self.modules {
            let is_this_module = row.module_id == module_id;
            if !is_this_module && !row.is_active {
                continue;
            }
            if range_family(row.kind) != Some(family) {
                continue;
            }
            total += match family {
                RangeFamily::Weapon => {
                    row.stat_delta.weapon_range_add + row.stat_delta.falloff_range_add
                }
                RangeFamily::Tackle => row.stat_delta.tackle_range_add,
                RangeFamily::Repair => row.stat_delta.repair_range_add,
            };
        }
        Some(total)
    }

    /// The `active_index`-th currently-active-capable module (0-based,
    /// counting only `is_active_module` rows), or `None` if out of range.
    /// Mirrors the old `player_loadout.gd::toggle_at`.
    pub fn toggle_at(&self, active_index: usize) -> Option<ActivationIntent> {
        self.modules
            .iter()
            .filter(|row| row.is_active_module)
            .nth(active_index)
            .map(|row| ActivationIntent {
                module_id: row.module_id,
                slot: row.slot.clone(),
                kind: row.kind,
                is_active: row.is_active,
                requires_target: range_family(row.kind).is_some(),
                effective_range: self.effective_range_for_activation(row.kind, row.module_id),
            })
    }

    /// Advance every fitted module's capacitor cycle by `ticks`, mirroring
    /// the server's `CapacitorSystem` (`crates/dawn-ecs/src/systems/capacitor.rs`).
    /// Mutates `cycle_remaining` on each active module row and returns the
    /// resulting capacitor charge.
    pub fn simulate_capacitor_ticks(
        &mut self,
        cap_current: f64,
        cap_max: f64,
        cap_recharge: f64,
        ticks: u32,
    ) -> f64 {
        let mut cap = cap_current;
        for _ in 0..ticks {
            cap = (cap + cap_recharge).min(cap_max);
            for row in &mut self.modules {
                if !row.is_active_module || !row.is_active {
                    continue;
                }
                if row.cycle_remaining == 0 {
                    if row.cap_cost_per_cycle <= 0.0 || cap >= row.cap_cost_per_cycle {
                        cap -= row.cap_cost_per_cycle;
                        row.cycle_remaining = row.cycle_time_ticks;
                    }
                } else {
                    row.cycle_remaining -= 1;
                }
            }
            cap = cap.max(0.0);
        }
        cap
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StatDelta;

    fn zero_stat_delta() -> StatDelta {
        StatDelta {
            weapon_damage_add: 0.0,
            weapon_range_add: 0.0,
            falloff_range_add: 0.0,
            tracking_speed_add: 0.0,
            speed_multiplier: 1.0,
            mass_add: 0.0,
            max_shield_add: 0.0,
            max_armor_add: 0.0,
            max_hull_add: 0.0,
            tackle_range_add: 0.0,
            repair_range_add: 0.0,
        }
    }

    fn weapon_row(module_id: u32, is_active: bool, range: f64, falloff: f64) -> ModuleRow {
        ModuleRow {
            slot: "High".to_string(),
            index: 0,
            module_id,
            name: "Test Weapon".to_string(),
            kind: ModuleKind::Weapon,
            is_active,
            is_active_module: true,
            cap_cost_per_cycle: 5.0,
            cycle_time_ticks: 10,
            stat_delta: StatDelta {
                weapon_range_add: range,
                falloff_range_add: falloff,
                ..zero_stat_delta()
            },
            cycle_remaining: 0,
            forced_reason: String::new(),
        }
    }

    fn empty_loadout(modules: Vec<ModuleRow>) -> PlayerLoadoutMsg {
        PlayerLoadoutMsg {
            tick: 0,
            modules,
            inventory: Vec::new(),
            station_inventory: Vec::new(),
            docked_station_id: None,
            docked_station_name: None,
            slot_capacity: SlotCapacity {
                high: 4,
                mid: 4,
                low: 4,
                rig: 3,
            },
            active_ship_id: Some(1),
            owned_ships: Vec::new(),
        }
    }

    #[test]
    fn weapon_ranges_sums_only_active_weapons() {
        let loadout = empty_loadout(vec![
            weapon_row(1, true, 100.0, 20.0),
            weapon_row(2, false, 999.0, 999.0),
        ]);
        assert_eq!(loadout.weapon_ranges(), (100.0, 20.0));
    }

    #[test]
    fn effective_range_for_activation_includes_the_module_being_activated() {
        let loadout = empty_loadout(vec![weapon_row(1, false, 50.0, 10.0)]);
        let range = loadout.effective_range_for_activation(ModuleKind::Weapon, 1);
        assert_eq!(range, Some(60.0));
    }

    #[test]
    fn effective_range_for_activation_returns_none_for_kinds_without_a_range_family() {
        let mut row = weapon_row(1, true, 50.0, 0.0);
        row.kind = ModuleKind::Propulsion;
        let loadout = empty_loadout(vec![row]);
        assert_eq!(
            loadout.effective_range_for_activation(ModuleKind::Propulsion, 1),
            None
        );
    }

    #[test]
    fn toggle_at_counts_only_active_capable_modules() {
        let mut passive = weapon_row(2, false, 0.0, 0.0);
        passive.is_active_module = false;
        let loadout = empty_loadout(vec![passive, weapon_row(3, false, 40.0, 0.0)]);
        let intent = loadout.toggle_at(0).expect("one active-capable module");
        assert_eq!(intent.module_id, 3);
        assert!(intent.requires_target);
    }

    #[test]
    fn simulate_capacitor_ticks_drains_and_recharges() {
        let mut loadout = empty_loadout(vec![weapon_row(1, true, 0.0, 0.0)]);
        let cap = loadout.simulate_capacitor_ticks(10.0, 100.0, 1.0, 1);
        // recharge +1 -> 11, then cycle starts (cost 5) -> 6
        assert_eq!(cap, 6.0);
        assert_eq!(loadout.modules[0].cycle_remaining, 10);
    }

    #[test]
    fn simulate_capacitor_ticks_does_not_drain_when_capacitor_is_insufficient() {
        let mut loadout = empty_loadout(vec![weapon_row(1, true, 0.0, 0.0)]);
        let cap = loadout.simulate_capacitor_ticks(1.0, 100.0, 0.0, 1);
        // cap stays at 1 (below cap_cost_per_cycle 5.0), no cycle starts
        assert_eq!(cap, 1.0);
        assert_eq!(loadout.modules[0].cycle_remaining, 0);
    }

    #[test]
    fn is_docked_reflects_docked_station_id() {
        let mut loadout = empty_loadout(Vec::new());
        assert!(!loadout.is_docked());
        loadout.docked_station_id = Some(3);
        assert!(loadout.is_docked());
    }
}
