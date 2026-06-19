//! JSON serialization helpers for `SimulationNode`.
//!
//! All methods that convert node state into the wire JSON the Godot client
//! expects live here, keeping the core simulation logic in `mod.rs` separate
//! from the presentation layer.

use dawn_core::{CelestialBodyKind, Position, ShipId};
use dawn_ecs::components::{FittingComp, HullComp, PositionComp, ShipStatsComp};
use dawn_event_store::store::EventStore;

use super::SimulationNode;

impl<S: EventStore> SimulationNode<S> {
    /// Return the player ship's fitting state as a PlayerFitting JSON message.
    ///
    /// Sent after Welcome + InitialState on connect. Format:
    /// ```json
    /// {"type":"PlayerFitting","modules":[
    ///   {"slot":"High","index":0,"module_id":1,"name":"Small Railgun I","is_active":false}
    /// ]}
    /// ```
    pub fn build_player_fitting_json(&self, ship_id: ShipId) -> Option<String> {
        let entity = self.ship_index.get(&ship_id)?;
        let fitting = self.world.inner().get::<&FittingComp>(*entity).ok()?;

        let mut modules: Vec<serde_json::Value> = Vec::new();
        let slot_names = [("High", &fitting.high), ("Mid", &fitting.mid),
                          ("Low", &fitting.low), ("Rig", &fitting.rig)];
        for (slot_name, slots) in &slot_names {
            for (i, slot) in slots.iter().enumerate() {
                let d = &slot.def.stat_delta;
                modules.push(serde_json::json!({
                    "slot"             : slot_name,
                    "index"            : i,
                    "module_id"        : slot.def.id.0,
                    "name"             : slot.def.name,
                    "kind"             : format!("{:?}", slot.def.kind),
                    "is_active"        : slot.is_active,
                    "is_active_module" : matches!(slot.def.activation_mode, dawn_core::ActivationMode::Active),
                    "cap_cost_per_cycle": slot.def.cap_cost_per_cycle,
                    "cycle_time_ticks" : slot.def.cycle_time_ticks,
                    "stat_delta": {
                        "weapon_damage_add"   : d.weapon_damage_add,
                        "weapon_range_add"    : d.weapon_range_add,
                        "falloff_range_add"   : d.falloff_range_add,
                        "tracking_speed_add"  : d.tracking_speed_add,
                        "speed_multiplier"    : d.speed_multiplier,
                        "mass_add"            : d.mass_add,
                        "max_shield_add"      : d.max_shield_add,
                        "max_armor_add"       : d.max_armor_add,
                        "max_hull_add"        : d.max_hull_add,
                    },
                }));
            }
        }

        Some(serde_json::json!({
            "type"   : "PlayerFitting",
            "modules": modules,
        }).to_string())
    }

    /// Full-world `InitialState` (every ship). Used for non-AoI callers.
    pub fn build_initial_state_json(&self) -> String {
        self.initial_state_json(self.ship_index.keys().copied())
    }

    /// `InitialState` scoped to an observer's Area of Interest: only ships in the
    /// 27-cell neighborhood of `observer_pos` (ADR-0019).
    pub fn build_initial_state_json_for(&self, observer_pos: Position, cell_size: f32) -> String {
        self.initial_state_json(self.ships_visible_to(observer_pos, cell_size).into_iter())
    }

    /// Serialise the given ships into an `InitialState` message.
    fn initial_state_json(&self, ship_ids: impl Iterator<Item = ShipId>) -> String {
        let ships: Vec<serde_json::Value> =
            ship_ids.filter_map(|ship_id| self.ship_state_json(ship_id)).collect();

        let bodies: Vec<serde_json::Value> = self.celestial_bodies.values().map(|b| {
            serde_json::json!({
                "id"           : b.id.0,
                "kind"         : match b.kind {
                    CelestialBodyKind::Star   => "Star",
                    CelestialBodyKind::Planet => "Planet",
                },
                "name"         : b.name,
                "position"     : { "x": b.position.x, "y": b.position.y, "z": b.position.z },
                "radius"       : b.radius,
                "spectral_type": b.spectral_type,
            })
        }).collect();

        serde_json::json!({
            "type"             : "InitialState",
            "ships"            : ships,
            "celestial_bodies" : bodies,
        }).to_string()
    }

    /// Per-ship state object (position, stats, hull, ownership). Shared by
    /// `InitialState` and `AoiEnter` (ADR-0019). `None` if the ship is gone.
    pub fn ship_state_json(&self, ship_id: ShipId) -> Option<serde_json::Value> {
        let entity  = self.ship_index.get(&ship_id)?;
        let pos     = self.world.inner().get::<&PositionComp>(*entity).ok()?.0;
        let stats   = self.world.inner().get::<&ShipStatsComp>(*entity).ok()?;
        let hull    = self.world.inner().get::<&HullComp>(*entity).ok()?;
        let is_player = self.ship_owners.contains_key(&ship_id);
        let ship_type_name = self.ship_type_ids.get(&ship_id)
            .and_then(|tid| self.ship_type_registry.get(tid))
            .map(|def| def.name.as_str())
            .unwrap_or("Unknown");
        Some(serde_json::json!({
            "ship_id"              : ship_id.raw(),
            "ship_type_name"       : ship_type_name,
            "position"             : { "x": pos.x, "y": pos.y, "z": pos.z },
            "max_shield"           : stats.max_shield,
            "max_armor"            : stats.max_armor,
            "max_hull"             : stats.max_hull,
            "current_shield"       : hull.current_shield,
            "current_armor"        : hull.current_armor,
            "current_hull"         : hull.current_hull,
            "cap_max"              : stats.cap_max,
            "cap_recharge_per_tick": stats.cap_recharge_per_tick,
            "is_player"            : is_player,
        }))
    }

    /// `AoiEnter` control message for a ship that just entered an observer's
    /// neighborhood (ADR-0019). `None` if the ship is gone. The matching
    /// `AoiLeave` is a free function ([`crate::aoi::aoi_leave_json`]) since it
    /// needs no node state.
    pub fn aoi_enter_json(&self, ship_id: ShipId) -> Option<String> {
        let ship = self.ship_state_json(ship_id)?;
        Some(serde_json::json!({ "type": "AoiEnter", "ship": ship }).to_string())
    }

    // ── Area of Interest (ADR-0019) ────────────────────────────────────────────

    /// `(ShipId, Position)` for every ship currently in the world — the input to
    /// the AoI cell grid. Iteration order is unspecified, but `CellGrid` sorts
    /// each bucket, so query results are deterministic regardless.
    pub fn ship_positions(&self) -> Vec<(ShipId, Position)> {
        self.ship_index.iter().filter_map(|(&id, &entity)| {
            let pos = self.world.inner().get::<&PositionComp>(entity).ok()?.0;
            Some((id, pos))
        }).collect()
    }

    /// ShipIds visible to an observer at `observer_pos`: those in the 27-cell
    /// neighborhood of its cell (ADR-0019). Returned in `ShipId` order.
    ///
    /// Builds the grid from a single position pass; the serve loop can instead
    /// build one [`crate::aoi::CellGrid`] per tick and query it per session.
    pub fn ships_visible_to(&self, observer_pos: Position, cell_size: f32) -> Vec<ShipId> {
        crate::aoi::CellGrid::build(cell_size, self.ship_positions())
            .neighbors_of(observer_pos)
    }
}
