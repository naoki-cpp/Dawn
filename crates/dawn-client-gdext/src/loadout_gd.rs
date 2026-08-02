use dawn_client_core::PlayerLoadoutMsg;
use godot::prelude::*;

#[cfg(test)]
use crate::client_outcome::validate_player_loadout_godot_ranges;
use crate::item_row_gd::ItemRow;
use crate::module_row_gd::{parse_kind, ModuleRow};
use crate::owned_ship_row_gd::OwnedShipRow;

fn wire_module_kind(kind: dawn_core::ModuleKind) -> dawn_client_core::ModuleKind {
    match kind {
        dawn_core::ModuleKind::Weapon => dawn_client_core::ModuleKind::Weapon,
        dawn_core::ModuleKind::ShieldBooster => dawn_client_core::ModuleKind::ShieldBooster,
        dawn_core::ModuleKind::ArmorRepairer => dawn_client_core::ModuleKind::ArmorRepairer,
        dawn_core::ModuleKind::Propulsion => dawn_client_core::ModuleKind::Propulsion,
        dawn_core::ModuleKind::Sensor => dawn_client_core::ModuleKind::Sensor,
        dawn_core::ModuleKind::Rig => dawn_client_core::ModuleKind::Rig,
        dawn_core::ModuleKind::Tackle => dawn_client_core::ModuleKind::Tackle,
        dawn_core::ModuleKind::RemoteShieldBooster => {
            dawn_client_core::ModuleKind::RemoteShieldBooster
        }
        dawn_core::ModuleKind::RemoteArmorRepairer => {
            dawn_client_core::ModuleKind::RemoteArmorRepairer
        }
    }
}

pub(crate) fn wire_to_loadout_msg(wire: dawn_wire::PlayerLoadoutWire) -> PlayerLoadoutMsg {
    PlayerLoadoutMsg {
        tick: wire.tick,
        modules: wire.modules.into_iter().map(wire_to_module_row).collect(),
        inventory: wire.inventory.into_iter().map(wire_to_item_row).collect(),
        station_inventory: wire
            .station_inventory
            .into_iter()
            .map(wire_to_item_row)
            .collect(),
        docked_station_id: wire.docked_station_id,
        docked_station_name: wire.docked_station_name,
        slot_capacity: dawn_client_core::SlotCapacity {
            high: wire.slot_capacity.high as u32,
            mid: wire.slot_capacity.mid as u32,
            low: wire.slot_capacity.low as u32,
            rig: wire.slot_capacity.rig as u32,
        },
        active_ship_id: wire.active_ship_id,
        owned_ships: wire
            .owned_ships
            .into_iter()
            .map(|ship| dawn_client_core::OwnedShipRow {
                ship_id: ship.ship_id,
                ship_type_id: ship.ship_type_id,
                ship_type_name: ship.ship_type_name,
                docked_station_id: ship.docked_station_id,
                is_active: ship.is_active,
            })
            .collect(),
    }
}

fn wire_to_module_row(row: dawn_wire::ModuleRowWire) -> dawn_client_core::ModuleRow {
    dawn_client_core::ModuleRow {
        slot: row.slot,
        index: row.index,
        module_id: row.module_id,
        name: row.name,
        kind: wire_module_kind(row.kind),
        is_active: row.is_active,
        is_active_module: row.is_active_module,
        cap_cost_per_cycle: row.cap_cost_per_cycle as f64,
        cycle_time_ticks: u32::try_from(row.cycle_time_ticks)
            .expect("PlayerLoadout range validation covers cycle_time_ticks"),
        stat_delta: row.stat_delta,
        cycle_remaining: 0,
        forced_reason: String::new(),
    }
}

fn wire_to_item_row(row: dawn_wire::ItemRowWire) -> dawn_client_core::ItemRow {
    dawn_client_core::ItemRow {
        item_id: dawn_core::ItemId::try_from(row.item_id)
            .expect("server emitted an invalid Item wire identity"),
        name: row.name,
        kind: row.kind,
        slot: row.slot,
        count: row.count,
    }
}

type Dict = Dictionary<Variant, Variant>;

#[derive(GodotClass)]
#[class(init, base=RefCounted)]
pub struct PlayerLoadout {
    loadout: Option<PlayerLoadoutMsg>,
}

#[godot_api]
impl PlayerLoadout {
    pub(crate) fn core_mut(&mut self) -> Option<&mut PlayerLoadoutMsg> {
        self.loadout.as_mut()
    }

    pub(crate) fn core_slot_mut(&mut self) -> &mut Option<PlayerLoadoutMsg> {
        &mut self.loadout
    }

    #[func]
    fn reset(&mut self) {
        self.loadout = None;
    }

    #[cfg(debug_assertions)]
    #[func]
    fn test_fixture(
        &mut self,
        tick: i64,
        modules: Array<Gd<ModuleRow>>,
        docked_station_id: i64,
        docked_station_name: GString,
        active_ship_id: i64,
        owned_ships: Array<Gd<OwnedShipRow>>,
    ) -> bool {
        let Ok(tick) = u64::try_from(tick) else {
            return false;
        };
        self.loadout = Some(PlayerLoadoutMsg {
            tick,
            modules: modules
                .iter_shared()
                .map(|row| row.bind().inner_clone())
                .collect(),
            inventory: Vec::new(),
            station_inventory: Vec::new(),
            docked_station_id: u32::try_from(docked_station_id).ok(),
            docked_station_name: (!docked_station_name.is_empty())
                .then(|| docked_station_name.to_string()),
            slot_capacity: dawn_client_core::SlotCapacity {
                high: 0,
                mid: 0,
                low: 0,
                rig: 0,
            },
            active_ship_id: u64::try_from(active_ship_id).ok(),
            owned_ships: owned_ships
                .iter_shared()
                .map(|row| row.bind().inner_clone())
                .collect(),
        });
        true
    }

    #[func]
    fn tick(&self) -> i64 {
        self.loadout
            .as_ref()
            .map(|loadout| {
                i64::try_from(loadout.tick).expect("PlayerLoadout range validation covers the tick")
            })
            .unwrap_or(0)
    }

    #[func]
    fn active_ship_id(&self) -> i64 {
        self.loadout
            .as_ref()
            .and_then(|loadout| loadout.active_ship_id)
            .map(|id| {
                i64::try_from(id).expect("PlayerLoadout range validation covers the active ship ID")
            })
            .unwrap_or(-1)
    }

    #[func]
    fn owned_ships(&self) -> Array<Gd<OwnedShipRow>> {
        let mut out = Array::new();
        if let Some(loadout) = &self.loadout {
            for ship in &loadout.owned_ships {
                out.push(&OwnedShipRow::wrap(ship.clone()));
            }
        }
        out
    }

    #[func]
    fn docked_station_id(&self) -> i64 {
        self.loadout
            .as_ref()
            .and_then(|loadout| loadout.docked_station_id)
            .map(i64::from)
            .unwrap_or(-1)
    }

    #[func]
    fn docked_station_name(&self) -> GString {
        self.loadout
            .as_ref()
            .and_then(|loadout| loadout.docked_station_name.as_deref())
            .unwrap_or_default()
            .into()
    }

    #[func]
    fn is_docked(&self) -> bool {
        self.loadout
            .as_ref()
            .is_some_and(PlayerLoadoutMsg::is_docked)
    }

    #[func]
    fn modules(&self) -> Array<Gd<ModuleRow>> {
        let mut out = Array::new();
        if let Some(loadout) = &self.loadout {
            for row in &loadout.modules {
                out.push(&ModuleRow::wrap(row.clone()));
            }
        }
        out
    }

    #[func]
    fn inventory(&self) -> Array<Gd<ItemRow>> {
        let mut out = Array::new();
        if let Some(loadout) = &self.loadout {
            for row in &loadout.inventory {
                out.push(&ItemRow::wrap(row.clone()));
            }
        }
        out
    }

    #[func]
    fn station_inventory(&self) -> Array<Gd<ItemRow>> {
        let mut out = Array::new();
        if let Some(loadout) = &self.loadout {
            for row in &loadout.station_inventory {
                out.push(&ItemRow::wrap(row.clone()));
            }
        }
        out
    }

    #[func]
    fn apply_module_activation(&mut self, module_id: i64, active: bool, forced_reason: GString) {
        let Some(loadout) = &mut self.loadout else {
            return;
        };
        let Ok(module_id) = u32::try_from(module_id) else {
            return;
        };
        loadout.apply_module_activation(module_id, active, forced_reason.to_string());
    }

    #[func]
    fn toggle_at(&self, active_index: i64) -> Dict {
        let mut result = Dict::new();
        let Some(loadout) = &self.loadout else {
            return result;
        };
        let Ok(index) = usize::try_from(active_index) else {
            return result;
        };
        let Some(intent) = loadout.toggle_at(index) else {
            return result;
        };
        result.set("module_id", intent.module_id as i64);
        result.set("slot", intent.slot);
        result.set("kind", crate::module_row_gd::kind_str(intent.kind));
        result.set("is_active", intent.is_active);
        result.set("requires_target", intent.requires_target);
        result.set("effective_range", intent.effective_range.unwrap_or(-1.0));
        result
    }

    #[func]
    fn weapon_optimal_range(&self) -> f64 {
        self.loadout
            .as_ref()
            .map(PlayerLoadoutMsg::weapon_ranges)
            .map(|(optimal, _)| optimal)
            .unwrap_or(0.0)
    }

    #[func]
    fn weapon_falloff_range(&self) -> f64 {
        self.loadout
            .as_ref()
            .map(PlayerLoadoutMsg::weapon_ranges)
            .map(|(_, falloff)| falloff)
            .unwrap_or(0.0)
    }

    #[func]
    fn effective_range_for_activation(&self, kind: GString, module_id: i64) -> f64 {
        let Some(loadout) = &self.loadout else {
            return -1.0;
        };
        let Ok(module_id) = u32::try_from(module_id) else {
            return -1.0;
        };
        loadout
            .effective_range_for_activation(parse_kind(&kind.to_string()), module_id)
            .unwrap_or(-1.0)
    }

    #[func]
    fn simulate_capacitor_ticks(
        &mut self,
        cap_current: f64,
        cap_max: f64,
        cap_recharge: f64,
        ticks: i64,
    ) -> f64 {
        let Some(loadout) = &mut self.loadout else {
            return cap_current;
        };
        loadout.simulate_capacitor_ticks(
            cap_current,
            cap_max,
            cap_recharge,
            u32::try_from(ticks).unwrap_or(0),
        )
    }

    #[func]
    fn simulate_modules_capacitor_ticks(
        modules: Array<Gd<ModuleRow>>,
        cap_current: f64,
        cap_max: f64,
        cap_recharge: f64,
        ticks: i64,
    ) -> f64 {
        let mut core_rows: Vec<_> = modules
            .iter_shared()
            .map(|row| row.bind().inner_clone())
            .collect();
        let cap = dawn_client_core::simulate_modules_capacitor_ticks(
            &mut core_rows,
            cap_current,
            cap_max,
            cap_recharge,
            u32::try_from(ticks).unwrap_or(0),
        );
        for (mut row, core) in modules.iter_shared().zip(core_rows) {
            row.bind_mut()
                .apply_simulated_cycle_remaining(core.cycle_remaining);
        }
        cap
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{FitModuleCommand, NodeId, Position, SectorBounds, SectorId, SlotKind};
    use dawn_sector::node::SimulationNode;

    fn test_node() -> SimulationNode {
        let mut node = SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(dawn_sector::galaxy::Galaxy::demo()),
        );
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let catalog = dawn_sector::game_data::GameDataCatalog::load_from_paths(
            root.join(dawn_sector::game_data::PRODUCTION_MODULES_PATH),
            root.join(dawn_sector::game_data::PRODUCTION_SHIP_TYPES_PATH),
        )
        .expect("repository game-data catalog");
        catalog.register_into(&mut node);
        node
    }

    #[test]
    fn production_player_loadout_converts_into_client_state() {
        let mut node = test_node();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);
        let wire = node.build_player_loadout_json(ship_id).unwrap();
        validate_player_loadout_godot_ranges(&wire).unwrap();
        let loadout = wire_to_loadout_msg(wire);
        assert_eq!(loadout.active_ship_id, Some(ship_id.raw()));
        assert!(!loadout.modules.is_empty());
        assert!(!loadout.inventory.is_empty());
    }

    #[test]
    fn fitted_module_stat_delta_converts() {
        let mut node = test_node();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);
        let module_id = dawn_sector::modules::MODULE_RAILGUN_SMALL;
        assert!(node.fit_module(FitModuleCommand {
            ship_id,
            slot: SlotKind::High,
            module_id,
        }));
        let wire = node.build_player_loadout_json(ship_id).unwrap();
        validate_player_loadout_godot_ranges(&wire).unwrap();
        let loadout = wire_to_loadout_msg(wire);
        let row = loadout
            .modules
            .iter()
            .find(|row| row.module_id == module_id.0)
            .unwrap();
        let _ = row.stat_delta.weapon_range_add;
    }
}
