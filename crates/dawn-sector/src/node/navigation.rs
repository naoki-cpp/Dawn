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
