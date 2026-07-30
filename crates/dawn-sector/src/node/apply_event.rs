use dawn_core::{DomainEvent, Velocity};
use dawn_ecs::components::{
    FittingComp, HullComp, LockComp, PositionComp, TackledComp, VelocityComp,
};
use dawn_event_store::store::EventStore;

use super::SimulationNode;

impl<S: EventStore> SimulationNode<S> {
    /// Apply a single domain event to the ECS World without appending it.
    /// Used during `restore_from` to replay post-snapshot events.
    pub(super) fn apply_event(&mut self, event: &DomainEvent) {
        // Transit replay policy belongs to the same deep module as the live
        // Request/Commit/Ack and retry policy. This generic EventStore adapter
        // only executes the directive using node-private ECS mechanisms.
        if let Some(directive) = crate::transit::pipeline::replay_directive(event) {
            match directive {
                crate::transit::pipeline::ReplayDirective::Requested(event) => {
                    self.replay_sector_transit_requested(event);
                }
                crate::transit::pipeline::ReplayDirective::Completed(event) => {
                    self.replay_sector_transit_completed(event);
                }
                crate::transit::pipeline::ReplayDirective::Aborted(event) => {
                    self.replay_sector_transit_aborted(event);
                }
            }
            return;
        }

        match event {
            DomainEvent::ShipSpawned(e) => {
                if !self.ships.index.contains_key(&e.ship_id) {
                    self.insert_to_world(e.ship_id, dawn_core::Position::ORIGIN, Velocity::ZERO);
                    // ADR-0029 review #1: anchor on the nearest body (deterministic
                    // — same initial_position reproduces the same anchor on replay).
                    self.set_spawn_anchor_abs(e.ship_id, e.initial_position);
                    // Shared with the live spawn path (issue #197): this used
                    // to be hand-rolled here and silently omitted the
                    // `ships.type_ids` insertion and `CapacitorComp` init that
                    // `materialize_ship_stats` (and therefore live spawning)
                    // always did, so a ship spawned after the last snapshot
                    // could come back from a snapshot + tail-log restore
                    // missing state the live node had.
                    self.materialize_ship_stats(
                        e.ship_id,
                        e.ship_type_id,
                        dawn_ecs::components::ShipStatsComp::NPC,
                    );
                    if let Some(&entity) = self.ships.index.get(&e.ship_id) {
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
                        .get::<VelocityComp>(entity)
                        .map(|v| v.0)
                        .unwrap_or(Velocity::ZERO);
                    if let Some(mut pos) = self.world.get_mut::<PositionComp>(entity) {
                        let gap_ticks = gap_ticks as f64;
                        pos.0.x += old_vel.dx * gap_ticks + e.velocity.dx;
                        pos.0.y += old_vel.dy * gap_ticks + e.velocity.dy;
                        pos.0.z += old_vel.dz * gap_ticks + e.velocity.dz;
                    }
                    if let Some(mut vel) = self.world.get_mut::<VelocityComp>(entity) {
                        vel.0 = e.velocity;
                    }
                }
                if e.tick > self.current_tick {
                    self.current_tick = e.tick;
                }
            }

            DomainEvent::ShipDespawned(e) => {
                self.remove_ship(e.ship_id);
            }

            DomainEvent::ShipFitted(e) => {
                if let Some(&entity) = self.ships.index.get(&e.ship_id) {
                    let fitting = FittingComp::from_snapshot(&e.fitting, &self.module_registry);
                    let _ = self.world.insert_one(entity, fitting);
                    self.reapply_fitting(e.ship_id);
                    // Inventory snapshot (ADR-0032): always present alongside
                    // the fitting it changed together with.
                    let _ = self.world.insert_one(
                        entity,
                        dawn_ecs::components::InventoryComp {
                            items: e.inventory.iter().copied().fold(
                                std::collections::BTreeMap::new(),
                                |mut items, item_id| {
                                    *items.entry(item_id).or_default() += 1;
                                    items
                                },
                            ),
                        },
                    );
                }
            }

            DomainEvent::ModuleActivated(e) => {
                if let Some(&entity) = self.ships.index.get(&e.ship_id) {
                    if let Some(mut fitting) = self.world.get_mut::<FittingComp>(entity) {
                        if let Some(slot) = fitting.find_slot_mut(e.module_id, e.slot) {
                            slot.is_active = true;
                            slot.target_ship_id = e.target_ship_id;
                        }
                    }
                    self.reapply_fitting(e.ship_id);
                }
            }

            DomainEvent::ModuleDeactivated(e) => {
                if let Some(&entity) = self.ships.index.get(&e.ship_id) {
                    if let Some(mut fitting) = self.world.get_mut::<FittingComp>(entity) {
                        if let Some(slot) = fitting.find_slot_mut(e.module_id, e.slot) {
                            slot.force_off();
                        }
                    }
                    self.reapply_fitting(e.ship_id);
                }
            }

            // TargetLocked: set the LockComp entry to the Locked state
            DomainEvent::TargetLocked(e) => {
                use dawn_ecs::components::{LockEntry, LockState};
                if let Some(&entity) = self.ships.index.get(&e.locker_id) {
                    if let Some(mut lock) = self.world.get_mut::<LockComp>(entity) {
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
                    if let Some(mut lock) = self.world.get_mut::<LockComp>(entity) {
                        lock.entries.retain(|en| en.target_id != e.target_id);
                    }
                }
            }

            DomainEvent::WeaponFired(_) => {}

            DomainEvent::DamageTaken(e) => {
                if let Some(&entity) = self.ships.index.get(&e.ship_id) {
                    if let Some(mut hull) = self.world.get_mut::<HullComp>(entity) {
                        hull.set_hp(e.current_shield, e.current_armor, e.current_hull);
                    }
                }
            }

            DomainEvent::RepairApplied(e) => {
                if let Some(&entity) = self.ships.index.get(&e.ship_id) {
                    if let Some(mut hull) = self.world.get_mut::<HullComp>(entity) {
                        hull.set_hp(e.current_shield, e.current_armor, e.current_hull);
                    }
                }
            }

            DomainEvent::ShipDestroyed(e) => {
                self.remove_ship(e.ship_id);
            }

            DomainEvent::SectorTransitRequested(_)
            | DomainEvent::SectorTransitCompleted(_)
            | DomainEvent::SectorTransitAborted(_) => {
                unreachable!("Transit replay directives return before the generic event match")
            }

            // Jump Gate Navigation (ADR-0009): Sector/StarSystem transfer on
            // Replay is added when the Jump pipeline is wired in dawn-simulation.
            DomainEvent::JumpGateUsed(_) | DomainEvent::StarSystemChanged(_) => {}

            // Tackle (ADR-0024): TackledComp is managed live; on replay, apply
            // the same add/remove logic to keep the component consistent.
            DomainEvent::TackleApplied(e) => {
                if let Some(&entity) = self.ships.index.get(&e.ship_id) {
                    let has_comp = {
                        if let Some(mut tackled) = self.world.get_mut::<TackledComp>(entity) {
                            if !tackled.tacklers.contains(&e.by) {
                                tackled.tacklers.push(e.by);
                            }
                            true
                        } else {
                            false
                        }
                    };
                    if !has_comp {
                        let _ = self.world.insert_one(
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
                        if let Some(mut tackled) = self.world.get_mut::<TackledComp>(entity) {
                            tackled.tacklers.retain(|&id| id != e.by);
                            tackled.tacklers.is_empty()
                        } else {
                            false
                        }
                    };
                    if should_remove {
                        let _ = self.world.remove_one::<TackledComp>(entity);
                    }
                }
            }

            DomainEvent::AnchorRebased(e) => {
                // Frame rebase (ADR-0029): set anchor + position offset directly.
                // Absolute position is unchanged; this only updates the
                // (anchor, offset) representation so replay stays consistent.
                if let Some(&entity) = self.ships.index.get(&e.ship_id) {
                    self.world.set_ship_anchor(entity, e.anchor);
                    if let Some(mut pos) = self.world.get_mut::<PositionComp>(entity) {
                        pos.0 = e.offset;
                    }
                }
                if e.tick > self.current_tick {
                    self.current_tick = e.tick;
                }
            }

            DomainEvent::ShipDocked(e) => {
                self.settle_ship_into_station(e.ship_id, e.station_id);
                self.docked_ships.insert(e.ship_id, e.station_id);
                if let Some(player_id) = self.ships.owners.get(&e.ship_id).copied() {
                    self.docked_players.insert(player_id, e.station_id);
                }
                if e.tick > self.current_tick {
                    self.current_tick = e.tick;
                }
            }

            DomainEvent::ShipUndocked(e) => {
                if let Some(player_id) = self.ships.owners.get(&e.ship_id).copied() {
                    self.docked_players.remove(&player_id);
                }
                self.docked_ships.remove(&e.ship_id);
                if e.tick > self.current_tick {
                    self.current_tick = e.tick;
                }
            }

            DomainEvent::PackagedShipBuilt(e) => {
                // ADR-0038: Station inventory is durable in SQLite, written
                // through synchronously by the live command handler
                // (`station_operation_execution.rs`) before this event
                // was even appended. Replaying the credit/debit here would
                // double-apply it.
                if e.tick > self.current_tick {
                    self.current_tick = e.tick;
                }
            }

            DomainEvent::ShipDisassembled(e) => {
                // ADR-0038: see PackagedShipBuilt above -- the credit already
                // happened live in `disassemble_ship_owned`.
                self.remove_ship(e.ship_id);
                if e.tick > self.current_tick {
                    self.current_tick = e.tick;
                }
            }

            DomainEvent::ShipAssembled(e) => {
                // ADR-0038: see PackagedShipBuilt above -- the debit already
                // happened live in `assemble_ship_owned`.
                if !self.ships.index.contains_key(&e.ship_id) {
                    self.insert_ship_entity(
                        e.ship_id,
                        e.ship_type_id,
                        dawn_core::Position::ORIGIN,
                        Velocity::ZERO,
                    );
                    if let Some(&entity) = self.ships.index.get(&e.ship_id) {
                        let _ = self
                            .world
                            .remove_one::<dawn_ecs::components::IsNpcComp>(entity);
                    }
                    self.settle_ship_into_station(e.ship_id, e.station_id);
                }
                self.docked_ships.insert(e.ship_id, e.station_id);
                self.ships.owners.insert(e.ship_id, e.player_id);
                let counter = e.ship_id.0.counter();
                if counter >= self.id_counter {
                    self.id_counter = counter + 1;
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
mod tests;
