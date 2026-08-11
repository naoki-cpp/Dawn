//! Client-side ship motion prediction and reconciliation (Phase 10).
//!
//! The server remains authoritative. This module advances each ship's client
//! presentation between authoritative corrections, using prediction for the
//! local ship and dead-reckoning for remote ships. Both modes delegate the
//! one-tick movement policy to `dawn-core` (ADR-0023).

const VECTOR_EPSILON: f64 = f64::EPSILON;
const RENDER_CORRECTION_DECAY_PER_TICK: f64 = 0.35;

use dawn_core::{
    MovementInput as CoreMovementInput, MovementProfile as CoreMovementProfile, Velocity,
};

/// The movement inputs that can be predicted locally.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MotionInput {
    /// Keep the current velocity (inertial flight).
    Coast,
    /// Accelerate toward this direction at `MotionProfile::max_speed`.
    Thrust([f64; 3]),
    /// Decelerate toward zero using the ship's inertia.
    Brake,
}

/// The presentation state owned by one client motion track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionState {
    Prediction,
    DeadReckoning,
    WarpPresentation,
    Docked,
}

/// Runtime movement values needed to mirror the server's movement system.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionProfile {
    max_speed: f64,
    mass: f64,
    inertia_modifier: f64,
}

impl Default for MotionProfile {
    fn default() -> Self {
        Self {
            max_speed: 500.0,
            mass: 10_000_000.0,
            inertia_modifier: 0.3,
        }
    }
}

impl MotionProfile {
    /// Creates a profile when all movement values are finite and valid.
    pub fn new(max_speed: f64, mass: f64, inertia_modifier: f64) -> Option<Self> {
        if !max_speed.is_finite()
            || max_speed < 0.0
            || !mass.is_finite()
            || mass <= 0.0
            || !inertia_modifier.is_finite()
            || inertia_modifier <= 0.0
        {
            return None;
        }

        Some(Self {
            max_speed,
            mass,
            inertia_modifier,
        })
    }

    pub fn max_speed(self) -> f64 {
        self.max_speed
    }

    pub fn mass(self) -> f64 {
        self.mass
    }

    pub fn inertia_modifier(self) -> f64 {
        self.inertia_modifier
    }

    fn as_core(self) -> CoreMovementProfile {
        CoreMovementProfile::new(self.max_speed, self.mass, self.inertia_modifier)
            .expect("MotionProfile validates the shared movement profile")
    }
}

/// A deterministic client motion track for one ship.
///
/// `position` and `velocity` use server units. `advance` consumes fractional
/// server ticks but applies the same whole-tick update as the authoritative
/// movement system. Local tracks apply `MotionInput`; remote tracks advance
/// their last authoritative velocity. The fractional remainder is exposed
/// through [`Self::predicted_position`] so rendering stays smooth without
/// changing the simulation rule.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionPredictor {
    profile: MotionProfile,
    state: MotionState,
    position: [f64; 3],
    velocity: [f64; 3],
    input: MotionInput,
    tick: u64,
    fractional_ticks: f64,
    last_authoritative_tick: Option<u64>,
    render_correction: [f64; 3],
    has_rendered: bool,
    warp_render_position: Option<[f64; 3]>,
    warp_world_position: Option<[f64; 3]>,
    warp_server_speed_cap: Option<f64>,
}

impl Default for MotionPredictor {
    fn default() -> Self {
        Self::new(MotionProfile::default(), [0.0; 3], [0.0; 3], 0)
    }
}

impl MotionPredictor {
    pub fn new(profile: MotionProfile, position: [f64; 3], velocity: [f64; 3], tick: u64) -> Self {
        Self {
            profile,
            state: MotionState::Prediction,
            position,
            velocity,
            input: MotionInput::Coast,
            tick,
            fractional_ticks: 0.0,
            last_authoritative_tick: Some(tick),
            render_correction: [0.0; 3],
            has_rendered: false,
            warp_render_position: None,
            warp_world_position: None,
            warp_server_speed_cap: None,
        }
    }

    /// Replace the ship profile and seed the predictor from an authoritative
    /// spawn state. This also clears any stale local input.
    pub fn configure(
        &mut self,
        profile: MotionProfile,
        position: [f64; 3],
        velocity: [f64; 3],
        tick: u64,
    ) {
        *self = Self::new(profile, position, velocity, tick);
    }

    /// Configure the same motion track for a remote ship.
    ///
    /// Remote ships do not receive local steering input. They advance their
    /// last authoritative velocity between `VelocityChanged` messages while
    /// retaining the same fractional-tick presentation and reset behavior as
    /// the local prediction path.
    pub fn configure_dead_reckoning(
        &mut self,
        profile: MotionProfile,
        position: [f64; 3],
        velocity: [f64; 3],
        tick: u64,
    ) {
        *self = Self::new(profile, position, velocity, tick);
        self.state = MotionState::DeadReckoning;
    }

    /// Switch an existing track to local movement prediction without
    /// discarding its authoritative position, velocity, or tick.
    pub fn enable_prediction(&mut self) {
        self.state = MotionState::Prediction;
        self.warp_render_position = None;
        self.warp_world_position = None;
        self.warp_server_speed_cap = None;
    }

    /// Switch an existing track to remote constant-velocity dead reckoning.
    /// Local input is cleared because it is not meaningful for remote ships.
    pub fn enable_dead_reckoning(&mut self) {
        self.state = MotionState::DeadReckoning;
        self.input = MotionInput::Coast;
        self.warp_render_position = None;
        self.warp_world_position = None;
        self.warp_server_speed_cap = None;
    }

    /// Enter the committed-warp presentation state.
    ///
    /// The authoritative track still advances at the reported velocity, but
    /// rendering advances at a bounded speed until an authoritative reset
    /// arrives. This keeps warp presentation policy in the shared Rust track.
    /// `server_speed_cap` is expressed in server-space units per tick.
    pub fn begin_warp(&mut self, server_speed_cap: f64) -> bool {
        if !server_speed_cap.is_finite() || server_speed_cap <= 0.0 {
            return false;
        }
        let presentation_position = self.predicted_position();
        self.state = MotionState::WarpPresentation;
        self.input = MotionInput::Coast;
        self.warp_render_position
            .get_or_insert(presentation_position);
        self.warp_world_position
            .get_or_insert(presentation_position);
        self.warp_server_speed_cap = Some(server_speed_cap);
        true
    }

    /// Snap the track into the docked state. Docked tracks do not integrate.
    /// Returns `false` when the event is older than the latest authoritative
    /// state already accepted by this track.
    pub fn dock(&mut self, position: [f64; 3], tick: u64) -> bool {
        if self.last_authoritative_tick.is_some_and(|last| tick < last) {
            return false;
        }
        self.position = position;
        self.velocity = [0.0; 3];
        self.input = MotionInput::Coast;
        self.state = MotionState::Docked;
        self.tick = tick;
        self.fractional_ticks = 0.0;
        self.last_authoritative_tick = Some(tick);
        self.render_correction = [0.0; 3];
        self.has_rendered = false;
        self.warp_render_position = None;
        self.warp_world_position = None;
        self.warp_server_speed_cap = None;
        true
    }

    /// Leave the docked state with an explicit local/remote motion mode.
    /// Returns `false` when the event is older than the latest authoritative
    /// state already accepted by this track.
    pub fn undock(
        &mut self,
        position: [f64; 3],
        velocity: [f64; 3],
        tick: u64,
        predict_locally: bool,
    ) -> bool {
        if self.last_authoritative_tick.is_some_and(|last| tick < last) {
            return false;
        }
        self.reset(position, velocity, tick);
        if predict_locally {
            self.enable_prediction();
        } else {
            self.enable_dead_reckoning();
        }
        true
    }

    pub fn state(&self) -> MotionState {
        self.state
    }

    pub fn profile(&self) -> MotionProfile {
        self.profile
    }

    pub fn input(&self) -> MotionInput {
        self.input
    }

    pub fn set_thrust(&mut self, direction: [f64; 3]) {
        self.input = if magnitude(direction) <= VECTOR_EPSILON {
            MotionInput::Coast
        } else {
            MotionInput::Thrust(normalize(direction))
        };
    }

    pub fn set_braking(&mut self) {
        self.input = MotionInput::Brake;
    }

    pub fn clear_input(&mut self) {
        self.input = MotionInput::Coast;
    }

    /// Apply a server velocity update without rewinding the presentation clock.
    ///
    /// `VelocityChanged` carries no position, so the track cannot reconstruct a
    /// past position from the event alone. It records the event's authority
    /// tick for ordering, updates the velocity used by future integration, and
    /// leaves the current predicted position/tick untouched. An accompanying
    /// `MotionCorrection` supplies the authoritative position for the owner.
    /// Returns `false` for stale events or events targeting a docked track.
    pub fn set_velocity_at_tick(&mut self, velocity: [f64; 3], tick: u64) -> bool {
        if self.state == MotionState::Docked
            || self.last_authoritative_tick.is_some_and(|last| tick < last)
        {
            return false;
        }
        let rendered_position = self.has_rendered.then(|| self.predicted_position());
        self.velocity = velocity;
        self.last_authoritative_tick = Some(tick);
        if let Some(rendered_position) = rendered_position {
            let base_position = add(self.position, scale(self.velocity, self.fractional_ticks));
            self.render_correction = subtract(rendered_position, base_position);
        }
        true
    }

    /// Advance by a fractional number of server ticks.
    pub fn advance(&mut self, ticks: f64) {
        if !ticks.is_finite() || ticks <= 0.0 {
            return;
        }

        if self.state == MotionState::Docked {
            return;
        }

        if self.state == MotionState::WarpPresentation {
            let world_position = self
                .warp_world_position
                .unwrap_or_else(|| self.predicted_position());
            self.warp_world_position = Some(add(world_position, scale(self.velocity, ticks)));

            let speed = magnitude(self.velocity);
            if speed > VECTOR_EPSILON {
                let direction = scale(self.velocity, 1.0 / speed);
                let cap = self.warp_server_speed_cap.unwrap_or(speed);
                let render_position = self
                    .warp_render_position
                    .unwrap_or_else(|| self.predicted_position());
                self.warp_render_position =
                    Some(add(render_position, scale(direction, cap * ticks)));
            }
        }

        self.has_rendered = true;
        self.render_correction = scale(
            self.render_correction,
            (-RENDER_CORRECTION_DECAY_PER_TICK * ticks).exp(),
        );
        self.fractional_ticks += ticks;
        while self.fractional_ticks >= 1.0 {
            self.step_tick();
            self.fractional_ticks -= 1.0;
        }
    }

    /// Accept an authoritative position and velocity if it is not stale.
    /// Local input is intentionally preserved so prediction resumes after the
    /// correction instead of waiting for another click.
    pub fn reconcile(&mut self, position: [f64; 3], velocity: [f64; 3], tick: u64) -> bool {
        if self.last_authoritative_tick.is_some_and(|last| tick < last) {
            return false;
        }
        let rendered_position = self.has_rendered.then(|| self.predicted_position());
        self.position = position;
        self.velocity = velocity;
        self.tick = tick;
        self.fractional_ticks = 0.0;
        self.last_authoritative_tick = Some(tick);
        self.render_correction = rendered_position
            .map(|rendered| subtract(rendered, position))
            .unwrap_or([0.0; 3]);
        true
    }

    /// Reset to an authoritative discontinuity such as docking or warp arrival.
    pub fn reset(&mut self, position: [f64; 3], velocity: [f64; 3], tick: u64) {
        self.position = position;
        self.velocity = velocity;
        self.input = MotionInput::Coast;
        self.tick = tick;
        self.fractional_ticks = 0.0;
        self.last_authoritative_tick = Some(tick);
        self.render_correction = [0.0; 3];
        self.has_rendered = false;
        self.warp_render_position = None;
        self.warp_world_position = None;
        self.warp_server_speed_cap = None;
    }

    /// Shift the motion track when the Godot floating origin moves.
    ///
    /// Velocity and tick state are invariant under a coordinate-frame shift;
    /// only the authoritative position needs to move with the rendered node.
    pub fn rebase(&mut self, shift: [f64; 3]) {
        self.position = add(self.position, shift);
        if let Some(render_position) = self.warp_render_position.as_mut() {
            *render_position = add(*render_position, shift);
        }
        if let Some(world_position) = self.warp_world_position.as_mut() {
            *world_position = add(*world_position, shift);
        }
    }

    pub fn position(&self) -> [f64; 3] {
        self.position
    }

    pub fn predicted_position(&self) -> [f64; 3] {
        if self.state == MotionState::WarpPresentation {
            if let Some(render_position) = self.warp_render_position {
                return render_position;
            }
        }
        add(
            add(self.position, scale(self.velocity, self.fractional_ticks)),
            self.render_correction,
        )
    }

    pub(crate) fn world_presentation_position(&self) -> [f64; 3] {
        if self.state == MotionState::WarpPresentation {
            if let Some(world_position) = self.warp_world_position {
                return world_position;
            }
        }
        self.predicted_position()
    }

    pub fn velocity(&self) -> [f64; 3] {
        self.velocity
    }

    pub fn tick(&self) -> u64 {
        self.tick
    }

    fn step_tick(&mut self) {
        if self.state != MotionState::Prediction {
            self.position = add(self.position, self.velocity);
            self.tick = self.tick.saturating_add(1);
            return;
        }

        let input = match self.input {
            MotionInput::Coast => CoreMovementInput::Coast,
            MotionInput::Thrust(direction) => {
                CoreMovementInput::Thrust(to_core_velocity(direction))
            }
            MotionInput::Brake => CoreMovementInput::Brake,
        };

        let step = self
            .profile
            .as_core()
            .step(to_core_velocity(self.velocity), input);
        self.velocity = from_core_velocity(step.velocity);
        if step.braking_complete {
            self.input = MotionInput::Coast;
        }

        self.position = add(self.position, self.velocity);
        self.tick = self.tick.saturating_add(1);
    }
}

fn normalize(vector: [f64; 3]) -> [f64; 3] {
    let length = magnitude(vector);
    if length <= VECTOR_EPSILON {
        [0.0; 3]
    } else {
        scale(vector, 1.0 / length)
    }
}

fn magnitude(vector: [f64; 3]) -> f64 {
    vector
        .iter()
        .map(|component| component * component)
        .sum::<f64>()
        .sqrt()
}

fn add(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [left[0] + right[0], left[1] + right[1], left[2] + right[2]]
}

fn subtract(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn to_core_velocity(vector: [f64; 3]) -> Velocity {
    Velocity::new(vector[0], vector[1], vector[2])
}

fn from_core_velocity(vector: Velocity) -> [f64; 3] {
    [vector.dx, vector.dy, vector.dz]
}

fn scale(vector: [f64; 3], factor: f64) -> [f64; 3] {
    [vector[0] * factor, vector[1] * factor, vector[2] * factor]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_predicted_tick_matches_server_exponential_approach() {
        let profile = MotionProfile::new(500.0, 10_000_000.0, 0.3).expect("valid profile");
        let mut predictor = MotionPredictor::new(profile, [0.0; 3], [0.0; 3], 0);
        predictor.set_thrust([2.0, 0.0, 0.0]);
        predictor.advance(1.0);

        let alpha = 1.0_f64 - (-1.0 / 30.0_f64).exp();
        assert!((predictor.velocity()[0] - (500.0 * alpha)).abs() < 1e-6);
        assert_eq!(predictor.position(), predictor.velocity());
    }

    #[test]
    fn fractional_ticks_render_forward_without_advancing_authoritative_tick() {
        let mut predictor = MotionPredictor::default();
        predictor.reconcile([10.0, 0.0, 0.0], [4.0, 0.0, 0.0], 7);
        predictor.advance(0.5);

        assert_eq!(predictor.position(), [10.0, 0.0, 0.0]);
        assert_eq!(predictor.predicted_position(), [12.0, 0.0, 0.0]);
    }

    #[test]
    fn remote_track_dead_reckons_with_authoritative_velocity() {
        let profile = MotionProfile::default();
        let mut track = MotionPredictor::default();
        track.configure_dead_reckoning(profile, [10.0, 0.0, 0.0], [4.0, 0.0, 0.0], 7);

        track.advance(0.5);
        assert_eq!(track.position(), [10.0, 0.0, 0.0]);
        assert_eq!(track.predicted_position(), [12.0, 0.0, 0.0]);
        assert_eq!(track.tick(), 7);

        track.advance(0.5);
        assert_eq!(track.position(), [14.0, 0.0, 0.0]);
        assert_eq!(track.predicted_position(), [14.0, 0.0, 0.0]);
        assert_eq!(track.tick(), 8);
    }

    #[test]
    fn warp_state_caps_rendering_without_changing_authoritative_position() {
        let mut track = MotionPredictor::default();
        track.reconcile([0.0; 3], [3_000.0, 0.0, 0.0], 7);

        assert!(track.begin_warp(2_000.0));
        assert_eq!(track.state(), MotionState::WarpPresentation);
        track.advance(1.0);

        assert_eq!(track.position(), [3_000.0, 0.0, 0.0]);
        assert_eq!(track.predicted_position(), [2_000.0, 0.0, 0.0]);
    }

    #[test]
    fn docking_stops_integration_until_explicit_undock() {
        let mut track = MotionPredictor::default();
        assert!(track.dock([10.0, 0.0, 0.0], 12));

        track.advance(5.0);
        assert_eq!(track.state(), MotionState::Docked);
        assert_eq!(track.position(), [10.0, 0.0, 0.0]);
        assert_eq!(track.velocity(), [0.0; 3]);

        assert!(track.undock([10.0, 0.0, 0.0], [4.0, 0.0, 0.0], 13, true));
        track.advance(1.0);
        assert_eq!(track.state(), MotionState::Prediction);
        assert!(track.position()[0] > 10.0);
    }

    #[test]
    fn stale_dock_transitions_are_ignored() {
        let mut track = MotionPredictor::default();
        assert!(track.dock([10.0, 0.0, 0.0], 12));
        assert!(track.undock([10.0, 0.0, 0.0], [4.0, 0.0, 0.0], 13, true));

        assert!(!track.dock([20.0, 0.0, 0.0], 12));
        assert!(!track.undock([20.0, 0.0, 0.0], [8.0, 0.0, 0.0], 11, false));
        assert_eq!(track.state(), MotionState::Prediction);
        assert_eq!(track.position(), [10.0, 0.0, 0.0]);
    }

    #[test]
    fn a_remote_track_can_become_a_local_prediction_track_without_losing_state() {
        let profile = MotionProfile::new(500.0, 10_000_000.0, 0.3).expect("valid profile");
        let mut track = MotionPredictor::default();
        track.configure_dead_reckoning(profile, [10.0, 0.0, 0.0], [4.0, 0.0, 0.0], 7);
        track.enable_prediction();
        track.set_thrust([1.0, 0.0, 0.0]);
        track.advance(1.0);

        assert!(track.position()[0] > 10.0);
        assert!(track.velocity()[0] > 4.0);
        assert_eq!(track.tick(), 8);
    }

    #[test]
    fn rebasing_shifts_authoritative_and_fractional_positions_only() {
        let mut track = MotionPredictor::default();
        track.reconcile([10.0, 0.0, 0.0], [4.0, 0.0, 0.0], 7);
        track.advance(0.5);

        track.rebase([100.0, 2.0, -3.0]);

        assert_eq!(track.position(), [110.0, 2.0, -3.0]);
        assert_eq!(track.predicted_position(), [112.0, 2.0, -3.0]);
        assert_eq!(track.velocity(), [4.0, 0.0, 0.0]);
        assert_eq!(track.tick(), 7);
    }

    #[test]
    fn stale_correction_is_ignored() {
        let mut predictor = MotionPredictor::default();
        predictor.reconcile([5.0, 0.0, 0.0], [1.0, 0.0, 0.0], 10);

        assert!(!predictor.reconcile([1.0, 0.0, 0.0], [9.0, 0.0, 0.0], 9));
        assert_eq!(predictor.position(), [5.0, 0.0, 0.0]);
        assert_eq!(predictor.velocity(), [1.0, 0.0, 0.0]);
    }

    #[test]
    fn velocity_events_record_authority_tick_without_rewinding_prediction() {
        let mut predictor = MotionPredictor::default();
        predictor.reconcile([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], 10);
        predictor.advance(2.5);
        let rendered_before = predictor.predicted_position();

        assert!(predictor.set_velocity_at_tick([2.0, 0.0, 0.0], 12));
        assert_eq!(predictor.tick(), 12);
        assert_eq!(predictor.velocity(), [2.0, 0.0, 0.0]);
        assert_eq!(predictor.predicted_position(), rendered_before);

        assert!(!predictor.set_velocity_at_tick([9.0, 0.0, 0.0], 11));
        assert_eq!(predictor.velocity(), [2.0, 0.0, 0.0]);
    }

    #[test]
    fn velocity_events_do_not_reactivate_a_docked_track() {
        let mut predictor = MotionPredictor::default();
        assert!(predictor.dock([10.0, 0.0, 0.0], 12));

        assert!(!predictor.set_velocity_at_tick([4.0, 0.0, 0.0], 13));
        assert_eq!(predictor.velocity(), [0.0; 3]);
        assert_eq!(predictor.state(), MotionState::Docked);
    }

    #[test]
    fn late_correction_does_not_teleport_the_rendered_position_backwards() {
        let mut predictor = MotionPredictor::default();
        predictor.reconcile([0.0, 0.0, 0.0], [10.0, 0.0, 0.0], 0);
        predictor.advance(3.25);
        let rendered_before = predictor.predicted_position()[0];

        // A correction for tick 1 arrives after the client has already
        // rendered beyond tick 3. The authoritative base may move backwards,
        // but the presentation position must converge instead of snapping.
        predictor.reconcile([10.0, 0.0, 0.0], [10.0, 0.0, 0.0], 1);

        assert!(
            predictor.predicted_position()[0] >= rendered_before,
            "late correction moved presentation backwards: before={rendered_before}, after={}",
            predictor.predicted_position()[0]
        );
    }

    #[test]
    fn braking_stops_and_clears_the_brake_input() {
        let mut predictor = MotionPredictor::default();
        predictor.reconcile([0.0; 3], [0.0005, 0.0, 0.0], 1);
        predictor.set_braking();

        predictor.advance(1.0);

        assert_eq!(predictor.velocity(), [0.0; 3]);
        assert_eq!(predictor.input(), MotionInput::Coast);
    }

    #[test]
    fn invalid_motion_profiles_are_rejected() {
        assert!(MotionProfile::new(-1.0, 10_000_000.0, 0.3).is_none());
        assert!(MotionProfile::new(500.0, 0.0, 0.3).is_none());
        assert!(MotionProfile::new(500.0, 10_000_000.0, f64::NAN).is_none());
        assert!(MotionProfile::new(f64::INFINITY, 1.0, 0.3).is_none());
        assert!(MotionProfile::new(500.0, f64::MIN_POSITIVE * f64::MIN_POSITIVE, 0.3).is_none());
    }
}
