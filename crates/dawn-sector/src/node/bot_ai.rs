//! Bot AI — the decision loop for `IsBotComp` ships (extracted from
//! `spawner_logic.rs`, `/improve-codebase-architecture` candidate 3,
//! 2026-07-03). Spawning a bot (`spawn_bot_ship`) is spawn mechanics and
//! stays in `spawner_logic.rs`; deciding what a bot does each tick is a
//! separate concern with its own test fixtures, isolated here.

use dawn_core::{PlayerId, Position, ShipId};
use dawn_ecs::components::{
    FittingComp, HullComp, IsBotComp, IsNpcComp, LockComp, PositionComp, ShipStatsComp, WarpComp,
};

use super::SimulationNode;

impl SimulationNode {
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
        for (&ship_id, &entity) in &self.simulation.ships.index {
            if self.simulation.world.get::<IsBotComp>(entity).is_none() {
                continue;
            }
            if self
                .simulation
                .world
                .get::<HullComp>(entity)
                .is_some_and(|hull| hull.is_destroyed())
            {
                continue;
            }
            let Some(&player_id) = self.players.owners.get(&ship_id) else {
                continue;
            };
            let Some(pos) = self.simulation.world.get::<PositionComp>(entity) else {
                continue;
            };
            let Some(stats) = self.simulation.world.get::<ShipStatsComp>(entity) else {
                continue;
            };
            let Some(lock) = self.simulation.world.get::<LockComp>(entity) else {
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
                .simulation
                .world
                .get::<FittingComp>(entity)
                .map(|f| {
                    f.iter_slots()
                        .filter(|s| s.def.kind == dawn_core::fitting::ModuleKind::Weapon)
                        .map(|s| (s.def.id, s.def.slot))
                        .collect()
                })
                .unwrap_or_default();
            let hp_fraction = if let Some(hull) = self.simulation.world.get::<HullComp>(entity) {
                let max_hp = stats.max_shield + stats.max_armor + stats.max_hull;
                let cur_hp = hull.total_hp();
                if max_hp > 0.0 {
                    cur_hp / max_hp
                } else {
                    1.0
                }
            } else {
                1.0
            };
            let is_warping = self.simulation.world.get::<WarpComp>(entity).is_some();
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
        for (&ship_id, &entity) in &self.simulation.ships.index {
            if self.simulation.world.get::<IsBotComp>(entity).is_some() {
                continue;
            }
            if self
                .simulation
                .world
                .get::<HullComp>(entity)
                .is_some_and(|hull| hull.is_destroyed())
            {
                continue;
            }
            if self.simulation.world.get::<IsNpcComp>(entity).is_some() {
                continue;
            }
            if !self.players.owners.contains_key(&ship_id) {
                continue;
            }
            let Some(pos) = self.simulation.world.get::<PositionComp>(entity) else {
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
            .topology
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
                self.simulation
                    .pending_bot_lock_commands
                    .push(dawn_core::LockOnCommand {
                        ship_id: bot.ship_id,
                        target_id: target.ship_id,
                    });
            }

            // Move: approach until within 75% of weapon range, then brake to stop.
            let engage_range = (bot.weapon_range * 0.75).max(500.0);
            if dist > f64::from(engage_range) {
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
                    let _ = self.activate_module_owned(
                        bot.player_id,
                        bot.ship_id,
                        dawn_core::ActivateModuleCommand {
                            module_id: *module_id,
                            slot: *slot,
                            target_ship_id: Some(target.ship_id),
                        },
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{DomainEvent, NodeId, SectorBounds, SectorId, Tick};
    use dawn_ecs::components::WarpPhase;

    fn node_with_modules() -> SimulationNode {
        SimulationNode::new_test(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        )
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

        let _ = node.activate_module_owned(
            player_id,
            player_ship_id,
            ActivateModuleCommand {
                module_id: MODULE_FOLD_DISRUPTOR,
                slot: SlotKind::Mid,
                target_ship_id: Some(bot_ship_id),
            },
        );
        for _ in 0..5 {
            node.tick_with_lock_commands(std::slice::from_ref(&lock_cmd));
        }

        let gate_id = node
            .topology
            .sector_map
            .gates
            .keys()
            .next()
            .copied()
            .unwrap();
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
