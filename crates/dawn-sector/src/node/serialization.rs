//! JSON serialization helpers for `SimulationNode`.
//!
//! All methods that convert node state into the wire JSON the Godot client
//! expects live here, keeping the core simulation logic in `mod.rs` separate
//! from the presentation layer.

use dawn_core::{AbsolutePosition, ShipId};
use dawn_ecs::components::{HullComp, ShipStatsComp, VelocityComp};
use dawn_event_store::store::EventStore;
use dawn_wire::{
    AbsPosWire, BuildableShipTypeWire, CelestialBodyWire, InitialStateWire, JumpGateWire,
    PlayerLoadoutWire, ShipStateWire, StationWire, SystemWire,
};

use super::SimulationNode;

/// The two payloads sent to a client immediately after handshake (before
/// Welcome), regardless of whether the identity was freshly spawned or
/// resumed. Both are typed wire messages (stage 2a/2b, ADR-0042).
#[derive(Debug)]
pub struct HandoffPayload {
    pub initial_state: InitialStateWire,
    pub player_loadout: Option<PlayerLoadoutWire>,
}

/// The observer ship needed to scope an InitialState could not be resolved.
///
/// Network admission, resume, and post-transit handoff must reject this
/// condition instead of substituting an empty or full-world payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissingObserverShip {
    pub ship_id: ShipId,
}

impl std::fmt::Display for MissingObserverShip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "observer ship #{} could not be resolved",
            self.ship_id.raw()
        )
    }
}

impl std::error::Error for MissingObserverShip {}

/// Wire shape for an absolute (f64, ADR-0029) position. The one seam this
/// file's three position-carrying messages (celestial body, jump gate, ship)
/// go through, instead of each authoring the same literal. Kept local to
/// `dawn-sector` rather than reusing `dawn-actor`'s `PosWire` -- `dawn-actor`
/// sits one layer up in the crate DAG (CONTEXT.md Runtime Boundaries) and
/// `dawn-sector` must not depend on it.
fn abs_pos_json(p: AbsolutePosition) -> AbsPosWire {
    AbsPosWire {
        x: p[0],
        y: p[1],
        z: p[2],
    }
}

impl<S: EventStore> SimulationNode<S> {
    /// Build the observer-scoped `InitialState` + `PlayerLoadout` pair to
    /// hand a client once its identity (fresh or resumed) has been selected.
    pub fn build_handoff_payload(
        &self,
        ship_id: ShipId,
        aoi_cell_size: f64,
    ) -> Result<HandoffPayload, MissingObserverShip> {
        let initial_state = self.build_initial_state_for_observer(ship_id, aoi_cell_size)?;
        let player_loadout = self.build_player_loadout_json(ship_id);
        Ok(HandoffPayload {
            initial_state,
            player_loadout,
        })
    }

    /// Build an `InitialState` scoped to `observer_ship`'s 27-cell AoI.
    pub fn build_initial_state_for_observer(
        &self,
        observer_ship: ShipId,
        cell_size: f64,
    ) -> Result<InitialStateWire, MissingObserverShip> {
        let observer_abs = self
            .ship_absolute_pos(observer_ship)
            .ok_or(MissingObserverShip {
                ship_id: observer_ship,
            })?;
        Ok(self.build_initial_state_json_for(observer_abs, cell_size))
    }

    /// Full-world state for diagnostics and non-network tests. Admission,
    /// resume, and handoff paths must use the observer-scoped builders above.
    pub fn build_initial_state_json(&self) -> InitialStateWire {
        self.initial_state_json(self.ships.index.keys().copied())
    }

    /// `InitialState` scoped to an observer's Area of Interest: only ships in the
    /// 27-cell neighborhood of `observer_pos` (ADR-0019).
    pub fn build_initial_state_json_for(
        &self,
        observer_abs: dawn_core::AbsolutePosition,
        cell_size: f64,
    ) -> InitialStateWire {
        self.initial_state_json(self.ships_visible_to(observer_abs, cell_size).into_iter())
    }

    /// Build the given ships into an `InitialState` message.
    fn initial_state_json(&self, ship_ids: impl Iterator<Item = ShipId>) -> InitialStateWire {
        let ships: Vec<ShipStateWire> = ship_ids
            .filter_map(|ship_id| self.ship_state_json(ship_id))
            .collect();

        let celestial_bodies: Vec<CelestialBodyWire> = self
            .sector_map
            .bodies
            .values()
            .map(|b| CelestialBodyWire {
                id: b.id.0,
                kind: b.kind,
                name: b.name.clone(),
                position: abs_pos_json(b.abs_m),
                radius: b.radius,
                spectral_type: b.spectral_type,
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

        let systems: Vec<SystemWire> = galaxy
            .systems
            .iter()
            .map(|s| SystemWire {
                id: s.id.0,
                name: s.name.clone(),
            })
            .collect();

        let jump_gates: Vec<JumpGateWire> = self
            .sector_map
            .gates
            .values()
            .map(|g| JumpGateWire {
                gate_id: g.id.0,
                position: abs_pos_json(g.abs_m),
                activation_radius: g.activation_radius,
                to_system_name: system_name_of(g.to_sector),
            })
            .collect();

        let stations: Vec<StationWire> = self
            .sector_map
            .stations
            .values()
            .map(|station| StationWire {
                station_id: station.id.0,
                name: station.name.clone(),
                position: abs_pos_json(station.abs_m),
                docking_radius: station.docking_radius,
            })
            .collect();

        // Buildable Packaged Ship catalog (ADR-0034 9B): static registry data,
        // not per-tick, so it's cheapest to send once alongside the rest of
        // InitialState rather than as its own message type.
        let buildable_ship_types: Vec<BuildableShipTypeWire> = self
            .ship_type_registry
            .values()
            .filter(|def| def.buildable)
            .map(|def| BuildableShipTypeWire {
                ship_type_id: def.id.0,
                name: def.name.clone(),
            })
            .collect();

        InitialStateWire {
            ships,
            system_name: system_name_of(self.sector_id),
            systems,
            jump_gates,
            stations,
            celestial_bodies,
            buildable_ship_types,
        }
    }

    /// Per-ship state object (position, stats, hull, ownership). Shared by
    /// `InitialState` and `AoiEnter` (ADR-0019) -- `AoiEnter` wraps this
    /// directly (`ServerMessage::AoiEnter`), no separate wrapper needed.
    /// `None` if the ship is gone.
    pub fn ship_state_json(&self, ship_id: ShipId) -> Option<ShipStateWire> {
        let entity = self.ships.index.get(&ship_id)?;
        // Send the ABSOLUTE position (anchor + offset, f64), not the raw
        // anchor-relative offset (ADR-0029). After a warp rebase the offset is
        // body-relative, so a client that read it as absolute would misplace the
        // ship near the origin. The client renders absolute coords via its
        // floating origin.
        let pos = self.ship_absolute(ship_id)?;
        let stats = self.world.get::<ShipStatsComp>(*entity)?;
        let hull = self.world.get::<HullComp>(*entity)?;
        let is_player = self.ships.owners.contains_key(&ship_id);
        let ship_type_name = self
            .ships
            .type_ids
            .get(&ship_id)
            .and_then(|tid| self.ship_type_registry.get(tid))
            .map(|def| def.name.as_str())
            .unwrap_or("Unknown");
        Some(ShipStateWire {
            ship_id: ship_id.raw(),
            ship_type_name: ship_type_name.to_string(),
            position: abs_pos_json(pos),
            velocity: dawn_wire::VelWire::from(self.world.get::<VelocityComp>(*entity)?.0),
            max_speed: stats.max_speed,
            mass: stats.mass,
            inertia_modifier: stats.inertia_modifier,
            max_shield: stats.max_shield,
            max_armor: stats.max_armor,
            max_hull: stats.max_hull,
            current_shield: hull.shield(),
            current_armor: hull.armor(),
            current_hull: hull.hull(),
            cap_max: stats.cap_max,
            cap_recharge_per_tick: stats.cap_recharge_per_tick,
            is_player,
        })
    }

    // ── Area of Interest (ADR-0019) ────────────────────────────────────────────

    /// `(ShipId, absolute position f64)` for every ship — the input to the AoI
    /// cell grid (ADR-0029 review #2 / R2). Each position composes the ship's
    /// anchor + offset in f64, so ships on different anchors are placed in the
    /// same Sector-frame grid *and* the binning stays precise at true-AU
    /// distances (an f32 absolute would have a ~16 km ulp). `CellGrid` sorts each
    /// bucket, so query results are deterministic.
    pub fn ship_absolute_positions(&self) -> Vec<(ShipId, dawn_core::AbsolutePosition)> {
        self.ships
            .index
            .iter()
            .map(|(&id, &entity)| (id, self.entity_abs_pos_f64(entity)))
            .collect()
    }

    /// Absolute (Sector-frame, f64) position of a ship by id, or `None` if
    /// unknown. The observer position to pass to AoI queries (ADR-0029 R2).
    pub fn ship_absolute_pos(&self, ship_id: ShipId) -> Option<dawn_core::AbsolutePosition> {
        self.ship_absolute(ship_id)
    }

    /// ShipIds visible to an observer at `observer_abs` (an ABSOLUTE Sector-frame
    /// f64 position): those in the 27-cell neighborhood of its cell (ADR-0019).
    /// Returned in `ShipId` order. The grid is built from absolute f64 positions
    /// so it is correct across anchors and precise at true AU (ADR-0029 R2).
    pub fn ships_visible_to(
        &self,
        observer_abs: dawn_core::AbsolutePosition,
        cell_size: f64,
    ) -> Vec<ShipId> {
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
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
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

        let visible = node.ships_visible_to([0.0, 0.0, 0.0].into(), cell);
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
        let off = Position::new(-forge[0], -forge[1], -forge[2]);
        node.apply_event_pub(DomainEvent::AnchorRebased(AnchorRebased {
            ship_id: b,
            anchor: AnchorId(1),
            offset: off,
            tick: Tick(1),
        }));
        // Sanity: raw offsets differ wildly, but absolute positions coincide.
        assert_eq!(node.get_ship_anchor(b), Some(AnchorId(1)));
        let visible = node.ships_visible_to([0.0, 0.0, 0.0].into(), cell);
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

        let json = node.build_initial_state_json_for([0.0, 0.0, 0.0].into(), cell);
        let ids: Vec<u64> = json.ships.iter().map(|s| s.ship_id).collect();
        assert!(
            ids.contains(&observer.raw()),
            "observer is in its own scoped state"
        );
        assert!(
            !ids.contains(&far.raw()),
            "distant ship is excluded from scoped InitialState"
        );
        let full = node.build_initial_state_json();
        assert_eq!(full.ships.len(), 2);
    }

    #[test]
    fn initial_state_carries_the_sector_navigation_map() {
        // mem_node() serves Sector 0, which the demo galaxy maps to "Alpha".
        let node = mem_node();
        let v = node.build_initial_state_json();

        assert_eq!(v.system_name, "Alpha");
        assert_eq!(v.systems.len(), 3, "all star systems are listed");

        let gates = &v.jump_gates;
        assert_eq!(gates.len(), 1, "Sector 0 has exactly one gate");
        assert_eq!(gates[0].gate_id, 0);
        assert_eq!(gates[0].to_system_name, "Beta", "gate 0 leads to Beta");
        let gate = node.jump_gate(dawn_core::JumpGateId(0)).unwrap();
        assert_eq!(
            gates[0].position.x, gate.abs_m[0],
            "client gate marker/proximity source must match the f64 jump range source"
        );
        assert_eq!(
            gates[0].position.z, gate.abs_m[2],
            "client gate marker/proximity source must match the f64 jump range source"
        );

        let bodies_json = &v.celestial_bodies;
        assert_eq!(bodies_json.len(), 3, "Helios + Forge + Meridian");
        let first_body = node.sector_map.bodies.values().next().unwrap();
        let first_body_json = bodies_json
            .iter()
            .find(|b| b.id == first_body.id.0)
            .expect("every body in sector_map appears in the JSON");
        assert_eq!(
            first_body_json.position.x, first_body.abs_m[0],
            "client body marker source must match the f64 anchor source (abs_m), not the f32 position"
        );
        assert_eq!(
            first_body_json.position.z, first_body.abs_m[2],
            "client body marker source must match the f64 anchor source (abs_m), not the f32 position"
        );

        let stations = &v.stations;
        assert_eq!(stations.len(), 1, "Sector 0 has exactly one NPC station");
        assert_eq!(stations[0].station_id, 0);
        assert_eq!(stations[0].name, "Forge Station");
    }

    #[test]
    fn initial_state_lists_only_buildable_ship_types() {
        let node = mem_node();
        let v = node.build_initial_state_json();

        let buildable = &v.buildable_ship_types;
        assert_eq!(
            buildable.len(),
            1,
            "only the Magpie is buildable, the NPC Frigate must not appear"
        );
        assert_eq!(
            buildable[0].ship_type_id,
            crate::ship_types::SHIP_TYPE_MAGPIE.0
        );
        assert_eq!(buildable[0].name, "Magpie");
    }

    #[test]
    fn ship_state_json_reports_the_ships_own_id_and_absolute_position() {
        // ship_state_json is what ServerMessage::AoiEnter (ADR-0042 stage 2c)
        // wraps directly, so its own contract is tested here rather than
        // through a removed AoiEnter-specific wrapper.
        let mut node = mem_node();
        let sid = node.spawn_ship(
            crate::ship_types::SHIP_TYPE_NPC_FRIGATE,
            Position::new(1.0, 2.0, 3.0),
            Velocity::ZERO,
        );
        let ship = node
            .ship_state_json(sid)
            .expect("known ship yields a state");
        assert_eq!(ship.ship_id, sid.raw());
        assert_eq!(ship.position.x, 1.0);
        assert_eq!(ship.velocity.dx, 0.0);
        assert_eq!(ship.velocity.dy, 0.0);
        assert_eq!(ship.velocity.dz, 0.0);
        assert!(ship.max_speed > 0.0);
        assert!(ship.mass > 0.0);
        assert!(ship.inertia_modifier > 0.0);
    }

    #[test]
    fn ship_state_json_is_none_for_an_unknown_ship() {
        let node = mem_node();
        let unknown = ShipId::new(NodeId(9), 999);
        assert!(node.ship_state_json(unknown).is_none());
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

        let payload = node
            .build_handoff_payload(ship_id, cell)
            .expect("known observer ship");

        let ids: Vec<u64> = payload
            .initial_state
            .ships
            .iter()
            .map(|s| s.ship_id)
            .collect();
        assert!(ids.contains(&ship_id.raw()), "ship sees its own state");
        assert!(
            !ids.contains(&far.raw()),
            "handoff scopes InitialState to the ship's AoI, not the whole sector"
        );
        assert!(
            payload.player_loadout.is_some(),
            "every ship with a FittingComp gets a PlayerLoadout payload"
        );
    }

    #[test]
    fn handoff_payload_rejects_an_unresolved_observer() {
        let node = mem_node();
        let missing = ShipId::new(NodeId(9), 999);

        let error = node
            .build_handoff_payload(missing, 1_000.0)
            .expect_err("missing observer must not receive full-world state");

        assert_eq!(error, MissingObserverShip { ship_id: missing });
    }
}
