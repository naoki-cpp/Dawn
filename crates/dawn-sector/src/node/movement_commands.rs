//! Player Move/Stop command policy for [`SimulationNode`].
//!
//! This module owns the shared admission rules for direct movement commands:
//! docked, in-transit, and committed-warp ships cannot be steered; an aligning
//! warp may be cancelled by Move/Stop; and a manual command clears any
//! persistent steering mode before updating thrust. Approach, Orbit, and Keep
//! at Range reuse the thrust helpers here without becoming part of this
//! command module's interface.

use super::SimulationNode;
use crate::transition::{
    PreparedSectorTransition, SectorEngine, SectorRecoveryDelta, SectorTransitionId,
    StopCommandState, StopRecoveryDelta, TransitionApplyError, TransitionContext, TransitionError,
};
use dawn_core::{PlayerId, Position, ShipId, Velocity};
use dawn_ecs::{
    components::{PositionComp, ThrustComp, WarpComp},
    Entity,
};

impl SimulationNode {
    fn stop_command_state(&self, ship_id: ShipId) -> StopCommandState {
        let Some(&entity) = self.ships.index.get(&ship_id) else {
            return StopCommandState {
                ship_id,
                exists: false,
                is_docked: false,
                is_in_transit: false,
                is_warping: false,
            };
        };

        StopCommandState {
            ship_id,
            exists: true,
            is_docked: self.is_ship_docked(ship_id),
            is_in_transit: self.world.transit_state(entity).is_in_transit(),
            is_warping: self.is_warping(entity),
        }
    }

    /// Prepare Stop without changing the live ECS world.
    pub fn prepare_stop_transition(
        &self,
        ship_id: ShipId,
        transition_id: SectorTransitionId,
        owner_epoch: u64,
    ) -> Result<PreparedSectorTransition, TransitionError> {
        SectorEngine::prepare_stop(
            self.stop_command_state(ship_id),
            transition_id,
            TransitionContext {
                sector_id: self.sector_id,
                owner_epoch,
            },
        )
    }

    /// Apply the exact Stop delta after its durable transition is committed.
    pub fn apply_stop_transition(
        &mut self,
        delta: StopRecoveryDelta,
    ) -> Result<(), TransitionApplyError> {
        let entity = self.stop_entity(delta.ship_id)?;
        self.apply_stop_delta(entity, delta);
        Ok(())
    }

    pub(crate) fn stop_entity(&self, ship_id: ShipId) -> Result<Entity, TransitionApplyError> {
        self.ships
            .index
            .get(&ship_id)
            .copied()
            .ok_or(TransitionApplyError::UnknownShip(ship_id))
    }

    pub(crate) fn apply_stop_delta(&mut self, entity: Entity, delta: StopRecoveryDelta) {
        if delta.clear_warp {
            let _ = self.world.remove_one::<WarpComp>(entity);
        }
        if delta.clear_steering {
            self.clear_steering_modes(entity);
        }
        if let Some(mut thrust) = self.world.get_mut::<ThrustComp>(entity) {
            thrust.direction = delta.thrust.direction;
            thrust.is_braking = delta.thrust.is_braking;
        }
    }

    /// Steer `ship_id` toward `target`. Cancels any active warp/approach.
    /// No-op if the ship is unknown, in transit, or in committed warp.
    pub fn apply_move_command(&mut self, ship_id: ShipId, target: Position) {
        if self.is_ship_docked(ship_id) {
            return;
        }
        let entity = match self.ships.index.get(&ship_id) {
            Some(&e) => e,
            None => return,
        };
        if self.world.transit_state(entity).is_in_transit() {
            return;
        }
        // A committed warp cannot be interrupted; an aligning warp is cancelled
        // (ADR-0022 §7).
        if self.is_warping(entity) {
            return;
        }
        let _ = self.world.remove_one::<WarpComp>(entity);
        // Manual thrust overrides any active steering mode (Approach ADR-0015
        // §4, Orbit / Keep at Range ADR-0031).
        self.clear_steering_modes(entity);
        let pos = match self.world.get::<PositionComp>(entity) {
            Some(c) => c.0,
            None => return,
        };
        let target = self.dest_in_ship_frame_abs(entity, [target.x, target.y, target.z].into());
        self.steer_thrust_toward(entity, pos, target);
    }

    /// `apply_move_command` wrapped with an active-ship check (ADR-0037: only
    /// the caller's active ship can be flown).
    ///
    /// Unlike the other flight commands, Move/Stop do *not* reject a docked
    /// ship at this layer -- `apply_move_command` already no-ops on a docked
    /// ship internally and this wrapper's return value has always meant
    /// "the caller's active ship" rather than "the command took effect", so
    /// this only checks ownership, not dock state (`resolve_flight_command`
    /// would additionally reject on dock state, which would flip this
    /// method's return value for a docked ship and break that contract).
    pub fn apply_move_command_owned(
        &mut self,
        player_id: PlayerId,
        ship_id: ShipId,
        target: Position,
    ) -> bool {
        if !self.is_active_ship(player_id, ship_id) {
            return false;
        }
        self.apply_move_command(ship_id, target);
        true
    }

    /// Begin decelerating the ship toward zero velocity using its thrust.
    ///
    /// The movement system applies thrust opposite to velocity each tick until
    /// the ship stops. Cancels any active thrust direction.
    pub fn apply_stop_command(&mut self, ship_id: ShipId) {
        let Ok(prepared) = self.prepare_stop_transition(ship_id, SectorTransitionId(0), 0) else {
            return;
        };
        let SectorRecoveryDelta::Stop(delta) = prepared.recovery_delta else {
            unreachable!("prepare_stop_transition always produces a Stop delta");
        };
        let _ = self.apply_stop_transition(delta);
    }

    /// `apply_stop_command` wrapped with an active-ship check (ADR-0037).
    pub fn apply_stop_command_owned(&mut self, player_id: PlayerId, ship_id: ShipId) -> bool {
        if !self.is_active_ship(player_id, ship_id) {
            return false;
        }
        self.apply_stop_command(ship_id);
        true
    }

    /// True if the ship is in the committed warping phase (ADR-0022): its warp
    /// cannot be interrupted by Move/Stop. Aligning or absent warp -> false.
    /// Used only by `apply_move_command`, which is the one command allowed to
    /// cancel an aligning warp outright (ADR-0022 §7) -- every other steering
    /// command must use `has_active_warp` instead, which also covers the
    /// aligning phase.
    pub(super) fn is_warping(&self, entity: Entity) -> bool {
        self.world
            .get::<WarpComp>(entity)
            .map(|w| w.is_warping())
            .unwrap_or(false)
    }

    /// True if `entity` has a `WarpComp` in any phase, aligning or committed
    /// (ADR-0022/ADR-0031). Warp takes priority over Approach / Orbit / Keep at
    /// Range: a new steering command must not silently race an in-progress
    /// warp, whether or not it has engaged yet.
    pub(super) fn has_active_warp(&self, entity: Entity) -> bool {
        self.world.get::<WarpComp>(entity).is_some()
    }

    /// Point `entity`'s thrust at `to` from `from` (unit direction, not
    /// braking). Zero thrust if already at the target. Shared by direct Move,
    /// Approach, Orbit, and Keep at Range so the steering math lives in one
    /// place.
    pub(super) fn steer_thrust_toward(&mut self, entity: Entity, from: Position, to: Position) {
        let dx = to.x - from.x;
        let dy = to.y - from.y;
        let dz = to.z - from.z;
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        let dir = if dist > f64::EPSILON {
            Velocity {
                dx: dx / dist,
                dy: dy / dist,
                dz: dz / dist,
            }
        } else {
            Velocity::ZERO
        };
        if let Some(mut t) = self.world.get_mut::<ThrustComp>(entity) {
            t.direction = dir;
            t.is_braking = false;
        }
    }

    /// Set `entity`'s thrust to braking (decelerate toward zero velocity).
    /// Shared by direct Stop and the Approach/Warp steering systems.
    pub(super) fn brake_thrust(&mut self, entity: Entity) {
        if let Some(mut t) = self.world.get_mut::<ThrustComp>(entity) {
            t.direction = Velocity::ZERO;
            t.is_braking = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{AbsolutePosition, AnchorId, ApproachTarget, NodeId, SectorBounds, SectorId};
    use dawn_ecs::components::{ApproachComp, KeepAtRangeComp, OrbitComp, WarpPhase};
    use dawn_event_store::{
        AppendReceipt, DurabilityMode, DurableJournal, InMemoryJournal, JournalBatch, JournalError,
        JournalIndex, JournalRecord, JournalStream,
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
    fn move_command_preserves_direction_in_the_ship_anchor_frame() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let entity = *node.ships.index.get(&ship_id).unwrap();
        let anchor = AnchorId(1);
        let anchor_abs = node.anchor_table().abs(anchor).expect("demo anchor exists");
        let local_pos = Position::new(250.0, 0.0, -100.0);
        node.world.set_ship_anchor(entity, anchor);
        node.world.get_mut::<PositionComp>(entity).unwrap().0 = local_pos;

        let target_abs = Position::new(
            anchor_abs[0] + local_pos.x,
            anchor_abs[1] + local_pos.y + 1_000_000.0,
            anchor_abs[2] + local_pos.z,
        );

        node.apply_move_command(ship_id, target_abs);

        let thrust = node.world.get::<ThrustComp>(entity).unwrap();
        assert!(
            thrust.direction.dy > 0.99,
            "move command should preserve the local +Y intent after an anchor rebase, got {:?}",
            thrust.direction
        );
        assert!(
            thrust.direction.dx.abs() < 0.01 && thrust.direction.dz.abs() < 0.01,
            "move command must not be dominated by the far anchor offset, got {:?}",
            thrust.direction
        );
    }

    #[test]
    fn stop_transition_appends_before_applying_the_live_change() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let entity = *node.ships.index.get(&ship_id).unwrap();
        node.world.get_mut::<ThrustComp>(entity).unwrap().direction = Velocity::new(1.0, 0.0, 0.0);

        let prepared = node
            .prepare_stop_transition(ship_id, SectorTransitionId(9), 4)
            .expect("Stop should be preparable");
        assert!(!node.world.get::<ThrustComp>(entity).unwrap().is_braking);

        let mut journal = InMemoryJournal::new();
        let receipt = crate::transit::commit_stop_transition(
            &mut node,
            &mut journal,
            ship_id,
            prepared.transition_id,
            prepared.context.owner_epoch,
            DurabilityMode::Synced,
        )
        .expect("durable Stop should apply");

        assert_eq!(receipt.range.first, JournalIndex::ZERO);
        assert_eq!(journal.records()[0].stream, JournalStream::RecoveryDelta);
        let thrust = node.world.get::<ThrustComp>(entity).unwrap();
        assert!(thrust.is_braking);
        assert_eq!(thrust.direction, Velocity::ZERO);
    }

    #[test]
    fn committed_stop_reduces_every_declared_movement_field() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let target_id = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(10.0, 0.0, 0.0),
            Velocity::ZERO,
        );
        let entity = *node.ships.index.get(&ship_id).unwrap();
        let target = ApproachTarget::Ship(target_id);

        node.world.insert_one(
            entity,
            ApproachComp {
                target,
                auto_jump_gate: None,
            },
        );
        node.world.insert_one(
            entity,
            OrbitComp {
                target,
                radius: 10.0,
            },
        );
        node.world.insert_one(
            entity,
            KeepAtRangeComp {
                target,
                range: 20.0,
            },
        );
        node.world.insert_one(
            entity,
            WarpComp {
                target: dawn_core::WarpTarget::Body(dawn_core::CelestialBodyId(0)),
                phase: WarpPhase::Aligning,
                auto_jump: false,
                warp_start_abs: AbsolutePosition::ORIGIN,
                warp_total: 0,
                warp_elapsed: 0,
                warp_arrival_abs: AbsolutePosition::ORIGIN,
                warp_start_vel: Velocity::new(3.0, 0.0, 0.0),
            },
        );
        node.world.get_mut::<ThrustComp>(entity).unwrap().direction = Velocity::new(3.0, 0.0, 0.0);

        let mut journal = InMemoryJournal::new();
        crate::transit::commit_stop_transition(
            &mut node,
            &mut journal,
            ship_id,
            SectorTransitionId(11),
            4,
            DurabilityMode::Synced,
        )
        .expect("durable Stop should apply");

        assert!(node.world.get::<WarpComp>(entity).is_none());
        assert!(node.world.get::<ApproachComp>(entity).is_none());
        assert!(node.world.get::<OrbitComp>(entity).is_none());
        assert!(node.world.get::<KeepAtRangeComp>(entity).is_none());
        let thrust = node.world.get::<ThrustComp>(entity).unwrap();
        assert_eq!(thrust.direction, Velocity::ZERO);
        assert!(thrust.is_braking);
    }

    #[test]
    fn failed_stop_append_does_not_mutate_live_state() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let target_id = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(10.0, 0.0, 0.0),
            Velocity::ZERO,
        );
        let entity = *node.ships.index.get(&ship_id).unwrap();
        let target = ApproachTarget::Ship(target_id);
        node.world.insert_one(
            entity,
            ApproachComp {
                target,
                auto_jump_gate: None,
            },
        );
        node.world.insert_one(
            entity,
            OrbitComp {
                target,
                radius: 10.0,
            },
        );
        node.world.insert_one(
            entity,
            KeepAtRangeComp {
                target,
                range: 20.0,
            },
        );
        node.world.get_mut::<ThrustComp>(entity).unwrap().direction = Velocity::new(1.0, 0.0, 0.0);
        let mut journal = FailingJournal;

        assert!(crate::transit::commit_stop_transition(
            &mut node,
            &mut journal,
            ship_id,
            SectorTransitionId(10),
            4,
            DurabilityMode::Synced,
        )
        .is_err());
        let thrust = node.world.get::<ThrustComp>(entity).unwrap();
        assert!(!thrust.is_braking);
        assert_eq!(thrust.direction, Velocity::new(1.0, 0.0, 0.0));
        assert!(node.world.get::<ApproachComp>(entity).is_some());
        assert!(node.world.get::<OrbitComp>(entity).is_some());
        assert!(node.world.get::<KeepAtRangeComp>(entity).is_some());
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
}
