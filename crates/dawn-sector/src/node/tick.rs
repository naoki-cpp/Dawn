use std::collections::{BTreeMap, HashSet};

use super::{SimulationNode, TickResult};
use crate::persistence::ShipSnapshot;
use crate::transition::{
    PreparedSectorTransition, SectorEngine, SectorRecoveryDelta, SectorTransitionId, ShipState,
    TickApproachState, TickCommandState, TickFittingState, TickKeepAtRangeState, TickLockEntry,
    TickLockPhase, TickLockState, TickMovementState, TickOrbitState, TickRecoveryDelta,
    TickRecoveryMode, TickSlotState, TickWarpPhase, TickWarpState, TransitionApplyError,
    TransitionContext, TransitionError,
};
use dawn_core::{DomainEvent, ItemId, ShipId};
use dawn_ecs::{
    components::{
        ApproachComp, CapacitorComp, FittedSlot, FittingComp, HullComp, InventoryComp,
        KeepAtRangeComp, LockComp, LockEntry, LockState, OrbitComp, PositionComp, TackledComp,
        VelocityComp, WarpComp, WarpPhase, WeaponComp,
    },
    systems::{CapacitorSystem, CombatSystem, LockSystem, MovementSystem, RepairSystem},
    Entity,
};
use thiserror::Error;

const SCRAP_METAL_PER_SHIP_DESTROYED: u64 = 1;

/// Failure while building the bounded, storage-independent Tick write set.
#[derive(Debug, Error)]
pub enum TickPreparationError {
    #[error(transparent)]
    Transition(#[from] TransitionError),
    #[error("prepared Tick could not restore the live state: {0}")]
    Restoration(#[from] TransitionApplyError),
}

fn lock_event_touches_transit(event: &DomainEvent, transit_ids: &HashSet<ShipId>) -> bool {
    match event {
        DomainEvent::TargetLocked(event) => {
            transit_ids.contains(&event.locker_id) || transit_ids.contains(&event.target_id)
        }
        DomainEvent::LockLost(event) => {
            transit_ids.contains(&event.locker_id) || transit_ids.contains(&event.target_id)
        }
        _ => transit_ids.contains(&event.ship_id()),
    }
}

/// `(InTransit ship ids, their frozen optional components, related ships'
/// LockComp frozen for consistency)` -- see `freeze_transit_components`.
type FrozenTransitState = (
    HashSet<ShipId>,
    Vec<(Entity, dawn_ecs::OptionalShipComponents)>,
    Vec<(Entity, LockComp)>,
);

impl SimulationNode {
    /// Prepare the logical Tick counter without changing the live state.
    pub(crate) fn prepare_tick_transition(
        &self,
        transition_id: SectorTransitionId,
        owner_epoch: u64,
    ) -> Result<PreparedSectorTransition, TransitionError> {
        SectorEngine::prepare_tick(
            TickCommandState {
                current_tick: self.simulation.current_tick,
            },
            transition_id,
            TransitionContext {
                sector_id: self.sector_id,
                owner_epoch,
            },
        )
    }

    /// Apply a committed logical Tick counter delta during recovery or catch-up.
    pub(crate) fn apply_tick_transition(
        &mut self,
        delta: TickRecoveryDelta,
        context: TransitionContext,
    ) -> Result<(), TransitionApplyError> {
        self.validate_tick_transition(&delta, context)?;
        self.apply_validated_full_tick(delta)?;
        Ok(())
    }

    pub(crate) fn validate_tick_transition(
        &self,
        delta: &TickRecoveryDelta,
        context: TransitionContext,
    ) -> Result<(), TransitionApplyError> {
        if context.sector_id != self.sector_id {
            return Err(TransitionApplyError::SectorMismatch {
                expected: self.sector_id,
                actual: context.sector_id,
            });
        }
        let Some(expected_to) = delta.from.0.checked_add(1).map(dawn_core::Tick) else {
            return Err(TransitionApplyError::InvalidTickStep {
                from: delta.from,
                to: delta.to,
            });
        };
        if delta.to != expected_to {
            return Err(TransitionApplyError::InvalidTickStep {
                from: delta.from,
                to: delta.to,
            });
        }
        if self.simulation.current_tick != delta.from {
            return Err(TransitionApplyError::TickMismatch {
                expected: delta.from,
                actual: self.simulation.current_tick,
            });
        }
        let mut seen = HashSet::new();
        for state in &delta.ship_states {
            if !seen.insert(state.snapshot.ship_id) {
                return Err(TransitionApplyError::InvalidTickShipState(
                    state.snapshot.ship_id,
                ));
            }
        }
        let mut removed_seen = HashSet::new();
        for ship_id in &delta.removed_ships {
            if !removed_seen.insert(*ship_id) {
                return Err(TransitionApplyError::InvalidTickShipState(*ship_id));
            }
            if delta
                .ship_states
                .iter()
                .any(|state| state.snapshot.ship_id == *ship_id)
            {
                return Err(TransitionApplyError::InvalidTickShipState(*ship_id));
            }
        }
        for state in &delta.ship_states {
            if !state.fitting_present {
                continue;
            }
            let mut fitting = FittingComp::from_snapshot(
                &state.snapshot.fitting,
                &self.game_data.module_registry,
            );
            if !Self::restore_tick_fitting(&mut fitting, &state.fitting) {
                return Err(TransitionApplyError::InvalidTickShipState(
                    state.snapshot.ship_id,
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn apply_validated_logical_tick(&mut self, delta: TickRecoveryDelta) {
        self.simulation.current_tick = delta.to;
    }

    pub(crate) fn apply_validated_full_tick(
        &mut self,
        delta: TickRecoveryDelta,
    ) -> Result<(), TransitionApplyError> {
        if delta.mode == TickRecoveryMode::Logical {
            self.apply_validated_logical_tick(delta);
            return Ok(());
        }
        let expected_ships: HashSet<_> = delta
            .ship_states
            .iter()
            .map(|state| state.snapshot.ship_id)
            .collect();
        let stale_ships: Vec<_> = self
            .simulation
            .ships
            .index
            .keys()
            .copied()
            .filter(|ship_id| !expected_ships.contains(ship_id))
            .collect();
        for ship_id in stale_ships {
            self.remove_ship(ship_id);
        }

        // Restore node-level maps before materialising a ship so a newly
        // admitted player ship receives PLAYER stats rather than NPC stats.
        // Same restore function take_snapshot's checkpoint path uses
        // (ADR-0049, issue #312); transit_saga is restored separately below
        // via restore_transit_saga's Saga reconciliation.
        self.restore_node_state(&crate::transition::NodeState {
            id_counter: delta.id_counter,
            player_id_counter: delta.player_id_counter,
            transit_attempt_counter: delta.transit_attempt_counter,
            active_ships: delta.active_ships.clone(),
            owners: delta.owners.clone(),
            docked_ships: delta.docked_ships.clone(),
            docked_players: delta.docked_players.clone(),
            pending_bot_lock_commands: delta.pending_bot_lock_commands.clone(),
            pending_auto_jumps: delta.pending_auto_jumps.clone(),
            applied_market_settlements: delta.applied_market_settlements.clone(),
            transit_saga: delta.transit_saga.clone(),
        });

        for state in &delta.ship_states {
            if !self
                .simulation
                .ships
                .index
                .contains_key(&state.snapshot.ship_id)
            {
                self.restore_ship_from_snapshot(&state.snapshot);
            }
            self.apply_tick_ship_state(state)?;
        }
        for ship_id in &delta.removed_ships {
            self.remove_ship(*ship_id);
        }
        // pending_bot_lock_commands/pending_auto_jumps were already restored
        // above via restore_node_state.
        self.frame_outputs.completed_warps = delta.completed_warps;
        self.restore_transit_saga(delta.transit_saga)
            .map_err(TransitionApplyError::InvalidTransitSaga)?;
        self.simulation.current_tick = delta.to;
        Ok(())
    }

    /// Prepare the complete ECS Tick write set without changing the live
    /// authoritative state. The legacy systems execute once against a
    /// bounded ship-state image, then the image is restored before the
    /// transition is returned to the runtime.
    #[cfg(test)]
    pub(crate) fn prepare_tick_state_transition(
        &mut self,
        input: crate::transition::FrameInput<'_>,
        transition_id: SectorTransitionId,
        owner_epoch: u64,
    ) -> Result<PreparedSectorTransition, TickPreparationError> {
        self.prepare_tick_state_transition_with_result(input, transition_id, owner_epoch)
            .map(|(prepared, _)| prepared)
    }

    pub(crate) fn prepare_tick_state_transition_with_result(
        &mut self,
        input: crate::transition::FrameInput<'_>,
        transition_id: SectorTransitionId,
        owner_epoch: u64,
    ) -> Result<(PreparedSectorTransition, TickResult), TickPreparationError> {
        let logical = self.prepare_tick_transition(transition_id, owner_epoch)?;
        let TransitionContext {
            sector_id,
            owner_epoch,
        } = logical.context;
        let before_tick = self.simulation.current_tick;
        let before_states = self.capture_tick_write_set();
        let before_pending_event_count = self.frame_outputs.pending_events.len();
        let before_bot_commands = self.simulation.pending_bot_lock_commands.clone();
        let before_auto_jumps = self.frame_outputs.pending_auto_jumps.clone();
        let before_completed_warps = self.frame_outputs.completed_warps.clone();
        let before_transit_attempt_counter = self.transit.transit_attempt_counter;
        let before_transit_saga = self.transit_saga_snapshot();
        let before_transit_journal = self.transit.transit_journal.clone();
        let before_market_settlements = self.simulation.applied_market_settlements.clone();

        // Market settlement mutations (issue #315) apply before the write set
        // is captured below, so they are part of the same durable delta as
        // this tick's other state and roll back with it if preparation does
        // not lead to a committed apply.
        let settlement_outcomes: Vec<crate::transition::MarketSettlementOutcome> = input
            .market_settlements
            .iter()
            .map(|settlement| crate::transition::MarketSettlementOutcome {
                settlement_id: settlement.settlement_id(),
                applied: self.apply_market_settlement_input(*settlement),
            })
            .collect();

        let result = self.tick_with_lock_commands_mode(input.lock_commands, false, false);
        let deferred_events = self
            .frame_outputs
            .pending_events
            .split_off(before_pending_event_count);
        let mut result = result;
        result.events.extend(deferred_events);
        result.events_emitted = result.events.len();
        result.market_settlement_outcomes = settlement_outcomes;
        let after_states = self.capture_tick_write_set();
        // Keep the complete post-tick ship image. Commands are admitted before
        // this preparation starts, so a diff against `before_states` would
        // omit command-only mutations from the durable record. The full image
        // is still bounded by the Sector population and makes each committed
        // Tick a self-contained recovery boundary.
        let ship_states = after_states
            .values()
            .filter(|state| !state.snapshot.is_destroyed)
            .cloned()
            .collect();
        let removed_ships = before_states
            .keys()
            .filter(|ship_id| {
                after_states
                    .get(ship_id)
                    .map(|state| state.snapshot.is_destroyed)
                    .unwrap_or(true)
            })
            .copied()
            .collect();
        // Node-level scalars/maps this Sector must reproduce exactly
        // (ADR-0049, issue #312), shared with take_snapshot_at's
        // construction (snapshot_io.rs) through the same capture function.
        let crate::transition::NodeState {
            id_counter,
            player_id_counter,
            transit_attempt_counter,
            active_ships,
            owners,
            docked_ships,
            docked_players,
            pending_bot_lock_commands,
            pending_auto_jumps,
            applied_market_settlements,
            transit_saga,
        } = self.capture_node_state();

        let delta = TickRecoveryDelta {
            from: before_tick,
            to: result.tick,
            mode: TickRecoveryMode::Full,
            ship_states,
            removed_ships,
            pending_bot_lock_commands,
            pending_auto_jumps,
            completed_warps: self.frame_outputs.completed_warps.clone(),
            transit_attempt_counter,
            id_counter,
            player_id_counter,
            active_ships,
            owners,
            docked_ships,
            docked_players,
            applied_market_settlements,
            transit_saga,
        };

        self.simulation.current_tick = before_tick;
        let restore_result = self.restore_tick_write_set(&before_states);
        // The legacy systems may emit public facts while preparing. Their
        // node-local projections are speculative until the enclosing journal
        // batch commits, just like the ECS write set.
        self.transit.transit_journal = before_transit_journal;
        restore_result?;
        self.simulation.pending_bot_lock_commands = before_bot_commands;
        self.frame_outputs.pending_auto_jumps = before_auto_jumps;
        self.frame_outputs.completed_warps = before_completed_warps;
        self.transit.transit_attempt_counter = before_transit_attempt_counter;
        self.simulation.applied_market_settlements = before_market_settlements;
        self.restore_transit_saga(before_transit_saga)
            .expect("prepared Tick must restore the previous Transit Saga");
        self.frame_outputs
            .pending_events
            .truncate(before_pending_event_count);

        let prepared = PreparedSectorTransition {
            transition_id,
            context: TransitionContext {
                sector_id,
                owner_epoch,
            },
            recovery_delta: SectorRecoveryDelta::Tick(Box::new(delta)),
            public_events: result.events.clone(),
            reliable_effects: Vec::new(),
        };
        Ok((prepared, result))
    }

    /// Execute one simulation tick.
    pub fn tick(&mut self) -> TickResult {
        self.tick_with_lock_commands(&[])
    }

    // Mirrors the durable SectorTransitRequested payload at the public-output
    // adapter boundary.
    pub(crate) fn append_incoming_transit_marker(
        &mut self,
        ship_id: ShipId,
        from: dawn_core::SectorId,
        to: dawn_core::SectorId,
        request_tick: dawn_core::Tick,
        gate_id: Option<dawn_core::JumpGateId>,
        entry_pos: dawn_core::AbsolutePosition,
    ) {
        self.emit_event(DomainEvent::SectorTransitRequested(
            dawn_core::events::SectorTransitRequested {
                ship_id,
                from,
                to,
                request_tick,
                gate_id,
                entry_pos,
                tick: self.simulation.current_tick,
            },
        ));
    }

    fn freeze_transit_components(&mut self) -> FrozenTransitState {
        let transit: Vec<(ShipId, Entity)> = self
            .simulation
            .ships
            .index
            .iter()
            .filter_map(|(&ship_id, &entity)| {
                self.simulation
                    .world
                    .transit_state(entity)
                    .is_in_transit()
                    .then_some((ship_id, entity))
            })
            .collect();
        let ids: HashSet<_> = transit.iter().map(|(ship_id, _)| *ship_id).collect();
        let related_locks = self
            .simulation
            .ships
            .index
            .iter()
            .filter_map(|(&ship_id, &entity)| {
                let lock = self.simulation.world.get::<LockComp>(entity)?;
                (ids.contains(&ship_id)
                    || lock
                        .entries
                        .iter()
                        .any(|entry| ids.contains(&entry.target_id)))
                .then(|| (entity, (*lock).clone()))
            })
            .collect();
        let components = transit
            .into_iter()
            .map(|(_, entity)| {
                // Explicit subset of the canonical optional-component list
                // (dawn_ecs::OptionalShipComponents, ADR-0049 / issue #312):
                // only these seven need removing from an InTransit ship's
                // ECS query surface. Lock/Weapon/Thrust/Inventory are
                // deliberately left in place -- Movement/Combat already
                // skip InTransit ships via TransitComp, so those four
                // components being present is harmless, and restoring them
                // here would cost a remove/reinsert every tick for no
                // behavioural gain. If dawn-ecs adds a new optional
                // component, this struct literal fails to compile until
                // this list explicitly decides whether it needs freezing
                // too.
                let frozen = dawn_ecs::OptionalShipComponents {
                    capacitor: self.simulation.world.remove_one::<CapacitorComp>(entity),
                    fitting: self.simulation.world.remove_one::<FittingComp>(entity),
                    tackled: self.simulation.world.remove_one::<TackledComp>(entity),
                    warp: self.simulation.world.remove_one::<WarpComp>(entity),
                    approach: self.simulation.world.remove_one::<ApproachComp>(entity),
                    orbit: self.simulation.world.remove_one::<OrbitComp>(entity),
                    keep_at_range: self.simulation.world.remove_one::<KeepAtRangeComp>(entity),
                    weapon: None,
                    lock: None,
                    inventory: None,
                    thrust: None,
                };
                (entity, frozen)
            })
            .collect();
        (ids, components, related_locks)
    }

    fn restore_transit_components(
        &mut self,
        frozen: Vec<(Entity, dawn_ecs::OptionalShipComponents)>,
        related_locks: Vec<(Entity, LockComp)>,
    ) {
        for (entity, frozen) in frozen {
            // Only reinsert what freeze_transit_components actually removed
            // (see its comment). weapon/lock/inventory/thrust are always
            // None here; restore_optional_components would treat that as
            // "remove", which would incorrectly clear components this
            // freeze never touched -- so this stays a manual per-field
            // insert rather than one restore_optional_components call.
            debug_assert!(
                frozen.weapon.is_none()
                    && frozen.lock.is_none()
                    && frozen.inventory.is_none()
                    && frozen.thrust.is_none(),
                "freeze_transit_components must not populate fields it doesn't freeze"
            );
            if let Some(component) = frozen.capacitor {
                let _ = self.simulation.world.insert_one(entity, component);
            }
            if let Some(component) = frozen.fitting {
                let _ = self.simulation.world.insert_one(entity, component);
            }
            if let Some(component) = frozen.tackled {
                let _ = self.simulation.world.insert_one(entity, component);
            }
            if let Some(component) = frozen.warp {
                let _ = self.simulation.world.insert_one(entity, component);
            }
            if let Some(component) = frozen.approach {
                let _ = self.simulation.world.insert_one(entity, component);
            }
            if let Some(component) = frozen.orbit {
                let _ = self.simulation.world.insert_one(entity, component);
            }
            if let Some(component) = frozen.keep_at_range {
                let _ = self.simulation.world.insert_one(entity, component);
            }
        }
        for (entity, component) in related_locks {
            let _ = self.simulation.world.remove_one::<LockComp>(entity);
            let _ = self.simulation.world.insert_one(entity, component);
        }
    }

    fn capture_tick_fitting(fitting: &FittingComp) -> TickFittingState {
        fn capture_slots(slots: &[FittedSlot]) -> Vec<TickSlotState> {
            slots
                .iter()
                .map(|slot| TickSlotState {
                    module_id: slot.def.id,
                    is_active: slot.is_active,
                    cycle_remaining: slot.cycle_remaining,
                    target_ship_id: slot.target_ship_id,
                })
                .collect()
        }

        TickFittingState {
            high: capture_slots(&fitting.high),
            mid: capture_slots(&fitting.mid),
            low: capture_slots(&fitting.low),
            rig: capture_slots(&fitting.rig),
        }
    }

    pub(super) fn capture_tick_ship_state(&self, ship_id: ShipId) -> Option<ShipState> {
        let entity = *self.simulation.ships.index.get(&ship_id)?;
        let position = self.simulation.world.get::<PositionComp>(entity)?.0;
        let velocity = self.simulation.world.get::<VelocityComp>(entity)?.0;
        let hull = self.simulation.world.get::<HullComp>(entity)?;

        // One read of every optional component (ADR-0049, issue #312). The
        // exhaustive destructure means a component dawn-ecs adds to its
        // declared list fails this function to compile instead of silently
        // going uncaptured.
        let dawn_ecs::OptionalShipComponents {
            capacitor,
            weapon,
            lock,
            fitting,
            inventory,
            approach,
            orbit,
            keep_at_range,
            warp,
            tackled,
            thrust,
        } = self.simulation.world.capture_optional_components(entity);

        let fitting_snapshot = fitting
            .as_ref()
            .map(FittingComp::to_snapshot)
            .unwrap_or_else(dawn_core::fitting::FittingSnapshot::empty);
        let fitting_state = fitting
            .as_ref()
            .map(Self::capture_tick_fitting)
            .unwrap_or_else(|| TickFittingState {
                high: Vec::new(),
                mid: Vec::new(),
                low: Vec::new(),
                rig: Vec::new(),
            });
        let fitting_present = fitting.is_some();
        let inventory_present = inventory.is_some();
        let snapshot = ShipSnapshot {
            ship_id,
            ship_type_id: self
                .simulation
                .ships
                .type_ids
                .get(&ship_id)
                .copied()
                .unwrap_or(dawn_core::ShipTypeId(0)),
            absolute_position: self.ship_absolute_pos(ship_id),
            position,
            anchor: self
                .simulation
                .world
                .ship_anchor(entity)
                .unwrap_or_default(),
            velocity,
            current_shield: hull.shield(),
            current_armor: hull.armor(),
            current_hull: hull.hull(),
            is_destroyed: hull.is_destroyed(),
            capacitor: capacitor.map(|capacitor| capacitor.current),
            fitting: fitting_snapshot,
            tackled_by: tackled.map(|tackled| tackled.tacklers).unwrap_or_default(),
            inventory: inventory
                .map(|inventory| inventory.items)
                .unwrap_or_default(),
        };
        let movement = TickMovementState {
            thrust: thrust.map(|thrust| crate::transition::StopThrustState {
                direction: thrust.direction,
                is_braking: thrust.is_braking,
            }),
            approach: approach.map(|approach| TickApproachState {
                target: approach.target,
                auto_jump_gate: approach.auto_jump_gate,
            }),
            orbit: orbit.map(|orbit| TickOrbitState {
                target: orbit.target,
                radius: orbit.radius,
            }),
            keep_at_range: keep_at_range.map(|keep_at_range| TickKeepAtRangeState {
                target: keep_at_range.target,
                range: keep_at_range.range,
            }),
            warp: warp.map(|warp| TickWarpState {
                target: warp.target,
                phase: match warp.phase {
                    WarpPhase::Aligning => TickWarpPhase::Aligning,
                    WarpPhase::Warping => TickWarpPhase::Warping,
                },
                auto_jump: warp.auto_jump,
                warp_start_abs: warp.warp_start_abs,
                warp_total: warp.warp_total,
                warp_elapsed: warp.warp_elapsed,
                warp_arrival_abs: warp.warp_arrival_abs,
                warp_start_vel: warp.warp_start_vel,
            }),
        };
        let locks = lock.map(|lock| TickLockState {
            entries: lock
                .entries
                .into_iter()
                .map(|entry| TickLockEntry {
                    target_id: entry.target_id,
                    state: match entry.state {
                        LockState::Locking { remaining_ticks } => {
                            TickLockPhase::Locking { remaining_ticks }
                        }
                        LockState::Locked => TickLockPhase::Locked,
                    },
                })
                .collect(),
        });

        Some(ShipState {
            snapshot,
            fitting_present,
            inventory_present,
            movement,
            fitting: fitting_state,
            locks,
            weapon_last_fired_tick: weapon.map(|weapon| weapon.last_fired_tick),
        })
    }

    fn capture_tick_write_set(&self) -> BTreeMap<ShipId, ShipState> {
        self.simulation
            .ships
            .index
            .keys()
            .copied()
            .filter_map(|ship_id| {
                self.capture_tick_ship_state(ship_id)
                    .map(|state| (ship_id, state))
            })
            .collect()
    }

    fn restore_tick_fitting(fitting: &mut FittingComp, state: &TickFittingState) -> bool {
        fn restore_slots(slots: &mut [FittedSlot], state: &[TickSlotState]) -> bool {
            if slots.len() != state.len() {
                return false;
            }
            for (slot, saved) in slots.iter_mut().zip(state) {
                if slot.def.id != saved.module_id {
                    return false;
                }
                slot.is_active = saved.is_active;
                slot.cycle_remaining = saved.cycle_remaining;
                slot.target_ship_id = saved.target_ship_id;
            }
            true
        }

        restore_slots(&mut fitting.high, &state.high)
            && restore_slots(&mut fitting.mid, &state.mid)
            && restore_slots(&mut fitting.low, &state.low)
            && restore_slots(&mut fitting.rig, &state.rig)
    }

    pub(super) fn apply_tick_ship_state(
        &mut self,
        state: &ShipState,
    ) -> Result<(), TransitionApplyError> {
        let ship = &state.snapshot;
        let entity = self
            .simulation
            .ships
            .index
            .get(&ship.ship_id)
            .copied()
            .ok_or(TransitionApplyError::UnknownShip(ship.ship_id))?;

        self.simulation
            .ships
            .type_ids
            .insert(ship.ship_id, ship.ship_type_id);
        if let Some(absolute_position) = ship.absolute_position {
            if let Some(offset) = self
                .topology
                .anchor_table
                .to_relative(ship.anchor, absolute_position)
            {
                self.simulation.world.set_ship_anchor(entity, ship.anchor);
                if let Some(mut position) = self.simulation.world.get_mut::<PositionComp>(entity) {
                    position.0 = offset;
                }
            } else {
                self.place_entity_at_absolute(entity, absolute_position);
            }
        } else {
            self.simulation.world.set_ship_anchor(entity, ship.anchor);
            if let Some(mut position) = self.simulation.world.get_mut::<PositionComp>(entity) {
                position.0 = ship.position;
            }
        }
        if self.simulation.world.get::<VelocityComp>(entity).is_some() {
            self.simulation
                .world
                .get_mut::<VelocityComp>(entity)
                .unwrap()
                .0 = ship.velocity;
        } else {
            let _ = self
                .simulation
                .world
                .insert_one(entity, VelocityComp(ship.velocity));
        }

        let fitting = if state.fitting_present {
            let mut fitting =
                FittingComp::from_snapshot(&ship.fitting, &self.game_data.module_registry);
            if !Self::restore_tick_fitting(&mut fitting, &state.fitting) {
                return Err(TransitionApplyError::InvalidTickShipState(ship.ship_id));
            }
            Some(fitting)
        } else {
            None
        };

        // One write of every optional component (ADR-0049, issue #312):
        // capacitor/tackled/inventory/fitting come from `ship` (the durable
        // ShipSnapshot); movement/lock/weapon come from this Tick's extras.
        // Replaces what used to be up to eleven individual
        // remove_one/insert_one pairs with one call.
        self.simulation.world.restore_optional_components(
            entity,
            &dawn_ecs::OptionalShipComponents {
                capacitor: ship.capacitor.map(|current| CapacitorComp { current }),
                weapon: state
                    .weapon_last_fired_tick
                    .map(|last_fired_tick| WeaponComp { last_fired_tick }),
                lock: state.locks.as_ref().map(|lock| LockComp {
                    entries: lock
                        .entries
                        .iter()
                        .map(|entry| LockEntry {
                            target_id: entry.target_id,
                            state: match entry.state {
                                TickLockPhase::Locking { remaining_ticks } => {
                                    LockState::Locking { remaining_ticks }
                                }
                                TickLockPhase::Locked => LockState::Locked,
                            },
                        })
                        .collect(),
                }),
                fitting,
                inventory: state.inventory_present.then(|| InventoryComp {
                    items: ship.inventory.clone(),
                }),
                approach: state.movement.approach.map(|approach| ApproachComp {
                    target: approach.target,
                    auto_jump_gate: approach.auto_jump_gate,
                }),
                orbit: state.movement.orbit.map(|orbit| OrbitComp {
                    target: orbit.target,
                    radius: orbit.radius,
                }),
                keep_at_range: state
                    .movement
                    .keep_at_range
                    .map(|keep_at_range| KeepAtRangeComp {
                        target: keep_at_range.target,
                        range: keep_at_range.range,
                    }),
                warp: state.movement.warp.map(|warp| WarpComp {
                    target: warp.target,
                    phase: match warp.phase {
                        TickWarpPhase::Aligning => WarpPhase::Aligning,
                        TickWarpPhase::Warping => WarpPhase::Warping,
                    },
                    auto_jump: warp.auto_jump,
                    warp_start_abs: warp.warp_start_abs,
                    warp_total: warp.warp_total,
                    warp_elapsed: warp.warp_elapsed,
                    warp_arrival_abs: warp.warp_arrival_abs,
                    warp_start_vel: warp.warp_start_vel,
                }),
                tackled: (!ship.tackled_by.is_empty()).then(|| TackledComp {
                    tacklers: ship.tackled_by.clone(),
                }),
                thrust: state
                    .movement
                    .thrust
                    .map(|thrust| dawn_ecs::components::ThrustComp {
                        direction: thrust.direction,
                        is_braking: thrust.is_braking,
                    }),
            },
        );

        if state.fitting_present {
            // reapply_fitting recomputes ShipStatsComp and rescales HullComp
            // (max shield/armor/hull may have changed); restore the exact HP
            // layers afterwards, same as restore_ship_from_snapshot does.
            self.reapply_fitting(ship.ship_id);
        }
        if let Some(mut hull) = self.simulation.world.get_mut::<HullComp>(entity) {
            hull.set_hp(ship.current_shield, ship.current_armor, ship.current_hull);
        }

        Ok(())
    }

    fn restore_tick_write_set(
        &mut self,
        states: &BTreeMap<ShipId, ShipState>,
    ) -> Result<(), TransitionApplyError> {
        for state in states.values() {
            self.apply_tick_ship_state(state)?;
        }
        Ok(())
    }

    pub fn tick_with_lock_commands(
        &mut self,
        lock_commands: &[dawn_core::LockOnCommand],
    ) -> TickResult {
        self.tick_with_lock_commands_mode(lock_commands, true, true)
    }

    fn tick_with_lock_commands_mode(
        &mut self,
        lock_commands: &[dawn_core::LockOnCommand],
        publish_events: bool,
        remove_destroyed: bool,
    ) -> TickResult {
        self.simulation.current_tick = self.simulation.current_tick.next();
        let tick = self.simulation.current_tick;
        let (transit_ids, frozen_transit, frozen_related_locks) = self.freeze_transit_components();

        // 2.5 Approach System — re-aim thrust at approach targets (ADR-0015)
        self.process_approach();

        // 2.55 Orbit System — sweep around the target at a chosen radius (ADR-0031)
        self.process_orbit();

        // 2.56 Keep at Range System — hold at least a chosen range from the target (ADR-0031)
        self.process_keep_at_range();

        // 2.6 Warp System — advance intra-Sector warps (short-range Fold, ADR-0022)
        let warp_events = self.process_warp(tick);

        // 3. Movement System (skips ships in the committed warping phase)
        let move_events = MovementSystem::run(&mut self.simulation.world, tick);

        // 4. Capacitor System — recharge and drain for active modules
        let cap = CapacitorSystem(&mut self.simulation.world, tick);

        // Drain pending bot lock commands and merge with human-issued ones.
        let bot_locks: Vec<dawn_core::LockOnCommand> =
            std::mem::take(&mut self.simulation.pending_bot_lock_commands);
        // Re-apply fitting for any ship whose module was force-deactivated.
        for ship_id in &cap.refitted {
            self.reapply_fitting(*ship_id);
        }

        // 4.5 Tackle System — update TackledComp for active Tackle modules (ADR-0024)
        let tackle_events: Vec<_> = self
            .process_tackle(tick)
            .into_iter()
            .filter(|event| !transit_ids.contains(&event.ship_id()))
            .collect();

        // 5. Lock System — merge human commands with queued bot commands. A
        // pending Transit can neither acquire a lock nor become a new target.
        let merged_locks: Vec<dawn_core::LockOnCommand> = bot_locks
            .into_iter()
            .chain(lock_commands.iter().cloned())
            .filter(|command| {
                !transit_ids.contains(&command.ship_id) && !transit_ids.contains(&command.target_id)
            })
            .collect();
        let mut lock = LockSystem(&mut self.simulation.world, tick, &merged_locks);
        lock.events
            .retain(|event| !lock_event_touches_transit(event, &transit_ids));

        // Docked ships are not valid space-combat participants: any lock held
        // by a docked ship, or onto a docked ship, is torn down before range
        // gating and combat run for this tick.
        let docked_lock_lost = self.clear_docked_lock_targets(tick);

        // 5.5 Range Gate System — force OFF targeted modules (Weapon/Tackle)
        // whose target has drifted out of effective range (ADR-0035).
        let range_gate_events = self.process_range_gate(tick);

        // 6. Combat System — fire only when the capacitor weapon cycle started this tick.
        // Pass the anchor table so distances resolve across anchors (ADR-0029).
        let combat = CombatSystem(
            &mut self.simulation.world,
            tick,
            &cap.weapon_cycles_started,
            self.topology.anchor_table.abs_map(),
        );

        // 6.5 Repair System — ignore repair cycles targeting a frozen Ship.
        let repair_cycles: Vec<_> = cap
            .repair_cycles_started
            .iter()
            .copied()
            .filter(|cycle| !transit_ids.contains(&cycle.target_ship_id))
            .collect();
        let repair = RepairSystem(&mut self.simulation.world, tick, &repair_cycles);

        self.restore_transit_components(frozen_transit, frozen_related_locks);

        for event in &combat.events {
            let DomainEvent::ShipDestroyed(destroyed) = event else {
                continue;
            };
            let Some(&killer_entity) = self.simulation.ships.index.get(&destroyed.killer_id) else {
                continue;
            };
            if let Some(mut inventory) = self
                .simulation
                .world
                .get_mut::<dawn_ecs::components::InventoryComp>(killer_entity)
            {
                inventory.add_item(ItemId::ScrapMetal, SCRAP_METAL_PER_SHIP_DESTROYED);
            }
        }

        // Remove destroyed ships from the ECS and all lookup maps.
        // CLAUDE.md §6: run the Bot System after Combat.
        if remove_destroyed {
            for ship_id in &combat.destroyed {
                self.remove_ship(*ship_id);
            }
        }

        // 7. Bot System — bot commands pass through the same InTransit guards.
        self.process_bots();

        // 8. Collect public output for the runtime-owned journal/publisher.
        let all_events: Vec<DomainEvent> = warp_events
            .iter()
            .chain(move_events.iter())
            .chain(cap.events.iter())
            .chain(tackle_events.iter())
            .chain(lock.events.iter())
            .chain(docked_lock_lost.iter())
            .chain(range_gate_events.iter())
            .chain(combat.events.iter())
            .chain(repair.events.iter())
            .cloned()
            .collect();

        let count = all_events.len();
        if publish_events {
            self.emit_events(all_events.iter().cloned());
        }

        TickResult {
            tick,
            events_emitted: count,
            events: all_events,
            cap_depletions: cap.refitted.clone(),
            market_settlement_outcomes: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use dawn_core::{
        commands::TransitCommand, NodeId, Position, SectorBounds, SectorId, Tick, Velocity,
    };
    use dawn_ecs::components::{LockEntry, LockState};
    use dawn_storage::{
        AppendReceipt, DurabilityMode, DurableJournal, InMemoryJournal, JournalBatch, JournalError,
        JournalIndex, JournalRecord,
    };

    fn mem_node() -> SimulationNode {
        SimulationNode::new_test(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        )
    }

    #[test]
    fn tick_advances_the_logical_tick_counter_by_one() {
        let mut node = mem_node();
        assert_eq!(node.current_tick(), Tick::ZERO);
        node.tick();
        assert_eq!(node.current_tick(), Tick(1));
        node.tick();
        assert_eq!(node.current_tick(), Tick(2));
    }

    #[test]
    fn tick_transition_appends_before_advancing_the_counter() {
        let mut node = mem_node();
        let mut journal = InMemoryJournal::new();

        let receipt = crate::transit::commit_tick_transition(
            &mut node,
            &mut journal,
            SectorTransitionId(20),
            4,
            DurabilityMode::Synced,
        )
        .expect("durable Tick should apply");

        assert_eq!(receipt.range.first, JournalIndex::ZERO);
        assert_eq!(node.current_tick(), Tick(1));
        assert_eq!(journal.records().len(), 1);
        assert!(matches!(
            journal.records()[0].stream,
            dawn_storage::JournalStream::RecoveryDelta
        ));
    }

    /// issue #315: a Market settlement admitted through `FrameInput` must
    /// apply inside Tick preparation's rollback window -- live state stays
    /// untouched until a commit actually applies the delta -- while still
    /// being captured in the prepared delta so a durable commit persists it.
    #[test]
    fn market_settlement_input_applies_within_prepare_rollback_window() {
        let mut node = mem_node();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship(player_id);
        let entity = *node.simulation.ships.index.get(&ship_id).unwrap();

        let settlement =
            crate::transition::MarketSettlementInput::CreditItem(dawn_core::CreditItemCommand {
                player_id,
                ship_id,
                item_id: dawn_core::ItemId::ScrapMetal,
                quantity: 7,
                settlement_id: 42,
            });
        let market_settlements = [settlement];
        let input = crate::transition::FrameInput {
            lock_commands: &[],
            market_settlements: &market_settlements,
        };

        let before_count = node
            .simulation
            .world
            .get::<dawn_ecs::components::InventoryComp>(entity)
            .map(|inv| inv.item_count(dawn_core::ItemId::ScrapMetal))
            .unwrap_or(0);

        let (prepared, result) = node
            .prepare_tick_state_transition_with_result(input, SectorTransitionId(30), 4)
            .expect("full Tick with a Market settlement should be preparable");

        assert_eq!(
            result.market_settlement_outcomes,
            vec![crate::transition::MarketSettlementOutcome {
                settlement_id: 42,
                applied: true,
            }],
            "the settlement outcome must be reported so the runtime can ACK it"
        );

        // Live state must be untouched: prepare is a dry run.
        let after_prepare_count = node
            .simulation
            .world
            .get::<dawn_ecs::components::InventoryComp>(entity)
            .map(|inv| inv.item_count(dawn_core::ItemId::ScrapMetal))
            .unwrap_or(0);
        assert_eq!(
            after_prepare_count, before_count,
            "Tick preparation must not mutate live state -- the settlement's \
             effect must be rolled back with everything else until commit"
        );
        assert!(
            !node.simulation.applied_market_settlements.contains(&42),
            "the dedup guard must not be live either before a commit"
        );

        // The prepared delta must carry the settlement's effect so a durable
        // commit actually persists it.
        let SectorRecoveryDelta::Tick(delta) = &prepared.recovery_delta else {
            panic!("full Tick must produce a Tick recovery delta");
        };
        assert!(
            delta.applied_market_settlements.contains(&42),
            "the prepared delta must carry the settlement dedup guard"
        );
        let ship_state = delta
            .ship_states
            .iter()
            .find(|state| state.snapshot.ship_id == ship_id)
            .expect("the credited ship must be in the prepared write set");
        assert_eq!(
            ship_state
                .snapshot
                .inventory
                .get(&dawn_core::ItemId::ScrapMetal)
                .copied()
                .unwrap_or(0),
            before_count + 7,
            "the prepared delta must carry the credited item count"
        );

        // A durable commit must actually apply it to live state.
        let mut journal = InMemoryJournal::new();
        crate::transit::commit_tick_state_transition(
            &mut node,
            &mut journal,
            input,
            SectorTransitionId(31),
            4,
            DurabilityMode::Synced,
        )
        .expect("durable full Tick with a Market settlement should apply");
        let after_commit_count = node
            .simulation
            .world
            .get::<dawn_ecs::components::InventoryComp>(entity)
            .map(|inv| inv.item_count(dawn_core::ItemId::ScrapMetal))
            .unwrap_or(0);
        assert_eq!(after_commit_count, before_count + 7);
        assert!(node.simulation.applied_market_settlements.contains(&42));
    }

    #[test]
    fn full_tick_prepare_restores_live_state_and_recovery_replays_it() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(10.0, 20.0, 30.0),
            Velocity::new(2.0, 0.0, 0.0),
        );
        let destroyed_ship = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(40.0, 50.0, 60.0),
            Velocity::ZERO,
        );
        let destroyed_entity = *node.simulation.ships.index.get(&destroyed_ship).unwrap();
        node.simulation
            .world
            .get_mut::<dawn_ecs::components::HullComp>(destroyed_entity)
            .unwrap()
            .set_hp(0.0, 0.0, 0.0);
        let before = node.capture_tick_write_set();

        let prepared = node
            .prepare_tick_state_transition(
                crate::transition::FrameInput::lock_only(&[]),
                SectorTransitionId(23),
                4,
            )
            .expect("full Tick should be preparable");

        assert_eq!(node.current_tick(), Tick::ZERO);
        assert_eq!(node.capture_tick_write_set(), before);
        assert_eq!(prepared.context.sector_id, SectorId(0));
        let SectorRecoveryDelta::Tick(delta) = prepared.recovery_delta else {
            panic!("full Tick must produce a Tick recovery delta");
        };
        assert_eq!(delta.from, Tick::ZERO);
        assert_eq!(delta.to, Tick(1));

        let mut journal = InMemoryJournal::new();
        crate::transit::commit_tick_state_transition(
            &mut node,
            &mut journal,
            crate::transition::FrameInput::lock_only(&[]),
            SectorTransitionId(24),
            4,
            DurabilityMode::Synced,
        )
        .expect("durable full Tick should apply");
        let committed = node.capture_tick_write_set();
        assert_eq!(node.current_tick(), Tick(1));

        let record = journal.records()[0].clone();
        let delta = crate::transition::decode_recovery_delta(&record.payload)
            .expect("journal payload should contain the full Tick delta");
        let SectorRecoveryDelta::Tick(delta) = delta else {
            panic!("full Tick must persist a Tick delta");
        };
        let mut recovered = mem_node();
        recovered.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(10.0, 20.0, 30.0),
            Velocity::new(2.0, 0.0, 0.0),
        );
        recovered
            .apply_tick_transition(
                *delta,
                TransitionContext {
                    sector_id: record.context.sector_id,
                    owner_epoch: record.context.owner_epoch,
                },
            )
            .expect("full Tick delta should replay");
        assert_eq!(recovered.current_tick(), Tick(1));
        assert_eq!(recovered.capture_tick_write_set(), committed);
        assert!(recovered.simulation.ships.index.contains_key(&ship_id));
        assert!(!node.simulation.ships.index.contains_key(&destroyed_ship));
        assert!(!recovered
            .simulation
            .ships
            .index
            .contains_key(&destroyed_ship));
    }

    /// ADR-0049 / issue #312: `applied_market_settlements` used to be
    /// present in `StateSnapshot` but missing from `TickRecoveryDelta`, so a
    /// settlement dedup guard survived a checkpoint restore but not a
    /// tick-rollback recovery replay. `capture_node_state`/`restore_node_state`
    /// close that gap for both.
    #[test]
    fn applied_market_settlements_survive_a_full_tick_recovery_replay() {
        let mut node = mem_node();
        node.simulation.applied_market_settlements.insert(99);

        let mut journal = InMemoryJournal::new();
        crate::transit::commit_tick_state_transition(
            &mut node,
            &mut journal,
            crate::transition::FrameInput::lock_only(&[]),
            SectorTransitionId(1),
            4,
            DurabilityMode::Synced,
        )
        .expect("durable full Tick should apply");

        let record = journal.records()[0].clone();
        let delta = crate::transition::decode_recovery_delta(&record.payload)
            .expect("journal payload should contain the full Tick delta");
        assert!(
            matches!(&delta, SectorRecoveryDelta::Tick(delta)
                if delta.applied_market_settlements.contains(&99)),
            "the delta must carry the settlement dedup guard, not just the checkpoint"
        );

        let mut recovered = mem_node();
        recovered
            .apply_recovery_delta(
                delta,
                TransitionContext {
                    sector_id: record.context.sector_id,
                    owner_epoch: record.context.owner_epoch,
                },
            )
            .expect("full Tick delta should replay");
        assert!(
            recovered
                .simulation
                .applied_market_settlements
                .contains(&99),
            "a settlement dedup guard must survive a tick-rollback recovery replay"
        );
    }

    #[test]
    fn failed_full_tick_append_leaves_live_state_and_clock_unchanged() {
        let mut node = mem_node();
        node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(10.0, 20.0, 30.0),
            Velocity::new(2.0, 0.0, 0.0),
        );
        let before = node.capture_tick_write_set();
        let mut journal = FailingJournal;

        assert!(crate::transit::commit_tick_state_transition(
            &mut node,
            &mut journal,
            crate::transition::FrameInput::lock_only(&[]),
            SectorTransitionId(25),
            4,
            DurabilityMode::Synced,
        )
        .is_err());
        assert_eq!(node.current_tick(), Tick::ZERO);
        assert_eq!(node.capture_tick_write_set(), before);
    }

    #[test]
    fn committed_tick_delta_can_be_applied_from_its_journal_record() {
        let mut node = mem_node();
        let mut journal = InMemoryJournal::new();

        crate::transit::commit_tick_transition(
            &mut node,
            &mut journal,
            SectorTransitionId(22),
            4,
            DurabilityMode::Synced,
        )
        .expect("durable Tick should apply");

        let record = journal.records()[0].clone();
        let delta = crate::transition::decode_recovery_delta(&record.payload)
            .expect("journal payload should contain a Tick delta");
        let SectorRecoveryDelta::Tick(delta) = delta else {
            panic!("logical Tick transition must persist a Tick delta");
        };

        let mut recovered = mem_node();
        recovered
            .apply_tick_transition(
                *delta,
                TransitionContext {
                    sector_id: record.context.sector_id,
                    owner_epoch: record.context.owner_epoch,
                },
            )
            .expect("the journal record should recover on the matching Sector");

        assert_eq!(recovered.current_tick(), Tick(1));
    }

    #[test]
    fn failed_tick_append_does_not_advance_the_counter() {
        let mut node = mem_node();
        let mut journal = FailingJournal;

        assert!(crate::transit::commit_tick_transition(
            &mut node,
            &mut journal,
            SectorTransitionId(21),
            4,
            DurabilityMode::Synced,
        )
        .is_err());
        assert_eq!(node.current_tick(), Tick::ZERO);
    }

    #[test]
    fn stale_tick_delta_is_rejected_without_changing_the_counter() {
        let mut node = mem_node();

        let error = node
            .apply_tick_transition(
                TickRecoveryDelta::logical(Tick(1), Tick(2)),
                TransitionContext {
                    sector_id: SectorId(0),
                    owner_epoch: 4,
                },
            )
            .expect_err("a delta from a future Tick must be rejected");

        assert_eq!(
            error,
            TransitionApplyError::TickMismatch {
                expected: Tick(1),
                actual: Tick::ZERO,
            }
        );
        assert_eq!(node.current_tick(), Tick::ZERO);
    }

    #[test]
    fn invalid_tick_step_is_rejected_without_changing_the_counter() {
        let mut node = mem_node();

        let error = node
            .apply_tick_transition(
                TickRecoveryDelta::logical(Tick::ZERO, Tick(2)),
                TransitionContext {
                    sector_id: SectorId(0),
                    owner_epoch: 4,
                },
            )
            .expect_err("a Tick delta must advance exactly one step");

        assert_eq!(
            error,
            TransitionApplyError::InvalidTickStep {
                from: Tick::ZERO,
                to: Tick(2),
            }
        );
        assert_eq!(node.current_tick(), Tick::ZERO);
    }

    #[test]
    fn tick_recovery_rejects_a_delta_from_another_sector() {
        let mut node = mem_node();

        let error = node
            .apply_tick_transition(
                TickRecoveryDelta::logical(Tick::ZERO, Tick(1)),
                TransitionContext {
                    sector_id: SectorId(99),
                    owner_epoch: 4,
                },
            )
            .expect_err("a delta for another Sector must be rejected");

        assert_eq!(
            error,
            TransitionApplyError::SectorMismatch {
                expected: SectorId(0),
                actual: SectorId(99),
            }
        );
        assert_eq!(node.current_tick(), Tick::ZERO);
    }

    struct FailingJournal;

    impl DurableJournal for FailingJournal {
        fn append_batch(&mut self, _batch: JournalBatch) -> Result<AppendReceipt, JournalError> {
            Err(JournalError::Io(std::io::Error::other(
                "injected append failure",
            )))
        }

        fn read_from(
            &self,
            _index: JournalIndex,
        ) -> Result<Box<dyn Iterator<Item = Result<JournalRecord, JournalError>> + '_>, JournalError>
        {
            Ok(Box::new(std::iter::empty()))
        }

        fn next_index(&self) -> Result<JournalIndex, JournalError> {
            Ok(JournalIndex::ZERO)
        }
    }

    #[test]
    fn npc_ships_at_constant_velocity_produce_no_velocity_changed_events() {
        let mut node = mem_node();
        node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(100.0, 100.0, 100.0),
            Velocity::new(1.0, 0.0, 0.0),
        );
        node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(200.0, 100.0, 100.0),
            Velocity::new(0.0, 1.0, 0.0),
        );
        assert_eq!(
            node.tick().events_emitted,
            0,
            "NPC ships at constant velocity do not emit VelocityChanged"
        );
    }

    #[test]
    fn stationary_ships_produce_no_events() {
        let mut node = mem_node();
        node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        assert_eq!(node.tick().events_emitted, 0);
    }

    #[test]
    fn in_transit_ship_keeps_snapshot_state_across_ticks() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(100.0, 0.0, 0.0),
            Velocity::new(10.0, 0.0, 0.0),
        );
        let entity = *node.simulation.ships.index.get(&ship_id).unwrap();
        node.simulation
            .world
            .get_mut::<CapacitorComp>(entity)
            .unwrap()
            .current = 1.0;
        let _ = node.simulation.world.insert_one(
            entity,
            TackledComp {
                tacklers: vec![dawn_core::ShipId::new(NodeId(9), 1)],
            },
        );
        node.propose_transit(TransitCommand {
            ship_id,
            to: SectorId(1),
        })
        .unwrap();

        let before = node
            .take_snapshot()
            .ships
            .into_iter()
            .find(|ship| ship.snapshot.ship_id == ship_id)
            .unwrap()
            .snapshot;
        let tick_result = node.tick();
        let after = node
            .take_snapshot()
            .ships
            .into_iter()
            .find(|ship| ship.snapshot.ship_id == ship_id)
            .unwrap()
            .snapshot;

        assert_eq!(after.position, before.position);
        assert_eq!(after.velocity, before.velocity);
        assert_eq!(after.current_shield, before.current_shield);
        assert_eq!(after.current_armor, before.current_armor);
        assert_eq!(after.current_hull, before.current_hull);
        assert_eq!(after.capacitor, before.capacitor);
        assert_eq!(after.inventory, before.inventory);
        assert_eq!(after.fitting, before.fitting);
        assert_eq!(after.tackled_by, before.tackled_by);
        assert!(tick_result.events.iter().all(|event| !matches!(
            event,
            DomainEvent::TackleApplied(_)
                | DomainEvent::TackleReleased(_)
                | DomainEvent::TargetLocked(_)
                | DomainEvent::LockLost(_)
        )));
    }

    #[test]
    fn in_transit_target_freezes_existing_lock_progress_and_events() {
        let mut node = mem_node();
        let locker_player = node.next_player_id();
        let locker = node.spawn_player_ship_at_pub(locker_player, Position::ORIGIN);
        let target_player = node.next_player_id();
        let target = node.spawn_player_ship_at_pub(target_player, Position::new(500.0, 0.0, 0.0));
        let locker_entity = *node.simulation.ships.index.get(&locker).unwrap();
        node.simulation
            .world
            .get_mut::<LockComp>(locker_entity)
            .unwrap()
            .entries
            .push(LockEntry {
                target_id: target,
                state: LockState::Locking { remaining_ticks: 1 },
            });
        node.propose_transit(TransitCommand {
            ship_id: target,
            to: SectorId(1),
        })
        .unwrap();

        let result = node.tick();
        let lock = node
            .simulation
            .world
            .get::<LockComp>(locker_entity)
            .unwrap();
        assert_eq!(lock.entries.len(), 1);
        assert!(matches!(
            lock.entries[0].state,
            LockState::Locking { remaining_ticks: 1 }
        ));
        assert!(result.events.iter().all(|event| match event {
            DomainEvent::TargetLocked(event) => {
                event.locker_id != locker || event.target_id != target
            }
            DomainEvent::LockLost(event) => {
                event.locker_id != locker || event.target_id != target
            }
            _ => true,
        }));
    }

    #[test]
    fn velocity_changed_events_carry_the_current_tick_value() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(100.0, 100.0, 100.0),
            Velocity::ZERO,
        );
        node.set_player_ship(ship_id);
        node.apply_move_command(ship_id, Position::new(10000.0, 0.0, 0.0));
        node.tick();
        node.tick();
        let last = node.pending_events().last().unwrap();
        assert_eq!(last.tick(), Tick(2));
    }

    #[test]
    fn ship_destroyed_immediately_credits_scrap_metal_to_the_killer_inventory() {
        let mut node = mem_node();

        let killer_player = node.next_player_id();
        let killer = node.spawn_player_ship_at_pub(killer_player, Position::ORIGIN);
        let (_bot_player, victim) = node.spawn_bot_ship(Position::new(500.0, 0.0, 0.0));
        let killer_entity = *node.simulation.ships.index.get(&killer).unwrap();
        let before = node
            .simulation
            .world
            .get::<dawn_ecs::components::InventoryComp>(killer_entity)
            .unwrap()
            .item_count(ItemId::ScrapMetal);

        let victim_entity = *node.simulation.ships.index.get(&victim).unwrap();
        if let Some(mut hull) = node
            .simulation
            .world
            .get_mut::<dawn_ecs::components::HullComp>(victim_entity)
        {
            hull.set_hp(0.0, 0.0, 1.0);
        }

        let lock_cmd = dawn_core::LockOnCommand {
            ship_id: killer,
            target_id: victim,
        };
        for _ in 0..5 {
            node.tick_with_lock_commands(std::slice::from_ref(&lock_cmd));
        }
        assert!(
            node.activate_module_owned(
                killer_player,
                killer,
                dawn_core::ActivateModuleCommand {
                    module_id: dawn_core::ModuleId(1),
                    slot: dawn_core::SlotKind::High,
                    target_ship_id: Some(victim),
                }
            )
            .is_ok(),
            "weapon activation should succeed once the lock has completed"
        );

        let destroyed = (0..25).any(|_| {
            node.tick_with_lock_commands(std::slice::from_ref(&lock_cmd))
                .events
                .iter()
                .any(
                    |e| matches!(e, DomainEvent::ShipDestroyed(d) if d.ship_id == victim && d.killer_id == killer),
                )
        });
        assert!(
            destroyed,
            "combat should eventually destroy the victim ship"
        );
        let after = node
            .simulation
            .world
            .get::<dawn_ecs::components::InventoryComp>(killer_entity)
            .unwrap()
            .item_count(ItemId::ScrapMetal);
        assert_eq!(after, before + SCRAP_METAL_PER_SHIP_DESTROYED);
    }
}
