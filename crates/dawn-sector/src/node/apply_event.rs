use dawn_core::{DomainEvent, Velocity};
use dawn_ecs::components::{
    FittingComp, HullComp, LockComp, PositionComp, ShipStatsComp, TackledComp, VelocityComp,
};
use dawn_ecs::systems::apply_fitting;
use dawn_event_store::store::EventStore;

use super::SimulationNode;

impl<S: EventStore> SimulationNode<S> {
    /// Apply a single domain event to the ECS World without appending it.
    /// Used during `restore_from` to replay post-snapshot events.
    pub(super) fn apply_event(&mut self, event: &DomainEvent) {
        match event {
            DomainEvent::ShipSpawned(e) => {
                if !self.ships.index.contains_key(&e.ship_id) {
                    self.insert_to_world(e.ship_id, e.initial_position, Velocity::ZERO);
                    // ADR-0029 review #1: anchor on the nearest body (deterministic
                    // — same initial_position reproduces the same anchor on replay).
                    self.set_spawn_anchor(e.ship_id, e.initial_position);
                    // Restore base_stats from ship type registry
                    let base = self
                        .ship_type_registry
                        .get(&e.ship_type_id)
                        .map(|def| ShipStatsComp::from_base(&def.base_stats))
                        .unwrap_or(ShipStatsComp::NPC);
                    self.base_stats.insert(e.ship_id, base);
                    if let Some(&entity) = self.ships.index.get(&e.ship_id) {
                        self.world.set_ship_stats(entity, base);
                        if let Ok(mut hull) = self.world.inner_mut().get::<&mut HullComp>(entity) {
                            *hull = HullComp::new(base.max_shield, base.max_armor, base.max_hull);
                        }
                        // Starting inventory (ADR-0032) is a pure function of
                        // module_registry, loaded identically before replay
                        // starts -- reproduce it here exactly like the live
                        // spawn path does, the same way base/Hull are derived
                        // above rather than event-sourced.
                        if e.ship_type_id == crate::ship_types::SHIP_TYPE_MAGPIE {
                            self.seed_player_inventory(entity);
                        }
                    }
                }
                let counter = e.ship_id.0.counter();
                if counter >= self.id_counter {
                    self.id_counter = counter + 1;
                }
            }

            DomainEvent::VelocityChanged(e) => {
                if let Some(&entity) = self.ships.index.get(&e.ship_id) {
                    let gap_ticks = e
                        .tick
                        .value()
                        .saturating_sub(self.current_tick.value())
                        .saturating_sub(1);
                    let old_vel = self
                        .world
                        .inner()
                        .get::<&VelocityComp>(entity)
                        .ok()
                        .map(|v| v.0)
                        .unwrap_or(Velocity::ZERO);
                    if let Ok(mut pos) = self.world.inner_mut().get::<&mut PositionComp>(entity) {
                        pos.0.x += old_vel.dx * gap_ticks as f32 + e.velocity.dx;
                        pos.0.y += old_vel.dy * gap_ticks as f32 + e.velocity.dy;
                        pos.0.z += old_vel.dz * gap_ticks as f32 + e.velocity.dz;
                    }
                    if let Ok(mut vel) = self.world.inner_mut().get::<&mut VelocityComp>(entity) {
                        vel.0 = e.velocity;
                    }
                }
                if e.tick > self.current_tick {
                    self.current_tick = e.tick;
                }
            }

            DomainEvent::ShipDespawned(e) => {
                if let Some(entity) = self.ships.index.remove(&e.ship_id) {
                    self.world.despawn_ship(entity);
                }
                self.ships.type_ids.remove(&e.ship_id);
                self.base_stats.remove(&e.ship_id);
                if let Some(player_id) = self.ships.owners.remove(&e.ship_id) {
                    self.ships.by_player.remove(&player_id);
                }
            }

            DomainEvent::ShipFitted(e) => {
                if let Some(&entity) = self.ships.index.get(&e.ship_id) {
                    let fitting = FittingComp::from_snapshot(&e.fitting, &self.module_registry);
                    let base = self
                        .base_stats
                        .get(&e.ship_id)
                        .copied()
                        .unwrap_or(ShipStatsComp::NPC);
                    let _ = self.world.inner_mut().insert_one(entity, fitting);
                    apply_fitting(&mut self.world, e.ship_id, base);
                    // Inventory snapshot (ADR-0032): always present alongside
                    // the fitting it changed together with.
                    let _ = self.world.inner_mut().insert_one(
                        entity,
                        dawn_ecs::components::InventoryComp {
                            items: e.inventory.clone(),
                        },
                    );
                }
            }

            DomainEvent::ModuleActivated(e) => {
                if let Some(&entity) = self.ships.index.get(&e.ship_id) {
                    if let Ok(mut fitting) = self.world.inner_mut().get::<&mut FittingComp>(entity)
                    {
                        if let Some(slot) = fitting.find_slot_mut(e.module_id, e.slot) {
                            slot.is_active = true;
                            slot.target_ship_id = e.target_ship_id;
                        }
                    }
                    let base = self
                        .base_stats
                        .get(&e.ship_id)
                        .copied()
                        .unwrap_or(ShipStatsComp::NPC);
                    apply_fitting(&mut self.world, e.ship_id, base);
                }
            }

            DomainEvent::ModuleDeactivated(e) => {
                if let Some(&entity) = self.ships.index.get(&e.ship_id) {
                    if let Ok(mut fitting) = self.world.inner_mut().get::<&mut FittingComp>(entity)
                    {
                        if let Some(slot) = fitting.find_slot_mut(e.module_id, e.slot) {
                            slot.is_active = false;
                            slot.target_ship_id = None;
                        }
                    }
                    let base = self
                        .base_stats
                        .get(&e.ship_id)
                        .copied()
                        .unwrap_or(ShipStatsComp::NPC);
                    apply_fitting(&mut self.world, e.ship_id, base);
                }
            }

            // TargetLocked: set the LockComp entry to the Locked state
            DomainEvent::TargetLocked(e) => {
                use dawn_ecs::components::{LockEntry, LockState};
                if let Some(&entity) = self.ships.index.get(&e.locker_id) {
                    if let Ok(mut lock) = self.world.inner_mut().get::<&mut LockComp>(entity) {
                        if let Some(entry) = lock
                            .entries
                            .iter_mut()
                            .find(|en| en.target_id == e.target_id)
                        {
                            entry.state = LockState::Locked;
                        } else {
                            lock.entries.push(LockEntry {
                                target_id: e.target_id,
                                state: LockState::Locked,
                            });
                        }
                    }
                }
            }

            // LockLost: remove the entry from LockComp
            DomainEvent::LockLost(e) => {
                if let Some(&entity) = self.ships.index.get(&e.locker_id) {
                    if let Ok(mut lock) = self.world.inner_mut().get::<&mut LockComp>(entity) {
                        lock.entries.retain(|en| en.target_id != e.target_id);
                    }
                }
            }

            DomainEvent::WeaponFired(_) => {}

            DomainEvent::DamageTaken(e) => {
                if let Some(&entity) = self.ships.index.get(&e.ship_id) {
                    if let Ok(mut hull) = self.world.inner_mut().get::<&mut HullComp>(entity) {
                        hull.current_shield = e.current_shield;
                        hull.current_armor = e.current_armor;
                        hull.current_hull = e.current_hull;
                        if e.current_hull <= 0.0 {
                            hull.is_destroyed = true;
                        }
                    }
                }
            }

            DomainEvent::RepairApplied(e) => {
                if let Some(&entity) = self.ships.index.get(&e.ship_id) {
                    if let Ok(mut hull) = self.world.inner_mut().get::<&mut HullComp>(entity) {
                        hull.current_shield = e.current_shield;
                        hull.current_armor = e.current_armor;
                        hull.current_hull = e.current_hull;
                    }
                }
            }

            DomainEvent::ShipDestroyed(e) => {
                if let Some(entity) = self.ships.index.remove(&e.ship_id) {
                    self.world.despawn_ship(entity);
                }
                self.ships.type_ids.remove(&e.ship_id);
                self.base_stats.remove(&e.ship_id);
                if let Some(player_id) = self.ships.owners.remove(&e.ship_id) {
                    self.ships.by_player.remove(&player_id);
                }
            }

            // Sector Transit (ADR-0014): TransitState component and ownership
            // transfer are added in a later Phase 7 task.
            DomainEvent::SectorTransitRequested(_)
            | DomainEvent::SectorTransitCompleted(_)
            | DomainEvent::SectorTransitAborted(_) => {}

            // Jump Gate Navigation (ADR-0009): Sector/StarSystem transfer on
            // Replay is added when the Jump pipeline is wired in dawn-simulation.
            DomainEvent::JumpGateUsed(_) | DomainEvent::StarSystemChanged(_) => {}

            // Tackle (ADR-0024): TackledComp is managed live; on replay, apply
            // the same add/remove logic to keep the component consistent.
            DomainEvent::TackleApplied(e) => {
                if let Some(&entity) = self.ships.index.get(&e.ship_id) {
                    let has_comp = {
                        if let Ok(mut tackled) =
                            self.world.inner_mut().get::<&mut TackledComp>(entity)
                        {
                            if !tackled.tacklers.contains(&e.by) {
                                tackled.tacklers.push(e.by);
                            }
                            true
                        } else {
                            false
                        }
                    };
                    if !has_comp {
                        let _ = self.world.inner_mut().insert_one(
                            entity,
                            TackledComp {
                                tacklers: vec![e.by],
                            },
                        );
                    }
                }
            }

            DomainEvent::TackleReleased(e) => {
                if let Some(&entity) = self.ships.index.get(&e.ship_id) {
                    let should_remove = {
                        if let Ok(mut tackled) =
                            self.world.inner_mut().get::<&mut TackledComp>(entity)
                        {
                            tackled.tacklers.retain(|&id| id != e.by);
                            tackled.tacklers.is_empty()
                        } else {
                            false
                        }
                    };
                    if should_remove {
                        let _ = self.world.inner_mut().remove_one::<TackledComp>(entity);
                    }
                }
            }

            DomainEvent::AnchorRebased(e) => {
                // Frame rebase (ADR-0029): set anchor + position offset directly.
                // Absolute position is unchanged; this only updates the
                // (anchor, offset) representation so replay stays consistent.
                if let Some(&entity) = self.ships.index.get(&e.ship_id) {
                    self.world.set_ship_anchor(entity, e.anchor);
                    if let Ok(mut pos) = self.world.inner_mut().get::<&mut PositionComp>(entity) {
                        pos.0 = e.offset;
                    }
                }
                if e.tick > self.current_tick {
                    self.current_tick = e.tick;
                }
            }
        }
    }

    /// Public test wrapper for `apply_event`.
    #[cfg(test)]
    pub fn apply_event_pub(&mut self, event: DomainEvent) {
        self.apply_event(&event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{DomainEvent, NodeId, Position, SectorBounds, SectorId, Tick, Velocity};

    fn mem_node() -> SimulationNode {
        SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        )
    }

    #[test]
    fn damage_taken_event_is_replayed_to_restore_current_hp() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);

        node.apply_event_pub(DomainEvent::DamageTaken(dawn_core::events::DamageTaken {
            ship_id,
            damage: 100.0,
            current_shield: 100.0,
            current_armor: 150.0,
            current_hull: 150.0,
            tick: Tick(1),
        }));

        let hp = node.get_ship_hp(ship_id).unwrap();
        assert_eq!(hp, 400.0, "HP total after replay = 100 + 150 + 150 = 400");
    }

    #[test]
    fn repair_applied_event_is_replayed_to_restore_current_hp() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);

        node.apply_event_pub(DomainEvent::RepairApplied(
            dawn_core::events::RepairApplied {
                ship_id,
                amount: 50.0,
                layer: dawn_core::events::RepairLayer::Shield,
                current_shield: 150.0,
                current_armor: 150.0,
                current_hull: 150.0,
                tick: Tick(2),
            },
        ));

        let hp = node.get_ship_hp(ship_id).unwrap();
        assert_eq!(hp, 450.0, "HP total after replay = 150 + 150 + 150 = 450");
    }
}
