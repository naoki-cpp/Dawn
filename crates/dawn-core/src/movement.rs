//! Pure one-tick movement policy shared by the authoritative server and client prediction.
//!
//! This module owns the EVE-style exponential approach rule from ADR-0023.
//! ECS state updates, domain-event emission, prediction reconciliation, and
//! rendering remain responsibilities of their respective adapters.

use thiserror::Error;

use crate::Velocity;

/// Converts `mass × inertia_modifier` into a time constant measured in ticks.
pub const MASS_SCALE: f64 = 100_000.0;

/// Velocity magnitude below which braking snaps to exactly zero.
pub const BRAKE_STOP_EPSILON: f64 = 0.001;

/// Movement values required by the one-tick policy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MovementProfile {
    max_speed: f64,
    mass: f64,
    inertia_modifier: f64,
}

impl Default for MovementProfile {
    fn default() -> Self {
        Self {
            max_speed: 500.0,
            mass: 10_000_000.0,
            inertia_modifier: 0.3,
        }
    }
}

impl MovementProfile {
    /// Creates a profile when all values are finite and physically valid.
    pub fn new(
        max_speed: f64,
        mass: f64,
        inertia_modifier: f64,
    ) -> Result<Self, MovementProfileError> {
        if !max_speed.is_finite() {
            return Err(MovementProfileError::MaxSpeedNotFinite { value: max_speed });
        }
        if max_speed < 0.0 {
            return Err(MovementProfileError::MaxSpeedNegative { value: max_speed });
        }
        if !mass.is_finite() {
            return Err(MovementProfileError::MassNotFinite { value: mass });
        }
        if mass <= 0.0 {
            return Err(MovementProfileError::MassNotPositive { value: mass });
        }
        if !inertia_modifier.is_finite() {
            return Err(MovementProfileError::InertiaModifierNotFinite {
                value: inertia_modifier,
            });
        }
        if inertia_modifier <= 0.0 {
            return Err(MovementProfileError::InertiaModifierNotPositive {
                value: inertia_modifier,
            });
        }

        Ok(Self {
            max_speed,
            mass,
            inertia_modifier,
        })
    }

    /// Returns the effective maximum speed in units per tick.
    pub fn max_speed(self) -> f64 {
        self.max_speed
    }

    /// Returns the total mass used to derive the time constant.
    pub fn mass(self) -> f64 {
        self.mass
    }

    /// Returns the inertia modifier used to derive the time constant.
    pub fn inertia_modifier(self) -> f64 {
        self.inertia_modifier
    }

    /// Advance one logical tick without owning any simulation state.
    ///
    /// `Velocity` is also the one-tick displacement in Dawn's domain model.
    /// A zero thrust direction therefore has the same inertial coast behavior
    /// as the server's existing `ThrustComp::ZERO` state.
    pub fn step(self, velocity: Velocity, input: MovementInput) -> MovementStep {
        let target = match input {
            MovementInput::Brake => Velocity::ZERO,
            MovementInput::Thrust(direction) => {
                let magnitude = direction.speed();
                if magnitude > f64::EPSILON {
                    let scale = self.max_speed / magnitude;
                    Velocity::new(
                        direction.dx * scale,
                        direction.dy * scale,
                        direction.dz * scale,
                    )
                } else {
                    velocity
                }
            }
            MovementInput::Coast => velocity,
        };

        let tau = (self.mass * self.inertia_modifier / MASS_SCALE).max(f64::EPSILON);
        let alpha = 1.0_f64 - (-1.0 / tau).exp();
        let mut next_velocity = Velocity::new(
            velocity.dx + (target.dx - velocity.dx) * alpha,
            velocity.dy + (target.dy - velocity.dy) * alpha,
            velocity.dz + (target.dz - velocity.dz) * alpha,
        );

        let braking_complete =
            matches!(input, MovementInput::Brake) && next_velocity.speed() < BRAKE_STOP_EPSILON;
        if braking_complete {
            next_velocity = Velocity::ZERO;
        }

        MovementStep {
            velocity: next_velocity,
            braking_complete,
        }
    }
}

/// Steering input understood by the shared one-tick policy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MovementInput {
    /// Preserve the current velocity (inertial flight).
    Coast,
    /// Approach max speed in this direction. The policy normalizes it.
    Thrust(Velocity),
    /// Approach zero velocity and report when the stop guard completes.
    Brake,
}

/// The state transition produced by one policy tick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MovementStep {
    /// The next velocity and the displacement for this logical tick.
    pub velocity: Velocity,
    /// Whether the caller should clear its braking input.
    pub braking_complete: bool,
}

/// Why a movement profile was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum MovementProfileError {
    #[error("max_speed must be finite, got {value}")]
    MaxSpeedNotFinite { value: f64 },
    #[error("max_speed must not be negative, got {value}")]
    MaxSpeedNegative { value: f64 },
    #[error("mass must be finite, got {value}")]
    MassNotFinite { value: f64 },
    #[error("mass must be positive, got {value}")]
    MassNotPositive { value: f64 },
    #[error("inertia_modifier must be finite, got {value}")]
    InertiaModifierNotFinite { value: f64 },
    #[error("inertia_modifier must be positive, got {value}")]
    InertiaModifierNotPositive { value: f64 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_thrust_tick_matches_the_authoritative_exponential_rule() {
        let profile = MovementProfile::new(500.0, 10_000_000.0, 0.3).unwrap();
        let step = profile.step(
            Velocity::ZERO,
            MovementInput::Thrust(Velocity::new(2.0, 0.0, 0.0)),
        );
        let alpha = 1.0_f64 - (-1.0 / 30.0_f64).exp();

        assert!((step.velocity.dx - 500.0 * alpha).abs() < 1e-6);
        assert_eq!(step.velocity.dy, 0.0);
        assert_eq!(step.velocity.dz, 0.0);
        assert!(!step.braking_complete);
    }

    #[test]
    fn zero_thrust_direction_preserves_inertial_coasting() {
        let profile = MovementProfile::default();
        let velocity = Velocity::new(12.0, -3.0, 1.0);

        assert_eq!(
            profile
                .step(velocity, MovementInput::Thrust(Velocity::ZERO))
                .velocity,
            velocity
        );
    }

    #[test]
    fn braking_snaps_to_zero_and_reports_completion() {
        let profile = MovementProfile::default();
        let step = profile.step(Velocity::new(0.0005, 0.0, 0.0), MovementInput::Brake);

        assert_eq!(step.velocity, Velocity::ZERO);
        assert!(step.braking_complete);
    }

    #[test]
    fn invalid_profiles_return_descriptive_errors() {
        assert!(matches!(
            MovementProfile::new(f64::NAN, 1.0, 0.3),
            Err(MovementProfileError::MaxSpeedNotFinite { value }) if value.is_nan()
        ));
        assert!(matches!(
            MovementProfile::new(-1.0, 1.0, 0.3),
            Err(MovementProfileError::MaxSpeedNegative { value }) if value == -1.0
        ));
        assert!(matches!(
            MovementProfile::new(1.0, 0.0, 0.3),
            Err(MovementProfileError::MassNotPositive { value }) if value == 0.0
        ));
        assert!(matches!(
            MovementProfile::new(1.0, 1.0, f64::NAN),
            Err(MovementProfileError::InertiaModifierNotFinite { value }) if value.is_nan()
        ));
    }
}
