//! Orbit and Keep-at-Range commands and their per-tick steering systems for
//! `SimulationNode` (ADR-0031). Both are persistent operator-issued steering
//! modes in the same style as `node::approach`, but where Approach closes to
//! zero distance, these maintain a chosen stand-off:
//!
//! - `apply_orbit_command(_owned)` / `process_orbit` — sweep around the
//!   target at `radius`, leveraging the tracking-speed hit-chance penalty
//!   (ADR-0012) as a defensive tactic.
//! - `apply_keep_at_range_command(_owned)` / `process_keep_at_range` — hold
//!   at `range` away, closing in when farther and retreating when closer
//!   (no tangential component, unlike Orbit -- a pure stand-off distance).

use dawn_core::{ApproachTarget, PlayerId, Position, ShipId};
use dawn_ecs::{
    components::{KeepAtRangeComp, OrbitComp, PositionComp, ShipStatsComp},
    Entity,
};
use dawn_event_store::store::EventStore;

use super::{
    SimulationNode, DEFAULT_MANEUVER_RADIUS, KEEP_AT_RANGE_DEADBAND_FRACTION, ORBIT_LEAD_FACTOR,
};

impl<S: EventStore> SimulationNode<S> {
    /// Begin orbiting a Ship or a Jump Gate at `radius` (ADR-0031). Falls back
    /// to the ship's fitted weapon range, or `DEFAULT_MANEUVER_RADIUS` if
    /// unarmed, when `radius` is `None`.
    ///
    /// Rejected (returns `false`, no component attached) if the ship is
    /// unknown or in transit, a `Ship` target is unknown or the ship itself,
    /// or a `Gate` target does not originate in this Sector.
    pub fn apply_orbit_command(
        &mut self,
        ship_id: ShipId,
        target: ApproachTarget,
        radius: Option<f64>,
    ) -> bool {
        let Some((entity, radius)) = self.begin_maneuver(ship_id, target, radius) else {
            return false;
        };
        let _ = self.world.insert_one(entity, OrbitComp { target, radius });
        true
    }

    /// `apply_orbit_command` wrapped with the shared flight-command seam
    /// (active-ship + undocked, ADR-0037; `ship_command.rs`).
    pub fn apply_orbit_command_owned(
        &mut self,
        player_id: PlayerId,
        ship_id: ShipId,
        cmd: dawn_core::OrbitCommand,
    ) -> bool {
        if self.resolve_flight_command(player_id, ship_id).is_err() {
            return false;
        }
        self.apply_orbit_command(ship_id, cmd.target, cmd.radius)
    }

    /// Begin holding at least `range` away from a Ship or a Jump Gate
    /// (ADR-0031). Falls back to the ship's fitted weapon range, or
    /// `DEFAULT_MANEUVER_RADIUS` if unarmed, when `range` is `None`.
    ///
    /// Rejection conditions mirror `apply_orbit_command`.
    pub fn apply_keep_at_range_command(
        &mut self,
        ship_id: ShipId,
        target: ApproachTarget,
        range: Option<f64>,
    ) -> bool {
        let Some((entity, range)) = self.begin_maneuver(ship_id, target, range) else {
            return false;
        };
        let _ = self
            .world
            .insert_one(entity, KeepAtRangeComp { target, range });
        true
    }

    /// `apply_keep_at_range_command` wrapped with the shared flight-command
    /// seam (active-ship + undocked, ADR-0037; `ship_command.rs`).
    pub fn apply_keep_at_range_command_owned(
        &mut self,
        player_id: PlayerId,
        ship_id: ShipId,
        cmd: dawn_core::KeepAtRangeCommand,
    ) -> bool {
        if self.resolve_flight_command(player_id, ship_id).is_err() {
            return false;
        }
        self.apply_keep_at_range_command(ship_id, cmd.target, cmd.range)
    }

    /// Shared target validation for Orbit / Keep at Range / Approach
    /// (ADR-0031/ADR-0015): a `Ship` target must exist and not be the
    /// maneuvering ship itself; a `Gate` target must originate in this Sector.
    pub(super) fn validate_maneuver_target(&self, ship_id: ShipId, target: ApproachTarget) -> bool {
        match target {
            ApproachTarget::Ship(target_id) => {
                ship_id != target_id && self.ships.index.contains_key(&target_id)
            }
            ApproachTarget::Gate(gate_id) => self.jump_gate(gate_id).is_some(),
        }
    }

    /// Default orbit radius / keep-at-range distance for `entity`: its fitted
    /// weapon range, or `DEFAULT_MANEUVER_RADIUS` if unarmed (ADR-0031) --
    /// orbiting or holding at your own optimal range is the common case, so
    /// the unspecified default should already be a useful fighting distance.
    fn default_maneuver_radius(&self, entity: Entity) -> f64 {
        let weapon_range = self
            .world
            .get::<ShipStatsComp>(entity)
            .map(|s| s.weapon_range)
            .unwrap_or(0.0);
        if weapon_range > f32::EPSILON {
            f64::from(weapon_range)
        } else {
            DEFAULT_MANEUVER_RADIUS
        }
    }

    /// Shared "begin a persistent steering mode" scaffold (ADR-0015/ADR-0031):
    /// resolves `ship_id`'s entity, runs the rejection checklist (unknown ship
    /// / in transit / Warp priority / invalid target) shared by
    /// `apply_orbit_command`, `apply_keep_at_range_command`, and
    /// `apply_approach_command`, resolves the maneuver distance default, and
    /// clears any other active steering mode. Returns `None` on rejection
    /// (nothing is changed); otherwise the caller only needs to insert its
    /// own component (`OrbitComp` / `KeepAtRangeComp` / `ApproachComp`).
    /// `distance` is `None` and the resolved value ignored for Approach,
    /// which has no notion of a stand-off distance.
    pub(super) fn begin_maneuver(
        &mut self,
        ship_id: ShipId,
        target: ApproachTarget,
        distance: Option<f64>,
    ) -> Option<(Entity, f64)> {
        let &entity = self.ships.index.get(&ship_id)?;
        if self.world.transit_state(entity).is_in_transit() {
            return None;
        }
        // Warp takes priority over Orbit/Keep at Range (ADR-0031), in either
        // phase: a committed warp must not be interrupted, and an aligning
        // warp should be cancelled via Move/Stop, not silently raced by a
        // new steering mode.
        if self.has_active_warp(entity) {
            return None;
        }
        if !self.validate_maneuver_target(ship_id, target) {
            return None;
        }
        let distance = distance.unwrap_or_else(|| self.default_maneuver_radius(entity));
        self.clear_steering_modes(entity);
        Some((entity, distance))
    }

    /// Remove any other persistent steering mode before attaching a new one
    /// (ADR-0031): Orbit, Keep at Range, and Approach (ADR-0015) are mutually
    /// exclusive -- a ship holds at most one operator-issued steering intent
    /// at a time. Does not touch `WarpComp`; warp rejects these commands
    /// outright via the transit/warp guards in each `apply_*` method's caller.
    pub(super) fn clear_steering_modes(&mut self, entity: Entity) {
        let _ = self.world.remove_one::<OrbitComp>(entity);
        let _ = self.world.remove_one::<KeepAtRangeComp>(entity);
        let _ = self
            .world
            .remove_one::<dawn_ecs::components::ApproachComp>(entity);
    }

    /// Resolve `target`'s current absolute (f64) position and arrival-style
    /// reference point in `entity`'s anchor frame, shared by `process_orbit`
    /// and `process_keep_at_range`. Returns `None` if the target has vanished.
    fn resolve_maneuver_target(&self, entity: Entity, target: ApproachTarget) -> Option<Position> {
        match target {
            ApproachTarget::Ship(target_id) => {
                let &te = self.ships.index.get(&target_id)?;
                let off = self.world.get::<PositionComp>(te)?.0;
                let target_abs = self.entity_absolute_f64(te, off);
                Some(self.dest_in_ship_frame_abs(entity, target_abs))
            }
            ApproachTarget::Gate(gate_id) => {
                let gate = self.jump_gate(gate_id)?;
                Some(self.dest_in_ship_frame_abs(entity, gate.abs_m))
            }
        }
    }

    /// Orbit System (ADR-0031): for every ship carrying an `OrbitComp`, steer
    /// toward a point on the circle of `radius` around the target, led
    /// tangentially so the ship sweeps around it. Brakes and drops the
    /// component if the target has vanished.
    ///
    /// Runs at Step 2.55, after Approach and before Keep at Range / Warp.
    pub fn process_orbit(&mut self) {
        let orbiters: Vec<(Entity, ApproachTarget, f64, Position)> = self
            .world
            .query::<(Entity, &OrbitComp, &PositionComp)>()
            .iter()
            .map(|(entity, orbit, pos)| (entity, orbit.target, orbit.radius, pos.0))
            .collect();

        for (entity, target, radius, ship_pos) in orbiters {
            let Some(target_pos) = self.resolve_maneuver_target(entity, target) else {
                let _ = self.world.remove_one::<OrbitComp>(entity);
                self.brake_thrust(entity);
                continue;
            };

            let target_point = compute_orbit_steering_point(ship_pos, target_pos, radius);
            self.steer_thrust_toward(entity, ship_pos, target_point);
        }
    }

    /// Keep at Range System (ADR-0031): for every ship carrying a
    /// `KeepAtRangeComp`, steers to hold at `range` from the target —
    /// retreating when closer, closing in when farther — rather than just
    /// "never get closer than range" (a stand-off that only ever retreated
    /// was a trap if the player picked a distance the target hadn't closed
    /// to yet: nothing would happen, and the command looked broken). A small
    /// deadband around `range` avoids thrust flapping back and forth every
    /// tick once the ship settles in. Brakes and drops the component if the
    /// target has vanished.
    ///
    /// Runs at Step 2.56, after Orbit and before Warp.
    pub fn process_keep_at_range(&mut self) {
        let holders: Vec<(Entity, ApproachTarget, f64, Position)> = self
            .world
            .query::<(Entity, &KeepAtRangeComp, &PositionComp)>()
            .iter()
            .map(|(entity, keep, pos)| (entity, keep.target, keep.range, pos.0))
            .collect();

        for (entity, target, range, ship_pos) in holders {
            let Some(target_pos) = self.resolve_maneuver_target(entity, target) else {
                let _ = self.world.remove_one::<KeepAtRangeComp>(entity);
                self.brake_thrust(entity);
                continue;
            };

            match compute_keep_at_range_step(ship_pos, target_pos, range) {
                KeepAtRangeStepOutcome::Brake => self.brake_thrust(entity),
                KeepAtRangeStepOutcome::CloseIn => {
                    self.steer_thrust_toward(entity, ship_pos, target_pos)
                }
                KeepAtRangeStepOutcome::StepAway { toward } => {
                    self.steer_thrust_toward(entity, ship_pos, toward)
                }
            }
        }
    }
}

/// Steering aim point for an orbiting ship this tick (candidate 6): pure,
/// given its current position, the target's resolved position, and the
/// orbit radius. Sweeps around the target with a tangential lead so the
/// ship doesn't just sit on the radial line (ADR-0031).
fn compute_orbit_steering_point(ship_pos: Position, target_pos: Position, radius: f64) -> Position {
    let radial = Position::new(
        ship_pos.x - target_pos.x,
        ship_pos.y - target_pos.y,
        ship_pos.z - target_pos.z,
    );
    let dist = (radial.x * radial.x + radial.y * radial.y + radial.z * radial.z).sqrt();
    // Arbitrary stable unit vector when sitting exactly on the target
    // (degenerate radial direction) -- avoids a NaN steering target.
    let radial_unit = if dist > f64::EPSILON {
        Position::new(radial.x / dist, radial.y / dist, radial.z / dist)
    } else {
        Position::new(1.0, 0.0, 0.0)
    };
    // Fixed UP axis (ADR-0031): a consistent, predictable sweep direction
    // rather than a true axis-free 3D orbit.
    const UP: (f64, f64, f64) = (0.0, 1.0, 0.0);
    let cross = (
        UP.1 * radial_unit.z - UP.2 * radial_unit.y,
        UP.2 * radial_unit.x - UP.0 * radial_unit.z,
        UP.0 * radial_unit.y - UP.1 * radial_unit.x,
    );
    let cross_len = (cross.0 * cross.0 + cross.1 * cross.1 + cross.2 * cross.2).sqrt();
    let tangent = if cross_len > f64::EPSILON {
        (
            cross.0 / cross_len,
            cross.1 / cross_len,
            cross.2 / cross_len,
        )
    } else {
        // radial_unit is parallel to UP -- pick an arbitrary perpendicular
        // axis instead of leaving the ship without a tangential pull.
        (1.0, 0.0, 0.0)
    };

    let lead = radius * ORBIT_LEAD_FACTOR;
    Position::new(
        target_pos.x + radial_unit.x * radius + tangent.0 * lead,
        target_pos.y + radial_unit.y * radius + tangent.1 * lead,
        target_pos.z + radial_unit.z * radius + tangent.2 * lead,
    )
}

/// What a Keep-at-Range ship should do this tick (candidate 6): pure,
/// given its current position, the target's resolved position, and the
/// held range.
#[derive(Debug, Clone, Copy, PartialEq)]
enum KeepAtRangeStepOutcome {
    /// Within the deadband around `range`: hold position.
    Brake,
    /// Farther than `range`: close in, same steering as Approach.
    CloseIn,
    /// Closer than `range`: steer away, toward this point.
    StepAway { toward: Position },
}

fn compute_keep_at_range_step(
    ship_pos: Position,
    target_pos: Position,
    range: f64,
) -> KeepAtRangeStepOutcome {
    let dx = ship_pos.x - target_pos.x;
    let dy = ship_pos.y - target_pos.y;
    let dz = ship_pos.z - target_pos.z;
    let dist = (dx * dx + dy * dy + dz * dz).sqrt();

    let deadband = (range * KEEP_AT_RANGE_DEADBAND_FRACTION).max(1.0);
    if (dist - range).abs() <= deadband {
        return KeepAtRangeStepOutcome::Brake;
    }
    if dist > range {
        return KeepAtRangeStepOutcome::CloseIn;
    }
    // Steer straight away: aim at a point further out along the current
    // radial direction (steer_thrust_toward only needs the direction, not
    // an exact arrival point).
    let radial_unit = if dist > f64::EPSILON {
        Position::new(dx / dist, dy / dist, dz / dist)
    } else {
        Position::new(1.0, 0.0, 0.0)
    };
    KeepAtRangeStepOutcome::StepAway {
        toward: Position::new(
            ship_pos.x + radial_unit.x,
            ship_pos.y + radial_unit.y,
            ship_pos.z + radial_unit.z,
        ),
    }
}

#[cfg(test)]
mod compute_step_tests {
    use super::*;

    #[test]
    fn orbit_steering_point_sits_radius_plus_lead_from_the_target() {
        let ship = Position::new(2000.0, 0.0, 0.0);
        let target = Position::ORIGIN;
        let radius = 2000.0;
        let point = compute_orbit_steering_point(ship, target, radius);
        assert!(
            point.x.is_finite() && point.y.is_finite() && point.z.is_finite(),
            "steering point must be finite, got {point:?}"
        );
        let dist = ((point.x - target.x).powi(2)
            + (point.y - target.y).powi(2)
            + (point.z - target.z).powi(2))
        .sqrt();
        let lead = radius * ORBIT_LEAD_FACTOR;
        let expected = (radius * radius + lead * lead).sqrt();
        assert!(
            (dist - expected).abs() < 1e-6,
            "steering point should be sqrt(radius^2 + lead^2) from the target: got {dist}, expected {expected}"
        );
    }

    #[test]
    fn orbit_steering_point_is_finite_when_the_ship_sits_exactly_on_the_target() {
        // Degenerate radial direction (dist == 0): must not produce NaN.
        let point = compute_orbit_steering_point(Position::ORIGIN, Position::ORIGIN, 2000.0);
        assert!(
            point.x.is_finite() && point.y.is_finite() && point.z.is_finite(),
            "steering point must stay finite in the degenerate case, got {point:?}"
        );
    }

    #[test]
    fn orbit_steering_point_is_finite_when_radial_is_parallel_to_the_up_axis() {
        // radial_unit == UP (0, 1, 0): the tangent cross-product degenerates
        // and must fall back to a stable axis instead of NaN.
        let ship = Position::new(0.0, 2000.0, 0.0);
        let target = Position::ORIGIN;
        let point = compute_orbit_steering_point(ship, target, 2000.0);
        assert!(
            point.x.is_finite() && point.y.is_finite() && point.z.is_finite(),
            "steering point must stay finite when radial is parallel to UP, got {point:?}"
        );
    }

    #[test]
    fn keep_at_range_within_deadband_brakes() {
        // range 5000, deadband = 5000 * 0.05 = 250 -> 5100 sits inside it.
        let ship = Position::new(5100.0, 0.0, 0.0);
        let target = Position::ORIGIN;
        assert_eq!(
            compute_keep_at_range_step(ship, target, 5000.0),
            KeepAtRangeStepOutcome::Brake
        );
    }

    #[test]
    fn keep_at_range_farther_than_range_closes_in() {
        let ship = Position::new(20_000.0, 0.0, 0.0);
        let target = Position::ORIGIN;
        assert_eq!(
            compute_keep_at_range_step(ship, target, 5000.0),
            KeepAtRangeStepOutcome::CloseIn
        );
    }

    #[test]
    fn keep_at_range_closer_than_range_steps_away() {
        let ship = Position::new(1000.0, 0.0, 0.0);
        let target = Position::ORIGIN;
        let outcome = compute_keep_at_range_step(ship, target, 5000.0);
        match outcome {
            KeepAtRangeStepOutcome::StepAway { toward } => {
                assert!(toward.x > ship.x, "should aim further away along +X");
            }
            other => panic!("expected StepAway, got {other:?}"),
        }
    }
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
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        )
    }

    fn spawn_owned_player_at(node: &mut SimulationNode, pos: Position) -> (PlayerId, ShipId) {
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship_at_pub(player_id, pos);
        (player_id, ship_id)
    }

    // ── Orbit ──────────────────────────────────────────────────────────────

    #[test]
    fn orbit_command_attaches_an_orbit_target_to_the_owned_ship() {
        let mut node = mem_node();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(10_000.0, 0.0, 0.0),
            Velocity::ZERO,
        );

        assert!(node.apply_orbit_command_owned(
            player,
            chaser,
            dawn_core::OrbitCommand {
                target: dawn_core::ApproachTarget::Ship(target),
                radius: Some(2000.0),
            }
        ));
    }

    #[test]
    fn orbit_command_is_rejected_while_aligning_to_warp() {
        // Warp takes priority over Orbit (ADR-0031) from the moment it's
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

        assert!(!node.apply_orbit_command_owned(
            player,
            chaser,
            dawn_core::OrbitCommand {
                target: dawn_core::ApproachTarget::Ship(target),
                radius: Some(2000.0),
            }
        ));
    }

    #[test]
    fn keep_at_range_command_is_rejected_while_aligning_to_warp() {
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

        assert!(!node.apply_keep_at_range_command_owned(
            player,
            chaser,
            dawn_core::KeepAtRangeCommand {
                target: dawn_core::ApproachTarget::Ship(target),
                range: Some(2000.0),
            }
        ));
    }

    #[test]
    fn orbit_command_is_rejected_for_a_ship_the_player_does_not_own() {
        let mut node = mem_node();
        let (_owner, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(10_000.0, 0.0, 0.0),
            Velocity::ZERO,
        );
        let stranger = node.next_player_id();

        assert!(!node.apply_orbit_command_owned(
            stranger,
            chaser,
            dawn_core::OrbitCommand {
                target: dawn_core::ApproachTarget::Ship(target),
                radius: Some(2000.0),
            }
        ));
    }

    #[test]
    fn orbit_command_is_rejected_when_target_is_self() {
        let mut node = mem_node();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);

        assert!(!node.apply_orbit_command(
            chaser,
            dawn_core::ApproachTarget::Ship(chaser),
            Some(2000.0)
        ));
        let _ = player;
    }

    #[test]
    fn orbit_with_no_radius_falls_back_to_default_maneuver_radius() {
        let mut node = mem_node();
        let (_player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(10_000.0, 0.0, 0.0),
            Velocity::ZERO,
        );

        assert!(node.apply_orbit_command(chaser, dawn_core::ApproachTarget::Ship(target), None));
        let entity = *node.ships.index.get(&chaser).unwrap();
        let radius = node.world.get::<OrbitComp>(entity).unwrap().radius;
        assert_eq!(radius, super::super::DEFAULT_MANEUVER_RADIUS);
    }

    #[test]
    fn orbiting_ship_steers_with_a_tangential_component_not_straight_at_the_target() {
        let mut node = mem_node();
        let (_player, chaser) = spawn_owned_player_at(&mut node, Position::new(2000.0, 0.0, 0.0));
        let target = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        assert!(node.apply_orbit_command(
            chaser,
            dawn_core::ApproachTarget::Ship(target),
            Some(2000.0)
        ));

        node.process_orbit();
        let entity = *node.ships.index.get(&chaser).unwrap();
        let thrust = node.world.get::<ThrustComp>(entity).unwrap();
        // Pure radial-only steering (straight at or away from the target)
        // would have dz == 0 here (target is directly along +X); a nonzero Z
        // component proves the tangential lead is contributing.
        assert!(
            thrust.direction.dz.abs() > 0.01,
            "expected a tangential component, got {:?}",
            thrust.direction
        );
    }

    #[test]
    fn orbiting_ship_converges_toward_the_target_radius_over_time() {
        let mut node = mem_node();
        // Start much closer than the orbit radius so the radial correction
        // (pushing outward) dominates and distance should trend toward 2000.
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::new(200.0, 0.0, 0.0));
        let target = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        node.apply_orbit_command_owned(
            player,
            chaser,
            dawn_core::OrbitCommand {
                target: dawn_core::ApproachTarget::Ship(target),
                radius: Some(2000.0),
            },
        );

        let start = node
            .get_ship_position(chaser)
            .unwrap()
            .distance(Position::ORIGIN);
        for _ in 0..300 {
            node.tick();
        }
        let end = node
            .get_ship_position(chaser)
            .unwrap()
            .distance(Position::ORIGIN);
        assert!(
            end > start,
            "orbiting ship starting inside the radius should move outward: {start} -> {end}"
        );
    }

    #[test]
    fn orbit_is_dropped_and_ship_brakes_when_the_target_disappears() {
        let mut node = mem_node();
        let (_player, chaser) = spawn_owned_player_at(&mut node, Position::new(2000.0, 0.0, 0.0));
        let target = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        node.apply_orbit_command(
            chaser,
            dawn_core::ApproachTarget::Ship(target),
            Some(2000.0),
        );

        let target_entity = node.ships.index.remove(&target).unwrap();
        node.world.despawn_ship(target_entity);

        node.process_orbit();
        let entity = *node.ships.index.get(&chaser).unwrap();
        assert!(
            node.world.get::<OrbitComp>(entity).is_none(),
            "OrbitComp should be removed"
        );
        assert!(node.world.get::<ThrustComp>(entity).unwrap().is_braking);
    }

    #[test]
    fn move_command_cancels_an_active_orbit() {
        let mut node = mem_node();
        let (_player, chaser) = spawn_owned_player_at(&mut node, Position::new(2000.0, 0.0, 0.0));
        let target = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        node.apply_orbit_command(
            chaser,
            dawn_core::ApproachTarget::Ship(target),
            Some(2000.0),
        );

        node.apply_move_command(chaser, Position::new(-2000.0, 0.0, 0.0));
        let entity = *node.ships.index.get(&chaser).unwrap();
        assert!(
            node.world.get::<OrbitComp>(entity).is_none(),
            "move must cancel orbit"
        );
    }

    #[test]
    fn approach_command_cancels_an_active_orbit() {
        let mut node = mem_node();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::new(2000.0, 0.0, 0.0));
        let target = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        node.apply_orbit_command(
            chaser,
            dawn_core::ApproachTarget::Ship(target),
            Some(2000.0),
        );

        assert!(node.apply_approach_command_owned(
            player,
            chaser,
            dawn_core::ApproachCommand {
                target: dawn_core::ApproachTarget::Ship(target),
            }
        ));
        let entity = *node.ships.index.get(&chaser).unwrap();
        assert!(
            node.world.get::<OrbitComp>(entity).is_none(),
            "approach must cancel orbit"
        );
    }

    // ── Keep at Range ────────────────────────────────────────────────────────

    #[test]
    fn keep_at_range_command_attaches_a_target_to_the_owned_ship() {
        let mut node = mem_node();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(10_000.0, 0.0, 0.0),
            Velocity::ZERO,
        );

        assert!(node.apply_keep_at_range_command_owned(
            player,
            chaser,
            dawn_core::KeepAtRangeCommand {
                target: dawn_core::ApproachTarget::Ship(target),
                range: Some(5000.0),
            }
        ));
    }

    #[test]
    fn keep_at_range_command_is_rejected_for_a_ship_the_player_does_not_own() {
        let mut node = mem_node();
        let (_owner, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(10_000.0, 0.0, 0.0),
            Velocity::ZERO,
        );
        let stranger = node.next_player_id();

        assert!(!node.apply_keep_at_range_command_owned(
            stranger,
            chaser,
            dawn_core::KeepAtRangeCommand {
                target: dawn_core::ApproachTarget::Ship(target),
                range: Some(5000.0),
            }
        ));
    }

    #[test]
    fn ship_closer_than_range_is_steered_directly_away() {
        let mut node = mem_node();
        let (_player, chaser) = spawn_owned_player_at(&mut node, Position::new(1000.0, 0.0, 0.0));
        let target = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        assert!(node.apply_keep_at_range_command(
            chaser,
            dawn_core::ApproachTarget::Ship(target),
            Some(5000.0)
        ));

        node.process_keep_at_range();
        let entity = *node.ships.index.get(&chaser).unwrap();
        let thrust = node.world.get::<ThrustComp>(entity).unwrap();
        assert!(
            thrust.direction.dx > 0.9,
            "should thrust away from target (+X), got {:?}",
            thrust.direction
        );
        assert!(!thrust.is_braking);
    }

    #[test]
    fn ship_at_range_within_the_deadband_brakes() {
        let mut node = mem_node();
        // range 5000, deadband = 5000 * 0.05 = 250 -> 5100 sits inside it.
        let (_player, chaser) = spawn_owned_player_at(&mut node, Position::new(5100.0, 0.0, 0.0));
        let target = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        assert!(node.apply_keep_at_range_command(
            chaser,
            dawn_core::ApproachTarget::Ship(target),
            Some(5000.0)
        ));

        node.process_keep_at_range();
        let entity = *node.ships.index.get(&chaser).unwrap();
        assert!(node.world.get::<ThrustComp>(entity).unwrap().is_braking);
    }

    #[test]
    fn ship_farther_than_range_closes_in() {
        let mut node = mem_node();
        let (_player, chaser) = spawn_owned_player_at(&mut node, Position::new(20_000.0, 0.0, 0.0));
        let target = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        assert!(node.apply_keep_at_range_command(
            chaser,
            dawn_core::ApproachTarget::Ship(target),
            Some(5000.0)
        ));

        node.process_keep_at_range();
        let entity = *node.ships.index.get(&chaser).unwrap();
        let thrust = node.world.get::<ThrustComp>(entity).unwrap();
        assert!(
            thrust.direction.dx < -0.9,
            "should thrust toward target (-X) when farther than range, got {:?}",
            thrust.direction
        );
        assert!(!thrust.is_braking);
    }

    #[test]
    fn ship_farther_than_range_decreases_distance_over_several_ticks() {
        let mut node = mem_node();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::new(20_000.0, 0.0, 0.0));
        let target = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        node.apply_keep_at_range_command_owned(
            player,
            chaser,
            dawn_core::KeepAtRangeCommand {
                target: dawn_core::ApproachTarget::Ship(target),
                range: Some(5000.0),
            },
        );

        let start = node
            .get_ship_position(chaser)
            .unwrap()
            .distance(Position::ORIGIN);
        for _ in 0..30 {
            node.tick();
        }
        let end = node
            .get_ship_position(chaser)
            .unwrap()
            .distance(Position::ORIGIN);
        assert!(
            end < start,
            "keep-at-range should decrease distance while outside range: {start} -> {end}"
        );
    }

    #[test]
    fn ship_closer_than_range_increases_distance_over_several_ticks() {
        let mut node = mem_node();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::new(1000.0, 0.0, 0.0));
        let target = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        node.apply_keep_at_range_command_owned(
            player,
            chaser,
            dawn_core::KeepAtRangeCommand {
                target: dawn_core::ApproachTarget::Ship(target),
                range: Some(5000.0),
            },
        );

        let start = node
            .get_ship_position(chaser)
            .unwrap()
            .distance(Position::ORIGIN);
        for _ in 0..30 {
            node.tick();
        }
        let end = node
            .get_ship_position(chaser)
            .unwrap()
            .distance(Position::ORIGIN);
        assert!(
            end > start,
            "keep-at-range should increase distance while inside range: {start} -> {end}"
        );
    }

    #[test]
    fn keep_at_range_is_dropped_and_ship_brakes_when_the_target_disappears() {
        let mut node = mem_node();
        let (_player, chaser) = spawn_owned_player_at(&mut node, Position::new(1000.0, 0.0, 0.0));
        let target = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        node.apply_keep_at_range_command(
            chaser,
            dawn_core::ApproachTarget::Ship(target),
            Some(5000.0),
        );

        let target_entity = node.ships.index.remove(&target).unwrap();
        node.world.despawn_ship(target_entity);

        node.process_keep_at_range();
        let entity = *node.ships.index.get(&chaser).unwrap();
        assert!(node.world.get::<KeepAtRangeComp>(entity).is_none());
        assert!(node.world.get::<ThrustComp>(entity).unwrap().is_braking);
    }

    #[test]
    fn stop_command_cancels_an_active_keep_at_range() {
        let mut node = mem_node();
        let (_player, chaser) = spawn_owned_player_at(&mut node, Position::new(1000.0, 0.0, 0.0));
        let target = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        node.apply_keep_at_range_command(
            chaser,
            dawn_core::ApproachTarget::Ship(target),
            Some(5000.0),
        );

        node.apply_stop_command(chaser);
        let entity = *node.ships.index.get(&chaser).unwrap();
        assert!(
            node.world.get::<KeepAtRangeComp>(entity).is_none(),
            "stop must cancel keep-at-range"
        );
    }

    #[test]
    fn orbit_command_cancels_an_active_keep_at_range() {
        let mut node = mem_node();
        let (_player, chaser) = spawn_owned_player_at(&mut node, Position::new(1000.0, 0.0, 0.0));
        let target = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        node.apply_keep_at_range_command(
            chaser,
            dawn_core::ApproachTarget::Ship(target),
            Some(5000.0),
        );

        assert!(node.apply_orbit_command(
            chaser,
            dawn_core::ApproachTarget::Ship(target),
            Some(2000.0)
        ));
        let entity = *node.ships.index.get(&chaser).unwrap();
        assert!(
            node.world.get::<KeepAtRangeComp>(entity).is_none(),
            "orbit must cancel keep-at-range"
        );
    }

    #[test]
    fn keep_at_range_command_is_rejected_for_a_gate_not_in_this_sector() {
        let mut node = mem_node();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);

        assert!(!node.apply_keep_at_range_command_owned(
            player,
            chaser,
            dawn_core::KeepAtRangeCommand {
                target: dawn_core::ApproachTarget::Gate(dawn_core::JumpGateId(1)),
                range: Some(5000.0),
            }
        ));
    }
}
