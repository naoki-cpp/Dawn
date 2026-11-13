//! Player loadout projection, activation policy, and client-side capacitor simulation.

use serde::Deserialize;

use dawn_core::{ModuleId, ModuleKind, ShipId, ShipTypeId, StationId};

use crate::{ItemRow, ModuleRow};

/// One row of `PlayerLoadout`'s `owned_ships` array (ADR-0037's full owned-ship
/// roster, active or not). Mirrors
/// `player_loadout_projection.rs::owned_ships_json`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OwnedShipRow {
    pub ship_id: ShipId,
    pub ship_type_id: Option<ShipTypeId>,
    pub ship_type_name: Option<String>,
    pub docked_station_id: Option<StationId>,
    pub is_active: bool,
}

/// Per-slot-kind fitted module capacity, keyed by slot name
/// (`"High"`/`"Mid"`/`"Low"`/`"Rig"`). Mirrors the `slot_capacity` object in
/// `player_loadout_projection.rs::build_player_loadout_json`.
#[derive(Debug, Clone, Copy, PartialEq, Default, Deserialize)]
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
///
/// Every field is `#[serde(default)]`: the old GDScript read this payload
/// with `payload.get(key, default)` everywhere, tolerating a partial
/// Dictionary (tests build minimal fixtures like `{"modules": [...]}`), and
/// this type keeps that same tolerance rather than requiring every key.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct PlayerLoadoutMsg {
    #[serde(default)]
    pub tick: u64,
    #[serde(default)]
    pub modules: Vec<ModuleRow>,
    #[serde(default)]
    pub inventory: Vec<ItemRow>,
    #[serde(default)]
    pub station_inventory: Vec<ItemRow>,
    #[serde(default)]
    pub docked_station_id: Option<StationId>,
    #[serde(default)]
    pub docked_station_name: Option<String>,
    #[serde(default)]
    pub slot_capacity: SlotCapacity,
    #[serde(default)]
    pub active_ship_id: Option<ShipId>,
    #[serde(default)]
    pub owned_ships: Vec<OwnedShipRow>,
}

/// The resolved result of one player module-toggle request.
#[derive(Debug, Clone, PartialEq)]
pub enum ModuleActivationDecision {
    Noop,
    Activate {
        module_id: ModuleId,
        slot: String,
        target_ship_id: Option<ShipId>,
    },
    Deactivate {
        module_id: ModuleId,
        slot: String,
    },
    Rejected(ModuleActivationRejection),
}

/// Why a requested module activation cannot be sent without server rejection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleActivationRejection {
    NoLockedTarget,
    TargetUnavailable,
    OutOfRange,
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
            optimal += f64::from(row.stat_delta.weapon_range_add);
            falloff += f64::from(row.stat_delta.falloff_range_add);
        }
        (optimal, falloff)
    }

    /// Total effective range for `kind`'s range family if module `module_id`
    /// were active alongside every other currently-active module sharing
    /// that family. `None` if `kind` has no range concept.
    pub fn effective_range_for_activation(
        &self,
        kind: ModuleKind,
        module_id: ModuleId,
    ) -> Option<f64> {
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
            total += f64::from(match family {
                RangeFamily::Weapon => {
                    row.stat_delta.weapon_range_add + row.stat_delta.falloff_range_add
                }
                RangeFamily::Tackle => row.stat_delta.tackle_range_add,
                RangeFamily::Repair => row.stat_delta.repair_range_add,
            });
        }
        Some(total)
    }

    /// Resolve one module toggle using the current loadout and client world facts.
    /// `target_distance` is measured in server units. A missing distance means
    /// either ship is unavailable in the local world projection. The server
    /// remains authoritative for the command.
    pub fn resolve_activation(
        &self,
        active_index: usize,
        has_player_ship: bool,
        locked_target_id: Option<ShipId>,
        target_distance: Option<f64>,
    ) -> ModuleActivationDecision {
        if !has_player_ship {
            return ModuleActivationDecision::Noop;
        }

        let Some(row) = self
            .modules
            .iter()
            .filter(|row| row.is_active_module)
            .nth(active_index)
        else {
            return ModuleActivationDecision::Noop;
        };

        if row.is_active {
            return ModuleActivationDecision::Deactivate {
                module_id: row.module_id,
                slot: row.slot.clone(),
            };
        }

        let requires_target = row.kind.requires_target();
        let target_ship_id = if requires_target {
            let Some(target_ship_id) = locked_target_id else {
                return ModuleActivationDecision::Rejected(
                    ModuleActivationRejection::NoLockedTarget,
                );
            };
            let Some(distance) =
                target_distance.filter(|distance| distance.is_finite() && *distance >= 0.0)
            else {
                return ModuleActivationDecision::Rejected(
                    ModuleActivationRejection::TargetUnavailable,
                );
            };
            if self
                .effective_range_for_activation(row.kind, row.module_id)
                .is_some_and(|range| distance > range)
            {
                return ModuleActivationDecision::Rejected(ModuleActivationRejection::OutOfRange);
            }
            Some(target_ship_id)
        } else {
            None
        };

        ModuleActivationDecision::Activate {
            module_id: row.module_id,
            slot: row.slot.clone(),
            target_ship_id,
        }
    }

    /// Applies a server-confirmed activation state to the module matching
    /// `module_id`, resetting its cycle timer (mirrors the old
    /// `player_loadout.gd::apply_module_activation`). No-op if no module
    /// with that id is fitted.
    pub fn apply_module_activation(
        &mut self,
        module_id: ModuleId,
        active: bool,
        forced_reason: String,
    ) {
        for row in &mut self.modules {
            if row.module_id == module_id {
                row.is_active = active;
                row.cycle_remaining = 0;
                row.forced_reason = forced_reason;
                return;
            }
        }
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
        simulate_modules_capacitor_ticks(
            &mut self.modules,
            cap_current,
            cap_max,
            cap_recharge,
            ticks,
        )
    }
}

/// Advance an arbitrary set of module rows' capacitor cycles by `ticks`,
/// mirroring the server's `CapacitorSystem`
/// (`crates/dawn-ecs/src/systems/capacitor.rs`). This is the free-function
/// form of [`PlayerLoadoutMsg::simulate_capacitor_ticks`] -- it exists
/// separately (rather than requiring a full `PlayerLoadoutMsg`) for
/// simulating a capacitor bar with an ad-hoc module list, or none at all
/// (an empty slice just recharges/decays with no drain).
pub fn simulate_modules_capacitor_ticks(
    modules: &mut [ModuleRow],
    cap_current: f64,
    cap_max: f64,
    cap_recharge: f64,
    ticks: u32,
) -> f64 {
    let mut cap = cap_current;
    for _ in 0..ticks {
        cap = (cap + cap_recharge).min(cap_max);
        for row in modules.iter_mut() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StatDelta;
    use dawn_core::{ModuleId, NodeId, ShipId, StationId};

    fn ship_id(id: u64) -> ShipId {
        ShipId::new(NodeId(0), id)
    }

    fn weapon_row(module_id: u32, is_active: bool, range: f64, falloff: f64) -> ModuleRow {
        ModuleRow {
            slot: "High".to_string(),
            index: 0,
            module_id: ModuleId(module_id),
            name: "Test Weapon".to_string(),
            kind: ModuleKind::Weapon,
            is_active,
            is_active_module: true,
            cap_cost_per_cycle: 5.0,
            cycle_time_ticks: 10,
            stat_delta: StatDelta {
                weapon_range_add: range as f32,
                falloff_range_add: falloff as f32,
                ..StatDelta::ZERO
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
            active_ship_id: Some(ship_id(1)),
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
        let range = loadout.effective_range_for_activation(ModuleKind::Weapon, ModuleId(1));
        assert_eq!(range, Some(60.0));
    }

    #[test]
    fn effective_range_for_activation_returns_none_for_kinds_without_a_range_family() {
        let mut row = weapon_row(1, true, 50.0, 0.0);
        row.kind = ModuleKind::Propulsion;
        let loadout = empty_loadout(vec![row]);
        assert_eq!(
            loadout.effective_range_for_activation(ModuleKind::Propulsion, ModuleId(1)),
            None
        );
    }

    #[test]
    fn apply_module_activation_updates_the_matching_module_and_resets_its_cycle() {
        let mut row = weapon_row(1, false, 0.0, 0.0);
        row.cycle_remaining = 7;
        let mut loadout = empty_loadout(vec![row]);

        loadout.apply_module_activation(ModuleId(1), true, "".to_string());

        assert!(loadout.modules[0].is_active);
        assert_eq!(loadout.modules[0].cycle_remaining, 0);
    }

    #[test]
    fn apply_module_activation_is_a_no_op_for_an_unknown_module_id() {
        let mut loadout = empty_loadout(vec![weapon_row(1, false, 0.0, 0.0)]);

        loadout.apply_module_activation(ModuleId(999), true, "".to_string());

        assert!(!loadout.modules[0].is_active);
    }

    #[test]
    fn resolve_activation_ignores_non_active_modules_when_selecting_an_index() {
        let mut passive = weapon_row(2, false, 0.0, 0.0);
        passive.is_active_module = false;
        let loadout = empty_loadout(vec![passive, weapon_row(3, false, 40.0, 0.0)]);
        assert_eq!(
            loadout.resolve_activation(0, true, Some(ship_id(9)), Some(1.0)),
            ModuleActivationDecision::Activate {
                module_id: ModuleId(3),
                slot: "High".to_string(),
                target_ship_id: Some(ship_id(9)),
            }
        );
    }

    #[test]
    fn resolve_activation_deactivates_an_active_module_without_target_facts() {
        let loadout = empty_loadout(vec![weapon_row(3, true, 40.0, 0.0)]);

        assert_eq!(
            loadout.resolve_activation(0, true, None, None),
            ModuleActivationDecision::Deactivate {
                module_id: ModuleId(3),
                slot: "High".to_string(),
            }
        );
    }

    #[test]
    fn resolve_activation_rejects_targeted_modules_without_a_locked_target() {
        let loadout = empty_loadout(vec![weapon_row(3, false, 40.0, 0.0)]);

        assert_eq!(
            loadout.resolve_activation(0, true, None, None),
            ModuleActivationDecision::Rejected(ModuleActivationRejection::NoLockedTarget)
        );
    }

    #[test]
    fn resolve_activation_requires_a_target_for_remote_repair_modules() {
        let mut module = weapon_row(3, false, 40.0, 0.0);
        module.kind = ModuleKind::RemoteArmorRepairer;
        let loadout = empty_loadout(vec![module]);

        assert_eq!(
            loadout.resolve_activation(0, true, None, None),
            ModuleActivationDecision::Rejected(ModuleActivationRejection::NoLockedTarget)
        );
    }

    #[test]
    fn resolve_activation_rejects_a_locked_target_without_local_position_data() {
        let loadout = empty_loadout(vec![weapon_row(3, false, 40.0, 0.0)]);

        assert_eq!(
            loadout.resolve_activation(0, true, Some(ship_id(9)), None),
            ModuleActivationDecision::Rejected(ModuleActivationRejection::TargetUnavailable)
        );
    }

    #[test]
    fn resolve_activation_rejects_targets_beyond_effective_range() {
        let loadout = empty_loadout(vec![weapon_row(3, false, 50.0, 10.0)]);

        assert_eq!(
            loadout.resolve_activation(0, true, Some(ship_id(9)), Some(60.1)),
            ModuleActivationDecision::Rejected(ModuleActivationRejection::OutOfRange)
        );
    }

    #[test]
    fn resolve_activation_allows_targets_at_the_effective_range_boundary() {
        let loadout = empty_loadout(vec![weapon_row(3, false, 50.0, 10.0)]);

        assert_eq!(
            loadout.resolve_activation(0, true, Some(ship_id(9)), Some(60.0)),
            ModuleActivationDecision::Activate {
                module_id: ModuleId(3),
                slot: "High".to_string(),
                target_ship_id: Some(ship_id(9)),
            }
        );
    }

    #[test]
    fn resolve_activation_activates_self_targeted_modules_without_a_target() {
        let mut propulsion = weapon_row(3, false, 0.0, 0.0);
        propulsion.kind = ModuleKind::Propulsion;
        let loadout = empty_loadout(vec![propulsion]);

        assert_eq!(
            loadout.resolve_activation(0, true, None, None),
            ModuleActivationDecision::Activate {
                module_id: ModuleId(3),
                slot: "High".to_string(),
                target_ship_id: None,
            }
        );
    }

    #[test]
    fn resolve_activation_is_a_noop_without_a_player_ship_or_matching_module() {
        let loadout = empty_loadout(vec![weapon_row(3, false, 40.0, 0.0)]);

        assert_eq!(
            loadout.resolve_activation(0, false, None, None),
            ModuleActivationDecision::Noop
        );
        assert_eq!(
            loadout.resolve_activation(1, true, None, None),
            ModuleActivationDecision::Noop
        );
    }

    #[test]
    fn resolve_activation_rejects_invalid_target_distance() {
        let loadout = empty_loadout(vec![weapon_row(3, false, 40.0, 0.0)]);

        assert_eq!(
            loadout.resolve_activation(0, true, Some(ship_id(9)), Some(f64::NAN)),
            ModuleActivationDecision::Rejected(ModuleActivationRejection::TargetUnavailable)
        );
        assert_eq!(
            loadout.resolve_activation(0, true, Some(ship_id(9)), Some(-0.1)),
            ModuleActivationDecision::Rejected(ModuleActivationRejection::TargetUnavailable)
        );
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
        loadout.docked_station_id = Some(StationId(3));
        assert!(loadout.is_docked());
    }
}
