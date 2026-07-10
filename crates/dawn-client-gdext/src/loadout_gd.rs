use dawn_client_core::PlayerLoadoutMsg;
use godot::prelude::*;

use crate::item_row_gd::ItemRow;
use crate::module_row_gd::{parse_kind, ModuleRow};

/// Godot's `Dictionary` is generic over key/value element type as of gdext
/// 0.5; these dictionaries hold a mix of String/int/bool/nested-collection
/// values (mirroring the old GDScript code's untyped `Dictionary` literals),
/// so both parameters are the type-erased `Variant`.
type Dict = Dictionary<Variant, Variant>;

/// Client-side deep module for the server's `PlayerLoadout` wire message
/// (ADR-0039/ADR-0040). GDScript-facing method surface mirrors the old
/// `player_loadout.gd` exactly (same method names, same `-1`/empty-string/
/// empty-Dictionary "nothing yet" sentinels) so `main.gd` needed no changes
/// beyond swapping the constructor and JSON-encoding the payload before
/// calling `apply_payload`.
#[derive(GodotClass)]
#[class(init, base=RefCounted)]
pub struct PlayerLoadout {
    loadout: Option<PlayerLoadoutMsg>,
}

#[godot_api]
impl PlayerLoadout {
    #[func]
    fn reset(&mut self) {
        self.loadout = None;
    }

    /// Parses a `PlayerLoadout` wire JSON string (see
    /// `player_loadout_projection.rs::build_player_loadout_json`). Returns
    /// `false` (and leaves the previous state untouched) if the JSON doesn't
    /// match this crate's expected shape.
    #[func]
    fn apply_payload(&mut self, json: GString) -> bool {
        match serde_json::from_str::<PlayerLoadoutMsg>(&json.to_string()) {
            Ok(loadout) => {
                self.loadout = Some(loadout);
                true
            }
            Err(err) => {
                godot_error!("PlayerLoadout.apply_payload: {err}");
                false
            }
        }
    }

    #[func]
    fn tick(&self) -> i64 {
        self.loadout.as_ref().map(|l| l.tick as i64).unwrap_or(0)
    }

    #[func]
    fn active_ship_id(&self) -> i64 {
        self.loadout
            .as_ref()
            .and_then(|l| l.active_ship_id)
            .map(|id| id as i64)
            .unwrap_or(-1)
    }

    #[func]
    fn owned_ships(&self) -> Array<Dict> {
        let mut out = Array::new();
        let Some(loadout) = &self.loadout else {
            return out;
        };
        for ship in &loadout.owned_ships {
            let mut d = Dict::new();
            d.set("ship_id", ship.ship_id as i64);
            d.set(
                "ship_type_id",
                ship.ship_type_id.map(|id| id as i64).unwrap_or(-1),
            );
            d.set(
                "ship_type_name",
                ship.ship_type_name.clone().unwrap_or_default(),
            );
            d.set(
                "docked_station_id",
                ship.docked_station_id.map(|id| id as i64).unwrap_or(-1),
            );
            d.set("is_active", ship.is_active);
            out.push(&d);
        }
        out
    }

    #[func]
    fn dock_status(&self) -> Dict {
        let mut d = Dict::new();
        match &self.loadout {
            Some(loadout) => {
                d.set(
                    "docked_station_id",
                    loadout.docked_station_id.map(|id| id as i64).unwrap_or(-1),
                );
                d.set(
                    "docked_station_name",
                    loadout.docked_station_name.clone().unwrap_or_default(),
                );
                d.set("is_docked", loadout.is_docked());
            }
            None => {
                d.set("docked_station_id", -1_i64);
                d.set("docked_station_name", "");
                d.set("is_docked", false);
            }
        }
        d
    }

    #[func]
    fn hud_snapshot(&self) -> Dict {
        let mut d = Dict::new();
        d.set("modules", &self.modules());
        d.set("inventory", &self.inventory());
        d.set("station_inventory", &self.station_inventory());
        d.set("dock_status", &self.dock_status());
        d.set("owned_ships", &self.owned_ships());
        d
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
        for row in &mut loadout.modules {
            if row.module_id as i64 == module_id {
                row.is_active = active;
                row.cycle_remaining = 0;
                row.forced_reason = forced_reason.to_string();
                return;
            }
        }
    }

    /// `{}` (empty Dictionary) if `active_index` is out of range, matching
    /// the old `player_loadout.gd::toggle_at`'s "nothing to toggle" sentinel.
    #[func]
    fn toggle_at(&self, active_index: i64) -> Dict {
        let mut d = Dict::new();
        let Some(loadout) = &self.loadout else {
            return d;
        };
        let Ok(index) = usize::try_from(active_index) else {
            return d;
        };
        let Some(intent) = loadout.toggle_at(index) else {
            return d;
        };
        d.set("module_id", intent.module_id as i64);
        d.set("slot", intent.slot);
        d.set("kind", crate::module_row_gd::kind_str(intent.kind));
        d.set("is_active", intent.is_active);
        d.set("requires_target", intent.requires_target);
        d.set("effective_range", intent.effective_range.unwrap_or(-1.0));
        d
    }

    #[func]
    fn weapon_ranges(&self) -> Dict {
        let (optimal, falloff) = self
            .loadout
            .as_ref()
            .map(|l| l.weapon_ranges())
            .unwrap_or((0.0, 0.0));
        let mut d = Dict::new();
        d.set("optimal", optimal);
        d.set("falloff", falloff);
        d
    }

    /// `-1.0` if `kind` has no range concept, matching the old
    /// `player_loadout.gd::effective_range_for_activation`'s sentinel.
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
        let ticks = u32::try_from(ticks).unwrap_or(0);
        loadout.simulate_capacitor_ticks(cap_current, cap_max, cap_recharge, ticks)
    }

    /// Static form of [`Self::simulate_capacitor_ticks`], operating on an
    /// ad-hoc module list instead of this instance's own state. Used by
    /// `world_session.gd::simulate_cap` for the "no `PlayerLoadout` instance
    /// yet, but I have a raw module list" case -- mirrors the old
    /// `player_loadout.gd::simulate_modules_capacitor_ticks` static function.
    #[func]
    fn simulate_modules_capacitor_ticks(
        modules: Array<Gd<ModuleRow>>,
        cap_current: f64,
        cap_max: f64,
        cap_recharge: f64,
        ticks: i64,
    ) -> f64 {
        let ticks = u32::try_from(ticks).unwrap_or(0);
        let mut core_rows: Vec<_> = modules
            .iter_shared()
            .map(|gd| gd.bind().inner_clone())
            .collect();
        let cap = dawn_client_core::simulate_modules_capacitor_ticks(
            &mut core_rows,
            cap_current,
            cap_max,
            cap_recharge,
            ticks,
        );
        for (mut gd, row) in modules.iter_shared().zip(core_rows) {
            gd.bind_mut()
                .apply_simulated_cycle_remaining(row.cycle_remaining);
        }
        cap
    }
}
