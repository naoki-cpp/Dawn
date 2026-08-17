//! Warp commands and the per-tick Warp System for `SimulationNode`
//! (short-range Fold, ADR-0022/0023/0025/0029). Split out of `navigation.rs`
//! (ADR-0030 review R-1, 2026-06-23): pure move, no behavior change.
//!
//! # Contents
//!
//! - `apply_warp_command` / `apply_warp_command_owned` — attach `WarpComp`
//! - `drain_pending_auto_jumps` / `drain_completed_warps` — per-tick drain queues
//! - `process_warp` — Warp System (Step 2.6, ADR-0022/0023/0025)

use dawn_core::{
    AnchorId, DomainEvent, JumpGateId, PlayerId, Position, ShipId, Tick, Velocity, WarpTarget,
};
use dawn_ecs::{
    components::{
        PositionComp, ShipIdComp, ShipStatsComp, ThrustComp, VelocityComp, WarpComp, WarpPhase,
    },
    Entity,
};

use super::{
    SimulationNode, BODY_WARP_ARRIVAL_FACTOR, WARP_ALIGN_FRACTION, WARP_ARRIVAL_FACTOR,
    WARP_MIN_TICKS, WARP_SPEED,
};

impl SimulationNode {
    /// Begin an intra-Sector warp toward a gate, body, or station
    /// (short-range Fold, ADR-0022).
    ///
    /// Attaches a `WarpComp` in the `Aligning` phase if `can_propose_warp`
    /// accepts the request; `process_warp()` then advances the alignment and,
    /// once aligned, flies the ship to the gate at warp speed. Returns `false`
    /// (no component attached) on rejection.
    pub(crate) fn apply_warp_command(
        &mut self,
        ship_id: ShipId,
        target: WarpTarget,
        auto_jump: bool,
    ) -> bool {
        if !self.can_propose_warp(ship_id, target) {
            return false;
        }
        let &entity = match self.simulation.ships.index.get(&ship_id) {
            Some(e) => e,
            None => return false,
        };
        // Warp is also a persistent steering mode (ADR-0031): starting one
        // must clear any active Orbit/KeepAtRange/Approach so they don't keep
        // computing thrust for an entity that's now warping (previously
        // relied on tick order alone -- process_warp happens to run after
        // them and overwrite ThrustComp each tick, which masked this).
        self.clear_steering_modes(entity);
        let _ = self.simulation.world.insert_one(
            entity,
            WarpComp {
                target,
                phase: WarpPhase::Aligning,
                auto_jump,
                warp_start_abs: dawn_core::AbsolutePosition::ORIGIN, // set when warp engages (Aligning -> Warping)
                warp_total: 0,
                warp_elapsed: 0,
                warp_arrival_abs: dawn_core::AbsolutePosition::ORIGIN, // set at engage
                warp_start_vel: Velocity::ZERO,                        // set at engage
            },
        );
        true
    }

    /// `apply_warp_command` wrapped with the shared flight-command seam
    /// (active-ship + undocked, ADR-0037; `ship_command.rs`).
    pub(crate) fn apply_warp_command_owned(
        &mut self,
        player_id: PlayerId,
        ship_id: ShipId,
        cmd: dawn_core::WarpCommand,
    ) -> bool {
        if self.resolve_flight_command(player_id, ship_id).is_err() {
            return false;
        }
        self.apply_warp_command(ship_id, cmd.target, false)
    }

    /// Drain auto-jump triggers accumulated during `process_warp()`.
    ///
    /// The current server loop proposes each returned `(ship_id, gate_id)` pair
    /// to the Raft log in cluster mode or ignores it in single-node mode where
    /// Jump is not supported. This in-memory drain is a migration baseline,
    /// not the durable authority: ADR-0049 requires the Warp transition to
    /// commit a replayable/idempotent continuation obligation, which #276 may
    /// represent as Transit Saga state before this queue is drained.
    pub(crate) fn drain_pending_auto_jumps(&mut self) -> Vec<(ShipId, JumpGateId)> {
        std::mem::take(&mut self.frame_outputs.pending_auto_jumps)
    }

    /// Drain the ships that finished a warp this tick (ADR-0029 warp-arrival
    /// authority). The serve loop sends each one's owner an authoritative
    /// `PositionSnap` so the client's capped warp-visual dead-reckoning lands
    /// on the true arrival point. This vector is a lossy presentation queue,
    /// cleared after delivery; it is separate from the durable auto-jump
    /// obligation required by ADR-0049.
    pub(crate) fn drain_completed_warps(&mut self) -> Vec<ShipId> {
        std::mem::take(&mut self.frame_outputs.completed_warps)
    }

    #[cfg(test)]
    pub(crate) fn queue_runtime_transients_for_test(
        &mut self,
        auto_jump: (ShipId, JumpGateId),
        completed_warp: ShipId,
    ) {
        self.frame_outputs.pending_auto_jumps.push(auto_jump);
        self.frame_outputs.completed_warps.push(completed_warp);
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
    pub(crate) fn process_warp(&mut self, tick: Tick) -> Vec<DomainEvent> {
        // Collect warpers up front so the ECS query borrow is released before
        // the mutable write pass below.
        let warpers: Vec<(Entity, ShipId, WarpComp, Position, Velocity, f64)> = self
            .simulation
            .world
            .query::<(
                Entity,
                &ShipIdComp,
                &WarpComp,
                &PositionComp,
                &VelocityComp,
                &ShipStatsComp,
            )>()
            .iter()
            .map(|(e, id, w, p, v, s)| (e, id.0, *w, p.0, v.0, s.max_speed))
            .collect();

        let mut events = Vec::new();

        for (entity, ship_id, warp, pos, vel, max_speed) in warpers {
            // Resolve destination, arrival distance, auto-jump gate, and the
            // destination anchor (Body/Station warps rebase onto the nearest
            // local body anchor on arrival,
            // ADR-0029; Gate warps stay on the current anchor).
            let resolved = match warp.target {
                // Gate arrival is now symmetric with Body (ADR-0029 R1 remnant):
                // the f64 `abs_m` source feeds the precise arrival point, and the
                // ship rebases onto whichever body anchor sits nearest the gate
                // (gates have no anchor of their own — §2 anchors are per-body).
                WarpTarget::Gate(gate_id) => self.jump_gate(gate_id).map(|g| {
                    (
                        g.position,
                        g.activation_radius * WARP_ARRIVAL_FACTOR,
                        warp.auto_jump.then_some(gate_id),
                        self.topology
                            .anchor_table
                            .nearest_anchor(g.from_sector, g.abs_m),
                        g.abs_m,
                    )
                }),
                WarpTarget::Body(body_id) => {
                    self.topology.sector_map.bodies.get(&body_id).map(|b| {
                        (
                            b.position,
                            b.radius * BODY_WARP_ARRIVAL_FACTOR,
                            None,
                            Some(AnchorId::from(body_id)),
                            b.abs_m,
                        )
                    })
                }
                WarpTarget::Station(station_id) => self
                    .topology
                    .sector_map
                    .stations
                    .get(&station_id)
                    .map(|station| {
                        (
                            station.position,
                            station.docking_radius * WARP_ARRIVAL_FACTOR,
                            None,
                            self.topology
                                .anchor_table
                                .nearest_anchor(station.sector, station.abs_m),
                            station.abs_m,
                        )
                    }),
            };
            let Some((dest_world, arrival, auto_jump_gate, dest_anchor, target_abs)) = resolved
            else {
                // Target vanished — cancel and brake.
                let _ = self.simulation.world.remove_one::<WarpComp>(entity);
                self.brake_thrust(entity);
                continue;
            };
            // Body/gate positions are Sector-frame (== absolute). Express the
            // destination in the ship's CURRENT anchor frame for the Aligning-phase
            // steering check below (direction only, not the precise transit math —
            // that runs in absolute f64, see warp_step). No-op while anchored on the star.
            let dest_pos = self.dest_in_ship_frame(entity, dest_world);

            // Tackle interrupts the aligning phase (ADR-0024): a tackled ship
            // cannot enter warp. Cancel and brake; the Warping phase is committed.
            if warp.phase == WarpPhase::Aligning && self.simulation.world.is_tackled(entity) {
                let _ = self.simulation.world.remove_one::<WarpComp>(entity);
                self.brake_thrust(entity);
                continue;
            }

            // Engage warp once aligned: moving at ≥ 75% of max speed toward the
            // destination. While not yet aligned, keep steering/accelerating at it.
            let aligned = max_speed > f64::EPSILON
                && speed_toward(vel, pos, dest_pos) >= WARP_ALIGN_FRACTION * max_speed;

            match warp.phase {
                WarpPhase::Aligning if !aligned => {
                    self.steer_thrust_toward(entity, pos, dest_pos);
                }
                // Aligned: engage warp. Fix the parametric plan (start = here,
                // duration from distance) and take the first step this tick.
                // The whole transit is planned in absolute f64 (ADR-0029 §1.3):
                // unlike the old low-precision anchor-relative plan, this does not pick up
                // ulp error from the ship's anchor when the destination sits at
                // true-AU distance from it, and PositionComp remains f64 throughout.
                WarpPhase::Aligning => {
                    self.set_warp_phase(entity, WarpPhase::Warping);
                    let start_abs = self.entity_absolute_f64(entity, pos);
                    let arrival_abs = self.warp_arrival_abs(entity, target_abs, arrival);
                    let d = [
                        arrival_abs[0] - start_abs[0],
                        arrival_abs[1] - start_abs[1],
                        arrival_abs[2] - start_abs[2],
                    ];
                    let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                    let total = warp_total_ticks(dist);
                    // Carry the ship's actual speed at engage into the transit
                    // curve (lore pass, ADR-0022/0029): warp_step blends this in
                    // as the curve's start tangent so the Aligning cruise speed
                    // flows continuously into the warp-speed ramp, instead of
                    // snapping to near-zero before re-accelerating.
                    if let Some(mut w) = self.simulation.world.get_mut::<WarpComp>(entity) {
                        w.warp_start_abs = start_abs;
                        w.warp_arrival_abs = arrival_abs.into();
                        w.warp_start_vel = vel;
                    }
                    self.warp_step(
                        entity,
                        ship_id,
                        pos,
                        vel,
                        start_abs.as_array(),
                        arrival_abs,
                        vel,
                        total,
                        1,
                        auto_jump_gate,
                        dest_anchor,
                        tick,
                        &mut events,
                    );
                }
                // Already warping: advance one tick along the fixed segment plan.
                WarpPhase::Warping => {
                    self.warp_step(
                        entity,
                        ship_id,
                        pos,
                        vel,
                        warp.warp_start_abs.as_array(),
                        warp.warp_arrival_abs.as_array(),
                        warp.warp_start_vel,
                        warp.warp_total,
                        warp.warp_elapsed + 1,
                        auto_jump_gate,
                        dest_anchor,
                        tick,
                        &mut events,
                    );
                }
            }
        }

        events
    }

    /// One warping-phase step (ADR-0022 amendment, ADR-0029 absolute-frame
    /// transit, 2026-06-23 lore pass): walk the segment from `start_abs` to
    /// `arrival_abs` — both Sector-frame f64 — over `total` ticks. This tick
    /// is number `elapsed`.
    ///
    /// The path is a cubic Hermite spline with `start_vel` (the ship's actual
    /// speed at the moment warp engaged) as its start tangent and a zero end
    /// tangent, instead of the plain smoothstep ease this used before: plain
    /// smoothstep has a zero derivative at *both* ends, so velocity used to
    /// snap from the Aligning-phase cruise speed down to near-zero the instant
    /// warp engaged, then ramp back up — a visible stutter at the very moment
    /// that should read as "entering the tunnel". The Hermite blend keeps
    /// speed continuous into the ramp while still decelerating smoothly to a
    /// full stop exactly at `arrival_abs`, so the whole transit reads as one
    /// continuous accelerate → cruise → decelerate motion.
    ///
    /// The eased point is computed in f64 and written directly to the ship's
    /// f64 `PositionComp`, so error does not compound across ticks the way
    /// repeated low-precision interpolation did. Nothing reads a
    /// warping ship's exact position — Movement skips it, combat can't target
    /// it). Each tick sets `velocity = planned_point - current_pos` and emits
    /// a `VelocityChanged` for public motion projections. Exact position and
    /// velocity recovery comes from RecoveryDelta/checkpoint state, not from
    /// integrating the public event stream.
    /// `position += velocity` (INV-MOVE) — no direct position writes leak past
    /// The final tick settles and stops (velocity ZERO).
    /// `auto_jump_gate`: if `Some(gate_id)`, queue an auto-jump on arrival.
    #[allow(clippy::too_many_arguments)]
    fn warp_step(
        &mut self,
        entity: Entity,
        ship_id: ShipId,
        pos: Position,
        old_vel: Velocity,
        start_abs: [f64; 3],
        arrival_abs: [f64; 3],
        start_vel: Velocity,
        total: u32,
        elapsed: u32,
        auto_jump_gate: Option<JumpGateId>,
        dest_anchor: Option<AnchorId>,
        tick: Tick,
        events: &mut Vec<DomainEvent>,
    ) {
        let total = total.max(1);
        let anchor_abs = self
            .simulation
            .world
            .ship_anchor(entity)
            .and_then(|a| self.topology.anchor_table.abs(a))
            .unwrap_or(dawn_core::AbsolutePosition::ORIGIN);
        let (new_pos, new_vel, arrived) = compute_warp_step(
            pos,
            start_abs,
            arrival_abs,
            start_vel,
            anchor_abs.as_array(),
            total,
            elapsed,
        );

        if let Some(mut p) = self.simulation.world.get_mut::<PositionComp>(entity) {
            p.0 = new_pos;
        }
        if let Some(mut v) = self.simulation.world.get_mut::<VelocityComp>(entity) {
            v.0 = new_vel;
        }

        if arrived {
            // Warp complete: drop the component and clear thrust. Movement resumes next tick.
            let _ = self.simulation.world.remove_one::<WarpComp>(entity);
            if let Some(mut t) = self.simulation.world.get_mut::<ThrustComp>(entity) {
                t.direction = Velocity::ZERO;
                t.is_braking = false;
            }
            // Queue auto-jump for Gate targets (ADR-0023).
            if let Some(gid) = auto_jump_gate {
                self.frame_outputs.pending_auto_jumps.push((ship_id, gid));
            }
            // Authoritative arrival: the serve loop sends the owner a PositionSnap
            // so its capped warp-visual dead-reckoning is corrected, regardless of
            // whether the arrival rebased the anchor (ADR-0029 warp-arrival authority).
            self.frame_outputs.completed_warps.push(ship_id);
        } else if let Some(mut w) = self.simulation.world.get_mut::<WarpComp>(entity) {
            // Persist the plan + progress for the next tick.
            w.warp_start_abs = start_abs.into();
            w.warp_total = total;
            w.warp_elapsed = elapsed;
        }

        // Emit VelocityChanged only when velocity actually changed (INV-MOVE).
        let changed = (new_vel.dx - old_vel.dx).abs() > f64::EPSILON
            || (new_vel.dy - old_vel.dy).abs() > f64::EPSILON
            || (new_vel.dz - old_vel.dz).abs() > f64::EPSILON;
        if changed {
            events.push(DomainEvent::VelocityChanged(
                dawn_core::events::VelocityChanged {
                    ship_id,
                    velocity: new_vel,
                    tick,
                },
            ));
        }

        // On Body arrival, rebase the ship onto the destination body's anchor
        // (ADR-0029 step 4): keep the absolute position, re-express the offset
        // relative to the new anchor so subsequent local f64 motion near the
        // body stays precise at true-AU distances.
        if arrived {
            if let Some(to) = dest_anchor {
                if let Some(ev) = self.rebase_arrival_event(entity, ship_id, to, arrival_abs, tick)
                {
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
    fn rebase_arrival_event(
        &mut self,
        entity: Entity,
        ship_id: ShipId,
        to: AnchorId,
        arrival_abs: [f64; 3],
        tick: Tick,
    ) -> Option<DomainEvent> {
        let cur_anchor = self.simulation.world.ship_anchor(entity)?;
        if cur_anchor == to {
            return None;
        }
        let to_abs = self.topology.anchor_table.abs(to)?;
        // Prefer the precise f64 arrival point (set at engage) over a position
        // reconstructed from an unrelated anchor (ADR-0029). Fall back to the
        // offset compose if arrival is unset.
        let world = if arrival_abs != [0.0, 0.0, 0.0] {
            dawn_core::AbsolutePosition::from(arrival_abs)
        } else {
            let offset = self.simulation.world.get::<PositionComp>(entity)?.0;
            self.topology.anchor_table.absolute(cur_anchor, offset)?
        };
        let new_off = Position::new(
            world[0] - to_abs[0],
            world[1] - to_abs[1],
            world[2] - to_abs[2],
        );
        self.simulation.world.set_ship_anchor(entity, to);
        if let Some(mut p) = self.simulation.world.get_mut::<PositionComp>(entity) {
            p.0 = new_off;
        }
        Some(DomainEvent::AnchorRebased(
            dawn_core::events::AnchorRebased {
                ship_id,
                anchor: to,
                offset: new_off,
                tick,
            },
        ))
    }

    /// Precise absolute (f64) warp arrival point: `arrival` metres short of
    /// `target_abs` along the ship's approach, using the f64 source for the
    /// warp target — a body centre (`CelestialBodyDef.abs_m`) or a gate
    /// (`JumpGateDef.abs_m`, ADR-0029 R1). Symmetric for Gate and Body targets.
    fn warp_arrival_abs<P: Into<[f64; 3]>>(
        &self,
        entity: Entity,
        target_abs: P,
        arrival: f64,
    ) -> [f64; 3] {
        let target_abs = target_abs.into();
        let offset = self
            .simulation
            .world
            .get::<PositionComp>(entity)
            .map(|p| p.0)
            .unwrap_or(Position::ORIGIN);
        let start = self.entity_absolute_f64(entity, offset);
        let d = [
            target_abs[0] - start[0],
            target_abs[1] - start[1],
            target_abs[2] - start[2],
        ];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let a = arrival;
        // Already inside the arrival ring: stay put rather than overshoot past
        // `start` away from the target (the old `warp_arrival_point` guarded
        // the same case before this method became the sole flight target, not
        // just the rebase point — ADR-0029 absolute-frame transit).
        if len <= a || len <= f64::EPSILON {
            return start.into();
        }
        [
            target_abs[0] - d[0] / len * a,
            target_abs[1] - d[1] / len * a,
            target_abs[2] - d[2] / len * a,
        ]
    }

    /// Convert a Sector-frame (absolute) destination position into the ship's
    /// current anchor frame, so warp math stays in the same frame as the ship's
    /// anchor-relative `PositionComp` (ADR-0029). Takes an f64 `dest_world`, so
    /// it preserves the precision needed for warp's
    /// Aligning-phase steering (direction only, ADR-0029), not for anything
    /// needing arrival-radius precision (use `dest_in_ship_frame_abs` with an
    /// f64 source for that — this delegates to it after upcasting).
    fn dest_in_ship_frame(&self, entity: Entity, dest_world: Position) -> Position {
        self.dest_in_ship_frame_abs(entity, [dest_world.x, dest_world.y, dest_world.z].into())
    }

    /// Overwrite the phase of a ship's `WarpComp` (no-op if absent).
    fn set_warp_phase(&mut self, entity: Entity, phase: WarpPhase) {
        if let Some(mut w) = self.simulation.world.get_mut::<WarpComp>(entity) {
            w.phase = phase;
        }
    }
}

/// One warping-phase step's position/velocity/arrival (candidate 6): pure
/// Hermite-spline math extracted from `warp_step`, given the ship's current
/// anchor-relative position, the absolute-frame start/arrival points, the
/// tangent start velocity, the current anchor's absolute position, and the
/// segment's total/elapsed tick counts. No ECS/event-store access -- see
/// `warp_step`'s doc comment for the curve rationale.
#[allow(clippy::too_many_arguments)]
fn compute_warp_step(
    pos: Position,
    start_abs: [f64; 3],
    arrival_abs: [f64; 3],
    start_vel: Velocity,
    anchor_abs: [f64; 3],
    total: u32,
    elapsed: u32,
) -> (Position, Velocity, bool) {
    if elapsed > total {
        // One tick past the final step: settle and stop. The move tick at
        // `elapsed == total` already landed exactly on arrival_abs (below,
        // h01(1) = 1), so this just zeroes velocity — keeping the motion
        // velocity-recorded (INV-MOVE) AND the arrival exact.
        return (pos, Velocity::ZERO, true);
    }
    // Cubic Hermite spline (start tangent = start_vel, end tangent = zero)
    // along the segment this tick, in absolute f64; velocity carries the
    // delta so the move is recorded by VelocityChanged (INV-MOVE). At
    // elapsed == total, t = 1 → planned = arrival_abs exactly
    // (h00(1)=h10(1)=0, h01(1)=1) regardless of start_vel.
    let t = (elapsed as f64 / total as f64).clamp(0.0, 1.0);
    let t2 = t * t;
    let t3 = t2 * t;
    let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
    let h10 = t3 - 2.0 * t2 + t;
    let h01 = -2.0 * t3 + 3.0 * t2;
    // Tangent scaled by `total` so its per-tick rate at t=0 equals the
    // ship's actual entering speed (the curve parameter spans total ticks
    // over t in [0,1], so d(planned)/d(elapsed) = d(planned)/dt / total).
    let m0 = [
        start_vel.dx * total as f64,
        start_vel.dy * total as f64,
        start_vel.dz * total as f64,
    ];
    let planned_abs = [
        h00 * start_abs[0] + h10 * m0[0] + h01 * arrival_abs[0],
        h00 * start_abs[1] + h10 * m0[1] + h01 * arrival_abs[1],
        h00 * start_abs[2] + h10 * m0[2] + h01 * arrival_abs[2],
    ];
    let planned = Position::new(
        planned_abs[0] - anchor_abs[0],
        planned_abs[1] - anchor_abs[1],
        planned_abs[2] - anchor_abs[2],
    );
    let v = Velocity {
        dx: planned.x - pos.x,
        dy: planned.y - pos.y,
        dz: planned.z - pos.z,
    };
    (planned, v, false)
}

/// Warp duration in ticks from the warp distance (start→arrival point), floored
/// at `WARP_MIN_TICKS` so even a short warp reads as a warp. ADR-0022 amendment.
fn warp_total_ticks(warp_dist: f64) -> u32 {
    let n = (warp_dist / WARP_SPEED).ceil().max(0.0) as u32;
    n.max(WARP_MIN_TICKS)
}

/// Component of velocity along the vector from `pos` toward `target`.
/// Negative if moving away. Used by the warp alignment check.
fn speed_toward(vel: Velocity, pos: Position, target: Position) -> f64 {
    let dx = target.x - pos.x;
    let dy = target.y - pos.y;
    let dz = target.z - pos.z;
    let dist = (dx * dx + dy * dy + dz * dz).sqrt();
    if dist < f64::EPSILON {
        return 0.0;
    }
    (vel.dx * dx + vel.dy * dy + vel.dz * dz) / dist
}

#[cfg(test)]
mod compute_warp_step_tests {
    use super::*;

    #[test]
    fn one_tick_past_total_settles_and_stops() {
        let pos = Position::new(5.0, 0.0, 0.0);
        let (new_pos, new_vel, arrived) = compute_warp_step(
            pos,
            [0.0, 0.0, 0.0],
            [10_000.0, 0.0, 0.0],
            Velocity::ZERO,
            [0.0, 0.0, 0.0],
            10,
            11,
        );
        assert_eq!(new_pos, pos);
        assert_eq!(new_vel, Velocity::ZERO);
        assert!(arrived);
    }

    #[test]
    fn final_tick_lands_exactly_on_arrival() {
        let (new_pos, _new_vel, arrived) = compute_warp_step(
            Position::new(9_000.0, 0.0, 0.0),
            [0.0, 0.0, 0.0],
            [10_000.0, 0.0, 0.0],
            Velocity::ZERO,
            [0.0, 0.0, 0.0],
            10,
            10,
        );
        assert!(!arrived, "the final move tick has not settled yet");
        assert!(
            (new_pos.x - 10_000.0).abs() < 1e-6,
            "t=1 should land exactly on arrival_abs, got {new_pos:?}"
        );
    }

    #[test]
    fn first_tick_velocity_is_close_to_the_start_velocity_rate() {
        // Near t=0 the Hermite tangent should closely track start_vel
        // (per-tick), not snap toward a different rate (the whole point of
        // the Hermite blend over plain smoothstep -- see warp_step's doc
        // comment).
        let start_vel = Velocity {
            dx: 100.0,
            dy: 0.0,
            dz: 0.0,
        };
        let (_pos_t0, vel_t1, _arrived) = compute_warp_step(
            Position::ORIGIN,
            [0.0, 0.0, 0.0],
            [1_000_000.0, 0.0, 0.0],
            start_vel,
            [0.0, 0.0, 0.0],
            1000,
            1,
        );
        assert!(
            (vel_t1.dx - start_vel.dx).abs() < start_vel.dx * 0.05,
            "first-tick velocity {:?} should be within 5% of the start velocity {:?}",
            vel_t1,
            start_vel
        );
        assert_eq!(vel_t1.dy, 0.0);
        assert_eq!(vel_t1.dz, 0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{
        NodeId, PlayerId, Position, SectorBounds, SectorId, ShipId, Velocity, WarpTarget,
    };
    use dawn_ecs::components::{ShipStatsComp, VelocityComp, WarpPhase};

    fn mem_node() -> SimulationNode {
        SimulationNode::new_test(
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

    #[test]
    fn warp_is_rejected_when_the_gate_is_closer_than_the_minimum_warp_distance() {
        // Spawning a ship via a raw absolute `Position` at the gate's AU-scale
        // coordinate bypasses the normal anchor split -- real ships always end
        // up at
        // anchor + small offset (spawn-near-body, warp-arrival rebase). So
        // drive the ship to the gate through the actual (precision-correct)
        // warp arrival instead of spawning "at" the gate directly, then
        // confirm a second warp to the same gate is rejected as too close.
        let mut node = mem_node();
        let (player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        assert!(node.apply_warp_command_owned(
            player,
            ship,
            dawn_core::WarpCommand {
                target: WarpTarget::Gate(dawn_core::JumpGateId(0)),
            }
        ));
        for _ in 0..5_000 {
            node.tick();
            if node.warp_phase(ship).is_none() {
                break;
            }
        }
        assert_eq!(
            node.warp_phase(ship),
            None,
            "first warp should have completed"
        );

        assert!(!node.can_propose_warp(ship, WarpTarget::Gate(dawn_core::JumpGateId(0))));
        assert!(!node.apply_warp_command_owned(
            player,
            ship,
            dawn_core::WarpCommand {
                target: WarpTarget::Gate(dawn_core::JumpGateId(0)),
            }
        ));
        assert_eq!(
            node.warp_phase(ship),
            None,
            "second warp should be rejected, not attached"
        );
    }

    #[test]
    fn starting_a_warp_clears_an_active_orbit() {
        // Orbit and Warp are both persistent steering modes (ADR-0031);
        // starting a Warp must clear an active OrbitComp, not just rely on
        // process_warp() running after process_orbit() in tick order to
        // overwrite ThrustComp every tick.
        let mut node = mem_node();
        let (player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let entity = *node.simulation.ships.index.get(&ship).unwrap();

        assert!(node.apply_orbit_command(
            ship,
            dawn_core::ApproachTarget::Gate(dawn_core::JumpGateId(0)),
            None,
        ));
        assert!(node
            .simulation
            .world
            .get::<dawn_ecs::components::OrbitComp>(entity)
            .is_some());

        assert!(node.apply_warp_command_owned(
            player,
            ship,
            dawn_core::WarpCommand {
                target: WarpTarget::Gate(dawn_core::JumpGateId(0)),
            }
        ));

        assert!(
            node.simulation
                .world
                .get::<dawn_ecs::components::OrbitComp>(entity)
                .is_none(),
            "OrbitComp should be cleared once Warp starts"
        );
    }

    #[test]
    fn warp_is_rejected_for_a_gate_not_in_this_sector() {
        let mut node = mem_node();
        let (player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        assert!(!node.apply_warp_command_owned(
            player,
            ship,
            dawn_core::WarpCommand {
                target: WarpTarget::Gate(dawn_core::JumpGateId(1)),
            }
        ));
        assert_eq!(node.warp_phase(ship), None);
    }

    #[test]
    fn warp_aligns_by_accelerating_then_flies_into_gate_range_and_completes() {
        let mut node = mem_node();
        let (player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        assert!(node.apply_warp_command_owned(
            player,
            ship,
            dawn_core::WarpCommand {
                target: WarpTarget::Gate(dawn_core::JumpGateId(0)),
            }
        ));

        node.tick();
        assert_eq!(node.warp_phase(ship), Some(WarpPhase::Aligning));
        assert!(
            node.get_ship_position(ship).unwrap().x < 100.0,
            "an aligning ship accelerates sublight, far short of warp speed"
        );

        for _ in 0..250 {
            node.tick();
        }
        assert_eq!(
            node.warp_phase(ship),
            None,
            "warp completes and the component is removed"
        );
        assert!(
            node.can_propose_jump(ship, dawn_core::JumpGateId(0)),
            "warp drops the ship inside the gate's activation radius"
        );
    }

    #[test]
    fn warp_align_time_emerges_from_ship_agility() {
        fn ticks_to_engage(mass: f64) -> u32 {
            let mut node = SimulationNode::new_test(
                NodeId(0),
                SectorId(0),
                SectorBounds::centered(SectorBounds::DEFAULT_HALF),
                std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
            );
            let player_id = node.next_player_id();
            let ship = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);
            let entity = *node.simulation.ships.index.get(&ship).unwrap();
            let mut stats = *node.simulation.world.get::<ShipStatsComp>(entity).unwrap();
            stats.mass = mass;
            node.simulation.world.set_ship_stats(entity, stats);
            node.apply_warp_command_owned(
                player_id,
                ship,
                dawn_core::WarpCommand {
                    target: WarpTarget::Gate(dawn_core::JumpGateId(0)),
                },
            );
            for t in 1..=500u32 {
                node.tick();
                if node.warp_phase(ship) == Some(WarpPhase::Warping) {
                    return t;
                }
            }
            u32::MAX
        }
        assert!(
            ticks_to_engage(50_000_000.0) > ticks_to_engage(1_000_000.0),
            "a heavier ship spends longer aligning (a longer tackle window)"
        );
    }

    #[test]
    fn warp_decelerates_smoothly_near_the_gate_instead_of_stopping_dead() {
        let mut node = mem_node();
        let (player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        node.apply_warp_command_owned(
            player,
            ship,
            dawn_core::WarpCommand {
                target: WarpTarget::Gate(dawn_core::JumpGateId(0)),
            },
        );

        let entity = *node.simulation.ships.index.get(&ship).unwrap();
        let mut saw_decel_step = false;
        for _ in 0..250 {
            node.tick();
            let warping = node.warp_phase(ship) == Some(WarpPhase::Warping);
            let v = node.simulation.world.get::<VelocityComp>(entity).unwrap().0;
            let speed = (v.dx * v.dx + v.dy * v.dy + v.dz * v.dz).sqrt();
            if warping && speed > f64::EPSILON && speed < WARP_SPEED * 0.9 {
                saw_decel_step = true;
            }
            if node.warp_phase(ship).is_none() {
                break;
            }
        }
        assert!(
            saw_decel_step,
            "warp must ramp down through intermediate speeds, not stop dead"
        );
        assert_eq!(node.warp_phase(ship), None, "warp should have completed");
    }

    #[test]
    fn warp_engage_carries_the_aligning_speed_into_the_transit_no_snap() {
        // Lore pass (2026-06-23): the Hermite-blended transit must inherit the
        // ship's actual cruise speed at the instant warp engages, instead of
        // snapping toward zero before ramping to warp speed (the plain
        // smoothstep curve this replaced has a zero derivative at t=0, which
        // caused exactly that snap).
        let mut node = mem_node();
        let (player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        node.apply_warp_command_owned(
            player,
            ship,
            dawn_core::WarpCommand {
                target: WarpTarget::Gate(dawn_core::JumpGateId(0)),
            },
        );

        let entity = *node.simulation.ships.index.get(&ship).unwrap();
        let mut speed_before_engage = 0.0_f64;
        loop {
            let phase = node.warp_phase(ship);
            let v = node.simulation.world.get::<VelocityComp>(entity).unwrap().0;
            let speed = (v.dx * v.dx + v.dy * v.dy + v.dz * v.dz).sqrt();
            if phase == Some(WarpPhase::Aligning) {
                speed_before_engage = speed;
            }
            node.tick();
            if node.warp_phase(ship) == Some(WarpPhase::Warping) {
                break;
            }
            assert!(
                node.warp_phase(ship).is_some(),
                "warp should not cancel before engaging"
            );
        }
        let v_after_engage = node.simulation.world.get::<VelocityComp>(entity).unwrap().0;
        let speed_after_engage = (v_after_engage.dx * v_after_engage.dx
            + v_after_engage.dy * v_after_engage.dy
            + v_after_engage.dz * v_after_engage.dz)
            .sqrt();

        assert!(speed_before_engage > 1.0,
            "sanity: should have built up real cruise speed during Aligning (got {speed_before_engage})");
        // The first Warping-phase tick's speed should be close to the cruise
        // speed just before engage, not collapsed toward zero.
        assert!(speed_after_engage > speed_before_engage * 0.5,
            "warp engage should not snap speed toward zero: before={speed_before_engage}, after={speed_after_engage}");
    }

    #[test]
    fn a_move_command_cancels_an_aligning_warp() {
        let mut node = mem_node();
        let (player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        node.apply_warp_command_owned(
            player,
            ship,
            dawn_core::WarpCommand {
                target: WarpTarget::Gate(dawn_core::JumpGateId(0)),
            },
        );
        node.tick();
        assert_eq!(node.warp_phase(ship), Some(WarpPhase::Aligning));

        node.apply_move_command(ship, Position::new(0.0, 1000.0, 0.0));
        assert_eq!(
            node.warp_phase(ship),
            None,
            "a move during alignment cancels the warp"
        );
    }

    #[test]
    fn a_move_command_is_ignored_during_the_committed_warping_phase() {
        let mut node = mem_node();
        let (player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        node.apply_warp_command_owned(
            player,
            ship,
            dawn_core::WarpCommand {
                target: WarpTarget::Gate(dawn_core::JumpGateId(0)),
            },
        );
        for _ in 0..100 {
            node.tick();
            if node.warp_phase(ship) == Some(WarpPhase::Warping) {
                break;
            }
        }
        assert_eq!(
            node.warp_phase(ship),
            Some(WarpPhase::Warping),
            "warp should be committed"
        );

        node.apply_move_command(ship, Position::new(0.0, 1000.0, 0.0));
        assert_eq!(
            node.warp_phase(ship),
            Some(WarpPhase::Warping),
            "a committed warp cannot be interrupted by a move command"
        );
    }

    #[test]
    fn auto_jump_is_queued_in_pending_list_when_warp_completes_with_auto_jump_true() {
        let mut node = mem_node();
        let (_player, ship) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        assert!(node.apply_warp_command(ship, WarpTarget::Gate(dawn_core::JumpGateId(0)), true));

        for _ in 0..250 {
            node.tick();
        }
        assert_eq!(node.warp_phase(ship), None, "warp must complete");
        assert!(
            node.can_propose_jump(ship, dawn_core::JumpGateId(0)),
            "ship must be within gate range after warp"
        );

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

        for _ in 0..250 {
            node.tick();
        }
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
        node.apply_warp_command_owned(
            player,
            ship,
            dawn_core::WarpCommand {
                target: WarpTarget::Gate(dawn_core::JumpGateId(0)),
            },
        );

        let mut warping_ticks: u32 = 0;
        for _ in 0..400 {
            node.tick();
            match node.warp_phase(ship) {
                Some(WarpPhase::Warping) => warping_ticks += 1,
                None if warping_ticks > 0 => break, // warp finished
                _ => {}
            }
        }
        // Allow a small boundary fuzz (the arriving tick removes the component).
        assert!(
            warping_ticks >= WARP_MIN_TICKS - 2,
            "warp should ride the parametric segment for ~WARP_MIN_TICKS ticks, \
             got {warping_ticks} (floor {WARP_MIN_TICKS})"
        );
    }

    // ── Warp-arrival authority (ADR-0029) ────────────────────────────────

    #[test]
    fn finishing_a_warp_surfaces_the_ship_in_drain_completed_warps() {
        // The serve loop drains this to send an authoritative PositionSnap. It
        // must fire for a *gate* warp too (no anchor change), which is exactly
        // the case the old AnchorRebased-only snap missed.
        let mut node = mem_node();
        let (player, ship_id) = spawn_owned_player_at(&mut node, Position::new(0.0, 0.0, 0.0));
        assert!(node.apply_warp_command_owned(
            player,
            ship_id,
            dawn_core::WarpCommand {
                target: WarpTarget::Gate(dawn_core::JumpGateId(0)),
            }
        ));

        let mut arrived = false;
        for _ in 0..5_000 {
            node.tick();
            if node.drain_completed_warps().contains(&ship_id) {
                arrived = true;
                break;
            }
        }
        assert!(
            arrived,
            "a completed gate warp must surface in drain_completed_warps"
        );
        // Drained: it is a per-tick transient, not re-reported.
        assert!(
            node.drain_completed_warps().is_empty(),
            "completed warps drain once"
        );
    }

    #[test]
    fn gate_warp_arrival_is_symmetric_with_body_warp() {
        // ADR-0029 R1 remnant: a Gate warp must land via the gate's f64 `abs_m`
        // source and rebase onto the nearest body anchor on arrival, exactly
        // like a Body warp does -- not stay pinned to the star with a coarse
        // precise f64 arrival point.
        let mut node = mem_node();
        let (player, ship_id) = spawn_owned_player_at(&mut node, Position::new(0.0, 0.0, 0.0));
        assert!(node.apply_warp_command_owned(
            player,
            ship_id,
            dawn_core::WarpCommand {
                target: WarpTarget::Gate(dawn_core::JumpGateId(0)),
            }
        ));

        for _ in 0..5_000 {
            node.tick();
            if node.warp_phase(ship_id).is_none() {
                break;
            }
        }
        assert!(
            node.warp_phase(ship_id).is_none(),
            "gate warp should have completed"
        );

        let gate = *node
            .jump_gate(dawn_core::JumpGateId(0))
            .expect("demo gate 0 exists");
        let expected_anchor = node
            .anchor_table()
            .nearest_anchor(gate.from_sector, gate.abs_m)
            .expect("gate sector has at least one anchor");
        assert_eq!(
            node.get_ship_anchor(ship_id),
            Some(expected_anchor),
            "gate warp arrival should rebase onto the nearest body anchor, same as a body warp"
        );

        // Arrival point must sit `activation_radius * WARP_ARRIVAL_FACTOR`
        // from the gate's f64 source, not an anchor-relative `position`.
        let ship_abs = node.ship_absolute(ship_id).expect("ship exists");
        let d = [
            ship_abs[0] - gate.abs_m[0],
            ship_abs[1] - gate.abs_m[1],
            ship_abs[2] - gate.abs_m[2],
        ];
        let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        // The authoritative position and anchor-relative offset are both f64,
        // so the assertion only needs a small numerical tolerance.
        let expected = gate.activation_radius * WARP_ARRIVAL_FACTOR;
        let tolerance = 1e-6_f64.max(expected * 1e-8);
        assert!((dist - expected).abs() < tolerance,
            "gate arrival distance from the f64 gate source should match the arrival ring (got {dist}, tolerance {tolerance})");
    }

    // ── Celestial body warp (ADR-0025) ───────────────────────────────────

    #[test]
    fn warp_to_body_reaches_arrival_distance_of_radius_times_1_5() {
        let mut node = mem_node();
        let (player, ship_id) = spawn_owned_player_at(&mut node, Position::new(0.0, 0.0, 0.0));

        let body_id = dawn_core::CelestialBodyId(1);
        let ok = node.apply_warp_command_owned(
            player,
            ship_id,
            dawn_core::WarpCommand {
                target: WarpTarget::Body(body_id),
            },
        );
        assert!(ok, "warp to body should be accepted");
        assert!(
            node.warp_phase(ship_id).is_some(),
            "ship should have WarpComp"
        );

        for _ in 0..5_000 {
            node.tick();
            if node.warp_phase(ship_id).is_none() {
                break;
            }
        }
        assert!(
            node.warp_phase(ship_id).is_none(),
            "warp should have completed"
        );

        // ADR-0029 step 4: arriving at a Body rebases the ship onto that body's anchor.
        assert_eq!(
            node.get_ship_anchor(ship_id),
            Some(dawn_core::AnchorId::from(body_id)),
            "warp-to-body should rebase the ship onto the body anchor"
        );

        let body = crate::galaxy::Galaxy::demo()
            .bodies_in_sector(SectorId(0))
            .into_iter()
            .find(|b| b.id == body_id)
            .unwrap();
        // After a Body warp the ship is rebased onto the body's anchor (ADR-0029),
        // so its raw PositionComp is now body-relative. Compare in absolute terms.
        let ship_abs = node.ship_absolute(ship_id).expect("ship exists");
        let dx = ship_abs[0] - body.position.x;
        let dy = ship_abs[1] - body.position.y;
        let dz = ship_abs[2] - body.position.z;
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        let arrival_max = body.radius * 1.5 * 1.05;
        assert!(
            dist <= arrival_max,
            "ship distance {:.0} should be within {:.0} of body centre",
            dist,
            arrival_max,
        );
    }

    #[test]
    fn warp_to_body_is_rejected_for_body_not_in_this_sector() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(0.0, 0.0, 0.0),
            Velocity::ZERO,
        );
        let ok = node.apply_warp_command(
            ship_id,
            WarpTarget::Body(dawn_core::CelestialBodyId(2)),
            false,
        );
        assert!(!ok, "warp to body in another sector should be rejected");
    }

    #[test]
    fn warp_to_station_arrives_inside_the_docking_radius() {
        let mut node = mem_node();
        let (player, ship_id) = spawn_owned_player_at(&mut node, Position::ORIGIN);
        let station_id = dawn_core::StationId(0);
        let station = node
            .station(station_id)
            .expect("demo station exists")
            .clone();

        assert!(
            node.apply_warp_command_owned(
                player,
                ship_id,
                dawn_core::WarpCommand {
                    target: WarpTarget::Station(station_id),
                },
            ),
            "warp to a local station should be accepted"
        );

        for _ in 0..5_000 {
            node.tick();
            if node.warp_phase(ship_id).is_none() {
                break;
            }
        }

        assert!(node.warp_phase(ship_id).is_none(), "warp should complete");
        let ship_abs = node.ship_absolute(ship_id).expect("ship exists");
        assert!(
            station.is_in_range_abs(ship_abs),
            "station warp should arrive inside the docking radius"
        );
        let distance = station.distance_abs(ship_abs);
        let expected = station.docking_radius * WARP_ARRIVAL_FACTOR;
        let tolerance = 1e-6_f64.max(expected * 1e-8);
        assert!(
            (distance - expected).abs() < tolerance,
            "station arrival distance should match the docking ring: got {distance}, expected {expected}"
        );
    }

    #[test]
    fn warp_to_station_is_rejected_for_a_station_not_in_this_sector() {
        let mut node = mem_node();
        let ship_id = node.spawn_ship(
            dawn_core::ShipTypeId(1),
            Position::new(0.0, 0.0, 0.0),
            Velocity::ZERO,
        );
        assert!(!node.apply_warp_command(
            ship_id,
            WarpTarget::Station(dawn_core::StationId(2)),
            false,
        ));
    }
}
