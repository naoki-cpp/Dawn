use dawn_core::{
    events::ShipSpawned, ship_type::ShipTypeDefinition, DomainEvent, FitModuleCommand, PlayerId,
    Position, ShipId, Velocity,
};
use dawn_ecs::{
    components::{
        CapacitorComp, FittingComp, HullComp, IsBotComp, IsNpcComp, LockComp, PositionComp,
        ShipStatsComp, WarpComp,
    },
    systems::apply_fitting,
};
use dawn_event_store::store::EventStore;

use crate::persistence::ShipSnapshot;

use super::SimulationNode;

impl<S: EventStore> SimulationNode<S> {
    // ── Spawn ─────────────────────────────────────────────────────────────────

    /// Spawn a Ship, record it in the ECS, append a `ShipSpawned` event.
    ///
    /// INV-004: the ID is generated from a monotonically increasing counter
    /// combined with `NodeId`.  IDs are never reused.
    pub fn spawn_ship(
        &mut self,
        ship_type_id: dawn_core::ship_type::ShipTypeId,
        position: Position,
        velocity: Velocity,
    ) -> ShipId {
        let ship_id = ShipId::new(self.node_id, self.id_counter);
        self.id_counter += 1;

        let base = self
            .ship_type_registry
            .get(&ship_type_id)
            .map(|def| ShipStatsComp::from_base(&def.base_stats))
            .unwrap_or(ShipStatsComp::NPC);

        self.insert_to_world(ship_id, position, velocity);
        self.set_spawn_anchor(ship_id, position);
        self.base_stats.insert(ship_id, base);
        self.ships.type_ids.insert(ship_id, ship_type_id);

        // Update ShipStatsComp, HullComp, and CapacitorComp to match base stats.
        if let Some(&entity) = self.ships.index.get(&ship_id) {
            self.world.set_ship_stats(entity, base);
            if let Ok(mut hull) = self.world.inner_mut().get::<&mut HullComp>(entity) {
                *hull = HullComp::new(base.max_shield, base.max_armor, base.max_hull);
            }
            // Initialize capacitor to full.
            let _ = self.world.inner_mut().insert_one(
                entity,
                CapacitorComp {
                    current: base.cap_max,
                },
            );
        }

        self.event_store
            .append(DomainEvent::ShipSpawned(ShipSpawned {
                ship_id,
                sector_id: self.sector_id,
                initial_position: position,
                ship_type_id,
                tick: self.current_tick,
            }));

        ship_id
    }

    // ── ShipType ──────────────────────────────────────────────────────────────

    pub fn register_ship_type(&mut self, def: ShipTypeDefinition) {
        self.ship_type_registry.insert(def.id, def);
    }

    // ── Phase 5: PlayerId / ownership management ─────────────────────────────

    pub fn next_player_id(&mut self) -> PlayerId {
        let id = PlayerId(self.player_id_counter);
        self.player_id_counter += 1;
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
        let ship_id = ShipId::new(self.node_id, self.id_counter);
        self.id_counter += 1;

        let base = self
            .ship_type_registry
            .get(&SHIP_TYPE_MAGPIE)
            .map(|def| ShipStatsComp::from_base(&def.base_stats))
            .unwrap_or(ShipStatsComp::PLAYER);

        self.insert_to_world(ship_id, pos, Velocity::ZERO);
        self.set_spawn_anchor(ship_id, pos);
        self.base_stats.insert(ship_id, base);
        self.ships.type_ids.insert(ship_id, SHIP_TYPE_MAGPIE);

        if let Some(&entity) = self.ships.index.get(&ship_id) {
            self.world.set_ship_stats(entity, base);
            if let Ok(mut hull) = self.world.inner_mut().get::<&mut HullComp>(entity) {
                *hull = HullComp::new(base.max_shield, base.max_armor, base.max_hull);
            }
            let _ = self.world.inner_mut().insert_one(
                entity,
                CapacitorComp {
                    current: base.cap_max,
                },
            );
            let _ = self.world.inner_mut().remove_one::<IsNpcComp>(entity);
        }

        // Record ownership before fit_module (needed for is_npc check).
        self.ships.by_player.insert(player_id, ship_id);
        self.ships.owners.insert(ship_id, player_id);

        // Seed the starting inventory (ADR-0032) before the default loadout
        // below: those fit_module calls are the unchecked, privileged spawn
        // path and don't consume from it, so seeding first vs. after makes no
        // functional difference -- but it reads naturally as "the player owns
        // everything, some of it happens to already be fitted."
        if let Some(&entity) = self.ships.index.get(&ship_id) {
            self.seed_player_inventory(entity);
        }

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

        self.event_store
            .append(DomainEvent::ShipSpawned(ShipSpawned {
                ship_id,
                sector_id: self.sector_id,
                initial_position: pos,
                ship_type_id: SHIP_TYPE_MAGPIE,
                tick: self.current_tick,
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
        if !self.ships.index.contains_key(&ship_id) {
            return false;
        }
        self.ships.by_player.insert(player_id, ship_id);
        self.ships.owners.insert(ship_id, player_id);
        true
    }

    /// Spawn a Bot ship at the given position.
    ///
    /// The Bot receives a real `PlayerId` and goes through the same player
    /// command pipeline. `IsBotComp` marks it for `process_bots()` each tick.
    pub fn spawn_bot_ship(&mut self, spawn_pos: Position) -> (PlayerId, ShipId) {
        let player_id = self.next_player_id();
        let ship_id = self.spawn_player_ship_at(player_id, spawn_pos);
        if let Some(&entity) = self.ships.index.get(&ship_id) {
            let _ = self.world.inner_mut().insert_one(entity, IsBotComp);
        }
        (player_id, ship_id)
    }

    /// Run the Bot AI for all `IsBotComp` ships.
    ///
    /// Bots issue the exact same commands as a human player:
    /// `MoveCommand`, `LockOnCommand`, `ActivateModuleCommand`.
    /// Called each tick after Combat System.
    pub fn process_bots(&mut self) {
        // ── 1. Snapshot bot state (read-only pass) ────────────────────────────
        struct BotState {
            player_id: PlayerId,
            ship_id: ShipId,
            position: Position,
            weapon_range: f32,
            locked_targets: Vec<ShipId>,
            weapon_modules: Vec<(dawn_core::ModuleId, dawn_core::fitting::SlotKind)>,
            // HP fraction for flee decision (current / max across all three layers).
            hp_fraction: f32,
            // True if WarpComp is already attached (alignment or warping in progress).
            is_warping: bool,
        }

        let mut bots: Vec<BotState> = Vec::new();
        for (&ship_id, &entity) in &self.ships.index {
            if self.world.inner().get::<&IsBotComp>(entity).is_err() {
                continue;
            }
            let Some(&player_id) = self.ships.owners.get(&ship_id) else {
                continue;
            };
            let Ok(pos) = self.world.inner().get::<&PositionComp>(entity) else {
                continue;
            };
            let Ok(stats) = self.world.inner().get::<&ShipStatsComp>(entity) else {
                continue;
            };
            let Ok(lock) = self.world.inner().get::<&LockComp>(entity) else {
                continue;
            };
            let locked: Vec<ShipId> = lock
                .entries
                .iter()
                .filter(|e| matches!(e.state, dawn_ecs::components::LockState::Locked))
                .map(|e| e.target_id)
                .collect();
            // Bots only auto-activate Weapon modules. Other Active modules
            // (e.g. Afterburner) would drain the capacitor pointlessly while
            // the bot is braking to hold its firing position.
            let weapon_modules = self
                .world
                .inner()
                .get::<&FittingComp>(entity)
                .map(|f| {
                    f.iter_slots()
                        .filter(|s| s.def.kind == dawn_core::fitting::ModuleKind::Weapon)
                        .map(|s| (s.def.id, s.def.slot))
                        .collect()
                })
                .unwrap_or_default();
            let hp_fraction = if let Ok(hull) = self.world.inner().get::<&HullComp>(entity) {
                let max_hp = stats.max_shield + stats.max_armor + stats.max_hull;
                let cur_hp = hull.current_shield + hull.current_armor + hull.current_hull;
                if max_hp > 0.0 {
                    cur_hp / max_hp
                } else {
                    1.0
                }
            } else {
                1.0
            };
            let is_warping = self.world.inner().get::<&WarpComp>(entity).is_ok();
            bots.push(BotState {
                player_id,
                ship_id,
                // Absolute (Sector-frame) position so distance/steering toward a
                // target on a different anchor is correct (ADR-0029).
                position: self.entity_absolute(entity, pos.0),
                weapon_range: stats.weapon_range,
                locked_targets: locked,
                weapon_modules,
                hp_fraction,
                is_warping,
            });
        }

        // ── 2. Snapshot human player target positions ─────────────────────────
        struct TargetInfo {
            ship_id: ShipId,
            position: Position,
        }
        let mut targets: Vec<TargetInfo> = Vec::new();
        for (&ship_id, &entity) in &self.ships.index {
            if self.world.inner().get::<&IsBotComp>(entity).is_ok() {
                continue;
            }
            if self.world.inner().get::<&IsNpcComp>(entity).is_ok() {
                continue;
            }
            if !self.ships.owners.contains_key(&ship_id) {
                continue;
            }
            let Ok(pos) = self.world.inner().get::<&PositionComp>(entity) else {
                continue;
            };
            let abs = self.entity_absolute(entity, pos.0);
            targets.push(TargetInfo {
                ship_id,
                position: abs,
            });
        }

        if targets.is_empty() {
            return;
        }

        // ── 3. Issue commands (same pipeline as human player) ─────────────────
        // Collect gate list once — shared by all bots this tick.
        let gates: Vec<(dawn_core::JumpGateId, Position)> = self
            .sector_map
            .gates
            .iter()
            .map(|(&id, def)| (id, def.position))
            .collect();

        for bot in bots {
            // Below 50% HP the bot attempts to warp to the nearest gate.
            // If warp succeeds (or is already in progress), pause combat AI —
            // the bot is committed to fleeing.
            // If warp is blocked by tackle, keep fighting; the tackle is what
            // holds the bot here and the player must sustain it to finish the kill.
            if bot.hp_fraction < 0.50 {
                if bot.is_warping {
                    continue; // alignment/warp in progress — skip combat AI
                }
                let warp_started = if let Some(&(gate_id, _)) = gates.iter().min_by(|a, b| {
                    bot.position
                        .distance_squared(a.1)
                        .partial_cmp(&bot.position.distance_squared(b.1))
                        .unwrap_or(std::cmp::Ordering::Equal)
                }) {
                    self.apply_warp_command(
                        bot.ship_id,
                        dawn_core::WarpTarget::Gate(gate_id),
                        false,
                    )
                } else {
                    false
                };
                if warp_started {
                    continue; // just attached WarpComp — skip combat AI this tick
                }
                // Warp blocked (tackled or no gate) — fall through to combat AI.
            }

            // Find closest human target.
            let Some(target) = targets.iter().min_by(|a, b| {
                bot.position
                    .distance_squared(a.position)
                    .partial_cmp(&bot.position.distance_squared(b.position))
                    .unwrap_or(std::cmp::Ordering::Equal)
            }) else {
                continue;
            };

            let dist = bot.position.distance(target.position);

            // Lock on if not already locked or locking.
            let already_targeting = bot.locked_targets.contains(&target.ship_id);
            if !already_targeting {
                // Queue lock command for the NEXT tick's LockSystem.
                // (LockSystem already ran this tick before process_bots.)
                self.pending_bot_lock_commands
                    .push(dawn_core::LockOnCommand {
                        ship_id: bot.ship_id,
                        target_id: target.ship_id,
                    });
            }

            // Move: approach until within 75% of weapon range, then brake to stop.
            let engage_range = (bot.weapon_range * 0.75).max(500.0);
            if dist > engage_range {
                let dx = target.position.x - bot.position.x;
                let dy = target.position.y - bot.position.y;
                let dz = target.position.z - bot.position.z;
                let len = (dx * dx + dy * dy + dz * dz).sqrt().max(1.0);
                let thrust_target = Position::new(
                    bot.position.x + dx / len * 1_000_000.0,
                    bot.position.y + dy / len * 1_000_000.0,
                    bot.position.z + dz / len * 1_000_000.0,
                );
                self.apply_move_command_owned(bot.player_id, bot.ship_id, thrust_target);
            } else {
                // Within engage range: brake to stop so turrets have a clean shot.
                self.apply_stop_command_owned(bot.player_id, bot.ship_id);
            }

            // Activate weapons once target is locked.
            if already_targeting {
                for (module_id, slot) in &bot.weapon_modules {
                    self.activate_module_owned(
                        bot.player_id,
                        dawn_core::ActivateModuleCommand {
                            ship_id: bot.ship_id,
                            module_id: *module_id,
                            slot: *slot,
                            target_ship_id: Some(target.ship_id),
                        },
                    );
                }
            }
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
        let entity = self.world.spawn_ship(ship_id, position, velocity);
        self.ships.index.insert(ship_id, entity);
        // Default to the Sector origin anchor (the star). Spawn paths override
        // this with the nearest body via `set_spawn_anchor`; restore overrides it
        // with the persisted anchor. `position` here is treated as the offset.
        let anchor = self
            .anchor_table
            .sector_origin_anchor(self.sector_id)
            .unwrap_or(dawn_core::AnchorId(0));
        self.world.set_ship_anchor(entity, anchor);
    }

    /// Anchor a freshly-spawned ship on the NEAREST celestial body and store its
    /// position as a small offset from that anchor (ADR-0029 review #1). The
    /// argument is the spawn position in the Sector-absolute (star-origin) frame.
    ///
    /// Keeping each ship anchored to a nearby body is the invariant that makes
    /// method B work at true AU: a ship far from the star must not hold a ~10^11 m
    /// star-relative f32 offset (which loses ~km of precision). At the compressed
    /// scale the star is the nearest body for all current spawns, so this is a
    /// no-op today; it becomes load-bearing once bodies sit at real AU.
    ///
    /// Deterministic: `ShipSpawned` replay calls this with the same
    /// `initial_position`, reproducing the same anchor (later `AnchorRebased`
    /// events replay the warp rebases on top).
    pub(super) fn set_spawn_anchor(&mut self, ship_id: ShipId, abs_pos: Position) {
        let Some(&entity) = self.ships.index.get(&ship_id) else {
            return;
        };
        let world = [abs_pos.x as f64, abs_pos.y as f64, abs_pos.z as f64];
        let anchor = self
            .anchor_table
            .nearest_anchor(self.sector_id, world)
            .unwrap_or(dawn_core::AnchorId(0));
        let offset = match self.anchor_table.abs(anchor) {
            Some(a) => Position::new(
                (world[0] - a[0]) as f32,
                (world[1] - a[1]) as f32,
                (world[2] - a[2]) as f32,
            ),
            None => {
                super::debug_assert_missing_anchor(anchor, "set_spawn_anchor");
                abs_pos
            }
        };
        self.world.set_ship_anchor(entity, anchor);
        if let Ok(mut p) = self.world.inner_mut().get::<&mut PositionComp>(entity) {
            p.0 = offset;
        }
    }

    /// Test-only: re-anchor a ship from an absolute f64 point directly,
    /// bypassing the f32 `Position` round trip `set_spawn_anchor` takes.
    ///
    /// Real callers always go through `PositionComp` (f32) — that's the whole
    /// method-B contract (ADR-0029 §1). But constructing an absolute test
    /// fixture "near gate G" or "at body B" via f32 arithmetic at true-AU
    /// magnitude is itself lossy in ways that don't reflect any real bug: a
    /// single cast loses ~tens of km of ulp, and subtracting a small offset
    /// (e.g. "12,000 m short of the gate") from an AU-scale f32 number can
    /// vanish entirely to catastrophic cancellation. This sidesteps that by
    /// doing the anchor/offset split directly in f64, the same way production
    /// code's f64 paths (warp arrival, AnchorTable) already do.
    #[cfg(test)]
    pub(crate) fn set_spawn_anchor_abs(&mut self, ship_id: ShipId, world: [f64; 3]) {
        let Some(&entity) = self.ships.index.get(&ship_id) else {
            return;
        };
        let anchor = self
            .anchor_table
            .nearest_anchor(self.sector_id, world)
            .unwrap_or(dawn_core::AnchorId(0));
        let offset = match self.anchor_table.abs(anchor) {
            Some(a) => Position::new(
                (world[0] - a[0]) as f32,
                (world[1] - a[1]) as f32,
                (world[2] - a[2]) as f32,
            ),
            None => Position::new(world[0] as f32, world[1] as f32, world[2] as f32),
        };
        self.world.set_ship_anchor(entity, anchor);
        if let Ok(mut p) = self.world.inner_mut().get::<&mut PositionComp>(entity) {
            p.0 = offset;
        }
    }

    /// Reconstruct a Ship's full ECS state (stats, hull, capacitor, fitting)
    /// from a [`ShipSnapshot`]. Used by `from_snapshot` (node restart) and
    /// `import_transit` (Sector Transit, ADR-0014). Does NOT append any event.
    pub(super) fn restore_ship_from_snapshot(&mut self, ship: &ShipSnapshot) {
        use dawn_ecs::components::{FittingComp, TackledComp};
        self.insert_to_world(ship.ship_id, ship.position, ship.velocity);
        self.ships.type_ids.insert(ship.ship_id, ship.ship_type_id);
        // Restore the coordinate anchor (ADR-0029): insert_to_world defaults to
        // the Sector-origin anchor, but a rebased ship's `position` offset is
        // relative to its saved anchor, so restore that to keep absolute position.
        if let Some(&entity) = self.ships.index.get(&ship.ship_id) {
            self.world.set_ship_anchor(entity, ship.anchor);
        }

        let base = self
            .ship_type_registry
            .get(&ship.ship_type_id)
            .map(|def| ShipStatsComp::from_base(&def.base_stats))
            .unwrap_or(ShipStatsComp::NPC);
        self.base_stats.insert(ship.ship_id, base);

        if let Some(&entity) = self.ships.index.get(&ship.ship_id) {
            self.world.set_ship_stats(entity, base);

            let fitting = FittingComp::from_snapshot(&ship.fitting, &self.module_registry);
            let _ = self.world.inner_mut().insert_one(entity, fitting);

            // apply_fitting recomputes ShipStatsComp and rescales HullComp;
            // restore the exact HP layers from the snapshot afterwards.
            apply_fitting(&mut self.world, ship.ship_id, base);
            if let Ok(mut hull) = self.world.inner_mut().get::<&mut HullComp>(entity) {
                hull.current_shield = ship.current_shield;
                hull.current_armor = ship.current_armor;
                hull.current_hull = ship.current_hull;
                hull.is_destroyed = ship.is_destroyed;
            }

            if let Some(cap) = ship.capacitor {
                let _ = self
                    .world
                    .inner_mut()
                    .insert_one(entity, CapacitorComp { current: cap });
            }

            if !ship.tackled_by.is_empty() {
                let _ = self.world.inner_mut().insert_one(
                    entity,
                    TackledComp {
                        tacklers: ship.tackled_by.clone(),
                    },
                );
            }

            // Inventory (ADR-0032): restore exactly what was persisted,
            // regardless of ship type -- post-spawn Fit/Unfit could have
            // emptied or refilled it differently from the deterministic seed.
            let _ = self.world.inner_mut().insert_one(
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
        if let Some(&entity) = self.ships.index.get(&ship_id) {
            self.base_stats.insert(ship_id, ShipStatsComp::PLAYER);
            self.world.set_ship_stats(entity, ShipStatsComp::PLAYER);
            if let Ok(mut hull) = self.world.inner_mut().get::<&mut HullComp>(entity) {
                *hull = HullComp::new(
                    ShipStatsComp::PLAYER.max_shield,
                    ShipStatsComp::PLAYER.max_armor,
                    ShipStatsComp::PLAYER.max_hull,
                );
            }
            let base = ShipStatsComp::PLAYER;
            apply_fitting(&mut self.world, ship_id, base);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{DomainEvent, NodeId, Position, SectorBounds, SectorId, Tick};
    use dawn_ecs::components::WarpPhase;

    fn node_with_modules() -> SimulationNode {
        use crate::{modules, ship_types};
        let mut node = SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        );
        for def in modules::all_modules() {
            node.register_module(def);
        }
        for def in ship_types::all_ship_types() {
            node.register_ship_type(def);
        }
        node
    }

    #[test]
    fn spawned_ship_is_anchored_on_the_nearest_body() {
        // ADR-0029 review #1: a ship anchors on the nearest celestial body.
        // A spawn near the star anchors on Helios (id 0); a spawn at Forge's
        // position (160,000, 0, 100,000 in compressed units) anchors on Forge
        // (id 1) with a ~zero offset, keeping the offset small (the method-B
        // invariant). Distances stay correct via the absolute accessors.
        let mut node = node_with_modules();
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
        // (set_spawn_anchor_abs) -- routing forge_abs through a single f32
        // `Position` cast first would lose ~tens of km of ulp at true AU
        // (not a bug; f32 simply can't hold an AU-scale absolute coordinate
        // exactly), which isn't what this test is checking.
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
        let mut node = node_with_modules();
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
        let mut node = node_with_modules();
        let ship = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(160_000.0, 0.0, 0.0),
            dawn_core::Velocity::ZERO,
        );
        // Rebase onto Forge (AnchorId 1), preserving absolute position.
        let world = node.ship_absolute(ship).unwrap();
        let forge = node.anchor_table().abs(AnchorId(1)).unwrap();
        let off = Position::new(
            (world[0] - forge[0]) as f32,
            (world[1] - forge[1]) as f32,
            (world[2] - forge[2]) as f32,
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

        let node2 = SimulationNode::restore_from(
            InMemoryEventStore::new(),
            &snap,
            &crate::modules::all_modules(),
            &crate::ship_types::all_ship_types(),
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
        let mut node = node_with_modules();
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
    fn bot_starts_aligning_when_hp_drops_below_50_percent() {
        use dawn_core::events::DamageTaken;

        let mut node = node_with_modules();

        let bot_pos = Position::new(1200.0, 0.0, 0.0);
        let (_, bot_ship_id) = node.spawn_bot_ship(bot_pos);

        let player_id = node.next_player_id();
        let _ = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);

        // Magpie max HP: shield=200, armor=120, hull=100, total=420.
        // Deal 215 → shield=0, armor=105, hull=100 → 205/420 ≈ 48.8% < 50%.
        node.apply_event_pub(DomainEvent::DamageTaken(DamageTaken {
            ship_id: bot_ship_id,
            damage: 215.0,
            current_shield: 0.0,
            current_armor: 105.0,
            current_hull: 100.0,
            tick: Tick(1),
        }));

        node.tick();

        assert!(
            matches!(node.warp_phase(bot_ship_id), Some(WarpPhase::Aligning)),
            "bot should be in WarpPhase::Aligning after hp drops below 50%"
        );
    }

    #[test]
    fn tackled_bot_cannot_warp_but_keeps_fighting() {
        use crate::modules::MODULE_FOLD_DISRUPTOR;
        use dawn_core::{
            events::DamageTaken, ActivateModuleCommand, FitModuleCommand, LockOnCommand, SlotKind,
        };

        let mut node = node_with_modules();

        let bot_pos = Position::new(1000.0, 0.0, 0.0);
        let (_, bot_ship_id) = node.spawn_bot_ship(bot_pos);

        let player_id = node.next_player_id();
        let player_ship_id = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);

        node.fit_module(FitModuleCommand {
            ship_id: player_ship_id,
            slot: SlotKind::Mid,
            module_id: MODULE_FOLD_DISRUPTOR,
        });

        let lock_cmd = LockOnCommand {
            ship_id: player_ship_id,
            target_id: bot_ship_id,
        };
        // Tackle activation requires a Locked target (ADR-0035 Q4) — tick
        // until the lock completes before activating.
        for _ in 0..5 {
            node.tick_with_lock_commands(std::slice::from_ref(&lock_cmd));
        }

        node.activate_module_owned(
            player_id,
            ActivateModuleCommand {
                ship_id: player_ship_id,
                module_id: MODULE_FOLD_DISRUPTOR,
                slot: SlotKind::Mid,
                target_ship_id: Some(bot_ship_id),
            },
        );
        for _ in 0..5 {
            node.tick_with_lock_commands(std::slice::from_ref(&lock_cmd));
        }

        let gate_id = node.sector_map.gates.keys().next().copied().unwrap();
        assert!(
            !node.can_propose_warp(bot_ship_id, dawn_core::WarpTarget::Gate(gate_id)),
            "bot should be tackled"
        );

        node.apply_event_pub(DomainEvent::DamageTaken(DamageTaken {
            ship_id: bot_ship_id,
            damage: 215.0,
            current_shield: 0.0,
            current_armor: 105.0,
            current_hull: 100.0,
            tick: Tick(10),
        }));

        node.tick_with_lock_commands(std::slice::from_ref(&lock_cmd));

        assert!(
            node.warp_phase(bot_ship_id).is_none(),
            "tackled bot should not enter warp"
        );
    }
}
