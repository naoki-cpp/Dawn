//! Approach commands and the per-tick Approach System for `SimulationNode`
//! (semi-automatic piloting, ADR-0015). Split out of `navigation.rs` (ADR-0030
//! review R-1, 2026-06-23): pure move, no behavior change.
//!
//! # Contents
//!
//! - `apply_approach_command` / `apply_approach_command_owned` — attach `ApproachComp`
//! - `process_approach` — Approach System (Step 2.5, ADR-0015)

use dawn_core::{PlayerId, Position, ShipId};
use dawn_ecs::{
    components::{ApproachComp, PositionComp},
    Entity,
};
use dawn_event_store::store::EventStore;

use super::SimulationNode;

impl<S: EventStore> SimulationNode<S> {
    /// Begin approaching a Ship or a Jump Gate (semi-automatic piloting, ADR-0015).
    ///
    /// Attaches an `ApproachComp` so `process_approach()` re-aims thrust at the
    /// target each tick. Rejected (returns `false`, no component attached) if
    /// the ship is unknown or in transit, a `Ship` target is unknown or the
    /// ship itself, or a `Gate` target does not originate in this Sector.
    pub fn apply_approach_command(
        &mut self,
        ship_id: ShipId,
        target: dawn_core::ApproachTarget,
    ) -> bool {
        self.apply_approach_command_with_auto_jump(ship_id, target, None)
    }

    /// Begin approaching toward `target`, optionally queuing an auto-jump on
    /// arrival (used by `apply_jump_with_fallback`'s too-close-to-warp case,
    /// jump.rs). A plain `ApproachCommand` (above) always passes `None`.
    pub(super) fn apply_approach_command_with_auto_jump(
        &mut self,
        ship_id: ShipId,
        target: dawn_core::ApproachTarget,
        auto_jump_gate: Option<dawn_core::JumpGateId>,
    ) -> bool {
        // `begin_maneuver` runs the same rejection checklist (unknown ship /
        // in transit / Warp priority / invalid target) and clears any other
        // active steering mode -- shared with Orbit/Keep at Range. Approach
        // has no stand-off distance, so the resolved one is unused.
        let Some((entity, _)) = self.begin_maneuver(ship_id, target, None) else {
            return false;
        };
        let _ = self.world.insert_one(
            entity,
            ApproachComp {
                target,
                auto_jump_gate,
            },
        );
        true
    }

    /// `apply_approach_command` wrapped with an active-ship check (ADR-0037).
    pub fn apply_approach_command_owned(
        &mut self,
        player_id: PlayerId,
        ship_id: ShipId,
        cmd: dawn_core::ApproachCommand,
    ) -> bool {
        if !self.is_active_ship(player_id, ship_id) {
            return false;
        }
        if self.is_ship_docked(ship_id) {
            return false;
        }
        self.apply_approach_command(ship_id, cmd.target)
    }

    /// Approach System (ADR-0015 §3): for every ship carrying an `ApproachComp`,
    /// re-aim thrust at the target's latest position, or brake on arrival.
    ///
    /// Runs each tick just before the Movement System so the refreshed thrust
    /// takes effect the same tick. Mirrors the Bot AI steering (`process_bots`)
    /// but is driven by a player-issued `ApproachCommand`.
    ///
    /// Removes the component (and brakes) if the target no longer exists.
    pub fn process_approach(&mut self) {
        use dawn_core::ApproachTarget;
        /// Stop and hold once within this distance of a Ship target (units).
        const SHIP_ARRIVAL_RADIUS: f64 = 500.0;

        // Collect approachers up front (entity, target, current position) so the
        // ECS query borrow is released before the mutable write pass below.
        let approachers: Vec<(
            Entity,
            dawn_core::ApproachTarget,
            Option<dawn_core::JumpGateId>,
            Position,
        )> = self
            .world
            .query::<(&ApproachComp, &PositionComp)>()
            .iter()
            .map(|(entity, (approach, pos))| {
                (entity, approach.target, approach.auto_jump_gate, pos.0)
            })
            .collect();

        for (entity, target, auto_jump_gate, ship_offset) in approachers {
            // Work in the approacher's CURRENT anchor frame (small numbers),
            // not absolute Sector-frame Position (ADR-0029): composing the
            // `ship_offset` is already anchor-relative. The target's f64
            // absolute position is brought into the same frame via
            // `dest_in_ship_frame_abs`, so the tight arrival-radius comparison
            // stays in f64 throughout.
            let ship_pos = ship_offset;
            // Resolve the target's current position and the arrival distance.
            // `None` means the target no longer exists.
            let resolved: Option<(Position, f64)> = match target {
                ApproachTarget::Ship(target_id) => self
                    .ships
                    .index
                    .get(&target_id)
                    .copied()
                    .and_then(|te| self.world.get::<PositionComp>(te).map(|p| (te, p.0)))
                    .map(|(te, off)| {
                        let target_abs = self.entity_absolute_f64(te, off);
                        (
                            self.dest_in_ship_frame_abs(entity, target_abs),
                            SHIP_ARRIVAL_RADIUS,
                        )
                    }),
                // Stop comfortably inside the gate's activation radius so the
                // jump prompt becomes available on arrival (ADR-0015).
                ApproachTarget::Gate(gate_id) => self.jump_gate(gate_id).map(|g| {
                    (
                        self.dest_in_ship_frame_abs(entity, g.abs_m),
                        g.activation_radius * 0.8,
                    )
                }),
            };

            match resolved {
                // Target gone: drop the approach and brake (ADR-0015 §4).
                None => {
                    let _ = self.world.remove_one::<ApproachComp>(entity);
                    self.brake_thrust(entity);
                }
                // Arrived: hold position, keep ApproachComp so the ship resumes
                // if a Ship target later drifts back out of range.
                Some((tp, arrival)) if ship_pos.distance(tp) <= arrival => {
                    if let Some(gate_id) = auto_jump_gate {
                        if let Some((&ship_id, _)) =
                            self.ships.index.iter().find(|(_, &e)| e == entity)
                        {
                            self.pending_auto_jumps.push((ship_id, gate_id));
                        }
                        let _ = self.world.remove_one::<ApproachComp>(entity);
                    }
                    self.brake_thrust(entity)
                }
                // Still closing: steer toward the target's latest position.
                Some((tp, _)) => self.steer_thrust_toward(entity, ship_pos, tp),
            }
        }
    }

    // `dest_in_ship_frame_abs` (Sector-frame f64 point -> ship's anchor frame)
    // moved to `node/mod.rs`, alongside the rest of the anchor-composition
    // family (`entity_absolute_f64`/`entity_absolute`/`ship_absolute`) — it's
    // called from here, `commands.rs`, `orbit.rs`, and `warp.rs`, so having
    // its one implementation live in a single submodule was arbitrary.
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{NodeId, PlayerId, Position, SectorBounds, SectorId, ShipId, Velocity};
    use dawn_ecs::components::ThrustComp;

    fn mem_node() -> SimulationNode {
        SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        )
    }

    fn spawn_owned_player_at(node: &mut SimulationNode, pos: Position) -> (PlayerId, ShipId) {
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship_at_pub(player_id, pos);
        (player_id, ship_id)
    }

    #[test]
    fn approach_command_attaches_an_approach_target_to_the_owned_ship() {
        let mut node = mem_node();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(10_000.0, 0.0, 0.0),
            Velocity::ZERO,
        );

        assert!(node.apply_approach_command_owned(
            player,
            chaser,
            dawn_core::ApproachCommand {
                target: dawn_core::ApproachTarget::Ship(target),
            }
        ));
        assert_eq!(
            node.approach_target(chaser),
            Some(dawn_core::ApproachTarget::Ship(target))
        );
    }

    #[test]
    fn approach_command_is_rejected_while_aligning_to_warp() {
        // Warp takes priority over Approach (ADR-0031) from the moment it's
        // issued, not just once it commits (ADR-0022) -- `has_active_warp`
        // covers the Aligning phase too, unlike `is_warping`.
        let mut node = mem_node();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(10_000.0, 0.0, 0.0),
            Velocity::ZERO,
        );

        assert!(node.apply_warp_command(
            chaser,
            dawn_core::WarpTarget::Gate(dawn_core::JumpGateId(0)),
            false
        ));
        assert_eq!(
            node.warp_phase(chaser),
            Some(dawn_ecs::components::WarpPhase::Aligning),
            "warp should still be aligning, not committed"
        );

        assert!(!node.apply_approach_command_owned(
            player,
            chaser,
            dawn_core::ApproachCommand {
                target: dawn_core::ApproachTarget::Ship(target),
            }
        ));
        assert_eq!(node.approach_target(chaser), None);
    }

    #[test]
    fn approach_command_is_rejected_while_warping() {
        // Warp takes priority over Approach (ADR-0031), mirroring Orbit/Keep
        // at Range: a committed warp must not be interrupted by a new
        // steering mode racing in underneath it.
        let mut node = mem_node();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(10_000.0, 0.0, 0.0),
            Velocity::ZERO,
        );

        assert!(node.apply_warp_command(
            chaser,
            dawn_core::WarpTarget::Gate(dawn_core::JumpGateId(0)),
            false
        ));
        for _ in 0..500 {
            node.tick();
            if node.warp_phase(chaser) == Some(dawn_ecs::components::WarpPhase::Warping) {
                break;
            }
        }
        assert_eq!(
            node.warp_phase(chaser),
            Some(dawn_ecs::components::WarpPhase::Warping),
            "warp should have engaged by now"
        );

        assert!(!node.apply_approach_command_owned(
            player,
            chaser,
            dawn_core::ApproachCommand {
                target: dawn_core::ApproachTarget::Ship(target),
            }
        ));
        assert_eq!(node.approach_target(chaser), None);
    }

    #[test]
    fn approach_command_is_rejected_for_a_ship_the_player_does_not_own() {
        let mut node = mem_node();
        let (_owner, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(10_000.0, 0.0, 0.0),
            Velocity::ZERO,
        );

        let stranger = node.next_player_id();
        assert!(!node.apply_approach_command_owned(
            stranger,
            chaser,
            dawn_core::ApproachCommand {
                target: dawn_core::ApproachTarget::Ship(target),
            }
        ));
        assert_eq!(node.approach_target(chaser), None);
    }

    #[test]
    fn approaching_ship_steers_thrust_toward_its_target_each_tick() {
        let mut node = mem_node();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(10_000.0, 0.0, 0.0),
            Velocity::ZERO,
        );
        node.apply_approach_command_owned(
            player,
            chaser,
            dawn_core::ApproachCommand {
                target: dawn_core::ApproachTarget::Ship(target),
            },
        );

        let entity = *node.ships.index.get(&chaser).unwrap();
        node.process_approach();
        let thrust = node.world.get::<ThrustComp>(entity).unwrap();
        assert!(
            thrust.direction.dx > 0.9,
            "thrust should point toward +X target, got {:?}",
            thrust.direction
        );
        assert!(!thrust.is_braking);
    }

    #[test]
    fn approaching_ship_closes_distance_to_its_target_over_several_ticks() {
        let mut node = mem_node();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(10_000.0, 0.0, 0.0),
            Velocity::ZERO,
        );
        node.apply_approach_command_owned(
            player,
            chaser,
            dawn_core::ApproachCommand {
                target: dawn_core::ApproachTarget::Ship(target),
            },
        );

        let start = node
            .get_ship_position(chaser)
            .unwrap()
            .distance(Position::new(10_000.0, 0.0, 0.0));
        for _ in 0..30 {
            node.tick();
        }
        let end = node
            .get_ship_position(chaser)
            .unwrap()
            .distance(Position::new(10_000.0, 0.0, 0.0));
        assert!(
            end < start,
            "approaching ship should reduce distance: {start} -> {end}"
        );
    }

    #[test]
    fn move_command_cancels_an_active_approach() {
        let mut node = mem_node();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(10_000.0, 0.0, 0.0),
            Velocity::ZERO,
        );
        node.apply_approach_command_owned(
            player,
            chaser,
            dawn_core::ApproachCommand {
                target: dawn_core::ApproachTarget::Ship(target),
            },
        );
        assert_eq!(
            node.approach_target(chaser),
            Some(dawn_core::ApproachTarget::Ship(target))
        );

        node.apply_move_command(chaser, Position::new(-10_000.0, 0.0, 0.0));
        assert_eq!(
            node.approach_target(chaser),
            None,
            "manual move must cancel approach"
        );
    }

    #[test]
    fn stop_command_cancels_an_active_approach() {
        let mut node = mem_node();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(10_000.0, 0.0, 0.0),
            Velocity::ZERO,
        );
        node.apply_approach_command_owned(
            player,
            chaser,
            dawn_core::ApproachCommand {
                target: dawn_core::ApproachTarget::Ship(target),
            },
        );

        node.apply_stop_command(chaser);
        assert_eq!(
            node.approach_target(chaser),
            None,
            "stop must cancel approach"
        );
    }

    #[test]
    fn approach_is_dropped_and_ship_brakes_when_the_target_disappears() {
        let mut node = mem_node();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(10_000.0, 0.0, 0.0),
            Velocity::ZERO,
        );
        node.apply_approach_command_owned(
            player,
            chaser,
            dawn_core::ApproachCommand {
                target: dawn_core::ApproachTarget::Ship(target),
            },
        );

        let target_entity = node.ships.index.remove(&target).unwrap();
        node.world.despawn_ship(target_entity);

        node.process_approach();
        assert_eq!(
            node.approach_target(chaser),
            None,
            "approach must drop when target is gone"
        );
        let entity = *node.ships.index.get(&chaser).unwrap();
        assert!(
            node.world.get::<ThrustComp>(entity).unwrap().is_braking,
            "ship should brake when target vanishes"
        );
    }

    #[test]
    fn approach_command_is_rejected_when_target_is_self() {
        let mut node = mem_node();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        assert!(!node.apply_approach_command_owned(
            player,
            chaser,
            dawn_core::ApproachCommand {
                target: dawn_core::ApproachTarget::Ship(chaser),
            }
        ));
        assert_eq!(node.approach_target(chaser), None);
    }

    #[test]
    fn approaching_a_jump_gate_steers_the_ship_toward_the_gate_and_into_range() {
        let mut node = mem_node();
        let gate = *node
            .jump_gate(dawn_core::JumpGateId(0))
            .expect("Sector 0 has Gate 0");
        // Start near the gate: at the wide system scale you warp across the
        // system and only sublight-approach over the last stretch (the gate is
        // far beyond sublight range in a test budget). Compute "12,000 m short
        // of the gate" in f64 and re-anchor directly (set_spawn_anchor_abs) --
        // subtracting 12,000 from the absolute gate position must remain in the
        // f64 absolute frame, so the fixture stays meaningfully short of it.
        let near_gate_abs = [gate.abs_m[0] - 12_000.0, gate.abs_m[1], gate.abs_m[2]];
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        node.set_spawn_anchor_abs(chaser, near_gate_abs);

        assert!(node.apply_approach_command_owned(
            player,
            chaser,
            dawn_core::ApproachCommand {
                target: dawn_core::ApproachTarget::Gate(dawn_core::JumpGateId(0)),
            }
        ));
        assert_eq!(
            node.approach_target(chaser),
            Some(dawn_core::ApproachTarget::Gate(dawn_core::JumpGateId(0)))
        );

        // Distance via the f64 absolute accessors throughout.
        let dist_to_gate = |node: &SimulationNode| {
            let abs = node.ship_absolute(chaser).unwrap();
            let d = [
                abs[0] - gate.abs_m[0],
                abs[1] - gate.abs_m[1],
                abs[2] - gate.abs_m[2],
            ];
            (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
        };
        let start = dist_to_gate(&node);
        for _ in 0..600 {
            node.tick();
        }
        let end = dist_to_gate(&node);
        assert!(
            end < start,
            "ship should close on the gate: {start} -> {end}"
        );
        assert!(
            node.can_propose_jump(chaser, dawn_core::JumpGateId(0)),
            "after approaching, the ship should be within the gate's activation radius"
        );
        assert!(
            node.drain_pending_auto_jumps().is_empty(),
            "manual Approach must not auto-jump on gate arrival"
        );
    }

    // The "too close to warp -> approach -> auto-jump on arrival" path
    // (formerly `apply_approach_jump_fallback`) is now exercised in
    // node/jump.rs, where the orchestration lives (apply_jump_with_fallback).

    #[test]
    fn approach_command_is_rejected_for_a_gate_not_in_this_sector() {
        let mut node = mem_node();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        assert!(!node.apply_approach_command_owned(
            player,
            chaser,
            dawn_core::ApproachCommand {
                target: dawn_core::ApproachTarget::Gate(dawn_core::JumpGateId(1)),
            }
        ));
        assert_eq!(node.approach_target(chaser), None);
    }
}
