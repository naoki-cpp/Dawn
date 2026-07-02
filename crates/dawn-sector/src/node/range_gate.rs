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
use dawn_ecs::components::{FittingComp, PositionComp, ShipStatsComp};
use dawn_ecs::Entity;
use dawn_event_store::store::EventStore;

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

impl<S: EventStore> SimulationNode<S> {
    /// Effective range for a targeted `ModuleKind`, read from `entity`'s
    /// fitted `ShipStatsComp` (weapon: range+falloff, tackle: tackle_range).
    /// `None` for kinds that are not range-gated (ADR-0035).
    pub(super) fn effective_range_for_kind(
        &self,
        entity: Entity,
        kind: dawn_core::fitting::ModuleKind,
    ) -> Option<f32> {
        let stats = self.world.inner().get::<&ShipStatsComp>(entity).ok()?;
        match kind {
            dawn_core::fitting::ModuleKind::Weapon => {
                Some(stats.weapon_range + stats.weapon_falloff)
            }
            dawn_core::fitting::ModuleKind::Tackle => Some(stats.tackle_range),
            _ => None,
        }
    }

    /// Whether `target_id` is currently within `range` of `entity`, in
    /// Sector-frame absolute coordinates. f64 absolutes (ADR-0029): anchor
    /// offsets can sit at true-AU scale, so the anchor+offset sum must stay
    /// f64 until *after* the two absolutes are subtracted — casting each one
    /// to f32 first (as the plain `entity_absolute` helper does) would round
    /// away the offset entirely and report a bogus distance even for ships
    /// sitting right next to each other.
    pub(super) fn is_target_within_range(
        &self,
        entity: Entity,
        target_id: ShipId,
        range: f32,
    ) -> bool {
        let Ok(pos) = self.world.inner().get::<&PositionComp>(entity) else {
            return false;
        };
        let self_abs = self.entity_absolute_f64(entity, pos.0);
        let Some(&target_entity) = self.ships.index.get(&target_id) else {
            return false;
        };
        let Ok(tp) = self.world.inner().get::<&PositionComp>(target_entity) else {
            return false;
        };
        let target_abs = self.entity_absolute_f64(target_entity, tp.0);
        let dx = (target_abs[0] - self_abs[0]) as f32;
        let dy = (target_abs[1] - self_abs[1]) as f32;
        let dz = (target_abs[2] - self_abs[2]) as f32;
        (dx * dx + dy * dy + dz * dz).sqrt() <= range
    }

    /// Range Gate System — Step 5.5 (ADR-0035).
    pub fn process_range_gate(&mut self, tick: Tick) -> Vec<DomainEvent> {
        // ── 1. Snapshot Active, targeted slots + their ship's absolute position ──
        let mut candidates: Vec<TargetedSlot> = Vec::new();
        for (&ship_id, &entity) in self.ships.index.iter() {
            let Ok(stats) = self.world.inner().get::<&ShipStatsComp>(entity) else {
                continue;
            };
            let weapon_effective_range = stats.weapon_range + stats.weapon_falloff;
            let tackle_range = stats.tackle_range;
            let Ok(fitting) = self.world.inner().get::<&FittingComp>(entity) else {
                continue;
            };
            for (flat_idx, slot) in fitting.iter_slots().enumerate() {
                if !slot.is_active {
                    continue;
                }
                let Some(target) = slot.target_ship_id else {
                    continue;
                };
                let range = match slot.def.kind {
                    dawn_core::fitting::ModuleKind::Weapon => weapon_effective_range,
                    dawn_core::fitting::ModuleKind::Tackle => tackle_range,
                    _ => continue, // Not a range-gated kind.
                };
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
            if self.is_target_within_range(c.entity, c.target, c.range) {
                continue;
            }

            if let Ok(mut fitting) = self.world.inner_mut().get::<&mut FittingComp>(c.entity) {
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
            if let Some(&base) = self.base_stats.get(&ship_id) {
                dawn_ecs::systems::apply_fitting(&mut self.world, ship_id, base);
            }
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
                ActivateModuleCommand {
                    ship_id: ship_a,
                    module_id: MODULE_RAILGUN_SMALL,
                    slot: SlotKind::High,
                    target_ship_id: Some(ship_b),
                }
            ),
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

        assert!(
            !node.activate_module_owned(
                player_id,
                ActivateModuleCommand {
                    ship_id: ship_a,
                    module_id: MODULE_RAILGUN_SMALL,
                    slot: SlotKind::High,
                    target_ship_id: Some(ship_b),
                }
            ),
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

        assert!(node.activate_module_owned(
            player_id,
            ActivateModuleCommand {
                ship_id: ship_a,
                module_id: MODULE_FOLD_DISRUPTOR,
                slot: SlotKind::Mid,
                target_ship_id: Some(ship_b),
            }
        ));

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

        assert!(
            !node.activate_module_owned(
                player_id,
                ActivateModuleCommand {
                    ship_id: ship_a,
                    module_id: MODULE_RAILGUN_SMALL,
                    slot: SlotKind::High,
                    target_ship_id: Some(ship_b),
                }
            ),
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

        assert!(
            !node.activate_module_owned(
                player_id,
                ActivateModuleCommand {
                    ship_id: ship_a,
                    module_id: MODULE_RAILGUN_SMALL,
                    slot: SlotKind::High,
                    target_ship_id: None,
                }
            ),
            "Weapon requires a target (ModuleKind::requires_target) — activation without one must be rejected"
        );
    }
}
