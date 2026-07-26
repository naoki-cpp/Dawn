//! Player Move/Stop command policy for [`SimulationNode`].
//!
//! This module owns the shared admission rules for direct movement commands:
//! docked, in-transit, and committed-warp ships cannot be steered; an aligning
//! warp may be cancelled by Move/Stop; and a manual command clears any
//! persistent steering mode before updating thrust. Approach, Orbit, and Keep
//! at Range reuse the thrust helpers here without becoming part of this
//! command module's interface.

use dawn_core::{PlayerId, Position, ShipId, Velocity};
use dawn_ecs::{
    components::{PositionComp, ThrustComp, WarpComp},
    Entity,
};
use dawn_event_store::store::EventStore;

use super::SimulationNode;

impl<S: EventStore> SimulationNode<S> {
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
        // Stopping cancels any active steering mode (Approach ADR-0015 §4,
        // Orbit / Keep at Range ADR-0031).
        self.clear_steering_modes(entity);
        self.brake_thrust(entity);
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
    use dawn_core::{AnchorId, NodeId, SectorBounds, SectorId};

    fn mem_node() -> SimulationNode {
        SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
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
}
