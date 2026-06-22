//! Navigation commands and per-tick navigation systems for `SimulationNode`.
//!
//! # Contents
//!
//! - `can_propose_jump`  / `can_propose_warp`                  — pre-Raft validation (INV-006)
//! - `apply_approach_command` / `apply_approach_command_owned` — attach `ApproachComp`
//! - `apply_warp_command`     / `apply_warp_command_owned`     — attach `WarpComp`
//! - `drain_pending_auto_jumps`                                — drain the auto-jump queue
//! - `process_approach`  — Approach System (Step 2.5, ADR-0015)
//! - `process_warp`      — Warp System    (Step 2.6, ADR-0022/0023/0025)

use dawn_core::{
    AnchorId, DomainEvent, JumpGateId, PlayerId, Position, ShipId, Tick, Velocity, WarpTarget,
};
use dawn_ecs::{
    components::{ApproachComp, PositionComp, ShipIdComp, ShipStatsComp, ThrustComp, VelocityComp, WarpComp, WarpPhase},
    Entity,
};
use dawn_event_store::store::EventStore;

use super::{
    SimulationNode, WARP_ALIGN_FRACTION, WARP_ARRIVAL_FACTOR, BODY_WARP_ARRIVAL_FACTOR,
    WARP_MIN_TICKS, WARP_SPEED,
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
            warp_start  : Position::ORIGIN,  // set when warp engages (Aligning -> Warping)
            warp_total  : 0,
            warp_elapsed: 0,
            warp_arrival_abs: [0.0, 0.0, 0.0],  // set at engage for Body warps
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
            // Resolve destination, arrival distance, auto-jump gate, and the
            // destination anchor (Body warps rebase onto that body on arrival,
            // ADR-0029; Gate warps stay on the current anchor).
            let resolved = match warp.target {
                WarpTarget::Gate(gate_id) => self.jump_gate(gate_id)
                    .map(|g| (g.position, g.activation_radius * WARP_ARRIVAL_FACTOR, warp.auto_jump.then_some(gate_id), None)),
                WarpTarget::Body(body_id) => self.sector_map.bodies.get(&body_id)
                    .map(|b| (b.position, b.radius * BODY_WARP_ARRIVAL_FACTOR, None, Some(AnchorId::from(body_id)))),
            };
            let Some((dest_world, arrival, auto_jump_gate, dest_anchor)) = resolved else {
                // Target vanished — cancel and brake.
                let _ = self.world.inner_mut().remove_one::<WarpComp>(entity);
                self.brake_thrust(entity);
                continue;
            };
            // Body/gate positions are Sector-frame (== absolute). Express the
            // destination in the ship's CURRENT anchor frame so the parametric
            // walk (pos/vel are anchor-relative) is consistent even if the ship
            // is anchored on a body (ADR-0029). No-op while anchored on the star.
            let dest_pos = self.dest_in_ship_frame(entity, dest_world);

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
                // Aligned: engage warp. Fix the parametric plan (start = here,
                // duration from distance) and take the first step this tick.
                WarpPhase::Aligning => {
                    self.set_warp_phase(entity, WarpPhase::Warping);
                    let arrival_point = warp_arrival_point(pos, dest_pos, arrival);
                    let total = warp_total_ticks(pos.distance(arrival_point));
                    // Precise f64 arrival for Body warps (ADR-0029), stored for
                    // the rebase at arrival so it doesn't read the coarse f32 pos.
                    let arrival_abs = self.warp_arrival_abs(entity, dest_anchor, arrival);
                    if let Ok(mut w) = self.world.inner_mut().get::<&mut WarpComp>(entity) {
                        w.warp_arrival_abs = arrival_abs;
                    }
                    self.warp_step(entity, ship_id, pos, vel, pos, arrival_point, total, 1, auto_jump_gate, dest_anchor, arrival_abs, tick, &mut events);
                }
                // Already warping: advance one tick along the fixed segment plan.
                WarpPhase::Warping => {
                    let arrival_point = warp_arrival_point(warp.warp_start, dest_pos, arrival);
                    self.warp_step(entity, ship_id, pos, vel, warp.warp_start, arrival_point, warp.warp_total, warp.warp_elapsed + 1, auto_jump_gate, dest_anchor, warp.warp_arrival_abs, tick, &mut events);
                }
            }
        }

        events
    }

    /// One warping-phase step (ADR-0022 amendment): walk the segment from
    /// `start` to `arrival_point` over `total` ticks with smoothstep easing,
    /// reaching the destination exactly. This tick is number `elapsed`.
    ///
    /// Each tick sets `velocity = planned_point - current_pos` and emits a
    /// `VelocityChanged`, so replay reconstructs the same path purely by
    /// `position += velocity` (INV-MOVE) — no direct position writes leak past
    /// the velocity record. The final tick settles and stops (velocity ZERO).
    /// `auto_jump_gate`: if `Some(gate_id)`, queue an auto-jump on arrival.
    #[allow(clippy::too_many_arguments)]
    fn warp_step(
        &mut self,
        entity          : Entity,
        ship_id         : ShipId,
        pos             : Position,
        old_vel         : Velocity,
        start           : Position,
        arrival_point   : Position,
        total           : u32,
        elapsed         : u32,
        auto_jump_gate  : Option<JumpGateId>,
        dest_anchor     : Option<AnchorId>,
        arrival_abs     : [f64; 3],
        tick            : Tick,
        events          : &mut Vec<DomainEvent>,
    ) {
        let total = total.max(1);
        let (new_pos, new_vel, arrived) = if elapsed > total {
            // One tick past the final step: settle and stop. The move tick at
            // `elapsed == total` already landed exactly on arrival_point (below,
            // smoothstep(1) = 1), so this just zeroes velocity — keeping the
            // motion velocity-recorded (INV-MOVE) AND the arrival exact.
            (pos, Velocity::ZERO, true)
        } else {
            // Eased point along the segment this tick; velocity carries the
            // delta so the move is recorded by VelocityChanged (INV-MOVE).
            // At elapsed == total, smoothstep(1) = 1 → planned = arrival_point.
            let s = smoothstep(elapsed as f32 / total as f32);
            let planned = Position {
                x: start.x + (arrival_point.x - start.x) * s,
                y: start.y + (arrival_point.y - start.y) * s,
                z: start.z + (arrival_point.z - start.z) * s,
            };
            let v = Velocity {
                dx: planned.x - pos.x,
                dy: planned.y - pos.y,
                dz: planned.z - pos.z,
            };
            (planned, v, false)
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
        } else if let Ok(mut w) = self.world.inner_mut().get::<&mut WarpComp>(entity) {
            // Persist the plan + progress for the next tick.
            w.warp_start   = start;
            w.warp_total   = total;
            w.warp_elapsed = elapsed;
        }

        // Emit VelocityChanged only when velocity actually changed (INV-MOVE).
        let changed = (new_vel.dx - old_vel.dx).abs() > f32::EPSILON
            || (new_vel.dy - old_vel.dy).abs() > f32::EPSILON
            || (new_vel.dz - old_vel.dz).abs() > f32::EPSILON;
        if changed {
            events.push(DomainEvent::VelocityChanged(dawn_core::events::VelocityChanged {
                ship_id,
                velocity: new_vel,
                tick,
            }));
        }

        // On Body arrival, rebase the ship onto the destination body's anchor
        // (ADR-0029 step 4): keep the absolute position, re-express the offset
        // relative to the new anchor so subsequent local f32 motion near the
        // body stays precise at true-AU distances.
        if arrived {
            if let Some(to) = dest_anchor {
                if let Some(ev) = self.rebase_arrival_event(entity, ship_id, to, arrival_abs, tick) {
                    events.push(ev);
                }
            }
        }
    }

    /// Compute and apply a coordinate rebase for a ship that just arrived at a
    /// body anchor (ADR-0029). Re-expresses the ship's current absolute position
    /// relative to `to`, writes the new anchor + offset, and returns the
    /// authoritative `AnchorRebased` event. Returns `None` if either the current
    /// or destination anchor is unknown (leaves the ship on its old anchor).
    fn rebase_arrival_event(&mut self, entity: Entity, ship_id: ShipId, to: AnchorId, arrival_abs: [f64; 3], tick: Tick) -> Option<DomainEvent> {
        let cur_anchor = self.world.ship_anchor(entity)?;
        if cur_anchor == to {
            return None;
        }
        let to_abs = self.anchor_table.abs(to)?;
        // Prefer the precise f64 arrival point (set at engage) over the coarse
        // f32 PositionComp, which is ~tens of km off near a true-AU anchor
        // (ADR-0029). Fall back to the offset compose if arrival is unset.
        let world = if arrival_abs != [0.0, 0.0, 0.0] {
            arrival_abs
        } else {
            let offset = self.world.inner().get::<&PositionComp>(entity).ok()?.0;
            self.anchor_table.absolute(cur_anchor, offset)?
        };
        let new_off = Position::new(
            (world[0] - to_abs[0]) as f32,
            (world[1] - to_abs[1]) as f32,
            (world[2] - to_abs[2]) as f32,
        );
        self.world.set_ship_anchor(entity, to);
        if let Ok(mut p) = self.world.inner_mut().get::<&mut PositionComp>(entity) {
            p.0 = new_off;
        }
        Some(DomainEvent::AnchorRebased(dawn_core::events::AnchorRebased {
            ship_id,
            anchor: to,
            offset: new_off,
            tick,
        }))
    }

    /// Precise absolute (f64) warp arrival point for a Body warp: `arrival`
    /// metres short of the body centre along the ship's approach, using the f64
    /// anchor source (ADR-0029). Returns `[0,0,0]` for Gate warps (no rebase).
    fn warp_arrival_abs(&self, entity: Entity, dest_anchor: Option<AnchorId>, arrival: f32) -> [f64; 3] {
        let Some(to) = dest_anchor else { return [0.0, 0.0, 0.0] };
        let Some(body_abs) = self.anchor_table.abs(to) else { return [0.0, 0.0, 0.0] };
        let offset = self.world.inner().get::<&PositionComp>(entity).ok().map(|p| p.0).unwrap_or(Position::ORIGIN);
        let start = self.entity_absolute_f64(entity, offset);
        let d = [body_abs[0] - start[0], body_abs[1] - start[1], body_abs[2] - start[2]];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if len <= f64::EPSILON {
            return body_abs;
        }
        let a = arrival as f64;
        [body_abs[0] - d[0] / len * a, body_abs[1] - d[1] / len * a, body_abs[2] - d[2] / len * a]
    }

    /// Convert a Sector-frame (absolute) destination position into the ship's
    /// current anchor frame, so warp math stays in the same frame as the ship's
    /// anchor-relative `PositionComp` (ADR-0029). Falls back to the raw position
    /// if the anchor is unknown (no-op while anchored on the star at the origin).
    fn dest_in_ship_frame(&self, entity: Entity, dest_world: Position) -> Position {
        let Some(anchor) = self.world.ship_anchor(entity) else { return dest_world };
        let Some(a) = self.anchor_table.abs(anchor) else { return dest_world };
        Position::new(
            dest_world.x - a[0] as f32,
            dest_world.y - a[1] as f32,
            dest_world.z - a[2] as f32,
        )
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

        for (entity, target, ship_offset) in approachers {
            // Work in absolute (Sector-frame) coordinates so distance/steering
            // are correct even if approacher and target sit on different anchors
            // (ADR-0029). Gate positions are already Sector-frame.
            let ship_pos = self.entity_absolute(entity, ship_offset);
            // Resolve the target's current position and the arrival distance.
            // `None` means the target no longer exists.
            let resolved: Option<(Position, f32)> = match target {
                ApproachTarget::Ship(target_id) => self.ships.index.get(&target_id).copied()
                    .and_then(|te| self.world.inner().get::<&PositionComp>(te).ok().map(|p| (te, p.0)))
                    .map(|(te, off)| (self.entity_absolute(te, off), SHIP_ARRIVAL_RADIUS)),
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

/// The point on the segment from `start` toward `dest`, `arrival` units short
/// of `dest` — where a warp settles (gate ring / body orbit). If the ship is
/// already inside the arrival ring, returns `start` (no forward motion).
/// ADR-0022 amendment (parametric warp).
fn warp_arrival_point(start: Position, dest: Position, arrival: f32) -> Position {
    let dx = dest.x - start.x;
    let dy = dest.y - start.y;
    let dz = dest.z - start.z;
    let dist = (dx * dx + dy * dy + dz * dz).sqrt();
    if dist <= arrival || dist < f32::EPSILON {
        return start;
    }
    let f = (dist - arrival) / dist;
    Position { x: start.x + dx * f, y: start.y + dy * f, z: start.z + dz * f }
}

/// Warp duration in ticks from the warp distance (start→arrival point), floored
/// at `WARP_MIN_TICKS` so even a short warp reads as a warp. ADR-0022 amendment.
fn warp_total_ticks(warp_dist: f32) -> u32 {
    let n = (warp_dist / WARP_SPEED).ceil().max(0.0) as u32;
    n.max(WARP_MIN_TICKS)
}

/// Smoothstep ease (0→1): accelerate out of warp entry, decelerate into the
/// arrival ring. ADR-0022 amendment (parametric warp).
fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
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

// ── Pre-Raft validation (INV-006) ────────────────────────────────────────────

impl<S: EventStore> SimulationNode<S> {
    /// Whether a `JumpCommand` for `ship_id` via `gate_id` would currently be
    /// accepted: the Ship exists, is not already in transit, the gate
    /// originates in this Sector, and the Ship is within its `activation_radius`.
    /// Used to reject commands up front, before proposing to the Raft Log (INV-006).
    pub fn can_propose_jump(&self, ship_id: ShipId, gate_id: JumpGateId) -> bool {
        let Some(&entity) = self.ships.index.get(&ship_id) else { return false };
        if self.world.transit_state(entity).is_in_transit() { return false; }
        if self.world.is_tackled(entity) { return false; }
        let Some(gate) = self.sector_map.gates.get(&gate_id) else { return false };
        // Compare in absolute (Sector-frame) f64 coords: the gate's f64 `abs_m` is
        // Sector-frame, the ship offset is anchor-relative. f64 keeps the check
        // precise at true-AU distances (ADR-0029 review R1 / #4).
        let offset = self.world.inner().get::<&PositionComp>(entity).map(|p| p.0).unwrap_or(Position::ORIGIN);
        gate.is_in_range_abs(self.entity_absolute_f64(entity, offset))
    }

    /// Whether a `WarpCommand` for `ship_id` toward `target` would currently be
    /// accepted (INV-006 Validation, before attaching `WarpComp`):
    /// the Ship exists, is not in transit, is not already warping, not tackled,
    /// the target belongs to this Sector, and is at least `MIN_WARP_DISTANCE` away.
    pub fn can_propose_warp(&self, ship_id: ShipId, target: WarpTarget) -> bool {
        let Some(&entity) = self.ships.index.get(&ship_id) else { return false };
        if self.world.transit_state(entity).is_in_transit() { return false; }
        if self.world.inner().get::<&WarpComp>(entity).is_ok() { return false; }
        if self.world.is_tackled(entity) { return false; }
        // Absolute (Sector-frame) f64 ship position vs the f64 gate/body source
        // (ADR-0029 R1 / #4 — never compare a raw anchor offset to absolute data,
        // and keep the distance precise at true-AU scale).
        let offset   = self.world.inner().get::<&PositionComp>(entity).map(|p| p.0).unwrap_or(Position::ORIGIN);
        let ship_abs = self.entity_absolute_f64(entity, offset);
        let min      = super::MIN_WARP_DISTANCE as f64;
        match target {
            WarpTarget::Gate(gate_id) => {
                let Some(gate) = self.sector_map.gates.get(&gate_id) else { return false };
                gate.distance_abs(ship_abs) >= min
            }
            WarpTarget::Body(body_id) => {
                let Some(body) = self.sector_map.bodies.get(&body_id) else { return false };
                let d = [ship_abs[0] - body.abs_m[0], ship_abs[1] - body.abs_m[1], ship_abs[2] - body.abs_m[2]];
                (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() >= min
            }
        }
    }
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
        // Start near the gate: at the wide system scale you warp across the
        // system and only sublight-approach over the last stretch (the gate is
        // ~600,000 units out — far beyond sublight range in a test budget).
        let near_gate = Position::new(gate.position.x - 12_000.0, 0.0, 0.0);
        let (player, chaser) = spawn_owned_player_at(&mut node, near_gate);

        assert!(node.apply_approach_command_owned(player, dawn_core::ApproachCommand {
            ship_id: chaser,
            target : dawn_core::ApproachTarget::Gate(dawn_core::JumpGateId(0)),
        }));
        assert_eq!(node.approach_target(chaser), Some(dawn_core::ApproachTarget::Gate(dawn_core::JumpGateId(0))));

        let start = node.ship_distance_to_point(chaser, gate.position).unwrap();
        for _ in 0..600 { node.tick(); }
        let end = node.ship_distance_to_point(chaser, gate.position).unwrap();
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

        for _ in 0..250 { node.tick(); }
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
        for _ in 0..250 {
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

        for _ in 0..250 { node.tick(); }
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

        for _ in 0..250 { node.tick(); }
        assert_eq!(node.warp_phase(ship), None);
        assert!(node.drain_pending_auto_jumps().is_empty());
    }

    #[test]
    fn parametric_warp_lasts_a_floored_duration_not_an_instant_teleport() {
        // ADR-0022 amendment: warp walks the start→arrival segment over
        // max(WARP_MIN_TICKS, ceil(dist / WARP_SPEED)) ticks, so even a warp
        // whose distance would finish in a couple of ticks still spends a
        // floored number of ticks in the committed Warping phase.
        let mut node = mem_node();
        let (player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        node.apply_warp_command_owned(player, dawn_core::WarpCommand {
            ship_id: ship, target: WarpTarget::Gate(dawn_core::JumpGateId(0)),
        });

        let mut warping_ticks: u32 = 0;
        for _ in 0..400 {
            node.tick();
            match node.warp_phase(ship) {
                Some(WarpPhase::Warping) => warping_ticks += 1,
                None if warping_ticks > 0 => break,  // warp finished
                _ => {}
            }
        }
        // Allow a small boundary fuzz (the arriving tick removes the component).
        assert!(warping_ticks >= WARP_MIN_TICKS - 2,
            "warp should ride the parametric segment for ~WARP_MIN_TICKS ticks, \
             got {warping_ticks} (floor {WARP_MIN_TICKS})");
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

        // ADR-0029 step 4: arriving at a Body rebases the ship onto that body's anchor.
        assert_eq!(node.get_ship_anchor(ship_id), Some(dawn_core::AnchorId::from(body_id)),
            "warp-to-body should rebase the ship onto the body anchor");

        let body = crate::galaxy::Galaxy::demo()
            .bodies_in_sector(SectorId(0))
            .into_iter()
            .find(|b| b.id == body_id)
            .unwrap();
        // After a Body warp the ship is rebased onto the body's anchor (ADR-0029),
        // so its raw PositionComp is now body-relative. Compare in absolute terms.
        let ship_abs = node.ship_absolute(ship_id).expect("ship exists");
        let dx = ship_abs[0] - body.position.x as f64;
        let dy = ship_abs[1] - body.position.y as f64;
        let dz = ship_abs[2] - body.position.z as f64;
        let dist = (dx * dx + dy * dy + dz * dz).sqrt() as f32;
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
