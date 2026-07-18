//! Client-side ship motion prediction and reconciliation (Phase 10).
//!
//! The server remains authoritative. This module only advances the local
//! player's presentation between authoritative corrections, delegating the
//! one-tick movement policy to `dawn-core` (ADR-0023).

const VECTOR_EPSILON: f64 = f64::EPSILON;

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
        let core_max_speed = max_speed as f32;
        let core_mass = mass as f32;
        let core_inertia_modifier = inertia_modifier as f32;

        if !max_speed.is_finite()
            || max_speed < 0.0
            || !mass.is_finite()
            || mass <= 0.0
            || !inertia_modifier.is_finite()
            || inertia_modifier <= 0.0
            || !core_max_speed.is_finite()
            || !core_mass.is_finite()
            || core_mass <= 0.0
            || !core_inertia_modifier.is_finite()
            || core_inertia_modifier <= 0.0
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
        CoreMovementProfile::new(
            self.max_speed as f32,
            self.mass as f32,
            self.inertia_modifier as f32,
        )
        .expect("MotionProfile validates the shared movement profile")
    }
}

/// A deterministic local predictor for one ship.
///
/// `position` and `velocity` use server units. `advance` consumes fractional
/// server ticks but applies the same whole-tick update as the authoritative
/// movement system. The fractional remainder is exposed through
/// [`Self::predicted_position`] so rendering stays smooth without changing
/// the simulation rule.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionPredictor {
    profile: MotionProfile,
    position: [f64; 3],
    velocity: [f64; 3],
    input: MotionInput,
    tick: u64,
    fractional_ticks: f64,
    last_authoritative_tick: Option<u64>,
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
            position,
            velocity,
            input: MotionInput::Coast,
            tick,
            fractional_ticks: 0.0,
            last_authoritative_tick: Some(tick),
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

    /// Apply a server velocity update without discarding the player's current
    /// input. The following correction will still reconcile the exact position.
    pub fn set_velocity(&mut self, velocity: [f64; 3]) {
        self.velocity = velocity;
    }

    /// Advance by a fractional number of server ticks.
    pub fn advance(&mut self, ticks: f64) {
        if !ticks.is_finite() || ticks <= 0.0 {
            return;
        }

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
        self.position = position;
        self.velocity = velocity;
        self.tick = tick;
        self.fractional_ticks = 0.0;
        self.last_authoritative_tick = Some(tick);
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
    }

    pub fn position(&self) -> [f64; 3] {
        self.position
    }

    pub fn predicted_position(&self) -> [f64; 3] {
        add(self.position, scale(self.velocity, self.fractional_ticks))
    }

    pub fn velocity(&self) -> [f64; 3] {
        self.velocity
    }

    pub fn tick(&self) -> u64 {
        self.tick
    }

    fn step_tick(&mut self) {
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

fn to_core_velocity(vector: [f64; 3]) -> Velocity {
    Velocity::new(vector[0] as f32, vector[1] as f32, vector[2] as f32)
}

fn from_core_velocity(vector: Velocity) -> [f64; 3] {
    [vector.dx as f64, vector.dy as f64, vector.dz as f64]
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

        let alpha = 1.0_f32 - (-1.0 / 30.0_f32).exp();
        assert!((predictor.velocity()[0] - f64::from(500.0 * alpha)).abs() < 1e-6);
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
    fn stale_correction_is_ignored() {
        let mut predictor = MotionPredictor::default();
        predictor.reconcile([5.0, 0.0, 0.0], [1.0, 0.0, 0.0], 10);

        assert!(!predictor.reconcile([1.0, 0.0, 0.0], [9.0, 0.0, 0.0], 9));
        assert_eq!(predictor.position(), [5.0, 0.0, 0.0]);
        assert_eq!(predictor.velocity(), [1.0, 0.0, 0.0]);
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
        assert!(MotionProfile::new(f64::from(f32::MAX) * 2.0, 1.0, 0.3).is_none());
        assert!(MotionProfile::new(
            500.0,
            f64::from(f32::MIN_POSITIVE) * f64::from(f32::MIN_POSITIVE),
            0.3
        )
        .is_none());
    }
}
