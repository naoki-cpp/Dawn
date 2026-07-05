//! JSON serialization helpers for `SimulationNode`.
//!
//! All methods that convert node state into the wire JSON the Godot client
//! expects live here, keeping the core simulation logic in `mod.rs` separate
//! from the presentation layer.

use dawn_core::{CelestialBodyKind, ItemId, ShipId};
use dawn_ecs::components::{FittingComp, HullComp, InventoryComp, ShipStatsComp};
use dawn_event_store::store::EventStore;

use super::SimulationNode;

/// The two JSON payloads sent to a client immediately after handshake
/// (before Welcome), regardless of whether the identity was freshly spawned
/// or resumed.
#[derive(Debug)]
pub struct HandoffPayload {
    pub initial_state: String,
    pub player_fitting: Option<String>,
}

/// Wire shape for an absolute (f64, ADR-0029) position: `{"x":...,"y":...,"z":...}`.
/// The one seam this file's three position-carrying messages (celestial body,
/// jump gate, ship) go through, instead of each authoring the same literal.
/// Kept local to `dawn-sector` rather than reusing `dawn-actor`'s `PosJson` --
/// `dawn-actor` sits one layer up in the crate DAG (CONTEXT.md Runtime
/// Boundaries) and `dawn-sector` must not depend on it.
fn abs_pos_json(p: [f64; 3]) -> serde_json::Value {
    serde_json::json!({ "x": p[0], "y": p[1], "z": p[2] })
}

impl<S: EventStore> SimulationNode<S> {
    /// Build the `InitialState` + `PlayerFitting` pair to hand a client once
    /// its identity (fresh or resumed) has already been decided by the caller.
    pub fn build_handoff_payload(&self, ship_id: ShipId, aoi_cell_size: f32) -> HandoffPayload {
        let initial_state = self
            .ship_absolute_pos(ship_id)
            .map(|pos| self.build_initial_state_json_for(pos, aoi_cell_size))
            .unwrap_or_else(|| self.build_initial_state_json());
        let player_fitting = self.build_player_fitting_json(ship_id);
        HandoffPayload {
            initial_state,
            player_fitting,
        }
    }

    /// Return the player ship's fitting state as a PlayerFitting JSON message.
    ///
    /// Sent after Welcome + InitialState on connect, and again after every
    /// Fit/Unfit (ADR-0032). Format:
    /// ```json
    /// {"type":"PlayerFitting","modules":[
    ///   {"slot":"High","index":0,"module_id":1,"name":"Small Railgun I","is_active":false}
    /// ],"inventory":[
    ///   {"module_id":2,"name":"Medium Railgun I","kind":"Weapon","slot":"High"}
    /// ],"slot_capacity":{"High":3,"Mid":3,"Low":2,"Rig":3}}
    /// ```
    pub fn build_player_fitting_json(&self, ship_id: ShipId) -> Option<String> {
        let entity = self.ships.index.get(&ship_id)?;
        let fitting = self.world.inner().get::<&FittingComp>(*entity).ok()?;

        let mut modules: Vec<serde_json::Value> = Vec::new();
        let slot_names = [
            ("High", &fitting.high),
            ("Mid", &fitting.mid),
            ("Low", &fitting.low),
            ("Rig", &fitting.rig),
        ];
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
                        "tackle_range_add"    : d.tackle_range_add,
                        "repair_range_add"    : d.repair_range_add,
                    },
                }));
            }
        }

        // Unfitted owned items (ADR-0034). For now the client only knows how
        // to render module inventory rows, so non-module items are skipped
        // until the Phase 9 UI lands.
        let inventory: Vec<serde_json::Value> = self
            .world
            .inner()
            .get::<&InventoryComp>(*entity)
            .ok()
            .map(|inv| {
                inv.items
                    .iter()
                    .filter_map(|(item_id, count)| match item_id {
                        ItemId::Module(module_id) => {
                            self.module_registry.get(module_id).map(|def| {
                                serde_json::json!({
                                    "module_id": def.id.0,
                                    "name"     : def.name,
                                    "kind"     : format!("{:?}", def.kind),
                                    "slot"     : format!("{:?}", def.slot),
                                    "count"    : count,
                                })
                            })
                        }
                        ItemId::PackagedShip(_) | ItemId::ScrapMetal => None,
                    })
                    .collect()
            })
            .unwrap_or_default();

        let layout = self
            .ships
            .type_ids
            .get(&ship_id)
            .and_then(|t| self.ship_type_registry.get(t))
            .map(|d| d.slot_layout);
        let slot_capacity = serde_json::json!({
            "High": layout.map(|l| l.high).unwrap_or(0),
            "Mid" : layout.map(|l| l.mid).unwrap_or(0),
            "Low" : layout.map(|l| l.low).unwrap_or(0),
            "Rig" : layout.map(|l| l.rig).unwrap_or(0),
        });

        Some(
            serde_json::json!({
                "type"         : "PlayerFitting",
                "modules"      : modules,
                "inventory"    : inventory,
                "slot_capacity": slot_capacity,
            })
            .to_string(),
        )
    }

    /// Full-world `InitialState` (every ship). Used for non-AoI callers.
    pub fn build_initial_state_json(&self) -> String {
        self.initial_state_json(self.ships.index.keys().copied())
    }

    /// `InitialState` scoped to an observer's Area of Interest: only ships in the
    /// 27-cell neighborhood of `observer_pos` (ADR-0019).
    pub fn build_initial_state_json_for(&self, observer_abs: [f64; 3], cell_size: f32) -> String {
        self.initial_state_json(self.ships_visible_to(observer_abs, cell_size).into_iter())
    }

    /// Serialise the given ships into an `InitialState` message.
    fn initial_state_json(&self, ship_ids: impl Iterator<Item = ShipId>) -> String {
        let ships: Vec<serde_json::Value> = ship_ids
            .filter_map(|ship_id| self.ship_state_json(ship_id))
            .collect();

        let bodies: Vec<serde_json::Value> = self
            .sector_map
            .bodies
            .values()
            .map(|b| {
                serde_json::json!({
                    "id"           : b.id.0,
                    "kind"         : match b.kind {
                        CelestialBodyKind::Star   => "Star",
                        CelestialBodyKind::Planet => "Planet",
                    },
                    "name"         : b.name,
                    "position"     : abs_pos_json(b.abs_m),
                    "radius"       : b.radius,
                    "spectral_type": b.spectral_type,
                })
            })
            .collect();

        // Navigation topology (ADR-0009/0025). The client renders gates/bodies
        // and resolves system names from this instead of holding a hard-coded
        // copy of the galaxy. Gates and bodies are already scoped to this Sector.
        let galaxy = &self.sector_map.galaxy;
        let system_name_of = |sector| {
            galaxy
                .system_for_sector_opt(sector)
                .and_then(|sys_id| galaxy.systems.iter().find(|s| s.id == sys_id))
                .map(|s| s.name.clone())
                .unwrap_or_else(|| "Unknown".to_string())
        };

        let systems: Vec<serde_json::Value> = galaxy
            .systems
            .iter()
            .map(|s| serde_json::json!({ "id": s.id.0, "name": s.name }))
            .collect();

        let gates: Vec<serde_json::Value> = self
            .sector_map
            .gates
            .values()
            .map(|g| {
                serde_json::json!({
                    "gate_id"          : g.id.0,
                    "position"         : abs_pos_json(g.abs_m),
                    "activation_radius": g.activation_radius,
                    "to_system_name"   : system_name_of(g.to_sector),
                })
            })
            .collect();

        serde_json::json!({
            "type"             : "InitialState",
            "ships"            : ships,
            "system_name"      : system_name_of(self.sector_id),
            "systems"          : systems,
            "jump_gates"       : gates,
            "celestial_bodies" : bodies,
        })
        .to_string()
    }

    /// Per-ship state object (position, stats, hull, ownership). Shared by
    /// `InitialState` and `AoiEnter` (ADR-0019). `None` if the ship is gone.
    pub fn ship_state_json(&self, ship_id: ShipId) -> Option<serde_json::Value> {
        let entity = self.ships.index.get(&ship_id)?;
        // Send the ABSOLUTE position (anchor + offset, f64), not the raw
        // anchor-relative offset (ADR-0029). After a warp rebase the offset is
        // body-relative, so a client that read it as absolute would misplace the
        // ship near the origin. The client renders absolute coords via its
        // floating origin.
        let pos = self.ship_absolute(ship_id)?;
        let stats = self.world.inner().get::<&ShipStatsComp>(*entity).ok()?;
        let hull = self.world.inner().get::<&HullComp>(*entity).ok()?;
        let is_player = self.ships.owners.contains_key(&ship_id);
        let ship_type_name = self
            .ships
            .type_ids
            .get(&ship_id)
            .and_then(|tid| self.ship_type_registry.get(tid))
            .map(|def| def.name.as_str())
            .unwrap_or("Unknown");
        Some(serde_json::json!({
            "ship_id"              : ship_id.raw(),
            "ship_type_name"       : ship_type_name,
            "position"             : abs_pos_json(pos),
            "max_shield"           : stats.max_shield,
            "max_armor"            : stats.max_armor,
            "max_hull"             : stats.max_hull,
            "current_shield"       : hull.shield(),
            "current_armor"        : hull.armor(),
            "current_hull"         : hull.hull(),
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

    /// `(ShipId, absolute position f64)` for every ship — the input to the AoI
    /// cell grid (ADR-0029 review #2 / R2). Each position composes the ship's
    /// anchor + offset in f64, so ships on different anchors are placed in the
    /// same Sector-frame grid *and* the binning stays precise at true-AU
    /// distances (an f32 absolute would have a ~16 km ulp). `CellGrid` sorts each
    /// bucket, so query results are deterministic.
    pub fn ship_absolute_positions(&self) -> Vec<(ShipId, [f64; 3])> {
        self.ships
            .index
            .iter()
            .map(|(&id, &entity)| (id, self.entity_abs_pos_f64(entity)))
            .collect()
    }

    /// Absolute (Sector-frame, f64) position of a ship by id, or `None` if
    /// unknown. The observer position to pass to AoI queries (ADR-0029 R2).
    pub fn ship_absolute_pos(&self, ship_id: ShipId) -> Option<[f64; 3]> {
        self.ship_absolute(ship_id)
    }

    /// ShipIds visible to an observer at `observer_abs` (an ABSOLUTE Sector-frame
    /// f64 position): those in the 27-cell neighborhood of its cell (ADR-0019).
    /// Returned in `ShipId` order. The grid is built from absolute f64 positions
    /// so it is correct across anchors and precise at true AU (ADR-0029 R2).
    pub fn ships_visible_to(&self, observer_abs: [f64; 3], cell_size: f32) -> Vec<ShipId> {
        crate::aoi::CellGrid::build(cell_size, self.ship_absolute_positions())
            .neighbors_of(observer_abs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{NodeId, Position, SectorBounds, SectorId, Velocity};

    fn mem_node() -> SimulationNode {
        SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        )
    }

    #[test]
    fn ships_visible_to_an_observer_are_only_those_in_the_27_cell_neighborhood() {
        let mut node = mem_node();
        let cell = 1_000.0;
        let observer = node.spawn_ship(
            crate::ship_types::SHIP_TYPE_NPC_FRIGATE,
            Position::ORIGIN,
            Velocity::ZERO,
        );
        let near = node.spawn_ship(
            crate::ship_types::SHIP_TYPE_NPC_FRIGATE,
            Position::new(1_500.0, 0.0, 0.0),
            Velocity::ZERO,
        );
        let far = node.spawn_ship(
            crate::ship_types::SHIP_TYPE_NPC_FRIGATE,
            Position::new(2_500.0, 0.0, 0.0),
            Velocity::ZERO,
        );

        let visible = node.ships_visible_to([0.0, 0.0, 0.0], cell);
        assert!(
            visible.contains(&observer),
            "observer's own cell is visible"
        );
        assert!(visible.contains(&near), "adjacent-cell ship is visible");
        assert!(
            !visible.contains(&far),
            "two-cells-away ship is not visible"
        );
    }

    #[test]
    fn aoi_is_computed_in_absolute_coords_across_anchors() {
        // ADR-0029 review #2: two ships at the same absolute point are mutually
        // visible even when anchored on different bodies. A star-anchored ship at
        // the origin and a Forge-anchored ship whose offset places it back at the
        // origin must land in the same AoI cell.
        use dawn_core::{events::AnchorRebased, AnchorId, DomainEvent, Tick};
        let mut node = mem_node();
        // Forcing b's offset to exactly cancel Forge's own (true-AU-scale)
        // absolute position is itself an unrealistic, maximally-imprecise case
        // (an offset is only meant to be small, ADR-0029 §2) -- real gameplay
        // never produces an offset this large. The cell is sized to absorb that
        // f32 ulp (a few km at this magnitude) rather than expecting the exact
        // 1-km binning a realistic small offset would get.
        let cell = 50_000.0;
        let a = node.spawn_ship(
            crate::ship_types::SHIP_TYPE_NPC_FRIGATE,
            Position::ORIGIN,
            Velocity::ZERO,
        );
        let b = node.spawn_ship(
            crate::ship_types::SHIP_TYPE_NPC_FRIGATE,
            Position::ORIGIN,
            Velocity::ZERO,
        );
        // Rebase b onto Forge with an offset that returns it to absolute origin.
        let forge = node.anchor_table().abs(AnchorId(1)).unwrap();
        let off = Position::new(-forge[0] as f32, -forge[1] as f32, -forge[2] as f32);
        node.apply_event_pub(DomainEvent::AnchorRebased(AnchorRebased {
            ship_id: b,
            anchor: AnchorId(1),
            offset: off,
            tick: Tick(1),
        }));
        // Sanity: raw offsets differ wildly, but absolute positions coincide.
        assert_eq!(node.get_ship_anchor(b), Some(AnchorId(1)));
        let visible = node.ships_visible_to([0.0, 0.0, 0.0], cell);
        assert!(
            visible.contains(&a) && visible.contains(&b),
            "both ships share the origin cell in absolute coords despite different anchors"
        );
    }

    #[test]
    fn scoped_initial_state_excludes_ships_outside_the_observer_neighborhood() {
        let mut node = mem_node();
        let cell = 1_000.0;
        let observer = node.spawn_ship(
            crate::ship_types::SHIP_TYPE_NPC_FRIGATE,
            Position::ORIGIN,
            Velocity::ZERO,
        );
        let far = node.spawn_ship(
            crate::ship_types::SHIP_TYPE_NPC_FRIGATE,
            Position::new(9_000.0, 0.0, 0.0),
            Velocity::ZERO,
        );

        let json = node.build_initial_state_json_for([0.0, 0.0, 0.0], cell);
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let ids: Vec<u64> = v["ships"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["ship_id"].as_u64().unwrap())
            .collect();
        assert!(
            ids.contains(&observer.raw()),
            "observer is in its own scoped state"
        );
        assert!(
            !ids.contains(&far.raw()),
            "distant ship is excluded from scoped InitialState"
        );
        let full: serde_json::Value =
            serde_json::from_str(&node.build_initial_state_json()).unwrap();
        assert_eq!(full["ships"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn initial_state_carries_the_sector_navigation_map() {
        // mem_node() serves Sector 0, which the demo galaxy maps to "Alpha".
        let node = mem_node();
        let v: serde_json::Value = serde_json::from_str(&node.build_initial_state_json()).unwrap();

        assert_eq!(v["system_name"], "Alpha");
        assert_eq!(
            v["systems"].as_array().unwrap().len(),
            3,
            "all star systems are listed"
        );

        let gates = v["jump_gates"].as_array().unwrap();
        assert_eq!(gates.len(), 1, "Sector 0 has exactly one gate");
        assert_eq!(gates[0]["gate_id"].as_u64().unwrap(), 0);
        assert_eq!(gates[0]["to_system_name"], "Beta", "gate 0 leads to Beta");
        let gate = node.jump_gate(dawn_core::JumpGateId(0)).unwrap();
        assert_eq!(
            gates[0]["position"]["x"].as_f64().unwrap(),
            gate.abs_m[0],
            "client gate marker/proximity source must match the f64 jump range source"
        );
        assert_eq!(
            gates[0]["position"]["z"].as_f64().unwrap(),
            gate.abs_m[2],
            "client gate marker/proximity source must match the f64 jump range source"
        );

        let bodies_json = v["celestial_bodies"].as_array().unwrap();
        assert_eq!(bodies_json.len(), 3, "Helios + Forge + Meridian");
        let first_body = node.sector_map.bodies.values().next().unwrap();
        let first_body_json = bodies_json
            .iter()
            .find(|b| b["id"].as_u64().unwrap() == first_body.id.0 as u64)
            .expect("every body in sector_map appears in the JSON");
        assert_eq!(
            first_body_json["position"]["x"].as_f64().unwrap(),
            first_body.abs_m[0],
            "client body marker source must match the f64 anchor source (abs_m), not the f32 position"
        );
        assert_eq!(
            first_body_json["position"]["z"].as_f64().unwrap(),
            first_body.abs_m[2],
            "client body marker source must match the f64 anchor source (abs_m), not the f32 position"
        );
    }

    #[test]
    fn aoi_enter_json_wraps_the_ship_state_for_a_known_ship() {
        let mut node = mem_node();
        let sid = node.spawn_ship(
            crate::ship_types::SHIP_TYPE_NPC_FRIGATE,
            Position::new(1.0, 2.0, 3.0),
            Velocity::ZERO,
        );
        let json = node
            .aoi_enter_json(sid)
            .expect("known ship yields a message");
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

    #[test]
    fn build_handoff_payload_scopes_initial_state_to_the_ship_and_carries_its_fitting() {
        let mut node = mem_node();
        let cell = 1_000.0;
        let ship_id = node.spawn_ship(
            crate::ship_types::SHIP_TYPE_NPC_FRIGATE,
            Position::ORIGIN,
            Velocity::ZERO,
        );
        let far = node.spawn_ship(
            crate::ship_types::SHIP_TYPE_NPC_FRIGATE,
            Position::new(9_000.0, 0.0, 0.0),
            Velocity::ZERO,
        );

        let payload = node.build_handoff_payload(ship_id, cell);

        let v: serde_json::Value = serde_json::from_str(&payload.initial_state).unwrap();
        let ids: Vec<u64> = v["ships"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["ship_id"].as_u64().unwrap())
            .collect();
        assert!(ids.contains(&ship_id.raw()), "ship sees its own state");
        assert!(
            !ids.contains(&far.raw()),
            "handoff scopes InitialState to the ship's AoI, not the whole sector"
        );
        assert!(
            payload.player_fitting.is_some(),
            "every ship with a FittingComp gets a PlayerFitting payload"
        );
    }
}
