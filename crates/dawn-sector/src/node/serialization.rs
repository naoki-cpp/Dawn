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
        let entity = self.ships.index.get(&ship_id)?;
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
        self.initial_state_json(self.ships.index.keys().copied())
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

        let bodies: Vec<serde_json::Value> = self.sector_map.bodies.values().map(|b| {
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

        // Navigation topology (ADR-0009/0025). The client renders gates/bodies
        // and resolves system names from this instead of holding a hard-coded
        // copy of the galaxy. Gates and bodies are already scoped to this Sector.
        let galaxy = &self.sector_map.galaxy;
        let system_name_of = |sector| galaxy
            .system_for_sector_opt(sector)
            .and_then(|sys_id| galaxy.systems.iter().find(|s| s.id == sys_id))
            .map(|s| s.name.clone())
            .unwrap_or_else(|| "Unknown".to_string());

        let systems: Vec<serde_json::Value> = galaxy.systems.iter()
            .map(|s| serde_json::json!({ "id": s.id.0, "name": s.name }))
            .collect();

        let gates: Vec<serde_json::Value> = self.sector_map.gates.values().map(|g| {
            serde_json::json!({
                "gate_id"          : g.id.0,
                "position"         : { "x": g.position.x, "y": g.position.y, "z": g.position.z },
                "activation_radius": g.activation_radius,
                "to_system_name"   : system_name_of(g.to_sector),
            })
        }).collect();

        serde_json::json!({
            "type"             : "InitialState",
            "ships"            : ships,
            "system_name"      : system_name_of(self.sector_id),
            "systems"          : systems,
            "jump_gates"       : gates,
            "celestial_bodies" : bodies,
        }).to_string()
    }

    /// Per-ship state object (position, stats, hull, ownership). Shared by
    /// `InitialState` and `AoiEnter` (ADR-0019). `None` if the ship is gone.
    pub fn ship_state_json(&self, ship_id: ShipId) -> Option<serde_json::Value> {
        let entity  = self.ships.index.get(&ship_id)?;
        // Send the ABSOLUTE position (anchor + offset, f64), not the raw
        // anchor-relative offset (ADR-0029). After a warp rebase the offset is
        // body-relative, so a client that read it as absolute would misplace the
        // ship near the origin. The client renders absolute coords via its
        // floating origin.
        let pos     = self.ship_absolute(ship_id)?;
        let stats   = self.world.inner().get::<&ShipStatsComp>(*entity).ok()?;
        let hull    = self.world.inner().get::<&HullComp>(*entity).ok()?;
        let is_player = self.ships.owners.contains_key(&ship_id);
        let ship_type_name = self.ships.type_ids.get(&ship_id)
            .and_then(|tid| self.ship_type_registry.get(tid))
            .map(|def| def.name.as_str())
            .unwrap_or("Unknown");
        Some(serde_json::json!({
            "ship_id"              : ship_id.raw(),
            "ship_type_name"       : ship_type_name,
            "position"             : { "x": pos[0], "y": pos[1], "z": pos[2] },
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

    /// `(ShipId, Position)` for every ship currently in the world, as raw
    /// anchor-relative offsets. Internal use only — for AoI / cross-ship geometry
    /// use [`Self::ship_absolute_positions`] (offsets are not comparable across
    /// different anchors, ADR-0029).
    pub fn ship_positions(&self) -> Vec<(ShipId, Position)> {
        self.ships.index.iter().filter_map(|(&id, &entity)| {
            let pos = self.world.inner().get::<&PositionComp>(entity).ok()?.0;
            Some((id, pos))
        }).collect()
    }

    /// `(ShipId, absolute Position)` for every ship — the input to the AoI cell
    /// grid (ADR-0029 review #2). Each position composes the ship's anchor + its
    /// offset, so ships on different anchors are placed in the same Sector-frame
    /// grid. `CellGrid` sorts each bucket, so query results are deterministic.
    pub fn ship_absolute_positions(&self) -> Vec<(ShipId, Position)> {
        self.ships.index.iter().map(|(&id, &entity)| (id, self.entity_abs_pos(entity))).collect()
    }

    /// Absolute (Sector-frame) position of a ship by id, or `None` if unknown.
    /// The observer position to pass to AoI queries (ADR-0029 review #2).
    pub fn ship_absolute_pos(&self, ship_id: ShipId) -> Option<Position> {
        let &entity = self.ships.index.get(&ship_id)?;
        Some(self.entity_abs_pos(entity))
    }

    /// ShipIds visible to an observer at `observer_abs` (an ABSOLUTE Sector-frame
    /// position): those in the 27-cell neighborhood of its cell (ADR-0019).
    /// Returned in `ShipId` order. The grid is built from absolute positions so
    /// it is correct across anchors (ADR-0029 review #2).
    pub fn ships_visible_to(&self, observer_abs: Position, cell_size: f32) -> Vec<ShipId> {
        crate::aoi::CellGrid::build(cell_size, self.ship_absolute_positions())
            .neighbors_of(observer_abs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{NodeId, SectorBounds, SectorId, Velocity};

    fn mem_node() -> SimulationNode {
        SimulationNode::new(NodeId(0), SectorId(0), SectorBounds::centered(SectorBounds::DEFAULT_HALF))
    }

    #[test]
    fn ships_visible_to_an_observer_are_only_those_in_the_27_cell_neighborhood() {
        let mut node = mem_node();
        let cell = 1_000.0;
        let observer = node.spawn_ship(crate::ship_types::SHIP_TYPE_NPC_FRIGATE, Position::ORIGIN, Velocity::ZERO);
        let near = node.spawn_ship(crate::ship_types::SHIP_TYPE_NPC_FRIGATE, Position::new(1_500.0, 0.0, 0.0), Velocity::ZERO);
        let far  = node.spawn_ship(crate::ship_types::SHIP_TYPE_NPC_FRIGATE, Position::new(2_500.0, 0.0, 0.0), Velocity::ZERO);

        let visible = node.ships_visible_to(Position::ORIGIN, cell);
        assert!(visible.contains(&observer), "observer's own cell is visible");
        assert!(visible.contains(&near),     "adjacent-cell ship is visible");
        assert!(!visible.contains(&far),     "two-cells-away ship is not visible");
    }

    #[test]
    fn aoi_is_computed_in_absolute_coords_across_anchors() {
        // ADR-0029 review #2: two ships at the same absolute point are mutually
        // visible even when anchored on different bodies. A star-anchored ship at
        // the origin and a Forge-anchored ship whose offset places it back at the
        // origin must land in the same AoI cell.
        use dawn_core::{AnchorId, DomainEvent, events::AnchorRebased, Tick};
        let mut node = mem_node();
        let cell = 1_000.0;
        let a = node.spawn_ship(crate::ship_types::SHIP_TYPE_NPC_FRIGATE, Position::ORIGIN, Velocity::ZERO);
        let b = node.spawn_ship(crate::ship_types::SHIP_TYPE_NPC_FRIGATE, Position::ORIGIN, Velocity::ZERO);
        // Rebase b onto Forge with an offset that returns it to absolute origin.
        let forge = node.anchor_table().abs(AnchorId(1)).unwrap();
        let off = Position::new(-forge[0] as f32, -forge[1] as f32, -forge[2] as f32);
        node.apply_event_pub(DomainEvent::AnchorRebased(AnchorRebased { ship_id: b, anchor: AnchorId(1), offset: off, tick: Tick(1) }));
        // Sanity: raw offsets differ wildly, but absolute positions coincide.
        assert_eq!(node.get_ship_anchor(b), Some(AnchorId(1)));
        let visible = node.ships_visible_to(Position::ORIGIN, cell);
        assert!(visible.contains(&a) && visible.contains(&b),
            "both ships share the origin cell in absolute coords despite different anchors");
    }

    #[test]
    fn scoped_initial_state_excludes_ships_outside_the_observer_neighborhood() {
        let mut node = mem_node();
        let cell = 1_000.0;
        let observer = node.spawn_ship(crate::ship_types::SHIP_TYPE_NPC_FRIGATE, Position::ORIGIN, Velocity::ZERO);
        let far      = node.spawn_ship(crate::ship_types::SHIP_TYPE_NPC_FRIGATE, Position::new(9_000.0, 0.0, 0.0), Velocity::ZERO);

        let json = node.build_initial_state_json_for(Position::ORIGIN, cell);
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let ids: Vec<u64> = v["ships"].as_array().unwrap().iter()
            .map(|s| s["ship_id"].as_u64().unwrap())
            .collect();
        assert!(ids.contains(&observer.raw()), "observer is in its own scoped state");
        assert!(!ids.contains(&far.raw()),     "distant ship is excluded from scoped InitialState");
        let full: serde_json::Value = serde_json::from_str(&node.build_initial_state_json()).unwrap();
        assert_eq!(full["ships"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn initial_state_carries_the_sector_navigation_map() {
        // mem_node() serves Sector 0, which the demo galaxy maps to "Alpha".
        let node = mem_node();
        let v: serde_json::Value =
            serde_json::from_str(&node.build_initial_state_json()).unwrap();

        assert_eq!(v["system_name"], "Alpha");
        assert_eq!(v["systems"].as_array().unwrap().len(), 3, "all star systems are listed");

        let gates = v["jump_gates"].as_array().unwrap();
        assert_eq!(gates.len(), 1, "Sector 0 has exactly one gate");
        assert_eq!(gates[0]["gate_id"].as_u64().unwrap(), 0);
        assert_eq!(gates[0]["to_system_name"], "Beta", "gate 0 leads to Beta");

        assert_eq!(v["celestial_bodies"].as_array().unwrap().len(), 3, "Helios + Forge + Meridian");
    }

    #[test]
    fn aoi_enter_json_wraps_the_ship_state_for_a_known_ship() {
        let mut node = mem_node();
        let sid = node.spawn_ship(crate::ship_types::SHIP_TYPE_NPC_FRIGATE, Position::new(1.0, 2.0, 3.0), Velocity::ZERO);
        let json = node.aoi_enter_json(sid).expect("known ship yields a message");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "AoiEnter");
        assert_eq!(v["ship"]["ship_id"].as_u64().unwrap(), sid.raw());
        assert_eq!(v["ship"]["position"]["x"].as_f64().unwrap() as f32, 1.0);
    }

    #[test]
    fn aoi_enter_json_is_none_for_an_unknown_ship() {
        let node = mem_node();
        let unknown = ShipId::new(NodeId(9), 999);
        assert!(node.aoi_enter_json(unknown).is_none());
    }
}
