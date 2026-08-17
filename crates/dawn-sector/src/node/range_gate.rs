//! Range Gate System — Step 5.5 (ADR-0035).
//!
//! Forces OFF any Active, targeted module (Weapon, Tackle) whose
//! `FittedSlot.target_ship_id` has drifted out of the module's effective
//! range. Unifies what used to be three inconsistent behaviours: Weapon
//! (guaranteed miss beyond range+falloff, module stayed ON), Tackle (effect
//! silently dropped each tick, module stayed ON), and the as-yet-unbuilt
//! Logistics (which needs this from day one since its whole value is
//! choosing who to keep in range).
//!
//! Runs after Lock System (Step 5) so Range Gate sees this tick's freshest
//! lock state, and before Combat/Tackle-effect/Repair (Step 6+) so those
//! systems only ever see modules that are still in range.

use dawn_core::events::ModuleDeactivated;
use dawn_core::{DomainEvent, ShipId, Tick};
use dawn_ecs::components::{FittingComp, ShipStatsComp};
use dawn_ecs::Entity;

use super::SimulationNode;

/// One Active, targeted slot that needs a range check this tick.
struct TargetedSlot {
    entity: Entity,
    ship_id: ShipId,
    flat_idx: usize,
    module_id: dawn_core::fitting::ModuleId,
    slot_kind: dawn_core::fitting::SlotKind,
    target: ShipId,
    /// Effective range for this module kind (weapon: range+falloff, tackle: tackle_range).
    range: f32,
}

/// Effective range for a `RangeGateKind` family, read from `stats`
/// (weapon: range+falloff, tackle: tackle_range, remote repair: repair_range).
/// Single source of truth for the stat lookup so per-ship-batch callers
/// (`process_range_gate`) and per-slot callers (`effective_range_for_kind`)
/// can't drift apart on which stat backs which kind.
fn effective_range_from_stats(
    kind: dawn_core::fitting::RangeGateKind,
    stats: &ShipStatsComp,
) -> f32 {
    match kind {
        dawn_core::fitting::RangeGateKind::Weapon => stats.weapon_range + stats.weapon_falloff,
        dawn_core::fitting::RangeGateKind::Tackle => stats.tackle_range,
        dawn_core::fitting::RangeGateKind::RemoteRepair => stats.repair_range,
    }
}

impl SimulationNode {
    /// Effective range for a targeted `ModuleKind`, read from `entity`'s
    /// fitted `ShipStatsComp`. `None` for kinds that are not range-gated
    /// (ADR-0035/0036).
    pub(super) fn effective_range_for_kind(
        &self,
        entity: Entity,
        kind: dawn_core::fitting::ModuleKind,
    ) -> Option<f32> {
        let stats = self.simulation.world.get::<ShipStatsComp>(entity)?;
        Some(effective_range_from_stats(kind.range_gate_kind()?, &stats))
    }

    /// Whether `target_id` is currently within `range` of `ship_id`. Delegates
    /// to `ship_distance()`, which already composes both ships' anchors in
    /// f64 before subtracting (ADR-0029) — the precision-safe pattern this
    /// function used to hand-roll itself.
    pub(super) fn is_target_within_range(
        &self,
        ship_id: ShipId,
        target_id: ShipId,
        range: f32,
    ) -> bool {
        self.ship_distance(ship_id, target_id)
            .is_some_and(|d| d <= range as f64)
    }

    /// Range Gate System — Step 5.5 (ADR-0035).
    pub(crate) fn process_range_gate(&mut self, tick: Tick) -> Vec<DomainEvent> {
        // ── 1. Snapshot Active, targeted slots + their ship's absolute position ──
        let mut candidates: Vec<TargetedSlot> = Vec::new();
        for (&ship_id, &entity) in self.simulation.ships.index.iter() {
            let Some(stats) = self.simulation.world.get::<ShipStatsComp>(entity) else {
                continue;
            };
            let Some(fitting) = self.simulation.world.get::<FittingComp>(entity) else {
                continue;
            };
            for (flat_idx, slot) in fitting.iter_slots().enumerate() {
                if !slot.is_active {
                    continue;
                }
                let Some(target) = slot.target_ship_id else {
                    continue;
                };
                let Some(gate_kind) = slot.def.kind.range_gate_kind() else {
                    continue; // Not a range-gated kind.
                };
                let range = effective_range_from_stats(gate_kind, &stats);
                candidates.push(TargetedSlot {
                    entity,
                    ship_id,
                    flat_idx,
                    module_id: slot.def.id,
                    slot_kind: slot.def.slot,
                    target,
                    range,
                });
            }
        }
        if candidates.is_empty() {
            return Vec::new();
        }

        // ── 2. Resolve absolute positions and find out-of-range targets ──────────
        let mut events: Vec<DomainEvent> = Vec::new();
        let mut refitted: Vec<ShipId> = Vec::new();
        for c in candidates {
            if self.is_target_within_range(c.ship_id, c.target, c.range) {
                continue;
            }

            if let Some(mut fitting) = self.simulation.world.get_mut::<FittingComp>(c.entity) {
                if let Some(slot) = fitting.slot_at_flat_mut(c.flat_idx) {
                    if slot.is_active {
                        slot.force_off();
                        events.push(DomainEvent::ModuleDeactivated(ModuleDeactivated {
                            ship_id: c.ship_id,
                            module_id: c.module_id,
                            slot: c.slot_kind,
                            forced_reason: Some(
                                dawn_core::events::ModuleDeactivationReason::OutOfRange,
                            ),
                            tick,
                        }));
                        refitted.push(c.ship_id);
                    }
                }
            }
        }

        for ship_id in refitted {
            self.reapply_fitting(ship_id);
        }

        events
    }
}

#[cfg(test)]
mod tests {
    use dawn_core::{
        ActivateModuleCommand, DomainEvent, FitModuleCommand, LockOnCommand, NodeId, Position,
        SectorBounds, SectorId, ShipId, SlotKind,
    };

    use super::SimulationNode;

    fn node_with_modules() -> SimulationNode {
        SimulationNode::new_test(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        )
    }

    /// Locks `locker` onto `target` by ticking until `TargetLocked` fires.
    fn lock_until_locked(node: &mut SimulationNode, locker: ShipId, target: ShipId) {
        let lock_cmd = LockOnCommand {
            ship_id: locker,
            target_id: target,
        };
        for _ in 0..5 {
            node.tick_with_lock_commands(std::slice::from_ref(&lock_cmd));
        }
    }

    #[test]
    fn weapon_is_force_deactivated_once_its_locked_target_drifts_beyond_range_plus_falloff() {
        use crate::modules::MODULE_RAILGUN_SMALL;

        let mut node = node_with_modules();
        let player_id = node.next_player_id();
        let ship_a = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);
        // Small Railgun: weapon_range 3000 + falloff 2000 = 5000 effective range.
        // Spawn well within range so activation itself succeeds.
        let ship_b_owner = node.next_player_id();
        let ship_b = node.spawn_player_ship_at_pub(ship_b_owner, Position::new(500.0, 0.0, 0.0));

        node.fit_module(FitModuleCommand {
            ship_id: ship_a,
            slot: SlotKind::High,
            module_id: MODULE_RAILGUN_SMALL,
        });
        lock_until_locked(&mut node, ship_a, ship_b);

        assert!(
            node.activate_module_owned(
                player_id,
                ship_a,
                ActivateModuleCommand {
                    module_id: MODULE_RAILGUN_SMALL,
                    slot: SlotKind::High,
                    target_ship_id: Some(ship_b),
                }
            )
            .is_ok(),
            "activation against a Locked, in-range target must succeed"
        );

        // Drift the target beyond effective range (5000) after activation.
        node.set_spawn_anchor_abs(ship_b, [50_000.0, 0.0, 0.0]);

        let result = node.tick_with_lock_commands(&[]);
        assert!(
            result
                .events
                .iter()
                .any(|e| matches!(e, DomainEvent::ModuleDeactivated(d) if d.ship_id == ship_a)),
            "Range Gate must force the weapon OFF once the target is beyond range+falloff"
        );

        let stats = node.get_ship_stats(ship_a).unwrap();
        assert_eq!(
            stats.weapon_damage, 0.0,
            "deactivated weapon must no longer contribute to ShipStatsComp"
        );
    }

    #[test]
    fn weapon_activation_is_rejected_outright_against_a_locked_but_out_of_range_target() {
        use crate::modules::MODULE_RAILGUN_SMALL;

        let mut node = node_with_modules();
        let player_id = node.next_player_id();
        let ship_a = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);
        // Small Railgun: weapon_range 3000 + falloff 2000 = 5000 effective range.
        let ship_b_owner = node.next_player_id();
        let ship_b = node.spawn_player_ship_at_pub(ship_b_owner, Position::new(10_000.0, 0.0, 0.0));

        node.fit_module(FitModuleCommand {
            ship_id: ship_a,
            slot: SlotKind::High,
            module_id: MODULE_RAILGUN_SMALL,
        });
        lock_until_locked(&mut node, ship_a, ship_b);

        assert_eq!(
            node.activate_module_owned(
                player_id,
                ship_a,
                ActivateModuleCommand {
                    module_id: MODULE_RAILGUN_SMALL,
                    slot: SlotKind::High,
                    target_ship_id: Some(ship_b),
                }
            ),
            Err(super::super::ModuleActivationRejection::OutOfRange),
            "activation must be rejected outright when the Locked target is already out of \
             range — turning ON then having Range Gate turn it back OFF next tick would flicker"
        );
    }

    #[test]
    fn tackle_is_force_deactivated_once_its_locked_target_drifts_beyond_tackle_range() {
        use crate::modules::MODULE_FOLD_DISRUPTOR;

        let mut node = node_with_modules();
        let player_id = node.next_player_id();
        let ship_a = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);
        // Fold Disruptor: tackle_range 20_000. Spawn well within range so
        // activation itself succeeds.
        let ship_b_owner = node.next_player_id();
        let ship_b = node.spawn_player_ship_at_pub(ship_b_owner, Position::new(1_000.0, 0.0, 0.0));

        node.fit_module(FitModuleCommand {
            ship_id: ship_a,
            slot: SlotKind::Mid,
            module_id: MODULE_FOLD_DISRUPTOR,
        });
        lock_until_locked(&mut node, ship_a, ship_b);

        assert!(node
            .activate_module_owned(
                player_id,
                ship_a,
                ActivateModuleCommand {
                    module_id: MODULE_FOLD_DISRUPTOR,
                    slot: SlotKind::Mid,
                    target_ship_id: Some(ship_b),
                }
            )
            .is_ok());

        // Drift the target beyond tackle_range (20_000) after activation.
        node.set_spawn_anchor_abs(ship_b, [50_000.0, 0.0, 0.0]);

        let result = node.tick_with_lock_commands(&[]);
        assert!(
            result
                .events
                .iter()
                .any(|e| matches!(e, DomainEvent::ModuleDeactivated(d) if d.ship_id == ship_a)),
            "Range Gate must force the disruptor OFF once the target is beyond tackle_range"
        );
    }

    #[test]
    fn activation_is_rejected_against_a_target_that_is_not_locked() {
        use crate::modules::MODULE_RAILGUN_SMALL;

        let mut node = node_with_modules();
        let player_id = node.next_player_id();
        let ship_a = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);
        let ship_b_owner = node.next_player_id();
        let ship_b = node.spawn_player_ship_at_pub(ship_b_owner, Position::new(100.0, 0.0, 0.0));

        node.fit_module(FitModuleCommand {
            ship_id: ship_a,
            slot: SlotKind::High,
            module_id: MODULE_RAILGUN_SMALL,
        });

        assert_eq!(
            node.activate_module_owned(
                player_id,
                ship_a,
                ActivateModuleCommand {
                    module_id: MODULE_RAILGUN_SMALL,
                    slot: SlotKind::High,
                    target_ship_id: Some(ship_b),
                }
            ),
            Err(super::super::ModuleActivationRejection::TargetNotLocked),
            "activation must be rejected when target is not yet a Locked LockComp entry"
        );
    }

    #[test]
    fn activation_is_rejected_without_a_target_for_a_targeted_module_kind() {
        use crate::modules::MODULE_RAILGUN_SMALL;

        let mut node = node_with_modules();
        let player_id = node.next_player_id();
        let ship_a = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);

        node.fit_module(FitModuleCommand {
            ship_id: ship_a,
            slot: SlotKind::High,
            module_id: MODULE_RAILGUN_SMALL,
        });

        assert_eq!(
            node.activate_module_owned(
                player_id,
                ship_a,
                ActivateModuleCommand {
                    module_id: MODULE_RAILGUN_SMALL,
                    slot: SlotKind::High,
                    target_ship_id: None,
                }),
            Err(super::super::ModuleActivationRejection::TargetRequirementMismatch),
            "Weapon requires a target (ModuleKind::requires_target) — activation without one must be rejected"
        );
    }

    #[test]
    fn remote_shield_booster_repairs_a_locked_ally_within_range() {
        // ADR-0036: Remote Repair follows the exact same target/Locked/Range
        // Gate machinery as Weapon/Tackle, just healing instead of damaging.
        use crate::modules::MODULE_SMALL_REMOTE_SHIELD_BOOSTER;
        let mut node = node_with_modules();
        let repairer_id = node.next_player_id();
        let repairer = node.spawn_player_ship_at_pub(repairer_id, Position::ORIGIN);
        let ally_owner = node.next_player_id();
        let ally = node.spawn_player_ship_at_pub(ally_owner, Position::new(1_000.0, 0.0, 0.0));

        node.fit_module(FitModuleCommand {
            ship_id: repairer,
            slot: SlotKind::Mid,
            module_id: MODULE_SMALL_REMOTE_SHIELD_BOOSTER,
        });
        lock_until_locked(&mut node, repairer, ally);

        // Magpie max HP: shield=200, armor=120, hull=100 (matches ADR-0033 tests).
        let hp_before = node.get_ship_hp(ally).unwrap();
        let ally_entity = *node.simulation.ships.index.get(&ally).unwrap();
        node.simulation
            .world
            .get_mut::<dawn_ecs::components::HullComp>(ally_entity)
            .unwrap()
            .set_hp(0.0, 100.0, 100.0);

        assert!(
            node.activate_module_owned(
                repairer_id,
                repairer,
                ActivateModuleCommand {
                    module_id: MODULE_SMALL_REMOTE_SHIELD_BOOSTER,
                    slot: SlotKind::Mid,
                    target_ship_id: Some(ally),
                }
            )
            .is_ok(),
            "activation against a Locked, in-range ally must succeed"
        );

        node.tick_with_lock_commands(&[]);

        let hp_after = node.get_ship_hp(ally).unwrap();
        assert!(
            hp_after > 0.0 && hp_after < hp_before,
            "sanity: ally took damage before repair could offset all of it in one cycle \
             (hp_before={hp_before}, hp_after={hp_after})"
        );
        assert!(
            hp_after > 100.0 + 100.0,
            "ally's shield must have started recovering from the Remote Shield Booster \
             (repair_amount 50, so current_shield > 0 after one cycle; hp_after={hp_after})"
        );
    }

    #[test]
    fn remote_shield_booster_is_force_deactivated_once_its_locked_target_drifts_beyond_repair_range(
    ) {
        use crate::modules::MODULE_SMALL_REMOTE_SHIELD_BOOSTER;

        let mut node = node_with_modules();
        let repairer_id = node.next_player_id();
        let repairer = node.spawn_player_ship_at_pub(repairer_id, Position::ORIGIN);
        let ally_owner = node.next_player_id();
        // repair_range_add = 15,000; spawn well within range so activation succeeds.
        let ally = node.spawn_player_ship_at_pub(ally_owner, Position::new(1_000.0, 0.0, 0.0));

        node.fit_module(FitModuleCommand {
            ship_id: repairer,
            slot: SlotKind::Mid,
            module_id: MODULE_SMALL_REMOTE_SHIELD_BOOSTER,
        });
        lock_until_locked(&mut node, repairer, ally);

        assert!(node
            .activate_module_owned(
                repairer_id,
                repairer,
                ActivateModuleCommand {
                    module_id: MODULE_SMALL_REMOTE_SHIELD_BOOSTER,
                    slot: SlotKind::Mid,
                    target_ship_id: Some(ally),
                }
            )
            .is_ok());

        // Drift the ally beyond repair_range (15,000) after activation.
        node.set_spawn_anchor_abs(ally, [50_000.0, 0.0, 0.0]);

        let result = node.tick_with_lock_commands(&[]);
        assert!(
            result
                .events
                .iter()
                .any(|e| matches!(e, DomainEvent::ModuleDeactivated(d) if d.ship_id == repairer)),
            "Range Gate must force the Remote Shield Booster OFF once the ally drifts beyond repair_range"
        );
    }
}
