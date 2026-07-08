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
    pub player_loadout: Option<String>,
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
    /// The one seam every ItemRow wire row (ship cargo, station inventory)
    /// goes through. The client's `ItemRow.from_json()` requires all of
    /// `item_type`/`module_id`/`ship_type_id`/`name`/`kind`/`slot`/`count` on
    /// every row and silently drops (push_error + null) any row missing one
    /// -- `0`/`""` fill the fields a given `ItemId` variant doesn't use, so
    /// the invariant is enforced here instead of copy-pasted at each call
    /// site. `None` if the registry backing `item_id` no longer has a
    /// definition for it (stale/renamed module or ship type).
    fn item_id_to_row_json(&self, item_id: ItemId, count: u64) -> Option<serde_json::Value> {
        match item_id {
            ItemId::Module(module_id) => self.module_registry.get(&module_id).map(|def| {
                serde_json::json!({
                    "item_type": "Module",
                    "module_id": def.id.0,
                    "ship_type_id": 0,
                    "name"     : def.name,
                    "kind"     : format!("{:?}", def.kind),
                    "slot"     : format!("{:?}", def.slot),
                    "count"    : count,
                })
            }),
            ItemId::PackagedShip(ship_type_id) => {
                self.ship_type_registry.get(&ship_type_id).map(|def| {
                    serde_json::json!({
                        "item_type": "PackagedShip",
                        "module_id": 0,
                        "ship_type_id": def.id.0,
                        "name"        : def.name,
                        "kind"        : "",
                        "slot"        : "",
                        "count"       : count,
                    })
                })
            }
            ItemId::ScrapMetal => Some(serde_json::json!({
                "item_type": "ScrapMetal",
                "module_id": 0,
                "ship_type_id": 0,
                "name"     : "Scrap Metal",
                "kind"     : "",
                "slot"     : "",
                "count"    : count,
            })),
        }
    }

    /// Build the `InitialState` + `PlayerLoadout` pair to hand a client once
    /// its identity (fresh or resumed) has already been decided by the caller.
    pub fn build_handoff_payload(&self, ship_id: ShipId, aoi_cell_size: f32) -> HandoffPayload {
        let initial_state = self
            .ship_absolute_pos(ship_id)
            .map(|pos| self.build_initial_state_json_for(pos, aoi_cell_size))
            .unwrap_or_else(|| self.build_initial_state_json());
        let player_loadout = self.build_player_loadout_json(ship_id);
        HandoffPayload {
            initial_state,
            player_loadout,
        }
    }

    /// Return the player ship's loadout state as a PlayerLoadout JSON message.
    ///
    /// Sent after Welcome + InitialState on connect, and again after every
    /// Fit/Unfit (ADR-0032). Format:
    /// ```json
    /// {"type":"PlayerLoadout","modules":[
    ///   {"slot":"High","index":0,"module_id":1,"name":"Small Railgun I","is_active":false}
    /// ],"inventory":[
    ///   {"module_id":2,"name":"Medium Railgun I","kind":"Weapon","slot":"High"}
    /// ],"slot_capacity":{"High":3,"Mid":3,"Low":2,"Rig":3},"active_ship_id":1}
    /// ```
    ///
    /// `active_ship_id` is `null` when the caller has no active ship at all
    /// (ADR-0037 -- Disembark, or before a freshly-Assembled/Disassembled
    /// state settles); the client uses it to detect that its active ship
    /// changed independently of any command it sent.
    pub fn build_player_loadout_json(&self, ship_id: ShipId) -> Option<String> {
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

        // Unfitted owned items (ADR-0034). The client inventory panel can now
        // show both fittable modules and passive item stacks such as Scrap
        // Metal, so keep the wire rows generic enough for both.
        let inventory: Vec<serde_json::Value> = self
            .world
            .inner()
            .get::<&InventoryComp>(*entity)
            .ok()
            .map(|inv| {
                inv.items
                    .iter()
                    .filter_map(|(&item_id, &count)| self.item_id_to_row_json(item_id, count))
                    .collect()
            })
            .unwrap_or_default();

        let player_id = self.ships.owners.get(&ship_id).copied();
        let docked_station_id = player_id.and_then(|pid| self.player_docked_station(pid));
        let docked_station_name = docked_station_id
            .and_then(|station_id| self.station(station_id))
            .map(|station| station.name.clone());
        let station_inventory: Vec<serde_json::Value> = player_id
            .map(|pid| self.station_inventory_json(pid))
            .unwrap_or_default();
        // Derived from the player's true active_ship, not echoed back from the
        // `ship_id` argument -- callers may pass a session's original ship_id
        // (e.g. periodic resends), which can be stale once active_ship changes
        // independently (Disembark/SelectActiveShip/Assemble, ADR-0037). The
        // client needs this to know whether it still has a ship to fly at all.
        let active_ship_id = player_id.and_then(|pid| self.ships.active_ship.get(&pid).copied());

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

        let owned_ships = player_id
            .map(|pid| self.owned_ships_json(pid))
            .unwrap_or_default();

        Some(
            serde_json::json!({
                "type"         : "PlayerLoadout",
                "tick"         : self.current_tick.value(),
                "modules"      : modules,
                "inventory"    : inventory,
                "station_inventory": station_inventory,
                "docked_station_id": docked_station_id.map(|id| id.0),
                "docked_station_name": docked_station_name,
                "slot_capacity": slot_capacity,
                "active_ship_id": active_ship_id.map(|id| id.raw()),
                "owned_ships": owned_ships,
            })
            .to_string(),
        )
    }

    /// Same wire message as [`Self::build_player_loadout_json`], keyed by
    /// `player_id` instead of a specific ship. Delegates to the ship-keyed
    /// builder when the player has an active ship; otherwise (no active ship
    /// -- e.g. right after Disassemble removed the caller's only ship, or
    /// before a freshly-Assembled ship is made active, `docs/architecture/ownership.md`
    /// §8) still reports the player's docked station and station inventory,
    /// with empty fitting/cargo/slot_capacity sections since there's no ship
    /// to report those for.
    pub fn build_player_loadout_json_for_player(
        &self,
        player_id: dawn_core::PlayerId,
    ) -> Option<String> {
        if let Some(ship_id) = self.ships.active_ship.get(&player_id).copied() {
            return self.build_player_loadout_json(ship_id);
        }
        let docked_station_id = self.player_docked_station(player_id);
        let docked_station_name = docked_station_id
            .and_then(|station_id| self.station(station_id))
            .map(|station| station.name.clone());
        let station_inventory = self.station_inventory_json(player_id);
        let owned_ships = self.owned_ships_json(player_id);
        Some(
            serde_json::json!({
                "type"         : "PlayerLoadout",
                "tick"         : self.current_tick.value(),
                "modules"      : Vec::<serde_json::Value>::new(),
                "inventory"    : Vec::<serde_json::Value>::new(),
                "station_inventory": station_inventory,
                "docked_station_id": docked_station_id.map(|id| id.0),
                "docked_station_name": docked_station_name,
                "slot_capacity": serde_json::json!({
                    "High": 0, "Mid": 0, "Low": 0, "Rig": 0,
                }),
                "active_ship_id": Option::<u64>::None,
                "owned_ships": owned_ships,
            })
            .to_string(),
        )
    }

    /// Every ship `player_id` owns (`docs/architecture/ownership.md` §7),
    /// regardless of whether it's the active one -- the client needs the
    /// full roster to offer a "select active ship" UI. Each row:
    /// `{ship_id, ship_type_id, ship_type_name, docked_station_id, is_active}`.
    /// `docked_station_id` is `null` for a ship not currently docked anywhere
    /// this node knows about (e.g. it flew off, or is in another Sector).
    fn owned_ships_json(&self, player_id: dawn_core::PlayerId) -> Vec<serde_json::Value> {
        let active_ship_id = self.ships.active_ship.get(&player_id).copied();
        self.ships
            .owners
            .iter()
            .filter(|(_, owner)| **owner == player_id)
            .map(|(&ship_id, _)| {
                let ship_type_id = self.ships.type_ids.get(&ship_id).copied();
                let ship_type_name = ship_type_id
                    .and_then(|t| self.ship_type_registry.get(&t))
                    .map(|def| def.name.clone());
                serde_json::json!({
                    "ship_id": ship_id.raw(),
                    "ship_type_id": ship_type_id.map(|t| t.0),
                    "ship_type_name": ship_type_name,
                    "docked_station_id": self.docked_station(ship_id).map(|id| id.0),
                    "is_active": Some(ship_id) == active_ship_id,
                })
            })
            .collect()
    }

    /// Station inventory as wire rows for `player_id`, empty if the player
    /// isn't currently docked anywhere. Shared by `build_player_loadout_json`
    /// (ship-keyed) and `build_player_loadout_json_for_player` (player-keyed).
    fn station_inventory_json(&self, player_id: dawn_core::PlayerId) -> Vec<serde_json::Value> {
        if self.player_docked_station(player_id).is_none() {
            return Vec::new();
        }
        self.station_inventory(player_id)
            .map(|inventory| {
                inventory
                    .iter()
                    .filter_map(|(&item_id, &count)| self.item_id_to_row_json(item_id, count))
                    .collect()
            })
            .unwrap_or_default()
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

        let stations: Vec<serde_json::Value> = self
            .sector_map
            .stations
            .values()
            .map(|station| {
                serde_json::json!({
                    "station_id": station.id.0,
                    "name": station.name,
                    "position": abs_pos_json(station.abs_m),
                    "docking_radius": station.docking_radius,
                })
            })
            .collect();

        // Buildable Packaged Ship catalog (ADR-0034 9B): static registry data,
        // not per-tick, so it's cheapest to send once alongside the rest of
        // InitialState rather than as its own message type.
        let buildable_ship_types: Vec<serde_json::Value> = self
            .ship_type_registry
            .values()
            .filter(|def| def.buildable)
            .map(|def| serde_json::json!({ "ship_type_id": def.id.0, "name": def.name }))
            .collect();

        serde_json::json!({
            "type"                 : "InitialState",
            "ships"                : ships,
            "system_name"          : system_name_of(self.sector_id),
            "systems"              : systems,
            "jump_gates"           : gates,
            "stations"             : stations,
            "celestial_bodies"     : bodies,
            "buildable_ship_types" : buildable_ship_types,
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
    use crate::node::station::StationOperationOutcome;
    use dawn_core::{AssembleCommand, NodeId, Position, SectorBounds, SectorId, Velocity};

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

        let stations = v["stations"].as_array().unwrap();
        assert_eq!(stations.len(), 1, "Sector 0 has exactly one NPC station");
        assert_eq!(stations[0]["station_id"].as_u64().unwrap(), 0);
        assert_eq!(stations[0]["name"], "Forge Station");
    }

    #[test]
    fn initial_state_lists_only_buildable_ship_types() {
        let mut node = mem_node();
        for def in crate::ship_types::all_ship_types() {
            node.register_ship_type(def);
        }
        let v: serde_json::Value = serde_json::from_str(&node.build_initial_state_json()).unwrap();

        let buildable = v["buildable_ship_types"].as_array().unwrap();
        assert_eq!(
            buildable.len(),
            1,
            "only the Magpie is buildable, the NPC Frigate must not appear"
        );
        assert_eq!(
            buildable[0]["ship_type_id"].as_u64().unwrap(),
            crate::ship_types::SHIP_TYPE_MAGPIE.0 as u64
        );
        assert_eq!(buildable[0]["name"], "Magpie");
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
            payload.player_loadout.is_some(),
            "every ship with a FittingComp gets a PlayerLoadout payload"
        );
    }

    #[test]
    fn player_loadout_json_includes_scrap_metal_inventory_rows() {
        use dawn_core::ItemId;

        let mut node = mem_node();
        for def in crate::modules::all_modules() {
            node.register_module(def);
        }
        for def in crate::ship_types::all_ship_types() {
            node.register_ship_type(def);
        }

        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);
        let entity = *node.ships.index.get(&ship_id).unwrap();
        node.world
            .inner_mut()
            .get::<&mut InventoryComp>(entity)
            .unwrap()
            .add_item(ItemId::ScrapMetal, 3);

        let json = node.build_player_loadout_json(ship_id).unwrap();
        let payload: serde_json::Value = serde_json::from_str(&json).unwrap();
        let inventory = payload["inventory"].as_array().unwrap();
        let scrap = inventory
            .iter()
            .find(|row| row["item_type"] == "ScrapMetal")
            .expect("scrap rows must be serialized to the client");
        assert_eq!(scrap["name"], "Scrap Metal");
        assert_eq!(scrap["count"].as_u64().unwrap(), 3);
    }

    #[test]
    fn every_item_row_carries_all_the_keys_item_row_gd_requires() {
        // Regression: item_row.gd's ItemRow.from_json() requires item_type/
        // module_id/ship_type_id/name/kind/slot/count on every row, and drops
        // (push_error + null) any row missing even one -- silently, with no
        // client-visible symptom beyond "the row just isn't there". The
        // PackagedShip/ScrapMetal branches (both ship inventory and station
        // inventory) and the unfitted-Module branch used to omit some of
        // these, so a station's starter Packaged Ship (or any Scrap Metal,
        // or any unfitted Module) never reached the client's inventory panel.
        use dawn_core::{DockCommand, ItemId, StationId};

        const REQUIRED_KEYS: &[&str] = &[
            "item_type",
            "module_id",
            "ship_type_id",
            "name",
            "kind",
            "slot",
            "count",
        ];

        let mut node = mem_node();
        for def in crate::modules::all_modules() {
            node.register_module(def);
        }
        for def in crate::ship_types::all_ship_types() {
            node.register_ship_type(def);
        }

        let player_id = node.next_player_id();
        let station = node.station(StationId(0)).unwrap().clone();
        let ship_id = node.spawn_player_ship_at_pub(player_id, station.position);
        assert!(matches!(
            node.dock_owned(
                player_id,
                ship_id,
                DockCommand {
                    station_id: StationId(0),
                }
            ),
            StationOperationOutcome::Accepted { .. }
        ));

        let entity = *node.ships.index.get(&ship_id).unwrap();
        node.world
            .inner_mut()
            .get::<&mut InventoryComp>(entity)
            .unwrap()
            .add_item(ItemId::ScrapMetal, 1);
        // Unfit one already-fitted module so an unfitted Module row exists.
        node.unfit_module_owned(
            player_id,
            dawn_core::UnfitModuleCommand {
                ship_id,
                slot: dawn_core::SlotKind::High,
                module_id: crate::modules::MODULE_RAILGUN_SMALL,
            },
        );
        // Starter Packaged Ship (spawner_logic.rs) already sits in station
        // inventory from spawn_player_ship_at_pub above.

        let json = node.build_player_loadout_json(ship_id).unwrap();
        let payload: serde_json::Value = serde_json::from_str(&json).unwrap();

        let mut rows: Vec<&serde_json::Value> =
            payload["inventory"].as_array().unwrap().iter().collect();
        rows.extend(payload["station_inventory"].as_array().unwrap().iter());
        assert!(
            rows.iter().any(|r| r["item_type"] == "Module"),
            "expected an unfitted Module row: {rows:?}"
        );
        assert!(
            rows.iter().any(|r| r["item_type"] == "ScrapMetal"),
            "expected a ScrapMetal row: {rows:?}"
        );
        assert!(
            rows.iter().any(|r| r["item_type"] == "PackagedShip"),
            "expected the starter PackagedShip row: {rows:?}"
        );
        for row in &rows {
            for key in REQUIRED_KEYS {
                assert!(
                    row.get(key).is_some(),
                    "row {row:?} is missing required key '{key}' (ItemRow.from_json would drop it)"
                );
            }
        }
    }

    #[test]
    fn item_id_to_row_json_fills_every_required_key_for_every_item_id_variant() {
        // Direct unit test on the seam itself (item_id_to_row_json), rather
        // than only through the end-to-end fixture above -- a future ItemId
        // variant that forgets a key fails here immediately, without needing
        // to route through a docked ship/station setup.
        const REQUIRED_KEYS: &[&str] = &[
            "item_type",
            "module_id",
            "ship_type_id",
            "name",
            "kind",
            "slot",
            "count",
        ];

        let mut node = mem_node();
        for def in crate::modules::all_modules() {
            node.register_module(def);
        }
        for def in crate::ship_types::all_ship_types() {
            node.register_ship_type(def);
        }

        let module_id = crate::modules::MODULE_RAILGUN_SMALL;
        let ship_type_id = crate::ship_types::SHIP_TYPE_MAGPIE;
        for item_id in [
            ItemId::Module(module_id),
            ItemId::PackagedShip(ship_type_id),
            ItemId::ScrapMetal,
        ] {
            let row = node
                .item_id_to_row_json(item_id, 3)
                .unwrap_or_else(|| panic!("expected a row for {item_id:?}"));
            for key in REQUIRED_KEYS {
                assert!(
                    row.get(key).is_some(),
                    "{item_id:?} row {row:?} is missing required key '{key}'"
                );
            }
            assert_eq!(row["count"].as_u64().unwrap(), 3);
        }
    }

    #[test]
    fn item_id_to_row_json_returns_none_for_an_item_id_with_no_registry_definition() {
        let node = mem_node(); // no modules/ship types registered
        assert!(node
            .item_id_to_row_json(ItemId::Module(dawn_core::ModuleId(999)), 1)
            .is_none());
        assert!(node
            .item_id_to_row_json(ItemId::PackagedShip(dawn_core::ShipTypeId(999)), 1)
            .is_none());
    }

    #[test]
    fn player_loadout_json_carries_docked_station_context_and_station_inventory() {
        use dawn_core::{DockCommand, ItemId, StationId};

        let mut node = mem_node();
        for def in crate::modules::all_modules() {
            node.register_module(def);
        }
        for def in crate::ship_types::all_ship_types() {
            node.register_ship_type(def);
        }

        let player_id = node.next_player_id();
        let station = node.station(StationId(0)).unwrap().clone();
        let ship_id = node.spawn_player_ship_at_pub(player_id, station.position);
        node.credit_station_item(player_id, ItemId::ScrapMetal, 5);
        assert!(matches!(
            node.dock_owned(
                player_id,
                ship_id,
                DockCommand {
                    station_id: StationId(0),
                }
            ),
            StationOperationOutcome::Accepted { .. }
        ));

        let json = node.build_player_loadout_json(ship_id).unwrap();
        let payload: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(payload["docked_station_id"].as_u64().unwrap(), 0);
        assert_eq!(payload["docked_station_name"], "Forge Station");
        let station_inventory = payload["station_inventory"].as_array().unwrap();
        let scrap = station_inventory
            .iter()
            .find(|row| row["item_type"] == "ScrapMetal")
            .expect("docked station inventory should be serialized");
        assert_eq!(scrap["count"].as_u64().unwrap(), 5);
    }

    #[test]
    fn player_loadout_json_uses_null_dock_context_after_undock() {
        use dawn_core::{DockCommand, StationId};

        let mut node = mem_node();
        let player_id = node.next_player_id();
        let station = node.station(StationId(0)).unwrap().clone();
        let ship_id = node.spawn_player_ship_at_pub(player_id, station.position);
        assert!(matches!(
            node.dock_owned(
                player_id,
                ship_id,
                DockCommand {
                    station_id: StationId(0),
                }
            ),
            StationOperationOutcome::Accepted { .. }
        ));
        assert!(matches!(
            node.undock_owned(player_id, ship_id),
            StationOperationOutcome::Accepted { .. }
        ));

        let json = node.build_player_loadout_json(ship_id).unwrap();
        let payload: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(payload["docked_station_id"].is_null());
        assert!(payload["docked_station_name"].is_null());
    }

    #[test]
    fn player_loadout_json_reports_the_true_active_ship_id() {
        let mut node = mem_node();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship(player_id);

        let json = node.build_player_loadout_json(ship_id).unwrap();
        let payload: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(payload["active_ship_id"].as_u64().unwrap(), ship_id.raw());
    }

    #[test]
    fn player_loadout_json_for_player_reports_null_active_ship_id_when_shipless() {
        use dawn_core::StationId;

        let mut node = mem_node();
        let player_id = node.next_player_id();
        node.docked_players.insert(player_id, StationId(0));

        let json = node
            .build_player_loadout_json_for_player(player_id)
            .unwrap();
        let payload: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(payload["active_ship_id"].is_null());
        assert!(payload["modules"].as_array().unwrap().is_empty());
    }

    #[test]
    fn player_loadout_json_for_player_reports_active_ship_id_when_disembarked_ship_still_owned() {
        use dawn_core::{DockCommand, StationId};

        let mut node = mem_node();
        let player_id = node.next_player_id();
        let station = node.station(StationId(0)).unwrap().clone();
        let ship_id = node.spawn_player_ship_at_pub(player_id, station.position);
        assert!(matches!(
            node.dock_owned(
                player_id,
                ship_id,
                DockCommand {
                    station_id: StationId(0),
                }
            ),
            StationOperationOutcome::Accepted { .. }
        ));
        node.disembark_owned(player_id).expect("disembark succeeds");

        let json = node
            .build_player_loadout_json_for_player(player_id)
            .unwrap();
        let payload: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(
            payload["active_ship_id"].is_null(),
            "disembarked player has no active ship, even though {ship_id:?} is still owned"
        );
    }

    #[test]
    fn player_loadout_json_lists_every_owned_ship_with_active_and_docked_flags() {
        use dawn_core::{DockCommand, StationId};

        let mut node = mem_node();
        for def in crate::ship_types::all_ship_types() {
            node.register_ship_type(def);
        }
        let player_id = node.next_player_id();
        let station = node.station(StationId(0)).unwrap().clone();
        let ship_id = node.spawn_player_ship_at_pub(player_id, station.position);
        assert!(matches!(
            node.dock_owned(
                player_id,
                ship_id,
                DockCommand {
                    station_id: StationId(0),
                }
            ),
            StationOperationOutcome::Accepted { .. }
        ));

        // A second owned ship, via Assemble (the starter Packaged Ship every
        // new player gets, spawner_logic.rs).
        let second_ship_id = node
            .assemble_ship_owned(
                player_id,
                AssembleCommand {
                    station_id: StationId(0),
                    ship_type_id: crate::ship_types::SHIP_TYPE_MAGPIE,
                },
            )
            .expect("assemble should succeed with the starter packaged ship");

        let json = node.build_player_loadout_json(ship_id).unwrap();
        let payload: serde_json::Value = serde_json::from_str(&json).unwrap();
        let owned_ships = payload["owned_ships"].as_array().unwrap();
        assert_eq!(owned_ships.len(), 2);

        let first_row = owned_ships
            .iter()
            .find(|row| row["ship_id"].as_u64().unwrap() == ship_id.raw())
            .expect("original ship listed");
        assert!(first_row["is_active"].as_bool().unwrap());
        assert_eq!(first_row["docked_station_id"].as_u64().unwrap(), 0);

        let second_row = owned_ships
            .iter()
            .find(|row| row["ship_id"].as_u64().unwrap() == second_ship_id.raw())
            .expect("assembled ship listed");
        assert!(
            !second_row["is_active"].as_bool().unwrap(),
            "Assemble must not auto-activate the new ship (ADR-0037)"
        );
        assert_eq!(second_row["docked_station_id"].as_u64().unwrap(), 0);
    }
}
