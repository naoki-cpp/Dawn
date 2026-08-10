use dawn_core::{
    events::ShipSpawned, DomainEvent, FitModuleCommand, NodeId, PlayerId, Position, ShipId,
    Velocity,
};
use dawn_ecs::components::{
    CapacitorComp, HullComp, IsBotComp, IsNpcComp, PositionComp, ShipStatsComp,
};
use dawn_ecs::Entity;

use crate::persistence::ShipSnapshot;
use crate::spawner::{generate_ships, SpawnConfig};
use crate::{modules, ship_types};

use super::SimulationNode;

impl SimulationNode {
    // ── Spawn ─────────────────────────────────────────────────────────────────

    /// The base-stats/ECS-component portion of ship materialization: derives
    /// `ShipStatsComp` from `ship_type_id`, records it in `base_stats` and
    /// `ships.type_ids`, and writes `ShipStatsComp`/`HullComp`/`CapacitorComp`
    /// onto the entity. Requires the entity and its `ships.index` mapping to
    /// already exist (via `insert_to_world`).
    ///
    /// This is the single core shared by every path that brings a ship into
    /// existence -- `insert_ship_entity` (live spawn/assemble),
    /// `spawn_player_ship_at`, and `ShipSpawned` replay (`apply_event.rs`).
    /// Before this was pulled out, `ShipSpawned` replay hand-rolled its own
    /// copy that omitted the `ships.type_ids` insertion and the
    /// `CapacitorComp` initialization -- both present on the live path -- so
    /// a ship spawned after the last legacy snapshot could come back from the
    /// current snapshot + tail-log restore missing state the live node had
    /// (issue #197). ADR-0049's future RecoveryDelta reducer must preserve the
    /// same invariant. Position/anchor setup is deliberately NOT part of this
    /// function: it differs legitimately per caller (a fresh `ShipSpawned`
    /// anchors at a real spawn position via `set_spawn_anchor`/
    /// `set_spawn_anchor_abs`; `ShipAssembled` starts at `Position::ORIGIN`
    /// and is placed by `settle_ship_into_station` instead), so each caller
    /// keeps doing it their own way before calling this.
    ///
    /// `unregistered_fallback` is a defensive policy for an unknown dynamic
    /// `ship_type_id` or legacy event. Required production IDs are validated
    /// before construction, so normal player/NPC creation never takes it.
    pub(super) fn materialize_ship_stats(
        &mut self,
        ship_id: ShipId,
        ship_type_id: dawn_core::ship_type::ShipTypeId,
        unregistered_fallback: ShipStatsComp,
    ) {
        let base = self
            .game_data
            .ship_type_registry
            .get(&ship_type_id)
            .map(|def| ShipStatsComp::from_base(&def.base_stats))
            .unwrap_or(unregistered_fallback);

        self.simulation.base_stats.insert(ship_id, base);
        self.simulation.ships.type_ids.insert(ship_id, ship_type_id);

        if let Some(&entity) = self.simulation.ships.index.get(&ship_id) {
            self.simulation.world.set_ship_stats(entity, base);
            if let Some(mut hull) = self.simulation.world.get_mut::<HullComp>(entity) {
                *hull = HullComp::new(base.max_shield, base.max_armor, base.max_hull);
            }
            let _ = self.simulation.world.insert_one(
                entity,
                CapacitorComp {
                    current: base.cap_max,
                },
            );
        }
    }

    /// Insert a ship entity into the ECS with stats derived from
    /// `ship_type_id`, for a `ship_id` the caller has already allocated (or is
    /// replaying from an event). Shared by `spawn_ship` (fresh ID, appends
    /// `ShipSpawned`) and `assemble_ship_owned`/its replay arm (appends
    /// `ShipAssembled` instead) -- each call site emits its own public event
    /// afterward since which event fits depends on why the ship came into
    /// being.
    pub(super) fn insert_ship_entity(
        &mut self,
        ship_id: ShipId,
        ship_type_id: dawn_core::ship_type::ShipTypeId,
        position: Position,
        velocity: Velocity,
    ) {
        self.insert_to_world(ship_id, position, velocity);
        self.set_spawn_anchor(ship_id, position);
        self.materialize_ship_stats(ship_id, ship_type_id, ShipStatsComp::NPC);
    }

    /// Spawn a Ship, record it in the ECS, and emit a `ShipSpawned` event.
    ///
    /// INV-004: the ID is generated from a monotonically increasing counter
    /// combined with `NodeId`.  IDs are never reused.
    pub fn spawn_ship(
        &mut self,
        ship_type_id: dawn_core::ship_type::ShipTypeId,
        position: Position,
        velocity: Velocity,
    ) -> ShipId {
        let ship_id = ShipId::new(self.node_id, self.simulation.id_counter);
        self.simulation.id_counter += 1;

        self.insert_ship_entity(ship_id, ship_type_id, position, velocity);

        self.emit_event(DomainEvent::ShipSpawned(ShipSpawned {
            ship_id,
            sector_id: self.sector_id,
            initial_position: position.into(),
            ship_type_id,
            tick: self.simulation.current_tick,
        }));

        ship_id
    }

    // ── Phase 5: PlayerId / ownership management ─────────────────────────────

    pub fn next_player_id(&mut self) -> PlayerId {
        let id = PlayerId(self.players.player_id_counter);
        self.players.player_id_counter = self
            .players
            .player_id_counter
            .checked_add(1)
            .expect("PlayerId allocator exhausted");
        self.persistence
            .observe_materialized_identities(std::iter::empty(), [id])
            .expect("repository identity watermark update");
        id
    }

    /// Default player spawn point: 2x the demo galaxy's Alpha star (Helios)
    /// radius (15_000 units) along +X, clear of the star body itself and
    /// far short of Gate 0 (600_000 units, at the Sector edge) so a fresh spawn
    /// doesn't start already inside the star or already in jump range.
    pub const DEFAULT_PLAYER_SPAWN: Position = Position {
        x: 30_000.0,
        y: 0.0,
        z: 0.0,
    };

    /// Spawn a player ship at the default starting position.
    pub fn spawn_player_ship(&mut self, player_id: PlayerId) -> ShipId {
        self.spawn_player_ship_at(player_id, Self::DEFAULT_PLAYER_SPAWN)
    }

    /// Spawn a player ship at a specific position.
    pub fn spawn_player_ship_at_pub(&mut self, player_id: PlayerId, pos: Position) -> ShipId {
        self.spawn_player_ship_at(player_id, pos)
    }

    pub(super) fn spawn_player_ship_at(&mut self, player_id: PlayerId, pos: Position) -> ShipId {
        use crate::ship_types::SHIP_TYPE_MAGPIE;
        let ship_id = ShipId::new(self.node_id, self.simulation.id_counter);
        self.simulation.id_counter += 1;

        self.insert_to_world(ship_id, pos, Velocity::ZERO);
        self.set_spawn_anchor(ship_id, pos);
        self.materialize_ship_stats(ship_id, SHIP_TYPE_MAGPIE, ShipStatsComp::PLAYER);

        if let Some(&entity) = self.simulation.ships.index.get(&ship_id) {
            let _ = self.simulation.world.remove_one::<IsNpcComp>(entity);
        }

        // Record ownership before fit_module (needed for is_npc check).
        self.players.active_ship.insert(player_id, ship_id);
        self.players.owners.insert(ship_id, player_id);
        let resume_ticket = self.issue_resume_ticket();
        self.record_client_resume_ownership(ship_id, player_id, resume_ticket);

        // Seed the starting inventory (ADR-0032) before the default loadout
        // below: those fit_module calls are the unchecked, privileged spawn
        // path and don't consume from it, so seeding first vs. after makes no
        // functional difference -- but it reads naturally as "the player owns
        // everything, some of it happens to already be fitted."
        if let Some(&entity) = self.simulation.ships.index.get(&ship_id) {
            self.seed_player_inventory(entity);
        }
        // Starter Packaged Ship (ADR-0034/0037 round trip): every new player
        // gets one immediately, so Assemble/Disembark/SelectActiveShip/Undock
        // is exercisable from a fresh connect without first Disassembling
        // their only ship. For now the starter packaged ship lives at the
        // demo NPC station (`StationId(0)`), which is also where fresh-play
        // station flows are exercised.
        self.credit_station_item(
            player_id,
            dawn_core::StationId(0),
            dawn_core::ItemId::PackagedShip(SHIP_TYPE_MAGPIE),
            1,
        );

        use dawn_core::SlotKind;
        self.fit_module(FitModuleCommand {
            ship_id,
            slot: SlotKind::High,
            module_id: crate::modules::MODULE_RAILGUN_SMALL,
        });
        self.fit_module(FitModuleCommand {
            ship_id,
            slot: SlotKind::Mid,
            module_id: crate::modules::MODULE_AFTERBURNER,
        });
        self.fit_module(FitModuleCommand {
            ship_id,
            slot: SlotKind::Mid,
            module_id: crate::modules::MODULE_FOLD_DISRUPTOR,
        });

        self.emit_event(DomainEvent::ShipSpawned(ShipSpawned {
            ship_id,
            sector_id: self.sector_id,
            initial_position: pos.into(),
            ship_type_id: SHIP_TYPE_MAGPIE,
            tick: self.simulation.current_tick,
        }));

        ship_id
    }

    /// Register `player_id` as the owner of a ship already present in this
    /// node's ECS — the ownership handoff after a Sector Transit moved a
    /// player's ship into this Sector (ADR-0009 / ADR-0014).
    ///
    /// Returns `false` (and registers nothing) if the ship is not in this
    /// node's ECS.
    pub fn adopt_player_ship(&mut self, ship_id: ShipId, player_id: PlayerId) -> bool {
        if !self.simulation.ships.index.contains_key(&ship_id) {
            return false;
        }
        self.players.active_ship.insert(player_id, ship_id);
        self.players.owners.insert(ship_id, player_id);
        true
    }

    /// Spawn a Bot ship at the given position.
    ///
    /// The Bot receives a real `PlayerId` and goes through the same player
    /// command pipeline. `IsBotComp` marks it for `process_bots()` each tick.
    pub fn spawn_bot_ship(&mut self, spawn_pos: Position) -> (PlayerId, ShipId) {
        let player_id = self.next_player_id();
        let ship_id = self.spawn_player_ship_at(player_id, spawn_pos);
        if let Some(&entity) = self.simulation.ships.index.get(&ship_id) {
            let _ = self.simulation.world.insert_one(entity, IsBotComp);
        }
        (player_id, ship_id)
    }

    // `process_bots` (Bot AI decision loop) moved to `node/bot_ai.rs`
    // (`/improve-codebase-architecture` candidate 3, 2026-07-03) — spawning a
    // bot is spawn mechanics, deciding what it does each tick is a separate
    // concern with its own test fixtures.

    /// Spawn `count` NPC frigates, each fitted with a small railgun.
    ///
    /// Shared by both `dawn-server` binaries.
    /// (`/improve-codebase-architecture` deepening, 2026-07-05) — both binaries
    /// used to hand-roll an identical copy of this loop.
    pub fn spawn_npc_frigates(&mut self, count: usize) {
        let config = SpawnConfig::default_for_node(NodeId(0));
        for (_, pos, vel) in generate_ships(count, &config, 0) {
            let ship_id = self.spawn_ship(ship_types::SHIP_TYPE_NPC_FRIGATE, pos, vel);
            self.fit_module(FitModuleCommand {
                ship_id,
                slot: dawn_core::SlotKind::High,
                module_id: modules::MODULE_RAILGUN_SMALL,
            });
        }
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    /// Add a Ship to the ECS World and record the entity in `ship_index`.
    /// Does NOT append any event — used by `spawn_ship` and replay.
    pub(super) fn insert_to_world(
        &mut self,
        ship_id: ShipId,
        position: Position,
        velocity: Velocity,
    ) {
        let entity = self
            .simulation
            .world
            .spawn_ship(ship_id, position, velocity);
        self.simulation.ships.index.insert(ship_id, entity);
        self.persistence
            .observe_materialized_identities([ship_id], std::iter::empty())
            .expect("repository identity watermark update");
        // Default to the Sector origin anchor (the star). Spawn paths override
        // this with the nearest body via `set_spawn_anchor`; restore overrides it
        // with the persisted anchor. `position` here is treated as the offset.
        let anchor = self
            .topology
            .anchor_table
            .sector_origin_anchor(self.sector_id)
            .unwrap_or(dawn_core::AnchorId(0));
        self.simulation.world.set_ship_anchor(entity, anchor);
    }

    /// Anchor a freshly-spawned ship on the NEAREST celestial body and store its
    /// position as a small offset from that anchor (ADR-0029 review #1). The
    /// argument is the spawn position in the Sector-absolute (star-origin) frame.
    ///
    /// Keeping each ship anchored to a nearby body is the invariant that makes
    /// method B work at true AU: a ship far from the star must not hold a ~10^11 m
    /// star-relative offset when a nearer body anchor is available. At the compressed
    /// scale the star is the nearest body for all current spawns, so this is a
    /// no-op today; it becomes load-bearing once bodies sit at real AU.
    ///
    /// Deterministic: `ShipSpawned` replay calls this with the same
    /// `initial_position`, reproducing the same anchor (later `AnchorRebased`
    /// events replay the warp rebases on top).
    pub(super) fn set_spawn_anchor(&mut self, ship_id: ShipId, abs_pos: Position) {
        let Some(&entity) = self.simulation.ships.index.get(&ship_id) else {
            return;
        };
        let world = [abs_pos.x, abs_pos.y, abs_pos.z];
        let anchor = self
            .topology
            .anchor_table
            .nearest_anchor(self.sector_id, world.into())
            .unwrap_or(dawn_core::AnchorId(0));
        let offset = match self.topology.anchor_table.abs(anchor) {
            Some(a) => Position::new(world[0] - a[0], world[1] - a[1], world[2] - a[2]),
            None => {
                super::debug_assert_missing_anchor(anchor, "set_spawn_anchor");
                abs_pos
            }
        };
        self.simulation.world.set_ship_anchor(entity, anchor);
        if let Some(mut p) = self.simulation.world.get_mut::<PositionComp>(entity) {
            p.0 = offset;
        }
    }

    /// Re-anchor a ship from an absolute f64 point directly, preserving the
    /// anchor/offset split for a precise spawn location.
    ///
    /// This is also useful for tests that construct a fixture "near gate G" or
    /// "at body B" from a true-AU absolute coordinate: the anchor/offset split
    /// is performed directly in f64, the same way production code handles warp
    /// arrival and `AnchorTable` composition.
    #[cfg(test)]
    pub(crate) fn set_spawn_anchor_abs<P: Into<[f64; 3]>>(&mut self, ship_id: ShipId, world: P) {
        let Some(&entity) = self.simulation.ships.index.get(&ship_id) else {
            return;
        };
        self.place_entity_at_absolute(entity, world.into());
    }

    /// Re-anchor an existing entity to the nearest anchor for `world` and set
    /// its local offset so its absolute position becomes exactly `world`.
    pub(super) fn place_entity_at_absolute<P: Into<[f64; 3]>>(&mut self, entity: Entity, world: P) {
        let world = world.into();
        let anchor = self
            .topology
            .anchor_table
            .nearest_anchor(self.sector_id, world.into())
            .unwrap_or(dawn_core::AnchorId(0));
        let offset = match self.topology.anchor_table.abs(anchor) {
            Some(a) => Position::new(world[0] - a[0], world[1] - a[1], world[2] - a[2]),
            None => Position::new(world[0], world[1], world[2]),
        };
        self.simulation.world.set_ship_anchor(entity, anchor);
        if let Some(mut p) = self.simulation.world.get_mut::<PositionComp>(entity) {
            p.0 = offset;
        }
    }

    /// Reconstruct a Ship's full ECS state (stats, hull, capacitor, fitting)
    /// from a [`ShipSnapshot`]. Used by `from_snapshot` (node restart) and
    /// `import_transit` (Sector Transit, ADR-0014). Does NOT append any event.
    pub(super) fn restore_ship_from_snapshot(&mut self, ship: &ShipSnapshot) {
        use dawn_ecs::components::{FittingComp, TackledComp};
        self.insert_to_world(ship.ship_id, ship.position, ship.velocity);
        self.simulation
            .ships
            .type_ids
            .insert(ship.ship_id, ship.ship_type_id);
        // Restore the coordinate anchor (ADR-0029): insert_to_world defaults to
        // the Sector-origin anchor, but a rebased ship's `position` offset is
        // relative to its saved anchor, so restore that to keep absolute position.
        if let Some(&entity) = self.simulation.ships.index.get(&ship.ship_id) {
            if let Some(absolute_position) = ship.absolute_position {
                // New snapshots preserve the f64 authority. Keep the saved
                // anchor when it is still known so restore does not silently
                // change the representation of an anchored ship.
                if let Some(offset) = self
                    .topology
                    .anchor_table
                    .to_relative(ship.anchor, absolute_position)
                {
                    self.simulation.world.set_ship_anchor(entity, ship.anchor);
                    if let Some(mut position) =
                        self.simulation.world.get_mut::<PositionComp>(entity)
                    {
                        position.0 = offset;
                    }
                } else {
                    self.place_entity_at_absolute(entity, absolute_position);
                }
            } else {
                // Pre-ADR-0044 snapshots only contain the local offset.
                self.simulation.world.set_ship_anchor(entity, ship.anchor);
            }
        }

        let base = if self.players.owners.contains_key(&ship.ship_id) {
            // Ownership is the persisted discriminator for the player profile.
            // Rebuilding an owned ship with NPC base stats changes thrust,
            // capacitor, and every subsequent tick after restart.
            ShipStatsComp::PLAYER
        } else {
            self.game_data
                .ship_type_registry
                .get(&ship.ship_type_id)
                .map(|def| ShipStatsComp::from_base(&def.base_stats))
                .unwrap_or(ShipStatsComp::NPC)
        };
        self.simulation.base_stats.insert(ship.ship_id, base);

        if let Some(&entity) = self.simulation.ships.index.get(&ship.ship_id) {
            self.simulation.world.set_ship_stats(entity, base);

            let fitting =
                FittingComp::from_snapshot(&ship.fitting, &self.game_data.module_registry);
            let _ = self.simulation.world.insert_one(entity, fitting);

            // apply_fitting recomputes ShipStatsComp and rescales HullComp;
            // restore the exact HP layers from the snapshot afterwards.
            self.reapply_fitting(ship.ship_id);
            if let Some(mut hull) = self.simulation.world.get_mut::<HullComp>(entity) {
                hull.set_hp(ship.current_shield, ship.current_armor, ship.current_hull);
            }

            if let Some(cap) = ship.capacitor {
                let _ = self
                    .simulation
                    .world
                    .insert_one(entity, CapacitorComp { current: cap });
            }

            if !ship.tackled_by.is_empty() {
                let _ = self.simulation.world.insert_one(
                    entity,
                    TackledComp {
                        tacklers: ship.tackled_by.clone(),
                    },
                );
            }

            // Inventory (ADR-0032): restore exactly what was persisted,
            // regardless of ship type -- post-spawn Fit/Unfit could have
            // emptied or refilled it differently from the deterministic seed.
            let _ = self.simulation.world.insert_one(
                entity,
                dawn_ecs::components::InventoryComp {
                    items: ship.inventory.clone(),
                },
            );
        }
    }

    /// Mark a ship as a player ship and apply the PLAYER stat profile.
    ///
    /// Test-only setup helper; the serve path adopts player ships via
    /// `adopt_player_ship`.
    #[cfg(test)]
    pub fn set_player_ship(&mut self, ship_id: ShipId) {
        if let Some(&entity) = self.simulation.ships.index.get(&ship_id) {
            self.simulation
                .base_stats
                .insert(ship_id, ShipStatsComp::PLAYER);
            self.simulation
                .world
                .set_ship_stats(entity, ShipStatsComp::PLAYER);
            if let Some(mut hull) = self.simulation.world.get_mut::<HullComp>(entity) {
                *hull = HullComp::new(
                    ShipStatsComp::PLAYER.max_shield,
                    ShipStatsComp::PLAYER.max_armor,
                    ShipStatsComp::PLAYER.max_hull,
                );
            }
            self.reapply_fitting(ship_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{DomainEvent, NodeId, Position, SectorBounds, SectorId, Tick};

    fn node_with_catalog() -> SimulationNode {
        SimulationNode::new_test(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        )
    }

    /// Player construction uses the validated catalog selected for the node.
    /// The default Magpie is a required definition, so the old empty-registry
    /// fallback state no longer exists.
    #[test]
    fn player_spawn_uses_the_validated_magpie_stats() {
        let mut node = node_with_catalog();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship(player_id);

        let stats = node
            .get_ship_stats(ship_id)
            .expect("spawned ship must have stats");
        let magpie = crate::game_data::test_catalog()
            .ship_type(crate::ship_types::SHIP_TYPE_MAGPIE)
            .expect("validated catalog contains the required Magpie definition");
        assert_eq!(stats.max_shield, magpie.base_stats.max_shield);
        assert_eq!(stats.max_speed, magpie.base_stats.max_speed);
        assert_eq!(stats.cap_max, magpie.base_stats.cap_max);
    }

    #[test]
    fn a_new_player_starts_with_one_packaged_ship_in_station_inventory() {
        // ADR-0034/0037 round trip: every new player can exercise
        // Assemble/Disembark/SelectActiveShip/Undock immediately, without
        // first Disassembling their only ship.
        let mut node = node_with_catalog();
        let player_id = node.next_player_id();
        node.spawn_player_ship(player_id);

        assert_eq!(
            node.station_item_count(
                player_id,
                dawn_core::StationId(0),
                dawn_core::ItemId::PackagedShip(crate::ship_types::SHIP_TYPE_MAGPIE)
            ),
            1
        );
    }

    #[test]
    fn spawned_ship_is_anchored_on_the_nearest_body() {
        // ADR-0029 review #1: a ship anchors on the nearest celestial body.
        // A spawn near the star anchors on Helios (id 0); a spawn at Forge's
        // position (160,000, 0, 100,000 in compressed units) anchors on Forge
        // (id 1) with a ~zero offset, keeping the offset small (the method-B
        // invariant). Distances stay correct via the absolute accessors.
        let mut node = node_with_catalog();
        let near_star = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(30_000.0, 0.0, 0.0),
            dawn_core::Velocity::ZERO,
        );
        assert_eq!(
            node.get_ship_anchor(near_star),
            Some(dawn_core::AnchorId(0))
        );

        let forge_abs = node.anchor_table().abs(dawn_core::AnchorId(1)).unwrap();
        // Spawn anywhere, then re-anchor from the f64 source directly
        // (`set_spawn_anchor_abs`) so the fixture exercises the same precise
        // anchor/offset split used by production code.
        let at_forge = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::ORIGIN,
            dawn_core::Velocity::ZERO,
        );
        node.set_spawn_anchor_abs(at_forge, forge_abs);
        assert_eq!(node.get_ship_anchor(at_forge), Some(dawn_core::AnchorId(1)));
        // Offset under the anchor is ~zero (small), and the absolute position is
        // recovered exactly.
        let abs = node.ship_absolute(at_forge).unwrap();
        assert!((abs[0] - forge_abs[0]).abs() < 1.0 && (abs[2] - forge_abs[2]).abs() < 1.0);
    }

    #[test]
    fn anchor_rebased_preserves_absolute_position_and_updates_anchor() {
        use dawn_core::{events::AnchorRebased, AnchorId};
        let mut node = node_with_catalog();
        let ship = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(30_000.0, 0.0, 0.0),
            dawn_core::Velocity::ZERO,
        );
        // AnchorRebased sets the anchor and a (small, near-Forge) offset; the
        // absolute position composes anchor_abs(f64) + offset exactly (ADR-0029).
        let forge_abs = node.anchor_table().abs(AnchorId(1)).unwrap();
        let new_off = Position::new(2_000.0, 0.0, -1_500.0);
        node.apply_event_pub(DomainEvent::AnchorRebased(AnchorRebased {
            ship_id: ship,
            anchor: AnchorId(1),
            offset: new_off,
            tick: Tick(1),
        }));
        assert_eq!(node.get_ship_anchor(ship), Some(AnchorId(1)));
        let after = node.ship_absolute(ship).unwrap();
        assert!(
            (after[0] - (forge_abs[0] + 2_000.0)).abs() < 1e-2,
            "x {}",
            after[0]
        );
        assert!(
            (after[2] - (forge_abs[2] - 1_500.0)).abs() < 1e-2,
            "z {}",
            after[2]
        );
    }

    #[test]
    fn snapshot_restore_preserves_a_rebased_ships_anchor_and_absolute_position() {
        use dawn_core::{events::AnchorRebased, AnchorId};
        use dawn_event_store::InMemoryEventStore;
        let mut node = node_with_catalog();
        let ship = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(160_000.0, 0.0, 0.0),
            dawn_core::Velocity::ZERO,
        );
        // Rebase onto Forge (AnchorId 1), preserving absolute position.
        let world = node.ship_absolute(ship).unwrap();
        let forge = node.anchor_table().abs(AnchorId(1)).unwrap();
        let off = Position::new(
            world[0] - forge[0],
            world[1] - forge[1],
            world[2] - forge[2],
        );
        node.apply_event_pub(DomainEvent::AnchorRebased(AnchorRebased {
            ship_id: ship,
            anchor: AnchorId(1),
            offset: off,
            tick: Tick(1),
        }));
        let before = node.ship_absolute(ship).unwrap();

        let snap = node.take_snapshot();
        assert_eq!(
            snap.ships
                .iter()
                .find(|s| s.ship_id == ship)
                .unwrap()
                .anchor,
            AnchorId(1),
            "snapshot must capture the rebased anchor"
        );

        let node2 = SimulationNode::restore_from_test(
            InMemoryEventStore::new(),
            &snap,
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
            crate::game_data::test_catalog().modules(),
            crate::game_data::test_catalog().ship_types(),
        );
        assert_eq!(
            node2.get_ship_anchor(ship),
            Some(AnchorId(1)),
            "restore must keep the anchor"
        );
        let after = node2.ship_absolute(ship).unwrap();
        let err = ((before[0] - after[0]).powi(2)
            + (before[1] - after[1]).powi(2)
            + (before[2] - after[2]).powi(2))
        .sqrt();
        assert!(err < 1.0, "restore moved absolute position by {err} m");
    }

    #[test]
    fn ship_distance_is_correct_across_different_anchors() {
        use dawn_core::{events::AnchorRebased, AnchorId};
        let mut node = node_with_catalog();
        // Ship a near the star (small offset under Helios), ship b near Forge
        // (small offset under its own anchor). Each is precise locally; the
        // cross-anchor distance composes both in f64 (ADR-0029 / spike B-3).
        let a = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(1000.0, 0.0, 0.0),
            dawn_core::Velocity::ZERO,
        );
        let b = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(2000.0, 0.0, 0.0),
            dawn_core::Velocity::ZERO,
        );
        let off_b = Position::new(500.0, 0.0, 0.0);
        node.apply_event_pub(DomainEvent::AnchorRebased(AnchorRebased {
            ship_id: b,
            anchor: AnchorId(1),
            offset: off_b,
            tick: Tick(1),
        }));
        let forge_abs = node.anchor_table().abs(AnchorId(1)).unwrap();
        let a_abs = [1000.0_f64, 0.0, 0.0];
        let b_abs = [forge_abs[0] + 500.0, forge_abs[1], forge_abs[2]];
        let d = [
            a_abs[0] - b_abs[0],
            a_abs[1] - b_abs[1],
            a_abs[2] - b_abs[2],
        ];
        let expected = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let after = node.ship_distance(a, b).unwrap();
        assert!(
            (after - expected).abs() < 1.0,
            "cross-anchor distance {after} != {expected}"
        );
    }

    #[test]
    fn spawn_npc_frigates_spawns_the_requested_count_each_fitted_with_a_railgun() {
        // `/improve-codebase-architecture` deepening (2026-07-05): this method
        // replaces what used to be two hand-rolled copies, one per binary
        // (the `dawn-server` simulate and sector-node binaries).
        use crate::modules;

        let mut node = node_with_catalog();
        node.spawn_npc_frigates(3);
        assert_eq!(node.ship_count(), 3);

        for &ship_id in node.simulation.ships.index.keys() {
            let loadout = node
                .build_player_loadout_json(ship_id)
                .expect("every spawned ship has a loadout");
            assert!(
                loadout
                    .modules
                    .iter()
                    .any(|m| m.slot == "High" && m.module_id == modules::MODULE_RAILGUN_SMALL.0),
                "ship {ship_id:?} should have a Small Railgun in its High slot"
            );
        }
    }
}
