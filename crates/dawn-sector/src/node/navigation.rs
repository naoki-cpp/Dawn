//! Navigation commands and per-tick navigation systems for `SimulationNode`.
//!
//! # Contents
//!
//! - `apply_approach_command` / `apply_approach_command_owned` — attach `ApproachComp`
//! - `apply_warp_command`     / `apply_warp_command_owned`     — attach `WarpComp`
//! - `drain_pending_auto_jumps`                                — drain the auto-jump queue
//! - `process_approach`  — Approach System (Step 2.5, ADR-0015)
//! - `process_warp`      — Warp System    (Step 2.6, ADR-0022/0023/0025)

use dawn_core::{
    DomainEvent, JumpGateId, PlayerId, Position, ShipId, Tick, Velocity, WarpTarget,
};
use dawn_ecs::{
    components::{ApproachComp, PositionComp, ShipIdComp, ShipStatsComp, ThrustComp, VelocityComp, WarpComp, WarpPhase},
    Entity,
};
use dawn_event_store::store::EventStore;

use super::{
    SimulationNode, WARP_ALIGN_FRACTION, WARP_ARRIVAL_FACTOR, BODY_WARP_ARRIVAL_FACTOR,
    WARP_DECEL_RATE, WARP_EXIT_SPEED, WARP_SPEED,
};

impl<S: EventStore> SimulationNode<S> {
    /// Begin approaching a Ship or a Jump Gate (semi-automatic piloting, ADR-0015).
    ///
    /// Attaches an `ApproachComp` so `process_approach()` re-aims thrust at the
    /// target each tick. Rejected (returns `false`, no component attached) if
    /// the ship is unknown or in transit, a `Ship` target is unknown or the
    /// ship itself, or a `Gate` target does not originate in this Sector.
    pub fn apply_approach_command(&mut self, ship_id: ShipId, target: dawn_core::ApproachTarget) -> bool {
        use dawn_core::ApproachTarget;
        let &entity = match self.ships.index.get(&ship_id) {
            Some(e) => e,
            None    => return false,
        };
        if self.world.transit_state(entity).is_in_transit() {
            return false;
        }
        match target {
            ApproachTarget::Ship(target_id) => {
                if ship_id == target_id || !self.ships.index.contains_key(&target_id) {
                    return false;
                }
            }
            ApproachTarget::Gate(gate_id) => {
                if self.jump_gate(gate_id).is_none() {
                    return false;
                }
            }
        }
        let _ = self.world.inner_mut().insert_one(entity, ApproachComp { target });
        true
    }

    /// `apply_approach_command` wrapped with an ownership check.
    pub fn apply_approach_command_owned(
        &mut self,
        player_id : PlayerId,
        cmd       : dawn_core::ApproachCommand,
    ) -> bool {
        if !self.owns_ship(player_id, cmd.ship_id) { return false; }
        self.apply_approach_command(cmd.ship_id, cmd.target)
    }

    /// Begin an intra-Sector warp toward a Jump Gate (short-range Fold, ADR-0022).
    ///
    /// Attaches a `WarpComp` in the `Aligning` phase if `can_propose_warp`
    /// accepts the request; `process_warp()` then advances the alignment and,
    /// once aligned, flies the ship to the gate at warp speed. Returns `false`
    /// (no component attached) on rejection.
    pub fn apply_warp_command(&mut self, ship_id: ShipId, target: WarpTarget, auto_jump: bool) -> bool {
        if !self.can_propose_warp(ship_id, target) {
            return false;
        }
        let &entity = match self.ships.index.get(&ship_id) {
            Some(e) => e,
            None    => return false,
        };
        let _ = self.world.inner_mut().insert_one(entity, WarpComp {
            target,
            phase: WarpPhase::Aligning,
            auto_jump,
        });
        true
    }

    /// `apply_warp_command` wrapped with an ownership check.
    pub fn apply_warp_command_owned(
        &mut self,
        player_id : PlayerId,
        cmd       : dawn_core::WarpCommand,
    ) -> bool {
        if !self.owns_ship(player_id, cmd.ship_id) { return false; }
        self.apply_warp_command(cmd.ship_id, cmd.target, false)
    }

    /// Drain auto-jump triggers accumulated during `process_warp()`.
    ///
    /// The caller (server loop) is responsible for proposing each returned
    /// `(ship_id, gate_id)` pair to the Raft Log (cluster mode) or ignoring it
    /// (single-node mode where Jump is not supported).
    pub fn drain_pending_auto_jumps(&mut self) -> Vec<(ShipId, JumpGateId)> {
        std::mem::take(&mut self.pending_auto_jumps)
    }

    /// Warp System (ADR-0022 §6): advance every ship carrying a `WarpComp`.
    ///
    /// Runs each tick as Step 2.6 (after Approach, before Movement). The
    /// `Aligning` phase steers the ship at the gate and accelerates it; warp
    /// engages once it is moving at ≥ `WARP_ALIGN_FRACTION` of max speed toward
    /// the gate (EVE-style alignment — interruptible by Move/Stop and, later,
    /// tackle). Once `Warping`, the ship flies straight to the gate at warp
    /// speed and decelerates into its activation radius. Warping ships are
    /// skipped by the Movement System, so this method owns their position,
    /// velocity, and `VelocityChanged` events. Returns those events.
    pub fn process_warp(&mut self, tick: Tick) -> Vec<DomainEvent> {
        // Collect warpers up front so the ECS query borrow is released before
        // the mutable write pass below.
        let warpers: Vec<(Entity, ShipId, WarpComp, Position, Velocity, f32)> = self.world.inner()
            .query::<(&ShipIdComp, &WarpComp, &PositionComp, &VelocityComp, &ShipStatsComp)>()
            .iter()
            .map(|(e, (id, w, p, v, s))| (e, id.0, *w, p.0, v.0, s.max_speed))
            .collect();

        let mut events = Vec::new();

        for (entity, ship_id, warp, pos, vel, max_speed) in warpers {
            // Resolve destination and arrival distance from WarpTarget.
            let resolved = match warp.target {
                WarpTarget::Gate(gate_id) => self.jump_gate(gate_id)
                    .map(|g| (g.position, g.activation_radius * WARP_ARRIVAL_FACTOR, warp.auto_jump.then_some(gate_id))),
                WarpTarget::Body(body_id) => self.sector_map.bodies.get(&body_id)
                    .map(|b| (b.position, b.radius * BODY_WARP_ARRIVAL_FACTOR, None)),
            };
            let Some((dest_pos, arrival, auto_jump_gate)) = resolved else {
                // Target vanished — cancel and brake.
                let _ = self.world.inner_mut().remove_one::<WarpComp>(entity);
                self.brake_thrust(entity);
                continue;
            };

            // Tackle interrupts the aligning phase (ADR-0024): a tackled ship
            // cannot enter warp. Cancel and brake; the Warping phase is committed.
            if warp.phase == WarpPhase::Aligning && self.world.is_tackled(entity) {
                let _ = self.world.inner_mut().remove_one::<WarpComp>(entity);
                self.brake_thrust(entity);
                continue;
            }

            // Engage warp once aligned: moving at ≥ 75% of max speed toward the
            // destination. While not yet aligned, keep steering/accelerating at it.
            let aligned = max_speed > f32::EPSILON
                && speed_toward(vel, pos, dest_pos) >= WARP_ALIGN_FRACTION * max_speed;

            match warp.phase {
                WarpPhase::Aligning if !aligned => {
                    self.steer_thrust_toward(entity, pos, dest_pos);
                }
                // Aligned (engage warp) or already warping: fly toward the destination.
                WarpPhase::Aligning | WarpPhase::Warping => {
                    self.set_warp_phase(entity, WarpPhase::Warping);
                    if let Some(ev) = self.warp_step(entity, ship_id, pos, vel, dest_pos, arrival, auto_jump_gate, tick) {
                        events.push(ev);
                    }
                }
            }
        }

        events
    }

    /// One warping-phase step: move `entity` toward `dest_pos` at `WARP_SPEED`,
    /// stopping inside `arrival`. Returns a `VelocityChanged` if velocity moved.
    /// `auto_jump_gate`: if `Some(gate_id)`, queue an auto-jump on arrival.
    fn warp_step(
        &mut self,
        entity          : Entity,
        ship_id         : ShipId,
        pos             : Position,
        old_vel         : Velocity,
        dest_pos        : Position,
        arrival         : f32,
        auto_jump_gate  : Option<JumpGateId>,
        tick            : Tick,
    ) -> Option<DomainEvent> {
        let dist      = pos.distance(dest_pos);
        let remaining = dist - arrival;
        let (new_pos, new_vel, arrived) = if remaining <= WARP_EXIT_SPEED {
            // Close enough (inside the arrival ring or one slow step away):
            // settle and stop.
            (pos, Velocity::ZERO, true)
        } else {
            // Ease in: cap speed by remaining distance so the ship decelerates
            // smoothly instead of stopping dead (ADR-0022 §9).
            let speed = (remaining * WARP_DECEL_RATE).clamp(WARP_EXIT_SPEED, WARP_SPEED);
            let step  = speed.min(remaining);
            let inv   = step / dist;
            let v = Velocity {
                dx: (dest_pos.x - pos.x) * inv,
                dy: (dest_pos.y - pos.y) * inv,
                dz: (dest_pos.z - pos.z) * inv,
            };
            let p = Position { x: pos.x + v.dx, y: pos.y + v.dy, z: pos.z + v.dz };
            (p, v, false)
        };

        if let Ok(mut p) = self.world.inner_mut().get::<&mut PositionComp>(entity) { p.0 = new_pos; }
        if let Ok(mut v) = self.world.inner_mut().get::<&mut VelocityComp>(entity) { v.0 = new_vel; }

        if arrived {
            // Warp complete: drop the component and clear thrust. Movement resumes next tick.
            let _ = self.world.inner_mut().remove_one::<WarpComp>(entity);
            if let Ok(mut t) = self.world.inner_mut().get::<&mut ThrustComp>(entity) {
                t.direction  = Velocity::ZERO;
                t.is_braking = false;
            }
            // Queue auto-jump for Gate targets (ADR-0023).
            if let Some(gid) = auto_jump_gate {
                self.pending_auto_jumps.push((ship_id, gid));
            }
        }

        // Emit VelocityChanged only when velocity actually changed (INV-MOVE).
        let changed = (new_vel.dx - old_vel.dx).abs() > f32::EPSILON
            || (new_vel.dy - old_vel.dy).abs() > f32::EPSILON
            || (new_vel.dz - old_vel.dz).abs() > f32::EPSILON;
        changed.then(|| DomainEvent::VelocityChanged(dawn_core::events::VelocityChanged {
            ship_id,
            velocity: new_vel,
            tick,
        }))
    }

    /// Overwrite the phase of a ship's `WarpComp` (no-op if absent).
    fn set_warp_phase(&mut self, entity: Entity, phase: WarpPhase) {
        if let Ok(mut w) = self.world.inner_mut().get::<&mut WarpComp>(entity) {
            w.phase = phase;
        }
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
        const SHIP_ARRIVAL_RADIUS: f32 = 500.0;

        // Collect approachers up front (entity, target, current position) so the
        // ECS query borrow is released before the mutable write pass below.
        let approachers: Vec<(Entity, dawn_core::ApproachTarget, Position)> = self.world.inner()
            .query::<(&ApproachComp, &PositionComp)>()
            .iter()
            .map(|(entity, (approach, pos))| (entity, approach.target, pos.0))
            .collect();

        for (entity, target, ship_pos) in approachers {
            // Resolve the target's current position and the arrival distance.
            // `None` means the target no longer exists.
            let resolved: Option<(Position, f32)> = match target {
                ApproachTarget::Ship(target_id) => self.ships.index.get(&target_id)
                    .and_then(|&te| self.world.inner().get::<&PositionComp>(te).ok().map(|p| (p.0, SHIP_ARRIVAL_RADIUS))),
                // Stop comfortably inside the gate's activation radius so the
                // jump prompt becomes available on arrival (ADR-0015).
                ApproachTarget::Gate(gate_id) => self.jump_gate(gate_id)
                    .map(|g| (g.position, g.activation_radius * 0.8)),
            };

            match resolved {
                // Target gone: drop the approach and brake (ADR-0015 §4).
                None => {
                    let _ = self.world.inner_mut().remove_one::<ApproachComp>(entity);
                    self.brake_thrust(entity);
                }
                // Arrived: hold position, keep ApproachComp so the ship resumes
                // if a Ship target later drifts back out of range.
                Some((tp, arrival)) if ship_pos.distance(tp) <= arrival => self.brake_thrust(entity),
                // Still closing: steer toward the target's latest position.
                Some((tp, _)) => self.steer_thrust_toward(entity, ship_pos, tp),
            }
        }
    }
}

/// Component of velocity along the vector from `pos` toward `target`.
/// Negative if moving away. Used by the warp alignment check.
pub(super) fn speed_toward(vel: Velocity, pos: Position, target: Position) -> f32 {
    let dx   = target.x - pos.x;
    let dy   = target.y - pos.y;
    let dz   = target.z - pos.z;
    let dist = (dx * dx + dy * dy + dz * dz).sqrt();
    if dist < f32::EPSILON { return 0.0; }
    (vel.dx * dx + vel.dy * dy + vel.dz * dz) / dist
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{NodeId, PlayerId, Position, SectorBounds, SectorId, ShipId, Velocity, WarpTarget};
    use dawn_ecs::components::{ThrustComp, VelocityComp, WarpPhase, ShipStatsComp};

    fn mem_node() -> SimulationNode {
        SimulationNode::new(NodeId(0), SectorId(0), SectorBounds::centered(SectorBounds::DEFAULT_HALF))
    }

    fn spawn_owned_player_at(node: &mut SimulationNode, pos: Position) -> (PlayerId, ShipId) {
        let player_id = node.next_player_id();
        let ship_id   = node.spawn_player_ship_at_pub(player_id, pos);
        (player_id, ship_id)
    }

    // ── Approach (ADR-0015) ──────────────────────────────────────────────

    #[test]
    fn approach_command_attaches_an_approach_target_to_the_owned_ship() {
        let mut node = mem_node();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target = node.spawn_ship(dawn_core::ShipTypeId(1), Position::new(10_000.0, 0.0, 0.0), Velocity::ZERO);

        assert!(node.apply_approach_command_owned(player, dawn_core::ApproachCommand { ship_id: chaser, target: dawn_core::ApproachTarget::Ship(target) }));
        assert_eq!(node.approach_target(chaser), Some(dawn_core::ApproachTarget::Ship(target)));
    }

    #[test]
    fn approach_command_is_rejected_for_a_ship_the_player_does_not_own() {
        let mut node = mem_node();
        let (_owner, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target = node.spawn_ship(dawn_core::ShipTypeId(1), Position::new(10_000.0, 0.0, 0.0), Velocity::ZERO);

        let stranger = node.next_player_id();
        assert!(!node.apply_approach_command_owned(stranger, dawn_core::ApproachCommand { ship_id: chaser, target: dawn_core::ApproachTarget::Ship(target) }));
        assert_eq!(node.approach_target(chaser), None);
    }

    #[test]
    fn approaching_ship_steers_thrust_toward_its_target_each_tick() {
        let mut node = mem_node();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target = node.spawn_ship(dawn_core::ShipTypeId(1), Position::new(10_000.0, 0.0, 0.0), Velocity::ZERO);
        node.apply_approach_command_owned(player, dawn_core::ApproachCommand { ship_id: chaser, target: dawn_core::ApproachTarget::Ship(target) });

        let entity = *node.ships.index.get(&chaser).unwrap();
        node.process_approach();
        let thrust = node.world.inner().get::<&ThrustComp>(entity).unwrap();
        assert!(thrust.direction.dx > 0.9, "thrust should point toward +X target, got {:?}", thrust.direction);
        assert!(!thrust.is_braking);
    }

    #[test]
    fn approaching_ship_closes_distance_to_its_target_over_several_ticks() {
        let mut node = mem_node();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target = node.spawn_ship(dawn_core::ShipTypeId(1), Position::new(10_000.0, 0.0, 0.0), Velocity::ZERO);
        node.apply_approach_command_owned(player, dawn_core::ApproachCommand { ship_id: chaser, target: dawn_core::ApproachTarget::Ship(target) });

        let start = node.get_ship_position(chaser).unwrap().distance(Position::new(10_000.0, 0.0, 0.0));
        for _ in 0..30 { node.tick(); }
        let end = node.get_ship_position(chaser).unwrap().distance(Position::new(10_000.0, 0.0, 0.0));
        assert!(end < start, "approaching ship should reduce distance: {start} -> {end}");
    }

    #[test]
    fn move_command_cancels_an_active_approach() {
        let mut node = mem_node();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target = node.spawn_ship(dawn_core::ShipTypeId(1), Position::new(10_000.0, 0.0, 0.0), Velocity::ZERO);
        node.apply_approach_command_owned(player, dawn_core::ApproachCommand { ship_id: chaser, target: dawn_core::ApproachTarget::Ship(target) });
        assert_eq!(node.approach_target(chaser), Some(dawn_core::ApproachTarget::Ship(target)));

        node.apply_move_command(chaser, Position::new(-10_000.0, 0.0, 0.0));
        assert_eq!(node.approach_target(chaser), None, "manual move must cancel approach");
    }

    #[test]
    fn stop_command_cancels_an_active_approach() {
        let mut node = mem_node();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target = node.spawn_ship(dawn_core::ShipTypeId(1), Position::new(10_000.0, 0.0, 0.0), Velocity::ZERO);
        node.apply_approach_command_owned(player, dawn_core::ApproachCommand { ship_id: chaser, target: dawn_core::ApproachTarget::Ship(target) });

        node.apply_stop_command(chaser);
        assert_eq!(node.approach_target(chaser), None, "stop must cancel approach");
    }

    #[test]
    fn approach_is_dropped_and_ship_brakes_when_the_target_disappears() {
        let mut node = mem_node();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let target = node.spawn_ship(dawn_core::ShipTypeId(1), Position::new(10_000.0, 0.0, 0.0), Velocity::ZERO);
        node.apply_approach_command_owned(player, dawn_core::ApproachCommand { ship_id: chaser, target: dawn_core::ApproachTarget::Ship(target) });

        let target_entity = node.ships.index.remove(&target).unwrap();
        node.world.despawn_ship(target_entity);

        node.process_approach();
        assert_eq!(node.approach_target(chaser), None, "approach must drop when target is gone");
        let entity = *node.ships.index.get(&chaser).unwrap();
        assert!(node.world.inner().get::<&ThrustComp>(entity).unwrap().is_braking, "ship should brake when target vanishes");
    }

    #[test]
    fn approach_command_is_rejected_when_target_is_self() {
        let mut node = mem_node();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        assert!(!node.apply_approach_command_owned(player, dawn_core::ApproachCommand { ship_id: chaser, target: dawn_core::ApproachTarget::Ship(chaser) }));
        assert_eq!(node.approach_target(chaser), None);
    }

    #[test]
    fn approaching_a_jump_gate_steers_the_ship_toward_the_gate_and_into_range() {
        let mut node = mem_node();
        let gate = node.jump_gate(dawn_core::JumpGateId(0)).expect("Sector 0 has Gate 0").clone();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);

        assert!(node.apply_approach_command_owned(player, dawn_core::ApproachCommand {
            ship_id: chaser,
            target : dawn_core::ApproachTarget::Gate(dawn_core::JumpGateId(0)),
        }));
        assert_eq!(node.approach_target(chaser), Some(dawn_core::ApproachTarget::Gate(dawn_core::JumpGateId(0))));

        let start = node.get_ship_position(chaser).unwrap().distance(gate.position);
        for _ in 0..400 { node.tick(); }
        let end = node.get_ship_position(chaser).unwrap().distance(gate.position);
        assert!(end < start, "ship should close on the gate: {start} -> {end}");
        assert!(node.can_propose_jump(chaser, dawn_core::JumpGateId(0)),
            "after approaching, the ship should be within the gate's activation radius");
    }

    #[test]
    fn approach_command_is_rejected_for_a_gate_not_in_this_sector() {
        let mut node = mem_node();
        let (player, chaser) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        assert!(!node.apply_approach_command_owned(player, dawn_core::ApproachCommand {
            ship_id: chaser,
            target : dawn_core::ApproachTarget::Gate(dawn_core::JumpGateId(1)),
        }));
        assert_eq!(node.approach_target(chaser), None);
    }

    // ── Warp (short-range Fold, ADR-0022) ────────────────────────────────

    #[test]
    fn warp_is_rejected_when_the_gate_is_closer_than_the_minimum_warp_distance() {
        let mut node = mem_node();
        let gate = node.jump_gate(dawn_core::JumpGateId(0)).unwrap().clone();
        let (player, ship) = spawn_owned_player_at(&mut node, gate.position);
        assert!(!node.can_propose_warp(ship, WarpTarget::Gate(dawn_core::JumpGateId(0))));
        assert!(!node.apply_warp_command_owned(player, dawn_core::WarpCommand {
            ship_id: ship, target: WarpTarget::Gate(dawn_core::JumpGateId(0)),
        }));
        assert_eq!(node.warp_phase(ship), None);
    }

    #[test]
    fn warp_is_rejected_for_a_gate_not_in_this_sector() {
        let mut node = mem_node();
        let (player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        assert!(!node.apply_warp_command_owned(player, dawn_core::WarpCommand {
            ship_id: ship, target: WarpTarget::Gate(dawn_core::JumpGateId(1)),
        }));
        assert_eq!(node.warp_phase(ship), None);
    }

    #[test]
    fn warp_aligns_by_accelerating_then_flies_into_gate_range_and_completes() {
        let mut node = mem_node();
        let (player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        assert!(node.apply_warp_command_owned(player, dawn_core::WarpCommand {
            ship_id: ship, target: WarpTarget::Gate(dawn_core::JumpGateId(0)),
        }));

        node.tick();
        assert_eq!(node.warp_phase(ship), Some(WarpPhase::Aligning));
        assert!(node.get_ship_position(ship).unwrap().x < 100.0,
            "an aligning ship accelerates sublight, far short of warp speed");

        for _ in 0..80 { node.tick(); }
        assert_eq!(node.warp_phase(ship), None, "warp completes and the component is removed");
        assert!(node.can_propose_jump(ship, dawn_core::JumpGateId(0)),
            "warp drops the ship inside the gate's activation radius");
    }

    #[test]
    fn warp_align_time_emerges_from_ship_agility() {
        fn ticks_to_engage(mass: f32) -> u32 {
            let mut node = SimulationNode::new(NodeId(0), SectorId(0), SectorBounds::centered(SectorBounds::DEFAULT_HALF));
            let player_id = node.next_player_id();
            let ship = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);
            let entity = *node.ships.index.get(&ship).unwrap();
            let mut stats = *node.world.inner().get::<&ShipStatsComp>(entity).unwrap();
            stats.mass = mass;
            node.world.set_ship_stats(entity, stats);
            node.apply_warp_command_owned(player_id, dawn_core::WarpCommand { ship_id: ship, target: WarpTarget::Gate(dawn_core::JumpGateId(0)) });
            for t in 1..=500u32 {
                node.tick();
                if node.warp_phase(ship) == Some(WarpPhase::Warping) { return t; }
            }
            u32::MAX
        }
        assert!(ticks_to_engage(50_000_000.0) > ticks_to_engage(1_000_000.0),
            "a heavier ship spends longer aligning (a longer tackle window)");
    }

    #[test]
    fn warp_decelerates_smoothly_near_the_gate_instead_of_stopping_dead() {
        let mut node = mem_node();
        let (player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        node.apply_warp_command_owned(player, dawn_core::WarpCommand { ship_id: ship, target: WarpTarget::Gate(dawn_core::JumpGateId(0)) });

        let entity = *node.ships.index.get(&ship).unwrap();
        let mut saw_decel_step = false;
        for _ in 0..100 {
            node.tick();
            let warping = node.warp_phase(ship) == Some(WarpPhase::Warping);
            let v = node.world.inner().get::<&VelocityComp>(entity).unwrap().0;
            let speed = (v.dx * v.dx + v.dy * v.dy + v.dz * v.dz).sqrt();
            if warping && speed > f32::EPSILON && speed < WARP_SPEED * 0.9 {
                saw_decel_step = true;
            }
            if node.warp_phase(ship).is_none() { break; }
        }
        assert!(saw_decel_step, "warp must ramp down through intermediate speeds, not stop dead");
        assert_eq!(node.warp_phase(ship), None, "warp should have completed");
    }

    #[test]
    fn a_move_command_cancels_an_aligning_warp() {
        let mut node = mem_node();
        let (player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        node.apply_warp_command_owned(player, dawn_core::WarpCommand { ship_id: ship, target: WarpTarget::Gate(dawn_core::JumpGateId(0)) });
        node.tick();
        assert_eq!(node.warp_phase(ship), Some(WarpPhase::Aligning));

        node.apply_move_command(ship, Position::new(0.0, 1000.0, 0.0));
        assert_eq!(node.warp_phase(ship), None, "a move during alignment cancels the warp");
    }

    #[test]
    fn a_move_command_is_ignored_during_the_committed_warping_phase() {
        let mut node = mem_node();
        let (player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        node.apply_warp_command_owned(player, dawn_core::WarpCommand { ship_id: ship, target: WarpTarget::Gate(dawn_core::JumpGateId(0)) });
        for _ in 0..100 {
            node.tick();
            if node.warp_phase(ship) == Some(WarpPhase::Warping) { break; }
        }
        assert_eq!(node.warp_phase(ship), Some(WarpPhase::Warping), "warp should be committed");

        node.apply_move_command(ship, Position::new(0.0, 1000.0, 0.0));
        assert_eq!(node.warp_phase(ship), Some(WarpPhase::Warping),
            "a committed warp cannot be interrupted by a move command");
    }

    #[test]
    fn auto_jump_is_queued_in_pending_list_when_warp_completes_with_auto_jump_true() {
        let mut node = mem_node();
        let (_player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        assert!(node.apply_warp_command(ship, WarpTarget::Gate(dawn_core::JumpGateId(0)), true));

        for _ in 0..100 { node.tick(); }
        assert_eq!(node.warp_phase(ship), None, "warp must complete");
        assert!(node.can_propose_jump(ship, dawn_core::JumpGateId(0)),
            "ship must be within gate range after warp");

        let pending = node.drain_pending_auto_jumps();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0], (ship, dawn_core::JumpGateId(0)));

        assert!(node.drain_pending_auto_jumps().is_empty());
    }

    #[test]
    fn normal_warp_without_auto_jump_does_not_queue_pending_jump() {
        let mut node = mem_node();
        let (_player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        assert!(node.apply_warp_command(ship, WarpTarget::Gate(dawn_core::JumpGateId(0)), false));

        for _ in 0..100 { node.tick(); }
        assert_eq!(node.warp_phase(ship), None);
        assert!(node.drain_pending_auto_jumps().is_empty());
    }

    // ── Celestial body warp (ADR-0025) ───────────────────────────────────

    #[test]
    fn warp_to_body_reaches_arrival_distance_of_radius_times_1_5() {
        let mut node = mem_node();
        let (player, ship_id) = spawn_owned_player_at(&mut node, Position::new(0.0, 0.0, 0.0));

        let body_id = dawn_core::CelestialBodyId(1);
        let ok = node.apply_warp_command_owned(player, dawn_core::WarpCommand {
            ship_id: ship_id, target: WarpTarget::Body(body_id),
        });
        assert!(ok, "warp to body should be accepted");
        assert!(node.warp_phase(ship_id).is_some(), "ship should have WarpComp");

        for _ in 0..5_000 {
            node.tick();
            if node.warp_phase(ship_id).is_none() { break; }
        }
        assert!(node.warp_phase(ship_id).is_none(), "warp should have completed");

        let body = crate::galaxy::Galaxy::builtin()
            .bodies_in_sector(SectorId(0))
            .into_iter()
            .find(|b| b.id == body_id)
            .unwrap();
        let ship_pos = node.ship_positions()
            .into_iter()
            .find(|(id, _)| *id == ship_id)
            .map(|(_, p)| p)
            .expect("ship exists");
        let dx = ship_pos.x - body.position.x;
        let dy = ship_pos.y - body.position.y;
        let dz = ship_pos.z - body.position.z;
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        let arrival_max = body.radius * 1.5 * 1.05;
        assert!(
            dist <= arrival_max,
            "ship distance {:.0} should be within {:.0} of body centre",
            dist, arrival_max,
        );
    }

    #[test]
    fn warp_to_body_is_rejected_for_body_not_in_this_sector() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::new(0.0, 0.0, 0.0), Velocity::ZERO);
        let ok = node.apply_warp_command(ship_id, WarpTarget::Body(dawn_core::CelestialBodyId(2)), false);
        assert!(!ok, "warp to body in another sector should be rejected");
    }
}
