use std::collections::HashMap;

use dawn_core::events::{TackleApplied, TackleReleased};
use dawn_core::fitting::ModuleKind;
use dawn_core::{DomainEvent, ShipId, Tick};
use dawn_ecs::components::{FittingComp, LockComp, ShipStatsComp, TackledComp};
use dawn_ecs::Entity;

use super::SimulationNode;

impl SimulationNode {
    /// Tackle System — Step 4.5 (after Capacitor, before Lock). ADR-0024.
    ///
    /// Computes the desired tackle state from active Fold Disruptors + locked
    /// targets in range, diffs against the current `TackledComp` state, emits
    /// `TackleApplied`/`TackleReleased` events for changes, and updates the ECS.
    pub(crate) fn process_tackle(&mut self, tick: Tick) -> Vec<DomainEvent> {
        // Collect ships with at least one active Tackle module.
        let tacklers: Vec<(ShipId, f32, Vec<ShipId>)> = self
            .simulation
            .ships
            .index
            .iter()
            .filter_map(|(&ship_id, &entity)| {
                let stats = self.simulation.world.get::<ShipStatsComp>(entity)?;
                if stats.tackle_range <= 0.0 {
                    return None;
                }
                let fitting = self.simulation.world.get::<FittingComp>(entity)?;
                if !fitting.has_active_module_of_kind(ModuleKind::Tackle) {
                    return None;
                }
                let lock = self.simulation.world.get::<LockComp>(entity)?;
                let locked: Vec<ShipId> = lock.locked_targets().collect();
                if locked.is_empty() {
                    return None;
                }
                Some((ship_id, stats.tackle_range, locked))
            })
            .collect();

        // desired[target] = Vec of tacklers currently in range and holding a lock.
        // ship_distance() composes both ships' anchors in f64 before
        // subtracting (ADR-0029) — the precision-safe pattern this used to
        // hand-roll itself, matching combat.rs's hit-chance distance.
        let mut desired: HashMap<ShipId, Vec<ShipId>> = HashMap::new();
        for (tackler_id, range, locked) in &tacklers {
            for &target_id in locked {
                if self
                    .ship_distance(*tackler_id, target_id)
                    .is_some_and(|d| d <= *range as f64)
                {
                    desired.entry(target_id).or_default().push(*tackler_id);
                }
            }
        }

        // Snapshot current tackle state — single ECS scan.
        let current: Vec<(ShipId, Entity, Vec<ShipId>)> = self
            .simulation
            .ships
            .index
            .iter()
            .filter_map(|(&sid, &entity)| {
                let t = self.simulation.world.get::<TackledComp>(entity)?;
                Some((sid, entity, t.tacklers.clone()))
            })
            .collect();
        let current_ids: std::collections::HashSet<ShipId> =
            current.iter().map(|(s, _, _)| *s).collect();

        let mut events: Vec<DomainEvent> = Vec::new();

        // Update existing TackledComps: emit diffs, then write new list.
        for (target_id, entity, old_tacklers) in current {
            let new_tacklers = desired.get(&target_id).cloned().unwrap_or_default();

            for &tid in &old_tacklers {
                if !new_tacklers.contains(&tid) {
                    events.push(DomainEvent::TackleReleased(TackleReleased {
                        ship_id: target_id,
                        by: tid,
                        tick,
                    }));
                }
            }
            for &tid in &new_tacklers {
                if !old_tacklers.contains(&tid) {
                    events.push(DomainEvent::TackleApplied(TackleApplied {
                        ship_id: target_id,
                        by: tid,
                        tick,
                    }));
                }
            }

            if new_tacklers.is_empty() {
                let _ = self.simulation.world.remove_one::<TackledComp>(entity);
            } else {
                if let Some(mut tackled) = self.simulation.world.get_mut::<TackledComp>(entity) {
                    tackled.tacklers = new_tacklers;
                }
            }
        }

        // Insert TackledComp for ships newly entering tackled state.
        for (&target_id, new_tacklers) in &desired {
            if current_ids.contains(&target_id) {
                continue;
            }
            for &tid in new_tacklers {
                events.push(DomainEvent::TackleApplied(TackleApplied {
                    ship_id: target_id,
                    by: tid,
                    tick,
                }));
            }
            if let Some(&entity) = self.simulation.ships.index.get(&target_id) {
                let _ = self.simulation.world.insert_one(
                    entity,
                    TackledComp {
                        tacklers: new_tacklers.clone(),
                    },
                );
            }
        }

        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{NodeId, Position, SectorBounds, SectorId, ShipId};

    fn node_with_modules() -> SimulationNode {
        SimulationNode::new_test(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        )
    }

    fn fit_fold_disruptor(node: &mut SimulationNode, ship_id: ShipId) {
        use crate::modules::MODULE_FOLD_DISRUPTOR;
        use dawn_core::{FitModuleCommand, SlotKind};
        node.fit_module(FitModuleCommand {
            ship_id,
            slot: SlotKind::Mid,
            module_id: MODULE_FOLD_DISRUPTOR,
        });
    }

    #[test]
    fn tackled_ship_cannot_warp() {
        use crate::modules::MODULE_FOLD_DISRUPTOR;
        use dawn_core::{ActivateModuleCommand, LockOnCommand, SlotKind};

        let mut node = node_with_modules();

        let ship_a = {
            let id = node.next_player_id();
            node.spawn_player_ship_at_pub(id, Position::new(0.0, 0.0, 0.0))
        };
        let ship_b = {
            let id = node.next_player_id();
            node.spawn_player_ship_at_pub(id, Position::new(1000.0, 0.0, 0.0))
        };

        fit_fold_disruptor(&mut node, ship_a);
        let owner_a = node.players.owners.get(&ship_a).copied().unwrap();

        let lock_cmd = LockOnCommand {
            ship_id: ship_a,
            target_id: ship_b,
        };
        // Tackle activation requires a Locked target (ADR-0035 Q4) — tick
        // until the lock completes before activating.
        for _ in 0..5 {
            node.tick_with_lock_commands(std::slice::from_ref(&lock_cmd));
        }

        let _ = node.activate_module_owned(
            owner_a,
            ship_a,
            ActivateModuleCommand {
                module_id: MODULE_FOLD_DISRUPTOR,
                slot: SlotKind::Mid,
                target_ship_id: Some(ship_b),
            },
        );

        for _ in 0..10 {
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
            !node.can_propose_warp(ship_b, dawn_core::WarpTarget::Gate(gate_id)),
            "tackled ship must not be allowed to warp"
        );
        assert!(
            !node.can_propose_jump(ship_b, gate_id),
            "tackled ship must not be allowed to jump"
        );
    }

    #[test]
    fn tackle_releases_when_tackler_dies() {
        use crate::modules::MODULE_FOLD_DISRUPTOR;
        use dawn_core::{ActivateModuleCommand, DomainEvent, LockOnCommand, SlotKind};

        let mut node = node_with_modules();

        let ship_a = {
            let id = node.next_player_id();
            node.spawn_player_ship_at_pub(id, Position::new(0.0, 0.0, 0.0))
        };
        let ship_b = {
            let id = node.next_player_id();
            node.spawn_player_ship_at_pub(id, Position::new(1000.0, 0.0, 0.0))
        };

        fit_fold_disruptor(&mut node, ship_a);
        let owner_a = node.players.owners.get(&ship_a).copied().unwrap();

        let lock_cmd = LockOnCommand {
            ship_id: ship_a,
            target_id: ship_b,
        };
        // Tackle activation requires a Locked target (ADR-0035 Q4) — tick
        // until the lock completes before activating.
        for _ in 0..5 {
            node.tick_with_lock_commands(std::slice::from_ref(&lock_cmd));
        }

        let _ = node.activate_module_owned(
            owner_a,
            ship_a,
            ActivateModuleCommand {
                module_id: MODULE_FOLD_DISRUPTOR,
                slot: SlotKind::Mid,
                target_ship_id: Some(ship_b),
            },
        );

        for _ in 0..10 {
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
            !node.can_propose_warp(ship_b, dawn_core::WarpTarget::Gate(gate_id)),
            "should be tackled first"
        );

        node.apply_event_pub(DomainEvent::ShipDestroyed(
            dawn_core::events::ShipDestroyed {
                ship_id: ship_a,
                killer_id: ship_b,
                tick: node.current_tick(),
            },
        ));
        node.tick_with_lock_commands(&[]);

        assert!(
            node.can_propose_warp(ship_b, dawn_core::WarpTarget::Gate(gate_id)),
            "tackle should release when the tackler ship is destroyed"
        );
    }

    #[test]
    fn tackle_snapshot_round_trip_preserves_tackle_state() {
        use crate::modules::MODULE_FOLD_DISRUPTOR;
        use dawn_core::{ActivateModuleCommand, LockOnCommand, SlotKind};

        let mut node = node_with_modules();

        let ship_a = {
            let id = node.next_player_id();
            node.spawn_player_ship_at_pub(id, Position::new(0.0, 0.0, 0.0))
        };
        let ship_b = {
            let id = node.next_player_id();
            node.spawn_player_ship_at_pub(id, Position::new(1000.0, 0.0, 0.0))
        };

        fit_fold_disruptor(&mut node, ship_a);
        let owner_a = node.players.owners.get(&ship_a).copied().unwrap();

        let lock_cmd = LockOnCommand {
            ship_id: ship_a,
            target_id: ship_b,
        };
        // Tackle activation requires a Locked target (ADR-0035 Q4) — tick
        // until the lock completes before activating.
        for _ in 0..5 {
            node.tick_with_lock_commands(std::slice::from_ref(&lock_cmd));
        }

        let _ = node.activate_module_owned(
            owner_a,
            ship_a,
            ActivateModuleCommand {
                module_id: MODULE_FOLD_DISRUPTOR,
                slot: SlotKind::Mid,
                target_ship_id: Some(ship_b),
            },
        );

        for _ in 0..10 {
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
            !node.can_propose_warp(ship_b, dawn_core::WarpTarget::Gate(gate_id)),
            "should be tackled before snapshot"
        );

        let snapshot = node.take_snapshot();
        let ship_b_snap = snapshot
            .ships
            .iter()
            .find(|s| s.snapshot.ship_id == ship_b)
            .unwrap();
        assert!(
            ship_b_snap.snapshot.tackled_by.contains(&ship_a),
            "snapshot must record the tackler"
        );

        let store2 = dawn_storage::InMemoryEventStore::new();
        let modules: Vec<_> = crate::game_data::test_catalog().modules().to_vec();
        let ship_types: Vec<_> = crate::game_data::test_catalog().ship_types().to_vec();
        let node2 = SimulationNode::restore_from_test(
            store2,
            &snapshot,
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
            &modules,
            &ship_types,
        );
        assert!(
            !node2.can_propose_warp(ship_b, dawn_core::WarpTarget::Gate(gate_id)),
            "restored node must still prevent tackled ship from warping"
        );
    }
}
